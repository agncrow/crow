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
    fn pred_cast(&self, pred: &Expr, items: &CrowType) -> &'static str {
        let param = fn_named(pred).and_then(|f| self.fns.get(f)).and_then(|sig| sig.params.first()).map(|(_, t)| t.clone());
        match (items, param) {
            (CrowType::NumArray(_), Some(Ty::Int)) => " as i64",
            (CrowType::NumArray(_), Some(Ty::Float)) => "",
            (CrowType::NumArray(inner), _) if matches!(inner.as_ref(), CrowType::Int) => " as i64",
            (CrowType::Array(inner), Some(Ty::Float)) if matches!(inner.as_ref(), CrowType::Int) => " as f64",
            _ => "",
        }
    }

    fn note_db_backend(&mut self, url: &str) {
        if url.contains("clickhouse://") {
            self.db_features.insert("clickhouse".to_string());
        }
        if url.contains("postgres://") || url.contains("postgresql://") {
            self.db_features.insert("postgres".to_string());
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
                CrowType::Result(i) => (true, *i),
                o => (false, o),
            };
            let value = if f { format!("({}).value", s) } else { format!("({})", s) };
            let json = if Self::is_empty_list(e) {
                "serde_json::Value::Array(vec![])".to_string()
            } else if matches!(inner, CrowType::NumArray(_)) {
                format!("serde_json::json!({}.to_vec().iter().map(|&_f| if _f.is_finite() && _f == (_f as i64) as f64 {{ serde_json::json!(_f as i64) }} else {{ serde_json::json!(_f) }}).collect::<Vec<_>>())", value)
            } else {
                format!("serde_json::json!({})", value)
            };
            out.push(json);
            if f {
                fallible.push(s);
            }
        }
        (out.join(", "), fallible)
    }

    fn db_guard(&self, fallible: &[String], err_value: &str, body: String) -> String {
        let stop_as = self.reactive_epilogue.is_some().then(|| self.shown_fn_name());
        let mut out = body;
        for f in fallible.iter().rev() {
            out = match &stop_as {
                Some(who) => format!("{{ if !({f}).ok {{ state.stop(format!(\"{who} stopped: {{}}\", ({f}).reason)); return; }} else {out} }}", f = f, who = who, out = out),
                None => format!(
                    "{{ if !({f}).ok {{ CrowResult {{ ok: false, value: {ev}, reason: ({f}).reason.clone(), kind: ({f}).kind.clone(), trace: vec![], captured: 0.0 }} }} else {out} }}",
                    f = f,
                    ev = err_value,
                    out = out
                ),
            };
        }
        out
    }

    fn db_query(&mut self, conn: &str, sql: &str, params: &[Expr]) -> String {
        self.needs_serde_json = true;
        let (p, fallible) = self.db_params(params);
        let body = format!(
            "{{ match {}.query(&{}, &[{}]) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: {} }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
            conn, sql, p, STAMP
        );
        self.db_guard(&fallible, "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])", body)
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
            self.current_line
        )
    }

    fn db_exec(&mut self, conn: &str, sql: &str, params: &[Expr]) -> String {
        self.needs_serde_json = true;
        let (p, fallible) = self.db_params(params);
        let body = format!(
            "{{ match {}.exec(&{}, &[{}]) {{ Ok(_n) => CrowResult {{ ok: true, value: _n, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: 0_i64, reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
            conn, sql, p
        );
        self.db_guard(&fallible, "0_i64", body)
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
        let mut arg_types: Vec<CrowType> = args.iter().map(|x| self.tt(x)).collect();
        let mut strs = arg_strs.to_vec();
        // a view's tree handed to a module function is its markup
        for (i, a) in args.iter().enumerate() {
            if let Expr::Call { target: Target::User(v), .. } = a {
                if self.views.contains(v) {
                    strs[i] = format!("({}).to_html()", strs[i]);
                    arg_types[i] = CrowType::Str;
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
        emitted.map(|code| code.replace(crate::modules::STOP_LINE, &self.current_line.to_string())).unwrap_or_else(|e| format!("compile_error!({:?})", e))
    }

    pub(crate) fn module_call(&mut self, m: &str, f: &str, args: &[Expr], arg_strs: &[String], whole: &Expr) -> String {
        if m == "db" && self.needs_crow_db {
            match f {
                "connect" => {
                    let url = arg_strs.first().cloned().unwrap_or_default();
                    self.note_db_backend(&url);
                    return self.db_connect(args, arg_strs);
                }
                "query" if arg_strs.len() >= 2 => return self.db_query(&arg_strs[0], &arg_strs[1], &args[2..]),
                "exec" if arg_strs.len() >= 2 => return self.db_exec(&arg_strs[0], &arg_strs[1], &args[2..]),
                "insert" if arg_strs.len() == 3 => return format!("{}.insert(&{}, &{}).unwrap()", arg_strs[0], arg_strs[1], arg_strs[2]),
                "migrate" if arg_strs.len() == 2 => return format!("{}.migrate(&{}).unwrap()", arg_strs[0], arg_strs[1]),
                "transact" if arg_strs.len() == 2 => return format!("{}.transact(&|_tx| {{ {}(_tx); Ok(()) }}).unwrap()", arg_strs[0], arg_strs[1]),
                _ => {}
            }
        }
        if m == "web" && self.needs_crow_web {
            match f {
                "new" => return "CrowApp::new()".to_string(),
                "docs" if arg_strs.len() == 2 => return format!("crow_web::docs::CrowDocs::new(&{}, &{})", arg_strs[0], arg_strs[1]),
                "queue" if arg_strs.len() == 1 => return format!("CrowQueue::new({} as usize)", arg_strs[0]),
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
        let ty0 = args.first().map(|x| self.tt(x)).unwrap_or(CrowType::Unknown);
        let in0 = Self::inner(&ty0);
        match name {
            "grad" if n == 1 => {
                if let Some(fname) = fn_named(&args[0]) {
                    if let Some(sig) = self.fns.get(fname).cloned() {
                        let dual = format!("_dual_{}", Self::rust_fn_name(fname));
                        let ps: Vec<String> = sig.params.iter().map(|(p, _)| p.clone()).collect();
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

        // module functions: a scalar name core and math both answer goes to
        // the module only when a tensor is among the arguments
        let overlapping = matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");
        if overlapping && self.registry.has_function(name) && args.iter().any(|x| matches!(self.tt(x), CrowType::NumArray(_))) {
            return Some(self.registry_call(name, args, a));
        }
        let frame_select_widget = name == "select" && self.needs_ui && n == 2 && self.registry.module_name_for(name).map_or(false, |m| m == "frame");
        let frame_core = self.registry.module_name_for(name).map_or(false, |m| m == "frame")
            && crate::checker::BUILTINS.contains(&name)
            && !args.first().map(|x| matches!(Self::inner(&self.tt(x)), CrowType::Frame(_) | CrowType::FrameCol)).unwrap_or(false);
        let element_in_view = self.in_view() && crate::lower::UI_ELEMENTS.contains(&name);
        if !overlapping && !frame_select_widget && !frame_core && !element_in_view && self.registry.has_function(name) {
            return Some(self.registry_call(name, args, a));
        }

        Some(match name {
            // math on the dual numbers a function grad differentiates, each
            // the derivative's own form (a plain number's is its row)
            "abs" if n == 1 => format!("({}).abs()", a[0]),
            "sqrt" if n == 1 => format!("({}).sqrt()", a[0]),
            "floor" | "ceil" | "round" if n == 1 => format!("({}).{}()", a[0], name),
            "captured" if n == 1 => {
                self.needs_crow_runtime = true;
                if matches!(ty0, CrowType::Result(_)) {
                    format!(
                        "{{ let _v = &({}); if !_v.ok {{ CrowResult {{ ok: false, value: 0.0, reason: _v.reason.clone(), kind: _v.kind.clone(), trace: _v.trace.clone(), captured: 0.0 }} }} else if _v.captured > 0.0 {{ CrowResult {{ ok: true, value: _v.captured, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} else {{ CrowResult {{ ok: false, value: 0.0, reason: \"this value was computed here, not read from outside - it has no capture moment\".to_string(), kind: \"no_capture\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                        a[0]
                    )
                } else {
                    format!(
                        "{{ let _ = {}; CrowResult {{ ok: false, value: 0.0, reason: \"this value was computed here, not read from outside - it has no capture moment\".to_string(), kind: \"no_capture\".to_string(), trace: vec![], captured: 0.0 }} }}",
                        a[0]
                    )
                }
            }
            // a web queue's push (crow_web): a collection's is its row
            "push" if n == 2 => format!("{}.push({})", a[0], a[1]),
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
                    CrowType::Frame(_) => format!("{{ match ({}).to_csv() {{ Ok(_t) => state.download(&{}, _t.into_bytes()), Err(_e) => state.stop(format!(\"download: {{}}\", _e)) }} {} }}", a[0], a[1], stop),
                    CrowType::Map(_) => format!(
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
                let answer = |sent: &str| {
                    format!(
                        "match {} {{ Ok(()) => CrowResult {{ ok: true, value: true, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: false, reason: format!(\"mail to {{}} failed: {{}}\", _to, _e), kind: \"mail\".to_string(), trace: vec![{}], captured: 0.0 }} }}",
                        sent, tr
                    )
                };
                if self.reactive_event_handlers.contains(&self.cur_fn) {
                    let send = format!(
                        "{{ let _m = ({}).clone(); let _s = _mail.clone(); let _f: crow_web::live::MailSender = std::sync::Arc::new(move || _m.send_blocking(&_s.to, &_s.subject, &_s.body)); _f }}",
                        a[0]
                    );
                    if matches!(whole, Expr::Effect { answered: true, .. }) {
                        format!("{{ let _mail = {}; let _to = _mail.to.clone(); let _send = {}; {} }}", mail, send, answer("state.mail_now(_mail, _send)"))
                    } else {
                        format!("{{ let _mail = {}; let _send = {}; state.mail(_mail, _send); CrowResult {{ ok: true, value: true, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }}", mail, send)
                    }
                } else {
                    format!(
                        "{{ let _to = format!(\"{{}}\", {}); let _subject = format!(\"{{}}\", {}); let _body = format!(\"{{}}\", {}); {} }}",
                        a[1],
                        a[2],
                        a[3],
                        answer(&format!("({}).send_blocking(&_to, &_subject, &_body)", a[0]))
                    )
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
                    ls, rs, cond, self.current_line, op
                )
            }
            "assert" if n == 1 || n == 2 => {
                let cond = match &ty0 {
                    CrowType::Bool => a[0].clone(),
                    CrowType::Result(_) => format!("{{ let _a = &({}); if !_a.ok {{ crow_stop({}, format!(\"cannot branch on an error value — {{}}\", _a.reason)); }} _a.value }}", a[0], self.current_line),
                    CrowType::Int => format!("({}) != 0", a[0]),
                    CrowType::Str => format!("!({}).is_empty()", a[0]),
                    _ => format!("({}) != 0", a[0]),
                };
                // a failed assert stops with the interpreter's words, not
                // the Rust expression it was printed as
                if n == 2 {
                    format!("if !({}) {{ crow_stop({}, format!(\"assertion failed: {{}}\", {})); }}", cond, self.current_line, a[1])
                } else {
                    format!("if !({}) {{ crow_stop({}, \"assertion failed\".to_string()); }}", cond, self.current_line)
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
                fn word(t: &CrowType, v: &str) -> String {
                    match t {
                        CrowType::Int => "\"int\"".to_string(),
                        CrowType::Float => "\"float\"".to_string(),
                        CrowType::Str | CrowType::Markup => "\"str\"".to_string(),
                        CrowType::Bool => "\"bool\"".to_string(),
                        // a frame's column is the list of its cells there
                        CrowType::Array(_) | CrowType::NumArray(_) | CrowType::FrameCol => "\"array\"".to_string(),
                        CrowType::Sparse => "\"sparse\"".to_string(),
                        CrowType::Map(_) => "\"map\"".to_string(),
                        CrowType::Fn(_, _) => "\"fn\"".to_string(),
                        CrowType::Void => "\"none\"".to_string(),
                        CrowType::Frame(_) => "\"frame\"".to_string(),
                        CrowType::DbHandle => "\"db\"".to_string(),
                        // the runtime's records are the interpreter's maps
                        CrowType::Struct(nm) if matches!(nm.as_str(), "CrowShell" | "CrowFindResult") => "\"map\"".to_string(),
                        CrowType::Struct(nm) => format!("\"{}\"", nm),
                        CrowType::Json => format!("crow_runtime::json::type_word(&({}))", v),
                        CrowType::Result(inner) => format!("{{ let _v = &({}); if _v.ok {{ {} }} else {{ \"error\" }} }}", v, word(inner, "_v.value")),
                        _ => "\"unknown\"".to_string(),
                    }
                }
                self.needs_crow_runtime = true;
                format!("({}).to_string()", word(&ty0, &a[0]))
            }
            // `json[map]`, a web response (use web): `json[text]` is its row
            "json" if n == 1 => {
                self.needs_serde_json = true;
                format!("CrowResponse::json(serde_json::to_value(&{}).unwrap())", a[0])
            }
            // search: a parallel early exit
            "any" | "all" if n == 2 => {
                self.needs_rayon = true;
                if matches!(in0, CrowType::NumArray(_)) {
                    let cast = self.pred_cast(&args[1], &in0);
                    return Some(format!("{}.to_vec().par_iter().{}(|&_x| {}(_x{}))", a[0], name, a[1], cast));
                }
                let call = match self.pred_cast(&args[1], &in0) {
                    "" if self.pred_takes_ref(&args[1]) => "(_x)".to_string(),
                    "" => "(_x.clone())".to_string(),
                    cast => format!("(*_x{})", cast),
                };
                format!("{}.par_iter().{}(|_x| {}{})", a[0], name, a[1], call)
            }
            "find" if n == 2 && matches!(in0, CrowType::Array(_) | CrowType::NumArray(_)) => {
                self.needs_rayon = true;
                let el = match &in0 {
                    CrowType::Array(i) | CrowType::NumArray(i) => i.as_ref().clone(),
                    _ => CrowType::Unknown,
                };
                let default = el.default_rust_value();
                if matches!(in0, CrowType::NumArray(_)) {
                    let cast = if matches!(el, CrowType::Int) { " as i64" } else { "" };
                    let call_cast = self.pred_cast(&args[1], &in0);
                    return Some(format!(
                        "{{ match {}.to_vec().par_iter().copied().find_first(|&_x| {}(_x{})) {{ Some(_v) => CrowResult {{ ok: true, value: _v{}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, None => CrowResult {{ ok: false, value: {}, reason: \"no match\".to_string(), kind: \"find\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                        a[0], a[1], call_cast, cast, default
                    ));
                }
                let call = match self.pred_cast(&args[1], &in0) {
                    "" if self.pred_takes_ref(&args[1]) => "(*_x)".to_string(),
                    "" => "((*_x).clone())".to_string(),
                    cast => format!("(**_x{})", cast),
                };
                format!(
                    "{{ match {}.par_iter().find_first(|_x| {}{}) {{ Some(_v) => CrowResult {{ ok: true, value: _v.clone(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, None => CrowResult {{ ok: false, value: {}, reason: \"no match\".to_string(), kind: \"find\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
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
            "date" if n == 3 => {
                self.needs_chrono = true;
                format!(
                    "{{ let _y = {} as i32; let _m = {} as u32; let _d = {} as u32; let _dt = chrono::NaiveDate::from_ymd_opt(_y, _m, _d).expect(\"invalid date\"); let mut _map = std::collections::BTreeMap::new(); _map.insert(\"year\".to_string(), _y as i64); _map.insert(\"month\".to_string(), _m as i64); _map.insert(\"day\".to_string(), _d as i64); _map.insert(\"timestamp\".to_string(), _dt.and_hms_opt(0,0,0).unwrap().and_utc().timestamp()); _map }}",
                    a[0], a[1], a[2]
                )
            }
            "elapsed" if n == 2 => {
                self.needs_elapsed_struct = true;
                format!(
                    "{{ let _t1 = {}.get(\"timestamp\").copied().unwrap_or(0i64); let _t2 = {}.get(\"timestamp\").copied().unwrap_or(0i64); let _d = (_t2 - _t1) as f64; CrowElapsed {{ seconds: _d, minutes: _d / 60.0, hours: _d / 3600.0, days: _d / 86400.0 }} }}",
                    a[0], a[1]
                )
            }
            "today" if n == 0 => {
                self.needs_chrono = true;
                "chrono::Local::now().format(\"%Y-%m-%d\").to_string()".to_string()
            }
            _ => return self.web_builtin(name, args, a).or_else(|| self.ui_builtin(name, args, a)),
        })
    }

    /// The web module's builtins, the databases', and the page's.
    fn web_builtin(&mut self, name: &str, args: &[Expr], a: &[String]) -> Option<String> {
        let n = a.len();
        let web = self.needs_crow_web;
        let db = self.needs_crow_db;
        Some(match name {
            "route" if web && n == 2 => format!("__crow_app = __crow_app.route(&{}, Arc::new({}))", a[0], a[1]),
            "route" if web && n == 3 && matches!(self.tt(&args[0]), CrowType::Str) => format!("__crow_app = __crow_app.route_method(&{}, {}, Arc::new({}))", a[0], a[1], a[2]),
            "route" if web && n == 3 => format!("{}.route(&{}, Arc::new({}))", a[0], a[1], a[2]),
            "route" if web && n == 4 => format!("{}.route_method(&{}, {}, Arc::new({}))", a[0], a[1], a[2], a[3]),
            "serve" if web && n == 1 => format!("__crow_app.serve({} as u16).await.unwrap()", a[0]),
            "serve" if web && n == 9 => {
                if self.has_page_routes {
                    format!(
                        "{{ crow_web::live::configure({} as i64, {} as i64); crow_web::live::configure_look(&{}, &{}, &{}, {}); crow_web::live::configure_keep({}); crow_web::live::files::configure_uploads({} as i64); __crow_app.serve({} as u16).await.unwrap() }}",
                        a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[0]
                    )
                } else {
                    format!("__crow_app.serve({} as u16).await.unwrap()", a[0])
                }
            }
            "respond" if web && n == 2 => format!("CrowResponse::respond({} as u16, {})", a[0], a[1]),
            "redirect" if web && n == 1 => format!("CrowResponse::redirect({})", a[0]),
            "html" if web && n == 1 => format!("CrowResponse::html({})", a[0]),
            "text" if web && n == 1 => format!("CrowResponse::text({})", a[0]),
            "msgpack" if web && n == 1 => {
                self.needs_serde_json = true;
                self.web_features.insert("msgpack".to_string());
                format!("CrowResponse::msgpack(rmp_serde::to_vec(&serde_json::to_value(&{}).unwrap()).unwrap())", a[0])
            }
            "arrow" if web && n == 1 => {
                self.web_features.insert("arrow_ipc".to_string());
                self.needs_crow_runtime = true;
                if matches!(self.tt(&args[0]), CrowType::Result(_)) {
                    format!(
                        "{{ let _fr = &({}); if !_fr.ok {{ CrowResponse::text(format!(\"error: {{}}\", _fr.reason)) }} else {{ match crow_runtime::frame::to_arrow_ipc(&_fr.value) {{ Ok(_b) => CrowResponse::arrow_ipc(_b), Err(_e) => CrowResponse::text(format!(\"error: {{}}\", _e)) }} }} }}",
                        a[0]
                    )
                } else {
                    format!("match crow_runtime::frame::to_arrow_ipc(&{}) {{ Ok(_b) => CrowResponse::arrow_ipc(_b), Err(_e) => CrowResponse::text(format!(\"error: {{}}\", _e)) }}", a[0])
                }
            }
            "bytes" if web && n == 1 => format!("CrowResponse::binary(200, {}.into_bytes(), \"application/octet-stream\")", a[0]),
            "stream" if web && n == 1 => {
                self.web_features.insert("stream".to_string());
                format!(
                    "{{ let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(256); tokio::spawn(async move {{ let send = |data: String| {{ let tx = tx.clone(); tokio::task::block_in_place(|| {{ tx.blocking_send(Ok(data)).ok(); }}); }}; ({})(&send); }}); let stream = tokio_stream::wrappers::ReceiverStream::new(rx); let body = axum::body::Body::from_stream(stream); CrowResponse {{ status: 200, headers: std::collections::HashMap::from([(\"content-type\".to_string(), \"application/octet-stream\".to_string())]), body: String::new(), binary_body: None }} }}",
                    a[0]
                )
            }
            "connect" if db && !web => {
                self.note_db_backend(&a[0]);
                self.db_connect(args, a)
            }
            "connect" if db && web && matches!(Self::lit_str(&args[0]), Some(u) if u.contains("://") && !u.starts_with("ws")) => {
                self.note_db_backend(&a[0]);
                self.db_connect(args, a)
            }
            "query" if db && n >= 2 => self.db_query(&a[0], &a[1], &args[2..]),
            "exec" if db && n >= 2 => self.db_exec(&a[0], &a[1], &args[2..]),
            "insert" if db && n == 3 => format!("{}.insert(&{}, &{}).unwrap()", a[0], a[1], a[2]),
            "migrate" if db && n == 2 => format!("{}.migrate(&{}).unwrap()", a[0], a[1]),
            "transact" if db && n == 2 => format!("{}.transact(&|_tx| {{ {}(_tx); Ok(()) }}).unwrap()", a[0], a[1]),
            "sqlite" if web && n == 1 => format!("CrowSqlite::open({})", a[0]),
            "exec" if web && n >= 2 => format!("{}.execute({}, &[{}])", a[0], a[1], a[2..].join(", ")),
            "query" if web && n >= 2 => format!("{}.query({}, &[{}])", a[0], a[1], a[2..].join(", ")),
            "sign" if web && n == 2 => format!("CrowJwt::sign({}, {})", a[0], a[1]),
            "verify" if web && n == 2 => format!("CrowJwt::verify({}, {})", a[0], a[1]),
            "cors" if web && n == 1 => format!("{}.cors_permissive()", a[0]),
            "cors" if web && n == 2 => format!("{}.cors(CrowCors::origins({}))", a[0], a[1]),
            "sessions" if web && n == 0 => "CrowSessionStore::new()".to_string(),
            "templates" if web && n == 1 => format!("CrowTemplates::from_glob({}).unwrap()", a[0]),
            "render" if web && n == 3 => {
                self.needs_serde_json = true;
                format!("{}.render({}, serde_json::to_value(&{}).unwrap()).unwrap()", a[0], a[1], a[2])
            }
            "cache" if web && n == 1 => format!("CrowCache::new({} as u64)", a[0]),
            "cache" if web && n == 2 => format!("CrowCache::with_ttl({} as u64, {} as u64)", a[0], a[1]),
            "limit" if web && n == 2 => format!("CrowRateLimiter::new({} as u64, {} as u64)", a[0], a[1]),
            "uploads" if web && n == 1 => format!("{}.uploads().await", a[0]),
            "sse" if web && n == 3 => format!("{}.sse_route(&{}, Arc::new({}))", a[0], a[1], a[2]),
            "ws" if web && n == 3 => format!("{}.ws_route(&{}, Arc::new({}))", a[0], a[1], a[2]),
            "connect" if web && n == 1 => format!("ws::connect(&{})", a[0]),
            "send" if web && n == 2 => format!("{}.send(&{})", a[0], a[1]),
            "recv" if web && n == 1 => format!("{}.recv().unwrap_or_default()", a[0]),
            "close" if web && n == 1 => format!("{}.close()", a[0]),
            "page" if web && (n == 2 || n == 3) => self.page_route(args, a),
            "static_dir" if web && n == 3 => format!("{}.static_dir(&{}, &{})", a[0], a[1], a[2]),
            "smtp" if web && n == 4 => format!("CrowMailer::new(&{}, {} as u16, &{}, &{}).unwrap()", a[0], a[1], a[2], a[3]),
            "fetch" if n == 1 => format!("{{ __crow_state.register_fetch(\"_anon\", &{}); serde_json::Value::Null }}", a[0]),
            "poll" if n == 2 => format!("{{ __crow_state.register_poll(\"_anon\", &{}, {} as u64); serde_json::Value::Null }}", a[0], a[1]),
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

    #[allow(dead_code)]
    fn string_lit(s: &str) -> String {
        rust_str_lit(s)
    }
}
