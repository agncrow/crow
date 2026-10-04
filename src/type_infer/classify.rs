//! What the program declares and wires: its tables, records, functions,
//! views and derivations, which functions are handlers and which names
//! are facts.

use super::*;

impl TypeContext {
    /// The tables the program makes, wherever it makes them: its CREATE
    /// statements, which describe a query's columns before its database exists.
    pub(super) fn collect_ddl(&mut self, stmts: &[Stmt]) {
        // the tables the program makes, wherever it makes them
        {
            fn in_expr(e: &Expr, out: &mut Vec<String>) {
                if let Expr::Call { callee, args } = e {
                    let is_exec = match callee.as_ref() {
                        Expr::Ident(n) => n == "exec",
                        Expr::FieldAccess { field, .. } => field == "exec",
                        _ => false,
                    };
                    if is_exec {
                        if let Some(Expr::Str(sql)) = args.get(1) {
                            if sql.trim_start().to_ascii_uppercase().starts_with("CREATE") && !out.contains(sql) {
                                out.push(sql.clone());
                            }
                        }
                    }
                    for a in args {
                        in_expr(a, out);
                    }
                }
            }
            fn in_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
                for st in stmts {
                    match st {
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Assign { value: e, .. } => in_expr(e, out),
                        Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } | Stmt::For { body, .. } | Stmt::While { body, .. } => in_stmts(body, out),
                        Stmt::If { then_body, else_body, .. } => {
                            in_stmts(then_body, out);
                            if let Some(eb) = else_body {
                                in_stmts(eb, out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut ddl = std::mem::take(&mut self.ddl);
            in_stmts(stmts, &mut ddl);
            self.ddl = ddl;
        }
    }

    /// The program's declarations: its connections (so a helper scanned
    /// before the part that declares its handle still describes its query
    /// against the right database), records, functions, views,
    /// derivations, modules and resources.
    pub(super) fn collect_declarations(&mut self, stmts: &[Stmt]) {
        // Pass 1: collect struct and function declarations — and every
        // connection, so a helper scanned before the part that declares its
        // handle still describes its query against the right database
        for stmt in stmts {
            if let Stmt::Assign { name, value } = stmt {
                self.note_connection(name, value);
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::StructDecl { name, fields } => {
                    self.structs.insert(name.clone(), StructSig {
                        fields: fields.iter().map(|f| (f.clone(), CrowType::Unknown)).collect(),
                    });
                }
                Stmt::FnDecl { name, params, body, is_view, .. } => {
                    // a builtin written in Crow (a standard module's
                    // function, inlined behind its prefix) takes the
                    // parameters its row states — the row is its contract,
                    // however it is called: bare, qualified, or from
                    // another module's function
                    let contract: Option<Vec<CrowType>> = crate::modules::is_crow_fn(name)
                        .then(|| crate::table::ROWS.iter().find(|r| r.is_crow() && crate::table::crow_fn_name(r) == *name))
                        .flatten()
                        .map(|r| r.params.iter().map(|k| k.crow_type()).collect::<Vec<CrowType>>())
                        .filter(|given| given.len() == params.len());
                    self.functions.insert(name.clone(), FnSig {
                        params: params.iter().enumerate().map(|(i, p)| (p.clone(), contract.as_ref().map_or(CrowType::Unknown, |g| g[i].clone()))).collect(),
                        return_type: CrowType::Unknown,
                        body: body.clone(),
                    });
                    if *is_view {
                        self.views.insert(name.clone());
                    }
                }
                // A derivation is typed like a parameterless function and read
                // like a variable of its return type
                Stmt::Derive { name, body, .. } => {
                    self.functions.insert(name.clone(), FnSig {
                        params: vec![],
                        return_type: CrowType::Unknown,
                        body: body.clone(),
                    });
                    self.derives.insert(name.clone());
                }
                Stmt::Use { path, alias } => {
                    match alias {
                        Some(a) => {
                            self.module_aliases.insert(a.clone(), path.clone());
                        }
                        None => {
                            self.bare_modules.insert(path.clone());
                        }
                    }
                    if path == "math" {
                        if !self.registry.is_loaded("math") {
                            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
                        }
                    }
                    if path == "math.stats" {
                        if !self.registry.is_loaded("math") {
                            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
                        }
                        if !self.registry.is_loaded("math.stats") {
                            self.registry.load(std::sync::Arc::new(crate::modules::stats::StatsModule));
                        }
                    }
                    if path == "math.nn" {
                        if !self.registry.is_loaded("math") {
                            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
                        }
                        if !self.registry.is_loaded("math.nn") {
                            self.registry.load(std::sync::Arc::new(crate::modules::nn::NnModule));
                        }
                    }
                    if path == "io" {
                        if !self.registry.is_loaded("io") {
                            self.registry.load(std::sync::Arc::new(crate::modules::io::IoModule));
                        }
                    }
                    if path == "sys" {
                        if !self.registry.is_loaded("sys") {
                            self.registry.load(std::sync::Arc::new(crate::modules::sys::SysModule));
                        }
                    }
                    if path == "frame" {
                        if !self.registry.is_loaded("frame") {
                            self.registry.load(std::sync::Arc::new(crate::modules::frame::FrameModule));
                        }
                    }
                    if path == "doc" {
                        if !self.registry.is_loaded("doc") {
                            self.registry.load(std::sync::Arc::new(crate::modules::doc::DocModule));
                        }
                    }
                    if path == "chart" {
                        if !self.registry.is_loaded("chart") {
                            self.registry.load(std::sync::Arc::new(crate::modules::chart::ChartModule));
                        }
                    }
                    if path == "db" {
                        if !self.registry.is_loaded("db") {
                            self.registry.load(std::sync::Arc::new(crate::modules::db::DbModule));
                        }
                    }
                }
                Stmt::Resource { kind, count } => {
                    if kind == "cpu" {
                        if let Expr::Int(n) = count {
                            self.cpu_count = Some(*n as usize);
                        }
                    }
                }
                _ => {}
            }
        }
        // a standard module written in Crow, inlined, resolves its names in
        // its own scope: what its `use` lines import, which the program's
        // scope does not see
        crate::modules::load_crow_scopes(&mut self.registry, stmts.iter().filter_map(|s| if let Stmt::FnDecl { name, .. } = s { Some(name.as_str()) } else { None }));
    }

    /// Which functions are handlers, which top-level names are facts, and
    /// the parameters the wiring types (a seek handler's moment, a widget
    /// handler's text).
    pub(super) fn classify_handlers(&mut self, stmts: &[Stmt]) {
        // Event handlers: fns referenced as bare values inside view bodies
        // (click = inc — lowered into attr maps) or in page[] registration
        // args. Generous by design: worst case a fn gains reactive-state
        // visibility it didn't need.
        {
            let mut handlers = std::collections::BTreeSet::new();
            // F-11: click = set_speed[10.0] — the callee of a call-shape in
            // event-attribute position is a handler too.
            fn collect_event_call_handlers(
                e: &Expr,
                fns: &std::collections::BTreeMap<String, crate::type_infer::FnSig>,
                out: &mut std::collections::BTreeSet<String>,
            ) {
                match e {
                    Expr::Map(pairs) => {
                        for (k, v) in pairs {
                            let is_event = matches!(k, Expr::Ident(n) | Expr::Str(n)
                                if matches!(n.as_str(), "click" | "input" | "submit" | "change" | "seek"));
                            if is_event {
                                if let Expr::Call { callee, .. } = v {
                                    if let Expr::Ident(f) = callee.as_ref() {
                                        if fns.contains_key(f) {
                                            out.insert(f.clone());
                                        }
                                    }
                                }
                            }
                            collect_event_call_handlers(v, fns, out);
                        }
                    }
                    Expr::Call { callee, args } => {
                        // chart[..., seek = jump]: the seek target is a handler
                        if let Some(f) = crate::lower::chart_seek_handler(callee, args) {
                            if fns.contains_key(f) {
                                out.insert(f.to_string());
                            }
                        }
                        for a in args { collect_event_call_handlers(a, fns, out); }
                    }
                    Expr::Array(items) => for i in items { collect_event_call_handlers(i, fns, out); },
                    Expr::Named { name, value } => {
                        if matches!(name.as_str(), "click" | "input" | "submit" | "change" | "seek") {
                            if let Expr::Call { callee, .. } = value.as_ref() {
                                if let Expr::Ident(f) = callee.as_ref() {
                                    if fns.contains_key(f) {
                                        out.insert(f.clone());
                                    }
                                }
                            }
                        }
                        collect_event_call_handlers(value, fns, out);
                    }
                    Expr::BinOp { left, right, .. } => {
                        collect_event_call_handlers(left, fns, out);
                        collect_event_call_handlers(right, fns, out);
                    }
                    _ => {}
                }
            }
            fn collect_event_call_stmts(
                stmts: &[Stmt],
                fns: &std::collections::BTreeMap<String, crate::type_infer::FnSig>,
                out: &mut std::collections::BTreeSet<String>,
            ) {
                for st in stmts {
                    match st {
                        Stmt::Assign { value, .. } => collect_event_call_handlers(value, fns, out),
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => collect_event_call_handlers(e, fns, out),
                        Stmt::If { condition, then_body, else_body } => {
                            collect_event_call_handlers(condition, fns, out);
                            collect_event_call_stmts(then_body, fns, out);
                            if let Some(eb) = else_body {
                                collect_event_call_stmts(eb, fns, out);
                            }
                        }
                        Stmt::For { body, .. } | Stmt::While { body, .. } => {
                            collect_event_call_stmts(body, fns, out)
                        }
                        _ => {}
                    }
                }
            }
            for stmt in stmts {
                if let Stmt::FnDecl { body, is_view: true, .. } = stmt {
                    for st in body {
                        Self::collect_fn_value_refs_stmt(st, &self.functions, &mut handlers);
                    }
                    collect_event_call_stmts(body, &self.functions, &mut handlers);
                }
            }
            for stmt in stmts {
                if let Stmt::FnDecl { .. } = stmt {}
                if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
                    if matches!(callee.as_ref(), Expr::Ident(n) if n == "page") {
                        for a in args.iter().skip(1) {
                            if let Expr::Ident(n) = a {
                                if self.functions.contains_key(n) {
                                    handlers.insert(n.clone());
                                }
                            }
                        }
                    }
                }
            }
            // click[a, f] inside a test block names f as a handler, and
            // click[a, f[x]] too, with the arguments it is given: the harness
            // drives handlers, so they get the reactive exemption, and a
            // compiled program dispatches them.
            fn collect_click_handlers(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
                fn walk(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
                    if let Expr::Call { callee, args } = e {
                        if matches!(callee.as_ref(), Expr::Ident(n) if n == "click") && args.len() == 2 {
                            match &args[1] {
                                Expr::Ident(h) => {
                                    out.insert(h.clone());
                                }
                                Expr::Call { callee: h, .. } => {
                                    if let Expr::Ident(h) = h.as_ref() {
                                        out.insert(h.clone());
                                    }
                                }
                                _ => {}
                            }
                        }
                        for a in args {
                            walk(a, out);
                        }
                    }
                }
                for st in stmts {
                    match st {
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk(e, out),
                        Stmt::Assign { value, .. } => walk(value, out),
                        Stmt::TestBlock { body, .. } => collect_click_handlers(body, out),
                        Stmt::If { then_body, else_body, .. } => {
                            collect_click_handlers(then_body, out);
                            if let Some(eb) = else_body {
                                collect_click_handlers(eb, out);
                            }
                        }
                        Stmt::For { body, .. } | Stmt::While { body, .. } => {
                            collect_click_handlers(body, out)
                        }
                        _ => {}
                    }
                }
            }
            {
                let mut clicked = std::collections::BTreeSet::new();
                collect_click_handlers(stmts, &mut clicked);
                for h in clicked {
                    if self.functions.contains_key(&h) {
                        handlers.insert(h);
                    }
                }
            }
            // the module's reactive state: every top-level assignment
            let module_vars: std::collections::BTreeSet<String> = stmts.iter().filter_map(|s| if let Stmt::Assign { name, .. } = s { Some(name.clone()) } else { None }).filter(|n| !crate::plain_entry_name(n)).collect();
            self.facts = module_vars.clone();
            // a function that assigns reactive state — or tells the reader
            // something with notice[], which the session keeps, or sends
            // mail, which waits for the session's commit — acts for the
            // session and joins the handler class. A function's last line
            // is written as its return.
            fn assigns_any(stmts: &[Stmt], names: &std::collections::BTreeSet<String>) -> bool {
                stmts.iter().any(|st| match st {
                    Stmt::ExprStmt(Expr::Call { callee, .. }) | Stmt::Return(Some(Expr::Call { callee, .. })) if matches!(callee.as_ref(), Expr::Ident(n) if SESSION_EFFECTS.contains(&n.as_str())) => true,
                    Stmt::Assign { name, .. } => names.contains(name),
                    Stmt::MultiAssign { names: ns, .. } => ns.iter().any(|n| names.contains(n)),
                    Stmt::If { then_body, else_body, .. } => assigns_any(then_body, names) || else_body.as_ref().map_or(false, |eb| assigns_any(eb, names)),
                    Stmt::For { body, .. } | Stmt::While { body, .. } => assigns_any(body, names),
                    _ => false,
                })
            }

            // the reactive state a function can write: a part's function any
            // part's — its own, or one it uses (`session.picked = symbol` in
            // scan) — which carries that part's prefix; never the entry's,
            // which it cannot see (a local `ch` in a part is not the entry's
            // connection: the expansion renamed it)
            fn own_state(f: &str, module_vars: &std::collections::BTreeSet<String>) -> std::collections::BTreeSet<String> {
                match crate::part_prefix_of_declaration(f) {
                    Some(_) => module_vars.iter().filter(|m| crate::part_prefix_of_declaration(m).is_some()).cloned().collect(),
                    None => module_vars.clone(),
                }
            }

            // a function that writes its part's state or reaches a session's
            // effect — itself, or through the functions it calls — acts for
            // the handler that calls it, so it joins with them (`clock.jump`
            // moves the head through `moved_to`, which writes it)
            fn effects_any(stmts: &[Stmt]) -> bool {
                assigns_any(stmts, &std::collections::BTreeSet::new())
            }
            let mut reaches: std::collections::BTreeSet<String> = self.functions.iter().filter(|(n, f)| effects_any(&f.body) || assigns_any(&f.body, &own_state(n, &module_vars))).map(|(n, _)| n.clone()).collect();
            loop {
                let before = reaches.len();
                for (n, f) in &self.functions {
                    if !reaches.contains(n) {
                        let mut called = std::collections::BTreeSet::new();
                        Self::collect_called_fns_stmts(&f.body, &mut called);
                        if called.iter().any(|c| reaches.contains(c)) {
                            reaches.insert(n.clone());
                        }
                    }
                }
                if reaches.len() == before {
                    break;
                }
            }

            // later[f] targets scheduled from views/handlers are deferred
            // handler halves: they read/write reactive state off the loop,
            // so they join the handler class (validator exemption, interp
            // exempt frames, the Rust backend's state signature). Fixpoint covers
            // later-chains (handler → later f → later g).
            loop {
                let mut added = false;
                let roots: Vec<String> = handlers.iter().cloned().collect();
                let view_fns: Vec<String> = self
                    .functions
                    .iter()
                    .filter(|(n, _)| self.views.contains(*n))
                    .map(|(n, _)| n.clone())
                    .collect();
                for name in roots.iter().chain(view_fns.iter()) {
                    if let Some(sig) = self.functions.get(name) {
                        let mut targets = std::collections::BTreeSet::new();
                        Self::collect_later_targets_stmts(&sig.body, &mut targets);
                        // and, for a handler, every function it calls that writes
                        // module state: a helper moving state on the handler's
                        // behalf. A pure helper keeps its own scope.
                        if !self.views.contains(name) {
                            let mut called = std::collections::BTreeSet::new();
                            Self::collect_called_fns_stmts(&sig.body, &mut called);
                            for c in called {
                                if reaches.contains(&c) {
                                    targets.insert(c);
                                }
                            }
                        }
                        for t in targets {
                            if self.functions.contains_key(&t) && !self.views.contains(&t) && handlers.insert(t) {
                                added = true;
                            }
                        }
                    }
                }
                if !added {
                    break;
                }
            }
            // a derivation referenced in a view is read, not wired
            let handlers: std::collections::BTreeSet<String> = handlers.into_iter().filter(|h| !self.derives.contains(h)).collect();
            self.view_handlers = handlers;

            // A seek handler receives the clicked bucket's moment: its
            // first parameter is a float whether or not a call site says so.
            // A widget's handler receives what its module sends, as a field's
            // handler receives its text: a parameter nothing else types is a
            // string. A function grad differentiates is a function of floats
            // (`grad[f][3]` too, whose call the scan does not read).
            fn seek_expr(e: &Expr, out: &mut Vec<String>, widgets: &mut Vec<String>) {
                match e {
                    Expr::Call { callee, args } => {
                        if let Some(f) = crate::lower::chart_seek_handler(callee, args) {
                            out.push(f.to_string());
                        }
                        if matches!(callee.as_ref(), Expr::Ident(n) if n == "grad") {
                            if let Some(Expr::Ident(f)) = args.first() {
                                out.push(format!("grad:{}", f));
                            }
                        }
                        if matches!(callee.as_ref(), Expr::Ident(n) if n == "widget") {
                            for a in args {
                                let Expr::Map(pairs) = a else { continue };
                                for (k, v) in pairs {
                                    if matches!(k, Expr::Ident(s) | Expr::Str(s) if s == "on") {
                                        for (_, h) in crate::lower::map_literal_pairs(v).unwrap_or_default() {
                                            if let Expr::Ident(f) = h {
                                                widgets.push(f);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        seek_expr(callee, out, widgets);
                        for a in args { seek_expr(a, out, widgets); }
                    }
                    Expr::BinOp { left, right, .. } => { seek_expr(left, out, widgets); seek_expr(right, out, widgets); }
                    Expr::UnaryOp { expr, .. } => seek_expr(expr, out, widgets),
                    Expr::FieldAccess { object, .. } => seek_expr(object, out, widgets),
                    Expr::Array(items) => for i in items { seek_expr(i, out, widgets); },
                    Expr::Map(pairs) => for (k, v) in pairs { seek_expr(k, out, widgets); seek_expr(v, out, widgets); },
                    Expr::Named { value, .. } => seek_expr(value, out, widgets),
                    Expr::ForExpr { iter, body, .. } => { seek_expr(iter, out, widgets); seek_stmts(body, out, widgets); }
                    _ => {}
                }
            }
            fn seek_stmts(stmts: &[Stmt], out: &mut Vec<String>, widgets: &mut Vec<String>) {
                for st in stmts {
                    match st {
                        Stmt::Assign { value, .. } => seek_expr(value, out, widgets),
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => seek_expr(e, out, widgets),
                        Stmt::If { condition, then_body, else_body } => {
                            seek_expr(condition, out, widgets);
                            seek_stmts(then_body, out, widgets);
                            if let Some(eb) = else_body { seek_stmts(eb, out, widgets); }
                        }
                        Stmt::For { body, .. } | Stmt::While { body, .. } => seek_stmts(body, out, widgets),
                        Stmt::FnDecl { body, .. } => seek_stmts(body, out, widgets),
                        _ => {}
                    }
                }
            }
            let mut seeks = Vec::new();
            let mut widgets = Vec::new();
            seek_stmts(stmts, &mut seeks, &mut widgets);
            for f in seeks {
                if let Some(g) = f.strip_prefix("grad:") {
                    // a function grad differentiates is typed by the calls of
                    // its gradient (the lowering's Index path): of numbers, an
                    // int counted as its float, or of tensors; until one is
                    // seen its parameters are not known (`grad_forced`)
                    if self.functions.contains_key(g) {
                        self.grad_forced.insert(g.to_string());
                    }
                } else if let Some(sig) = self.functions.get_mut(&f) {
                    if let Some(p) = sig.params.first_mut() {
                        p.1 = CrowType::Float;
                    }
                }
            }
            for f in widgets {
                if let Some(sig) = self.functions.get_mut(&f) {
                    for p in sig.params.iter_mut().filter(|p| matches!(p.1, CrowType::Unknown)) {
                        p.1 = CrowType::Str;
                    }
                }
            }
        }

    }

    /// Scan a statement for call sites to infer parameter types.
    /// Fn names appearing as later[f]/running[f] targets.
    pub(super) fn collect_later_targets_stmts(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
        fn walk(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
            if let Expr::Call { callee, args } = e {
                if let Expr::Ident(name) = callee.as_ref() {
                    if (name == "later" || name == "running") && !args.is_empty() {
                        if let Expr::Ident(target) = &args[0] {
                            out.insert(target.clone());
                        }
                    }
                }
                walk(callee, out);
                for a in args {
                    walk(a, out);
                }
            } else if let Expr::BinOp { left, right, .. } = e {
                walk(left, out);
                walk(right, out);
            } else if let Expr::UnaryOp { expr, .. } = e {
                walk(expr, out);
            } else if let Expr::FieldAccess { object, .. } = e {
                walk(object, out);
            } else if let Expr::Array(items) = e {
                for i in items {
                    walk(i, out);
                }
            } else if let Expr::Named { value, .. } = e {
                walk(value, out);
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => walk(value, out),
                Stmt::ExprStmt(e) => walk(e, out),
                Stmt::Return(Some(e)) => walk(e, out),
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    walk(condition, out);
                    Self::collect_later_targets_stmts(then_body, out);
                    if let Some(eb) = else_body {
                        Self::collect_later_targets_stmts(eb, out);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    Self::collect_later_targets_stmts(body, out);
                }
                _ => {}
            }
        }
    }

    /// Collect bare references to known fn names (Ident in value position —
    /// not as a call's callee) inside view bodies: these are event handlers.
    pub(super) fn collect_fn_value_refs_stmt(
        stmt: &Stmt,
        fns: &BTreeMap<String, FnSig>,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        fn walk_expr(e: &Expr, fns: &BTreeMap<String, FnSig>, out: &mut std::collections::BTreeSet<String>) {
            match e {
                Expr::Ident(n) => {
                    if fns.contains_key(n) {
                        out.insert(n.clone());
                    }
                }
                Expr::Call { callee, args } => {
                    // The callee name itself is a call, not a value ref —
                    // but nested exprs inside it still count
                    if !matches!(callee.as_ref(), Expr::Ident(_)) {
                        walk_expr(callee, fns, out);
                    }
                    for a in args {
                        walk_expr(a, fns, out);
                    }
                }
                Expr::Named { value, .. } => walk_expr(value, fns, out),
                Expr::BinOp { left, right, .. } => {
                    walk_expr(left, fns, out);
                    walk_expr(right, fns, out);
                }
                Expr::UnaryOp { expr, .. } => walk_expr(expr, fns, out),
                Expr::Array(items) => {
                    for i in items {
                        walk_expr(i, fns, out);
                    }
                }
                Expr::Map(pairs) => {
                    for (_, v) in pairs {
                        walk_expr(v, fns, out);
                    }
                }
                Expr::FieldAccess { object, .. } => walk_expr(object, fns, out),
                Expr::ForExpr { iter, body, .. } => {
                    walk_expr(iter, fns, out);
                    for st in body {
                        TypeContext::collect_fn_value_refs_stmt(st, fns, out);
                    }
                }
                _ => {}
            }
        }
        match stmt {
            Stmt::Assign { value, .. } => walk_expr(value, fns, out),
            Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => walk_expr(e, fns, out),
            Stmt::If { condition, then_body, else_body } => {
                walk_expr(condition, fns, out);
                for st in then_body {
                    Self::collect_fn_value_refs_stmt(st, fns, out);
                }
                if let Some(eb) = else_body {
                    for st in eb {
                        Self::collect_fn_value_refs_stmt(st, fns, out);
                    }
                }
            }
            Stmt::For { iter, body, .. } => {
                walk_expr(iter, fns, out);
                for st in body {
                    Self::collect_fn_value_refs_stmt(st, fns, out);
                }
            }
            Stmt::While { condition, body } => {
                walk_expr(condition, fns, out);
                for st in body {
                    Self::collect_fn_value_refs_stmt(st, fns, out);
                }
            }
            _ => {}
        }
    }

    /// Names of the functions a body calls or references.
    pub(super) fn collect_called_fns_expr(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() { out.insert(n.clone()); } else { Self::collect_called_fns_expr(callee, out); }
                for a in args { Self::collect_called_fns_expr(a, out); }
            }
            Expr::Ident(n) => { out.insert(n.clone()); }
            Expr::Named { value, .. } => Self::collect_called_fns_expr(value, out),
            Expr::BinOp { left, right, .. } => { Self::collect_called_fns_expr(left, out); Self::collect_called_fns_expr(right, out); }
            Expr::UnaryOp { expr, .. } => Self::collect_called_fns_expr(expr, out),
            Expr::Array(items) => for i in items { Self::collect_called_fns_expr(i, out); },
            Expr::Map(pairs) => for (k, v) in pairs { Self::collect_called_fns_expr(k, out); Self::collect_called_fns_expr(v, out); },
            Expr::FieldAccess { object, .. } => Self::collect_called_fns_expr(object, out),
            Expr::ForExpr { iter, body, .. } => { Self::collect_called_fns_expr(iter, out); Self::collect_called_fns_stmts(body, out); }
            _ => {}
        }
    }

    pub(super) fn collect_called_fns_stmts(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } => Self::collect_called_fns_expr(value, out),
                Stmt::MultiAssign { values, .. } => for v in values { Self::collect_called_fns_expr(v, out); },
                Stmt::ExprStmt(e) => Self::collect_called_fns_expr(e, out),
                Stmt::Return(Some(e)) => Self::collect_called_fns_expr(e, out),
                Stmt::If { condition, then_body, else_body } => {
                    Self::collect_called_fns_expr(condition, out);
                    Self::collect_called_fns_stmts(then_body, out);
                    if let Some(eb) = else_body { Self::collect_called_fns_stmts(eb, out); }
                }
                Stmt::For { iter, body, .. } => { Self::collect_called_fns_expr(iter, out); Self::collect_called_fns_stmts(body, out); }
                Stmt::While { condition, body } => { Self::collect_called_fns_expr(condition, out); Self::collect_called_fns_stmts(body, out); }
                Stmt::FieldAssign { value, .. } | Stmt::IndexAssign { value, .. } => Self::collect_called_fns_expr(value, out),
                _ => {}
            }
        }
    }
}
