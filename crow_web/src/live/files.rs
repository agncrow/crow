//! Files in and out of a page (spec: *Files in and out*). A download a
//! handler hands the reader waits under a one-time ticket the page fetches
//! it by; the socket carries events, never files.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a download waits for its page to fetch it.
const WAIT: Duration = Duration::from_secs(120);

/// A file handed to the reader: its name and its bytes.
pub struct Download {
    pub name: String,
    pub bytes: Vec<u8>,
}

fn waiting() -> &'static Mutex<HashMap<String, (Instant, Download)>> {
    static W: OnceLock<Mutex<HashMap<String, (Instant, Download)>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Put a download aside for its page: the ticket to fetch it by, once.
pub fn offer(name: &str, bytes: Vec<u8>) -> Result<String, String> {
    let t = crow_runtime::signin::random_hex(16)?;
    let mut all = waiting().lock().unwrap_or_else(|p| p.into_inner());
    all.retain(|_, (at, _)| at.elapsed() < WAIT);
    all.insert(t.clone(), (Instant::now(), Download { name: name.to_string(), bytes }));
    Ok(t)
}

/// The download a ticket stands for, once.
pub fn take(t: &str) -> Option<Download> {
    let mut all = waiting().lock().unwrap_or_else(|p| p.into_inner());
    let (at, d) = all.remove(t)?;
    (at.elapsed() < WAIT).then_some(d)
}

/// The media type a file's name says it is.
pub fn media_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).as_deref() {
        Some("csv") => "text/csv; charset=utf-8",
        Some("txt") | Some("log") | Some("md") => "text/plain; charset=utf-8",
        Some("json") => "application/json",
        Some("html") | Some("htm") => "application/octet-stream",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// `Content-Disposition` for a file saved under `name`: an ASCII name for
/// every browser, and the name as written for those that read it.
pub fn attachment(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' && c != '/' || c == ' ' { c } else { '_' })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect();
    format!("attachment; filename=\"{}\"; filename*=UTF-8''{}", plain.trim(), encoded)
}

/// How big a file a page may send, in bytes: `serve[…, uploads = N]`
/// megabytes, 10 unless the program says.
static UPLOAD_LIMIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(10 * 1024 * 1024);

/// `serve[…, uploads = N]`: files up to N megabytes (a count under one keeps 10).
pub fn configure_uploads(mb: i64) {
    let mb = if mb > 0 { mb as u64 } else { 10 };
    UPLOAD_LIMIT.store(mb * 1024 * 1024, std::sync::atomic::Ordering::SeqCst);
}

/// The biggest file a page may send, in bytes.
pub fn upload_limit() -> u64 {
    UPLOAD_LIMIT.load(std::sync::atomic::Ordering::SeqCst)
}

/// A size as a reader reads it.
pub fn describe_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", (bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
}

/// A file a page is about to send for a field: the session it is for, the
/// fact it fills, what the page said it is, and the way back to the
/// session's loop.
pub struct Upload {
    pub session: u64,
    pub key: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub tx: tokio::sync::mpsc::UnboundedSender<super::LaterDone>,
}

fn expected() -> &'static Mutex<HashMap<String, (Instant, Upload)>> {
    static U: OnceLock<Mutex<HashMap<String, (Instant, Upload)>>> = OnceLock::new();
    U.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A ticket for one upload, good for a couple of minutes.
pub fn expect_upload(u: Upload) -> Result<String, String> {
    let t = crow_runtime::signin::random_hex(16)?;
    let mut all = expected().lock().unwrap_or_else(|p| p.into_inner());
    all.retain(|_, (at, _)| at.elapsed() < WAIT);
    all.insert(t.clone(), (Instant::now(), u));
    Ok(t)
}

/// The upload a ticket stands for, once.
pub fn claim_upload(t: &str) -> Option<Upload> {
    let mut all = expected().lock().unwrap_or_else(|p| p.into_inner());
    let (at, u) = all.remove(t)?;
    (at.elapsed() < WAIT).then_some(u)
}

/// Where pages' files are kept: one directory per file, which ignores
/// itself in version control.
const UPLOADS_DIR: &str = ".crow/uploads";

fn kept() -> &'static Mutex<HashMap<u64, Vec<std::path::PathBuf>>> {
    static K: OnceLock<Mutex<HashMap<u64, Vec<std::path::PathBuf>>>> = OnceLock::new();
    K.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Keep a page's file for its session: where it is.
pub fn keep_upload(session: u64, name: &str, bytes: &[u8]) -> Result<String, String> {
    let root = std::path::Path::new(UPLOADS_DIR);
    std::fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {}", UPLOADS_DIR, e))?;
    let _ = std::fs::write(root.join(".gitignore"), "*\n");
    let dir = root.join(crow_runtime::signin::random_hex(12)?);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    // the name as the reader's computer called it, made safe for this one
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || " ._-()".contains(c) { c } else { '_' })
        .collect();
    let safe = safe.trim().trim_start_matches('.').to_string();
    let path = dir.join(if safe.is_empty() { "file".to_string() } else { safe });
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    kept().lock().unwrap_or_else(|p| p.into_inner()).entry(session).or_default().push(dir);
    Ok(path.to_string_lossy().to_string())
}

/// A session ended: the files its pages sent go with it.
pub fn forget_uploads(session: u64) {
    let dirs = kept().lock().unwrap_or_else(|p| p.into_inner()).remove(&session).unwrap_or_default();
    for dir in dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_s_files_go_with_it() {
        // a name that climbs is only a name: the file is in its own directory
        let path = keep_upload(987_654, "../../etc/pass wd.csv", b"x").unwrap();
        let p = std::path::Path::new(&path);
        assert_eq!(p.parent().and_then(|d| d.parent()), Some(std::path::Path::new(UPLOADS_DIR)), "{}", path);
        assert!(p.file_name().unwrap().to_string_lossy().ends_with("pass wd.csv"), "{}", path);
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
        forget_uploads(987_654);
        assert!(!std::path::Path::new(&path).exists());
        assert_eq!(describe_size(10 * 1024 * 1024), "10 MB");
    }

    #[test]
    fn a_download_is_fetched_once() {
        let t = offer("trades.csv", b"a,b\n1,2\n".to_vec()).unwrap();
        let d = take(&t).expect("the page fetches it");
        assert_eq!(d.name, "trades.csv");
        assert_eq!(d.bytes, b"a,b\n1,2\n");
        assert!(take(&t).is_none(), "once");
    }

    /// A name cannot break out of its header, and a page's own kinds of
    /// file are never shown in its place
    #[test]
    fn the_name_and_kind_are_safe() {
        let a = attachment("q3 \"report\"\r\nX: y.csv");
        assert!(!a.contains('\r') && !a.contains('\n'), "{}", a);
        assert!(a.starts_with("attachment; filename=\"q3 _report___X: y.csv\""), "{}", a);
        assert_eq!(media_type("t.CSV"), "text/csv; charset=utf-8");
        assert_eq!(media_type("page.html"), "application/octet-stream");
    }
}
