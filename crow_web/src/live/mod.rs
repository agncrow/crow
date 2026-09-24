//! `web.live` module — LiveView: server-rendered HTML with real-time updates.
//!
//! Two modes:
//! - **VNode mode**: separate render + handle functions, VNode tree diffing
//! - **Legacy mode**: single handler returning HTML string
//!
//! ```text
//! use web
//!
//! app = web.new[]
//!
//! fn render[state]
//!     count = state.count
//!     return div[h1['Count: {count}'], button[click = 'inc', '+'], button[click = 'dec', '-']]
//!
//! fn handle[state, event]
//!     if event eq 'inc'
//!         state.count = state.count + 1
//!     if event eq 'dec'
//!         state.count = state.count - 1
//!
//! live[app, '/', render, handle]
//! serve[app, 8080]
//! ```

use std::collections::HashMap;
use crate::view::CrowVNode;

pub mod files;
pub mod signin;

/// Internal action type for the WebSocket event loop.
enum LiveAction {
    /// A WebSocket message arrived from the client.
    Message(axum::extract::ws::Message),
    /// A poll timer fired — time to re-fetch data sources.
    PollTick,
    /// A later[] background fn finished — apply its state writes.
    LaterDelta(Vec<(String, serde_json::Value)>),
    /// The serve[…, refresh = N] interval fired — re-render this session.
    RefreshTick,
    /// A background recompute landed, or another session wrote a table
    /// this page reads — render again.
    Wake,
    /// The running handler ended: its working copy, to commit (none when it
    /// panicked).
    /// the handler's working copy, or what made it panic
    HandlerDone(Result<CrowLiveState, String>),
}

/// A data source that fetches from a URL — one-time or periodic.
#[derive(Debug, Clone)]
pub struct DataSource {
    /// URL to fetch from.
    pub url: String,
    /// State key to store the result under.
    pub key: String,
    /// If Some, poll every N seconds. If None, fetch once on mount.
    pub interval_secs: Option<u64>,
}

/// What a derivation reads (derive-design.md §3.2), fixed at compile time
/// by the same analysis the interpreter uses.
pub struct DeriveSpec {
    pub name: &'static str,
    pub facts: &'static [&'static str],
    pub derives: &'static [&'static str],
    pub tables: &'static [&'static str],
    pub now: bool,
    pub external: bool,
    pub fresh: f64,
    /// reads no fact a handler writes and no per-session derivation: one
    /// value for the app, computed once and shared by every session
    pub shared: bool,
}

/// A derivation's cached value and the input versions it was computed from.
#[derive(Clone)]
pub struct DerivedSlot {
    value: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    version: u64,
    facts_seen: Vec<(String, u64)>,
    derives_seen: Vec<(String, u64)>,
    tables_seen: Vec<(String, u64)>,
    tick_seen: u64,
    computed_at: f64,
    /// when the computation that produced it started, in the app's count:
    /// a recompute that started earlier never replaces a later one
    stamp: u64,
}

impl std::fmt::Debug for DerivedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DerivedSlot(v{})", self.version)
    }
}

/// Where a derivation's inputs stand at a read.
struct Inputs {
    facts: Vec<(String, u64)>,
    tables: Vec<(String, u64)>,
    derives: Vec<(String, u64)>,
    now: f64,
}

impl Inputs {
    /// What the inputs are, apart from the clock: a recompute started from
    /// other inputs is superseded.
    fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.facts.hash(&mut h);
        self.tables.hash(&mut h);
        self.derives.hash(&mut h);
        h.finish()
    }
}

/// What the cache holds for a derivation at a read.
enum Found<T> {
    Fresh(T),
    Stale(T),
    Missing,
}

/// How a read of a derivation behaves. A render goes ahead with what it
/// has while a slow value catches up; a handler, a scheduled function and
/// a background recompute compute what is stale before reading it.
const MODE_FRESH: u8 = 0;
const MODE_RENDER: u8 = 1;

/// What a session's last render read: the derivations and the tables
/// under them — whom a landed recompute or another session's write wakes.
#[derive(Default)]
struct ReadSet {
    derives: std::collections::HashSet<&'static str>,
    tables: std::collections::HashSet<&'static str>,
}

/// A live session as the rest of the app sees it: the signal that wakes
/// its loop, and what its page last read.
pub struct SessionHandle {
    id: u64,
    wake: tokio::sync::Notify,
    reads: std::sync::Mutex<ReadSet>,
    /// the sign-in this session's reader came from (empty: nobody)
    reader_sid: std::sync::Mutex<String>,
}

/// A session gone — its page closed, its wait for a socket over — stops
/// the recomputes it started: nobody will read them.
impl Drop for SessionHandle {
    fn drop(&mut self) {
        files::forget_uploads(self.id);
        let inflight = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        for ((holder, _), job) in inflight.iter() {
            if *holder == self.id {
                job.flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionHandle({})", self.id)
    }
}

/// Every live session, by id; a session gone is a dead weak reference,
/// swept as the registry is walked.
static SESSIONS: std::sync::LazyLock<std::sync::Mutex<HashMap<u64, std::sync::Weak<SessionHandle>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// A recompute in flight: the flag that stops it, and the inputs it was
/// started from.
struct InFlightJob {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    inputs: u64,
}

/// Recomputes in flight, by holder and derivation: the holder is the
/// session, or 0 for a derivation the app shares. One at a time each.
static INFLIGHT: std::sync::LazyLock<std::sync::Mutex<HashMap<(u64, &'static str), InFlightJob>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// The order computations start in, app-wide.
static STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Versions of facts and tables, unique app-wide: a version names one value,
/// so a working copy a handler threw away can never pass for a later one.
static VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_version() -> u64 {
    VERSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

/// An effect a handler asked for whose answer it does not use: performed
/// when the handler commits, in program order; dropped when it stops.
#[derive(Clone)]
enum Held {
    Notice(String, f64),
    Later(String, fn(&mut CrowLiveState)),
    /// sign_in[record] (Some) or sign_out[] (None)
    Sign(Option<std::collections::BTreeMap<String, String>>),
    /// download[value, name]: the file's name and bytes
    Download(String, Vec<u8>),
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Held::Notice(t, _) => write!(f, "notice {:?}", t),
            Held::Later(n, _) => write!(f, "later {}", n),
            Held::Sign(r) => write!(f, "sign {}", if r.is_some() { "in" } else { "out" }),
            Held::Download(n, b) => write!(f, "download {} ({} bytes)", n, b.len()),
        }
    }
}

/// The session's signed-in reader, and the sign-in it came from — kept with
/// its facts, so a kept session keeps them; no input binds either.
const READER: &str = "reader";
const READER_SID: &str = "__reader_sid";
/// The page each paged table shows, kept with the session's facts.
const PAGES: &str = "__pages";

/// How long a handler may run before its page shows it working: a quick
/// one is answered by one render, a slow one by two.
const HANDLER_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

/// How long the runtime's toast stays on a page that shows no notice itself.
const NOTICE_SECS: f64 = 4.0;

/// A table write lands for the app: every derivation over the table, in
/// every session, is stale, and every other session whose page read one is
/// woken to catch up.
fn publish_table(table: &str, version: u64, except: u64) {
    TABLE_VERSIONS.lock().unwrap_or_else(|p| p.into_inner()).insert(table.to_string(), version);
    wake_sessions(except, |reads| reads.tables.contains(table));
}

/// Wake every live session whose last render passes `reads`, except one.
fn wake_sessions(except: u64, reads: impl Fn(&ReadSet) -> bool) {
    let live: Vec<std::sync::Arc<SessionHandle>> = {
        let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        sessions.retain(|_, w| w.strong_count() > 0);
        sessions.values().filter_map(|w| w.upgrade()).collect()
    };
    for h in live {
        if h.id != except && reads(&h.reads.lock().unwrap_or_else(|p| p.into_inner())) {
            h.wake.notify_one();
        }
    }
}

/// Wake every live session holding a sign-in, except one: it has ended.
fn wake_holders(sid: &str, except: u64) {
    let live: Vec<std::sync::Arc<SessionHandle>> = {
        let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        sessions.retain(|_, w| w.strong_count() > 0);
        sessions.values().filter_map(|w| w.upgrade()).collect()
    };
    for h in live {
        if h.id != except && *h.reader_sid.lock().unwrap_or_else(|p| p.into_inner()) == sid {
            h.wake.notify_one();
        }
    }
}

/// A new session from the app's initial state: its own facts, its own
/// derivation cache, and a handle the app can wake it through.
pub fn start_session(init: &CrowLiveState) -> CrowLiveState {
    let mut state = init.clone();
    state.session_id = next_session_id();
    let sid = state.data.get(READER_SID).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let handle = std::sync::Arc::new(SessionHandle {
        id: state.session_id,
        wake: tokio::sync::Notify::new(),
        reads: std::sync::Mutex::new(ReadSet::default()),
        reader_sid: std::sync::Mutex::new(sid),
    });
    SESSIONS.lock().unwrap_or_else(|p| p.into_inner()).insert(state.session_id, std::sync::Arc::downgrade(&handle));
    state.handle = Some(handle);
    state
}

/// Server-side component state.
#[derive(Debug)]
pub struct CrowLiveState {
    pub data: HashMap<String, serde_json::Value>,
    /// one version per fact key, bumped by every set — what derivations
    /// compare their inputs against
    pub versions: HashMap<String, u64>,
    /// the render this session is at, taken from the app-wide counter:
    /// what "as fresh as the loop" means
    pub tick: u64,
    /// the derivation cache of this session, shared with its background
    /// recomputes; derivations over sources, constants and the clock alone
    /// live in SHARED_DERIVED instead
    derived: std::sync::Arc<std::sync::Mutex<HashMap<String, DerivedSlot>>>,
    /// Registered data sources (fetch/poll).
    pub sources: Vec<DataSource>,
    /// Session-scoped channel delivering later[] state deltas to the live loop.
    pub later_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<(String, serde_json::Value)>>>,
    /// Distinguishes sessions in the later[] in-flight registry.
    pub session_id: u64,
    /// MODE_RENDER while a render reads, MODE_FRESH otherwise
    mode: std::sync::atomic::AtomicU8,
    /// how the app wakes this session's loop (none outside a served page)
    handle: Option<std::sync::Arc<SessionHandle>>,
    /// a background recompute's way back to its session, which it does not
    /// keep alive: a session gone stops its recomputes
    waker: Option<std::sync::Weak<SessionHandle>>,
    /// the tables the current event wrote: a derivation over one is part of
    /// the event's answer and is recomputed before its render
    tables_written: std::sync::Mutex<std::collections::HashSet<String>>,
    /// one entry per view being rendered: whether it read a value still
    /// being recomputed, which marks its root `data-crow-stale`
    view_stale: std::sync::Mutex<Vec<bool>>,
    /// a handler's working copy (core-design.md §1): its writes land when it
    /// commits, as one, or not at all
    copy: bool,
    /// tables the working copy wrote, with their new versions, published
    /// when it commits
    local_tables: std::sync::Mutex<HashMap<String, u64>>,
    /// the reason the handler stopped, if it did
    stopped: Option<String>,
    /// effects held to the commit
    held: Vec<Held>,
    /// the session's notice and the moment it landed
    pub notice: Option<(String, f64)>,
    /// a working copy's fact versions when it was taken: what the handler
    /// changed is what commits, so a value that landed meanwhile (a
    /// schedule's, a poll's) is not overwritten
    base_versions: HashMap<String, u64>,
    /// whether the render read `notice` (a page that does not gets the
    /// runtime's toast)
    notice_read: std::sync::atomic::AtomicBool,
    /// the question a `confirm =` click asked and the page is showing: the
    /// handler, the arguments its click froze, the text
    asking: Option<(String, Option<String>, String)>,
    /// a sign-in (its sealed cookie) or a sign-out (none) the last commit
    /// made, for the page to trade for its cookie
    pub(crate) signing: Option<Option<String>>,
    /// paged tables drawn so far in the render under way
    tables_drawn: std::sync::atomic::AtomicUsize,
    /// files the last commits handed the reader, for the page to fetch
    pub(crate) downloads: Vec<(String, Vec<u8>)>,
}

/// Derivations shared by every session (derive-design.md §3.5): one value
/// for the app, whichever session computed it.
static SHARED_DERIVED: std::sync::LazyLock<std::sync::Mutex<HashMap<String, DerivedSlot>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
/// Tables this program wrote, versioned app-wide: a write in any session
/// is a write every session's derivations see.
static TABLE_VERSIONS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, u64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
/// The render counter every session advances.
static TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// serve[port, refresh = secs, budget = ms] configuration — set once before
/// serve, read by every session loop. 0 = unset.
static REFRESH_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BUDGET_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The look `serve[..., theme, density, accent, wide]` asked for, as the
/// attributes of the page's `<html>` element.
static LOOK_ATTRS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// serve[…, theme = 'dark', density = 'compact', accent = '#c60', wide = true]:
/// remember the look for every page shell. A bad value is a startup error.
pub fn configure_look(theme: &str, density: &str, accent: &str, wide: bool) {
    match crow_runtime::layout::look_attrs(theme, density, accent) {
        Ok(mut attrs) => {
            if wide { attrs.push_str(" data-wide=\"true\""); }
            *LOOK_ATTRS.lock().unwrap_or_else(|p| p.into_inner()) = attrs;
        }
        Err(e) => {
            eprintln!("error: serve[]: {}", e);
            std::process::exit(1);
        }
    }
}

fn look_attrs() -> String {
    LOOK_ATTRS.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

pub fn configure(refresh_secs: i64, budget_ms: i64) {
    REFRESH_SECS.store(refresh_secs.max(0) as u64, std::sync::atomic::Ordering::SeqCst);
    BUDGET_MS.store(budget_ms.max(0) as u64, std::sync::atomic::Ordering::SeqCst);
}

/// serve[…, keep = true]: sessions survive a restart.
static KEEP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn configure_keep(on: bool) {
    KEEP.store(on, std::sync::atomic::Ordering::SeqCst);
}

fn keeping() -> bool {
    KEEP.load(std::sync::atomic::Ordering::SeqCst)
}

/// Where kept sessions wait for a restarted program, one file per tab token.
const KEPT_DIR: &str = ".crow/sessions";

fn kept_path(dir: &std::path::Path, token: &str) -> Option<std::path::PathBuf> {
    // a token is hex, and names nothing but its own file
    (!token.is_empty() && token.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| dir.join(format!("{}.json", token)))
}

fn keep_session(token: &str, state: &CrowLiveState) {
    if keeping() {
        keep_session_in(std::path::Path::new(KEPT_DIR), token, state);
    }
}

/// Write a session's facts under the token its tab holds, for a restart to
/// find — atomically, in a directory that ignores itself in version control.
fn keep_session_in(dir: &std::path::Path, token: &str, state: &CrowLiveState) {
    let Some(path) = kept_path(dir, token) else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(&ignore, "*\n");
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, serde_json::to_string(&state.data).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn forget_kept(token: &str) {
    if let Some(path) = kept_path(std::path::Path::new(KEPT_DIR), token).filter(|_| keeping()) {
        let _ = std::fs::remove_file(path);
    }
}

/// The same kind of value: what a kept fact must still be to come back.
fn same_shape(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value as V;
    matches!((a, b), (V::Null, V::Null) | (V::Bool(_), V::Bool(_)) | (V::Number(_), V::Number(_)) | (V::String(_), V::String(_)) | (V::Array(_), V::Array(_)) | (V::Object(_), V::Object(_)))
}

/// A session a restarted program kept for a tab that comes back with its
/// token: its facts, where the program still declares them with the same
/// shape; the rest start from their declared values, and the log says so.
/// Good once, and only within the grace period.
fn kept_session(token: &str, init: &CrowLiveState) -> Option<CrowLiveState> {
    if !keeping() {
        return None;
    }
    kept_session_in(std::path::Path::new(KEPT_DIR), token, init)
}

fn kept_session_in(dir: &std::path::Path, token: &str, init: &CrowLiveState) -> Option<CrowLiveState> {
    let path = kept_path(dir, token)?;
    let age = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().ok()?;
    let body = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    if age > PendingSessions::GRACE {
        return None;
    }
    let facts: HashMap<String, serde_json::Value> = serde_json::from_str(&body?).ok()?;
    let mut state = start_session(init);
    for (key, value) in facts {
        match init.data.get(&key) {
            Some(declared) if same_shape(declared, &value) => state.set(&key, value),
            _ => eprintln!("crow live: a kept session's fact '{}' changed since it was kept; it starts from its declared value", key),
        }
    }
    Some(state)
}

fn refresh_secs() -> Option<u64> {
    match REFRESH_SECS.load(std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        n => Some(n),
    }
}

fn budget_ms() -> Option<u128> {
    match BUDGET_MS.load(std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        n => Some(n as u128),
    }
}

/// In-flight registry for later[] work, keyed "session:fn".
fn later_registry() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static R: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// A bound field or slider always sends its value as text. Store it in the
/// shape the variable already has: an int var gets an int, a float var a
/// float, a bool var a bool, anything else the text. Text that does not
/// parse leaves a number or bool untouched (`None`), so a half-typed field
/// never resets a counter to zero.
pub fn coerce_field_value(current: Option<&serde_json::Value>, val: &str) -> Option<serde_json::Value> {
    use serde_json::Value;
    let t = val.trim();
    match current {
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => t
            .parse::<i64>()
            .ok()
            .or_else(|| t.parse::<f64>().ok().map(|f| f as i64))
            .map(Value::from),
        Some(Value::Number(_)) => t.parse::<f64>().ok().map(|f| serde_json::json!(f)),
        Some(Value::Bool(_)) => match t {
            "true" | "on" | "1" | "yes" => Some(Value::Bool(true)),
            "false" | "off" | "0" | "no" | "" => Some(Value::Bool(false)),
            _ => None,
        },
        _ => Some(Value::String(val.to_string())),
    }
}

/// A page's socket may open only from the app's own page. A browser always
/// names the page that opened a socket (`Origin`), so a page on another
/// site is refused before it can drive a handler; a client that is not a
/// browser sends no origin and is let through.
pub fn origin_allowed(origin: Option<&str>, host: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => {
            let authority = o.split_once("://").map(|(_, rest)| rest).unwrap_or(o).trim_end_matches('/');
            host.map(|h| authority.eq_ignore_ascii_case(h)).unwrap_or(false)
        }
    }
}

/// An attribute value as the browser reads it: the markup's escapes undone.
fn unescape_attr(v: &str) -> String {
    v.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// What a render let the page do: each click it carries, with the arguments
/// the render froze into it, and each bound input, select and form by its
/// event name. A session accepts an event only when a render it sent
/// offered it — a handler no button names, a helper, a confirmation's
/// second step without its first, or a fact no input binds are refused.
#[derive(Default, Clone, Debug)]
pub struct Offers {
    clicks: std::collections::HashSet<(String, Option<String>)>,
    named: std::collections::HashSet<String>,
    /// the file fields a render drew (`__field:name`)
    uploads: std::collections::HashSet<String>,
    /// the handlers a render's widgets name: each takes whatever value its
    /// widget sends, as a field's handler takes its text
    widget_handlers: std::collections::HashSet<String>,
}

impl Offers {
    pub fn of(node: &CrowVNode) -> Offers {
        let mut o = Offers::default();
        o.collect(node);
        o
    }

    fn collect(&mut self, node: &CrowVNode) {
        match node {
            CrowVNode::Element { attrs, children, .. } => {
                // a closed modal or a hidden element offers nothing: nobody
                // can press what the page does not show, so a confirmation's
                // second step is accepted only once its first opened it
                if attrs.get("data-open").map(|v| v == "false").unwrap_or(false) || attrs.contains_key("hidden") {
                    return;
                }
                if let Some(h) = attrs.get("data-crow-click") {
                    self.clicks.insert((h.clone(), attrs.get("data-crow-args").cloned()));
                }
                for a in ["data-crow-input", "data-crow-change", "data-crow-submit"] {
                    if let Some(n) = attrs.get(a) {
                        self.named.insert(n.clone());
                    }
                }
                if let Some(n) = attrs.get("data-crow-upload") {
                    self.uploads.insert(n.clone());
                }
                if let Some(on) = attrs.get("data-crow-on") {
                    self.note_widget(on);
                }
                for c in children {
                    self.collect(c);
                }
            }
            CrowVNode::Raw(html) => self.collect_markup(html),
            CrowVNode::Text(_) => {}
        }
    }

    /// Markup built as text — a chart's seek bars, a table's rows and row
    /// buttons: each tag's event attributes, unescaped as the browser reads
    /// them before sending them back.
    pub fn collect_markup(&mut self, html: &str) {
        fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
            let key = format!(" {}=\"", name);
            let start = tag.find(&key)? + key.len();
            let len = tag[start..].find('"')?;
            Some(&tag[start..start + len])
        }
        for piece in html.split('<').skip(1) {
            let tag = piece.split('>').next().unwrap_or("");
            if !tag.contains("data-crow-") {
                continue;
            }
            if let Some(h) = attr(tag, "data-crow-click") {
                self.clicks.insert((unescape_attr(h), attr(tag, "data-crow-args").map(unescape_attr)));
            }
            for a in ["data-crow-input", "data-crow-change", "data-crow-submit"] {
                if let Some(n) = attr(tag, a) {
                    self.named.insert(unescape_attr(n));
                }
            }
            if let Some(n) = attr(tag, "data-crow-upload") {
                self.uploads.insert(unescape_attr(n));
            }
            if let Some(on) = attr(tag, "data-crow-on") {
                self.note_widget(&unescape_attr(on));
            }
        }
    }

    /// A widget's events: the handlers they reach.
    fn note_widget(&mut self, on: &str) {
        if let Ok(serde_json::Value::Object(events)) = serde_json::from_str::<serde_json::Value>(on) {
            for h in events.values().filter_map(|h| h.as_str()) {
                self.widget_handlers.insert(h.to_string());
            }
        }
    }

    /// Whether this render offered the event: a bound input, select or form
    /// by name; a click by its handler and exactly the arguments it froze.
    pub fn allows(&self, event: &str, value: Option<&str>) -> bool {
        if self.named.contains(event) {
            return true;
        }
        // what a widget the page drew sends its handler
        if self.widget_handlers.contains(event) {
            return true;
        }
        // a file for a file field the page drew
        if event == "__upload" {
            let field = value.and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(v).ok()).and_then(|v| v.first().and_then(|f| f.as_str().map(|s| s.to_string())));
            return field.map_or(false, |f| self.uploads.contains(&f));
        }
        if event.starts_with("__field:") || event.starts_with("__select:") {
            return false;
        }
        self.clicks.contains(&(event.to_string(), value.map(|v| v.to_string())))
    }
}

/// Sessions a page request rendered, each waiting for its page's socket.
/// The page carries a token; the socket that brings it adopts the session
/// — its state, its derivations and the render the page already shows — so
/// a page loads with one render, not one for the page and another for the
/// socket. A socket without a live token (a reconnect, a restarted server)
/// starts a fresh session and is sent its render at once.
#[derive(Default)]
struct PendingSessions {
    parked: std::sync::Mutex<HashMap<String, Parked>>,
}

/// A session waiting for a socket: a page's, until its socket opens; or one
/// whose socket closed, until its tab comes back.
struct Parked {
    until: std::time::Instant,
    state: CrowLiveState,
    vnode: CrowVNode,
    /// a session that had a page before: its tab shows an old render, so it
    /// is sent its render when it comes back
    resumed: bool,
}

/// A fresh token: 128 random bits, the credential a tab resumes with.
fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

impl PendingSessions {
    /// How long a rendered page waits for its socket.
    const WAIT: std::time::Duration = std::time::Duration::from_secs(60);
    /// How long a session whose socket closed waits for its tab.
    const GRACE: std::time::Duration = std::time::Duration::from_secs(600);
    /// At most this many sessions wait at once; the soonest to expire give way.
    const MAX: usize = 1000;

    fn park(&self, state: CrowLiveState, vnode: CrowVNode) -> String {
        let token = new_token();
        self.park_as(token.clone(), state, vnode, Self::WAIT, false);
        token
    }

    fn park_as(&self, token: String, state: CrowLiveState, vnode: CrowVNode, wait: std::time::Duration, resumed: bool) {
        let now = std::time::Instant::now();
        let mut parked = self.parked.lock().unwrap_or_else(|p| p.into_inner());
        parked.retain(|_, p| p.until > now);
        while parked.len() >= Self::MAX {
            let soonest = parked.iter().min_by_key(|(_, p)| p.until).map(|(k, _)| k.clone());
            match soonest {
                Some(k) => { parked.remove(&k); }
                None => break,
            }
        }
        parked.insert(token, Parked { until: now + wait, state, vnode, resumed });
    }

    /// Take a waiting session by its token — once: a token is good once.
    fn claim(&self, token: &str) -> Option<(CrowLiveState, CrowVNode, bool)> {
        let mut parked = self.parked.lock().unwrap_or_else(|p| p.into_inner());
        match parked.remove(token) {
            Some(p) if p.until > std::time::Instant::now() => Some((p.state, p.vnode, p.resumed)),
            _ => None,
        }
    }
}

/// A new session's number, distinguishing sessions in the later[] registry.
fn next_session_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

/// Add what the runtime draws (a toast, a dialog) as the page's last child:
/// inside the root element, whether the view built it as nodes (compiled)
/// or as markup (the interpreter), so both modes serve one page.
fn append_child(vnode: CrowVNode, child: CrowVNode) -> CrowVNode {
    match vnode {
        CrowVNode::Element { tag, attrs, mut children } => {
            children.push(child);
            CrowVNode::Element { tag, attrs, children }
        }
        CrowVNode::Raw(html) if html.ends_with('>') => match html.rfind("</") {
            Some(at) => CrowVNode::Raw(format!("{}{}{}", &html[..at], child.to_html(), &html[at..])),
            None => CrowVNode::tag("div", vec![CrowVNode::Raw(html), child]),
        },
        other => CrowVNode::tag("div", vec![other, child]),
    }
}

/// The runtime's toast, added to a page that shows no notice itself: the
/// session's latest notice, fading after a few seconds.
fn with_toast(vnode: CrowVNode, text: &str) -> CrowVNode {
    let toast = CrowVNode::tag("div", vec![CrowVNode::text(text)])
        .attr("class", "crow-notice")
        .attr("role", "status");
    append_child(vnode, toast)
}

/// The runtime's dialog for a click that asks first: the question, and the
/// two buttons that answer it — offered only while it is open.
fn with_dialog(vnode: CrowVNode, text: &str) -> CrowVNode {
    let dialog = CrowVNode::tag("div", vec![CrowVNode::tag("div", vec![
        CrowVNode::p(vec![CrowVNode::text(text)]),
        CrowVNode::tag("div", vec![
            CrowVNode::button(vec![CrowVNode::text("Cancel")]).on_click("__cancel"),
            CrowVNode::button(vec![CrowVNode::text("Confirm")]).on_click("__confirm").attr("class", "primary"),
        ]).attr("class", "crow-confirm-actions"),
    ]).attr("class", "crow-confirm-box")])
    .attr("class", "crow-confirm")
    .attr("role", "dialog")
    .attr("aria-modal", "true");
    append_child(vnode, dialog)
}

/// Run a session's registered fetch[] sources once, so its first render has
/// their data.
async fn fetch_sources(state: &mut CrowLiveState) {
    for source in &state.sources.clone() {
        if let Ok(resp) = reqwest::get(&source.url).await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                state.set(&source.key, json);
            }
        }
    }
}

/// A message from the page: the event, the value it carries, and the
/// click's number, when it is a click the page marked busy.
fn parse_event(text: &str) -> (String, Option<String>, Option<u64>) {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => (v["event"].as_str().unwrap_or("unknown").to_string(), v["value"].as_str().map(|s| s.to_string()), v["seq"].as_u64()),
        Err(_) => (text.to_string(), None, None),
    }
}

/// A bound input's value, kept in the variable's shape: a tiny transaction
/// of its own.
fn apply_field(state: &mut CrowLiveState, event: &str, value: Option<&str>) -> bool {
    let Some(key) = event.strip_prefix("__field:").or_else(|| event.strip_prefix("__select:")) else { return false };
    // the reader is the runtime's, whatever a page offers
    if key == READER || key.starts_with("__") {
        eprintln!("crow live: refused '{}' — no input writes it", event);
        return true;
    }
    if let Some(val) = value {
        if let Some(v) = coerce_field_value(state.get(key), val) {
            state.set(key, v);
        }
    }
    true
}

/// Run a handler on its working copy as one event and one transaction:
/// its database writes commit with it, or roll back when it stopped, and a
/// commit the database refuses is a stop too. The copy comes back for the
/// session to commit.
fn run_handler<H>(mut copy: CrowLiveState, event: &str, value: Option<&str>, handle: &H) -> CrowLiveState
where
    H: Fn(&mut CrowLiveState, &str, Option<&str>),
{
    let t0 = std::time::Instant::now();
    crow_runtime::txn::begin();
    crow_runtime::moments::at(crow_runtime::moments::clock(), || handle(&mut copy, event, value));
    if let Err(e) = crow_runtime::txn::finish(!copy.is_stopped()) {
        copy.stop(format!("{} stopped: {}", event, e));
    }
    if let Some(budget) = budget_ms() {
        let ms = t0.elapsed().as_millis();
        if ms > budget {
            eprintln!("crow live: handler '{}' took {}ms (budget {}ms)", event, ms, budget);
        }
    }
    copy
}

/// A running handler shows on the page: every element that offers it is
/// disabled and marked busy until it ends (session-layer-design.md §3.3).
fn mark_running(node: CrowVNode, handler: &str) -> CrowVNode {
    match node {
        CrowVNode::Element { tag, mut attrs, children } => {
            if attrs.get("data-crow-click").map(|h| h == handler).unwrap_or(false) {
                attrs.insert("disabled".to_string(), String::new());
                attrs.insert("aria-busy".to_string(), "true".to_string());
            }
            let children = children.into_iter().map(|c| mark_running(c, handler)).collect();
            CrowVNode::Element { tag, attrs, children }
        }
        CrowVNode::Raw(html) => {
            let needle = format!("data-crow-click=\"{}\"", handler);
            CrowVNode::Raw(html.replace(&needle, &format!("{} disabled aria-busy=\"true\"", needle)))
        }
        text => text,
    }
}

/// Apply one message from the page if a render the session sent offered
/// it: a bound input's value, kept in the variable's shape, or a handler
/// run with the arguments the render froze. Returns whether state moved; a
/// refused event is logged and changes nothing.
#[cfg(test)]
fn apply_event<H>(state: &mut CrowLiveState, text: &str, offered: &[&Offers], handle: &H) -> (bool, Option<u64>)
where
    H: Fn(&mut CrowLiveState, &str, Option<&str>),
{
    let (event, value, seq) = parse_event(text);
    if !offered.iter().any(|o| o.allows(&event, value.as_deref())) {
        eprintln!("crow live: refused '{}' — the page did not offer it", event);
        return (false, seq);
    }
    if apply_field(state, &event, value.as_deref()) {
        return (true, seq);
    }
    let copy = run_handler(state.working_copy(), &event, value.as_deref(), handle);
    if let Some(reason) = state.commit(copy) {
        eprintln!("crow live: {}", reason);
    }
    (true, seq)
}

impl CrowLiveState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            versions: HashMap::new(),
            tick: 0,
            derived: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            sources: Vec::new(),
            later_tx: None,
            session_id: 0,
            mode: std::sync::atomic::AtomicU8::new(MODE_FRESH),
            handle: None,
            waker: None,
            tables_written: std::sync::Mutex::new(std::collections::HashSet::new()),
            view_stale: std::sync::Mutex::new(Vec::new()),
            copy: false,
            local_tables: std::sync::Mutex::new(HashMap::new()),
            stopped: None,
            held: Vec::new(),
            notice: None,
            notice_read: std::sync::atomic::AtomicBool::new(false),
            base_versions: HashMap::new(),
            asking: None,
            signing: None,
            tables_drawn: std::sync::atomic::AtomicUsize::new(0),
            downloads: Vec::new(),
        }
    }

    /// A table this program wrote: derivations over it, in every session,
    /// are stale. This session's are part of the current event's answer;
    /// every other session whose page read one is woken to catch up.
    pub fn note_table_write(&self, table: &str) {
        let v = next_version();
        self.tables_written.lock().unwrap_or_else(|p| p.into_inner()).insert(table.to_string());
        if self.copy {
            // the handler sees its own write at once; everyone else when it commits
            self.local_tables.lock().unwrap_or_else(|p| p.into_inner()).insert(table.to_string(), v);
        } else {
            publish_table(table, v, self.session_id);
        }
    }

    /// A working copy for a handler: the session's facts to write, its
    /// derivation cache shared, its writes kept until `commit`.
    pub fn working_copy(&self) -> CrowLiveState {
        let mut copy = self.snapshot();
        copy.copy = true;
        copy.sources = self.sources.clone();
        copy.later_tx = self.later_tx.clone();
        copy.waker = None;
        copy.base_versions = self.versions.clone();
        copy
    }

    /// The handler ended: its writes land as one — its facts, the tables it
    /// wrote (every other reader of them is woken now), then its held
    /// effects in program order. A stopped handler lands nothing; its reason
    /// becomes the session's notice, which is returned.
    pub fn commit(&mut self, copy: CrowLiveState) -> Option<String> {
        let tables: Vec<(String, u64)> = copy.local_tables.lock().unwrap_or_else(|p| p.into_inner()).drain().collect();
        if let Some(reason) = copy.stopped {
            // its transactions rolled back, but a store without them
            // (ClickHouse) kept what it was given: its readers look again
            for (t, v) in tables {
                publish_table(&t, v, self.session_id);
            }
            self.notice = Some((reason.clone(), crow_runtime::moments::now()));
            return Some(reason);
        }
        // the facts the handler changed land; the rest stay as they are now
        for (key, value) in copy.data {
            let version = copy.versions.get(&key).copied();
            if version.is_some() && version != copy.base_versions.get(&key).copied() {
                self.data.insert(key.clone(), value);
                self.versions.insert(key, version.unwrap_or_default());
            }
        }
        for (t, v) in tables {
            self.tables_written.lock().unwrap_or_else(|p| p.into_inner()).insert(t.clone());
            publish_table(&t, v, self.session_id);
        }
        for effect in copy.held {
            match effect {
                Held::Notice(text, at) => self.notice = Some((text, at)),
                Held::Later(name, f) => {
                    self.spawn_later(&name, f);
                }
                Held::Sign(record) => self.apply_sign(record),
                Held::Download(name, bytes) => self.downloads.push((name, bytes)),
            }
        }
        None
    }

    /// The page's signed-in reader: a map of strings, empty when nobody is.
    pub fn reader(&self) -> std::collections::BTreeMap<String, String> {
        match self.data.get(READER) {
            Some(serde_json::Value::Object(m)) => m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
            _ => std::collections::BTreeMap::new(),
        }
    }

    /// `sign_in[record]`: the reader, from when the handler commits. A record
    /// too big for its cookie, or an app with no secret to seal it, stops
    /// the handler.
    pub fn sign_in(&mut self, record: &std::collections::BTreeMap<String, String>) {
        if let Err(e) = crow_runtime::signin::check(record).and_then(|_| signin::secret().map(|_| ())) {
            self.stop(e);
            return;
        }
        if self.copy {
            self.held.push(Held::Sign(Some(record.clone())));
        } else {
            self.apply_sign(Some(record.clone()));
        }
    }

    /// `download[value, name]`: hand the reader a file when the handler
    /// commits (the modes turn the value into its bytes: a frame's CSV, a
    /// string, a picked file's contents).
    pub fn download(&mut self, name: &str, bytes: Vec<u8>) {
        if self.copy {
            self.held.push(Held::Download(name.to_string(), bytes));
        } else {
            self.downloads.push((name.to_string(), bytes));
        }
    }

    /// `sign_out[]`: nobody, from when the handler commits — and the sign-in
    /// ended everywhere it is held.
    pub fn sign_out(&mut self) {
        if self.copy {
            self.held.push(Held::Sign(None));
        } else {
            self.apply_sign(None);
        }
    }

    fn apply_sign(&mut self, record: Option<std::collections::BTreeMap<String, String>>) {
        let old = self.data.get(READER_SID).and_then(|v| v.as_str()).unwrap_or("").to_string();
        // a sign-in replaces the one before it, which ends — in every session
        // that holds it, woken now to find it gone
        if !old.is_empty() {
            signin::end(&old);
            wake_holders(&old, self.session_id);
        }
        match record {
            Some(r) => {
                let sid = crow_runtime::signin::random_hex(16).unwrap_or_default();
                match signin::seal(&r, &sid) {
                    Ok(cookie) => self.signing = Some(Some(cookie)),
                    Err(e) => eprintln!("crow live: the sign-in holds for this session only — its cookie could not be sealed: {}", e),
                }
                self.set_reader(r, sid);
            }
            None => {
                self.signing = Some(None);
                self.set_reader(std::collections::BTreeMap::new(), String::new());
            }
        }
    }

    /// `table[frame, page = n]` drawn: which paged table of this render it
    /// is, and the page the session is on (the writer clamps it to the
    /// frame).
    pub fn table_page(&self, size: usize, _rows: usize) -> crow_runtime::layout::Paging {
        let index = self.tables_drawn.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let at = self.table_pages().get(&index).copied().unwrap_or(0);
        crow_runtime::layout::Paging { size, index, at }
    }

    /// The page each paged table is on, by its place among the render's.
    pub fn table_pages(&self) -> HashMap<usize, usize> {
        match self.data.get(PAGES) {
            Some(serde_json::Value::Object(m)) => m
                .iter()
                .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_u64()? as usize)))
                .collect(),
            _ => HashMap::new(),
        }
    }

    /// A page's pager pressed: the table shows that page from now on.
    fn turn_page(&mut self, index: usize, at: usize) {
        let mut pages = match self.data.get(PAGES) {
            Some(serde_json::Value::Object(m)) => m.clone(),
            _ => serde_json::Map::new(),
        };
        pages.insert(index.to_string(), serde_json::json!(at));
        self.set(PAGES, serde_json::Value::Object(pages));
    }

    /// The reader this session holds, and the sign-in it came from.
    pub fn set_reader(&mut self, record: std::collections::BTreeMap<String, String>, sid: String) {
        if let Some(h) = &self.handle {
            *h.reader_sid.lock().unwrap_or_else(|p| p.into_inner()) = sid.clone();
        }
        self.set(READER, serde_json::json!(record));
        self.set(READER_SID, serde_json::json!(sid));
    }

    /// A sign-in signed out elsewhere: this session's reader is nobody now.
    /// Whether it was.
    fn drop_ended_sign_in(&mut self) -> bool {
        let sid = self.data.get(READER_SID).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if signin::is_ended(&sid) {
            self.set_reader(std::collections::BTreeMap::new(), String::new());
            return true;
        }
        false
    }

    /// A handler that failed outright (a key its map lacks, a panic in the
    /// runtime) stopped: nothing it did landed, and the reader is told why,
    /// as a stop is told — the interpreter's page says the same.
    fn stopped_by(&mut self, handler: &str, why: &str) {
        let reason = format!("{} stopped: {}", handler, why);
        eprintln!("crow live: {}", reason);
        self.notice = Some((reason, crow_runtime::moments::now()));
    }

    /// The handler stops: nothing it did lands (core-design.md §2).
    pub fn stop(&mut self, reason: String) {
        if self.stopped.is_none() {
            self.stopped = Some(reason);
        }
    }

    /// Whether the handler has stopped — a helper that stopped stops its caller.
    pub fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }

    /// `notice[text]`: tell the reader, when the handler commits.
    pub fn notice(&mut self, text: String) {
        let at = crow_runtime::moments::now();
        if self.copy {
            self.held.push(Held::Notice(text, at));
        } else {
            self.notice = Some((text, at));
        }
    }

    /// `notice` read by a view: the session's latest notice and the moment it
    /// landed (empty text and 0.0 before the first).
    pub fn notice_record(&self) -> (String, f64) {
        self.notice_read.store(true, std::sync::atomic::Ordering::SeqCst);
        self.notice.clone().unwrap_or((String::new(), 0.0))
    }

    /// The same session for a background recompute: its facts as they are
    /// now, its cache shared, so what the recompute stores the page reads.
    fn snapshot(&self) -> CrowLiveState {
        CrowLiveState {
            data: self.data.clone(),
            versions: self.versions.clone(),
            tick: self.tick,
            derived: self.derived.clone(),
            sources: Vec::new(),
            later_tx: None,
            session_id: self.session_id,
            mode: std::sync::atomic::AtomicU8::new(MODE_FRESH),
            handle: None,
            waker: self.handle.as_ref().map(std::sync::Arc::downgrade),
            tables_written: std::sync::Mutex::new(std::collections::HashSet::new()),
            view_stale: std::sync::Mutex::new(Vec::new()),
            copy: false,
            local_tables: std::sync::Mutex::new(HashMap::new()),
            stopped: None,
            held: Vec::new(),
            notice: None,
            notice_read: std::sync::atomic::AtomicBool::new(false),
            base_versions: HashMap::new(),
            asking: None,
            signing: None,
            tables_drawn: std::sync::atomic::AtomicUsize::new(0),
            downloads: Vec::new(),
        }
    }

    fn rendering(&self) -> bool {
        self.mode.load(std::sync::atomic::Ordering::SeqCst) == MODE_RENDER
    }

    /// A render begins: reads go ahead with what they have, and what they
    /// read is recorded for the wakes.
    pub fn begin_render(&self) {
        self.mode.store(MODE_RENDER, std::sync::atomic::Ordering::SeqCst);
        self.tables_drawn.store(0, std::sync::atomic::Ordering::SeqCst);
        self.notice_read.store(false, std::sync::atomic::Ordering::SeqCst);
        self.view_stale.lock().unwrap_or_else(|p| p.into_inner()).clear();
        if let Some(h) = &self.handle {
            let mut reads = h.reads.lock().unwrap_or_else(|p| p.into_inner());
            reads.derives.clear();
            reads.tables.clear();
        }
    }

    /// The render is done: the event it answered is over, and the next
    /// read is a handler's or a recompute's.
    pub fn end_render(&self) {
        self.mode.store(MODE_FRESH, std::sync::atomic::Ordering::SeqCst);
        self.tables_written.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// Render `render` against this session as a page render does.
    pub fn render_with<R: Fn(&CrowLiveState) -> CrowVNode>(&self, render: &R) -> CrowVNode {
        // a render is one event: every view and derivation in it reads one moment
        crow_runtime::moments::at(crow_runtime::moments::clock(), || {
            self.begin_render();
            let vnode = render(self);
            self.end_render();
            // a page that shows no notice of its own gets the runtime's toast
            let vnode = match &self.notice {
                Some((text, at)) if !self.notice_read.load(std::sync::atomic::Ordering::SeqCst)
                    && !text.is_empty()
                    && crow_runtime::moments::now() - at < NOTICE_SECS => with_toast(vnode, text),
                _ => vnode,
            };
            // a question a click asked is part of the page until answered
            match &self.asking {
                Some((_, _, text)) => with_dialog(vnode, text),
                None => vnode,
            }
        })
    }

    fn note_read(&self, spec: &'static DeriveSpec) {
        if !self.rendering() {
            return;
        }
        if let Some(h) = &self.handle {
            let mut reads = h.reads.lock().unwrap_or_else(|p| p.into_inner());
            reads.derives.insert(spec.name);
            reads.tables.extend(spec.tables.iter().copied());
        }
    }

    /// The view being rendered read a value still being recomputed.
    fn mark_stale(&self) {
        if let Some(top) = self.view_stale.lock().unwrap_or_else(|p| p.into_inner()).last_mut() {
            *top = true;
        }
    }

    /// A view starts rendering; the depth it returns closes it.
    pub fn view_enter(&self) -> usize {
        let mut stack = self.view_stale.lock().unwrap_or_else(|p| p.into_inner());
        stack.push(false);
        stack.len() - 1
    }

    /// A view's result: marked `data-crow-stale` when the view read a value
    /// still being recomputed, so the page can show it is catching up.
    pub fn view_exit(&self, depth: usize, node: CrowVNode) -> CrowVNode {
        let stale = {
            let mut stack = self.view_stale.lock().unwrap_or_else(|p| p.into_inner());
            let s = stack.get(depth).copied().unwrap_or(false);
            stack.truncate(depth);
            s
        };
        if stale { node.attr("data-crow-stale", "") } else { node }
    }

    /// `running[x]` for a derivation: is a recompute of it in flight?
    pub fn derive_running(&self, name: &str) -> bool {
        let inflight = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        inflight.keys().any(|(holder, n)| *n == name && (*holder == 0 || *holder == self.session_id))
    }

    /// Run `refresh` for `spec` on a worker thread unless one is already
    /// computing it from the same inputs; one computing it from inputs that
    /// have since moved is stopped and replaced. A time tick alone replaces
    /// nothing, so a query slower than the tick still lands. Whoever reads
    /// the value is woken when it lands.
    fn schedule<T: Clone + Send + Sync + 'static>(&self, spec: &'static DeriveSpec, refresh: fn(&CrowLiveState) -> T, inputs: u64) {
        use std::sync::atomic::Ordering::SeqCst;
        let holder = if spec.shared { 0 } else { self.session_id };
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut inflight = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(job) = inflight.get(&(holder, spec.name)) {
                if job.inputs == inputs {
                    return;
                }
                job.flag.store(true, SeqCst);
            }
            inflight.insert((holder, spec.name), InFlightJob { flag: flag.clone(), inputs });
        }
        let snap = self.snapshot();
        // the recompute answers the render that asked for it, at its moment
        let moment = crow_runtime::moments::now();
        let job = move || {
            struct Done((u64, &'static str), std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    let mut inflight = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
                    if inflight.get(&self.0).map(|j| std::sync::Arc::ptr_eq(&j.flag, &self.1)).unwrap_or(false) {
                        inflight.remove(&self.0);
                    }
                }
            }
            let done = Done((holder, spec.name), flag.clone());
            crow_runtime::cancel::with_flag(flag.clone(), || {
                crow_runtime::moments::at(moment, || {
                    let _ = refresh(&snap);
                })
            });
            let stopped = flag.load(SeqCst);
            drop(done);
            // a stopped recompute stored nothing; its replacement wakes the readers
            if stopped {
                return;
            }
            if spec.shared {
                wake_sessions(u64::MAX, |reads| reads.derives.contains(spec.name));
            } else if let Some(h) = snap.waker.as_ref().and_then(|w| w.upgrade()) {
                h.wake.notify_one();
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn_blocking(job);
            }
            Err(_) => {
                std::thread::spawn(job);
            }
        }
    }

    /// A render is a tick: derivations that read the clock, or external
    /// sources at fresh = 0, recompute once per tick. The counter is the
    /// app's, so a shared derivation computed by another session's render
    /// counts as this tick's.
    pub fn next_tick(&mut self) {
        self.tick = TICK.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    }

    /// Where a derivation's slot lives: this session, or the app.
    fn cache_for(&self, spec: &DeriveSpec) -> &std::sync::Mutex<HashMap<String, DerivedSlot>> {
        if spec.shared { &SHARED_DERIVED } else { &*self.derived }
    }

    /// The version of a derivation wherever it lives — 0 before its first
    /// computation. Locks are taken one at a time, never nested.
    fn derive_version(&self, name: &str) -> u64 {
        if let Some(v) = self.derived.lock().unwrap_or_else(|p| p.into_inner()).get(name).map(|s| s.version) {
            return v;
        }
        SHARED_DERIVED.lock().unwrap_or_else(|p| p.into_inner()).get(name).map(|s| s.version).unwrap_or(0)
    }

    /// Read a derivation (derive-design.md §3.3): the cached value while
    /// every input it saw is unchanged, else `compute` runs and the value
    /// is cached with the versions it was computed from. Equal scalars
    /// keep their version (early cutoff). Inputs that are derivations must
    /// be refreshed by the caller first, so their versions are current.
    pub fn derive<T: Clone + Send + Sync + 'static>(
        &self,
        spec: &'static DeriveSpec,
        compute: impl Fn(&CrowLiveState) -> T,
        equal: impl Fn(&T, &T) -> bool,
    ) -> T {
        self.note_read(spec);
        let inputs = self.inputs_now(spec);
        if let Found::Fresh(v) = self.lookup::<T>(spec, &inputs) {
            return v;
        }
        self.compute_and_store(spec, inputs, compute, equal)
    }

    /// Read a derivation that reaches out — its read set includes a table or
    /// an external source (session-layer-design.md §3.4). A handler or a
    /// recompute gets the recomputed value. A render gets it only when the
    /// current event wrote a table the derivation reads; otherwise the
    /// render goes ahead with the last value — or `pending` before the
    /// first — while `refresh` recomputes it on a worker thread, and the
    /// page renders again when it lands.
    pub fn derive_bg<T: Clone + Send + Sync + 'static>(
        &self,
        spec: &'static DeriveSpec,
        compute: fn(&CrowLiveState) -> T,
        equal: fn(&T, &T) -> bool,
        pending: Option<fn() -> T>,
        refresh: fn(&CrowLiveState) -> T,
    ) -> T {
        self.note_read(spec);
        let inputs = self.inputs_now(spec);
        let found = self.lookup::<T>(spec, &inputs);
        if let Found::Fresh(v) = found {
            return v;
        }
        let own_write = {
            let written = self.tables_written.lock().unwrap_or_else(|p| p.into_inner());
            spec.tables.iter().any(|t| written.contains(*t))
        };
        if self.rendering() && !own_write {
            let waiting = match found {
                Found::Stale(v) => Some(v),
                _ => pending.map(|p| p()),
            };
            if let Some(v) = waiting {
                self.schedule(spec, refresh, inputs.signature());
                self.mark_stale();
                return v;
            }
        }
        self.compute_and_store(spec, inputs, compute, equal)
    }

    /// Where every input of `spec` stands now: facts in this session,
    /// tables app-wide, derivations wherever each one lives.
    fn inputs_now(&self, spec: &DeriveSpec) -> Inputs {
        let facts = spec.facts.iter()
            .map(|f| (f.to_string(), self.versions.get(*f).copied().unwrap_or(0))).collect();
        let tables = {
            // a handler's own writes first: it reads what it wrote
            let local = self.local_tables.lock().unwrap_or_else(|p| p.into_inner());
            let tv = TABLE_VERSIONS.lock().unwrap_or_else(|p| p.into_inner());
            spec.tables.iter().map(|t| (t.to_string(), local.get(*t).or_else(|| tv.get(*t)).copied().unwrap_or(0))).collect()
        };
        let derives = spec.derives.iter().filter(|d| **d != spec.name)
            .map(|d| (d.to_string(), self.derive_version(d))).collect();
        Inputs { facts, tables, derives, now: crow_runtime::moments::now() }
    }

    /// The cached value, and whether every input it saw is where it is now.
    fn lookup<T: Clone + 'static>(&self, spec: &DeriveSpec, inputs: &Inputs) -> Found<T> {
        // a shared slot computed during a later render than this session's
        // is as fresh as this tick
        let fresh_tick = |seen: u64| if spec.shared { seen >= self.tick } else { seen == self.tick };
        let same = |seen: &[(String, u64)], now: &[(String, u64)]| now.iter()
            .all(|(n, v)| seen.iter().find(|(m, _)| m == n).map(|(_, s)| s == v).unwrap_or(false));
        let cache = self.cache_for(spec).lock().unwrap_or_else(|p| p.into_inner());
        let Some(slot) = cache.get(spec.name) else { return Found::Missing };
        let Some(v) = slot.value.downcast_ref::<T>() else { return Found::Missing };
        let valid = same(&slot.facts_seen, &inputs.facts)
            && same(&slot.derives_seen, &inputs.derives)
            && same(&slot.tables_seen, &inputs.tables)
            && (!spec.now || fresh_tick(slot.tick_seen))
            && (!spec.external || if spec.fresh > 0.0 { inputs.now - slot.computed_at <= spec.fresh } else { fresh_tick(slot.tick_seen) });
        if valid { Found::Fresh(v.clone()) } else { Found::Stale(v.clone()) }
    }

    /// Compute the derivation and cache it with the inputs it saw. Equal
    /// scalars keep their version (early cutoff); a computation that
    /// started before the one already cached never replaces it.
    fn compute_and_store<T: Clone + Send + Sync + 'static>(
        &self,
        spec: &DeriveSpec,
        inputs: Inputs,
        compute: impl Fn(&CrowLiveState) -> T,
        equal: impl Fn(&T, &T) -> bool,
    ) -> T {
        let stamp = STAMP.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let value = compute(self);
        // a recompute stopped under its query answers nobody: its value
        // (the query's "cancelled") is never cached
        if crow_runtime::cancel::current_stopped() {
            return value;
        }
        let mut cache = self.cache_for(spec).lock().unwrap_or_else(|p| p.into_inner());
        if let Some(slot) = cache.get(spec.name) {
            if slot.stamp > stamp {
                return value;
            }
        }
        let (old_version, changed) = match cache.get(spec.name) {
            Some(slot) => (slot.version, !slot.value.downcast_ref::<T>().map(|old| equal(old, &value)).unwrap_or(false)),
            None => (0, true),
        };
        cache.insert(spec.name.to_string(), DerivedSlot {
            value: std::sync::Arc::new(value.clone()),
            version: if changed { old_version + 1 } else { old_version },
            facts_seen: inputs.facts,
            derives_seen: inputs.derives,
            tables_seen: inputs.tables,
            tick_seen: self.tick,
            computed_at: inputs.now,
            stamp,
        });
        value
    }

    /// Schedule a background fn for this session (skip-if-running). The fn
    /// runs against a snapshot of state on its own thread; the keys it
    /// changed are sent back to the live loop, which applies them and
    /// re-renders. State never mutates concurrently.
    pub fn spawn_later(&mut self, name: &str, f: fn(&mut CrowLiveState)) -> bool {
        // a handler's schedule waits for its commit, and a stop drops it
        if self.copy {
            if !self.later_running(name) {
                self.held.push(Held::Later(name.to_string(), f));
            }
            return true;
        }
        let key = format!("{}:{}", self.session_id, name);
        {
            let mut reg = later_registry().lock().unwrap();
            if !reg.insert(key.clone()) {
                return false;
            }
        }
        let mut snapshot = self.clone();
        let tx = self.later_tx.clone();
        std::thread::spawn(move || {
            struct InFlight(String);
            impl Drop for InFlight {
                fn drop(&mut self) {
                    later_registry().lock().unwrap().remove(&self.0);
                }
            }
            let _guard = InFlight(key);
            let before = snapshot.data.clone();
            // a scheduled function is one event and one transaction of its own
            crow_runtime::txn::begin();
            crow_runtime::moments::at(crow_runtime::moments::clock(), || f(&mut snapshot));
            let _ = crow_runtime::txn::finish(!snapshot.is_stopped());
            let delta: Vec<(String, serde_json::Value)> = snapshot
                .data
                .iter()
                .filter(|(k, v)| before.get(*k) != Some(*v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if let Some(tx) = tx {
                let _ = tx.send(delta);
            }
        });
        true
    }

    /// Is a later[] schedule for this session still in flight?
    pub fn later_running(&self, name: &str) -> bool {
        later_registry()
            .lock()
            .unwrap()
            .contains(&format!("{}:{}", self.session_id, name))
    }

    /// Get a value from state.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }

    /// Set a value in state. The key's version bumps only when the value
    /// changed, so a derivation recomputes when a fact it reads actually
    /// moved — a handler that rewrites a fact to the same value costs
    /// nothing downstream.
    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        let changed = self.data.get(key) != Some(&value);
        self.data.insert(key.to_string(), value);
        if changed {
            self.versions.insert(key.to_string(), next_version());
        }
    }

    /// Get an i64 from state, with default.
    pub fn get_i64(&self, key: &str, default: i64) -> i64 {
        self.data.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
    }

    /// Get a string from state, with default.
    pub fn get_str(&self, key: &str, default: &str) -> String {
        self.data.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| default.to_string())
    }

    /// Get an f64 from state, with default.
    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        self.data.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    /// Get a bool from state, with default.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.data.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    /// Register a one-time fetch: `fetch[url]` → stores result under `key`.
    pub fn register_fetch(&mut self, key: &str, url: &str) {
        self.sources.push(DataSource {
            url: url.to_string(),
            key: key.to_string(),
            interval_secs: None,
        });
    }

    /// Register a polling fetch: `poll[url, secs]` → stores result under `key`, refetches every `secs`.
    pub fn register_poll(&mut self, key: &str, url: &str, interval_secs: u64) {
        self.sources.push(DataSource {
            url: url.to_string(),
            key: key.to_string(),
            interval_secs: Some(interval_secs),
        });
    }
}

impl Clone for CrowLiveState {
    /// A copy with a cache of its own: a new session from the app's initial
    /// state, or the snapshot a `later` function runs against.
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            versions: self.versions.clone(),
            tick: self.tick,
            derived: std::sync::Arc::new(std::sync::Mutex::new(self.derived.lock().unwrap_or_else(|p| p.into_inner()).clone())),
            sources: self.sources.clone(),
            later_tx: self.later_tx.clone(),
            session_id: self.session_id,
            mode: std::sync::atomic::AtomicU8::new(MODE_FRESH),
            handle: self.handle.clone(),
            waker: self.waker.clone(),
            tables_written: std::sync::Mutex::new(std::collections::HashSet::new()),
            view_stale: std::sync::Mutex::new(Vec::new()),
            copy: false,
            local_tables: std::sync::Mutex::new(HashMap::new()),
            stopped: None,
            held: Vec::new(),
            notice: None,
            notice_read: std::sync::atomic::AtomicBool::new(false),
            base_versions: HashMap::new(),
            asking: None,
            signing: None,
            tables_drawn: std::sync::atomic::AtomicUsize::new(0),
            downloads: Vec::new(),
        }
    }
}

impl Default for CrowLiveState {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate the LiveView client-side JavaScript runtime.
/// Handles the WebSocket connection, applying a render's changes (or the
/// whole page), and event delegation.
pub fn live_js(ws_path: &str) -> String {
    live_js_for(ws_path, "")
}

/// The client for a page whose session waits under `page_token`: the tab
/// keeps the token of its session (`sessionStorage`), replaced at every
/// connect, and brings it back when it reconnects or reloads.
pub fn live_js_for(ws_path: &str, page_token: &str) -> String {
    format!(r#"<script>
(function() {{
  var ws, reconDelay = 100, seq = 0, busy = [];
  var pageToken = '{page_token}', key = 'crow-session:' + location.pathname;
  function tabToken() {{
    try {{ return sessionStorage.getItem(key) || ''; }} catch (e) {{ return ''; }}
  }}
  function wsUrl() {{
    var q = [], t = tabToken();
    if (t) q.push('t=' + t);
    if (pageToken) q.push('p=' + pageToken);
    var path = '{ws_path}';
    return location.origin.replace('http','ws') + path + (q.length ? (path.indexOf('?') >= 0 ? '&' : '?') + q.join('&') : '');
  }}

  // A click is marked busy the moment it is made and stays marked until
  // the server's answer to it arrives: every click carries a number, every
  // answer the last number it applied.
  function answered(n) {{
    busy = busy.filter(function(b) {{
      if (b.n <= n) {{ b.el.removeAttribute('data-crow-busy'); return false; }}
      return true;
    }});
  }}
  function remark() {{
    for (var i = 0; i < busy.length; i++) busy[i].el.setAttribute('data-crow-busy', '');
  }}

  // While the socket is down the page says so and takes no input: a click
  // it could not deliver is never silently dropped. On reconnect the server
  // sends the session's render first, so the page never keeps showing what
  // the server no longer holds.
  function offline(on) {{
    var r = document.documentElement;
    if (on) r.setAttribute('data-crow-offline', ''); else r.removeAttribute('data-crow-offline');
  }}
  function send(msg) {{
    if (ws && ws.readyState === 1) {{ ws.send(JSON.stringify(msg)); return true; }}
    offline(true);
    return false;
  }}

  function connect() {{
    ws = new WebSocket(wsUrl());
    ws.onopen = function() {{ reconDelay = 100; offline(false); answered(Infinity); }};
    ws.onclose = function() {{
      offline(true);
      setTimeout(connect, Math.min(reconDelay, 5000));
      reconDelay *= 2;
    }};
    ws.onmessage = function(e) {{
      var msg = JSON.parse(e.data);
      if (msg.type === 'session') {{
        try {{ sessionStorage.setItem(key, msg.token); }} catch (err) {{}}
        return;
      }}
      if (msg.type === 'upload') {{
        // the ticket for a file picked a moment ago: send it by it
        var f = picked[msg.field];
        delete picked[msg.field];
        if (f) fetch(msg.url, {{method: 'POST', body: f, credentials: 'same-origin'}});
        return;
      }}
      if (msg.type === 'download') {{
        // a file a handler handed the reader, saved under its name
        var link = document.createElement('a');
        link.href = msg.url;
        link.download = msg.name;
        document.body.appendChild(link);
        link.click();
        link.remove();
        return;
      }}
      if (msg.type === 'sign') {{
        // the cookie a sign-in (or sign-out) set, traded for its ticket
        fetch(msg.url, {{method: 'POST', credentials: 'same-origin'}});
        return;
      }}
      if (msg.type === 'url') {{
        var target = location.pathname + (msg.query ? '?' + msg.query : '') + location.hash;
        if (target !== location.pathname + location.search + location.hash) history.replaceState(null, '', target);
        return;
      }}
      if (msg.type === 'html') {{
        // the whole page, morphed into place
        var root = document.getElementById('crow-root');
        if (root) {{
          var tmp = document.createElement('div');
          tmp.innerHTML = msg.html;
          morphChildren(root, tmp);
        }}
        resyncing = false;
      }} else if (msg.type === 'patches' && !resyncing) {{
        // what changed; a change that does not fit this page, or would take
        // away the field being typed in, asks for the whole page instead
        var ok = !msg.patches.some(disturbs);
        for (var i = 0; ok && i < msg.patches.length; i++) ok = applyChange(msg.patches[i]);
        if (!ok) {{ resyncing = true; send({{event: '__resync'}}); }}
      }}
      if (msg.ack) answered(msg.ack);
      remark();
      widgets();
    }};
  }}

  // Morph the live DOM into the server's render in place: nodes keep their
  // identity, so focus, caret and scroll survive a render. Children with an
  // id are matched by id — a keyed row moves instead of being rewritten —
  // and the rest by position.
  function morphChildren(target, source) {{
    var focused = document.activeElement;
    var focusKey = focused ? (focused.getAttribute('data-crow-input') || focused.getAttribute('data-crow-change')) : null;
    var draft = focusKey && focused.type !== 'radio' && focused.type !== 'checkbox'
      ? {{ value: focused.value, start: focused.selectionStart, end: focused.selectionEnd }} : null;
    morphKids(target, source);
    // the input being typed in keeps its draft, even when its node moved;
    // every other bound input took the value the server rendered
    if (focusKey) {{
      var el = document.activeElement;
      var mine = el && (el.getAttribute('data-crow-input') === focusKey || el.getAttribute('data-crow-change') === focusKey);
      if (!mine) {{
        el = target.querySelector('[data-crow-input="' + focusKey + '"],[data-crow-change="' + focusKey + '"]');
        if (el) el.focus();
      }}
      if (el && draft) {{
        if (el.value !== draft.value) el.value = draft.value;
        try {{ el.setSelectionRange(draft.start, draft.end); }} catch(e) {{}}
      }}
    }}
  }}
  function sameKind(a, b) {{
    return a.nodeType === b.nodeType && (a.nodeType !== 1 || a.tagName === b.tagName);
  }}
  function morphKids(from, to) {{
    var byId = {{}}, wanted = {{}}, c;
    for (c = to.firstChild; c; c = c.nextSibling) if (c.nodeType === 1 && c.id) wanted[c.id] = true;
    for (c = from.firstChild; c; c = c.nextSibling) if (c.nodeType === 1 && c.id) byId[c.id] = c;
    var cur = from.firstChild;
    for (var n = to.firstChild; n; n = n.nextSibling) {{
      // a keyed child the new render dropped goes now, so its unkeyed
      // neighbours still line up by position
      while (cur && cur.nodeType === 1 && cur.id && !wanted[cur.id]) {{
        var gone = cur; cur = cur.nextSibling; from.removeChild(gone);
      }}
      var match = null;
      if (n.nodeType === 1 && n.id) {{
        if (byId[n.id]) {{ match = byId[n.id]; delete byId[n.id]; }}
      }} else if (cur && !(cur.nodeType === 1 && cur.id) && sameKind(cur, n)) {{
        match = cur;
      }}
      if (match) {{
        if (match !== cur) from.insertBefore(match, cur);
        // taken before the morph, which may swap the node (a drawing)
        var after = match.nextSibling;
        morphNode(match, n);
        cur = after;
      }} else {{
        from.insertBefore(n.cloneNode(true), cur);
      }}
    }}
    while (cur) {{ var rest = cur.nextSibling; from.removeChild(cur); cur = rest; }}
  }}
  function morphNode(from, to) {{
    if (from.nodeType !== 1) {{
      if (from.nodeValue !== to.nodeValue) from.nodeValue = to.nodeValue;
      return;
    }}
    // a widget's insides are its module's: only its attributes follow
    if (from.hasAttribute('data-crow-widget') && to.getAttribute('data-crow-widget') === from.getAttribute('data-crow-widget')) {{
      var j, name;
      for (j = from.attributes.length - 1; j >= 0; j--) {{
        name = from.attributes[j].name;
        if (!to.hasAttribute(name)) from.removeAttribute(name);
      }}
      for (j = 0; j < to.attributes.length; j++) {{
        if (from.getAttribute(to.attributes[j].name) !== to.attributes[j].value) from.setAttribute(to.attributes[j].name, to.attributes[j].value);
      }}
      return;
    }}
    // a chart is one drawing: swapped whole when it changed
    if (from.tagName === 'svg' || from.tagName === 'SVG') {{
      if (from.outerHTML !== to.outerHTML) from.parentNode.replaceChild(to.cloneNode(true), from);
      return;
    }}
    var i, a;
    for (i = from.attributes.length - 1; i >= 0; i--) {{
      a = from.attributes[i].name;
      if (!to.hasAttribute(a)) from.removeAttribute(a);
    }}
    for (i = 0; i < to.attributes.length; i++) {{
      a = to.attributes[i];
      if (from.getAttribute(a.name) !== a.value) from.setAttribute(a.name, a.value);
    }}
    // form controls carry live state the attributes do not reach; the
    // control being typed in is left to morphChildren
    var tag = from.tagName;
    if (tag === 'INPUT') {{
      if (from.type === 'checkbox' || from.type === 'radio') {{ if (from.checked !== to.checked) from.checked = to.checked; }}
      else if (from !== document.activeElement && from.value !== to.value) from.value = to.value;
    }} else if (tag === 'TEXTAREA' || tag === 'SELECT') {{
      if (from !== document.activeElement && from.value !== to.value) from.value = to.value;
    }}
    morphKids(from, to);
  }}

  // A change the server sent, applied where its path says; false when the
  // page does not have that shape (the server then sends the whole page)
  var SVG = 'http://www.w3.org/2000/svg', resyncing = false;
  function fragment(html, parent) {{
    if (parent && parent.namespaceURI === SVG && parent.localName !== 'foreignObject') {{
      var holder = document.createElementNS(SVG, 'svg');
      holder.innerHTML = html;
      return holder.firstChild;
    }}
    // a template reads a row, a cell or an option in place
    var t = document.createElement('template');
    t.innerHTML = html;
    return t.content.firstChild;
  }}
  function disturbs(p) {{
    var active = document.activeElement;
    if (!active || active === document.body || (p.op !== 'replace' && p.op !== 'remove')) return false;
    var node = nodeAt(p.path);
    if (node && p.op === 'remove') node = node.childNodes[p.index];
    return !!node && node.contains(active);
  }}
  // a field shows what the server rendered, unless the reader is typing in it
  function follow(el) {{
    if (!el || el === document.activeElement) return;
    if (el.tagName === 'TEXTAREA') el.value = el.defaultValue;
    else if (el.tagName === 'OPTION' && el.parentNode && el.parentNode.tagName === 'SELECT' && el.parentNode !== document.activeElement) el.selected = el.defaultSelected;
    else if (el.tagName === 'INPUT') {{
      if (el.type === 'checkbox' || el.type === 'radio') el.checked = el.defaultChecked;
      else el.value = el.defaultValue;
    }}
  }}
  function applyChange(p) {{
    var node = nodeAt(p.path), k, j, fresh;
    if (!node) return false;
    if (p.tag && (node.nodeType !== 1 || node.nodeName.toLowerCase() !== p.tag.toLowerCase())) return false;
    if (p['in'] && (!node.parentNode || node.parentNode.nodeName.toLowerCase() !== p['in'].toLowerCase())) return false;
    switch (p.op) {{
      case 'text':
        if (node.nodeType !== 3) return false;
        node.nodeValue = p.text;
        follow(node.parentNode);
        return true;
      case 'attrs':
        for (k in p.set) node.setAttribute(k, p.set[k]);
        for (j = 0; j < p.remove.length; j++) node.removeAttribute(p.remove[j]);
        follow(node);
        return true;
      case 'replace':
        if (!node.parentNode) return false;
        fresh = fragment(p.html, node.parentNode);
        if (!fresh) return false;
        node.parentNode.replaceChild(fresh, node);
        follow(fresh.parentNode);
        return true;
      case 'insert':
        fresh = fragment(p.html, node);
        if (!fresh || p.index > node.childNodes.length) return false;
        node.insertBefore(fresh, node.childNodes[p.index] || null);
        return true;
      case 'remove':
        if (!node.childNodes[p.index]) return false;
        node.removeChild(node.childNodes[p.index]);
        return true;
    }}
    return false;
  }}

  // Navigate to a DOM node by path (array of child indices)
  function nodeAt(path) {{
    var node = document.getElementById('crow-root');
    for (var i = 0; node && i < path.length; i++) {{
      node = node.childNodes[path[i]];
    }}
    return node;
  }}

  // Event delegation: click
  document.addEventListener('click', function(e) {{
    var t = e.target;
    while (t && t !== document) {{
      var ev = t.getAttribute('data-crow-click');
      if (ev) {{
        e.preventDefault();
        var ar = t.getAttribute('data-crow-args');
        var n = ++seq;
        if (send(ar ? {{event: ev, value: ar, seq: n}} : {{event: ev, seq: n}})) {{
          t.setAttribute('data-crow-busy', '');
          busy.push({{n: n, el: t}});
        }}
        return;
      }}
      t = t.parentElement;
    }}
  }});

  // Event delegation: input
  document.addEventListener('input', function(e) {{
    var ev = e.target.getAttribute('data-crow-input');
    if (ev) {{
      var v = e.target.type === 'checkbox' ? (e.target.checked ? 'true' : 'false') : e.target.value;
      send({{event: ev, value: v}});
    }}
  }});

  // Widgets: the program's own modules draw them (widgets/<name>.js). A
  // module is loaded once; mount(el, props, send) draws a widget, and
  // update(el, props) is called when a render changes its props. What it
  // sends reaches the handler the element names for the event.
  var widgetBase = '/_widget' + location.pathname;
  function widgets() {{
    var els = document.querySelectorAll('[data-crow-widget]');
    for (var i = 0; i < els.length; i++) (function(el) {{
      var props = el.getAttribute('data-crow-props');
      var w = el.__crow;
      if (!w) {{
        el.__crow = w = {{props: props, mod: null}};
        var sendFor = function(ev, value) {{
          var on = JSON.parse(el.getAttribute('data-crow-on') || '{{}}');
          if (on[ev]) send({{event: on[ev], value: JSON.stringify([value])}});
        }};
        import(widgetBase + '?m=' + encodeURIComponent(el.getAttribute('data-crow-widget'))).then(function(mod) {{
          w.mod = mod;
          mod.mount(el, JSON.parse(w.props), sendFor);
        }});
      }} else if (w.props !== props) {{
        w.props = props;
        if (w.mod && w.mod.update) w.mod.update(el, JSON.parse(props));
      }}
    }})(els[i]);
  }}

  // A file field: the file travels beside the socket — the page asks for a
  // ticket with what it is, then sends the bytes by it
  var picked = {{}};
  document.addEventListener('change', function(e) {{
    var field = e.target.getAttribute && e.target.getAttribute('data-crow-upload');
    if (!field || !e.target.files || !e.target.files[0]) return;
    var f = e.target.files[0];
    picked[field] = f;
    send({{event: '__upload', value: JSON.stringify([field, f.name, f.type || 'application/octet-stream', f.size])}});
  }});

  // Event delegation: change (select dropdowns)
  document.addEventListener('change', function(e) {{
    var ev = e.target.getAttribute('data-crow-change');
    if (ev) {{
      send({{event: ev, value: e.target.value}});
    }}
  }});

  // Event delegation: submit
  document.addEventListener('submit', function(e) {{
    var ev = e.target.getAttribute('data-crow-submit');
    if (ev) {{
      e.preventDefault();
      var data = {{}};
      new FormData(e.target).forEach(function(v, k) {{ data[k] = v; }});
      send({{event: ev, form: data}});
    }}
  }});

  connect();
  widgets();
}})();
</script>"#, ws_path = ws_path, page_token = page_token)
}

/// The default stylesheet — Crow's whole styling story ships here.
/// Layer 0: elements look right with zero classes. Layer 1: the blessed
/// class vocabulary (card stats controls row primary up down stale).
/// Tokens are CSS custom properties; a project's css/app.css may
/// override just the tokens, or everything (see style-module-design.md).
pub use crow_runtime::layout::DEFAULT_CSS;

/// A project's own css/app.css, inlined after the defaults when present —
/// override tokens for a theme, or anything for a full custom look.
fn project_css() -> String {
    match std::fs::read_to_string("css/app.css") {
        Ok(css) => format!("<style>\n{}\n</style>", css),
        Err(_) => String::new(),
    }
}

/// Generate a full HTML page shell wrapping the initial content.
fn page_shell(body_html: &str, js: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html{look}>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Crow App</title>
<style>{css}</style>
<style>{offline}{live}</style>
{project}
</head>
<body>
<div id="crow-root">{body}</div>
{js}
</body>
</html>"#,
        look = look_attrs(),
        css = format!("{}\n{}", DEFAULT_CSS, crow_runtime::layout::RESPONSIVE_CSS),
        project = project_css(),
        offline = OFFLINE_CSS,
        live = LIVE_CSS,
        body = body_html,
        js = js
    )
}

/// A click the server has not answered yet, and a view still catching up
/// with a value being recomputed — dimmed after a moment, so a fast answer
/// never flashes.
const LIVE_CSS: &str = "[data-crow-busy]{cursor:progress;opacity:.75}\
.crow-confirm{position:fixed;inset:0;z-index:2147483645;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.45)}\
.crow-confirm-box{max-width:min(440px,calc(100vw - 32px));padding:18px 20px;border-radius:10px;background:var(--crow-surface,#fff);\
color:var(--crow-text,#111);border:1px solid var(--crow-border,#ccc);box-shadow:0 8px 32px rgba(0,0,0,.3)}\
.crow-confirm-actions{display:flex;gap:8px;justify-content:flex-end;margin-top:14px}\
[data-crow-stale]{opacity:.55;transition:opacity .2s ease .3s}\
.crow-notice{position:fixed;right:16px;bottom:16px;z-index:2147483646;max-width:min(420px,calc(100vw - 32px));\
padding:10px 14px;border-radius:8px;font:500 14px/1.4 system-ui,sans-serif;background:var(--crow-surface,#222);\
color:var(--crow-text,#eee);border:1px solid var(--crow-border,#444);box-shadow:0 4px 16px rgba(0,0,0,.25);\
animation:crow-notice 4s ease forwards}\
@keyframes crow-notice{0%,85%{opacity:1}100%{opacity:0;visibility:hidden}}";

/// A page whose socket is down: dimmed, taking no input, and saying why.
const OFFLINE_CSS: &str = "html[data-crow-offline] #crow-root{opacity:.5;pointer-events:none;transition:opacity .2s}\
html[data-crow-offline]::after{content:'reconnecting\u{2026}';position:fixed;top:12px;right:12px;z-index:2147483647;\
padding:6px 12px;border-radius:999px;font:500 13px/1.3 system-ui,sans-serif;background:var(--crow-surface,#222);\
color:var(--crow-text,#eee);border:1px solid var(--crow-border,#444)}";


/// A LiveView endpoint with separate render and handle functions.
///
/// - `R`: render function `Fn(&CrowLiveState) -> CrowVNode`
/// - `H`: event handler `Fn(&mut CrowLiveState, &str, Option<&str>)` (event, optional value)
pub struct CrowLiveView<R, H>
where
    R: Fn(&CrowLiveState) -> CrowVNode + Send + Sync + 'static,
    H: Fn(&mut CrowLiveState, &str, Option<&str>) + Send + Sync + 'static,
{
    pub initial_state: CrowLiveState,
    pub render: R,
    pub handle: H,
    pub ws_path: String,
    /// `page[…, url = …]`: the facts the page's address carries, as (the
    /// query's name, the fact)
    pub url: Vec<(String, String)>,
}

/// Percent-encode a query value: letters, digits and `-_.~` stay.
fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// The page's address for a session: its `url` facts as a query string.
fn url_query(state: &CrowLiveState, url: &[(String, String)]) -> String {
    url.iter()
        .filter_map(|(name, fact)| {
            let text = match state.get(fact)? {
                serde_json::Value::String(t) => t.clone(),
                serde_json::Value::Null => return None,
                other => other.to_string(),
            };
            Some(format!("{}={}", url_encode(name), url_encode(&text)))
        })
        .collect::<Vec<_>>()
        .join("&")
}

impl<R, H> CrowLiveView<R, H>
where
    R: Fn(&CrowLiveState) -> CrowVNode + Send + Sync + Clone + 'static,
    H: Fn(&mut CrowLiveState, &str, Option<&str>) + Send + Sync + Clone + 'static,
{
    pub fn new(ws_path: &str, initial: CrowLiveState, render: R, handle: H) -> Self {
        Self {
            initial_state: initial,
            render,
            handle,
            ws_path: ws_path.to_string(),
            url: Vec::new(),
        }
    }

    /// `page[…, url = …]`: these facts come from the page's query string and
    /// the address follows them (query name, fact).
    pub fn with_url(mut self, pairs: Vec<(&str, &str)>) -> Self {
        self.url = pairs.into_iter().map(|(n, f)| (n.to_string(), f.to_string())).collect();
        self
    }

    /// Build an Axum router with:
    /// - GET `page_path` → initial HTML page with embedded JS
    /// - WS  `ws_path`   → event loop: receive events, mutate state, send diffs
    pub fn into_router(self, page_path: &str) -> axum::Router {
        if let Err(e) = signin::check_secret_env() {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
        let ws_path = self.ws_path.clone();
        let render = std::sync::Arc::new(self.render);
        let handle = std::sync::Arc::new(self.handle);
        // every session holds its reader (nobody, to begin with) with its
        // facts, so a kept session keeps who it was
        let mut initial_state = self.initial_state;
        if !initial_state.data.contains_key(READER) {
            initial_state.set(READER, serde_json::json!({}));
        }
        if !initial_state.data.contains_key(READER_SID) {
            initial_state.set(READER_SID, serde_json::json!(""));
        }
        if !initial_state.data.contains_key(PAGES) {
            initial_state.set(PAGES, serde_json::json!({}));
        }
        let init_state = std::sync::Arc::new(initial_state);
        // where a page trades its ticket for its cookie, and fetches a file
        let sign_path = format!("/_sign{}", page_path);
        let download_path = format!("/_download{}", page_path);
        let upload_path = format!("/_upload{}", page_path);
        let widget_path = format!("/_widget{}", page_path);
        let pending = std::sync::Arc::new(PendingSessions::default());
        let url = std::sync::Arc::new(self.url.clone());

        // GET page: the session rendered once and parked for its socket,
        // which the page's token claims
        let r1 = render.clone();
        let s1 = init_state.clone();
        let p1 = pending.clone();
        let ws_p = ws_path.clone();
        let u1 = url.clone();
        let page_route = axum::routing::get(move |query: axum::extract::Query<HashMap<String, String>>, headers: axum::http::HeaderMap| {
            let r1 = r1.clone();
            let s1 = s1.clone();
            let p1 = p1.clone();
            let ws_p = ws_p.clone();
            let u1 = u1.clone();
            async move {
                let mut state = start_session(&s1);
                // who the reader is: the sign-in their cookie carries
                if let Some((record, sid)) = signin::from_headers(&headers) {
                    state.set_reader(record, sid);
                }
                // the address's facts, each in its fact's shape
                for (name, fact) in u1.iter() {
                    if let Some(text) = query.get(name) {
                        if let Some(v) = coerce_field_value(state.get(fact), text) {
                            state.set(fact, v);
                        }
                    }
                }
                fetch_sources(&mut state).await;
                state.next_tick();
                // a render runs off the threads that carry the sockets
                let rendered = tokio::task::spawn_blocking(move || {
                    let vnode = state.render_with(&*r1).as_built();
                    (state, vnode)
                })
                .await;
                let Ok((state, vnode)) = rendered else {
                    return axum::response::Html("<p>the page failed to render; the server log says why</p>".to_string());
                };
                let body_html = vnode.to_html();
                let token = p1.park(state, vnode);
                let js = live_js_for(&ws_p, &token);
                axum::response::Html(page_shell(&body_html, &js))
            }
        });

        // WS: the session's loop — events from the page, refresh ticks,
        // background deltas and wakes, each answered by one render, sent
        // when it changed. Handlers and renders run on worker threads.
        let r2 = render.clone();
        let h2 = handle.clone();
        let s2 = init_state.clone();
        let p2 = pending.clone();
        let u2 = url.clone();
        let ws_route = axum::routing::get(
            move |ws: axum::extract::WebSocketUpgrade,
                  headers: axum::http::HeaderMap,
                  query: axum::extract::Query<HashMap<String, String>>| {
                let r2 = r2.clone();
                let h2 = h2.clone();
                let s2 = s2.clone();
                let p2 = p2.clone();
                let u2 = u2.clone();
                async move {
                    use axum::response::IntoResponse;
                    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                    let origin = header("origin");
                    if !origin_allowed(origin.as_deref(), header("host").as_deref()) {
                        eprintln!(
                            "crow live: refused a socket opened by {} — a page drives its own app only",
                            origin.unwrap_or_default()
                        );
                        return (axum::http::StatusCode::FORBIDDEN, "forbidden").into_response();
                    }
                    // the tab's own session, if it had one; else the page's
                    let tab_token = query.get("t").cloned();
                    let page_token = query.get("p").cloned();
                    let signed = signin::from_headers(&headers);
                    let sign_path = sign_path.clone();
                    let download_path = download_path.clone();
                    let upload_path = upload_path.clone();
                    ws.on_upgrade(move |mut socket| async move {
                        use axum::extract::ws::Message;
                        // The page's own session when it brings a live token.
                        // Otherwise — a reconnect, a restarted server — a fresh
                        // session, whose render is sent at once: the page never
                        // keeps showing what the server no longer holds.
                        let claimed = tab_token.as_deref().and_then(|t| p2.claim(t))
                            .or_else(|| page_token.as_deref().and_then(|t| p2.claim(t)));
                        if let Some(t) = tab_token.as_deref() {
                            forget_kept(t);
                        }
                        // a restarted program: the tab's session may have been kept
                        let claimed = match claimed {
                            Some(c) => Some(c),
                            None => match tab_token.as_deref().and_then(|t| kept_session(t, &s2)) {
                                Some(mut state) => {
                                    state.next_tick();
                                    let r = r2.clone();
                                    let rendered = tokio::task::spawn_blocking(move || {
                                        let vnode = state.render_with(&*r).as_built();
                                        (state, vnode)
                                    })
                                    .await;
                                    let Ok((state, vnode)) = rendered else { return };
                                    Some((state, vnode, true))
                                }
                                None => None,
                            },
                        };
                        let (mut state, mut last_vnode, mut send_first) =
                            match claimed {
                                // a session its tab comes back to is sent its
                                // render: the tab shows an old one
                                Some((state, vnode, resumed)) => (state, vnode, resumed),
                                None => {
                                    let mut state = start_session(&s2);
                                    if let Some((record, sid)) = signed {
                                        state.set_reader(record, sid);
                                    }
                                    fetch_sources(&mut state).await;
                                    state.next_tick();
                                    let r = r2.clone();
                                    let rendered = tokio::task::spawn_blocking(move || {
                                        let vnode = state.render_with(&*r).as_built();
                                        (state, vnode)
                                    })
                                    .await;
                                    let Ok((state, vnode)) = rendered else { return };
                                    (state, vnode, true)
                                }
                            };
                        // a sign-in signed out while the tab was away is gone:
                        // the tab is drawn again as nobody's
                        if state.drop_ended_sign_in() {
                            let r = r2.clone();
                            let rendered = tokio::task::spawn_blocking(move || {
                                let vnode = state.render_with(&*r).as_built();
                                (state, vnode)
                            })
                            .await;
                            let Ok((s, vnode)) = rendered else { return };
                            state = s;
                            last_vnode = vnode;
                            send_first = true;
                        }
                        let (later_tx, mut later_rx) = tokio::sync::mpsc::unbounded_channel::<
                            Vec<(String, serde_json::Value)>,
                        >();
                        state.later_tx = Some(later_tx);
                        // what wakes this loop: a recompute landing, another
                        // session writing a table this page reads
                        let session = match state.handle.clone() {
                            Some(h) => h,
                            None => {
                                let fresh = start_session(&state);
                                let h = fresh.handle.clone();
                                state.session_id = fresh.session_id;
                                state.handle = h.clone();
                                match h { Some(h) => h, None => return }
                            }
                        };
                        if send_first {
                            let payload = serde_json::json!({ "type": "html", "html": last_vnode.to_html() });
                            if futures_util::SinkExt::send(&mut socket, Message::Text(payload.to_string().into()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        // the token the tab resumes with: good once, replaced at
                        // every connect, sent only over this socket
                        let resume = new_token();
                        let hello = serde_json::json!({ "type": "session", "token": resume });
                        if futures_util::SinkExt::send(&mut socket, Message::Text(hello.to_string().into())).await.is_err() {
                            return;
                        }
                        // a sign-in made just before the socket closed is handed on
                        if let Some(msg) = sign_message(&mut state, &sign_path) {
                            if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                return;
                            }
                        }
                        keep_session(&resume, &state);
                        let mut kept_versions = state.versions.clone();
                        // the address follows the page's url facts
                        let mut sent_query = String::new();
                        if !u2.is_empty() {
                            sent_query = url_query(&state, &u2);
                            let msg = serde_json::json!({ "type": "url", "query": sent_query });
                            if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                return;
                            }
                        }
                        // What the last two renders the page received offered:
                        // a click may cross the render that removed its button.
                        let mut offers = Offers::of(&last_vnode);
                        let mut offers_before = Offers::default();

                        // Build poll intervals — collect sources that have interval_secs
                        let poll_sources: Vec<DataSource> = state.sources.iter()
                            .filter(|s| s.interval_secs.is_some())
                            .cloned()
                            .collect();

                        // Create a merged poll interval (tick at shortest interval)
                        let tick_secs = poll_sources.iter()
                            .filter_map(|s| s.interval_secs)
                            .min();

                        // the interval always exists (a year-long period when
                        // there is no poll source) so the select! stays one shape
                        let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(
                            tick_secs.unwrap_or(31_536_000),
                        ));
                        poll_interval.tick().await;

                        // serve[…, refresh = N]: re-render every N seconds. The
                        // interval always exists (a year-long period when unset)
                        // so the select! stays one shape; missed ticks are
                        // skipped — a slow render coalesces, never stacks.
                        let refresh_period = std::time::Duration::from_secs(
                            refresh_secs().unwrap_or(31_536_000),
                        );
                        let mut refresh_interval = tokio::time::interval_at(
                            tokio::time::Instant::now() + refresh_period,
                            refresh_period,
                        );
                        refresh_interval
                            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                        // Track elapsed ticks per source for per-source intervals
                        let mut tick_count: u64 = 0;
                        let base_secs = tick_secs.unwrap_or(1);

                        // Events are applied one at a time, in arrival order; a
                        // handler runs off this loop on its working copy while
                        // the page stays live, and holds the queue until it ends
                        let session_id = state.session_id;
                        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
                        // the next render goes as the whole page (the page asked)
                        let mut whole_page = false;
                        let mut running: Option<(String, Option<u64>, tokio::task::JoinHandle<CrowLiveState>)> = None;
                        let mut acks: Vec<u64> = Vec::new();

                        loop {
                            let action = tokio::select! {
                                msg = futures_util::StreamExt::next(&mut socket) => {
                                    match msg {
                                        Some(Ok(m)) => LiveAction::Message(m),
                                        _ => break,
                                    }
                                }
                                _ = poll_interval.tick(), if tick_secs.is_some() => LiveAction::PollTick,
                                delta = later_rx.recv() => {
                                    match delta {
                                        Some(d) => LiveAction::LaterDelta(d),
                                        None => continue,
                                    }
                                }
                                _ = refresh_interval.tick() => LiveAction::RefreshTick,
                                _ = session.wake.notified() => LiveAction::Wake,
                                done = async {
                                    match running.as_mut() {
                                        Some((_, _, job)) => job.await,
                                        None => std::future::pending().await,
                                    }
                                } => LiveAction::HandlerDone(done.map_err(panic_reason)),
                            };

                            let mut render = false;
                            // A refresh tick is the clock (derive-design §3.3):
                            // a click's render is not a tick, so it recomputes only
                            // the derivations whose inputs the click moved.
                            let mut tick = false;
                            let mut closed = false;

                            match action {
                                LiveAction::Message(Message::Text(text)) => {
                                    let mut texts = vec![text.to_string()];
                                    // Input that queued while the last render ran is
                                    // taken now: a slider drag is a burst of events.
                                    loop {
                                        match futures_util::FutureExt::now_or_never(futures_util::StreamExt::next(&mut socket)) {
                                            Some(Some(Ok(Message::Text(more)))) => texts.push(more.to_string()),
                                            Some(Some(Ok(Message::Close(_)))) | Some(Some(Err(_))) | Some(None) => {
                                                closed = true;
                                                break;
                                            }
                                            Some(Some(Ok(_))) => {}
                                            None => break,
                                        }
                                    }
                                    for text in texts {
                                        // a second click on the handler that is running
                                        // is dropped: the double-click
                                        let (event, _, seq) = parse_event(&text);
                                        // a page that could not place a change asks
                                        // for the whole page, at once
                                        if event == "__resync" {
                                            whole_page = true;
                                            render = true;
                                            continue;
                                        }
                                        if running.as_ref().map(|(h, _, _)| *h == event).unwrap_or(false) {
                                            acks.extend(seq);
                                            continue;
                                        }
                                        queue.push_back(text);
                                    }
                                }
                                LiveAction::PollTick => {
                                    tick_count += 1;
                                    let elapsed_secs = tick_count * base_secs;
                                    // Re-fetch only sources whose interval has elapsed
                                    for source in &poll_sources {
                                        let interval = source.interval_secs.unwrap_or(base_secs);
                                        if elapsed_secs % interval == 0 {
                                            if let Ok(resp) = reqwest::get(&source.url).await {
                                                if let Ok(json) = resp.json::<serde_json::Value>().await {
                                                    state.set(&source.key, json);
                                                    render = true;
                                                }
                                            }
                                        }
                                    }
                                }
                                LiveAction::LaterDelta(delta) => {
                                    for (k, v) in delta {
                                        state.set(&k, v);
                                    }
                                    render = true;
                                }
                                LiveAction::RefreshTick => {
                                    // Render-time derivations recompute — that's the point.
                                    render = true;
                                    tick = true;
                                }
                                LiveAction::Wake => {
                                    // a recompute landed, another session wrote a
                                    // table this page reads, or a sign-in ended
                                    state.drop_ended_sign_in();
                                    render = true;
                                }
                                LiveAction::HandlerDone(done) => {
                                    let (handler, seq, _) = running.take().expect("a handler was running");
                                    later_registry().lock().unwrap_or_else(|p| p.into_inner()).remove(&format!("{}:{}", state.session_id, handler));
                                    match done {
                                        Ok(copy) => {
                                            if let Some(reason) = state.commit(copy) {
                                                eprintln!("crow live: {}", reason);
                                            }
                                        }
                                        Err(why) => state.stopped_by(&handler, &why),
                                    }
                                    acks.extend(seq);
                                    render = true;
                                }
                                _ => {}
                            }
                            if closed {
                                break;
                            }

                            // messages the events below hand the page (an upload's ticket)
                            let mut outgoing: Vec<serde_json::Value> = Vec::new();
                            // The next events, one at a time: bound inputs land at
                            // once; a click starts its handler, which holds the queue
                            while running.is_none() {
                                let Some(text) = queue.pop_front() else { break };
                                let (event, value, seq) = parse_event(&text);
                                if ![&offers, &offers_before].iter().any(|o| o.allows(&event, value.as_deref())) {
                                    eprintln!("crow live: refused '{}' — the page did not offer it", event);
                                    acks.extend(seq);
                                    continue;
                                }
                                if apply_field(&mut state, &event, value.as_deref()) {
                                    acks.extend(seq);
                                    render = true;
                                    continue;
                                }
                                // a click that asks first: the question opens, the
                                // answer runs the handler or closes it
                                let (event, value) = match event.as_str() {
                                    "__ask" => {
                                        if let Some((h, args, text)) = value.as_deref().and_then(|v| serde_json::from_str::<(String, Option<String>, String)>(v).ok()) {
                                            state.asking = Some((h, args, text));
                                        }
                                        acks.extend(seq);
                                        render = true;
                                        continue;
                                    }
                                    "__cancel" => {
                                        state.asking = None;
                                        acks.extend(seq);
                                        render = true;
                                        continue;
                                    }
                                    // a file picked for a file field: refused over the
                                    // limit before it is sent, else a ticket to send it by
                                    "__upload" => {
                                        if let Some((field, name, kind, size)) = value.as_deref().and_then(|v| serde_json::from_str::<(String, String, String, u64)>(v).ok()) {
                                            let key = field.strip_prefix("__field:").unwrap_or(&field).to_string();
                                            if key == READER || key.starts_with("__") {
                                                eprintln!("crow live: refused an upload into '{}' — no input writes it", key);
                                            } else if size > files::upload_limit() {
                                                state.notice(format!("{} is {} — files up to {}", name, files::describe_size(size), files::describe_size(files::upload_limit())));
                                            } else if let Some(tx) = state.later_tx.clone() {
                                                let up = files::Upload { session: state.session_id, key: key.clone(), name, kind, size, tx };
                                                match files::expect_upload(up) {
                                                    Ok(t) => outgoing.push(serde_json::json!({ "type": "upload", "field": field, "url": format!("{}?t={}", upload_path, t) })),
                                                    Err(e) => eprintln!("crow live: an upload could not be expected — {}", e),
                                                }
                                            }
                                        }
                                        acks.extend(seq);
                                        render = true;
                                        continue;
                                    }
                                    // a paged table's previous or next page
                                    "__page" => {
                                        if let Some((index, at)) = value.as_deref().and_then(|v| serde_json::from_str::<(usize, usize)>(v).ok()) {
                                            state.turn_page(index, at);
                                        }
                                        acks.extend(seq);
                                        render = true;
                                        continue;
                                    }
                                    "__confirm" => match state.asking.take() {
                                        Some((h, args, _)) => (h, args),
                                        None => {
                                            acks.extend(seq);
                                            continue;
                                        }
                                    },
                                    _ => (event, value),
                                };
                                // running[event] reads true while it runs
                                let key = format!("{}:{}", state.session_id, event);
                                later_registry().lock().unwrap_or_else(|p| p.into_inner()).insert(key.clone());
                                let copy = state.working_copy();
                                let h = h2.clone();
                                let (ev, val) = (event.clone(), value.clone());
                                let mut job = tokio::task::spawn_blocking(move || run_handler(copy, &ev, val.as_deref(), &*h));
                                // a quick handler is answered by one render; one still
                                // running after a moment shows the page it is working
                                match tokio::time::timeout(HANDLER_GRACE, &mut job).await {
                                    Ok(done) => {
                                        later_registry().lock().unwrap_or_else(|p| p.into_inner()).remove(&key);
                                        match done {
                                            Ok(copy) => {
                                                if let Some(reason) = state.commit(copy) {
                                                    eprintln!("crow live: {}", reason);
                                                }
                                            }
                                            Err(e) => state.stopped_by(&event, &panic_reason(e)),
                                        }
                                        acks.extend(seq);
                                    }
                                    Err(_) => running = Some((event, seq, job)),
                                }
                                render = true;
                            }

                            // an upload's ticket, for the page to send its file by
                            let mut gone = false;
                            for msg in outgoing {
                                if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                    gone = true;
                                    break;
                                }
                            }
                            if gone {
                                break;
                            }
                            // a commit signed the reader in or out: the page
                            // trades a ticket for its cookie
                            if let Some(msg) = sign_message(&mut state, &sign_path) {
                                if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                            // files a commit handed the reader: the page fetches each
                            let mut gone = false;
                            for msg in download_messages(&mut state, &download_path) {
                                if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                    gone = true;
                                    break;
                                }
                            }
                            if gone {
                                break;
                            }
                            if !render {
                                if !acks.is_empty() {
                                    let n = acks.iter().copied().max().unwrap_or(0);
                                    acks.clear();
                                    let msg = serde_json::json!({ "type": "ack", "ack": n });
                                    if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                        break;
                                    }
                                }
                                continue;
                            }

                            // The render runs on a worker thread, from the committed
                            // facts: a handler running meanwhile is never seen halfway
                            let r = r2.clone();
                            let busy = running.as_ref().map(|(h, _, _)| h.clone());
                            let mut state_out = std::mem::take(&mut state);
                            let turn = tokio::task::spawn_blocking(move || {
                                let mut state = std::mem::take(&mut state_out);
                                if tick {
                                    state.next_tick();
                                }
                                let t0 = std::time::Instant::now();
                                let mut v = state.render_with(&*r).as_built();
                                if let Some(budget) = budget_ms() {
                                    let ms = t0.elapsed().as_millis();
                                    if ms > budget {
                                        eprintln!(
                                            "crow live: render took {}ms (budget {}ms) — move slow work out of the render path",
                                            ms, budget
                                        );
                                    }
                                }
                                if let Some(h) = &busy {
                                    v = mark_running(v, h);
                                }
                                (state, v)
                            })
                            .await;
                            let (moved_state, new_vnode) = match turn {
                                Ok(t) => t,
                                // a render panicked: the session is gone
                                Err(_) => break,
                            };
                            state = moved_state;
                            if keeping() && state.versions != kept_versions {
                                keep_session(&resume, &state);
                                kept_versions = state.versions.clone();
                            }

                            let ack = acks.iter().copied().max();
                            acks.clear();
                            // what changed, when that is smaller than the page and
                            // both renders are trees the browser builds as read
                            let changes = CrowVNode::changes(&last_vnode, &new_vnode);
                            let placeable = matches!((&last_vnode, &new_vnode), (CrowVNode::Element { .. }, CrowVNode::Element { .. }));
                            let payload = if !changes.is_empty() || whole_page {
                                let html = new_vnode.to_html();
                                let patch = serde_json::Value::Array(changes);
                                let mut msg = if placeable && !whole_page && patch.to_string().len() * 4 < html.len() * 3 {
                                    serde_json::json!({ "type": "patches", "patches": patch })
                                } else {
                                    serde_json::json!({ "type": "html", "html": html })
                                };
                                whole_page = false;
                                if let Some(n) = ack {
                                    msg["ack"] = serde_json::json!(n);
                                }
                                offers_before = std::mem::replace(&mut offers, Offers::of(&new_vnode));
                                Some(msg)
                            } else {
                                // a click the page marked busy is answered even
                                // when its render changed nothing
                                ack.map(|n| serde_json::json!({ "type": "ack", "ack": n }))
                            };
                            last_vnode = new_vnode;
                            if let Some(msg) = payload {
                                if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            if !u2.is_empty() {
                                let query = url_query(&state, &u2);
                                if query != sent_query {
                                    let msg = serde_json::json!({ "type": "url", "query": query });
                                    if futures_util::SinkExt::send(&mut socket, Message::Text(msg.to_string().into())).await.is_err() {
                                        break;
                                    }
                                    sent_query = query;
                                }
                            }
                        }
                        // the socket closed: a handler still running lands first,
                        // then the session waits for its tab to come back
                        if let Some((handler, _, job)) = running {
                            if let Ok(copy) = job.await {
                                if let Some(reason) = state.commit(copy) {
                                    eprintln!("crow live: {}", reason);
                                }
                            }
                            later_registry().lock().unwrap_or_else(|p| p.into_inner()).remove(&format!("{}:{}", session_id, handler));
                        }
                        // a session lost to a panicking render has nothing to keep
                        if state.session_id == session_id && session_id != 0 {
                            keep_session(&resume, &state);
                            p2.park_as(resume, state, last_vnode, PendingSessions::GRACE, true);
                        }
                    })
                    .into_response()
                }
            },
        );

        // POST /_sign/…?t=: a page trades its one-time ticket for the cookie
        // its sign-in or sign-out set — HttpOnly, which no socket can set
        let sign_route = axum::routing::post(
            move |query: axum::extract::Query<HashMap<String, String>>, headers: axum::http::HeaderMap| async move {
                use axum::response::IntoResponse;
                let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                if !origin_allowed(header("origin").as_deref(), header("host").as_deref()) {
                    return (axum::http::StatusCode::FORBIDDEN, "forbidden").into_response();
                }
                let Some(cookie) = query.get("t").and_then(|t| signin::redeem(t)) else {
                    return (axum::http::StatusCode::GONE, "that ticket was used or has expired").into_response();
                };
                let secure = header("x-forwarded-proto").map_or(false, |p| p.eq_ignore_ascii_case("https"));
                (
                    axum::http::StatusCode::NO_CONTENT,
                    [(axum::http::header::SET_COOKIE, signin::set_cookie(cookie.as_deref(), secure))],
                )
                    .into_response()
            },
        );

        // GET /_download/…?t=: the file a handler handed the reader, once
        let download_route = axum::routing::get(move |query: axum::extract::Query<HashMap<String, String>>| async move {
            use axum::response::IntoResponse;
            let Some(file) = query.get("t").and_then(|t| files::take(t)) else {
                return (axum::http::StatusCode::GONE, "that download was fetched or has expired").into_response();
            };
            (
                [
                    (axum::http::header::CONTENT_TYPE, files::media_type(&file.name).to_string()),
                    (axum::http::header::CONTENT_DISPOSITION, files::attachment(&file.name)),
                    (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
                    (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                ],
                file.bytes,
            )
                .into_response()
        });

        // POST /_upload/…?t=: a file for a file field, by the ticket the page
        // was handed; read no further than the app's limit
        let upload_route = axum::routing::post(
            move |query: axum::extract::Query<HashMap<String, String>>, headers: axum::http::HeaderMap, body: axum::body::Body| async move {
                use axum::response::IntoResponse;
                let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                if !origin_allowed(header("origin").as_deref(), header("host").as_deref()) {
                    return (axum::http::StatusCode::FORBIDDEN, "forbidden").into_response();
                }
                let Some(up) = query.get("t").and_then(|t| files::claim_upload(t)) else {
                    return (axum::http::StatusCode::GONE, "that upload ticket was used or has expired").into_response();
                };
                let Ok(bytes) = axum::body::to_bytes(body, files::upload_limit() as usize).await else {
                    return (axum::http::StatusCode::PAYLOAD_TOO_LARGE, "the file is over the app's limit").into_response();
                };
                if bytes.len() as u64 != up.size {
                    return (axum::http::StatusCode::BAD_REQUEST, "the file is not the size the page said").into_response();
                }
                match files::keep_upload(up.session, &up.name, &bytes) {
                    Ok(path) => {
                        // the field's variable holds the file, as the page's
                        // next render shows
                        let record = serde_json::json!({ "name": up.name, "type": up.kind, "size": up.size.to_string(), "path": path });
                        let _ = up.tx.send(vec![(up.key, record)]);
                        axum::http::StatusCode::NO_CONTENT.into_response()
                    }
                    Err(e) => {
                        eprintln!("crow live: an upload could not be kept — {}", e);
                        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "the file could not be kept").into_response()
                    }
                }
            },
        );

        // GET /_widget/…?m=name: the program's module widgets/name.js, and
        // nothing else of its files
        let widget_route = axum::routing::get(move |query: axum::extract::Query<HashMap<String, String>>| async move {
            use axum::response::IntoResponse;
            let name = query.get("m").cloned().unwrap_or_default();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                return (axum::http::StatusCode::NOT_FOUND, "no such widget").into_response();
            }
            match std::fs::read_to_string(std::path::Path::new("widgets").join(format!("{}.js", name))) {
                Ok(js) => (
                    [
                        (axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                        (axum::http::header::CACHE_CONTROL, "no-cache"),
                    ],
                    js,
                )
                    .into_response(),
                Err(_) => (axum::http::StatusCode::NOT_FOUND, format!("no widgets/{}.js", name)).into_response(),
            }
        });

        axum::Router::new()
            .route(page_path, page_route)
            .route(&ws_path, ws_route)
            .route(&format!("/_sign{}", page_path), sign_route)
            .route(&format!("/_download{}", page_path), download_route)
            .route(&format!("/_upload{}", page_path), upload_route)
            .route(&widget_path, widget_route)
    }
}

/// The messages that tell a page to fetch the files the last commits handed
/// its reader.
fn download_messages(state: &mut CrowLiveState, download_path: &str) -> Vec<serde_json::Value> {
    state
        .downloads
        .drain(..)
        .filter_map(|(name, bytes)| match files::offer(&name, bytes) {
            Ok(t) => Some(serde_json::json!({ "type": "download", "url": format!("{}?t={}", download_path, t), "name": name })),
            Err(e) => {
                eprintln!("crow live: the download '{}' could not be handed on — {}", name, e);
                None
            }
        })
        .collect()
}

/// What made a handler's thread panic, in its own words.
fn panic_reason(e: tokio::task::JoinError) -> String {
    match e.try_into_panic() {
        Ok(p) => p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "it panicked".to_string()),
        Err(_) => "it was cancelled".to_string(),
    }
}

/// The message that hands a page the ticket for a sign-in's cookie (or a
/// sign-out's), when the last commit made one.
fn sign_message(state: &mut CrowLiveState, sign_path: &str) -> Option<serde_json::Value> {
    let cookie = state.signing.take()?;
    match signin::ticket(cookie) {
        Ok(t) => Some(serde_json::json!({ "type": "sign", "url": format!("{}?t={}", sign_path, t) })),
        Err(e) => {
            eprintln!("crow live: the sign-in holds for this session only — {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A socket opens only from the app's own page; a page on another site
    /// (it deleted a session through finch's socket) is refused, and a
    /// client that is not a browser, sending no origin, is let through
    #[test]
    fn a_socket_opens_from_its_own_page_only() {
        assert!(origin_allowed(Some("http://127.0.0.1:3100"), Some("127.0.0.1:3100")));
        assert!(origin_allowed(Some("http://localhost:3100/"), Some("localhost:3100")));
        assert!(!origin_allowed(Some("https://evil.example"), Some("127.0.0.1:3100")));
        assert!(!origin_allowed(Some("http://127.0.0.1:3101"), Some("127.0.0.1:3100")), "another app's page");
        assert!(!origin_allowed(Some("null"), Some("127.0.0.1:3100")), "a sandboxed page");
        assert!(origin_allowed(None, Some("127.0.0.1:3100")));
    }

    /// What a render offered: clicks with their frozen arguments, bound
    /// inputs and selects by name — in elements, and in markup built as
    /// text (a chart's seek bars, a table's rows), unescaped as the browser
    /// sends them back. A helper no button names, a click with arguments
    /// the render did not freeze, and a fact no input binds are refused.
    #[test]
    fn a_render_offers_what_its_page_can_do() {
        let tree = CrowVNode::div(vec![
            CrowVNode::button(vec![CrowVNode::text("Play")]).on_click("toggle_play"),
            CrowVNode::button(vec![CrowVNode::text("10x")]).on_click_args("set_speed", "[10.0]"),
            CrowVNode::input().attr("data-crow-input", "__field:seek_pct"),
            CrowVNode::select(vec![]).attr("data-crow-change", "__select:sym"),
            CrowVNode::Raw("<svg><rect class=\"crow-seek\" data-crow-click=\"jump\" data-crow-args=\"[1788763600]\"/></svg>".into()),
            CrowVNode::Raw("<table><tbody><tr data-crow-click=\"focus\" data-crow-args=\"[&quot;IMRN&quot;]\"><td>IMRN</td></tr></tbody></table>".into()),
        ]);
        let offers = Offers::of(&tree);
        assert!(offers.allows("toggle_play", None));
        assert!(offers.allows("set_speed", Some("[10.0]")));
        assert!(!offers.allows("set_speed", Some("[99.0]")), "arguments the render did not freeze");
        assert!(offers.allows("__field:seek_pct", Some("50")));
        assert!(offers.allows("__select:sym", Some("CHPT")));
        assert!(offers.allows("jump", Some("[1788763600]")));
        assert!(offers.allows("focus", Some("[\"IMRN\"]")), "a table row's key, unescaped");
        assert!(!offers.allows("say", Some("[\"hi\"]")), "a helper no button names");
        assert!(!offers.allows("__field:session_id", Some("5")), "a fact no input binds");
        assert!(!offers.allows("delete_confirmed", None), "a confirmation's second step without its first");
    }

    /// A closed modal offers nothing: its confirm button is in the page but
    /// hidden, so the second step of a confirmation is accepted only after
    /// the first step opened it (finch's "Delete session?" dialog)
    #[test]
    fn a_closed_modal_offers_nothing() {
        let page = |open: bool| {
            CrowVNode::div(vec![
                CrowVNode::button(vec![CrowVNode::text("Delete")]).on_click_args("ask_delete", "[7]"),
                CrowVNode::div(vec![CrowVNode::button(vec![CrowVNode::text("Delete")]).on_click("delete_confirmed")])
                    .attr("class", "modal")
                    .attr("data-open", if open { "true" } else { "false" }),
                CrowVNode::button(vec![CrowVNode::text("Hidden")]).on_click("secret").attr("hidden", "hidden"),
            ])
        };
        let closed = Offers::of(&page(false));
        assert!(closed.allows("ask_delete", Some("[7]")));
        assert!(!closed.allows("delete_confirmed", None), "the dialog is closed");
        assert!(!closed.allows("secret", None), "a hidden button");
        assert!(Offers::of(&page(true)).allows("delete_confirmed", None), "the dialog is open");
    }

    /// The page request parks its rendered session for the page's socket,
    /// which claims it once; a second claim (a reconnect) gets nothing and
    /// starts fresh
    #[test]
    fn a_page_session_is_claimed_once() {
        let pending = PendingSessions::default();
        let mut state = CrowLiveState::new();
        state.set("n", serde_json::json!(4));
        let token = pending.park(state, CrowVNode::text("four"));
        assert_eq!(token.len(), 32, "a random token: {}", token);
        let (claimed, vnode, resumed) = pending.claim(&token).expect("the page's socket claims its session");
        assert_eq!(claimed.get_i64("n", 0), 4);
        assert_eq!(vnode.to_html(), "four");
        assert!(!resumed, "a page's own session: its page already shows the render");
        assert!(pending.claim(&token).is_none(), "a token claims once");
        assert!(pending.claim("not-a-token").is_none());
    }

    /// A session whose socket closed waits for its tab under the token the
    /// tab was given, once, and comes back as a resumed session
    #[test]
    fn a_session_waits_for_its_tab_under_its_token() {
        let pending = PendingSessions::default();
        let mut state = CrowLiveState::new();
        state.set("n", serde_json::json!(9));
        pending.park_as("tabtoken".to_string(), state, CrowVNode::text("nine"), PendingSessions::GRACE, true);
        let (back, _, resumed) = pending.claim("tabtoken").expect("the tab resumes its session");
        assert_eq!(back.get_i64("n", 0), 9, "the same facts");
        assert!(resumed, "a resumed session is sent its render");
        assert!(pending.claim("tabtoken").is_none(), "the token is good once");
        pending.park_as("gone".to_string(), CrowLiveState::new(), CrowVNode::text(""), std::time::Duration::from_millis(0), true);
        assert!(pending.claim("gone").is_none(), "a session past its wait is gone");
    }

    /// The page says when its socket is down and takes no input then, and
    /// every message goes through the one guarded send
    #[test]
    fn the_page_shows_when_it_is_offline() {
        let js = live_js_for("/_ws/", "abc");
        assert!(js.contains("pageToken = 'abc'"), "the socket carries the page's token");
        assert!(js.contains("sessionStorage.getItem(key)"), "and the tab's own session's");
        assert!(js.contains("r.setAttribute('data-crow-offline', '')"));
        assert!(js.contains("ws.onclose = function() {\n      offline(true);"), "{}", js);
        assert!(js.contains("if (ws && ws.readyState === 1)"));
        assert_eq!(js.matches("ws.send(").count(), 1, "one guarded send: {}", js);
        let page = page_shell("<p>hi</p>", &js);
        assert!(page.contains("html[data-crow-offline] #crow-root{opacity:.5;pointer-events:none"));
        assert!(page.contains("reconnecting\u{2026}"));
    }

    /// A counter page for the served tests: `inc` takes no arguments, `set`
    /// freezes one, `name` is a bound field, and `secret` is a handler no
    /// element names. The render shows the session's tick.
    fn counter_router() -> axum::Router {
        let mut initial = CrowLiveState::new();
        initial.set("n", serde_json::json!(0));
        initial.set("name", serde_json::json!(""));
        let render = |s: &CrowLiveState| {
            CrowVNode::div(vec![
                CrowVNode::p(vec![CrowVNode::text(&format!("n={} name={} tick={}", s.get_i64("n", 0), s.get_str("name", ""), s.tick))]),
                CrowVNode::button(vec![CrowVNode::text("inc")]).on_click("inc"),
                CrowVNode::button(vec![CrowVNode::text("seven")]).on_click_args("set", "[7]"),
                CrowVNode::input().attr("data-crow-input", "__field:name"),
            ])
        };
        let handle = |s: &mut CrowLiveState, event: &str, value: Option<&str>| match event {
            "inc" => {
                let n = s.get_i64("n", 0);
                s.set("n", serde_json::json!(n + 1));
            }
            "set" => {
                let v: Vec<i64> = serde_json::from_str(value.unwrap_or("[0]")).unwrap_or(vec![0]);
                s.set("n", serde_json::json!(v[0]));
            }
            "secret" => s.set("n", serde_json::json!(-1)),
            _ => {}
        };
        CrowLiveView::new("/_ws/", initial, render, handle).into_router("/")
    }

    /// The page a socket's messages make, as the browser keeps it: a whole
    /// page replaces it, changes apply to it. Whether `m` was a render.
    fn follow_page(page: &mut CrowVNode, m: &serde_json::Value) -> bool {
        if let Some(h) = m["html"].as_str() {
            *page = CrowVNode::raw(h).as_built();
            true
        } else if m["type"] == "patches" {
            let changes = m["patches"].as_array().cloned().unwrap_or_default();
            page.apply(&changes).unwrap_or_else(|e| panic!("the changes fit the page: {}", e));
            true
        } else {
            false
        }
    }

    /// The page a served document holds, as its socket's first state.
    fn served_root(document: &str) -> CrowVNode {
        let inner = document
            .split("<div id=\"crow-root\">")
            .nth(1)
            .and_then(|rest| rest.rsplit_once("</div>\n<script>").map(|(inner, _)| inner.to_string()))
            .unwrap_or_default();
        CrowVNode::raw(&inner).as_built()
    }

    fn slow_router() -> axum::Router {
        let mut initial = CrowLiveState::new();
        initial.set("slow", serde_json::json!(0));
        initial.set("quick", serde_json::json!(0));
        let render = |s: &CrowLiveState| {
            CrowVNode::div(vec![
                CrowVNode::p(vec![CrowVNode::text(&format!("slow={} quick={}", s.get_i64("slow", 0), s.get_i64("quick", 0)))]),
                CrowVNode::button(vec![CrowVNode::text("slow")]).on_click("slow"),
                CrowVNode::button(vec![CrowVNode::text("quick")]).on_click("quick"),
            ])
        };
        let handle = |s: &mut CrowLiveState, event: &str, _: Option<&str>| match event {
            "slow" => {
                std::thread::sleep(std::time::Duration::from_millis(600));
                let n = s.get_i64("slow", 0);
                s.set("slow", serde_json::json!(n + 1));
            }
            "quick" => {
                let n = s.get_i64("quick", 0);
                s.set("quick", serde_json::json!(n + 1));
            }
            _ => {}
        };
        CrowLiveView::new("/_ws/", initial, render, handle).into_router("/")
    }

    /// A slow handler runs off the page's loop: the page answers the click at
    /// once with the button working, a second click on it is dropped, and
    /// another click waits its turn (session-layer-design.md §3.3)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_slow_handler_runs_off_the_page() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, slow_router()).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            use tungstenite::Message as M;
            let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}/_ws/", port)).expect("the socket opens");
            if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                s.set_read_timeout(Some(std::time::Duration::from_millis(3000))).unwrap();
            }
            // the page's messages, the session's resume token aside
            let mut read = || -> serde_json::Value {
                loop {
                    match ws.read() {
                        Ok(M::Text(t)) => {
                            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                            if m["type"] != "session" {
                                return m;
                            }
                        }
                        other => panic!("expected a message, got {:?}", other),
                    }
                }
            };
            let first = read();
            let mut page = CrowVNode::text("");
            assert!(follow_page(&mut page, &first) && page.to_html().contains("slow=0"));
            drop(read);
            let t0 = std::time::Instant::now();
            ws.send(M::Text(r#"{"event":"slow","seq":1}"#.into())).unwrap();
            // the second click of a double-click, while the first runs
            std::thread::sleep(std::time::Duration::from_millis(200));
            ws.send(M::Text(r#"{"event":"slow","seq":2}"#.into())).unwrap();
            ws.send(M::Text(r#"{"event":"quick","seq":3}"#.into())).unwrap();
            // the page's messages, the session's resume token aside
            let mut read = || -> serde_json::Value {
                loop {
                    match ws.read() {
                        Ok(M::Text(t)) => {
                            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                            if m["type"] != "session" {
                                return m;
                            }
                        }
                        other => panic!("expected a message, got {:?}", other),
                    }
                }
            };
            let working = read();
            assert!(follow_page(&mut page, &working), "the page answers the click with a render: {}", working);
            let html = page.to_html();
            assert!(t0.elapsed() < std::time::Duration::from_millis(500), "answered while the handler runs");
            assert!(html.contains("slow=0"), "nothing landed yet: {}", html);
            let slow_button = html.split("<button").find(|b| b.contains("data-crow-click=\"slow\"")).unwrap();
            assert!(slow_button.contains("disabled") && slow_button.contains("aria-busy=\"true\""), "the button shows it is working: {}", slow_button);
            let mut acks = Vec::new();
            let mut last = String::new();
            while !acks.contains(&3) {
                let m = read();
                if let Some(n) = m["ack"].as_u64() {
                    acks.push(n);
                }
                if follow_page(&mut page, &m) {
                    last = page.to_html();
                }
            }
            assert!(acks.contains(&2), "the second click on the running handler was answered: {:?}", acks);
            assert!(last.contains("slow=1 quick=1"), "the handler ran once, then the waiting click: {}", last);
        })
        .await
        .unwrap();
    }

    /// A session outlives its socket: the tab comes back with its token and
    /// finds its facts; the token was good once (session-layer-design.md §3.8)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_survives_its_socket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, counter_router()).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            use tungstenite::Message as M;
            type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;
            let open = |q: &str| -> Ws {
                let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}/_ws/{}", port, q)).expect("the socket opens");
                if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                    s.set_read_timeout(Some(std::time::Duration::from_millis(3000))).unwrap();
                }
                ws
            };
            // (the first render, the session's token)
            let hello = |ws: &mut Ws| -> (String, String) {
                let (mut html, mut token) = (String::new(), String::new());
                while html.is_empty() || token.is_empty() {
                    if let Ok(M::Text(t)) = ws.read() {
                        let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                        if let Some(h) = m["html"].as_str() { html = h.to_string(); }
                        if let Some(k) = m["token"].as_str() { token = k.to_string(); }
                    }
                }
                (html, token)
            };
            let mut ws = open("");
            let (first, token) = hello(&mut ws);
            assert!(first.contains("n=0"));
            ws.send(M::Text(r#"{"event":"inc","seq":1}"#.into())).unwrap();
            loop {
                if let Ok(M::Text(t)) = ws.read() {
                    if t.contains("n=1") { break; }
                }
            }
            ws.close(None).unwrap();
            let _ = ws.read();
            std::thread::sleep(std::time::Duration::from_millis(200));
            // the tab comes back with its token: the same session, a new token
            let mut back = open(&format!("?t={}", token));
            let (html, next) = hello(&mut back);
            assert!(html.contains("n=1"), "the session came back: {}", html);
            assert_ne!(next, token, "every connect replaces the token");
            // the old token resumes nothing: a fresh session
            let mut stale = open(&format!("?t={}", token));
            let (fresh, _) = hello(&mut stale);
            assert!(fresh.contains("n=0"), "a used token starts fresh: {}", fresh);
        })
        .await
        .unwrap();
    }

    /// page[…, url = ['n']]: the page starts from its address, and the
    /// address follows the fact (session-layer-design.md §3.9)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_address_carries_the_url_facts() {
        let mut initial = CrowLiveState::new();
        initial.set("n", serde_json::json!(0));
        let render = |s: &CrowLiveState| CrowVNode::div(vec![
            CrowVNode::p(vec![CrowVNode::text(&format!("n={}", s.get_i64("n", 0)))]),
            CrowVNode::button(vec![CrowVNode::text("inc")]).on_click("inc"),
        ]);
        let handle = |s: &mut CrowLiveState, event: &str, _: Option<&str>| {
            if event == "inc" {
                let n = s.get_i64("n", 0);
                s.set("n", serde_json::json!(n + 1));
            }
        };
        let router = CrowLiveView::new("/_ws/", initial, render, handle).with_url(vec![("n", "n")]).into_router("/");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            use tungstenite::Message as M;
            let mut http = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(http, "GET /?n=5 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port).unwrap();
            let mut page = String::new();
            http.read_to_string(&mut page).unwrap();
            assert!(page.contains("n=5"), "the page starts from its address");
            let token: String = page.split("pageToken = '").nth(1).unwrap().chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}/_ws/?p={}", port, token)).unwrap();
            if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                s.set_read_timeout(Some(std::time::Duration::from_millis(3000))).unwrap();
            }
            let mut next_url = || loop {
                if let Ok(M::Text(t)) = ws.read() {
                    let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if m["type"] == "url" {
                        return m["query"].as_str().unwrap_or("").to_string();
                    }
                }
            };
            assert_eq!(next_url(), "n=5");
            drop(next_url);
            ws.send(M::Text(r#"{"event":"inc","seq":1}"#.into())).unwrap();
            let mut next_url = || loop {
                if let Ok(M::Text(t)) = ws.read() {
                    let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if m["type"] == "url" {
                        return m["query"].as_str().unwrap_or("").to_string();
                    }
                }
            };
            assert_eq!(next_url(), "n=6", "the address follows the fact");
        })
        .await
        .unwrap();
    }

    /// Chrome under its DevTools protocol, for the tests that need a real
    /// browser (ignored in CI, which has none; CHROME names another binary).
    /// Dropping it closes the browser and its profile.
    struct Browser {
        child: std::process::Child,
        dir: std::path::PathBuf,
        devtools_port: u16,
        browser_path: String,
        ws: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
        id: u64,
        /// the browser's own session, kept open: what it sets (where files
        /// download) lasts as long as the session does
        browser: Option<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>>,
    }

    impl Browser {
        /// A headless Chrome and its first page — none when there is no Chrome.
        fn open(tag: &str) -> Option<Browser> {
            use std::io::{Read, Write};
            let chrome = std::env::var("CHROME").unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string());
            if !std::path::Path::new(&chrome).exists() {
                eprintln!("no browser at {}", chrome);
                return None;
            }
            let dir = std::env::temp_dir().join(format!("crow_live_{}_{}", tag, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let child = std::process::Command::new(&chrome)
                .args(["--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--remote-debugging-port=0", "--remote-allow-origins=*"])
                .arg(format!("--user-data-dir={}", dir.join("profile").display()))
                .arg("about:blank")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("the browser starts");
            // the browser names its DevTools port and its own target in its profile
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let (devtools_port, browser_path) = loop {
                if let Ok(text) = std::fs::read_to_string(dir.join("profile").join("DevToolsActivePort")) {
                    let mut lines = text.lines();
                    if let (Some(p), Some(path)) = (lines.next().and_then(|l| l.trim().parse::<u16>().ok()), lines.next()) {
                        break (p, path.trim().to_string());
                    }
                }
                assert!(std::time::Instant::now() < deadline, "the browser never opened DevTools");
                std::thread::sleep(std::time::Duration::from_millis(100));
            };
            let mut http = std::net::TcpStream::connect(("127.0.0.1", devtools_port)).unwrap();
            write!(http, "GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", devtools_port).unwrap();
            // the DevTools server keeps the connection: read to the body's length
            http.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            let body = loop {
                let n = http.read(&mut buf).unwrap_or(0);
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length: usize = text[..head_end]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                        .unwrap_or(0);
                    if raw.len() >= head_end + 4 + length || n == 0 {
                        break text[head_end + 4..].to_string();
                    }
                }
                assert!(n > 0, "the DevTools listing ended early");
            };
            let targets: serde_json::Value = serde_json::from_str(&body).expect("the browser lists its pages");
            let page_url = targets.as_array().unwrap().iter().find(|t| t["type"] == "page").expect("a page")["webSocketDebuggerUrl"].as_str().unwrap().to_string();
            let (ws, _) = tungstenite::connect(page_url).expect("DevTools opens");
            Some(Browser { child, dir, devtools_port, browser_path, ws, id: 0, browser: None })
        }

        fn call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
            use tungstenite::Message as M;
            self.id += 1;
            let msg = serde_json::json!({ "id": self.id, "method": method, "params": params });
            self.ws.send(M::Text(msg.to_string().into())).unwrap();
            loop {
                if let Ok(M::Text(t)) = self.ws.read() {
                    let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if m["id"] == self.id {
                        return m;
                    }
                }
            }
        }

        /// A call to the browser itself, not its page (downloads are the browser's).
        fn browser_call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
            use tungstenite::Message as M;
            if self.browser.is_none() {
                let (b, _) = tungstenite::connect(format!("ws://127.0.0.1:{}{}", self.devtools_port, self.browser_path)).expect("the browser's own DevTools");
                self.browser = Some(b);
            }
            self.id += 1;
            let id = self.id;
            let b = self.browser.as_mut().unwrap();
            b.send(M::Text(serde_json::json!({ "id": id, "method": method, "params": params }).to_string().into())).unwrap();
            loop {
                if let Ok(M::Text(t)) = b.read() {
                    let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if m["id"] == id {
                        return m;
                    }
                }
            }
        }

        fn eval(&mut self, js: &str) -> serde_json::Value {
            let r = self.call("Runtime.evaluate", serde_json::json!({ "expression": js, "returnByValue": true, "awaitPromise": true }));
            assert!(r["result"]["exceptionDetails"].is_null(), "the page threw: {}", r);
            r["result"]["result"]["value"].clone()
        }

        /// Wait until a script says true.
        fn until(&mut self, js: &str, what: &str) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while self.eval(js) != serde_json::json!(true) {
                assert!(std::time::Instant::now() < deadline, "{} never happened", what);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }

    impl Drop for Browser {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A widget in a real browser: the program's module mounts once and
    /// draws; what it sends reaches the handler its element names; the
    /// render that follows calls its update with the new props and leaves
    /// what it drew in place (spec: *One way out of the vocabulary*).
    /// `cargo test -p crow_web --features live,http a_widget_draws -- --ignored`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn a_widget_draws_and_talks_to_its_handler() {
        // the module, where the page's server looks for it
        struct Module(std::path::PathBuf);
        impl Drop for Module {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
                let _ = std::fs::remove_dir("widgets");
            }
        }
        std::fs::create_dir_all("widgets").unwrap();
        let module = Module(std::path::PathBuf::from("widgets/crow_test_pick.js"));
        std::fs::write(&module.0, "window.__mounts = 0;\n\
            export function mount(el, props, send) {\n\
              window.__mounts += 1;\n\
              const b = document.createElement('button');\n\
              b.className = 'pick';\n\
              b.textContent = 'shows ' + props.city;\n\
              b.onclick = () => send('pick', 'Paris');\n\
              el.appendChild(b);\n\
            }\n\
            export function update(el, props) {\n\
              el.querySelector('.pick').textContent = 'shows ' + props.city;\n\
            }\n").unwrap();
        let mut initial = CrowLiveState::new();
        initial.set("place", serde_json::json!("London"));
        let render = |s: &CrowLiveState| {
            let place = s.get_str("place", "");
            CrowVNode::div(vec![
                CrowVNode::p(vec![CrowVNode::text(&format!("place {}", place))]),
                CrowVNode::widget("crow_test_pick", &serde_json::json!({ "city": place }), &[("pick", "choose")]),
            ])
        };
        let handle = |s: &mut CrowLiveState, event: &str, value: Option<&str>| {
            if event == "choose" {
                let args: Vec<String> = value.and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
                s.set("place", serde_json::json!(args.first().cloned().unwrap_or_default()));
            }
        };
        let router = CrowLiveView::new("/_ws/", initial, render, handle).into_router("/");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            let Some(mut b) = Browser::open("widget") else { return };
            b.call("Page.enable", serde_json::json!({}));
            b.call("Page.navigate", serde_json::json!({ "url": format!("http://127.0.0.1:{}/", port) }));
            b.until("!!document.querySelector('.crow-widget .pick')", "the widget's mount");
            assert_eq!(b.eval("document.querySelector('.pick').textContent"), serde_json::json!("shows London"));
            b.eval("document.querySelector('.pick').click(); 1");
            b.until("document.body.innerText.indexOf('place Paris') >= 0", "the handler's render");
            b.until("document.querySelector('.pick') && document.querySelector('.pick').textContent === 'shows Paris'", "the widget's update");
            assert_eq!(b.eval("window.__mounts"), serde_json::json!(1), "mounted once, updated after");
        })
        .await
        .unwrap();
        drop(module);
    }

    /// A file picked in a real browser reaches its field, and handed back
    /// with `download` it is saved byte for byte (spec: *Files in and out*).
    /// `cargo test -p crow_web --features live,http a_file_round_trips -- --ignored`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn a_file_round_trips_through_the_browser() {
        let mut initial = CrowLiveState::new();
        initial.set("upload", serde_json::json!({}));
        let render = |s: &CrowLiveState| {
            let up = s.get("upload").cloned().unwrap_or_default();
            let name = up["name"].as_str().unwrap_or("").to_string();
            let contents = up["path"].as_str().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
            CrowVNode::div(vec![
                CrowVNode::field_typed("Trades", "upload", &name, "file", 0),
                CrowVNode::p(vec![CrowVNode::text(&format!("got {} {}", name, contents.trim()))]),
                CrowVNode::button(vec![CrowVNode::text("Back")]).on_click("back"),
            ])
        };
        let handle = |s: &mut CrowLiveState, event: &str, _: Option<&str>| {
            if event == "back" {
                let path = s.get("upload").and_then(|u| u["path"].as_str().map(|p| p.to_string())).unwrap_or_default();
                if let Ok(bytes) = std::fs::read(&path) {
                    s.download("back.csv", bytes);
                }
            }
        };
        let router = CrowLiveView::new("/_ws/", initial, render, handle).into_router("/");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            let Some(mut b) = Browser::open("files") else { return };
            let picked = b.dir.join("trades.csv");
            let body = "symbol,qty\nIMRN,100\n";
            std::fs::write(&picked, body).unwrap();
            let saved = b.dir.join("downloads");
            std::fs::create_dir_all(&saved).unwrap();
            let r = b.browser_call("Browser.setDownloadBehavior", serde_json::json!({ "behavior": "allow", "downloadPath": saved.to_string_lossy() }));
            assert!(r["error"].is_null(), "downloads allowed: {}", r);
            b.call("Page.enable", serde_json::json!({}));
            // what the page is sent, by kind
            let hook = "(function(){var W=window.WebSocket;window.__kinds=[];window.WebSocket=function(u,p){var w=new W(u,p);\
                w.addEventListener('message',function(e){try{window.__kinds.push(JSON.parse(e.data).type);}catch(x){}});return w;};\
                window.WebSocket.prototype=W.prototype;})();";
            b.call("Page.addScriptToEvaluateOnNewDocument", serde_json::json!({ "source": hook }));
            b.call("Page.navigate", serde_json::json!({ "url": format!("http://127.0.0.1:{}/", port) }));
            b.until("!!document.querySelector('input[type=file]') && !document.documentElement.classList.contains('crow-offline')", "the page");
            std::thread::sleep(std::time::Duration::from_millis(300));
            // the reader picks the file
            let doc = b.call("DOM.getDocument", serde_json::json!({}));
            let root = doc["result"]["root"]["nodeId"].clone();
            let input = b.call("DOM.querySelector", serde_json::json!({ "nodeId": root, "selector": "input[type=file]" }));
            let r = b.call("DOM.setFileInputFiles", serde_json::json!({ "files": [picked.to_string_lossy()], "nodeId": input["result"]["nodeId"] }));
            assert!(r["error"].is_null(), "the file is picked: {}", r);
            b.until("document.body.innerText.indexOf('got trades.csv symbol,qty') >= 0", "the file's arrival on the page");
            assert_eq!(b.eval("document.querySelector('.crow-file') && document.querySelector('.crow-file').textContent"), serde_json::json!("trades.csv"));
            // and hands it back
            b.eval("document.querySelector('[data-crow-click=back]').click(); 1");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let back = saved.join("back.csv");
            while !back.exists() {
                if std::time::Instant::now() > deadline {
                    let listed: Vec<String> = std::fs::read_dir(&saved).map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
                    let page = b.eval("document.body.innerText");
                    let kinds = b.eval("window.__kinds.join(',')");
                    panic!("the download never arrived: saved {:?}, page {}, messages {}", listed, page, kinds);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert_eq!(std::fs::read_to_string(&back).unwrap(), body, "the file came back as it went");
        })
        .await
        .unwrap();
    }

    /// The page the browser test serves at step `n`: a text, a chart's mark
    /// that moves, a drawing's shape that changes kind, rows that come and
    /// go, an element that changes kind, text with an entity, a bound field,
    /// and a long list that makes the changes smaller than the page.
    fn changing_page(n: i64) -> CrowVNode {
        let shape = if n % 2 == 0 { "<circle cx=\"5\" cy=\"5\" r=\"2\"/>" } else { "<rect x=\"4\" y=\"4\" width=\"2\" height=\"2\"/>" };
        CrowVNode::div(vec![
            CrowVNode::p(vec![CrowVNode::text(&format!("n={}", n))]),
            CrowVNode::button(vec![CrowVNode::text("inc")]).on_click("inc"),
            CrowVNode::raw(&format!("<svg viewBox=\"0 0 10 10\" width=\"100\"><line class=\"head\" x1=\"{0}\" x2=\"{0}\" y1=\"0\" y2=\"10\"/>{1}</svg>", n, shape)),
            CrowVNode::raw(&format!("<table><thead><tr><th>i</th></tr></thead><tbody>{}</tbody></table>", (0..n % 4).map(|i| format!("<tr><td>{}</td></tr>", i)).collect::<String>())),
            if n % 2 == 0 { CrowVNode::tag("span", vec![CrowVNode::text("even")]) } else { CrowVNode::tag("b", vec![CrowVNode::text("odd")]) },
            CrowVNode::raw(&format!("<p>{} &amp; more</p>", n)),
            CrowVNode::input().attr("data-crow-input", "__field:name").attr("value", &format!("v{}", n)),
            CrowVNode::tag("ul", (0..200).map(|i| CrowVNode::tag("li", vec![CrowVNode::text(&format!("row {}", i))])).collect()),
        ])
    }

    /// A page's shape as the browser test compares it: tags (SVG marked),
    /// sorted attributes, texts.
    fn shape_of(node: &CrowVNode, in_svg: bool) -> String {
        match node {
            CrowVNode::Text(t) => serde_json::to_string(t).unwrap_or_default(),
            CrowVNode::Raw(r) if r.starts_with("<!--") => "#".to_string(),
            CrowVNode::Raw(r) => serde_json::to_string(&r.replace("&amp;", "&")).unwrap_or_default(),
            CrowVNode::Element { tag, attrs, children } => {
                let svg = in_svg || tag.eq_ignore_ascii_case("svg");
                let mut a: Vec<String> = attrs.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                a.sort();
                let kids: Vec<String> = children.iter().map(|c| shape_of(c, svg)).collect();
                format!("{}{}[{}]({})", if svg { "svg:" } else { "" }, tag.to_ascii_lowercase(), a.join(","), kids.join(" "))
            }
        }
    }

    /// The live script in a real browser: a page driven through Chrome's
    /// DevTools protocol takes each render as changes and shows exactly what
    /// the server rendered — a text, a moved mark, a drawing's shape made in
    /// the drawing, rows through a table, an element that changed kind, text
    /// with an entity; a field being typed in keeps its draft; and a page
    /// knocked out of shape asks for the whole page and recovers.
    /// `cargo test -p crow_web --features live,http the_browser_applies_changes
    /// -- --ignored` (CHROME names another binary)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn the_browser_applies_changes() {
        let mut initial = CrowLiveState::new();
        initial.set("n", serde_json::json!(0));
        let render = |s: &CrowLiveState| changing_page(s.get_i64("n", 0));
        let handle = |s: &mut CrowLiveState, event: &str, _: Option<&str>| {
            if event == "inc" {
                let n = s.get_i64("n", 0);
                s.set("n", serde_json::json!(n + 1));
            }
        };
        let router = CrowLiveView::new("/_ws/", initial, render, handle).into_router("/");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            let Some(mut b) = Browser::open("changes") else { return };
            /// Wait until the page shows what the server renders at `n`; the
            /// page's shape and the rendered one.
            fn settle(b: &mut Browser, n: i64) -> (String, String) {
                const SHAPE: &str = "(function(){function shape(n){if(n.nodeType===3)return JSON.stringify(n.nodeValue);if(n.nodeType===8)return '#';\
                    var svg=n.namespaceURI==='http://www.w3.org/2000/svg',a=[];for(var i=0;i<n.attributes.length;i++){var at=n.attributes[i];\
                    if(at.name.indexOf('data-crow-busy')!==0)a.push(at.name+'='+at.value);}a.sort();\
                    return (svg?'svg:':'')+n.nodeName.toLowerCase()+'['+a.join(',')+']('+Array.prototype.map.call(n.childNodes,shape).join(' ')+')';}\
                    var r=document.getElementById('crow-root');return r&&r.firstChild?shape(r.firstChild):'';})()";
                let want = shape_of(&changing_page(n).as_built(), false);
                let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    let got = b.eval(SHAPE).as_str().unwrap_or("").to_string();
                    if got == want || std::time::Instant::now() > until {
                        return (got, want);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            let kinds = |b: &mut Browser| b.eval("window.__kinds.join(',')").as_str().unwrap_or("").to_string();
            // what the page is sent, by kind, from its first message on
            let hook = "(function(){var W=window.WebSocket;window.__kinds=[];window.WebSocket=function(u,p){var w=new W(u,p);\
                w.addEventListener('message',function(e){try{window.__kinds.push(JSON.parse(e.data).type);}catch(x){}});return w;};\
                window.WebSocket.prototype=W.prototype;})();";
            b.call("Page.enable", serde_json::json!({}));
            b.call("Page.addScriptToEvaluateOnNewDocument", serde_json::json!({ "source": hook }));
            b.call("Page.navigate", serde_json::json!({ "url": format!("http://127.0.0.1:{}/", port) }));
            let (got, want) = settle(&mut b, 0);
            assert_eq!(got, want, "the first page");
            // the page's socket is up (its session token arrived)
            b.until("window.__kinds && window.__kinds.indexOf('session') >= 0", "the socket");
            const CLICK: &str = "document.querySelector('[data-crow-click=inc]').click();";
            for n in 1..=5 {
                b.eval(&format!("{}1", CLICK));
                let (got, want) = settle(&mut b, n);
                assert_eq!(got, want, "after click {}", n);
            }
            let seen = kinds(&mut b);
            assert!(!seen.contains("html"), "every render came as changes: {}", seen);
            assert_eq!(seen.matches("patches").count(), 5, "{}", seen);
            // a field being typed in keeps its draft while its value changes
            b.eval(&format!("var f=document.querySelector('input');f.focus();f.value='draft';{}1", CLICK));
            let (got, want) = settle(&mut b, 6);
            assert_eq!(got, want, "with a draft");
            assert_eq!(b.eval("document.activeElement===document.querySelector('input') && document.querySelector('input').value"), serde_json::json!("draft"));
            // once it is left, it shows what the server rendered
            b.eval(&format!("document.querySelector('input').blur();{}1", CLICK));
            let (got, want) = settle(&mut b, 7);
            assert_eq!(got, want, "after the draft");
            assert_eq!(b.eval("document.querySelector('input').value"), serde_json::json!("v7"));
            // a page knocked out of shape asks for the whole page, and recovers
            b.eval(&format!("var r=document.getElementById('crow-root').firstChild;r.removeChild(r.firstChild);{}1", CLICK));
            let (got, want) = settle(&mut b, 8);
            assert_eq!(got, want, "after the whole page came back");
            let seen = kinds(&mut b);
            assert!(seen.ends_with(",html") || seen.contains(",html,"), "the whole page came back: {}", seen);
        })
        .await
        .unwrap();
    }

    /// A page that signs its reader in and out: the socket's sign message is
    /// traded once for an HttpOnly cookie; a page opened with the cookie
    /// knows the reader; a sign-out ends the sign-in in another tab at once,
    /// and the old cookie is refused afterwards (spec: *Who the reader is*)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reader_signs_in_and_out_through_a_cookie() {
        let render = |s: &CrowLiveState| {
            let r = s.reader();
            CrowVNode::div(vec![
                CrowVNode::p(vec![CrowVNode::text(&format!("reader={}", r.get("name").cloned().unwrap_or_default()))]),
                CrowVNode::button(vec![CrowVNode::text("in")]).on_click("log_in"),
                CrowVNode::button(vec![CrowVNode::text("out")]).on_click("log_out"),
            ])
        };
        let handle = |s: &mut CrowLiveState, event: &str, _: Option<&str>| match event {
            "log_in" => {
                let mut r = std::collections::BTreeMap::new();
                r.insert("name".to_string(), "ada".to_string());
                s.sign_in(&r);
            }
            "log_out" => s.sign_out(),
            _ => {}
        };
        let router = CrowLiveView::new("/_ws/", CrowLiveState::new(), render, handle).into_router("/");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            use tungstenite::client::IntoClientRequest;
            use tungstenite::Message as M;
            type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;
            let http = |request: String| -> String {
                let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                c.write_all(request.as_bytes()).unwrap();
                let mut out = String::new();
                c.read_to_string(&mut out).unwrap();
                out
            };
            let get = |cookie: &str| -> String {
                let cookie_line = if cookie.is_empty() { String::new() } else { format!("Cookie: crow_reader={}\r\n", cookie) };
                http(format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{}Connection: close\r\n\r\n", port, cookie_line))
            };
            let post = |url: &str| -> String {
                http(format!("POST {} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", url, p = port))
            };
            let socket_for = |page: &str, cookie: &str| -> Ws {
                let token: String = page.split("pageToken = '").nth(1).unwrap().chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                let mut req = format!("ws://127.0.0.1:{}/_ws/?p={}", port, token).into_client_request().unwrap();
                if !cookie.is_empty() {
                    req.headers_mut().insert("Cookie", format!("crow_reader={}", cookie).parse().unwrap());
                }
                let (mut ws, _) = tungstenite::connect(req).unwrap();
                if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                    s.set_read_timeout(Some(std::time::Duration::from_millis(3000))).unwrap();
                }
                ws
            };
            // the next message of a kind
            let next = |ws: &mut Ws, kind: &str| -> serde_json::Value {
                loop {
                    match ws.read() {
                        Ok(M::Text(t)) => {
                            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                            if m["type"] == kind {
                                return m;
                            }
                        }
                        other => panic!("waiting for a '{}' message, got {:?}", kind, other),
                    }
                }
            };
            let first = get("");
            assert!(first.contains("reader=<"), "nobody yet: {}", first);
            let mut ws = socket_for(&first, "");
            ws.send(M::Text(r#"{"event":"log_in","seq":1}"#.into())).unwrap();
            let url = next(&mut ws, "sign")["url"].as_str().unwrap().to_string();
            assert!(url.starts_with("/_sign/?t="), "{}", url);
            let answer = post(&url);
            assert!(answer.starts_with("HTTP/1.1 204"), "{}", answer);
            let set = answer.lines().find(|l| l.to_ascii_lowercase().starts_with("set-cookie:")).expect("a cookie");
            assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Max-Age=2592000"), "{}", set);
            let cookie = set.split("crow_reader=").nth(1).unwrap().split(';').next().unwrap().to_string();
            assert!(post(&url).starts_with("HTTP/1.1 410"), "a ticket is good once");
            // a page opened with the cookie knows its reader; so does a
            // second tab's own session
            assert!(get(&cookie).contains("reader=ada"), "the cookie signs the next page in");
            let other_page = get(&cookie);
            let mut other = socket_for(&other_page, &cookie);
            let _ = next(&mut other, "session");
            // signing out in the first tab ends the sign-in in the second
            ws.send(M::Text(r#"{"event":"log_out","seq":2}"#.into())).unwrap();
            let out_url = next(&mut ws, "sign")["url"].as_str().unwrap().to_string();
            let cleared = post(&out_url);
            assert!(cleared.to_ascii_lowercase().contains("set-cookie: crow_reader=; path=/; max-age=0"), "{}", cleared);
            let mut seen_signed_out = false;
            for _ in 0..5 {
                match other.read() {
                    Ok(M::Text(t)) if t.contains("reader=\u{3c}") || t.contains("\"text\":\"reader=\"") => {
                        seen_signed_out = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            assert!(seen_signed_out, "the other tab is signed out at once");
            // and the old cookie signs nobody in
            assert!(get(&cookie).contains("reader=<"), "an ended sign-in's cookie is refused");
        })
        .await
        .unwrap();
    }

    /// A file field: the page asks for a ticket with what the file is, sends
    /// the bytes by it, and the field's variable holds the file — its name
    /// on the page, its copy on the server; a file over the limit is
    /// refused before it is sent, with the limit named; a field the page did
    /// not draw takes nothing (spec: *Files in and out*)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_file_picked_on_the_page_lands_in_its_field() {
        let mut initial = CrowLiveState::new();
        initial.set("upload", serde_json::json!({}));
        let render = |s: &CrowLiveState| {
            let name = s.get("upload").and_then(|u| u["name"].as_str()).unwrap_or("").to_string();
            CrowVNode::div(vec![CrowVNode::field_typed("Trades", "upload", &name, "file", 0)])
        };
        let handle = |_: &mut CrowLiveState, _: &str, _: Option<&str>| {};
        let router = CrowLiveView::new("/_ws/", initial, render, handle).into_router("/");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            use tungstenite::Message as M;
            // a refusal that does not read the body may reset the connection
            // before its answer is read: that is a refusal too
            let http = |request: String, body: &[u8]| -> String {
                let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                c.write_all(request.as_bytes()).unwrap();
                let _ = c.write_all(body);
                let mut out = String::new();
                match c.read_to_string(&mut out) {
                    Ok(_) => out,
                    Err(e) => format!("reset: {}", e),
                }
            };
            let post = |url: &str, body: &[u8]| {
                http(format!("POST {} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", url, body.len(), p = port), body)
            };
            type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;
            // the next message of a kind ("render": a whole page or its changes)
            fn next(ws: &mut Ws, kind: &str) -> serde_json::Value {
                loop {
                    match ws.read() {
                        Ok(M::Text(t)) => {
                            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                            if m["type"] == kind || (kind == "render" && (m["type"] == "html" || m["type"] == "patches")) {
                                return m;
                            }
                        }
                        other => panic!("waiting for '{}', got {:?}", kind, other),
                    }
                }
            }
            let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}/_ws/", port)).unwrap();
            if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                s.set_read_timeout(Some(std::time::Duration::from_millis(3000))).unwrap();
            }
            let page = next(&mut ws, "html");
            assert!(page["html"].as_str().unwrap().contains("data-crow-upload=\"__field:upload\""), "{}", page);
            // a file of 9 bytes, sent by its ticket
            let ask = serde_json::json!({ "event": "__upload", "value": serde_json::json!(["__field:upload", "t.csv", "text/csv", 9]).to_string(), "seq": 1 });
            ws.send(M::Text(ask.to_string().into())).unwrap();
            let url = next(&mut ws, "upload")["url"].as_str().unwrap().to_string();
            assert!(post(&url, b"a,b\n1,2\n\n").starts_with("HTTP/1.1 204"));
            let again = post(&url, b"a,b\n1,2\n\n");
            assert!(again.starts_with("HTTP/1.1 410") || again.starts_with("reset"), "a ticket is good once: {}", again);
            let shown = next(&mut ws, "render");
            assert!(shown.to_string().contains("t.csv"), "the page shows the file picked: {}", shown);
            // the file's copy is on the server, where the variable says
            let kept: Vec<_> = std::fs::read_dir(".crow/uploads").unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("t.csv").exists()).collect();
            assert!(!kept.is_empty(), "the copy is kept");
            // too big: refused before it is sent, the limit named
            let big = serde_json::json!({ "event": "__upload", "value": serde_json::json!(["__field:upload", "huge.zip", "application/zip", 50_000_000u64]).to_string(), "seq": 2 });
            ws.send(M::Text(big.to_string().into())).unwrap();
            let told = next(&mut ws, "render");
            assert!(told.to_string().contains("files up to 10 MB"), "{}", told);
            // a field the page did not draw takes nothing
            let forged = serde_json::json!({ "event": "__upload", "value": serde_json::json!(["__field:reader", "x", "text/plain", 1]).to_string(), "seq": 3 });
            ws.send(M::Text(forged.to_string().into())).unwrap();
            let mut offered_one = false;
            loop {
                match ws.read() {
                    Ok(M::Text(t)) if t.contains("\"upload\"") && t.contains("_upload/") => offered_one = true,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            assert!(!offered_one, "no ticket for a field the page did not draw");
            // a wrong size is refused
            let (mut ws2, _) = tungstenite::connect(format!("ws://127.0.0.1:{}/_ws/", port)).unwrap();
            if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws2.get_mut() {
                s.set_read_timeout(Some(std::time::Duration::from_millis(3000))).unwrap();
            }
            let ask = serde_json::json!({ "event": "__upload", "value": serde_json::json!(["__field:upload", "t.csv", "text/csv", 3]).to_string(), "seq": 1 });
            ws2.send(M::Text(ask.to_string().into())).unwrap();
            let url2 = loop {
                if let Ok(M::Text(t)) = ws2.read() {
                    let m: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if m["type"] == "upload" {
                        break m["url"].as_str().unwrap().to_string();
                    }
                }
            };
            assert!(post(&url2, b"too many bytes").starts_with("HTTP/1.1 400"));
        })
        .await
        .unwrap();
    }

    /// A served counter page, driven the way a browser drives it
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_served_page_session_end_to_end() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, counter_router()).await.unwrap() });
        tokio::task::spawn_blocking(move || drive_counter(port)).await.unwrap();
    }

    fn drive_counter(port: u16) {
        use std::io::{Read, Write};
        use tungstenite::client::IntoClientRequest;
        use tungstenite::Message as M;

        let mut http = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(http, "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port).unwrap();
        let mut page = String::new();
        http.read_to_string(&mut page).unwrap();
        let token: String = page.split("pageToken = '").nth(1).expect("the page names its session").chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let first_tick: String = page.split("tick=").nth(1).unwrap().chars().take_while(|c| c.is_ascii_digit()).collect();

        let open = |url: String, origin: Option<&str>| {
            let mut req = url.into_client_request().unwrap();
            if let Some(o) = origin {
                req.headers_mut().insert("Origin", o.parse().unwrap());
            }
            tungstenite::connect(req)
        };
        let set_timeout = |ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>, ms: u64| {
            if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                s.set_read_timeout(Some(std::time::Duration::from_millis(ms))).unwrap();
            }
        };
        // the next render, as the page shows it after applying it
        let next_html = |ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>, shown: &mut CrowVNode| -> Option<String> {
            match ws.read() {
                Ok(M::Text(t)) => {
                    let m = serde_json::from_str::<serde_json::Value>(&t).ok()?;
                    follow_page(shown, &m).then(|| shown.to_html())
                }
                _ => None,
            }
        };
        let mut shown = served_root(&page);

        // the page's socket adopts the session the page request rendered:
        // nothing is rendered or sent a second time
        let (mut ws, _) = open(format!("ws://127.0.0.1:{}/_ws/?p={}", port, token), Some(&format!("http://127.0.0.1:{}", port))).expect("the page's own socket opens");
        set_timeout(&mut ws, 400);
        assert!(next_html(&mut ws, &mut shown).is_none(), "an adopted session sends nothing until something changes");

        // a click is answered by one render, and is not a tick
        set_timeout(&mut ws, 5000);
        ws.send(M::Text(r#"{"event":"inc"}"#.into())).unwrap();
        let h = next_html(&mut ws, &mut shown).expect("the click's render");
        assert!(h.contains(&format!("n=1 name= tick={}", first_tick)), "{} (the page's tick was {})", h, first_tick);

        // only what the page offered is accepted
        set_timeout(&mut ws, 400);
        ws.send(M::Text(r#"{"event":"secret"}"#.into())).unwrap();
        ws.send(M::Text(r#"{"event":"set","value":"[99]"}"#.into())).unwrap();
        ws.send(M::Text(r#"{"event":"__field:n","value":"500"}"#.into())).unwrap();
        assert!(next_html(&mut ws, &mut shown).is_none(), "refused events change nothing");
        set_timeout(&mut ws, 5000);
        ws.send(M::Text(r#"{"event":"set","value":"[7]"}"#.into())).unwrap();
        assert!(next_html(&mut ws, &mut shown).unwrap().contains("n=7 "));

        // a burst of input is applied together and answered once
        for i in 1..=20 {
            ws.write(M::Text(format!(r#"{{"event":"__field:name","value":"v{}"}}"#, i).into())).unwrap();
        }
        ws.flush().unwrap();
        let mut renders = 0;
        loop {
            let h = next_html(&mut ws, &mut shown).expect("the burst's render");
            renders += 1;
            if h.contains("name=v20 ") {
                break;
            }
        }
        assert!(renders <= 2, "20 queued inputs answered by {} renders", renders);

        // a reconnect, whose token is spent, gets a fresh session and is
        // told at once what it holds: the page never shows a stale session
        let (mut again, _) = open(format!("ws://127.0.0.1:{}/_ws/?p={}", port, token), None).expect("a reconnect opens");
        set_timeout(&mut again, 5000);
        let mut shown_again = CrowVNode::text("");
        assert!(next_html(&mut again, &mut shown_again).expect("the fresh session's render, unasked").contains("n=0 "));

        // a page on another site cannot open the socket
        match open(format!("ws://127.0.0.1:{}/_ws/", port), Some("https://evil.example")) {
            Err(tungstenite::Error::Http(resp)) => assert_eq!(resp.status(), 403),
            other => panic!("a foreign page's socket must be refused, got {:?}", other.map(|_| "an open socket")),
        }
    }

    #[test]
    fn a_shared_derivation_is_computed_once_for_every_session() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        static SPEC: DeriveSpec = DeriveSpec { name: "test_shared_universe", facts: &[], derives: &[], tables: &[], now: false, external: false, fresh: 0.0, shared: true };
        let runs = AtomicUsize::new(0);
        let compute = |_: &CrowLiveState| { runs.fetch_add(1, SeqCst); 42i64 };
        let mut s1 = CrowLiveState::new();
        s1.next_tick();
        let mut s2 = CrowLiveState::new();
        s2.next_tick();
        assert_eq!(s1.derive(&SPEC, &compute, |a, b| a == b), 42);
        assert_eq!(s2.derive(&SPEC, &compute, |a, b| a == b), 42);
        assert_eq!(runs.load(SeqCst), 1, "the second session reads the app's value");
    }

    #[test]
    fn a_table_write_in_one_session_is_seen_by_every_session() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        static SPEC: DeriveSpec = DeriveSpec { name: "test_session_rows", facts: &["sym"], derives: &[], tables: &["test_trades"], now: false, external: false, fresh: 0.0, shared: false };
        let runs = AtomicUsize::new(0);
        let compute = |_: &CrowLiveState| { runs.fetch_add(1, SeqCst); 1i64 };
        let mut s1 = CrowLiveState::new();
        s1.set("sym", serde_json::json!("IMRN"));
        s1.next_tick();
        let mut s2 = s1.clone();
        s2.next_tick();
        s1.derive(&SPEC, &compute, |a, b| a == b);
        s2.derive(&SPEC, &compute, |a, b| a == b);
        assert_eq!(runs.load(SeqCst), 2, "a derivation over a session fact is per session");
        s1.derive(&SPEC, &compute, |a, b| a == b);
        assert_eq!(runs.load(SeqCst), 2, "nothing changed: cached");
        s1.note_table_write("test_trades");
        s2.derive(&SPEC, &compute, |a, b| a == b);
        assert_eq!(runs.load(SeqCst), 3, "the other session's write reaches this one");
    }

    /// Wait until no recompute of `name` is in flight (or give up).
    fn settle(state: &CrowLiveState, name: &str) {
        for _ in 0..400 {
            if !state.derive_running(name) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the recompute of {} never landed", name);
    }

    /// Whether a session was woken, waiting up to `ms`.
    fn woken(state: &CrowLiveState, ms: u64) -> bool {
        let h = state.handle.clone().expect("a served session has a handle");
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async move {
            tokio::time::timeout(std::time::Duration::from_millis(ms), h.wake.notified()).await.is_ok()
        })
    }

    mod slow_rows {
        use super::super::*;
        pub static SPEC: DeriveSpec = DeriveSpec { name: "test_bg_rows", facts: &["n"], derives: &[], tables: &["test_bg_table"], now: false, external: false, fresh: 0.0, shared: false };
        pub static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        pub fn body(state: &CrowLiveState) -> i64 {
            RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(80));
            state.get_i64("n", 0) * 10
        }
        pub fn read(state: &CrowLiveState) -> i64 {
            state.derive_bg(&SPEC, body, |a, b| a == b, None, read)
        }
    }

    #[test]
    fn a_render_goes_ahead_with_the_last_value_while_a_query_recomputes() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut init = CrowLiveState::new();
        init.set("n", serde_json::json!(1));
        let mut s = start_session(&init);
        s.next_tick();
        assert_eq!(slow_rows::read(&s), 10, "outside a render the value is computed");
        s.set("n", serde_json::json!(2));
        s.begin_render();
        let t0 = std::time::Instant::now();
        assert_eq!(slow_rows::read(&s), 10, "the render reads the last value");
        assert!(t0.elapsed() < std::time::Duration::from_millis(40), "the render did not wait for the query");
        s.end_render();
        assert!(s.derive_running("test_bg_rows"), "the recompute is in flight");
        settle(&s, "test_bg_rows");
        assert!(woken(&s, 500), "the landed value wakes the session");
        s.begin_render();
        assert_eq!(slow_rows::read(&s), 20, "the next render has the new value");
        s.end_render();
        assert_eq!(slow_rows::RUNS.load(SeqCst), 2);
    }

    mod pending_rows {
        use super::super::*;
        pub static SPEC: DeriveSpec = DeriveSpec { name: "test_bg_pending", facts: &[], derives: &[], tables: &["test_bg_pending_t"], now: false, external: true, fresh: 60.0, shared: false };
        pub fn body(_: &CrowLiveState) -> i64 {
            std::thread::sleep(std::time::Duration::from_millis(30));
            7
        }
        pub fn read(state: &CrowLiveState) -> i64 {
            state.derive_bg(&SPEC, body, |a, b| a == b, Some((|| -1) as fn() -> i64), read)
        }
    }

    #[test]
    fn a_first_render_shows_pending_and_the_value_follows() {
        let mut s = start_session(&CrowLiveState::new());
        s.next_tick();
        s.begin_render();
        assert_eq!(pending_rows::read(&s), -1, "before its first value a query reads as pending");
        s.end_render();
        settle(&s, "test_bg_pending");
        s.begin_render();
        assert_eq!(pending_rows::read(&s), 7);
        s.end_render();
    }

    mod own_rows {
        use super::super::*;
        pub static SPEC: DeriveSpec = DeriveSpec { name: "test_bg_own", facts: &[], derives: &[], tables: &["test_bg_own_t"], now: false, external: false, fresh: 0.0, shared: false };
        pub static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        pub fn body(_: &CrowLiveState) -> usize {
            RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
        }
        pub fn read(state: &CrowLiveState) -> usize {
            state.derive_bg(&SPEC, body, |a, b| a == b, None, read)
        }
    }

    #[test]
    fn a_clicks_own_write_is_in_its_answer() {
        let mut s = start_session(&CrowLiveState::new());
        s.next_tick();
        assert_eq!(own_rows::read(&s), 1);
        // the handler writes the table; the render that answers it recomputes
        // the derivation in place instead of catching up later
        s.note_table_write("test_bg_own_t");
        s.begin_render();
        assert_eq!(own_rows::read(&s), 2, "the render shows the event's own write");
        s.end_render();
        assert!(!s.derive_running("test_bg_own"), "nothing was left to catch up");
    }

    mod shared_rows {
        use super::super::*;
        pub static SPEC: DeriveSpec = DeriveSpec { name: "test_bg_shared", facts: &[], derives: &[], tables: &["test_bg_shared_t"], now: false, external: false, fresh: 0.0, shared: true };
        pub static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        pub fn body(_: &CrowLiveState) -> usize {
            std::thread::sleep(std::time::Duration::from_millis(60));
            RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
        }
        pub fn read(state: &CrowLiveState) -> usize {
            state.derive_bg(&SPEC, body, |a, b| a == b, None, read)
        }
    }

    #[test]
    fn a_write_wakes_every_reader_and_a_shared_query_runs_once() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut writer = start_session(&CrowLiveState::new());
        let mut a = start_session(&CrowLiveState::new());
        let mut b = start_session(&CrowLiveState::new());
        for s in [&mut writer, &mut a, &mut b] {
            s.next_tick();
        }
        assert_eq!(shared_rows::read(&a), 1);
        // both readers' pages read the derivation over the table
        for s in [&a, &b] {
            s.begin_render();
            shared_rows::read(s);
            s.end_render();
        }
        let _ = woken(&a, 1);
        let _ = woken(&b, 1);
        writer.note_table_write("test_bg_shared_t");
        assert!(woken(&a, 500) && woken(&b, 500), "both readers are woken by the write");
        assert!(!woken(&writer, 50), "the writer answers its own event");
        // both render at once: one recompute for the app
        for s in [&a, &b] {
            s.begin_render();
            assert_eq!(shared_rows::read(s), 1, "the last value while it recomputes");
            s.end_render();
        }
        settle(&a, "test_bg_shared");
        assert_eq!(shared_rows::RUNS.load(SeqCst), 2, "one recompute served both sessions");
        assert!(woken(&a, 500) && woken(&b, 500), "the landed value wakes both readers");
        a.begin_render();
        assert_eq!(shared_rows::read(&a), 2);
        a.end_render();
    }

    #[test]
    fn a_view_that_read_a_value_still_recomputing_is_marked() {
        let mut init = CrowLiveState::new();
        init.set("n", serde_json::json!(5));
        let mut s = start_session(&init);
        s.next_tick();
        static SPEC: DeriveSpec = DeriveSpec { name: "test_bg_marked", facts: &["n"], derives: &[], tables: &["test_bg_marked_t"], now: false, external: false, fresh: 0.0, shared: false };
        fn body(state: &CrowLiveState) -> i64 {
            std::thread::sleep(std::time::Duration::from_millis(30));
            state.get_i64("n", 0)
        }
        fn read(state: &CrowLiveState) -> i64 {
            state.derive_bg(&SPEC, body, |a, b| a == b, None, read)
        }
        read(&s);
        s.set("n", serde_json::json!(6));
        s.begin_render();
        let outer = s.view_enter();
        let inner = s.view_enter();
        read(&s);
        let inner_node = s.view_exit(inner, CrowVNode::tag("p", vec![]));
        let outer_node = s.view_exit(outer, CrowVNode::tag("div", vec![inner_node]));
        s.end_render();
        let html = outer_node.to_html();
        assert!(html.contains("<p data-crow-stale"), "the view that read it is marked: {}", html);
        assert!(!html.contains("<div data-crow-stale"), "its parent is not: {}", html);
        settle(&s, "test_bg_marked");
    }

    #[test]
    fn a_click_and_a_render_each_read_one_moment() {
        let mut s = start_session(&CrowLiveState::new());
        let mut offers = Offers::default();
        offers.clicks.insert(("go".to_string(), None));
        let handler = |st: &mut CrowLiveState, _: &str, _: Option<&str>| {
            let a = crow_runtime::moments::now();
            std::thread::sleep(std::time::Duration::from_millis(3));
            let b = crow_runtime::moments::now();
            st.set("same", serde_json::json!(a == b));
        };
        apply_event(&mut s, &serde_json::json!({ "event": "go" }).to_string(), &[&offers], &handler);
        assert_eq!(s.get("same"), Some(&serde_json::json!(true)), "the handler read one moment");
        let vnode = s.render_with(&|_: &CrowLiveState| {
            let a = crow_runtime::moments::now();
            std::thread::sleep(std::time::Duration::from_millis(3));
            let b = crow_runtime::moments::now();
            CrowVNode::text(if a == b { "one" } else { "two" })
        });
        assert_eq!(vnode.to_html(), "one", "the render read one moment");
    }

    #[test]
    fn a_handler_lands_as_one_or_not_at_all() {
        let mut init = CrowLiveState::new();
        init.set("a", serde_json::json!(1));
        init.set("b", serde_json::json!(1));
        let mut s = start_session(&init);
        // commits: both writes and the notice land together
        let mut copy = s.working_copy();
        copy.set("a", serde_json::json!(2));
        copy.notice("saved".to_string());
        assert_eq!(s.get("a"), Some(&serde_json::json!(1)), "nothing lands before the commit");
        assert_eq!(s.commit(copy), None);
        assert_eq!(s.get("a"), Some(&serde_json::json!(2)));
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("saved"));
        // stops: nothing lands, the reason is the notice
        let mut copy = s.working_copy();
        copy.set("a", serde_json::json!(3));
        copy.set("b", serde_json::json!(3));
        copy.notice("saved again".to_string());
        copy.stop("apply stopped: no quote".to_string());
        assert_eq!(s.commit(copy), Some("apply stopped: no quote".to_string()));
        assert_eq!((s.get("a"), s.get("b")), (Some(&serde_json::json!(2)), Some(&serde_json::json!(1))), "a stop lands nothing");
        assert_eq!(s.notice.as_ref().map(|n| n.0.as_str()), Some("apply stopped: no quote"));
    }

    #[test]
    fn a_table_write_reaches_other_readers_when_the_handler_commits() {
        static SPEC: DeriveSpec = DeriveSpec { name: "test_commit_rows", facts: &[], derives: &[], tables: &["test_commit_t"], now: false, external: false, fresh: 0.0, shared: true };
        fn read(state: &CrowLiveState) -> i64 {
            state.derive(&SPEC, |_| 1, |a, b| a == b)
        }
        let mut writer = start_session(&CrowLiveState::new());
        let mut reader = start_session(&CrowLiveState::new());
        writer.next_tick();
        reader.next_tick();
        reader.begin_render();
        read(&reader);
        reader.end_render();
        let _ = woken(&reader, 1);
        let copy = writer.working_copy();
        copy.note_table_write("test_commit_t");
        assert!(!woken(&reader, 50), "an uncommitted write wakes nobody");
        assert_eq!(writer.commit(copy), None);
        assert!(woken(&reader, 500), "the commit wakes every reader of the table");
    }

    #[test]
    fn a_page_without_a_notice_of_its_own_gets_the_toast() {
        let mut s = start_session(&CrowLiveState::new());
        s.notice("bought 100 IMRN".to_string());
        let plain = s.render_with(&|_: &CrowLiveState| CrowVNode::tag("main", vec![]));
        assert!(plain.to_html().contains("class=\"crow-notice\""), "{}", plain.to_html());
        let own = s.render_with(&|st: &CrowLiveState| CrowVNode::tag("main", vec![CrowVNode::text(&st.notice_record().0)]));
        assert!(!own.to_html().contains("crow-notice"), "a page that shows the notice gets no toast: {}", own.to_html());
    }

    /// A kept session comes back to a restarted program for its token, once:
    /// its facts where the program still declares them with the same shape,
    /// the rest from their declared values; a token is never a path
    #[test]
    fn a_kept_session_comes_back_once_with_the_facts_that_still_fit() {
        let dir = std::env::temp_dir().join(format!("crow_kept_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut before = CrowLiveState::new();
        before.set("day", serde_json::json!("2026-09-04"));
        before.set("speed", serde_json::json!(10.0));
        before.set("mode", serde_json::json!("replay"));
        keep_session_in(&dir, "abc123", &before);
        assert!(dir.join(".gitignore").exists(), "the directory ignores itself");
        // the program changed: speed is now text, mode is gone
        let mut init = CrowLiveState::new();
        init.set("day", serde_json::json!(""));
        init.set("speed", serde_json::json!("1x"));
        let back = kept_session_in(&dir, "abc123", &init).expect("the tab's session was kept");
        assert_eq!(back.get_str("day", ""), "2026-09-04", "a fact that still fits comes back");
        assert_eq!(back.get_str("speed", ""), "1x", "a fact whose shape changed starts from its declared value");
        assert!(back.get("mode").is_none(), "a fact the program no longer declares stays behind");
        assert!(kept_session_in(&dir, "abc123", &init).is_none(), "good once");
        assert!(kept_path(&dir, "../../etc/passwd").is_none(), "a token is never a path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bound_fields_keep_the_variable_shape() {
        use serde_json::json;
        assert_eq!(coerce_field_value(Some(&json!(0)), "72"), Some(json!(72)));
        assert_eq!(coerce_field_value(Some(&json!(0)), "72.9"), Some(json!(72)));
        assert_eq!(coerce_field_value(Some(&json!(0)), "abc"), None);
        assert_eq!(coerce_field_value(Some(&json!(1.5)), "2.25"), Some(json!(2.25)));
        assert_eq!(coerce_field_value(Some(&json!(false)), "on"), Some(json!(true)));
        assert_eq!(coerce_field_value(Some(&json!("IMRN")), "AXTX"), Some(json!("AXTX")));
        assert_eq!(coerce_field_value(None, "7"), Some(json!("7")));
    }

    #[test]
    fn test_live_state() {
        let mut state = CrowLiveState::new();
        state.set("count", serde_json::json!(0));
        assert_eq!(state.get("count"), Some(&serde_json::json!(0)));
    }

    #[test]
    fn test_live_state_get_i64() {
        let mut state = CrowLiveState::new();
        state.set("count", serde_json::json!(42));
        assert_eq!(state.get_i64("count", 0), 42);
        assert_eq!(state.get_i64("missing", 99), 99);
    }

    #[test]
    fn test_live_state_get_str() {
        let mut state = CrowLiveState::new();
        state.set("name", serde_json::json!("crow"));
        assert_eq!(state.get_str("name", ""), "crow");
        assert_eq!(state.get_str("missing", "default"), "default");
    }

    #[test]
    fn test_live_state_default() {
        let state = CrowLiveState::default();
        assert!(state.data.is_empty());
    }

    #[test]
    fn test_live_js_contains_websocket() {
        let js = live_js("/ws/live");
        assert!(js.contains("WebSocket"));
        assert!(js.contains("/ws/live"));
        assert!(js.contains("data-crow-click"));
        // The default stylesheet ships in every page shell
        let shell = page_shell("<p>x</p>", "");
        assert!(shell.contains("--crow-bg"));
        assert!(shell.contains(".card{"));
        assert!(shell.contains("prefers-color-scheme"));
        assert!(js.contains("data-crow-input"));
        assert!(js.contains("data-crow-submit"));
        assert!(js.contains("crow-root"));
        assert!(js.contains("reconnect") || js.contains("reconDelay"));
    }

    #[test]
    fn test_page_shell() {
        let shell = page_shell("<h1>Hello</h1>", "<script>alert(1)</script>");
        assert!(shell.contains("<!DOCTYPE html>"));
        assert!(shell.contains("crow-root"));
        assert!(shell.contains("<h1>Hello</h1>"));
        assert!(shell.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn test_vnode_render_handle_pattern() {
        // Simulate the render/handle pattern
        let mut state = CrowLiveState::new();
        state.set("count", serde_json::json!(0));

        // Render function
        let render = |s: &CrowLiveState| -> CrowVNode {
            let count = s.get_i64("count", 0);
            CrowVNode::div(vec![
                CrowVNode::h1(vec![CrowVNode::text(&format!("Count: {}", count))]),
                CrowVNode::button(vec![CrowVNode::text("+")]).on_click("inc"),
                CrowVNode::button(vec![CrowVNode::text("-")]).on_click("dec"),
            ])
        };

        // Handle function
        let handle = |s: &mut CrowLiveState, event: &str, _value: Option<&str>| {
            let count = s.get_i64("count", 0);
            match event {
                "inc" => s.set("count", serde_json::json!(count + 1)),
                "dec" => s.set("count", serde_json::json!(count - 1)),
                _ => {}
            }
        };

        // Initial render
        let vnode1 = render(&state);
        let html1 = vnode1.to_html();
        assert!(html1.contains("Count: 0"));

        // Handle inc
        handle(&mut state, "inc", None);
        let vnode2 = render(&state);
        let html2 = vnode2.to_html();
        assert!(html2.contains("Count: 1"));

        // the render's changes are what moved
        let patches = CrowVNode::changes(&vnode1, &vnode2);
        assert!(!patches.is_empty());

        // Handle dec twice
        handle(&mut state, "dec", None);
        handle(&mut state, "dec", None);
        let vnode3 = render(&state);
        assert!(vnode3.to_html().contains("Count: -1"));
    }

    #[test]
    fn test_live_state_register_fetch() {
        let mut state = CrowLiveState::new();
        state.register_fetch("positions", "http://localhost:8000/api/positions");
        assert_eq!(state.sources.len(), 1);
        assert_eq!(state.sources[0].key, "positions");
        assert_eq!(state.sources[0].url, "http://localhost:8000/api/positions");
        assert!(state.sources[0].interval_secs.is_none());
    }

    #[test]
    fn test_live_state_register_poll() {
        let mut state = CrowLiveState::new();
        state.register_poll("prices", "http://localhost:8000/api/prices", 5);
        assert_eq!(state.sources.len(), 1);
        assert_eq!(state.sources[0].key, "prices");
        assert_eq!(state.sources[0].interval_secs, Some(5));
    }

    #[test]
    fn test_live_js_contains_change_handler() {
        let js = live_js("/ws/live");
        assert!(js.contains("data-crow-change"));
    }
}