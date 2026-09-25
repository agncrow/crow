//! The crate as a whole: what the program needs, the header, the types,
//! every function, the event dispatch and `main`.

use super::*;

// ── walking the IR (the visitors live with the IR) ──────────────────

pub(crate) use crate::ir::{call_name, visit_block, visit_expr, visit_stmts};

/// The arguments of a call or an effect.
pub(crate) fn call_args(e: &Expr) -> &[Expr] {
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } => args,
        _ => &[],
    }
}

/// A function named by a value: `f`, or a held call `f[args]`.
pub(crate) fn fn_named(e: &Expr) -> Option<&str> {
    match Emitter::raw(e) {
        Expr::FnRef(n, _) => Some(n),
        Expr::Deferred { name, .. } => Some(name),
        _ => None,
    }
}

/// The pairs of a map literal as the lowering leaves it: a `set` chain
/// over `map[]`, or a literal map.
pub(crate) fn map_literal_pairs(e: &Expr) -> Option<Vec<(String, Expr)>> {
    if let Expr::MapLit(pairs, _) = e {
        return Some(pairs.clone());
    }
    let mut out: Vec<(String, Expr)> = Vec::new();
    let mut cur = Emitter::raw(e);
    loop {
        match cur {
            Expr::Call { target: Target::Runtime(p), args, .. } if (p == "core.map_set" || p == "set") && args.len() == 3 => {
                match Emitter::raw(&args[1]) {
                    Expr::Const(Const::Str(k), _) => out.push((k.clone(), Emitter::raw(&args[2]).clone())),
                    _ => return None,
                }
                cur = Emitter::raw(&args[0]);
            }
            Expr::Call { target: Target::Runtime(p), args, .. } if p == "map" && args.is_empty() => {
                out.reverse();
                return Some(out);
            }
            _ => return None,
        }
    }
}

impl Emitter {
    /// Every block of the program: the functions', the derivations', main's.
    fn all_blocks(&self) -> Vec<Block> {
        let mut out: Vec<Block> = self.part.fns.iter().map(|f| f.body.clone()).collect();
        out.extend(self.part.derives.iter().map(|d| d.body.clone()));
        out.push(self.part.main.clone());
        out.extend(self.part.facts.iter().map(|f| vec![Stmt::Do(f.default.clone())]));
        out
    }

    /// Does any block hold an expression the predicate accepts?
    fn any_expr(&self, pred: &dyn std::ops::Fn(&Expr) -> bool) -> bool {
        let mut found = false;
        for b in self.all_blocks() {
            visit_block(&b, &mut |e| {
                if pred(e) {
                    found = true;
                }
            });
        }
        found
    }

    /// The functions a call names anywhere in a block, by the call's name.
    fn fn_args_of_calls(block: &[Stmt], name: &str, pick: &dyn std::ops::Fn(&[Expr]) -> Vec<String>) -> Vec<String> {
        let mut out = Vec::new();
        visit_block(block, &mut |e| {
            if call_name(e).as_deref() == Some(name) {
                out.extend(pick(call_args(e)));
            }
        });
        out
    }

    // ── what the program needs ──────────────────────────────────────

    fn load_module(&mut self, name: &str) {
        if self.registry.is_loaded(name) {
            return;
        }
        let m: std::sync::Arc<dyn crate::modules::CrowModule> = match name {
            "math" => std::sync::Arc::new(crate::modules::math::MathModule),
            "math.stats" => std::sync::Arc::new(crate::modules::stats::StatsModule),
            "math.nn" => std::sync::Arc::new(crate::modules::nn::NnModule),
            "frame" => std::sync::Arc::new(crate::modules::frame::FrameModule),
            "doc" => std::sync::Arc::new(crate::modules::doc::DocModule),
            "chart" => std::sync::Arc::new(crate::modules::chart::ChartModule),
            "io" => std::sync::Arc::new(crate::modules::io::IoModule),
            _ => return,
        };
        self.registry.load(m);
    }

    /// `use` paths: the crates and features they bring, the modules the
    /// registry answers for.
    fn read_uses(&mut self) {
        self.registry = crate::modules::ModuleRegistry::new();
        let uses: Vec<String> = self.part.uses.clone();
        for u in uses {
            let path = u.split(" as ").next().unwrap_or(&u).trim().to_string();
            let module_name = path.rsplit('/').next().unwrap_or(&path).to_string();
            if module_name == "math" || path.starts_with("math.") {
                self.needs_crow_runtime = true;
            }
            match path.as_str() {
                "math" => self.load_module("math"),
                "math.stats" => {
                    self.load_module("math");
                    self.load_module("math.stats");
                    self.modules.insert("math".to_string());
                }
                "math.nn" => {
                    self.load_module("math");
                    self.load_module("math.nn");
                    self.modules.insert("math".to_string());
                }
                "io" => {
                    self.needs_crow_runtime = true;
                    self.load_module("io");
                }
                "frame" | "doc" | "chart" => {
                    self.needs_crow_runtime = true;
                    self.load_module(&path);
                }
                "db" => {
                    self.needs_crow_db = true;
                    self.needs_crow_runtime = true;
                    self.db_features.insert("sqlite".to_string());
                    self.modules.insert("db".to_string());
                }
                "ui" => {
                    self.needs_ui = true;
                    self.needs_crow_web = true;
                    self.web_features.insert("live".to_string());
                    self.modules.insert("ui".to_string());
                }
                "web" => {
                    self.needs_crow_web = true;
                    self.web_features.insert("http".to_string());
                    self.modules.insert("web".to_string());
                }
                p if p.starts_with("web.") => {
                    self.needs_crow_web = true;
                    let sub = &p[4..];
                    let feature = if sub == "static" { "static_files" } else { sub };
                    self.web_features.insert(feature.to_string());
                    self.modules.insert("web".to_string());
                }
                _ => {}
            }
            self.modules.insert(module_name);
        }
    }

    /// The whole-program analyses the backend prints from, made over the
    /// IR: which functions are handlers, renders, route handlers
    /// and gradient targets, which parameters pass by reference, which
    /// facts are state, statics or data sources.
    fn analyse(&mut self) {
        self.read_uses();
        // parallel loops, search builtins and cpu[n>1] need rayon
        let par = self.any_expr(&|e| matches!(e, Expr::ForExpr { .. }))
            || self.all_blocks().iter().any(|b| {
                let mut f = false;
                visit_stmts(b, &mut |s| {
                    if matches!(s, Stmt::For { .. }) {
                        f = true;
                    }
                });
                f
            })
            || self.any_expr(&|e| matches!(call_name(e).as_deref(), Some("any" | "all" | "find")) && call_args(e).len() == 2)
            || self.part.cpu.map_or(false, |n| n > 1)
            || self.any_expr(&|e| call_name(e).as_deref() == Some("cpu") && matches!(call_args(e).first().map(Emitter::raw), Some(Expr::Const(Const::Int(n), _)) if *n > 1));
        self.needs_rayon = par;

        // gradient targets: their parameters and value are floats
        let blocks = self.all_blocks();
        let mut grads = BTreeSet::new();
        for b in &blocks {
            for f in Self::fn_args_of_calls(b, "grad", &|args| args.iter().filter_map(fn_named).map(|s| s.to_string()).collect()) {
                grads.insert(f);
            }
        }
        if !grads.is_empty() {
            self.needs_grad = true;
            for g in &grads {
                if let Some(f) = self.fns.get_mut(g) {
                    for p in f.params.iter_mut() {
                        p.1 = Ty::Float;
                    }
                    f.ret = Ty::Float;
                }
            }
            self.grad_functions = grads;
        }

        // route handlers take the request and answer a response
        for b in &blocks {
            for f in Self::fn_args_of_calls(b, "route", &|args| {
                if args.len() >= 2 {
                    args.last().and_then(fn_named).map(|s| vec![s.to_string()]).unwrap_or_default()
                } else {
                    vec![]
                }
            }) {
                self.web_handlers.insert(f);
            }
        }
        for h in self.web_handlers.clone() {
            if let Some(f) = self.fns.get_mut(&h) {
                if let Some(p) = f.params.first_mut() {
                    p.1 = Ty::Handle("web.request".into());
                }
                f.ret = Ty::Handle("web.response".into());
            }
        }

        if self.needs_ui {
            // page[path, render] and page[path, render, handle]
            let mut has_page = false;
            for b in &blocks {
                visit_block(b, &mut |e| {
                    if call_name(e).as_deref() == Some("page") {
                        let args = call_args(e);
                        if args.len() == 2 || args.len() == 3 {
                            has_page = true;
                            if let Some(r) = args.get(1).and_then(fn_named) {
                                self.live_render_fns.insert(r.to_string());
                            }
                            if args.len() == 3 {
                                if let Expr::FnRef(h, _) = Emitter::raw(&args[2]) {
                                    self.live_handle_fns.insert(h.clone());
                                }
                            }
                        }
                    }
                });
            }
            // a part the test harness drives is a page program with no page
            let has_page = has_page || self.test_control;
            self.has_page_routes = has_page;
            for r in self.live_render_fns.clone() {
                if self.fns.get(&r).map_or(false, |f| f.params.is_empty()) {
                    self.reactive_render_fns.insert(r);
                }
            }
            // every view of a page program renders from the session's state
            if has_page {
                for v in self.views.clone() {
                    self.reactive_render_fns.insert(v);
                }
            }
            if !self.reactive_render_fns.is_empty() {
                self.read_facts();
            }
            // derivations render from the state too
            for d in self.derives.keys().cloned().collect::<Vec<_>>() {
                self.derive_fns.insert(d.clone());
                self.reactive_render_fns.insert(d);
            }
            if !self.reactive_render_fns.is_empty() {
                // the handlers a render wires: events, table actions,
                // widget events, a chart's seek
                for r in self.reactive_render_fns.clone() {
                    let body = match self.fns.get(&r) {
                        Some(f) => f.body.clone(),
                        None => continue,
                    };
                    let mut found: Vec<String> = Vec::new();
                    visit_block(&body, &mut |e| {
                        match e {
                            Expr::Tree { attrs, .. } => {
                                for (k, v) in attrs {
                                    match k.as_str() {
                                        "click" | "input" | "submit" => {
                                            if let Some(f) = fn_named(v) {
                                                found.push(f.to_string());
                                            }
                                        }
                                        "actions" | "on" => {
                                            if let Some(pairs) = map_literal_pairs(v) {
                                                for (_, h) in pairs {
                                                    if let Some(f) = fn_named(&h) {
                                                        found.push(f.to_string());
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ if call_name(e).as_deref() == Some("chart") && call_args(e).len() == 12 => {
                                if let Some(f) = fn_named(&call_args(e)[11]) {
                                    found.push(f.to_string());
                                }
                            }
                            _ => {}
                        }
                    });
                    for f in found {
                        if self.fns.contains_key(&f) {
                            self.reactive_event_handlers.insert(f);
                        }
                    }
                }
                for h in self.reactive_event_handlers.clone() {
                    self.live_handle_fns.insert(h);
                }
                // later[f] from a handler or a render: f is a handler too
                loop {
                    let mut added = false;
                    let roots: Vec<String> = self.reactive_event_handlers.iter().chain(self.reactive_render_fns.iter()).cloned().collect();
                    for h in roots {
                        let body = match self.fns.get(&h) {
                            Some(f) => f.body.clone(),
                            None => continue,
                        };
                        let targets = Self::fn_args_of_calls(&body, "later", &|args| args.first().and_then(fn_named).map(|s| vec![s.to_string()]).unwrap_or_default());
                        for t in targets {
                            if self.fns.contains_key(&t) && !self.derive_fns.contains(&t) && self.reactive_event_handlers.insert(t.clone()) {
                                self.live_handle_fns.insert(t);
                                added = true;
                            }
                        }
                    }
                    if !added {
                        break;
                    }
                }
                // helpers that write facts on a handler's behalf are handlers
                let handler_kind: Vec<String> = self.fns.values().filter(|f| f.kind == FnKind::Handler).map(|f| f.name.clone()).collect();
                for h in handler_kind {
                    if !self.views.contains(&h) && self.reactive_event_handlers.insert(h.clone()) {
                        self.live_handle_fns.insert(h);
                    }
                }
                for d in self.derive_fns.clone() {
                    self.reactive_event_handlers.remove(&d);
                    self.live_handle_fns.remove(&d);
                }
            }
        }

        // a struct parameter a function never reassigns passes by reference
        // (a handler's are its own: it may be sent them as an event's
        // arguments, so it takes them by value, and so do its callers)
        let names: Vec<String> = self.fns.keys().cloned().collect();
        for n in names {
            if self.reactive_event_handlers.contains(&n) || self.live_handle_fns.contains(&n) {
                continue;
            }
            let f = self.fns.get(&n).unwrap().clone();
            let mut refs = HashSet::new();
            for (p, t) in &f.params {
                if let Ty::Record(s) = t {
                    if !self.struct_is_copy(s) && !Self::assigns(&f.body, p) {
                        refs.insert(p.clone());
                    }
                }
            }
            if !refs.is_empty() {
                self.by_ref_params.insert(n, refs);
            }
        }

        // a tensor anywhere needs the runtime
        let mut tensor = false;
        let mut json = false;
        let mut check = |t: &Ty| {
            fn has(t: &Ty, want: &dyn std::ops::Fn(&Ty) -> bool) -> bool {
                want(t)
                    || match t {
                        Ty::List(i) | Ty::Map(i) | Ty::Result(i) | Ty::Tensor(i) | Ty::Column(i) => has(i, want),
                        _ => false,
                    }
            }
            if has(t, &|t| matches!(t, Ty::Tensor(_))) {
                tensor = true;
            }
            if has(t, &|t| matches!(t, Ty::Json)) {
                json = true;
            }
        };
        for f in self.fns.values() {
            for (_, t) in &f.params {
                check(t);
            }
            check(&f.ret);
        }
        for b in self.all_blocks() {
            visit_stmts(&b, &mut |s| {
                if let Stmt::Let { ty, .. } = s {
                    check(ty);
                }
            });
        }
        for fact in &self.part.facts {
            check(&fact.ty);
        }
        if tensor {
            self.needs_crow_runtime = true;
        }
        if json {
            self.needs_serde_json = true;
        }
    }

    /// Does the block assign this local (a whole rebinding)?
    pub(crate) fn assigns(block: &[Stmt], name: &str) -> bool {
        let mut found = false;
        visit_stmts(block, &mut |s| {
            if matches!(s, Stmt::Set { var, .. } if var == name) {
                found = true;
            }
        });
        found
    }

    /// Every function the compiled program can run: what its top level,
    /// its facts, its derivations and its views and handlers call, name or
    /// hand on, and what those call in turn.
    fn reached_functions(&self) -> BTreeSet<String> {
        let mut todo: Vec<String> = Vec::new();
        let note = |e: &Expr, todo: &mut Vec<String>| match e {
            Expr::Call { target: Target::User(n), .. } | Expr::FnRef(n, _) => todo.push(n.clone()),
            Expr::Deferred { name, .. } => todo.push(name.clone()),
            _ => {}
        };
        visit_block(&self.part.main, &mut |e| note(e, &mut todo));
        for fact in &self.part.facts {
            visit_expr(&fact.default, &mut |e| note(e, &mut todo));
        }
        for d in &self.part.derives {
            visit_block(&d.body, &mut |e| note(e, &mut todo));
        }
        // what runs by itself: handlers, views and the program's callbacks;
        // a helper runs when one of them calls it
        for f in self.fns.values() {
            if !matches!(f.kind, FnKind::Pure | FnKind::Helper) || self.views.contains(&f.name) || self.reactive_event_handlers.contains(&f.name) || self.web_handlers.contains(&f.name) {
                todo.push(f.name.clone());
            }
        }
        let mut reached: BTreeSet<String> = BTreeSet::new();
        while let Some(n) = todo.pop() {
            if !reached.insert(n.clone()) {
                continue;
            }
            if let Some(f) = self.fns.get(&n) {
                visit_block(&f.body, &mut |e| note(e, &mut todo));
            }
        }
        reached
    }

    pub(crate) fn struct_is_copy(&self, name: &str) -> bool {
        self.structs.get(name).map_or(false, |s| s.fields.iter().all(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool)))
    }

    /// The program's facts, as a page program holds them: session state,
    /// a static connection, or a data source.
    fn read_facts(&mut self) {
        let facts = self.part.facts.clone();
        for fact in facts {
            if matches!(call_name(&fact.default).as_deref(), Some("fetch")) && call_args(&fact.default).len() == 1
                || matches!(call_name(&fact.default).as_deref(), Some("poll")) && call_args(&fact.default).len() == 2
            {
                self.data_source_vars.insert(fact.name.clone());
                continue;
            }
            // a connection is infrastructure: a process-wide static, as a
            // mailer is
            if matches!(fact.ty, Ty::Handle(ref k) if k == "db" || k == "mailer") || matches!(self.tt(&fact.default), CrowType::DbHandle | CrowType::Mailer) {
                self.static_handle_vars.insert(fact.name.clone());
                continue;
            }
        }
        let facts = self.part.facts.clone();
        for fact in facts {
            if self.static_handle_vars.contains(&fact.name) || self.data_source_vars.contains(&fact.name) {
                continue;
            }
            let value = fact.default.clone();
            let declared = crow_type(&fact.ty).text();
            let vty = self.tt(&value);
            let ty = if matches!(declared, CrowType::Unknown) { vty.clone() } else { declared };
            let widened = matches!(vty, CrowType::Int) && matches!(ty, CrowType::Float);
            let text = self.top_level_expr(&value);
            let open_list = matches!(Emitter::raw(&value), Expr::ListLit(items, _) if items.is_empty());
            let open_map = matches!(Emitter::raw(&value), Expr::MapLit(p, _) if p.is_empty())
                || matches!(Emitter::raw(&value), Expr::Call { target: Target::Runtime(n), args, .. } if n == "map" && args.is_empty());
            let default = if widened {
                format!("({}) as f64", text)
            } else if open_list {
                match &ty {
                    CrowType::Array(inner) => format!("Vec::<{}>::new()", inner.to_rust_type()),
                    _ => text,
                }
            } else if open_map && matches!(ty, CrowType::Map(_)) {
                ty.default_rust_value()
            } else {
                text
            };
            self.reactive_vars.insert(fact.name.clone(), (ty, default));
        }
    }

    /// An expression printed as main prints it.
    fn top_level_expr(&mut self, e: &Expr) -> String {
        let saved = std::mem::take(&mut self.cur_fn);
        let s = self.expr(e);
        self.cur_fn = saved;
        s
    }

    // ── the crate ───────────────────────────────────────────────────

    pub(crate) fn program(&mut self) -> Result<String, String> {
        self.analyse();
        if let Some(n) = self.part.cpu {
            self.cpu_count = Some(n as usize);
        }

        // header
        self.line("use std::collections::BTreeMap;");
        if self.needs_rayon {
            self.line("use rayon::prelude::*;");
        }
        if self.needs_crow_runtime {
            self.line("#[allow(unused_imports)]");
            self.line("use crow_runtime::tensor::CrowTensor;");
        }
        if self.any_type(&|t| matches!(t, Ty::Handle(k) if k == "sparse")) {
            self.line("#[allow(unused_imports)]");
            self.line("use crow_runtime::sparse::CrowSparse;");
        }
        if self.needs_crow_web {
            self.line("#[allow(unused_imports)]");
            self.line("use crow_web::server::CrowApp;");
            self.line("#[allow(unused_imports)]");
            self.line("use crow_web::core::{CrowRequest, CrowResponse};");
            self.line("#[allow(unused_imports)]");
            self.line("use std::sync::Arc;");
            let web: Vec<(&str, &str)> = vec![
                ("db", "use crow_web::db::CrowSqlite;"),
                ("auth", "use crow_web::auth::CrowJwt;"),
                ("cors", "use crow_web::cors::CrowCors;"),
                ("session", "use crow_web::session::CrowSessionStore;"),
                ("template", "use crow_web::template::CrowTemplates;"),
                ("cache", "use crow_web::cache::CrowCache;"),
                ("queue", "use crow_web::queue::CrowQueue;"),
                ("rate", "use crow_web::rate::CrowRateLimiter;"),
                ("mail", "use crow_web::mail::CrowMailer;"),
                ("ws", "use crow_web::ws;"),
                ("sse", "use crow_web::sse;"),
            ];
            for (f, l) in web {
                if self.web_features.contains(f) {
                    self.line("#[allow(unused_imports)]");
                    self.line(l);
                }
            }
            if self.needs_ui {
                self.line("#[allow(unused_imports)]");
                self.line("use crow_web::view::CrowVNode;");
                self.line("#[allow(unused_imports)]");
                self.line("use crow_web::live::{CrowLiveView, CrowLiveState};");
            }
        }
        if self.needs_crow_db {
            self.line("#[allow(unused_imports)]");
            self.line("use crow_db::DbHandle;");
        }
        self.line("");

        // the records a fact holds are kept as JSON by the session, and a
        // handler's record arguments arrive as JSON
        self.serde_records = self.records_as_json();
        let result_serde = self.part.structs.iter().any(|s| self.serde_records.contains(&s.name) && s.fields.iter().any(|(_, t)| t.is_result()));
        if !self.serde_records.is_empty() {
            self.needs_serde_json = true;
        }
        // the one result type
        for l in [
            "#[allow(dead_code)]",
            if result_serde { "#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]" } else { "#[derive(Debug, Clone)]" },
            "struct CrowResult<T: Clone> {",
            "    ok: bool,",
            "    value: T,",
            "    reason: String,",
            "    kind: String,",
            "    trace: Vec<String>,",
            "    captured: f64,",
            "}",
            "impl<T: Clone + std::fmt::Display> std::fmt::Display for CrowResult<T> {",
            "    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {",
            "        if !self.ok { write!(f, \"error: {}\", self.reason) } else { write!(f, \"{}\", self.value) }",
            "    }",
            "}",
            "",
        ] {
            self.line(l);
        }

        if self.needs_grad {
            self.emit_crow_dual_struct();
        }
        if self.needs_serde_json {
            self.emit_json_display();
        }

        // a user module's top-level values are consts every function reads
        self.emit_module_consts();

        // structs
        let structs: Vec<StructDef> = self.part.structs.clone();
        for s in &structs {
            if !matches!(s.name.as_str(), "CrowFindResult" | "CrowShell" | "CrowElapsed") {
                self.emit_struct(s);
            }
        }

        if !self.data_source_vars.is_empty() {
            self.emit_json_to_frame();
        }

        // functions; the original of a specialized function is never
        // compiled — its clones are
        let specialized: BTreeSet<String> = self
            .fns
            .keys()
            .filter_map(|n| n.rsplit_once("__").filter(|(_, s)| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())).map(|(b, _)| b.to_string()))
            .collect();
        let names: Vec<String> = self.fns.keys().cloned().collect();
        let reached = self.reached_functions();
        for n in &names {
            if specialized.contains(n) {
                continue;
            }
            let f = self.fns.get(n).unwrap().clone();
            // a parameter of no known type would be text in Rust; and a pure
            // function nothing the program runs calls (a shared part's, in a
            // part's test app) is not compiled — its types came from no call,
            // or from a query nothing could describe, and could meet a kernel
            // that takes numbers. Nor is a helper nothing calls: it writes
            // facts for a handler no handler is (a part whose button moved to
            // another part), and it can never run
            let has_unknown = f.params.iter().any(|(_, t)| matches!(t, Ty::Unknown))
                || (!reached.contains(n) && (matches!(f.kind, FnKind::Pure | FnKind::Helper) || f.params.iter().any(|(_, t)| unknown_inside(t))));
            let is_callback = self.live_render_fns.contains(n)
                || self.live_handle_fns.contains(n)
                || self.reactive_render_fns.contains(n)
                || self.reactive_event_handlers.contains(n)
                || self.web_handlers.contains(n)
                || self.views.contains(n)
                || f.kind == FnKind::Handler;
            if has_unknown && !is_callback {
                continue;
            }
            self.emit_function(&f);
        }
        let derives: Vec<Derive> = self.part.derives.clone();
        for d in &derives {
            if !self.needs_ui || !self.has_page_routes {
                return Err(format!("derive {} needs reactive state — a `use ui` program with a page[] (derive-design.md)", d.name));
            }
            self.emit_derive(d);
        }
        if self.needs_grad {
            for g in self.grad_functions.clone() {
                if let Some(f) = self.fns.get(&g).cloned() {
                    self.emit_dual_function(&f);
                }
            }
        }
        if !self.reactive_event_handlers.is_empty() {
            self.emit_handle_dispatch();
        }
        if self.test_control && self.has_page_routes {
            self.emit_test_tables();
        }
        self.emit_main();
        self.emit_synthetic_types();
        // a result as the display kernel shows it inside a list: its value,
        // or its reason (written last: only a program that links the kernel
        // knows by now that it does)
        if self.needs_crow_runtime {
            for l in [
                "impl<T: Clone + crow_runtime::display::CrowDisplay> crow_runtime::display::CrowDisplay for CrowResult<T> {",
                "    fn crow_repr(&self) -> String { if !self.ok { format!(\"error: {}\", self.reason) } else { self.value.crow_repr() } }",
                "    fn crow_fmt(&self) -> String { if !self.ok { format!(\"error: {}\", self.reason) } else { self.value.crow_fmt() } }",
                "}",
            ] {
                self.line(l);
            }
        }

        if let Some(e) = self.error.take() {
            return Err(e);
        }
        Ok(std::mem::take(&mut self.out))
    }

    fn any_type(&self, pred: &dyn std::ops::Fn(&Ty) -> bool) -> bool {
        fn has(t: &Ty, pred: &dyn std::ops::Fn(&Ty) -> bool) -> bool {
            pred(t)
                || match t {
                    Ty::List(i) | Ty::Map(i) | Ty::Result(i) | Ty::Tensor(i) | Ty::Column(i) => has(i, pred),
                    Ty::Fn(ps, r) => ps.iter().any(|p| has(p, pred)) || has(r, pred),
                    _ => false,
                }
        }
        let mut found = self.fns.values().any(|f| f.params.iter().any(|(_, t)| has(t, pred)) || has(&f.ret, pred));
        found |= self.part.facts.iter().any(|f| has(&f.ty, pred));
        for b in self.all_blocks() {
            visit_stmts(&b, &mut |s| {
                if let Stmt::Let { ty, .. } = s {
                    if has(ty, pred) {
                        found = true;
                    }
                }
            });
        }
        found
    }

    fn emit_json_display(&mut self) {
        for l in [
            "#[allow(dead_code)]",
            "fn crow_json_display(v: &serde_json::Value) -> String {",
            "    match v {",
            "        serde_json::Value::String(s) => s.clone(),",
            "        serde_json::Value::Null => \"null\".to_string(),",
            "        serde_json::Value::Array(items) => format!(\"[{}]\", items.iter().map(crow_json_repr).collect::<Vec<_>>().join(\", \")),",
            "        serde_json::Value::Object(m) => format!(\"[{}]\", m.iter().map(|(k, v)| format!(\"{} = {}\", k, crow_json_repr(v))).collect::<Vec<_>>().join(\", \")),",
            "        other => other.to_string(),",
            "    }",
            "}",
            "#[allow(dead_code)]",
            "fn crow_json_repr(v: &serde_json::Value) -> String {",
            "    match v {",
            "        serde_json::Value::String(s) => format!(\"'{}'\", s),",
            "        other => crow_json_display(other),",
            "    }",
            "}",
            "",
        ] {
            self.line(l);
        }
    }

    /// A user module's top-level values (`mymath_pi`), printed as consts.
    fn emit_module_consts(&mut self) {
        if self.needs_ui {
            return;
        }
        let main = self.part.main.clone();
        let mut any = false;
        for s in &main {
            if let Stmt::Let { var, expr, .. } = s {
                let prefixed = self.modules.iter().any(|m| var.starts_with(&format!("{}_", m)));
                if !prefixed {
                    continue;
                }
                let text = self.expr(expr);
                let ty = self.tt(expr);
                self.line("#[allow(non_upper_case_globals)]");
                if ty.to_rust_type() == "String" {
                    let v = text.trim_end_matches(".to_string()");
                    self.line(&format!("const {}: &str = {};", var, v));
                } else {
                    self.line(&format!("const {}: {} = {};", var, ty.to_rust_type(), text));
                }
                self.module_consts.insert(var.clone());
                any = true;
            }
        }
        if any {
            self.line("");
        }
    }

    /// The records a fact holds, and the records their fields hold.
    fn records_as_json(&self) -> BTreeSet<String> {
        fn records(t: &Ty, out: &mut Vec<String>) {
            match t {
                Ty::Record(n) | Ty::OptRecord(n) => out.push(n.clone()),
                Ty::List(i) | Ty::Map(i) | Ty::Result(i) => records(i, out),
                _ => {}
            }
        }
        let mut todo: Vec<String> = Vec::new();
        for f in &self.part.facts {
            records(&f.ty, &mut todo);
        }
        for h in &self.reactive_event_handlers {
            if let Some(f) = self.fns.get(h) {
                for (_, t) in &f.params {
                    records(t, &mut todo);
                }
            }
        }

        let mut seen = BTreeSet::new();
        while let Some(n) = todo.pop() {
            if seen.insert(n.clone()) {
                if let Some(s) = self.part.structs.iter().find(|s| s.name == n) {
                    for (_, t) in &s.fields {
                        records(t, &mut todo);
                    }
                }
            }
        }
        seen
    }

    pub(crate) fn emit_struct(&mut self, s: &StructDef) {
        let all_copy = s.fields.iter().all(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool));
        let serde = if self.serde_records.contains(&s.name) { ", serde::Serialize, serde::Deserialize" } else { "" };
        self.line("#[allow(non_camel_case_types)]");
        self.line(&format!("#[derive(Debug, Clone{}{})]", if all_copy { ", Copy" } else { "" }, serde));
        self.line(&format!("struct {} {{", s.name));
        self.indent += 1;
        for (f, t) in &s.fields {
            let rust = crow_type(t).to_rust_type();
            // a tensor, a frame or a column field is the runtime's type
            if rust.contains("crow_runtime::") {
                self.needs_crow_runtime = true;
            }
            self.line(&format!("{}: {},", Self::rust_field(f), rust));
        }
        self.indent -= 1;
        self.line("}");
        // the value an error of this struct's type carries: each field's default
        let defaults: Vec<String> = s.fields.iter().map(|(f, t)| format!("{}: {}", Self::rust_field(f), crow_type(t).default_rust_value())).collect();
        self.line("#[allow(dead_code)]");
        self.line(&format!("impl {} {{ fn crow_default() -> Self {{ {} {{ {} }} }} }}", s.name, s.name, defaults.join(", ")));
        self.line("");
        if self.struct_kernel_displayable(&s.name) {
            self.needs_crow_runtime = true;
            let mut sorted: Vec<&(String, Ty)> = s.fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let parts: Vec<String> = sorted.iter().map(|(f, _)| format!("{} = {{}}", f)).collect();
            let vals: Vec<String> = sorted.iter().map(|(f, _)| format!("self.{}.crow_repr()", Self::rust_field(f))).collect();
            self.line(&format!("impl crow_runtime::display::CrowDisplay for {} {{", s.name));
            self.line("    fn crow_repr(&self) -> String {");
            self.line(&format!(
                "        {{ use crow_runtime::display::CrowDisplay as _; format!(\"{}[{}]\", {}) }}",
                s.name,
                parts.join(", "),
                vals.join(", ")
            ));
            self.line("    }");
            self.line("}");
            self.line("");
        }
    }

    /// The records whose every field has a JSON form (numbers, text, lists
    /// and maps of them, results of them, tensors, and such records).
    fn json_records(&self) -> BTreeSet<String> {
        fn jsonable(t: &CrowType, ok: &BTreeSet<String>) -> bool {
            match t {
                CrowType::Int | CrowType::Float | CrowType::Bool | CrowType::Str | CrowType::Markup | CrowType::Json | CrowType::NumArray(_) => true,
                CrowType::Array(i) | CrowType::Map(i) | CrowType::Result(i) => jsonable(i, ok),
                CrowType::Struct(n) => ok.contains(n),
                _ => false,
            }
        }
        // a record whose fields are records joins once they have
        let mut ok: BTreeSet<String> = BTreeSet::new();
        loop {
            let before = ok.len();
            for (name, def) in &self.structs {
                if matches!(name.as_str(), "CrowFindResult" | "CrowShell" | "CrowElapsed") || ok.contains(name) {
                    continue;
                }
                if def.fields.iter().all(|(_, t)| jsonable(&crow_type(t), &ok)) {
                    ok.insert(name.clone());
                }
            }
            if ok.len() == before {
                return ok;
            }
        }
    }

    pub(crate) fn struct_kernel_displayable(&self, name: &str) -> bool {
        if matches!(name, "CrowShell" | "CrowFindResult") {
            return true;
        }
        if name == "CrowElapsed" {
            return false;
        }
        self.structs.get(name).map_or(false, |s| s.fields.iter().all(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)))
    }

    pub(crate) fn kernel_displayable(&self, ty: &CrowType) -> bool {
        match ty {
            CrowType::Array(inner) => match inner.as_ref() {
                CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool => true,
                // an item that can fail shows its value or its reason
                // (`CrowResult`'s CrowDisplay, written with the program)
                CrowType::Result(item) => matches!(item.as_ref(), CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool),
                CrowType::NumArray(_) => true,
                nested @ CrowType::Array(_) => self.kernel_displayable(nested),
                _ => false,
            },
            CrowType::Map(inner) => match inner.as_ref() {
                CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool | CrowType::Unknown => true,
                nested @ (CrowType::Array(_) | CrowType::Map(_)) => self.kernel_displayable(nested),
                _ => false,
            },
            CrowType::Struct(n) => self.struct_kernel_displayable(n),
            _ => false,
        }
    }

    pub(crate) fn json_encodable(&self, ty: &CrowType) -> bool {
        match ty {
            CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool | CrowType::NumArray(_) | CrowType::Json => true,
            CrowType::Array(inner) | CrowType::Map(inner) => self.json_encodable(inner),
            _ => false,
        }
    }

    fn emit_json_to_frame(&mut self) {
        self.needs_serde_json = true;
        self.needs_crow_runtime = true;
        for l in [
            "fn __json_to_frame(v: &serde_json::Value) -> crow_runtime::frame::CrowFrame {",
            "    use crow_runtime::frame::{CrowFrame, Column};",
            "    let arr = match v.as_array() { Some(a) => a, None => return CrowFrame::new(vec![], vec![], vec![]) };",
            "    if arr.is_empty() { return CrowFrame::new(vec![], vec![], vec![]); }",
            "    let cols: Vec<String> = arr[0].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();",
            "    let mut data: Vec<Column> = Vec::new();",
            "    let mut validity: Vec<Vec<bool>> = Vec::new();",
            "    for col_name in &cols {",
            "        let vals: Vec<&serde_json::Value> = arr.iter().map(|row| row.get(col_name.as_str()).unwrap_or(&serde_json::Value::Null)).collect();",
            "        let valid: Vec<bool> = vals.iter().map(|v| !v.is_null()).collect();",
            "        let all_int = vals.iter().all(|v| v.is_i64() || v.is_null());",
            "        let all_float = vals.iter().all(|v| v.is_f64() || v.is_i64() || v.is_null());",
            "        let col = if all_int && vals.iter().any(|v| v.is_i64()) {",
            "            Column::Int(vals.iter().map(|v| v.as_i64().unwrap_or(0)).collect())",
            "        } else if all_float && vals.iter().any(|v| v.is_f64() || v.is_i64()) {",
            "            Column::Float(vals.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())",
            "        } else {",
            "            Column::Str(vals.iter().map(|v| v.as_str().unwrap_or(\"\").to_string()).collect())",
            "        };",
            "        data.push(col);",
            "        validity.push(valid);",
            "    }",
            "    CrowFrame::new(cols, data, validity)",
            "}",
            "",
        ] {
            self.line(l);
        }
    }

    /// For the test harness (`crow test --compiled`): each view without
    /// parameters by name, and each derivation by name as JSON — a failed
    /// one as its reason, a value with no JSON form as none.
    fn emit_test_tables(&mut self) {
        self.needs_serde_json = true;
        self.line("#[allow(unused_variables, dead_code)]");
        self.line("fn __crow_view(state: &CrowLiveState, name: &str) -> Option<CrowVNode> {");
        self.indent += 1;
        self.line("match name {");
        self.indent += 1;
        let views: Vec<String> = self.views.iter().cloned().collect();
        for v in views {
            if self.fns.get(&v).map_or(false, |f| f.params.is_empty()) && self.reactive_render_fns.contains(&v) {
                self.line(&format!("\"{}\" => Some({}(state)),", v, Self::rust_fn_name(&v)));
            }
        }
        self.line("_ => None,");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        self.line("");
        // a record reads as the map of its fields, each as JSON — when every
        // field has a JSON form (a detector's result read by a test)
        let records = self.json_records();
        for name in &records {
            let def = self.structs.get(name).cloned().expect("a record the program defines");
            let inserts: Vec<String> = def
                .fields
                .iter()
                .map(|(f, _)| format!("m.insert({}.to_string(), crow_runtime::json::ToJson::to_json(&self.{}));", crate::emit_rs::exprs::rust_str_lit(f), Self::rust_field(f)))
                .collect();
            self.line(&format!(
                "impl crow_runtime::json::ToJson for {} {{ fn to_json(&self) -> serde_json::Value {{ let mut m = serde_json::Map::new(); {} serde_json::Value::Object(m) }} }}",
                name,
                inserts.join(" ")
            ));
        }
        if !records.is_empty() {
            self.needs_crow_runtime = true;
            self.line("impl<T: Clone + crow_runtime::json::ToJson> crow_runtime::json::ToJson for CrowResult<T> { fn to_json(&self) -> serde_json::Value { if self.ok { self.value.to_json() } else { serde_json::json!({ \"ok\": false, \"reason\": self.reason, \"kind\": self.kind }) } } }");
        }
        self.line("#[allow(unused_variables, dead_code)]");
        self.line("fn __crow_derived(state: &CrowLiveState, name: &str) -> Option<serde_json::Value> {");
        self.indent += 1;
        self.line("match name {");
        self.indent += 1;
        let as_json = |t: &CrowType, v: &str| -> Option<String> {
            match t {
                CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Markup | CrowType::Bool | CrowType::Json | CrowType::Frame => {
                    Some(format!("serde_json::to_value(&{}).unwrap_or(serde_json::Value::Null)", v))
                }
                CrowType::NumArray(_) => Some(format!("crow_runtime::json::ToJson::to_json(&{})", v)),
                CrowType::Array(i) | CrowType::Map(i) if matches!(i.as_ref(), CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Markup | CrowType::Bool | CrowType::Json) => {
                    Some(format!("serde_json::to_value(&{}).unwrap_or(serde_json::Value::Null)", v))
                }
                CrowType::Struct(n) if records.contains(n) => Some(format!("crow_runtime::json::ToJson::to_json(&{})", v)),
                _ => None,
            }
        };
        let derives: Vec<(String, Ty)> = self.part.derives.iter().map(|d| (d.name.clone(), d.ty.clone())).collect();
        for (d, ty) in derives {
            let rt = crow_type(&ty);
            let read = format!("__derive_{}(state)", d);
            match &rt {
                CrowType::Result(inner) => {
                    if let Some(value) = as_json(inner, "_r.value") {
                        self.line(&format!(
                            "\"{}\" => {{ let _r = {}; Some(if _r.ok {{ serde_json::json!({{ \"ok\": true, \"value\": {} }}) }} else {{ serde_json::json!({{ \"ok\": false, \"reason\": _r.reason, \"kind\": _r.kind }}) }}) }},",
                            d, read, value
                        ));
                    }
                }
                other => {
                    if let Some(value) = as_json(other, &read) {
                        if matches!(other, CrowType::NumArray(_)) {
                            self.needs_crow_runtime = true;
                        }
                        self.line(&format!("\"{}\" => Some(serde_json::json!({{ \"ok\": true, \"value\": {} }})),", d, value));
                    }
                }
            }
        }
        self.line("_ => None,");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    /// The live loop's one entry for events: each handler by name, its
    /// frozen arguments read as its parameters' types.
    fn emit_handle_dispatch(&mut self) {
        self.needs_serde_json = true;
        self.line("#[allow(unused_variables)]");
        self.line("fn __crow_handle(state: &mut CrowLiveState, event: &str, _value: Option<&str>) {");
        self.indent += 1;
        self.line("match event {");
        self.indent += 1;
        for h in self.reactive_event_handlers.clone() {
            let rust = Self::rust_fn_name(&h);
            let params: Vec<(String, Ty)> = self.fns.get(&h).map(|f| f.params.clone()).unwrap_or_default();
            // a list, a tensor, a frame or a record arrives as JSON its own
            // serde reads (a test clicks a handler with one as it clicks any)
            let by_serde = |t: &Ty| matches!(t, Ty::List(_) | Ty::Map(_) | Ty::Tensor(_) | Ty::Frame(_) | Ty::Record(_) | Ty::OptRecord(_));
            let callable = params.iter().all(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Unknown) || by_serde(t));
            if params.is_empty() {
                self.line(&format!("\"{}\" => {}(state),", h, rust));
            } else if !callable {
                // a helper taking a handle or a function: only other handlers call it
            } else {
                let mut decoded: Vec<(String, String)> = Vec::new();
                let extracts: Vec<String> = params
                    .iter()
                    .enumerate()
                    .map(|(i, (_, t))| match t {
                        Ty::Int => format!("_a[{i}].as_i64().or_else(|| _a[{i}].as_f64().map(|x| x as i64)).or_else(|| _a[{i}].as_str().and_then(|s| s.trim().parse::<i64>().ok())).unwrap_or(0)", i = i),
                        Ty::Float => format!("_a[{i}].as_f64().or_else(|| _a[{i}].as_str().and_then(|s| s.trim().parse::<f64>().ok())).unwrap_or(0.0)", i = i),
                        Ty::Bool => format!("_a[{i}].as_bool().or_else(|| _a[{i}].as_str().map(|s| s == \"true\")).unwrap_or(false)", i = i),
                        t if by_serde(t) => {
                            decoded.push((format!("_p{}", i), format!("serde_json::from_value::<{}>(_a[{}].clone()).ok()", crow_type(t).to_rust_type(), i)));
                            format!("_p{}", i)
                        }
                        _ => format!("_a[{i}].as_str().map(|s| s.to_string()).unwrap_or_else(|| _a[{i}].to_string())", i = i),
                    })
                    .collect();
                let call = format!("{}(state, {});", rust, extracts.join(", "));
                let call = if decoded.is_empty() {
                    call
                } else {
                    let names: Vec<String> = decoded.iter().map(|(n, _)| format!("Some({})", n)).collect();
                    let values: Vec<String> = decoded.iter().map(|(_, v)| v.clone()).collect();
                    format!("if let ({},) = ({},) {{ {} }}", names.join(", "), values.join(", "), call)
                };
                self.line(&format!(
                    "\"{}\" => {{ if let Some(_v) = _value {{ if let Ok(_a) = serde_json::from_str::<Vec<serde_json::Value>>(_v) {{ if _a.len() == {} {{ {} }} }} }} }},",
                    h,
                    params.len(),
                    call
                ));
            }
        }
        self.line("_ => {}");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    fn emit_main(&mut self) {
        // no-app route[]/serve[] calls (plain `use web`, no page) still need
        // the implicit app
        if self.needs_crow_web && !self.has_page_routes {
            let main = self.part.main.clone();
            for s in &main {
                if let Stmt::Do(e) = s {
                    let name = call_name(e);
                    let args = call_args(e);
                    let noapp_route = name.as_deref() == Some("route")
                        && (args.len() == 2 || (args.len() == 3 && matches!(self.tt(&args[0]), CrowType::Str)));
                    let noapp_serve = name.as_deref() == Some("serve") && (args.len() == 1 || args.len() == 9);
                    if noapp_route || noapp_serve {
                        self.has_noapp_web = true;
                    }
                }
            }
        }
        if !self.static_handle_vars.is_empty() {
            for n in self.static_handle_vars.clone() {
                let mailer = self.part.facts.iter().any(|f| f.name == n && (matches!(f.ty, Ty::Handle(ref k) if k == "mailer") || matches!(self.tt(&f.default), CrowType::Mailer)));
                let ty = if mailer { "crow_web::mail::CrowMailer" } else { "crow_db::DbHandle" };
                self.line("#[allow(non_upper_case_globals)]");
                self.line(&format!("static __crow_handle_{}: std::sync::OnceLock<{}> = std::sync::OnceLock::new();", n, ty));
            }
            self.line("");
        }
        if self.needs_crow_web {
            // the runtime's threads compute a page's derivations and run its
            // handlers: their stacks are sized for that work, not tokio's
            // 2 MiB (a debug build of finch's book walk overflowed it)
            self.line("fn main() {");
            self.line("    tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 * 1024 * 1024).build().expect(\"the async runtime\").block_on(__crow_main());");
            self.line("}");
            self.line("");
            self.line("async fn __crow_main() {");
        } else {
            self.line("fn main() {");
        }
        self.indent += 1;
        self.emit_panic_hook();
        self.cur_fn = String::new();
        self.cur_ret = None;
        self.returns_unit = true;
        self.scopes = vec![HashMap::new()];
        self.mutated = Self::mutated_in(&self.part.main.clone());
        // the part's resources
        if self.part.gpu.is_some() {
            self.needs_crow_runtime = true;
            self.line("crow_runtime::gpu::set_gpu_enabled(true);");
        }
        if let Some(n) = self.part.nodes {
            self.needs_crow_runtime = true;
            self.needs_distributed = true;
            self.line(&format!("crow_runtime::distributed::init({});", n));
        }
        if self.has_noapp_web && !self.has_page_routes {
            self.line("let mut __crow_app = CrowApp::new();");
        }
        let main = self.part.main.clone();
        if self.has_page_routes {
            self.line("let mut __crow_app = CrowApp::new();");
            self.line("let mut __crow_state = CrowLiveState::new();");
            if !self.reactive_vars.is_empty() {
                self.needs_serde_json = true;
                for (v, (_, d)) in self.reactive_vars.clone() {
                    self.line(&format!("__crow_state.set(\"{}\", serde_json::json!({}));", v, d));
                }
            }
            // connections and data sources: set once, before anything reads them
            let facts = self.part.facts.clone();
            for fact in &facts {
                if self.static_handle_vars.contains(&fact.name) {
                    let v = self.expr(&fact.default);
                    self.line(&format!("let _ = __crow_handle_{}.set({});", fact.name, v));
                } else if self.data_source_vars.contains(&fact.name) {
                    let args = call_args(&fact.default).to_vec();
                    if call_name(&fact.default).as_deref() == Some("fetch") {
                        let url = self.expr(&args[0]);
                        self.line(&format!("__crow_state.register_fetch(\"{}\", &{});", fact.name, url));
                    } else {
                        let url = self.expr(&args[0]);
                        let secs = self.expr(&args[1]);
                        self.line(&format!("__crow_state.register_poll(\"{}\", &{}, {} as u64);", fact.name, url, secs));
                    }
                }
            }
            // serve comes last, after every page is registered; a fact's
            // later top-level assignment set its default above
            let (serve, other): (Vec<Stmt>, Vec<Stmt>) = main.into_iter().partition(|s| matches!(s, Stmt::Do(e) if call_name(e).as_deref() == Some("serve")));
            let other: Vec<Stmt> = other.into_iter().filter(|s| !matches!(s, Stmt::Store { .. })).collect();
            self.emit_block(&other);
            if self.test_control {
                // the harness's routes beside the pages, on the port the
                // test gave (CROW_PORT)
                let handle = if self.reactive_event_handlers.is_empty() { "|_: &mut CrowLiveState, _: &str, _: Option<&str>| {}" } else { "__crow_handle" };
                self.line(&format!("__crow_app = __crow_app.merge(crow_web::live::test_control::router(__crow_state.clone(), __crow_view, {}, __crow_derived));", handle));
                if serve.is_empty() {
                    self.line("__crow_app.serve(0).await.unwrap();");
                }
            }
            self.emit_block(&serve);
        } else {
            // facts outside a page program are plain values, set first as
            // the interpreter sets them
            let facts = self.part.facts.clone();
            for fact in &facts {
                let v = self.owned_expr(&fact.default);
                let ty = crow_type(&fact.ty).text();
                let vty = self.tt(&fact.default);
                let (ty, v) = if matches!(ty, CrowType::Float) && matches!(vty, CrowType::Int) { (ty, format!("({}) as f64", v)) } else if matches!(ty, CrowType::Unknown) { (vty, v) } else { (ty, v) };
                let kw = if self.mutated.contains(&fact.name) { "let mut" } else { "let" };
                if format!("{:?}", ty).contains("Unknown") {
                    self.line(&format!("{} {} = {};", kw, fact.name, v));
                } else {
                    self.line(&format!("{} {}: {} = {};", kw, fact.name, ty.to_rust_type(), v));
                }
                self.declare(&fact.name, ty);
            }
            let main: Vec<Stmt> = main.into_iter().filter(|s| !matches!(s, Stmt::Let { var, .. } if self.module_consts.contains(var))).collect();
            self.emit_block(&main);
        }
        if self.needs_distributed {
            self.line("crow_runtime::distributed::finalize();");
        }
        if self.needs_later {
            self.line("crow_runtime::later::wait_all();");
        }
        self.indent -= 1;
        self.line("}");
    }

    /// The locals a block rebinds or changes in place: they are `mut`.
    pub(crate) fn mutated_in(block: &[Stmt]) -> HashSet<String> {
        let mut out = HashSet::new();
        visit_stmts(block, &mut |s| match s {
            Stmt::Set { var, .. } | Stmt::SetPath { var, .. } => {
                out.insert(var.clone());
            }
            Stmt::Store { fact, .. } => {
                out.insert(fact.clone());
            }
            _ => {}
        });
        out
    }

    fn emit_synthetic_types(&mut self) {
        if self.needs_time_result {
            for l in [
                "",
                "#[allow(dead_code)]",
                "fn __crow_time_result(r: Result<f64, String>) -> CrowResult<f64> {",
                "    match r {",
                "        Ok(t) => CrowResult { ok: true, value: t, reason: String::new(), kind: String::new(), trace: Vec::new(), captured: 0.0 },",
                "        Err(e) => CrowResult { ok: false, value: 0.0, reason: e, kind: \"time\".to_string(), trace: Vec::new(), captured: 0.0 },",
                "    }",
                "}",
            ] {
                self.line(l);
            }
        }
        if self.needs_find_struct {
            self.needs_crow_runtime = true;
            self.line("");
            self.line("#[derive(Clone, Debug)]");
            self.line("struct CrowFindResult { groups: Vec<String> }");
            self.line("impl crow_runtime::display::CrowDisplay for CrowFindResult {");
            self.line("    fn crow_repr(&self) -> String { use crow_runtime::display::CrowDisplay as _; format!(\"[groups = {}]\", self.groups.crow_repr()) }");
            self.line("}");
        }
        if self.needs_shell_struct {
            self.needs_crow_runtime = true;
            self.line("");
            self.line("#[derive(Clone, Debug)]");
            self.line("struct CrowShell { status: i64, stdout: String, stderr: String }");
            self.line("impl crow_runtime::display::CrowDisplay for CrowShell {");
            self.line("    fn crow_repr(&self) -> String { use crow_runtime::display::CrowDisplay as _; format!(\"[status = {}, stderr = {}, stdout = {}]\", self.status.crow_repr(), self.stderr.crow_repr(), self.stdout.crow_repr()) }");
            self.line("}");
        }
        if self.needs_elapsed_struct {
            self.line("");
            self.line("#[derive(Clone, Debug)]");
            self.line("struct CrowElapsed { seconds: f64, minutes: f64, hours: f64, days: f64 }");
        }
    }

    fn emit_panic_hook(&mut self) {
        for l in [
            "std::panic::set_hook(Box::new(|info| {",
            "    let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {",
            "        s.to_string()",
            "    } else if let Some(s) = info.payload().downcast_ref::<String>() {",
            "        s.clone()",
            "    } else {",
            "        \"unknown error\".to_string()",
            "    };",
            "    let crow_msg = if msg.contains(\"index out of bounds\") {",
            "        let parts: Vec<&str> = msg.split(\"the len is \").collect();",
            "        if parts.len() > 1 {",
            "            let rest: Vec<&str> = parts[1].split(\" but the index is \").collect();",
            "            if rest.len() > 1 {",
            "                format!(\"index {} is out of range (array has {} items)\", rest[1], rest[0])",
            "            } else { format!(\"index out of range\") }",
            "        } else { format!(\"index out of range\") }",
            "    } else if msg.contains(\"divide by zero\") || msg.contains(\"division by zero\") {",
            "        format!(\"cannot divide by zero\")",
            "    } else if msg.contains(\"attempt to divide by zero\") {",
            "        format!(\"cannot divide by zero\")",
            "    } else if msg.contains(\"assertion failed\") {",
            "        if msg.starts_with(\"assertion failed: \") {",
            "            msg.clone()",
            "        } else { format!(\"assertion failed\") }",
            "    } else if msg.contains(\"stack overflow\") {",
            "        format!(\"stack overflow (possible infinite recursion)\")",
            "    } else if msg.contains(\"arithmetic operation overflowed\") || msg.contains(\"attempt to \") {",
            "        let crow = msg.replace(\"attempt to \", \"\");",
            "        format!(\"arithmetic error: {}\", crow)",
            "    } else {",
            "        format!(\"runtime error: {}\", msg)",
            "    };",
            "    eprintln!(\"error: {}\", crow_msg);",
            "    // A panic on the main thread is fatal; a contained panic in a",
            "    // worker/server task is reported but must not kill the server.",
            "    if std::thread::current().name() == Some(\"main\") {",
            "        std::process::exit(1);",
            "    }",
            "}));",
        ] {
            self.line(l);
        }
    }

    fn emit_crow_dual_struct(&mut self) {
        self.line("#[derive(Debug, Clone, Copy)]");
        self.line("struct CrowDual { real: f64, dual: f64 }");
        self.line("#[allow(dead_code)]");
        self.line("impl CrowDual {");
        self.indent += 1;
        self.line("fn new(real: f64, dual: f64) -> Self { CrowDual { real, dual } }");
        self.line("fn abs(self) -> Self { CrowDual { real: self.real.abs(), dual: self.dual * self.real.signum() } }");
        self.line("fn sqrt(self) -> Self { let s = self.real.sqrt(); CrowDual { real: s, dual: self.dual / (2.0 * s) } }");
        self.line("fn powf(self, n: CrowDual) -> Self { let r = self.real.powf(n.real); CrowDual { real: r, dual: r * (n.dual * self.real.ln() + n.real * self.dual / self.real) } }");
        self.line("fn powi_const(self, n: i64) -> Self { CrowDual { real: self.real.powi(n as i32), dual: (n as f64) * self.real.powi(n as i32 - 1) * self.dual } }");
        self.line("fn floor(self) -> CrowDual { CrowDual { real: self.real.floor(), dual: 0.0 } }");
        self.line("fn ceil(self) -> CrowDual { CrowDual { real: self.real.ceil(), dual: 0.0 } }");
        self.line("fn round(self) -> CrowDual { CrowDual { real: self.real.round(), dual: 0.0 } }");
        self.indent -= 1;
        self.line("}");
        self.line("impl std::fmt::Display for CrowDual {");
        self.line("    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, \"{}\", self.real) }");
        self.line("}");
        for (tr, m, body) in [
            ("Add", "add", "CrowDual { real: self.real + rhs.real, dual: self.dual + rhs.dual }"),
            ("Sub", "sub", "CrowDual { real: self.real - rhs.real, dual: self.dual - rhs.dual }"),
            ("Mul", "mul", "CrowDual { real: self.real * rhs.real, dual: self.dual * rhs.real + self.real * rhs.dual }"),
            ("Div", "div", "CrowDual { real: self.real / rhs.real, dual: (self.dual * rhs.real - self.real * rhs.dual) / (rhs.real * rhs.real) }"),
        ] {
            self.line(&format!("impl std::ops::{} for CrowDual {{ type Output = CrowDual; fn {}(self, rhs: CrowDual) -> CrowDual {{ {} }} }}", tr, m, body));
        }
        self.line("impl std::ops::Neg for CrowDual { type Output = CrowDual; fn neg(self) -> CrowDual { CrowDual { real: -self.real, dual: -self.dual } } }");
        for (tr, m, df, fd) in [
            ("Add", "add", "CrowDual { real: self.real + rhs, dual: self.dual }", "CrowDual { real: self + rhs.real, dual: rhs.dual }"),
            ("Sub", "sub", "CrowDual { real: self.real - rhs, dual: self.dual }", "CrowDual { real: self - rhs.real, dual: -rhs.dual }"),
            ("Mul", "mul", "CrowDual { real: self.real * rhs, dual: self.dual * rhs }", "CrowDual { real: self * rhs.real, dual: self * rhs.dual }"),
            ("Div", "div", "CrowDual { real: self.real / rhs, dual: self.dual / rhs }", "CrowDual { real: self / rhs.real, dual: -self * rhs.dual / (rhs.real * rhs.real) }"),
        ] {
            self.line(&format!("impl std::ops::{}<f64> for CrowDual {{ type Output = CrowDual; fn {}(self, rhs: f64) -> CrowDual {{ {} }} }}", tr, m, df));
            self.line(&format!("impl std::ops::{}<CrowDual> for f64 {{ type Output = CrowDual; fn {}(self, rhs: CrowDual) -> CrowDual {{ {} }} }}", tr, m, fd));
        }
        for (tr, m, di, id) in [
            ("Add", "add", "CrowDual { real: self.real + rhs as f64, dual: self.dual }", "CrowDual { real: self as f64 + rhs.real, dual: rhs.dual }"),
            ("Sub", "sub", "CrowDual { real: self.real - rhs as f64, dual: self.dual }", "CrowDual { real: self as f64 - rhs.real, dual: -rhs.dual }"),
            ("Mul", "mul", "CrowDual { real: self.real * rhs as f64, dual: self.dual * rhs as f64 }", "CrowDual { real: self as f64 * rhs.real, dual: self as f64 * rhs.dual }"),
            ("Div", "div", "CrowDual { real: self.real / rhs as f64, dual: self.dual / rhs as f64 }", "CrowDual { real: self as f64 / rhs.real, dual: -(self as f64) * rhs.dual / (rhs.real * rhs.real) }"),
        ] {
            self.line(&format!("impl std::ops::{}<i64> for CrowDual {{ type Output = CrowDual; fn {}(self, rhs: i64) -> CrowDual {{ {} }} }}", tr, m, di));
            self.line(&format!("impl std::ops::{}<CrowDual> for i64 {{ type Output = CrowDual; fn {}(self, rhs: CrowDual) -> CrowDual {{ {} }} }}", tr, m, id));
        }
        self.line("impl From<f64> for CrowDual { fn from(v: f64) -> Self { CrowDual { real: v, dual: 0.0 } } }");
        self.line("impl From<i64> for CrowDual { fn from(v: i64) -> Self { CrowDual { real: v as f64, dual: 0.0 } } }");
        self.line("impl PartialEq for CrowDual { fn eq(&self, other: &Self) -> bool { self.real == other.real } }");
        self.line("impl PartialOrd for CrowDual { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { self.real.partial_cmp(&other.real) } }");
        self.line("");
    }
}

/// Whether a type holds one not known: text in Rust, for a list's or a
/// map's items as for a value.
fn unknown_inside(t: &Ty) -> bool {
    match t {
        Ty::Unknown => true,
        Ty::List(i) | Ty::Map(i) | Ty::Result(i) => unknown_inside(i),
        _ => false,
    }
}
