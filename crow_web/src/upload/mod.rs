//! `web.upload` module — Multipart file upload handling.
//!
//! ```text
//! use web
//! use web.upload
//!
//! app = web.new[]
//! route[app, '/upload', 'POST', fn[req]
//!   files = uploads[req]
//!   log[files.0.name]
//! ]
//! ```

use std::collections::HashMap;

/// A single uploaded file.
#[derive(Debug, Clone)]
pub struct CrowUploadedFile {
    /// Original filename from the client (may be empty).
    pub name: String,
    /// MIME content type (e.g. "image/png").
    pub content_type: String,
    /// Raw file bytes.
    pub data: Vec<u8>,
    /// Field name from the form.
    pub field_name: String,
}

impl CrowUploadedFile {
    /// File size in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Save the file to disk at the given path.
    pub fn save(&self, path: &str) -> bool {
        std::fs::write(path, &self.data).is_ok()
    }

    /// Get the file data as a UTF-8 string (if valid).
    pub fn text(&self) -> Option<String> {
        String::from_utf8(self.data.clone()).ok()
    }
}

/// Result of parsing a multipart upload.
#[derive(Debug, Clone)]
pub struct CrowUploadResult {
    /// Uploaded files.
    pub files: Vec<CrowUploadedFile>,
    /// Text fields from the form.
    pub fields: HashMap<String, String>,
}

impl CrowUploadResult {
    /// Number of uploaded files.
    pub fn count(&self) -> usize {
        self.files.len()
    }

    /// Get a file by field name.
    pub fn file(&self, field_name: &str) -> Option<&CrowUploadedFile> {
        self.files.iter().find(|f| f.field_name == field_name)
    }

    /// Get a text field value.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(|s| s.as_str())
    }
}

/// Parse multipart form data from raw body bytes and content-type header.
///
/// This is a synchronous parser that works with the body already extracted.
/// The axum integration extracts the Multipart in the handler adapter.
pub fn parse_multipart_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .find_map(|part| {
            let part = part.trim();
            if part.starts_with("boundary=") {
                Some(part["boundary=".len()..].trim_matches('"').to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uploaded_file_size() {
        let file = CrowUploadedFile {
            name: "test.txt".to_string(),
            content_type: "text/plain".to_string(),
            data: b"hello world".to_vec(),
            field_name: "file".to_string(),
        };
        assert_eq!(file.size(), 11);
    }

    #[test]
    fn test_uploaded_file_text() {
        let file = CrowUploadedFile {
            name: "test.txt".to_string(),
            content_type: "text/plain".to_string(),
            data: b"hello".to_vec(),
            field_name: "file".to_string(),
        };
        assert_eq!(file.text(), Some("hello".to_string()));
    }

    #[test]
    fn test_upload_result_file_lookup() {
        let result = CrowUploadResult {
            files: vec![CrowUploadedFile {
                name: "photo.jpg".to_string(),
                content_type: "image/jpeg".to_string(),
                data: vec![0xFF, 0xD8],
                field_name: "avatar".to_string(),
            }],
            fields: HashMap::from([("name".to_string(), "alice".to_string())]),
        };
        assert_eq!(result.count(), 1);
        assert_eq!(result.file("avatar").unwrap().name, "photo.jpg");
        assert_eq!(result.field("name"), Some("alice"));
        assert!(result.file("missing").is_none());
    }

    #[test]
    fn test_parse_boundary() {
        let ct = "multipart/form-data; boundary=----WebKitFormBoundary";
        assert_eq!(
            parse_multipart_boundary(ct),
            Some("----WebKitFormBoundary".to_string())
        );
    }

    #[test]
    fn test_parse_boundary_none() {
        assert_eq!(parse_multipart_boundary("application/json"), None);
    }
}

