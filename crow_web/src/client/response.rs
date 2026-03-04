//! Client response type — what `get[]`, `post[]`, etc. return.

use std::collections::HashMap;

/// HTTP response received from an external server.
///
/// Crow code accesses fields naturally:
/// ```text
/// res = get['https://api.example.com/users']
/// log[res.status]       --- 200 ---
/// log[res.ok]           --- true ---
/// users = res.json      --- parsed JSON ---
/// log[res.body]         --- raw string ---
/// log[res.headers]      --- map ---
/// ```
#[derive(Debug, Clone)]
pub struct CrowClientResponse {
    /// HTTP status code (e.g. 200, 404, 500)
    pub status: u16,
    /// Response headers (lowercase keys)
    pub headers: HashMap<String, String>,
    /// Raw response body as string
    pub body: String,
}

impl CrowClientResponse {
    /// Parse the body as JSON. Returns None if body is not valid JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.body).ok()
    }

    /// Check if response status is 2xx.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Get a header value by name (case-insensitive lookup).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(|s| s.as_str())
    }
}

/// Convert a `reqwest::blocking::Response` into a `CrowClientResponse`.
pub(crate) fn from_reqwest(resp: reqwest::blocking::Response) -> CrowClientResponse {
    let status = resp.status().as_u16();
    let mut headers = HashMap::new();
    for (key, value) in resp.headers().iter() {
        if let Ok(v) = value.to_str() {
            headers.insert(key.as_str().to_lowercase(), v.to_string());
        }
    }
    let body = resp.text().unwrap_or_default();
    CrowClientResponse {
        status,
        headers,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_response_json() {
        let resp = CrowClientResponse {
            status: 200,
            headers: HashMap::new(),
            body: r#"{"name": "alice"}"#.to_string(),
        };
        let json = resp.json().unwrap();
        assert_eq!(json["name"], "alice");
    }

    #[test]
    fn test_client_response_invalid_json() {
        let resp = CrowClientResponse {
            status: 200,
            headers: HashMap::new(),
            body: "not json".to_string(),
        };
        assert!(resp.json().is_none());
    }

    #[test]
    fn test_client_response_ok() {
        let ok = CrowClientResponse {
            status: 200,
            headers: HashMap::new(),
            body: String::new(),
        };
        assert!(ok.ok());

        let created = CrowClientResponse {
            status: 201,
            headers: HashMap::new(),
            body: String::new(),
        };
        assert!(created.ok());

        let not_found = CrowClientResponse {
            status: 404,
            headers: HashMap::new(),
            body: String::new(),
        };
        assert!(!not_found.ok());
    }

    #[test]
    fn test_client_response_header() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let resp = CrowClientResponse {
            status: 200,
            headers,
            body: String::new(),
        };
        assert_eq!(resp.header("Content-Type"), Some("application/json"));
        assert_eq!(resp.header("content-type"), Some("application/json"));
        assert_eq!(resp.header("x-missing"), None);
    }
}

