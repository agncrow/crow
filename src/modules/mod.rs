pub mod chart;
pub mod db;
pub mod doc;
pub mod frame;
pub mod io;
pub mod math;
pub mod nn;
pub mod stats;
pub mod sys;

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
        _ => None,
    }
}

/// The core builtins written in Crow (`request`): no import brings them,
/// so the source is inlined into a program that calls one of them.
pub(crate) const CORE_SOURCE: &str = include_str!("core.co");

/// The modules written in Crow, with the prefix their functions carry (the
/// core's, imported by no `use`, carries `_core`).
const CROW_MODULES: &[(&str, &str)] = &[("math.nn", "_math_nn"), ("", "_core")];

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
        _ => return None,
    })
}

/// Every inlined module's imports loaded into its own scope of a
/// registry (a module's `use math.nn` loads math too, as a program's does).
pub fn load_crow_scopes(registry: &mut ModuleRegistry) {
    for (prefix, imports) in crow_imports() {
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

/// The imports of every standard module the program inlined, by prefix.
pub fn crow_imports() -> Vec<(String, Vec<String>)> {
    CROW_IMPORTS.with(|c| c.borrow().clone())
}

/// The scope a function's names are resolved in: a standard module's
/// prefix for one of its functions (inlined, or its traced copy), none for
/// the program's own.
pub fn crow_scope_of(name: &str) -> Option<String> {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    CROW_MODULES.iter().find(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_').map(|(_, p)| p.to_string())
}

/// Whether a function of the program is a standard module's, inlined: its
/// stops name the line it was called from, not a line of the module's.
pub fn is_crow_fn(name: &str) -> bool {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    CROW_MODULES.iter().any(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_')
}

/// The program with every standard module it imports that is written in
/// Crow inlined after the import: the module's functions the table's rows
/// name, and the helpers they reach (a function only the module's tests
/// use is left out), each renamed behind the module's prefix. Inside the
/// module a call of a helper is rewritten to the helper's new name; a call
/// of a row's function stays bare, so it is the row's call wherever it is
/// made — read as the row's parameters take it, one signature. The
/// module's tests and its own imports are left out (the import loads what
/// the module needs), and its line marks dropped, so a stop inside one of
/// its functions names the line of the call in the program.
pub fn inline_crow_modules(stmts: &[crate::ast::Stmt]) -> Result<Vec<crate::ast::Stmt>, String> {
    use crate::ast::Stmt;
    let mut out: Vec<Stmt> = Vec::with_capacity(stmts.len());
    let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();
    CROW_IMPORTS.with(|c| c.borrow_mut().clear());
    // the core's, where the program calls one of its rows (a name of the
    // core's that is written in Crow: `request`)
    let core_rows: Vec<&str> = crate::table::ROWS.iter().filter(|r| r.module.is_empty() && r.is_crow()).map(|r| r.name).collect();
    let mut called = std::collections::BTreeSet::new();
    stmts.iter().for_each(|s| called_in_stmt(s, &mut called));
    let imports: Vec<(String, Option<&'static str>)> = std::iter::once(("".to_string(), core_rows.iter().any(|r| called.contains(*r)).then_some(CORE_SOURCE)))
        .chain(stmts.iter().filter_map(|s| if let Stmt::Use { path, .. } = s { Some((path.clone(), crow_source(path))) } else { None }))
        .collect();
    for st in stmts {
        out.push(st.clone());
    }
    for (path, source) in imports {
        let Some(source) = source else { continue };
        if !done.insert(path.clone()) {
            continue;
        }
        let path = path.as_str();
        let prefix = CROW_MODULES.iter().find(|(m, _)| *m == path).map(|(_, p)| *p).unwrap_or("_");
        let mut lexer = crate::lexer::Lexer::new(source);
        let tokens = lexer.tokenize().map_err(|e| format!("a bug in Crow: the module {} does not lex: {}", path, e))?;
        let mut parser = crate::parser::Parser::new(tokens);
        let mod_stmts = parser.parse().map_err(|e| format!("a bug in Crow: the module {} does not parse: {}", path, e))?;
        let imports: Vec<String> = mod_stmts.iter().filter_map(|m| if let Stmt::Use { path, .. } = m { Some(path.clone()) } else { None }).collect();
        CROW_IMPORTS.with(|c| c.borrow_mut().push((prefix.to_string(), imports)));
        // the rows' functions, and what they reach among the module's own
        let rows: std::collections::BTreeSet<String> = crate::table::ROWS.iter().filter(|r| r.module == path && r.is_crow()).map(|r| r.name.to_string()).collect();
        let bodies: std::collections::BTreeMap<String, &Vec<Stmt>> = mod_stmts.iter().filter_map(|m| if let Stmt::FnDecl { name, body, .. } = m { Some((name.clone(), body)) } else { None }).collect();
        let mut kept: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut todo: Vec<String> = rows.iter().cloned().collect();
        while let Some(n) = todo.pop() {
            if !kept.insert(n.clone()) {
                continue;
            }
            if let Some(body) = bodies.get(&n) {
                let mut called = std::collections::BTreeSet::new();
                for st in body.iter() {
                    called_in_stmt(st, &mut called);
                }
                todo.extend(called.into_iter().filter(|c| bodies.contains_key(c)));
            }
        }
        // a helper's references are rewritten; a row's function is reached
        // through its row
        let mut names: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        for m in &mod_stmts {
            match m {
                Stmt::FnDecl { name, .. } if kept.contains(name) && !rows.contains(name) => {
                    names.insert(name.clone(), format!("{}_{}", prefix, name));
                }
                Stmt::StructDecl { name, .. } => {
                    names.insert(name.clone(), format!("{}_{}", prefix, name));
                }
                _ => {}
            }
        }
        for m in &mod_stmts {
            match m {
                Stmt::FnDecl { name, params, defaults, body, is_view } if kept.contains(name) => {
                    let mut body = body.clone();
                    let shadowed: std::collections::HashSet<String> = params.iter().cloned().collect();
                    for st in &mut body {
                        crate::rewrite_refs_stmt(st, &names, &shadowed);
                    }
                    drop_lines(&mut body);
                    out.push(Stmt::FnDecl { name: format!("{}_{}", prefix, name), params: params.clone(), defaults: defaults.clone(), body, is_view: *is_view });
                }
                Stmt::StructDecl { name, fields } => {
                    out.push(Stmt::StructDecl { name: format!("{}_{}", prefix, name), fields: fields.clone() });
                }
                // the module's other functions (its tests' helpers), its
                // imports, its tests, and its line marks
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
            if let Expr::Ident(n) = callee.as_ref() {
                out.insert(n.clone());
            } else {
                called_in_expr(callee, out);
            }
            args.iter().for_each(|a| called_in_expr(a, out));
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
        let stmts = crate::build("use math, math.nn\nlog[relu[[1.0]]]\n").expect("parses");
        let out = inline_crow_modules(&stmts).expect("inlines");
        let fns: std::collections::BTreeMap<String, Vec<Stmt>> = out.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None }).collect();
        for f in nn::NN_FUNCTIONS {
            assert!(fns.contains_key(&format!("_math_nn_{}", f)), "{} is inlined", f);
        }
        assert!(!fns.keys().any(|n| n.contains("through_all")), "a test's helper is not inlined: {:?}", fns.keys());
        for (n, body) in &fns {
            assert!(!has_line(body), "{} keeps no line mark", n);
        }
        let ce = names_called(&fns["_math_nn_crossentropy"]);
        assert!(ce.contains("onehot") && ce.contains("halt") && !ce.iter().any(|c| c.starts_with("_math_nn_")), "{:?}", ce);
        assert!(is_crow_fn("_math_nn_relu") && is_crow_fn("_traced__math_nn_relu") && !is_crow_fn("relu") && !is_crow_fn("_math_nnx"));
        let plain = crate::build("use math\nlog[1]\n").expect("parses");
        assert_eq!(inline_crow_modules(&plain).expect("inlines").len(), plain.len());
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
