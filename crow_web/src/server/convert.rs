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
    let mut builder = Builder::new().status(
        StatusCode::from_u16(crow_resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    );

    for (k, v) in &crow_resp.headers {
        builder = builder.header(k.as_str(), v.as_str());
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

