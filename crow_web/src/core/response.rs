use std::collections::HashMap;

/// HTTP response returned by Crow route handlers.
///
/// Handlers return responses via helper functions:
/// ```text
/// route[app, '/hello', fn[req]
///   json[{message, 'hello'}]
/// ]
/// ```
///
/// Supports both text and binary bodies. Binary body takes priority
/// when converting to an axum response (used by `msgpack[]`, `arrow[]`).
#[derive(Debug, Clone)]
pub struct CrowResponse {
    /// HTTP status code (e.g. 200, 404, 500)
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Text response body (used by json, html, text, respond)
    pub body: String,
    /// Binary response body (used by msgpack, arrow, bytes)
    /// When Some, takes priority over `body` in response_to_axum.
    pub binary_body: Option<Vec<u8>>,
}

impl CrowResponse {
    /// Create a response with the given status and text body.
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: body.into(),
            binary_body: None,
        }
    }

    /// Create a response with the given status and binary body.
    pub fn binary(status: u16, bytes: Vec<u8>, content_type: impl Into<String>) -> Self {
        let mut resp = Self::new(status, "");
        resp.binary_body = Some(bytes);
        resp.headers.insert("content-type".to_string(), content_type.into());
        resp
    }

    /// JSON response (status 200, content-type: application/json).
    /// Serializes the value to JSON.
    pub fn json(value: serde_json::Value) -> Self {
        let body = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        let mut resp = Self::new(200, body);
        resp.headers.insert(
            "content-type".to_string(),
            "application/json".to_string(),
        );
        resp
    }

    /// MessagePack binary response (status 200, content-type: application/msgpack).
    /// Serializes the value to MessagePack format using rmp-serde.
    pub fn msgpack(bytes: Vec<u8>) -> Self {
        Self::binary(200, bytes, "application/msgpack")
    }

    /// Arrow IPC binary response (status 200, content-type: application/vnd.apache.arrow.stream).
    /// Takes pre-serialized Arrow IPC bytes.
    pub fn arrow_ipc(bytes: Vec<u8>) -> Self {
        Self::binary(200, bytes, "application/vnd.apache.arrow.stream")
    }

    /// HTML response (status 200, content-type: text/html).
    pub fn html(body: impl Into<String>) -> Self {
        let mut resp = Self::new(200, body);
        resp.headers
            .insert("content-type".to_string(), "text/html".to_string());
        resp
    }

    /// Plain text response (status 200, content-type: text/plain).
    pub fn text(body: impl Into<String>) -> Self {
        let mut resp = Self::new(200, body);
        resp.headers
            .insert("content-type".to_string(), "text/plain".to_string());
        resp
    }

    /// Response with a specific status code and body.
    pub fn respond(status: u16, body: impl Into<String>) -> Self {
        Self::new(status, body)
    }

    /// Redirect response (302 Found).
    pub fn redirect(url: impl Into<String>) -> Self {
        let mut resp = Self::new(302, "");
        resp.headers
            .insert("location".to_string(), url.into());
        resp
    }

    /// Set a header on the response. Returns self for chaining.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Check if response status is 2xx.
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl Default for CrowResponse {
    fn default() -> Self {
        Self::new(200, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_json() {
        let resp = CrowResponse::json(serde_json::json!({"name": "alice"}));
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/json");
        assert!(resp.body.contains("alice"));
    }

    #[test]
    fn test_response_html() {
        let resp = CrowResponse::html("<h1>Hello</h1>");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "text/html");
        assert_eq!(resp.body, "<h1>Hello</h1>");
    }

    #[test]
    fn test_response_text() {
        let resp = CrowResponse::text("hello");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "text/plain");
        assert_eq!(resp.body, "hello");
    }

    #[test]
    fn test_response_redirect() {
        let resp = CrowResponse::redirect("https://example.com");
        assert_eq!(resp.status, 302);
        assert_eq!(resp.headers.get("location").unwrap(), "https://example.com");
    }

    #[test]
    fn test_response_respond() {
        let resp = CrowResponse::respond(404, "not found");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, "not found");
    }

    #[test]
    fn test_response_is_ok() {
        assert!(CrowResponse::new(200, "").is_ok());
        assert!(CrowResponse::new(201, "").is_ok());
        assert!(!CrowResponse::new(404, "").is_ok());
        assert!(!CrowResponse::new(500, "").is_ok());
    }

    #[test]
    fn test_response_with_header() {
        let resp = CrowResponse::text("ok").with_header("x-custom", "value");
        assert_eq!(resp.headers.get("x-custom").unwrap(), "value");
    }

    #[test]
    fn test_response_binary() {
        let resp = CrowResponse::binary(200, vec![1, 2, 3], "application/octet-stream");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.binary_body.as_ref().unwrap(), &[1, 2, 3]);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/octet-stream");
    }

    #[test]
    fn test_response_msgpack() {
        let resp = CrowResponse::msgpack(vec![0x93, 0x01, 0x02, 0x03]);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/msgpack");
        assert!(resp.binary_body.is_some());
    }

    #[test]
    fn test_response_arrow_ipc() {
        let resp = CrowResponse::arrow_ipc(vec![0x41, 0x52, 0x52, 0x4F, 0x57]);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/vnd.apache.arrow.stream");
        assert!(resp.binary_body.is_some());
    }
}

