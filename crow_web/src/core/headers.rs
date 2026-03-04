//! Common HTTP header names and status codes.
//!
//! All header names are lowercase strings, matching the convention
//! used by CrowRequest and CrowResponse.

// ---- Common header names ----

pub const CONTENT_TYPE: &str = "content-type";
pub const CONTENT_LENGTH: &str = "content-length";
pub const AUTHORIZATION: &str = "authorization";
pub const ACCEPT: &str = "accept";
pub const LOCATION: &str = "location";
pub const HOST: &str = "host";
pub const USER_AGENT: &str = "user-agent";
pub const COOKIE: &str = "cookie";
pub const SET_COOKIE: &str = "set-cookie";
pub const CACHE_CONTROL: &str = "cache-control";

// ---- Common content type values ----

pub const APPLICATION_JSON: &str = "application/json";
pub const TEXT_HTML: &str = "text/html";
pub const TEXT_PLAIN: &str = "text/plain";
pub const APPLICATION_FORM: &str = "application/x-www-form-urlencoded";
pub const MULTIPART_FORM: &str = "multipart/form-data";

// ---- Common status codes ----

pub const STATUS_OK: u16 = 200;
pub const STATUS_CREATED: u16 = 201;
pub const STATUS_NO_CONTENT: u16 = 204;
pub const STATUS_MOVED: u16 = 301;
pub const STATUS_FOUND: u16 = 302;
pub const STATUS_NOT_MODIFIED: u16 = 304;
pub const STATUS_BAD_REQUEST: u16 = 400;
pub const STATUS_UNAUTHORIZED: u16 = 401;
pub const STATUS_FORBIDDEN: u16 = 403;
pub const STATUS_NOT_FOUND: u16 = 404;
pub const STATUS_METHOD_NOT_ALLOWED: u16 = 405;
pub const STATUS_CONFLICT: u16 = 409;
pub const STATUS_UNPROCESSABLE: u16 = 422;
pub const STATUS_TOO_MANY_REQUESTS: u16 = 429;
pub const STATUS_INTERNAL_ERROR: u16 = 500;
pub const STATUS_BAD_GATEWAY: u16 = 502;
pub const STATUS_SERVICE_UNAVAILABLE: u16 = 503;

/// Return a human-readable reason phrase for a status code.
pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reason_phrases() {
        assert_eq!(reason_phrase(200), "OK");
        assert_eq!(reason_phrase(404), "Not Found");
        assert_eq!(reason_phrase(500), "Internal Server Error");
        assert_eq!(reason_phrase(999), "Unknown");
    }

    #[test]
    fn test_header_constants() {
        assert_eq!(CONTENT_TYPE, "content-type");
        assert_eq!(APPLICATION_JSON, "application/json");
        assert_eq!(STATUS_OK, 200);
    }
}

