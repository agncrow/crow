//! `crow run -i` serves pages (session-layer-design.md §3.11). The live
//! runtime keeps the sessions — their facts, the commit, the notice, the
//! questions, the address, the reconnects — exactly as it does for a
//! compiled program; the IR interpreter renders the views and runs the
//! handlers, against each session's facts. The interpreter that ran the
//! program's main is the template: every render and every handler works
//! on a copy of it holding that session's facts.

use std::sync::Arc;

use crate::ir_interp::IrInterp;
use crate::value::*;
use crow_web::live::{CrowLiveState, CrowLiveView};
use crow_web::view::CrowVNode;

/// Whether a fact is data a session holds, rather than a resource every
/// session shares (a database handle, a mailer) or a function: resources stay the
/// template's, never copied into a session.
pub(crate) fn is_data(v: &CrowValue) -> bool {
    !matches!(v.value, Value::DbConn(_) | Value::Mailer(..) | Value::Fn(_) | Value::GradFn(_) | Value::App(..))
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
        Value::StructInstance(si) => serde_json::Value::Object(si.fields.iter().map(|(k, x)| (k.clone(), fact_to_json(x))).collect()),
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
        _ => crate::builtins::json_to_crow("", j),
    }
}

/// A value back from JSON by the type the program gives it: a record
/// from the object of its fields, a list of records item by item, a
/// tensor from its numbers; anything else as `json_to_fact` reads it.
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
                CrowValue::ok(Value::StructInstance(Box::new(crate::value::StructInstance { name: name.clone(), fields })))
            }
            None => json_to_fact(j, declared),
        },
        (Ty::List(item), serde_json::Value::Array(items)) if matches!(item.payload(), Ty::Record(_) | Ty::OptRecord(_) | Ty::List(_) | Ty::Tensor(_)) => {
            let none = CrowValue::ok(Value::Done);
            CrowValue::ok(Value::Array(items.iter().map(|x| json_to_typed(x, item, template, &none)).collect()))
        }
        (Ty::Tensor(_), serde_json::Value::Array(items)) if items.iter().all(|x| x.is_number()) => {
            let data: Vec<f64> = items.iter().filter_map(|x| x.as_f64()).collect();
            let n = data.len();
            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_vec(data, vec![n]))))
        }
        (Ty::Float, serde_json::Value::Number(n)) => CrowValue::ok(Value::Float(n.as_f64().unwrap_or(0.0))),
        _ => json_to_fact(j, declared),
    }
}

/// The template with one session's facts and notice loaded.
fn with_session(template: &IrInterp, state: &CrowLiveState) -> IrInterp {
    let mut it = template.clone();
    for name in template.fact_names.clone() {
        if let (Some(j), Some(declared)) = (state.get(&name), template.var(&name)) {
            let v = match template.fact_types.get(&name) {
                Some(ty) => json_to_typed(j, ty, template, declared),
                None => json_to_fact(j, declared),
            };
            it.set_fact(&name, v);
        }
    }
    if let Some(n) = &state.notice {
        it.session_notice.insert(0, n.clone());
    }
    // the session's signed-in reader, as the runtime holds it
    let reader = state.reader().into_iter().map(|(k, v)| (k, CrowValue::ok(Value::Str(v)))).collect();
    it.session_reader.insert(0, reader);
    // the page each paged table shows
    it.session_pages = state.table_pages();
    it
}

/// The fact a `page[…, url = …]` name means, as the Rust backend resolves it.
fn url_fact(template: &IrInterp, written: &str) -> Option<String> {
    let flat = written.replace('.', "_");
    if template.fact_names.contains(&flat) {
        return Some(flat);
    }
    let suffix = format!("_{}", flat);
    let mut found = template.fact_names.iter().filter(|k| k.ends_with(&suffix));
    let first = found.next()?.clone();
    found.next().is_none().then_some(first)
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

/// Serve the page the program's main asked for, from the interpreter.
pub fn serve(template: IrInterp) -> Result<(), String> {
    let (path, view, url_names) = template.page_config.clone().ok_or("the program serves no page: add page['/', render]")?;
    let serve_args = template.serve_config.clone().unwrap_or_default();
    let port = int_of(serve_args.first()).max(0) as u16;

    let mut initial = CrowLiveState::new();
    for name in &template.fact_names {
        if let Some(v) = template.var(name).filter(|v| is_data(v)) {
            initial.set(name, fact_to_json(v));
        }
    }
    let mut url_pairs: Vec<(String, String)> = Vec::new();
    for written in &url_names {
        let fact = url_fact(&template, written).ok_or_else(|| format!("page[…, url = …]: no reactive fact named '{}'", written))?;
        url_pairs.push((written.rsplit('.').next().unwrap_or(written).to_string(), fact));
    }
    // the runtime draws the toast for a served page
    let mut template = template;
    template.harness_toast = false;
    let template = Arc::new(template);

    let t = template.clone();
    let view_name = view.clone();
    let render = move |state: &CrowLiveState| -> CrowVNode {
        let mut it = with_session(&t, state);
        match it.guard(|it| it.render_view(&view_name)) {
            Ok(html) => {
                if it.notice_read {
                    // the view shows the notice itself: no runtime toast
                    let _ = state.notice_record();
                }
                CrowVNode::raw(&html)
            }
            Err(e) => CrowVNode::text(&format!("error: {}", e)),
        }
    };

    let t = template.clone();
    let handle = move |state: &mut CrowLiveState, event: &str, value: Option<&str>| {
        let mut it = with_session(&t, state);
        let before_notice = it.session_notice.get(&0).cloned();
        let args: Vec<CrowValue> = value
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .and_then(|j| j.as_array().cloned())
            .map(|items| items.iter().map(|x| crate::builtins::json_to_crow("", x)).collect())
            .unwrap_or_default();
        let _ = crow_db::take_write_keys();
        let outcome = it.guard(|it| it.transaction(event, args));
        let written = crow_db::take_write_keys();
        if let Err(e) = outcome {
            state.stop(format!("{} stopped: {}", event, e));
            return;
        }
        if let Some(reason) = it.last_stop.clone() {
            state.stop(reason);
            return;
        }
        // what the handler changed lands in the session's working copy
        for name in t.fact_names.clone() {
            if let Some(v) = it.var(&name).filter(|v| is_data(v)) {
                let j = fact_to_json(v);
                if state.get(&name) != Some(&j) {
                    state.set(&name, j);
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
            let send = crate::ir_interp::mail_sender(mailer, &mail);
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
    };

    // a handler's thread holds the interpreter's frames to the recursion limit
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(crow_runtime::stops::STACK).build().map_err(|e| format!("cannot start the server: {}", e))?;
    rt.block_on(async move {
        let ws_path = format!("/_ws{}", path);
        let pairs: Vec<(&str, &str)> = url_pairs.iter().map(|(n, f)| (n.as_str(), f.as_str())).collect();
        let live = CrowLiveView::new(&ws_path, initial, render, handle).with_url(pairs);
        let app = crow_web::server::CrowApp::new().merge(live.into_router(&path));
        crow_web::live::configure(int_of(serve_args.get(1)), int_of(serve_args.get(2)));
        crow_web::live::configure_look(&text_of(serve_args.get(3)), &text_of(serve_args.get(4)), &text_of(serve_args.get(5)), bool_of(serve_args.get(6)));
        crow_web::live::configure_keep(bool_of(serve_args.get(7)));
        crow_web::live::files::configure_uploads(int_of(serve_args.get(8)));
        app.serve(port).await.map_err(|e| format!("the server stopped: {}", e))
    })
}
