//! `web.client` module — HTTP client powered by reqwest.
//!
//! Enabled by the `client` feature flag.
//! Uses `reqwest::blocking` for synchronous requests.
//!
//! ```text
//! use web.client
//!
//! res = get['https://api.example.com/users']
//! log[res.status]
//! users = res.json
//! ```

mod response;

pub use response::CrowClientResponse;
use response::from_reqwest;
use std::collections::HashMap;

/// Shared blocking client (reuses connections).
fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::new()
}

/// GET request.
///
/// ```text
/// res = get['https://api.example.com/users']
/// ```
pub fn get(url: &str, headers: Option<&HashMap<String, String>>) -> CrowClientResponse {
    let mut req = client().get(url);
    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            req = req.header(k, v);
        }
    }
    match req.send() {
        Ok(resp) => from_reqwest(resp),
        Err(e) => error_response(e),
    }
}

/// POST request with JSON body.
///
/// ```text
/// res = post['https://api.example.com/users', {name, 'alice'}]
/// ```
pub fn post(
    url: &str,
    body: serde_json::Value,
    headers: Option<&HashMap<String, String>>,
) -> CrowClientResponse {
    let mut req = client().post(url).json(&body);
    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            req = req.header(k, v);
        }
    }
    match req.send() {
        Ok(resp) => from_reqwest(resp),
        Err(e) => error_response(e),
    }
}

/// PUT request with JSON body.
///
/// ```text
/// res = put['https://api.example.com/users/1', {name, 'bob'}]
/// ```
pub fn put(
    url: &str,
    body: serde_json::Value,
    headers: Option<&HashMap<String, String>>,
) -> CrowClientResponse {
    let mut req = client().put(url).json(&body);
    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            req = req.header(k, v);
        }
    }
    match req.send() {
        Ok(resp) => from_reqwest(resp),
        Err(e) => error_response(e),
    }
}

/// DELETE request.
///
/// ```text
/// res = del['https://api.example.com/users/1']
/// ```
pub fn del(url: &str, headers: Option<&HashMap<String, String>>) -> CrowClientResponse {
    let mut req = client().delete(url);
    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            req = req.header(k, v);
        }
    }
    match req.send() {
        Ok(resp) => from_reqwest(resp),
        Err(e) => error_response(e),
    }
}

/// Convert a reqwest error into an error response (status 0, error message in body).
fn error_response(e: reqwest::Error) -> CrowClientResponse {
    CrowClientResponse {
        status: 0,
        headers: HashMap::new(),
        body: format!("request error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_invalid_url() {
        let resp = get("http://localhost:1", None);
        assert_eq!(resp.status, 0);
        assert!(resp.body.contains("request error"));
        assert!(!resp.ok());
    }

    #[test]
    fn test_post_invalid_url() {
        let resp = post(
            "http://localhost:1",
            serde_json::json!({"key": "value"}),
            None,
        );
        assert_eq!(resp.status, 0);
        assert!(resp.body.contains("request error"));
    }

    #[test]
    fn test_put_invalid_url() {
        let resp = put(
            "http://localhost:1",
            serde_json::json!({"key": "value"}),
            None,
        );
        assert_eq!(resp.status, 0);
    }

    #[test]
    fn test_del_invalid_url() {
        let resp = del("http://localhost:1", None);
        assert_eq!(resp.status, 0);
    }

    #[test]
    fn test_error_response_not_ok() {
        let resp = error_response(
            reqwest::blocking::Client::new()
                .get("http://localhost:1")
                .send()
                .unwrap_err(),
        );
        assert_eq!(resp.status, 0);
        assert!(!resp.ok());
    }
}

