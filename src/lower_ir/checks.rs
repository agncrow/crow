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
    "rand", "time", "sleep", "env", "exit",
    "json", "encode",
    "error",
    "shell",
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
    "sqlite", "sign", "verify",
    "app", "click", "elements", "change", "box", "fits", "style",
    "cors", "sessions", "templates", "cache", "limit",
    "uploads", "sse", "ws", "connect", "send", "recv", "close",
    "page", "static_dir", "smtp", "mail", "mails", "msgpack", "arrow", "bytes", "stream",
    "reader", "sign_in", "sign_out", "download", "downloads",
];

/// Web builtins the compiler implements but deliberately does NOT reserve
/// (`render` is free for views; the response encoders are rarely-collided
/// names). Listed so `crow test --docs` can hold the spec to them.
pub(crate) const WEB_EXTRA_FUNCTIONS: &[&str] = &["msgpack", "arrow", "bytes", "stream", "render"];

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
    "later", "running", "notice", "__notice_text", "__notice_at",
    "sign_in", "sign_out", "__reader", "__known", "download",
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
    "page", "static_dir", "smtp", "mail",
    // UI reactive builtins
    "fetch", "poll", "field", "slider", "checkbox", "toggle", "radio", "tabs", "select", "progress", "icon", "today",
];

/// The modules whose functions and constants a bare `use` makes names.
const REGISTRY_MODULES: &[&str] = &["math", "math.stats", "math.nn", "io", "frame", "doc", "chart"];

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
    /// The program's types as its signatures were inferred, before any
    /// body was lowered: what a check comparing types reads (`seen_type`).
    pub types: Option<TypeContext>,
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

    /// A value's type as a check comparing types reads it: as the
    /// program's signatures were inferred, where a function's parameters
    /// and locals are not yet known. The lowering types a body more
    /// precisely, but some of what it knows is a guess — a cell of a column
    /// no database described is text until the program runs, and a
    /// parameter given one is typed by it — and a guess must not refuse a
    /// program. When frames are typed (ir-design.md §8) the checks read the
    /// lowering's types.
    ///
    /// A call to a function whose returned value's type is not known yet is
    /// of an unknown type, not of none (inference types such a function as
    /// returning nothing until it knows), so an error in the function is not
    /// blamed on its callers too.
    pub(super) fn seen_type(&self, e: &A) -> CrowType {
        if let A::Call { callee, .. } = e {
            if let A::Ident(f) = callee.as_ref() {
                let returns = self.ctx.functions.get(f).map_or(false, |sig| returns_value(&sig.body));
                if returns && matches!(self.ctx.functions.get(f).map(|s| &s.return_type), Some(CrowType::Void)) {
                    return CrowType::Unknown;
                }
            }
        }
        match &self.v.types {
            Some(types) => types.infer_expr(e),
            None => CrowType::Unknown,
        }
    }

    /// An assignment's target, before the value it is given: its variable
    /// is read, and each field it names must be the record's.
    pub(super) fn check_target(&mut self, object: &A) {
        match object {
            A::Ident(n) => self.check_ident(n),
            A::FieldAccess { object, field } => {
                self.check_target(object);
                let object_type = self.ctx.infer_expr(object);
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
        } else if in_fn.is_some() && self.v.module_vars.contains(name) {
            self.refuse(format!(
                "{}module variables are not visible inside functions — pass '{}' as a parameter, or wire the function as a handler",
                prefix, name
            ));
        } else {
            self.refuse(format!("undefined variable '{}'", name));
        }
    }

    /// Reservation: a user definition (variable, fn, struct, param, loop var)
    /// may not take a name the language or an imported module owns.
    /// (Spec Design Note "Names Are Never Ambiguous".)
    pub(super) fn check_reserved(&mut self, name: &str, what: &str) {
        if CORE_RESERVED.contains(&name) {
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
            self.refuse(format!(
                "cannot reassign '{}' from {} to {} — variable type cannot change",
                name,
                existing_ty.crow_name(),
                new_ty.crow_name()
            ));
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
        // Numeric ops: both must be numeric (including NumArray, Sparse,
        // FrameCol, and legacy Array(Str))
        let is_num_like = |t: &CrowType| {
            matches!(t, CrowType::Int | CrowType::Float | CrowType::NumArray(_) | CrowType::Sparse | CrowType::FrameCol)
                || matches!(t, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str))
        };
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

    /// A field read from a record.
    pub(super) fn check_field(&mut self, obj_type: &CrowType, field: &str) {
        if matches!(field, "ok" | "reason" | "kind" | "trace") {
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
        let registry_module = REGISTRY_MODULES.contains(&real.as_str()) || real == "db";
        let registry_module = registry_module && self.ctx.registry.has_function(field);
        if !registry_module && !self.ctx.functions.contains_key(&prefixed) && !self.ctx.structs.contains_key(&prefixed) && !self.defined(module) {
            self.refuse(format!("undefined module or variable '{}'", module));
        }
    }

    /// A call by name, its arguments already lowered and checked.
    pub(super) fn check_call(&mut self, name: &str, args: &[A]) {
        let arg_type = |l: &Self, i: usize| -> CrowType { args.get(i).map(|a| l.seen_type(a)).unwrap_or(CrowType::Unknown) };
        // Test-harness builtins exist only inside test blocks (which never
        // reach a build) — accept them there, refuse them anywhere else.
        if matches!(name, "app" | "click" | "elements" | "change" | "box" | "fits" | "style" | "downloads" | "mails") {
            if self.at != Where::Test {
                self.refuse(format!("'{}' is a test-harness builtin — it only works inside a test block", name));
            }
            return;
        }
        let argc = args.len();

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
        if BUILTINS.contains(&name) {
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
            if !builtin_arity_ok(name, argc) {
                self.refuse(format!("wrong number of arguments for '{}'", name));
            }
            return;
        }

        // User-defined function?
        if let Some(sig) = self.ctx.functions.get(name) {
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
            if let Some(CrowType::Fn(params, _)) = self.ctx.vars.get(name) {
                let expected = params.len();
                if argc != expected {
                    self.refuse(format!("'{}' expects {} argument{}, got {}", name, expected, if expected == 1 { "" } else { "s" }, argc));
                }
            }
            return;
        }

        self.refuse(format!("undefined function '{}'", name));
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
/// lowered to positions).
fn builtin_arity_ok(name: &str, argc: usize) -> bool {
    match name {
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
        "notice" => argc == 1,
        "__notice_text" | "__notice_at" => argc == 0,
        "sign_in" => argc == 1,
        "download" => argc == 2,
        "sign_out" | "__reader" => argc == 0,
        "__known" => argc == 1,
        "chart" => argc == 12,
        "push" | "append" => argc == 2,
        "pop" | "sort" | "reverse" => argc == 1,
        "range" => (1..=3).contains(&argc),
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
        "error" => (1..=2).contains(&argc),
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
        "serve" => argc == 1 || argc == 9,   // serve[port] (pre-lower) or serve[port, refresh, budget, theme, density, accent, wide, keep, uploads]
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
        "page" => argc == 2 || argc == 3,     // page[path, render] or page[path, render, handle]
        "static_dir" => argc == 3,            // static_dir[app, url_path, dir_path]
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

    /// A mask decides every row: a list of values that can fail — cells of
    /// a join's unmatched side — is refused as one in both modes (compiled,
    /// it did not build; the interpreter stopped only when a cell was
    /// missing), and a filled column's cells make one
    #[test]
    fn a_mask_of_values_that_can_fail_is_refused() {
        let src = "use frame\na = with[with[frame.new[], 'k', [0, 1]], 'p', [1.5, 2.5]]\nb = with[with[frame.new[], 'k', [0]], 'q', [7.5]]\nj = join[a, b, 'k', 'left']\nkeep = for i in range[len[j]]\n    j.q[i] gt 1.0\nlog[len[filter[j, keep]]]\n";
        let err = check_source(src).unwrap_err();
        assert!(err.contains("filter: 'keep' holds values that can fail — a cell of a column that can be missing — and a mask decides every row: fill the column first (fill[frame, 'column', value]) or guard each value with .ok"), "{}", err);
        assert!(check_source(&src.replace("j = join[a, b, 'k', 'left']", "j = fill[join[a, b, 'k', 'left'], 'q', 0.0]")).is_ok());
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
        assert!(check_source("x = min[1]\n").unwrap_err().contains("wrong number of arguments for 'min'"));
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
