//! Who the reader is (spec: *Who the reader is*; session-layer-design.md
//! §4.1). A sign-in travels in a cookie the app signs; a page trades a
//! one-time ticket for it, because a socket cannot set an `HttpOnly`
//! cookie; a sign-out ends the sign-in everywhere, and the ended ones are
//! kept until their cookies would have expired, so a copied cookie is
//! refused too.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The cookie a sign-in travels in.
pub const COOKIE: &str = "crow_reader";
/// How long a sign-in lasts.
pub const DAYS: u64 = 30;
/// How long a ticket waits for its page to trade it.
const TICKET: Duration = Duration::from_secs(60);
/// Where the app's key and its ended sign-ins live; the directory ignores
/// itself in version control.
const KEYS_DIR: &str = ".crow/keys";

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).ok()
}

/// A file only its owner reads, in the keys directory.
fn write_private(name: &str, body: &str) -> Result<(), String> {
    write_private_in(std::path::Path::new(KEYS_DIR), name, body)
}

fn write_private_in(dir: &std::path::Path, name: &str, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    let _ = std::fs::write(dir.join(".gitignore"), "*\n");
    let path = dir.join(name);
    // created owner-only: written first and narrowed after, another user
    // on the machine could have read it in between
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    std::io::Write::write_all(&mut file, body.as_bytes()).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        // (a file an older run wrote keeps the mode it was made with)
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A `CROW_SECRET` that is set must be long enough to sign with.
pub fn check_secret_env() -> Result<(), String> {
    match std::env::var("CROW_SECRET") {
        Ok(s) if s.chars().count() < 32 => Err(format!(
            "CROW_SECRET is {} characters; a secret to sign readers in with needs at least 32",
            s.chars().count()
        )),
        _ => Ok(()),
    }
}

/// The app's secret: `CROW_SECRET`, or a key the runtime writes to
/// `.crow/keys/` the first time it needs one. Changing it signs everyone out.
pub fn secret() -> Result<&'static [u8], String> {
    static SECRET: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    SECRET
        .get_or_init(|| {
            if let Ok(s) = std::env::var("CROW_SECRET") {
                check_secret_env()?;
                return Ok(s.into_bytes());
            }
            let path = std::path::Path::new(KEYS_DIR).join("secret");
            if let Ok(hex) = std::fs::read_to_string(&path) {
                if hex.trim().len() >= 64 {
                    return Ok(hex.trim().as_bytes().to_vec());
                }
            }
            let hex = crow_runtime::signin::random_hex(32)?;
            write_private("secret", &hex)?;
            Ok(hex.into_bytes())
        })
        .as_ref()
        .map(|v| v.as_slice())
        .map_err(|e| e.clone())
}

fn mac(key: &[u8], body: &str) -> Result<Hmac<Sha256>, String> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).map_err(|e| e.to_string())?;
    m.update(body.as_bytes());
    Ok(m)
}

/// A sign-in as its cookie carries it: the record, the sign-in's id and
/// when it ends, and the app's seal over them.
pub fn seal(record: &BTreeMap<String, String>, sid: &str) -> Result<String, String> {
    seal_with(secret()?, record, sid, now_secs() + DAYS * 86_400)
}

fn seal_with(key: &[u8], record: &BTreeMap<String, String>, sid: &str, exp: u64) -> Result<String, String> {
    let payload = serde_json::json!({ "r": record, "sid": sid, "exp": exp }).to_string();
    let body = b64(payload.as_bytes());
    let tag = mac(key, &body)?.finalize().into_bytes();
    Ok(format!("{}.{}", body, b64(&tag)))
}

/// The sign-in a cookie carries: when the seal is the app's, the sign-in
/// has not ended and has not been signed out.
pub fn open(cookie: &str) -> Option<(BTreeMap<String, String>, String)> {
    let (record, sid) = open_with(secret().ok()?, cookie, now_secs())?;
    if is_ended(&sid) {
        return None;
    }
    Some((record, sid))
}

fn open_with(key: &[u8], cookie: &str, now: u64) -> Option<(BTreeMap<String, String>, String)> {
    let (body, tag) = cookie.split_once('.')?;
    // the seal is checked in constant time before anything is read
    mac(key, body).ok()?.verify_slice(&unb64(tag)?).ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&unb64(body)?).ok()?;
    if payload["exp"].as_u64()? <= now {
        return None;
    }
    let sid = payload["sid"].as_str()?.to_string();
    let record = payload["r"]
        .as_object()?
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();
    Some((record, sid))
}

/// The sign-in the request's cookies carry, if one is good.
pub fn from_headers(headers: &axum::http::HeaderMap) -> Option<(BTreeMap<String, String>, String)> {
    for value in headers.get_all(axum::http::header::COOKIE) {
        let Ok(text) = value.to_str() else { continue };
        for part in text.split(';') {
            if let Some(v) = part.trim().strip_prefix(COOKIE).and_then(|r| r.strip_prefix('=')) {
                if let Some(found) = open(v) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// The `Set-Cookie` a page is answered with: a sign-in, or the cookie
/// cleared. `Secure` when the page came over https through a proxy.
pub fn set_cookie(value: Option<&str>, secure: bool) -> String {
    let tail = if secure { "; Secure" } else { "" };
    match value {
        Some(v) => format!("{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{}", COOKIE, v, DAYS * 86_400, tail),
        None => format!("{}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax{}", COOKIE, tail),
    }
}

/// Tickets waiting for their page: the cookie each is traded for (none: the
/// cookie is cleared).
fn tickets() -> &'static Mutex<HashMap<String, (Instant, Option<String>)>> {
    static T: OnceLock<Mutex<HashMap<String, (Instant, Option<String>)>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A one-time ticket for this cookie, good for a minute.
pub fn ticket(cookie: Option<String>) -> Result<String, String> {
    let t = crow_runtime::signin::random_hex(16)?;
    let mut all = tickets().lock().unwrap_or_else(|p| p.into_inner());
    all.retain(|_, (at, _)| at.elapsed() < TICKET);
    all.insert(t.clone(), (Instant::now(), cookie));
    Ok(t)
}

/// Trade a ticket, once: the cookie it stands for.
pub fn redeem(t: &str) -> Option<Option<String>> {
    let mut all = tickets().lock().unwrap_or_else(|p| p.into_inner());
    let (at, cookie) = all.remove(t)?;
    (at.elapsed() < TICKET).then_some(cookie)
}

/// Sign-ins that were signed out, until their cookies would have ended.
fn ended() -> &'static Mutex<HashMap<String, u64>> {
    static E: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    E.get_or_init(|| {
        let mut m = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(std::path::Path::new(KEYS_DIR).join("signed_out")) {
            let now = now_secs();
            for line in text.lines() {
                if let Some((sid, exp)) = line.split_once(' ') {
                    if let Ok(exp) = exp.trim().parse::<u64>() {
                        if exp > now {
                            m.insert(sid.to_string(), exp);
                        }
                    }
                }
            }
        }
        Mutex::new(m)
    })
}

/// A sign-in ends: its cookie is refused from now on, here and after a
/// restart.
pub fn end(sid: &str) {
    if sid.is_empty() {
        return;
    }
    let now = now_secs();
    let body = {
        let mut all = ended().lock().unwrap_or_else(|p| p.into_inner());
        all.retain(|_, exp| *exp > now);
        all.insert(sid.to_string(), now + DAYS * 86_400);
        all.iter().map(|(s, e)| format!("{} {}\n", s, e)).collect::<String>()
    };
    if let Err(e) = write_private("signed_out", &body) {
        eprintln!("crow live: {} — a sign-out holds until the program restarts", e);
    }
}

/// Whether a sign-in was signed out.
pub fn is_ended(sid: &str) -> bool {
    !sid.is_empty() && ended().lock().unwrap_or_else(|p| p.into_inner()).contains_key(sid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key is the owner's from the moment it exists: created so, not
    /// written and then narrowed
    #[cfg(unix)]
    #[test]
    fn a_key_is_created_for_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("crow_keys_{}", std::process::id()));
        write_private_in(&dir, "secret", "abc").unwrap();
        let mode = std::fs::metadata(dir.join("secret")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read_to_string(dir.join("secret")).unwrap(), "abc");
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "*\n");
        // written again, it holds the new text alone
        write_private_in(&dir, "secret", "d").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("secret")).unwrap(), "d");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn ada() -> BTreeMap<String, String> {
        let mut r = BTreeMap::new();
        r.insert("id".to_string(), "42".to_string());
        r.insert("role".to_string(), "admin".to_string());
        r
    }

    /// A sealed sign-in opens with the app's key, before it ends, and in no
    /// other case: a changed byte, another key or a past end is nobody
    #[test]
    fn a_cookie_opens_only_as_sealed() {
        let key = b"k".repeat(32);
        let cookie = seal_with(&key, &ada(), "s1", 2_000).unwrap();
        let (record, sid) = open_with(&key, &cookie, 1_000).expect("the app's own cookie opens");
        assert_eq!(record, ada());
        assert_eq!(sid, "s1");
        assert!(open_with(&b"x".repeat(32), &cookie, 1_000).is_none(), "another key");
        assert!(open_with(&key, &cookie, 2_000).is_none(), "an ended sign-in");
        // a reader who edits the record to make themselves someone else
        let (body, tag) = cookie.split_once('.').unwrap();
        let forged = String::from_utf8(unb64(body).unwrap()).unwrap().replace("admin", "owner");
        assert!(open_with(&key, &format!("{}.{}", b64(forged.as_bytes()), tag), 1_000).is_none(), "an edited record");
        assert!(open_with(&key, "nonsense", 1_000).is_none());
    }

    /// A ticket is traded once, and a sign-out is refused afterwards
    #[test]
    fn a_ticket_is_good_once_and_an_ended_sign_in_is_refused() {
        let t = ticket(Some("c".to_string())).unwrap();
        assert_eq!(redeem(&t), Some(Some("c".to_string())));
        assert_eq!(redeem(&t), None, "a ticket is good once");
        // a fresh id: the ended sign-ins persist across runs
        let sid = crow_runtime::signin::random_hex(8).unwrap();
        assert!(!is_ended(&sid));
        ended().lock().unwrap().insert(sid.clone(), now_secs() + 60);
        assert!(is_ended(&sid));
        assert!(!is_ended(""));
    }

    #[test]
    fn the_cookie_is_http_only_and_lax() {
        let c = set_cookie(Some("v"), false);
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Lax") && c.contains("Path=/") && !c.contains("Secure"));
        assert!(set_cookie(None, true).contains("Max-Age=0") && set_cookie(None, true).contains("Secure"));
    }
}
