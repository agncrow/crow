/// Crow -> Rust transpiler: emits clean Rust source from typed AST.

use crate::ast::*;
use crate::parallel::group_into_waves;
use crate::type_infer::{CrowType, TypeContext};

pub struct Transpiler {
    ctx: TypeContext,
    indent: usize,
    output: String,
    /// Track which variables have been declared with `let`
    declared: std::collections::HashSet<String>,
    /// Variables that are reassigned after initial declaration (need `let mut`)
    mutated_vars: std::collections::HashSet<String>,
    /// Current cpu thread count (set by cpu[n] statements)
    cpu_count: Option<usize>,
    /// Whether the transpiled code needs rayon for parallel iteration
    pub needs_rayon: bool,
    /// Whether the transpiled code needs serde_json for JSON parsing/encoding
    pub needs_serde_json: bool,
    /// Whether the transpiled code needs crow_runtime for CrowTensor
    pub needs_crow_runtime: bool,
    /// Whether the transpiled code uses distributed computing (nodes[])
    pub needs_distributed: bool,
    /// later[]/running[] used outside live handlers — wait for stragglers at exit
    needs_later: bool,
    /// Map of function name -> set of param names that should be passed by reference
    by_ref_params: std::collections::BTreeMap<String, std::collections::HashSet<String>>,
    /// By-ref params in the current function being emitted
    current_ref_params: std::collections::HashSet<String>,
    /// Module names from `use` imports (for resolving module.field → module_field)
    modules: std::collections::HashSet<String>,
    /// Current source line number (from Stmt::Line markers)
    current_line: usize,
    /// Flag: when true, CrowResult args are replaced with _prop_N.value.clone()
    unwrap_result_args: bool,
    /// Indices of CrowResult args being unwrapped (used with unwrap_result_args)
    propagation_indices: Option<Vec<usize>>,
    /// Return type of the current function being emitted (for wrapping plain returns in CrowResult)
    current_fn_return_type: Option<CrowType>,
    /// Name of the current function being emitted (for trace entries)
    current_function_name: String,
    /// Source file name (for trace entries)
    source_file: String,
    /// Counter for generating unique variable names in BinOp propagation
    binop_counter: usize,
    /// Whether the transpiled code uses crow_db (use db)
    pub needs_crow_db: bool,
    /// Which crow_db features are needed (e.g. "sqlite", "postgres", "clickhouse")
    pub db_features: std::collections::BTreeSet<String>,
    /// Whether the transpiled code uses crow_web (any web.* module)
    pub needs_crow_web: bool,
    /// Which crow_web features are needed (e.g. "auth", "session", "db")
    pub web_features: std::collections::BTreeSet<String>,
    /// Whether the transpiled code uses regex
    pub needs_regex: bool,
    needs_find_struct: bool,
    /// moment[]/parse_time[] used — emit the Result<f64,String> → CrowResult bridge
    needs_time_result: bool,
    /// Top-level non-serializable handles (db connections) in ui programs:
    /// emitted as static OnceLocks, not reactive state
    static_handle_vars: std::collections::BTreeSet<String>,
    /// Module pinned by a prefixed call (frame.load) while its rebuilt bare
    /// form is emitted — resolves overlapping registry exports
    pinned_module: Option<String>,
    needs_shell_struct: bool,
    needs_elapsed_struct: bool,
    /// Whether the transpiled code uses sha2/md5 hashing
    pub needs_sha2: bool,
    /// Whether the transpiled code uses chrono (date/time)
    pub needs_chrono: bool,
    /// Whether the transpiled code uses grad[] (needs CrowDual struct)
    pub needs_grad: bool,
    /// Set of function names that are passed to grad[] and need dual-mode versions
    grad_functions: std::collections::HashSet<String>,
    /// Flag: when true, we are emitting a dual-mode version of a function
    emitting_dual: bool,
    /// Module registry for generic module dispatch
    registry: crate::modules::ModuleRegistry,
    /// Function names that are used as web route handlers (need CrowRequest param + CrowResponse return)
    web_handlers: std::collections::HashSet<String>,
    /// Whether the transpiled code uses `use ui` (live view element functions)
    pub needs_ui: bool,
    /// Function names used as live[]/page[] render callbacks (need &CrowLiveState param, CrowVNode return)
    live_render_fns: std::collections::HashSet<String>,
    /// Function names used as live[]/page[] handle callbacks (need &mut CrowLiveState, &str, Option<&str> params)
    live_handle_fns: std::collections::HashSet<String>,
    /// Whether the code uses page[] (implicit app pattern — no user-visible app variable)
    has_page_routes: bool,
    has_noapp_web: bool,
    /// Reactive variables: top-level assignments in UI files that become state-backed.
    /// Maps var_name -> (type, default_value_as_rust_string)
    reactive_vars: std::collections::BTreeMap<String, (CrowType, String)>,
    /// Functions used as event handlers via function references (click = fn_name)
    reactive_event_handlers: std::collections::HashSet<String>,
    /// Render functions that use reactive variables (0-param render fns in page[])
    reactive_render_fns: std::collections::HashSet<String>,
    /// Data source variables from poll[]/fetch[] assignments (name → type is Frame)
    data_source_vars: std::collections::HashSet<String>,
}

impl Transpiler {
    pub fn new(ctx: TypeContext) -> Self {
        Transpiler {
            ctx,
            indent: 0,
            output: String::new(),
            declared: std::collections::HashSet::new(),
            mutated_vars: std::collections::HashSet::new(),
            cpu_count: None,
            needs_rayon: false,
            needs_serde_json: false,
            needs_crow_runtime: false,
            needs_distributed: false,
            needs_later: false,
            needs_time_result: false,
            by_ref_params: std::collections::BTreeMap::new(),
            current_ref_params: std::collections::HashSet::new(),
            modules: std::collections::HashSet::new(),
            current_line: 0,
            unwrap_result_args: false,
            propagation_indices: None,
            current_fn_return_type: None,
            current_function_name: String::new(),
            source_file: String::new(),
            binop_counter: 0,
            needs_regex: false,
            needs_find_struct: false,
            pinned_module: None,
            static_handle_vars: std::collections::BTreeSet::new(),
            needs_shell_struct: false,
            needs_elapsed_struct: false,
            needs_sha2: false,
            needs_chrono: false,
            needs_crow_db: false,
            db_features: std::collections::BTreeSet::new(),
            needs_crow_web: false,
            web_features: std::collections::BTreeSet::new(),
            needs_grad: false,
            grad_functions: std::collections::HashSet::new(),
            emitting_dual: false,
            registry: crate::modules::ModuleRegistry::new(),
            web_handlers: std::collections::HashSet::new(),
            needs_ui: false,
            live_render_fns: std::collections::HashSet::new(),
            live_handle_fns: std::collections::HashSet::new(),
            has_page_routes: false,
            has_noapp_web: false,
            reactive_vars: std::collections::BTreeMap::new(),
            reactive_event_handlers: std::collections::HashSet::new(),
            reactive_render_fns: std::collections::HashSet::new(),
            data_source_vars: std::collections::HashSet::new(),
        }
    }

    pub fn set_cpu_count(&mut self, n: usize) {
        self.cpu_count = Some(n);
    }

    pub fn set_source_file(&mut self, file: &str) {
        self.source_file = file.to_string();
    }

    /// Generate a Rust format string for a trace entry.
    /// Inside a function: "file_stem.func_name:N"
    /// At top level: "file_stem:N"
    fn trace_format_expr(&self) -> String {
        let file_stem = if self.source_file.is_empty() {
            "unknown"
        } else {
            self.source_file.strip_suffix(".crow").unwrap_or(&self.source_file)
        };
        if self.current_function_name.is_empty() {
            format!("format!(\"{}:{{}}\", {})", file_stem, self.current_line)
        } else {
            format!("format!(\"{}.{}:{{}}\", {})", file_stem, self.current_function_name, self.current_line)
        }
    }

    /// Map Crow function names to valid Rust names.
    /// User-defined `main` becomes `crow_main` to avoid clashing with Rust's entry point.
    fn rust_fn_name(name: &str) -> String {
        if name == "main" {
            "crow_main".to_string()
        } else {
            name.to_string()
        }
    }

    /// Check if an array of statements contains any parallel waves (2+ independent stmts).
    fn has_parallel_waves(stmts: &[Stmt]) -> bool {
        let waves = group_into_waves(stmts);
        waves.iter().any(|w| w.len() > 1)
    }

    /// Check if a statement is a cpu[n] call. Returns the count if so.
    fn extract_cpu_call(stmt: &Stmt) -> Option<i64> {
        if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "cpu" && args.len() == 1 {
                    if let Expr::Int(n) = &args[0] {
                        return Some(*n);
                    }
                }
            }
        }
        // Also handle Stmt::Resource (in case parser ever emits it)
        if let Stmt::Resource { kind, count } = stmt {
            if kind == "cpu" {
                if let Expr::Int(n) = count {
                    return Some(*n);
                }
            }
        }
        None
    }

    /// Check if a statement is a gpu[] or gpu[n] call.
    fn is_gpu_call(stmt: &Stmt) -> bool {
        if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "gpu" && args.len() <= 1 {
                    return true;
                }
            }
        }
        if let Stmt::Resource { kind, .. } = stmt {
            return kind == "gpu";
        }
        false
    }

    /// Check if a statement is a nodes[n] call. Returns the count if so.
    fn extract_nodes_call(stmt: &Stmt) -> Option<i64> {
        if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "nodes" && args.len() == 1 {
                    if let Expr::Int(n) = &args[0] {
                        return Some(*n);
                    }
                }
            }
        }
        if let Stmt::Resource { kind, count } = stmt {
            if kind == "nodes" {
                if let Expr::Int(n) = count {
                    return Some(*n);
                }
            }
        }
        None
    }

    /// Recursively scan expressions for grad[func_name] calls and collect function names.
    fn collect_grad_functions(expr: &Expr, out: &mut std::collections::HashSet<String>) {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "grad" && args.len() == 1 {
                        if let Expr::Ident(fn_name) = &args[0] {
                            out.insert(fn_name.clone());
                        }
                    }
                }
                // Recurse into callee and args
                Self::collect_grad_functions(callee, out);
                for a in args {
                    Self::collect_grad_functions(a, out);
                }
            }
            Expr::BinOp { left, right, .. } => {
                Self::collect_grad_functions(left, out);
                Self::collect_grad_functions(right, out);
            }
            Expr::UnaryOp { expr, .. } => {
                Self::collect_grad_functions(expr, out);
            }
            Expr::Array(elems) => {
                for e in elems {
                    Self::collect_grad_functions(e, out);
                }
            }
            _ => {}
        }
    }

    /// Scan all statements for grad[func_name] calls.
    fn scan_grad_functions(stmts: &[Stmt]) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => Self::collect_grad_functions(value, &mut out),
                Stmt::ExprStmt(e) => Self::collect_grad_functions(e, &mut out),
                Stmt::If { condition, then_body, else_body, .. } => {
                    Self::collect_grad_functions(condition, &mut out);
                    let sub = Self::scan_grad_functions(then_body);
                    out.extend(sub);
                    if let Some(eb) = else_body {
                        let sub = Self::scan_grad_functions(eb);
                        out.extend(sub);
                    }
                }
                Stmt::While { condition, body, .. } => {
                    Self::collect_grad_functions(condition, &mut out);
                    let sub = Self::scan_grad_functions(body);
                    out.extend(sub);
                }
                Stmt::For { body, .. } => {
                    let sub = Self::scan_grad_functions(body);
                    out.extend(sub);
                }
                Stmt::FnDecl { body, .. } => {
                    let sub = Self::scan_grad_functions(body);
                    out.extend(sub);
                }
                _ => {}
            }
        }
        out
    }

    /// Scan statements for route[] calls and collect the handler function names.
    fn scan_web_handlers(stmts: &[Stmt], out: &mut std::collections::HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => Self::collect_web_handlers_expr(value, out),
                Stmt::ExprStmt(e) => Self::collect_web_handlers_expr(e, out),
                Stmt::If { condition, then_body, else_body, .. } => {
                    Self::collect_web_handlers_expr(condition, out);
                    Self::scan_web_handlers(then_body, out);
                    if let Some(eb) = else_body { Self::scan_web_handlers(eb, out); }
                }
                Stmt::While { condition, body, .. } => {
                    Self::collect_web_handlers_expr(condition, out);
                    Self::scan_web_handlers(body, out);
                }
                Stmt::For { body, .. } => Self::scan_web_handlers(body, out),
                Stmt::FnDecl { body, .. } => Self::scan_web_handlers(body, out),
                _ => {}
            }
        }
    }

    /// Collect handler function names from route[app, path, handler] call expressions.
    fn collect_web_handlers_expr(expr: &Expr, out: &mut std::collections::HashSet<String>) {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "route" && args.len() >= 2 {
                        // Handler is the last arg — Ident (named fn) or Lambda
                        if let Some(Expr::Ident(handler_name)) = args.last() {
                            out.insert(handler_name.clone());
                        }
                    }
                }
                // Recurse into subexpressions
                Self::collect_web_handlers_expr(callee, out);
                for a in args { Self::collect_web_handlers_expr(a, out); }
            }
            Expr::BinOp { left, right, .. } => {
                Self::collect_web_handlers_expr(left, out);
                Self::collect_web_handlers_expr(right, out);
            }
            Expr::FieldAccess { object, .. } => Self::collect_web_handlers_expr(object, out),
            _ => {}
        }
    }

    /// Fn names referenced as later[f] / running[f] targets in a body.
    fn collect_later_targets(stmts: &[Stmt], out: &mut std::collections::HashSet<String>) {
        fn walk_expr(e: &Expr, out: &mut std::collections::HashSet<String>) {
            if let Expr::Call { callee, args } = e {
                if let Expr::Ident(name) = callee.as_ref() {
                    if (name == "later" || name == "running") && !args.is_empty() {
                        if let Expr::Ident(target) = &args[0] {
                            out.insert(target.clone());
                        }
                    }
                }
                walk_expr(callee, out);
                for a in args {
                    walk_expr(a, out);
                }
            } else if let Expr::BinOp { left, right, .. } = e {
                walk_expr(left, out);
                walk_expr(right, out);
            } else if let Expr::UnaryOp { expr, .. } = e {
                walk_expr(expr, out);
            } else if let Expr::FieldAccess { object, .. } = e {
                walk_expr(object, out);
            } else if let Expr::Array(items) = e {
                for i in items {
                    walk_expr(i, out);
                }
            } else if let Expr::Named { value, .. } = e {
                walk_expr(value, out);
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => walk_expr(value, out),
                Stmt::ExprStmt(e) => walk_expr(e, out),
                Stmt::Return(Some(e)) => walk_expr(e, out),
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    walk_expr(condition, out);
                    Self::collect_later_targets(then_body, out);
                    if let Some(eb) = else_body {
                        Self::collect_later_targets(eb, out);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    Self::collect_later_targets(body, out);
                }
                _ => {}
            }
        }
    }

    /// Scan statements for page[path, render, handle] calls
    /// and collect render/handle function names for signature overriding.
    fn scan_live_callbacks(stmts: &[Stmt], render_fns: &mut std::collections::HashSet<String>, handle_fns: &mut std::collections::HashSet<String>, has_page: &mut bool) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => Self::collect_live_callbacks_expr(value, render_fns, handle_fns, has_page),
                Stmt::ExprStmt(e) => Self::collect_live_callbacks_expr(e, render_fns, handle_fns, has_page),
                Stmt::If { condition, then_body, else_body, .. } => {
                    Self::collect_live_callbacks_expr(condition, render_fns, handle_fns, has_page);
                    Self::scan_live_callbacks(then_body, render_fns, handle_fns, has_page);
                    if let Some(eb) = else_body { Self::scan_live_callbacks(eb, render_fns, handle_fns, has_page); }
                }
                Stmt::While { condition, body, .. } => {
                    Self::collect_live_callbacks_expr(condition, render_fns, handle_fns, has_page);
                    Self::scan_live_callbacks(body, render_fns, handle_fns, has_page);
                }
                Stmt::For { body, .. } => Self::scan_live_callbacks(body, render_fns, handle_fns, has_page),
                Stmt::FnDecl { body, .. } => Self::scan_live_callbacks(body, render_fns, handle_fns, has_page),
                _ => {}
            }
        }
    }

    /// Collect render/handle function names from page[path, render, handle] call expressions.
    fn collect_live_callbacks_expr(expr: &Expr, render_fns: &mut std::collections::HashSet<String>, handle_fns: &mut std::collections::HashSet<String>, has_page: &mut bool) {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "page" && (args.len() == 2 || args.len() == 3) {
                        // page[path, render] or page[path, render, handle]
                        *has_page = true;
                        if let Expr::Ident(render_name) = &args[1] {
                            render_fns.insert(render_name.clone());
                        }
                        if args.len() == 3 {
                            if let Expr::Ident(handle_name) = &args[2] {
                                handle_fns.insert(handle_name.clone());
                            }
                        }
                    }
                }
                Self::collect_live_callbacks_expr(callee, render_fns, handle_fns, has_page);
                for a in args { Self::collect_live_callbacks_expr(a, render_fns, handle_fns, has_page); }
            }
            Expr::BinOp { left, right, .. } => {
                Self::collect_live_callbacks_expr(left, render_fns, handle_fns, has_page);
                Self::collect_live_callbacks_expr(right, render_fns, handle_fns, has_page);
            }
            Expr::FieldAccess { object, .. } => Self::collect_live_callbacks_expr(object, render_fns, handle_fns, has_page),
            _ => {}
        }
    }

    /// Scan an expression tree for event handler function references.
    /// Looks for Map entries like `click = fn_name` where fn_name is a known function.
    fn collect_event_handler_refs(expr: &Expr, known_fns: &std::collections::HashSet<String>, handlers: &mut std::collections::HashSet<String>) {
        match expr {
            Expr::Call { callee, args } => {
                // Check first arg for Map with event keys
                if let Some(first_arg) = args.first() {
                    if let Expr::Map(pairs) = first_arg {
                        for (k, v) in pairs {
                            let key_str = match k {
                                Expr::Ident(s) | Expr::Str(s) => s.clone(),
                                _ => continue,
                            };
                            if matches!(key_str.as_str(), "click" | "input" | "submit") {
                                if let Expr::Ident(fn_name) = v {
                                    if known_fns.contains(fn_name) {
                                        handlers.insert(fn_name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                Self::collect_event_handler_refs(callee, known_fns, handlers);
                for a in args { Self::collect_event_handler_refs(a, known_fns, handlers); }
            }
            Expr::BinOp { left, right, .. } => {
                Self::collect_event_handler_refs(left, known_fns, handlers);
                Self::collect_event_handler_refs(right, known_fns, handlers);
            }
            Expr::FieldAccess { object, .. } => Self::collect_event_handler_refs(object, known_fns, handlers),
            Expr::Array(items) => {
                for item in items { Self::collect_event_handler_refs(item, known_fns, handlers); }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    Self::collect_event_handler_refs(k, known_fns, handlers);
                    Self::collect_event_handler_refs(v, known_fns, handlers);
                }
            }
            _ => {}
        }
    }

    /// Scan statements for event handler references.
    fn collect_event_handler_refs_stmts(stmts: &[Stmt], known_fns: &std::collections::HashSet<String>, handlers: &mut std::collections::HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => Self::collect_event_handler_refs(value, known_fns, handlers),
                Stmt::ExprStmt(e) => Self::collect_event_handler_refs(e, known_fns, handlers),
                Stmt::Return(Some(e)) => Self::collect_event_handler_refs(e, known_fns, handlers),
                Stmt::If { condition, then_body, else_body, .. } => {
                    Self::collect_event_handler_refs(condition, known_fns, handlers);
                    Self::collect_event_handler_refs_stmts(then_body, known_fns, handlers);
                    if let Some(eb) = else_body { Self::collect_event_handler_refs_stmts(eb, known_fns, handlers); }
                }
                Stmt::For { body, .. } => Self::collect_event_handler_refs_stmts(body, known_fns, handlers),
                _ => {}
            }
        }
    }

    /// Compute which function parameters should be passed by reference.
    /// A struct param is by-ref if: the struct is non-Copy AND the param is not mutated in the body.
    fn compute_by_ref_params(&mut self) {
        let fn_names: Vec<String> = self.ctx.functions.keys().cloned().collect();
        for name in &fn_names {
            let sig = self.ctx.functions.get(name).unwrap().clone();
            let mut ref_params = std::collections::HashSet::new();
            for (pname, ptype) in &sig.params {
                if let CrowType::Struct(sname) = ptype {
                    if !self.is_struct_copy(sname) && !Self::is_param_mutated(pname, &sig.body) {
                        ref_params.insert(pname.clone());
                    }
                }
            }
            if !ref_params.is_empty() {
                self.by_ref_params.insert(name.clone(), ref_params);
            }
        }
    }

    /// Check if a struct type derives Copy (all fields are primitive).
    fn is_struct_copy(&self, struct_name: &str) -> bool {
        if let Some(sig) = self.ctx.structs.get(struct_name) {
            sig.fields.iter().all(|(_, ftype)| {
                matches!(ftype, CrowType::Int | CrowType::Float | CrowType::Bool)
            })
        } else {
            false
        }
    }

    /// Check if a parameter is assigned to (mutated) anywhere in the body.
    fn is_param_mutated(param_name: &str, body: &[Stmt]) -> bool {
        for stmt in body {
            match stmt {
                Stmt::Assign { name, .. } if name == param_name => return true,
                Stmt::If { then_body, else_body, .. } => {
                    if Self::is_param_mutated(param_name, then_body) { return true; }
                    if let Some(eb) = else_body {
                        if Self::is_param_mutated(param_name, eb) { return true; }
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    if Self::is_param_mutated(param_name, body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    /// Like maybe_unwrap_optbox, but returns a reference instead of cloning.
    /// Used when passing to a by-ref parameter.
    fn maybe_unwrap_optbox_ref(&mut self, expr: &Expr) -> Option<String> {
        if let Expr::FieldAccess { object, field } = expr {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                let obj_str = self.expr_to_string(object);
                                return Some(format!("{}.{}.as_ref().unwrap()", obj_str, field));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Operator precedence (higher = binds tighter).
    fn op_precedence(op: &BinOp) -> u8 {
        match op {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => 3,
            BinOp::Add | BinOp::Sub => 4,
            BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::ElemMul | BinOp::ElemDiv => 5,
            BinOp::Pow | BinOp::ElemPow => 6,
        }
    }

    /// Get the precedence of an expression (only returns Some for BinOps).
    fn expr_precedence(expr: &Expr) -> Option<u8> {
        if let Expr::BinOp { op, .. } = expr {
            Some(Self::op_precedence(op))
        } else {
            None
        }
    }

    /// Check if a CrowType contains a Map anywhere.
    fn type_uses_map(ty: &CrowType) -> bool {
        match ty {
            CrowType::Map(_) => true,
            CrowType::Array(inner) | CrowType::Result(inner) => Self::type_uses_map(inner),
            _ => false,
        }
    }

    /// Check if the program uses any Map types (to decide whether to import BTreeMap).
    fn program_uses_map(&self) -> bool {
        for ty in self.ctx.vars.values() {
            if Self::type_uses_map(ty) { return true; }
        }
        for sig in self.ctx.functions.values() {
            for (_, ty) in &sig.params {
                if Self::type_uses_map(ty) { return true; }
            }
            if Self::type_uses_map(&sig.return_type) { return true; }
        }
        for sig in self.ctx.structs.values() {
            for (_, ty) in &sig.fields {
                if Self::type_uses_map(ty) { return true; }
            }
        }
        false
    }

    /// Check if a CrowType contains a Result anywhere.
    fn type_uses_result(ty: &CrowType) -> bool {
        match ty {
            CrowType::Result(_) => true,
            CrowType::Array(inner) => Self::type_uses_result(inner),
            _ => false,
        }
    }

    /// Check if a CrowType contains NumArray anywhere.
    fn type_uses_numarray(ty: &CrowType) -> bool {
        match ty {
            CrowType::NumArray(_) => true,
            CrowType::Result(inner) => Self::type_uses_numarray(inner),
            CrowType::Array(inner) => Self::type_uses_numarray(inner),
            _ => false,
        }
    }

    /// Check if a CrowType contains Sparse anywhere.
    fn type_uses_sparse(ty: &CrowType) -> bool {
        match ty {
            CrowType::Sparse => true,
            CrowType::Result(inner) => Self::type_uses_sparse(inner),
            CrowType::Array(inner) => Self::type_uses_sparse(inner),
            _ => false,
        }
    }

    /// Check if a CrowType contains Json anywhere.
    fn type_uses_json(ty: &CrowType) -> bool {
        match ty {
            CrowType::Json => true,
            CrowType::Result(inner) => Self::type_uses_json(inner),
            CrowType::Array(inner) => Self::type_uses_json(inner),
            _ => false,
        }
    }

    /// Check if the program uses any CrowResult types.
    fn program_uses_result(&self) -> bool {
        for ty in self.ctx.vars.values() {
            if Self::type_uses_result(ty) { return true; }
        }
        for sig in self.ctx.functions.values() {
            if Self::type_uses_result(&sig.return_type) { return true; }
        }
        false
    }

    /// Collect all variable names that are reassigned (assigned more than once) in a sequence of statements.
    fn collect_reassigned_vars(stmts: &[Stmt], seen: &mut std::collections::HashSet<String>, mutated: &mut std::collections::HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, .. } => {
                    if !seen.insert(name.clone()) {
                        mutated.insert(name.clone());
                    }
                }
                Stmt::MultiAssign { names, .. } => {
                    for name in names {
                        if !seen.insert(name.clone()) {
                            mutated.insert(name.clone());
                        }
                    }
                }
                Stmt::If { then_body, else_body, .. } => {
                    Self::collect_reassigned_vars(then_body, seen, mutated);
                    if let Some(eb) = else_body {
                        Self::collect_reassigned_vars(eb, seen, mutated);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    // Any variable assigned inside a loop body is effectively mutated
                    // because the loop may execute multiple times
                    let mut loop_seen = std::collections::HashSet::new();
                    Self::collect_loop_assigned_vars(body, &mut loop_seen);
                    for name in &loop_seen {
                        mutated.insert(name.clone());
                    }
                    Self::collect_reassigned_vars(body, seen, mutated);
                }
                // Field assignment and index assignment mutate the root variable
                Stmt::FieldAssign { object, .. } | Stmt::IndexAssign { object, .. } => {
                    if let Some(var_name) = Self::extract_root_var(object) {
                        mutated.insert(var_name);
                    }
                }
                _ => {}
            }
        }
    }

    /// Extract the root variable name from a nested expression (for mutation tracking)
    fn extract_root_var(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::FieldAccess { object, .. } => Self::extract_root_var(object),
            Expr::Call { callee, .. } => Self::extract_root_var(callee),
            _ => None,
        }
    }

    /// Collect all variable names assigned inside a loop body (they need mut).
    fn collect_loop_assigned_vars(stmts: &[Stmt], assigned: &mut std::collections::HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, .. } => { assigned.insert(name.clone()); }
                Stmt::MultiAssign { names, .. } => {
                    for name in names { assigned.insert(name.clone()); }
                }
                Stmt::If { then_body, else_body, .. } => {
                    Self::collect_loop_assigned_vars(then_body, assigned);
                    if let Some(eb) = else_body {
                        Self::collect_loop_assigned_vars(eb, assigned);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    Self::collect_loop_assigned_vars(body, assigned);
                }
                _ => {}
            }
        }
    }

    /// Check if a for loop body mutates variables from the outer scope.
    /// If it does, the loop must run sequentially (not with par_iter).
    /// "Outer scope" means any variable that was declared before the loop,
    /// excluding the loop variable itself.
    fn for_body_mutates_outer(body: &[Stmt], loop_var: &str, outer_vars: &std::collections::HashSet<String>) -> bool {
        let mut body_writes = std::collections::HashSet::new();
        Self::collect_loop_assigned_vars(body, &mut body_writes);
        // Check if any written variable is from the outer scope (and not the loop var)
        for name in &body_writes {
            if name != loop_var && outer_vars.contains(name) {
                return true;
            }
        }
        false
    }

    /// Recursively check if any for loop exists in the statements (for rayon detection).
    /// Check if an expression contains a ForExpr (for-as-expression).
    fn expr_has_for_expr(expr: &Expr) -> bool {
        match expr {
            Expr::ForExpr { .. } => true,
            _ => false,
        }
    }

    fn has_for_loop(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::For { .. } => return true,
                Stmt::Assign { value, .. } => {
                    if Self::expr_has_for_expr(value) { return true; }
                }
                Stmt::If { then_body, else_body, .. } => {
                    if Self::has_for_loop(then_body) { return true; }
                    if let Some(eb) = else_body {
                        if Self::has_for_loop(eb) { return true; }
                    }
                }
                Stmt::While { body, .. } => {
                    if Self::has_for_loop(body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    /// Transpile a complete program to Rust source.
    pub fn transpile(&mut self, stmts: &[Stmt]) -> String {
        // Register all module names upfront so function bodies can resolve module references
        for stmt in stmts {
            if let Stmt::Use { path, alias: _ } = stmt {
                let module_name = path.rsplit('/').next().unwrap_or(path).to_string();
                if module_name == "math" || path.starts_with("math.") {
                    self.needs_crow_runtime = true;
                }
                // Detect db module early
                if path == "db" {
                    self.needs_crow_db = true;
                    // db queries return crow_runtime frames
                    self.needs_crow_runtime = true;
                    // Default to sqlite feature; URL-based dispatch adds others at runtime
                    self.db_features.insert("sqlite".to_string());
                }
                // Detect web modules early so imports/async main can be emitted
                if path == "web" || path.starts_with("web.") {
                    self.needs_crow_web = true;
                    if path == "web" {
                        self.web_features.insert("http".to_string());
                    } else {
                        let sub = &path[4..];
                        let feature = match sub {
                            "static" => "static_files",
                            other => other,
                        };
                        self.web_features.insert(feature.to_string());
                    }
                }
                // Detect `use ui` early for imports
                if path == "ui" {
                    self.needs_ui = true;
                    self.needs_crow_web = true;
                    self.web_features.insert("live".to_string());
                }
                // Load modules into registry early
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
                if path == "frame" {
                    if !self.registry.is_loaded("frame") {
                        self.registry.load(std::sync::Arc::new(crate::modules::frame::FrameModule));
                    }
                }
                self.modules.insert(module_name);
            }
        }

        // Scan for rayon need
        let needs_par_iter = stmts.iter().any(|s| {
            Self::extract_cpu_call(s).map_or(false, |n| n > 1)
        });
        // For loops are parallel by default — any for loop means we need rayon
        let has_for = Self::has_for_loop(stmts) || self.ctx.functions.values().any(|sig| Self::has_for_loop(&sig.body));
        // Search builtins (any/all/find with a predicate) emit par_iter
        let has_search = Self::has_search_builtin(stmts)
            || self.ctx.functions.values().any(|sig| Self::has_search_builtin(&sig.body));
        let has_for = has_for || has_search;
        // Check for parallel waves (uses rayon::join, no prelude needed)
        let needs_parallel_waves = Self::has_parallel_waves(stmts) || self.ctx.functions.values().any(|sig| Self::has_parallel_waves(&sig.body));
        self.needs_rayon = needs_par_iter || needs_parallel_waves || has_for;

        // Scan for grad[] usage
        let mut grad_fns = Self::scan_grad_functions(stmts);
        // Also scan function bodies
        for sig in self.ctx.functions.values() {
            let sub = Self::scan_grad_functions(&sig.body);
            grad_fns.extend(sub);
        }
        if !grad_fns.is_empty() {
            self.needs_grad = true;
            self.grad_functions = grad_fns.clone();
            // Force grad-target function params to Float (they may be Unknown/String
            // if the function is never called directly)
            for fn_name in &grad_fns {
                if let Some(sig) = self.ctx.functions.get_mut(fn_name) {
                    for (_pname, ptype) in sig.params.iter_mut() {
                        *ptype = CrowType::Float;
                    }
                    sig.return_type = CrowType::Float;
                }
            }
        }

        // Pre-scan: find web handler functions (passed to route[])
        Self::scan_web_handlers(stmts, &mut self.web_handlers);
        for sig in self.ctx.functions.values() {
            Self::scan_web_handlers(&sig.body, &mut self.web_handlers);
        }
        // Update handler function signatures: param = CrowRequest, return = CrowResponse
        for handler_name in self.web_handlers.clone() {
            if let Some(sig) = self.ctx.functions.get_mut(&handler_name) {
                if let Some((_, ptype)) = sig.params.first_mut() {
                    *ptype = CrowType::WebRequest;
                }
                sig.return_type = CrowType::WebResponse;
            }
        }

        // Pre-scan: find live[]/page[] render/handle callback functions
        if self.needs_ui {
            let mut has_page = false;
            Self::scan_live_callbacks(stmts, &mut self.live_render_fns, &mut self.live_handle_fns, &mut has_page);
            for sig in self.ctx.functions.values() {
                Self::scan_live_callbacks(&sig.body, &mut self.live_render_fns, &mut self.live_handle_fns, &mut has_page);
            }
            self.has_page_routes = has_page;

            // Detect reactive render functions (0-param render fns in page[])
            for rname in self.live_render_fns.clone() {
                if let Some(sig) = self.ctx.functions.get(&rname) {
                    if sig.params.is_empty() {
                        self.reactive_render_fns.insert(rname.clone());
                    }
                }
            }

            // Collect reactive variables: top-level assignments that aren't fetch/poll/serve
            if !self.reactive_render_fns.is_empty() {
                for stmt in stmts.iter() {
                    if let Stmt::Assign { name, value } = stmt {
                        // Skip fetch[] and poll[] — those are data sources, not reactive vars
                        if let Expr::Call { callee, args } = value {
                            if let Expr::Ident(fn_name) = callee.as_ref() {
                                if (fn_name == "fetch" && args.len() == 1) || (fn_name == "poll" && args.len() == 2) {
                                    self.data_source_vars.insert(name.clone());
                                    continue;
                                }
                            }
                        }
                        let ty = self.ctx.infer_expr(value);
                        // A db connection is infrastructure, not UI state —
                        // it becomes a process-wide static, never serialized
                        if matches!(ty, CrowType::DbHandle) {
                            self.static_handle_vars.insert(name.clone());
                            continue;
                        }
                        let default_str = self.expr_to_string(value);
                        self.reactive_vars.insert(name.clone(), (ty, default_str));
                    }
                }
            }

            // Scan reactive render function bodies for event handler references
            if !self.reactive_render_fns.is_empty() {
                let known_fns: std::collections::HashSet<String> = self.ctx.functions.keys().cloned().collect();
                for rname in self.reactive_render_fns.clone() {
                    if let Some(sig) = self.ctx.functions.get(&rname) {
                        let body = sig.body.clone();
                        Self::collect_event_handler_refs_stmts(&body, &known_fns, &mut self.reactive_event_handlers);
                    }
                }
                // Event handlers are also live_handle_fns for signature purposes
                for h in self.reactive_event_handlers.clone() {
                    self.live_handle_fns.insert(h);
                }
                // Fns scheduled via later[] from handlers/renders are
                // handler-class too: they read/write reactive state, so they
                // compile with the state preamble/epilogue. Iterate to a
                // fixpoint so later-chains (handler → later f → later g) all
                // get the state signature.
                loop {
                    let mut added = false;
                    for h in self
                        .reactive_event_handlers
                        .clone()
                        .iter()
                        .chain(self.reactive_render_fns.clone().iter())
                    {
                        if let Some(sig) = self.ctx.functions.get(h) {
                            let mut targets = std::collections::HashSet::new();
                            Self::collect_later_targets(&sig.body, &mut targets);
                            for t in targets {
                                if self.ctx.functions.contains_key(&t)
                                    && self.reactive_event_handlers.insert(t.clone())
                                {
                                    self.live_handle_fns.insert(t);
                                    added = true;
                                }
                            }
                        }
                    }
                    if !added {
                        break;
                    }
                }
            }
        }

        // Compute which params should be passed by reference
        self.compute_by_ref_params();

        // Pre-scan: check if any variable uses NumArray type (sets needs_crow_runtime)
        for ty in self.ctx.vars.values() {
            if Self::type_uses_numarray(ty) {
                self.needs_crow_runtime = true;
                break;
            }
        }

        // Emit header — only import BTreeMap if actually used
        if self.program_uses_map() {
            self.emit_line("use std::collections::BTreeMap;");
        }
        if needs_par_iter || has_for {
            self.emit_line("use rayon::prelude::*;");
        }
        if self.needs_crow_runtime {
            self.emit_line("use crow_runtime::tensor::CrowTensor;");
        }
        // Import CrowSparse if any variable uses Sparse type
        let needs_sparse = self.ctx.vars.values().any(|ty| Self::type_uses_sparse(ty));
        if needs_sparse {
            self.emit_line("use crow_runtime::sparse::CrowSparse;");
        }
        // Import crow_web types if web modules are used
        if self.needs_crow_web {
            self.emit_line("use crow_web::server::CrowApp;");
            self.emit_line("use crow_web::core::{CrowRequest, CrowResponse};");
            self.emit_line("use std::sync::Arc;");
            if self.web_features.contains("db") {
                self.emit_line("use crow_web::db::CrowSqlite;");
            }
            if self.web_features.contains("auth") {
                self.emit_line("use crow_web::auth::CrowJwt;");
            }
            if self.web_features.contains("cors") {
                self.emit_line("use crow_web::cors::CrowCors;");
            }
            if self.web_features.contains("session") {
                self.emit_line("use crow_web::session::CrowSessionStore;");
            }
            if self.web_features.contains("template") {
                self.emit_line("use crow_web::template::CrowTemplates;");
            }
            if self.web_features.contains("cache") {
                self.emit_line("use crow_web::cache::CrowCache;");
            }
            if self.web_features.contains("queue") {
                self.emit_line("use crow_web::queue::CrowQueue;");
            }
            if self.web_features.contains("rate") {
                self.emit_line("use crow_web::rate::CrowRateLimiter;");
            }
            if self.web_features.contains("mail") {
                self.emit_line("use crow_web::mail::CrowMailer;");
            }
            if self.web_features.contains("ws") {
                self.emit_line("use crow_web::ws;");
            }
            if self.web_features.contains("sse") {
                self.emit_line("use crow_web::sse;");
            }
            if self.needs_ui {
                self.emit_line("use crow_web::view::CrowVNode;");
                self.emit_line("use crow_web::live::{CrowLiveView, CrowLiveState};");
            }
        }
        // Import crow_db types if db module is used
        if self.needs_crow_db {
            self.emit_line("use crow_db::DbHandle;");
        }
        self.emit_line("");

        // Emit CrowResult<T> unconditionally: fallible builtins can appear in
        // any expression position, and the need is only known during body
        // generation (after this header) — allow(dead_code) covers unused
        {
            let _ = self.program_uses_result();
            self.emit_line("#[allow(dead_code)]");
            self.emit_line("#[derive(Debug, Clone)]");
            self.emit_line("struct CrowResult<T: Clone> {");
            self.indent += 1;
            self.emit_line("ok: bool,");
            self.emit_line("value: T,");
            self.emit_line("reason: String,");
            self.emit_line("kind: String,");
            self.emit_line("trace: Vec<String>,");
            self.emit_line("captured: f64,");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("impl<T: Clone + std::fmt::Display> std::fmt::Display for CrowResult<T> {");
            self.indent += 1;
            self.emit_line("fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {");
            self.indent += 1;
            self.emit_line("if !self.ok { write!(f, \"error: {}\", self.reason) } else { write!(f, \"{}\", self.value) }");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }

        // Emit CrowDual struct if grad[] is used
        if self.needs_grad {
            self.emit_crow_dual_struct();
        }

        // Pre-scan: check if any variable uses Json type (sets needs_serde_json)
        for ty in self.ctx.vars.values() {
            if Self::type_uses_json(ty) {
                self.needs_serde_json = true;
                break;
            }
        }

        // Emit JSON display helper if serde_json is used
        if self.needs_serde_json {
            // Crow-style json display (matches interpreter: arrays with spaces,
            // objects as [k = v], strings quoted inside containers only)
            self.emit_line("fn crow_json_display(v: &serde_json::Value) -> String {");
            self.indent += 1;
            self.emit_line("match v {");
            self.indent += 1;
            self.emit_line("serde_json::Value::String(s) => s.clone(),");
            self.emit_line("serde_json::Value::Null => \"null\".to_string(),");
            self.emit_line("serde_json::Value::Array(items) => format!(\"[{}]\", items.iter().map(crow_json_repr).collect::<Vec<_>>().join(\", \")),");
            self.emit_line("serde_json::Value::Object(m) => format!(\"[{}]\", m.iter().map(|(k, v)| format!(\"{} = {}\", k, crow_json_repr(v))).collect::<Vec<_>>().join(\", \")),");
            self.emit_line("other => other.to_string(),");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("fn crow_json_repr(v: &serde_json::Value) -> String {");
            self.indent += 1;
            self.emit_line("match v {");
            self.indent += 1;
            self.emit_line("serde_json::Value::String(s) => format!(\"'{}'\", s),");
            self.emit_line("other => crow_json_display(other),");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }

        // Emit module-level variable assigns as top-level statics so functions can access them
        let mut module_vars = std::collections::HashSet::new();
        for stmt in stmts {
            if let Stmt::Assign { name, value } = stmt {
                // Check if this variable is prefixed with a known module name
                for mod_name in &self.modules {
                    if name.starts_with(&format!("{}_", mod_name)) {
                        let expr_str = self.expr_to_string(value);
                        let ty = self.ctx.infer_expr(value);
                        let rust_ty = ty.to_rust_type();
                        if rust_ty == "String" {
                            // For string literals, emit as &str const
                            let str_val = expr_str.trim_end_matches(".to_string()");
                            self.emit_line("#[allow(non_upper_case_globals)]");
                            self.emit_line(&format!("const {}: &str = {};", name, str_val));
                        } else {
                            self.emit_line("#[allow(non_upper_case_globals)]");
                            self.emit_line(&format!("const {}: {} = {};", name, rust_ty, expr_str));
                        }
                        module_vars.insert(name.clone());
                        break;
                    }
                }
            }
        }
        if !module_vars.is_empty() {
            self.emit_line("");
        }

        // Emit struct definitions. CrowFindResult is synthetic (registered for
        // the find[] builtin's return type) and is emitted after main, gated on
        // actual use — its need isn't known until the body has been generated.
        let struct_names: Vec<String> = self.ctx.structs.keys().cloned().collect();
        for name in &struct_names {
            if !matches!(name.as_str(), "CrowFindResult" | "CrowShell" | "CrowElapsed") {
                self.emit_struct(name);
            }
        }

        // Emit __json_to_frame helper for data source variables (poll/fetch → CrowFrame)
        if !self.data_source_vars.is_empty() {
            self.needs_serde_json = true;
            self.needs_crow_runtime = true;
            self.emit_line("fn __json_to_frame(v: &serde_json::Value) -> crow_runtime::frame::CrowFrame {");
            self.indent += 1;
            self.emit_line("use crow_runtime::frame::{CrowFrame, Column};");
            self.emit_line("use std::sync::Arc;");
            self.emit_line("let arr = match v.as_array() { Some(a) => a, None => return CrowFrame::new(vec![], vec![], vec![]) };");
            self.emit_line("if arr.is_empty() { return CrowFrame::new(vec![], vec![], vec![]); }");
            self.emit_line("let cols: Vec<String> = arr[0].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();");
            self.emit_line("let nrows = arr.len();");
            self.emit_line("let mut data: Vec<Column> = Vec::new();");
            self.emit_line("let mut validity: Vec<Vec<bool>> = Vec::new();");
            self.emit_line("for col_name in &cols {");
            self.indent += 1;
            self.emit_line("let vals: Vec<&serde_json::Value> = arr.iter().map(|row| row.get(col_name.as_str()).unwrap_or(&serde_json::Value::Null)).collect();");
            self.emit_line("let valid: Vec<bool> = vals.iter().map(|v| !v.is_null()).collect();");
            self.emit_line("let all_int = vals.iter().all(|v| v.is_i64() || v.is_null());");
            self.emit_line("let all_float = vals.iter().all(|v| v.is_f64() || v.is_i64() || v.is_null());");
            self.emit_line("let col = if all_int && vals.iter().any(|v| v.is_i64()) {");
            self.indent += 1;
            self.emit_line("Column::Int(vals.iter().map(|v| v.as_i64().unwrap_or(0)).collect())");
            self.indent -= 1;
            self.emit_line("} else if all_float && vals.iter().any(|v| v.is_f64() || v.is_i64()) {");
            self.indent += 1;
            self.emit_line("Column::Float(vals.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())");
            self.indent -= 1;
            self.emit_line("} else {");
            self.indent += 1;
            self.emit_line("Column::Str(vals.iter().map(|v| v.as_str().unwrap_or(\"\").to_string()).collect())");
            self.indent -= 1;
            self.emit_line("};");
            self.emit_line("data.push(col);");
            self.emit_line("validity.push(valid);");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("CrowFrame::new(cols, data, validity)");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }

        // Emit function definitions (skip uncalled module functions whose types couldn't be inferred)
        let fn_names: Vec<String> = self.ctx.functions.keys().cloned().collect();
        for name in &fn_names {
            // F2 monomorphization: the original of a specialized function is
            // never emitted — its clones (name__1, name__2, ...) are
            if self.ctx.specializations.contains_key(name) {
                continue;
            }
            // Skip functions with unresolved param types: they are never
            // called (a call site would have typed them) and cannot compile.
            // Exception: live/reactive callbacks get injected signatures.
            let sig = self.ctx.functions.get(name).unwrap();
            let has_unknown = sig.params.iter().any(|(_, t)| matches!(t, CrowType::Unknown));
            let is_callback = self.live_render_fns.contains(name)
                || self.live_handle_fns.contains(name)
                || self.reactive_render_fns.contains(name)
                || self.reactive_event_handlers.contains(name)
                || self.web_handlers.contains(name)
                || self.ctx.views.contains(name)
                || self.ctx.view_handlers.contains(name);
            if has_unknown && !is_callback {
                continue;
            }
            self.emit_function(name);
        }

        // Emit dual-mode versions of functions passed to grad[]
        if self.needs_grad {
            let grad_fns: Vec<String> = self.grad_functions.iter().cloned().collect();
            for name in &grad_fns {
                self.emit_dual_function(name);
            }
        }

        // Generate __crow_handle dispatch function for reactive event handlers
        if !self.reactive_event_handlers.is_empty() {
            self.emit_line("fn __crow_handle(state: &mut CrowLiveState, event: &str, _value: Option<&str>) {");
            self.indent += 1;
            self.emit_line("match event {");
            self.indent += 1;
            let handlers: Vec<String> = self.reactive_event_handlers.iter().cloned().collect();
            for h in &handlers {
                let rust_name = Self::rust_fn_name(h);
                self.emit_line(&format!("\"{}\" => {}(state),", h, rust_name));
            }
            self.emit_line("_ => {}");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }

        // Compute which top-level variables need `mut`
        let top_level_stmts: Vec<&Stmt> = stmts.iter()
            .filter(|s| !matches!(s, Stmt::FnDecl { .. } | Stmt::StructDecl { .. } | Stmt::Line(_)))
            .collect();
        let top_stmts_owned: Vec<Stmt> = top_level_stmts.iter().map(|s| (*s).clone()).collect();
        let mut seen = std::collections::HashSet::new();
        let mut mutated = std::collections::HashSet::new();
        Self::collect_reassigned_vars(&top_stmts_owned, &mut seen, &mut mutated);
        self.mutated_vars = mutated;

        // Collect main-body statements (excluding already-emitted items)
        let main_stmts: Vec<Stmt> = stmts.iter()
            .filter(|s| !matches!(s, Stmt::FnDecl { .. } | Stmt::StructDecl { .. }))
            .filter(|s| {
                if let Stmt::Assign { name, .. } = s {
                    !module_vars.contains(name)
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        // Pre-scan: no-app route[]/serve[] calls (plain `use web`, no ui)
        // still need the implicit __crow_app
        if self.needs_crow_web && !self.has_page_routes {
            for stmt in stmts {
                if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
                    if let Expr::Ident(name) = callee.as_ref() {
                        let noapp_route = name == "route"
                            && (args.len() == 2
                                || (args.len() == 3
                                    && matches!(self.ctx.infer_expr(&args[0]), CrowType::Str)));
                        let noapp_serve =
                            name == "serve" && (args.len() == 1 || args.len() == 3);
                        if noapp_route || noapp_serve {
                            self.has_noapp_web = true;
                        }
                    }
                }
            }
        }

        // Process-wide statics for non-serializable handles (db connections
        // in ui programs) — views and handlers read them via OnceLock
        if !self.static_handle_vars.is_empty() {
            let handle_names: Vec<String> = self.static_handle_vars.iter().cloned().collect();
            for name in handle_names {
                self.emit_line("#[allow(non_upper_case_globals)]");
                self.emit_line(&format!(
                    "static __crow_handle_{}: std::sync::OnceLock<crow_db::DbHandle> = std::sync::OnceLock::new();",
                    name
                ));
            }
            self.emit_line("");
        }

        // Emit main function with automatic parallelism
        if self.needs_crow_web {
            self.emit_line("#[tokio::main]");
            self.emit_line("async fn main() {");
        } else {
            self.emit_line("fn main() {");
        }
        self.indent += 1;
        self.emit_panic_hook();
        // No-app web calls without LiveView state: just the implicit app
        if self.has_noapp_web && !self.has_page_routes {
            self.emit_line("let mut __crow_app = CrowApp::new();");
        }
        // If page[] routes are used, create implicit app and reactive state at start of main
        if self.has_page_routes {
            self.emit_line("let mut __crow_app = CrowApp::new();");
            self.emit_line("let mut __crow_state = CrowLiveState::new();");
            // Register reactive variable defaults in state
            if !self.reactive_vars.is_empty() {
                self.needs_serde_json = true;
                let reactive_vars = self.reactive_vars.clone();
                for (vname, (_, default_str)) in &reactive_vars {
                    self.emit_line(&format!("__crow_state.set(\"{}\", serde_json::json!({}));", vname, default_str));
                }
            }
            // Separate serve[] calls from other statements — serve must come last
            // (after all page[] routes are registered)
            let reactive_var_names: std::collections::HashSet<String> = self.reactive_vars.keys().cloned().collect();
            let (serve_stmts, other_stmts): (Vec<_>, Vec<_>) = main_stmts.iter().cloned().partition(|s| {
                matches!(s, Stmt::ExprStmt(Expr::Call { callee, .. }) if matches!(callee.as_ref(), Expr::Ident(name) if name == "serve"))
            });
            // Filter out reactive var assignments from other_stmts (already registered above)
            let filtered_stmts: Vec<_> = other_stmts.into_iter().filter(|s| {
                if let Stmt::Assign { name, .. } = s {
                    !reactive_var_names.contains(name)
                } else {
                    true
                }
            }).collect();
            self.emit_stmts_parallel(&filtered_stmts);
            self.emit_stmts_parallel(&serve_stmts);
        } else {
            self.emit_stmts_parallel(&main_stmts);
        }
        if self.needs_distributed {
            self.emit_line("crow_runtime::distributed::finalize();");
        }
        if self.needs_later {
            self.emit_line("crow_runtime::later::wait_all();");
        }
        self.indent -= 1;
        self.emit_line("}");

        // Synthetic types used by builtin codegen. Emitted after main (item
        // order is irrelevant in Rust) because the need is only known once
        // the body has been generated.
        if self.needs_time_result {
            self.emit_line("");
            self.emit_line("#[allow(dead_code)]");
            self.emit_line("fn __crow_time_result(r: Result<f64, String>) -> CrowResult<f64> {");
            self.indent += 1;
            self.emit_line("match r {");
            self.indent += 1;
            self.emit_line("Ok(t) => CrowResult { ok: true, value: t, reason: String::new(), kind: String::new(), trace: Vec::new(), captured: 0.0 },");
            self.emit_line("Err(e) => CrowResult { ok: false, value: 0.0, reason: e, kind: \"time\".to_string(), trace: Vec::new(), captured: 0.0 },");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
        }
        if self.needs_find_struct {
            self.emit_line("");
            self.emit_line("#[derive(Clone, Debug)]");
            self.emit_line("struct CrowFindResult { ok: bool, groups: Vec<String> }");
        }
        if self.needs_shell_struct {
            self.emit_line("");
            self.emit_line("#[derive(Clone, Debug)]");
            self.emit_line("struct CrowShell { status: i64, stdout: String, stderr: String }");
        }
        if self.needs_elapsed_struct {
            self.emit_line("");
            self.emit_line("#[derive(Clone, Debug)]");
            self.emit_line("struct CrowElapsed { seconds: f64, minutes: f64, hours: f64, days: f64 }");
        }

        self.output.clone()
    }

    fn emit_panic_hook(&mut self) {
        self.emit_line("std::panic::set_hook(Box::new(|info| {");
        self.indent += 1;
        self.emit_line("let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {");
        self.emit_line("    s.to_string()");
        self.emit_line("} else if let Some(s) = info.payload().downcast_ref::<String>() {");
        self.emit_line("    s.clone()");
        self.emit_line("} else {");
        self.emit_line("    \"unknown error\".to_string()");
        self.emit_line("};");
        // Translate common Rust panic messages to Crow-friendly messages
        self.emit_line("let crow_msg = if msg.contains(\"index out of bounds\") {");
        self.emit_line("    let parts: Vec<&str> = msg.split(\"the len is \").collect();");
        self.emit_line("    if parts.len() > 1 {");
        self.emit_line("        let rest: Vec<&str> = parts[1].split(\" but the index is \").collect();");
        self.emit_line("        if rest.len() > 1 {");
        self.emit_line("            format!(\"index {} is out of range (array has {} items)\", rest[1], rest[0])");
        self.emit_line("        } else { format!(\"index out of range\") }");
        self.emit_line("    } else { format!(\"index out of range\") }");
        self.emit_line("} else if msg.contains(\"divide by zero\") || msg.contains(\"division by zero\") {");
        self.emit_line("    format!(\"cannot divide by zero\")");
        self.emit_line("} else if msg.contains(\"attempt to divide by zero\") {");
        self.emit_line("    format!(\"cannot divide by zero\")");
        self.emit_line("} else if msg.contains(\"assertion failed\") {");
        self.emit_line("    if msg.starts_with(\"assertion failed: \") {");
        self.emit_line("        msg.clone()");
        self.emit_line("    } else { format!(\"assertion failed\") }");
        self.emit_line("} else if msg.contains(\"stack overflow\") {");
        self.emit_line("    format!(\"stack overflow (possible infinite recursion)\")");
        self.emit_line("} else if msg.contains(\"arithmetic operation overflowed\") || msg.contains(\"attempt to \") {");
        self.emit_line("    let crow = msg.replace(\"attempt to \", \"\");");
        self.emit_line("    format!(\"arithmetic error: {}\", crow)");
        self.emit_line("} else {");
        self.emit_line("    format!(\"runtime error: {}\", msg)");
        self.emit_line("};");
        self.emit_line("eprintln!(\"error: {}\", crow_msg);");
        self.emit_line("// A panic on the main thread is fatal; a contained panic in a");
        self.emit_line("// worker/server task is reported but must not kill the server.");
        self.emit_line("if std::thread::current().name() == Some(\"main\") {");
        self.emit_line("    std::process::exit(1);");
        self.emit_line("}");
        self.indent -= 1;
        self.emit_line("}));");
    }

    fn emit_line(&mut self, line: &str) {
        for _ in 0..self.indent {
            self.output.push_str("    ");
        }
        self.output.push_str(line);
        self.output.push('\n');
    }

    fn emit_struct(&mut self, name: &str) {
        let sig = self.ctx.structs.get(name).unwrap().clone();
        // Use Copy+Clone for simple structs (all fields are primitive types)
        let all_copy = sig.fields.iter().all(|(_, ftype)| {
            matches!(ftype, CrowType::Int | CrowType::Float | CrowType::Bool)
        });
        self.emit_line("#[allow(non_camel_case_types)]");
        if all_copy {
            self.emit_line("#[derive(Debug, Clone, Copy)]");
        } else {
            self.emit_line("#[derive(Debug, Clone)]");
        }
        self.emit_line(&format!("struct {} {{", name));
        self.indent += 1;
        for (fname, ftype) in &sig.fields {
            self.emit_line(&format!("{}: {},", fname, ftype.to_rust_type()));
        }
        self.indent -= 1;
        self.emit_line("}");
        self.emit_line("");
        // Display through the shared kernel, matching the interpreter's
        // Name[field = value] format (fields alphabetical — the interpreter
        // stores them in a BTreeMap)
        if self.struct_kernel_displayable(name) {
            self.needs_crow_runtime = true;
            let mut sorted: Vec<&(String, CrowType)> = sig.fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let parts: Vec<String> = sorted.iter().map(|(f, _)| format!("{} = {{}}", f)).collect();
            let vals: Vec<String> = sorted.iter().map(|(f, _)| format!("self.{}.crow_repr()", f)).collect();
            self.emit_line(&format!("impl crow_runtime::display::CrowDisplay for {} {{", name));
            self.indent += 1;
            self.emit_line("fn crow_repr(&self) -> String {");
            self.indent += 1;
            self.emit_line(&format!(
                "{{ use crow_runtime::display::CrowDisplay as _; format!(\"{}[{}]\", {}) }}",
                name, parts.join(", "), vals.join(", ")
            ));
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }
    }

    fn emit_function(&mut self, name: &str) {
        let sig = self.ctx.functions.get(name).unwrap().clone();
        let ref_set = self.by_ref_params.get(name).cloned().unwrap_or_default();

        let is_live_render = self.live_render_fns.contains(name);
        let is_live_handle = self.live_handle_fns.contains(name);
        let is_reactive_render = self.reactive_render_fns.contains(name);
        let is_reactive_handler = self.reactive_event_handlers.contains(name);

        // Build param array — override for live render/handle callbacks
        let params: Vec<String> = if is_reactive_render {
            // Reactive render: inject state param (user wrote 0 params)
            vec!["state: &CrowLiveState".to_string()]
        } else if is_reactive_handler {
            // Reactive handler: inject mut state param (user wrote 0 params)
            vec!["state: &mut CrowLiveState".to_string()]
        } else if is_live_render {
            // render(state: &CrowLiveState) -> CrowVNode
            sig.params.iter().enumerate().map(|(i, (pname, _))| {
                if i == 0 { format!("{}: &CrowLiveState", pname) }
                else { format!("{}: String", pname) }
            }).collect()
        } else if is_live_handle {
            // handle(state: &mut CrowLiveState, event: &str, value: Option<&str>)
            sig.params.iter().enumerate().map(|(i, (pname, _))| {
                match i {
                    0 => format!("{}: &mut CrowLiveState", pname),
                    1 => format!("{}: &str", pname),
                    2 => format!("{}: Option<&str>", pname),
                    _ => format!("{}: String", pname),
                }
            }).collect()
        } else {
            sig.params.iter()
                .map(|(pname, ptype)| {
                    if ref_set.contains(pname) {
                        format!("{}: &{}", pname, ptype.to_rust_type())
                    } else {
                        format!("{}: {}", pname, ptype.to_rust_type())
                    }
                })
                .collect()
        };

        // Any `view` declaration returns a UI tree, whether or not the
        // page[]-driven detection saw it (parameterized components)
        let is_component_view = self.ctx.views.contains(name);
        let ret_str = if is_live_render || is_component_view {
            " -> CrowVNode".to_string()
        } else if is_live_handle {
            String::new() // void
        } else {
            match &sig.return_type {
                CrowType::Void => String::new(),
                t => format!(" -> {}", t.to_rust_type()),
            }
        };

        let rust_name = Self::rust_fn_name(name);
        self.emit_line(&format!("fn {}({}){} {{", rust_name, params.join(", "), ret_str));
        self.indent += 1;

        // Save and reset declared vars for function scope
        let saved_declared = self.declared.clone();
        let saved_vars = self.ctx.vars.clone();
        let saved_ref_params = self.current_ref_params.clone();
        let saved_mutated = self.mutated_vars.clone();
        let saved_fn_return_type = self.current_fn_return_type.clone();
        let saved_function_name = self.current_function_name.clone();
        self.declared.clear();
        self.current_ref_params = ref_set;
        self.current_fn_return_type = Some(sig.return_type.clone());
        self.current_function_name = name.to_string();

        // Compute which local variables need `mut` in this function
        let mut seen = std::collections::HashSet::new();
        let mut mutated = std::collections::HashSet::new();
        // Params count as first "assignment"
        for (pname, _) in &sig.params {
            seen.insert(pname.clone());
        }
        Self::collect_reassigned_vars(&sig.body, &mut seen, &mut mutated);
        self.mutated_vars = mutated;

        // Mark params as declared and add to vars for type inference
        for (pname, ptype) in &sig.params {
            self.declared.insert(pname.clone());
            self.ctx.vars.insert(pname.clone(), ptype.clone());
        }

        // Reactive preamble: inject state.get calls for all reactive variables
        if is_reactive_render || is_reactive_handler {
            let reactive_vars = self.reactive_vars.clone();
            for (vname, (vtype, default_str)) in &reactive_vars {
                let kw = if is_reactive_handler { "let mut" } else { "let" };
                let getter = match vtype {
                    CrowType::Int => format!("state.get_i64(\"{}\", {})", vname, default_str),
                    CrowType::Float => format!("state.get_f64(\"{}\", {})", vname, default_str),
                    CrowType::Str => format!("state.get_str(\"{}\", &{})", vname, default_str),
                    CrowType::Bool => format!("state.get_bool(\"{}\", {})", vname, default_str),
                    _ => format!("state.get_i64(\"{}\", {})", vname, default_str), // fallback
                };
                self.emit_line(&format!("{} {} = {};", kw, vname, getter));
                self.declared.insert(vname.clone());
                self.ctx.vars.insert(vname.clone(), vtype.clone());
            }
            // Data source preamble: inject JSON-to-CrowFrame conversion for poll/fetch vars
            if is_reactive_render && !self.data_source_vars.is_empty() {
                let ds_vars = self.data_source_vars.clone();
                for vname in &ds_vars {
                    self.emit_line(&format!(
                        "let {} = __json_to_frame(state.get(\"{}\").unwrap_or(&serde_json::Value::Null));",
                        vname, vname
                    ));
                    self.declared.insert(vname.clone());
                    self.ctx.vars.insert(vname.clone(), CrowType::Frame);
                }
            }
        }

        // For functions with non-void return type, convert last ExprStmt to Return
        // so it becomes a tail expression (no semicolon) in Rust
        let mut body = sig.body.clone();
        if !matches!(sig.return_type, CrowType::Void) {
            if let Some(last_idx) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
                if let Stmt::ExprStmt(e) = &body[last_idx] {
                    body[last_idx] = Stmt::Return(Some(e.clone()));
                }
            }
        }
        // Views render sequentially — they describe a tree; wave-parallelizing
        // render statements risks moving shared reads into closures for no win
        if is_component_view || is_live_render || is_reactive_render {
            for stmt in &body {
                self.emit_stmt(stmt);
            }
        } else {
            self.emit_stmts_parallel(&body);
        }

        // Reactive epilogue: inject state.set calls for all reactive variables (handlers only)
        if is_reactive_handler {
            self.needs_serde_json = true;
            let reactive_vars = self.reactive_vars.clone();
            for (vname, _) in &reactive_vars {
                self.emit_line(&format!("state.set(\"{}\", serde_json::json!({}));", vname, vname));
            }
        }

        self.declared = saved_declared;
        self.ctx.vars = saved_vars;
        self.current_ref_params = saved_ref_params;
        self.mutated_vars = saved_mutated;
        self.current_fn_return_type = saved_fn_return_type;
        self.current_function_name = saved_function_name;
        self.indent -= 1;
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emit the CrowDual struct with operator implementations for forward-mode AD.
    fn emit_crow_dual_struct(&mut self) {
        self.emit_line("#[derive(Debug, Clone, Copy)]");
        self.emit_line("struct CrowDual { real: f64, dual: f64 }");
        self.emit_line("impl CrowDual {");
        self.indent += 1;
        self.emit_line("fn new(real: f64, dual: f64) -> Self { CrowDual { real, dual } }");
        self.emit_line("fn abs(self) -> Self { CrowDual { real: self.real.abs(), dual: self.dual * self.real.signum() } }");
        self.emit_line("fn sqrt(self) -> Self { let s = self.real.sqrt(); CrowDual { real: s, dual: self.dual / (2.0 * s) } }");
        self.emit_line("fn powf(self, n: CrowDual) -> Self { let r = self.real.powf(n.real); CrowDual { real: r, dual: r * (n.dual * self.real.ln() + n.real * self.dual / self.real) } }");
        self.emit_line("fn powi_const(self, n: i64) -> Self { CrowDual { real: self.real.powi(n as i32), dual: (n as f64) * self.real.powi(n as i32 - 1) * self.dual } }");
        self.emit_line("fn floor(self) -> CrowDual { CrowDual { real: self.real.floor(), dual: 0.0 } }");
        self.emit_line("fn ceil(self) -> CrowDual { CrowDual { real: self.real.ceil(), dual: 0.0 } }");
        self.emit_line("fn round(self) -> CrowDual { CrowDual { real: self.real.round(), dual: 0.0 } }");
        self.indent -= 1;
        self.emit_line("}");
        // Implement Display to show only real part
        self.emit_line("impl std::fmt::Display for CrowDual {");
        self.indent += 1;
        self.emit_line("fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, \"{}\", self.real) }");
        self.indent -= 1;
        self.emit_line("}");
        // Arithmetic ops
        for (trait_name, op, body) in &[
            ("Add", "+", "CrowDual { real: self.real + rhs.real, dual: self.dual + rhs.dual }"),
            ("Sub", "-", "CrowDual { real: self.real - rhs.real, dual: self.dual - rhs.dual }"),
            ("Mul", "*", "CrowDual { real: self.real * rhs.real, dual: self.dual * rhs.real + self.real * rhs.dual }"),
            ("Div", "/", "CrowDual { real: self.real / rhs.real, dual: (self.dual * rhs.real - self.real * rhs.dual) / (rhs.real * rhs.real) }"),
        ] {
            self.emit_line(&format!("impl std::ops::{} for CrowDual {{", trait_name));
            self.indent += 1;
            self.emit_line("type Output = CrowDual;");
            self.emit_line(&format!("fn {}(self, rhs: CrowDual) -> CrowDual {{ {} }}", op.to_lowercase().replace("+", "add").replace("-", "sub").replace("*", "mul").replace("/", "div"), body));
            self.indent -= 1;
            self.emit_line("}");
        }
        // Neg
        self.emit_line("impl std::ops::Neg for CrowDual {");
        self.indent += 1;
        self.emit_line("type Output = CrowDual;");
        self.emit_line("fn neg(self) -> CrowDual { CrowDual { real: -self.real, dual: -self.dual } }");
        self.indent -= 1;
        self.emit_line("}");
        // Ops with f64 on both sides
        for (trait_name, method, body_df, body_fd) in &[
            ("Add", "add", "CrowDual { real: self.real + rhs, dual: self.dual }", "CrowDual { real: self + rhs.real, dual: rhs.dual }"),
            ("Sub", "sub", "CrowDual { real: self.real - rhs, dual: self.dual }", "CrowDual { real: self - rhs.real, dual: -rhs.dual }"),
            ("Mul", "mul", "CrowDual { real: self.real * rhs, dual: self.dual * rhs }", "CrowDual { real: self * rhs.real, dual: self * rhs.dual }"),
            ("Div", "div", "CrowDual { real: self.real / rhs, dual: self.dual / rhs }", "CrowDual { real: self / rhs.real, dual: -self * rhs.dual / (rhs.real * rhs.real) }"),
        ] {
            // CrowDual op f64
            self.emit_line(&format!("impl std::ops::{}<f64> for CrowDual {{", trait_name));
            self.indent += 1;
            self.emit_line("type Output = CrowDual;");
            self.emit_line(&format!("fn {}(self, rhs: f64) -> CrowDual {{ {} }}", method, body_df));
            self.indent -= 1;
            self.emit_line("}");
            // f64 op CrowDual
            self.emit_line(&format!("impl std::ops::{}<CrowDual> for f64 {{", trait_name));
            self.indent += 1;
            self.emit_line("type Output = CrowDual;");
            self.emit_line(&format!("fn {}(self, rhs: CrowDual) -> CrowDual {{ {} }}", method, body_fd));
            self.indent -= 1;
            self.emit_line("}");
        }
        // Ops with i64 on both sides
        for (trait_name, method, body_di, body_id) in &[
            ("Add", "add", "CrowDual { real: self.real + rhs as f64, dual: self.dual }", "CrowDual { real: self as f64 + rhs.real, dual: rhs.dual }"),
            ("Sub", "sub", "CrowDual { real: self.real - rhs as f64, dual: self.dual }", "CrowDual { real: self as f64 - rhs.real, dual: -rhs.dual }"),
            ("Mul", "mul", "CrowDual { real: self.real * rhs as f64, dual: self.dual * rhs as f64 }", "CrowDual { real: self as f64 * rhs.real, dual: self as f64 * rhs.dual }"),
            ("Div", "div", "CrowDual { real: self.real / rhs as f64, dual: self.dual / rhs as f64 }", "CrowDual { real: self as f64 / rhs.real, dual: -(self as f64) * rhs.dual / (rhs.real * rhs.real) }"),
        ] {
            // CrowDual op i64
            self.emit_line(&format!("impl std::ops::{}<i64> for CrowDual {{", trait_name));
            self.indent += 1;
            self.emit_line("type Output = CrowDual;");
            self.emit_line(&format!("fn {}(self, rhs: i64) -> CrowDual {{ {} }}", method, body_di));
            self.indent -= 1;
            self.emit_line("}");
            // i64 op CrowDual
            self.emit_line(&format!("impl std::ops::{}<CrowDual> for i64 {{", trait_name));
            self.indent += 1;
            self.emit_line("type Output = CrowDual;");
            self.emit_line(&format!("fn {}(self, rhs: CrowDual) -> CrowDual {{ {} }}", method, body_id));
            self.indent -= 1;
            self.emit_line("}");
        }
        // From conversions
        self.emit_line("impl From<f64> for CrowDual { fn from(v: f64) -> Self { CrowDual { real: v, dual: 0.0 } } }");
        self.emit_line("impl From<i64> for CrowDual { fn from(v: i64) -> Self { CrowDual { real: v as f64, dual: 0.0 } } }");
        // PartialEq and PartialOrd for comparisons
        self.emit_line("impl PartialEq for CrowDual { fn eq(&self, other: &Self) -> bool { self.real == other.real } }");
        self.emit_line("impl PartialOrd for CrowDual { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { self.real.partial_cmp(&other.real) } }");
        self.emit_line("");
    }

    /// Emit a dual-mode version of a function for use with grad[].
    /// All numeric params become CrowDual, return type becomes CrowDual.
    fn emit_dual_function(&mut self, name: &str) {
        let sig = match self.ctx.functions.get(name) {
            Some(s) => s.clone(),
            None => return,
        };

        let params: Vec<String> = sig.params.iter()
            .map(|(pname, _)| format!("{}: CrowDual", pname))
            .collect();

        let rust_name = format!("_dual_{}", Self::rust_fn_name(name));
        self.emit_line(&format!("fn {}({}) -> CrowDual {{", rust_name, params.join(", ")));
        self.indent += 1;

        // Save state
        let saved_declared = self.declared.clone();
        let saved_vars = self.ctx.vars.clone();
        let saved_ref_params = self.current_ref_params.clone();
        let saved_mutated = self.mutated_vars.clone();
        let saved_fn_return_type = self.current_fn_return_type.clone();
        let saved_function_name = self.current_function_name.clone();
        let saved_emitting_dual = self.emitting_dual;
        self.declared.clear();
        self.current_ref_params = std::collections::HashSet::new();
        self.current_fn_return_type = Some(CrowType::Float); // placeholder
        self.current_function_name = name.to_string();
        self.emitting_dual = true;

        // Compute mutated vars
        let mut seen = std::collections::HashSet::new();
        let mut mutated = std::collections::HashSet::new();
        for (pname, _) in &sig.params {
            seen.insert(pname.clone());
        }
        Self::collect_reassigned_vars(&sig.body, &mut seen, &mut mutated);
        self.mutated_vars = mutated;

        // Mark params as declared
        for (pname, _) in &sig.params {
            self.declared.insert(pname.clone());
            self.ctx.vars.insert(pname.clone(), CrowType::Float);
        }

        // Convert last ExprStmt to Return for tail expression (no semicolon in Rust)
        let mut body = sig.body.clone();
        if let Some(last_idx) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if let Stmt::ExprStmt(e) = &body[last_idx] {
                body[last_idx] = Stmt::Return(Some(e.clone()));
            }
        }
        // Dual functions must be emitted sequentially — CrowDual types
        // are incompatible with the f64 type annotations in rayon::join
        for stmt in &body {
            self.emit_stmt(stmt);
        }

        // Restore state
        self.emitting_dual = false;
        self.declared = saved_declared;
        self.ctx.vars = saved_vars;
        self.current_ref_params = saved_ref_params;
        self.mutated_vars = saved_mutated;
        self.current_fn_return_type = saved_fn_return_type;
        self.current_function_name = saved_function_name;
        self.emitting_dual = saved_emitting_dual;
        self.indent -= 1;
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emit a block of statements with automatic parallelism.
    /// Groups independent assignments into waves and emits rayon::join for parallel waves.
    fn emit_stmts_parallel(&mut self, stmts: &[Stmt]) {
        let waves = group_into_waves(stmts);
        for wave in &waves {
            // Update current_line from nearest preceding Line marker for each wave
            if let Some(&first_idx) = wave.first() {
                for j in (0..first_idx).rev() {
                    if let Stmt::Line(n) = &stmts[j] {
                        self.current_line = *n;
                        break;
                    }
                }
            }
            // Handle-backed vars (db/ui handles read through OnceCell) must go
            // through emit_stmt's .set() protocol — fusing them into a
            // rayon::join let-binding would leave the handle forever unset.
            let touches_handle = wave.iter().any(|&i| {
                matches!(&stmts[i], Stmt::Assign { name, .. } if self.static_handle_vars.contains(name))
            });
            if wave.len() <= 1 || touches_handle {
                // Single statement — emit normally
                for &i in wave {
                    // Update line for this specific statement
                    for j in (0..i).rev() {
                        if let Stmt::Line(n) = &stmts[j] {
                            self.current_line = *n;
                            break;
                        }
                    }
                    if let Some(n) = Self::extract_cpu_call(&stmts[i]) {
                        self.cpu_count = Some(n as usize);
                    } else {
                        self.emit_stmt(&stmts[i]);
                    }
                }
            } else {
                // Multiple independent assignments — emit in parallel
                self.emit_parallel_wave(wave, stmts);
            }
        }
    }

    /// Emit a wave of independent assignments using rayon::join.
    /// For 2 stmts: let (a, b) = rayon::join(|| expr1, || expr2);
    /// For 3+: nested rayon::join
    fn emit_parallel_wave(&mut self, wave: &[usize], stmts: &[Stmt]) {
        self.needs_rayon = true;

        // Collect (name, expr, type) for each assignment in the wave
        let mut assigns: Vec<(String, String, CrowType)> = vec![];
        for &i in wave {
            // Update current_line for this specific statement
            for j in (0..i).rev() {
                if let Stmt::Line(n) = &stmts[j] {
                    self.current_line = *n;
                    break;
                }
            }
            match &stmts[i] {
                Stmt::Assign { name, value } => {
                    // Intercept fetch/poll assignments — emit as state registration, not parallel
                    if self.has_page_routes {
                        if let Expr::Call { callee, args } = value {
                            if let Expr::Ident(fn_name) = callee.as_ref() {
                                if (fn_name == "fetch" && args.len() == 1) || (fn_name == "poll" && args.len() == 2) {
                                    self.emit_stmt(&stmts[i]);
                                    continue;
                                }
                            }
                        }
                    }
                    let mut expr_str = self.expr_to_string(value);
                    let mut ty = self.ctx.infer_expr(value);
                    if format!("{:?}", ty).contains("Unknown") {
                        if let Some(resolved) = self.ctx.vars.get(name) {
                            ty = resolved.clone();
                        }
                    }
                    // Clone Frame values in parallel closures to avoid moving
                    if matches!(ty, CrowType::Frame) {
                        if let Expr::Ident(_) = value {
                            expr_str = format!("{}.clone()", expr_str);
                        }
                    }
                    assigns.push((name.clone(), expr_str, ty));
                }
                Stmt::MultiAssign { names, values } => {
                    for (name, value) in names.iter().zip(values.iter()) {
                        let expr_str = self.expr_to_string(value);
                        let ty = self.ctx.infer_expr(value);
                        assigns.push((name.clone(), expr_str, ty));
                    }
                }
                _ => {
                    // Shouldn't happen (barriers are filtered), but emit normally
                    self.emit_stmt(&stmts[i]);
                }
            }
        }

        if assigns.is_empty() {
            return;
        }

        // Fn-typed values (grad closures) can't be parallelized —
        // closures don't coerce to fn pointers in rayon::join's generic return
        if assigns.iter().any(|(_, _, ty)| matches!(ty, CrowType::Fn(_, _))) {
            for &i in wave {
                self.emit_stmt(&stmts[i]);
            }
            return;
        }

        // Check which vars are new declarations vs reassignments
        let all_new = assigns.iter().all(|(name, _, _)| !self.declared.contains(name));

        // Build the nested rayon::join expression
        let join_expr = self.build_rayon_join(&assigns, 0);

        // Build the destructuring pattern
        let pattern = self.build_join_pattern(&assigns, 0);

        if all_new {
            // All new declarations: let (a, b, ...) = rayon::join(...)
            // Build type annotation
            let type_pattern = self.build_join_type_pattern(&assigns, 0);
            self.emit_line(&format!("let {}: {} = {};", pattern, type_pattern, join_expr));
            for (name, _, ty) in &assigns {
                self.declared.insert(name.clone());
                self.ctx.vars.insert(name.clone(), ty.clone());
            }
        } else {
            // Mixed or all reassignments: use temp names
            let temp_pattern = self.build_join_temp_pattern(&assigns, 0);
            let type_pattern = self.build_join_type_pattern(&assigns, 0);
            self.emit_line(&format!("let {}: {} = {};", temp_pattern, type_pattern, join_expr));
            for (idx, (name, _, ty)) in assigns.iter().enumerate() {
                let temp = format!("_par_{}", idx);
                if self.declared.contains(name) {
                    self.emit_line(&format!("{} = {};", name, temp));
                } else {
                    let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                    self.emit_line(&format!("{} {} = {};", kw, name, temp));
                    self.declared.insert(name.clone());
                    self.ctx.vars.insert(name.clone(), ty.clone());
                }
            }
        }
    }

    /// Build nested rayon::join expression from assignments.
    /// 2 items: rayon::join(|| e1, || e2)
    /// 3+ items: rayon::join(|| e1, || rayon::join(|| e2, || e3))
    fn build_rayon_join(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        if remaining == 1 {
            assigns[start].1.clone()
        } else if remaining == 2 {
            format!("rayon::join(|| {}, || {})", assigns[start].1, assigns[start + 1].1)
        } else {
            let rest = self.build_rayon_join(assigns, start + 1);
            format!("rayon::join(|| {}, || {})", assigns[start].1, rest)
        }
    }

    /// Build destructuring pattern: (mut a, (b, mut c)) for nested join
    fn build_join_pattern(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        let name_with_mut = |idx: usize| -> String {
            let name = &assigns[idx].0;
            if self.mutated_vars.contains(name) {
                format!("mut {}", name)
            } else {
                name.clone()
            }
        };
        if remaining == 1 {
            name_with_mut(start)
        } else if remaining == 2 {
            format!("({}, {})", name_with_mut(start), name_with_mut(start + 1))
        } else {
            let rest = self.build_join_pattern(assigns, start + 1);
            format!("({}, {})", name_with_mut(start), rest)
        }
    }

    /// Build temp name pattern: (_par_0, (_par_1, _par_2))
    fn build_join_temp_pattern(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        if remaining == 1 {
            format!("_par_{}", start)
        } else if remaining == 2 {
            format!("(_par_{}, _par_{})", start, start + 1)
        } else {
            let rest = self.build_join_temp_pattern(assigns, start + 1);
            format!("(_par_{}, {})", start, rest)
        }
    }

    /// Build type pattern: (i64, (String, f64))
    fn build_join_type_pattern(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        if remaining == 1 {
            assigns[start].2.to_rust_type()
        } else if remaining == 2 {
            format!("({}, {})", assigns[start].2.to_rust_type(), assigns[start + 1].2.to_rust_type())
        } else {
            let rest = self.build_join_type_pattern(assigns, start + 1);
            format!("({}, {})", assigns[start].2.to_rust_type(), rest)
        }
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                // Intercept fetch/poll assignments → register on __crow_state
                if self.has_page_routes {
                    if let Expr::Call { callee, args } = value {
                        if let Expr::Ident(fn_name) = callee.as_ref() {
                            if fn_name == "fetch" && args.len() == 1 {
                                let url_str = self.expr_to_string(&args[0]);
                                self.emit_line(&format!("__crow_state.register_fetch(\"{}\", &{});", name, url_str));
                                self.data_source_vars.insert(name.clone());
                                return;
                            }
                            if fn_name == "poll" && args.len() == 2 {
                                let url_str = self.expr_to_string(&args[0]);
                                let secs_str = self.expr_to_string(&args[1]);
                                self.emit_line(&format!("__crow_state.register_poll(\"{}\", &{}, {} as u64);", name, url_str, secs_str));
                                self.data_source_vars.insert(name.clone());
                                return;
                            }
                        }
                    }
                }
                if self.static_handle_vars.contains(name) {
                    let expr_str = self.expr_to_string(value);
                    self.emit_line(&format!(
                        "let _ = __crow_handle_{}.set({});",
                        name, expr_str
                    ));
                    return;
                }
                let expr_str = self.owned_expr_str(value);
                if self.declared.contains(name) {
                    // Check for type-changing reassignment
                    let new_ty = self.ctx.infer_expr(value);
                    if let Some(existing_ty) = self.ctx.vars.get(name).cloned() {
                        // Assignment into a typed variable is a consumption
                        // point: result[T] unwraps into T, aborting this fn
                        // on error (handlers stop; result-fns propagate).
                        if let CrowType::Result(inner) = &new_ty {
                            if existing_ty.compatible(inner)
                                || (matches!(existing_ty, CrowType::Float)
                                    && matches!(inner.as_ref(), CrowType::Int))
                            {
                                let abort = if matches!(
                                    self.current_fn_return_type,
                                    Some(CrowType::Result(_))
                                ) {
                                    "return CrowResult { ok: false, value: Default::default(), reason: _asg.reason.clone(), kind: _asg.kind.clone(), trace: _asg.trace.clone(), captured: 0.0 };".to_string()
                                } else {
                                    "return;".to_string()
                                };
                                let cast = if matches!(existing_ty, CrowType::Float)
                                    && matches!(inner.as_ref(), CrowType::Int)
                                {
                                    " as f64"
                                } else {
                                    ""
                                };
                                self.emit_line(&format!(
                                    "{{ let _asg = {}; if !_asg.ok {{ {} }} {} = _asg.value{}; }}",
                                    expr_str, abort, name, cast
                                ));
                                return;
                            }
                        }
                        if !existing_ty.compatible(&new_ty) {
                            panic!("error: cannot reassign '{}' from {} to {} — variable type cannot change", name, existing_ty.crow_name(), new_ty.crow_name());
                        }
                        // int/float rule: a float variable stays float — coerce int values
                        if matches!(existing_ty, CrowType::Float) && matches!(new_ty, CrowType::Int) {
                            self.emit_line(&format!("{} = ({}) as f64;", name, expr_str));
                            return;
                        }
                    }
                    self.emit_line(&format!("{} = {};", name, expr_str));
                } else {
                    let mut ty = self.ctx.infer_expr(value);
                    // If the expression type has Unknown (e.g. map[]), use the
                    // resolved variable type from the inference pass instead
                    if format!("{:?}", ty).contains("Unknown") {
                        if let Some(resolved) = self.ctx.vars.get(name) {
                            ty = resolved.clone();
                        }
                    }
                    // int/float rule: a variable ever assigned a float is a float
                    // for its whole scope — declare from the unified type
                    let mut expr_str = expr_str.clone();
                    if matches!(ty, CrowType::Int) {
                        if matches!(self.ctx.vars.get(name), Some(CrowType::Float)) {
                            ty = CrowType::Float;
                            expr_str = format!("({}) as f64", expr_str);
                        }
                    }
                    let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                    let in_live_callback = self.live_render_fns.contains(&self.current_function_name)
                        || self.live_handle_fns.contains(&self.current_function_name);
                    if self.emitting_dual || in_live_callback {
                        // In dual/live mode, let Rust infer the type
                        self.emit_line(&format!("{} {} = {};", kw, name, expr_str));
                    } else {
                        self.emit_line(&format!("{} {}: {} = {};", kw, name, ty.to_rust_type(), expr_str));
                    }
                    self.declared.insert(name.clone());
                    self.ctx.vars.insert(name.clone(), ty);
                }
            }
            Stmt::MultiAssign { names, values } => {
                // Evaluate all RHS into temporaries first (needed for swap: a, b = b, a)
                let mut exprs: Vec<String> = Vec::new();
                let mut types: Vec<CrowType> = Vec::new();
                for value in values.iter() {
                    exprs.push(self.expr_to_string(value));
                    types.push(self.ctx.infer_expr(value));
                }
                // Check for type-changing reassignment
                for (i, name) in names.iter().enumerate() {
                    if self.declared.contains(name) {
                        if let Some(existing_ty) = self.ctx.vars.get(name).cloned() {
                            if !existing_ty.compatible(&types[i]) {
                                panic!("error: cannot reassign '{}' from {} to {} — variable type cannot change", name, existing_ty.crow_name(), types[i].crow_name());
                            }
                        }
                    }
                }
                // Check if any RHS reads a variable that a previous LHS writes (swap pattern)
                let rhs_reads: std::collections::HashSet<String> = values.iter().flat_map(|v| crate::parallel::expr_reads(v)).collect();
                let lhs_names: std::collections::HashSet<String> = names.iter().cloned().collect();
                let needs_temps = !rhs_reads.is_disjoint(&lhs_names) && names.iter().any(|n| self.declared.contains(n));
                if needs_temps {
                    // Use temporaries for safe swap
                    for (i, (expr, ty)) in exprs.iter().zip(types.iter()).enumerate() {
                        self.emit_line(&format!("let _multi_{}: {} = {};", i, ty.to_rust_type(), expr));
                    }
                    for (i, name) in names.iter().enumerate() {
                        if self.declared.contains(name) {
                            self.emit_line(&format!("{} = _multi_{};", name, i));
                        } else {
                            let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                            self.emit_line(&format!("{} {}: {} = _multi_{};", kw, name, types[i].to_rust_type(), i));
                            self.declared.insert(name.clone());
                            self.ctx.vars.insert(name.clone(), types[i].clone());
                        }
                    }
                } else {
                    // No swap conflict — emit directly
                    for (i, name) in names.iter().enumerate() {
                        if self.declared.contains(name) {
                            self.emit_line(&format!("{} = {};", name, exprs[i]));
                        } else {
                            let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                            self.emit_line(&format!("{} {}: {} = {};", kw, name, types[i].to_rust_type(), exprs[i]));
                            self.declared.insert(name.clone());
                            self.ctx.vars.insert(name.clone(), types[i].clone());
                        }
                    }
                }
            }
            Stmt::Return(Some(e)) => {
                let expr_str = self.owned_expr_str(e);
                let expr_type = self.ctx.infer_expr(e);
                // If the function returns CrowResult<T> but this expression is a plain T, wrap it
                if let Some(CrowType::Result(_)) = &self.current_fn_return_type {
                    if !matches!(expr_type, CrowType::Result(_)) {
                        self.emit_line(&format!("return CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }};", expr_str));
                    } else {
                        self.emit_line(&format!("return {};", expr_str));
                    }
                } else {
                    self.emit_line(&format!("return {};", expr_str));
                }
            }
            Stmt::Return(None) => {
                self.emit_line("return;");
            }
            Stmt::Break => {
                self.emit_line("break;");
            }
            Stmt::Continue => {
                self.emit_line("continue;");
            }
            Stmt::ExprStmt(_) => {
                // Check if this is a cpu[n] call - update cpu_count instead of emitting code
                if let Some(n) = Self::extract_cpu_call(stmt) {
                    self.cpu_count = Some(n as usize);
                    return;
                }
                // Check if this is a gpu[] or gpu[n] call
                if Self::is_gpu_call(stmt) {
                    self.needs_crow_runtime = true;
                    self.emit_line("crow_runtime::gpu::set_gpu_enabled(true);");
                    return;
                }
                // Check if this is a nodes[n] call
                if let Some(n) = Self::extract_nodes_call(stmt) {
                    self.needs_crow_runtime = true;
                    self.needs_distributed = true;
                    self.emit_line(&format!("crow_runtime::distributed::init({});", n));
                    return;
                }
                if let Stmt::ExprStmt(e) = stmt {
                    let expr_str = self.expr_to_string(e);
                    self.emit_line(&format!("{};", expr_str));
                }
            }
            Stmt::If { condition, then_body, else_body } => {
                let cond_str = self.expr_to_string(condition);
                let cond_str = self.guard_condition(condition, cond_str);
                self.emit_line(&format!("if {} {{", cond_str));
                self.indent += 1;
                let saved = self.declared.clone();
                for s in then_body { self.emit_stmt(s); }
                self.declared = saved;
                self.indent -= 1;
                if let Some(eb) = else_body {
                    self.emit_line("} else {");
                    self.indent += 1;
                    let saved = self.declared.clone();
                    for s in eb { self.emit_stmt(s); }
                    self.declared = saved;
                    self.indent -= 1;
                }
                self.emit_line("}");
            }
            Stmt::While { condition, body } => {
                // Detect `while true` and emit `loop {}` to avoid Rust warning
                let is_infinite = matches!(condition, Expr::Bool(true));
                if is_infinite {
                    self.emit_line("loop {");
                } else {
                    let cond_str = self.expr_to_string(condition);
                    let cond_str = self.guard_condition(condition, cond_str);
                    self.emit_line(&format!("while {} {{", cond_str));
                }
                self.indent += 1;
                let saved = self.declared.clone();
                for s in body { self.emit_stmt(s); }
                self.declared = saved;
                self.indent -= 1;
                self.emit_line("}");
            }
            Stmt::For { var, iter, body } => {
                let iter_str = self.expr_to_string(iter);

                // Determine element type for closure pattern
                let iter_type = self.ctx.infer_expr(iter);
                let is_numarray = matches!(&iter_type, CrowType::NumArray(_));
                let elem_type = match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => *inner.clone(),
                    _ => CrowType::Unknown,
                };
                let is_copy = matches!(elem_type, CrowType::Int | CrowType::Float | CrowType::Bool);

                // Parallel by default. Fall back to sequential if:
                // 1. cpu[1] was explicitly set, OR
                // 2. The loop body mutates variables from the outer scope, OR
                // 3. The loop body contains break or continue
                let forced_sequential = self.cpu_count.map_or(false, |n| n <= 1);
                let mutates_outer = Self::for_body_mutates_outer(body, var, &self.declared);
                let use_par = !forced_sequential && !mutates_outer;

                // For NumArray, convert to Vec<f64> first, then iterate
                let actual_iter_str = if is_numarray {
                    if matches!(elem_type, CrowType::NumArray(_)) {
                        // Rank >= 2: iterate rows as tensors
                        format!("{}.rows()", iter_str)
                    } else if matches!(elem_type, CrowType::Int) {
                        format!("{}.to_vec().iter().map(|&v| v as i64).collect::<Vec<i64>>()", iter_str)
                    } else {
                        format!("{}.to_vec()", iter_str)
                    }
                } else {
                    iter_str.clone()
                };

                let saved = self.declared.clone();
                if use_par {
                    let param = if is_copy { format!("|&{}|", var) } else { format!("|{}|", var) };
                    self.emit_line(&format!("{}.par_iter().for_each({} {{", actual_iter_str, param));
                    self.indent += 1;
                    self.declared.insert(var.clone());
                    self.ctx.vars.insert(var.clone(), elem_type);
                    for s in body { self.emit_stmt(s); }
                    self.indent -= 1;
                    self.emit_line("});");
                } else {
                    let param = if is_copy { format!("&{}", var) } else { var.clone() };
                    self.emit_line(&format!("for {} in {}.iter() {{", param, actual_iter_str));
                    self.indent += 1;
                    self.declared.insert(var.clone());
                    self.ctx.vars.insert(var.clone(), elem_type);
                    for s in body { self.emit_stmt(s); }
                    self.indent -= 1;
                    self.emit_line("}");
                }
                self.declared = saved;
            }
            Stmt::FieldAssign { object, field, value } => {
                let obj_str = self.expr_to_string(object);
                let val_str = self.expr_to_string(value);
                let obj_type = self.ctx.infer_expr(object);
                if matches!(obj_type, CrowType::Map(_)) {
                    self.emit_line(&format!("{}.insert(\"{}\".to_string(), {});", obj_str, field, val_str));
                } else {
                    self.emit_line(&format!("{}.{} = {};", obj_str, field, val_str));
                }
            }
            Stmt::IndexAssign { object, index, value } => {
                let obj_str = self.expr_to_string(object);
                let val_str = self.expr_to_string(value);
                // Determine if object is a map or list
                let obj_type = self.ctx.infer_expr(object);
                match obj_type {
                    CrowType::Map(_) => {
                        let key_str = self.expr_to_string(index);
                        self.emit_line(&format!("{}.insert({}, {});", obj_str, key_str, val_str));
                    }
                    CrowType::NumArray(_) => {
                        // CrowTensor is immutable — rebuild with the element replaced
                        let idx_str = self.expr_to_string(index);
                        self.needs_crow_runtime = true;
                        self.emit_line(&format!(
                            "{{ let _i = {}; let mut _tmp = {}.to_vec(); let _idx = if _i < 0 {{ (_tmp.len() as i64 + _i) as usize }} else {{ _i as usize }}; _tmp[_idx] = {} as f64; {} = crow_runtime::tensor::CrowTensor::from_vec(_tmp, {}.shape().to_vec()); }}",
                            idx_str, obj_str, val_str, obj_str, obj_str
                        ));
                    }
                    _ => {
                        let idx_str = self.expr_to_string(index);
                        self.emit_line(&format!("{{ let _i = {}; let _idx = if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }}; {}[_idx] = {}; }}", idx_str, obj_str, obj_str, val_str));
                    }
                }
            }
            Stmt::Use { path, alias: _ } => {
                self.emit_use(path);
            }
            Stmt::Resource { kind, count } => {
                if kind == "cpu" {
                    if let Expr::Int(n) = count {
                        self.cpu_count = Some(*n as usize);
                    }
                } else if kind == "gpu" {
                    self.needs_crow_runtime = true;
                    self.emit_line("crow_runtime::gpu::set_gpu_enabled(true);");
                } else if kind == "nodes" {
                    if let Expr::Int(n) = count {
                        self.needs_crow_runtime = true;
                        self.needs_distributed = true;
                        self.emit_line(&format!("crow_runtime::distributed::init({});", n));
                    }
                }
            }
            // Verification blocks are check-time only — stripped from builds.
            Stmt::TestBlock { .. } => {}
            Stmt::Line(n) => {
                self.current_line = *n;
            }
            _ => {}
        }
    }

    /// Handle a `use` import. Module declarations have already been expanded and
    /// type-inferred as prefixed names (e.g., mymath_double) by expand_module_imports.
    /// We just need to record the module name so FieldAccess can resolve module.var.
    fn emit_use(&mut self, path: &str) {
        let module_name = path.rsplit('/').next().unwrap_or(path).to_string();
        if module_name == "math" || path.starts_with("math.") {
            self.needs_crow_runtime = true;
        }
        // Load modules into registry (with is_loaded guards to avoid overwriting fn_map entries)
        if path == "math" && !self.registry.is_loaded("math") {
            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
        }
        if path == "math.stats" {
            if !self.registry.is_loaded("math") {
                self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
            }
            if !self.registry.is_loaded("math.stats") {
                self.registry.load(std::sync::Arc::new(crate::modules::stats::StatsModule));
            }
            self.modules.insert("math".to_string());
        }
        if path == "math.nn" {
            if !self.registry.is_loaded("math") {
                self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
            }
            if !self.registry.is_loaded("math.nn") {
                self.registry.load(std::sync::Arc::new(crate::modules::nn::NnModule));
            }
            self.modules.insert("math".to_string());
        }
        if path == "io" {
            self.needs_crow_runtime = true;
            if !self.registry.is_loaded("io") {
                self.registry.load(std::sync::Arc::new(crate::modules::io::IoModule));
            }
        }
        if path == "frame" {
            self.needs_crow_runtime = true;
            if !self.registry.is_loaded("frame") {
                self.registry.load(std::sync::Arc::new(crate::modules::frame::FrameModule));
            }
        }
        // Handle web.* modules → crow_web crate with feature flags
        if path.starts_with("web.") {
            self.needs_crow_web = true;
            let sub = &path[4..]; // e.g. "auth", "session", "db"
            // Map Crow module names to crow_web feature names
            let feature = match sub {
                "static" => "static_files",
                other => other,
            };
            self.web_features.insert(feature.to_string());
            self.modules.insert("web".to_string());
        } else if path == "web" {
            // `use web` alone → the http server module
            self.needs_crow_web = true;
            self.web_features.insert("http".to_string());
            self.modules.insert("web".to_string());
        } else if path == "ui" {
            // `use ui` → enable live view element functions (div, span, h1, button, etc.)
            self.needs_ui = true;
            self.needs_crow_web = true;
            self.web_features.insert("live".to_string());
            self.modules.insert("ui".to_string());
        } else if path == "db" {
            self.needs_crow_db = true;
            self.db_features.insert("sqlite".to_string());
            self.modules.insert("db".to_string());
        }
        self.modules.insert(module_name);
    }

    /// A connect[] url literal names its backend — the generated crate must
    /// compile that feature in.
    fn note_db_backend(&mut self, url_expr: &str) {
        if url_expr.contains("clickhouse://") {
            self.db_features.insert("clickhouse".to_string());
        }
        if url_expr.contains("postgres://") || url_expr.contains("postgresql://") {
            self.db_features.insert("postgres".to_string());
        }
    }

    /// Convert Crow SQL params to serde_json::Value for crow_db calls.
    fn crow_params_to_json(params: &[String]) -> String {
        params.iter()
            .map(|p| format!("serde_json::json!({})", p))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// SQL bind params: a literal array like ['alice', 30] is positional open
    /// data — emit each element as its own json! value (a single Rust vec
    /// could not hold the mixed types).
    fn db_params_to_json(&mut self, params: &[Expr]) -> String {
        let mut out = Vec::new();
        for p in params {
            match p {
                Expr::Array(elems) => {
                    for e in elems {
                        let s = self.expr_to_string(e);
                        out.push(format!("serde_json::json!({})", s));
                    }
                }
                other => {
                    let s = self.expr_to_string(other);
                    out.push(format!("serde_json::json!({})", s));
                }
            }
        }
        out.join(", ")
    }

    /// Emit an expression as an OWNED value. A by-ref struct parameter used
    /// where ownership is required (array literal, struct field, return,
    /// assignment) must be cloned — call sites pass structs as &T.
    fn owned_expr_str(&mut self, e: &Expr) -> String {
        let s = self.expr_to_string(e);
        if let Expr::Ident(name) = e {
            if self.current_ref_params.contains(name) {
                return format!("{}.clone()", s);
            }
            // Value semantics: assigning/embedding a non-Copy VARIABLE clones —
            // the source variable stays usable (a = b; use b)
            if !self.ctx.functions.contains_key(name) && !self.static_handle_vars.contains(name) {
                let ty = self.ctx.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                let non_copy = matches!(
                    ty,
                    CrowType::Str
                        | CrowType::Array(_)
                        | CrowType::NumArray(_)
                        | CrowType::Map(_)
                        | CrowType::Struct(_)
                        | CrowType::Frame
                        | CrowType::Json
                        | CrowType::Result(_)
                );
                if non_copy {
                    return format!("{}.clone()", s);
                }
            }
        }
        s
    }

    fn expr_to_string(&mut self, expr: &Expr) -> String {
        match expr {
            // Lowering resolves all named arguments before codegen
            Expr::Named { value, .. } => self.expr_to_string(value),
            Expr::Int(n) => {
                if self.emitting_dual {
                    format!("CrowDual::from({}_i64)", n)
                } else {
                    format!("{}_i64", n)
                }
            }
            Expr::Float(n) => {
                let s = format!("{}", n);
                let lit = if s.contains('.') { format!("{}_f64", s) }
                    else { format!("{}.0_f64", s) };
                if self.emitting_dual {
                    format!("CrowDual::from({})", lit)
                } else {
                    lit
                }
            }
            Expr::Bool(b) => format!("{}", b),
            Expr::Str(s) => self.string_to_rust(s),
            Expr::Ident(name) => {
                // Static handles (db connections in ui programs) read from
                // their process-wide OnceLock
                if self.static_handle_vars.contains(name) {
                    return format!("__crow_handle_{}.get().unwrap().clone()", name);
                }
                // Module constants: pi, e, inf, nan.
                // A user variable with the same name shadows the constant
                // (matches interpreter scoping: environment first, then constants)
                if self.registry.has_constant(name) && !self.ctx.vars.contains_key(name) {
                    return match name.as_str() {
                        "pi" => "std::f64::consts::PI".to_string(),
                        "e" => "std::f64::consts::E".to_string(),
                        "inf" => "f64::INFINITY".to_string(),
                        "nan" => "f64::NAN".to_string(),
                        _ => name.clone(),
                    };
                }
                // If this ident refers to a user-defined function (used as a value),
                // apply the same renaming to avoid Rust collisions
                if self.ctx.functions.contains_key(name) {
                    if self.ctx.specializations.contains_key(name) {
                        panic!("error: '{}' is used at multiple type signatures and cannot be passed as a value — a function value has one type. Wrap it in a single-type function", name);
                    }
                    Self::rust_fn_name(name)
                } else {
                    name.clone()
                }
            }
            Expr::BinOp { left, op, right } => {
                // Check for OptBoxStruct comparison with sentinel: field eq 0 => field.is_none()
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    if let Some(result) = self.try_optbox_comparison(left, op, right) {
                        return result;
                    }
                }
                let parent_prec = Self::op_precedence(op);
                // Wrap left child in parens if it has strictly lower precedence
                let l = self.expr_to_string(left);
                let l = if Self::expr_precedence(left).map_or(false, |p| p < parent_prec) {
                    format!("({})", l)
                } else { l };
                // Wrap right child in parens if it has lower or equal precedence (left-associative)
                let r = self.expr_to_string(right);
                let r = if Self::expr_precedence(right).map_or(false, |p| p <= parent_prec) {
                    format!("({})", r)
                } else { r };

                let lt = self.ctx.infer_expr(left);
                let rt = self.ctx.infer_expr(right);

                let l_is_result = matches!(&lt, CrowType::Result(_));
                let r_is_result = matches!(&rt, CrowType::Result(_));

                // Unwrap inner types
                let lt_inner = if let CrowType::Result(inner) = &lt { *inner.clone() } else { lt.clone() };
                let rt_inner = if let CrowType::Result(inner) = &rt { *inner.clone() } else { rt.clone() };

                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Pow => "pow",
                    BinOp::ElemMul => ".*",
                    BinOp::ElemDiv => "./",
                    BinOp::ElemPow => ".^",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Gt => ">",
                    BinOp::Lt => "<",
                    BinOp::Gte => ">=",
                    BinOp::Lte => "<=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                };

                if l_is_result || r_is_result {
                    // Auto-propagation: check .ok and propagate errors
                    let ctr = self.binop_counter;
                    self.binop_counter += 1;
                    let result_type = match op {
                        BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt
                        | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or => CrowType::Bool,
                        BinOp::Mod => CrowType::Int,
                        _ => lt_inner.unify(&rt_inner),
                    };
                    let default_val = result_type.default_rust_value();
                    let trace_expr = self.trace_format_expr();

                    // Build the inner arithmetic expression using .value
                    let l_val = if l_is_result { format!("_binl_{}.value", ctr) } else { l.clone() };
                    let r_val = if r_is_result { format!("_binr_{}.value", ctr) } else { r.clone() };
                    let inner_expr = match (&lt_inner, &rt_inner) {
                        (CrowType::Int, CrowType::Float) => format!("({}) as f64 {} {}", l_val, op_str, r_val),
                        (CrowType::Float, CrowType::Int) => format!("{} {} ({}) as f64", l_val, op_str, r_val),
                        _ => format!("{} {} {}", l_val, op_str, r_val),
                    };

                    let success = format!(
                        "CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}",
                        inner_expr
                    );

                    // Build nested checks: check left first, then right
                    let mut result = success;
                    if r_is_result {
                        let error_r = format!(
                            "{{ let mut _err = CrowResult {{ ok: false, value: {}, reason: _binr_{}.reason.clone(), kind: _binr_{}.kind.clone(), trace: _binr_{}.trace.clone(), captured: 0.0 }}; _err.trace.push({}); _err }}",
                            default_val, ctr, ctr, ctr, trace_expr
                        );
                        result = format!(
                            "{{ let _binr_{} = {}.clone(); if !_binr_{}.ok {{ {} }} else {{ {} }} }}",
                            ctr, r, ctr, error_r, result
                        );
                    }
                    if l_is_result {
                        let error_l = format!(
                            "{{ let mut _err = CrowResult {{ ok: false, value: {}, reason: _binl_{}.reason.clone(), kind: _binl_{}.kind.clone(), trace: _binl_{}.trace.clone(), captured: 0.0 }}; _err.trace.push({}); _err }}",
                            default_val, ctr, ctr, ctr, trace_expr
                        );
                        result = format!(
                            "{{ let _binl_{} = {}.clone(); if !_binl_{}.ok {{ {} }} else {{ {} }} }}",
                            ctr, l, ctr, error_l, result
                        );
                    }
                    result
                } else {
                    // No CrowResult operands — simple arithmetic
                    // Sparse binary ops
                    let l_is_sparse = matches!(&lt_inner, CrowType::Sparse);
                    let r_is_sparse = matches!(&rt_inner, CrowType::Sparse);
                    if l_is_sparse && r_is_sparse && matches!(op, BinOp::Add) {
                        return format!("{}.add(&{})", l, r);
                    }
                    if l_is_sparse && matches!(&rt_inner, CrowType::NumArray(_)) && matches!(op, BinOp::Mul) {
                        return format!("{}.matmul_dense(&{})", l, r);
                    }
                    // Frame columns bridge into tensor ops via to_tensor()
                    let (l, lt_inner) = if matches!(&lt_inner, CrowType::FrameCol)
                        && matches!(op, BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow | BinOp::Add | BinOp::Sub)
                    {
                        (format!("({}).to_tensor()", l), CrowType::NumArray(Box::new(CrowType::Float)))
                    } else {
                        (l.clone(), lt_inner.clone())
                    };
                    let (r, rt_inner) = if matches!(&rt_inner, CrowType::FrameCol)
                        && matches!(op, BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow | BinOp::Add | BinOp::Sub)
                    {
                        (format!("({}).to_tensor()", r), CrowType::NumArray(Box::new(CrowType::Float)))
                    } else {
                        (r.clone(), rt_inner.clone())
                    };
                    // NumArray element-wise ops
                    let l_is_numarray = matches!(&lt_inner, CrowType::NumArray(_));
                    let r_is_numarray = matches!(&rt_inner, CrowType::NumArray(_));
                    if l_is_numarray && r_is_numarray {
                        // tensor op tensor
                        return match op {
                            BinOp::Add => format!("{}.add(&{})", l, r),
                            BinOp::Sub => format!("{}.sub(&{})", l, r),
                            BinOp::Mul => format!("{}.matmul(&{})", l, r),
                            BinOp::Div => panic!("use ./ for element-wise tensor division"),
                            BinOp::ElemMul => format!("{}.mul(&{})", l, r),
                            BinOp::ElemDiv => format!("{}.div(&{})", l, r),
                            BinOp::ElemPow => format!("{}.pow(&{})", l, r),
                            BinOp::Eq => format!("{} == {}", l, r),
                            BinOp::Ne => format!("{} != {}", l, r),
                            _ => format!("{} {} {}", l, op_str, r),
                        };
                    } else if l_is_numarray && matches!(&rt_inner, CrowType::Int | CrowType::Float) {
                        // tensor op scalar
                        return match op {
                            BinOp::Add => format!("{}.add_scalar({} as f64)", l, r),
                            BinOp::Sub => format!("{}.sub_scalar({} as f64)", l, r),
                            BinOp::Mul | BinOp::ElemMul => format!("{}.mul_scalar({} as f64)", l, r),
                            BinOp::Div | BinOp::ElemDiv => format!("{}.mul_scalar(1.0 / ({} as f64))", l, r),
                            BinOp::ElemPow => format!("{}.map(|x| x.powf({} as f64))", l, r),
                            // Comparisons broadcast to boolean masks
                            BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte | BinOp::Eq | BinOp::Ne => {
                                format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _v {} _s).collect::<Vec<bool>>() }}", r, l, op_str)
                            }
                            _ => format!("{} {} {}", l, op_str, r),
                        };
                    } else if r_is_numarray && matches!(&lt_inner, CrowType::Int | CrowType::Float) {
                        // scalar op tensor
                        if matches!(op, BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte | BinOp::Eq | BinOp::Ne) {
                            return format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _s {} _v).collect::<Vec<bool>>() }}", l, r, op_str);
                        }
                        return match op {
                            BinOp::Add => format!("{}.add_scalar({} as f64)", r, l),
                            BinOp::Sub => format!("({}).neg().add_scalar({} as f64)", r, l),
                            BinOp::Mul | BinOp::ElemMul => format!("{}.mul_scalar({} as f64)", r, l),
                            BinOp::Div | BinOp::ElemDiv => format!("{{ let _t = &{}; crow_runtime::tensor::CrowTensor::from_vec(_t.to_vec().iter().map(|&v| ({} as f64) / v).collect(), _t.shape().to_vec()) }}", r, l),
                            _ => format!("{} {} {}", l, op_str, r),
                        };
                    // Tensor vs scalar comparisons broadcast to boolean masks
                    } else if matches!(op, BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte | BinOp::Eq | BinOp::Ne)
                        && matches!(&lt_inner, CrowType::NumArray(_))
                        && matches!(&rt_inner, CrowType::Int | CrowType::Float) {
                        let rust_op = match op {
                            BinOp::Gt => ">", BinOp::Lt => "<", BinOp::Gte => ">=",
                            BinOp::Lte => "<=", BinOp::Eq => "==", BinOp::Ne => "!=",
                            _ => unreachable!(),
                        };
                        return format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _v {} _s).collect::<Vec<bool>>() }}", r, l, rust_op);
                    } else if matches!(op, BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte | BinOp::Eq | BinOp::Ne)
                        && matches!(&rt_inner, CrowType::NumArray(_))
                        && matches!(&lt_inner, CrowType::Int | CrowType::Float) {
                        let rust_op = match op {
                            BinOp::Gt => ">", BinOp::Lt => "<", BinOp::Gte => ">=",
                            BinOp::Lte => "<=", BinOp::Eq => "==", BinOp::Ne => "!=",
                            _ => unreachable!(),
                        };
                        return format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _s {} _v).collect::<Vec<bool>>() }}", l, r, rust_op);
                    // Boolean masks: and/or combine element-wise
                    } else if matches!(op, BinOp::And | BinOp::Or)
                        && matches!(&lt_inner, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool))
                        && matches!(&rt_inner, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool)) {
                        return format!(
                            "{}.iter().zip({}.iter()).map(|(_a, _b)| *_a {} *_b).collect::<Vec<bool>>()",
                            l, r, op_str
                        );
                    // Frame column (FrameCol) compared with string scalar
                    } else if matches!(&lt_inner, CrowType::FrameCol) && matches!(&rt_inner, CrowType::Str) {
                        let is_comparison = matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte);
                        if is_comparison {
                            let cmp_name = match op {
                                BinOp::Gt => "gt", BinOp::Lt => "lt", BinOp::Gte => "gte",
                                BinOp::Lte => "lte", BinOp::Eq => "eq", BinOp::Ne => "ne",
                                _ => unreachable!(),
                            };
                            return format!("{}.compare_str(&{}, \"{}\")", l, r, cmp_name);
                        }
                        return format!("{} {} {}", l, op_str, r);
                    // Frame column (FrameCol or legacy Array(Str)) ops with scalar
                    } else if (matches!(&lt_inner, CrowType::FrameCol) || matches!(&lt_inner, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str)))
                        && matches!(&rt_inner, CrowType::Int | CrowType::Float) {
                        // column op scalar — use native typed operations
                        let is_comparison = matches!(op, BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte | BinOp::Eq | BinOp::Ne);
                        if is_comparison {
                            let cmp_name = match op {
                                BinOp::Gt => "gt", BinOp::Lt => "lt", BinOp::Gte => "gte",
                                BinOp::Lte => "lte", BinOp::Eq => "eq", BinOp::Ne => "ne",
                                _ => unreachable!(),
                            };
                            if matches!(&lt_inner, CrowType::FrameCol) {
                                return format!("{}.compare({} as f64, \"{}\")", l, r, cmp_name);
                            }
                            return format!("crow_runtime::frame::col_compare(&{}, {} as f64, \"{}\")", l, r, cmp_name);
                        }
                        let arith_name = match op {
                            BinOp::Add => "add", BinOp::Sub => "sub",
                            BinOp::Mul => "mul", BinOp::Div => "div",
                            _ => return format!("{} {} {}", l, op_str, r),
                        };
                        if matches!(&lt_inner, CrowType::FrameCol) {
                            return format!("{}.arithmetic({} as f64, \"{}\")", l, r, arith_name);
                        }
                        return format!("crow_runtime::frame::col_arithmetic(&{}, {} as f64, \"{}\")", l, r, arith_name);
                    } else {
                        if self.emitting_dual {
                            // In dual mode, CrowDual handles mixed-type ops via trait impls
                            if matches!(op, BinOp::Pow) {
                                return format!("({}).powf({})", l, r);
                            }
                            format!("{} {} {}", l, op_str, r)
                        } else if matches!(op, BinOp::Pow) {
                            // Scalar power
                            if lt_inner == CrowType::Float || rt_inner == CrowType::Float {
                                format!("(({}) as f64).powf(({}) as f64)", l, r)
                            } else {
                                format!("({}).pow(({}) as u32)", l, r)
                            }
                        } else {
                            match (&lt_inner, &rt_inner) {
                                (CrowType::Int, CrowType::Float) => format!("({}) as f64 {} {}", l, op_str, r),
                                (CrowType::Float, CrowType::Int) => format!("{} {} ({}) as f64", l, op_str, r),
                                // String + is not allowed — use interpolation instead
                                _ => format!("{} {} {}", l, op_str, r),
                            }
                        }
                    }
                }
            }
            Expr::UnaryOp { op, expr: e } => {
                let inner = self.expr_to_string(e);
                match op {
                    UnaryOp::Neg => {
                        let ty = self.ctx.infer_expr(e);
                        if matches!(ty, CrowType::NumArray(_)) {
                            format!("{}.neg()", inner)
                        } else {
                            format!("(-{})", inner)
                        }
                    }
                    UnaryOp::Not => {
                        let ty = self.ctx.infer_expr(e);
                        let ty = if let CrowType::Result(i) = ty { *i } else { ty };
                        // Boolean masks negate element-wise
                        if matches!(&ty, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool)) {
                            format!("{}.iter().map(|_b| !*_b).collect::<Vec<bool>>()", inner)
                        } else {
                            // Inner parens required: Rust's ! binds tighter than
                            // comparisons, so `not (x eq 8)` must not become `!x == 8`
                            format!("(!({}))", inner)
                        }
                    }
                }
            }
            Expr::Call { callee, args } => {
                // Determine which params are by-ref for the callee
                let callee_name = if let Expr::Ident(n) = callee.as_ref() { Some(n.clone()) } else { None };
                let ref_flags: Vec<bool> = if let Some(ref cname) = callee_name {
                    if let Some(ref_set) = self.by_ref_params.get(cname) {
                        if let Some(sig) = self.ctx.functions.get(cname) {
                            sig.params.iter().map(|(pn, _)| ref_set.contains(pn)).collect()
                        } else { vec![false; args.len()] }
                    } else { vec![false; args.len()] }
                } else { vec![false; args.len()] };

                // Build arg strings with by-ref awareness
                let mut arg_strs: Vec<String> = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    // When unwrapping CrowResult args, replace with temp var .value
                    if self.unwrap_result_args {
                        if let Some(ref indices) = self.propagation_indices {
                            if indices.contains(&i) {
                                arg_strs.push(format!("_prop_{}.value.clone()", i));
                                continue;
                            }
                        }
                    }
                    let is_by_ref = ref_flags.get(i).copied().unwrap_or(false);
                    if is_by_ref {
                        // By-ref param: pass reference, no clone
                        if let Some(unwrapped) = self.maybe_unwrap_optbox_ref(a) {
                            arg_strs.push(unwrapped);
                        } else if let Expr::Ident(vname) = a {
                            if self.current_ref_params.contains(vname) {
                                arg_strs.push(vname.clone()); // already a reference
                            } else {
                                arg_strs.push(format!("&{}", vname));
                            }
                        } else {
                            let expr_str = self.expr_to_string(a);
                            arg_strs.push(format!("&{}", expr_str));
                        }
                    } else {
                        // By-value param: pass a clone when handing a non-Copy
                        // VARIABLE to a user function — Crow has value
                        // semantics, and the variable may be used again
                        // (fixes use-of-moved-value for tensor/array/map args)
                        if let Some(unwrapped) = self.maybe_unwrap_optbox(a) {
                            arg_strs.push(unwrapped);
                        } else {
                            let is_user_fn = callee_name
                                .as_ref()
                                .map_or(false, |n| self.ctx.functions.contains_key(n));
                            let needs_clone = is_user_fn
                                && matches!(a, Expr::Ident(_))
                                && {
                                    let t = self.ctx.infer_expr(a);
                                    let t = if let CrowType::Result(i) = t { *i } else { t };
                                    matches!(
                                        t,
                                        CrowType::NumArray(_)
                                            | CrowType::Array(_)
                                            | CrowType::Map(_)
                                            | CrowType::Str
                                            | CrowType::Frame
                                            | CrowType::Struct(_)
                                    )
                                };
                            let e = self.expr_to_string(a);
                            if needs_clone {
                                arg_strs.push(format!("{}.clone()", e));
                            } else {
                                arg_strs.push(e);
                            }
                        }
                    }
                }

                // --- Auto-propagation: detect CrowResult args ---
                if !self.unwrap_result_args {
                    let skip_propagation = callee_name.as_ref()
                        .map(|n| {
                            // captured[] is metadata access — like .ok, it
                            // reads the header rather than the value, so it
                            // never propagates its argument's error
                            matches!(n.as_str(), "log" | "assert" | "cpu" | "gpu" | "nodes" | "type" | "error" | "captured")
                                // ui elements + table render what they're
                                // given — a fallible arg renders its reason,
                                // it doesn't wrap the tree in a Result
                                || (self.needs_ui
                                    && (n == "table"
                                        || crate::lower::UI_ELEMENTS.contains(&n.as_str())))
                                // len[] unwraps fallible frames itself
                                || n == "len"
                        })
                        .unwrap_or(true);
                    if !skip_propagation {
                        let result_arg_indices: Vec<usize> = args.iter().enumerate()
                            .filter(|(_, a)| matches!(self.ctx.infer_expr(a), CrowType::Result(_)))
                            .map(|(i, _)| i)
                            .collect();
                        if !result_arg_indices.is_empty() {
                            // Save original arg expressions for temp var bindings
                            let orig_arg_strs: Vec<String> = result_arg_indices.iter()
                                .map(|&i| arg_strs[i].clone())
                                .collect();
                            // Re-enter with unwrapping enabled
                            self.unwrap_result_args = true;
                            self.propagation_indices = Some(result_arg_indices.clone());
                            let inner_expr = self.expr_to_string(expr);
                            self.unwrap_result_args = false;
                            self.propagation_indices = None;
                            // Determine return type for error default
                            let full_type = self.ctx.infer_expr(expr);
                            let inner_type = match &full_type {
                                CrowType::Result(inner) => inner.as_ref().clone(),
                                other => other.clone(),
                            };
                            // Check if the inner call already returns CrowResult
                            let base_name = callee_name.as_deref().unwrap_or("");
                            let base_type = self.ctx.infer_call_base(base_name, args, callee);
                            let already_result = matches!(base_type, CrowType::Result(_));
                            return self.emit_propagation_block(
                                &result_arg_indices,
                                &orig_arg_strs,
                                &inner_expr,
                                &inner_type,
                                already_result,
                            );
                        }
                    }
                }

                if let Expr::Ident(name) = callee.as_ref() {
                    // Builtins
                    if name == "log" {
                        return self.emit_log_call(&arg_strs, args);
                    }

                    // range[stop], range[start, stop], range[start, stop, step]
                    if name == "range" {
                        return match arg_strs.len() {
                            1 => format!("(0..{}).collect::<Vec<i64>>()", arg_strs[0]),
                            2 => format!("({}..{}).collect::<Vec<i64>>()", arg_strs[0], arg_strs[1]),
                            3 => format!("({}..{}).step_by(({}) as usize).collect::<Vec<i64>>()", arg_strs[0], arg_strs[1], arg_strs[2]),
                            _ => "vec![]".to_string(),
                        };
                    }

                    // map[] — empty map constructor
                    if name == "map" && arg_strs.is_empty() {
                        return "BTreeMap::new()".to_string();
                    }

                    // --- grad[] builtin ---
                    if name == "grad" && args.len() == 1 {
                        if let Expr::Ident(fn_name) = &args[0] {
                            let sig = self.ctx.functions.get(fn_name).cloned();
                            if let Some(sig) = sig {
                                let n_params = sig.params.len();
                                let dual_fn = format!("_dual_{}", Self::rust_fn_name(fn_name));
                                let param_names: Vec<String> = sig.params.iter().map(|(p, _)| p.clone()).collect();

                                if n_params == 1 {
                                    // Single param: return f64
                                    return format!(
                                        "|{p}: f64| -> f64 {{ {df}(CrowDual::new({p}, 1.0)).dual }}",
                                        p = param_names[0],
                                        df = dual_fn,
                                    );
                                } else {
                                    // Multiple params: return Vec<f64>
                                    let closure_params: Vec<String> = param_names.iter()
                                        .map(|p| format!("{}: f64", p))
                                        .collect();
                                    let mut body_parts = Vec::new();
                                    for i in 0..n_params {
                                        let call_args: Vec<String> = (0..n_params).map(|j| {
                                            if j == i {
                                                format!("CrowDual::new({}, 1.0)", param_names[j])
                                            } else {
                                                format!("CrowDual::new({}, 0.0)", param_names[j])
                                            }
                                        }).collect();
                                        body_parts.push(format!("{}({}).dual", dual_fn, call_args.join(", ")));
                                    }
                                    return format!(
                                        "|{}| -> Vec<f64> {{ vec![{}] }}",
                                        closure_params.join(", "),
                                        body_parts.join(", "),
                                    );
                                }
                            }
                        }
                    }

                    // --- Module function dispatch ---
                    // Overlapping builtins: check if any arg is NumArray → route to module
                    let is_overlapping = matches!(name.as_str(), "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");
                    if is_overlapping && self.registry.has_function(name) {
                        let arg_types: Vec<_> = args.iter().map(|a| self.ctx.infer_expr(a)).collect();
                        let has_numarray = arg_types.iter().any(|t| matches!(t, CrowType::NumArray(_)));
                        if has_numarray {
                            self.needs_crow_runtime = true;
                            let emitted = match &self.pinned_module {
                                Some(m) => self.registry.emit_rust_in(m, name, &arg_strs, &arg_types),
                                None => self.registry.emit_rust(name, &arg_strs, &arg_types),
                            };
                            return emitted.unwrap_or_else(|e| format!("/* {} */", e));
                        }
                    }
                    // Non-overlapping module functions: route to registry
                    // Skip frame `select` when in UI context with 2 args (it's a UI widget)
                    let skip_frame_select = name == "select" && self.needs_ui && args.len() == 2
                        && self.registry.module_name_for(name).map_or(false, |m| m == "frame");
                    if !is_overlapping && !skip_frame_select && self.registry.has_function(name) {
                        let mut arg_types: Vec<_> = args.iter().map(|a| self.ctx.infer_expr(a)).collect();
                        // Frame columns bridge into tensor math via to_tensor()
                        let mut arg_strs = arg_strs.clone();
                        for (i, t) in arg_types.iter_mut().enumerate() {
                            if matches!(t, CrowType::FrameCol) {
                                arg_strs[i] = format!("({}).to_tensor()", arg_strs[i]);
                                *t = CrowType::NumArray(Box::new(CrowType::Float));
                            }
                        }
                        self.needs_crow_runtime = true;
                        let emitted = match &self.pinned_module {
                            Some(m) => self.registry.emit_rust_in(m, name, &arg_strs, &arg_types),
                            None => self.registry.emit_rust(name, &arg_strs, &arg_types),
                        };
                        return emitted.unwrap_or_else(|e| format!("/* {} */", e));
                    }

                    // --- Math builtins (scalar) ---
                    if name == "abs" && arg_strs.len() == 1 {
                        // In dual mode, CrowDual has .abs()
                        return format!("({}).abs()", arg_strs[0]);
                    }
                    if name == "sqrt" && arg_strs.len() == 1 {
                        if self.emitting_dual {
                            return format!("({}).sqrt()", arg_strs[0]);
                        }
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).sqrt()", arg_strs[0]),
                            _ => format!("(({}) as f64).sqrt()", arg_strs[0]),
                        };
                    }
                    if name == "floor" && arg_strs.len() == 1 {
                        if self.emitting_dual {
                            return format!("({}).floor()", arg_strs[0]);
                        }
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).floor() as i64", arg_strs[0]),
                            _ => arg_strs[0].clone(),
                        };
                    }
                    if name == "ceil" && arg_strs.len() == 1 {
                        if self.emitting_dual {
                            return format!("({}).ceil()", arg_strs[0]);
                        }
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).ceil() as i64", arg_strs[0]),
                            _ => arg_strs[0].clone(),
                        };
                    }
                    if name == "round" && arg_strs.len() == 1 {
                        if self.emitting_dual {
                            return format!("({}).round()", arg_strs[0]);
                        }
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).round() as i64", arg_strs[0]),
                            _ => arg_strs[0].clone(),
                        };
                    }
                    // pow removed — use ^ operator instead
                    if name == "min" && arg_strs.len() == 2 {
                        let t1 = self.ctx.infer_expr(&args[0]);
                        let t2 = self.ctx.infer_expr(&args[1]);
                        return if t1 == CrowType::Float || t2 == CrowType::Float {
                            format!("(({}) as f64).min(({}) as f64)", arg_strs[0], arg_strs[1])
                        } else {
                            format!("std::cmp::min({}, {})", arg_strs[0], arg_strs[1])
                        };
                    }
                    if name == "max" && arg_strs.len() == 2 {
                        let t1 = self.ctx.infer_expr(&args[0]);
                        let t2 = self.ctx.infer_expr(&args[1]);
                        return if t1 == CrowType::Float || t2 == CrowType::Float {
                            format!("(({}) as f64).max(({}) as f64)", arg_strs[0], arg_strs[1])
                        } else {
                            format!("std::cmp::max({}, {})", arg_strs[0], arg_strs[1])
                        };
                    }
                    // --- String builtins ---
                    if name == "split" && arg_strs.len() == 2 {
                        return format!("({}).split(&{}).map(|s| s.to_string()).collect::<Vec<String>>()", arg_strs[0], arg_strs[1]);
                    }
                    if name == "trim" && arg_strs.len() == 1 {
                        return format!("({}).trim().to_string()", arg_strs[0]);
                    }
                    if name == "replace" && arg_strs.len() == 3 {
                        return format!("({}).replace(&{}, &{})", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }
                    if name == "upper" && arg_strs.len() == 1 {
                        return format!("({}).to_uppercase()", arg_strs[0]);
                    }
                    if name == "lower" && arg_strs.len() == 1 {
                        return format!("({}).to_lowercase()", arg_strs[0]);
                    }
                    if name == "join" && arg_strs.len() == 2 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        if matches!(ty, CrowType::NumArray(_)) {
                            return format!("{}.to_vec().iter().map(|v| {{ let i = *v; if i == (i as i64) as f64 && i.is_finite() {{ (i as i64).to_string() }} else {{ i.to_string() }} }}).collect::<Vec<String>>().join(&{})", arg_strs[0], arg_strs[1]);
                        }
                        return format!("{}.iter().map(|s| s.to_string()).collect::<Vec<String>>().join(&{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "starts" && arg_strs.len() == 2 {
                        return format!("({}).starts_with(&*{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "ends" && arg_strs.len() == 2 {
                        return format!("({}).ends_with(&*{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "repeat" && arg_strs.len() == 2 {
                        return format!("({}).repeat(({}) as usize)", arg_strs[0], arg_strs[1]);
                    }
                    if name == "captured" && arg_strs.len() == 1 {
                        self.needs_crow_runtime = true;
                        let _ty = self.ctx.infer_expr(&args[0]);
                        if matches!(_ty, CrowType::Result(_)) {
                            return format!(
                                "{{ let _v = {}; if _v.captured > 0.0 {{ CrowResult {{ ok: true, value: _v.captured, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} else {{ CrowResult {{ ok: false, value: 0.0, reason: \"this value was computed here, not read from outside - it has no capture moment\".to_string(), kind: \"no_capture\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                                arg_strs[0]
                            );
                        }
                        return format!(
                            "{{ let _ = {}; CrowResult {{ ok: false, value: 0.0, reason: \"this value was computed here, not read from outside - it has no capture moment\".to_string(), kind: \"no_capture\".to_string(), trace: vec![], captured: 0.0 }} }}",
                            arg_strs[0]
                        );
                    }
                    if name == "now" && arg_strs.is_empty() {
                        self.needs_crow_runtime = true;
                        return "crow_runtime::moments::now()".to_string();
                    }
                    if name == "moment" && arg_strs.len() == 7 {
                        self.needs_crow_runtime = true;
                        self.needs_time_result = true;
                        return format!(
                            "__crow_time_result(crow_runtime::moments::moment(({}) as i64, ({}) as i64, ({}) as i64, ({}) as i64, ({}) as i64, ({}) as f64, &{}))",
                            arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[3], arg_strs[4], arg_strs[5], arg_strs[6]
                        );
                    }
                    if name == "parse_time" && arg_strs.len() == 2 {
                        self.needs_crow_runtime = true;
                        self.needs_time_result = true;
                        return format!(
                            "__crow_time_result(crow_runtime::moments::parse_time(&{}, &{}))",
                            arg_strs[0], arg_strs[1]
                        );
                    }
                    if name == "calendar" && arg_strs.len() == 2 {
                        self.needs_crow_runtime = true;
                        return format!(
                            "crow_runtime::moments::calendar(({}) as f64, &{}).map(|f| f.iter().map(|(k, v)| (k.to_string(), *v)).collect::<std::collections::BTreeMap<String, i64>>()).unwrap_or_else(|e| {{ eprintln!(\"calendar[]: {{}}\", e); std::collections::BTreeMap::new() }})",
                            arg_strs[0], arg_strs[1]
                        );
                    }
                    if name == "fmt" && arg_strs.len() == 9 {
                        self.needs_crow_runtime = true;
                        // Time display path: the 8th arg (style) is set.
                        if !matches!(&args[7], Expr::Str(s) if s.is_empty()) {
                            return format!(
                                "match crow_runtime::moments::fmt_time(({}) as f64, &{}, &{}) {{ Ok(s) => s, Err(e) => format!(\"[fmt: {{}}]\", e) }}",
                                arg_strs[0], arg_strs[7], arg_strs[8]
                            );
                        }
                        let ty = self.ctx.infer_expr(&args[0]);
                        let inner = match &ty { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                        if matches!(inner, CrowType::Str) {
                            return format!(
                                "crow_runtime::display::crow_fmt_str(&{}, {}, {}, &{}, &{}, {})",
                                arg_strs[0], arg_strs[1], arg_strs[3], arg_strs[4], arg_strs[5], arg_strs[6]
                            );
                        }
                        return format!(
                            "crow_runtime::display::crow_fmt_f64(({}) as f64, {}, {}, {}, &{}, &{}, {})",
                            arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[3], arg_strs[4], arg_strs[5], arg_strs[6]
                        );
                    }
                    // --- Conversion builtins ---
                    if name == "int" && arg_strs.len() == 1 {
                        // Dispatch on the inner type: the auto-propagation
                        // wrapper hands this emission the unwrapped .value
                        let ty = match self.ctx.infer_expr(&args[0]) {
                            CrowType::Result(inner) => *inner,
                            other => other,
                        };
                        return match ty {
                            CrowType::Int => arg_strs[0].clone(),
                            CrowType::Float => format!("({}) as i64", arg_strs[0]),
                            CrowType::Str => format!("{{ let _int_arg = {}; match _int_arg.parse::<i64>() {{ Ok(v) => CrowResult {{ ok: true, value: v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_) => CrowResult {{ ok: false, value: 0_i64, reason: format!(\"cannot convert '{{}}' to int\", _int_arg), kind: \"parse\".to_string(), trace: vec![{}], captured: 0.0 }} }} }}", arg_strs[0], self.trace_format_expr()),
                            CrowType::Bool => format!("if {} {{ 1_i64 }} else {{ 0_i64 }}", arg_strs[0]),
                            _ => format!("({}) as i64", arg_strs[0]),
                        };
                    }
                    if name == "float" && arg_strs.len() == 1 {
                        // Dispatch on the inner type: the auto-propagation
                        // wrapper hands this emission the unwrapped .value
                        let ty = match self.ctx.infer_expr(&args[0]) {
                            CrowType::Result(inner) => *inner,
                            other => other,
                        };
                        return match ty {
                            CrowType::Float => arg_strs[0].clone(),
                            CrowType::Int => format!("({}) as f64", arg_strs[0]),
                            CrowType::Str => format!("{{ let _float_arg = {}; match _float_arg.parse::<f64>() {{ Ok(v) => CrowResult {{ ok: true, value: v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_) => CrowResult {{ ok: false, value: 0.0_f64, reason: format!(\"cannot convert '{{}}' to float\", _float_arg), kind: \"parse\".to_string(), trace: vec![{}], captured: 0.0 }} }} }}", arg_strs[0], self.trace_format_expr()),
                            CrowType::Bool => format!("if {} {{ 1.0_f64 }} else {{ 0.0_f64 }}", arg_strs[0]),
                            _ => format!("({}) as f64", arg_strs[0]),
                        };
                    }
                    if name == "str" && arg_strs.len() == 1 {
                        // Dispatch on the inner type: the auto-propagation
                        // wrapper hands this emission the unwrapped .value
                        let ty = match self.ctx.infer_expr(&args[0]) {
                            CrowType::Result(inner) => *inner,
                            other => other,
                        };
                        return match ty {
                            CrowType::Str => arg_strs[0].clone(),
                            _ => format!("format!(\"{{}}\", {})", arg_strs[0]),
                        };
                    }
                    // --- Collection builtins ---
                    if name == "len" && arg_strs.len() == 1 {
                        let arg_type = self.ctx.infer_expr(&args[0]);
                        if matches!(arg_type, CrowType::Frame) {
                            return format!("({}).nrows() as i64", arg_strs[0]);
                        }
                        // Fallible values count their payload — an error
                        // frame/array has 0 rows/items
                        if let CrowType::Result(inner) = &arg_type {
                            if matches!(inner.as_ref(), CrowType::Frame) {
                                return format!("({}).value.nrows() as i64", arg_strs[0]);
                            }
                            return format!("(({}).value.len() as i64)", arg_strs[0]);
                        }
                        return format!("(({}).len() as i64)", arg_strs[0]);
                    }

                    if name == "contains" && arg_strs.len() == 2 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::NumArray(_) => format!("{}.to_vec().contains(&({} as f64))", arg_strs[0], arg_strs[1]),
                            CrowType::Str => format!("({}).contains(&*{})", arg_strs[0], arg_strs[1]),
                            _ => format!("({}).contains(&{})", arg_strs[0], arg_strs[1]),
                        };
                    }
                    if name == "slice" && arg_strs.len() == 3 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::NumArray(_) => format!("{}.slice(0, ({}) as usize, ({}) as usize)", arg_strs[0], arg_strs[1], arg_strs[2]),
                            CrowType::Str => format!("({})[({}) as usize..({}) as usize].to_string()", arg_strs[0], arg_strs[1], arg_strs[2]),
                            _ => format!("({})[({}) as usize..({}) as usize].to_vec()", arg_strs[0], arg_strs[1], arg_strs[2]),
                        };
                    }
                    // Immutable collection builtins (return new values)
                    if name == "push" && arg_strs.len() == 2 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        if matches!(ty, CrowType::NumArray(_)) {
                            let arg1_ty = self.ctx.infer_expr(&args[1]);
                            let arg1_inner = match &arg1_ty { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                            if matches!(arg1_inner, CrowType::NumArray(_)) {
                                // push[m, row]: append along axis 0, shape-checked
                                return format!("{}.push_row(&{}).unwrap_or_else(|e| panic!(\"{{}}\", e))", arg_strs[0], arg_strs[1]);
                            }
                            return format!("{}.push_scalar({} as f64).unwrap_or_else(|e| panic!(\"{{}}\", e))", arg_strs[0], arg_strs[1]);
                        }
                        return format!("{{ let mut tmp = {}.clone(); tmp.push({}); tmp }}", arg_strs[0], arg_strs[1]);
                    }
                    if name == "pop" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        if matches!(ty, CrowType::NumArray(_)) {
                            return format!("{{ let mut tmp = {}.to_vec(); tmp.pop(); crow_runtime::tensor::CrowTensor::from_vec(tmp.clone(), vec![tmp.len()]) }}", arg_strs[0]);
                        }
                        return format!("{{ let mut tmp = {}.clone(); tmp.pop(); tmp }}", arg_strs[0]);
                    }
                    if name == "sort" {
                        let ty = self.ctx.infer_expr(&args[0]);
                        if matches!(ty, CrowType::NumArray(_)) {
                            return format!("{{ let mut tmp = {}.to_vec(); tmp.sort_by(|a, b| a.partial_cmp(b).unwrap()); crow_runtime::tensor::CrowTensor::from_vec(tmp.clone(), vec![tmp.len()]) }}", arg_strs[0]);
                        }
                        return format!("{{ let mut tmp = {}.clone(); tmp.sort(); tmp }}", arg_strs[0]);
                    }
                    if name == "reverse" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::NumArray(_) => format!("{{ let mut tmp = {}.to_vec(); tmp.reverse(); crow_runtime::tensor::CrowTensor::from_vec(tmp.clone(), vec![tmp.len()]) }}", arg_strs[0]),
                            CrowType::Str => format!("({}).chars().rev().collect::<String>()", arg_strs[0]),
                            _ => format!("{{ let mut tmp = {}.clone(); tmp.reverse(); tmp }}", arg_strs[0]),
                        };
                    }
                    // concat: the single collection-merge operation (replaces
                    // append and the former math-module concat)
                    if name == "concat" && (arg_strs.len() == 2 || arg_strs.len() == 3) {
                        let ty = self.ctx.infer_expr(&args[0]);
                        let ty = if let CrowType::Result(i) = ty { *i } else { ty };
                        if matches!(ty, CrowType::Str) {
                            return format!("format!(\"{{}}{{}}\", {}, {})", arg_strs[0], arg_strs[1]);
                        }
                        if matches!(ty, CrowType::NumArray(_)) {
                            self.needs_crow_runtime = true;
                            let axis = if arg_strs.len() == 3 {
                                format!("{} as usize", arg_strs[2])
                            } else {
                                "0".to_string()
                            };
                            return format!("{}.concat(&{}, {})", arg_strs[0], arg_strs[1], axis);
                        }
                        return format!("{{ let mut tmp = {}.clone(); tmp.extend({}.iter().cloned()); tmp }}", arg_strs[0], arg_strs[1]);
                    }
                    // --- Map builtins ---
                    if name == "keys" && arg_strs.len() == 1 {
                        return format!("{}.keys().cloned().collect::<Vec<String>>()", arg_strs[0]);
                    }
                    if name == "values" && arg_strs.len() == 1 {
                        return format!("{}.values().cloned().collect::<Vec<_>>()", arg_strs[0]);
                    }
                    if name == "has" && arg_strs.len() == 2 {
                        return format!("{}.contains_key(&{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "set" && arg_strs.len() == 3 {
                        // Fn values: cast to the fn-pointer type so distinct
                        // fn items unify inside one map
                        let val = match self.ctx.infer_expr(&args[2]) {
                            CrowType::Fn(_, _) => {
                                let t = self.ctx.infer_expr(&args[2]).to_rust_type();
                                format!("{} as {}", arg_strs[2], t)
                            }
                            _ => arg_strs[2].clone(),
                        };
                        return format!("{{ let mut tmp = {}.clone(); tmp.insert({}, {}); tmp }}", arg_strs[0], arg_strs[1], val);
                    }
                    if name == "del" && arg_strs.len() == 2 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.remove(&{}); tmp }}", arg_strs[0], arg_strs[1]);
                    }
                    // --- CLI / IO builtins ---
                    if name == "args" && arg_strs.is_empty() {
                        return "std::env::args().skip(1).collect::<Vec<String>>()".to_string();
                    }
                    if name == "input" {
                        let trace = self.trace_format_expr();
                        if arg_strs.len() == 1 {
                            return format!("{{ print!(\"{{}}\", {}); use std::io::Write; std::io::stdout().flush().unwrap(); let mut _line = String::new(); match std::io::stdin().read_line(&mut _line) {{ Ok(_) => CrowResult {{ ok: true, value: _line.trim_end().to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: String::new(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }} }}", arg_strs[0], trace);
                        }
                        return format!("{{ let mut _line = String::new(); match std::io::stdin().read_line(&mut _line) {{ Ok(_) => CrowResult {{ ok: true, value: _line.trim_end().to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: String::new(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }} }}", trace);
                    }
                    // --- System builtins ---
                    if name == "rand" {
                        if arg_strs.is_empty() {
                            return "{ use std::collections::hash_map::DefaultHasher; use std::hash::{Hash, Hasher}; let mut h = DefaultHasher::new(); std::time::SystemTime::now().hash(&mut h); std::thread::current().id().hash(&mut h); (h.finish() as f64) / (u64::MAX as f64) }".to_string();
                        } else if arg_strs.len() == 1 {
                            return format!("{{ use std::collections::hash_map::DefaultHasher; use std::hash::{{Hash, Hasher}}; let mut h = DefaultHasher::new(); std::time::SystemTime::now().hash(&mut h); std::thread::current().id().hash(&mut h); (h.finish() % ({} as u64)) as i64 }}", arg_strs[0]);
                        } else if arg_strs.len() == 2 {
                            return format!("{{ use std::collections::hash_map::DefaultHasher; use std::hash::{{Hash, Hasher}}; let mut h = DefaultHasher::new(); std::time::SystemTime::now().hash(&mut h); std::thread::current().id().hash(&mut h); let _min = {}; let _max = {}; _min + (h.finish() % ((_max - _min + 1) as u64)) as i64 }}", arg_strs[0], arg_strs[1]);
                        }
                    }
                    if name == "time" && arg_strs.is_empty() {
                        return "std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()".to_string();
                    }
                    // later[work, …] — background scheduling (skip-if-running).
                    // Handlers/renders: session-scoped via state, delta applied
                    // on the live loop. Scripts/servers: kernel thread + args
                    // captured by value; main waits for stragglers at exit.
                    if name == "later" && !args.is_empty() {
                        if let Expr::Ident(fname) = &args[0] {
                            let f = &self.current_function_name;
                            let in_live = self.reactive_event_handlers.contains(f)
                                || self.reactive_render_fns.contains(f);
                            if in_live {
                                return format!("{{ let _ = state.spawn_later(\"{}\", {}); }}", fname, fname);
                            }
                            self.needs_later = true;
                            self.needs_crow_runtime = true;
                            let mut captures = String::new();
                            let mut params: Vec<String> = Vec::new();
                            for (i, a) in arg_strs.iter().skip(1).enumerate() {
                                captures.push_str(&format!("let __l{} = ({}).clone(); ", i, a));
                                params.push(format!("__l{}", i));
                            }
                            return format!(
                                "{{ {}let _ = crow_runtime::later::spawn(\"{}\", move || {{ let _ = {}({}); }}); }}",
                                captures,
                                fname,
                                fname,
                                params.join(", ")
                            );
                        }
                    }
                    if name == "running" && args.len() == 1 {
                        if let Expr::Ident(fname) = &args[0] {
                            let f = &self.current_function_name;
                            let in_live = self.reactive_event_handlers.contains(f)
                                || self.reactive_render_fns.contains(f);
                            if in_live {
                                return format!("state.later_running(\"{}\")", fname);
                            }
                            self.needs_later = true;
                            self.needs_crow_runtime = true;
                            return format!("crow_runtime::later::running(\"{}\")", fname);
                        }
                    }
                    if name == "sleep" && arg_strs.len() == 1 {
                        let arg_type = self.ctx.infer_expr(&args[0]);
                        return match arg_type {
                            CrowType::Int => format!("std::thread::sleep(std::time::Duration::from_millis({} as u64))", arg_strs[0]),
                            _ => format!("std::thread::sleep(std::time::Duration::from_secs_f64({}))", arg_strs[0]),
                        };
                    }
                    if name == "env" {
                        if arg_strs.len() == 2 {
                            return format!("std::env::var({}).unwrap_or_else(|_| {}.to_string())", arg_strs[0], arg_strs[1]);
                        } else if arg_strs.len() == 1 {
                            return format!("std::env::var({}).unwrap_or_default()", arg_strs[0]);
                        }
                    }
                    if name == "exit" {
                        if arg_strs.is_empty() {
                            return "std::process::exit(0)".to_string();
                        } else if arg_strs.len() == 1 {
                            return format!("std::process::exit({} as i32)", arg_strs[0]);
                        }
                    }
                    // --- Assert ---
                    if name == "assert" && (arg_strs.len() == 1 || arg_strs.len() == 2) {
                        let ty = self.ctx.infer_expr(&args[0]);
                        let cond = match ty {
                            CrowType::Bool => arg_strs[0].clone(),
                            // An assert is a decision: a fallible condition is
                            // guarded like if/while — error halts with reason
                            CrowType::Result(_) => {
                                format!("{{ let _a = {}; if !_a.ok {{ panic!(\"cannot branch on an error value — {{}}\", _a.reason); }} _a.value }}", arg_strs[0])
                            }
                            CrowType::Int => format!("({}) != 0", arg_strs[0]),
                            CrowType::Str => format!("!({}).is_empty()", arg_strs[0]),
                            _ => format!("({}) != 0", arg_strs[0]),
                        };
                        if arg_strs.len() == 2 {
                            return format!("if !({}) {{ panic!(\"assertion failed: {{}}\", {}); }}", cond, arg_strs[1]);
                        } else {
                            return format!("assert!({})", cond);
                        }
                    }
                    // --- Error builtin ---
                    if name == "error" && (arg_strs.len() == 1 || arg_strs.len() == 2) {
                        let trace = self.trace_format_expr();
                        let kind_expr = if arg_strs.len() == 2 {
                            format!("{}.to_string()", arg_strs[1])
                        } else {
                            "String::new()".to_string()
                        };
                        // Determine the default value for the Result's inner type
                        let default_val = if let Some(CrowType::Result(inner)) = &self.current_fn_return_type {
                            inner.default_rust_value()
                        } else {
                            "String::new()".to_string()
                        };
                        return format!("CrowResult {{ ok: false, value: {}, reason: {}.to_string(), kind: {}, trace: vec![{}], captured: 0.0 }}", default_val, arg_strs[0], kind_expr, trace);
                    }
                    // --- Type builtin ---
                    if name == "type" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        let type_str = match ty {
                            CrowType::Int => "\"int\"".to_string(),
                            CrowType::Float => "\"float\"".to_string(),
                            CrowType::Str => "\"str\"".to_string(),
                            CrowType::Bool => "\"bool\"".to_string(),
                            CrowType::Array(_) | CrowType::NumArray(_) => "\"array\"".to_string(),
                            CrowType::Sparse => "\"sparse\"".to_string(),
                            CrowType::Map(_) => "\"map\"".to_string(),
                            CrowType::Fn(_, _) => "\"fn\"".to_string(),
                            CrowType::Void => "\"none\"".to_string(),
                            CrowType::Struct(name) => format!("\"{}\"", name),
                            CrowType::Result(_) => format!("{{ let _v = &{}; if _v.ok {{ \"{}\" }} else {{ \"error\" }} }}", arg_strs[0], {
                                match &ty {
                                    CrowType::Result(inner) => match inner.as_ref() {
                                        CrowType::Int => "int",
                                        CrowType::Float => "float",
                                        CrowType::Str => "str",
                                        CrowType::Bool => "bool",
                                        _ => "unknown",
                                    },
                                    _ => "unknown",
                                }
                            }),
                            _ => "\"unknown\"".to_string(),
                        };
                        return format!("{}.to_string()", type_str);
                    }
                    // --- File I/O builtins ---
                    if name == "read" && arg_strs.len() == 1 {
                        let trace = self.trace_format_expr();
                        return format!("match std::fs::read_to_string(&*{}) {{ Ok(v) => CrowResult {{ ok: true, value: v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: String::new(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], trace);
                    }
                    if name == "write" && arg_strs.len() == 2 {
                        let trace = self.trace_format_expr();
                        return format!("match std::fs::write(&*{}, &*{}) {{ Ok(_) => CrowResult {{ ok: true, value: \"\".to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: \"\".to_string(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], arg_strs[1], trace);
                    }
                    // --- File system builtins ---
                    if name == "exists" && arg_strs.len() == 1 {
                        return format!("std::path::Path::new(&*{}).exists()", arg_strs[0]);
                    }
                    if name == "mkdir" && arg_strs.len() == 1 {
                        let trace = self.trace_format_expr();
                        return format!("match std::fs::create_dir_all(&*{}) {{ Ok(_) => CrowResult {{ ok: true, value: \"\".to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: \"\".to_string(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], trace);
                    }
                    if name == "ls" && arg_strs.len() == 1 {
                        let trace = self.trace_format_expr();
                        return format!("match std::fs::read_dir(&*{}) {{ Ok(entries) => {{ let mut names: Vec<String> = entries.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string())).collect(); names.sort(); CrowResult {{ ok: true, value: names, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }}, Err(e) => CrowResult {{ ok: false, value: Vec::<String>::new(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], trace);
                    }
                    if name == "rm" && arg_strs.len() == 1 {
                        let trace = self.trace_format_expr();
                        return format!("{{ let _s = {}; let _p = std::path::Path::new(&*_s); let _r = if _p.is_dir() {{ std::fs::remove_dir_all(_p) }} else {{ std::fs::remove_file(_p) }}; match _r {{ Ok(_) => CrowResult {{ ok: true, value: \"\".to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: \"\".to_string(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }} }}", arg_strs[0], trace);
                    }
                    if name == "mv" && arg_strs.len() == 2 {
                        let trace = self.trace_format_expr();
                        return format!("match std::fs::rename(&*{}, &*{}) {{ Ok(_) => CrowResult {{ ok: true, value: \"\".to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: \"\".to_string(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], arg_strs[1], trace);
                    }
                    if name == "cp" && arg_strs.len() == 2 {
                        let trace = self.trace_format_expr();
                        return format!("match std::fs::copy(&*{}, &*{}) {{ Ok(_) => CrowResult {{ ok: true, value: \"\".to_string(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(e) => CrowResult {{ ok: false, value: \"\".to_string(), reason: e.to_string(), kind: \"io\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], arg_strs[1], trace);
                    }
                    // --- JSON builtins ---
                    if name == "json" && arg_strs.len() == 1 {
                        self.needs_serde_json = true;
                        let trace = self.trace_format_expr();
                        return format!("match serde_json::from_str::<serde_json::Value>(&*{}) {{ Ok(v) => if v.is_null() {{ CrowResult {{ ok: false, value: serde_json::Value::Null, reason: \"null\".to_string(), kind: \"parse\".to_string(), trace: vec![{}], captured: 0.0 }} }} else {{ CrowResult {{ ok: true, value: v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }}, Err(e) => CrowResult {{ ok: false, value: serde_json::Value::Null, reason: format!(\"invalid JSON: {{}}\", e), kind: \"parse\".to_string(), trace: vec![{}], captured: 0.0 }} }}", arg_strs[0], trace, trace);
                    }
                    if name == "encode" && arg_strs.len() == 1 {
                        self.needs_serde_json = true;
                        // Numeric arrays are CrowTensor (f64) — serialize whole
                        // numbers as ints to match the interpreter's output
                        let arg_ty = self.ctx.infer_expr(&args[0]);
                        let arg_inner = match &arg_ty { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                        if matches!(arg_inner, CrowType::NumArray(_)) {
                            return format!("{{ let _vals: Vec<serde_json::Value> = {}.to_vec().iter().map(|&_f| if _f == (_f as i64) as f64 && _f.is_finite() {{ serde_json::json!(_f as i64) }} else {{ serde_json::json!(_f) }}).collect(); CrowResult {{ ok: true, value: serde_json::to_string(&_vals).unwrap_or_default(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }}", arg_strs[0]);
                        }
                        return format!("CrowResult {{ ok: true, value: serde_json::to_string(&{}).unwrap_or_default(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", arg_strs[0]);
                    }

                    // --- Shell builtin ---
                    if name == "shell" && arg_strs.len() == 1 {
                        self.needs_shell_struct = true;
                        return format!("{{ let _cmd = &*{}; match std::process::Command::new(\"sh\").arg(\"-c\").arg(_cmd).output() {{ Ok(out) => CrowShell {{ status: out.status.code().unwrap_or(-1) as i64, stdout: String::from_utf8_lossy(&out.stdout).trim_end().to_string(), stderr: String::from_utf8_lossy(&out.stderr).trim_end().to_string() }}, Err(e) => panic!(\"shell failed: {{}}\", e) }} }}", arg_strs[0]);
                    }
                    // --- Hashing builtin ---
                    if name == "hash" && arg_strs.len() == 2 {
                        self.needs_sha2 = true;
                        return format!("{{ let _s = &*{}; let _algo = &*{}; match _algo {{ \"sha256\" => {{ use sha2::Digest; format!(\"{{:x}}\", sha2::Sha256::digest(_s.as_bytes())) }}, \"sha512\" => {{ use sha2::Digest; format!(\"{{:x}}\", sha2::Sha512::digest(_s.as_bytes())) }}, \"md5\" => {{ use md5::Digest; format!(\"{{:x}}\", md5::Md5::digest(_s.as_bytes())) }}, _ => panic!(\"unsupported hash algorithm '{{}}'\", _algo) }} }}", arg_strs[0], arg_strs[1]);
                    }
                    // --- Search builtins: parallel early-exit via rayon ---
                    if (name == "any" || name == "all") && arg_strs.len() == 2 {
                        let arr_ty = self.ctx.infer_expr(&args[0]);
                        let arr_ty = if let CrowType::Result(i) = arr_ty { *i } else { arr_ty };
                        self.needs_rayon = true;
                        let method = if name == "any" { "any" } else { "all" };
                        if let CrowType::NumArray(inner) = &arr_ty {
                            let cast = if matches!(inner.as_ref(), CrowType::Int) { " as i64" } else { "" };
                            return format!("{}.to_vec().par_iter().{}(|&_x| {}(_x{}))", arg_strs[0], method, arg_strs[1], cast);
                        }
                        let by_ref = self.pred_takes_ref(&args[1]);
                        let call = if by_ref { "(_x)" } else { "(_x.clone())" };
                        return format!("{}.par_iter().{}(|_x| {}{})", arg_strs[0], method, arg_strs[1], call);
                    }
                    if name == "find" && arg_strs.len() == 2 {
                        let arr_ty = self.ctx.infer_expr(&args[0]);
                        let arr_ty = if let CrowType::Result(i) = arr_ty { *i } else { arr_ty };
                        // find[array, predicate] → parallel first match as a Result
                        let elem = match &arr_ty {
                            CrowType::Array(inner) => Some(inner.as_ref().clone()),
                            CrowType::NumArray(inner) => Some(inner.as_ref().clone()),
                            _ => None,
                        };
                        if let Some(elem) = elem {
                            self.needs_rayon = true;
                            let default = elem.default_rust_value();
                            if matches!(arr_ty, CrowType::NumArray(_)) {
                                let cast = if matches!(elem, CrowType::Int) { " as i64" } else { "" };
                                return format!(
                                    "{{ match {}.to_vec().par_iter().copied().find_first(|&_x| {}(_x{})) {{ Some(_v) => CrowResult {{ ok: true, value: _v{}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, None => CrowResult {{ ok: false, value: {}, reason: \"no match\".to_string(), kind: \"find\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                                    arg_strs[0], arg_strs[1], cast, cast, default
                                );
                            }
                            let by_ref = self.pred_takes_ref(&args[1]);
                            let call = if by_ref { "(*_x)" } else { "((*_x).clone())" };
                            return format!(
                                "{{ match {}.par_iter().find_first(|_x| {}{}) {{ Some(_v) => CrowResult {{ ok: true, value: _v.clone(), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, None => CrowResult {{ ok: false, value: {}, reason: \"no match\".to_string(), kind: \"find\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                                arg_strs[0], arg_strs[1], call, default
                            );
                        }
                        // fall through to the regex find below
                    }

                    // --- Regex builtins ---
                    if name == "matches" && arg_strs.len() == 2 {
                        self.needs_regex = true;
                        return format!("regex::Regex::new(&*{}).unwrap().is_match(&*{})", arg_strs[1], arg_strs[0]);
                    }
                    if name == "find" && arg_strs.len() == 2 {
                        self.needs_regex = true;
                        self.needs_find_struct = true;
                        return format!("{{ let _re = regex::Regex::new(&*{}).unwrap(); let _s = &*{}; if let Some(_caps) = _re.captures(&_s) {{ let _groups: Vec<String> = _caps.iter().map(|m| m.map_or(String::new(), |m| m.as_str().to_string())).collect(); CrowFindResult {{ ok: true, groups: _groups }} }} else {{ CrowFindResult {{ ok: false, groups: Vec::new() }} }} }}", arg_strs[1], arg_strs[0]);
                    }
                    if name == "sub" && arg_strs.len() == 3 {
                        self.needs_regex = true;
                        return format!("regex::Regex::new(&*{}).unwrap().replace_all(&*{}, &*{}).to_string()", arg_strs[1], arg_strs[0], arg_strs[2]);
                    }
                    // --- Date/time builtins ---
                    if name == "now" && arg_strs.is_empty() {
                        self.needs_chrono = true;
                        return format!("{{ let _now = chrono::Local::now(); let mut _m = std::collections::BTreeMap::new(); _m.insert(\"year\".to_string(), _now.format(\"%Y\").to_string().parse::<i64>().unwrap()); _m.insert(\"month\".to_string(), _now.format(\"%m\").to_string().parse::<i64>().unwrap()); _m.insert(\"day\".to_string(), _now.format(\"%d\").to_string().parse::<i64>().unwrap()); _m.insert(\"hour\".to_string(), _now.format(\"%H\").to_string().parse::<i64>().unwrap()); _m.insert(\"minute\".to_string(), _now.format(\"%M\").to_string().parse::<i64>().unwrap()); _m.insert(\"second\".to_string(), _now.format(\"%S\").to_string().parse::<i64>().unwrap()); _m.insert(\"timestamp\".to_string(), _now.timestamp()); _m }}");
                    }
                    if name == "date" && arg_strs.len() == 3 {
                        self.needs_chrono = true;
                        return format!("{{ let _y = {} as i32; let _m = {} as u32; let _d = {} as u32; let _dt = chrono::NaiveDate::from_ymd_opt(_y, _m, _d).expect(\"invalid date\"); let mut _map = std::collections::BTreeMap::new(); _map.insert(\"year\".to_string(), _y as i64); _map.insert(\"month\".to_string(), _m as i64); _map.insert(\"day\".to_string(), _d as i64); _map.insert(\"timestamp\".to_string(), _dt.and_hms_opt(0,0,0).unwrap().and_utc().timestamp()); _map }}", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }
                    if name == "elapsed" && arg_strs.len() == 2 {
                        self.needs_elapsed_struct = true;
                        return format!("{{ let _t1 = {}.get(\"timestamp\").copied().unwrap_or(0i64); let _t2 = {}.get(\"timestamp\").copied().unwrap_or(0i64); let _d = (_t2 - _t1) as f64; CrowElapsed {{ seconds: _d, minutes: _d / 60.0, hours: _d / 3600.0, days: _d / 86400.0 }} }}", arg_strs[0], arg_strs[1]);
                    }

                    // --- UI element builtins (use ui) ---
                    if self.needs_ui {
                        // HTML element names → CrowVNode builder calls
                        const HTML_ELEMENTS: &[&str] = &[
                            "div", "span", "p", "h1", "h2", "h3", "h4", "h5", "h6",
                            "button", "ul", "ol", "li", "form", "a", "nav",
                            "header", "footer", "section", "tr", "td", "th",
                            "thead", "tbody", "option", "textarea", "label",
                            "pre", "code",
                        ];
                        // Void elements (no children)
                        const VOID_ELEMENTS: &[&str] = &["input", "img"];

                        let is_html_el = HTML_ELEMENTS.contains(&name.as_str());
                        let is_void_el = VOID_ELEMENTS.contains(&name.as_str());

                        if is_html_el || is_void_el {
                            // Rust method name: "main" → "main_el" (keyword conflict)
                            let rust_name = if name == "main" { "main_el" } else { name.as_str() };

                            // Separate first Map arg (attrs) from children
                            let (attr_pairs, child_args): (Vec<(&Expr, &Expr)>, Vec<&Expr>) =
                                if !args.is_empty() {
                                    if let Expr::Map(pairs) = &args[0] {
                                        let attrs: Vec<(&Expr, &Expr)> = pairs.iter().map(|(k, v)| (k, v)).collect();
                                        let children: Vec<&Expr> = args[1..].iter().collect();
                                        (attrs, children)
                                    } else {
                                        (vec![], args.iter().collect())
                                    }
                                } else {
                                    (vec![], vec![])
                                };

                            // Build children as CrowVNode expressions
                            let child_strs: Vec<String> = child_args.iter().map(|child| {
                                match child {
                                    Expr::Str(_) => {
                                        // String literal → CrowVNode::text(...)
                                        let s_str = self.expr_to_string(child);
                                        format!("CrowVNode::text(&{})", s_str)
                                    }
                                    _ => {
                                        // Any string/number-typed expression is
                                        // a text node (h3[label] with label a
                                        // param); trees pass through
                                        let ty = self.ctx.infer_expr(child);
                                        let inner = match &ty {
                                            CrowType::Result(i) => i.as_ref().clone(),
                                            o => o.clone(),
                                        };
                                        let c_str = self.expr_to_string(child);
                                        match inner {
                                            CrowType::Str => format!("CrowVNode::text(&{})", c_str),
                                            CrowType::Int | CrowType::Float | CrowType::Bool => {
                                                format!("CrowVNode::text(&format!(\"{{}}\", {}))", c_str)
                                            }
                                            _ => c_str,
                                        }
                                    }
                                }
                            }).collect();

                            // Build the element call
                            let base = if is_void_el {
                                format!("CrowVNode::{}()", rust_name)
                            } else {
                                format!("CrowVNode::{}(vec![{}])", rust_name, child_strs.join(", "))
                            };

                            // Chain .attr() and .on_*() calls from the Map
                            let mut result = base;
                            for (k, v) in &attr_pairs {
                                let key_str = match k {
                                    Expr::Ident(s) => s.clone(),
                                    Expr::Str(s) => s.clone(),
                                    _ => self.expr_to_string(k),
                                };
                                // Check if value is a function reference for event handlers
                                let is_fn_ref = matches!(v, Expr::Ident(fn_name) if self.reactive_event_handlers.contains(fn_name));
                                let event_name = if is_fn_ref {
                                    if let Expr::Ident(fn_name) = v { format!("\"{}\"", fn_name) } else { self.expr_to_string(v) }
                                } else {
                                    self.expr_to_string(v)
                                };
                                match key_str.as_str() {
                                    "click" => result = if is_fn_ref { format!("{}.on_click({})", result, event_name) } else { format!("{}.on_click(&{})", result, event_name) },
                                    "input" => result = if is_fn_ref { format!("{}.on_input({})", result, event_name) } else { format!("{}.on_input(&{})", result, event_name) },
                                    "submit" => result = if is_fn_ref { format!("{}.on_submit({})", result, event_name) } else { format!("{}.on_submit(&{})", result, event_name) },
                                    _ => result = format!("{}.attr(&{}, &{})", result, self.expr_to_string(k), self.expr_to_string(v)),
                                }
                            }

                            return result;
                        }
                    }

                    // --- Web builtins ---
                    if name == "route" && self.needs_crow_web {
                        // No-app forms (implicit __crow_app, like page[]/serve[])
                        if arg_strs.len() == 2 {
                            // route[path, handler]
                            return format!("__crow_app = __crow_app.route(&{}, Arc::new({}))", arg_strs[0], arg_strs[1]);
                        }
                        if arg_strs.len() == 3 && matches!(self.ctx.infer_expr(&args[0]), CrowType::Str) {
                            // route[path, method, handler]
                            return format!("__crow_app = __crow_app.route_method(&{}, {}, Arc::new({}))", arg_strs[0], arg_strs[1], arg_strs[2]);
                        }
                        if arg_strs.len() == 3 {
                            // legacy route[app, path, handler]
                            return format!("{}.route(&{}, Arc::new({}))", arg_strs[0], arg_strs[1], arg_strs[2]);
                        }
                        if arg_strs.len() == 4 {
                            // legacy route[app, path, method, handler]
                            return format!("{}.route_method(&{}, {}, Arc::new({}))", arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[3]);
                        }
                    }
                    if name == "serve" && self.needs_crow_web && arg_strs.len() == 1 {
                        // serve[port] → implicit app: __crow_app.serve(port as u16).await.unwrap()
                        return format!("__crow_app.serve({} as u16).await.unwrap()", arg_strs[0]);
                    }
                    if name == "serve" && self.needs_crow_web && arg_strs.len() == 3 {
                        // serve[port, refresh, budget] (lowered form, 0 = unset):
                        // configure the live runtime, then serve.
                        if self.has_page_routes {
                            return format!(
                                "{{ crow_web::live::configure({} as i64, {} as i64); __crow_app.serve({} as u16).await.unwrap() }}",
                                arg_strs[1], arg_strs[2], arg_strs[0]
                            );
                        }
                        return format!("__crow_app.serve({} as u16).await.unwrap()", arg_strs[0]);
                    }
                    if name == "respond" && self.needs_crow_web && arg_strs.len() == 2 {
                        // respond[status, body] → CrowResponse::respond(status as u16, body)
                        return format!("CrowResponse::respond({} as u16, {})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "redirect" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("CrowResponse::redirect({})", arg_strs[0]);
                    }
                    if name == "html" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("CrowResponse::html({})", arg_strs[0]);
                    }
                    if name == "text" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("CrowResponse::text({})", arg_strs[0]);
                    }
                    // json[] with a Map argument in web context → CrowResponse::json()
                    if name == "json" && self.needs_crow_web && arg_strs.len() == 1 {
                        let arg_type = self.ctx.infer_expr(&args[0]);
                        if matches!(arg_type, CrowType::Map(_)) {
                            self.needs_serde_json = true;
                            // Convert BTreeMap to serde_json::Value then wrap in CrowResponse
                            return format!("CrowResponse::json(serde_json::to_value(&{}).unwrap())", arg_strs[0]);
                        }
                    }
                    // msgpack[value] → CrowResponse::msgpack(rmp_serde::to_vec(&serde_json::to_value(&val).unwrap()).unwrap())
                    if name == "msgpack" && self.needs_crow_web && arg_strs.len() == 1 {
                        self.needs_serde_json = true;
                        self.web_features.insert("msgpack".to_string());
                        return format!(
                            "CrowResponse::msgpack(rmp_serde::to_vec(&serde_json::to_value(&{}).unwrap()).unwrap())",
                            arg_strs[0]
                        );
                    }
                    // arrow[frame] → CrowResponse::arrow_ipc(crow_runtime::frame::to_arrow_ipc(&frame))
                    if name == "arrow" && self.needs_crow_web && arg_strs.len() == 1 {
                        self.web_features.insert("arrow_ipc".to_string());
                        self.needs_crow_runtime = true;
                        return format!(
                            "CrowResponse::arrow_ipc(crow_runtime::frame::to_arrow_ipc(&{}))",
                            arg_strs[0]
                        );
                    }
                    // bytes[data] → CrowResponse::binary(200, data, "application/octet-stream")
                    if name == "bytes" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!(
                            "CrowResponse::binary(200, {}.into_bytes(), \"application/octet-stream\")",
                            arg_strs[0]
                        );
                    }
                    // stream[fn[send] ...] → streaming response using channel + Body::from_stream
                    if name == "stream" && self.needs_crow_web && arg_strs.len() == 1 {
                        self.web_features.insert("stream".to_string());
                        // The arg is a closure fn[send] ... — we emit channel-based streaming
                        return format!(
                            "{{ let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(256); \
                            tokio::spawn(async move {{ \
                            let send = |data: String| {{ let tx = tx.clone(); tokio::task::block_in_place(|| {{ tx.blocking_send(Ok(data)).ok(); }}); }}; \
                            ({})(&send); \
                            }}); \
                            let stream = tokio_stream::wrappers::ReceiverStream::new(rx); \
                            let body = axum::body::Body::from_stream(stream); \
                            CrowResponse {{ status: 200, headers: std::collections::HashMap::from([(\"content-type\".to_string(), \"application/octet-stream\".to_string())]), body: String::new(), binary_body: None }} }}",
                            arg_strs[0]
                        );
                    }
                    // --- db module builtins (top-level `use db`) ---
                    if name == "connect" && self.needs_crow_db && !self.needs_crow_web {
                        // connect[url] or connect[url, pool_size]
                        self.note_db_backend(&arg_strs[0]);
                        if arg_strs.len() == 1 {
                            return format!("crow_db::connect(&{}, None).unwrap()", arg_strs[0]);
                        } else if arg_strs.len() == 2 {
                            return format!("crow_db::connect(&{}, Some({} as usize)).unwrap()", arg_strs[0], arg_strs[1]);
                        }
                    }
                    if name == "query" && self.needs_crow_db && arg_strs.len() >= 2 {
                        if arg_strs.len() == 2 {
                            return format!("{{ match {}.query(&{}, &[{}]) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], "");
                        } else {
                            let params = self.db_params_to_json(&args[2..]);
                            return format!("{{ match {}.query(&{}, &[{}]) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], params);
                        }
                    }
                    if name == "exec" && self.needs_crow_db && arg_strs.len() >= 2 {
                        if arg_strs.len() == 2 {
                            return format!("{{ match {}.exec(&{}, &[{}]) {{ Ok(_n) => CrowResult {{ ok: true, value: _n, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: 0_i64, reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], "");
                        } else {
                            let params = self.db_params_to_json(&args[2..]);
                            return format!("{{ match {}.exec(&{}, &[{}]) {{ Ok(_n) => CrowResult {{ ok: true, value: _n, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: 0_i64, reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], params);
                        }
                    }
                    if name == "insert" && self.needs_crow_db && arg_strs.len() == 3 {
                        return format!("{}.insert(&{}, &{}).unwrap()", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }
                    if name == "migrate" && self.needs_crow_db && arg_strs.len() == 2 {
                        return format!("{}.migrate(&{}).unwrap()", arg_strs[0], arg_strs[1]);
                    }
                    if name == "transact" && self.needs_crow_db && arg_strs.len() == 2 {
                        // transact[conn, fn] → conn.transact(&|tx| { fn(tx); Ok(()) })
                        return format!("{}.transact(&|_tx| {{ {}(_tx); Ok(()) }}).unwrap()", arg_strs[0], arg_strs[1]);
                    }

                    // Web.db builtins (legacy web.db module)
                    if name == "sqlite" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("CrowSqlite::open({})", arg_strs[0]);
                    }
                    if name == "exec" && self.needs_crow_web && arg_strs.len() >= 2 {
                        if arg_strs.len() == 2 {
                            return format!("{}.execute({}, &[])", arg_strs[0], arg_strs[1]);
                        } else {
                            let params: Vec<String> = arg_strs[2..].to_vec();
                            return format!("{}.execute({}, &[{}])", arg_strs[0], arg_strs[1], params.join(", "));
                        }
                    }
                    if name == "query" && self.needs_crow_web && arg_strs.len() >= 2 {
                        if arg_strs.len() == 2 {
                            return format!("{}.query({}, &[])", arg_strs[0], arg_strs[1]);
                        } else {
                            let params: Vec<String> = arg_strs[2..].to_vec();
                            return format!("{}.query({}, &[{}])", arg_strs[0], arg_strs[1], params.join(", "));
                        }
                    }
                    // Web.auth builtins
                    if name == "sign" && self.needs_crow_web && arg_strs.len() == 2 {
                        return format!("CrowJwt::sign({}, {})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "verify" && self.needs_crow_web && arg_strs.len() == 2 {
                        return format!("CrowJwt::verify({}, {})", arg_strs[0], arg_strs[1]);
                    }

                    // --- Web P4 builtins ---

                    // cors[app] → app.cors_permissive()
                    // cors[app, origins] → app.cors(CrowCors::origins(origins))
                    if name == "cors" && self.needs_crow_web {
                        if arg_strs.len() == 1 {
                            return format!("{}.cors_permissive()", arg_strs[0]);
                        }
                        if arg_strs.len() == 2 {
                            return format!("{}.cors(CrowCors::origins({}))", arg_strs[0], arg_strs[1]);
                        }
                    }

                    // sessions[] → CrowSessionStore::new()
                    if name == "sessions" && self.needs_crow_web && arg_strs.is_empty() {
                        return "CrowSessionStore::new()".to_string();
                    }

                    // templates[glob] → CrowTemplates::from_glob(glob).unwrap()
                    if name == "templates" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("CrowTemplates::from_glob({}).unwrap()", arg_strs[0]);
                    }

                    // render[t, name, data] → t.render(name, serde_json::to_value(&data).unwrap()).unwrap()
                    if name == "render" && self.needs_crow_web && arg_strs.len() == 3 {
                        self.needs_serde_json = true;
                        return format!("{}.render({}, serde_json::to_value(&{}).unwrap()).unwrap()", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }

                    // cache[max] → CrowCache::new(max as u64)
                    // cache[max, ttl] → CrowCache::with_ttl(max as u64, ttl as u64)
                    if name == "cache" && self.needs_crow_web {
                        if arg_strs.len() == 1 {
                            return format!("CrowCache::new({} as u64)", arg_strs[0]);
                        }
                        if arg_strs.len() == 2 {
                            return format!("CrowCache::with_ttl({} as u64, {} as u64)", arg_strs[0], arg_strs[1]);
                        }
                    }

                    // limit[max, window] → CrowRateLimiter::new(max as u64, window as u64)
                    if name == "limit" && self.needs_crow_web && arg_strs.len() == 2 {
                        return format!("CrowRateLimiter::new({} as u64, {} as u64)", arg_strs[0], arg_strs[1]);
                    }

                    // push[queue, fn] → queue.push(fn)
                    if name == "push" && self.needs_crow_web && arg_strs.len() == 2 {
                        return format!("{}.push({})", arg_strs[0], arg_strs[1]);
                    }

                    // uploads[req] → req.uploads().await
                    if name == "uploads" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("{}.uploads().await", arg_strs[0]);
                    }

                    // sse[app, path, handler] → app.sse_route(path, Arc::new(handler))
                    if name == "sse" && self.needs_crow_web && arg_strs.len() == 3 {
                        return format!("{}.sse_route(&{}, Arc::new({}))", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }

                    // ws[app, path, handler] → app.ws_route(path, Arc::new(handler))
                    if name == "ws" && self.needs_crow_web && arg_strs.len() == 3 {
                        return format!("{}.ws_route(&{}, Arc::new({}))", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }

                    // connect[url] → ws::connect(url)
                    if name == "connect" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("ws::connect(&{})", arg_strs[0]);
                    }

                    // send[sock, msg] → sock.send(&msg)
                    if name == "send" && self.needs_crow_web && arg_strs.len() == 2 {
                        return format!("{}.send(&{})", arg_strs[0], arg_strs[1]);
                    }

                    // recv[sock] → sock.recv().unwrap_or_default()
                    if name == "recv" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("{}.recv().unwrap_or_default()", arg_strs[0]);
                    }

                    // close[sock] → sock.close()
                    if name == "close" && self.needs_crow_web && arg_strs.len() == 1 {
                        return format!("{}.close()", arg_strs[0]);
                    }

                    // page[path, render, handle] or page[path, render] → implicit app: merge LiveView into __crow_app
                    if name == "page" && self.needs_crow_web && (arg_strs.len() == 2 || arg_strs.len() == 3) {
                        if arg_strs.len() == 3 {
                            return format!(
                                "{{ let _ws_path = format!(\"/_ws{{}}\", {}); let _lv = CrowLiveView::new(&_ws_path, __crow_state.clone(), {}, {}); let _lr = _lv.into_router(&{}); __crow_app = __crow_app.merge(_lr); }}",
                                arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[0]
                            );
                        } else if !self.reactive_event_handlers.is_empty() {
                            // page[path, render] with reactive handlers — auto-inject __crow_handle
                            return format!(
                                "{{ let _ws_path = format!(\"/_ws{{}}\", {}); let _lv = CrowLiveView::new(&_ws_path, __crow_state.clone(), {}, __crow_handle); let _lr = _lv.into_router(&{}); __crow_app = __crow_app.merge(_lr); }}",
                                arg_strs[0], arg_strs[1], arg_strs[0]
                            );
                        } else {
                            // page[path, render] — no handle function, use a no-op handler
                            return format!(
                                "{{ let _ws_path = format!(\"/_ws{{}}\", {}); let _lv = CrowLiveView::new(&_ws_path, __crow_state.clone(), {}, |_: &mut CrowLiveState, _: &str, _: Option<&str>| {{}}); let _lr = _lv.into_router(&{}); __crow_app = __crow_app.merge(_lr); }}",
                                arg_strs[0], arg_strs[1], arg_strs[0]
                            );
                        }
                    }

                    // static_dir[app, url_path, dir_path] → app.static_dir(url_path, dir_path)
                    if name == "static_dir" && self.needs_crow_web && arg_strs.len() == 3 {
                        return format!("{}.static_dir(&{}, &{})", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }

                    // smtp[host, port, user, pass] → CrowMailer::new(host, port, user, pass).unwrap()
                    if name == "smtp" && self.needs_crow_web && arg_strs.len() == 4 {
                        return format!("CrowMailer::new(&{}, {} as u16, &{}, &{}).unwrap()", arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[3]);
                    }

                    // --- UI reactive builtins ---
                    // fetch[url] → register_fetch was handled at assignment level; expression fallback
                    if name == "fetch" && arg_strs.len() == 1 {
                        return format!("{{ __crow_state.register_fetch(\"_anon\", &{}); serde_json::Value::Null }}", arg_strs[0]);
                    }
                    // poll[url, seconds] → register_poll was handled at assignment level; expression fallback
                    if name == "poll" && arg_strs.len() == 2 {
                        return format!("{{ __crow_state.register_poll(\"_anon\", &{}, {} as u64); serde_json::Value::Null }}", arg_strs[0], arg_strs[1]);
                    }
                    // field[label, var] → CrowVNode::field(label, key, current_value)
                    if name == "field" && self.needs_ui && arg_strs.len() == 2 {
                        // Extract the state key from the second argument
                        let key = match &args[1] {
                            Expr::Ident(vname) => vname.clone(),
                            Expr::FieldAccess { field, .. } => field.clone(),
                            _ => "unknown".to_string(),
                        };
                        return format!("CrowVNode::field(&{}, \"{}\", &format!(\"{{}}\", {}))", arg_strs[0], key, arg_strs[1]);
                    }
                    // select[options, var] → CrowVNode::select_input(options, key, current_value)
                    if name == "select" && self.needs_ui && arg_strs.len() == 2 {
                        let key = match &args[1] {
                            Expr::Ident(vname) => vname.clone(),
                            Expr::FieldAccess { field, .. } => field.clone(),
                            _ => "unknown".to_string(),
                        };
                        return format!("CrowVNode::select_input(&{}, \"{}\", &format!(\"{{}}\", {}))", arg_strs[0], key, arg_strs[1]);
                    }
                    // table[frame] → CrowVNode::table_from_data from CrowFrame columns/rows.
                    // A fallible frame (query result) renders its reason on error —
                    // a view renders what it's given.
                    if name == "table" && self.needs_ui && arg_strs.len() == 1 {
                        let arg_ty = self.ctx.infer_expr(&args[0]);
                        if matches!(arg_ty, CrowType::Result(_)) {
                            return format!(
                                "{{ let _fr = {}; if !_fr.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _fr.reason)) }} else {{ let _f = &_fr.value; let _headers: Vec<String> = _f.columns.clone(); let _rows: Vec<Vec<String>> = (0.._f.nrows()).map(|r| (0.._f.ncols()).map(|c| _f.data[c].fmt_cell(r, _f.validity[c][r])).collect()).collect(); CrowVNode::table_from_data(&_headers, &_rows) }} }}",
                                arg_strs[0]
                            );
                        }
                        return format!(
                            "{{ let _f = &{}; let _headers: Vec<String> = _f.columns.clone(); let _rows: Vec<Vec<String>> = (0.._f.nrows()).map(|r| (0.._f.ncols()).map(|c| _f.data[c].fmt_cell(r, _f.validity[c][r])).collect()).collect(); CrowVNode::table_from_data(&_headers, &_rows) }}",
                            arg_strs[0]
                        );
                    }
                    // today[] → chrono::Local::now().format("%Y-%m-%d").to_string()
                    if name == "today" && arg_strs.is_empty() {
                        self.needs_chrono = true;
                        return "chrono::Local::now().format(\"%Y-%m-%d\").to_string()".to_string();
                    }

                    // Struct constructor
                    if self.ctx.structs.contains_key(name) {
                        let sig = self.ctx.structs.get(name).unwrap().clone();
                        let field_inits: Vec<String> = sig.fields.iter()
                            .zip(args.iter())
                            .zip(arg_strs.iter())
                            .map(|(((fname, ftype), orig_expr), val)| {
                                // By-ref params need an owned clone in field position
                                let is_ref_param = matches!(orig_expr, Expr::Ident(n) if self.current_ref_params.contains(n));
                                let owned = if is_ref_param { format!("{}.clone()", val) } else { val.clone() };
                                if let CrowType::OptBoxStruct(_) = ftype {
                                    // Check if the arg is a sentinel (literal 0)
                                    if matches!(orig_expr, Expr::Int(0)) {
                                        format!("{}: None", fname)
                                    } else {
                                        format!("{}: Some(Box::new({}))", fname, owned)
                                    }
                                } else {
                                    format!("{}: {}", fname, owned)
                                }
                            })
                            .collect();
                        return format!("{} {{ {} }}", name, field_inits.join(", "));
                    }

                    // Known function → direct call (routing to the matching
                    // specialization when the function is monomorphized)
                    if self.ctx.functions.contains_key(name) {
                        if let Some(clones) = self.ctx.specializations.get(name) {
                            let call_sig: Vec<CrowType> = args.iter()
                                .map(|a| {
                                    let t = self.ctx.infer_expr(a);
                                    match t {
                                        CrowType::Result(inner) => *inner,
                                        CrowType::OptBoxStruct(n) => CrowType::Struct(n),
                                        other => other,
                                    }
                                })
                                .collect();
                            let target = clones.iter().find(|(sig, _)| {
                                sig.len() == call_sig.len()
                                    && sig.iter().zip(&call_sig).all(|(a, b)| a.compatible(b))
                            });
                            match target {
                                Some((_, mangled)) => {
                                    return format!("{}({})", Self::rust_fn_name(mangled), arg_strs.join(", "));
                                }
                                None => {
                                    // fall back to the first clone (arg types unresolved)
                                    let (_, mangled) = &clones[0];
                                    return format!("{}({})", Self::rust_fn_name(mangled), arg_strs.join(", "));
                                }
                            }
                        }
                        return format!("{}({})", Self::rust_fn_name(name), arg_strs.join(", "));
                    }

                    // Check variable type for list/map/json indexing vs indirect function call
                    let var_type = self.ctx.vars.get(name).cloned()
                        .unwrap_or(CrowType::Unknown);
                    // Unwrap CrowResult for indexing purposes
                    let inner_type = match &var_type {
                        CrowType::Result(inner) => inner.as_ref(),
                        other => other,
                    };
                    let is_result = matches!(&var_type, CrowType::Result(_));
                    let base_name = if is_result {
                        format!("{}.value", name)
                    } else {
                        name.to_string()
                    };
                    match inner_type {
                        CrowType::NumArray(inner) => {
                            // Check if index is a NumArray (gather/advanced indexing)
                            let idx_type = self.ctx.infer_expr(&args[0]);
                            if matches!(idx_type, CrowType::NumArray(_)) {
                                let idx = &arg_strs[0];
                                return format!("{}.gather(&{})", base_name, idx);
                            }
                            // Rank >= 2: the element at [i] is a row tensor
                            if matches!(inner.as_ref(), CrowType::NumArray(_)) {
                                let idx = &arg_strs[0];
                                return format!("{{ let _i = {}; let _rows = {}.shape()[0] as i64; let _idx = if _i < 0 {{ (_rows + _i) as usize }} else {{ _i as usize }}; {}.index0(_idx) }}", idx, base_name, base_name);
                            }
                            // CrowTensor indexing with negative index support
                            let idx = &arg_strs[0];
                            let cast = if matches!(inner.as_ref(), CrowType::Int) { " as i64" } else { "" };
                            return format!("{{ let _i = {}; let _idx = if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }}; {}.get_flat(_idx){} }}", idx, base_name, base_name, cast);
                        }
                        CrowType::Array(inner) => {
                            // Array indexing with negative index support
                            let idx = &arg_strs[0];
                            // Clone for non-Copy types (Map, Array, Str, etc.)
                            let needs_clone = !matches!(inner.as_ref(), CrowType::Int | CrowType::Float | CrowType::Bool);
                            let suffix = if needs_clone { ".clone()" } else { "" };
                            return format!("{}[{{ let _i = {}; if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }} }}]{}", base_name, idx, base_name, suffix);
                        }
                        CrowType::Str => {
                            // String indexing with negative index support
                            let idx = &arg_strs[0];
                            return format!("{{ let _s: Vec<char> = {}.chars().collect(); let _i = {}; let _idx = if _i < 0 {{ (_s.len() as i64 + _i) as usize }} else {{ _i as usize }}; _s[_idx].to_string() }}", base_name, idx);
                        }
                        CrowType::FrameCol => {
                            // CrowColumn indexing: col[i] → col.get_str(i)
                            let idx = &arg_strs[0];
                            return format!("{{ let _i = {}; let _idx = if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }}; {}.get_str(_idx) }}", idx, base_name, base_name);
                        }
                        CrowType::Map(_) => {
                            // Map lookup: map[key] → map[&key]
                            return format!("{}[&{}].clone()", base_name, arg_strs.join(", "));
                        }
                        CrowType::Json => {
                            // JSON indexing: data["key"] or data[0] → data[key].clone()
                            let idx = &arg_strs[0];
                            // serde_json::Value indexes by &str or usize
                            let arg_type = self.ctx.infer_expr(&args[0]);
                            let idx_expr = match arg_type {
                                CrowType::Int => format!("{} as usize", idx),
                                _ => format!("&*{}", idx),
                            };
                            return format!("{}[{}].clone()", base_name, idx_expr);
                        }
                        CrowType::Fn(param_types, _) => {
                            // Cast args to expected types (e.g., i64 → f64 for grad closures)
                            let casted: Vec<String> = arg_strs.iter().enumerate().map(|(i, s)| {
                                if let Some(CrowType::Float) = param_types.get(i) {
                                    let arg_type = self.ctx.infer_expr(&args[i]);
                                    if matches!(arg_type, CrowType::Int) {
                                        format!("{} as f64", s)
                                    } else { s.clone() }
                                } else { s.clone() }
                            }).collect();
                            return format!("{}({})", name, casted.join(", "));
                        }
                        _ => {
                            // Function pointer call or unknown: name(args)
                            return format!("{}({})", name, arg_strs.join(", "));
                        }
                    }
                }


                // Module function/struct call: module.func[args] or module.Struct[args]
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(mod_name) = object.as_ref() {
                        // Registry-module prefixed call (math.zeros[...]):
                        // route exactly like the bare call — spec: module
                        // prefixes always work
                        // db has its own dedicated branch below (DbHandle codegen)
                        let real_mod = self
                            .ctx
                            .module_aliases
                            .get(mod_name.as_str())
                            .cloned()
                            .unwrap_or_else(|| mod_name.to_string());
                        if matches!(real_mod.as_str(), "math" | "io" | "frame" | "math.stats" | "math.nn")
                            && self.registry.has_function(field)
                        {
                            let bare = Expr::Call {
                                callee: Box::new(Expr::Ident(field.clone())),
                                args: args.clone(),
                            };
                            let saved_pin = self.pinned_module.take();
                            self.pinned_module = Some(real_mod.clone());
                            let out = self.expr_to_string(&bare);
                            self.pinned_module = saved_pin;
                            return out;
                        }
                        // db.connect[url] → crow_db::connect(url, None)
                        if mod_name == "db" && self.needs_crow_db {
                            match field.as_str() {
                                "connect" => {
                                    self.note_db_backend(&arg_strs[0]);
                                    if arg_strs.len() == 1 {
                                        return format!("crow_db::connect(&{}, None).unwrap()", arg_strs[0]);
                                    } else if arg_strs.len() == 2 {
                                        return format!("crow_db::connect(&{}, Some({} as usize)).unwrap()", arg_strs[0], arg_strs[1]);
                                    }
                                }
                                "query" => {
                                    if arg_strs.len() == 2 {
                                        return format!("{{ match {}.query(&{}, &[{}]) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], "");
                                    } else if arg_strs.len() >= 3 {
                                        let params = Self::crow_params_to_json(&arg_strs[2..]);
                                        return format!("{{ match {}.query(&{}, &[{}]) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], params);
                                    }
                                }
                                "exec" => {
                                    if arg_strs.len() == 2 {
                                        return format!("{{ match {}.exec(&{}, &[{}]) {{ Ok(_n) => CrowResult {{ ok: true, value: _n, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: 0_i64, reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], "");
                                    } else if arg_strs.len() >= 3 {
                                        let params = Self::crow_params_to_json(&arg_strs[2..]);
                                        return format!("{{ match {}.exec(&{}, &[{}]) {{ Ok(_n) => CrowResult {{ ok: true, value: _n, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: 0_i64, reason: _e, kind: \"db\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], arg_strs[1], params);
                                    }
                                }
                                "insert" => {
                                    if arg_strs.len() == 3 {
                                        return format!("{}.insert(&{}, &{}).unwrap()", arg_strs[0], arg_strs[1], arg_strs[2]);
                                    }
                                }
                                "migrate" => {
                                    if arg_strs.len() == 2 {
                                        return format!("{}.migrate(&{}).unwrap()", arg_strs[0], arg_strs[1]);
                                    }
                                }
                                "transact" => {
                                    if arg_strs.len() == 2 {
                                        return format!("{}.transact(&|_tx| {{ {}(_tx); Ok(()) }}).unwrap()", arg_strs[0], arg_strs[1]);
                                    }
                                }
                                _ => {}
                            }
                        }
                        // ui.input[...] → CrowVNode::input().attr(...)
                        if mod_name == "ui" && field == "input" && self.needs_ui {
                            let (attr_pairs, _child_args): (Vec<(&Expr, &Expr)>, Vec<&Expr>) =
                                if !args.is_empty() {
                                    if let Expr::Map(pairs) = &args[0] {
                                        let attrs: Vec<(&Expr, &Expr)> = pairs.iter().map(|(k, v)| (k, v)).collect();
                                        let children: Vec<&Expr> = args[1..].iter().collect();
                                        (attrs, children)
                                    } else {
                                        (vec![], args.iter().collect())
                                    }
                                } else {
                                    (vec![], vec![])
                                };
                            let mut result = "CrowVNode::input()".to_string();
                            for (k, v) in &attr_pairs {
                                let key_str = match k {
                                    Expr::Ident(s) => s.clone(),
                                    Expr::Str(s) => s.clone(),
                                    _ => self.expr_to_string(k),
                                };
                                // Check if value is a function reference for event handlers
                                let is_fn_ref = matches!(v, Expr::Ident(fn_name) if self.reactive_event_handlers.contains(fn_name));
                                let event_name = if is_fn_ref {
                                    if let Expr::Ident(fn_name) = v { format!("\"{}\"", fn_name) } else { self.expr_to_string(v) }
                                } else {
                                    self.expr_to_string(v)
                                };
                                match key_str.as_str() {
                                    "click" => result = if is_fn_ref { format!("{}.on_click({})", result, event_name) } else { format!("{}.on_click(&{})", result, event_name) },
                                    "input" => result = if is_fn_ref { format!("{}.on_input({})", result, event_name) } else { format!("{}.on_input(&{})", result, event_name) },
                                    "submit" => result = if is_fn_ref { format!("{}.on_submit({})", result, event_name) } else { format!("{}.on_submit(&{})", result, event_name) },
                                    _ => result = format!("{}.attr(&{}, &{})", result, self.expr_to_string(k), self.expr_to_string(v)),
                                }
                            }
                            return result;
                        }
                        // web.new[] → CrowApp::new()
                        if mod_name == "web" && field == "new" && self.needs_crow_web {
                            return "CrowApp::new()".to_string();
                        }
                        // web.docs[title, version] → CrowDocs::new(title, version)
                        if mod_name == "web" && field == "docs" && self.needs_crow_web {
                            if arg_strs.len() == 2 {
                                return format!("crow_web::docs::CrowDocs::new(&{}, &{})", arg_strs[0], arg_strs[1]);
                            }
                        }
                        // web.queue[n] → CrowQueue::new(n)
                        if mod_name == "web" && field == "queue" && self.needs_crow_web && arg_strs.len() == 1 {
                            return format!("CrowQueue::new({} as usize)", arg_strs[0]);
                        }
                        let prefixed = format!("{}_{}", mod_name, field);
                        // Module struct constructor
                        if self.ctx.structs.contains_key(&prefixed) {
                            let sig = self.ctx.structs.get(&prefixed).unwrap().clone();
                            let field_inits: Vec<String> = sig.fields.iter()
                                .zip(args.iter())
                                .zip(arg_strs.iter())
                                .map(|(((fname, ftype), orig_expr), val)| {
                                    if let CrowType::OptBoxStruct(_) = ftype {
                                        if matches!(orig_expr, Expr::Int(0)) {
                                            format!("{}: None", fname)
                                        } else {
                                            format!("{}: Some(Box::new({}))", fname, val)
                                        }
                                    } else {
                                        format!("{}: {}", fname, val)
                                    }
                                })
                                .collect();
                            return format!("{} {{ {} }}", prefixed, field_inits.join(", "));
                        }
                        // Module function call
                        if self.ctx.functions.contains_key(&prefixed) {
                            return format!("{}({})", prefixed, arg_strs.join(", "));
                        }
                    }

                    // state.get['key', default] → state.get_i64/get_str/get_f64/get_bool
                    // based on the inferred type of the default value (2nd arg).
                    // This hides Rust type names from Crow users.
                    if field == "get" && args.len() == 2 && self.needs_ui {
                        let obj_str = self.expr_to_string(object);
                        let default_type = self.ctx.infer_expr(&args[1]);
                        match default_type {
                            CrowType::Int => return format!("{}.get_i64(&{}, {})", obj_str, arg_strs[0], arg_strs[1]),
                            CrowType::Float => return format!("{}.get_f64(&{}, {})", obj_str, arg_strs[0], arg_strs[1]),
                            CrowType::Str => return format!("{}.get_str(&{}, &{})", obj_str, arg_strs[0], arg_strs[1]),
                            CrowType::Bool => return format!("{}.get_bool(&{}, {})", obj_str, arg_strs[0], arg_strs[1]),
                            _ => {} // fall through to generic handling
                        }
                    }

                    // state.set['key', value] → state.set("key", serde_json::json!(value))
                    if field == "set" && args.len() == 2 && self.needs_ui {
                        self.needs_serde_json = true;
                        let obj_str = self.expr_to_string(object);
                        return format!("{}.set(&{}, serde_json::json!({}))", obj_str, arg_strs[0], arg_strs[1]);
                    }
                }

                // Non-ident callee (e.g., list[0][3] or expr returning a function)
                let callee_type = self.ctx.infer_expr(callee);
                let callee_str = self.expr_to_string(callee);
                match &callee_type {
                    CrowType::NumArray(inner) => {
                        // Check if index is a NumArray (gather/advanced indexing)
                        let idx_type = self.ctx.infer_expr(&args[0]);
                        if matches!(idx_type, CrowType::NumArray(_)) {
                            let idx = &arg_strs[0];
                            format!("{}.gather(&{})", callee_str, idx)
                        } else if matches!(inner.as_ref(), CrowType::NumArray(_)) {
                            // Rank >= 2: element at [i] is a row tensor
                            let idx = &arg_strs[0];
                            format!("{{ let _i = {}; let _rows = {}.shape()[0] as i64; let _idx = if _i < 0 {{ (_rows + _i) as usize }} else {{ _i as usize }}; {}.index0(_idx) }}", idx, callee_str, callee_str)
                        } else {
                            let idx = &arg_strs[0];
                            let cast = if matches!(inner.as_ref(), CrowType::Int) { " as i64" } else { "" };
                            format!("{{ let _i = {}; let _idx = if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }}; {}.get_flat(_idx){} }}", idx, callee_str, callee_str, cast)
                        }
                    }
                    CrowType::Array(_) => {
                        let idx = &arg_strs[0];
                        format!("{}[{{ let _i = {}; if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }} }}]", callee_str, idx, callee_str)
                    }
                    CrowType::Str => {
                        let idx = &arg_strs[0];
                        format!("{{ let _s: Vec<char> = {}.chars().collect(); let _i = {}; let _idx = if _i < 0 {{ (_s.len() as i64 + _i) as usize }} else {{ _i as usize }}; _s[_idx].to_string() }}", callee_str, idx)
                    }
                    CrowType::FrameCol => {
                        let idx = &arg_strs[0];
                        format!("{{ let _i = {}; let _idx = if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }}; {}.get_str(_idx) }}", idx, callee_str, callee_str)
                    }
                    CrowType::Map(_) => format!("{}[&{}].clone()", callee_str, arg_strs.join(", ")),
                    CrowType::Json => {
                        let arg_type = self.ctx.infer_expr(&args[0]);
                        let idx_expr = match arg_type {
                            CrowType::Int => format!("{} as usize", arg_strs[0]),
                            _ => format!("&*{}", arg_strs[0]),
                        };
                        format!("{}[{}].clone()", callee_str, idx_expr)
                    }
                    CrowType::Fn(param_types, _) => {
                        // Cast args to expected types (e.g., i64 → f64 for grad closures)
                        let casted: Vec<String> = arg_strs.iter().enumerate().map(|(i, s)| {
                            if let Some(CrowType::Float) = param_types.get(i) {
                                let arg_type = self.ctx.infer_expr(&args[i]);
                                if matches!(arg_type, CrowType::Int) {
                                    format!("{} as f64", s)
                                } else { s.clone() }
                            } else { s.clone() }
                        }).collect();
                        format!("{}({})", callee_str, casted.join(", "))
                    }
                    _ => format!("{}({})", callee_str, arg_strs.join(", ")),
                }
            }
            Expr::FieldAccess { object, field } => {
                // Module variable access: module.var → module_var
                if let Expr::Ident(mod_name) = object.as_ref() {
                    if self.modules.contains(mod_name) {
                        let prefixed = format!("{}_{}", mod_name, field);
                        return prefixed;
                    }
                }
                let obj = self.expr_to_string(object);
                // Recursive struct link (OptBoxStruct): unwrap the box —
                // field access on the empty sentinel (0) is a runtime error
                {
                    let obj_type = self.ctx.infer_expr(object);
                    if let CrowType::OptBoxStruct(sname) = &obj_type {
                        let is_copy = self
                            .ctx
                            .structs
                            .get(sname)
                            .and_then(|sig| {
                                sig.fields.iter().find(|(f, _)| f == field).map(|(_, t)| {
                                    matches!(t, CrowType::Int | CrowType::Float | CrowType::Bool)
                                })
                            })
                            .unwrap_or(false);
                        let suffix = if is_copy { "" } else { ".clone()" };
                        return format!(
                            "{}.as_ref().expect(\"field access on an empty (0) node\").{}{}",
                            obj, field, suffix
                        );
                    }
                }
                // Auto-unwrap: struct field access through a Result reads .value
                {
                    let obj_type = self.ctx.infer_expr(object);
                    if let CrowType::Result(inner) = &obj_type {
                        if let CrowType::Struct(sname) = inner.as_ref() {
                            if !matches!(field.as_str(), "ok" | "reason" | "kind" | "trace") {
                                let is_copy = self
                                    .ctx
                                    .structs
                                    .get(sname)
                                    .and_then(|sig| {
                                        sig.fields.iter().find(|(f, _)| f == field).map(|(_, t)| {
                                            matches!(t, CrowType::Int | CrowType::Float | CrowType::Bool)
                                        })
                                    })
                                    .unwrap_or(false);
                                let suffix = if is_copy { "" } else { ".clone()" };
                                return format!("{}.value.{}{}", obj, field, suffix);
                            }
                        }
                    }
                }
                // Map dot access through a Result: r.key → r.value["key"]
                {
                    let obj_type = self.ctx.infer_expr(object);
                    if let CrowType::Result(inner) = &obj_type {
                        if matches!(inner.as_ref(), CrowType::Map(_))
                            && !matches!(field.as_str(), "ok" | "reason" | "kind" | "trace")
                        {
                            return format!("{}.value[\"{}\"].clone()", obj, field);
                        }
                    }
                }
                // Json object field access: j.a → j["a"] (Null if missing)
                {
                    let obj_type = self.ctx.infer_expr(object);
                    let obj_inner = match &obj_type { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                    if matches!(obj_inner, CrowType::Json)
                        && !matches!(field.as_str(), "ok" | "reason" | "kind" | "trace")
                    {
                        let obj = if matches!(obj_type, CrowType::Result(_)) {
                            format!("{}.value", obj)
                        } else {
                            obj.clone()
                        };
                        return format!("{}.get(\"{}\").cloned().unwrap_or(serde_json::Value::Null)", obj, field);
                    }
                }
                // Map Crow's .len to Rust's .len()
                if field == "len" {
                    let obj_type = self.ctx.infer_expr(object);
                    if matches!(obj_type, CrowType::Frame) {
                        return format!("{}.nrows() as i64", obj);
                    }
                    if matches!(obj_type, CrowType::Array(_) | CrowType::NumArray(_) | CrowType::Str | CrowType::FrameCol) {
                        return format!("({}.len() as i64)", obj);
                    }
                }
                // Error-value struct field access: .ok, .reason, .kind, .trace
                if field == "ok" || field == "reason" || field == "kind" || field == "trace" {
                    let obj_type = self.ctx.infer_expr(object);
                    if let CrowType::Result(_) = &obj_type {
                        // Real CrowResult<T> — access the field directly
                        if field == "ok" {
                            return format!("{}.ok", obj);
                        }
                        if field == "trace" {
                            return format!("{}.trace.clone()", obj);
                        }
                        return format!("{}.{}.clone()", obj, field);
                    } else if field == "ok" && matches!(&obj_type, CrowType::FrameCol) {
                        // .ok on a CrowColumn → validity bitmap (native)
                        return format!("{}.ok()", obj);
                    } else if field == "ok" && matches!(&obj_type, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str)) {
                        // .ok on a legacy Vec<String> frame column → validity bitmap
                        return format!("crow_runtime::frame::col_ok(&{})", obj);
                    } else if !matches!(obj_type, CrowType::Struct(_)) {
                        // Raw type — return synthetic defaults
                        return match field.as_str() {
                            "ok" => "true".to_string(),
                            "reason" => "String::new()".to_string(),
                            "kind" => "String::new()".to_string(),
                            "trace" => "Vec::<String>::new()".to_string(),
                            _ => unreachable!(),
                        };
                    }
                }
                // DateTime properties on strings: .year, .month, .day, .hour, .minute, .second, .offset
                if matches!(field.as_str(), "year" | "month" | "day" | "hour" | "minute" | "second" | "offset") {
                    let obj_type = self.ctx.infer_expr(object);
                    if matches!(obj_type, CrowType::Str) {
                        return format!("crow_runtime::frame::datetime_field(&{}, \"{}\").unwrap_or(0)", obj, field);
                    }
                }
                // NumArray properties: .shape, .dtype, .device, .grad
                {
                    let obj_type = self.ctx.infer_expr(object);
                    let is_result_numarray = matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::NumArray(_)));
                    if matches!(obj_type, CrowType::NumArray(_)) || is_result_numarray {
                        let tensor_expr = if is_result_numarray {
                            format!("{}.value.clone()", obj)
                        } else {
                            obj.clone()
                        };
                        match field.as_str() {
                            "shape" => return format!("{}.shape().iter().map(|&s| s as i64).collect::<Vec<i64>>()", tensor_expr),
                            "dtype" => return "\"float\".to_string()".to_string(),
                            "device" => return format!("if {}.is_gpu() {{ \"gpu\".to_string() }} else {{ \"cpu\".to_string() }}", tensor_expr),
                            "grad" => return "\"none\".to_string()".to_string(),
                            _ => {}
                        }
                    }
                }
                // Frame properties: .shape, .columns, .dtypes, .column_name
                {
                    let obj_type = self.ctx.infer_expr(object);
                    if matches!(obj_type, CrowType::Frame) || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Frame)) {
                        let frame_expr = if matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Frame)) {
                            format!("{}.value.clone()", obj)
                        } else {
                            obj.clone()
                        };
                        match field.as_str() {
                            "shape" => return format!("vec![{f}.nrows() as i64, {f}.ncols() as i64]", f = frame_expr),
                            "columns" => return format!("{}.columns.iter().map(|c: &String| c.clone()).collect::<Vec<String>>()", frame_expr),
                            "dtypes" => return format!("{}.dtypes().iter().map(|d: &&str| d.to_string()).collect::<Vec<String>>()", frame_expr),
                            _ => {
                                // Column access by name: df.age → native CrowColumn
                                return format!(
                                    "{}.col_native(\"{}\").unwrap_or_else(|| panic!(\"column '{}' not found\"))",
                                    frame_expr, field, field
                                );
                            }
                        }
                    }
                }
                // Map dot access: map.key → map["key"].clone()
                let obj_type = self.ctx.infer_expr(object);
                if matches!(obj_type, CrowType::Map(_)) {
                    return format!("{}[\"{}\"].clone()", obj, field);
                }
                // Check if this field is OptBoxStruct — if so, the caller
                // will handle unwrapping when it's used as a function argument.
                // Here we just emit the raw field access.
                format!("{}.{}", obj, field)
            }
            Expr::Array(items) => {
                // Check if this is an array of functions — need explicit fn pointer cast
                let inner_type = if !items.is_empty() {
                    let mut t = CrowType::Unknown;
                    for item in items {
                        t = t.unify(&self.ctx.infer_expr(item));
                    }
                    t
                } else {
                    CrowType::Unknown
                };
                // All-numeric arrays become CrowTensor
                if matches!(inner_type, CrowType::Int | CrowType::Float) && !items.is_empty() {
                    let arr_type = self.ctx.infer_expr(&Expr::Array(items.clone()));
                    if matches!(arr_type, CrowType::NumArray(_)) {
                        let vals: Vec<String> = items.iter()
                            .map(|i| format!("{} as f64", self.expr_to_string(i)))
                            .collect();
                        let n = items.len();
                        self.needs_crow_runtime = true;
                        return format!("crow_runtime::tensor::CrowTensor::from_vec(vec![{}], vec![{}])", vals.join(", "), n);
                    }
                }
                // Nested numeric literals build one shaped tensor: [[1,2],[3,4]]
                // → rank-2 CrowTensor (spec: unified arrays)
                if matches!(inner_type, CrowType::NumArray(_)) && !items.is_empty() {
                    let elems: Vec<String> = items.iter()
                        .map(|i| self.expr_to_string(i))
                        .collect();
                    self.needs_crow_runtime = true;
                    return format!(
                        "crow_runtime::tensor::CrowTensor::from_elements(&[{}]).unwrap_or_else(|e| panic!(\"{{}}\", e))",
                        elems.join(", ")
                    );
                }
                let vals: Vec<String> = if let CrowType::Fn(_, _) = &inner_type {
                    // Cast each item to fn pointer type to avoid Rust's distinct fn item types
                    let fn_type = inner_type.to_rust_type();
                    items.iter()
                        .map(|i| format!("{} as {}", self.expr_to_string(i), fn_type))
                        .collect()
                } else {
                    items.iter()
                        .map(|i| self.owned_expr_str(i))
                        .collect()
                };
                format!("vec![{}]", vals.join(", "))
            }
            Expr::Map(pairs) => {
                if pairs.is_empty() {
                    "BTreeMap::new()".to_string()
                } else {
                    // Check individual value types to detect mixed-type maps
                    let val_types: Vec<CrowType> = pairs.iter()
                        .map(|(_, v)| self.ctx.infer_expr(v))
                        .collect();
                    let val_type = {
                        let mut t = CrowType::Unknown;
                        for vt in &val_types {
                            t = t.unify(vt);
                        }
                        t
                    };
                    // Detect mixed types: if any value type differs from the unified type,
                    // convert everything to String
                    let has_mixed = val_types.iter().any(|vt| vt != &val_type && *vt != CrowType::Unknown);
                    let entries: Vec<String> = if let CrowType::Fn(_, _) = &val_type {
                        let fn_type = val_type.to_rust_type();
                        pairs.iter()
                            .map(|(k, v)| format!("({}, {} as {})", self.expr_to_string(k), self.expr_to_string(v), fn_type))
                            .collect()
                    } else if has_mixed {
                        // Mixed-type map: convert all values to String
                        pairs.iter()
                            .zip(val_types.iter())
                            .map(|((k, v), vt)| {
                                let v_str = self.expr_to_string(v);
                                let v_as_str = match vt {
                                    CrowType::Str => v_str,
                                    _ => format!("{}.to_string()", v_str),
                                };
                                format!("({}, {})", self.expr_to_string(k), v_as_str)
                            })
                            .collect()
                    } else {
                        pairs.iter()
                            .map(|(k, v)| format!("({}, {})", self.expr_to_string(k), self.expr_to_string(v)))
                            .collect()
                    };
                    format!("BTreeMap::from([{}])", entries.join(", "))
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.emit_for_expr(var, iter, body)
            }
        }
    }

    /// Check if a for-expr body is a filter pattern:
    /// The last non-line stmt is an `if` without `else`, whose then_body's last stmt
    /// is an ExprStmt (the value to keep).
    fn is_filter_pattern(body: &[Stmt]) -> bool {
        let last = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
        if let Some(Stmt::If { else_body: None, then_body, .. }) = last {
            // Check that then_body ends with an ExprStmt
            let last_then = then_body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
            matches!(last_then, Some(Stmt::ExprStmt(_)))
        } else {
            false
        }
    }

    /// Extract the filter condition and value expression from a filter pattern.
    fn extract_filter_parts(body: &[Stmt]) -> Option<(&Expr, &Expr)> {
        let last = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
        if let Some(Stmt::If { condition, else_body: None, then_body }) = last {
            let last_then = then_body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
            if let Some(Stmt::ExprStmt(val_expr)) = last_then {
                return Some((condition, val_expr));
            }
        }
        None
    }

    /// Emit a ForExpr as .iter().map(|var| { body }).collect()
    /// Parallel by default. Falls back to sequential if body mutates outer state or cpu[1].
    /// Detects filter patterns (if without else) and uses filter_map instead.
    fn emit_for_expr(&mut self, var: &str, iter: &Expr, body: &[Stmt]) -> String {
        let iter_str = self.expr_to_string(iter);
        let forced_sequential = self.cpu_count.map_or(false, |n| n <= 1);
        let mutates_outer = Self::for_body_mutates_outer(body, var, &self.declared);
        let use_par = !forced_sequential && !mutates_outer;
        if use_par {
            self.needs_rayon = true;
        }
        let iter_method = if use_par { "par_iter" } else { "iter" };

        // Determine element type of the iterator for the closure pattern
        let iter_type = self.ctx.infer_expr(iter);
        let is_numarray = matches!(&iter_type, CrowType::NumArray(_));
        let elem_type = match &iter_type {
            CrowType::Array(inner) | CrowType::NumArray(inner) => *inner.clone(),
            _ => CrowType::Unknown,
        };
        let is_copy = matches!(elem_type, CrowType::Int | CrowType::Float | CrowType::Bool);

        // For NumArray, convert to Vec first (rows for rank >= 2)
        let iter_str = if is_numarray {
            if matches!(elem_type, CrowType::NumArray(_)) {
                format!("{}.rows()", iter_str)
            } else if matches!(elem_type, CrowType::Int) {
                format!("{}.to_vec().iter().map(|&v| v as i64).collect::<Vec<i64>>()", iter_str)
            } else {
                format!("{}.to_vec()", iter_str)
            }
        } else {
            iter_str
        };

        // Build the closure parameter pattern
        let param_pattern = if is_copy {
            format!("|&{}|", var)
        } else {
            format!("|{}|", var)
        };

        // Check for filter pattern: for x in array \n if cond \n x
        if Self::is_filter_pattern(body) {
            if let Some((cond, val_expr)) = Self::extract_filter_parts(body) {
                // Save state for closure body
                let saved_vars = self.ctx.vars.clone();
                self.ctx.vars.insert(var.to_string(), elem_type.clone());

                let cond_str = self.expr_to_string(cond);
                let val_str = self.expr_to_string(val_expr);
                let val_type = self.ctx.infer_expr(val_expr);

                self.ctx.vars = saved_vars;

                let collect_type = val_type.to_rust_type();
                return format!(
                    "{}.{}().filter_map({} {{ if {} {{ Some({}) }} else {{ None }} }}).collect::<Vec<{}>>()",
                    iter_str, iter_method, param_pattern, cond_str, val_str, collect_type
                );
            }
        }

        // Determine the return type of each iteration (last expression)
        let last_type = if let Some(last_stmt) = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
            match last_stmt {
                Stmt::ExprStmt(e) => self.ctx.infer_expr(e),
                _ => CrowType::Unknown,
            }
        } else {
            CrowType::Unknown
        };

        // Save transpiler state for closure body
        let saved_output = std::mem::take(&mut self.output);
        let saved_declared = self.declared.clone();
        let saved_vars = self.ctx.vars.clone();
        let saved_indent = self.indent;
        self.indent = saved_indent + 1;
        self.declared.clear();

        // Add loop variable to context
        self.declared.insert(var.to_string());
        self.ctx.vars.insert(var.to_string(), elem_type.clone());

        // Split body into init statements and last expression
        let last_non_line = body.iter().rposition(|s| !matches!(s, Stmt::Line(_)));
        let has_last_expr = last_non_line.map_or(false, |i| matches!(body[i], Stmt::ExprStmt(_)));
        let (init_stmts, last_expr) = if has_last_expr {
            let idx = last_non_line.unwrap();
            if idx > 0 {
                (&body[..idx], Some(&body[idx]))
            } else {
                (&body[..0], Some(&body[idx]))
            }
        } else {
            (body, None)
        };

        for stmt in init_stmts {
            self.emit_stmt(stmt);
        }

        // Emit last expression without semicolon (it's the return value)
        if let Some(Stmt::ExprStmt(e)) = last_expr {
            let expr_str = self.expr_to_string(e);
            let indent = "    ".repeat(self.indent);
            self.output.push_str(&format!("{}{}\n", indent, expr_str));
        }

        let closure_body = std::mem::take(&mut self.output);

        // Restore state
        self.output = saved_output;
        self.declared = saved_declared;
        self.ctx.vars = saved_vars;
        self.indent = saved_indent;

        let indent = "    ".repeat(self.indent);
        let collect_type = last_type.to_rust_type();

        format!(
            "{}.{}().map({} {{\n{}{}}}).collect::<Vec<{}>>()",
            iter_str, iter_method, param_pattern,
            closure_body,
            indent,
            collect_type
        )
    }

    /// Convert a Crow interpolated string to Rust format!() macro.
    fn string_to_rust(&mut self, s: &str) -> String {
        // Crow strings use {expr} for interpolation
        let mut fmt_str = String::new();
        let mut args: Vec<String> = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            // Escaped backtick from the lexer: \` is a literal character
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                fmt_str.push('`');
                i += 2;
                continue;
            }
            // Literal braces are inert characters now — double them for format!
            if chars[i] == '{' || chars[i] == '}' {
                fmt_str.push(chars[i]);
                fmt_str.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '`' {
                // Interpolation span: scan to the closing backtick, skipping
                // string literals inside the expression (quote-aware)
                let start = i + 1;
                i += 1;
                let mut in_str = false;
                while i < chars.len() {
                    let c = chars[i];
                    if in_str {
                        if c == '\\' && i + 1 < chars.len() { i += 2; continue; }
                        if c == '\'' { in_str = false; }
                    } else if c == '\'' {
                        in_str = true;
                    } else if c == '`' {
                        break;
                    }
                    i += 1;
                }
                let expr_str: String = chars[start..i].iter().collect();
                i += 1; // skip closing `

                // Parse the expression as Crow code and transpile to Rust
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                if let Ok(tokens) = lexer.tokenize() {
                    let mut parser = crate::parser::Parser::new(tokens);
                    if let Ok(stmts) = parser.parse() {
                        // Lower named arguments (fmt[x, decimals = 2], etc.) the
                        // same way top-level code is lowered — otherwise
                        // interpolated calls with named args aren't recognized.
                        let stmts = crate::lower::lower_program(&stmts).unwrap_or(stmts);
                        if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, Stmt::Line(_))) {
                            let rust_expr = self.expr_to_string(expr);
                            // Use {:?} for collection/struct types since they don't impl Display
                            let expr_type = self.ctx.infer_expr(expr);
                            let inner_type = match &expr_type {
                                CrowType::Result(inner) => inner.as_ref().clone(),
                                other => other.clone(),
                            };
                            match &inner_type {
                                CrowType::FrameCol => {
                                    fmt_str.push_str("{:?}");
                                    args.push(format!("{}.to_strings()", rust_expr));
                                }
                                t if self.kernel_displayable(t) => {
                                    // Shared display kernel: interpolated
                                    // arrays/maps match interpreter output
                                    self.needs_crow_runtime = true;
                                    fmt_str.push_str("{}");
                                    args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; ({}).crow_fmt() }}", rust_expr));
                                }
                                CrowType::Array(_) | CrowType::Map(_) | CrowType::Struct(_) => {
                                    fmt_str.push_str("{:?}");
                                    args.push(rust_expr);
                                }
                                _ => {
                                    fmt_str.push_str("{}");
                                    args.push(rust_expr);
                                }
                            }
                        } else {
                            // Fallback: use raw text
                            fmt_str.push_str("{}");
                            args.push(expr_str);
                        }
                    } else {
                        fmt_str.push_str("{}");
                        args.push(expr_str);
                    }
                } else {
                    fmt_str.push_str("{}");
                    args.push(expr_str);
                }
            } else {
                // Escape special characters for Rust string literal
                match chars[i] {
                    '\n' => fmt_str.push_str("\\n"),
                    '\t' => fmt_str.push_str("\\t"),
                    '\\' => fmt_str.push_str("\\\\"),
                    '"' => fmt_str.push_str("\\\""),
                    '\r' => fmt_str.push_str("\\r"),
                    ch => fmt_str.push(ch),
                }
                i += 1;
            }
        }

        if args.is_empty() {
            // No interpolation → plain literal: un-double the escaped braces
            // (they are only doubled for format!'s syntax)
            let raw = fmt_str.replace("{{", "{").replace("}}", "}");
            format!("\"{}\".to_string()", raw)
        } else {
            format!("format!(\"{}\", {})", fmt_str, args.join(", "))
        }
    }

    /// Emit a propagation block that checks CrowResult args for errors before calling.
    /// If any arg is an error, returns a CrowResult with that error.
    /// Otherwise, executes the inner call and wraps the result.
    fn emit_propagation_block(
        &self,
        result_indices: &[usize],
        orig_arg_strs: &[String],
        inner_expr: &str,
        inner_type: &CrowType,
        already_result: bool,
    ) -> String {
        let default_val = inner_type.default_rust_value();
        let trace_expr = self.trace_format_expr();
        let error_result = |idx: usize| -> String {
            format!(
                "{{ let mut _err = CrowResult {{ ok: false, value: {}, reason: _prop_{}.reason.clone(), kind: _prop_{}.kind.clone(), trace: _prop_{}.trace.clone(), captured: 0.0 }}; _err.trace.push({}); _err }}",
                default_val, idx, idx, idx, trace_expr
            )
        };
        let success_expr = if already_result {
            inner_expr.to_string()
        } else {
            format!(
                "CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}",
                inner_expr
            )
        };
        // Build nested if-else from inside out
        let mut result = success_expr;
        for (pos, &idx) in result_indices.iter().enumerate().rev() {
            result = format!(
                "{{ let _prop_{} = {}.clone(); if !_prop_{}.ok {{ {} }} else {{ {} }} }}",
                idx, orig_arg_strs[pos], idx, error_result(idx), result
            );
        }
        result
    }

    /// Emit a log[] call as println!()
    /// A condition that infers to a Result cannot choose a branch on an
    /// error (spec section 11: Errors in Conditions). Wrap it: ok -> the bool
    /// value; error -> propagate out of the function, or halt at top level.
    fn guard_condition(&mut self, condition: &Expr, cond_str: String) -> String {
        let cond_ty = self.ctx.infer_expr(condition);
        if !matches!(cond_ty, CrowType::Result(_)) {
            return cond_str;
        }
        let on_error = if let Some(ret) = self.current_fn_return_type.clone() {
            let inner_default = match &ret {
                CrowType::Result(inner) => inner.default_rust_value(),
                other => other.default_rust_value(),
            };
            let trace = self.trace_format_expr();
            format!(
                "return CrowResult {{ ok: false, value: {}, reason: _c.reason.clone(), kind: _c.kind.clone(), trace: {{ let mut _t = _c.trace.clone(); _t.push({}); _t }}, captured: 0.0 }};",
                inner_default, trace
            )
        } else {
            "{ eprintln!(\"error: cannot branch on an error value — {}\", _c.reason); std::process::exit(1); }".to_string()
        };
        format!(
            "{{ let _c = {}; if !_c.ok {{ {} }} _c.value }}",
            cond_str, on_error
        )
    }

    /// Does a predicate function take its first parameter by reference?
    /// (The by-ref-params optimization passes non-Copy params as &T.)
    fn pred_takes_ref(&self, pred: &Expr) -> bool {
        if let Expr::Ident(fn_name) = pred {
            if let Some(sig) = self.ctx.functions.get(fn_name) {
                if let Some((pname, _)) = sig.params.first() {
                    return self
                        .by_ref_params
                        .get(fn_name)
                        .map_or(false, |set| set.contains(pname));
                }
            }
        }
        false
    }

    /// Does this body call any of the search builtins (any/all/find)?
    /// They emit par_iter, which needs the rayon prelude in the header.
    fn has_search_builtin(stmts: &[Stmt]) -> bool {
        fn expr_has(e: &Expr) -> bool {
            match e {
                Expr::Call { callee, args } => {
                    if let Expr::Ident(n) = callee.as_ref() {
                        if matches!(n.as_str(), "any" | "all" | "find") && args.len() == 2 {
                            return true;
                        }
                    }
                    expr_has(callee) || args.iter().any(expr_has)
                }
                Expr::BinOp { left, right, .. } => expr_has(left) || expr_has(right),
                Expr::UnaryOp { expr, .. } => expr_has(expr),
                Expr::FieldAccess { object, .. } => expr_has(object),
                Expr::Array(items) => items.iter().any(expr_has),
                Expr::Map(pairs) => pairs.iter().any(|(k, v)| expr_has(k) || expr_has(v)),
                Expr::Named { value, .. } => expr_has(value),
                Expr::ForExpr { iter, body, .. } => expr_has(iter) || body.iter().any(stmt_has),
                _ => false,
            }
        }
        fn stmt_has(s: &Stmt) -> bool {
            match s {
                Stmt::Assign { value, .. } => expr_has(value),
                Stmt::MultiAssign { values, .. } => values.iter().any(expr_has),
                Stmt::ExprStmt(e) => expr_has(e),
                Stmt::Return(Some(e)) => expr_has(e),
                Stmt::If { condition, then_body, else_body } => {
                    expr_has(condition)
                        || then_body.iter().any(stmt_has)
                        || else_body.as_ref().map_or(false, |b| b.iter().any(stmt_has))
                }
                Stmt::For { iter, body, .. } => expr_has(iter) || body.iter().any(stmt_has),
                Stmt::While { condition, body } => expr_has(condition) || body.iter().any(stmt_has),
                Stmt::FnDecl { body, .. } => body.iter().any(stmt_has),
                _ => false,
            }
        }
        stmts.iter().any(stmt_has)
    }

    /// Arrays whose element types the shared display kernel
    /// (crow_runtime::display::CrowDisplay) can format.
    fn kernel_displayable(&self, ty: &CrowType) -> bool {
        match ty {
            CrowType::Array(inner) => match inner.as_ref() {
                CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool => true,
                CrowType::NumArray(_) => true,
                nested @ CrowType::Array(_) => self.kernel_displayable(nested),
                _ => false,
            },
            CrowType::Map(inner) => match inner.as_ref() {
                CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool
                | CrowType::Unknown => true,
                nested @ (CrowType::Array(_) | CrowType::Map(_)) => self.kernel_displayable(nested),
                _ => false,
            },
            CrowType::Struct(n) => self.struct_kernel_displayable(n),
            _ => false,
        }
    }

    /// A struct prints through the shared display kernel when a CrowDisplay
    /// impl is emitted for it: simple field types only, and never the
    /// synthetic builtin-return structs.
    fn struct_kernel_displayable(&self, name: &str) -> bool {
        if matches!(name, "CrowShell" | "CrowElapsed" | "CrowFindResult") {
            return false;
        }
        self.ctx.structs.get(name).map(|sig| {
            sig.fields.iter().all(|(_, t)| {
                matches!(t, CrowType::Int | CrowType::Float | CrowType::Bool | CrowType::Str)
            })
        }).unwrap_or(false)
    }

    fn emit_log_call(&mut self, args: &[String], orig_exprs: &[Expr]) -> String {
        if args.is_empty() {
            return "println!()".to_string();
        }
        if args.len() == 1 {
            let arg = &args[0];
            if arg.starts_with("format!(") {
                return format!("println!{}", &arg[7..]);
            }
            let (needs_debug, json_kind) = if let Some(expr) = orig_exprs.first() {
                let ty = self.ctx.infer_expr(expr);
                let (inner, is_result) = match &ty {
                    CrowType::Result(inner) => (inner.as_ref().clone(), true),
                    other => (other.clone(), false),
                };
                let is_col = matches!(inner, CrowType::FrameCol);
                let debug = matches!(inner, CrowType::Array(_) | CrowType::Map(_) | CrowType::Struct(_)) || is_col;
                let jk = if matches!(inner, CrowType::Json) {
                    if is_result { 2 } else { 1 }  // 2=Result<Json>, 1=Json
                } else { 0 };
                (debug, jk)
            } else {
                (false, 0)
            };
            // FrameCol → convert to strings before display
            if let Some(expr) = orig_exprs.first() {
                let ty = self.ctx.infer_expr(expr);
                let inner = match &ty { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                if matches!(inner, CrowType::FrameCol) {
                    return format!("{{ use crow_runtime::display::CrowDisplay as _; println!(\"{{}}\", {}.to_strings().crow_fmt()) }}", arg);
                }
                // Arrays print via the shared display kernel so output matches
                // the interpreter exactly (strings single-quoted, etc.)
                if self.kernel_displayable(&inner) {
                    self.needs_crow_runtime = true;
                    return format!("{{ use crow_runtime::display::CrowDisplay as _; println!(\"{{}}\", ({}).crow_fmt()) }}", arg);
                }
            }
            return if needs_debug {
                format!("println!(\"{{:?}}\", {})", arg)
            } else if json_kind == 2 {
                // CrowResult<Json> — check status, display result with crow_json_display
                format!("{{ let _jv = {}.clone(); if !_jv.ok {{ println!(\"error: {{}}\", _jv.reason); }} else {{ println!(\"{{}}\", crow_json_display(&_jv.value)); }} }}", arg)
            } else if json_kind == 1 {
                format!("println!(\"{{}}\", crow_json_display(&{}))", arg)
            } else {
                format!("println!(\"{{}}\", {})", arg)
            };
        }
        // Multi-argument log: join all args with spaces
        let mut fmt_parts = Vec::new();
        let mut fmt_args = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let (needs_debug, json_kind, is_col) = if i < orig_exprs.len() {
                let ty = self.ctx.infer_expr(&orig_exprs[i]);
                let (inner, is_result) = match &ty {
                    CrowType::Result(inner) => (inner.as_ref().clone(), true),
                    other => (other.clone(), false),
                };
                let is_col = matches!(inner, CrowType::FrameCol);
                let debug = matches!(inner, CrowType::Array(_) | CrowType::Map(_) | CrowType::Struct(_)) || is_col;
                let jk = if matches!(inner, CrowType::Json) {
                    if is_result { 2 } else { 1 }
                } else { 0 };
                (debug, jk, is_col)
            } else {
                (false, 0, false)
            };
            // Shared display kernel for plain arrays (matches interpreter output)
            let kernel = if i < orig_exprs.len() {
                let ty = self.ctx.infer_expr(&orig_exprs[i]);
                let inner = match &ty { CrowType::Result(t) => t.as_ref().clone(), o => o.clone() };
                is_col || self.kernel_displayable(&inner)
            } else { false };
            if needs_debug && !kernel {
                fmt_parts.push("{:?}".to_string());
            } else {
                fmt_parts.push("{}".to_string());
            }
            if is_col {
                self.needs_crow_runtime = true;
                fmt_args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; {}.to_strings().crow_fmt() }}", arg));
            } else if kernel {
                self.needs_crow_runtime = true;
                fmt_args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; ({}).crow_fmt() }}", arg));
            } else if json_kind == 2 {
                fmt_args.push(format!("crow_json_display(&{}.value)", arg));
            } else if json_kind == 1 {
                fmt_args.push(format!("crow_json_display(&{})", arg));
            } else {
                fmt_args.push(arg.clone());
            }
        }
        format!("println!(\"{}\", {})", fmt_parts.join(" "), fmt_args.join(", "))
    }

    /// Check if a binary comparison involves an OptBoxStruct field compared to a sentinel (0).
    /// e.g. `tree.left eq 0` => `tree.left.is_none()`
    fn try_optbox_comparison(&mut self, left: &Expr, op: &BinOp, right: &Expr) -> Option<String> {
        // Check left is a field access on a struct with OptBoxStruct field
        if let Expr::FieldAccess { object, field } = left {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                // Check if right side is sentinel 0
                                if matches!(right, Expr::Int(0)) {
                                    let obj_str = self.expr_to_string(object);
                                    return match op {
                                        BinOp::Eq => Some(format!("{}.{}.is_none()", obj_str, field)),
                                        BinOp::Ne => Some(format!("{}.{}.is_some()", obj_str, field)),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }
        // Check right side too (0 eq tree.left)
        if let Expr::FieldAccess { object, field } = right {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                if matches!(left, Expr::Int(0)) {
                                    let obj_str = self.expr_to_string(object);
                                    return match op {
                                        BinOp::Eq => Some(format!("{}.{}.is_none()", obj_str, field)),
                                        BinOp::Ne => Some(format!("{}.{}.is_some()", obj_str, field)),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if an expression refers to an OptBoxStruct field, and if so
    /// return the unwrapped version for use as a function argument.
    fn maybe_unwrap_optbox(&mut self, expr: &Expr) -> Option<String> {
        if let Expr::FieldAccess { object, field } = expr {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                let obj_str = self.expr_to_string(object);
                                return Some(format!("*{}.{}.as_ref().unwrap().clone()", obj_str, field));
                            }
                        }
                    }
                }
            }
        }
        None
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// Helper: parse Crow source, run type inference, transpile to Rust
    fn transpile(source: &str) -> String {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("lexer failed");
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().expect("parser failed");
        // Same pipeline as the real build: lower named args before inference
        let stmts = crate::lower::lower_program(&stmts).expect("lowering failed");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let mut t = Transpiler::new(ctx);
        t.transpile(&stmts)
    }

    #[test]
    fn test_multi_assign() {
        let src = "x, y = 10, 20\nlog['{x} {y}']\n";
        let rust = transpile(src);
        assert!(rust.contains("let x: i64 = 10"), "should declare x\n{}", rust);
        assert!(rust.contains("let y: i64 = 20"), "should declare y\n{}", rust);
    }

    #[test]
    fn test_swap_uses_temporaries() {
        let src = "a = 1\nb = 2\na, b = b, a\nlog['{a} {b}']\n";
        let rust = transpile(src);
        // Swap should use temp variables
        assert!(rust.contains("_multi_"), "swap should use temp vars\n{}", rust);
    }

    #[test]
    fn test_int_parse_returns_crow_result() {
        let src = "x = int['42']\nlog['{x}']\n";
        let rust = transpile(src);
        assert!(rust.contains("CrowResult"), "int[] from string should return CrowResult\n{}", rust);
        assert!(!rust.contains(".unwrap()"), "int[] should not use .unwrap()\n{}", rust);
    }

    #[test]
    fn test_float_parse_returns_crow_result() {
        let src = "x = float['3.14']\nlog['{x}']\n";
        let rust = transpile(src);
        assert!(rust.contains("CrowResult"), "float[] from string should return CrowResult\n{}", rust);
        assert!(!rust.contains(".unwrap()"), "float[] should not use .unwrap()\n{}", rust);
    }

    #[test]
    fn test_scope_if_block() {
        // Variable declared inside if should not affect outer scope declarations
        let src = "x = 1\nif true\n    y = 2\n    x = 10\nx = 5\n";
        let rust = transpile(src);
        // x should be declared with let mut (reassigned)
        // y inside if should be a separate let
        assert!(rust.contains("let y:"), "y should be declared inside if\n{}", rust);
    }

    #[test]
    fn test_scope_while_block() {
        let src = "n = 5\nwhile n gt 0\n    step = n\n    n = n - 1\n";
        let rust = transpile(src);
        assert!(rust.contains("let") && rust.contains("step:"), "step should be declared inside while\n{}", rust);
    }

    #[test]
    fn test_scope_for_block() {
        let src = "for i in [1, 2, 3]\n    temp = i * 10\n    log['{temp}']\n";
        let rust = transpile(src);
        assert!(rust.contains("let") && rust.contains("temp:"), "temp should be declared inside for\n{}", rust);
    }

    #[test]
    fn test_cpu_count_wiring() {
        let mut lexer = Lexer::new("x = 1\nlog['{x}']\n");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let mut t = Transpiler::new(ctx);
        t.set_cpu_count(4);
        assert_eq!(t.cpu_count, Some(4));
    }

    #[test]
    fn test_read_returns_crow_result() {
        let src = "content = read['/tmp/crow_test_read.txt']\nlog['{content}']\n";
        let rust = transpile(src);
        assert!(rust.contains("CrowResult"), "read[] should return CrowResult\n{}", rust);
        assert!(!rust.contains(".unwrap()"), "read[] should not use .unwrap()\n{}", rust);
    }

    #[test]
    fn test_parallel_independent_stmts() {
        let src = "a = 1 + 2\nb = 3 + 4\nc = 5 + 6\n";
        let rust = transpile(src);
        // Independent assignments should be grouped into a rayon::join
        assert!(rust.contains("rayon::join"), "independent stmts should use rayon::join\n{}", rust);
    }

    #[test]
    fn test_dependent_stmts_sequential() {
        let src = "a = 1\nb = a + 1\nc = b + 1\n";
        let rust = transpile(src);
        // Dependent assignments should NOT be in rayon::join
        assert!(!rust.contains("rayon::join"), "dependent stmts should be sequential\n{}", rust);
    }

    #[test]
    fn test_crow_result_struct_emitted() {
        let src = "x = int['42']\n";
        let rust = transpile(src);
        assert!(rust.contains("struct CrowResult<T: Clone>"), "should emit CrowResult struct\n{}", rust);
        assert!(rust.contains("ok: bool"), "CrowResult should have ok field\n{}", rust);
        assert!(rust.contains("value: T"), "CrowResult should have value field\n{}", rust);
        assert!(rust.contains("reason: String"), "CrowResult should have reason field\n{}", rust);
        assert!(rust.contains("kind: String"), "CrowResult should have kind field\n{}", rust);
        assert!(rust.contains("trace: Vec<String>"), "CrowResult should have trace field\n{}", rust);
    }

    #[test]
    fn test_crow_result_not_emitted_when_unnecessary() {
        // CrowResult is now always emitted (allow(dead_code)) — flag-timing
        // made conditional emission unreliable. Assert no Result USAGE instead.
        let src = "x = 5\nlog[x]\n";
        let rust = transpile(src);
        assert!(!rust.contains("CrowResult {"), "no Result values should be constructed\n{}", rust);
    }

    #[test]
    fn test_ok_on_raw_type() {
        let src = "x = 5\nlog[x.ok]\n";
        let rust = transpile(src);
        assert!(rust.contains("true"), "ok on raw type should return true\n{}", rust);
    }

    #[test]
    fn test_reason_field_access() {
        let src = "x = int['42']\nlog[x.reason]\n";
        let rust = transpile(src);
        assert!(rust.contains(".reason"), "should access .reason field\n{}", rust);
    }

    #[test]
    fn test_ok_field_access_on_result() {
        let src = "x = int['42']\nlog[x.ok]\n";
        let rust = transpile(src);
        assert!(rust.contains(".ok"), "should access .ok field on CrowResult\n{}", rust);
    }

    #[test]
    fn test_auto_unwrap_in_binop() {
        let src = "x = int['42']\ny = x + 1\n";
        let rust = transpile(src);
        // Should generate propagation block that checks .ok and propagates errors
        assert!(rust.contains("_binl_"), "should generate binop propagation temp vars\n{}", rust);
        assert!(rust.contains(".ok"), "should check ok field for propagation\n{}", rust);
        assert!(rust.contains(".value"), "should access .value for success case\n{}", rust);
    }

    #[test]
    fn test_binop_propagation_right_operand() {
        let src = "x = int['42']\ny = 10 + x\n";
        let rust = transpile(src);
        // Right operand is CrowResult, should generate propagation
        assert!(rust.contains("_binr_"), "should generate binop right propagation temp var\n{}", rust);
        assert!(rust.contains(".ok"), "should check ok field\n{}", rust);
    }

    #[test]
    fn test_binop_no_propagation_plain() {
        let src = "x = 5\ny = x + 10\n";
        let rust = transpile(src);
        // No CrowResult operands, should NOT generate propagation
        assert!(!rust.contains("_binl_"), "should not generate binop propagation for plain types\n{}", rust);
        assert!(!rust.contains("_binr_"), "should not generate binop propagation for plain types\n{}", rust);
    }

    #[test]
    fn test_binop_result_type_is_crow_result() {
        let src = "x = int['42']\ny = x + 1\nlog[y.ok]\n";
        let rust = transpile(src);
        // y should be CrowResult since x is CrowResult
        assert!(rust.contains("y.ok"), "y should be CrowResult with .ok field\n{}", rust);
    }

    #[test]
    fn test_int_from_float_no_result() {
        let src = "x = 3.14\ny = int[x]\n";
        let rust = transpile(src);
        assert!(!rust.contains("CrowResult {"), "int from float should not construct a Result\n{}", rust);
    }

    #[test]
    #[should_panic(expected = "cannot reassign")]
    fn test_type_changing_reassignment_error() {
        // x = 5 (int), then x = read['f'] (CrowResult<String>) — should panic
        let src = "x = 5\nx = read['file.txt']\n";
        transpile(src);
    }

    #[test]
    fn test_same_type_reassignment_ok() {
        // x = 5, then x = 10 — same type, should be fine
        let src = "x = 5\nx = 10\n";
        let rust = transpile(src);
        assert!(rust.contains("x = 10"), "same-type reassignment should work\n{}", rust);
    }

    #[test]
    fn test_int_float_reassignment_ok() {
        // x = 5, then x = 3.14 — int/float are compatible
        let src = "x = 5\nx = 3.14\n";
        let rust = transpile(src);
        assert!(rust.contains("x = 3.14"), "int/float reassignment should work\n{}", rust);
    }

    #[test]
    fn test_for_loop_parallel_by_default() {
        let src = "items = [1, 2, 3]\nfor x in items\n    log['{x}']\n";
        let rust = transpile(src);
        assert!(rust.contains("par_iter()"), "for loop should use par_iter by default\n{}", rust);
        assert!(rust.contains("for_each"), "parallel for should use for_each\n{}", rust);
        assert!(rust.contains("use rayon::prelude::*"), "should import rayon prelude\n{}", rust);
    }

    #[test]
    fn test_for_loop_sequential_when_mutates_outer() {
        // Loop body assigns to `total` which is in outer scope — must be sequential
        let src = "total = 0\nitems = [1, 2, 3]\nfor x in items\n    total = total + x\nlog['{total}']\n";
        let rust = transpile(src);
        assert!(rust.contains("for") && rust.contains(".iter()"), "mutating loop should use iter()\n{}", rust);
        assert!(!rust.contains("par_iter"), "mutating loop should NOT use par_iter\n{}", rust);
    }

    #[test]
    fn test_for_loop_sequential_with_cpu_1() {
        let src = "cpu[1]\nitems = [1, 2, 3]\nfor x in items\n    log['{x}']\n";
        let rust = transpile(src);
        assert!(!rust.contains("par_iter"), "cpu[1] should force sequential for loop\n{}", rust);
    }

    #[test]
    fn test_for_expr_parallel_by_default() {
        let src = "items = [1, 2, 3]\ndoubled = for x in items\n    x * 2\nlog['{doubled}']\n";
        let rust = transpile(src);
        assert!(rust.contains("par_iter()"), "for-expr should use par_iter by default\n{}", rust);
    }

    #[test]
    fn test_error_trace_field_access() {
        let src = "x = int['bad']\ny = x.trace\n";
        let rust = transpile(src);
        assert!(rust.contains(".trace"), "should access .trace field on CrowResult\n{}", rust);
    }

    #[test]
    fn test_raw_value_kind_is_empty() {
        let src = "x = 5\ny = x.kind\n";
        let rust = transpile(src);
        assert!(rust.contains("String::new()"), "raw value .kind should be empty string\n{}", rust);
    }

    #[test]
    fn test_auto_propagation_uses_if_else() {
        let src = "fn double[x]\n    return x * 2\nbad = int['hello']\nresult = double[bad]\n";
        let rust = transpile(src);
        assert!(rust.contains("_prop_"), "should generate propagation temp vars\n{}", rust);
        assert!(rust.contains("!_prop_") || rust.contains(".ok"), "should check ok field for propagation\n{}", rust);
        assert!(rust.contains("else"), "should use if-else for propagation\n{}", rust);
    }

    #[test]
    fn test_auto_propagation_unwraps_args() {
        let src = "fn double[x]\n    return x * 2\nbad = int['hello']\nresult = double[bad]\n";
        let rust = transpile(src);
        // Function should take i64, not CrowResult<i64>
        assert!(rust.contains("fn double(x: i64)"), "function param should be base type, not CrowResult\n{}", rust);
    }

    #[test]
    fn test_int_error_message_format() {
        let src = "x = int['bad']\n";
        let rust = transpile(src);
        assert!(rust.contains("cannot convert"), "should use Crow-style error message\n{}", rust);
    }

    #[test]
    fn test_crow_result_display_shows_error() {
        let src = "x = int['bad']\nlog[x]\n";
        let rust = transpile(src);
        assert!(rust.contains("!self.ok"), "Display impl should check ok field\n{}", rust);
    }

    #[test]
    fn test_json_builtin_emits_serde() {
        let src = "items = json['[1,2,3]']\nlog[items[0]]\n";
        let rust = transpile(src);
        assert!(rust.contains("serde_json::from_str"), "json[] should use serde_json\n{}", rust);
        assert!(rust.contains("CrowResult"), "json[] should return CrowResult\n{}", rust);
    }

    #[test]
    fn test_json_null_returns_error() {
        let src = "n = json['null']\nlog[n.ok]\n";
        let rust = transpile(src);
        assert!(rust.contains("is_null()"), "json[] should check for null\n{}", rust);
        assert!(rust.contains("ok: false"), "null should map to ok: false\n{}", rust);
    }

    #[test]
    fn test_encode_builtin() {
        let src = "items = json['[1,2,3]']\ns = encode[items]\nlog[s]\n";
        let rust = transpile(src);
        assert!(rust.contains("serde_json::to_string"), "encode[] should use serde_json::to_string\n{}", rust);
    }

    #[test]
    fn test_json_display_helper_emitted() {
        let src = "s = json['\"hello\"']\nlog[s]\n";
        let rust = transpile(src);
        assert!(rust.contains("crow_json_display"), "should emit crow_json_display helper\n{}", rust);
    }

    #[test]
    fn test_json_indexing_with_int() {
        let src = "items = json['[1,2,3]']\nlog[items[0]]\n";
        let rust = transpile(src);
        assert!(rust.contains("as usize"), "int index on JSON should cast to usize\n{}", rust);
    }

    // ===== §1: Variables & Types =====

    #[test]
    fn test_var_int_transpile() {
        let rust = transpile("x = 42\n");
        assert!(rust.contains("let x: i64 = 42"), "int var\n{}", rust);
    }

    #[test]
    fn test_var_float_transpile() {
        let rust = transpile("x = 3.14\n");
        assert!(rust.contains("let x: f64 = 3.14"), "float var\n{}", rust);
    }

    #[test]
    fn test_var_string_transpile() {
        let rust = transpile("x = 'hello'\n");
        assert!(rust.contains("let x: String"), "string var\n{}", rust);
    }

    #[test]
    fn test_var_bool_transpile() {
        let rust = transpile("x = true\n");
        assert!(rust.contains("let x: bool = true"), "bool var\n{}", rust);
    }

    #[test]
    fn test_var_list_transpile() {
        let rust = transpile("x = [1, 2, 3]\n");
        assert!(rust.contains("Vec"), "array should use Vec\n{}", rust);
    }

    #[test]
    fn test_string_interpolation_transpile() {
        let rust = transpile("name = 'crow'\nx = 'hello {name}'\n");
        assert!(rust.contains("format!"), "interpolation should use format!\n{}", rust);
    }

    // ===== §3: Functions =====

    #[test]
    fn test_fn_declaration_transpile() {
        let rust = transpile("fn add[a, b]\n    a + b\nlog[add[1, 2]]\n");
        assert!(rust.contains("fn add("), "should declare function\n{}", rust);
    }

    #[test]
    fn test_fn_return_transpile() {
        let rust = transpile("fn greet[]\n    return 'hello'\nx = greet[]\n");
        assert!(rust.contains("return"), "should have return\n{}", rust);
    }

    // ===== §4: Control Flow =====

    #[test]
    fn test_if_else_transpile() {
        let rust = transpile("x = 5\nif x gt 3\n    log['big']\nelse\n    log['small']\n");
        assert!(rust.contains("if"), "should have if\n{}", rust);
        assert!(rust.contains("else"), "should have else\n{}", rust);
    }

    // ===== §5: Loops =====

    #[test]
    fn test_while_transpile() {
        let rust = transpile("x = 5\nwhile x gt 0\n    x = x - 1\n");
        assert!(rust.contains("while") || rust.contains("loop"), "should have loop\n{}", rust);
    }

    #[test]
    fn test_while_break_transpile() {
        let rust = transpile("x = 0\nwhile true\n    x = x + 1\n    if x eq 5\n        break\n");
        assert!(rust.contains("break"), "should have break\n{}", rust);
    }

    #[test]
    fn test_for_expr_transpile() {
        let rust = transpile("doubled = for x in [1, 2, 3]\n    x * 2\n");
        assert!(rust.contains("collect"), "for-expr should collect results\n{}", rust);
    }

    // ===== §7: Structs =====

    #[test]
    fn test_struct_transpile() {
        let rust = transpile("struct Bird [name, speed]\nb = Bird['crow', 50]\n");
        assert!(rust.contains("struct Bird"), "should declare struct\n{}", rust);
    }

    #[test]
    fn test_struct_field_transpile() {
        let rust = transpile("struct Bird [name, speed]\nb = Bird['crow', 50]\nlog[b.name]\n");
        assert!(rust.contains(".name"), "should access field\n{}", rust);
    }

    // ===== §8: Operators =====

    #[test]
    fn test_comparison_operators_transpile() {
        let rust = transpile("x = 5\ny = x gt 3\n");
        assert!(rust.contains(">"), "gt should map to >\n{}", rust);
    }

    #[test]
    fn test_logical_operators_transpile() {
        let rust = transpile("x = true and false\n");
        assert!(rust.contains("&&"), "and should map to &&\n{}", rust);
    }

    #[test]
    fn test_expression_grouping_transpile() {
        let rust = transpile("x = (1 + 2) * 3\n");
        assert!(rust.contains("(1") && rust.contains("* 3"), "should preserve grouping\n{}", rust);
    }

    // ===== Builtins =====

    #[test]
    fn test_range_transpile() {
        let rust = transpile("x = range[5]\n");
        assert!(rust.contains("0..5") || rust.contains("range"), "should generate range\n{}", rust);
    }

    #[test]
    fn test_len_transpile() {
        let rust = transpile("x = [1, 2, 3]\ny = len[x]\n");
        assert!(rust.contains(".len()"), "should use .len()\n{}", rust);
    }

    #[test]
    fn test_push_transpile() {
        let rust = transpile("x = [1, 2]\ny = push[x, 3]\n");
        assert!(rust.contains("push"), "should use push\n{}", rust);
    }

    #[test]
    fn test_sort_transpile() {
        let rust = transpile("x = [3, 1, 2]\ny = sort[x]\n");
        assert!(rust.contains("sort"), "should use sort\n{}", rust);
    }

    #[test]
    fn test_map_operations_transpile() {
        let rust = transpile("m = map[]\nm = set[m, 'a', 1]\n");
        assert!(rust.contains("BTreeMap") || rust.contains("HashMap"), "should use map type\n{}", rust);
    }

    #[test]
    fn test_error_builtin_transpile() {
        let rust = transpile("x = error['something went wrong']\n");
        assert!(rust.contains("ok: false"), "error[] should create error value\n{}", rust);
    }

    #[test]
    fn test_math_sqrt_transpile() {
        let rust = transpile("x = sqrt[16]\n");
        assert!(rust.contains("sqrt"), "should use sqrt\n{}", rust);
    }

    #[test]
    fn test_string_upper_transpile() {
        let rust = transpile("x = upper['hello']\n");
        assert!(rust.contains("to_uppercase"), "should use to_uppercase\n{}", rust);
    }

    #[test]
    fn test_string_split_transpile() {
        let rust = transpile("x = split['a,b', ',']\n");
        assert!(rust.contains("split"), "should use split\n{}", rust);
    }

    #[test]
    fn test_contains_transpile() {
        let rust = transpile("x = [1, 2, 3]\ny = contains[x, 2]\n");
        assert!(rust.contains("contains") || rust.contains("any"), "should check contains\n{}", rust);
    }

    #[test]
    fn test_then_chaining_transpile() {
        let rust = transpile("x = '  hello  '\ny = x then trim\n");
        assert!(rust.contains("trim"), "then should desugar to function call\n{}", rust);
    }

    #[test]
    fn test_negative_index_transpile() {
        let rust = transpile("nums = [10, 20, 30]\nx = nums[-1]\n");
        assert!(rust.contains("if _i < 0"), "should emit negative index handling\n{}", rust);
    }

    #[test]
    fn test_negative_index_assign_transpile() {
        let rust = transpile("nums = [10, 20, 30]\nnums[-1] = 99\n");
        assert!(rust.contains("if _i < 0"), "should emit negative index handling for assignment\n{}", rust);
    }

    #[test]
    fn test_type_builtin_transpile() {
        let rust = transpile("x = type[42]\n");
        assert!(rust.contains("\"int\""), "type[42] should produce 'int'\n{}", rust);
    }

    #[test]
    fn test_type_builtin_str_transpile() {
        let rust = transpile("x = type['hello']\n");
        assert!(rust.contains("\"str\""), "type['hello'] should produce 'str'\n{}", rust);
    }

    #[test]
    fn test_type_builtin_array_transpile() {
        let rust = transpile("x = type[[1, 2]]\n");
        assert!(rust.contains("\"array\""), "type[[1,2]] should produce 'array'\n{}", rust);
    }

    #[test]
    fn test_string_index_transpile() {
        let rust = transpile("s = 'hello'\nx = s[0]\n");
        assert!(rust.contains(".chars().collect()"), "string indexing should use chars()\n{}", rust);
    }

    #[test]
    fn test_string_negative_index_transpile() {
        let rust = transpile("s = 'hello'\nx = s[-1]\n");
        assert!(rust.contains("if _i < 0"), "should handle negative string index\n{}", rust);
    }

    // ===== NumArray transpiler tests =====

    #[test]
    fn test_numarray_literal_transpile() {
        let rust = transpile("x = [1, 2, 3]\n");
        assert!(rust.contains("CrowTensor::from_vec"), "numeric array should use CrowTensor::from_vec\n{}", rust);
        assert!(rust.contains("as f64"), "elements should be cast to f64\n{}", rust);
        assert!(rust.contains("vec![3]"), "shape should be [3]\n{}", rust);
    }

    #[test]
    fn test_numarray_import_transpile() {
        let rust = transpile("x = [1, 2, 3]\n");
        assert!(rust.contains("use crow_runtime::tensor::CrowTensor"), "should import CrowTensor\n{}", rust);
    }

    #[test]
    fn test_numarray_add_transpile() {
        let rust = transpile("a = [1, 2, 3]\nb = [4, 5, 6]\nc = a + b\n");
        assert!(rust.contains(".add(&"), "NumArray + NumArray should use .add()\n{}", rust);
    }

    #[test]
    fn test_numarray_scalar_mul_transpile() {
        let rust = transpile("a = [1, 2, 3]\nb = a * 2\n");
        assert!(rust.contains(".mul_scalar("), "NumArray * scalar should use .mul_scalar()\n{}", rust);
    }

    #[test]
    fn test_numarray_neg_transpile() {
        let rust = transpile("a = [1, 2, 3]\nb = -a\n");
        assert!(rust.contains(".neg()"), "negating NumArray should use .neg()\n{}", rust);
    }

    #[test]
    fn test_numarray_indexing_transpile() {
        let rust = transpile("nums = [10, 20, 30]\nx = nums[0]\n");
        assert!(rust.contains("get_flat"), "NumArray indexing should use get_flat\n{}", rust);
    }

    #[test]
    fn test_numarray_negative_index_transpile() {
        let rust = transpile("nums = [10, 20, 30]\nx = nums[-1]\n");
        assert!(rust.contains("get_flat"), "NumArray negative indexing should use get_flat\n{}", rust);
        assert!(rust.contains("if _i < 0"), "should handle negative index\n{}", rust);
    }

    #[test]
    fn test_numarray_type_builtin_transpile() {
        let rust = transpile("x = type[[1, 2]]\n");
        assert!(rust.contains("\"array\""), "type[[1,2]] should produce 'array'\n{}", rust);
    }

    #[test]
    fn test_numarray_len_field_transpile() {
        let rust = transpile("a = [1, 2, 3]\nn = a.len\n");
        assert!(rust.contains(".len() as i64"), "NumArray .len should use .len() as i64\n{}", rust);
    }

    #[test]
    fn test_numarray_sort_transpile() {
        let rust = transpile("a = [3, 1, 2]\nb = sort[a]\n");
        assert!(rust.contains("sort_by") || rust.contains("CrowTensor::from_vec"), "NumArray sort should produce CrowTensor\n{}", rust);
    }

    #[test]
    fn test_numarray_contains_transpile() {
        let rust = transpile("a = [1, 2, 3]\nb = contains[a, 2]\n");
        assert!(rust.contains("to_vec().contains"), "NumArray contains should use to_vec().contains\n{}", rust);
    }

    #[test]
    fn test_numarray_for_loop_transpile() {
        let rust = transpile("cpu[1]\na = [1, 2, 3]\nfor x in a\n    log['{x}']\n");
        assert!(rust.contains("to_vec()"), "for loop over NumArray should use to_vec()\n{}", rust);
    }

    #[test]
    fn test_gpu_call_transpile() {
        let rust = transpile("gpu[1]\n");
        assert!(rust.contains("set_gpu_enabled(true)"), "gpu[] should emit set_gpu_enabled\n{}", rust);
    }

    #[test]
    fn test_gpu_no_args_transpile() {
        let rust = transpile("gpu[]\n");
        assert!(rust.contains("set_gpu_enabled(true)"), "gpu[] should emit set_gpu_enabled\n{}", rust);
    }

    #[test]
    fn test_is_gpu_call() {
        // ExprStmt path
        let stmt = Stmt::ExprStmt(Expr::Call {
            callee: Box::new(Expr::Ident("gpu".to_string())),
            args: vec![],
        });
        assert!(Transpiler::is_gpu_call(&stmt));

        // Resource path
        let stmt2 = Stmt::Resource { kind: "gpu".to_string(), count: Expr::Int(2) };
        assert!(Transpiler::is_gpu_call(&stmt2));

        // Not a gpu call
        let stmt3 = Stmt::ExprStmt(Expr::Call {
            callee: Box::new(Expr::Ident("cpu".to_string())),
            args: vec![Expr::Int(4)],
        });
        assert!(!Transpiler::is_gpu_call(&stmt3));
    }

    // ===== nodes (distributed) transpiler tests =====

    #[test]
    fn test_nodes_call_transpile() {
        let rust = transpile("nodes[4]\n");
        assert!(rust.contains("distributed::init(4)"), "nodes[] should emit distributed::init\n{}", rust);
        assert!(rust.contains("distributed::finalize()"), "nodes[] should emit distributed::finalize\n{}", rust);
    }

    #[test]
    fn test_extract_nodes_call() {
        // ExprStmt path
        let stmt = Stmt::ExprStmt(Expr::Call {
            callee: Box::new(Expr::Ident("nodes".to_string())),
            args: vec![Expr::Int(4)],
        });
        assert_eq!(Transpiler::extract_nodes_call(&stmt), Some(4));

        // Resource path
        let stmt2 = Stmt::Resource { kind: "nodes".to_string(), count: Expr::Int(8) };
        assert_eq!(Transpiler::extract_nodes_call(&stmt2), Some(8));

        // Not a nodes call
        let stmt3 = Stmt::ExprStmt(Expr::Call {
            callee: Box::new(Expr::Ident("gpu".to_string())),
            args: vec![],
        });
        assert_eq!(Transpiler::extract_nodes_call(&stmt3), None);
    }

    // ===== grad transpiler tests =====

    #[test]
    fn test_grad_emits_crow_dual() {
        let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(rust.contains("struct CrowDual"), "should emit CrowDual struct\n{}", rust);
        assert!(rust.contains("_dual_f"), "should emit _dual_f function\n{}", rust);
    }

    #[test]
    fn test_grad_single_param_closure() {
        let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(rust.contains("-> f64"), "grad closure should return f64\n{}", rust);
        assert!(rust.contains("CrowDual::new"), "should use CrowDual::new\n{}", rust);
        assert!(rust.contains(".dual"), "should extract .dual\n{}", rust);
    }

    #[test]
    fn test_grad_multi_param_closure() {
        let rust = transpile("fn g[x, y]\n  x * x + y * y\ndg = grad[g]\nlog[dg[3, 4]]\n");
        assert!(rust.contains("Vec<f64>"), "multi-param grad should return Vec<f64>\n{}", rust);
        assert!(rust.contains("_dual_g"), "should emit _dual_g\n{}", rust);
    }

    #[test]
    fn test_grad_int_arg_cast() {
        let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(rust.contains("as f64"), "should cast int arg to f64\n{}", rust);
    }

    #[test]
    fn test_grad_dual_struct_has_ops() {
        let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(rust.contains("impl std::ops::Add"), "CrowDual should impl Add\n{}", rust);
        assert!(rust.contains("impl std::ops::Mul"), "CrowDual should impl Mul\n{}", rust);
        assert!(rust.contains("impl std::ops::Sub"), "CrowDual should impl Sub\n{}", rust);
        assert!(rust.contains("impl std::ops::Div"), "CrowDual should impl Div\n{}", rust);
    }

    #[test]
    fn test_math_pi_transpile() {
        let rust = transpile("use math\nx = pi\n");
        assert!(rust.contains("std::f64::consts::PI"), "should emit PI constant\n{}", rust);
    }

    #[test]
    fn test_math_e_transpile() {
        let rust = transpile("use math\nx = e\n");
        assert!(rust.contains("std::f64::consts::E"), "should emit E constant\n{}", rust);
    }

    #[test]
    fn test_math_inf_transpile() {
        let rust = transpile("use math\nx = inf\n");
        assert!(rust.contains("f64::INFINITY"), "should emit INFINITY\n{}", rust);
    }

    #[test]
    fn test_math_nan_transpile() {
        let rust = transpile("use math\nx = nan\n");
        assert!(rust.contains("f64::NAN"), "should emit NAN\n{}", rust);
    }

    #[test]
    fn test_numarray_dtype_transpile() {
        let rust = transpile("use math\nx = ones[3]\ny = x.dtype\n");
        assert!(rust.contains("\"float\""), "should emit dtype as float string\n{}", rust);
    }

    #[test]
    fn test_numarray_device_transpile() {
        let rust = transpile("use math\nx = ones[3]\ny = x.device\n");
        assert!(rust.contains("is_gpu()"), "should emit device check\n{}", rust);
    }

    #[test]
    fn test_math_sign_transpile() {
        let rust = transpile("use math\nx = sign[ones[3]]\n");
        assert!(rust.contains(".sign()"), "should emit .sign()\n{}", rust);
    }

    #[test]
    fn test_math_floor_tensor_transpile() {
        let rust = transpile("use math\nx = floor[ones[3]]\n");
        assert!(rust.contains(".floor()"), "should emit .floor()\n{}", rust);
    }

    #[test]
    fn test_math_clip_transpile() {
        let rust = transpile("use math\nx = clip[ones[3], 0, 1]\n");
        assert!(rust.contains(".clip("), "should emit .clip()\n{}", rust);
    }

    #[test]
    fn test_math_mod_transpile() {
        let rust = transpile("use math\nx = mod[ones[3], 2]\n");
        assert!(rust.contains("modulo_scalar"), "should emit modulo_scalar for scalar arg\n{}", rust);
    }

    #[test]
    fn test_math_tan_transpile() {
        let rust = transpile("use math\nx = tan[ones[3]]\n");
        assert!(rust.contains(".tan()"), "should emit .tan()\n{}", rust);
    }

    #[test]
    fn test_math_tanh_transpile() {
        let rust = transpile("use math\nx = tanh[ones[3]]\n");
        assert!(rust.contains(".tanh()"), "should emit .tanh()\n{}", rust);
    }

    #[test]
    fn test_math_atan2_transpile() {
        let rust = transpile("use math\nx = atan2[ones[3], ones[3]]\n");
        assert!(rust.contains(".atan2("), "should emit .atan2()\n{}", rust);
    }

    // ===== Operator tests: ^, .*, ./, .^ =====

    #[test]
    fn test_pow_int_transpile() {
        let rust = transpile("x = 2 ^ 10\n");
        assert!(rust.contains(".pow("), "should use .pow()\n{}", rust);
    }

    #[test]
    fn test_pow_float_transpile() {
        let rust = transpile("x = 2.0 ^ 3.0\n");
        assert!(rust.contains(".powf("), "should use .powf()\n{}", rust);
    }

    #[test]
    fn test_elem_mul_transpile() {
        let rust = transpile("use math\nx = ones[3] .* ones[3]\n");
        assert!(rust.contains(".mul("), "should use .mul()\n{}", rust);
    }

    #[test]
    fn test_elem_div_transpile() {
        let rust = transpile("use math\nx = ones[3] ./ ones[3]\n");
        assert!(rust.contains(".div("), "should use .div()\n{}", rust);
    }

    #[test]
    fn test_elem_pow_transpile() {
        let rust = transpile("use math\nx = ones[3] .^ ones[3]\n");
        assert!(rust.contains(".pow("), "should use .pow()\n{}", rust);
    }

    #[test]
    fn test_tensor_mul_is_matmul_transpile() {
        let rust = transpile("use math\nx = eye[2] * eye[2]\n");
        assert!(rust.contains(".matmul("), "* on tensors should emit .matmul()\n{}", rust);
    }

    // ===== Reduction tests: prod, argmax, argmin =====

    #[test]
    fn test_math_prod_transpile() {
        let rust = transpile("use math\nx = prod[ones[3]]\n");
        assert!(rust.contains(".prod()"), "should emit .prod()\n{}", rust);
    }

    #[test]
    fn test_math_argmax_transpile() {
        let rust = transpile("use math\nx = argmax[ones[3]]\n");
        assert!(rust.contains(".argmax()"), "should emit .argmax()\n{}", rust);
    }

    #[test]
    fn test_math_argmin_transpile() {
        let rust = transpile("use math\nx = argmin[ones[3]]\n");
        assert!(rust.contains(".argmin()"), "should emit .argmin()\n{}", rust);
    }

    #[test]
    fn test_math_sum_axis_transpile() {
        let rust = transpile("use math\nx = sum[ones[2, 3], 0]\n");
        assert!(rust.contains(".sum_axis("), "should emit .sum_axis()\n{}", rust);
    }

    #[test]
    fn test_math_max_axis_transpile() {
        let rust = transpile("use math\nx = max[ones[2, 3], 1]\n");
        assert!(rust.contains(".max_axis("), "should emit .max_axis()\n{}", rust);
    }

    #[test]
    fn test_math_prod_axis_transpile() {
        let rust = transpile("use math\nx = prod[ones[2, 3], 0]\n");
        assert!(rust.contains(".prod_axis("), "should emit .prod_axis()\n{}", rust);
    }

    // ===== Sorting & selection transpiler tests =====

    #[test]
    fn test_math_argsort_transpile() {
        let rust = transpile("use math\nx = argsort[ones[5]]\n");
        assert!(rust.contains(".argsort()"), "should emit .argsort()\n{}", rust);
    }

    #[test]
    fn test_math_topk_transpile() {
        let rust = transpile("use math\nx = topk[ones[5], 3]\n");
        assert!(rust.contains(".topk("), "should emit .topk()\n{}", rust);
    }

    #[test]
    fn test_math_unique_transpile() {
        let rust = transpile("use math\nx = unique[ones[5]]\n");
        assert!(rust.contains(".unique()"), "should emit .unique()\n{}", rust);
    }

    #[test]
    fn test_math_where_transpile() {
        let rust = transpile("use math\nx = where[ones[3], ones[3], zeros[3]]\n");
        assert!(rust.contains(".where_cond("), "should emit .where_cond()\n{}", rust);
    }

    // ===== Cumulative ops transpiler tests =====

    #[test]
    fn test_math_cumsum_transpile() {
        let rust = transpile("use math\nx = cumsum[ones[5]]\n");
        assert!(rust.contains(".cumsum()"), "should emit .cumsum()\n{}", rust);
    }

    #[test]
    fn test_math_cumprod_transpile() {
        let rust = transpile("use math\nx = cumprod[ones[5]]\n");
        assert!(rust.contains(".cumprod()"), "should emit .cumprod()\n{}", rust);
    }

    // ===== Combining ops transpiler tests =====

    #[test]
    fn test_math_concat_transpile() {
        let rust = transpile("use math\nx = concat[ones[3], ones[2]]\n");
        assert!(rust.contains(".concat("), "should emit .concat()\n{}", rust);
    }

    #[test]
    fn test_math_stack_transpile() {
        let rust = transpile("use math\nx = stack[[ones[3], ones[3]]]\n");
        assert!(rust.contains("CrowTensor::stack("), "should emit CrowTensor::stack()\n{}", rust);
    }

    // ===== Broadcasting transpiler tests =====

    #[test]
    fn test_broadcast_add_transpile() {
        let rust = transpile("use math\na = reshape[ones[3], 3, 1]\nb = reshape[ones[4], 1, 4]\nx = a + b\n");
        assert!(rust.contains(".add("), "should emit .add() for broadcast\n{}", rust);
    }

    #[test]
    fn test_broadcast_elem_mul_transpile() {
        let rust = transpile("use math\na = reshape[ones[6], 3, 1]\nb = reshape[ones[2], 1, 2]\nx = a .* b\n");
        assert!(rust.contains(".mul("), "should emit .mul() for broadcast elem mul\n{}", rust);
    }

    // ===== Advanced indexing (gather) transpiler tests =====

    #[test]
    fn test_gather_transpile() {
        let rust = transpile("use math\na = linspace[10, 50, 5]\nidx = linspace[0, 2, 3]\nx = a[idx]\n");
        assert!(rust.contains(".gather("), "should emit .gather() for tensor indexing by tensor\n{}", rust);
    }

    // ===== Norm transpiler tests =====

    #[test]
    fn test_norm_l2_transpile() {
        let rust = transpile("use math\nx = norm[ones[3]]\n");
        assert!(rust.contains(".norm(2.0)"), "should emit .norm(2.0) for L2 norm\n{}", rust);
    }

    #[test]
    fn test_norm_lp_transpile() {
        let rust = transpile("use math\nx = norm[ones[3], 1]\n");
        assert!(rust.contains(".norm("), "should emit .norm() for Lp norm\n{}", rust);
    }

    // ===== Layer 3: Random & Distributions =====

    #[test]
    fn test_math_seed_transpile() {
        let rust = transpile("use math\nseed[42]\n");
        assert!(rust.contains("set_seed(42"), "should emit set_seed\n{}", rust);
    }

    #[test]
    fn test_math_normal_transpile() {
        let rust = transpile("use math\nx = normal[0, 1, 3, 4]\n");
        assert!(rust.contains("CrowTensor::normal("), "should emit CrowTensor::normal\n{}", rust);
    }

    #[test]
    fn test_math_uniform_transpile() {
        let rust = transpile("use math\nx = uniform[2, 5, 6]\n");
        assert!(rust.contains("CrowTensor::rand_range("), "should emit CrowTensor::rand_range\n{}", rust);
    }

    #[test]
    fn test_math_poisson_transpile() {
        let rust = transpile("use math\nx = poisson[3.5, 4]\n");
        assert!(rust.contains("CrowTensor::poisson("), "should emit CrowTensor::poisson\n{}", rust);
    }

    #[test]
    fn test_math_bernoulli_transpile() {
        let rust = transpile("use math\nx = bernoulli[0.5, 8]\n");
        assert!(rust.contains("CrowTensor::bernoulli("), "should emit CrowTensor::bernoulli\n{}", rust);
    }

    #[test]
    fn test_math_shuffle_transpile() {
        let rust = transpile("use math\nx = shuffle[ones[5]]\n");
        assert!(rust.contains(".shuffle()"), "should emit .shuffle()\n{}", rust);
    }

    #[test]
    fn test_math_choice_transpile() {
        let rust = transpile("use math\nx = choice[ones[5], 3]\n");
        assert!(rust.contains(".choice("), "should emit .choice()\n{}", rust);
    }

    #[test]
    fn test_math_permutation_transpile() {
        let rust = transpile("use math\nx = permutation[5]\n");
        assert!(rust.contains("CrowTensor::permutation("), "should emit CrowTensor::permutation\n{}", rust);
    }

    // ===== Layer 4: Signal Processing =====

    #[test]
    fn test_math_fft_transpile() {
        let rust = transpile("use math\nx = fft[ones[4]]\n");
        assert!(rust.contains(".fft()"), "should emit .fft()\n{}", rust);
    }

    #[test]
    fn test_math_ifft_transpile() {
        let rust = transpile("use math\nx = ifft[ones[4], zeros[4]]\n");
        assert!(rust.contains("CrowTensor::ifft("), "should emit CrowTensor::ifft\n{}", rust);
    }

    #[test]
    fn test_math_fft2_transpile() {
        let rust = transpile("use math\nx = fft2[ones[4]]\n");
        assert!(rust.contains(".fft2()"), "should emit .fft2()\n{}", rust);
    }

    #[test]
    fn test_math_conv_transpile() {
        let rust = transpile("use math\nx = conv[ones[3], ones[2]]\n");
        assert!(rust.contains(".conv("), "should emit .conv()\n{}", rust);
    }

    #[test]
    fn test_math_conv2d_transpile() {
        let rust = transpile("use math\nx = conv2d[ones[4], ones[4]]\n");
        assert!(rust.contains(".conv2d("), "should emit .conv2d()\n{}", rust);
    }

    #[test]
    fn test_math_hamming_transpile() {
        let rust = transpile("use math\nx = hamming[5]\n");
        assert!(rust.contains("CrowTensor::hamming("), "should emit CrowTensor::hamming\n{}", rust);
    }

    #[test]
    fn test_math_hanning_transpile() {
        let rust = transpile("use math\nx = hanning[5]\n");
        assert!(rust.contains("CrowTensor::hanning("), "should emit CrowTensor::hanning\n{}", rust);
    }

    // ===== Layer 5: Special Functions & Complex Numbers =====

    #[test]
    fn test_math_gamma_transpile() {
        let rust = transpile("use math\nx = gamma[ones[3]]\n");
        assert!(rust.contains(".gamma()"), "should emit .gamma()\n{}", rust);
    }

    #[test]
    fn test_math_erf_transpile() {
        let rust = transpile("use math\nx = erf[ones[3]]\n");
        assert!(rust.contains(".erf()"), "should emit .erf()\n{}", rust);
    }

    #[test]
    fn test_math_bessel_transpile() {
        let rust = transpile("use math\nx = bessel[ones[3], 0]\n");
        assert!(rust.contains(".bessel("), "should emit .bessel()\n{}", rust);
    }

    #[test]
    fn test_math_factorial_transpile() {
        let rust = transpile("use math\nx = factorial[ones[3]]\n");
        assert!(rust.contains(".factorial()"), "should emit .factorial()\n{}", rust);
    }

    #[test]
    fn test_math_choose_transpile() {
        let rust = transpile("use math\nx = choose[ones[3], ones[3]]\n");
        assert!(rust.contains(".choose("), "should emit .choose()\n{}", rust);
    }

    #[test]
    fn test_math_complex_transpile() {
        let rust = transpile("use math\nc = complex[3, 4]\n");
        assert!(rust.contains("CrowTensor::complex("), "should emit CrowTensor::complex\n{}", rust);
    }

    #[test]
    fn test_math_magnitude_transpile() {
        let rust = transpile("use math\nc = complex[3, 4]\nm = magnitude[c]\n");
        assert!(rust.contains("CrowTensor::magnitude("), "should emit CrowTensor::magnitude\n{}", rust);
    }

    #[test]
    fn test_math_conjugate_transpile() {
        let rust = transpile("use math\nc = complex[3, 4]\ncc = conjugate[c]\n");
        assert!(rust.contains("CrowTensor::conjugate("), "should emit CrowTensor::conjugate\n{}", rust);
    }

    // ===== Sparse Arrays =====

    #[test]
    fn test_math_sparse_from_dense_transpile() {
        let rust = transpile("use math\ns = sparse[eye[3]]\n");
        assert!(rust.contains("CrowSparse::from_dense("), "should emit CrowSparse::from_dense\n{}", rust);
    }

    #[test]
    fn test_math_sparse_from_coo_transpile() {
        let rust = transpile("use math\ns = sparse[[0, 1], [0, 1], [5, 6], [2, 2]]\n");
        assert!(rust.contains("CrowSparse::from_coo("), "should emit CrowSparse::from_coo\n{}", rust);
    }

    #[test]
    fn test_math_dense_transpile() {
        let rust = transpile("use math\ns = sparse[eye[2]]\nd = dense[s]\n");
        assert!(rust.contains(".to_dense()"), "should emit .to_dense()\n{}", rust);
    }

    #[test]
    fn test_sparse_matmul_transpile() {
        let rust = transpile("use math\ns = sparse[eye[2]]\nv = [1, 2]\nr = s * v\n");
        assert!(rust.contains(".matmul_dense("), "should emit .matmul_dense()\n{}", rust);
    }

    #[test]
    fn test_sparse_add_transpile() {
        let rust = transpile("use math\na = sparse[eye[2]]\nb = sparse[eye[2]]\nc = a + b\n");
        assert!(rust.contains(".add("), "should emit .add() for sparse+sparse\n{}", rust);
    }

    #[test]
    fn test_sparse_solve_transpile() {
        let rust = transpile("use math\ns = sparse[eye[2]]\nx = solve[s, [1, 2]]\n");
        assert!(rust.contains(".solve("), "should emit .solve() for sparse solve\n{}", rust);
    }

    #[test]
    fn test_sparse_type_transpile() {
        let rust = transpile("use math\ns = sparse[eye[2]]\nt = type[s]\n");
        assert!(rust.contains("\"sparse\""), "should emit 'sparse' for type[]\n{}", rust);
    }

    #[test]
    fn test_sparse_import_transpile() {
        let rust = transpile("use math\ns = sparse[eye[2]]\n");
        assert!(rust.contains("use crow_runtime::sparse::CrowSparse"), "should import CrowSparse\n{}", rust);
    }

    // ===== Stats sub-module =====

    #[test]
    fn test_stats_median_transpile() {
        let rust = transpile("use math\nuse math.stats\nx = median[ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::median("), "should emit stats::median\n{}", rust);
    }

    #[test]
    fn test_stats_std_transpile() {
        let rust = transpile("use math\nuse math.stats\nx = std[ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::std("), "should emit stats::std\n{}", rust);
    }

    #[test]
    fn test_stats_var_transpile() {
        let rust = transpile("use math\nuse math.stats\nx = var[ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::var("), "should emit stats::var\n{}", rust);
    }

    #[test]
    fn test_stats_percentile_transpile() {
        let rust = transpile("use math\nuse math.stats\nx = percentile[ones[5], 50]\n");
        assert!(rust.contains("crow_runtime::stats::percentile("), "should emit stats::percentile\n{}", rust);
    }

    #[test]
    fn test_stats_zscore_transpile() {
        let rust = transpile("use math\nuse math.stats\nz = zscore[ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::zscore("), "should emit stats::zscore\n{}", rust);
    }

    #[test]
    fn test_stats_corr_transpile() {
        let rust = transpile("use math\nuse math.stats\nc = corr[ones[5], ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::corr("), "should emit stats::corr\n{}", rust);
    }

    #[test]
    fn test_stats_cov_transpile() {
        let rust = transpile("use math\nuse math.stats\nc = cov[ones[5], ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::cov("), "should emit stats::cov\n{}", rust);
    }

    #[test]
    fn test_stats_ttest_transpile() {
        let rust = transpile("use math\nuse math.stats\nr = ttest[ones[5], ones[5]]\n");
        assert!(rust.contains("crow_runtime::stats::ttest("), "should emit stats::ttest\n{}", rust);
    }

    // ===== File System Builtins =====

    #[test]
    fn test_fs_exists_transpile() {
        let rust = transpile("e = exists['foo.txt']\n");
        assert!(rust.contains("std::path::Path::new("), "should emit Path::new for exists\n{}", rust);
        assert!(rust.contains(".exists()"), "should emit .exists()\n{}", rust);
    }

    #[test]
    fn test_fs_mkdir_transpile() {
        let rust = transpile("mkdir['dir']\n");
        assert!(rust.contains("std::fs::create_dir_all("), "should emit create_dir_all for mkdir\n{}", rust);
    }

    #[test]
    fn test_fs_ls_transpile() {
        let rust = transpile("f = ls['.']\n");
        assert!(rust.contains("std::fs::read_dir("), "should emit read_dir for ls\n{}", rust);
    }

    #[test]
    fn test_fs_rm_transpile() {
        let rust = transpile("rm['file.txt']\n");
        assert!(rust.contains("remove_dir_all") || rust.contains("remove_file"), "should emit remove for rm\n{}", rust);
    }

    #[test]
    fn test_fs_mv_transpile() {
        let rust = transpile("mv['a.txt', 'b.txt']\n");
        assert!(rust.contains("std::fs::rename("), "should emit fs::rename for mv\n{}", rust);
    }

    #[test]
    fn test_fs_cp_transpile() {
        let rust = transpile("cp['a.txt', 'b.txt']\n");
        assert!(rust.contains("std::fs::copy("), "should emit fs::copy for cp\n{}", rust);
    }

    // ===== env builtin =====

    #[test]
    fn test_env_one_arg_transpile() {
        let rust = transpile("x = env['PORT']\n");
        assert!(rust.contains("std::env::var("), "should emit std::env::var\n{}", rust);
        assert!(rust.contains("unwrap_or_default()"), "should emit unwrap_or_default for 1-arg env\n{}", rust);
    }

    #[test]
    fn test_env_two_arg_transpile() {
        let rust = transpile("x = env['PORT', 3000]\n");
        assert!(rust.contains("std::env::var("), "should emit std::env::var\n{}", rust);
        assert!(rust.contains("unwrap_or_else"), "should emit unwrap_or_else for 2-arg env\n{}", rust);
    }

    // ===== Web transpiler tests =====

    #[test]
    fn test_web_new_transpile() {
        let rust = transpile("use web\napp = web.new[]\n");
        assert!(rust.contains("CrowApp::new()"), "should emit CrowApp::new()\n{}", rust);
    }

    #[test]
    fn test_web_imports() {
        let rust = transpile("use web\napp = web.new[]\n");
        assert!(rust.contains("use crow_web::server::CrowApp;"), "should import CrowApp\n{}", rust);
        assert!(rust.contains("use crow_web::core::{CrowRequest, CrowResponse};"), "should import CrowRequest/CrowResponse\n{}", rust);
        assert!(rust.contains("use std::sync::Arc;"), "should import Arc\n{}", rust);
    }

    #[test]
    fn test_web_async_main() {
        let rust = transpile("use web\napp = web.new[]\n");
        assert!(rust.contains("#[tokio::main]"), "should emit #[tokio::main]\n{}", rust);
        assert!(rust.contains("async fn main()"), "should emit async fn main\n{}", rust);
    }

    #[test]
    fn test_web_route_transpile() {
        let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains(".route("), "should emit .route()\n{}", rust);
        assert!(rust.contains("Arc::new("), "should wrap handler in Arc\n{}", rust);
    }

    #[test]
    fn test_web_serve_transpile() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nfn handle[s, e, v]\n    log[e]\n\nserve[3000]\npage['/', render, handle]\n");
        assert!(rust.contains("__crow_app.serve("), "should emit __crow_app.serve()\n{}", rust);
        assert!(rust.contains(".await"), "should emit .await\n{}", rust);
    }

    #[test]
    fn test_web_text_response() {
        let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::text("), "should emit CrowResponse::text()\n{}", rust);
    }

    #[test]
    fn test_web_html_response() {
        let rust = transpile("use web\nfn handler[req]\n    return html['<h1>hi</h1>']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::html("), "should emit CrowResponse::html()\n{}", rust);
    }

    #[test]
    fn test_web_redirect_response() {
        let rust = transpile("use web\nfn handler[req]\n    return redirect['/home']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::redirect("), "should emit CrowResponse::redirect()\n{}", rust);
    }

    #[test]
    fn test_web_respond_response() {
        let rust = transpile("use web\nfn handler[req]\n    return respond[404, 'not found']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::respond("), "should emit CrowResponse::respond()\n{}", rust);
    }

    #[test]
    fn test_web_handler_types() {
        let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        // Handler should have CrowRequest param and CrowResponse return
        assert!(rust.contains("crow_web::core::CrowRequest"), "handler param should be CrowRequest\n{}", rust);
        assert!(rust.contains("crow_web::core::CrowResponse"), "handler return should be CrowResponse\n{}", rust);
    }

    #[test]
    fn test_web_no_async_without_web() {
        let rust = transpile("x = 1 + 2\n");
        assert!(!rust.contains("async fn main"), "should NOT emit async main without web\n{}", rust);
        assert!(!rust.contains("#[tokio::main]"), "should NOT emit tokio::main without web\n{}", rust);
    }

    // --- P4: Web module codegen tests ---

    #[test]
    fn test_web_cors_permissive() {
        let rust = transpile("use web\nuse web.cors\napp = web.new[]\napp = cors[app]\n");
        assert!(rust.contains(".cors_permissive()"), "should emit cors_permissive()\n{}", rust);
    }

    #[test]
    fn test_web_cors_with_origins() {
        let rust = transpile("use web\nuse web.cors\napp = web.new[]\norigins = ['http://localhost']\napp = cors[app, origins]\n");
        assert!(rust.contains(".cors(CrowCors::origins("), "should emit cors with origins\n{}", rust);
    }

    #[test]
    fn test_web_cors_import() {
        let rust = transpile("use web\nuse web.cors\napp = web.new[]\napp = cors[app]\n");
        assert!(rust.contains("use crow_web::cors::CrowCors;"), "should import CrowCors\n{}", rust);
    }

    #[test]
    fn test_web_sessions() {
        let rust = transpile("use web\nuse web.session\nstore = sessions[]\n");
        assert!(rust.contains("CrowSessionStore::new()"), "should emit CrowSessionStore::new()\n{}", rust);
    }

    #[test]
    fn test_web_sessions_import() {
        let rust = transpile("use web\nuse web.session\nstore = sessions[]\n");
        assert!(rust.contains("use crow_web::session::CrowSessionStore;"), "should import CrowSessionStore\n{}", rust);
    }

    #[test]
    fn test_web_templates() {
        let rust = transpile("use web\nuse web.template\nt = templates['./templates/**/*']\n");
        assert!(rust.contains("CrowTemplates::from_glob("), "should emit CrowTemplates::from_glob()\n{}", rust);
    }

    #[test]
    fn test_web_render() {
        let rust = transpile("use web\nuse web.template\nt = templates['./t/**/*']\nfn handler[req]\n    data = map[name = 'crow']\n    return html[render[t, 'index', data]]\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains(".render("), "should emit .render()\n{}", rust);
    }

    #[test]
    fn test_web_cache() {
        let rust = transpile("use web\nuse web.cache\nc = cache[100]\n");
        assert!(rust.contains("CrowCache::new(100"), "should emit CrowCache::new()\n{}", rust);
    }

    #[test]
    fn test_web_cache_with_ttl() {
        let rust = transpile("use web\nuse web.cache\nc = cache[100, 60]\n");
        assert!(rust.contains("CrowCache::with_ttl("), "should emit CrowCache::with_ttl()\n{}", rust);
    }

    #[test]
    fn test_web_limit() {
        let rust = transpile("use web\nuse web.rate\nlimiter = limit[100, 60]\n");
        assert!(rust.contains("CrowRateLimiter::new("), "should emit CrowRateLimiter::new()\n{}", rust);
    }

    #[test]
    fn test_web_limit_import() {
        let rust = transpile("use web\nuse web.rate\nlimiter = limit[100, 60]\n");
        assert!(rust.contains("use crow_web::rate::CrowRateLimiter;"), "should import CrowRateLimiter\n{}", rust);
    }

    #[test]
    fn test_web_sse() {
        let rust = transpile("use web\nuse web.sse\nfn events[req]\n    return text['data']\napp = web.new[]\napp = sse[app, '/events', events]\n");
        assert!(rust.contains(".sse_route("), "should emit .sse_route()\n{}", rust);
    }

    #[test]
    fn test_web_ws() {
        let rust = transpile("use web\nuse web.ws\nfn handler[req]\n    return text['ok']\napp = web.new[]\napp = ws[app, '/ws', handler]\n");
        assert!(rust.contains(".ws_route("), "should emit .ws_route()\n{}", rust);
    }

    #[test]
    fn test_web_static_dir() {
        let rust = transpile("use web\nuse web.static\napp = web.new[]\napp = static_dir[app, '/assets', './public']\n");
        assert!(rust.contains(".static_dir("), "should emit .static_dir()\n{}", rust);
    }

    #[test]
    fn test_web_smtp() {
        let rust = transpile("use web\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']\n");
        assert!(rust.contains("CrowMailer::new("), "should emit CrowMailer::new()\n{}", rust);
    }

    #[test]
    fn test_web_smtp_import() {
        let rust = transpile("use web\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']\n");
        assert!(rust.contains("use crow_web::mail::CrowMailer;"), "should import CrowMailer\n{}", rust);
    }

    #[test]
    fn test_web_connect() {
        let rust = transpile("use web\nuse web.ws\nurl = 'ws://localhost:3000'\nsock = connect[url]\n");
        assert!(rust.contains("ws::connect("), "should emit ws::connect()\n{}", rust);
    }

    #[test]
    fn test_web_send_recv_close() {
        let rust = transpile("use web\nuse web.ws\nurl = 'ws://localhost:3000'\nsock = connect[url]\nsend[sock, 'hi']\nmsg = recv[sock]\nclose[sock]\n");
        assert!(rust.contains(".send("), "should emit .send()\n{}", rust);
        assert!(rust.contains(".recv()"), "should emit .recv()\n{}", rust);
        assert!(rust.contains(".close()"), "should emit .close()\n{}", rust);
    }

    #[test]
    fn test_web_queue() {
        let rust = transpile("use web\nuse web.queue\nfn job[req]\n    return text['done']\nq = web.queue[4]\n");
        assert!(rust.contains("CrowQueue::new("), "should emit CrowQueue::new()\n{}", rust);
    }

    #[test]
    fn test_web_docs() {
        let rust = transpile("use web\nuse web.docs\nd = web.docs['My API', '1.0']\n");
        assert!(rust.contains("crow_web::docs::CrowDocs::new("), "should emit CrowDocs::new()\n{}", rust);
    }

    #[test]
    fn test_web_uploads() {
        let rust = transpile("use web\nuse web.upload\nfn handler[req]\n    files = uploads[req]\n    return text['ok']\napp = web.new[]\napp = route[app, '/upload', handler]\n");
        assert!(rust.contains(".uploads()"), "should emit .uploads()\n{}", rust);
    }

    // --- DB module codegen tests ---

    #[test]
    fn test_db_connect() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db']\n");
        assert!(rust.contains("crow_db::connect("), "should emit crow_db::connect()\n{}", rust);
        assert!(rust.contains("None"), "should pass None for default pool\n{}", rust);
    }

    #[test]
    fn test_db_connect_pool_size() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db', 5]\n");
        assert!(rust.contains("crow_db::connect("), "should emit crow_db::connect()\n{}", rust);
        assert!(rust.contains("Some(5"), "should pass Some(5) for pool size\n{}", rust);
    }

    #[test]
    fn test_db_connect_prefixed() {
        let rust = transpile("use db\nconn = db.connect['sqlite://test.db']\n");
        assert!(rust.contains("crow_db::connect("), "should emit crow_db::connect() for db.connect\n{}", rust);
    }

    #[test]
    fn test_db_exec() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db']\nexec[conn, 'CREATE TABLE t [id INT]']\n");
        assert!(rust.contains(".exec("), "should emit .exec()\n{}", rust);
        assert!(rust.contains(".unwrap()"), "should unwrap result\n{}", rust);
    }

    #[test]
    fn test_db_query() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db']\ndf = query[conn, 'SELECT * FROM t']\n");
        assert!(rust.contains(".query("), "should emit .query()\n{}", rust);
    }

    #[test]
    fn test_db_migrate() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db']\nn = migrate[conn, 'migrations/']\n");
        assert!(rust.contains(".migrate("), "should emit .migrate()\n{}", rust);
    }

    #[test]
    fn test_db_import() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db']\n");
        assert!(rust.contains("use crow_db::DbHandle;"), "should import DbHandle\n{}", rust);
    }

    #[test]
    fn test_db_insert() {
        let rust = transpile("use db\nconn = connect['sqlite://test.db']\ndf = query[conn, 'SELECT * FROM t']\ninsert[conn, 'backup', df]\n");
        assert!(rust.contains(".insert("), "should emit .insert()\n{}", rust);
    }

    // --- P4: Response format codegen tests ---

    #[test]
    fn test_web_msgpack_response() {
        let rust = transpile("use web\nfn handler[req]\n    m = map[]\n    return msgpack[m]\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::msgpack("), "should emit CrowResponse::msgpack()\n{}", rust);
        assert!(rust.contains("rmp_serde::to_vec"), "should use rmp_serde\n{}", rust);
    }

    #[test]
    fn test_web_arrow_response() {
        let rust = transpile("use web\nuse frame\nfn handler[req]\n    f = load['data.csv']\n    return arrow[f]\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::arrow_ipc("), "should emit CrowResponse::arrow_ipc()\n{}", rust);
        assert!(rust.contains("to_arrow_ipc"), "should use to_arrow_ipc\n{}", rust);
    }

    #[test]
    fn test_web_bytes_response() {
        let rust = transpile("use web\nfn handler[req]\n    return bytes['binary data']\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("CrowResponse::binary("), "should emit CrowResponse::binary()\n{}", rust);
        assert!(rust.contains("application/octet-stream"), "should set octet-stream content-type\n{}", rust);
    }

    #[test]
    fn test_web_stream_response() {
        let rust = transpile("use web\nfn sender[send]\n    send['chunk1']\n\nfn handler[req]\n    return stream[sender]\n\napp = web.new[]\napp = route[app, '/', handler]\n");
        assert!(rust.contains("mpsc::channel"), "should use mpsc channel for streaming\n{}", rust);
        assert!(rust.contains("Body::from_stream"), "should use Body::from_stream\n{}", rust);
    }

    // --- UI / LiveView transpiler tests ---

    #[test]
    fn test_ui_imports() {
        let rust = transpile("use web\nuse ui\napp = web.new[]\n");
        assert!(rust.contains("use crow_web::view::CrowVNode;"), "should import CrowVNode\n{}", rust);
        assert!(rust.contains("use crow_web::live::{CrowLiveView, CrowLiveState};"), "should import CrowLiveView/CrowLiveState\n{}", rust);
    }

    #[test]
    fn test_ui_div_text() {
        let rust = transpile("use web\nuse ui\nx = div[h1['hello']]\n");
        assert!(rust.contains("CrowVNode::div(vec![CrowVNode::h1(vec![CrowVNode::text("), "should emit CrowVNode::div with h1 child\n{}", rust);
    }

    #[test]
    fn test_ui_button_click() {
        let rust = transpile("use web\nuse ui\nview v[]\n    return button['+', click = 'inc']\n");
        assert!(rust.contains("CrowVNode::button(vec![CrowVNode::text("), "should emit CrowVNode::button\n{}", rust);
        assert!(rust.contains(".on_click("), "should emit .on_click()\n{}", rust);
    }

    #[test]
    fn test_ui_input_via_module() {
        let rust = transpile("use web\nuse ui\nview v[]\n    return ui.input[type = 'text', input = 'change']\n");
        assert!(rust.contains("CrowVNode::input()"), "should emit CrowVNode::input()\n{}", rust);
        assert!(rust.contains(".attr("), "should emit .attr() for type\n{}", rust);
        assert!(rust.contains(".on_input("), "should emit .on_input()\n{}", rust);
    }

    #[test]
    fn test_ui_page_3arg() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nfn handle[s, e, v]\n    log[e]\n\nserve[3000]\npage['/counter', render, handle]\n");
        assert!(rust.contains("CrowLiveView::new("), "should emit CrowLiveView::new\n{}", rust);
        assert!(rust.contains("let mut __crow_state = CrowLiveState::new()"), "should init __crow_state\n{}", rust);
        assert!(rust.contains("__crow_state.clone()"), "should pass __crow_state.clone() to LiveView\n{}", rust);
        assert!(rust.contains("into_router("), "should emit into_router\n{}", rust);
        assert!(rust.contains("__crow_app = __crow_app.merge("), "should merge into implicit app\n{}", rust);
    }

    #[test]
    fn test_ui_page_2arg_noop_handler() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nserve[3000]\npage['/dash', render]\n");
        assert!(rust.contains("CrowLiveView::new("), "should emit CrowLiveView::new\n{}", rust);
        assert!(rust.contains("Option<&str>"), "no-op handler should use Option<&str> for value param\n{}", rust);
        assert!(rust.contains("__crow_state"), "should init __crow_state\n{}", rust);
    }

    #[test]
    fn test_ui_nested_elements() {
        let rust = transpile("use web\nuse ui\nx = div[span['a'], p['b']]\n");
        assert!(rust.contains("CrowVNode::div(vec![CrowVNode::span("), "should emit nested elements\n{}", rust);
        assert!(rust.contains("CrowVNode::p(vec![CrowVNode::text("), "should emit p element\n{}", rust);
    }

    #[test]
    fn test_ui_attr_class() {
        let rust = transpile("use web\nuse ui\nview v[]\n    return div[h1['title'], class = 'container']\n");
        assert!(rust.contains(".attr("), "should emit .attr() for class\n{}", rust);
        assert!(rust.contains("container"), "should contain class value\n{}", rust);
    }

    #[test]
    fn test_ui_state_get_int() {
        let rust = transpile("use web\nuse ui\nview render[state]\n    count = state.get['count', 0]\n    return div[h1['{count}']]\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains(".get_i64("), "state.get with int default should map to get_i64\n{}", rust);
        assert!(!rust.contains("get_i64["), "should not contain Crow syntax in output\n{}", rust);
    }

    #[test]
    fn test_ui_state_get_str() {
        let rust = transpile("use web\nuse ui\nview render[state]\n    name = state.get['name', '']\n    return div[h1['{name}']]\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains(".get_str("), "state.get with str default should map to get_str\n{}", rust);
    }

    #[test]
    fn test_ui_state_get_bool() {
        let rust = transpile("use web\nuse ui\nview render[state]\n    active = state.get['active', false]\n    return div[h1['{active}']]\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains(".get_bool("), "state.get with bool default should map to get_bool\n{}", rust);
    }

    #[test]
    fn test_ui_fetch_assignment() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\ndata = fetch['/api/data']\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains("register_fetch(\"data\""), "fetch assignment should emit register_fetch\n{}", rust);
        assert!(!rust.contains("let data"), "fetch should not create a local variable\n{}", rust);
    }

    #[test]
    fn test_ui_poll_assignment() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nprices = poll['/api/prices', 5]\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains("register_poll(\"prices\""), "poll assignment should emit register_poll\n{}", rust);
        assert!(rust.contains("as u64"), "poll should pass interval as u64\n{}", rust);
    }

    #[test]
    fn test_ui_field_codegen() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    vol = s.get['vol', 0]\n    return div[field['Min Vol', vol]]\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains("CrowVNode::field("), "field should emit CrowVNode::field\n{}", rust);
        assert!(rust.contains("\"vol\""), "field should extract key name\n{}", rust);
    }

    #[test]
    fn test_ui_select_codegen() {
        let rust = transpile("use web\nuse ui\nview render[s]\n    opts = ['a', 'b']\n    val = s.get['choice', 'a']\n    return div[select[opts, val]]\nserve[3000]\npage['/', render]\n");
        assert!(rust.contains("CrowVNode::select_input("), "select should emit CrowVNode::select_input\n{}", rust);
        assert!(rust.contains("\"val\""), "select should extract key name\n{}", rust);
    }

    #[test]
    fn test_reactive_counter() {
        let rust = transpile("use web, ui\n\ncount = 0\n\nfn inc[]\n    count = count + 1\n\nfn dec[]\n    count = count - 1\n\nview render[]\n    return div[\n        h1['Count: {count}'],\n        button['+', click = inc],\n        button['-', click = dec]\n    ]\n\nserve[3000]\npage['/', render]\n");
        // Reactive render function should get state param injected
        assert!(rust.contains("fn render(state: &CrowLiveState) -> CrowVNode"), "render should have injected state param\n{}", rust);
        // Reactive handler functions should get mut state param injected
        assert!(rust.contains("fn inc(state: &mut CrowLiveState)"), "inc should have injected mut state param\n{}", rust);
        assert!(rust.contains("fn dec(state: &mut CrowLiveState)"), "dec should have injected mut state param\n{}", rust);
        // State preamble: state.get_i64 for reactive vars
        assert!(rust.contains("state.get_i64(\"count\""), "should inject state.get_i64 preamble\n{}", rust);
        // State epilogue: state.set for reactive vars in handlers
        assert!(rust.contains("state.set(\"count\""), "should inject state.set epilogue in handlers\n{}", rust);
        // Dispatch function
        assert!(rust.contains("fn __crow_handle(state: &mut CrowLiveState, event: &str, _value: Option<&str>)"), "should generate __crow_handle dispatch\n{}", rust);
        assert!(rust.contains("\"inc\" => inc(state)"), "dispatch should route inc\n{}", rust);
        assert!(rust.contains("\"dec\" => dec(state)"), "dispatch should route dec\n{}", rust);
        // Event attributes should use string, not reference
        assert!(rust.contains(".on_click(\"inc\")"), "click = inc should emit .on_click(\"inc\")\n{}", rust);
        assert!(rust.contains(".on_click(\"dec\")"), "click = dec should emit .on_click(\"dec\")\n{}", rust);
        // Main should register defaults
        assert!(rust.contains("__crow_state.set(\"count\", serde_json::json!(0"), "should register count default in main\n{}", rust);
        // page[] should auto-inject __crow_handle
        assert!(rust.contains("__crow_handle"), "page should auto-inject __crow_handle\n{}", rust);
    }

    #[test]
    fn test_serve_refresh_budget_emission() {
        let rust = transpile("use web, ui\n\ncount = 0\n\nview render[]\n    return div[p['{count}']]\n\npage['/', render]\nserve[3000, refresh = 5, budget = 50]\n");
        assert!(
            rust.contains("crow_web::live::configure(5_i64 as i64, 50_i64 as i64)"),
            "serve named args should configure the live runtime\n{}",
            rust
        );
        assert!(rust.contains("__crow_app.serve(3000_i64 as u16)"), "{}", rust);
    }

    #[test]
    fn test_later_script_emission() {
        let rust = transpile("fn work[msg]\n    log[msg]\n\nlater[work, 'hi']\n");
        assert!(
            rust.contains("crow_runtime::later::spawn(\"work\""),
            "script later[] should emit kernel spawn\n{}",
            rust
        );
        assert!(
            rust.contains("crow_runtime::later::wait_all()"),
            "main should wait for stragglers at exit\n{}",
            rust
        );
    }

    #[test]
    fn test_later_handler_emission() {
        let rust = transpile("use web, ui\n\ncount = 0\n\nfn slow_work[]\n    count = count + 1\n\nfn go[]\n    later[slow_work]\n\nview render[]\n    return div[p['{count}'], button['go', click = go]]\n\npage['/', render]\nserve[3000]\n");
        assert!(
            rust.contains("state.spawn_later(\"slow_work\", slow_work)"),
            "handler later[] should go through session state\n{}",
            rust
        );
        assert!(
            rust.contains("fn slow_work(state: &mut CrowLiveState)"),
            "later target must compile as a handler-class fn\n{}",
            rust
        );
    }

    #[test]
    fn test_reactive_vars_skipped_in_main() {
        let rust = transpile("use web, ui\n\ncount = 0\n\nview render[]\n    return div[h1['Count: {count}']]\n\nserve[3000]\npage['/', render]\n");
        // count should NOT appear as a `let count` in main — it's registered via state.set
        let main_section = rust.split("fn main()").nth(1).unwrap_or("");
        assert!(!main_section.contains("let count"), "reactive var should not appear as local var in main\n{}", rust);
    }

    // ===== Boolean mask semantics (frame API: strings + expressions) =====

    #[test]
    fn test_mask_and_emits_elementwise_zip() {
        let rust = transpile("use frame\ndf = load['d.csv']\nm = df.age gt 5 and df.age lt 30\n");
        assert!(rust.contains(".zip("), "and on bool masks should zip element-wise\n{}", rust);
    }

    #[test]
    fn test_mask_not_emits_elementwise_negation() {
        let rust = transpile("use frame\ndf = load['d.csv']\nm = not (df.age gt 5)\n");
        assert!(rust.contains("!*_b"), "not on bool mask should negate element-wise\n{}", rust);
    }

    #[test]
    fn test_not_comparison_keeps_parens() {
        // Regression: `not (x eq 8)` must not compile to `!x == 8` (Rust ! binds tighter)
        let rust = transpile("x = 7\nif not (x eq 8)\n    log['ok']\n");
        assert!(rust.contains("(!("), "not must parenthesize its operand\n{}", rust);
    }

    #[test]
    fn test_implicit_return_gives_fn_return_type() {
        // Regression: implicit last-expression return must produce a typed fn
        let rust = transpile("fn add[a, b]\n    a + b\nr = add[3, 4]\nlog[r]\n");
        assert!(rust.contains("-> i64"), "implicit return should type the function\n{}", rust);
    }

    #[test]
    fn test_user_variable_shadows_module_constant() {
        // Regression: `e = eye[2]` then `log[e]` must reference the variable, not Euler's e
        let rust = transpile("use math\ne = eye[2]\nlog[e]\n");
        assert!(!rust.contains("consts::E"), "user variable must shadow constant\n{}", rust);
    }

    #[test]
    fn test_find_emits_struct_result() {
        // Regression: compiled find[] previously emitted an uncompilable mixed-type map
        let rust = transpile("m = find['abc', '[a-z]+']\nlog[m.ok]\n");
        assert!(rust.contains("CrowFindResult"), "find should use the synthetic struct\n{}", rust);
    }

    #[test]
    fn test_monomorphization_clones_per_signature() {
        // F2: a function used at two types compiles as two clones
        let rust = transpile("fn first[arr]\n    return arr[0]\nlog[first[[1, 2]]]\nlog[first[['a', 'b']]]\n");
        assert!(rust.contains("fn first__1"), "should emit first clone\n{}", rust);
        assert!(rust.contains("fn first__2"), "should emit second clone\n{}", rust);
        assert!(!rust.contains("fn first("), "original should not be emitted\n{}", rust);
    }

    #[test]
    fn test_single_signature_functions_untouched() {
        let rust = transpile("fn double[x]\n    return x * 2\nlog[double[5]]\nlog[double[7]]\n");
        assert!(rust.contains("fn double("), "single-sig fn keeps its name\n{}", rust);
        assert!(!rust.contains("double__"), "no clones for single signature\n{}", rust);
    }

    #[test]
    fn test_search_builtins_emit_parallel_early_exit() {
        let rust = transpile("fn big[x]\n    return x gt 10\na = any[[1, 2], big]\nlog[a]\n");
        assert!(rust.contains("par_iter().any"), "any should be parallel\n{}", rust);
        let rust = transpile("fn big[x]\n    return x gt 10\nh = find[[1, 2], big]\nlog[h.ok]\n");
        assert!(rust.contains("find_first"), "find should use parallel find_first\n{}", rust);
    }

    #[test]
    fn test_fmt_emits_kernel_call() {
        let rust = transpile("x = fmt[1234.5, decimals = 2, commas = true]\nlog[x]\n");
        assert!(rust.contains("crow_fmt_f64"), "fmt should call the display kernel\n{}", rust);
        let rust = transpile("x = fmt['hi', width = 4]\nlog[x]\n");
        assert!(rust.contains("crow_fmt_str"), "string fmt should call the str kernel\n{}", rust);
    }

    #[test]
    fn test_fallible_condition_guarded_and_return_widened() {
        // A fn with a fallible condition returns CrowResult and guards the branch
        let rust = transpile("fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nr = check['9']\nlog[r]\n");
        assert!(rust.contains("if !_c.ok"), "condition should be guarded\n{}", rust);
        assert!(rust.contains("-> CrowResult<String>"), "return type should widen to Result\n{}", rust);
    }

    #[test]
    fn test_float_var_declared_unified() {
        // int-then-float reassignment declares the variable as f64 and coerces
        let rust = transpile("v = 10\nv = 2.5\nv = 1\nlog[v]\n");
        assert!(rust.contains("let mut v: f64"), "v should be declared f64\n{}", rust);
        assert!(rust.contains("as f64"), "int assignments should coerce\n{}", rust);
    }

    #[test]
    fn test_frame_column_args_are_plain_strings() {
        // Regression: bare-name force conversion removed — quoted strings pass through
        let rust = transpile("use frame\ndf = load['d.csv']\ns = sort[df, 'age', desc = true]\n");
        assert!(rust.contains("\"age\""), "column name string should appear in codegen\n{}", rust);
    }
}