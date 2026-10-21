//! The crate as a whole: what the program needs, the header, the types,
//! every function, the event dispatch and `main`.

use super::*;

// ── walking the IR (the visitors live with the IR) ──────────────────

pub(crate) use crate::ir::{call_args, call_name, visit_block, visit_expr, visit_stmts};

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
            Expr::Call { target: Target::Runtime(_) | Target::Row(_), args, .. } if crate::ir::call_name(cur).as_deref() == Some("set") && args.len() == 3 => {
                match Emitter::raw(&args[1]) {
                    Expr::Const(Const::Str(k), _) => out.push((k.clone(), Emitter::raw(&args[2]).clone())),
                    _ => return None,
                }
                cur = Emitter::raw(&args[0]);
            }
            Expr::Call { target: Target::Runtime(_) | Target::Row(_), args, .. } if crate::ir::call_name(cur).as_deref() == Some("map") && args.is_empty() => {
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

    /// A module whose functions are rows, loaded into the registry (the
    /// manifest's `rows`: a module of own forms is answered by the
    /// compiler's own arms, not the registry's).
    fn load_module(&mut self, name: &str) {
        if self.registry.is_loaded(name) {
            return;
        }
        if let Some(m) = crate::modules::by_path(name).filter(|m| m.rows).and_then(|m| m.module) {
            self.registry.load(m());
        }
    }

    /// `use` paths: the crates and features they bring, the modules the
    /// registry answers for — each as the manifest declares it
    /// (crate::modules::MODULES).
    fn read_uses(&mut self) {
        self.registry = crate::modules::ModuleRegistry::new();
        let uses: Vec<String> = self.part.uses.clone();
        for u in uses {
            let path = u.split(" as ").next().unwrap_or(&u).trim().to_string();
            match crate::modules::by_path(&path) {
                Some(m) => {
                    for (krate, features) in m.needs {
                        let set = match *krate {
                            "crow_runtime" => {
                                self.needs_crow_runtime = true;
                                &mut self.runtime_features
                            }
                            "crow_web" => {
                                self.needs_crow_web = true;
                                &mut self.web_features
                            }
                            _ => {
                                self.needs_crow_db = true;
                                &mut self.db_features
                            }
                        };
                        set.extend(features.iter().map(|f| f.to_string()));
                    }
                    if m.ui {
                        self.needs_ui = true;
                    }
                    // what the import brings first (its parent, its drivers),
                    // then the module itself
                    for brought in crate::modules::brought_by(&path) {
                        self.load_module(brought);
                    }
                    self.load_module(&path);
                    if let Some(parent) = crate::modules::parent_of(&path) {
                        self.modules.insert(parent.to_string());
                    }
                }
                // (a module of web's with no entry is a feature of crow_web's)
                None if path.starts_with("web.") => {
                    self.needs_crow_web = true;
                    let sub = &path[4..];
                    let feature = if sub == "static" { "static_files" } else { sub };
                    self.web_features.insert(feature.to_string());
                    self.modules.insert("web".to_string());
                }
                None => {}
            }
            self.modules.insert(path);
        }
        crate::modules::load_crow_scopes(&mut self.registry, self.part.fns.iter().map(|f| f.name.as_str()));
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

        // gradient targets over numbers: their parameters and value are
        // floats; over tensors (the IR's `traced`), a copy over traced values
        self.traced_functions = self.part.traced.clone();
        let blocks = self.all_blocks();
        let mut grads = BTreeSet::new();
        for b in &blocks {
            for f in Self::fn_args_of_calls(b, "grad", &|args| args.iter().filter_map(fn_named).map(|s| s.to_string()).collect()) {
                if !self.traced_functions.contains(&f) {
                    grads.insert(f);
                }
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
            // the helpers they call, of numbers and answering one, have
            // dual versions too: a helper is differentiated through its own
            // run, as the interpreter runs it on dual numbers (the compiled
            // program had called the helper's float version with a dual,
            // and not built)
            let mut todo: Vec<String> = grads.iter().cloned().collect();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut helpers = BTreeSet::new();
            let numbers = |t: &Ty| matches!(t, Ty::Int | Ty::Float);
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                let Some(f) = self.fns.get(&n) else { continue };
                if !grads.contains(&n) {
                    // (a helper of anything else is left as it is)
                    if !(f.params.iter().all(|(_, t)| numbers(t)) && numbers(&f.ret)) {
                        continue;
                    }
                    helpers.insert(n.clone());
                }
                crate::ir::visit_block(&f.body, &mut |e| {
                    if let Expr::Call { target: Target::User(c), .. } = e {
                        todo.push(c.clone());
                    }
                });
            }
            self.grad_functions = grads;
            self.dual_helpers = helpers;
        }

        // route handlers take the request and answer a response
        for b in &blocks {
            for f in Self::fn_args_of_calls(b, "route", &|args| {
                // the handler: the last function named (a limit may follow it)
                if args.len() >= 2 {
                    args.iter().rev().find_map(fn_named).map(|s| vec![s.to_string()]).unwrap_or_default()
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
                    p.1 = Ty::WebRequest;
                }
                // a handler whose value can fail keeps its Result, and its
                // route answers the failure (route_handler)
                let response = Ty::Handle(&crate::table::RESPONSE);
                f.ret = if f.ret.is_result() { Ty::Result(Box::new(response)) } else { response };
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
        // a program without a page that declares derivations holds the app's
        // state: its routes read the derivations the app holds through it
        if !self.has_page_routes && !self.part.derives.is_empty() {
            self.app_state = true;
            self.needs_crow_web = true;
            self.web_features.insert("live".to_string());
            if self.reactive_vars.is_empty() && self.static_handle_vars.is_empty() && self.data_source_vars.is_empty() {
                self.read_facts();
            }
        }

        // a struct parameter a function never changes — reassigns, or
        // assigns a field or an item of — passes by reference (a handler's
        // are its own: it may be sent them as an event's arguments, so it
        // takes them by value, and so do its callers)
        // and a list, a map or a text a plain function never changes nor
        // hands on — no assignment, no move, no binding of its name — is
        // read where its caller holds it, printed `(*p)` (a call had copied
        // it: a long list handed to a function called once per item was
        // quadratic, revcomp's lines and k-nucleotide's chunks)
        let as_values = self.functions_as_values();
        let all: Vec<&Fn> = self.fns.values().collect();
        let lendable = crate::ir::lendable_params(&all);
        // (a function a builtin written in Crow is — `_frame_load`, a
        // driver's `ch_open` — is called through its row too, with values)
        let rows_of: HashSet<String> = crate::table::ROWS.iter().filter(|r| r.is_crow()).map(crate::table::crow_fn_name).collect();
        let names: Vec<String> = self.fns.keys().cloned().collect();
        for n in names {
            if self.reactive_event_handlers.contains(&n) || self.live_handle_fns.contains(&n) {
                continue;
            }
            let f = self.fns.get(&n).unwrap().clone();
            let changed = Self::mutated_in(&f.body);
            let mut refs = HashSet::new();
            let mut lent = HashSet::new();
            let plain = !(as_values.contains(&n)
                || rows_of.contains(crate::ir::copy_base(&n))
                || self.reactive_render_fns.contains(&n)
                || self.live_render_fns.contains(&n)
                || self.views.contains(&n)
                || self.web_handlers.contains(&n)
                || self.traced_functions.contains(&n)
                || self.grad_functions.contains(&n)
                || self.dual_helpers.contains(&n)
                || f.kind == FnKind::Handler);
            for (i, (p, t)) in f.params.iter().enumerate() {
                // (a record or none is lent as a record is: handed by value,
                // each call had counted the link up and down — but never to
                // a function called through a value, whose signature is the
                // function type that holds it: one taking a record of a list
                // had not built)
                if let Ty::Record(s) | Ty::OptRecord(s) = t {
                    if plain && !self.struct_is_copy(s) && !changed.contains(p) {
                        refs.insert(p.clone());
                    }
                } else if plain && matches!(as_inferred(t), Ty::List(_) | Ty::Map(..) | Ty::Str) && lendable.get(&n).and_then(|v| v.get(i)).copied().unwrap_or(false) {
                    refs.insert(p.clone());
                    lent.insert(p.clone());
                }
            }
            if !refs.is_empty() {
                self.by_ref_params.insert(n.clone(), refs);
            }
            if !lent.is_empty() {
                self.lent_params.insert(n, lent);
            }
        }

        // a tensor anywhere needs the runtime
        let mut tensor = false;
        let mut json = false;
        let mut check = |t: &Ty| {
            fn has(t: &Ty, want: &dyn std::ops::Fn(&Ty) -> bool) -> bool {
                want(t)
                    || match t {
                        Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) | Ty::Tensor(i, _) | Ty::Column(i) => has(i, want),
                        _ => false,
                    }
            }
            if has(t, &|t| matches!(t, Ty::Tensor(..))) {
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
        // and every expression's own type: a cell of a column nothing typed
        // is dynamic where it is read (`log[f.px[i]]`), bound to no local
        for b in self.all_blocks() {
            visit_block(&b, &mut |e| check(&e.ty()));
        }
        if tensor {
            self.needs_crow_runtime = true;
        }
        if json {
            self.needs_serde_json = true;
        }
    }

    /// Every function the compiled program can run: what its top level,
    /// its facts, its derivations and its views and handlers call, name or
    /// hand on, and what those call in turn.
    /// The backends whose driver written in Crow the program carries, by
    /// the functions it has (docs/db-in-crow-design.md).
    pub(crate) fn drivers_carried(&self) -> Vec<(&'static str, &'static str)> {
        [("clickhouse", "db.clickhouse"), ("postgres", "db.postgres")].iter().filter(|(_, m)| crate::table::ROWS.iter().any(|r| r.module == *m && r.is_crow() && self.fns.contains_key(&crate::table::crow_fn_name(r)))).copied().collect()
    }

    /// The table of calls crow_db::driven calls a driver through, each the
    /// generated function of the driver's row: a result's failure of kind
    /// `sys` says the connection is gone.
    fn driver_glue(&mut self, backend: &str) {
        let prefix = match backend {
            "clickhouse" => "_db_clickhouse_ch",
            "postgres" => "_db_postgres_pg",
            _ => return,
        };
        let failed = "Err(crow_db::driven::Failure { reason: _r.reason, lost: _r.kind == \"sys\" })";
        self.line(&format!("struct __CrowDriver{};", backend));
        self.line(&format!("impl crow_db::driven::Calls for __CrowDriver{} {{", backend));
        self.line(&format!("    fn open(&self, parts: &str) -> Result<crow_runtime::sys::CrowStream, crow_db::driven::Failure> {{ let _r = {p}_open(parts.to_string()); if _r.ok {{ Ok(_r.value) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line(&format!("    fn check(&self, parts: &str, s: &crow_runtime::sys::CrowStream) -> Result<(), crow_db::driven::Failure> {{ let _r = {p}_check(parts.to_string(), s.clone()); if _r.ok {{ Ok(()) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line(&format!("    fn query(&self, parts: &str, s: &crow_runtime::sys::CrowStream, sql: &str, params: &str) -> Result<crow_runtime::frame::CrowFrame, crow_db::driven::Failure> {{ let _r = {p}_query(parts.to_string(), s.clone(), sql.to_string(), params.to_string()); if _r.ok {{ Ok(_r.value) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line(&format!("    fn exec(&self, parts: &str, s: &crow_runtime::sys::CrowStream, sql: &str, params: &str) -> Result<i64, crow_db::driven::Failure> {{ let _r = {p}_exec(parts.to_string(), s.clone(), sql.to_string(), params.to_string()); if _r.ok {{ Ok(_r.value) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line(&format!("    fn insert(&self, parts: &str, s: &crow_runtime::sys::CrowStream, table: &str, frame: &crow_runtime::frame::CrowFrame) -> Result<i64, crow_db::driven::Failure> {{ let _r = {p}_insert(parts.to_string(), s.clone(), table.to_string(), frame.clone()); if _r.ok {{ Ok(_r.value) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line(&format!("    fn describe(&self, parts: &str, s: &crow_runtime::sys::CrowStream, sql: &str) -> Result<crow_runtime::frame::CrowFrame, crow_db::driven::Failure> {{ let _r = {p}_describe(parts.to_string(), s.clone(), sql.to_string()); if _r.ok {{ Ok(_r.value) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line(&format!("    fn cancel(&self, parts: &str, s: &crow_runtime::sys::CrowStream) -> Result<(), crow_db::driven::Failure> {{ let _r = {p}_cancel(parts.to_string(), s.clone()); if _r.ok {{ Ok(()) }} else {{ {f} }} }}", p = prefix, f = failed));
        self.line("}");
        self.line("");
    }

    fn reached_functions(&self) -> BTreeSet<String> {
        let mut todo: Vec<String> = Vec::new();
        // a database driver written in Crow is called from Rust, not the
        // program: every row of it is reached
        for (_, module) in self.drivers_carried() {
            todo.extend(crate::table::ROWS.iter().filter(|r| r.module == module && r.is_crow()).map(crate::table::crow_fn_name));
        }
        let note = |e: &Expr, todo: &mut Vec<String>| match e {
            Expr::Call { target: Target::User(n), .. } | Expr::FnRef(n, _) => todo.push(n.clone()),
            Expr::Deferred { name, .. } => todo.push(name.clone()),
            // a builtin written in Crow reaches its function (an effect's
            // row, or a call's)
            Expr::Call { target: Target::Row(row), .. } | Expr::Effect { row: Some(row), .. } if row.is_crow() => todo.push(crate::table::crow_fn_name(row)),
            // one the lowering left to the values: every row of the name,
            // the emitter picking one where it is called
            Expr::Effect { row: None, name, .. } => {
                let (m, f) = name.rsplit_once('.').unwrap_or(("", name.as_str()));
                crate::table::ROWS.iter().filter(|r| r.is_crow() && r.module == m && r.name == f).for_each(|r| todo.push(crate::table::crow_fn_name(r)));
            }
            Expr::Call { target: Target::Module(m, f), .. } => {
                if let Some((m, f)) = format!("{}.{}", m, f).rsplit_once('.') {
                    crate::table::ROWS.iter().filter(|r| r.is_crow() && r.module == m && r.name == f).for_each(|r| todo.push(crate::table::crow_fn_name(r)));
                }
            }
            Expr::Call { target: Target::Runtime(n), .. } => {
                crate::table::ROWS.iter().filter(|r| r.is_crow() && r.module.is_empty() && r.name == n.as_str()).for_each(|r| todo.push(crate::table::crow_fn_name(r)));
            }
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
                // a source is a frame fact of the session, typed by the
                // program's reads (Fact::held) and decoded from the frame's
                // own form as every frame fact is; main registers it where
                // it would set another
                let ty = match as_inferred(&fact.ty) {
                    Ty::Unknown => Ty::Frame(None),
                    t => t,
                };
                self.reactive_vars.insert(fact.name.clone(), (ty.clone(), ty.default_rust_value()));
                continue;
            }
            // a connection is infrastructure: a process-wide static, as a
            // mailer is
            if matches!(fact.ty, Ty::Handle(h) if !h.session) || matches!(self.tt(&fact.default), Ty::Handle(h) if !h.session) {
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
            let declared = as_inferred(&fact.ty).text();
            // (a declared value that can fail is refused where the facts are
            // set: what it holds is the payload)
            let vty = match self.tt(&value) {
                Ty::Result(inner) if fact.refuses => *inner,
                other => other,
            };
            let ty = if matches!(declared, Ty::Unknown) { vty.clone() } else { declared };
            let widened = matches!(vty, Ty::Int) && matches!(ty, Ty::Float);
            let saved_line = self.current_line;
            self.current_line = fact.line;
            let text = self.top_level_expr(&value);
            // (a view's node given to a name held as markup is its HTML)
            let text = if self.makes_node(&value) && matches!(ty, Ty::Markup | Ty::Str) { format!("({}).to_html()", text) } else { text };
            let text = Self::refused_fact(&fact, text);
            self.current_line = saved_line;
            let open_list = matches!(Emitter::raw(&value), Expr::ListLit(items, _) if items.is_empty());
            let open_map = matches!(Emitter::raw(&value), Expr::MapLit(p, _) if p.is_empty())
                || matches!(Emitter::raw(&value), e @ Expr::Call { target: Target::Runtime(_) | Target::Row(_), args, .. } if crate::ir::call_name(e).as_deref() == Some("map") && args.is_empty());
            let default = if widened {
                format!("({}) as f64", text)
            } else if open_list {
                match &ty {
                    Ty::List(inner) => format!("Vec::<{}>::new()", inner.to_rust_type()),
                    _ => text,
                }
            } else if open_map && matches!(ty, Ty::Map(..)) {
                ty.default_rust_value()
            } else {
                text
            };
            let mut reads_a_fact = false;
            crate::ir::visit_expr(&value, &mut |e| {
                if matches!(e, Expr::Load(..)) {
                    reads_a_fact = true;
                }
            });
            // (a refused value is computed once, where the facts are set:
            // a render's getter does not read the file again)
            if reads_a_fact || fact.refuses {
                self.fact_inits.insert(fact.name.clone(), default);
                let fallback = ty.default_rust_value();
                self.reactive_vars.insert(fact.name.clone(), (ty, fallback));
                continue;
            }
            self.reactive_vars.insert(fact.name.clone(), (ty, default));
        }
    }

    /// What a source is held to, as the session takes it: each column the
    /// program reads, its kind and the read that decided it.
    fn held_literal(held: &[(String, String, String)]) -> String {
        let items: Vec<String> = held.iter().map(|(n, k, h)| format!("({:?}.to_string(), {:?}.to_string(), {:?}.to_string())", n, k, h)).collect();
        format!("vec![{}]", items.join(", "))
    }

    /// One fact's declared value set in main's state: bound by its name too
    /// when a later declaration reads it. (A value written as a block is an
    /// expression to json!, not an object, once it is in parentheses.)
    fn set_fact_in_main(&mut self, v: &str, bound: bool) {
        let d = self.fact_inits.get(v).cloned().unwrap_or_else(|| self.reactive_vars[v].1.clone());
        if bound {
            self.line(&format!("let {} = {};", v, d));
            self.line(&format!("__crow_state.set(\"{}\", serde_json::json!({}));", v, v));
        } else {
            self.line(&format!("__crow_state.set(\"{}\", serde_json::json!(({})));", v, d));
        }
    }

    /// A fact's declared value as main sets it: one that can fail is
    /// refused there, with the reason, and the fact holds the payload.
    fn refused_fact(fact: &Fact, text: String) -> String {
        if !fact.refuses {
            return text;
        }
        format!(
            "{{ let _rv = {}; if !_rv.ok {{ crow_stop({}, format!(\"{} — {{}}\", _rv.reason)) }} _rv.value }}",
            text,
            fact.line,
            Decision::Assign(fact.name.clone()).refusal().replace('{', "{{").replace('}', "}}")
        )
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
        if self.cpu_count.is_some() {
            self.needs_crow_runtime = true;
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
        if self.any_type(&|t| matches!(t, Ty::Sparse)) {
            self.line("#[allow(unused_imports)]");
            self.line("use crow_runtime::sparse::CrowSparse;");
        }
        if self.needs_crow_web {
            // (the server comes with the `http` feature: a program that only
            // makes a response from bytes, or sends mail, links the core)
            if self.web_features.contains("http") || self.web_features.contains("live") {
                self.line("#[allow(unused_imports)]");
                self.line("use crow_web::server::CrowApp;");
            }
            self.line("#[allow(unused_imports)]");
            self.line("use crow_web::core::{CrowRequest, CrowResponse};");
            self.line("#[allow(unused_imports)]");
            self.line("use std::sync::Arc;");
            if self.needs_ui {
                self.line("#[allow(unused_imports)]");
                self.line("use crow_web::view::CrowVNode;");
                self.line("#[allow(unused_imports)]");
                self.line("use crow_web::live::{CrowLiveView, CrowLiveState};");
            } else if self.app_state {
                self.line("#[allow(unused_imports)]");
                self.line("use crow_web::live::CrowLiveState;");
            }
        }
        if self.needs_crow_db {
            self.line("#[allow(unused_imports)]");
            self.line("use crow_db::DbHandle;");
            for (backend, _) in self.drivers_carried() {
                self.driver_glue(backend);
            }
        }
        self.line("");

        // the records a fact holds are kept as JSON by the session, and a
        // handler's record arguments arrive as JSON
        self.serde_records = self.records_as_json();
        let result_serde = self.part.structs.iter().any(|s| self.serde_records.contains(&s.name) && s.fields.iter().any(|(_, t)| t.is_result()));
        if !self.serde_records.is_empty() {
            self.needs_serde_json = true;
        }
        // every program stops through the runtime (crow_stop, main_guard)
        self.needs_crow_runtime = true;
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
            // a trace names each place an error passed once: a refusal on
            // the line the error came from adds nothing
            "#[allow(dead_code)]",
            "fn crow_trace_push(t: &mut Vec<String>, at: String) { if t.last() != Some(&at) { t.push(at); } }",
            // a runtime failure the program cannot go on from stops it as
            // the interpreter stops, with the runtime's words
            "#[allow(dead_code)]",
            "fn crow_stop(line: usize, why: String) -> ! { crow_runtime::stops::halt(line, &why) }",
            // a value that can fail compares as `eq` compares one: an error
            // equals nothing, as the interpreter's does (a list item, a field)
            "impl<T: Clone + PartialEq> PartialEq for CrowResult<T> {",
            "    fn eq(&self, other: &Self) -> bool { self.ok && other.ok && self.value == other.value }",
            "}",
            "impl<T: Clone + std::fmt::Display> std::fmt::Display for CrowResult<T> {",
            "    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {",
            "        if !self.ok { write!(f, \"{}\", crow_runtime::display::failed(&self.reason, &self.trace)) } else { write!(f, \"{}\", self.value) }",
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
            if !matches!(s.name.as_str(), "CrowFindResult" | "CrowShell" | "CrowReply" | "CrowAnswer") {
                self.emit_struct(s);
            }
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
            self.in_crow_fn = crate::modules::is_crow_fn(&f.name);
            let scope = self.registry.enter(&f.name);
            self.emit_function(&f);
            self.registry.resume(scope);
            self.in_crow_fn = false;
        }
        let derives: Vec<Derive> = self.part.derives.clone();
        for d in &derives {
            if !(self.needs_ui && self.has_page_routes) && !self.app_state {
                return Err(format!("derive {} needs a program that serves — a page's views and handlers read it, or a route's handler (derive-design.md)", d.name));
            }
            self.emit_derive(d);
        }
        if self.needs_grad {
            for g in self.grad_functions.iter().chain(self.dual_helpers.iter()).cloned().collect::<Vec<_>>() {
                if let Some(f) = self.fns.get(&g).cloned() {
                    self.emit_dual_function(&f);
                }
            }
        }
        for t in self.traced_functions.clone() {
            if let Some(f) = self.fns.get(&t).cloned() {
                self.in_crow_fn = crate::modules::is_crow_fn(&f.name);
                let scope = self.registry.enter(&f.name);
                self.emit_traced_function(&f);
                self.registry.resume(scope);
                self.in_crow_fn = false;
            }
        }
        if !self.reactive_event_handlers.is_empty() {
            self.emit_handle_dispatch();
        }
        if self.test_control && self.has_page_routes {
            self.emit_test_tables();
        }
        self.emit_main();
        self.emit_facts_struct();
        self.emit_synthetic_types();
        // a result as the display kernel shows it inside a list: its value,
        // or its reason (written last: only a program that links the kernel
        // knows by now that it does)
        if self.needs_crow_runtime {
            for l in [
                "impl<T: Clone + crow_runtime::display::CrowDisplay> crow_runtime::display::CrowDisplay for CrowResult<T> {",
                "    fn crow_repr(&self) -> String { if !self.ok { crow_runtime::display::failed(&self.reason, &self.trace) } else { self.value.crow_repr() } }",
                "    fn crow_fmt(&self) -> String { if !self.ok { crow_runtime::display::failed(&self.reason, &self.trace) } else { self.value.crow_fmt() } }",
                "}",
                // a value that failed encodes as `null`
                "impl<T: Clone + crow_runtime::json::ToJson> crow_runtime::json::ToJson for CrowResult<T> {",
                "    fn to_json(&self) -> crow_runtime::json::Json { if self.ok { self.value.to_json() } else { crow_runtime::json::null() } }",
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
                    Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) | Ty::Tensor(i, _) | Ty::Column(i) => has(i, pred),
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
            // a parsed JSON value as the one display shows it (a whole float
            // as an int, JSON's null as the error it reads as)
            "fn crow_json_display(v: &serde_json::Value) -> String {",
            "    use crow_runtime::display::CrowDisplay as _;",
            "    v.crow_fmt()",
            "}",
            "",
        ] {
            self.line(l);
        }
    }

    /// A user module's top-level values (`mymath_pi`), printed as consts.
    /// A standard module hoists no value (the inliner carries its functions
    /// and records alone), so its prefix names none: a program's own
    /// `mcp_s = server[…]` had been printed as a const and the compiled
    /// program had not built (tests/llm_server.rs, the loop's servers).
    fn emit_module_consts(&mut self) {
        if self.needs_ui {
            return;
        }
        let main = self.part.main.clone();
        let mut any = false;
        for s in &main {
            if let Stmt::Let { var, expr, .. } = s {
                let prefixed = self.modules.iter().filter(|m| crate::modules::module_by_path(m).is_none()).any(|m| var.starts_with(&format!("{}_", m)));
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
                Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => records(i, out),
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

    /// Whether a record's values compare as `eq` compares them — field by
    /// field — in Rust too: every field's type does (a function, a handle
    /// and a view's tree do not). A record reached again through its own
    /// fields compares if the rest does.
    pub(crate) fn record_compares(&self, name: &str, seen: &mut BTreeSet<String>) -> bool {
        fn compares(e: &Emitter, t: &Ty, seen: &mut BTreeSet<String>) -> bool {
            match t {
                Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Json | Ty::Tensor(..) | Ty::Frame(_) | Ty::Column(_) => true,
                Ty::Sparse => true,
                Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => compares(e, i, seen),
                Ty::Record(n) | Ty::OptRecord(n) => e.record_compares(n, seen),
                _ => false,
            }
        }
        if !seen.insert(name.to_string()) {
            return true;
        }
        match self.part.structs.iter().find(|s| s.name == name) {
            Some(s) => s.fields.iter().all(|(_, t)| compares(self, t, seen)),
            None => false,
        }
    }

    pub(crate) fn emit_struct(&mut self, s: &StructDef) {
        let all_copy = s.fields.iter().all(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool));
        let serde = if self.serde_records.contains(&s.name) { ", serde::Serialize, serde::Deserialize" } else { "" };
        let eq = if self.record_compares(&s.name, &mut BTreeSet::new()) { ", PartialEq" } else { "" };
        self.line("#[allow(non_camel_case_types)]");
        self.line(&format!("#[derive(Debug, Clone{}{}{})]", if all_copy { ", Copy" } else { "" }, eq, serde));
        self.line(&format!("struct {} {{", s.name));
        self.indent += 1;
        for (f, t) in &s.fields {
            let rust = as_inferred(t).to_rust_type();
            // a tensor, a frame or a column field is the runtime's type
            if rust.contains("crow_runtime::") {
                self.needs_crow_runtime = true;
            }
            self.line(&format!("{}: {},", Self::rust_field(f), rust));
        }
        self.indent -= 1;
        self.line("}");
        // the value an error of this struct's type carries: each field's default
        let defaults: Vec<String> = s.fields.iter().map(|(f, t)| format!("{}: {}", Self::rust_field(f), as_inferred(t).default_rust_value())).collect();
        self.line("#[allow(dead_code)]");
        self.line(&format!("impl {} {{ fn crow_default() -> Self {{ {} {{ {} }} }} }}", s.name, s.name, defaults.join(", ")));
        self.line("");
        // a record as the one display shows it, `P[x = 1, y = 2]`, and as the
        // one JSON rule encodes it, an object of its fields
        if self.struct_kernel_displayable(&s.name) {
            self.needs_crow_runtime = true;
            let fields: Vec<String> = s.fields.iter().map(|(f, _)| format!("(\"{}\", self.{}.crow_repr())", f, Self::rust_field(f))).collect();
            self.line(&format!("impl crow_runtime::display::CrowDisplay for {} {{", s.name));
            self.line("    fn crow_repr(&self) -> String {");
            self.line(&format!("        {{ use crow_runtime::display::CrowDisplay as _; crow_runtime::display::record(\"{}\", vec![{}]) }}", s.shown, fields.join(", ")));
            self.line("    }");
            self.line("}");
            self.line("");
        }
        if self.struct_encodes(&s.name) {
            self.needs_crow_runtime = true;
            let fields: Vec<String> = s.fields.iter().map(|(f, _)| format!("(\"{}\", self.{}.to_json())", f, Self::rust_field(f))).collect();
            self.line(&format!("impl crow_runtime::json::ToJson for {} {{", s.name));
            self.line("    fn to_json(&self) -> crow_runtime::json::Json {");
            self.line(&format!("        {{ use crow_runtime::json::ToJson as _; crow_runtime::json::object(vec![{}]) }}", fields.join(", ")));
            self.line("    }");
            self.line("}");
            self.line("");
        }
    }

    /// The records whose every field has a JSON form (numbers, text, lists
    /// and maps of them, results of them, tensors, and such records).
    fn json_records(&self) -> BTreeSet<String> {
        fn jsonable(t: &Ty, ok: &BTreeSet<String>) -> bool {
            match t {
                Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Json | Ty::Tensor(..) => true,
                Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => jsonable(i, ok),
                Ty::Record(n) => ok.contains(n),
                _ => false,
            }
        }
        // a record whose fields are records joins once they have
        let mut ok: BTreeSet<String> = BTreeSet::new();
        loop {
            let before = ok.len();
            for (name, def) in &self.structs {
                if matches!(name.as_str(), "CrowFindResult" | "CrowShell" | "CrowReply" | "CrowAnswer") || ok.contains(name) {
                    continue;
                }
                if def.fields.iter().all(|(_, t)| jsonable(&as_inferred(t), &ok)) {
                    ok.insert(name.clone());
                }
            }
            if ok.len() == before {
                return ok;
            }
        }
    }

    /// Whether a record shows through the one display
    /// (crow_runtime::display): every field does. Every such record has a
    /// `CrowDisplay` written with the program — a record holding a list, a
    /// list of records and a map of them had shown in Rust's debug form.
    pub(crate) fn struct_kernel_displayable(&self, name: &str) -> bool {
        self.record_is(name, &|e, t, seen| e.ty_shows(t, seen), &mut BTreeSet::new())
    }

    /// Whether a record's values encode by the one JSON rule
    /// (crow_runtime::json::ToJson): every field does.
    pub(crate) fn struct_encodes(&self, name: &str) -> bool {
        self.record_is(name, &|e, t, seen| e.ty_encodes(t, seen), &mut BTreeSet::new())
    }

    /// Whether every field of a record has a property; the runtime's own
    /// records have it, and a record reached again through its own fields
    /// has it if the rest does.
    fn record_is(&self, name: &str, field: &dyn std::ops::Fn(&Emitter, &Ty, &mut BTreeSet<String>) -> bool, seen: &mut BTreeSet<String>) -> bool {
        match name {
            "CrowShell" | "CrowFindResult" | "CrowReply" | "CrowAnswer" => return true,
            _ => {}
        }
        if !seen.insert(name.to_string()) {
            return true;
        }
        self.structs.get(name).map_or(false, |s| s.fields.iter().all(|(_, t)| field(self, t, seen)))
    }

    /// Whether a value of this type shows through the one display.
    fn ty_shows(&self, t: &Ty, seen: &mut BTreeSet<String>) -> bool {
        match t {
            Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Json | Ty::Tensor(..) | Ty::Frame(_) | Ty::Column(_) => true,
            Ty::Sparse => true,
            // (a moment, bytes and the handles show as the runtime shows
            // them, and an item nothing typed is held as text: a record of
            // one had had no display, and `log[b]` had not built — crow check
            // --self)
            Ty::Moment | Ty::Bytes | Ty::Handle(_) | Ty::Unknown => true,
            Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => self.ty_shows(i, seen),
            Ty::Record(n) | Ty::OptRecord(n) => self.record_is(n, &|e, t, seen| e.ty_shows(t, seen), seen),
            _ => false,
        }
    }

    /// Whether a value of this type encodes by the one JSON rule.
    fn ty_encodes(&self, t: &Ty, seen: &mut BTreeSet<String>) -> bool {
        match t {
            Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Json | Ty::Tensor(..) => true,
            Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => self.ty_encodes(i, seen),
            Ty::Record(n) | Ty::OptRecord(n) => self.record_is(n, &|e, t, seen| e.ty_encodes(t, seen), seen),
            _ => false,
        }
    }

    /// Whether a list, a map or a record shows through the one display: its
    /// type's `CrowDisplay`, the runtime's or one written with the program.
    /// (A number, a bool, text, a tensor and a frame show as Rust displays
    /// them, which is the one display's; a JSON value through its own.)
    pub(crate) fn kernel_displayable(&self, ty: &Ty) -> bool {
        match ty {
            // an empty map's values are no type yet
            Ty::Map(_, inner) if matches!(inner.as_ref(), Ty::Unknown) => true,
            Ty::List(inner) | Ty::Map(_, inner) | Ty::Result(inner) => self.shows_inside(inner),
            Ty::Record(n) | Ty::OptRecord(n) => self.struct_kernel_displayable(n),
            _ => false,
        }
    }

    /// Whether a value shows inside a list, a map or a record.
    fn shows_inside(&self, t: &Ty) -> bool {
        // (and the layer's bytes, a stream, a socket, a mailer: a list of
        // bytes interpolated had shown in Rust's debug form, `Bytes([97,
        // 98])`, where the interpreter shows `<2 bytes>` — crow check --self)
        matches!(t, Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Json | Ty::Tensor(..) | Ty::Frame(_) | Ty::Column(_) | Ty::Sparse | Ty::Bytes | Ty::Handle(_) | Ty::Moment)
            || self.kernel_displayable(t)
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
        // field has a JSON form (a detector's result read by a test); one the
        // program encodes has its `ToJson` already, as `CrowResult` has
        // (written with the program: a value that failed encodes as null)
        let records = self.json_records();
        let unwritten: Vec<String> = records.iter().filter(|n| !self.struct_encodes(n)).cloned().collect();
        for name in &unwritten {
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
        if !unwritten.is_empty() {
            self.needs_crow_runtime = true;
        }
        self.line("#[allow(unused_variables, dead_code)]");
        self.line("fn __crow_derived(state: &CrowLiveState, name: &str) -> Option<serde_json::Value> {");
        self.indent += 1;
        self.line("match name {");
        self.indent += 1;
        let as_json = |t: &Ty, v: &str| -> Option<String> {
            match t {
                Ty::Int | Ty::Float | Ty::Str | Ty::Markup | Ty::Bool | Ty::Json | Ty::Frame(_) => {
                    Some(format!("serde_json::to_value(&{}).unwrap_or(serde_json::Value::Null)", v))
                }
                Ty::Tensor(..) => Some(format!("crow_runtime::json::ToJson::to_json(&{})", v)),
                Ty::List(i) | Ty::Map(_, i) if matches!(i.as_ref(), Ty::Int | Ty::Float | Ty::Str | Ty::Markup | Ty::Bool | Ty::Json) => {
                    Some(format!("serde_json::to_value(&{}).unwrap_or(serde_json::Value::Null)", v))
                }
                Ty::Record(n) if records.contains(n) => Some(format!("crow_runtime::json::ToJson::to_json(&{})", v)),
                // a list of records reads as the array of their maps (the
                // rag example's chunks: a test reads `a.chunks[0].file`
                // compiled as it does interpreted)
                Ty::List(i) if matches!(i.as_ref(), Ty::Record(n) if records.contains(n)) => Some(format!("crow_runtime::json::ToJson::to_json(&{})", v)),
                _ => None,
            }
        };
        let derives: Vec<(String, Ty)> = self.part.derives.iter().map(|d| (d.name.clone(), d.ty.clone())).collect();
        for (d, ty) in derives {
            let rt = as_inferred(&ty);
            let read = format!("__derive_{}(state)", d);
            match &rt {
                Ty::Result(inner) => {
                    if let Some(value) = as_json(inner, "_r.value") {
                        self.line(&format!(
                            "\"{}\" => {{ let _r = {}; Some(if _r.ok {{ serde_json::json!({{ \"ok\": true, \"value\": {} }}) }} else {{ serde_json::json!({{ \"ok\": false, \"reason\": _r.reason, \"kind\": _r.kind }}) }}) }},",
                            d, read, value
                        ));
                    }
                }
                other => {
                    if let Some(value) = as_json(other, &read) {
                        if value.contains("crow_runtime::") {
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
            // a specialized handler's original is not compiled, its clones
            // are: the event goes to the clone of the original's own
            // parameter types, which means what the original does, or else
            // the first one the page can hand arguments to
            let clones: Vec<String> = self
                .fns
                .keys()
                .filter(|k| k.strip_prefix(h.as_str()).and_then(|r| r.strip_prefix("__")).map_or(false, |d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())))
                .cloned()
                .collect();
            let target = if clones.is_empty() {
                h.clone()
            } else {
                let own: Vec<Ty> = self.fns.get(&h).map(|f| f.params.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
                let plain = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup);
                let same = clones.iter().find(|c| self.fns.get(*c).map_or(false, |f| f.params.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>() == own));
                match same.or_else(|| clones.iter().find(|c| self.fns.get(*c).map_or(false, |f| f.params.iter().all(|(_, t)| plain(t))))) {
                    Some(c) => c.clone(),
                    None => continue,
                }
            };
            let rust = Self::rust_fn_name(&target);
            let params: Vec<(String, Ty)> = self.fns.get(&target).map(|f| f.params.clone()).unwrap_or_default();
            // a list, a tensor, a frame or a record arrives as JSON its own
            // serde reads (a test clicks a handler with one as it clicks any)
            let by_serde = |t: &Ty| matches!(t, Ty::List(_) | Ty::Map(..) | Ty::Tensor(..) | Ty::Frame(_) | Ty::Record(_) | Ty::OptRecord(_) | Ty::Moment);
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
                            decoded.push((format!("_p{}", i), format!("serde_json::from_value::<{}>(_a[{}].clone()).ok()", as_inferred(t).to_rust_type(), i)));
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
                // arguments the handler does not take stop it, as the
                // interpreter's call does
                self.line(&format!(
                    "\"{h}\" => {{ let _a: Vec<serde_json::Value> = _value.and_then(|_v| serde_json::from_str::<Vec<serde_json::Value>>(_v).ok()).unwrap_or_default(); if _a.len() == {n} {{ {call} }} else {{ state.stop(format!(\"{h} stopped: {h}[] expects {n} arguments, got {{}}\", _a.len())); }} }},",
                    h = h,
                    n = params.len(),
                    call = call
                ));
            }
        }
        // the page's own events are answered before the program's; any
        // other the program has no handler for stops, as the interpreter's
        // does (it was dropped without a word)
        self.line("_ if event.starts_with(\"__\") => {}");
        self.line("_ => state.stop(format!(\"{} stopped: unknown function '{}'\", event, event)),");
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
                    let noapp_route = name.as_deref() == Some("route") && (2..=4).contains(&args.len());
                    let noapp_serve = name.as_deref() == Some("serve") && (args.len() == 1 || args.len() == 11);
                    if noapp_route || noapp_serve {
                        self.has_noapp_web = true;
                    }
                }
            }
        }
        if !self.static_handle_vars.is_empty() {
            for n in self.static_handle_vars.clone() {
                // (the handle's own Rust type, as its declaration says)
                let held = self.part.facts.iter().find(|f| f.name == n).map(|f| match (&f.ty, self.tt(&f.default)) {
                    (Ty::Handle(h), _) => h.rust,
                    (_, Ty::Handle(h)) => h.rust,
                    _ => crate::table::DB.rust,
                });
                let ty = held.unwrap_or(crate::table::DB.rust);
                self.line("#[allow(non_upper_case_globals)]");
                self.line(&format!("static __crow_handle_{}: std::sync::OnceLock<{}> = std::sync::OnceLock::new();", n, ty));
            }
            self.line("");
        }
        // what a route's handler wrote, landed as it ends — its stop too:
        // the app's derivations over a table it wrote read it again (the
        // record of a thread's writes had grown with every write a route
        // made, nothing taking it)
        if self.needs_crow_db && !self.web_handlers.is_empty() {
            let land = if self.part.derives.is_empty() { "let _ = crow_db::take_write_keys();" } else { "crow_web::live::publish_writes(crow_db::take_write_keys());" };
            self.line("struct __CrowWrites;");
            self.line("impl Drop for __CrowWrites {");
            self.line(&format!("    fn drop(&mut self) {{ {} }}", land));
            self.line("}");
            self.line("");
        }
        // a stop carried back from another thread (a parallel loop's) is the
        // program's stop, printed once; any other panic names itself in
        // Crow's words (crow_runtime::stops)
        self.line("#[global_allocator]");
        self.line("static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;");
        self.line("");
        self.line("fn main() {");
        self.line("    crow_runtime::stops::install_hook();");
        if let (true, Some(mode)) = (self.needs_crow_db, self.schema_mode) {
            self.line(&format!("    crow_db::schema::set_mode(crow_db::schema::Mode::{:?});", mode));
        }
        // the drivers written in Crow the program carries, registered for
        // crow_db to call (docs/db-in-crow-design.md)
        for (backend, _) in self.drivers_carried() {
            self.line(&format!("    crow_db::driven::register({:?}, std::sync::Arc::new(__CrowDriver{}));", backend, backend));
        }
        if self.needs_crow_web {
            // the runtime's threads compute a page's derivations and run its
            // handlers: their stacks are sized for that work, not tokio's
            // 2 MiB (a debug build of finch's book walk overflowed it)
            self.line("    crow_runtime::stops::main_guard(|| tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(64 * 1024 * 1024).max_blocking_threads(crow_runtime::cores::blocking_threads()).build().expect(\"the async runtime\").block_on(__crow_main()));");
            self.line("}");
            self.line("");
            self.line("async fn __crow_main() {");
        } else {
            self.line("    crow_runtime::stops::main_guard(__crow_main);");
            self.line("}");
            self.line("");
            self.line("fn __crow_main() {");
            // a script's output in blocks when it goes to a file or a pipe
            // (crow_runtime::display::buffer_output; `main_guard` writes it
            // out as the program ends)
            if !self.test_control {
                self.line("    crow_runtime::display::buffer_output();");
            }
        }
        self.needs_crow_runtime = true;
        self.indent += 1;
        self.cur_fn = String::new();
        self.cur_ret = None;
        self.returns_unit = true;
        self.scopes = vec![HashMap::new()];
        self.mutated = Self::mutated_in(&self.part.main.clone());
        // the part's resources: the crowfile's core count, every thread's
        // default (crow_runtime::cores)
        if let Some(n) = self.cpu_count {
            self.line(&format!("crow_runtime::cores::set_default({});", n));
        }
        if self.part.gpu.is_some() {
            self.note_feature("gpu");
            self.needs_crow_runtime = true;
            self.line("crow_runtime::gpu::set_gpu_enabled(true);");
        }
        if self.has_noapp_web && !self.has_page_routes {
            self.line("let mut __crow_app = CrowApp::new();");
        }
        let main = self.part.main.clone();
        // the top level's texts read by position (texts.rs)
        self.indexed_texts = super::texts::indexed_texts(&[], &main);
        if self.has_page_routes || self.app_state {
            if self.has_page_routes {
                self.line("let mut __crow_app = CrowApp::new();");
            }
            self.line("let mut __crow_state = CrowLiveState::new();");
            // the session's facts in the program's own struct, typed field by
            // field (typed-values-design.md §6)
            if !self.reactive_vars.is_empty() {
                self.line("__crow_state.install_facts(Box::new(CrowFacts::default()));");
            }
            // the facts in the order they are declared — a connection, a data
            // source, or a value the sessions start from — so a declaration
            // can read one declared before it: a value (`tripled = base * 3`,
            // which had read a name nothing bound) or a connection (`made =
            // query[book, …]`, which had read the connection before it was
            // set, and the program stopped as it started)
            let mut read_later = BTreeSet::new();
            for f in &self.part.facts {
                crate::ir::visit_expr(&f.default, &mut |e| {
                    if let Expr::Load(n, _) = e {
                        read_later.insert(n.clone());
                    }
                });
            }
            // (and one the top level reads after the declarations —
            // `log[count]` — as the interpreter reads the declared value)
            visit_block(&main, &mut |e| {
                if let Expr::Load(n, _) = e {
                    read_later.insert(n.clone());
                }
            });
            if !self.reactive_vars.is_empty() {
                self.needs_serde_json = true;
            }
            let facts = self.part.facts.clone();
            for fact in &facts {
                if self.static_handle_vars.contains(&fact.name) {
                    let v = self.expr(&fact.default);
                    self.line(&format!("let _ = __crow_handle_{}.set({});", fact.name, v));
                } else if self.data_source_vars.contains(&fact.name) {
                    let args = call_args(&fact.default).to_vec();
                    let held = Self::held_literal(&fact.held);
                    if call_name(&fact.default).as_deref() == Some("fetch") {
                        let url = self.expr(&args[0]);
                        self.line(&format!("__crow_state.register_fetch(\"{}\", &{}, {});", fact.name, url, held));
                    } else {
                        let url = self.expr(&args[0]);
                        let secs = self.expr(&args[1]);
                        self.line(&format!("__crow_state.register_poll(\"{}\", &{}, {} as u64, {});", fact.name, url, secs, held));
                    }
                } else if self.reactive_vars.contains_key(&fact.name) {
                    self.set_fact_in_main(&fact.name, read_later.contains(&fact.name));
                }
            }
            // (a session's value no declaration here holds keeps its place
            // after them)
            let rest: Vec<String> = self.reactive_vars.keys().filter(|k| !facts.iter().any(|f| &f.name == *k)).cloned().collect();
            for v in &rest {
                self.set_fact_in_main(v, read_later.contains(v));
            }
            // serve comes last, after every page is registered; a fact's
            // later top-level assignment set its default above
            let (serve, other): (Vec<Stmt>, Vec<Stmt>) = main.into_iter().partition(|s| matches!(s, Stmt::Do(e) if call_name(e).as_deref() == Some("serve")));
            let other: Vec<Stmt> = other.into_iter().filter(|s| !matches!(s, Stmt::Store { .. })).collect();
            // the app's own state, as declared: a route's handler reads the
            // derivations the app holds through it
            if !self.part.derives.is_empty() {
                self.line("crow_web::live::hold_app(&__crow_state);");
            }
            self.in_main = true;
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
            // what the top level wrote lands for the app before anything is
            // served: a derivation computed before a write reads it again
            if !self.part.derives.is_empty() && self.needs_crow_db {
                self.line("crow_web::live::publish_writes(crow_db::take_write_keys());");
            }
            self.emit_block(&serve);
            self.in_main = false;
        } else {
            // facts outside a page program are plain values, set first as
            // the interpreter sets them
            let facts = self.part.facts.clone();
            for fact in &facts {
                let saved_line = self.current_line;
                self.current_line = fact.line;
                let v = self.owned_expr(&fact.default);
                self.current_line = saved_line;
                let ty = as_inferred(&fact.ty).text();
                // (a view's node given to a name held as markup is its HTML)
                let v = if self.makes_node(&fact.default) && matches!(ty, Ty::Markup | Ty::Str) { format!("({}).to_html()", v) } else { v };
                let v = Self::refused_fact(fact, v);
                let vty = match self.tt(&fact.default) {
                    Ty::Result(inner) if fact.refuses => *inner,
                    other => other,
                };
                let (ty, v) = if matches!(ty, Ty::Float) && matches!(vty, Ty::Int) { (ty, format!("({}) as f64", v)) } else if matches!(ty, Ty::Unknown) { (vty, v) } else { (ty, v) };
                let kw = if self.mutated.contains(&fact.name) { "let mut" } else { "let" };
                if ty.has_unknown() {
                    self.line(&format!("{} {} = {};", kw, fact.name, v));
                } else {
                    self.line(&format!("{} {}: {} = {};", kw, fact.name, ty.to_rust_type(), v));
                }
                self.declare(&fact.name, ty);
            }
            let main: Vec<Stmt> = main.into_iter().filter(|s| !matches!(s, Stmt::Let { var, .. } if self.module_consts.contains(var))).collect();
            self.emit_block(&main);
        }
        if self.needs_later {
            self.line("crow_runtime::later::wait_all();");
        }
        self.indent -= 1;
        self.line("}");
    }

    /// The locals a block rebinds or changes in place: they are `mut`.
    /// The functions a program uses as values (`any[xs, f]`, `grad[f]`, a
    /// route's handler): their parameters are what the value's caller hands.
    fn functions_as_values(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for b in self.all_blocks() {
            visit_block(&b, &mut |e| {
                if let Expr::FnRef(n, _) = e {
                    out.insert(n.clone());
                }
            });
        }
        for f in self.fns.values() {
            visit_block(&f.body, &mut |e| {
                if let Expr::FnRef(n, _) = e {
                    out.insert(n.clone());
                }
            });
        }
        out
    }

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

    /// The session's facts as the program's own struct (typed-values-design
    /// §6): one typed field per fact, and the runtime's own keys beside them
    /// as JSON; read and written in place by the handlers, views and
    /// derivations through `__facts` and `__facts_mut`, and as JSON only at
    /// the edges the runtime asks for.
    fn emit_facts_struct(&mut self) {
        if self.reactive_vars.is_empty() || !(self.has_page_routes || self.app_state) {
            return;
        }
        let facts: Vec<(String, Ty)> = self.reactive_vars.iter().map(|(n, (t, _))| (n.clone(), t.clone())).collect();
        let rust_ty = |t: &Ty| t.to_rust_type();
        self.line("");
        self.line("#[allow(non_snake_case, dead_code)]");
        self.line("#[derive(Clone)]");
        self.line("struct CrowFacts {");
        for (n, t) in &facts {
            self.line(&format!("    {}: {},", Self::rust_field(n), rust_ty(t)));
        }
        self.line("    __rest: std::collections::HashMap<String, serde_json::Value>,");
        self.line("}");
        self.line("impl Default for CrowFacts {");
        self.line("    fn default() -> Self {");
        self.line("        CrowFacts {");
        for (n, t) in &facts {
            self.line(&format!("            {}: {},", Self::rust_field(n), t.default_rust_value()));
        }
        self.line("            __rest: std::collections::HashMap::new(),");
        self.line("        }");
        self.line("    }");
        self.line("}");
        self.line("impl crow_web::live::Facts for CrowFacts {");
        self.line("    fn box_clone(&self) -> Box<dyn crow_web::live::Facts> { Box::new(self.clone()) }");
        self.line("    fn get_json(&self, name: &str) -> Option<serde_json::Value> {");
        self.line("        match name {");
        for (n, _) in &facts {
            self.line(&format!("            \"{}\" => serde_json::to_value(&self.{}).ok(),", n, Self::rust_field(n)));
        }
        self.line("            _ => self.__rest.get(name).cloned(),");
        self.line("        }");
        self.line("    }");
        self.line("    fn set_json(&mut self, name: &str, value: serde_json::Value) -> bool {");
        self.line("        match name {");
        for (n, t) in &facts {
            self.line(&format!(
                "            \"{}\" => match serde_json::from_value::<{}>(value) {{ Ok(v) => {{ self.{} = v; true }} Err(_) => false }},",
                n,
                rust_ty(t),
                Self::rust_field(n)
            ));
        }
        self.line("            _ => { self.__rest.insert(name.to_string(), value); true }");
        self.line("        }");
        self.line("    }");
        self.line("    fn has(&self, name: &str) -> bool {");
        let names: Vec<String> = facts.iter().map(|(n, _)| format!("\"{}\"", n)).collect();
        self.line(&format!("        matches!(name, {}) || self.__rest.contains_key(name)", names.join(" | ")));
        self.line("    }");
        self.line("    fn names(&self) -> Vec<String> {");
        self.line(&format!("        let mut out: Vec<String> = vec![{}];", names.iter().map(|n| format!("{}.to_string()", n)).collect::<Vec<_>>().join(", ")));
        self.line("        out.extend(self.__rest.keys().cloned());");
        self.line("        out");
        self.line("    }");
        self.line("    fn copy_fact(&mut self, from: &dyn crow_web::live::Facts, name: &str) {");
        self.line("        let Some(f) = from.as_any().downcast_ref::<CrowFacts>() else {");
        self.line("            if let Some(v) = from.get_json(name) { self.set_json(name, v); }");
        self.line("            return;");
        self.line("        };");
        self.line("        match name {");
        for (n, _) in &facts {
            self.line(&format!("            \"{}\" => self.{f} = f.{f}.clone(),", n, f = Self::rust_field(n)));
        }
        self.line("            _ => match f.__rest.get(name) {");
        self.line("                Some(v) => { self.__rest.insert(name.to_string(), v.clone()); }");
        self.line("                None => { self.__rest.remove(name); }");
        self.line("            },");
        self.line("        }");
        self.line("    }");
        self.line("    fn as_any(&self) -> &dyn std::any::Any { self }");
        self.line("    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }");
        self.line("}");
        self.line("/// The session's facts, read in place.");
        self.line("#[allow(dead_code)]");
        self.line("fn __facts(state: &CrowLiveState) -> &CrowFacts {");
        self.line("    state.facts().as_any().downcast_ref::<CrowFacts>().unwrap_or_else(|| crow_stop(0, \"the session's facts are not this program's (a bug in Crow)\".to_string()))");
        self.line("}");
        self.line("/// The session's facts, written in place (each write followed by `touch` when the value moved).");
        self.line("#[allow(dead_code)]");
        self.line("fn __facts_mut(state: &mut CrowLiveState) -> &mut CrowFacts {");
        self.line("    state.facts_mut().as_any_mut().downcast_mut::<CrowFacts>().unwrap_or_else(|| crow_stop(0, \"the session's facts are not this program's (a bug in Crow)\".to_string()))");
        self.line("}");
    }

    fn emit_synthetic_types(&mut self) {
        if self.needs_find_struct {
            self.needs_crow_runtime = true;
            self.line("");
            self.line("#[derive(Clone, Debug)]");
            self.line("struct CrowFindResult { groups: Vec<String> }");
            // the value a failed find's record holds (a struct's default)
            self.line("#[allow(dead_code)]");
            self.line("impl CrowFindResult { fn crow_default() -> Self { CrowFindResult { groups: Vec::new() } } }");
            self.line("impl crow_runtime::display::CrowDisplay for CrowFindResult {");
            self.line("    fn crow_repr(&self) -> String { use crow_runtime::display::CrowDisplay as _; format!(\"[groups = {}]\", self.groups.crow_repr()) }");
            self.line("}");
            self.line("impl crow_runtime::json::ToJson for CrowFindResult {");
            self.line("    fn to_json(&self) -> crow_runtime::json::Json { use crow_runtime::json::ToJson as _; crow_runtime::json::object(vec![(\"groups\", self.groups.to_json())]) }");
            self.line("}");
        }
        if self.needs_shell_struct {
            // the runtime's own record, which `shell`'s row answers
            self.needs_crow_runtime = true;
            self.line("");
            self.line("#[allow(dead_code)]");
            self.line("type CrowShell = crow_runtime::core::Shell;");
        }
        // (the record `request` answers — made by `sys.reply` in the client
        // written in Crow, and named as a type wherever the answer is held)
        if self.out.contains("CrowAnswer") {
            // a connection's answer, the record `sys.exchange` makes
            self.needs_crow_runtime = true;
            self.line("");
            self.line("#[allow(dead_code)]");
            self.line("type CrowAnswer = crow_runtime::sys::Answer;");
        }
        if self.needs_reply_struct || self.out.contains("CrowReply") {
            // the runtime's own record, which `request`'s rows answer
            self.needs_crow_runtime = true;
            self.line("");
            self.line("#[allow(dead_code)]");
            self.line("type CrowReply = crow_runtime::sources::Reply;");
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
        self.line("fn sign(self) -> CrowDual { CrowDual { real: if self.real > 0.0 { 1.0 } else if self.real < 0.0 { -1.0 } else { 0.0 }, dual: 0.0 } }");
        self.line("fn exp(self) -> CrowDual { let v = self.real.exp(); CrowDual { real: v, dual: self.dual * v } }");
        self.line("fn ln(self) -> CrowDual { CrowDual { real: self.real.ln(), dual: self.dual / self.real } }");
        self.line("fn log2(self) -> CrowDual { CrowDual { real: self.real.log2(), dual: self.dual / (self.real * std::f64::consts::LN_2) } }");
        self.line("fn log10(self) -> CrowDual { CrowDual { real: self.real.log10(), dual: self.dual / (self.real * std::f64::consts::LN_10) } }");
        self.line("fn sin(self) -> CrowDual { CrowDual { real: self.real.sin(), dual: self.dual * self.real.cos() } }");
        self.line("fn cos(self) -> CrowDual { CrowDual { real: self.real.cos(), dual: -self.dual * self.real.sin() } }");
        self.line("fn tan(self) -> CrowDual { let t = self.real.tan(); CrowDual { real: t, dual: self.dual * (1.0 + t * t) } }");
        self.line("fn asin(self) -> CrowDual { CrowDual { real: self.real.asin(), dual: self.dual / (1.0 - self.real * self.real).sqrt() } }");
        self.line("fn acos(self) -> CrowDual { CrowDual { real: self.real.acos(), dual: -self.dual / (1.0 - self.real * self.real).sqrt() } }");
        self.line("fn atan(self) -> CrowDual { CrowDual { real: self.real.atan(), dual: self.dual / (1.0 + self.real * self.real) } }");
        self.line("fn sinh(self) -> CrowDual { CrowDual { real: self.real.sinh(), dual: self.dual * self.real.cosh() } }");
        self.line("fn cosh(self) -> CrowDual { CrowDual { real: self.real.cosh(), dual: self.dual * self.real.sinh() } }");
        self.line("fn tanh(self) -> CrowDual { let t = self.real.tanh(); CrowDual { real: t, dual: self.dual * (1.0 - t * t) } }");
        self.line("fn erf(self) -> CrowDual { CrowDual { real: crow_runtime::special::erf_scalar(self.real), dual: self.dual * std::f64::consts::FRAC_2_SQRT_PI * (-self.real * self.real).exp() } }");
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
        Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => unknown_inside(i),
        _ => false,
    }
}

