//! Builtins and module functions: each prints as the runtime call (or the
//! Rust) the generated programs have always used.

use super::exprs::rust_str_lit;
use super::program::fn_named;
use super::*;

/// The moment a reader stamps its answer with.
const STAMP: &str = "std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)";

impl Emitter {
    fn lit_str(e: &Expr) -> Option<&str> {
        match Emitter::raw(e) {
            Expr::Const(Const::Str(s), _) => Some(s),
            _ => None,
        }
    }

    fn is_empty_list(e: &Expr) -> bool {
        matches!(Emitter::raw(e), Expr::ListLit(items, _) if items.is_empty())
    }

    fn pred_takes_ref(&self, pred: &Expr) -> bool {
        if let Some(f) = fn_named(pred) {
            if let Some(sig) = self.fns.get(f) {
                if let Some((p, _)) = sig.params.first() {
                    return self.by_ref_params.get(f).map_or(false, |s| s.contains(p));
                }
            }
        }
        false
    }

    /// The cast an item takes to reach a predicate's parameter: an int list's
    /// item into a float parameter, a tensor's number (a float) into an int
    /// one — as the interpreter hands a function value its argument, an int
    /// to a float parameter a float (compiled, `any[[1.5], big]` beside
    /// `any[[1], big]` did not build).
    fn pred_cast(&self, pred: &Expr, items: &Ty) -> &'static str {
        let param = fn_named(pred).and_then(|f| self.fns.get(f)).and_then(|sig| sig.params.first()).map(|(_, t)| t.clone());
        match (items, param) {
            (Ty::Tensor(..), Some(Ty::Int)) => " as i64",
            (Ty::Tensor(..), Some(Ty::Float)) => "",
            (Ty::Tensor(inner, _), _) if matches!(inner.as_ref(), Ty::Int) => " as i64",
            (Ty::List(inner), Some(Ty::Float)) if matches!(inner.as_ref(), Ty::Int) => " as f64",
            _ => "",
        }
    }


    /// SQL parameters: each element of a literal list binds as its own
    /// value; a fallible element reports itself before the statement runs.
    fn db_params(&mut self, params: &[Expr]) -> (String, Vec<String>) {
        let mut out = Vec::new();
        let mut fallible = Vec::new();
        let elems: Vec<Expr> = params
            .iter()
            .flat_map(|p| match Emitter::raw(p) {
                Expr::ListLit(items, _) => items.iter().map(|i| Emitter::raw(i).clone()).collect::<Vec<_>>(),
                other => vec![other.clone()],
            })
            .collect();
        for e in &elems {
            let s = self.expr(e);
            let t = self.tt(e);
            let (f, inner) = match t {
                Ty::Result(i) => (true, *i),
                o => (false, o),
            };
            let value = if f { format!("({}).value", s) } else { format!("({})", s) };
            // a list parameter whose items can fail (`IN ?` given `[[1.5,
            // px]]`): each item's payload bound, the first that failed the
            // statement's answer (it had not built: a value that can fail is
            // no JSON)
            let items_fail = match &inner {
                Ty::List(item) if item.is_result() => Some(item.payload().clone()),
                _ => None,
            };
            let json = if Self::is_empty_list(e) {
                "serde_json::Value::Array(vec![])".to_string()
            } else if matches!(inner, Ty::Tensor(..)) {
                format!("serde_json::json!({}.to_vec().iter().map(|&_f| if _f.is_finite() && _f == (_f as i64) as f64 {{ serde_json::json!(_f as i64) }} else {{ serde_json::json!(_f) }}).collect::<Vec<_>>())", value)
            } else if items_fail.is_some() {
                format!("serde_json::Value::Array({}.iter().map(|_x| serde_json::json!(_x.value)).collect())", value)
            } else {
                format!("serde_json::json!({})", value)
            };
            out.push(json);
            if f {
                fallible.push(s.clone());
            }
            if let Some(item) = items_fail {
                fallible.push(format!(
                    "({}).iter().find(|_x| !_x.ok).cloned().unwrap_or_else(|| CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }})",
                    value,
                    item.default_rust_value()
                ));
            }
        }
        (out.join(", "), fallible)
    }

    /// A statement given a parameter that failed answers the failure, as
    /// any call does; in a handler, one held to its commit (a write) stops
    /// the handler instead (spec, A handler is a transaction) — a read had
    /// stopped it too, where the interpreter answered the failure.
    fn db_guard(&self, fallible: &[String], err_value: &str, body: String, held: bool) -> String {
        let stop_as = (held && self.reactive_epilogue.is_some()).then(|| self.shown_fn_name());
        let mut out = body;
        for f in fallible.iter().rev() {
            out = match &stop_as {
                Some(who) => format!("{{ if !({f}).ok {{ state.stop(format!(\"{who} stopped: {{}}\", ({f}).reason)); return; }} else {out} }}", f = f, who = who, out = out),
                None => format!(
                    "{{ if !({f}).ok {{ CrowResult {{ ok: false, value: {ev}, reason: ({f}).reason.clone(), kind: ({f}).kind.clone(), trace: ({f}).trace.clone(), captured: 0.0 }} }} else {out} }}",
                    f = f,
                    ev = err_value,
                    out = out
                ),
            };
        }
        out
    }

    fn db_query(&mut self, name: &str, conn: &str, sql: &str, params: &[Expr]) -> String {
        self.needs_serde_json = true;
        let (p, fallible) = self.db_params(params);
        let body = format!(
            "{{ match {}.query(&{}, &[{}]) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: {} }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
            conn, sql, p, STAMP
        );
        self.db_guard(&fallible, "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])", body, crate::table::held(name))
    }

    /// `connect[url]`, with a pool, and with the statements the lowering
    /// found declaring tables on the connection (spec, db, The schema a
    /// program declares): one call in both modes, which checks the database
    /// holds them, and a connection that fails stops the program with its
    /// reason, as the interpreter stops.
    fn db_connect(&mut self, args: &[Expr], a: &[String]) -> String {
        let pool = match a.get(1) {
            Some(n) => format!("Some({} as usize)", n),
            None => "None".to_string(),
        };
        let declared: Vec<String> = match args.get(2) {
            Some(Expr::ListLit(items, _)) => items.iter().filter_map(|i| if let Expr::Const(Const::Str(s), _) = i { Some(format!("{:?}", s)) } else { None }).collect(),
            _ => Vec::new(),
        };
        format!(
            "crow_db::schema::connect(&{}, {}, &[{}]).unwrap_or_else(|e| crow_runtime::stops::halt({}, &e))",
            a.first().cloned().unwrap_or_default(),
            pool,
            declared.join(", "),
            self.line_now()
        )
    }

    /// `describe[conn, sql]`: the columns a statement answers, as a frame
    /// of name and kind; what the compiler asks at build time, asked by
    /// the program as it runs.
    fn db_describe(&mut self, conn: &str, sql: &str) -> String {
        format!(
            "{{ match {}.describe(&{}) {{ Ok(_c) => CrowResult {{ ok: true, value: crow_db::driven::describe_frame(&_c), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
            conn, sql
        )
    }

    /// `transact[conn, f]`: `f`'s value, committed — or the error that
    /// rolled the transaction back: `f`'s own when it failed, the server's
    /// when the commit was refused (crow_db::DbHandle::transact; the
    /// interpreter's transact answers the same).
    fn db_transact(&mut self, conn: &str, f: &str) -> String {
        let ret = self.fns.get(f).map(|x| x.ret.clone()).unwrap_or(Ty::Unknown);
        let fallible = matches!(ret, Ty::Result(_));
        let default = Self::inner(&as_inferred(&ret)).default_rust_value();
        let failed = format!("CrowResult {{ ok: false, value: {}, reason: __e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }}", default);
        let (judged, kept, refused) = if fallible {
            ("let __failed = !__v.ok; let __why = __v.reason.clone();", "__v", "if !__v.ok { __v } else { CrowResult { ok: false, value: __v.value, reason: __e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 } }")
        } else {
            ("let __failed = false; let __why = String::new();", "CrowResult { ok: true, value: __v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }", "CrowResult { ok: false, value: __v, reason: __e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }")
        };
        format!(
            "{{ let __kept = std::cell::RefCell::new(None); let __done = {conn}.transact(&|_tx| {{ let __v = {f}(_tx); {judged} *__kept.borrow_mut() = Some(__v); if __failed {{ Err(__why) }} else {{ Ok(()) }} }}); match (__done, __kept.into_inner()) {{ (Ok(()), Some(__v)) => {kept}, (Err(__e), Some(__v)) => {refused}, (Err(__e), None) => {{ let __e = format!(\"transaction rolled back: {{}}\", __e); {failed} }}, (Ok(()), None) => {{ let __e = \"transaction rolled back: the function never ran\".to_string(); {failed} }} }} }}"
        )
    }

    /// A module function by the registry: `math.zeros`, `frame.load`.
    /// An argument a collection keeps (a pushed value, a map's key or
    /// value): a variable read is cloned, so the variable stays usable and a
    /// loop's borrowed variable becomes a value of its own.
    pub(crate) fn kept(arg: &Expr, printed: &str) -> String {
        if Emitter::place_read(arg) {
            format!("({}).clone()", printed)
        } else {
            printed.to_string()
        }
    }

    fn registry_call(&mut self, name: &str, args: &[Expr], arg_strs: &[String]) -> String {
        let mut arg_types: Vec<Ty> = args.iter().map(|x| self.tt(x)).collect();
        let mut strs = arg_strs.to_vec();
        // a view's tree handed to a module function is its markup
        for (i, a) in args.iter().enumerate() {
            if let Expr::Call { target: Target::User(v), .. } = a {
                if self.views.contains(v) {
                    strs[i] = format!("({}).to_html()", strs[i]);
                    arg_types[i] = Ty::Str;
                }
            }
        }
        // (a frame's column handed where numbers are taken arrives converted
        // by the lowering, the same way in both modes; handed to `with` it
        // is the column, its missing cells kept)
        self.needs_crow_runtime = true;
        let emitted = match &self.pinned_module {
            Some(m) => self.registry.emit_rust_in(m, name, &strs, &arg_types),
            None => self.registry.emit_rust(name, &strs, &arg_types),
        };
        // a module stops on the line its call is on (modules::STOP_LINE)
        emitted.map(|code| code.replace(crate::modules::STOP_LINE, &self.line_now())).unwrap_or_else(|e| format!("compile_error!({:?})", e))
    }

    pub(crate) fn module_call(&mut self, m: &str, f: &str, args: &[Expr], arg_strs: &[String], whole: &Expr) -> String {
        if m == "db" && self.needs_crow_db {
            match f {
                "connect" => return self.db_connect(args, arg_strs),
                "query" | "__query_write" if arg_strs.len() >= 2 => return self.db_query(f, &arg_strs[0], &arg_strs[1], &args[2..]),
                "describe" if arg_strs.len() == 2 => return self.db_describe(&arg_strs[0], &arg_strs[1]),
                "insert" if arg_strs.len() == 3 => return format!("{}.insert(&{}, &{}).unwrap()", arg_strs[0], arg_strs[1], arg_strs[2]),
                "transact" if arg_strs.len() == 2 => return self.db_transact(&arg_strs[0], &arg_strs[1]),
                _ => {}
            }
        }
        if m == "web" && self.needs_crow_web {
            match f {
                "new" => return "CrowApp::new()".to_string(),
                _ => {}
            }
        }
        // ui.input[type = 'text', input = handler]: the element, written
        // through its module
        if m == "ui" && f == "input" && self.needs_ui {
            let mut result = "CrowVNode::input()".to_string();
            if let Some(Expr::MapLit(pairs, _)) = args.first().map(Emitter::raw) {
                for (k, v) in pairs.clone() {
                    let is_fn_ref = matches!(Emitter::raw(&v), Expr::FnRef(h, _) if self.reactive_event_handlers.contains(h));
                    let x = if is_fn_ref { format!("\"{}\"", fn_named(&v).unwrap_or_default()) } else { self.expr(&v) };
                    result = match k.as_str() {
                        "click" | "input" | "submit" => {
                            if is_fn_ref {
                                format!("{}.on_{}({})", result, k, x)
                            } else {
                                format!("{}.on_{}(&{})", result, k, x)
                            }
                        }
                        _ => format!("{}.attr(&{}, &{})", result, rust_str_lit(&k), x),
                    };
                }
            }
            return result;
        }
        if self.registry.is_loaded(m) && self.registry.has_function(f) {
            let saved = self.pinned_module.replace(m.to_string());
            let out = self.registry_call(f, args, arg_strs);
            self.pinned_module = saved;
            return out;
        }
        // a bare builtin reached through its module's name
        match self.builtin(f, args, arg_strs, whole) {
            Some(s) => s,
            None => format!("{}_{}({})", m.replace('.', "_"), f, arg_strs.join(", ")),
        }
    }

    // ── the builtins ────────────────────────────────────────────────

    pub(crate) fn builtin(&mut self, name: &str, args: &[Expr], a: &[String], whole: &Expr) -> Option<String> {
        let n = a.len();
        let ty0 = args.first().map(|x| self.tt(x)).unwrap_or(Ty::Unknown);
        let in0 = Self::inner(&ty0);
        match name {
            "grad" if n == 1 => {
                if let Some(fname) = fn_named(&args[0]) {
                    if let Some(sig) = self.fns.get(fname).cloned() {
                        let ps: Vec<String> = sig.params.iter().map(|(p, _)| p.clone()).collect();
                        // over tensors: the function's traced copy run on
                        // leaves, its number differentiated backwards
                        if self.traced_functions.contains(fname) {
                            self.needs_crow_runtime = true;
                            let cps: Vec<String> = ps.iter().map(|p| format!("{}: crow_runtime::tensor::CrowTensor", p)).collect();
                            let leaves: Vec<String> = (0..ps.len()).map(|i| format!("_l[{}].clone()", i)).collect();
                            let (ret, pick) = if ps.len() == 1 { ("crow_runtime::tensor::CrowTensor", "[0].clone()") } else { ("Vec<crow_runtime::tensor::CrowTensor>", "") };
                            return Some(format!(
                                "|{}| -> {} {{ crow_runtime::autograd::gradient(&[{}], |_l| Ok(_traced_{}({}))).unwrap_or_else(|e| crow_stop({}, e)){} }}",
                                cps.join(", "), ret, ps.join(", "), Self::rust_fn_name(fname), leaves.join(", "), self.line_now(), pick
                            ));
                        }
                        let dual = format!("_dual_{}", Self::rust_fn_name(fname));
                        if ps.len() == 1 {
                            return Some(format!("|{p}: f64| -> f64 {{ {df}(CrowDual::new({p}, 1.0)).dual }}", p = ps[0], df = dual));
                        }
                        let cps: Vec<String> = ps.iter().map(|p| format!("{}: f64", p)).collect();
                        let mut parts = Vec::new();
                        for i in 0..ps.len() {
                            let cargs: Vec<String> = (0..ps.len()).map(|j| format!("CrowDual::new({}, {})", ps[j], if j == i { "1.0" } else { "0.0" })).collect();
                            parts.push(format!("{}({}).dual", dual, cargs.join(", ")));
                        }
                        return Some(format!("|{}| -> Vec<f64> {{ vec![{}] }}", cps.join(", "), parts.join(", ")));
                    }
                }
            }
            _ => {}
        }

        // module functions: the module a bare name reaches is the one the
        // arguments' kinds pick, by the one rule inference and the lowering
        // decide by (table::module_of_bare); none is the core's form below.
        // A select in a page is the widget, an element in a view the
        // element, and a message the session's to hold: their arms below,
        // not a module's row
        let kinds: Vec<crate::table::Arg> = args.iter().map(|x| crate::table::Arg::of_type(&self.tt(x))).collect();
        let providers = self.registry.providers_of(name);
        let frame_select_widget = name == "select" && self.needs_ui && n == 2 && providers.contains(&"frame");
        let element_in_view = self.in_view() && crate::lower::UI_ELEMENTS.contains(&name);
        if !frame_select_widget && !element_in_view && name != "mail" {
            if let Some(m) = crate::table::module_of_bare(name, &providers, &kinds) {
                let saved = self.pinned_module.replace(m);
                let out = self.registry_call(name, args, a);
                self.pinned_module = saved;
                return Some(out);
            }
        }

        Some(match name {
            // math on the dual numbers a function grad differentiates, each
            // the derivative's own form (a plain number's is its row)
            "abs" if n == 1 => format!("({}).abs()", a[0]),
            "sqrt" if n == 1 => format!("({}).sqrt()", a[0]),
            "floor" | "ceil" | "round" if n == 1 => format!("({}).{}()", a[0], name),
            "captured" if n == 1 => {
                self.needs_crow_runtime = true;
                if matches!(ty0, Ty::Result(_)) {
                    format!(
                        "{{ let _v = &({}); if !_v.ok {{ CrowResult {{ ok: false, value: crow_runtime::moments::Moment::default(), reason: _v.reason.clone(), kind: _v.kind.clone(), trace: _v.trace.clone(), captured: 0.0 }} }} else if _v.captured > 0.0 {{ CrowResult {{ ok: true, value: crow_runtime::moments::Moment::of_seconds(_v.captured), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} else {{ CrowResult {{ ok: false, value: crow_runtime::moments::Moment::default(), reason: \"this value was computed here, not read from outside - it has no capture moment\".to_string(), kind: \"no_capture\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                        a[0]
                    )
                } else {
                    format!(
                        "{{ let _ = {}; CrowResult {{ ok: false, value: crow_runtime::moments::Moment::default(), reason: \"this value was computed here, not read from outside - it has no capture moment\".to_string(), kind: \"no_capture\".to_string(), trace: vec![], captured: 0.0 }} }}",
                        a[0]
                    )
                }
            }
            // the command line and the terminal
            "seed" if n == 1 => {
                self.needs_crow_runtime = true;
                format!("crow_runtime::core::seed(({}) as i64)", a[0])
            }
            "later" if n >= 1 => {
                let fname = match fn_named(&args[0]) {
                    Some(f) => f.to_string(),
                    None => return Some(format!("compile_error!(\"later[] takes a function\")")),
                };
                if self.in_live() {
                    return Some(format!("{{ let _ = state.spawn_later(\"{}\", {}); }}", fname, Self::rust_fn_name(&fname)));
                }
                self.needs_later = true;
                self.needs_crow_runtime = true;
                let mut captures = String::new();
                let mut params = Vec::new();
                for (i, x) in a.iter().skip(1).enumerate() {
                    captures.push_str(&format!("let __l{} = ({}).clone(); ", i, x));
                    params.push(format!("__l{}", i));
                }
                format!("{{ {}let _ = crow_runtime::later::spawn(\"{}\", move || {{ let _ = {}({}); }}); }}", captures, fname, Self::rust_fn_name(&fname), params.join(", "))
            }
            "sign_in" if n == 1 => {
                let stop = self.stop_check();
                format!("{{ state.sign_in(&{}); {} }}", a[0], stop)
            }
            "sign_out" if n == 0 => "{ state.sign_out(); }".to_string(),
            "download" if n == 2 => {
                let stop = self.stop_check();
                match in0 {
                    Ty::Frame(_) => format!("{{ match ({}).to_csv() {{ Ok(_t) => state.download(&{}, _t.into_bytes()), Err(_e) => state.stop(format!(\"download: {{}}\", _e)) }} {} }}", a[0], a[1], stop),
                    Ty::Map(..) => format!(
                        "{{ match ({}).get(\"path\").map(|_p| std::fs::read(_p)) {{ Some(Ok(_b)) => state.download(&{}, _b), Some(Err(_e)) => state.stop(format!(\"download: {{}}\", _e)), None => state.stop(\"download: a file to hand on is a picked file, with a path\".to_string()) }} {} }}",
                        a[0], a[1], stop
                    ),
                    _ => format!("{{ state.download(&{}, format!(\"{{}}\", {}).into_bytes()); }}", a[1], a[0]),
                }
            }
            // mail[mailer, to, subject, body]: for a session, held to its
            // commit (or sent at once when the answer is used, `r = mail[…]`);
            // elsewhere sent at once. Either way a CrowResult<bool>.
            "mail" if n == 4 => {
                let tr = self.trace_format_expr();
                let mail = format!("crow_web::live::SentMail {{ to: format!(\"{{}}\", {}), subject: format!(\"{{}}\", {}), body: format!(\"{{}}\", {}) }}", a[1], a[2], a[3]);
                // the row written in Crow (src/modules/mail.co), handed the
                // mailer and the message: whether the server took it, or why
                // not — its reason names the message already
                let sent = |mailer: &str, to: &str, subject: &str, body: &str| {
                    format!("{{ let _r = _web_mail_mail({}, {}, {}, {}); if _r.ok {{ Ok(()) }} else {{ Err(_r.reason) }} }}", mailer, to, subject, body)
                };
                let answer = |call: &str| {
                    format!(
                        "match {} {{ Ok(()) => CrowResult {{ ok: true, value: true, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: false, reason: _e, kind: \"mail\".to_string(), trace: vec![{}], captured: 0.0 }} }}",
                        call, tr
                    )
                };
                if self.reactive_event_handlers.contains(&self.cur_fn) {
                    let send = format!(
                        "{{ let _m = {}; let _s = _mail.clone(); let _f: crow_web::live::MailSender = std::sync::Arc::new(move || {}); _f }}",
                        a[0],
                        sent("_m.clone()", "_s.to.clone()", "_s.subject.clone()", "_s.body.clone()")
                    );
                    if matches!(whole, Expr::Effect { answered: true, .. }) {
                        format!("{{ let _mail = {}; let _send = {}; {} }}", mail, send, answer("state.mail_now(_mail, _send)"))
                    } else {
                        format!("{{ let _mail = {}; let _send = {}; state.mail(_mail, _send); CrowResult {{ ok: true, value: true, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }}", mail, send)
                    }
                } else {
                    // (a script sends at once, through no session; under the
                    // test harness's control nothing is sent)
                    let call = format!(
                        "(if std::env::var(\"CROW_TEST_CONTROL\").is_ok() {{ Ok(()) }} else {{ {} }})",
                        sent(&a[0], &format!("format!(\"{{}}\", {})", a[1]), &format!("format!(\"{{}}\", {})", a[2]), &format!("format!(\"{{}}\", {})", a[3]))
                    );
                    answer(&call)
                }
            }
            "__reader" => {
                if self.in_live() {
                    "state.reader()".to_string()
                } else {
                    "std::collections::BTreeMap::<String, String>::new()".to_string()
                }
            }
            "notice" if n == 1 => format!("{{ state.notice(({}).to_string()); }}", a[0]),
            "__notice_text" | "__notice_at" => {
                let part = if name == "__notice_text" { "0" } else { "1" };
                if self.in_live() {
                    format!("state.notice_record().{}", part)
                } else if part == "0" {
                    "String::new()".to_string()
                } else {
                    "0.0_f64".to_string()
                }
            }
            // a function value compared with a function written: by name
            // where the value is a group's (text), by address where it is a
            // function value
            "__same_fn" if n == 2 => match self.tt(&args[0]) {
                Ty::Fns(_) => format!("({}.as_str() == {}.as_str())", a[0], a[1]),
                _ => format!("(({}) as usize == ({}) as usize)", a[0], a[1]),
            },
            "running" if n == 1 => {
                let fname = match fn_named(&args[0]).map(|s| s.to_string()).or_else(|| if let Expr::Derived(d, _) = Emitter::raw(&args[0]) { Some(d.clone()) } else { None }) {
                    Some(f) => f,
                    None => return None,
                };
                if self.derive_fns.contains(&fname) {
                    format!("state.derive_running(\"{}\")", fname)
                } else if self.in_live() {
                    format!("state.later_running(\"{}\")", fname)
                } else {
                    self.needs_later = true;
                    self.needs_crow_runtime = true;
                    format!("crow_runtime::later::running(\"{}\")", fname)
                }
            }
            // a comparison that fails reports both sides, in the runtime's
            // words, as the interpreter does (compiled, it had said only
            // "assertion failed"): each side is read once
            "assert" if n == 1 && matches!(&args[0], Expr::Binary { op, ty: Ty::Bool, .. } if matches!(op.as_str(), "eq" | "ne" | "gt" | "lt" | "gte" | "lte")) => {
                let Expr::Binary { op, l, r, .. } = &args[0] else { unreachable!() };
                self.needs_crow_runtime = true;
                let (ls, rs) = (self.expr(l), self.expr(r));
                self.push_scope();
                self.declare("_al", self.tt(l));
                self.declare("_ar", self.tt(r));
                let cond = self.binary(op, &Expr::Local("_al".to_string(), l.ty()), &Expr::Local("_ar".to_string(), r.ty()));
                self.pop_scope();
                format!(
                    "{{ let _al = {}; let _ar = {}; if !({}) {{ crow_stop({}, crow_runtime::display::assert_failed(\"{}\", &crow_runtime::display::shown(&_al), &crow_runtime::display::shown(&_ar))); }} }}",
                    ls, rs, cond, self.line_now(), op
                )
            }
            "assert" if n == 1 || n == 2 => {
                let cond = match &ty0 {
                    Ty::Bool => a[0].clone(),
                    Ty::Result(_) => format!("{{ let _a = &({}); if !_a.ok {{ crow_stop({}, format!(\"cannot branch on an error value — {{}}\", _a.reason)); }} _a.value }}", a[0], self.line_now()),
                    Ty::Int => format!("({}) != 0", a[0]),
                    Ty::Str => format!("!({}).is_empty()", a[0]),
                    _ => format!("({}) != 0", a[0]),
                };
                // a failed assert stops with the interpreter's words, not
                // the Rust expression it was printed as
                if n == 2 {
                    format!("if !({}) {{ crow_stop({}, format!(\"assertion failed: {{}}\", {})); }}", cond, self.line_now(), a[1])
                } else {
                    format!("if !({}) {{ crow_stop({}, \"assertion failed\".to_string()); }}", cond, self.line_now())
                }
            }
            "error" if n == 1 || n == 2 => {
                let tr = self.trace_format_expr();
                let kind = if n == 2 { format!("{}.to_string()", a[1]) } else { "String::new()".to_string() };
                // the value an error never has, of the type it is used as (a
                // `return` of it to a function of another type rewraps it)
                let default = Self::inner(&self.tt(whole)).default_rust_value();
                format!("CrowResult {{ ok: false, value: {}, reason: {}.to_string(), kind: {}, trace: vec![{}], captured: 0.0 }}", default, a[0], kind, tr)
            }
            "type" if n == 1 => {
                // the words the interpreter's values say, a value that can
                // fail by the value it holds (compiled, a list, a map, a
                // record, a frame or a JSON value had been "unknown")
                // a record by the name it was declared by (a module's `Day`)
                let shown: std::collections::BTreeMap<String, String> = self.structs.iter().map(|(n, d)| (n.clone(), d.shown.clone())).collect();
                fn word(t: &Ty, v: &str, shown: &std::collections::BTreeMap<String, String>) -> String {
                    match t {
                        Ty::Int | Ty::Big => "\"int\"".to_string(),
                        Ty::Decimal => "\"decimal\"".to_string(),
                        Ty::Fraction => "\"fraction\"".to_string(),
                        Ty::Float => "\"float\"".to_string(),
                        Ty::Moment => "\"moment\"".to_string(),
                        Ty::Str | Ty::Markup => "\"str\"".to_string(),
                        Ty::Bool => "\"bool\"".to_string(),
                        // a frame's column is the list of its cells there
                        Ty::List(_) | Ty::Tensor(..) | Ty::Column(_) => "\"array\"".to_string(),
                        Ty::Sparse => "\"sparse\"".to_string(),
                        Ty::Bytes => "\"bytes\"".to_string(),

                        Ty::Map(..) => "\"map\"".to_string(),
                        Ty::Fn(_, _) => "\"fn\"".to_string(),
                        Ty::Unit => "\"none\"".to_string(),
                        Ty::Frame(_) => "\"frame\"".to_string(),
                        // a handle, as the interpreter's value names it
                        Ty::Handle(h) => format!("\"{}\"", h.name),
                        // the runtime's records are the interpreter's maps
                        Ty::Record(nm) if matches!(nm.as_str(), "CrowShell" | "CrowFindResult" | "CrowReply" | "CrowAnswer") => "\"map\"".to_string(),
                        Ty::Record(nm) => format!("\"{}\"", shown.get(nm).unwrap_or(nm)),
                        // a record or none: none is the 0 it is written as
                        // (compiled, `type[t.left]` had said "unknown", and
                        // of none had stopped)
                        Ty::OptRecord(nm) => format!("(if ({}).is_some() {{ \"{}\" }} else {{ \"int\" }})", v, shown.get(nm).unwrap_or(nm)),
                        Ty::Json => format!("crow_runtime::json::type_word(&({}))", v),
                        Ty::Result(inner) => format!("{{ let _v = &({}); if _v.ok {{ {} }} else {{ \"error\" }} }}", v, word(inner, "_v.value", shown)),
                        _ => "\"unknown\"".to_string(),
                    }
                }
                self.needs_crow_runtime = true;
                // (the value is read first, as every argument is: a stop
                // inside it — an index outside its list — stops here too,
                // as it does interpreted; the word had been decided without
                // reading it)
                format!("{{ let _t = &({}); ({}).to_string() }}", a[0], word(&ty0, "(*_t)", &shown))
            }
            // `json[map]`, a web response (use web): `json[text]` is its row;
            // a frame answers as its records, the runtime's one form
            "json" if n == 1 && matches!(in0, Ty::Frame(_)) => {
                self.needs_crow_runtime = true;
                format!("CrowResponse::json(crow_runtime::frame::records(&{}))", a[0])
            }
            // (any other value as `encode` writes it — the interpreter's one
            // JSON rule: a failed value `null`, an int past 64 bits its
            // digits as text)
            "json" if n == 1 => {
                self.needs_serde_json = true;
                self.needs_crow_runtime = true;
                format!("CrowResponse::json(crow_runtime::json::ToJson::to_json(&{}))", a[0])
            }
            // search: a parallel early exit
            "any" | "all" if n == 2 => {
                self.needs_rayon = true;
                if matches!(in0, Ty::Tensor(..)) {
                    let cast = self.pred_cast(&args[1], &in0);
                    return Some(Self::on_cores(format!("{}.to_vec().par_iter().{}(|&_x| {}(_x{}))", a[0], name, a[1], cast), true));
                }
                let call = match self.pred_cast(&args[1], &in0) {
                    "" if self.pred_takes_ref(&args[1]) => "(_x)".to_string(),
                    "" => "(_x.clone())".to_string(),
                    cast => format!("(*_x{})", cast),
                };
                Self::on_cores(format!("{}.par_iter().{}(|_x| {}{})", a[0], name, a[1], call), true)
            }
            "find" if n == 2 && matches!(in0, Ty::List(_) | Ty::Tensor(..)) => {
                self.needs_rayon = true;
                let el = match &in0 {
                    Ty::List(i) | Ty::Tensor(i, _) => i.as_ref().clone(),
                    _ => Ty::Unknown,
                };
                let default = el.default_rust_value();
                if matches!(in0, Ty::Tensor(..)) {
                    let cast = if matches!(el, Ty::Int) { " as i64" } else { "" };
                    let call_cast = self.pred_cast(&args[1], &in0);
                    return Some(format!(
                        "{{ match crow_runtime::cores::run(|| {}.to_vec().par_iter().copied().find_first(|&_x| {}(_x{}))) {{ Some(_v) => CrowResult {{ ok: true, value: _v{}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, None => CrowResult {{ ok: false, value: {}, reason: \"no match\".to_string(), kind: \"find\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                        a[0], a[1], call_cast, cast, default
                    ));
                }
                let call = match self.pred_cast(&args[1], &in0) {
                    "" if self.pred_takes_ref(&args[1]) => "(*_x)".to_string(),
                    "" => "((*_x).clone())".to_string(),
                    cast => format!("(**_x{})", cast),
                };
                format!(
                    "{{ match crow_runtime::cores::run(|| {}.par_iter().find_first(|_x| {}{}).cloned()) {{ Some(_v) => CrowResult {{ ok: true, value: _v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, None => CrowResult {{ ok: false, value: {}, reason: \"no match\".to_string(), kind: \"find\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                    a[0], a[1], call, default
                )
            }
            // regular expressions
            "find" if n == 2 => {
                self.needs_crow_runtime = true;
                self.needs_find_struct = true;
                let tr = self.trace_format_expr();
                format!(
                    "{{ match crow_runtime::text::find(&*{}, &*{}) {{ Ok(Some(_g)) => CrowResult {{ ok: true, value: CrowFindResult {{ groups: _g }}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Ok(None) => CrowResult {{ ok: false, value: CrowFindResult {{ groups: Vec::new() }}, reason: \"no match\".to_string(), kind: String::new(), trace: vec![{}], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: CrowFindResult {{ groups: Vec::new() }}, reason: e, kind: \"regex\".to_string(), trace: vec![{}], captured: 0.0 }} }} }}",
                    a[0], a[1], tr, tr
                )
            }
            _ => return self.web_builtin(name, args, a).or_else(|| self.ui_builtin(name, args, a)),
        })
    }

    /// A route's handler as the server takes it, named: one whose value can
    /// fail answers its failure as the interpreter's route does (500, with
    /// its name and the reason), where it had not built; the server answers
    /// a stop the same way.
    fn route_handler(&self, arg: &Expr, text: &str) -> String {
        let name = fn_named(arg).unwrap_or(text);
        let quick = fn_named(arg).is_some_and(|f| crate::ir::quick_fns(self.part.fns.iter()).contains(f));
        // a handler that can write a table lands its writes as it ends, its
        // stop too (`__CrowWrites`, emit_main)
        let writes = if self.needs_crow_db && !quick { "let _w = __CrowWrites; " } else { "" };
        let answer = match fn_named(arg) {
            Some(f) if self.fns.get(f).is_some_and(|sig| sig.ret.is_result()) => format!(
                "move |__req: crow_web::core::CrowRequest| {{ {}let _r = {}(__req); if _r.ok {{ _r.value }} else {{ crow_web::server::route_stopped(\"{}\", &_r.reason) }} }}",
                writes,
                Self::rust_fn_name(f),
                f
            ),
            Some(f) if !writes.is_empty() => format!("move |__req: crow_web::core::CrowRequest| {{ {}{}(__req) }}", writes, Self::rust_fn_name(f)),
            _ => text.to_string(),
        };
        format!("crow_web::server::CrowHandler::{}(\"{}\", {})", if quick { "quick" } else { "new" }, name, answer)
    }

    /// The web module's builtins, the databases', and the page's.
    fn web_builtin(&mut self, name: &str, args: &[Expr], a: &[String]) -> Option<String> {
        let n = a.len();
        let web = self.needs_crow_web;
        let db = self.needs_crow_db;
        Some(match name {
            "route" if web && n == 2 => format!("__crow_app = __crow_app.route(&{}, {})", a[0], self.route_handler(&args[1], &a[1])),
            "route" if web && n == 3 => format!("__crow_app = __crow_app.route_method(&{}, &{}, {})", a[0], a[1], self.route_handler(&args[2], &a[2])),
            // route[path, method, handler, limit]: a limit a minute per client
            "route" if web && n == 4 => format!("__crow_app = __crow_app.route_limited(&{}, &{}, {}, ({}).max(0) as u64)", a[0], a[1], self.route_handler(&args[2], &a[2]), a[3]),
            "serve" if web && n == 1 => format!("__crow_app.serve({} as u16).await.unwrap()", a[0]),
            // the server's options are both modes' (crow_web): the uploads'
            // megabytes bound every route's body, the CORS origins and the
            // files directory are the app's
            "serve" if web && n == 11 => {
                let server = format!(
                    "crow_web::server::configure_uploads({} as i64); __crow_app.cors_origins(&({})[..]).files_from(&{}).serve({} as u16).await.unwrap()",
                    a[8], a[9], a[10], a[0]
                );
                if self.has_page_routes {
                    format!(
                        "{{ crow_web::live::configure({} as i64, {} as i64); crow_web::live::configure_look(&{}, &{}, &{}, {}); crow_web::live::configure_keep({}); crow_web::live::files::configure_uploads({} as i64); {} }}",
                        a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], server
                    )
                } else {
                    format!("{{ {} }}", server)
                }
            }
            // (a status HTTP has not stops the handler, as the interpreter's
            // does: `as u16` had answered 65736 as 200)
            "respond" if web && n == 2 => format!("CrowResponse::respond(crow_web::core::response::status_at(({}) as i64, {}), {})", a[0], self.line_now(), a[1]),
            "redirect" if web && n == 1 => format!("CrowResponse::redirect({})", a[0]),
            "html" if web && n == 1 => format!("CrowResponse::html({})", a[0]),
            "text" if web && n == 1 => format!("CrowResponse::text({})", a[0]),
            // as Arrow by crow_web, as the interpreter's is (msgpack is a row
            // written in Crow, src/modules/web.co)
            "arrow" if web && n == 1 => {
                self.note_feature("arrow");
                format!("CrowResponse::arrow_of(&{})", a[0])
            }
            "bytes" if web && n == 1 => format!("CrowResponse::binary(200, {}.into_bytes(), \"application/octet-stream\")", a[0]),
            // stream[feed, a, …]: feed[put, a, …] runs when the response is
            // sent, the arguments taken by value where the call is; a feed
            // whose value fails answers the reason, which crow_web logs
            "stream" if web && n >= 1 => {
                let fname = match fn_named(&args[0]) {
                    Some(f) => f.to_string(),
                    None => return Some("compile_error!(\"stream[] takes a function\")".to_string()),
                };
                let mut captures = String::new();
                let mut params = vec!["put".to_string()];
                for (i, x) in a.iter().skip(1).enumerate() {
                    captures.push_str(&format!("let __s{} = ({}).clone(); ", i, x));
                    params.push(format!("__s{}", i));
                }
                let call = format!("{}({})", Self::rust_fn_name(&fname), params.join(", "));
                let run = if self.fns.get(&fname).is_some_and(|f| f.ret.is_result()) {
                    format!("let _r = {}; if _r.ok {{ Ok(()) }} else {{ Err(_r.reason) }}", call)
                } else {
                    format!("let _ = {}; Ok(())", call)
                };
                format!("{{ {}CrowResponse::stream(\"{}\", move |put| {{ {} }}) }}", captures, fname, run)
            }
            // a connection is the database's alone, whatever else the
            // program uses: a web program's URL built at run time
            // (`connect[env['DATABASE_URL']]`) had compiled to a call to
            // nothing, from when web's socket client was `connect` too
            "connect" if db => self.db_connect(args, a),
            "query" | "__query_write" if db && n >= 2 => self.db_query(name, &a[0], &a[1], &args[2..]),
            "describe" if db && n == 2 => self.db_describe(&a[0], &a[1]),
            "insert" if db && n == 3 => format!("{}.insert(&{}, &{}).unwrap()", a[0], a[1], a[2]),
            "transact" if db && n == 2 => self.db_transact(&a[0], &a[1]),
            "page" if web && (n == 2 || n == 3) => self.page_route(args, a),
            "fetch" if n == 1 => format!("{{ __crow_state.register_fetch(\"_anon\", &{}, Vec::new()); serde_json::Value::Null }}", a[0]),
            "poll" if n == 2 => format!("{{ __crow_state.register_poll(\"_anon\", &{}, {} as u64, Vec::new()); serde_json::Value::Null }}", a[0], a[1]),
            _ => return None,
        })
    }

    /// `page[path, view]`, with its handlers and the facts in its address:
    /// those its path's `:name` segments name, and its `url` facts.
    fn page_route(&mut self, args: &[Expr], a: &[String]) -> String {
        let handle_all = if self.reactive_event_handlers.is_empty() {
            "|_: &mut CrowLiveState, _: &str, _: Option<&str>| {}".to_string()
        } else {
            "__crow_handle".to_string()
        };
        let mut pairs = Vec::new();
        if let Some(path) = Self::lit_str(&args[0]) {
            for name in crow_web::core::path::params(path) {
                match self.url_fact(&name) {
                    Some(fact) => pairs.push(format!("(\"{}\", \"{}\")", name, fact)),
                    None => return format!("compile_error!(\"page['{}', …]: ':{}' names no fact\")", path, name),
                }
            }
        }
        let listed = matches!(args.get(2).map(Emitter::raw), Some(Expr::ListLit(..)));
        if let Some(Expr::ListLit(names, _)) = args.get(2).map(Emitter::raw) {
            for n in names {
                let Some(written) = Self::lit_str(n) else {
                    return "compile_error!(\"page[…, url = …] takes fact names as text: url = ['day', 'sym']\")".to_string();
                };
                match self.url_fact(written) {
                    Some(fact) => pairs.push(format!("(\"{}\", \"{}\")", written.rsplit('.').next().unwrap_or(written), fact)),
                    None => return format!("compile_error!(\"page[…, url = …]: no reactive fact named '{}'\")", written),
                }
            }
        }
        let handle = if a.len() == 3 && !listed { a[2].clone() } else { handle_all };
        let with_url = if pairs.is_empty() { String::new() } else { format!(".with_url(vec![{}])", pairs.join(", ")) };
        format!(
            "{{ let _ws_path = format!(\"/_ws{{}}\", {}); let _lv = CrowLiveView::new(&_ws_path, __crow_state.clone(), {}, {}){}; let _lr = _lv.into_router(&{}); __crow_app = __crow_app.merge(_lr); }}",
            a[0], a[1], handle, with_url, a[0]
        )
    }

    /// The fact a `page[…, url = …]` name means: its name, or a part's fact
    /// by its path, however deep the part.
    fn url_fact(&self, written: &str) -> Option<String> {
        let flat = written.replace('.', "_");
        if self.reactive_vars.contains_key(&flat) {
            return Some(flat);
        }
        let suffix = format!("_{}", flat);
        let mut found = self.reactive_vars.keys().filter(|k| k.ends_with(&suffix));
        let first = found.next()?.clone();
        found.next().is_none().then_some(first)
    }
}
