use std::collections::BTreeMap;

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
    pub headers: BTreeMap<String, String>,
    /// Query string parameters, e.g. ?page=1&limit=10
    pub query: BTreeMap<String, String>,
    /// Raw request body as string
    pub body: String,
    /// Route parameters extracted from path patterns, e.g. /users/:id → {"id": "42"}
    pub params: BTreeMap<String, String>,
    /// A form's text fields: an urlencoded body's, or a multipart body's
    /// fields that are not files
    pub form: BTreeMap<String, String>,
    /// A multipart body's files, each as a page's picked file is: its
    /// `name`, `type`, `size` (as text) and the `path` of the copy the server
    /// keeps while the handler runs
    pub files: Vec<BTreeMap<String, String>>,
}

impl CrowRequest {
    /// Create an empty request (used for testing and builder pattern).
    pub fn new() -> Self {
        Self {
            method: String::new(),
            path: String::new(),
            headers: BTreeMap::new(),
            query: BTreeMap::new(),
            body: String::new(),
            params: BTreeMap::new(),
            form: BTreeMap::new(),
            files: Vec::new(),
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
    pub fn parse_query_string(qs: &str) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for pair in qs.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            map.insert(decode_component(k), decode_component(v));
        }
        map
    }
}

/// A query string's or a form's name or value as it was typed: `+` is a
/// space and `%XX` the byte it names (`q=hello%20world` is `hello world`);
/// a `%` that names no byte stays as it is. (The text had reached the
/// program still encoded.)
pub fn decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                match (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
                    (Some(h), Some(l)) => {
                        out.push(h * 16 + l);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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

    /// A query or a form reaches the program as it was typed: decoded
    #[test]
    fn a_query_is_decoded() {
        let q = CrowRequest::parse_query_string("q=hello%20world&name=J%C3%B6rg&sum=1+2&pct=100%&bad=%zz&flag");
        assert_eq!(q["q"], "hello world");
        assert_eq!(q["name"], "Jörg");
        assert_eq!(q["sum"], "1 2");
        assert_eq!(q["pct"], "100%");
        assert_eq!(q["bad"], "%zz");
        assert_eq!(q["flag"], "");
    }
}
