//! `crow test --compiled`: a test block drives the compiled program. The
//! block itself runs in the interpreter, as every test block does; the app
//! it opens with `app[view]` is this program, reached over plain HTTP. When
//! `CROW_TEST_CONTROL` is set, a program with a live page answers the
//! harness's verbs under `/__crow/test/`: open a session on a view with the
//! facts the test holds, run a handler as one event and one transaction,
//! set a fact, the clock or the reader, and read a fact, a derivation, the
//! page or the files the session was handed. Nothing here is served
//! otherwise. Each verb answers once everything it started has settled,
//! the scheduled work included, as the interpreter's harness does.

use super::*;

/// A view by name, rendered from a session's state: `None` for a name that
/// is not a view without parameters.
pub type ViewFn = fn(&CrowLiveState, &str) -> Option<CrowVNode>;
/// The program's event dispatch (`__crow_handle`).
pub type HandleFn = fn(&mut CrowLiveState, &str, Option<&str>);
/// A derivation by name, as JSON: `None` for a name that is not one, or
/// whose value has no JSON form.
pub type DerivedFn = fn(&CrowLiveState, &str) -> Option<serde_json::Value>;

struct Session {
    view: String,
    state: CrowLiveState,
    /// The session's moment, held by the test (`change[a, 'now', t]`), or
    /// the wall clock when the test never set one.
    clock: Option<f64>,
    later: tokio::sync::mpsc::UnboundedReceiver<Vec<(String, serde_json::Value)>>,
}

struct Control {
    initial: CrowLiveState,
    view: ViewFn,
    handle: HandleFn,
    derived: DerivedFn,
    sessions: tokio::sync::Mutex<HashMap<u64, Session>>,
}

impl Session {
    fn moment(&self) -> f64 {
        self.clock.unwrap_or_else(crow_runtime::moments::clock)
    }

    /// What the session's scheduled work wrote, once none of it is still
    /// running.
    fn settle(&mut self) {
        let prefix = format!("{}:", self.state.session_id);
        for _ in 0..2000 {
            let running = later_registry().lock().unwrap_or_else(|p| p.into_inner()).iter().any(|k| k.starts_with(&prefix));
            if !running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        while let Ok(delta) = self.later.try_recv() {
            for (k, v) in delta {
                self.state.set(&k, v);
            }
        }
    }

    /// The page, settled: a render that read a derivation still being
    /// computed in the background waits for it and renders again, as the
    /// interpreter's harness computes it before it answers.
    fn html(&self, view: ViewFn) -> Result<String, String> {
        let name = self.view.clone();
        let mut html = String::new();
        for _ in 0..4 {
            html = caught(|| {
                crow_runtime::moments::at(self.moment(), || {
                    self.state.render_with(&|s: &CrowLiveState| view(s, &name).unwrap_or_else(|| CrowVNode::text(""))).as_built().to_html()
                })
            })?;
            if !self.recomputing() {
                break;
            }
            for _ in 0..2000 {
                if !self.recomputing() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        Ok(html)
    }

    /// Whether a derivation this session reads is being computed.
    fn recomputing(&self) -> bool {
        let id = self.state.session_id;
        INFLIGHT.lock().unwrap_or_else(|p| p.into_inner()).keys().any(|(holder, _)| *holder == 0 || *holder == id)
    }
}

/// What a render or a handler panicked with, as the verb's error: the
/// program keeps serving the test, as it keeps serving its pages.
fn caught<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|p| {
        let said = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
        format!("the compiled program failed: {}", said)
    })
}

/// The routes, answering only when `CROW_TEST_CONTROL` is set.
pub fn router(initial: CrowLiveState, view: ViewFn, handle: HandleFn, derived: DerivedFn) -> axum::Router {
    let mut initial = initial;
    if !initial.data.contains_key(READER) {
        initial.set(READER, serde_json::json!({}));
    }
    if !initial.data.contains_key(READER_SID) {
        initial.set(READER_SID, serde_json::json!(""));
    }
    if !initial.data.contains_key(PAGES) {
        initial.set(PAGES, serde_json::json!({}));
    }
    let control = std::sync::Arc::new(Control { initial, view, handle, derived, sessions: tokio::sync::Mutex::new(HashMap::new()) });
    if std::env::var("CROW_TEST_CONTROL").is_err() {
        return axum::Router::new();
    }
    let route = |op: &'static str| {
        let c = control.clone();
        axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let c = c.clone();
            async move { axum::Json(answer(&c, op, body).await.unwrap_or_else(|e| serde_json::json!({ "error": e }))) }
        })
    };
    let mut r = axum::Router::new();
    for op in ["open", "event", "field", "clock", "reader", "read", "facts", "html", "downloads", "mails"] {
        r = r.route(&format!("/__crow/test/{}", op), route(op));
    }
    r
}

fn text(body: &serde_json::Value, key: &str) -> Result<String, String> {
    body.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()).ok_or_else(|| format!("the harness sent no '{}'", key))
}

async fn answer(c: &Control, op: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    if op == "open" {
        let view = text(&body, "view")?;
        let mut state = start_session(&c.initial);
        // the test's own facts, as it holds them when it opens the app
        if let Some(serde_json::Value::Object(facts)) = body.get("facts") {
            for (k, v) in facts {
                if state.data.contains_key(k) {
                    state.set(k, v.clone());
                }
            }
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        state.later_tx = Some(tx);
        fetch_sources(&mut state).await;
        state.next_tick();
        let clock = body.get("now").and_then(|v| v.as_f64());
        // nothing renders until the test asks: a view may read what the
        // test sets next (the reader)
        let session = Session { view, state, clock, later: rx };
        let id = session.state.session_id;
        c.sessions.lock().await.insert(id, session);
        return Ok(serde_json::json!({ "session": id }));
    }
    let id = body.get("session").and_then(|v| v.as_u64()).ok_or("the harness sent no session")?;
    let mut sessions = c.sessions.lock().await;
    let s = sessions.get_mut(&id).ok_or_else(|| format!("no test session {}", id))?;
    match op {
        // a handler, with the arguments a click froze, as one event and one
        // transaction on its working copy
        "event" => {
            let event = text(&body, "event")?;
            let args = body.get("args").cloned().unwrap_or(serde_json::Value::Array(vec![]));
            let value = match &args {
                serde_json::Value::Array(a) if a.is_empty() => None,
                other => Some(other.to_string()),
            };
            let moment = s.moment();
            let handle = c.handle;
            let copy = s.state.working_copy();
            let copy = caught(|| {
                crow_runtime::moments::at(moment, || {
                    crow_runtime::txn::begin();
                    let mut copy = copy;
                    handle(&mut copy, &event, value.as_deref());
                    if let Err(e) = crow_runtime::txn::finish(!copy.is_stopped()) {
                        copy.stop(format!("{} stopped: {}", event, e));
                    }
                    copy
                })
            })?;
            let stopped = s.state.commit(copy);
            s.settle();
            s.state.next_tick();
            Ok(match stopped {
                Some(reason) => serde_json::json!({ "ok": false, "reason": reason }),
                None => serde_json::json!({ "ok": true }),
            })
        }
        // `change[a, name, value]`: a fact set as a bound input sets it
        "field" => {
            let name = text(&body, "name")?;
            let value = body.get("value").cloned().unwrap_or(serde_json::Value::Null);
            if !s.state.data.contains_key(&name) {
                return Err(format!("change[]: no reactive variable named '{}' in the compiled program", name));
            }
            s.state.set(&name, value);
            s.state.next_tick();
            Ok(serde_json::json!({ "ok": true }))
        }
        // time is an input: the session's clock is set as a field is
        "clock" => {
            s.clock = body.get("now").and_then(|v| v.as_f64());
            s.state.next_tick();
            Ok(serde_json::json!({ "ok": true }))
        }
        "reader" => {
            let record: std::collections::BTreeMap<String, String> = match body.get("record") {
                Some(serde_json::Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect(),
                _ => return Err("change[a, 'reader', record] takes a map of strings".into()),
            };
            let sid = format!("test-{}", id);
            s.state.set_reader(record, sid);
            s.state.next_tick();
            Ok(serde_json::json!({ "ok": true }))
        }
        // a fact as the session holds it, else a derivation as it computes it
        "read" => {
            let name = text(&body, "name")?;
            if let Some(v) = s.state.get(&name) {
                return Ok(serde_json::json!({ "fact": v }));
            }
            let derived = c.derived;
            let moment = s.moment();
            match caught(|| crow_runtime::moments::at(moment, || derived(&s.state, &name)))? {
                Some(v) => Ok(serde_json::json!({ "derived": v })),
                None => Err(format!("the compiled program has no fact or readable derivation named '{}'", name)),
            }
        }
        // every fact the session holds, the runtime's own left out: the
        // harness copies them into the test, as its sessions swap theirs in
        "facts" => {
            let facts: serde_json::Map<String, serde_json::Value> =
                s.state.data.iter().filter(|(k, _)| !k.starts_with("__") && k.as_str() != READER).map(|(k, v)| (k.clone(), v.clone())).collect();
            // and what the harness reads beside them: the notice and the reader
            let notice = s.state.notice.as_ref().map(|(text, at)| serde_json::json!({ "text": text, "at": at }));
            Ok(serde_json::json!({ "facts": facts, "notice": notice, "reader": s.state.reader() }))
        }
        "html" => Ok(serde_json::json!({ "html": s.html(c.view)? })),
        "downloads" => {
            let files: Vec<serde_json::Value> =
                s.state.downloads.iter().map(|(name, bytes)| serde_json::json!({ "name": name, "text": String::from_utf8_lossy(bytes) })).collect();
            Ok(serde_json::json!({ "files": files }))
        }
        // what the session sent, recorded: nothing is sent under test
        "mails" => {
            let mails: Vec<serde_json::Value> = s.state.mails.iter().map(|m| serde_json::json!({ "to": m.to, "subject": m.subject, "body": m.body })).collect();
            Ok(serde_json::json!({ "mails": mails }))
        }
        other => Err(format!("no test verb '{}'", other)),
    }
}
