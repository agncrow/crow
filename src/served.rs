//! `crow run -i` serves pages (session-layer-design.md §3.11). The live
//! runtime keeps the sessions — their facts, the commit, the notice, the
//! questions, the address, the reconnects — exactly as it does for a
//! compiled program; the IR interpreter renders the views and runs the
//! handlers, against each session's facts. The interpreter that ran the
//! program's main is the template: every render and every handler works
//! on a copy of it holding that session's facts.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ir_interp::IrInterp;
use crate::value::*;
use crow_web::live::{CrowLiveState, CrowLiveView, Values};
use crow_web::view::tree::{self, Node};

/// Whether a fact is data a session holds, rather than a resource every
/// session shares (a database handle, a mailer) or a function: resources stay the
/// template's, never copied into a session.
pub(crate) fn is_data(v: &CrowValue) -> bool {
    !matches!(v.value, Value::DbConn(_) | Value::Mailer(..) | Value::Socket(_) | Value::Response(_) | Value::Fn(_) | Value::GradFn(_) | Value::App(..))
}

/// A fact as the session keeps it: JSON, a frame by its own shape, a
/// record as the object of its fields (in a list or a map too), as the
/// compiled program keeps its records.
pub(crate) fn fact_to_json(v: &CrowValue) -> serde_json::Value {
    if v.is_error() {
        return serde_json::Value::Null;
    }
    match &v.value {
        Value::Frame(f) => serde_json::to_value(f).unwrap_or(serde_json::Value::Null),
        Value::StructInstance(si) => serde_json::Value::Object(si.fields().map(|(k, x)| (k.clone(), fact_to_json(x))).collect()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(fact_to_json).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k, x)| (k.clone(), fact_to_json(x))).collect()),
        _ => crate::builtins::crow_to_json("", v).unwrap_or(serde_json::Value::Null),
    }
}

/// A fact back from the session, in the shape the program declared it.
pub(crate) fn json_to_fact(j: &serde_json::Value, declared: &CrowValue) -> CrowValue {
    match &declared.value {
        Value::Frame(_) => match serde_json::from_value::<crow_runtime::frame::CrowFrame>(j.clone()) {
            Ok(f) => CrowValue::ok(Value::Frame(Box::new(f))),
            Err(_) => declared.clone(),
        },
        Value::Float(_) => match j.as_f64() {
            Some(x) => CrowValue::ok(Value::Float(x)),
            None => crate::builtins::json_to_crow("", j),
        },
        // a moment comes back as the text it shows as (its serde form)
        Value::Moment(_) => match serde_json::from_value::<crow_runtime::moments::Moment>(j.clone()) {
            Ok(m) => CrowValue::ok(Value::Moment(m)),
            Err(_) => crate::builtins::json_to_crow("", j),
        },
        // a parsed value comes back JSON, as it was held
        Value::Json(_) => CrowValue::ok(Value::Json(std::sync::Arc::new(j.clone()))),
        _ => crate::builtins::json_to_crow("", j),
    }
}

/// A value back from JSON by the type the program gives it: a record
/// from the object of its fields, a list item by item (numbers the program
/// holds as a list are a list, not a tensor: the rows the IR names read
/// them as one), a tensor from its numbers; anything else as `json_to_fact`
/// reads it.
pub(crate) fn json_to_typed(j: &serde_json::Value, ty: &crate::ir::Ty, template: &IrInterp, declared: &CrowValue) -> CrowValue {
    use crate::ir::Ty;
    match (ty, j) {
        (Ty::Record(name) | Ty::OptRecord(name), serde_json::Value::Object(obj)) => match template.struct_def(name) {
            Some(def) => {
                let mut fields = std::collections::BTreeMap::new();
                for (field, fty) in &def.fields {
                    let fj = obj.get(field).cloned().unwrap_or(serde_json::Value::Null);
                    fields.insert(field.clone(), json_to_typed(&fj, fty, template, &CrowValue::ok(Value::Done)));
                }
                CrowValue::ok(Value::StructInstance(std::sync::Arc::new(crate::value::StructInstance::new(name.clone(), def.shown.clone(), fields))))
            }
            None => json_to_fact(j, declared),
        },
        (Ty::List(item), serde_json::Value::Array(items)) => {
            let none = CrowValue::ok(Value::Done);
            CrowValue::ok(Value::Array(std::sync::Arc::new(items.iter().map(|x| json_to_typed(x, item, template, &none)).collect())))
        }
        (Ty::Tensor(..), serde_json::Value::Array(items)) if items.iter().all(|x| x.is_number()) => {
            let data: Vec<f64> = items.iter().filter_map(|x| x.as_f64()).collect();
            let n = data.len();
            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_vec(data, vec![n]))))
        }
        (Ty::Float, serde_json::Value::Number(n)) => CrowValue::ok(Value::Float(n.as_f64().unwrap_or(0.0))),
        // an int the compiled program holds growing: a number, or the text
        // of its digits past 64 bits (its serde form, crow_runtime::int)
        (Ty::Big, _) => match crow_runtime::int::Int::of_json(j) {
            Some(n) => CrowValue::ok(Value::of_int(n)),
            None => json_to_fact(j, declared),
        },
        // an exact number comes back from the text it shows (its serde form)
        (Ty::Decimal, serde_json::Value::String(s)) => match crow_runtime::decimal::Dec::parse(s) {
            Some(d) => crate::table::write_decimal(d),
            None => json_to_fact(j, declared),
        },
        (Ty::Fraction, serde_json::Value::String(s)) => match crow_runtime::fraction::Frac::parse(s) {
            Some(q) => crate::table::write_fraction(q),
            None => json_to_fact(j, declared),
        },
        (Ty::Moment, serde_json::Value::String(_)) => match serde_json::from_value::<crow_runtime::moments::Moment>(j.clone()) {
            Ok(m) => CrowValue::ok(Value::Moment(m)),
            Err(_) => json_to_fact(j, declared),
        },
        // a map keyed by ints: its keys were encoded as text (JSON's objects
        // have no other kind), read back as ints
        (Ty::Map(crate::ir::Key::Int, item), serde_json::Value::Object(obj)) => {
            let mut map = std::collections::BTreeMap::new();
            for (k, v) in obj {
                if let Ok(n) = k.parse::<i64>() {
                    map.insert(n, json_to_typed(v, item, template, declared));
                }
            }
            CrowValue::ok(Value::int_map(map))
        }
        (Ty::Json, _) => CrowValue::ok(Value::Json(std::sync::Arc::new(j.clone()))),
        // a frame inside a record or a list comes as its records (the one
        // JSON form, crow_runtime::json), alone as its serde form (a
        // dashboard's tiles, records holding a frame, had been unreadable
        // to a compiled test)
        (Ty::Frame(_), serde_json::Value::Array(_)) => match crow_runtime::sys::frame_of_json(&j.to_string()) {
            Ok(f) => CrowValue::ok(Value::Frame(Box::new(f))),
            Err(_) => json_to_fact(j, declared),
        },
        (Ty::Frame(_), serde_json::Value::Object(_)) => match serde_json::from_value::<crow_runtime::frame::CrowFrame>(j.clone()) {
            Ok(f) => CrowValue::ok(Value::Frame(Box::new(f))),
            Err(_) => json_to_fact(j, declared),
        },
        _ => json_to_fact(j, declared),
    }
}

/// The session's facts loaded into `it`: each one whose version is not the
/// version `loaded` says `it` holds (every one, without `loaded`).
fn load_facts(it: &mut IrInterp, template: &IrInterp, state: &CrowLiveState, loaded: Option<&HashMap<String, u64>>) {
    for name in &template.fact_names {
        if let Some(loaded) = loaded
            && loaded.get(name) == state.versions.get(name)
        {
            continue;
        }
        match (state.get(name), template.var(name)) {
            (Some(j), Some(declared)) => {
                let v = match template.fact_types.get(name) {
                    Some(ty) => json_to_typed(&j, ty, template, declared),
                    None => json_to_fact(&j, declared),
                };
                it.set_fact(name, v);
            }
            // (a fact the session does not hold is the template's)
            (None, Some(declared)) if loaded.is_some() => it.set_fact(name, declared.clone()),
            _ => {}
        }
    }
}

/// A render thread's copy of the program: the template it was made from
/// (by address), the fact versions it holds, the copy.
type RenderCopy = (usize, HashMap<String, u64>, IrInterp);

thread_local! {
    /// A render thread's copy of the program with the facts it last loaded
    /// and their versions, by the template it was made from: a render loads
    /// only the facts whose versions moved since (a version is the app's
    /// count, so one names one value whichever session it is of) — the
    /// template had been copied and every fact read from JSON at every
    /// render.
    static RENDER_COPY: std::cell::RefCell<Option<RenderCopy>> = const { std::cell::RefCell::new(None) };
}

/// The program as a render of `state` reads it: the thread's copy with the
/// facts that moved loaded, or a fresh one.
fn for_render(template: &Arc<IrInterp>, state: &CrowLiveState) -> IrInterp {
    let tag = Arc::as_ptr(template) as usize;
    let kept = RENDER_COPY.with(|c| c.borrow_mut().take()).filter(|(t, _, _)| *t == tag);
    let mut it = match kept {
        Some((_, loaded, mut it)) => {
            load_facts(&mut it, template, state, Some(&loaded));
            it
        }
        None => {
            let mut it = (**template).clone();
            load_facts(&mut it, template, state, None);
            it
        }
    };
    session_parts(&mut it, state);
    it
}

/// A render's copy handed back to its thread, with the versions it holds.
fn after_render(template: &Arc<IrInterp>, state: &CrowLiveState, it: IrInterp) {
    RENDER_COPY.with(|c| *c.borrow_mut() = Some((Arc::as_ptr(template) as usize, loaded_versions(template, state), it)));
}

/// The versions of the facts `state` holds, as a copy loaded from it holds them.
fn loaded_versions(template: &IrInterp, state: &CrowLiveState) -> HashMap<String, u64> {
    template.fact_names.iter().filter_map(|n| state.versions.get(n).map(|v| (n.clone(), *v))).collect()
}

/// The tables written on this thread since the last call (`crow_db`'s
/// record), as a handler's commit or its next read lands them.
pub(crate) fn take_writes() -> Vec<crow_runtime::sql::WriteKeys> {
    crow_db::take_write_keys()
}

/// A version no fact holds: a fact a handler wrote is loaded again from
/// the session at the next use of the copy, as a fresh copy would read it
/// (the value it wrote is the one the session took as JSON, which may read
/// back other than as written).
const WRITTEN: u64 = u64::MAX;

/// The session's notice, reader and tables' pages and orders, as a render
/// or a handler reads them.
fn session_parts(it: &mut IrInterp, state: &CrowLiveState) {
    it.session_notice.remove(&0);
    if let Some(n) = &state.notice {
        it.session_notice.insert(0, n.clone());
    }
    // the session's signed-in reader, as the runtime holds it
    let reader = state.reader().into_iter().map(|(k, v)| (k, CrowValue::ok(Value::Str(v)))).collect();
    it.session_reader.insert(0, reader);
    // the page each paged table shows, and the order each sorted one is in
    it.session_pages = state.table_pages();
    it.session_sorts = state.table_sorts();
}

/// One event of a served page: its handler run as one transaction on the
/// session's working copy (`state`), on the render thread's copy of the
/// program with the facts that moved loaded (a copy of the whole program
/// had been made from the template at every event). The handler reads its
/// derivations through the session's cache (`IrInterp::begin_handling`):
/// each computed once per change of what it reads, a fact it writes landing
/// in the working copy before its next read, so a read after a write sees
/// the write — as the compiled program's handler reads them.
fn handle_event(t: &Arc<IrInterp>, state: &mut CrowLiveState, event: &str, value: Option<&str>) {
    let loaded = loaded_versions(t, state);
    let mut it = for_render(t, state);
    let before_notice = it.session_notice.get(&0).cloned();
    let args: Vec<CrowValue> = value
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        .and_then(|j| j.as_array().cloned())
        .map(|items| items.iter().map(|x| crate::builtins::json_to_crow("", x)).collect())
        .unwrap_or_default();
    let _ = take_writes();
    it.begin_handling(state);
    let outcome = it.guard(|it| it.transaction(event, args));
    let wrote = it.end_handling();
    let written = crow_db::take_write_keys();
    // (a handler that stopped leaves its copy behind: the next starts afresh)
    if let Err(e) = outcome {
        state.stop(format!("{} stopped: {}", event, e));
        return;
    }
    if let Some(reason) = it.last_stop.clone() {
        state.stop(reason);
        return;
    }
    // what the handler changed lands in the session's working copy: every
    // fact it wrote (a fact is written only by a store, which records it)
    for name in &t.fact_names {
        if !wrote.contains(name) {
            continue;
        }
        if let Some(v) = it.var(name).filter(|v| is_data(v)) {
            let j = fact_to_json(v);
            if state.get(name).as_ref() != Some(&j) {
                state.set(name, j);
            }
        }
    }
    if crate::ir_interp::view_cache_mode() == crate::ir_interp::ViewCacheMode::Check {
        // CROW_VIEW_CACHE=check: a fact no store recorded holds what the
        // session holds
        for name in &t.fact_names {
            if let Some(v) = it.var(name).filter(|v| is_data(v) && !wrote.contains(name)) {
                assert!(state.get(name).as_ref() == Some(&fact_to_json(v)), "CROW_VIEW_CACHE=check: {} moved in {} without a store", name, event);
            }
        }
    }
    if let Some((text, _)) = it.session_notice.get(&0).cloned().filter(|n| Some(n) != before_notice.as_ref()) {
        state.notice(text);
    }
    // the files the handler handed the reader go with its commit
    for (name, bytes) in it.session_downloads.remove(&0).unwrap_or_default() {
        state.download(&name, bytes);
    }
    // and the messages it sent, which the runtime sends when it commits
    for (mail, mailer) in it.session_mails.remove(&0).unwrap_or_default() {
        // sent by the row written in Crow, on a thread of its own: a
        // copy of the template runs it, as a session's handler runs
        let (template, message) = (t.clone(), mail.clone());
        let send: crow_web::live::MailSender = Arc::new(move || {
            let mut copy = (*template).clone();
            copy.send_mail(&mailer, &message)
        });
        state.mail(mail, send);
    }
    // a sign-in or sign-out the handler made lands with its commit, and
    // the runtime keeps the cookie
    match it.session_sign.take() {
        Some(Some(record)) => state.sign_in(&record),
        Some(None) => state.sign_out(),
        None => {}
    }
    // the tables it wrote wake their other readers when it commits, and a
    // derivation over a slice it did not touch keeps its value
    state.note_writes(written);
    // the copy goes back to the thread: what it holds of a fact it did not
    // write is what it loaded; a fact it wrote is loaded afresh next time
    let mut versions = loaded;
    for name in &wrote {
        versions.insert(name.clone(), WRITTEN);
    }
    RENDER_COPY.with(|c| *c.borrow_mut() = Some((Arc::as_ptr(t) as usize, versions, it)));
}

/// The fact a `page[…, url = …]` name means, as the Rust backend resolves it.
fn url_fact(template: &IrInterp, written: &str) -> Option<String> {
    crate::derive::address_fact(written, template.fact_names.iter())
}

fn int_of(v: Option<&CrowValue>) -> i64 {
    match v.map(|v| &v.value) {
        Some(Value::Int(n)) => *n,
        Some(Value::Float(x)) => *x as i64,
        _ => 0,
    }
}

fn text_of(v: Option<&CrowValue>) -> String {
    match v.map(|v| &v.value) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn bool_of(v: Option<&CrowValue>) -> bool {
    matches!(v.map(|v| &v.value), Some(Value::Bool(true)))
}

/// A page's view rendered for a session, as the live runtime asks for it.
fn page_render(t: Arc<IrInterp>, view_name: String) -> impl Fn(&CrowLiveState) -> Node + Clone + Send + Sync + 'static {
    move |state: &CrowLiveState| -> Node {
        let mut it = for_render(&t, state);
        // the render reads its derivations through the session's cache, and
        // keeps its views with what they read (crow_web/src/live/views.rs)
        it.served = Some(crate::ir_interp::Served::new(state));
        let page = it.guard(|it| it.render_page(&view_name));
        it.served = None;
        let notice_read = it.notice_read;
        // (a render that stopped leaves its copy behind: the next starts afresh)
        if page.is_ok() {
            after_render(&t, state, it);
        }
        match page {
            Ok(page) => {
                if notice_read {
                    // the view shows the notice itself: no runtime toast
                    let _ = state.notice_record();
                }
                // the view's tree, as the compiled page hands it over (the
                // runtime writes it as the page's nodes and adds its own
                // toast)
                crate::ops::page_node(page)
            }
            Err(e) => tree::failed(&e),
        }
    }
}

/// The program's bindings as the live runtime asks for them (a values
/// turn, one-view-design.md 3.5): computed on the render thread's copy of
/// the program, with the facts that moved loaded, as a render reads them.
struct InterpValues {
    t: Arc<IrInterp>,
    inputs: Vec<&'static [&'static str]>,
}

impl InterpValues {
    fn new(t: Arc<IrInterp>) -> Self {
        fn leak(s: &str) -> &'static str {
            Box::leak(s.to_string().into_boxed_str())
        }
        let inputs = t.bindings.iter().map(|b| &*Box::leak(b.inputs.iter().map(|n| leak(n)).collect::<Vec<_>>().into_boxed_slice())).collect();
        InterpValues { t, inputs }
    }

    /// `f` on the thread's copy of the program, reading the session.
    fn on_copy<T>(&self, state: &CrowLiveState, f: impl FnOnce(&mut IrInterp) -> Option<T>) -> Option<T> {
        let mut it = for_render(&self.t, state);
        it.served = Some(crate::ir_interp::Served::new(state));
        let r = it.guard(|it| Ok(f(it)));
        it.served = None;
        // (one that stopped leaves its copy behind: the next starts afresh)
        match r {
            Ok(v) => {
                after_render(&self.t, state, it);
                v
            }
            Err(_) => None,
        }
    }
}

impl Values for InterpValues {
    fn inputs(&self, id: u32) -> &'static [&'static str] {
        self.inputs.get(id as usize).copied().unwrap_or(&[])
    }

    fn refresh(&self, state: &CrowLiveState, name: &str) {
        let _ = self.on_copy(state, |it| {
            it.served_refresh(name);
            Some(())
        });
    }

    fn eval(&self, state: &CrowLiveState, ids: &[u32]) -> Option<Vec<tree::Bound>> {
        self.on_copy(state, |it| it.eval_bindings(ids))
    }
}

thread_local! {
    /// A thread's copy of the program a route's handler runs on, and the
    /// template it was made from: a handler is a function, which writes
    /// nothing a later request reads, so the copy is kept from request to
    /// request with the functions it has prepared (a copy made for every
    /// request had prepared its handler again each time, and copied every
    /// function the program holds).
    static ROUTE_COPY: std::cell::RefCell<Option<(usize, IrInterp)>> = const { std::cell::RefCell::new(None) };
}

/// A route's answer: its handler run on the thread's copy of the program,
/// handed the request (its method, path, headers, query, body and the
/// path's params), and what it returns sent as the response. A handler that
/// stops answers 500, the reason the log's, as a value that is no response
/// does, and the copy it stopped in is let go.
fn route_answer(template: &IrInterp, handler: &str, req: crow_web::core::CrowRequest) -> crow_web::core::CrowResponse {
    let pairs = |m: &std::collections::BTreeMap<String, String>| -> CrowValue {
        CrowValue::ok(Value::map(m.iter().map(|(k, v)| (k.clone(), CrowValue::ok(Value::Str(v.clone())))).collect()))
    };
    let text = |s: &str| CrowValue::ok(Value::Str(s.to_string()));
    let request: std::collections::BTreeMap<String, CrowValue> = [
        ("method".to_string(), text(&req.method)),
        ("path".to_string(), text(&req.path)),
        ("body".to_string(), text(&req.body)),
        ("headers".to_string(), pairs(&req.headers)),
        ("query".to_string(), pairs(&req.query)),
        ("params".to_string(), pairs(&req.params)),
        ("form".to_string(), pairs(&req.form)),
        ("files".to_string(), CrowValue::ok(Value::Array(std::sync::Arc::new(req.files.iter().map(pairs).collect())))),
    ]
    .into_iter()
    .collect();
    let made_from = template as *const IrInterp as usize;
    let answered = ROUTE_COPY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let it = match &mut *slot {
            Some((from, it)) if *from == made_from => it,
            _ => &mut slot.insert((made_from, template.clone())).1,
        };
        it.begin_route();
        let answered = it.guard(|it| it.call(handler, vec![CrowValue::ok(Value::map(request))]));
        it.end_route();
        if answered.is_err() {
            *slot = None;
        }
        answered
    });
    // what the handler wrote is the app's: a derivation over it is computed
    // again at its next read, and a page that read one renders again
    let writes = crow_db::take_write_keys();
    if template.has_derivations() {
        crow_web::live::publish_writes(writes);
    }
    match answered {
        Ok(v) if v.is_error() => crow_web::server::route_stopped(handler, v.reason()),
        // (one the lowering could not see: a stop, its reason the log's,
        // as every route's failure is — it had been the answer's body)
        Ok(v) => {
            let what = crate::table::Arg::of_value(&v).words();
            match v.value {
                Value::Response(r) => *r,
                _ => crow_web::server::route_stopped(handler, &crow_web::core::response::not_a_response(handler, what)),
            }
        }
        Err(e) => crow_web::server::route_stopped(handler, &e),
    }
}

/// The app's own state, which a route reads the derivations the app holds
/// through (crow_web::live), made from the program's state as declared; and
/// what the top level wrote before serving, landed for the app.
fn hold_app(template: &mut IrInterp, initial: &CrowLiveState) {
    if !template.has_derivations() {
        return;
    }
    template.app_tag = crate::ir_interp::next_app_tag();
    template.app_state = Some(Arc::new(crow_web::live::app_state_from(initial)));
    crow_web::live::publish_writes(crow_db::take_write_keys());
}

/// Serve the pages and the routes the program's main asked for, from the
/// interpreter: every page, each a session of its own, and every route
/// (spec, Core (Server)).
pub fn serve(template: IrInterp) -> Result<(), String> {
    if template.pages.is_empty() && template.routes.is_empty() {
        return Err("the program serves no page and no route: add page['/', render] or route['/health', health]".to_string());
    }
    let serve_args = template.serve_config.clone().unwrap_or_default();
    let port = int_of(serve_args.first()).max(0) as u16;

    let mut initial = CrowLiveState::new();
    for name in &template.fact_names {
        if let Some(v) = template.var(name).filter(|v| is_data(v)) {
            initial.set(name, fact_to_json(v));
        }
    }
    // the page's data sources, registered as the compiled program registers
    // them: the session loads each and holds it to the columns the program
    // reads (the interpreter had registered none)
    for (name, url, secs, held) in &template.sources {
        match secs {
            None => initial.register_fetch(name, url, held.clone()),
            Some(s) => initial.register_poll(name, url, *s, held.clone()),
        }
    }
    // each page's address: the facts its path's `:name` segments name, and
    // its `url` facts, as the Rust backend resolves them
    // (its path, its view, its address's (name, fact) pairs, its init)
    type Served = (String, String, Vec<(String, String)>, Option<String>);
    let mut pages: Vec<Served> = Vec::new();
    for (path, view, url_names, init) in &template.pages {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for name in crow_web::core::path::params(path) {
            let fact = url_fact(&template, &name).ok_or_else(|| format!("page['{}', …]: ':{}' names no fact — a page's path holds facts, as url = […] does", path, name))?;
            pairs.push((name, fact));
        }
        for written in url_names {
            let fact = url_fact(&template, written).ok_or_else(|| format!("page[…, url = …]: no reactive fact named '{}'", written))?;
            pairs.push((written.rsplit('.').next().unwrap_or(written).to_string(), fact));
        }
        pages.push((path.clone(), view.clone(), pairs, init.clone()));
    }
    // the runtime draws the toast for a served page
    let mut template = template;
    template.harness_toast = false;
    hold_app(&mut template, &initial);
    template.hold_view_specs();
    let template = Arc::new(template);

    let t = template.clone();
    let handle = move |state: &mut CrowLiveState, event: &str, value: Option<&str>| handle_event(&t, state, event, value);
    let values: Arc<dyn Values> = Arc::new(InterpValues::new(template.clone()));

    // a handler's thread holds the interpreter's frames to the recursion limit
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crow_runtime::stops::STACK)
        .max_blocking_threads(crow_runtime::cores::blocking_threads())
        .build()
        .map_err(|e| format!("cannot start the server: {}", e))?;
    rt.block_on(async move {
        let mut app = crow_web::server::CrowApp::new();
        let quick = template.quick_fns();
        for (method, path, handler, limit) in template.routes.clone() {
            let t = template.clone();
            let name = handler.clone();
            let answer = move |req| route_answer(&t, &handler, req);
            let answer = if quick.contains(&name) { crow_web::server::CrowHandler::quick(&name, answer) } else { crow_web::server::CrowHandler::new(&name, answer) };
            app = app.route_limited(&path, &method, answer, limit);
        }
        for (path, view, pairs, init) in pages {
            let ws_path = format!("/_ws{}", path);
            let pairs: Vec<(&str, &str)> = pairs.iter().map(|(n, f)| (n.as_str(), f.as_str())).collect();
            let live = CrowLiveView::new(&ws_path, initial.clone(), page_render(template.clone(), view), handle.clone()).with_url(pairs);
            let live = if template.bindings.is_empty() { live } else { live.with_values(values.clone()) };
            let live = match &init {
                Some(f) => live.with_init(f),
                None => live,
            };
            app = app.merge(live.into_router(&path));
        }
        crow_web::live::configure(int_of(serve_args.get(1)), int_of(serve_args.get(2)));
        crow_web::live::configure_look(&text_of(serve_args.get(3)), &text_of(serve_args.get(4)), &text_of(serve_args.get(5)), bool_of(serve_args.get(6)));
        crow_web::live::configure_keep(bool_of(serve_args.get(7)));
        crow_web::live::files::configure_uploads(int_of(serve_args.get(8)));
        // the server's options, as the compiled program's are
        crow_web::server::configure_uploads(int_of(serve_args.get(8)));
        let origins: Vec<String> = match serve_args.get(9).map(|v| &v.value) {
            Some(Value::Array(items)) => items.iter().map(|i| format!("{}", i.value)).collect(),
            _ => Vec::new(),
        };
        app.cors_origins(&origins).files_from(&text_of(serve_args.get(10))).serve(port).await.map_err(|e| format!("the server stopped: {}", e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Ty;

    /// A route reads the app's value of a derivation (spec, Derivations):
    /// kept while nothing it reads moves — a write made outside the program
    /// is not seen — and computed again once a route of the program writes
    /// the table it reads.
    #[test]
    fn a_route_reads_the_apps_value_until_the_program_writes() {
        let url = format!("sqlite://file:served_app_{}?mode=memory&cache=shared", std::process::id());
        let src = format!(
            "use web, db\n\nbook = db.connect['{u}']\nmade = query[book, 'CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT)']\n\nderive notes\n    return query[book, 'SELECT id FROM notes']\n\nfn count_h[req]\n    return text['`len[notes]`']\n\nfn add_h[req]\n    conn = db.connect['{u}']\n    query[conn, 'INSERT INTO notes (body) VALUES (?)', ['ours']]\n    return text['added']\n\nroute['/count', count_h]\nroute['/add', 'POST', add_h]\nserve[3998]\n",
            u = url
        );
        let (program, ctx) = crate::ir_interp::prepare(&crate::build(&src).unwrap(), "app").unwrap();
        let mut template = crate::ir_interp::run_to_serve(&program, &ctx.registry, "app.co").unwrap();
        hold_app(&mut template, &CrowLiveState::new());
        let ask = |handler: &str| route_answer(&template, handler, crow_web::core::CrowRequest::default()).body;
        assert_eq!(ask("count_h"), "0");
        // a write the program does not make: its record let go, as another
        // process's write never reaches it
        let outside = crow_db::connect(&url, None).unwrap();
        outside.exec("INSERT INTO notes (body) VALUES ('theirs')", &[]).unwrap();
        let _ = crow_db::take_write_keys();
        assert_eq!(ask("count_h"), "0", "kept: nothing the program wrote moved it");
        assert_eq!(ask("add_h"), "added");
        assert_eq!(ask("count_h"), "2", "the program's write: computed again, the other write with it");
    }

    /// The program the handler tests run: handlers that read a derivation,
    /// write a fact it reads and read it again (`step`), read one over a
    /// list they grow, stop partway (`refuse`), and hand work to `later`.
    const HANDLERS: &str = "use web, ui\n\na = 0\nb = 1\nseen = ['start']\nnames = ['x']\n\nderive total\n    return a + b\n\nderive doubled\n    return total * 2\n\nderive many\n    return len[names]\n\nfn step[n]\n    first = doubled\n    a = a + n\n    second = doubled\n    seen = push[seen, '`first`/`second`']\n\nfn other[n]\n    b = n\n    x = total\n    b = b + x\n    seen = push[seen, 't`total`']\n\nfn grow[s]\n    before = many\n    names = push[names, s]\n    seen = push[seen, 'n`before`-`many`']\n\nfn refuse[n]\n    a = a + n\n    x = doubled\n    px = float['no `x`']\n    if px gt 1.0\n        a = 99\n\nfn later_work[]\n    b = b + doubled\n\nfn defer[]\n    a = a + 1\n    later[later_work]\n\nview counts[]\n    return p['`a` `b` `total` `doubled` `many`']\n\nview render[]\n    return stack[counts[], p['`seen`'], button['s', click = step[1]], button['o', click = other[1]], button['g', click = grow['x']], button['r', click = refuse[1]], button['d', click = defer]]\n\npage['/', render]\nserve[3997]\n";

    fn handlers_template() -> Arc<IrInterp> {
        let (program, ctx) = crate::ir_interp::prepare(&crate::build(HANDLERS).unwrap(), "app").unwrap();
        let mut t = crate::ir_interp::run_to_serve(&program, &ctx.registry, "app.co").unwrap();
        t.harness_toast = false;
        t.hold_view_specs();
        Arc::new(t)
    }

    fn session_of(t: &IrInterp) -> CrowLiveState {
        let mut s = CrowLiveState::new();
        for name in &t.fact_names {
            if let Some(v) = t.var(name).filter(|v| is_data(v)) {
                s.set(name, fact_to_json(v));
            }
        }
        s
    }

    /// The handler as it ran before the cache: a copy of the template, every
    /// fact loaded, every derivation computed where it is read, every fact
    /// compared as JSON after.
    fn uncached_event(t: &IrInterp, state: &mut CrowLiveState, event: &str, value: Option<&str>) {
        let mut it = t.clone();
        load_facts(&mut it, t, state, None);
        session_parts(&mut it, state);
        let args: Vec<CrowValue> = value
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .and_then(|j| j.as_array().cloned())
            .map(|items| items.iter().map(|x| crate::builtins::json_to_crow("", x)).collect())
            .unwrap_or_default();
        let outcome = it.guard(|it| it.transaction(event, args));
        if let Err(e) = outcome {
            state.stop(format!("{} stopped: {}", event, e));
            return;
        }
        if let Some(reason) = it.last_stop.clone() {
            state.stop(reason);
            return;
        }
        for name in t.fact_names.clone() {
            if let Some(v) = it.var(&name).filter(|v| is_data(v)) {
                let j = fact_to_json(v);
                if state.get(&name).as_ref() != Some(&j) {
                    state.set(&name, j);
                }
            }
        }
    }

    /// A page with its toast left out (a stop's id is the session's own).
    fn shown(node: &Node) -> String {
        let html = tree::html(node);
        match html.find("<div class=\"crow-notice\"") {
            Some(i) => format!("{}{}", &html[..i], &html[i..].split_once("</div>").map_or("", |(_, rest)| rest)),
            None => html,
        }
    }

    fn uncached_page(t: &IrInterp, state: &CrowLiveState) -> String {
        let render = |state: &CrowLiveState| -> Node {
            let mut it = t.clone();
            load_facts(&mut it, t, state, None);
            session_parts(&mut it, state);
            crate::ops::page_node(it.render_page("render").unwrap())
        };
        shown(&state.render_with(&render))
    }

    fn event(t: &Arc<IrInterp>, state: &mut CrowLiveState, cached: bool, ev: &str, value: Option<&str>) -> Option<String> {
        let mut copy = state.working_copy();
        if cached {
            handle_event(t, &mut copy, ev, value);
        } else {
            uncached_event(t, &mut copy, ev, value);
        }
        state.commit(copy).map(|s| s.reason)
    }

    /// A handler reads a derivation through the session's cache: computed
    /// once per change of what it reads, its own write seen by its next read
    /// (`step`: 2 then 4 after a = 1, total 2 → doubled 4), as it was when
    /// every read computed it.
    #[test]
    fn a_handler_reads_a_derivation_its_write_moved() {
        let t = handlers_template();
        let mut s = session_of(&t);
        let render = page_render(t.clone(), "render".to_string());
        let page = tree::html(&s.render_with(&render));
        assert!(page.contains("0 1 1 2 1"), "{}", page);
        let computed = || crate::ir_interp::COMPUTED.with(|c| c.get());
        let before = computed();
        assert_eq!(event(&t, &mut s, true, "step", Some("[1]")), None);
        let page = tree::html(&s.render_with(&render));
        assert!(page.contains("1 1 2 4 1") && page.contains("2/4"), "{}", page);
        // the first read is the render's value; the write moves total and
        // doubled, computed once each for the second read and the render
        assert_eq!(computed() - before, 2, "every read computing: 7");
        // b written, then total read in the same handler: 2 + 0 → b = 2 + 2
        assert_eq!(event(&t, &mut s, true, "other", Some("[2]")), None);
        let page = tree::html(&s.render_with(&render));
        assert!(page.contains("1 5 6 12 1") && page.contains("t6"), "{}", page);
        assert_eq!(event(&t, &mut s, true, "grow", Some("[\"y\"]")), None);
        assert!(tree::html(&s.render_with(&render)).contains("n1-2"));
        // a stop lands nothing, and the cache it read through holds no value
        // a later read takes for the session's
        assert!(event(&t, &mut s, true, "refuse", Some("[5]")).is_some());
        let page = tree::html(&s.render_with(&render));
        assert!(page.contains("1 5 6 12 2"), "{}", page);
        assert_eq!(shown(&s.render_with(&render)), uncached_page(&t, &s));
    }

    /// Random events through the cached handler on the thread's kept copy,
    /// and the same events through the handler as it ran before (a fresh
    /// copy, every derivation computed where read): the facts, the stops
    /// and every page are the same.
    #[test]
    fn handlers_through_the_cache_do_what_uncached_handlers_do() {
        let t = handlers_template();
        let mut cached = session_of(&t);
        let mut plain = session_of(&t);
        let render = page_render(t.clone(), "render".to_string());
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for i in 0..300 {
            let (ev, value) = match next(6) {
                0 => ("step", Some(format!("[{}]", next(5) as i64 - 2))),
                1 => ("other", Some(format!("[{}]", next(4)))),
                2 => ("grow", Some(format!("[\"{}\"]", ["x", "y", "z"][next(3) as usize]))),
                3 => ("refuse", Some(format!("[{}]", next(3)))),
                4 => ("defer", None),
                _ => ("render", None),
            };
            if ev != "render" {
                let a = event(&t, &mut cached, true, ev, value.as_deref());
                let b = event(&t, &mut plain, false, ev, value.as_deref());
                assert_eq!(a.is_some(), b.is_some(), "event {} {}", i, ev);
            }
            // (the cached session renders only now and then: its handlers
            // read what renders left in the cache, or nothing)
            if next(2) == 0 || ev == "render" {
                assert_eq!(shown(&cached.render_with(&render)), uncached_page(&t, &plain), "event {} {}", i, ev);
            }
            let facts = |s: &CrowLiveState| t.fact_names.iter().map(|n| s.get(n)).collect::<Vec<_>>();
            assert_eq!(facts(&cached), facts(&plain), "event {} {}", i, ev);
        }
    }

    /// A value read back from a compiled session is held as the IR types it:
    /// numbers the program holds as a list are a list, a tensor's numbers a
    /// tensor (finch's `clock.chain.v`, read from the compiled app, had come
    /// back a tensor, which the row the IR names for `len` does not read).
    #[test]
    fn a_value_back_from_json_is_held_as_the_ir_types_it() {
        let it = crate::ir_interp::run_script(&crate::build("x = 1\n").unwrap(), "t").unwrap();
        let done = CrowValue::ok(Value::Done);
        let listed = json_to_typed(&serde_json::json!([1.5, 2.0]), &Ty::List(Box::new(Ty::Float)), &it, &done);
        assert!(matches!(&listed.value, Value::Array(items) if items.len() == 2 && items.iter().all(|x| matches!(x.value, Value::Float(_)))), "{}", listed.value);
        let ints = json_to_typed(&serde_json::json!([1, 2]), &Ty::List(Box::new(Ty::Int)), &it, &done);
        assert!(matches!(&ints.value, Value::Array(items) if items.iter().all(|x| matches!(x.value, Value::Int(_)))), "{}", ints.value);
        let tensor = json_to_typed(&serde_json::json!([1.5, 2.0]), &Ty::tensor(Ty::Float), &it, &done);
        assert!(matches!(tensor.value, Value::NumArray(_)), "{}", tensor.value);
    }
}
