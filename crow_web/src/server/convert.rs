//! Conversion between axum types and Crow core types.
//!
//! This module is the bridge: axum's `Request` → `CrowRequest`,
//! and `CrowResponse` → axum's `Response`.


use axum::{
    body::Body,
    extract::Request,
    http::{header, response::Builder, StatusCode},
    response::{IntoResponse, Response},
};

use crate::core::{CrowRequest, CrowResponse};

/// Extract a `CrowRequest` from an axum `Request`: its method, path,
/// headers, query string, body and form. Route params are not set here —
/// the handler wrapper in `app.rs` injects them from axum's `Path`.
///
/// A body bigger than the app allows is answered `413 Payload Too Large`
/// before a handler sees it, and one that does not parse as the form it
/// says it is `400 Bad Request` (a body had been read whatever its size).
pub async fn request_from_axum(req: Request) -> Result<CrowRequest, Response> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    // Headers → a map (lowercase keys), in Crow's own map order
    let headers: std::collections::BTreeMap<String, String> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Query string → a map
    let query = req
        .uri()
        .query()
        .map(CrowRequest::parse_query_string)
        .unwrap_or_default();

    let limit = super::uploads::limit();
    let body_bytes = match axum::body::to_bytes(req.into_body(), limit).await {
        Ok(b) => b,
        Err(e) => {
            let too_big = e.to_string().contains("length limit");
            let (status, why) = if too_big {
                (StatusCode::PAYLOAD_TOO_LARGE, format!("the body is bigger than this app takes ({} MB): serve[…, uploads = N] raises it", limit / (1024 * 1024)))
            } else {
                (StatusCode::BAD_REQUEST, format!("the body could not be read: {}", e))
            };
            return Err((status, why).into_response());
        }
    };
    let content_type = headers.get("content-type").cloned().unwrap_or_default();
    let (form, files) = match super::uploads::form_of(&content_type, &body_bytes).await {
        Ok(f) => f,
        Err(e) => return Err((StatusCode::BAD_REQUEST, e).into_response()),
    };
    // a multipart body is its fields and files, not text
    let body = if content_type.to_ascii_lowercase().starts_with("multipart/") { String::new() } else { String::from_utf8_lossy(&body_bytes).to_string() };

    Ok(CrowRequest {
        method,
        path,
        headers,
        query,
        body,
        params: std::collections::BTreeMap::new(), // filled by handler wrapper
        form,
        files,
    })
}

/// Convert a `CrowResponse` into an axum `Response`.
///
/// A feed (`stream[]`) takes priority: it runs on a blocking thread and
/// each chunk it puts is sent as it comes. Then a `binary_body` (used by
/// `msgpack[]`, `arrow[]` and `bytes[]`), then the text `body`.
pub fn response_to_axum(crow_resp: CrowResponse) -> Response {
    // a status or a header HTTP cannot carry — a header a handler built from
    // what a request sent, holding a line break that would split the
    // response — answers 500 with the reason, and the server's log says so
    // (it had answered `internal error`, and said nothing)
    if let Some(why) = unsendable(&crow_resp) {
        let id = crate::core::response::failure_id();
        eprintln!("crow: a response could not be sent [{}]: {}", id, why);
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("the request could not be answered — {}{}", crate::core::response::LOG_HAS_WHY, id)).into_response();
    }
    let mut builder = Builder::new().status(
        StatusCode::from_u16(crow_resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    );

    for (k, v) in &crow_resp.headers {
        // (a value's text beyond ASCII is sent as its bytes)
        if let Ok(value) = header::HeaderValue::from_bytes(v.as_bytes()) {
            builder = builder.header(k.as_str(), value);
        }
    }

    // If no content-type was set, default to text/plain
    if !crow_resp.headers.contains_key("content-type") {
        builder = builder.header(header::CONTENT_TYPE, "text/plain");
    }

    let body = if let Some(feed) = crow_resp.feed {
        // a few chunks wait while the reader catches up; a reader who has
        // gone closes the channel, and put answers false
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(16);
        tokio::task::spawn_blocking(move || feed.run(move |chunk| tx.blocking_send(Ok(chunk)).is_ok()));
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
    } else if let Some(bytes) = crow_resp.binary_body {
        Body::from(bytes)
    } else {
        Body::from(crow_resp.body)
    };

    builder
        .body(body)
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        })
}

/// Why a response cannot be sent, when it cannot: a status outside HTTP's,
/// a header's name that is not one, a header's value holding a line break
/// or another control character.
fn unsendable(r: &CrowResponse) -> Option<String> {
    if StatusCode::from_u16(r.status).is_err() {
        return Some(format!("{} is not an HTTP status (100 to 999)", r.status));
    }
    for (k, v) in &r.headers {
        if header::HeaderName::from_bytes(k.as_bytes()).is_err() {
            return Some(format!("'{}' is not a header's name", k.escape_debug()));
        }
        if header::HeaderValue::from_bytes(v.as_bytes()).is_err() {
            return Some(format!("the header '{}' holds a line break or another control character", k));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;

    #[tokio::test]
    async fn test_request_from_axum_basic() {
        let req = HttpRequest::builder()
            .method("POST")
            .uri("/users?page=2&limit=5")
            .header("content-type", "application/json")
            .header("x-custom", "hello")
            .body(Body::from(r#"{"name":"alice"}"#))
            .unwrap();

        let crow_req = request_from_axum(req).await.unwrap_or_else(|_| panic!("a request"));
        assert_eq!(crow_req.method, "POST");
        assert_eq!(crow_req.path, "/users");
        assert_eq!(crow_req.query.get("page").unwrap(), "2");
        assert_eq!(crow_req.query.get("limit").unwrap(), "5");
        assert_eq!(
            crow_req.headers.get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(crow_req.headers.get("x-custom").unwrap(), "hello");
        assert_eq!(crow_req.body, r#"{"name":"alice"}"#);
        assert!(crow_req.params.is_empty());
    }

    #[tokio::test]
    async fn test_request_from_axum_no_query() {
        let req = HttpRequest::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let crow_req = request_from_axum(req).await.unwrap_or_else(|_| panic!("a request"));
        assert_eq!(crow_req.method, "GET");
        assert_eq!(crow_req.path, "/health");
        assert!(crow_req.query.is_empty());
        assert!(crow_req.body.is_empty());
    }

    #[test]
    fn test_response_to_axum_json() {
        let crow_resp = CrowResponse::json(serde_json::json!({"ok": true}));
        let axum_resp = response_to_axum(crow_resp);
        assert_eq!(axum_resp.status(), StatusCode::OK);
        assert_eq!(
            axum_resp.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_response_to_axum_redirect() {
        let crow_resp = CrowResponse::redirect("https://example.com");
        let axum_resp = response_to_axum(crow_resp);
        assert_eq!(axum_resp.status(), StatusCode::FOUND);
        assert_eq!(
            axum_resp.headers().get("location").unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn test_response_to_axum_custom_status() {
        let crow_resp = CrowResponse::respond(404, "not found");
        let axum_resp = response_to_axum(crow_resp);
        assert_eq!(axum_resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_response_to_axum_default_content_type() {
        let crow_resp = CrowResponse::new(200, "hello");
        let axum_resp = response_to_axum(crow_resp);
        assert_eq!(
            axum_resp.headers().get("content-type").unwrap(),
            "text/plain"
        );
    }

    /// A response HTTP cannot carry answers 500 with why: a header whose
    /// value holds a line break (one a handler built from a request would
    /// otherwise split the response), a status outside HTTP's. Text beyond
    /// ASCII in a header's value is sent as its bytes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_response_http_cannot_carry_says_why() {
        let mut split = CrowResponse::new(200, "hello");
        split.headers.insert("x-name".to_string(), "a\r\nset-cookie: s=1".to_string());
        let resp = response_to_axum(split);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(resp.headers().get("set-cookie").is_none());
        let body = String::from_utf8_lossy(&axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()).to_string();
        // the client is told an id; the reason is the log's
        assert!(body.starts_with("the request could not be answered — the server's log has why, as "), "{}", body);
        assert!(!body.contains("x-name") && !body.contains("set-cookie"), "{}", body);
        assert_eq!(unsendable(&{ let mut r = CrowResponse::new(200, "x"); r.headers.insert("x-name".to_string(), "a\nb".to_string()); r }).unwrap(), "the header 'x-name' holds a line break or another control character");
        assert_eq!(unsendable(&CrowResponse::new(42, "odd")).unwrap(), "42 is not an HTTP status (100 to 999)");
        let resp = response_to_axum(CrowResponse::new(42, "odd"));
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let mut accented = CrowResponse::new(200, "ok");
        accented.headers.insert("x-city".to_string(), "Zürich".to_string());
        let resp = response_to_axum(accented);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-city").unwrap().as_bytes(), "Zürich".as_bytes());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_response_to_axum_streams_the_feed() {
        let crow_resp = CrowResponse::stream("chunks", |put| {
            for i in 0..3 {
                put(format!("chunk {}\n", i));
            }
            Ok(())
        });
        let axum_resp = response_to_axum(crow_resp);
        assert_eq!(axum_resp.headers().get("content-type").unwrap(), "text/plain");
        let body = axum::body::to_bytes(axum_resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], b"chunk 0\nchunk 1\nchunk 2\n");
    }
}

