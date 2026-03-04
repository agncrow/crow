use std::collections::HashMap;

/// HTTP response returned by Crow route handlers.
///
/// Handlers return responses via helper functions:
/// ```text
/// route[app, '/hello', fn[req]
///   json[{message, 'hello'}]
/// ]
/// ```
#[derive(Debug, Clone)]
pub struct CrowResponse {
    /// HTTP status code (e.g. 200, 404, 500)
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: String,
}

impl CrowResponse {
    /// Create a response with the given status and body.
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: body.into(),
        }
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
}

