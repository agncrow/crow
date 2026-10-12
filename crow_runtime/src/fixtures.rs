//! What a recording kept and a replay answers, shared by a statement's
//! fixture (`crow_db::fixtures`) and a request's (`sys.replayed`, which
//! `request`'s own Crow body asks before it sends — src/modules/core.co, so
//! both modes agree by construction): the hash that names a fixture's file,
//! the replay's switch (`CROW_REPLAY`, set by `crow test --fixtures` for
//! itself and the compiled apps its tests open), the misses a replay notes,
//! the recording's switch (set by the rehearsal `crow check
//! --record-fixtures` runs a test in), and a request's fixture itself —
//! `.crow/fixtures/<hash>.json`, `{"kind": "request", "method", "url",
//! "body", "reply": {"status", "headers", "body"}}`, named by the request's
//! method, its URL with the query, and what it sends; its headers are no
//! part of it, so a key sent in one lands in no fixture (spec, Requests;
//! crow-check.md, recordings).

use crate::sources::Reply;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The FNV-1a hash of the parts: a stable file name.
pub fn fnv(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h ^= b'\n' as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for b in p.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{:016x}", h)
}

/// The project whose recordings answer a replay: `CROW_REPLAY` names its
/// `.crow` directory (`crow test --fixtures` sets it, for itself and for the
/// compiled apps its tests open).
pub fn replaying() -> Option<PathBuf> {
    std::env::var_os("CROW_REPLAY").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// Where a recording keeps what it is answered: the project's `.crow`
/// directory while `crow check --record-fixtures` runs a test, none
/// otherwise. A recording runs interpreted, in the process that set it.
static RECORDING: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Start (`Some(.crow)`) or end (`None`) a recording.
pub fn record_to(crow_dir: Option<PathBuf>) {
    if let Ok(mut r) = RECORDING.lock() {
        *r = crow_dir;
    }
}

/// The `.crow` directory a recording keeps its fixtures under, if one runs.
pub fn recording() -> Option<PathBuf> {
    RECORDING.lock().ok().and_then(|r| r.clone())
}

/// Where a rehearsal (`crow check`, no recording) reads the fixtures a
/// recording kept: a request one holds is answered from it, any other
/// is answered `200` with nothing to say, by `request`'s own body.
static REHEARSING: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Start (`Some(.crow)`) or end (`None`) a rehearsal.
pub fn rehearse_in(crow_dir: Option<PathBuf>) {
    if let Ok(mut r) = REHEARSING.lock() {
        *r = crow_dir;
    }
}

/// The `.crow` directory a rehearsal reads fixtures from, if one runs.
pub fn rehearsing() -> Option<PathBuf> {
    REHEARSING.lock().ok().and_then(|r| r.clone())
}

/// `sys.replaying[]`: whether a replay answers the requests.
pub fn replaying_now() -> bool {
    replaying().is_some()
}

/// `sys.rehearsing[]`: whether a rehearsal stands in for the requests.
pub fn rehearsing_now() -> bool {
    rehearsing().is_some()
}

/// `sys.recording[]`: whether a recording keeps what the requests answer.
pub fn recording_now() -> bool {
    recording().is_some()
}

/// What a replay had no fixture for, kept where the test run reads it
/// (`CROW_REPLAY_MISSES`, a file every process of the run appends to — a
/// compiled app's misses are its own process's), so a test that fails says
/// what was not recorded: the whole sentence, as the run prints it.
pub fn note_miss(sentence: &str) {
    let Some(path) = std::env::var_os("CROW_REPLAY_MISSES") else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_all(&mut f, format!("{}\n", sentence).as_bytes());
    }
}

/// The misses noted since this was last asked, each once, in the order
/// they were met.
pub fn take_misses() -> Vec<String> {
    let Some(path) = std::env::var_os("CROW_REPLAY_MISSES") else { return Vec::new() };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::write(&path, "");
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        if !out.iter().any(|o| o == line) {
            out.push(line.to_string());
        }
    }
    out
}

/// The requests a recording kept since this was last asked, each as the
/// run prints it: `POST https://… — 200, 1234 bytes`.
static KEPT: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn take_kept() -> Vec<String> {
    KEPT.lock().map(|mut k| std::mem::take(&mut *k)).unwrap_or_default()
}

/// A request's fixture: `.crow/fixtures/<fnv(method, url, body)>.json`.
pub fn request_path(crow_dir: &Path, method: &str, url: &str, body: &str) -> PathBuf {
    crow_dir.join("fixtures").join(format!("{}.json", fnv(&[method, url, body])))
}

/// A request's fixture as text: the request as it was made, and what it
/// was answered.
pub fn fixture_text(method: &str, url: &str, body: &str, reply: &Reply) -> String {
    let headers: serde_json::Map<String, serde_json::Value> =
        reply.headers.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
    let v = serde_json::json!({
        "kind": "request",
        "method": method,
        "url": url,
        "body": body,
        "reply": { "status": reply.status, "headers": headers, "body": reply.body },
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// What a request's fixture holds, read back; none where the text is no
/// request's fixture.
pub fn held_reply(text: &str) -> Option<Reply> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("kind")?.as_str()? != "request" {
        return None;
    }
    let r = v.get("reply")?;
    let status = r.get("status")?.as_i64()?;
    let body = r.get("body")?.as_str()?.to_string();
    let headers: BTreeMap<String, String> =
        r.get("headers")?.as_object()?.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
    Some(Reply { status, body, headers })
}

/// The sentence a miss is named by: the request, and what records it.
fn missed(method: &str, url: &str, verb: &str) -> String {
    format!("no fixture {} `{} {}` — `crow check --record-fixtures` records it where the server is", verb, method, url)
}

/// The fixture under `crow_dir` for the request, or the miss: in a replay
/// the miss is noted for the run's report; in a recording it only tells
/// `request` to send.
pub fn answer_from(crow_dir: &Path, in_replay: bool, method: &str, url: &str, body: &str) -> Result<Reply, String> {
    let path = request_path(crow_dir, method, url, body);
    match std::fs::read_to_string(&path).ok().and_then(|t| held_reply(&t)) {
        Some(r) => Ok(r),
        None => {
            if in_replay {
                note_miss(&missed(method, url, "held"));
            }
            Err(missed(method, url, "holds"))
        }
    }
}

/// `sys.replayed[method, url, body]`: the request's answer from its fixture
/// — the replay's project's, else the recording's, else the rehearsal's —
/// or the miss, an error value of kind `'http'` as a request that got no
/// answer is.
pub fn replayed(method: &str, url: &str, body: &str) -> Result<Reply, String> {
    match (replaying(), recording().or_else(rehearsing)) {
        (Some(dir), _) => answer_from(&dir, true, method, url, body),
        (None, Some(dir)) => answer_from(&dir, false, method, url, body),
        (None, None) => Err(format!("no fixture holds `{} {}`: no replay, recording or rehearsal runs", method, url)),
    }
}

/// `sys.recorded[method, url, body, status, text, headers]`: in a
/// recording, what the request was answered, kept as its fixture; nothing
/// otherwise.
pub fn recorded(method: &str, url: &str, body: &str, status: i64, text: &str, headers: &BTreeMap<String, String>) -> Result<(), String> {
    let Some(dir) = recording() else { return Ok(()) };
    let path = request_path(&dir, method, url, body);
    let cannot = |e: std::io::Error| format!("cannot keep the fixture of {} {}: {}", method, url, e);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(cannot)?;
    }
    let reply = Reply { status, body: text.to_string(), headers: headers.clone() };
    std::fs::write(&path, fixture_text(method, url, body, &reply)).map_err(cannot)?;
    if let Ok(mut k) = KEPT.lock() {
        k.push(format!("{} {} — {}, {} bytes", method, url, status, text.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_the_files_name() {
        assert_eq!(fnv(&["a"]), "af63dc4c8601ec8c");
        assert_eq!(fnv(&["a", "b"]).len(), 16);
        assert_ne!(fnv(&["a", "b"]), fnv(&["ab"]), "the parts are kept apart");
    }

    /// A recording keeps what a request was answered; a replay reads it
    /// back by the request as it was made, and names what it never saw.
    #[test]
    fn a_recording_keeps_a_requests_answer_and_a_replay_reads_it() {
        let dir = std::env::temp_dir().join(format!("crow_request_fixtures_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut headers = BTreeMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        // nothing is kept where no recording runs
        recorded("POST", "http://x.test/v1", "{\"q\": 1}", 200, "{\"a\": 1}", &headers).unwrap();
        assert!(!dir.exists());
        record_to(Some(dir.clone()));
        recorded("POST", "http://x.test/v1", "{\"q\": 1}", 200, "{\"a\": 1}", &headers).unwrap();
        record_to(None);
        assert_eq!(take_kept(), vec!["POST http://x.test/v1 — 200, 8 bytes".to_string()]);
        let text = std::fs::read_to_string(request_path(&dir, "POST", "http://x.test/v1", "{\"q\": 1}")).unwrap();
        assert!(text.contains("\"kind\": \"request\"") && text.contains("\"status\": 200"), "{}", text);
        let held = answer_from(&dir, true, "POST", "http://x.test/v1", "{\"q\": 1}").unwrap();
        assert_eq!((held.status, held.body.as_str(), held.headers.get("content-type").map(String::as_str)), (200, "{\"a\": 1}", Some("application/json")));
        // another body is another request
        let miss = answer_from(&dir, false, "POST", "http://x.test/v1", "{\"q\": 2}").unwrap_err();
        assert_eq!(miss, "no fixture holds `POST http://x.test/v1` — `crow check --record-fixtures` records it where the server is");
        // a statement's fixture is no request's
        std::fs::write(dir.join("fixtures").join(format!("{}.json", fnv(&["GET", "http://x.test/t", ""]))), "{\"sql\": \"SELECT 1\", \"columns\": [], \"rows\": []}").unwrap();
        assert!(answer_from(&dir, false, "GET", "http://x.test/t", "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
