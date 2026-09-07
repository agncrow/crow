/// Validation pass for Crow programs.
/// Runs after type inference, before transpilation.
/// Catches user errors and reports them in Crow terms.

use std::collections::BTreeSet;
use crate::ast::*;
use crate::type_infer::{CrowType, TypeContext};

/// All builtin function names recognized by the Crow language.
/// True core — reserved in every Crow file, forever (frozen by the budget
/// rule). Names here can never be user definitions.
pub(crate) const CORE_RESERVED: &[&str] = &[
    "log", "assert", "type",
    "abs", "sqrt", "floor", "ceil", "round", "min", "max",
    "split", "trim", "replace", "upper", "lower", "join", "starts", "ends",
    "repeat", "fmt",
    "int", "float", "str",
    "len", "contains", "slice", "push", "pop", "sort", "reverse", "concat",
    "range", "map",
    "keys", "values", "has", "set", "del",
    "args", "input",
    "read", "write",
    "exists", "mkdir", "ls", "rm", "mv", "cp",
    "rand", "time", "sleep", "env", "exit",
    "json", "encode",
    "error",
    "shell",
    "hash",
    "find", "matches", "sub", "any", "all",
    "later", "running",
    "test",
    "now", "moment", "calendar", "parse_time", "captured", "chart",
    "cpu", "gpu", "nodes",
    "grad",
];

/// Reserved only in files that import web (use web / use web.*)
pub(crate) const WEB_RESERVED: &[&str] = &[
    "route", "serve", "respond", "redirect", "html", "text",
    "sqlite", "sign", "verify",
    "app", "click", "elements", "change", "box", "fits", "style",
    "cors", "sessions", "templates", "cache", "limit",
    "uploads", "sse", "ws", "connect", "send", "recv", "close",
    "page", "static_dir", "smtp", "msgpack", "arrow", "bytes", "stream",
];

/// Web builtins the compiler implements but deliberately does NOT reserve
/// (`render` is free for views; the response encoders are rarely-collided
/// names). Listed so `crow test --docs` can hold the spec to them.
pub(crate) const WEB_EXTRA_FUNCTIONS: &[&str] = &["msgpack", "arrow", "bytes", "stream", "render"];

/// Reserved only in files that import ui
pub(crate) const UI_RESERVED: &[&str] = &["fetch", "poll", "field", "slider", "checkbox", "toggle", "radio", "tabs", "progress", "icon", "today"];

const BUILTINS: &[&str] = &[
    "log", "assert", "type",
    // Math
    "abs", "sqrt", "floor", "ceil", "round", "min", "max",
    // String
    "split", "trim", "replace", "upper", "lower", "join", "starts", "ends",
    "repeat", "pad", "fmt",
    // Conversion
    "int", "float", "str",
    // Collections
    "len", "contains", "slice", "push", "pop", "sort", "reverse", "append", "concat",
    "range", "map",
    // Map
    "keys", "values", "has", "set", "del",
    // CLI / IO
    "args", "input",
    // File I/O
    "read", "write",
    // File system
    "exists", "mkdir", "ls", "rm", "mv", "cp",
    // System
    "rand", "time", "sleep", "env", "exit",
    // JSON
    "json", "encode",
    // Error
    "error",
    // Shell
    "shell",
    // Hashing
    "hash",
    // Regex
    "find", "matches", "sub", "any", "all",
    "later", "running",
    "test",
    // Date/time
    "now", "moment", "calendar", "parse_time", "captured", "chart",
    // Resource
    "cpu", "gpu", "nodes",
    // Autograd
    "grad",
    // Web
    "route", "serve", "respond", "redirect", "html", "text",
    "msgpack", "arrow", "bytes", "stream",
    "sqlite", "exec", "query", "sign", "verify",
    // Web P4 modules
    "cors", "sessions", "templates", "render", "cache", "limit",
    "uploads", "sse", "ws", "connect", "send", "recv", "close",
    "page", "static_dir", "smtp",
    // UI reactive builtins
    "fetch", "poll", "field", "slider", "checkbox", "toggle", "radio", "tabs", "select", "progress", "icon", "today",
];

pub struct Validator<'a> {
    ctx: &'a TypeContext,
    errors: Vec<String>,
    /// Variables known to be defined at the current point
    defined_vars: BTreeSet<String>,
    /// Top-level (module) variable names — invisible inside functions
    /// (spec Design Note "No Ambient Mutability")
    module_vars: BTreeSet<String>,
    /// Nesting depth of function bodies currently being validated
    fn_depth: usize,
    /// `use ui` programs: top-level assignments are reactive state, which the
    /// ui transpiler rewrites into state.get/state.set inside render/handler
    /// functions — the documented exception to function-scope isolation.
    ui_mode: bool,
    /// Inside a `test` block — harness builtins (app, click) are callable.
    in_test_block: bool,
    /// Validating inside a `view` body — the element vocabulary is active
    in_view: bool,
    /// Bare imports: module path → line of the `use` (reservation source)
    bare_import_lines: std::collections::BTreeMap<String, usize>,
    /// Aliases and module prefixes: name → line of the `use`
    prefix_lines: std::collections::BTreeMap<String, usize>,
    current_line: usize,
}

impl<'a> Validator<'a> {
    pub fn new(ctx: &'a TypeContext) -> Self {
        Validator {
            ctx,
            errors: Vec::new(),
            defined_vars: BTreeSet::new(),
            module_vars: BTreeSet::new(),
            fn_depth: 0,
            ui_mode: false,
            in_test_block: false,
            in_view: false,
            bare_import_lines: std::collections::BTreeMap::new(),
            prefix_lines: std::collections::BTreeMap::new(),
            current_line: 0,
        }
    }

    /// Run validation on a program. Returns Ok(()) or Err with all error messages.
    pub fn validate(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        // Pre-populate with function and struct names (these are always valid identifiers)
        // Do NOT pre-populate from ctx.vars — the type inferrer optimistically adds
        // unknown identifiers to vars (e.g. foo[1,2] -> Fn type), which would hide
        // "undefined variable/function" errors.
        for name in self.ctx.functions.keys() {
            self.defined_vars.insert(name.clone());
        }
        for name in self.ctx.structs.keys() {
            self.defined_vars.insert(name.clone());
        }
        // Pre-scan top-level assignments to populate defined_vars
        // (the validator will also add them as it walks, but we need forward refs).
        // These are module variables — visible at top level, invisible in functions.
        // Prescan A: imports first — reservation is file-scoped, so an
        // assignment above the `use` line is still checked against it
        let mut prescan_line = 0usize;
        for stmt in stmts {
            match stmt {
                Stmt::Line(n) => prescan_line = *n,
                Stmt::Use { path, alias } => {
                    if path == "ui" {
                        self.ui_mode = true;
                    }
                    match alias {
                        Some(a) => {
                            self.prefix_lines.insert(a.clone(), prescan_line);
                        }
                        None => {
                            self.bare_import_lines.insert(path.clone(), prescan_line);
                            // The module's prefix name is also taken
                            let prefix = path
                                .rsplit(['.', '/'])
                                .next()
                                .unwrap_or(path)
                                .to_string();
                            self.prefix_lines.insert(prefix, prescan_line);
                        }
                    }
                }
                _ => {}
            }
        }

        // Prescan B: top-level assignments (module variables) — reservation
        // checked here since the main walk sees them as already-defined
        let mut prescan_line = 0usize;
        for stmt in stmts {
            match stmt {
                Stmt::Line(n) => prescan_line = *n,
                Stmt::Assign { name, .. } => {
                    if !self.defined_vars.contains(name) {
                        self.current_line = prescan_line;
                        self.check_reserved(name, "variable");
                    }
                    self.defined_vars.insert(name.clone());
                    self.module_vars.insert(name.clone());
                }
                Stmt::MultiAssign { names, .. } => {
                    for n in names {
                        if !self.defined_vars.contains(n) {
                            self.current_line = prescan_line;
                            self.check_reserved(n, "variable");
                        }
                        self.defined_vars.insert(n.clone());
                        self.module_vars.insert(n.clone());
                    }
                }
                _ => {}
            }
        }
        self.current_line = 0;

        for stmt in stmts {
            self.validate_stmt(stmt);
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.join("\n"))
        }
    }

    fn validate_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Line(n) => { self.current_line = *n; }
            // Test blocks: params are generated inputs; the body sees what a
            // caller sees (fns, structs, builtins) plus its params — the
            // verification boundary mirrors function-scope isolation.
            Stmt::TestBlock { params, body, .. } => {
                let saved = self.defined_vars.clone();
                let saved_in_test = self.in_test_block;
                self.in_test_block = true;
                for mv in &self.module_vars.clone() {
                    self.defined_vars.remove(mv);
                }
                for p in params {
                    self.defined_vars.insert(p.clone());
                }
                for s in body {
                    self.validate_stmt(s);
                }
                self.defined_vars = saved;
                self.in_test_block = saved_in_test;
            }
            // A derivation reads facts and sources and returns a value: it
            // is a computation, so it writes no fact, and it is read by
            // name, so nothing assigns it.
            Stmt::Derive { name, body, .. } => {
                self.check_reserved(name, "derivation");
                if !self.ui_mode {
                    self.error(format!("derive {} needs reactive state — derivations live in a `use ui` program with a page", name));
                }
                self.defined_vars.insert(name.clone());
                let saved = self.defined_vars.clone();
                fn first_fact_write(stmts: &[Stmt], facts: &std::collections::BTreeSet<String>) -> Option<String> {
                    for s in stmts {
                        match s {
                            Stmt::Assign { name, .. } if facts.contains(name) => return Some(name.clone()),
                            Stmt::MultiAssign { names, .. } => { if let Some(n) = names.iter().find(|n| facts.contains(*n)) { return Some(n.clone()); } }
                            Stmt::If { then_body, else_body, .. } => {
                                if let Some(n) = first_fact_write(then_body, facts) { return Some(n); }
                                if let Some(eb) = else_body { if let Some(n) = first_fact_write(eb, facts) { return Some(n); } }
                            }
                            Stmt::For { body, .. } | Stmt::While { body, .. } => { if let Some(n) = first_fact_write(body, facts) { return Some(n); } }
                            _ => {}
                        }
                    }
                    None
                }
                if let Some(v) = first_fact_write(body, &self.module_vars) {
                    self.error(format!("derive {} writes '{}' — a derivation computes a value; only handlers write facts", name, v));
                }
                self.fn_depth += 1;
                for s in body { self.validate_stmt(s); }
                self.fn_depth -= 1;
                self.defined_vars = saved;
                self.defined_vars.insert(name.clone());
            }
            Stmt::Assign { name, value } => {
                if self.ctx.derives.contains(name) {
                    self.error(format!("'{}' is derived — it follows what it reads; change those facts instead of assigning it", name));
                }
                if !self.defined_vars.contains(name) {
                    self.check_reserved(name, "variable");
                }
                self.validate_expr(value);
                self.defined_vars.insert(name.clone());
                // Check type-changing reassignment
                let new_ty = self.ctx.infer_expr(value);
                if let Some(existing_ty) = self.ctx.vars.get(name) {
                    // Assignment into a typed variable is a consumption
                    // point: result[T] unwraps into T (the transpiler
                    // aborts the fn on error) — not a type change.
                    let unwraps = matches!(&new_ty, CrowType::Result(inner)
                        if existing_ty.compatible(inner)
                            || (matches!(existing_ty, CrowType::Float)
                                && matches!(inner.as_ref(), CrowType::Int)))
                        || matches!(&existing_ty, CrowType::Result(inner)
                            if inner.compatible(&new_ty));
                    if !unwraps
                        && !existing_ty.compatible(&new_ty)
                        && !matches!(existing_ty, CrowType::Unknown)
                        && !matches!(new_ty, CrowType::Unknown)
                    {
                        self.error(format!(
                            "cannot reassign '{}' from {} to {} — variable type cannot change",
                            name, existing_ty.crow_name(), new_ty.crow_name()
                        ));
                    }
                }
            }
            Stmt::MultiAssign { names, values } => {
                for v in values { self.validate_expr(v); }
                for n in names {
                    if !self.defined_vars.contains(n) {
                        self.check_reserved(n, "variable");
                    }
                    self.defined_vars.insert(n.clone());
                }
            }
            Stmt::FnDecl { name, params, body, is_view, .. } => {
                self.check_reserved(name, if *is_view { "view" } else { "function" });
                for p in params {
                    self.check_reserved(p, "parameter");
                }
                self.defined_vars.insert(name.clone());
                let saved = self.defined_vars.clone();
                // Function scope isolation: module variables are not visible
                // inside functions — only params, locals, functions, structs,
                // and builtins. Exception: views and event handlers see
                // reactive state (`use ui` — the sanctioned channel).
                let reactive_exempt = self.ui_mode
                    && (*is_view
                        || self.ctx.views.contains(name)
                        || self.ctx.view_handlers.contains(name));
                if !reactive_exempt {
                    for mv in &self.module_vars {
                        self.defined_vars.remove(mv);
                    }
                    for d in &self.ctx.derives {
                        self.defined_vars.remove(d);
                    }
                }
                // Inside a view, the element vocabulary is defined
                let saved_in_view = self.in_view;
                if *is_view {
                    self.in_view = true;
                    for &el in crate::lower::UI_ELEMENTS {
                        self.defined_vars.insert(el.to_string());
                    }
                }
                for p in params { self.defined_vars.insert(p.clone()); }
                self.fn_depth += 1;
                for s in body { self.validate_stmt(s); }
                self.fn_depth -= 1;
                self.in_view = saved_in_view;
                self.defined_vars = saved;
                // Re-add the function name itself
                self.defined_vars.insert(name.clone());
            }
            Stmt::Return(Some(e)) => self.validate_expr(e),
            Stmt::Return(None) => {}
            Stmt::ExprStmt(e) => self.validate_expr(e),
            Stmt::If { condition, then_body, else_body } => {
                self.validate_expr(condition);
                for s in then_body { self.validate_stmt(s); }
                if let Some(eb) = else_body {
                    for s in eb { self.validate_stmt(s); }
                }
            }
            Stmt::For { var, iter, body } => {
                self.check_reserved(var, "loop variable");
                self.validate_expr(iter);
                let saved = self.defined_vars.clone();
                self.defined_vars.insert(var.clone());
                for s in body { self.validate_stmt(s); }
                self.defined_vars = saved;
                self.defined_vars.insert(var.clone());
            }
            Stmt::While { condition, body } => {
                self.validate_expr(condition);
                for s in body { self.validate_stmt(s); }
            }
            Stmt::StructDecl { name, fields } => {
                self.check_reserved(name, "struct");
                // The error-metadata names (.ok/.reason/.kind/.trace) are a
                // fixed vocabulary on EVERY value — a struct field with one of
                // these names is unreachable through dot access
                for f in fields {
                    if matches!(f.as_str(), "ok" | "reason" | "kind" | "trace") {
                        self.error(format!(
                            "struct '{}' field '{}' collides with error-value metadata (.ok/.reason/.kind/.trace are reserved on every value) — rename it",
                            name, f
                        ));
                    }
                }
                self.defined_vars.insert(name.clone());
            }
            Stmt::FieldAssign { object, value, .. } => {
                self.validate_expr(object);
                self.validate_expr(value);
            }
            Stmt::IndexAssign { object, index, value } => {
                self.validate_expr(object);
                self.validate_expr(index);
                self.validate_expr(value);
            }
            Stmt::Use { path, alias: _ } => {
                // Built-in modules: register all their function and constant names
                // so validate_call doesn't flag them as undefined.
                // The registry was already populated during type inference (Pass 1).
                if path == "math" || path == "math.stats" || path == "math.nn" || path == "io" || path == "frame" || path == "doc" || path == "chart" {
                    if path == "math" || path.starts_with("math.") {
                        self.defined_vars.insert("math".to_string());
                    }
                    for name in self.ctx.registry.fn_names() {
                        self.defined_vars.insert(name.to_string());
                    }
                    for name in self.ctx.registry.const_names() {
                        self.defined_vars.insert(name.to_string());
                    }
                }
                // Web modules: register 'web' as a known variable (for web.new[] etc.)
                if path == "web" || path.starts_with("web.") {
                    self.defined_vars.insert("web".to_string());
                }
                // `use ui` reserves NOTHING at file level — the element
                // vocabulary lives inside `view` blocks only
                if path == "ui" {
                    self.defined_vars.insert("ui".to_string());
                }
                // Other modules handled by expand_module_imports
            }
            Stmt::Resource { .. } => {}
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn validate_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_) => {}
            // Lowering resolves named args before validation; validate the value
            Expr::Named { value, .. } => self.validate_expr(value),
            Expr::Ident(name) => {
                if !self.defined_vars.contains(name)
                    && !BUILTINS.contains(&name.as_str())
                {
                    if self.fn_depth > 0 && self.module_vars.contains(name) {
                        self.error(format!(
                            "module variables are not visible inside functions — pass '{}' as a parameter",
                            name
                        ));
                    } else {
                        self.error(format!("undefined variable '{}'", name));
                    }
                }
            }
            Expr::BinOp { left, op, right } => {
                self.validate_expr(left);
                self.validate_expr(right);
                self.check_binop_types(left, op, right);
            }
            Expr::UnaryOp { expr: e, .. } => {
                self.validate_expr(e);
            }
            Expr::Call { callee, args } => {
                for a in args { self.validate_expr(a); }
                match callee.as_ref() {
                    Expr::Ident(name) => {
                        self.validate_call(name, args);
                    }
                    Expr::FieldAccess { object, field } => {
                        // Module call: module.func[args]
                        if let Expr::Ident(mod_name) = object.as_ref() {
                            let prefixed = format!("{}_{}", mod_name, field);
                            let real_mod = self
                                .ctx
                                .module_aliases
                                .get(mod_name.as_str())
                                .cloned()
                                .unwrap_or_else(|| mod_name.to_string());
                            let registry_module = matches!(
                                real_mod.as_str(),
                                "math" | "io" | "frame" | "db" | "doc" | "chart" | "math.stats" | "math.nn"
                            ) && self.ctx.registry.has_function(field);
                            if !registry_module
                                && !self.ctx.functions.contains_key(&prefixed)
                                && !self.ctx.structs.contains_key(&prefixed)
                                && !self.defined_vars.contains(mod_name)
                            {
                                self.error(format!("undefined module or variable '{}'", mod_name));
                            }
                        } else {
                            self.validate_expr(object);
                        }
                    }
                    other => { self.validate_expr(other); }
                }
            }
            Expr::FieldAccess { object, field } => {
                // Module variable/function reference (mymath.pi): the import
                // expander registered it under the prefixed name
                if let Expr::Ident(mod_name) = object.as_ref() {
                    let prefixed = format!("{}_{}", mod_name, field);
                    if !self.defined_vars.contains(mod_name)
                        && (self.defined_vars.contains(&prefixed)
                            || self.ctx.vars.contains_key(&prefixed)
                            || self.ctx.functions.contains_key(&prefixed))
                    {
                        return;
                    }
                }
                self.validate_expr(object);
                // Check field access validity
                let obj_type = self.ctx.infer_expr(object);
                match field.as_str() {
                    "ok" | "reason" | "kind" | "trace" => {} // error-value fields, always valid
                    _ => {
                        if let CrowType::Struct(name) = &obj_type {
                            if let Some(sig) = self.ctx.structs.get(name) {
                                if !sig.fields.iter().any(|(f, _)| f == field) {
                                    self.error(format!(
                                        "struct '{}' has no field '{}'",
                                        name, field
                                    ));
                                }
                            }
                        }
                        // Module access: module.var — don't flag these
                        // since the type context handles them via prefixed names
                    }
                }
            }
            Expr::Array(items) => {
                for item in items { self.validate_expr(item); }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.validate_expr(k);
                    self.validate_expr(v);
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.validate_expr(iter);
                let saved = self.defined_vars.clone();
                self.defined_vars.insert(var.clone());
                for s in body { self.validate_stmt(s); }
                self.defined_vars = saved;
            }
        }
    }

    /// Reservation: a user definition (variable, fn, struct, param, loop var)
    /// may not take a name the language or an imported module owns.
    /// (Spec Design Note "Names Are Never Ambiguous".)
    fn check_reserved(&mut self, name: &str, what: &str) {
        if CORE_RESERVED.contains(&name) {
            self.error(format!(
                "'{}' is a core builtin — pick another name for this {}",
                name, what
            ));
            return;
        }
        // Bare stdlib imports reserve their exports
        if let Some(module) = self.ctx.registry.module_name_of(name) {
            if self.ctx.bare_modules.contains(module) {
                let line = self.bare_import_lines.get(module).copied().unwrap_or(0);
                self.error(format!(
                    "'{}' is provided by {} (imported on line {}) — rename the {}, or import qualified: use {} as m",
                    name, module, line, what, module
                ));
                return;
            }
        }
        // Constants (pi, e, inf, nan) reserve under a bare math import
        if self.ctx.registry.has_constant(name) && self.ctx.bare_modules.contains("math") {
            let line = self.bare_import_lines.get("math").copied().unwrap_or(0);
            self.error(format!(
                "'{}' is a math constant (imported on line {}) — rename the {}, or import qualified: use math as m",
                name, line, what
            ));
            return;
        }
        // Web/ui builtin names reserve under their imports
        let web_bare = self
            .bare_import_lines
            .keys()
            .any(|p| p == "web" || p.starts_with("web."));
        if web_bare && WEB_RESERVED.contains(&name) {
            self.error(format!(
                "'{}' is a web builtin (use web) — rename the {}",
                name, what
            ));
            return;
        }
        if self.ui_mode && UI_RESERVED.contains(&name) {
            self.error(format!(
                "'{}' is a ui builtin (use ui) — rename the {}",
                name, what
            ));
            return;
        }
        // Module prefixes and aliases are taken
        if let Some(line) = self.prefix_lines.get(name) {
            self.error(format!(
                "'{}' is a module name (use on line {}) — pick another name for this {}",
                name, line, what
            ));
        }
    }

    fn validate_call(&mut self, name: &str, args: &[Expr]) {
        // Test-harness builtins exist only inside test blocks (which never
        // reach a build) — accept them there, reject them anywhere else.
        if matches!(name, "app" | "click" | "elements" | "change" | "box" | "fits" | "style") {
            if self.in_test_block {
                for a in args {
                    self.validate_expr(a);
                }
                return;
            }
            self.error(format!(
                "'{}' is a test-harness builtin — it only works inside a test block",
                name
            ));
            return;
        }
        let argc = args.len();

        // Maps are dictionaries: set[] must not change the map's value type
        // (spec Design Note "Maps Are Dictionaries, Not Records"). Checked here
        // so compiled mode reports it in Crow terms instead of a Rust error.
        if name == "set" && argc == 3 {
            let map_ty = self.ctx.infer_expr(&args[0]);
            let map_ty = if let CrowType::Result(i) = map_ty { *i } else { map_ty };
            if let CrowType::Map(inner) = &map_ty {
                let val_ty = self.ctx.infer_expr(&args[2]);
                let val_ty = if let CrowType::Result(i) = val_ty { *i } else { val_ty };
                if !matches!(inner.as_ref(), CrowType::Unknown)
                    && !matches!(val_ty, CrowType::Unknown | CrowType::Void)
                    && !inner.compatible(&val_ty)
                {
                    self.error(format!(
                        "map values must all be the same type — this map holds {} values, cannot add {} (use a struct for mixed named data)",
                        inner.crow_name(), val_ty.crow_name()
                    ));
                }
            }
        }

        // Layout attributes are part of the vocabulary: literal, and valid per
        // the kernel table both renderers use (crow_runtime::layout).
        if crate::lower::UI_ELEMENTS.contains(&name) && self.in_view {
            if let Some(Expr::Map(pairs)) = args.first() {
                let mut literal: Vec<(String, String)> = Vec::new();
                for (k, v) in pairs {
                    let key = match k { Expr::Ident(s) | Expr::Str(s) => s.clone(), _ => continue };
                    if !crow_runtime::layout::LAYOUT_ATTRS.contains(&key.as_str()) { continue; }
                    // state-driven by design: open follows a bool, active a
                    // path, tip a text — any expression, checked at render
                    if matches!(key.as_str(), "open" | "active" | "tip") && !matches!(v, Expr::Str(_) | Expr::Int(_) | Expr::Bool(_)) { continue; }
                    match v {
                        Expr::Str(s) => literal.push((key, s.clone())),
                        Expr::Int(n) => literal.push((key, n.to_string())),
                        Expr::Bool(b) => literal.push((key, b.to_string())),
                        _ => self.error(format!("{}: '{}' is a layout attribute and must be a literal — a scale step like 'm', a size like '240px', or true", name, key)),
                    }
                }
                for e in crow_runtime::layout::render(name, &literal).errors {
                    self.error(format!("{}: {}", name, e));
                }
            }
            // Inside a view the element wins over a module function of the
            // same bare name (stack vs frame.stack / math.stack): the prefix
            // is how a view reaches the module version.
            return;
        }

        // ui elements exist only inside view blocks
        if crate::lower::UI_ELEMENTS.contains(&name) && self.ui_mode && !self.in_view {
            self.error(format!(
                "ui element '{}' can only be used inside a view block — declare the builder with 'view name[params]' instead of 'fn'",
                name
            ));
            return;
        }

        // Module function? Accept only when its module was imported bare —
        // an aliased import (use math as m) is qualified-only. And a name
        // several loaded modules export is ambiguous bare (spec: Conflict
        // Resolution) — the prefix picks.
        if self.ctx.registry.has_function(name) {
            let providers = self.ctx.registry.providers_of(name);
            if providers.len() > 1 {
                let forms: Vec<String> = providers.iter().map(|m| format!("{}.{}[...]", m, name)).collect();
                self.error(format!(
                    "'{}' is ambiguous — provided by {}; use {}",
                    name,
                    providers.join(" and "),
                    forms.join(" or ")
                ));
                return;
            }
            if let Some(module) = self.ctx.registry.module_name_of(name) {
                if self.ctx.bare_modules.contains(module) {
                    return;
                }
                // Imported under an alias?
                if let Some((alias, _)) = self
                    .ctx
                    .module_aliases
                    .iter()
                    .find(|(_, p)| p.as_str() == module)
                {
                    self.error(format!(
                        "'{}' comes from {} which was imported as '{}' — call {}.{}[...]",
                        name, module, alias, alias, name
                    ));
                    return;
                }
            }
            return;
        }

        if name == "pad" {
            self.error("pad was removed — use fmt[s, width = n]".to_string());
            return;
        }
        if name == "append" {
            self.error("append was removed — use concat[a, b] (one merge operation)".to_string());
            return;
        }

        // Known builtin? Check arg count
        if BUILTINS.contains(&name) {
            self.check_builtin_args(name, argc);
            return;
        }

        // User-defined function?
        if let Some(sig) = self.ctx.functions.get(name) {
            let expected = sig.params.len();
            if argc != expected {
                self.error(format!(
                    "'{}' expects {} argument{}, got {}",
                    name, expected, if expected == 1 { "" } else { "s" }, argc
                ));
            }
            return;
        }

        // Struct constructor?
        if let Some(sig) = self.ctx.structs.get(name) {
            let expected = sig.fields.len();
            if argc != expected {
                self.error(format!(
                    "struct '{}' has {} field{}, got {} argument{}",
                    name, expected, if expected == 1 { "" } else { "s" },
                    argc, if argc == 1 { "" } else { "s" }
                ));
            }
            return;
        }

        // Variable that holds a function, list, or map?
        // Only trust ctx.vars if the variable was actually defined (assigned) in the source.
        // The type inferrer optimistically adds unknown identifiers to vars.
        if self.defined_vars.contains(name) {
            if let Some(var_type) = self.ctx.vars.get(name) {
                match var_type {
                    CrowType::Fn(params, _) => {
                        if argc != params.len() {
                            self.error(format!(
                                "'{}' expects {} argument{}, got {}",
                                name, params.len(),
                                if params.len() == 1 { "" } else { "s" }, argc
                            ));
                        }
                    }
                    CrowType::Array(_) | CrowType::Map(_) | CrowType::Json
                    | CrowType::Result(_) => {
                        // indexing — valid
                    }
                    _ => {
                        // Unknown variable type used as function — might be dynamic
                    }
                }
            }
            return;
        }

        // Truly unknown
        self.error(format!("undefined function '{}'", name));
    }

    fn check_builtin_args(&mut self, name: &str, argc: usize) {
        let valid = match name {
            "log" | "assert" => argc >= 1,
            "type" => argc == 1,
            "abs" | "sqrt" | "floor" | "ceil" | "round" => argc == 1,
            "min" | "max" => argc == 2,
            "split" => argc == 2,
            "trim" | "upper" | "lower" => argc == 1,
            "replace" => argc == 3,
            "join" => argc == 2,
            "starts" | "ends" => argc == 2,
            "int" | "float" | "str" => argc == 1,
            "len" => argc == 1,
            "contains" => argc == 2,
            "slice" => argc == 3,
            "repeat" => argc == 2,
            "fmt" => argc == 9,
            "moment" => argc == 7,
            "calendar" | "parse_time" => argc == 2,
            "now" => argc == 0,
            "captured" => argc == 1,
            "chart" => argc == 12,
            "push" | "append" => argc == 2,
            "pop" | "sort" | "reverse" => argc == 1,
            "range" => argc >= 1 && argc <= 3,
            "map" => argc == 0,
            "keys" | "values" => argc == 1,
            "has" | "del" => argc == 2,
            "set" => argc == 3,
            "args" => argc == 0,
            "input" => argc <= 1,
            "read" => argc == 1,
            "write" => argc == 2,
            "exists" => argc == 1,
            "mkdir" => argc == 1,
            "ls" => argc == 1,
            "rm" => argc == 1,
            "mv" => argc == 2,
            "cp" => argc == 2,
            "rand" => argc <= 2,
            "time" => argc == 0,
            "sleep" => argc == 1,
            "env" => argc == 1 || argc == 2,
            "exit" => argc <= 1,
            "json" | "encode" => argc == 1,
            "error" => argc >= 1 && argc <= 2,
            "cpu" => argc == 1,
            "gpu" => argc <= 1,
            "nodes" => argc == 1,
            "grad" => argc == 1,
            // Shell / Hash / Regex / Date
            "shell" => argc == 1,
            "hash" => argc == 2,
            "find" | "matches" | "any" | "all" => argc == 2,
            "sub" => argc == 3,
            "date" => argc == 3,
            "elapsed" => argc == 2,
            // Web
            "route" => (2..=4).contains(&argc), // route[path, handler] / +method / legacy app-first
            "serve" => argc == 1 || argc == 7,   // serve[port] (pre-lower) or serve[port, refresh, budget, theme, density, accent, wide]
            "respond" => argc == 2,
            "redirect" | "html" | "text" => argc == 1,
            "msgpack" | "arrow" | "bytes" | "stream" => argc == 1,
            "sqlite" => argc == 1,
            "exec" | "query" => argc >= 2,
            "sign" | "verify" => argc == 2,
            // Web P4 modules
            "cors" => argc == 1 || argc == 2,   // cors[app] or cors[app, origins_list]
            "sessions" => argc == 0,              // sessions[]
            "templates" => argc == 1,             // templates[glob]
            "render" => argc == 3,                // render[t, name, data]
            "cache" => argc == 1 || argc == 2,    // cache[max] or cache[max, ttl]
            "limit" => argc == 2,                 // limit[max, window]
            "uploads" => argc == 1,               // uploads[req]
            "sse" => argc == 3,                   // sse[app, path, handler]
            "ws" => argc == 3,                    // ws[app, path, handler]
            "connect" => argc == 1,               // connect[url]
            "send" => argc == 2,                  // send[sock, msg] or send[sender, data]
            "recv" => argc == 1,                  // recv[sock]
            "close" => argc == 1,                 // close[sock]
            "page" => argc == 2 || argc == 3,      // page[path, render] or page[path, render, handle]
            "static_dir" => argc == 3,            // static_dir[app, url_path, dir_path]
            "smtp" => argc == 4,                  // smtp[host, port, user, pass]
            // UI reactive builtins
            "fetch" => argc == 1,                 // fetch[url]
            "poll" => argc == 2,                  // poll[url, seconds]
            "field" => argc == 4,                 // field[label, var, type, lines] (lowered)
            "slider" => argc == 5,                // slider[label, var, min, max, step] (lowered)
            "checkbox" | "toggle" => argc == 2,   // checkbox[label, var]
            "radio" | "tabs" => argc == 2,        // radio[options, var]
            "progress" => argc == 2,              // progress[value, max] (lowered)
            "icon" => argc == 3,                  // icon[name, size, color] (lowered)
            "today" => argc == 0,                 // today[]
            _ => true,
        };
        if !valid {
            self.error(format!("wrong number of arguments for '{}'", name));
        }
    }

    fn check_binop_types(&mut self, left: &Expr, op: &BinOp, right: &Expr) {
        let lt = self.ctx.infer_expr(left);
        let rt = self.ctx.infer_expr(right);
        // Unwrap CrowResult
        let lt = if let CrowType::Result(inner) = lt { *inner } else { lt };
        let rt = if let CrowType::Result(inner) = rt { *inner } else { rt };

        // Skip if either side is unknown
        if matches!(lt, CrowType::Unknown) || matches!(rt, CrowType::Unknown) {
            return;
        }

        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                // String + is not allowed — use interpolation instead
                if matches!(op, BinOp::Add) && (matches!(lt, CrowType::Str) || matches!(rt, CrowType::Str)) {
                    self.error("cannot use '+' with strings — interpolate to combine ('`a` `b`'), or collect parts and join[parts, sep] to build".to_string());
                    return;
                }
                // Numeric ops: both must be numeric (including NumArray, Sparse, FrameCol, and legacy Array(Str))
                let is_num_like = |t: &CrowType| matches!(t, CrowType::Int | CrowType::Float | CrowType::NumArray(_) | CrowType::Sparse | CrowType::FrameCol)
                    || matches!(t, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str));
                let lt_num = is_num_like(&lt);
                let rt_num = is_num_like(&rt);
                if !lt_num || !rt_num {
                    let op_str = match op {
                        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                        BinOp::Div => "/", BinOp::Mod => "%", _ => "?"
                    };
                    self.error(format!(
                        "cannot use '{}' with {} and {}",
                        op_str, lt.crow_name(), rt.crow_name()
                    ));
                }
            }
            // Comparison and logical ops — skip validation for now
            _ => {}
        }
    }

    fn error(&mut self, msg: String) {
        self.errors.push(format!("line {}: {}", self.current_line, msg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::type_infer::TypeContext;

    fn validate_source(source: &str) -> Result<(), String> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("lexer failed");
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().expect("parser failed");
        // Same pipeline as the real build: lower named args before inference.
        // Lowering errors (unknown/missing/duplicate arguments) surface as
        // validation results, matching how the CLI reports them.
        let stmts = crate::lower::lower_program(&stmts)?;
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let mut validator = Validator::new(&ctx);
        validator.validate(&stmts)
    }

    #[test]
    fn test_valid_program() {
        let result = validate_source("x = 10\ny = x + 5\nlog[y]\n");
        assert!(result.is_ok(), "valid program should pass: {:?}", result);
    }

    #[test]
    fn test_undefined_variable() {
        let result = validate_source("log[x]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("undefined variable 'x'"));
    }

    #[test]
    fn test_undefined_function() {
        let result = validate_source("y = foo[1, 2]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("undefined function 'foo'"));
    }

    #[test]
    fn test_wrong_arg_count_builtin() {
        let result = validate_source("x = min[1]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("wrong number of arguments for 'min'"));
    }

    #[test]
    fn test_wrong_arg_count_user_fn() {
        let result = validate_source("fn add[a, b]\n  return a + b\ny = add[1]\n");
        assert!(result.is_err());
        // Lowering reports missing arguments before the validator's arity check
        assert!(result.unwrap_err().contains("missing argument 'b'"));
    }

    #[test]
    fn test_arithmetic_type_mismatch() {
        let result = validate_source("x = true - 5\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot use '-'"));
    }

    #[test]
    fn test_string_concat_rejected() {
        let result = validate_source("x = 'hello' + ' world'\n");
        assert!(result.is_err(), "string + should be rejected");
        assert!(result.unwrap_err().contains("join"), "should suggest interpolation and join");
    }

    #[test]
    fn test_struct_bad_field() {
        let result = validate_source("struct Point[x, y]\np = Point[1, 2]\nlog[p.z]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("has no field 'z'"));
    }

    #[test]
    fn test_struct_wrong_arg_count() {
        let result = validate_source("struct Point[x, y]\np = Point[1]\n");
        assert!(result.is_err());
        // Lowering reports missing arguments before the validator's arity check
        assert!(result.unwrap_err().contains("missing argument 'y'"));
    }

    #[test]
    fn test_for_loop_var_defined() {
        let result = validate_source("for i in range[5]\n  log[i]\n");
        assert!(result.is_ok(), "for loop var should be defined: {:?}", result);
    }

    #[test]
    fn test_grad_valid() {
        let result = validate_source("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(result.is_ok(), "grad[f] should be valid: {:?}", result);
    }

    #[test]
    fn test_grad_wrong_arg_count() {
        let result = validate_source("fn f[x]\n  x * x\nfn g[x]\n  x + 1\ndf = grad[f, g]\n");
        assert!(result.is_err(), "grad should require exactly 1 arg");
    }

    #[test]
    fn test_fn_module_var_read_rejected_with_hint() {
        let result = validate_source("base = 10\nfn f[x]\n    return x + base\nr = f[1]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("pass 'base' as a parameter"));
    }

    #[test]
    fn test_fn_local_reusing_module_name_ok() {
        let result = validate_source("count = 5\nfn f[n]\n    count = n * 2\n    return count\nr = f[3]\n");
        assert!(result.is_ok(), "local reusing module name should validate: {:?}", result);
    }

    #[test]
    fn test_map_hetero_set_rejected_at_compile_time() {
        // Maps are dictionaries: set[] changing value type is a validation error
        let result = validate_source("m = map[a = 1]\nm2 = set[m, 'b', 'x']\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("map values must all be the same type"));
    }

    #[test]
    fn test_frame_bare_column_name_rejected() {
        // Regression: bare-name column DSL removed — undefined idents in frame
        // calls are normal undefined-variable errors now
        let result = validate_source("use frame\ndf = load['d.csv']\ns = sort[df, age]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("undefined variable 'age'"));
    }
}
