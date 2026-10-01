//! A route's request body, read within the app's limit, and its form: an
//! urlencoded body's fields, or a multipart body's fields and files — each
//! file copied under `.crow/uploads/` for the handler to read, and removed
//! once the handler has answered (spec, Core (Server)). The limit is the one
//! a page's uploads keep: 10 MB, or `serve[…, uploads = N]` megabytes.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// The most a request's body may hold, in bytes.
static LIMIT: AtomicU64 = AtomicU64::new(10 * 1024 * 1024);

/// `serve[…, uploads = N]`: N megabytes at most in a request's body.
pub fn configure_uploads(megabytes: i64) {
    if megabytes > 0 {
        LIMIT.store(megabytes as u64 * 1024 * 1024, Ordering::Relaxed);
    }
}

/// The most a request's body may hold, in bytes.
pub fn limit() -> usize {
    LIMIT.load(Ordering::Relaxed) as usize
}

/// Where a request's files are kept while its handler runs: the directory a
/// page's uploads use, which ignores itself in version control.
fn dir() -> Result<std::path::PathBuf, String> {
    let dir = std::path::PathBuf::from(".crow/uploads");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot keep an upload in {}: {}", dir.display(), e))?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(&ignore, "*\n");
    }
    Ok(dir)
}

/// A file's copy, under a name of its own: the original's extension kept,
/// nothing of its name that could reach another directory.
fn keep(name: &str, data: &[u8]) -> Result<String, String> {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let ext: String = std::path::Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect())
        .unwrap_or_default();
    let file = format!("r{}-{:x}-{}{}{}", std::process::id(), nanos, N.fetch_add(1, Ordering::Relaxed), if ext.is_empty() { "" } else { "." }, ext);
    let path = dir()?.join(file);
    std::fs::write(&path, data).map_err(|e| format!("cannot keep an upload: {}", e))?;
    Ok(path.display().to_string())
}

/// A form's fields and files, by its body's media type: an urlencoded
/// body's fields; a multipart body's text fields and its files, each a map
/// of its `field`, `name`, `type`, `size` and `path`. Another body has none.
pub async fn form_of(content_type: &str, body: &bytes::Bytes) -> Result<(BTreeMap<String, String>, Vec<BTreeMap<String, String>>), String> {
    let media = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if media == "application/x-www-form-urlencoded" {
        return Ok((crate::core::CrowRequest::parse_query_string(&String::from_utf8_lossy(body)), Vec::new()));
    }
    if media != "multipart/form-data" {
        return Ok((BTreeMap::new(), Vec::new()));
    }
    let boundary = multer::parse_boundary(content_type).map_err(|e| format!("a multipart body: {}", e))?;
    let whole = body.clone();
    let stream = futures_util::stream::once(async move { Ok::<bytes::Bytes, std::io::Error>(whole) });
    let mut parts = multer::Multipart::new(stream, boundary);
    let (mut form, mut files) = (BTreeMap::new(), Vec::new());
    while let Some(field) = parts.next_field().await.map_err(|e| format!("a multipart body: {}", e))? {
        let name = field.name().unwrap_or("").to_string();
        match field.file_name().map(str::to_string) {
            Some(file_name) => {
                let media = field.content_type().map(|m| m.to_string()).unwrap_or_else(|| "application/octet-stream".to_string());
                let data = field.bytes().await.map_err(|e| format!("a multipart body: {}", e))?;
                let path = match keep(&file_name, &data) {
                    Ok(p) => p,
                    Err(e) => {
                        release(&files);
                        return Err(e);
                    }
                };
                files.push(BTreeMap::from([
                    ("field".to_string(), name),
                    ("name".to_string(), file_name),
                    ("type".to_string(), media),
                    ("size".to_string(), data.len().to_string()),
                    ("path".to_string(), path),
                ]));
            }
            None => {
                let text = field.text().await.map_err(|e| format!("a multipart body: {}", e))?;
                form.insert(name, text);
            }
        }
    }
    Ok((form, files))
}

/// A request's files, removed once its handler has answered (a handler that
/// keeps one copies it: `cp[f.path, …]`).
pub fn release(files: &[BTreeMap<String, String>]) {
    for f in files {
        if let Some(p) = f.get("path") {
            let _ = std::fs::remove_file(p);
        }
    }
}
