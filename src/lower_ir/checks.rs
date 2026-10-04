//! What the lowering refuses before anything runs, in Crow's terms: a name
//! nobody declared, a builtin's name taken, a call with the wrong number of
//! arguments, text added with `+`, a variable whose type changes. Each check
//! is made where the lowering meets the node, against the lowering's own
//! scopes and the types it gave the node's children, so the program is
//! judged by the environment it is lowered in (the fold, core-rewrite-notes
//! §10). Every error is collected, with the line it is on and the top-level
//! declaration it is in, so a program is refused with all of them at once.

use super::*;
use std::collections::HashMap;

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
    "rand", "time", "sleep", "env", "exit", "halt",
    "json", "encode", "decode",
    "error",
    "shell",
    "request",
    "hash",
    "find", "matches", "sub", "any", "all",
    "later", "running", "notice",
    "test",
    "now", "moment", "calendar", "parse_time", "captured", "chart",
    "cpu", "gpu", "nodes",
    "grad",
];

/// Why `mail` is refused in a view or a derivation, directly or through a
/// function it calls.
pub(super) const MAIL_IN_A_RENDER: &str = "mail[] sends a message, and a view or a derivation runs whenever the page needs it: send it from a handler, or a function a handler calls";

/// Reserved only in files that import web (use web / use web.*)
pub(crate) const WEB_RESERVED: &[&str] = &[
    "route", "serve", "respond", "redirect", "html", "text",
    "sign", "verify",
    "app", "click", "elements", "change", "box", "fits", "style",
    "socket", "send", "recv", "close",
    "page", "smtp", "mail", "mails", "msgpack", "arrow", "bytes", "stream",
    "reader", "sign_in", "sign_out", "download", "downloads",
];

/// Web builtins the compiler implements but deliberately does NOT reserve
/// (the response encoders are rarely-collided names). Listed so `crow test
/// --docs` can hold the spec to them.
pub(crate) const WEB_EXTRA_FUNCTIONS: &[&str] = &["arrow", "bytes", "stream"];

/// The web module's modules, `use web.auth` and the rest: what web does
/// beyond serving pages and routes (spec, web). A server's options are
/// `serve`'s and `route`'s, not modules of their own.
pub(crate) const WEB_MODULES: &[&str] = &["auth", "mail", "ws"];

/// Reserved only in files that import ui
pub(crate) const UI_RESERVED: &[&str] = &["fetch", "poll", "field", "slider", "checkbox", "toggle", "radio", "tabs", "progress", "icon", "today"];

/// All builtin function names recognized by the Crow language.
pub(crate) const BUILTINS: &[&str] = &[
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
    "rand", "time", "sleep", "env", "exit", "halt",
    // JSON
    "json", "encode", "decode",
    // Error
    "error",
    // Shell
    "shell",
    "request",
    // Hashing
    "hash",
    // Regex
    "find", "matches", "sub", "any", "all",
    "later", "running", "notice", "__notice_text", "__notice_at",
    "sign_in", "sign_out", "__reader", "__known", "__fmt_time", "download",
    "test",
    // Date/time
    "now", "moment", "calendar", "parse_time", "captured", "chart",
    // Resource
    "cpu", "gpu", "nodes",
    // Autograd
    "grad",
    // Web
    "route", "serve", "respond", "redirect", "html", "text",
    "arrow", "bytes", "stream",
    "exec", "query",
    "connect", "socket", "send", "recv", "close",
    "page", "smtp", "mail",
    // UI reactive builtins
    "fetch", "poll", "field", "slider", "checkbox", "toggle", "radio", "tabs", "select", "progress", "icon", "today",
];

/// The modules whose functions and constants a bare `use` makes names.
const REGISTRY_MODULES: &[&str] = &["math", "math.stats", "math.nn", "io", "frame", "doc", "chart", "sys"];

/// What the checks keep while the program is lowered.
#[derive(Default)]
pub(super) struct Checks {
    /// Each error with the top-level declaration it was found in: after
    /// module expansion a part's declarations carry its prefix, so a caller
    /// can say which file an error belongs to.
    pub errors: Vec<(String, Option<String>)>,
    /// The source line being lowered, named in each error.
    pub line: usize,
    /// The top-level declaration being lowered.
    pub decl: Option<String>,
    /// Above zero while lowering what was already checked (a
    /// specialisation's clone of a function), or what no check reads (the
    /// callee of an index, checked as the call it is).
    pub mute: u32,
    /// The program's own variables: its top-level assignments.
    pub module_vars: BTreeSet<String>,
    /// Bare imports: module path → line of the `use` (reservation source).
    pub bare_import_lines: BTreeMap<String, usize>,
    /// Aliases and module prefixes: name → line of the `use`.
    pub prefix_lines: BTreeMap<String, usize>,
    /// A `use ui` program.
    pub ui_mode: bool,
    /// Names an imported module makes: its functions and constants.
    pub used: BTreeSet<String>,
    /// The type each of the current function's own variables was first
    /// given: a local reusing a module variable's name is a new binding
    /// (spec §7), so its type is its own, not the module's.
    pub local_types: HashMap<String, CrowType>,
    /// Inside a test attached to a function (`test[p]` after it), which
    /// sees what a caller of that function sees.
    pub attached_test: bool,
    /// Inside a view or handler of a ui program, where the module's
    /// reactive state is visible.
    pub reactive: bool,
    /// The facts the current function has written: a name assigned is
    /// one it has from then on.
    pub stored: BTreeSet<String>,
    /// The line each fact is declared on: where its first value is given.
    pub fact_lines: HashMap<String, usize>,
    /// The functions a top-level `route[…]` answers with: each is handed
    /// its request and nothing else.
    pub route_handlers: BTreeSet<String>,
}

impl<'a> Lowerer<'a> {
    /// An error at the line being lowered, owned by the declaration it is in.
    pub(super) fn refuse(&mut self, msg: String) {
        if self.v.mute > 0 {
            return;
        }
        self.v.errors.push((format!("line {}: {}", self.v.line, msg), self.v.decl.clone()));
    }

    /// A statement of a ui program's top level, after a fact's declaration,
    /// that assigns the fact. A fact gets one value at the top level: the
    /// declarations are computed before the top level runs and a test runs
    /// only them, so a later change reached the rest of the top level under
    /// `crow run` and no session, no test, and no declaration after it.
    pub(super) fn check_top_write(&mut self, name: &str) {
        if !matches!(self.at, Where::Top) || !self.is_fact(name) {
            return;
        }
        let line = self.v.fact_lines.get(name).copied().unwrap_or(0);
        self.refuse(format!(
            "'{}' is state, declared on line {}: it gets one value at the top level, which every session starts from, and handlers move it after that — give it its whole value where it is declared (a function can build it), or derive it",
            name, line
        ));
    }

    /// Lowers with the checks muted: `f` lowers what is checked elsewhere.
    pub(super) fn muted<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.v.mute += 1;
        let out = f(self);
        self.v.mute -= 1;
        out
    }

    /// Before the walk: what the program imports, and its own variables —
    /// reservation is file-scoped, so a variable assigned above the `use`
    /// line is still checked against it.
    pub(super) fn prescan(&mut self, stmts: &[S], known: &HashSet<String>) {
        self.v.ui_mode = self.ctx.bare_modules.contains("ui");
        let mut line = 0usize;
        for s in stmts {
            match s {
                S::Line(n) => line = *n,
                S::Use { path, alias } => {
                    if path == "ui" {
                        self.v.ui_mode = true;
                    }
                    if let Some(sub) = path.strip_prefix("web.")
                        && !WEB_MODULES.contains(&sub)
                    {
                        self.v.line = line;
                        let modules: Vec<String> = WEB_MODULES.iter().map(|m| format!("web.{}", m)).collect();
                        self.refuse(format!("web has no module '{}' — its modules are {}; a server's CORS, files and rate limits are serve's and route's options", sub, modules.join(", ")));
                    }
                    match alias {
                        Some(a) => {
                            self.v.prefix_lines.insert(a.clone(), line);
                        }
                        None => {
                            self.v.bare_import_lines.insert(path.clone(), line);
                            // the module's prefix name is also taken
                            let prefix = path.rsplit(['.', '/']).next().unwrap_or(path).to_string();
                            self.v.prefix_lines.insert(prefix, line);
                        }
                    }
                }
                _ => {}
            }
        }
        // the names imports make, as the context loaded them (a fragment of
        // the REPL sees the modules an earlier one imported)
        let imported: Vec<String> = self.ctx.bare_modules.iter().chain(self.ctx.module_aliases.values()).cloned().collect();
        if imported.iter().any(|p| REGISTRY_MODULES.contains(&p.as_str())) {
            let registry = self.ctx.registry.clone();
            self.v.used.extend(registry.fn_names().iter().map(|n| n.to_string()));
            self.v.used.extend(registry.const_names().iter().map(|n| n.to_string()));
        }
        if imported.iter().any(|p| p == "math" || p.starts_with("math.")) {
            self.v.used.insert("math".to_string());
        }
        if imported.iter().any(|p| p == "web" || p.starts_with("web.")) {
            self.v.used.insert("web".to_string());
        }
        if imported.iter().any(|p| p == "ui") {
            self.v.used.insert("ui".to_string());
        }
        // the functions routes answer with (the last name after the path: a
        // method's text may come before it, and a limit after)
        for s in stmts {
            if let S::ExprStmt(A::Call { callee, args }) = s
                && matches!(callee.as_ref(), A::Ident(n) if n == "route")
                && let Some(h) = args.iter().skip(1).rev().find_map(|a| if let A::Ident(h) = a { Some(h.clone()) } else { None })
            {
                self.v.route_handlers.insert(h);
            }
        }
        // the program's own variables, each checked against the names
        // taken where it is first assigned
        self.v.module_vars.extend(known.iter().cloned());
        let mut line = 0usize;
        for s in stmts {
            let names: Vec<&String> = match s {
                S::Line(n) => {
                    line = *n;
                    continue;
                }
                S::Assign { name, .. } => vec![name],
                S::MultiAssign { names, .. } => names.iter().collect(),
                _ => continue,
            };
            for n in names {
                if !self.v.module_vars.contains(n) && !self.ctx.functions.contains_key(n) && !self.ctx.structs.contains_key(n) {
                    self.v.line = line;
                    self.v.decl = Some(n.clone());
                    self.check_reserved(n, "variable");
                }
                self.v.module_vars.insert(n.clone());
            }
        }
        self.v.line = 0;
        self.v.decl = None;
    }

    /// The function or derivation being lowered, when one is.
    pub(super) fn in_fn(&self) -> Option<String> {
        matches!(self.at, Where::In(_) | Where::Derive).then(|| self.current_fn.clone())
    }

    fn in_view(&self) -> bool {
        self.at == Where::In(FnKind::View)
    }

    /// Whether a name read here is one the program declares or imports: a
    /// function, a struct, a name a module makes, a variable in scope. A
    /// function sees its parameters and its own locals, not the program's
    /// variables (spec, No Ambient Mutability), unless it is a view or
    /// handler of a ui program; a test attached to a function sees what a
    /// caller of it sees.
    pub(super) fn defined(&self, name: &str) -> bool {
        let plain_fn = matches!(self.at, Where::In(_)) && !self.v.reactive;
        if self.ctx.functions.contains_key(name) && !(plain_fn && self.ctx.derives.contains(name)) {
            return true;
        }
        if self.ctx.structs.contains_key(name) || self.v.used.contains(name) {
            return true;
        }
        if self.in_view() && crate::lower::UI_ELEMENTS.contains(&name) {
            return true;
        }
        if self.scopes.iter().skip(1).any(|s| s.contains(name)) || self.v.stored.contains(name) {
            return true;
        }
        let hidden = plain_fn || (self.at == Where::Test && self.v.attached_test);
        !hidden && (self.scopes[0].contains(name) || self.v.module_vars.contains(name))
    }

    /// A lowered value's type as a check compares it, read as text. A
    /// call to a function whose returned value's type is not known yet is
    /// of an unknown type, not of none (inference types such a function as
    /// returning nothing until it knows), so an error in the function is not
    /// blamed on its callers too.
    pub(super) fn checked(&self, x: &Expr) -> CrowType {
        if let Expr::Call { target: Target::User(f), .. } = x {
            let returns = self.ctx.functions.get(f).map_or(false, |sig| returns_value(&sig.body));
            if returns && matches!(self.ctx.functions.get(f).map(|s| &s.return_type), Some(CrowType::Void)) {
                return CrowType::Unknown;
            }
        }
        read(x)
    }

    /// An assignment's target, before the value it is given: its variable
    /// is read, and each field it names must be the record's.
    pub(super) fn check_target(&mut self, object: &A) {
        match object {
            A::Ident(n) => self.check_ident(n),
            A::FieldAccess { object, field } => {
                self.check_target(object);
                let object_type = self.target_type(object);
                self.check_field(&object_type, field);
            }
            // an index's expressions are checked as they are lowered
            _ => {}
        }
    }

    /// A name read as a value.
    pub(super) fn check_ident(&mut self, name: &str) {
        if self.defined(name) || BUILTINS.contains(&name) {
            return;
        }
        let in_fn = self.in_fn();
        let prefix = in_fn.as_ref().map(|f| format!("in {}: ", f)).unwrap_or_default();
        if in_fn.is_some() && self.ctx.derives.contains(name) {
            self.refuse(format!(
                "{}derivations are not visible inside functions — read '{}' in a view or handler, or make this function a derivation",
                prefix, name
            ));
        } else if in_fn.as_ref().is_some_and(|f| self.v.route_handlers.contains(f)) && self.v.module_vars.contains(name) {
            // a route calls its handler with the request alone, so no
            // parameter can carry the variable in
            self.refuse(format!(
                "{}module variables are not visible inside functions — a route's handler is handed its request alone: open what it needs inside it (every connect to one URL in a program is one handle, so a handler that connects shares one pool)",
                prefix
            ));
        } else if in_fn.is_some() && self.v.module_vars.contains(name) {
            self.refuse(format!(
                "{}module variables are not visible inside functions — pass '{}' as a parameter, or wire the function as a handler",
                prefix, name
            ));
        } else {
            // the names in view: this body's, and the program's where a
            // plain function sees them
            let plain_fn = matches!(self.at, Where::In(_)) && !self.v.reactive;
            let hidden = plain_fn || (self.at == Where::Test && self.v.attached_test);
            let mut seen: Vec<String> = self.scopes.iter().skip(1).flat_map(|s| s.iter().cloned()).collect();
            seen.extend(self.v.stored.iter().cloned());
            if !hidden {
                seen.extend(self.scopes[0].iter().cloned());
                seen.extend(self.v.module_vars.iter().cloned());
            }
            let near = self.nearest(name, seen.iter().map(String::as_str));
            self.refuse(format!("undefined variable '{}'{}", name, near));
        }
    }

    /// Reservation: a user definition (variable, fn, struct, param, loop var)
    /// may not take a name the language or an imported module owns.
    /// (Spec Design Note "Names Are Never Ambiguous".)
    pub(super) fn check_reserved(&mut self, name: &str, what: &str) {
        // a standard module's function written in Crow, inlined, was
        // checked as a program of its own (`crow test src/modules/sys.co`)
        // under its own imports: the program's imports, aliases and
        // elements reserve nothing in it (its `m` beside a `use math as m`)
        if crate::modules::is_crow_fn(&self.current_fn) {
            return;
        }
        // (the standard library's own source — the one kind of file that
        // imports `sys` — defines the functions its Crow rows name:
        // `fn request` in src/modules/core.co, tested on its own)
        let own_row = what == "function"
            && self.ctx.bare_modules.contains("sys")
            && crate::table::ROWS.iter().any(|r| r.module.is_empty() && r.is_crow() && crate::table::crow_fn_base(r) == name);
        if CORE_RESERVED.contains(&name) && !own_row {
            self.refuse(format!("'{}' is a core builtin — pick another name for this {}", name, what));
            return;
        }
        // An import reserves names in the file that makes it — a part's own:
        // the whole program carries every file's imports, and each part was
        // validated alone under its own (a part that never imports chart may
        // name a parameter `vwap`, one that never imports web `close`)
        if self.v.decl.as_deref().map_or(false, |d| crate::file_of_declaration(d).is_some()) {
            return;
        }
        // Bare stdlib imports reserve their exports — the entry's own: an
        // import a part makes has no line of the entry's and reserves names
        // in the part alone
        let entry_line = |module: &str| self.v.bare_import_lines.get(module).copied().filter(|l| *l > 0);
        if let Some(module) = self.ctx.registry.module_name_of(name) {
            // (the standard library's own source — the one kind of file that
            // imports `sys` — defines the functions its module's Crow rows
            // name: `fn fetch` in src/modules/sys.co, tested on its own)
            let own_row = what == "function"
                && self.ctx.bare_modules.contains("sys")
                && crate::table::ROWS.iter().any(|r| r.module == module && r.is_crow() && crate::table::crow_fn_base(r) == name);
            if own_row {
                return;
            }
            if let Some(line) = self.ctx.bare_modules.contains(module).then(|| entry_line(module)).flatten() {
                let module = module.to_string();
                self.refuse(format!(
                    "'{}' is provided by {} (imported on line {}) — rename the {}, or import qualified: use {} as m",
                    name, module, line, what, module
                ));
                return;
            }
        }
        // Constants (pi, e, inf, nan) reserve under a bare math import
        if self.ctx.registry.has_constant(name) && self.ctx.bare_modules.contains("math") {
            if let Some(line) = entry_line("math") {
                self.refuse(format!(
                    "'{}' is a math constant (imported on line {}) — rename the {}, or import qualified: use math as m",
                    name, line, what
                ));
                return;
            }
        }
        let web_bare = self.v.bare_import_lines.keys().any(|p| p == "web" || p.starts_with("web."));
        if web_bare && name == "reader" {
            self.refuse(format!("'reader' is the page's signed-in reader, which sign_in and sign_out set — rename the {}", what));
            return;
        }
        if web_bare && WEB_RESERVED.contains(&name) {
            self.refuse(format!("'{}' is a web builtin (use web) — rename the {}", name, what));
            return;
        }
        if self.v.ui_mode && UI_RESERVED.contains(&name) {
            self.refuse(format!("'{}' is a ui builtin (use ui) — rename the {}", name, what));
            return;
        }
        // Module prefixes and aliases are taken
        if let Some(line) = self.v.prefix_lines.get(name).copied() {
            self.refuse(format!("'{}' is a module name (use on line {}) — pick another name for this {}", name, line, what));
        }
    }

    /// An assignment: a derivation is not assigned, and a variable keeps the
    /// type it was first given.
    pub(super) fn check_assign(&mut self, name: &str, new_ty: &CrowType) {
        let in_local_scope = self.in_fn().is_some() && !(self.v.reactive && self.v.module_vars.contains(name));
        let existing = if in_local_scope { self.v.local_types.get(name).cloned() } else { self.ctx.vars.get(name).cloned() };
        if in_local_scope && !matches!(new_ty, CrowType::Unknown) {
            self.v.local_types.entry(name.to_string()).or_insert_with(|| new_ty.clone());
        }
        let Some(existing_ty) = existing else { return };
        // Assignment into a typed variable is a consumption point: result[T]
        // unwraps into T (the function stops on the error) — not a type change.
        let unwraps = matches!(new_ty, CrowType::Result(inner)
            if existing_ty.compatible(inner)
                || (matches!(existing_ty, CrowType::Float) && matches!(inner.as_ref(), CrowType::Int)))
            || matches!(&existing_ty, CrowType::Result(inner) if inner.compatible(new_ty));
        if !unwraps && !existing_ty.compatible(new_ty) && !matches!(existing_ty, CrowType::Unknown) && !matches!(new_ty, CrowType::Unknown) {
            // a value whose type the program does not know (a cell of a
            // column no database described, a parsed document's member)
            // says what it is first
            let unknown = matches!(new_ty, CrowType::Json) || matches!(new_ty, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Json));
            let why = if unknown {
                "a value whose type the program does not know is read as one first: str[…], int[…] or float[…]"
            } else {
                "variable type cannot change"
            };
            self.refuse(format!("cannot reassign '{}' from {} to {} — {}", name, existing_ty.crow_name(), new_ty.crow_name(), why));
        }
    }

    /// A derivation reads facts and sources and returns a value: it is a
    /// computation, so it writes no fact, and it lives in a ui program.
    pub(super) fn check_derive(&mut self, name: &str, body: &[S]) {
        self.check_reserved(name, "derivation");
        if !self.v.ui_mode {
            self.refuse(format!("derive {} needs reactive state — derivations live in a `use ui` program with a page", name));
        }
        fn first_fact_write(stmts: &[S], facts: &BTreeSet<String>) -> Option<String> {
            for s in stmts {
                match s {
                    S::Assign { name, .. } if facts.contains(name) => return Some(name.clone()),
                    S::MultiAssign { names, .. } => {
                        if let Some(n) = names.iter().find(|n| facts.contains(*n)) {
                            return Some(n.clone());
                        }
                    }
                    S::If { then_body, else_body, .. } => {
                        if let Some(n) = first_fact_write(then_body, facts) {
                            return Some(n);
                        }
                        if let Some(eb) = else_body {
                            if let Some(n) = first_fact_write(eb, facts) {
                                return Some(n);
                            }
                        }
                    }
                    S::For { body, .. } | S::While { body, .. } => {
                        if let Some(n) = first_fact_write(body, facts) {
                            return Some(n);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        if let Some(v) = first_fact_write(body, &self.v.module_vars) {
            self.refuse(format!("derive {} writes '{}' — a derivation computes a value; only handlers write facts", name, v));
        }
    }

    /// A struct's fields: the error-metadata names (.ok/.reason/.kind/.trace)
    /// are a fixed vocabulary on every value, so a field named one of them
    /// could never be read.
    pub(super) fn check_struct(&mut self, name: &str, fields: &[String]) {
        self.check_reserved(name, "struct");
        for f in fields {
            if matches!(f.as_str(), "ok" | "reason" | "kind" | "trace") {
                self.refuse(format!(
                    "struct '{}' field '{}' collides with error-value metadata (.ok/.reason/.kind/.trace are reserved on every value) — rename it",
                    name, f
                ));
            }
        }
    }

    /// An arithmetic operator's operands, by the types they were lowered to.
    pub(super) fn check_binop(&mut self, op: &BinOp, lt: CrowType, rt: CrowType) {
        let lt = if let CrowType::Result(inner) = lt { *inner } else { lt };
        let rt = if let CrowType::Result(inner) = rt { *inner } else { rt };
        if matches!(lt, CrowType::Unknown) || matches!(rt, CrowType::Unknown) {
            return;
        }
        if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
            return;
        }
        // String + is not allowed — use interpolation instead
        if matches!(op, BinOp::Add) && (matches!(lt, CrowType::Str) || matches!(rt, CrowType::Str)) {
            self.refuse("cannot use '+' with strings — interpolate to combine ('`a` `b`'), or collect parts and join[parts, sep] to build".to_string());
            return;
        }
        // Numeric ops: both must be numeric (a tensor, a sparse matrix, a
        // frame's column as its numbers); a list of text is no numbers (the
        // compiled program had parsed it, where the interpreter made each
        // item an error)
        let is_num_like = |t: &CrowType| matches!(t, CrowType::Int | CrowType::Float | CrowType::NumArray(..) | CrowType::Sparse | CrowType::FrameCol);
        if !is_num_like(&lt) || !is_num_like(&rt) {
            let op_str = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                _ => "%",
            };
            // a value whose type is not known (a cell of a column nothing
            // typed, a parsed document's member) says what it is first
            let hint = if matches!(lt, CrowType::Json) || matches!(rt, CrowType::Json) {
                " — a value whose type the program does not know is read as one first: int[…], float[…] or str[…]"
            } else {
                ""
            };
            self.refuse(format!("cannot use '{}' with {} and {}{}", op_str, lt.crow_name(), rt.crow_name(), hint));
        }
    }

    /// A comparison of two values: two of one kind — numbers (an int beside
    /// a float), text, bools, records, maps — and an order only of numbers
    /// or of text; a list's or a frame's column's items with one value of
    /// their kind (a mask), or two lists of one kind whole. Where both sides'
    /// kinds are known and differ, or an order is asked of a kind with
    /// none, the comparison is refused before the program runs, in the
    /// words the runtime stops with where they show only as it runs
    /// (crow_runtime::mask::incomparable): a number is not text (Vijay,
    /// 2026-10-01), nor a bool either. A parsed value is compared as the
    /// program runs, and is read as one type before an order. (Compiled,
    /// `true gt false` had answered and text had been parsed as numbers,
    /// where the interpreter stopped or answered false; several did not
    /// build.)
    ///
    /// A list of values that can fail compared with one value is a mask,
    /// and a mask decides every row: it is refused as a filter refuses it
    /// (compiled, it had not built, where the interpreter stopped on a
    /// failed item).
    pub(super) fn check_order(&mut self, op: &BinOp, l: &Ty, r: &Ty) {
        let cmp = match op {
            BinOp::Eq => crow_runtime::mask::Cmp::Eq,
            BinOp::Ne => crow_runtime::mask::Cmp::Ne,
            BinOp::Gt => crow_runtime::mask::Cmp::Gt,
            BinOp::Lt => crow_runtime::mask::Cmp::Lt,
            BinOp::Gte => crow_runtime::mask::Cmp::Gte,
            BinOp::Lte => crow_runtime::mask::Cmp::Lte,
            _ => return,
        };
        let failing = |t: &Ty| matches!(t.payload(), Ty::List(item) if item.is_result());
        let one = |t: &Ty| matches!(order_side(t), Some(OrderSide::One(_)));
        if (failing(l) && one(r)) || (one(l) && failing(r)) {
            self.refuse(format!("{} of a list of values that can fail is a mask, and a mask decides every row: guard each value with .ok (cells of a column that can be missing: compare the column, or fill it first, fill[frame, 'column', value])", cmp.word()));
            return;
        }
        if matches!(l.payload(), Ty::Json) || matches!(r.payload(), Ty::Json) {
            if cmp.orders() {
                self.refuse(format!("{} compares numbers with numbers or text with text — a value whose type the program does not know is read as one first: int[…], float[…] or str[…]", cmp.word()));
            }
            return;
        }
        let (Some(a), Some(b)) = (order_side(l), order_side(r)) else { return };
        let scalar = |k: &str| matches!(k, "a number" | "text" | "a bool");
        let (x, y) = match (&a, &b) {
            // two lists: an order has none, and equality is whole, by the
            // items each holds (a matrix's are its rows)
            (OrderSide::Many(_), OrderSide::Many(_)) => {
                let (Some(x), Some(y)) = (whole_items(l), whole_items(r)) else { return };
                if !cmp.orders() && x == y {
                    return;
                }
                (many_words(x), many_words(y))
            }
            // a mask: the items with a value of their kind
            (OrderSide::Many(x), OrderSide::One(y)) | (OrderSide::One(y), OrderSide::Many(x)) if scalar(y) => {
                let (x, y) = if matches!(a, OrderSide::Many(_)) { (*x, *y) } else { (*y, *x) };
                (x.to_string(), y.to_string())
            }
            (OrderSide::Many(x), OrderSide::One(y)) => (many_words(x), y.to_string()),
            (OrderSide::One(x), OrderSide::Many(y)) => (x.to_string(), many_words(y)),
            (OrderSide::One(x), OrderSide::One(y)) => (x.to_string(), y.to_string()),
        };
        let compares = x == y && if cmp.orders() { matches!(x.as_str(), "a number" | "text") } else { true };
        if !compares {
            self.refuse(crow_runtime::mask::incomparable(cmp, &x, &y));
        }
    }

    /// An item put in a collection, or looked for in one, is of the kind of
    /// its items (`push`, `contains`, `set`): a list holds one kind (spec,
    /// Arrays), and a number is not text. Refused where both kinds are
    /// known. (The interpreter had made a list of two kinds, and answered
    /// false for an item looked for in a list of another kind.)
    pub(super) fn check_item(&mut self, row: &crate::table::Row, args: &[Expr]) {
        let (holder, item, takes, what) = match row.id {
            "core.push" => (args.first(), args.get(1), "a list and an item of its kind", "items"),
            "core.contains" => (args.first(), args.get(1), "a list and an item of its kind", "items"),
            "core.set" => (args.first(), args.get(2), "a map, a key and a value of its kind", "values"),
            _ => return,
        };
        let (Some(holder), Some(item)) = (holder, item) else { return };
        let inner = match holder.ty().payload() {
            Ty::List(i) | Ty::Map(i) => (**i).clone(),
            _ => return,
        };
        if let (Some((many, _)), Some((others, one))) = (item_kind(&inner), item_kind(&item.ty()))
            && many != others
        {
            self.refuse(format!("{}[] takes {}: its {} are {}, not {}", row.name, takes, what, many, one));
        }
    }

    /// A field read from a record; a list has `len` and no other field (a
    /// list of tensors asked its `.shape` had stopped interpreted and not
    /// built compiled).
    pub(super) fn check_field(&mut self, obj_type: &CrowType, field: &str) {
        if matches!(field, "ok" | "reason" | "kind" | "trace") {
            return;
        }
        if matches!(obj_type, CrowType::Array(_)) && field != "len" {
            let owner = match field {
                "shape" | "dtype" | "device" => "a tensor's",
                "columns" | "dtypes" => "a frame's",
                _ => "a record's",
            };
            self.refuse(format!("a list has no field '{}': `.{}` is {} — index an item for its own", field, field, owner));
            return;
        }
        if let CrowType::Struct(name) = obj_type {
            if let Some(sig) = self.ctx.structs.get(name) {
                if !sig.fields.iter().any(|(f, _)| f == field) {
                    let name = name.clone();
                    self.refuse(format!("struct '{}' has no field '{}'", name, field));
                }
            }
        }
    }

    /// Whether `m.x` names something a module (or a part) declares, where
    /// `m` is not a variable: then `m` is not read as one.
    pub(super) fn names_module_member(&self, module: &str, field: &str) -> bool {
        let prefixed = format!("{}_{}", module, field);
        !self.defined(module) && (self.defined(&prefixed) || self.ctx.vars.contains_key(&prefixed) || self.ctx.functions.contains_key(&prefixed))
    }

    /// A call through a module's name, `m.f[…]`.
    pub(super) fn check_module_call(&mut self, module: &str, field: &str) {
        let prefixed = format!("{}_{}", module, field);
        let real = self.ctx.module_aliases.get(module).cloned().unwrap_or_else(|| module.to_string());
        // the app is implicit (an app value, and the calls that took one,
        // had built compiled only, and the interpreter read it as a path)
        if real == "web" && field == "new" {
            self.refuse("web.new[] is gone: the app is implicit — route[…], page[…] and serve[…] need none".to_string());
            return;
        }
        let registry_module = REGISTRY_MODULES.contains(&real.as_str()) || real == "db";
        let registry_module = registry_module && self.ctx.registry.has_function(field);
        if !registry_module && !self.ctx.functions.contains_key(&prefixed) && !self.ctx.structs.contains_key(&prefixed) && !self.defined(module) {
            // (a module that is one, asked for what it does not have — the
            // core's `read` through `io` — is told so, not that it is none)
            let known = REGISTRY_MODULES.contains(&real.as_str()) || real == "db";
            if known && CORE_RESERVED.contains(&field) {
                self.refuse(format!("{} has no '{}' — {} is the core's: {}[…]", module, field, field, field));
            } else if known {
                self.refuse(format!("{} has no '{}'", module, field));
            } else {
                self.refuse(format!("undefined module or variable '{}'", module));
            }
        }
    }

    /// A call by name, its arguments already lowered and checked.
    /// A builtin with rows takes the counts and kinds its rows take — the
    /// core's, and a loaded module's of the same name — refused in the
    /// table's words when none does.
    fn refuse_kinds(&mut self, name: &str, argc: usize, types: &[CrowType]) {
        if !crate::table::has_core_rows(name) {
            return;
        }
        let kinds: Vec<crate::table::Arg> = (0..argc).map(|i| types.get(i).map_or(crate::table::Arg::Unknown, crate::table::Arg::of_type)).collect();
        // `json[map]` is web's JSON response (use web), a form of its own
        let web = self.v.bare_import_lines.keys().any(|p| p == "web" || p.starts_with("web."));
        if name == "json" && web && kinds.first() == Some(&crate::table::Arg::Map) {
            return;
        }
        let providers: Vec<String> = self.ctx.registry.providers_of(name).into_iter().map(str::to_string).collect();
        let mut modules: Vec<&str> = vec![""];
        modules.extend(providers.iter().map(String::as_str));
        if let Some(e) = crate::table::refusal(&modules, name, &kinds) {
            self.refuse(e);
        }
    }

    /// The fact a page's address names, as both backends resolve it: the
    /// fact of that name, or the one fact of a part it is the last name of.
    fn address_fact(&self, written: &str) -> Option<String> {
        let flat = written.replace('.', "_");
        if self.ctx.facts.contains(&flat) {
            return Some(flat);
        }
        let suffix = format!("_{}", flat);
        let mut found = self.ctx.facts.iter().filter(|k| k.ends_with(&suffix));
        let first = found.next()?.clone();
        found.next().is_none().then_some(first)
    }

    pub(super) fn check_call(&mut self, name: &str, args: &[A], types: &[CrowType]) {
        let arg_type = |_: &Self, i: usize| -> CrowType { types.get(i).cloned().unwrap_or(CrowType::Unknown) };
        // a scheme encode and decode do not know, written in the call, is
        // refused before either mode runs (computed, encode stops on it)
        if matches!(name, "encode" | "decode") && args.len() == 2 {
            if let A::Str(scheme) = &args[1] {
                if !crow_runtime::text::SCHEMES.contains(&scheme.as_str()) {
                    self.refuse(crow_runtime::text::unknown_scheme(name, scheme));
                }
            }
        }
        if name == "route" && !self.defined("route") && !matches!(arg_type(self, 0), CrowType::Str | CrowType::Unknown) {
            self.refuse("route[] takes a path first — the app is implicit: route['/', handler]".to_string());
        }
        // Test-harness builtins exist only inside test blocks (which never
        // reach a build) — accept them there, refuse them anywhere else.
        if matches!(name, "app" | "click" | "elements" | "change" | "box" | "fits" | "style" | "downloads" | "mails") {
            if self.at != Where::Test {
                self.refuse(format!("'{}' is a test-harness builtin — it only works inside a test block", name));
            }
            return;
        }
        let argc = args.len();
        let own = self.ctx.functions.contains_key(name);

        // a route and its responses are web's: without `use web` neither
        // mode has them (the interpreter had answered msgpack, arrow, bytes
        // and stream with a map that only looked like a response)
        let web_program = self.ctx.bare_modules.iter().any(|m| m == "web" || m.starts_with("web."));
        // (a module imported bare that provides the name answers it: `bytes`
        // under `use sys` is the standard library's, not a response)
        // (inside a standard module written in Crow, the registry's scope is
        // the module's own imports: a provider there is one it imported)
        let provided = self.ctx.registry.providers_of(name).iter().any(|m| self.ctx.bare_modules.contains(*m) || crate::modules::is_crow_fn(&self.current_fn));
        if !own && !web_program && !provided && matches!(name, "route" | "serve" | "respond" | "redirect" | "msgpack" | "arrow" | "bytes" | "stream") {
            self.refuse(format!("{}[] is web's: add `use web`", name));
            return;
        }

        // stream[feed, a, …] calls feed[put, a, …]: the feed is a function
        // of the program, taking put and then what the call hands it
        if name == "stream" && !own {
            match args.first() {
                Some(A::Ident(f)) if self.ctx.functions.contains_key(f) => {
                    let takes = self.ctx.functions.get(f).map_or(0, |sig| sig.params.len());
                    if takes != argc {
                        self.refuse(format!(
                            "stream[] calls {}[put{}], and {} takes {} parameter{}: its first is put, and the rest are what stream[{}, …] hands it",
                            f,
                            ", …".repeat(usize::from(argc > 1)),
                            f,
                            takes,
                            if takes == 1 { "" } else { "s" },
                            f
                        ));
                    }
                }
                _ => self.refuse("stream[] takes a function of the program, which it calls with put: stream[rows], with fn rows[put]".to_string()),
            }
        }

        // a page's address holds facts (spec, Core (Server)): each name its
        // path's `:name` segments give, and each `url` name, is a fact —
        // one of a part by its last name — refused here, in both modes
        if name == "page" {
            if let Some(A::Str(path)) = args.first() {
                for p in crow_web::core::path::params(path) {
                    if self.address_fact(&p).is_none() {
                        self.refuse(format!("page['{}', …]: ':{}' names no fact — a page's path holds facts, as url = […] does", path, p));
                    }
                }
            }
            if let Some(A::Array(names)) = args.get(2) {
                for n in names {
                    if let A::Str(written) = n {
                        if self.address_fact(written).is_none() {
                            self.refuse(format!("page[…, url = …]: no reactive fact named '{}'", written));
                        }
                    }
                }
            }
        }

        // Maps are dictionaries: set[] must not change the map's value type
        // (spec Design Note "Maps Are Dictionaries, Not Records"), said in
        // Crow terms before either backend meets it.
        if name == "set" && argc == 3 {
            let map_ty = arg_type(self, 0);
            let map_ty = if let CrowType::Result(i) = map_ty { *i } else { map_ty };
            if let CrowType::Map(inner) = &map_ty {
                let val_ty = arg_type(self, 2);
                let val_ty = if let CrowType::Result(i) = val_ty { *i } else { val_ty };
                if !matches!(inner.as_ref(), CrowType::Unknown) && !matches!(val_ty, CrowType::Unknown | CrowType::Void) && !inner.compatible(&val_ty) {
                    self.refuse(format!(
                        "map values must all be the same type — this map holds {} values, cannot add {} (use a struct for mixed named data)",
                        inner.crow_name(),
                        val_ty.crow_name()
                    ));
                }
            }
        }

        // Layout attributes are part of the vocabulary: literal, and valid per
        // the kernel table both renderers use (crow_runtime::layout).
        if crate::lower::UI_ELEMENTS.contains(&name) && self.in_view() {
            if let Some(A::Map(pairs)) = args.first() {
                let mut literal: Vec<(String, String)> = Vec::new();
                for (k, v) in pairs {
                    let key = match k {
                        A::Ident(s) | A::Str(s) => s.clone(),
                        _ => continue,
                    };
                    if !crow_runtime::layout::LAYOUT_ATTRS.contains(&key.as_str()) {
                        continue;
                    }
                    // state-driven by design: open follows a bool, active a
                    // path, tip a text — any expression, checked at render
                    if matches!(key.as_str(), "open" | "active" | "tip") && !matches!(v, A::Str(_) | A::Int(_) | A::Bool(_)) {
                        continue;
                    }
                    match v {
                        A::Str(s) => literal.push((key, s.clone())),
                        A::Int(n) => literal.push((key, n.to_string())),
                        A::Bool(b) => literal.push((key, b.to_string())),
                        _ => self.refuse(format!(
                            "{}: '{}' is a layout attribute and must be a literal — a scale step like 'm', a size like '240px', or true",
                            name, key
                        )),
                    }
                }
                for e in crow_runtime::layout::render(name, &literal).errors {
                    self.refuse(format!("{}: {}", name, e));
                }
                // a table sorts by its headers when the program says so
                let sort = pairs.iter().find(|(k, _)| matches!(k, A::Ident(s) | A::Str(s) if s == "sort"));
                if name == "table" && sort.is_some_and(|(_, v)| !matches!(v, A::Bool(_))) {
                    self.refuse("table: sort takes true or false as written: sort = true makes each header sort the rows by its column".to_string());
                }
                // a shortcut presses its element, which must do something
                // when pressed
                let named = |want: &str| pairs.iter().any(|(k, _)| matches!(k, A::Ident(s) | A::Str(s) if s == want));
                if named("shortcut") && !named("click") && !matches!(name, "a" | "button") {
                    self.refuse(format!(
                        "{}: shortcut = … presses the element, and pressing a {} does nothing: give it click = a handler",
                        name, name
                    ));
                }
                // an image says what it shows, or that it only decorates
                if name == "img" && !named("alt") {
                    self.refuse("img: alt = '…' says what the image shows to a reader who cannot see it, or alt = '' that it only decorates".to_string());
                }
                // a widget's events each name a handler
                if name == "widget" {
                    for (k, v) in pairs {
                        if !matches!(k, A::Ident(s) | A::Str(s) if s == "on") {
                            continue;
                        }
                        let named = crate::lower::map_literal_pairs(v).unwrap_or_default();
                        for (event, h) in &named {
                            if !matches!(h, A::Ident(f) if self.ctx.functions.contains_key(f)) {
                                self.refuse(format!(
                                    "widget: on = map[{} = …] names a handler — a function the widget's event runs: on = map[{} = choose]",
                                    event, event
                                ));
                            }
                        }
                        if named.is_empty() && !matches!(v, A::Map(p) if p.is_empty()) {
                            self.refuse("widget: on is a map from each event the widget sends to its handler: on = map[pick = choose]".to_string());
                        }
                    }
                }
            }
            // Inside a view the element wins over a module function of the
            // same bare name (stack vs frame.stack / math.stack): the prefix
            // is how a view reaches the module version.
            return;
        }

        // ui elements exist only inside view blocks
        if crate::lower::UI_ELEMENTS.contains(&name) && self.v.ui_mode && !self.in_view() {
            self.refuse(format!(
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
                self.refuse(format!("'{}' is ambiguous — provided by {}; use {}", name, providers.join(" and "), forms.join(" or ")));
                return;
            }
            if let Some(module) = self.ctx.registry.module_name_of(name) {
                if self.ctx.bare_modules.contains(module) {
                    // a name the core answers too (frame's sort, math's max)
                    // takes what the core's rows or the module's take
                    self.refuse_kinds(name, argc, types);
                    return;
                }
                // Imported under an alias?
                let alias = self.ctx.module_aliases.iter().find(|(_, p)| p.as_str() == module).map(|(a, _)| a.clone());
                if let Some(alias) = alias {
                    let module = module.to_string();
                    self.refuse(format!("'{}' comes from {} which was imported as '{}' — call {}.{}[...]", name, module, alias, alias, name));
                    return;
                }
            }
            return;
        }

        // builtins that were removed, each with its replacement
        let removed = match name {
            "pad" => Some("pad was removed — use fmt[s, width = n]"),
            "append" => Some("append was removed — use concat[a, b] (one merge operation)"),
            // the time builtins the moment replaced (spec, Design Note on moments)
            "date" => Some("date[] was removed - a moment is a number: moment[y, m, d] (hour/minute/second/tz are named args)"),
            "elapsed" => Some("elapsed[] was removed - subtract moments: end - start is seconds"),
            "today" => Some("today[] was removed - use fmt[now[], time = 'date'] (tz = ... for a zone)"),
            _ => None,
        };
        if let Some(msg) = removed {
            self.refuse(msg.to_string());
            return;
        }

        // Known builtin? Check where it is called and its argument count
        // (a local or a parameter of the name, indexed — `query[0]` of a
        // parameter `query` — is a value read, not the builtin)
        if BUILTINS.contains(&name) && !self.declared(name) {
            let for_session = self.in_fn().map_or(false, |f| self.ctx.view_handlers.contains(&f));
            // the notice is the session's: only something acting for a
            // session — a handler, or a function a handler calls — can say it
            if name == "notice" && !for_session {
                self.refuse("notice[] tells a page's reader something: call it in a handler, or in a function a handler calls".to_string());
            }
            if name == "download" && !for_session {
                self.refuse("download[] hands the page's reader a file: call it in a handler, or in a function a handler calls".to_string());
            }
            // a message is sent once, and a render or a derivation runs
            // whenever the page needs it
            if name == "mail" && matches!(self.at, Where::In(FnKind::View) | Where::Derive) {
                self.refuse(MAIL_IN_A_RENDER.to_string());
            }
            // signing in and out is a session's: a handler does it
            if matches!(name, "sign_in" | "sign_out") && !for_session {
                self.refuse(format!(
                    "{}[] signs the page's reader {}: call it in a handler, or in a function a handler calls",
                    name,
                    if name == "sign_in" { "in" } else { "out" }
                ));
            }
            // a file field binds a map: the runtime puts the file's name,
            // type, size and path there
            if name == "field" && argc == 4 && matches!(&args[2], A::Str(k) if k == "file") {
                let t = match arg_type(self, 1) {
                    CrowType::Result(inner) => *inner,
                    other => other,
                };
                if !matches!(t, CrowType::Map(_) | CrowType::Unknown) {
                    self.refuse(format!("a file field binds a map, which holds the file picked — declare it `name = map[]` (got {})", t.crow_name()));
                }
            }
            // the reader is the page's, as a module variable is the module's:
            // a plain function takes it as a parameter
            if name == "__reader" {
                if let Some(f) = self.in_fn() {
                    let live = self.ctx.views.contains(&f) || self.ctx.derives.contains(&f) || self.ctx.view_handlers.contains(&f);
                    if !live {
                        self.refuse(format!(
                            "in {}: reader is the page's signed-in reader — read it in a view, a derivation or a handler; a plain function takes it as a parameter: {}[reader]",
                            f, f
                        ));
                    }
                }
            }
            // the reader is a map of strings: it travels in a cookie, read
            // before the program runs
            if name == "sign_in" && argc == 1 {
                let t = match arg_type(self, 0) {
                    CrowType::Result(inner) => *inner,
                    other => other,
                };
                if !matches!(&t, CrowType::Map(inner) if matches!(inner.as_ref(), CrowType::Str | CrowType::Unknown)) && !matches!(t, CrowType::Unknown) {
                    self.refuse(format!(
                        "sign_in[] takes the reader as a map of strings — an id, a name, a role: sign_in[map[id = '42', name = 'Ada']] (got {})",
                        t.crow_name()
                    ));
                }
            }
            // a builtin with rows takes the counts and kinds its rows take —
            // the core's, and a loaded module's of the same name (math's
            // min of a tensor) — refused before either mode runs, in the
            // table's words (the interpreter stopped on the value, the
            // compiled program did not build)
            if crate::table::has_core_rows(name) {
                self.refuse_kinds(name, argc, types);
            } else if !builtin_arity_ok(name, argc) {
                self.refuse(format!("wrong number of arguments for '{}'", name));
            }
            return;
        }

        // User-defined function? (one the caller can call: inside a
        // standard module's function, inlined, a program's function is
        // invisible — a local of the module's named like one is indexed)
        if let Some(sig) = self.ctx.functions.get(name).filter(|_| self.calls_fn(name)) {
            let expected = sig.params.len();
            if argc != expected {
                self.refuse(format!("'{}' expects {} argument{}, got {}", name, expected, if expected == 1 { "" } else { "s" }, argc));
            }
            return;
        }

        // Struct constructor?
        if let Some(sig) = self.ctx.structs.get(name) {
            let expected = sig.fields.len();
            if argc != expected {
                self.refuse(format!(
                    "struct '{}' has {} field{}, got {} argument{}",
                    name,
                    expected,
                    if expected == 1 { "" } else { "s" },
                    argc,
                    if argc == 1 { "" } else { "s" }
                ));
            }
            return;
        }

        // A variable holding a function, a list or a map
        if self.defined(name) {
            if let Some(CrowType::Fn(params, _)) = self.ctx.vars.get(name).cloned() {
                let expected = params.len();
                if argc != expected {
                    self.refuse(format!("'{}' expects {} argument{}, got {}", name, expected, if expected == 1 { "" } else { "s" }, argc));
                }
                // a function that takes text is handed text — a feed's put
                // among them (the compiled program did not build otherwise)
                for (i, p) in params.iter().enumerate() {
                    let given = match arg_type(self, i) {
                        CrowType::Result(inner) => *inner,
                        other => other,
                    };
                    if matches!(p, CrowType::Str) && !matches!(given, CrowType::Str | CrowType::Markup | CrowType::Unknown | CrowType::Void) {
                        self.refuse(format!("{}[] takes text, and this is {}: write it into text, {}['`…`']", name, given.crow_name(), name));
                    }
                }
            }
            return;
        }

        // a stdlib module's function the program does not import says which;
        // a name near one in view says that one
        match crate::modules::ModuleRegistry::with_stdlib().module_name_of(name) {
            Some(m) => self.refuse(format!("{}[] is {}'s: add `use {}`", name, m, m)),
            None => {
                let near = self.nearest(name, BUILTINS.iter().copied().chain(self.v.used.iter().map(String::as_str)).chain(self.ctx.functions.keys().map(String::as_str)));
                self.refuse(format!("undefined function '{}'{}", name, near));
            }
        }
    }

    /// ` — did you mean 'len'?` for the name among `candidates` nearest the
    /// one written, when it is near enough to be a slip (one edit in a name
    /// of up to five characters, two in a longer one; none for a name of one
    /// or two); empty otherwise.
    fn nearest<'n>(&self, name: &str, candidates: impl Iterator<Item = &'n str>) -> String {
        let limit = match name.chars().count() {
            0..=2 => return String::new(),
            3..=5 => 1,
            _ => 2,
        };
        candidates
            .filter(|c| *c != name && !c.starts_with("__"))
            .map(|c| (crate::db_check::catalog::levenshtein(name, c), c))
            .filter(|(d, _)| *d <= limit)
            .min()
            .map(|(_, c)| format!(" — did you mean '{}'?", c))
            .unwrap_or_default()
    }
}

/// Whether a body returns a value somewhere.
fn returns_value(body: &[S]) -> bool {
    body.iter().any(|s| match s {
        S::Return(Some(_)) => true,
        S::If { then_body, else_body, .. } => returns_value(then_body) || else_body.as_ref().map_or(false, |b| returns_value(b)),
        S::For { body, .. } | S::While { body, .. } => returns_value(body),
        _ => false,
    })
}

/// Whether a builtin takes this many arguments (after named arguments are
/// lowered to positions); a builtin with rows is counted by them
/// (crate::table).
fn builtin_arity_ok(name: &str, argc: usize) -> bool {
    match name {
        "log" | "assert" => argc >= 1,
        "type" => argc == 1,
        "abs" | "sqrt" | "floor" | "ceil" | "round" => argc == 1,
        "now" => argc == 0,
        "captured" => argc == 1,
        "notice" => argc == 1,
        "__notice_text" | "__notice_at" => argc == 0,
        "sign_in" => argc == 1,
        "download" => argc == 2,
        "sign_out" | "__reader" => argc == 0,
        "__known" => argc == 1,
        "chart" => argc == 12,
        "append" => argc == 2,
        "map" => argc == 0,
        "args" => argc == 0,
        "input" => argc <= 1,
        "time" => argc == 0,
        "sleep" => argc == 1,
        "env" => argc == 1 || argc == 2,
        "exit" => argc <= 1,
        "error" => (1..=2).contains(&argc),
        "cpu" => argc == 1,
        "gpu" => argc <= 1,
        "nodes" => argc == 1,
        "grad" => argc == 1,
        // Shell / Hash / Regex / Date
        "shell" => argc == 1,
        // url, method and body, then headers, query, form, files, save and
        // timeout (the lowering fills in what a call leaves out)
        "request" => argc == 9,
        "find" | "any" | "all" => argc == 2,
        "date" => argc == 3,
        "elapsed" => argc == 2,
        // Web
        "route" => (2..=4).contains(&argc), // route[path, handler] / +method / path, method, handler, limit
        "serve" => argc == 1 || argc == 11,  // serve[port] (pre-lower) or serve[port, refresh, budget, theme, density, accent, wide, keep, uploads, cors, files]
        "respond" => argc == 2,
        "redirect" | "html" | "text" => argc == 1,
        "arrow" | "bytes" => argc == 1,
        "stream" => argc >= 1, // stream[feed, a, …] calls feed[put, a, …]
        "exec" | "query" => argc >= 2,
        "connect" => argc == 1,               // connect[url] (db)
        "socket" => argc == 1,                // socket[url]
        "send" => argc == 2,                  // send[socket, text]
        "recv" => argc == 1 || argc == 2,     // recv[socket] or recv[socket, secs]
        "close" => argc == 1,                 // close[socket]
        "page" => argc == 2 || argc == 3,     // page[path, render] or page[path, render, handle]
        "smtp" => argc == 4,                  // smtp[host, port, user, pass]
        "mail" => argc == 4,                  // mail[mailer, to, subject, body]
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
    }
}

/// The name a top-level statement declares, which module expansion prefixes
/// with its part's path.
pub(super) fn declared_name(stmt: &S) -> Option<String> {
    match stmt {
        S::FnDecl { name, .. } | S::Derive { name, .. } | S::StructDecl { name, .. } | S::Assign { name, .. } => Some(name.clone()),
        S::MultiAssign { names, .. } => names.first().cloned(),
        S::TestBlock { name, .. } => name.clone(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source checked as every pipeline checks it: named arguments
    /// lowered, signatures inferred, then lowered to the IR.
    fn check_source(source: &str) -> Result<(), String> {
        let stmts = crate::lower::lower_program(&crate::build(source)?)?;
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        super::super::check(&stmts, &ctx).map_err(|r| r.message())
    }

    #[test]
    fn test_valid_program() {
        let result = check_source("x = 10\ny = x + 5\nlog[y]\n");
        assert!(result.is_ok(), "valid program should pass: {:?}", result);
    }

    #[test]
    fn test_undefined_variable() {
        let result = check_source("log[x]\n");
        assert!(result.unwrap_err().contains("undefined variable 'x'"));
    }

    /// A function sees its parameters and nothing of the program's state:
    /// a derivation read inside one is refused with the way out
    #[test]
    fn a_function_reading_a_derivation_is_told_to_derive() {
        let err = check_source("use web, ui\nn = 1\nderive cfg\n    return n + 1\nfn band[]\n    return cfg * 2\nview render[]\n    return p['`band[]`']\npage['/', render]\n").unwrap_err();
        assert!(err.contains("in band: derivations are not visible inside functions — read 'cfg' in a view or handler"), "{}", err);
    }

    /// A list handed to a `?` is sent as (?, ?, …), which only `IN` reads:
    /// finch's `= ANY(?)` handed a list failed at the database every time
    #[test]
    fn a_list_parameter_goes_after_in() {
        let src = "use db\nbook = db.connect['sqlite://:memory:']\nsyms = ['A', 'B']\nr = query[book, 'SELECT 1 AS x WHERE ? = ANY(?)', ['A', syms]]\n";
        let err = check_source(src).unwrap_err();
        assert!(err.contains("query: parameter 2 is a list, which is sent as (?, ?, …) — only `IN ?` reads one: write `x IN ?`"), "{}", err);
        assert!(check_source(&src.replace("? = ANY(?)", "? IN ?")).is_ok());
        assert_eq!(crate::lower_ir::words_before_placeholders("a = ? AND b NOT IN ? AND c = ANY( ?) AND d = '?'"), vec!["", "IN", "ANY"]);
    }

    /// A web response is what a route returns: a view placing one is refused
    /// in both modes (the interpreter showed `text[s]` as its text; the
    /// compiled page did not build)
    #[test]
    fn a_view_does_not_place_a_web_response() {
        let err = check_source("use web, ui\nn = 3\nview v[]\n    return row[text['`n` items'], p['ok']]\npage['/', v]\n").unwrap_err();
        assert!(err.contains("row: text[…] is a web response, which a route returns — a page shows elements and text: place the text itself, or p[…] or span[…]"), "{}", err);
        assert!(check_source("use web, ui\nn = 3\nview v[]\n    return row['`n` items', p['ok']]\npage['/', v]\n").is_ok());
    }

    /// A mask decides every row: a list of values that can fail — cells of
    /// a join's unmatched side — is refused as one in both modes (compiled,
    /// it did not build; the interpreter stopped only when a cell was
    /// missing), and a filled column's cells make one
    #[test]
    fn a_mask_of_values_that_can_fail_is_refused() {
        let src = "use frame\na = with[with[frame.new[], 'k', [0, 1]], 'p', [1.5, 2.5]]\nb = with[with[frame.new[], 'k', [0]], 'q', [7.5]]\nj = join[a, b, 'k', 'left']\nkeep = for i in range[len[j]]\n    j.q[i] gt 1.0\nlog[len[filter[j, keep]]]\n";
        let err = check_source(src).unwrap_err();
        assert!(err.contains("filter: 'keep' holds values that can fail, and a mask decides every row: guard each value with .ok (cells of a column that can be missing: fill the column first, fill[frame, 'column', value])"), "{}", err);
        assert!(check_source(&src.replace("j = join[a, b, 'k', 'left']", "j = fill[join[a, b, 'k', 'left'], 'q', 0.0]")).is_ok());
    }

    /// The app is implicit: `web.new[]` and a route handed an app are
    /// refused, with the form to write
    #[test]
    fn the_app_is_implicit() {
        let err = check_source("use web\nsite = web.new[]\n").unwrap_err();
        assert!(err.contains("web.new[] is gone: the app is implicit"), "{}", err);
        let err = check_source("use web\nfn h[req]\n    return text['ok']\nroute[3, '/', h]\n").unwrap_err();
        assert!(err.contains("route[] takes a path first — the app is implicit: route['/', handler]"), "{}", err);
    }

    /// The web module's modules are web.auth, web.mail and web.ws: another
    /// — one the web audit removed, or none there ever was — is refused by
    /// name (a `use web.queue` had turned on a feature that no longer
    /// exists, and the compiled program did not build)
    #[test]
    fn a_web_module_there_is_not_is_refused() {
        let err = check_source("use web\nuse web.queue\nroute['/', ok]\nfn ok[req]\n    return text['ok']\n").unwrap_err();
        assert!(err.contains("web has no module 'queue' — its modules are web.auth, web.mail, web.ws"), "{}", err);
        assert!(check_source("use web, web.mail\nm = smtp['h', 25, 'u', 'p']\n").is_ok());
    }

    /// A function only tests call, handed a frame another function builds
    /// from a list, reads the frame's columns as the numbers they are: the
    /// tests are read again once the builder settles. (The frame's column
    /// was not worked out when the tests were first read, the parameter
    /// kept that, and its cells were refused as JSON: market's `resample`.)
    #[test]
    fn a_tested_function_reads_a_built_frames_columns() {
        let src = "use frame\nfn table[xs]\n    out = frame.new[]\n    out = with[out, 'x', xs]\n    return out\nfn build[v]\n    hi = []\n    for x in v\n        hi = push[hi, x]\n    return table[hi]\nfn again[f]\n    hi = []\n    for i in range[f.shape[0]]\n        k = len[hi]\n        if k eq 0\n            hi = push[hi, f.x[i]]\n        else\n            hi[k - 1] = max[hi[k - 1], f.x[i]]\n    return table[hi]\ntest t[]\n    f = build[[1.0, 2.0]]\n    g = again[f]\n    assert[g.x[0] eq 2.0]\n";
        let checked = check_source(src);
        assert!(checked.is_ok(), "{:?}", checked);
    }

    /// A cell of a column nothing typed is dynamic: arithmetic on it is
    /// refused until the program says what it is (typed frames, decision 1)
    #[test]
    fn a_cell_nothing_typed_is_read_as_one_first() {
        let err = check_source("use frame\ndf = load['nowhere.csv']\nlog[df.px[0] * 2.0]\n").unwrap_err();
        assert!(err.contains("cannot use '*' with json and float — a value whose type the program does not know is read as one first: int[…], float[…] or str[…]"), "{}", err);
        assert!(check_source("use frame\ndf = load['nowhere.csv']\nlog[float[df.px[0]] * 2.0]\n").is_ok());
    }

    /// A fact gets one value at the top level of a ui program: a loop that
    /// pushed into it or a second assignment reached the rest of the top
    /// level under `crow run` and no session, no test, and no declaration
    /// after it (`n = len[xs]` after the loop was 0 in both modes). A loop
    /// of the top level may bind its own names (the book's site does), and
    /// a program without ui may assign its variables again.
    #[test]
    fn a_fact_gets_one_value_at_the_top_level() {
        let err = check_source("use web, ui\nxs = []\nfor i in range[3]\n    xs = push[xs, i]\nview render[]\n    return p['`len[xs]`']\npage['/', render]\n").unwrap_err();
        assert!(err.contains("line 4: 'xs' is state, declared on line 2: it gets one value at the top level"), "{}", err);
        let err = check_source("use web, ui\ncount = 5\ncount = 6\nview render[]\n    return p['`count`']\npage['/', render]\n").unwrap_err();
        assert!(err.contains("line 3: 'count' is state, declared on line 2"), "{}", err);
        let err = check_source("use web, ui\ncfg = map[a = 1]\ncfg['a'] = 2\nview render[]\n    return p['`cfg.a`']\npage['/', render]\n").unwrap_err();
        assert!(err.contains("'cfg' is state, declared on line 2"), "{}", err);
        let ok = check_source("use web, ui\nxs = [1, 2]\nfor x in xs\n    y = x * 2\n    log[y]\nview render[]\n    return p['`len[xs]`']\npage['/', render]\n");
        assert!(ok.is_ok(), "{:?}", ok);
        let ok = check_source("count = 5\ncount = 6\nlog[count]\n");
        assert!(ok.is_ok(), "{:?}", ok);
    }

    /// A derivation's locals are its own, as a function's are: two
    /// derivations may each bind `out` to a different type (finch's session
    /// part: the universe's symbols beside its dataset ids), while one
    /// derivation still cannot change a local's type
    #[test]
    fn two_derivations_keep_their_own_locals() {
        let result = check_source("use web, ui\nn = 3\nderive names\n    out = for i in range[n]\n        'a`i`'\n    return out\nderive ids\n    out = for i in range[n]\n        i\n    return out\nview render[]\n    return p['`len[names]` `len[ids]`']\npage['/', render]\n");
        assert!(result.is_ok(), "{:?}", result);
        let err = check_source("use web, ui\nderive d\n    x = 1\n    x = 'a'\n    return x\nview render[]\n    return p['`d`']\npage['/', render]\n").unwrap_err();
        assert!(err.contains("cannot reassign 'x' from int to str"), "{}", err);
    }

    /// A tensor's shape the program states — a literal, a constructor of
    /// literal sizes — is known before it runs, through the element-wise
    /// operators and `*`: two known dimensions that cannot meet are refused
    /// then; a dimension the program computes is left to run time.
    #[test]
    fn shapes_the_program_states_meet_before_it_runs() {
        let err = check_source("use math\na = [[1.0, 2.0], [3.0, 4.0]]\nb = zeros[2, 3]\nlog[a + b]\n").unwrap_err();
        assert!(err.contains("`+` of tensors of shapes [2, 2] and [2, 3]: dimension 2 is 2 in one and 3 in the other"), "{}", err);
        let err = check_source("use math\nlog[zeros[2, 2] * zeros[3, 3]]\n").unwrap_err();
        assert!(err.contains("matmul: incompatible shapes [2, 2] × [3, 3] — the first's columns (2) must be the second's rows (3)"), "{}", err);
        let err = check_source("use math\nlog[[1.0, 2.0] * eye[2]]\n").unwrap_err();
        assert!(err.contains("needs two 2-D tensors, got shapes [2] and [2, 2]"), "{}", err);
        // a shape kept through a local and a transpose
        let err = check_source("use math\nfn f[]\n    a = zeros[2, 3]\n    b = transpose[a]\n    return a .* b\nlog[f[]]\n").unwrap_err();
        assert!(err.contains("`.*` of tensors of shapes [2, 3] and [3, 2]"), "{}", err);
        // broadcasting, a scalar, a computed dimension, a reshaped local: run time's
        assert!(check_source("use math\nlog[zeros[2, 3] + ones[3], zeros[2, 3] * 2.0, eye[3] * zeros[3, 4]]\n").is_ok());
        assert!(check_source("use math\nn = 4\nlog[zeros[n, 3] + zeros[5, 3]]\n").is_ok());
        assert!(check_source("use math\nfn f[k]\n    a = zeros[2, 3]\n    if k gt 1\n        a = zeros[3, 3]\n    return a + zeros[3, 3]\nlog[f[2]]\n").is_ok());
    }

    /// A shortcut is a literal the kernel can name, on an element a press
    /// does something to: a click, a button (its form's submit), a link.
    #[test]
    fn a_shortcut_presses_something() {
        let page = |element: &str| format!("use web, ui\nn = 0\nfn save[]\n    log['saved']\nview v[]\n    return stack[p['`n`'], {}]\npage['/', v]\n", element);
        assert!(check_source(&page("button['Save', click = save, shortcut = 'ctrl+s']")).is_ok());
        assert!(check_source(&page("a['Home', href = '/', shortcut = 'g']")).is_ok());
        assert!(check_source(&page("div['row', click = save, shortcut = 'enter']")).is_ok());
        let err = check_source(&page("p['Save', shortcut = 'ctrl+s']")).unwrap_err();
        assert!(err.contains("p: shortcut = … presses the element, and pressing a p does nothing: give it click = a handler"), "{}", err);
        let err = check_source(&page("button['Save', click = save, shortcut = 'hyper+s']")).unwrap_err();
        assert!(err.contains("button: shortcut = 'hyper+s' is not a shortcut ('ctrl+s', 'escape', '?', 'ctrl+s meta+s')"), "{}", err);
        let err = check_source(&page("button['Save', click = save, shortcut = str[n]]")).unwrap_err();
        assert!(err.contains("button: 'shortcut' is a layout attribute and must be a literal"), "{}", err);
    }

    /// An image says what it shows to a reader who cannot see it, or that
    /// it only decorates (spec, Accessibility)
    #[test]
    fn an_image_says_what_it_shows() {
        let page = |element: &str| format!("use web, ui\nview v[]\n    return stack[{}]\npage['/', v]\n", element);
        assert!(check_source(&page("img[src = '/logo.png', alt = 'the logo']")).is_ok());
        assert!(check_source(&page("img[src = '/rule.png', alt = '']")).is_ok());
        let err = check_source(&page("img[src = '/logo.png']")).unwrap_err();
        assert!(err.contains("img: alt = '…' says what the image shows to a reader who cannot see it, or alt = '' that it only decorates"), "{}", err);
    }

    /// A table sorts by its headers when the program says so in so many
    /// words: `sort = true`, never a value that would sort in one mode.
    #[test]
    fn a_tables_sort_is_written_true() {
        let page = |table: &str| format!("use web, ui, frame\nlong = true\nview v[]\n    f = with[frame.new[], 'n', [2, 1]]\n    return stack[p['`long`'], {}]\npage['/', v]\n", table);
        assert!(check_source(&page("table[f, sort = true]")).is_ok());
        assert!(check_source(&page("table[f, sort = false]")).is_ok());
        let err = check_source(&page("table[f, sort = long]")).unwrap_err();
        assert!(err.contains("table: sort takes true or false as written: sort = true makes each header sort the rows by its column"), "{}", err);
    }

    /// A tensor's shape is in its type (second-half-design.md §3), so it
    /// reaches the function it is handed to and the caller it is returned
    /// to; a function's parameter has the dimensions its calls agree on (a
    /// function called with two shapes is refused nothing either could
    /// meet); `transpose` swaps the last two axes (it had reversed them all,
    /// and a rank-3 tensor's valid sum was refused); `reshape` states its
    /// dimensions; and a loop keeps what its rounds agree on.
    #[test]
    fn shapes_are_in_the_type() {
        let err = check_source("use math\nfn add[a, b]\n    return a + b\nlog[add[zeros[2, 3], zeros[3, 2]]]\n").unwrap_err();
        assert!(err.contains("`+` of tensors of shapes [2, 3] and [3, 2]: dimension 1 is 2 in one and 3 in the other — an element-wise operation needs each pair equal, or one of them 1 (the tensors add[] is handed as `a` and `b`)"), "{}", err);
        assert!(check_source("use math\nfn add[a, b]\n    return a + b\nlog[add[zeros[2, 3], zeros[2, 3]], add[zeros[3, 3], zeros[3, 3]]]\n").is_ok());
        let err = check_source("use math\nfn make[]\n    return zeros[2, 3]\nm = make[]\nlog[m * zeros[2, 2]]\n").unwrap_err();
        assert!(err.contains("matmul: incompatible shapes [2, 3] × [2, 2]"), "{}", err);
        assert!(check_source("use math\na = zeros[2, 3, 4]\nt = transpose[a]\nlog[t + zeros[2, 4, 3]]\n").is_ok());
        let err = check_source("use math\nlog[reshape[zeros[6], 2, 3] + zeros[3, 2]]\n").unwrap_err();
        assert!(err.contains("`+` of tensors of shapes [2, 3] and [3, 2]"), "{}", err);
        let err = check_source("use math\nx = zeros[2, 2]\nfor i in range[3]\n    x = x + 1.0\nlog[x * zeros[3, 3]]\n").unwrap_err();
        assert!(err.contains("matmul: incompatible shapes [2, 2] × [3, 3]"), "{}", err);
        let err = check_source("use math\nx = zeros[2, 3]\nx = zeros[3, 3]\nlog[x + zeros[2, 3]]\n").unwrap_err();
        assert!(err.contains("`+` of tensors of shapes [3, 3] and [2, 3]"), "the local bound anew: {}", err);
    }

    /// The checks read the lowering's types, a pure function's locals
    /// among them (they used to read the signatures, where a pure
    /// function's locals were not known): a local keeps its first type, and
    /// a value of a type the program does not know is read as one first.
    #[test]
    fn a_pure_functions_local_keeps_its_type() {
        let err = check_source("fn f[]\n    x = 1\n    x = 'a'\n    return x\nlog[f[]]\n").unwrap_err();
        assert!(err.contains("cannot reassign 'x' from int to str — variable type cannot change"), "{}", err);
        let err = check_source("fn f[s]\n    x = ''\n    x = json[s]\n    return x\nlog[f['1']]\n").unwrap_err();
        assert!(err.contains("cannot reassign 'x' from str to result[json] — a value whose type the program does not know is read as one first"), "{}", err);
        assert!(check_source("fn f[s]\n    x = ''\n    x = str[json[s]]\n    return x\nlog[f['1']]\n").is_ok());
    }

    /// Each error keeps the top-level declaration it was found in: after
    /// module expansion that name carries the part's prefix, which is how
    /// the gate names the part's file
    #[test]
    fn an_error_is_owned_by_its_declaration() {
        let src = "use web, ui\nn = 1\nderive d\n    x = 1\n    x = 'a'\n    return x\nview render[]\n    return p['`d` `n`']\npage['/', render]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let Err(Refusal::Invalid(owned)) = super::super::check(&stmts, &ctx) else { panic!("refused as invalid") };
        assert_eq!(owned.len(), 1, "{:?}", owned);
        assert!(owned[0].0.contains("cannot reassign 'x'"), "{:?}", owned);
        assert_eq!(owned[0].1.as_deref(), Some("d"));
    }

    #[test]
    fn test_undefined_function() {
        assert!(check_source("y = foo[1, 2]\n").unwrap_err().contains("undefined function 'foo'"));
    }

    #[test]
    fn test_wrong_arg_count_builtin() {
        assert!(check_source("x = min[1]\n").unwrap_err().contains("min[] takes 2 arguments, got 1"));
        // a kind no row takes is refused before either mode runs
        assert!(check_source("x = min['a', 'b']\n").unwrap_err().contains("min[] takes an int and an int or a number and a number, got text and text"));
        assert!(check_source("x = trim[3]\n").unwrap_err().contains("trim[] takes text, got an int"));
    }

    #[test]
    fn test_wrong_arg_count_user_fn() {
        // Lowering reports missing arguments before the arity check
        assert!(check_source("fn add[a, b]\n  return a + b\ny = add[1]\n").unwrap_err().contains("missing argument 'b'"));
    }

    #[test]
    fn test_arithmetic_type_mismatch() {
        assert!(check_source("x = true - 5\n").unwrap_err().contains("cannot use '-'"));
    }

    #[test]
    fn test_string_concat_rejected() {
        assert!(check_source("x = 'hello' + ' world'\n").unwrap_err().contains("join"), "should suggest interpolation and join");
    }

    #[test]
    fn test_struct_bad_field() {
        assert!(check_source("struct Point[x, y]\np = Point[1, 2]\nlog[p.z]\n").unwrap_err().contains("has no field 'z'"));
        let err = check_source("use math\nxs = for i in range[2]\n    zeros[2]\nlog[xs.shape]\nlog[xs.len]\n").unwrap_err();
        assert!(err.contains("a list has no field 'shape': `.shape` is a tensor's") && !err.contains("'len'"), "{}", err);
    }

    #[test]
    fn test_struct_wrong_arg_count() {
        // Lowering reports missing arguments before the arity check
        assert!(check_source("struct Point[x, y]\np = Point[1]\n").unwrap_err().contains("missing argument 'y'"));
    }

    #[test]
    fn test_for_loop_var_defined() {
        let result = check_source("for i in range[5]\n  log[i]\n");
        assert!(result.is_ok(), "for loop var should be defined: {:?}", result);
    }

    #[test]
    fn test_grad_valid() {
        let result = check_source("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(result.is_ok(), "grad[f] should be valid: {:?}", result);
    }

    #[test]
    fn test_grad_wrong_arg_count() {
        assert!(check_source("fn f[x]\n  x * x\nfn g[x]\n  x + 1\ndf = grad[f, g]\n").is_err(), "grad should require exactly 1 arg");
    }

    #[test]
    fn test_fn_module_var_read_rejected_with_hint() {
        assert!(check_source("base = 10\nfn f[x]\n    return x + base\nr = f[1]\n").unwrap_err().contains("pass 'base' as a parameter"));
    }

    #[test]
    fn test_fn_local_reusing_module_name_ok() {
        let result = check_source("count = 5\nfn f[n]\n    count = n * 2\n    return count\nr = f[3]\n");
        assert!(result.is_ok(), "local reusing module name should validate: {:?}", result);
    }

    #[test]
    fn test_map_hetero_set_rejected_at_compile_time() {
        // Maps are dictionaries: set[] changing value type is refused
        assert!(check_source("m = map[a = 1]\nm2 = set[m, 'b', 'x']\n").unwrap_err().contains("map values must all be the same type"));
    }

    /// A collection builtin given a kind none of its rows takes is refused
    /// before either mode runs, naming the forms it takes (compiled, these
    /// did not build; interpreted, they stopped as they ran).
    #[test]
    fn a_collection_given_a_wrong_kind_is_refused() {
        assert!(check_source("log[has[[1, 2], 1]]\n").unwrap_err().contains("has[] takes a map and text, got numbers and an int"));
        assert!(check_source("log[len[5]]\n").unwrap_err().contains("len[] takes a list, text, a map, numbers, a frame or bytes, got an int"));
        let sorted = check_source("struct P [x, y]\nlog[sort[[P[3, 4], P[1, 2]]]]\n").unwrap_err();
        assert!(sorted.contains("sort[] takes a list of ints, a list of numbers, a list of text or numbers, got a list of records"), "{}", sorted);
        assert!(check_source("log[range[2.5]]\n").unwrap_err().contains("range[] takes an int, got a number"));
        assert!(check_source("m = map[a = 1]\nlog[contains[m, 'a']]\n").is_err());
    }

    /// A parsed JSON value is read as one type before a builtin computes
    /// with it, in both modes; the builtins that navigate one take it, and a
    /// loop walks a parsed list. A module function given a kind none of its
    /// rows take is refused as a core builtin is (the interpreter stopped on
    /// the value, the compiled program did not build).
    #[test]
    fn a_parsed_value_is_read_before_a_builtin_computes_with_it() {
        let err = check_source("j = json['\"a\"']\nlog[upper[j]]\n").unwrap_err();
        assert!(err.contains("upper[] takes text, got a JSON value — a value whose type the program does not know is read as one first: int[…], float[…] or str[…], a list item by item (for x in …)"), "{}", err);
        assert!(check_source("j = json['\"a\"']\nlog[upper[str[j]]]\nlog[len[j]]\n").is_ok());
        assert!(check_source("j = json['[1, 2]']\nfor x in j\n    log[float[x]]\n").is_ok());
        assert!(check_source("use math\nlog[mean['a']]\n").unwrap_err().contains("mean[] takes numbers, got text"));
        assert!(check_source("use math\nlog[math.mean['a']]\n").unwrap_err().contains("mean[] takes numbers, got text"));
        // a name the core answers by a form of its own takes what it takes
        assert!(check_source("use math\nlog[math.sqrt[16.0]]\n").is_ok());
    }

    #[test]
    fn test_frame_bare_column_name_rejected() {
        // Regression: bare-name column DSL removed — undefined idents in frame
        // calls are normal undefined-variable errors now
        assert!(check_source("use frame\ndf = load['d.csv']\ns = sort[df, age]\n").unwrap_err().contains("undefined variable 'age'"));
    }

    // ── what checking in the lowering changed (core-rewrite-notes §10) ──

    /// A name read inside an interpolation is read like any other: an
    /// undefined one is refused before the program runs, not when the
    /// string is built (or, compiled, when Rust meets it)
    #[test]
    fn a_name_read_in_an_interpolation_is_checked() {
        let err = check_source("log['total: `totl`']\n").unwrap_err();
        assert!(err.contains("line 1: undefined variable 'totl'"), "{}", err);
        let err = check_source("amount = 3\nfn show[]\n    return 'amount is `amount`'\nlog[show[]]\n").unwrap_err();
        assert!(err.contains("in show: module variables are not visible inside functions — pass 'amount'"), "{}", err);
    }

    /// A route's handler is handed its request alone, so the advice for a
    /// module variable it reads is to open what it needs inside it — not a
    /// parameter no route can pass, nor wiring it is already given
    #[test]
    fn a_route_handler_reading_a_module_variable_is_told_to_open_it_itself() {
        let src = "use web, db\nconn = connect['sqlite://app.db']\nfn count[req]\n    q = query[conn, 'SELECT 1 AS one']\n    return text['one']\nroute['/count', count]\nroute['/n', 'GET', count, 3]\nserve[3000]\n";
        let err = check_source(src).unwrap_err();
        assert!(err.contains("in count: module variables are not visible inside functions — a route's handler is handed its request alone: open what it needs inside it"), "{}", err);
        assert!(!err.contains("wire the function as a handler"), "{}", err);
        let ok = "use web, db\nfn count[req]\n    conn = connect['sqlite://app.db']\n    q = query[conn, 'SELECT 1 AS one']\n    return text['one']\nroute['/count', count]\nserve[3000]\n";
        assert!(check_source(ok).is_ok(), "{:?}", check_source(ok));
    }

    /// A function handed in as a parameter is called with the arguments
    /// the function it holds takes: the body is typed with the parameter's
    /// type, which the program-wide pass could not see
    #[test]
    fn a_function_held_in_a_parameter_is_called_with_its_arity() {
        let err = check_source("fn cube[x]\n    return x * x * x\nfn apply[f, x]\n    return f[x, 1]\nlog[apply[cube, 2]]\n").unwrap_err();
        assert!(err.contains("line 4: 'f' expects 1 argument, got 2"), "{}", err);
    }

    /// A field read from a function's own record is checked against the
    /// record's fields
    #[test]
    fn a_field_read_inside_a_function_is_checked() {
        let err = check_source("struct P [x, y]\nfn f[n]\n    p = P[n, n]\n    return p.z\nlog[f[1]]\n").unwrap_err();
        assert!(err.contains("line 4: struct 'P' has no field 'z'"), "{}", err);
    }

    /// An error inside a function is not blamed on its callers: a function
    /// whose returned value's type is not known is of an unknown type where
    /// it is called, not of none
    #[test]
    fn an_undefined_return_is_not_blamed_on_the_caller() {
        let err = check_source("fn fib[n]\n    if n lte 1\n        return nope\n    return fib[n - 1] + fib[n - 2]\nlog[fib[5]]\n").unwrap_err();
        assert!(err.contains("line 3: undefined variable 'nope'"), "{}", err);
        assert!(!err.contains("cannot use '+'"), "{}", err);
    }

    /// A harness call's arguments are checked once, as any call's are
    #[test]
    fn a_harness_calls_arguments_are_checked_once() {
        let err = check_source("use web, ui\nn = 0\nfn go[]\n    n = n + 1\nview v[]\n    return button['go', click = go]\npage['/', v]\ntest t[]\n    a = app[v]\n    click[a, nope]\n").unwrap_err();
        assert_eq!(err.matches("undefined variable 'nope'").count(), 1, "{}", err);
    }

    /// A module's constant read through its alias is the constant, not an
    /// undefined variable
    #[test]
    fn an_aliased_modules_constant_is_read() {
        let result = check_source("use math as m\nlog[m.pi]\n");
        assert!(result.is_ok(), "{:?}", result);
    }

    /// mail takes a mailer, to, subject and body; what a session sent is a
    /// test's to read, and only a test's
    #[test]
    fn mail_is_checked_where_it_is_written() {
        let err = check_source("use web\nuse web.mail\nm = smtp['h', 587, 'u', 'p']\nmail[m, 'a@b.c', 'Hello']\n").unwrap_err();
        assert!(err.contains("mail"), "{}", err);
        let err = check_source("use web, ui\nuse web.mail\nn = 0\nview v[]\n    return p['`n`']\nlog[mails[v]]\n").unwrap_err();
        assert!(err.contains("'mails' is a test-harness builtin"), "{}", err);
        let result = check_source("use web\nuse web.mail\nm = smtp['h', 587, 'u', 'p']\nr = mail[m, 'a@b.c', 'Hello', 'Welcome']\nlog[r.ok]\n");
        assert!(result.is_ok(), "{:?}", result);
        // a render or a derivation runs whenever the page needs it: it
        // sends nothing, itself or through a function it calls
        let ui = "use web, ui\nuse web.mail\nmailer = smtp['h', 587, 'u', 'p']\nn = 0\n";
        let err = check_source(&format!("{}view v[]\n    mail[mailer, 'a@b.c', 'x', 'y']\n    return p['`n`']\n", ui)).unwrap_err();
        assert!(err.contains("line 6: mail[] sends a message, and a view or a derivation"), "{}", err);
        let err = check_source(&format!("{}derive d\n    r = mail[mailer, 'a@b.c', 'x', 'y']\n    return r.ok\nview v[]\n    return p['`d`']\n", ui)).unwrap_err();
        assert!(err.contains("line 6: mail[] sends a message"), "{}", err);
        let err = check_source(&format!("{}fn tell[m]\n    mail[m, 'a@b.c', 'x', 'y']\n    return 1\nview v[]\n    k = tell[mailer]\n    return p['`k`']\n", ui)).unwrap_err();
        assert!(err.contains("in v: calls tell, which sends mail"), "{}", err);
    }

    /// A cell of a column no database described is text only as a guess:
    /// assigning one to a number's variable is not refused as a type change
    #[test]
    fn an_undescribed_cell_is_not_a_type_change() {
        let result = check_source("use db\nconn = db.connect['sqlite://:memory:']\nseen = 0\nfn count[c]\n    r = query[c, 'SELECT count(*) AS n FROM t']\n    seen = r.n[0]\n    return seen\nlog[count[conn]]\n");
        assert!(result.is_ok(), "{:?}", result);
    }
}

/// A side of an order, by its type: one value of a kind, or a list's (or a
/// column's) items of a kind — none where the kind shows only as the
/// program runs.
enum OrderSide {
    One(&'static str),
    Many(&'static str),
}

/// What a list, a tensor or a frame's column holds, compared whole: a
/// vector's numbers, a matrix's rows (a list of rows is one), a column's
/// cells — none where a tensor's rank, or the items' kind, shows only as
/// the program runs.
fn whole_items(t: &Ty) -> Option<&'static str> {
    match t.payload() {
        Ty::Tensor(_, Some(dims)) => Some(if dims.len() > 1 { "a list" } else { "a number" }),
        Ty::List(item) | Ty::Column(item) => match item.payload() {
            Ty::Tensor(..) | Ty::List(_) => Some("a list"),
            other => match order_side(other)? {
                OrderSide::One(k) => Some(k),
                OrderSide::Many(_) => Some("a list"),
            },
        },
        _ => None,
    }
}

/// A list's items, as a reason names them.
fn many_words(item: &str) -> String {
    match item {
        "a number" => "a list of numbers".to_string(),
        "text" => "a list of text".to_string(),
        "a bool" => "a list of bools".to_string(),
        "a list" => "a list of lists".to_string(),
        other => format!("a list of {}s", other.trim_start_matches("a ")),
    }
}

/// An item's kind, as a collection's items and as one item: numbers and a
/// number, text, bools and a bool, a struct's records and one of them,
/// lists, maps — none where it shows only as the program runs.
fn item_kind(t: &Ty) -> Option<(String, String)> {
    Some(match t.payload() {
        Ty::Int | Ty::Float => ("numbers".to_string(), "a number".to_string()),
        Ty::Str | Ty::Markup => ("text".to_string(), "text".to_string()),
        Ty::Bool => ("bools".to_string(), "a bool".to_string()),
        Ty::Record(n) | Ty::OptRecord(n) => (format!("{} records", crate::lower::shown(n)), format!("a {} record", crate::lower::shown(n))),
        Ty::List(_) | Ty::Tensor(..) => ("lists".to_string(), "a list".to_string()),
        Ty::Map(_) => ("maps".to_string(), "a map".to_string()),
        Ty::Frame(_) => ("frames".to_string(), "a frame".to_string()),
        _ => return None,
    })
}

fn order_side(t: &Ty) -> Option<OrderSide> {
    Some(match t {
        Ty::Int | Ty::Float => OrderSide::One("a number"),
        Ty::Str | Ty::Markup => OrderSide::One("text"),
        Ty::Bool => OrderSide::One("a bool"),
        Ty::Result(x) => return order_side(x),
        Ty::Tensor(..) => OrderSide::Many("a number"),
        Ty::List(item) | Ty::Column(item) => match order_side(item)? {
            OrderSide::One(k) => OrderSide::Many(k),
            OrderSide::Many(_) => OrderSide::Many("a list"),
        },
        Ty::Map(_) => OrderSide::One("a map"),
        Ty::Record(_) | Ty::OptRecord(_) => OrderSide::One("a record"),
        Ty::Frame(_) => OrderSide::One("a frame"),
        Ty::Node => OrderSide::One("markup"),
        _ => return None,
    })
}
