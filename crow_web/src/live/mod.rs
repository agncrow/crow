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
}

impl std::fmt::Debug for DerivedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DerivedSlot(v{})", self.version)
    }
}

/// Server-side component state.
#[derive(Debug)]
pub struct CrowLiveState {
    pub data: HashMap<String, serde_json::Value>,
    /// one version per fact key, bumped by every set — what derivations
    /// compare their inputs against
    pub versions: HashMap<String, u64>,
    /// tables this program wrote, versioned the same way
    pub table_versions: HashMap<String, u64>,
    /// one per render: what "as fresh as the loop" means
    pub tick: u64,
    /// the derivation cache, per session
    derived: std::sync::Mutex<HashMap<String, DerivedSlot>>,
    /// Registered data sources (fetch/poll).
    pub sources: Vec<DataSource>,
    /// Session-scoped channel delivering later[] state deltas to the live loop.
    pub later_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<(String, serde_json::Value)>>>,
    /// Distinguishes sessions in the later[] in-flight registry.
    pub session_id: u64,
}

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

impl CrowLiveState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            versions: HashMap::new(),
            table_versions: HashMap::new(),
            tick: 0,
            derived: std::sync::Mutex::new(HashMap::new()),
            sources: Vec::new(),
            later_tx: None,
            session_id: 0,
        }
    }

    /// A table this program wrote: derivations over it are stale.
    pub fn note_table_write(&mut self, table: &str) {
        *self.table_versions.entry(table.to_string()).or_insert(0) += 1;
    }

    /// A render is a tick: derivations that read the clock, or external
    /// sources at fresh = 0, recompute once per tick.
    pub fn next_tick(&mut self) {
        self.tick += 1;
    }

    /// Read a derivation (derive-design.md §3.3): the cached value while
    /// every input it saw is unchanged, else `compute` runs and the value
    /// is cached with the versions it was computed from. Equal scalars
    /// keep their version (early cutoff). Inputs that are derivations must
    /// be refreshed by the caller first, so their versions are current.
    pub fn derive<T: Clone + Send + Sync + 'static>(
        &self,
        spec: &DeriveSpec,
        compute: impl Fn(&CrowLiveState) -> T,
        equal: impl Fn(&T, &T) -> bool,
    ) -> T {
        let now = crow_runtime::moments::now();
        {
            let cache = self.derived.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(slot) = cache.get(spec.name) {
                let valid = spec.facts.iter().all(|f| slot.facts_seen.iter().find(|(n, _)| n == f).map(|(_, v)| *v).unwrap_or(0) == self.versions.get(*f).copied().unwrap_or(0))
                    && spec.derives.iter().all(|d| slot.derives_seen.iter().find(|(n, _)| n == d).map(|(_, v)| *v).unwrap_or(0) == cache.get(*d).map(|s| s.version).unwrap_or(0))
                    && spec.tables.iter().all(|t| slot.tables_seen.iter().find(|(n, _)| n == t).map(|(_, v)| *v).unwrap_or(0) == self.table_versions.get(*t).copied().unwrap_or(0))
                    && (!spec.now || slot.tick_seen == self.tick)
                    && (!spec.external || if spec.fresh > 0.0 { now - slot.computed_at <= spec.fresh } else { slot.tick_seen == self.tick });
                if valid {
                    if let Some(v) = slot.value.downcast_ref::<T>() {
                        return v.clone();
                    }
                }
            }
        }
        let value = compute(self);
        let mut cache = self.derived.lock().unwrap_or_else(|p| p.into_inner());
        let (old_version, changed) = match cache.get(spec.name) {
            Some(slot) => (slot.version, !slot.value.downcast_ref::<T>().map(|old| equal(old, &value)).unwrap_or(false)),
            None => (0, true),
        };
        let derives_seen: Vec<(String, u64)> = spec.derives.iter().map(|d| (d.to_string(), cache.get(*d).map(|s| s.version).unwrap_or(0))).collect();
        cache.insert(spec.name.to_string(), DerivedSlot {
            value: std::sync::Arc::new(value.clone()),
            version: if changed { old_version + 1 } else { old_version },
            facts_seen: spec.facts.iter().map(|f| (f.to_string(), self.versions.get(*f).copied().unwrap_or(0))).collect(),
            derives_seen,
            tables_seen: spec.tables.iter().map(|t| (t.to_string(), self.table_versions.get(*t).copied().unwrap_or(0))).collect(),
            tick_seen: self.tick,
            computed_at: now,
        });
        value
    }

    /// Schedule a background fn for this session (skip-if-running). The fn
    /// runs against a snapshot of state on its own thread; the keys it
    /// changed are sent back to the live loop, which applies them and
    /// re-renders. State never mutates concurrently.
    pub fn spawn_later(&self, name: &str, f: fn(&mut CrowLiveState)) -> bool {
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
            f(&mut snapshot);
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
            *self.versions.entry(key.to_string()).or_insert(0) += 1;
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
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            versions: self.versions.clone(),
            table_versions: self.table_versions.clone(),
            tick: self.tick,
            derived: std::sync::Mutex::new(self.derived.lock().unwrap_or_else(|p| p.into_inner()).clone()),
            sources: self.sources.clone(),
            later_tx: self.later_tx.clone(),
            session_id: self.session_id,
        }
    }
}

impl Default for CrowLiveState {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate the LiveView client-side JavaScript runtime.
/// Handles WebSocket connection, DOM patching via VPatch, and event delegation.
pub fn live_js(ws_path: &str) -> String {
    format!(r#"<script>
(function() {{
  var ws, reconDelay = 100;

  function connect() {{
    ws = new WebSocket(location.origin.replace('http','ws') + '{ws_path}');
    ws.onopen = function() {{ reconDelay = 100; }};
    ws.onclose = function() {{
      setTimeout(connect, Math.min(reconDelay, 5000));
      reconDelay *= 2;
    }};
    ws.onmessage = function(e) {{
      var msg = JSON.parse(e.data);
      if (msg.type === 'html') {{
        // Full HTML replacement mode
        var root = document.getElementById('crow-root');
        if (root) {{
          var tmp = document.createElement('div');
          tmp.innerHTML = msg.html;
          morphChildren(root, tmp);
        }}
      }} else if (msg.type === 'patches') {{
        // VPatch mode
        msg.patches.forEach(function(p) {{ applyPatch(p); }});
      }}
    }};
  }}

  // Minimal DOM morphing: update root's children to match source's children
  function morphChildren(target, source) {{
    // Save focus and input values before replacing
    var focused = document.activeElement;
    var focusId = focused ? (focused.getAttribute('data-crow-input') || focused.getAttribute('data-crow-change') || focused.getAttribute('data-crow-id')) : null;
    var savedInputs = {{}};
    target.querySelectorAll('[data-crow-input],[data-crow-change]').forEach(function(el) {{
      // radios and checkboxes carry their state in `checked`, which the
      // server renders; a group shares one key, so never save or restore them
      if (el.type === 'radio' || el.type === 'checkbox') return;
      var key = el.getAttribute('data-crow-input') || el.getAttribute('data-crow-change');
      if (key) savedInputs[key] = {{ value: el.value, start: el.selectionStart, end: el.selectionEnd }};
    }});
    target.innerHTML = source.innerHTML;
    // Restore only the input being typed in; every other bound input takes
    // the value the server just rendered, so a handler that assigns to a
    // bound variable shows up in its field.
    target.querySelectorAll('[data-crow-input],[data-crow-change]').forEach(function(el) {{
      if (el.type === 'radio' || el.type === 'checkbox') return;
      var key = el.getAttribute('data-crow-input') || el.getAttribute('data-crow-change');
      if (key && savedInputs[key] && key === focusId) {{
        el.value = savedInputs[key].value;
        el.focus();
        try {{ el.setSelectionRange(savedInputs[key].start, savedInputs[key].end); }} catch(e) {{}}
      }}
    }});
  }}

  // Navigate to a DOM node by path (array of child indices)
  function nodeAt(path) {{
    var node = document.getElementById('crow-root');
    for (var i = 0; node && i < path.length; i++) {{
      node = node.childNodes[path[i]];
    }}
    return node;
  }}

  // Apply a single VPatch operation
  function applyPatch(p) {{
    var node = nodeAt(p.path);
    if (!node) return;
    switch (p.op) {{
      case 'SetText':
        node.textContent = p.text;
        break;
      case 'SetAttrs':
        // Remove old attrs, set new ones
        while (node.attributes.length > 0) node.removeAttribute(node.attributes[0].name);
        for (var k in p.attrs) node.setAttribute(k, p.attrs[k]);
        break;
      case 'Replace':
        var tmp = document.createElement('div');
        tmp.innerHTML = p.html;
        node.parentNode.replaceChild(tmp.firstChild, node);
        break;
      case 'InsertChild':
        var tmp2 = document.createElement('div');
        tmp2.innerHTML = p.html;
        var ref = node.childNodes[p.index];
        if (ref) node.insertBefore(tmp2.firstChild, ref);
        else node.appendChild(tmp2.firstChild);
        break;
      case 'RemoveChild':
        var child = node.childNodes[p.index];
        if (child) node.removeChild(child);
        break;
    }}
  }}

  // Event delegation: click
  document.addEventListener('click', function(e) {{
    var t = e.target;
    while (t && t !== document) {{
      var ev = t.getAttribute('data-crow-click');
      if (ev) {{
        e.preventDefault();
        var ar = t.getAttribute('data-crow-args');
        ws.send(JSON.stringify(ar ? {{event: ev, value: ar}} : {{event: ev}}));
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
      ws.send(JSON.stringify({{event: ev, value: v}}));
    }}
  }});

  // Event delegation: change (select dropdowns)
  document.addEventListener('change', function(e) {{
    var ev = e.target.getAttribute('data-crow-change');
    if (ev) {{
      ws.send(JSON.stringify({{event: ev, value: e.target.value}}));
    }}
  }});

  // Event delegation: submit
  document.addEventListener('submit', function(e) {{
    var ev = e.target.getAttribute('data-crow-submit');
    if (ev) {{
      e.preventDefault();
      var data = {{}};
      new FormData(e.target).forEach(function(v, k) {{ data[k] = v; }});
      ws.send(JSON.stringify({{event: ev, form: data}}));
    }}
  }});

  connect();
}})();
</script>"#, ws_path = ws_path)
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
        body = body_html,
        js = js
    )
}


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
        }
    }

    /// Build an Axum router with:
    /// - GET `page_path` → initial HTML page with embedded JS
    /// - WS  `ws_path`   → event loop: receive events, mutate state, send diffs
    pub fn into_router(self, page_path: &str) -> axum::Router {
        let ws_path = self.ws_path.clone();
        let render = std::sync::Arc::new(self.render);
        let handle = std::sync::Arc::new(self.handle);
        let init_state = std::sync::Arc::new(self.initial_state);

        // GET page: render initial HTML wrapped in page shell
        let r1 = render.clone();
        let s1 = init_state.clone();
        let ws_p = ws_path.clone();
        let page_route = axum::routing::get(move || {
            let r1 = r1.clone();
            let s1 = s1.clone();
            let ws_p = ws_p.clone();
            async move {
                let mut state = (*s1).clone();
                // Execute initial fetches so first paint has data
                for source in &state.sources.clone() {
                    if let Ok(resp) = reqwest::get(&source.url).await {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            state.set(&source.key, json);
                        }
                    }
                }
                state.next_tick();
                let vnode = (r1)(&state);
                let body_html = vnode.to_html();
                let js = live_js(&ws_p);
                axum::response::Html(page_shell(&body_html, &js))
            }
        });

        // WS: event loop with VNode diffing + reactive polling
        let r2 = render.clone();
        let h2 = handle.clone();
        let s2 = init_state.clone();
        let ws_route = axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| {
            let r2 = r2.clone();
            let h2 = h2.clone();
            let s2 = s2.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    use axum::extract::ws::Message;
                    let mut state = (*s2).clone();
                    let (later_tx, mut later_rx) = tokio::sync::mpsc::unbounded_channel::<
                        Vec<(String, serde_json::Value)>,
                    >();
                    state.later_tx = Some(later_tx);
                    state.session_id = {
                        static NEXT: std::sync::atomic::AtomicU64 =
                            std::sync::atomic::AtomicU64::new(1);
                        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    };

                    // Execute initial fetches for all registered data sources
                    for source in &state.sources.clone() {
                        if let Ok(resp) = reqwest::get(&source.url).await {
                            if let Ok(json) = resp.json::<serde_json::Value>().await {
                                state.set(&source.key, json);
                            }
                        }
                    }

                    state.next_tick();
                    let mut last_vnode = (r2)(&state);

                    // Build poll intervals — collect sources that have interval_secs
                    let poll_sources: Vec<DataSource> = state.sources.iter()
                        .filter(|s| s.interval_secs.is_some())
                        .cloned()
                        .collect();

                    // Create a merged poll interval (tick at shortest interval)
                    let tick_secs = poll_sources.iter()
                        .filter_map(|s| s.interval_secs)
                        .min();

                    let mut tick_stream = tick_secs.map(|secs| {
                        tokio_stream::wrappers::IntervalStream::new(
                            tokio::time::interval(std::time::Duration::from_secs(secs))
                        )
                    });

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

                    loop {
                        // Use tokio::select! to handle both WS messages and poll ticks
                        let action = if let Some(ref mut ticker) = tick_stream {
                            tokio::select! {
                                msg = futures_util::StreamExt::next(&mut socket) => {
                                    match msg {
                                        Some(Ok(m)) => LiveAction::Message(m),
                                        _ => break,
                                    }
                                }
                                _ = futures_util::StreamExt::next(ticker) => {
                                    LiveAction::PollTick
                                }
                                delta = later_rx.recv() => {
                                    match delta {
                                        Some(d) => LiveAction::LaterDelta(d),
                                        None => continue,
                                    }
                                }
                                _ = refresh_interval.tick() => LiveAction::RefreshTick,
                            }
                        } else {
                            tokio::select! {
                                msg = futures_util::StreamExt::next(&mut socket) => {
                                    match msg {
                                        Some(Ok(m)) => LiveAction::Message(m),
                                        _ => break,
                                    }
                                }
                                delta = later_rx.recv() => {
                                    match delta {
                                        Some(d) => LiveAction::LaterDelta(d),
                                        None => continue,
                                    }
                                }
                                _ = refresh_interval.tick() => LiveAction::RefreshTick,
                            }
                        };

                        let mut state_changed = false;

                        match action {
                            LiveAction::Message(Message::Text(text)) => {
                                // Parse event from client
                                let (event, value) = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                    let ev = v["event"].as_str().unwrap_or("unknown").to_string();
                                    let val = v["value"].as_str().map(|s| s.to_string());
                                    (ev, val)
                                } else {
                                    (text.to_string(), None)
                                };

                                // Two-way binding: __field:key or __select:key
                                if let Some(key) = event.strip_prefix("__field:").or_else(|| event.strip_prefix("__select:")) {
                                    if let Some(val) = &value {
                                        if let Some(v) = coerce_field_value(state.get(key), val) {
                                            state.set(key, v);
                                        }
                                    }
                                    state_changed = true;
                                } else {
                                    // Regular event → call user handle function
                                    let t0 = std::time::Instant::now();
                                    (h2)(&mut state, &event, value.as_deref());
                                    if let Some(budget) = budget_ms() {
                                        let ms = t0.elapsed().as_millis();
                                        if ms > budget {
                                            eprintln!(
                                                "crow live: handler '{}' took {}ms (budget {}ms) — schedule slow work with later[]",
                                                event, ms, budget
                                            );
                                        }
                                    }
                                    state_changed = true;
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
                                                state_changed = true;
                                            }
                                        }
                                    }
                                }
                            }
                            LiveAction::LaterDelta(delta) => {
                                for (k, v) in delta {
                                    state.set(&k, v);
                                }
                                state_changed = true;
                            }
                            LiveAction::RefreshTick => {
                                // Render-time queries re-run — that's the point.
                                state_changed = true;
                            }
                            _ => {}
                        }

                        if state_changed {
                            // Re-render and diff
                            let t0 = std::time::Instant::now();
                            state.next_tick();
                            let new_vnode = (r2)(&state);
                            if let Some(budget) = budget_ms() {
                                let ms = t0.elapsed().as_millis();
                                if ms > budget {
                                    eprintln!(
                                        "crow live: render took {}ms (budget {}ms) — move slow work out of the render path",
                                        ms, budget
                                    );
                                }
                            }
                            let patches = CrowVNode::diff(&last_vnode, &new_vnode);

                            if !patches.is_empty() {
                                let new_html = new_vnode.to_html();
                                let payload = serde_json::json!({
                                    "type": "html",
                                    "html": new_html
                                });
                                last_vnode = new_vnode;
                                if futures_util::SinkExt::send(
                                    &mut socket,
                                    Message::Text(payload.to_string().into()),
                                ).await.is_err() {
                                    break;
                                }
                            } else {
                                last_vnode = new_vnode;
                            }
                        }
                    }
                })
            }
        });

        axum::Router::new()
            .route(page_path, page_route)
            .route(&ws_path, ws_route)
    }
}

/// Legacy LiveView with a single combined handler returning HTML string.
/// Kept for backward compatibility with existing transpiler output.
pub struct CrowLiveViewLegacy<F>
where
    F: Fn(&mut CrowLiveState, &str) -> String + Send + Sync + 'static,
{
    pub initial_state: CrowLiveState,
    pub handler: F,
    pub ws_path: String,
}

impl<F> CrowLiveViewLegacy<F>
where
    F: Fn(&mut CrowLiveState, &str) -> String + Send + Sync + Clone + 'static,
{
    pub fn new(ws_path: &str, initial: CrowLiveState, handler: F) -> Self {
        Self {
            initial_state: initial,
            handler,
            ws_path: ws_path.to_string(),
        }
    }

    pub fn into_router(self, page_path: &str) -> axum::Router {
        let ws_path = self.ws_path.clone();
        let handler = std::sync::Arc::new(self.handler);
        let init_state = std::sync::Arc::new(self.initial_state);

        let h = handler.clone();
        let s = init_state.clone();
        let ws_p = ws_path.clone();
        let page_route = axum::routing::get(move || {
            let h = h.clone();
            let s = s.clone();
            let ws_p = ws_p.clone();
            async move {
                let mut state = (*s).clone();
                let html = (h)(&mut state, "init");
                let js = live_js(&ws_p);
                axum::response::Html(page_shell(&html, &js))
            }
        });

        let h2 = handler.clone();
        let s2 = init_state.clone();
        let ws_route = axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| {
            let h2 = h2.clone();
            let s2 = s2.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    use axum::extract::ws::Message;
                    let mut state = (*s2).clone();
                    let (later_tx, mut _later_rx) = tokio::sync::mpsc::unbounded_channel::<
                        Vec<(String, serde_json::Value)>,
                    >();
                    state.later_tx = Some(later_tx);
                    state.session_id = {
                        static NEXT: std::sync::atomic::AtomicU64 =
                            std::sync::atomic::AtomicU64::new(1);
                        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    };
                    let mut last_html = (h2)(&mut state, "init");

                    while let Some(Ok(msg)) = futures_util::StreamExt::next(&mut socket).await {
                        if let Message::Text(text) = msg {
                            let event = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                v["event"].as_str().unwrap_or("unknown").to_string()
                            } else {
                                text.to_string()
                            };

                            let new_html = (h2)(&mut state, &event);
                            if new_html != last_html {
                                let payload = serde_json::json!({
                                    "type": "html",
                                    "html": new_html
                                });
                                last_html = new_html;
                                if futures_util::SinkExt::send(
                                    &mut socket,
                                    Message::Text(payload.to_string().into()),
                                ).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                })
            }
        });

        axum::Router::new()
            .route(page_path, page_route)
            .route(&ws_path, ws_route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // Diff should show changes
        let patches = CrowVNode::diff(&vnode1, &vnode2);
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