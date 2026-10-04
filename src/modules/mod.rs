pub mod chart;
pub mod db;
pub mod doc;
pub mod frame;
pub mod io;
pub mod math;
pub mod nn;
pub mod stats;
pub mod sys;
pub mod web;

use std::collections::HashMap;
use std::sync::Arc;
use crate::value::CrowValue;
use crate::type_infer::CrowType;

/// The source of a standard module written in Crow (design rule 20), by
/// its import path. Each is inlined into the program that imports it, its
/// functions named by `crate::table::crow_fn_name` (`_math_nn_relu`).
pub fn crow_source(path: &str) -> Option<&'static str> {
    match path {
        "math.nn" => Some(nn::NN_SOURCE),
        "" => Some(CORE_SOURCE),
        "sys" => Some(SYS_SOURCE),
        "io" => Some(IO_SOURCE),
        "frame" => Some(FRAME_SOURCE),
        "math" => Some(MATH_SOURCE),
        "web.auth" => Some(AUTH_SOURCE),
        "web.ws" => Some(WS_SOURCE),
        "web.mail" => Some(MAIL_SOURCE),
        "web" => Some(WEB_SOURCE),
        _ => None,
    }
}

/// A token signed and read, written in Crow (`use web.auth`).
pub(crate) const AUTH_SOURCE: &str = include_str!("auth.co");

/// A value packed as MessagePack, written in Crow (`use web`).
pub(crate) const WEB_SOURCE: &str = include_str!("web.co");

/// The WebSocket client, written in Crow (`use web.ws`).
pub(crate) const WS_SOURCE: &str = include_str!("ws.co");

/// Mail over SMTP, written in Crow (`use web.mail`).
pub(crate) const MAIL_SOURCE: &str = include_str!("mail.co");

/// The protocol the HTTP client and the readers stand on, written in Crow
/// over the sys primitives: `sys.exchange`, `sys.fetch`, `sys.put`, the
/// formats; a module that imports `sys` inlines it.
pub(crate) const SYS_SOURCE: &str = include_str!("sys.co");
/// `io.lines`, `io.csv`: readers written in Crow.
pub(crate) const IO_SOURCE: &str = include_str!("io.co");
/// `frame.load`, `frame.save`: written in Crow.
pub(crate) const FRAME_SOURCE: &str = include_str!("frame.co");
/// `math.load`: written in Crow.
pub(crate) const MATH_SOURCE: &str = include_str!("math.co");

/// The core builtins written in Crow (`request`): no import brings them,
/// so the source is inlined into a program that calls one of them.
pub(crate) const CORE_SOURCE: &str = include_str!("core.co");

/// The modules written in Crow, with the prefix their functions carry (the
/// core's, imported by no `use`, carries `_core`).
const CROW_MODULES: &[(&str, &str)] = &[("math.nn", "_math_nn"), ("", "_core"), ("sys", "_sys"), ("io", "_io"), ("frame", "_frame"), ("math", "_math"), ("web.auth", "_web_auth"), ("web.ws", "_web_ws"), ("web.mail", "_web_mail"), ("web", "_web")];

thread_local! {
    /// What each inlined module imports (its own `use` lines), by its
    /// prefix: the registry loads them into the module's scope.
    static CROW_IMPORTS: std::cell::RefCell<Vec<(String, Vec<String>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A standard module by its import path, for a registry's scope.
pub fn module_by_path(path: &str) -> Option<Arc<dyn CrowModule>> {
    Some(match path {
        "math" => Arc::new(math::MathModule),
        "math.stats" => Arc::new(stats::StatsModule),
        "math.nn" => Arc::new(nn::NnModule),
        "io" => Arc::new(io::IoModule),
        "sys" => Arc::new(sys::SysModule),
        "frame" => Arc::new(frame::FrameModule),
        "doc" => Arc::new(doc::DocModule),
        "chart" => Arc::new(chart::ChartModule),
        "db" => Arc::new(db::DbModule),
        "web" => Arc::new(web::WebModule),
        "web.auth" => Arc::new(web::AuthModule),
        "web.ws" => Arc::new(web::WsModule),
        "web.mail" => Arc::new(web::MailModule),
        _ => return None,
    })
}

/// The standard modules written in Crow that a program carries, by the
/// functions inlined behind their prefixes, each with its own imports
/// (its source's `use` lines) — read from the program itself, since a
/// harness lowers several programs in turn (a part's tests, the app).
pub fn crow_modules_carried<'a>(fn_names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec<String>)> {
    let prefixes: std::collections::BTreeSet<String> = fn_names.filter_map(crow_scope_of).collect();
    prefixes
        .into_iter()
        .filter_map(|prefix| {
            let (path, _) = CROW_MODULES.iter().find(|(_, p)| *p == prefix)?;
            let imports: Vec<String> = crow_source(path)?
                .lines()
                .filter_map(|l| l.strip_prefix("use "))
                .flat_map(|rest| rest.split(',').map(|m| m.trim().split(' ').next().unwrap_or("").to_string()))
                .filter(|m| !m.is_empty())
                .collect();
            Some((prefix, imports))
        })
        .collect()
}

/// Every module a program carries, its imports loaded into its own scope
/// of a registry (a module's `use math.nn` loads math too, as a program's
/// does), given the program's function names.
pub fn load_crow_scopes<'a>(registry: &mut ModuleRegistry, fn_names: impl Iterator<Item = &'a str>) {
    for (prefix, imports) in crow_modules_carried(fn_names) {
        // (a module's own rows are in its scope: the protocol written in
        // sys.co calls sys's primitives)
        if let Some((own, _)) = CROW_MODULES.iter().find(|(_, p)| *p == prefix) {
            if let Some(m) = module_by_path(own) {
                registry.load_for(&prefix, m);
            }
        }
        for path in imports {
            if path.starts_with("math.") {
                if let Some(m) = module_by_path("math") {
                    registry.load_for(&prefix, m);
                }
            }
            if let Some(m) = module_by_path(&path) {
                registry.load_for(&prefix, m);
            }
        }
    }
}

/// The imports of every standard module the program inlined last, by
/// prefix (what the inliner recorded; the scopes are read from the
/// program itself, `crow_modules_carried`).
#[cfg(test)]
pub fn crow_imports() -> Vec<(String, Vec<String>)> {
    CROW_IMPORTS.with(|c| c.borrow().clone())
}

/// The scope a function's names are resolved in: a standard module's
/// prefix for one of its functions (inlined, or its traced copy), none for
/// the program's own.
pub fn crow_scope_of(name: &str) -> Option<String> {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    // (the longest prefix that fits: `_web_auth_sign` is web.auth's, not web's)
    CROW_MODULES
        .iter()
        .filter(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_')
        .max_by_key(|(_, prefix)| prefix.len())
        .map(|(_, p)| p.to_string())
}

/// Whether a function of the program is a standard module's, inlined: its
/// stops name the line it was called from, not a line of the module's.
pub fn is_crow_fn(name: &str) -> bool {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    CROW_MODULES.iter().any(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_')
}

/// The program with every standard module it imports that is written in
/// Crow inlined after the import: the module's functions the table's rows
/// name that the program reaches — a row called by a bare name or the
/// module's, or handed as a value, by the program or by a function kept
/// so far — and the helpers they reach, each renamed behind the module's
/// prefix. A module's own imports and the core are reached the same way
/// (io's readers are written over sys's protocol and the core's `read`),
/// to a fixpoint, so a program that uses math without `load` carries none
/// of the protocol. Inside the module a call of a helper is rewritten to
/// the helper's new name; a call of a row's function stays bare, so it is
/// the row's call wherever it is made — read as the row's parameters take
/// it, one signature. The module's tests, its own imports and its line
/// marks are left out (the import loads what the module needs; a stop
/// inside one of its functions names the line of the call in the program).
pub fn inline_crow_modules(stmts: &[crate::ast::Stmt]) -> Result<Vec<crate::ast::Stmt>, String> {
    use crate::ast::Stmt;
    use std::collections::{BTreeMap, BTreeSet};
    CROW_IMPORTS.with(|c| c.borrow_mut().clear());
    // the modules the program may reach — what it imports, each one's own
    // imports, and the core — parsed once each
    struct Module {
        path: String,
        prefix: &'static str,
        imports: Vec<String>,
        stmts: Vec<Stmt>,
    }
    let mut modules: Vec<Module> = Vec::new();
    let mut todo: Vec<String> = vec![String::new()];
    for s in stmts {
        if let Stmt::Use { path, .. } = s {
            todo.push(path.clone());
            // (`use math.nn` is math's too, as the registry reads it)
            if path.starts_with("math.") {
                todo.push("math".to_string());
            }
        }
    }
    // the modules whose rows the program itself may call: what it imports
    // (the core's rows need no import)
    let imported: BTreeSet<String> = todo.iter().cloned().collect();
    while let Some(path) = todo.pop() {
        if modules.iter().any(|m| m.path == path) {
            continue;
        }
        let Some(source) = crow_source(&path) else { continue };
        let prefix = CROW_MODULES.iter().find(|(m, _)| *m == path).map(|(_, p)| *p).unwrap_or("_");
        let mut lexer = crate::lexer::Lexer::new(source);
        let tokens = lexer.tokenize().map_err(|e| format!("a bug in Crow: the module {} does not lex: {}", path, e))?;
        let mut parser = crate::parser::Parser::new(tokens);
        let mod_stmts = parser.parse().map_err(|e| format!("a bug in Crow: the module {} does not parse: {}", path, e))?;
        let imports: Vec<String> = mod_stmts.iter().filter_map(|m| if let Stmt::Use { path, .. } = m { Some(path.clone()) } else { None }).collect();
        todo.extend(imports.iter().cloned());
        modules.push(Module { path, prefix, imports, stmts: mod_stmts });
    }
    // what the program names: a call by a bare name or a module's, a
    // function handed as a value
    let mut named: BTreeSet<String> = BTreeSet::new();
    stmts.iter().for_each(|s| called_in_stmt(s, &mut named));
    // each module's functions the program reaches: a row's, named by what
    // may call it — the program, where it imports the module, and the
    // functions kept so far of the modules that import it (the core's
    // rows from anywhere) — and what it reaches among the module's own; to
    // a fixpoint, since a module's function calls other modules' rows
    // (`read` from io's readers, `fetch` from `read`). A name a module
    // never imports reaches nothing of it: sys's `lines[0]` is its local,
    // not io's reader, and a feed's `put` is the program's parameter
    let mut kept: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    loop {
        let mut changed = false;
        for m in &modules {
            let mut called: BTreeSet<String> = BTreeSet::new();
            if m.path.is_empty() || imported.contains(&m.path) {
                called.extend(named.iter().cloned());
            }
            for k in &modules {
                if !(m.path.is_empty() || k.imports.contains(&m.path)) {
                    continue;
                }
                let Some(fns) = kept.get(&k.path) else { continue };
                for s in &k.stmts {
                    if let Stmt::FnDecl { name, body, .. } = s {
                        if fns.contains(name) {
                            body.iter().for_each(|st| called_in_stmt(st, &mut called));
                        }
                    }
                }
            }
            let bodies: BTreeMap<&str, &Vec<Stmt>> = m.stmts.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.as_str(), body)) } else { None }).collect();
            // the functions of every row of a name called (named by the
            // row's id: `read` and `read_asked` — the row a call runs is
            // decided later, by its arguments), and what they reach among
            // the module's own
            let mut reach: Vec<String> = crate::table::ROWS
                .iter()
                .filter(|r| r.module == m.path && r.is_crow() && called.contains(r.name))
                .map(|r| crate::table::crow_fn_base(r).to_string())
                .collect();
            let mut set: BTreeSet<String> = BTreeSet::new();
            while let Some(n) = reach.pop() {
                if !set.insert(n.clone()) {
                    continue;
                }
                if let Some(body) = bodies.get(n.as_str()) {
                    let mut inner = BTreeSet::new();
                    body.iter().for_each(|st| called_in_stmt(st, &mut inner));
                    reach.extend(inner.into_iter().filter(|c| bodies.contains_key(c.as_str())));
                }
            }
            if kept.get(&m.path) != Some(&set) {
                kept.insert(m.path.clone(), set);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut out: Vec<Stmt> = stmts.to_vec();
    for m in &modules {
        let Some(fns) = kept.get(&m.path).filter(|fns| !fns.is_empty()) else { continue };
        CROW_IMPORTS.with(|c| c.borrow_mut().push((m.prefix.to_string(), m.imports.clone())));
        let row_names: BTreeSet<&str> = crate::table::ROWS.iter().filter(|r| r.module == m.path && r.is_crow()).map(|r| r.name).collect();
        // a helper's references are rewritten, and a row function's whose
        // name is not the row's (`read_asked`); a call of a row's name is
        // the row's call wherever it is made
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        for s in &m.stmts {
            match s {
                Stmt::FnDecl { name, .. } if fns.contains(name) && !row_names.contains(name.as_str()) => {
                    names.insert(name.clone(), format!("{}_{}", m.prefix, name));
                }
                Stmt::StructDecl { name, .. } => {
                    names.insert(name.clone(), format!("{}_{}", m.prefix, name));
                }
                _ => {}
            }
        }
        for s in &m.stmts {
            match s {
                Stmt::FnDecl { name, params, defaults, body, is_view } if fns.contains(name) => {
                    let mut body = body.clone();
                    let shadowed: std::collections::HashSet<String> = params.iter().cloned().collect();
                    for st in &mut body {
                        crate::rewrite_refs_stmt(st, &names, &shadowed);
                    }
                    drop_lines(&mut body);
                    out.push(Stmt::FnDecl { name: format!("{}_{}", m.prefix, name), params: params.clone(), defaults: defaults.clone(), body, is_view: *is_view });
                }
                Stmt::StructDecl { name, fields } => {
                    out.push(Stmt::StructDecl { name: format!("{}_{}", m.prefix, name), fields: fields.clone() });
                }
                // the module's other functions (its tests' helpers, the
                // rows the program never reaches), its imports, its tests,
                // and its line marks
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The names a statement calls, by a bare name, through every nested
/// block and expression.
fn called_in_stmt(st: &crate::ast::Stmt, out: &mut std::collections::BTreeSet<String>) {
    use crate::ast::Stmt;
    match st {
        Stmt::Assign { value, .. } => called_in_expr(value, out),
        Stmt::MultiAssign { values, .. } => values.iter().for_each(|v| called_in_expr(v, out)),
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => called_in_expr(e, out),
        Stmt::If { condition, then_body, else_body } => {
            called_in_expr(condition, out);
            then_body.iter().for_each(|s| called_in_stmt(s, out));
            if let Some(e) = else_body {
                e.iter().for_each(|s| called_in_stmt(s, out));
            }
        }
        Stmt::For { iter, body, .. } => {
            called_in_expr(iter, out);
            body.iter().for_each(|s| called_in_stmt(s, out));
        }
        Stmt::While { condition, body } => {
            called_in_expr(condition, out);
            body.iter().for_each(|s| called_in_stmt(s, out));
        }
        Stmt::FieldAssign { object, value, .. } => {
            called_in_expr(object, out);
            called_in_expr(value, out);
        }
        Stmt::IndexAssign { object, index, value } => {
            called_in_expr(object, out);
            called_in_expr(index, out);
            called_in_expr(value, out);
        }
        Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } => body.iter().for_each(|s| called_in_stmt(s, out)),
        _ => {}
    }
}

fn called_in_expr(e: &crate::ast::Expr, out: &mut std::collections::BTreeSet<String>) {
    use crate::ast::Expr;
    match e {
        Expr::Call { callee, args } => {
            match callee.as_ref() {
                Expr::Ident(n) => {
                    out.insert(n.clone());
                }
                // a module's function called by the module's name
                // (`io.csv[…]`, `math.nn.relu[…]`)
                Expr::FieldAccess { object, field } => {
                    out.insert(field.clone());
                    called_in_expr(object, out);
                }
                other => called_in_expr(other, out),
            }
            args.iter().for_each(|a| called_in_expr(a, out));
        }
        // a function handed as a value (`stream[lines, 2]`, `f = relu`)
        Expr::Ident(n) => {
            out.insert(n.clone());
        }
        // a call inside an interpolation (`'`relu[x]`'`), read when the
        // string is lowered: every name between backticks counts
        Expr::Str(s) => {
            let mut inside = false;
            let mut word = String::new();
            for c in s.chars().chain(std::iter::once('`')) {
                if inside && (c.is_alphanumeric() || c == '_') {
                    word.push(c);
                    continue;
                }
                if !word.is_empty() && !word.starts_with(|d: char| d.is_ascii_digit()) {
                    out.insert(std::mem::take(&mut word));
                }
                word.clear();
                if c == '`' {
                    inside = !inside;
                }
            }
        }
        Expr::BinOp { left, right, .. } => {
            called_in_expr(left, out);
            called_in_expr(right, out);
        }
        Expr::UnaryOp { expr, .. } | Expr::Named { value: expr, .. } | Expr::FieldAccess { object: expr, .. } => called_in_expr(expr, out),
        Expr::Array(items) => items.iter().for_each(|i| called_in_expr(i, out)),
        Expr::Map(pairs) => pairs.iter().for_each(|(k, v)| {
            called_in_expr(k, out);
            called_in_expr(v, out);
        }),
        Expr::ForExpr { iter, body, .. } => {
            called_in_expr(iter, out);
            body.iter().for_each(|s| called_in_stmt(s, out));
        }
        _ => {}
    }
}

/// A block without its line marks, through every nested block.
fn drop_lines(body: &mut Vec<crate::ast::Stmt>) {
    use crate::ast::Stmt;
    body.retain(|s| !matches!(s, Stmt::Line(_)));
    for s in body.iter_mut() {
        match s {
            Stmt::If { then_body, else_body, .. } => {
                drop_lines(then_body);
                if let Some(e) = else_body {
                    drop_lines(e);
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } => drop_lines(body),
            _ => {}
        }
    }
}

/// Where a module's printed code stops the program, the line it stops on:
/// the emitter puts the call's line in its place, so a module's refusal
/// names its line as the interpreter's does (`crow_stop(__crow_line__, e)`).
pub const STOP_LINE: &str = "__crow_line__";

/// Trait that every Crow module implements.
/// Each module is self-contained: it declares its functions, constants,
/// and knows how to interpret calls, emit Rust code, and infer types.
pub trait CrowModule: Send + Sync {
    /// Module name (e.g. "math", "math.stats")
    fn name(&self) -> &str;

    /// List of function names this module provides.
    fn functions(&self) -> &[&str];

    /// Constants this module provides (e.g. pi, e, inf, nan).
    /// Returns (name, value) pairs.
    fn constants(&self) -> Vec<(&str, CrowValue)> {
        vec![]
    }

    // A function with a row in the builtin table (crate::table) is answered
    // by its row, for the interpreter, the emitter and inference alike; a
    // module's own arms answer the rest, and a module whose functions are
    // all rows has none.

    /// Interpret a function call. Called by the interpreter.
    fn call(&self, name: &str, _args: &[CrowValue]) -> Result<CrowValue, String> {
        Err(format!("module '{}' has no function '{}'", self.name(), name))
    }

    /// Emit Rust code for a function call. Called by the Rust backend.
    /// `arg_strs` are the argument expressions, already printed.
    /// `arg_types` are the inferred types of each argument.
    fn emit_rust(&self, name: &str, _arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        Err(format!("module '{}' has no function '{}'", self.name(), name))
    }

    /// Infer the return type of a function call. Called by the type inference engine.
    fn type_of(&self, _name: &str, _arg_count: usize) -> CrowType {
        CrowType::Unknown
    }
}

/// Registry of loaded modules. Provides generic dispatch.
#[derive(Clone)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn CrowModule>>,
    /// Maps function name → index into `modules` vec.
    /// Built when modules are loaded.
    fn_map: HashMap<String, Vec<usize>>,
    /// Maps constant name → index into `modules` vec.
    const_map: HashMap<String, usize>,
    /// Which modules each scope sees: the program's (`PROGRAM`, what its
    /// `use` lines import) and each standard module's written in Crow (its
    /// prefix, what its own `use` lines import — `sys`, which a program
    /// never sees). A name is resolved in the scope of the function it is
    /// read in (`enter`), so a program's imports never reach into a
    /// module's formulas, and a module's layer never reaches the program.
    scopes: HashMap<String, Vec<usize>>,
}

/// The program's own scope.
pub const PROGRAM: &str = "program";

thread_local! {
    /// The scope names are resolved in on this thread (a registry is shared
    /// by the threads a served program runs on; each resolves in the scope
    /// of the function it is running).
    static CURRENT_SCOPE: std::cell::RefCell<String> = std::cell::RefCell::new(PROGRAM.to_string());
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.modules.iter().map(|m| m.name()).collect();
        f.debug_struct("ModuleRegistry")
            .field("modules", &names)
            .finish()
    }
}

impl ModuleRegistry {
    pub fn new() -> Self {
        ModuleRegistry {
            modules: Vec::new(),
            fn_map: HashMap::new(),
            const_map: HashMap::new(),
            scopes: HashMap::new(),
        }
    }

    /// Load a module into the program's scope, registering all its
    /// functions and constants.
    pub fn load(&mut self, module: Arc<dyn CrowModule>) {
        self.load_for(PROGRAM, module);
    }

    /// Load a module into a scope: a module already loaded (by another
    /// scope) is shared, and becomes visible in this one.
    pub fn load_for(&mut self, scope: &str, module: Arc<dyn CrowModule>) {
        let idx = match self.modules.iter().position(|m| m.name() == module.name()) {
            Some(i) => i,
            None => {
                let idx = self.modules.len();
                for &fname in module.functions() {
                    self.fn_map.entry(fname.to_string()).or_default().push(idx);
                }
                for (cname, _) in module.constants() {
                    self.const_map.entry(cname.to_string()).or_insert(idx);
                }
                self.modules.push(module);
                idx
            }
        };
        let seen = self.scopes.entry(scope.to_string()).or_default();
        if !seen.contains(&idx) {
            seen.push(idx);
        }
    }

    /// Resolve names in the scope of a function from here on: a standard
    /// module's written in Crow sees its own imports, any other function
    /// the program's.
    pub fn enter(&self, fn_name: &str) -> String {
        let scope = crow_scope_of(fn_name).unwrap_or_else(|| PROGRAM.to_string());
        CURRENT_SCOPE.with(|c| c.replace(scope))
    }

    /// Back to a scope `enter` answered.
    pub fn resume(&self, scope: String) {
        CURRENT_SCOPE.with(|c| c.replace(scope));
    }

    /// The program's scope, where a pass over the program begins: a pass
    /// before it may have left a module's (the last function it typed).
    pub fn reset_scope() {
        CURRENT_SCOPE.with(|c| c.replace(PROGRAM.to_string()));
    }

    /// Whether the module at `idx` is visible in the current scope.
    fn visible(&self, idx: usize) -> bool {
        CURRENT_SCOPE.with(|c| self.scopes.get(c.borrow().as_str()).is_some_and(|v| v.contains(&idx)))
    }

    /// The modules providing a name, in the current scope.
    fn providers(&self, name: &str) -> Vec<usize> {
        self.fn_map.get(name).map(|idxs| idxs.iter().copied().filter(|&i| self.visible(i)).collect()).unwrap_or_default()
    }

    /// A registry with every stdlib module loaded — for tooling (checker,
    /// LSP) that needs the full export vocabulary without a `use` context.
    pub fn with_stdlib() -> Self {
        let mut r = Self::new();
        r.load(Arc::new(crate::modules::math::MathModule));
        r.load(Arc::new(crate::modules::stats::StatsModule));
        r.load(Arc::new(crate::modules::nn::NnModule));
        r.load(Arc::new(crate::modules::io::IoModule));
        r.load(Arc::new(crate::modules::frame::FrameModule));
        r.load(Arc::new(crate::modules::db::DbModule));
        r.load(Arc::new(crate::modules::doc::DocModule));
        r.load(Arc::new(crate::modules::chart::ChartModule));
        r
    }

    /// Every function name provided by any loaded module.
    pub fn all_function_names(&self) -> Vec<&str> {
        self.modules
            .iter()
            .enumerate()
            .filter(|(i, _)| self.visible(*i))
            .flat_map(|(_, m)| m.functions().iter().copied())
            .collect()
    }

    /// Check if a function name belongs to any loaded module.
    pub fn has_function(&self, name: &str) -> bool {
        !self.providers(name).is_empty()
    }

    /// All loaded modules providing this name — >1 means a bare call is
    /// ambiguous (spec: Conflict Resolution).
    pub fn providers_of(&self, name: &str) -> Vec<&str> {
        self.providers(name).into_iter().map(|i| self.modules[i].name()).collect()
    }

    /// Which loaded module provides this function (for reservation messages).
    /// Returns the first provider when several do.
    pub fn module_name_of(&self, name: &str) -> Option<&str> {
        let idx = self.providers(name).into_iter().next()?;
        Some(self.modules[idx].name())
    }

    /// Resolve a function within one specific module (prefixed calls).
    fn idx_in(&self, module_name: &str, fname: &str) -> Option<usize> {
        self.providers(fname).into_iter().find(|&i| self.modules[i].name() == module_name)
    }

    /// Call a module function through its module prefix.
    pub fn call_in(&self, module_name: &str, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        let idx = self.idx_in(module_name, name).ok_or_else(|| {
            format!("module '{}' has no function '{}'", module_name, name)
        })?;
        self.call_at(idx, name, args)
    }

    /// Emit Rust for a function through its module prefix.
    pub fn emit_rust_in(&self, module_name: &str, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        let idx = self.idx_in(module_name, name).ok_or_else(|| {
            format!("module '{}' has no function '{}'", module_name, name)
        })?;
        self.emit_at(idx, name, arg_strs, arg_types)
    }

    /// Infer return type through a module prefix, from the arguments' types.
    pub fn type_of_in(&self, module_name: &str, name: &str, arg_types: &[CrowType]) -> Option<CrowType> {
        let idx = self.idx_in(module_name, name)?;
        Some(self.type_at(idx, name, arg_types))
    }

    // A module function with a row in the builtin table is its runtime
    // function, for the interpreter, the emitter and inference alike
    // (crate::table); the module's own arms answer the rest.

    fn call_at(&self, idx: usize, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        if let Some(e) = crate::table::miscounted(self.modules[idx].name(), name, args.len()) {
            return Err(e);
        }
        let kinds: Vec<crate::table::Arg> = args.iter().map(crate::table::Arg::of_value).collect();
        match crate::table::module(self.modules[idx].name(), name, &kinds) {
            // a module's error value carries no trace, in either mode
            Some(row) => (row.call)(args, ""),
            None => self.modules[idx].call(name, args),
        }
    }

    fn emit_at(&self, idx: usize, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        if let Some(e) = crate::table::miscounted(self.modules[idx].name(), name, arg_strs.len()) {
            return Err(e);
        }
        let kinds: Vec<crate::table::Arg> = if arg_types.len() == arg_strs.len() {
            arg_types.iter().map(crate::table::Arg::of_type).collect()
        } else {
            vec![crate::table::Arg::Unknown; arg_strs.len()]
        };
        match crate::table::module(self.modules[idx].name(), name, &kinds) {
            Some(row) => crate::table::print(row, arg_strs, STOP_LINE, None),
            None => self.modules[idx].emit_rust(name, arg_strs, arg_types),
        }
    }

    fn type_at(&self, idx: usize, name: &str, arg_types: &[CrowType]) -> CrowType {
        let kinds: Vec<crate::table::Arg> = arg_types.iter().map(crate::table::Arg::of_type).collect();
        let module = self.modules[idx].name();
        // an argument whose kind is not known yet picks the row as the
        // program runs: the call's type is what every row of the name
        // answers, or not known — as a core builtin's is (the first row's
        // answer had been taken: `exp` of a parameter no call had typed was
        // a number, and stuck as the function's return)
        if kinds.iter().any(|k| matches!(k, crate::table::Arg::Unknown | crate::table::Arg::AnyList)) {
            // (the core's rows of the name too, where it has them: `slice`
            // of text or of bytes is picked as the program runs)
            let answers: Vec<CrowType> = crate::table::ROWS
                .iter()
                .filter(|r| (r.module == module || r.module.is_empty()) && r.name == name && crate::table::kinds_of(r.params, arg_types.len()).is_some())
                .map(|r| r.crow_type())
                .collect();
            if answers.iter().any(|t| *t != answers[0]) {
                return CrowType::Unknown;
            }
        }
        match crate::table::module(module, name, &kinds) {
            Some(row) => row.crow_type(),
            None => self.modules[idx].type_of(name, arg_types.len()),
        }
    }

    /// Check if a constant name belongs to any loaded module.
    pub fn has_constant(&self, name: &str) -> bool {
        self.const_map.get(name).is_some_and(|&i| self.visible(i))
    }

    /// Get the constant value for a name.
    pub fn get_constant(&self, name: &str) -> Option<CrowValue> {
        let idx = self.const_map.get(name).filter(|&&i| self.visible(i))?;
        let module = &self.modules[*idx];
        module.constants().into_iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    /// Bare-call ambiguity message, or None when the name is unique.
    fn ambiguity(&self, name: &str) -> Option<String> {
        let providers = self.providers_of(name);
        if providers.len() > 1 {
            let forms: Vec<String> = providers.iter().map(|m| format!("{}.{}[...]", m, name)).collect();
            Some(format!(
                "'{}' is ambiguous — provided by {}; use {}",
                name,
                providers.join(" and "),
                forms.join(" or ")
            ))
        } else {
            None
        }
    }

    /// Call a module function (interpreter, bare call).
    pub fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        if let Some(msg) = self.ambiguity(name) {
            return Err(msg);
        }
        let idx = self.providers(name).into_iter().next()
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.call_at(idx, name, args)
    }

    /// Emit Rust for a module function (the Rust backend, bare call).
    pub fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        if let Some(msg) = self.ambiguity(name) {
            return Err(msg);
        }
        let idx = self.providers(name).into_iter().next()
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.emit_at(idx, name, arg_strs, arg_types)
    }

    /// Infer return type of a module function (bare call; None when
    /// ambiguous), from the arguments' types.
    pub fn type_of(&self, name: &str, arg_types: &[CrowType]) -> Option<CrowType> {
        let idxs = self.providers(name);
        if idxs.len() != 1 {
            return None;
        }
        Some(self.type_at(idxs[0], name, arg_types))
    }


    /// Check if a specific module (by name) has been loaded.
    pub fn is_loaded(&self, module_name: &str) -> bool {
        self.modules.iter().enumerate().any(|(i, m)| m.name() == module_name && self.visible(i))
    }

    /// All registered function names, in the current scope.
    pub fn fn_names(&self) -> Vec<&str> {
        self.fn_map.iter().filter(|(_, idxs)| idxs.iter().any(|&i| self.visible(i))).map(|(s, _)| s.as_str()).collect()
    }

    /// All registered constant names, in the current scope.
    pub fn const_names(&self) -> Vec<&str> {
        self.const_map.iter().filter(|(_, i)| self.visible(**i)).map(|(s, _)| s.as_str()).collect()
    }

    /// Return the module name that owns a given function, if any.
    /// (First provider when several do — prefer providers_of for messages.)
    pub fn module_name_for(&self, fn_name: &str) -> Option<&str> {
        let idx = self.providers(fn_name).into_iter().next()?;
        Some(self.modules[idx].name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Stmt;

    fn names_called(body: &[Stmt]) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        body.iter().for_each(|s| called_in_stmt(s, &mut out));
        out
    }

    fn has_line(body: &[Stmt]) -> bool {
        body.iter().any(|s| match s {
            Stmt::Line(_) => true,
            Stmt::If { then_body, else_body, .. } => has_line(then_body) || else_body.as_ref().is_some_and(|e| has_line(e)),
            Stmt::For { body, .. } | Stmt::While { body, .. } => has_line(body),
            _ => false,
        })
    }

    /// `use math.nn` inlines the rows' functions and the helpers they reach,
    /// renamed behind the prefix, their line marks dropped; a row's function
    /// is called bare inside the module, a test-only helper is left out, and
    /// a program without the import gets nothing.
    #[test]
    fn a_crow_module_is_inlined_as_its_rows_reach_it() {
        let stmts = crate::build("use math, math.nn\nlog[relu[[1.0]]]\nlog[crossentropy[[[1.0, 0.0]], [0.0]]]\n").expect("parses");
        let out = inline_crow_modules(&stmts).expect("inlines");
        let fns: std::collections::BTreeMap<String, Vec<Stmt>> = out.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None }).collect();
        for f in ["relu", "crossentropy"] {
            assert!(fns.contains_key(&format!("_math_nn_{}", f)), "{} is inlined: {:?}", f, fns.keys());
        }
        for f in ["linear", "mse", "gelu"] {
            assert!(!fns.contains_key(&format!("_math_nn_{}", f)), "a row the program never reaches stays out: {:?}", fns.keys());
        }
        assert!(!fns.keys().any(|n| n.contains("through_all")), "a test's helper is not inlined: {:?}", fns.keys());
        for (n, body) in &fns {
            assert!(!has_line(body), "{} keeps no line mark", n);
        }
        let ce = names_called(&fns["_math_nn_crossentropy"]);
        assert!(ce.contains("onehot") && ce.contains("halt") && !ce.iter().any(|c| c.starts_with("_math_nn_")), "{:?}", ce);
        assert!(is_crow_fn("_math_nn_relu") && is_crow_fn("_traced__math_nn_relu") && !is_crow_fn("relu") && !is_crow_fn("_mathx_relu"));
        let plain = crate::build("use math\nlog[1]\n").expect("parses");
        assert_eq!(inline_crow_modules(&plain).expect("inlines").len(), plain.len());
        // a row reached by the module's name, handed as a value, or called
        // inside an interpolation
        let names = |src: &str| -> Vec<String> {
            let stmts = crate::build(src).expect("parses");
            inline_crow_modules(&stmts).expect("inlines").iter().filter_map(|s| if let Stmt::FnDecl { name, .. } = s { Some(name.clone()) } else { None }).collect()
        };
        let qualified = names("use math, math.nn\nlog[math.nn.sigmoid[[1.0]]]\n");
        assert!(qualified.contains(&"_math_nn_sigmoid".to_string()) && !qualified.contains(&"_math_nn_relu".to_string()), "{:?}", qualified);
        let handed = names("use math, math.nn\nf = gelu\nlog[f[[1.0]]]\n");
        assert!(handed.contains(&"_math_nn_gelu".to_string()), "{:?}", handed);
        let interpolated = names("use math, math.nn\nlog['`mse[[1.0], [2.0]]`']\n");
        assert!(interpolated.contains(&"_math_nn_mse".to_string()) && !interpolated.contains(&"_math_nn_relu".to_string()), "{:?}", interpolated);
    }

    /// A module's own imports are inlined after it (io's readers are
    /// written over sys's protocol), and its call of a core row written in
    /// Crow inlines the core — each module once, behind its own prefix,
    /// the row's function named by the row's id (`_core_read_asked`), and
    /// only what the program reaches: a reader it never calls stays out,
    /// as does every row of a module it never names.
    #[test]
    fn a_module_reaches_what_it_calls() {
        let names = |src: &str| -> Vec<String> {
            let stmts = crate::build(src).expect("parses");
            inline_crow_modules(&stmts).expect("inlines").iter().filter_map(|s| if let Stmt::FnDecl { name, .. } = s { Some(name.clone()) } else { None }).collect()
        };
        let fns = names("use io\nlog[csv['t.csv']]\n");
        // (every row of a name called: the one a call runs is decided
        // later, by its arguments — `csv` with the options is `csv_asked`)
        for f in ["_io_csv", "_io_csv_asked", "_core_read", "_core_read_asked", "_core_as_text", "_sys_remote", "_sys_fetch", "_sys_exchange", "_sys_read_answer"] {
            assert!(fns.contains(&f.to_string()), "{} is inlined: {:?}", f, fns);
        }
        for f in ["_io_lines", "_io_lines_asked", "_io_lines_of", "_core_write", "_core_request", "_math_load", "_frame_load"] {
            assert!(!fns.contains(&f.to_string()), "{} is never reached: {:?}", f, fns);
        }
        let mut seen = std::collections::BTreeSet::new();
        assert!(fns.iter().all(|f| seen.insert(f.clone())), "each function once: {:?}", fns);
        let imports = crow_imports();
        assert!(imports.iter().any(|(p, i)| p == "_io" && i == &["sys".to_string()]), "{:?}", imports);
        assert!(imports.iter().any(|(p, i)| p == "_core" && i == &["sys".to_string()]), "{:?}", imports);
        assert!(!imports.iter().any(|(p, _)| p == "_frame" || p == "_math"), "a module nothing reaches loads no scope: {:?}", imports);
        assert_eq!(crow_scope_of("_core_read_asked").as_deref(), Some("_core"));
        assert_eq!(crow_scope_of("_io_lines_of").as_deref(), Some("_io"));
        assert_eq!(crow_scope_of("lines_of"), None);
        // the options' row, by the module's name: both rows of the name
        let asked = names("use io\nlog[io.lines['t', timeout = 1.0]]\n");
        for f in ["_io_lines", "_io_lines_asked", "_io_lines_of", "_core_read_asked"] {
            assert!(asked.contains(&f.to_string()), "{} is inlined: {:?}", f, asked);
        }
        assert!(!asked.contains(&"_io_csv".to_string()), "{:?}", asked);
        // the core alone, for a program that reads a file
        let core = names("log[read['t.txt']]\n");
        assert!(core.contains(&"_core_read".to_string()) && !core.iter().any(|f| f.starts_with("_io_")), "{:?}", core);
        // a name of the program's that a module it never imports has a
        // row for reaches nothing: a feed's `put` is its parameter
        let feed = names("use web, frame\nfn lines[put, n]\n    put['x']\ns = stream[lines, 2]\n");
        assert_eq!(feed, vec!["lines".to_string()], "nothing is inlined: {:?}", feed);
    }

    /// The modules a program carries are read from its functions, with
    /// each one's own imports — whatever was inlined last on the thread
    /// (a harness lowers a part's tests and the app in turn).
    #[test]
    fn the_modules_carried_are_read_from_the_program() {
        let carried = crow_modules_carried(["_core_read", "_core_as_text", "_sys_fetch", "main", "_io_csv"].into_iter());
        assert_eq!(carried, vec![("_core".to_string(), vec!["sys".to_string()]), ("_io".to_string(), vec!["sys".to_string()]), ("_sys".to_string(), vec!["sys".to_string()])]);
        assert!(crow_modules_carried(["main", "helper"].into_iter()).is_empty());
        let frame = crow_modules_carried(["_frame_load"].into_iter());
        assert_eq!(frame, vec![("_frame".to_string(), vec!["sys".to_string(), "frame".to_string()])]);
    }

    /// A registry resolves a name in the scope of the function it is read
    /// in: the program's imports, or a standard module's own (`sys`, which
    /// the program never sees); the core's Crow source is inlined where a
    /// program calls one of its rows.
    #[test]
    fn a_name_is_resolved_in_its_functions_scope() {
        let mut r = ModuleRegistry::new();
        r.load(module_by_path("math").expect("math"));
        r.load_for("_core", module_by_path("sys").expect("sys"));
        assert!(r.has_function("exp") && !r.has_function("open"));
        let saved = r.enter("_core_request");
        assert!(r.has_function("open") && !r.has_function("exp"), "a module's scope is its own imports");
        assert_eq!(r.providers_of("slice"), vec!["sys"]);
        r.resume(saved);
        assert!(r.has_function("exp") && !r.has_function("open"));
        let stmts = crate::build("r = request['http://x/y']\nlog[r.status]\n").expect("parses");
        let out = inline_crow_modules(&stmts).expect("inlines");
        assert!(out.iter().any(|s| matches!(s, Stmt::FnDecl { name, .. } if name == "_core_request")), "the core's source is inlined for a call of request");
        assert!(crow_imports().iter().any(|(p, i)| p == "_core" && i.iter().any(|m| m == "sys")));
        let none = crate::build("log[1]\n").expect("parses");
        assert!(!inline_crow_modules(&none).expect("inlines").iter().any(|s| matches!(s, Stmt::FnDecl { .. })));
    }
}
