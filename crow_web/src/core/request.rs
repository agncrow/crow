use std::collections::HashMap;

/// HTTP request as seen by Crow route handlers.
///
/// Crow code accesses fields naturally:
/// ```text
/// fn handle[req]
///   log[req.method]       --- 'GET' ---
///   log[req.path]         --- '/users/42' ---
///   log[req.params.id]    --- '42' ---
///   log[req.query.page]   --- '1' ---
/// ```
#[derive(Debug, Clone)]
pub struct CrowRequest {
    /// HTTP method: "GET", "POST", "PUT", "DELETE", etc.
    pub method: String,
    /// Request path, e.g. "/users/42"
    pub path: String,
    /// Request headers (lowercase keys)
    pub headers: HashMap<String, String>,
    /// Query string parameters, e.g. ?page=1&limit=10
    pub query: HashMap<String, String>,
    /// Raw request body as string
    pub body: String,
    /// Route parameters extracted from path patterns, e.g. /users/:id → {"id": "42"}
    pub params: HashMap<String, String>,
}

impl CrowRequest {
    /// Create an empty request (used for testing and builder pattern).
    pub fn new() -> Self {
        Self {
            method: String::new(),
            path: String::new(),
            headers: HashMap::new(),
            query: HashMap::new(),
            body: String::new(),
            params: HashMap::new(),
        }
    }

    /// Parse the body as JSON. Returns None if body is not valid JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.body).ok()
    }

    /// Get a header value by name (case-insensitive lookup).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(|s| s.as_str())
    }

    /// Get a query parameter value.
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(|s| s.as_str())
    }

    /// Get a route parameter value.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(|s| s.as_str())
    }

    /// Parse query string from a raw query string like "page=1&limit=10".
    pub fn parse_query_string(qs: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if qs.is_empty() {
            return map;
        }
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            } else if !pair.is_empty() {
                map.insert(pair.to_string(), String::new());
            }
        }
        map
    }
}

impl Default for CrowRequest {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_new() {
        let req = CrowRequest::new();
        assert!(req.method.is_empty());
        assert!(req.path.is_empty());
        assert!(req.headers.is_empty());
        assert!(req.query.is_empty());
        assert!(req.body.is_empty());
        assert!(req.params.is_empty());
    }

    #[test]
    fn test_request_json_body() {
        let mut req = CrowRequest::new();
        req.body = r#"{"name": "alice", "age": 30}"#.to_string();
        let json = req.json().unwrap();
        assert_eq!(json["name"], "alice");
        assert_eq!(json["age"], 30);
    }

    #[test]
    fn test_request_invalid_json() {
        let mut req = CrowRequest::new();
        req.body = "not json".to_string();
        assert!(req.json().is_none());
    }

    #[test]
    fn test_request_header_case_insensitive() {
        let mut req = CrowRequest::new();
        req.headers.insert("content-type".to_string(), "application/json".to_string());
        assert_eq!(req.header("Content-Type"), Some("application/json"));
        assert_eq!(req.header("content-type"), Some("application/json"));
    }

    #[test]
    fn test_request_params() {
        let mut req = CrowRequest::new();
        req.params.insert("id".to_string(), "42".to_string());
        assert_eq!(req.param("id"), Some("42"));
        assert_eq!(req.param("missing"), None);
    }

    #[test]
    fn test_parse_query_string() {
        let qs = CrowRequest::parse_query_string("page=1&limit=10&active");
        assert_eq!(qs.get("page").unwrap(), "1");
        assert_eq!(qs.get("limit").unwrap(), "10");
        assert_eq!(qs.get("active").unwrap(), "");
    }

    #[test]
    fn test_parse_query_string_empty() {
        let qs = CrowRequest::parse_query_string("");
        assert!(qs.is_empty());
    }
}

