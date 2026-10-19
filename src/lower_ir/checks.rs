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
/// rule). Names here can never be user definitions: every core row's and
/// form's name (table::ROWS, table::FORMS) and the test block's word,
/// derived from the table, not written beside it (a builtin added as a
/// row had to be listed here too, and the list and the table drifted:
/// `bucket` reached one before the other).
pub(crate) static CORE_RESERVED: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| reserved_by(|m| m.is_empty(), &["test"]));

/// The names a module's bare import reserves: its rows' and forms', the
/// hidden ones apart, and those its control shares with another module
/// (`Prop::Unreserved`).
fn reserved_by(module: impl std::ops::Fn(&str) -> bool, more: &[&'static str]) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = crate::table::ROWS
        .iter()
        .chain(crate::table::FORMS.iter())
        .filter(|r| module(r.module) && !r.name.starts_with("__") && !r.props.contains(&crate::table::Prop::Unreserved))
        .map(|r| r.name)
        .collect();
    names.extend(more.iter().copied());
    names.sort_unstable();
    names.dedup();
    names
}

/// Why `mail` is refused in a view or a derivation, directly or through a
/// function it calls.
pub(super) const MAIL_IN_A_RENDER: &str = "mail[] sends a message, and a view or a derivation runs whenever the page needs it: send it from a handler, or a function a handler calls";

/// Reserved only in files that import web (use web / use web.*): web's
/// rows and forms, and its modules' (auth, ws, mail).
pub(crate) static WEB_RESERVED: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| reserved_by(|m| m == "web" || m.starts_with("web."), &[]));


/// The web module's modules, `use web.auth` and the rest: what web does
/// beyond serving pages and routes (spec, web). A server's options are
/// `serve`'s and `route`'s, not modules of their own.
pub(crate) const WEB_MODULES: &[&str] = &["auth", "mail", "ws"];

/// The names a module's section of the spec documents: its rows and
/// forms, and those of the modules documented under it (web's modules).
pub(crate) fn names_of(path: &str) -> Vec<&'static str> {
    reserved_by(|m| m == path || crate::modules::by_path(m).is_some_and(|e| e.under == Some(path)), &[])
}

/// Reserved only in files that import ui.
pub(crate) static UI_RESERVED: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| reserved_by(|m| m == "ui", &[]));

/// Every name the compiler answers as a builtin, bare: the core's names,
/// web's and ui's, every form's — the module functions called bare where
/// their module is imported (a frame's `select`, the database's verbs) and
/// the forms the compiler itself writes into a program (a value a guard
/// made known good, a notice's text and time, the page's reader) — and the
/// core's hidden rows; derived from the table, not written again (this
/// list had kept `pad` and `append` long after they went, and lacked
/// `bucket` the day it came).
pub(crate) static BUILTINS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    let mut names: Vec<&'static str> = CORE_RESERVED.iter().copied().collect();
    names.extend(crate::table::ROWS.iter().filter(|r| r.module.is_empty() && r.name.starts_with("__")).map(|r| r.name));
    names.extend(WEB_RESERVED.iter().copied());
    names.extend(UI_RESERVED.iter().copied());
    names.extend(crate::table::FORMS.iter().map(|r| r.name));
    names.sort_unstable();
    names.dedup();
    names
});

/// Whether a module's functions and constants are names a bare `use`
/// makes (the manifest's `registry`).
fn registry_module(path: &str) -> bool {
    crate::modules::by_path(path).is_some_and(|m| m.registry)
}

/// Whether the registry knows a module's functions: one a bare `use`
/// makes names of, or one the tooling knows without an import (`db`).
fn known_module(path: &str) -> bool {
    crate::modules::by_path(path).is_some_and(|m| m.registry || m.known)
}

/// The modules a program may import, for a suggestion.
fn importable() -> Vec<(String, Option<String>)> {
    crate::modules::MODULES.iter().filter(|m| !m.internal && !m.path.is_empty()).map(|m| (m.path.to_string(), None)).collect()
}

/// Whether a `use` names a standard module: one of the manifest's
/// (crate::modules::MODULES).
pub(crate) fn is_standard_module(path: &str) -> bool {
    !path.is_empty() && crate::modules::by_path(path).is_some()
}

/// The edits between two names, a swapped pair of neighbours counting as
/// one (the optimal string alignment distance): `scroe` is one slip from
/// `score`, as `lenn` is from `len`.
pub(crate) fn slips(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The refusals left when the echoes go: a refusal that only says
/// something is not known — a rank, a function value's parameters — beside
/// a refusal of something else is that mistake seen again (a slip in a
/// name leaves what is made of it unknown, wherever it goes), and one
/// mistake is one message (spec, Error Messages, rule 3). Alone, an echo is
/// the mistake itself. (An echo had been dropped only after a refusal on
/// its own line: a misspelled `reverse` had also said the rank of a
/// record's field it reached, in a function lowered before the line, and a
/// misspelled `floor` that a function value nothing types — crow test
/// --fuzz --mistakes.) An unknown makes no refusal of its own, the checks
/// passing it by, so no echo is the root of a refusal beside it.
pub(super) fn without_echoes(errors: Vec<(String, Option<String>)>) -> Vec<(String, Option<String>)> {
    let echo = |m: &str| m.contains("is not known here") || m.contains("nothing types its parameters");
    if errors.iter().any(|(m, _)| !echo(m)) { errors.into_iter().filter(|(m, _)| !echo(m)).collect() } else { errors }
}

/// A name as the program wrote it: a part's function without the prefix
/// the expansion gave it (`helpers_spread` is `spread`), a part's local
/// without its suffix (`df__helpers` is `df`).
pub(crate) fn plain_name(name: &str) -> String {
    let unprefixed = match crate::part_prefix_of_declaration(name) {
        Some(prefix) => name[prefix.len()..].to_string(),
        None => name.to_string(),
    };
    match unprefixed.find("__") {
        Some(i) if i > 0 => unprefixed[..i].to_string(),
        _ => unprefixed,
    }
}

/// The standard modules providing a function, imported or not.
fn stdlib_providers(name: &str) -> Vec<String> {
    crate::modules::ModuleRegistry::with_stdlib().providers_of(name).into_iter().map(String::from).collect()
}

/// What a module's function answers, as a refusal names a kind.
fn answers(module: &str, name: &str) -> Option<&'static str> {
    crate::table::ROWS.iter().find(|r| r.module == module && r.name == name).map(|r| r.ret.words())
}

/// `a`, `a and b`, `a, b and c`.
fn listed(items: &[String], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        _ => format!("{} {} {}", items[..items.len() - 1].join(", "), last, items[items.len() - 1]),
    }
}

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
    /// Variables given two kinds (their reassignment refused), or a value
    /// whose making was refused: what the program reads of them is not
    /// judged by either kind, so the one mistake is told once (spec, Error
    /// Messages, rule 3).
    pub contested: BTreeSet<String>,
    /// The imports asked for, by module: the line and the refusal each is
    /// in, and the names it lists so far (a module the program lacks is
    /// named once, where it is first needed).
    pub missing_use: BTreeMap<String, (usize, usize, Vec<String>, String)>,
    /// The modules a misspelled `use` meant (`use frams` is frame): the
    /// slip's refusal fixes their calls too, which are not asked again.
    pub meant: BTreeSet<String>,
    /// Names an imported module makes: its functions and constants.
    pub used: BTreeSet<String>,
    /// The type each of the current function's own variables was first
    /// given: a local reusing a module variable's name is a new binding
    /// (spec §7), so its type is its own, not the module's.
    pub local_types: HashMap<String, Ty>,
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
    /// Every derivation's read set and who holds it, read once before the
    /// walk (`derive::read_sets`): a route's handler sees the ones the app
    /// holds, and the lowering records them for both backends.
    pub read_sets: BTreeMap<String, crate::derive::ReadSet>,
}

impl<'a> Lowerer<'a> {
    /// Whether the program uses web itself, not only through a part.
    pub(super) fn web_program(&self) -> bool {
        self.ctx.bare_modules.iter().any(|m| (m == "web" || m.starts_with("web.")) && !self.only_a_part_imports(m))
    }

    /// Whether a module imported bare provides the name, which then answers
    /// it: `bytes` and `text` under `use sys` are the standard library's,
    /// not responses. (Inside a standard module written in Crow, the
    /// registry's scope is the module's own imports: a provider there is
    /// one it imported.)
    pub(super) fn provided_bare(&self, name: &str) -> bool {
        self.ctx.registry.providers_of(name).iter().any(|m| self.ctx.bare_modules.contains(*m) || crate::modules::is_crow_fn(&self.current_fn))
    }

    /// Whether a bare call of `name` is web's own form here (a response, a
    /// route, the server): a web program's, with no function of its own and
    /// no module imported bare by the name.
    pub(super) fn web_form(&self, name: &str) -> bool {
        !self.ctx.functions.contains_key(name) && self.web_program() && !self.provided_bare(name)
    }

    /// What a response form reads a parsed value handed to it as: its body
    /// text, `respond`'s status an int.
    pub(super) fn web_form_reads(name: &str, at: usize) -> Option<Ty> {
        match (name, at) {
            ("text" | "html" | "redirect" | "bytes", 0) | ("respond", 1) => Some(Ty::Str),
            ("respond", 0) => Some(Ty::Int),
            _ => None,
        }
    }

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
        if !self.v.ui_mode {
            // a web program's facts, which its derivations read
            self.refuse(format!(
                "'{}' is state, declared on line {}: a program with derivations gives each top-level name one value, which its derivations read — give it its whole value where it is declared (a function can build it), or derive it",
                name, line
            ));
            return;
        }
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
        // (a ui program imports `ui` or `web.ui`: the lowering reads both,
        // and this check had read `ui` alone)
        self.v.ui_mode = self.ctx.bare_modules.contains("ui") || self.ctx.bare_modules.contains("web.ui");
        let mut line = 0usize;
        for s in stmts {
            match s {
                S::Line(n) => line = *n,
                S::Use { path, alias } => {
                    if path == "ui" || path == "web.ui" {
                        self.v.ui_mode = true;
                    }
                    // a module there is no file for and no standard module
                    // of (the expansion found nothing at this line): the
                    // nearest standard module, if one is a slip away
                    if crate::UNRESOLVED_USES.with(|u| u.borrow().iter().any(|(l, p)| *l == line && p == path)) {
                        self.v.line = line;
                        let standard = importable();
                        let near = self.near(path, standard);
                        if let Some(meant) = near.split("did you mean '").nth(1).and_then(|r| r.split('\'').next()) {
                            self.v.meant.insert(meant.to_string());
                        }
                        let file = format!("{}.co", path.replace('.', "/"));
                        if near.is_empty() {
                            self.refuse(format!("no module '{}': there is no {} in the project, and no standard module of the name", path, file));
                        } else {
                            self.refuse(format!("no module '{}'{}", path, near));
                        }
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
        if imported.iter().any(|p| registry_module(p)) {
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
        // what each derivation reads and who holds it: a route's handler
        // reads the ones the app holds
        if !self.ctx.derives.is_empty() {
            let derive_names = self.ctx.derives.clone();
            let bodies: BTreeMap<String, Vec<S>> = self.ctx.functions.iter().filter(|(n, _)| !derive_names.contains(*n)).map(|(n, sig)| (n.clone(), sig.body.clone())).collect();
            let params: BTreeMap<String, Vec<String>> = self.ctx.functions.iter().map(|(n, sig)| (n.clone(), sig.params.iter().map(|(p, _)| p.clone()).collect())).collect();
            self.v.read_sets = crate::derive::read_sets(stmts, &derive_names, &bodies, &params);
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
        if self.ctx.functions.contains_key(name) && !(plain_fn && self.ctx.derives.contains(name) && !self.route_reads(name)) {
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
    pub(super) fn checked(&self, x: &Expr) -> Ty {
        if let Expr::Call { target: Target::User(f), .. } = x {
            let returns = self.ctx.functions.get(f).map_or(false, |sig| returns_value(&sig.body));
            if returns && matches!(self.ctx.functions.get(f).map(|s| &s.return_type), Some(Ty::Unit)) {
                return Ty::Unknown;
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
        if self.out_of_view(name) {
            return;
        }
        // the names in view: this body's, and the program's where a plain
        // function sees them; and the language's own words (`retrun`)
        let plain_fn = matches!(self.at, Where::In(_)) && !self.v.reactive;
        let hidden = plain_fn || (self.at == Where::Test && self.v.attached_test);
        let mut seen: Vec<String> = self.scopes.iter().skip(1).flat_map(|s| s.iter().cloned()).collect();
        seen.extend(self.v.stored.iter().cloned());
        if !hidden {
            seen.extend(self.scopes[0].iter().cloned());
            seen.extend(self.v.module_vars.iter().cloned());
        }
        let mut candidates: Vec<(String, Option<String>)> = seen.into_iter().map(|n| (n, None)).collect();
        candidates.extend(crate::lexer::KEYWORDS.iter().map(|k| (k.to_string(), None)));
        let near = self.near(name, candidates);
        self.refuse(format!("undefined variable '{}'{}", name, near));
    }

    /// Whether the body being lowered is a route's handler reading a
    /// derivation the app holds: a route has no session, so it reads the
    /// app's value (spec, Derivations — a route reads what the app holds).
    pub(super) fn route_reads(&self, name: &str) -> bool {
        matches!(self.at, Where::In(_)) && self.v.route_handlers.contains(&self.current_fn) && self.v.read_sets.get(name).is_some_and(|rs| rs.shared)
    }

    /// A name a body cannot see, though the program has it — a derivation
    /// or a module variable read inside a function — said the one way,
    /// whether the body reads it (`xs`) or indexes it (`xs[i]`). Whether it
    /// was said.
    fn out_of_view(&mut self, name: &str) -> bool {
        let in_fn = self.in_fn();
        let prefix = in_fn.as_ref().map(|f| format!("in {}: ", f)).unwrap_or_default();
        let route = in_fn.as_ref().is_some_and(|f| self.v.route_handlers.contains(f));
        if route && let Some(why) = self.v.read_sets.get(name).and_then(|rs| rs.per_session.clone()) {
            self.refuse(format!(
                "{}'{}' is held by each session, and a route's handler has no session — {}: a route reads a derivation the app holds, one over tables, the clock and facts no session moves",
                prefix, name, why
            ));
        } else if in_fn.is_some() && self.ctx.derives.contains(name) {
            self.refuse(format!(
                "{}'{}' is a derivation, which a function does not see — read it in a view, a handler or a route's handler, or make this function a derivation",
                prefix, name
            ));
        } else if in_fn.as_ref().is_some_and(|f| self.v.route_handlers.contains(f)) && self.v.module_vars.contains(name) {
            // a route calls its handler with the request alone, so no
            // parameter can carry the variable in
            self.refuse(format!(
                "{}'{}' is a module variable, which a route's handler does not see — the route hands it its request alone: open what it needs inside it (every connect to one URL in a program is one handle, so a handler that connects shares one pool)",
                prefix, name
            ));
        } else if in_fn.is_some() && self.v.module_vars.contains(name) {
            self.refuse(format!(
                "{}'{}' is a module variable, which a function does not see — pass it as a parameter, or wire the function as a handler",
                prefix, name
            ));
        } else {
            return false;
        }
        true
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
    pub(super) fn check_assign(&mut self, name: &str, new_ty: &Ty) {
        let in_local_scope = self.in_fn().is_some() && !(self.v.reactive && self.v.module_vars.contains(name));
        let existing = if in_local_scope { self.v.local_types.get(name).cloned() } else { self.ctx.vars.get(name).cloned() };
        if in_local_scope && !matches!(new_ty, Ty::Unknown) {
            self.v.local_types.entry(name.to_string()).or_insert_with(|| new_ty.clone());
        }
        let Some(existing_ty) = existing else { return };
        // Assignment into a typed variable is a consumption point: result[T]
        // unwraps into T (the function stops on the error) — not a type change.
        let unwraps = matches!(new_ty, Ty::Result(inner)
            if existing_ty.compatible(inner)
                || (matches!(existing_ty, Ty::Float) && matches!(inner.as_ref(), Ty::Int)))
            || matches!(&existing_ty, Ty::Result(inner) if inner.compatible(new_ty));
        if !unwraps && !existing_ty.compatible(new_ty) && !matches!(existing_ty, Ty::Unknown) && !matches!(new_ty, Ty::Unknown) {
            // a value whose type the program does not know (a cell of a
            // column no database described, a parsed document's member)
            // says what it is first
            let unknown = matches!(new_ty, Ty::Json) || matches!(new_ty, Ty::Result(inner) if matches!(inner.as_ref(), Ty::Json));
            let why = if unknown {
                "a value whose type the program does not know is read as one first: str[…], int[…] or float[…]"
            } else {
                "variable type cannot change"
            };
            // (known in the passes before the one that speaks)
            self.v.contested.insert(name.to_string());
            self.refuse(format!("cannot reassign '{}' from {} to {} — {}", name, existing_ty.crow_name(), new_ty.crow_name(), why));
        }
    }

    /// A derivation reads facts and sources and returns a value: it is a
    /// computation, so it writes no fact, and it lives in a ui program.
    pub(super) fn check_derive(&mut self, name: &str, body: &[S]) {
        self.check_reserved(name, "derivation");
        if !self.v.ui_mode && self.v.route_handlers.is_empty() {
            self.refuse(format!(
                "derive {} needs a program that serves — derivations live in a `use ui` program with a page, whose views and handlers read them, or in a `use web` program whose routes read them",
                name
            ));
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
    pub(super) fn check_binop(&mut self, op: &BinOp, lt: Ty, rt: Ty) {
        let lt = if let Ty::Result(inner) = lt { *inner } else { lt };
        let rt = if let Ty::Result(inner) = rt { *inner } else { rt };
        if matches!(lt, Ty::Unknown) || matches!(rt, Ty::Unknown) {
            return;
        }
        // `and` and `or` join truths: two bools, or two masks item by item
        // (a list of bools; a frame's column of bools is lowered as the mask
        // it makes). An int is no truth: `3 and 3` had been true
        // interpreted and 3 compiled (crow check --self).
        if matches!(op, BinOp::And | BinOp::Or) {
            // (a mask of a matrix's shape, its rows of bools, too)
            let truth = |t: &Ty| matches!(t, Ty::Bool | Ty::Json) || t.is_truths() || matches!(t, Ty::List(i) | Ty::Column(i) if matches!(i.payload(), Ty::Bool | Ty::Unknown));
            if let Some(side) = [&lt, &rt].into_iter().find(|t| !truth(t)) {
                let word = if matches!(op, BinOp::And) { "and" } else { "or" };
                self.refuse(format!("`{}` joins truths — two bools, or two masks item by item — and this side is {}: compare it first (x gt 0, x ne '')", word, crate::table::Arg::of_type(side).words()));
            }
            return;
        }
        // `^` is a number's power: two numbers, one at a time (`.^` raises a
        // tensor's numbers one by one). `3 ^ true` had stopped interpreted
        // and been 3 compiled; a power of a map or text had not built.
        if matches!(op, BinOp::Pow) {
            let number = |t: &Ty| matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction | Ty::Json);
            if !number(&lt) || !number(&rt) {
                let many = |t: &Ty| matches!(t, Ty::Tensor(..) | Ty::Column(_) | Ty::List(_));
                let hint = if many(&lt) || many(&rt) { " — a tensor's numbers are raised one by one with .^" } else { "" };
                self.refuse(format!("cannot use '^' with {} and {}{}", lt.crow_name(), rt.crow_name(), hint));
            } else if (matches!(lt, Ty::Decimal | Ty::Fraction) && !matches!(rt, Ty::Int | Ty::Big | Ty::Json)) || matches!(rt, Ty::Decimal | Ty::Fraction) {
                // (the runtime's words, where it is written: interpreted it
                // had stopped as it ran, and compiled it had not built)
                self.refuse(format!("an exact number's power is by an int, and this is {}: `d ^ 2`", crate::table::Arg::of_type(&rt).words()));
            }
            return;
        }
        // `.*`, `./` and `.^` work item by item: numbers, at least one side a
        // tensor, a frame's column or a list of numbers (a mask's truths are
        // 1 and 0). Two numbers, a bool, text or a map had been written into
        // the compiled program as they stood, which did not build.
        if matches!(op, BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow) {
            let numbers = |t: &Ty| matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Json | Ty::Tensor(..) | Ty::Column(_)) || matches!(t, Ty::List(i) if matches!(i.payload(), Ty::Int | Ty::Float | Ty::Bool));
            let many = |t: &Ty| matches!(t, Ty::Tensor(..) | Ty::Column(_) | Ty::List(_));
            let word = match op {
                BinOp::ElemMul => ".*",
                BinOp::ElemDiv => "./",
                _ => ".^",
            };
            if !numbers(&lt) || !numbers(&rt) {
                self.refuse(format!("cannot use '{}' with {} and {}", word, lt.crow_name(), rt.crow_name()));
            } else if !many(&lt) && !many(&rt) {
                let plain = match op {
                    BinOp::ElemMul => "*",
                    BinOp::ElemDiv => "/",
                    _ => "^",
                };
                self.refuse(format!("`{}` works item by item on tensors, and these are two numbers: write {}", word, plain));
            }
            return;
        }
        // a decimal or a fraction is compared with one number: a mask
        // compares each cell with a float (the interpreter had stopped,
        // "expected a decimal, got array", and the compiled program had not
        // built — crow check --self)
        if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte) {
            // a handle is its own: two connections, responses, sockets or
            // mailers are never equal, and none orders (the interpreter had
            // answered false, and the compiled program had not built)
            let handle = |t: &Ty| matches!(t, Ty::Db | Ty::Tx | Ty::WebResponse | Ty::Socket | Ty::Mailer | Ty::Stream);
            if let Some(h) = [&lt, &rt].into_iter().find(|t| handle(t)) {
                let words = match h {
                    Ty::Db | Ty::Tx => "a database connection",
                    Ty::WebResponse => "a response",
                    t => crate::table::Arg::of_type(t).words(),
                };
                self.refuse(format!("{} is not compared: a handle is its own, equal to no other — compare what it holds", words));
                return;
            }
            let exact = |t: &Ty| matches!(t, Ty::Decimal | Ty::Fraction);
            let many = |t: &Ty| matches!(t, Ty::Tensor(..) | Ty::Column(_) | Ty::List(_));
            if let Some((e, m)) = [(&lt, &rt), (&rt, &lt)].into_iter().find(|(e, m)| exact(e) && many(m)) {
                let words = |t: &Ty| crate::table::Arg::of_type(t).words();
                self.refuse(format!("{} is compared with one number, and the other side is {}: compare float[…] of it, as a mask compares each cell with a number", words(e), words(m)));
            }
            return;
        }
        if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
            return;
        }
        // `%` takes ints (a float is refused where the operator is lowered):
        // a tensor's or a column's remainders are `mod[t, d]`'s, and the
        // compiled program had not built them
        if matches!(op, BinOp::Mod) && [&lt, &rt].iter().any(|t| matches!(t, Ty::Tensor(..) | Ty::Column(_) | Ty::List(_) | Ty::Sparse)) {
            self.refuse("`%` takes ints and answers the remainder — round a float first (int[x], floor[x], round[x]), or use mod[t, d] on a tensor".to_string());
            return;
        }
        // `/` divides by a number: two tensors — a list of numbers, a frame's
        // column — are divided item by item with `./` (spec, Operators; the
        // interpreter had stopped as it ran, and the compiled program had
        // not built)
        let many = |t: &Ty| matches!(t, Ty::Tensor(..) | Ty::Column(_)) || matches!(t, Ty::List(i) if matches!(i.payload(), Ty::Int | Ty::Float | Ty::Bool));
        if matches!(op, BinOp::Div) && many(&lt) && many(&rt) {
            self.refuse("`/` divides by a number, and these are two tensors: divide item by item with ./".to_string());
            return;
        }
        // String + is not allowed — use interpolation instead
        if matches!(op, BinOp::Add) && (matches!(lt, Ty::Str) || matches!(rt, Ty::Str)) {
            self.refuse("cannot use '+' with strings — interpolate to combine ('`a` `b`'), or collect parts and join[parts, sep] to build".to_string());
            return;
        }
        // Numeric ops: both must be numeric (a tensor, a sparse matrix, a
        // frame's column as its numbers); a list of text is no numbers (the
        // compiled program had parsed it, where the interpreter made each
        // item an error)
        let is_num_like = |t: &Ty| matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Tensor(..) | Ty::Sparse | Ty::Column(_));
        // an exact number meets ints, floats (refused where the operator is
        // lowered, a literal read as the number it writes) and its kin
        let exact = |t: &Ty| matches!(t, Ty::Decimal | Ty::Fraction);
        if exact(&lt) || exact(&rt) {
            let scalar = |t: &Ty| matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction);
            if !scalar(&lt) || !scalar(&rt) {
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    _ => "%",
                };
                self.refuse(format!("cannot use '{}' with {} and {}", op_str, lt.crow_name(), rt.crow_name()));
            }
            return;
        }
        // a moment: on or back by seconds, and the seconds between two
        let moment = |t: &Ty| matches!(t, Ty::Moment);
        if moment(&lt) || moment(&rt) {
            // (a side nothing has typed yet — a parameter typed by its
            // callers — is judged where it is known)
            if matches!(lt, Ty::Unknown) || matches!(rt, Ty::Unknown) {
                return;
            }
            let number = |t: &Ty| matches!(t, Ty::Int | Ty::Float);
            let allowed = match op {
                BinOp::Add => (moment(&lt) && number(&rt)) || (number(&lt) && moment(&rt)),
                BinOp::Sub => moment(&lt) && (number(&rt) || moment(&rt)),
                _ => false,
            };
            if !allowed {
                self.refuse("a moment adds or subtracts seconds (t + 60), subtracts a moment (t2 - t1, the seconds between), and compares with a moment — nothing else".to_string());
            }
            return;
        }
        // a list of numbers (or of truths) beside numbers is its tensor, as
        // the lowering makes it after this check (spec, Tensor Arithmetic)
        let listed = |t: &Ty| matches!(t, Ty::List(i) if matches!(**i, Ty::Int | Ty::Float | Ty::Bool));
        let plain = |t: &Ty| listed(t) || matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Tensor(..) | Ty::Column(_));
        let lists_as_numbers = (listed(&lt) || listed(&rt)) && plain(&lt) && plain(&rt) && !matches!(op, BinOp::Mod);
        if !lists_as_numbers && (!is_num_like(&lt) || !is_num_like(&rt)) {
            let op_str = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                _ => "%",
            };
            // a value whose type is not known (a cell of a column nothing
            // typed, a parsed document's member) says what it is first
            let hint = if matches!(lt, Ty::Json) || matches!(rt, Ty::Json) {
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
    /// and a mask decides every row: it is refused as a frame's mask refuses it
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
        // two parsed values compare as the program runs, in order too (two
        // numbers, two texts; another pair stops there); one beside a value
        // of a kind was read as that kind before this check, and one beside
        // a value of no kind — a list, a map — has nothing to be read as
        if matches!(l.payload(), Ty::Json) && matches!(r.payload(), Ty::Json) {
            return;
        }
        if matches!(l.payload(), Ty::Json) || matches!(r.payload(), Ty::Json) {
            if cmp.orders() {
                self.refuse(format!("{} compares numbers with numbers or text with text — a value whose type the program does not know is read as one first: int[…], float[…] or str[…]", cmp.word()));
            }
            return;
        }
        let (Some(a), Some(b)) = (order_side(l), order_side(r)) else { return };
        let scalar = |k: &str| matches!(k, "a number" | "text" | "a bool" | "a moment");
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
        let compares = x == y && if cmp.orders() { matches!(x.as_str(), "a number" | "text" | "a moment") } else { true };
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
            // (a map's value is the map's own check's: one mistake, one message)
            Ty::Map(..) if row.id == "core.set" => return,
            Ty::List(i) | Ty::Map(_, i) => (**i).clone(),
            _ => return,
        };
        // a list variable's items are the kind they were first known to be:
        // the scan meets the pushes in order and notes it, and the final
        // pass holds each push to it (the variable's settled type is the last
        // binding's, so a push of another kind had been blamed on the first)
        let key = match holder {
            Expr::Local(name, _) | Expr::Move(name, _) if row.id == "core.push" => Some((self.current_fn.clone(), name.clone())),
            _ => None,
        };
        if self.pass != super::Pass::Final {
            if let Some(key) = key {
                if let Some((many, _)) = item_kind(&inner).or_else(|| item_kind(&item.ty())) {
                    self.ctx.first_items.entry(key).or_insert(many);
                }
            }
            return;
        }
        let first = key.and_then(|k| self.ctx.first_items.get(&k).cloned());
        let many = match (first, item_kind(&inner)) {
            (Some(first), _) => first,
            (None, Some((many, _))) => many,
            (None, None) => return,
        };
        if let Some((others, one)) = item_kind(&item.ty())
            && many != others
        {
            self.refuse(format!("{} takes {}: its {} are {}, not {}", crate::table::form(row.name), takes, what, many, one));
        }
        // (`[]` is a list of any kind: one only a list of lists holds)
        if matches!(item.ty().payload(), Ty::List(e) if **e == Ty::Unknown) && !many.starts_with("lists") {
            self.refuse(format!("{} takes {}: its {} are {}, not a list", crate::table::form(row.name), takes, what, many));
        }
    }

    /// A field read from a record; a list has `len` and no other field (a
    /// list of tensors asked its `.shape` had stopped interpreted and not
    /// built compiled).
    pub(super) fn check_field(&mut self, obj_type: &Ty, field: &str) {
        if matches!(field, "ok" | "reason" | "kind" | "trace") {
            return;
        }
        // (a value that can fail has its payload's fields: a list read from
        // a file asked `.b` had stopped interpreted and not built compiled —
        // crow check --self's io family)
        let obj_type = obj_type.payload();
        if matches!(obj_type, Ty::List(_)) && field != "len" {
            let owner = match field {
                "shape" | "dtype" | "device" => "a tensor's",
                "columns" | "dtypes" => "a frame's",
                _ => "a record's",
            };
            self.refuse(format!("a list has no field '{}': `.{}` is {} — index an item for its own", field, field, owner));
            return;
        }
        // a number or a bool has no fields (`x.name` of an int had stopped
        // the interpreter as it ran and the compiled program had not built)
        if matches!(obj_type, Ty::Int | Ty::Float | Ty::Bool) {
            let words = crate::table::Arg::of_type(obj_type).words();
            if BUILTINS.contains(&field) {
                self.refuse(format!("{} has no field '{}' — a function takes its value first: {}[…]", words, field, field));
            } else {
                self.refuse(format!("{} has no field '{}'", words, field));
            }
            return;
        }
        if let Ty::Record(name) = obj_type {
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
        let registry_module = known_module(&real) && self.ctx.registry.has_function(field);
        // the module's names reach the entry only through a part's import
        if registry_module && !self.ctx.functions.contains_key(&prefixed) && self.only_a_part_imports(&real) {
            self.refuse_missing_use(&format!("{}.{}", module, field), &real, "");
            return;
        }
        if registry_module || self.ctx.functions.contains_key(&prefixed) || self.ctx.structs.contains_key(&prefixed) || self.defined(module) {
            return;
        }
        let known = known_module(&real);
        let providers = stdlib_providers(field);
        if known {
            if CORE_RESERVED.contains(&field) {
                // the core's name (`read`, `sqrt`) is called bare, whether
                // or not a module has a form of its own
                self.refuse(format!("{} has no '{}' — {} is the core's: {}[…]", module, field, field, field));
            } else if providers.contains(&real) {
                // the module has it: the program does not import the module
                self.refuse_missing_use(&format!("{}.{}", module, field), &real, "");
            } else if let Some(m) = providers.first() {
                // another module has it
                let imported = self.ctx.registry.providers_of(field).iter().any(|p| p == m);
                if imported {
                    self.refuse(format!("{} has no '{}' — {}[] is {}'s: call it {}[…]", module, field, field, m, field));
                } else {
                    self.refuse(format!("{} has no '{}' — {}[] is {}'s: add `use {}`", module, field, field, m, m));
                }
            } else {
                // a slip in one of the module's own names
                let registry = crate::modules::ModuleRegistry::with_stdlib();
                let own: Vec<(String, Option<String>)> = registry.functions_of(&real).into_iter().map(|f| (f.to_string(), None)).collect();
                let near = self.near(field, own);
                self.refuse(format!("{} has no '{}'{}", module, field, near));
            }
            return;
        }
        // not a module the program has: the last name of one whose functions
        // are called by their own (`stats.mean` for math.stats's), or a slip
        // in a module's name
        if let Some(full) = crate::modules::MODULES.iter().filter(|m| m.registry).map(|m| m.path).find(|m| m.contains('.') && m.rsplit('.').next() == Some(module)) {
            match providers.first() {
                Some(m) if m == full => self.refuse(format!("'{}' is {}, whose functions are called by their own names: add `use {}`, then {}[…]", module, full, full, field)),
                Some(m) => self.refuse(format!("{}[] is {}'s, called by its own name: add `use {}`, then {}[…]", field, m, m, field)),
                None => self.refuse(format!("'{}' is {}, whose functions are called by their own names: add `use {}`", module, full, full)),
            }
            return;
        }
        let near = self.near(module, importable());
        self.refuse(format!("undefined module or variable '{}'{}", module, near));
    }

    /// A call by name, its arguments already lowered and checked.
    /// A builtin with rows takes the counts and kinds its rows take — the
    /// core's, and a loaded module's of the same name — refused in the
    /// table's words when none does.
    fn refuse_kinds(&mut self, name: &str, argc: usize, types: &[Ty]) {
        // (a local or a parameter of the name, indexed — `rsi[0]` of a list
        // named like chart's rsi — is a value read, not the builtin; and a
        // function of the program's of the name is the call, its own
        // parameters the contract: math.co's own tests hand `randn` its
        // sizes as one list)
        if self.declared(name) || self.ctx.functions.contains_key(name) {
            return;
        }
        // (a module's rows reached by the bare name are checked as the
        // core's are: web.mail's `mail` had taken an int, a list or a
        // column where it takes text, and the modes showed it differently)
        // (a module only a part imports is not the entry's to resolve by)
        let mut providers: Vec<String> = self.ctx.registry.providers_of(name).into_iter().map(str::to_string).filter(|m| !self.only_a_part_imports(m)).collect();
        // web's names are its builtins (WEB_RESERVED), their rows its
        // modules' (web.mail's `mail`, web.ws's `send` and `recv`)
        let web = self.v.bare_import_lines.keys().any(|p| p == "web" || p.starts_with("web."));
        if web && WEB_RESERVED.contains(&name) {
            for r in crate::table::ROWS.iter().filter(|r| r.name == name && r.module.starts_with("web")) {
                if !providers.iter().any(|m| m == r.module) {
                    providers.push(r.module.to_string());
                }
            }
        }
        let module_rows = providers.iter().any(|m| crate::table::ROWS.iter().any(|r| r.module == m.as_str() && r.name == name));
        if !crate::table::has_core_rows(name) && !module_rows {
            return;
        }
        let kinds: Vec<crate::table::Arg> = (0..argc).map(|i| types.get(i).map_or(crate::table::Arg::Unknown, crate::table::Arg::of_type)).collect();
        // `json[map]` is web's JSON response (use web), a form of its own, as
        // a record's and a list's are (an object, an array); `json[frame]`
        // answers the frame's records
        if name == "json" && web && kinds.first().is_some_and(crate::table::Arg::json_response) {
            return;
        }
        let mut modules: Vec<&str> = vec![""];
        modules.extend(providers.iter().map(String::as_str));
        // (a web program's `json` answers a response too: its refusal names
        // both uses, where the parse's alone had said it takes text)
        if name == "json" && web && argc == 1 && crate::table::refusal(&modules, name, &kinds).is_some() {
            let what = kinds.first().map_or("nothing", |k| k.words());
            self.refuse(format!("json[] takes text to parse, or a map, a list, a record or a frame to answer with, got {}", what));
            return;
        }
        if let Some(e) = crate::table::refusal(&modules, name, &kinds) {
            // the form for these arguments may be a module's the program does
            // not import: it says that module (`abs` of a column is math's —
            // it had said abs[] takes an int or a number)
            for m in stdlib_providers(name) {
                let has_rows = crate::table::ROWS.iter().any(|r| r.module == m && r.name == name);
                if !providers.contains(&m) && has_rows && crate::table::refusal(&["", m.as_str()], name, &kinds).is_none() {
                    let words: Vec<String> = kinds.iter().map(|k| k.words().to_string()).collect();
                    self.refuse_missing_use(name, &m, &format!(" of {}", listed(&words, "and")));
                    return;
                }
            }
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

    pub(super) fn check_call(&mut self, name: &str, args: &[A], types: &[Ty]) {
        let arg_type = |_: &Self, i: usize| -> Ty { types.get(i).cloned().unwrap_or(Ty::Unknown) };
        // a message is sent once, and a render or a derivation runs
        // whenever the page needs it (mail's row is web.mail's, written in
        // Crow; the rule is the session's, wherever the name resolves)
        if name == "mail" && !self.declared(name) && matches!(self.at, Where::In(FnKind::View) | Where::Derive) {
            self.refuse(MAIL_IN_A_RENDER.to_string());
        }
        // a scheme encode and decode do not know, written in the call, is
        // refused before either mode runs (computed, encode stops on it)
        if matches!(name, "encode" | "decode") && args.len() == 2 {
            if let A::Str(scheme) = &args[1] {
                if !crow_runtime::text::SCHEMES.contains(&scheme.as_str()) {
                    self.refuse(crow_runtime::text::unknown_scheme(name, scheme));
                }
            }
        }
        if name == "route" && !self.defined("route") && !matches!(arg_type(self, 0), Ty::Str | Ty::Unknown) {
            self.refuse("route[] takes a path first — the app is implicit: route['/', handler]".to_string());
        }
        // any[], all[] and find[] take a list and a function: a value of
        // another kind — a frame a variable named like the predicate held,
        // in a generated program — is refused before either mode runs (the
        // interpreter had stopped as it ran, "expected a function, got
        // frame", and the compiled program had not built)
        if matches!(name, "any" | "all" | "find") && args.len() == 2 && !self.defined(name) {
            // (`find[bytes, needle]` is sys's row, which its own kinds check)
            let (xs, f) = (arg_type(self, 0), arg_type(self, 1));
            let items = matches!(xs.payload(), Ty::List(_) | Ty::Tensor(..) | Ty::Column(_));
            let function = matches!(f.payload(), Ty::Fn(..) | Ty::Unknown);
            if items && !function {
                let what = |t: &Ty| crate::table::Arg::of_type(t.payload()).words();
                self.refuse(format!("{}[] takes a list and a function, got {} and {}", name, what(&xs), what(&f)));
            }
        }
        // Test-harness builtins exist only inside test blocks (which never
        // reach a build) — accept them there, refuse them anywhere else.
        if crate::table::has(name, crate::table::Prop::TestOnly) {
            if self.at != Where::Test {
                self.refuse(format!("'{}' is a test-harness builtin — it only works inside a test block", name));
            }
            return;
        }
        let argc = args.len();
        let own = self.ctx.functions.contains_key(name);

        // db's own forms — connect, query, exec — are db's: without a `use
        // db` of the file's own, neither mode has them (the interpreter had
        // stopped as it ran, "does not run 'connect' yet"; the drivers
        // written in Crow carry their own import)
        if !own && matches!(name, "connect" | "query" | "exec") && !self.declared(name) && !crate::modules::is_crow_fn(&self.current_fn) {
            let imported = (self.ctx.bare_modules.iter().any(|m| m == "db") && !self.only_a_part_imports("db")) || self.ctx.module_aliases.values().any(|m| m == "db");
            if !imported {
                self.refuse_missing_use(name, "db", "");
                return;
            }
        }

        // a route and its responses are web's: without `use web` neither
        // mode has them (the interpreter had answered msgpack, arrow, bytes
        // and stream with a map that only looked like a response)
        if !own && !self.web_program() && !self.provided_bare(name) && matches!(name, "route" | "serve" | "respond" | "redirect" | "msgpack" | "arrow" | "bytes" | "stream") {
            self.refuse(format!("{}[] is web's: add `use web`", name));
            return;
        }
        // a response's body is text, as a feed's put takes it, and its
        // status an HTTP status; arrow's a frame (spec, Responses) — refused
        // before either mode runs: the compiled program had not built, and
        // the interpreter had answered a number's text, a status of 1000 as
        // 599 (crow check --self, the routes family)
        if self.web_form(name) {
            let what = |t: &Ty| crate::table::Arg::of_type(t.payload()).words();
            // (the outer kind decides — a column or a list is no text,
            // whatever its items — and a parsed value a text form reads as
            // text)
            let known = |t: &Ty| !matches!(t.payload(), Ty::Unknown | Ty::Json);
            let text_at = match name {
                "text" | "html" | "redirect" | "bytes" => Some(0),
                "respond" => Some(1),
                _ => None,
            };
            if let Some(i) = text_at.filter(|i| *i < argc) {
                let t = arg_type(self, i);
                // (`html` of a test's app session is the harness's)
                if known(&t) && !matches!(t.payload(), Ty::Str | Ty::Markup) && !(name == "html" && matches!(t.payload(), Ty::App)) {
                    self.refuse(crow_web::core::response::takes_text(name, what(&t)));
                }
            }
            if name == "respond" && argc == 2 {
                let t = arg_type(self, 0);
                if known(&t) && !matches!(t.payload(), Ty::Int | Ty::Big) {
                    self.refuse(crow_web::core::response::takes_status(what(&t)));
                } else if let Some(A::Int(n)) = args.first()
                    && let Err(e) = crow_web::core::response::status(*n)
                {
                    self.refuse(e);
                }
            }
            if name == "arrow" && argc == 1 {
                let t = arg_type(self, 0);
                if !matches!(t.payload(), Ty::Unknown | Ty::Frame(_)) {
                    self.refuse(crow_web::core::response::takes_frame(what(&t)));
                }
            }
        }

        // stream[feed, a, …] calls feed[put, a, …]: the feed is a function
        // of the program, taking put and then what the call hands it
        // a page's `later[f]` runs f on the page's state, which carries what
        // f needs; arguments are a script's and a server's (spec, Background
        // Work) — in a page's handler they are refused before either mode
        // runs (compiled, the work dropped them and the page did not build;
        // the interpreter ran them inline)
        if name == "later" && !own && argc > 1 && self.v.ui_mode && matches!(self.at, Where::In(FnKind::Handler | FnKind::Helper)) {
            self.refuse("later[f, …] hands arguments in a script or a server; a page's later[f] runs f on the page's state — keep what f needs in a fact it reads".to_string());
        }
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

        // a matrix's items are its rows (spec, Collections): `sort` orders
        // numbers or text, and rows have no order; `x in m` asks whether a
        // row is x, which a number is not — refused where the rank shows,
        // in both modes (the runtime stops in the same words where it shows
        // only as the program runs)
        if !own {
            let rows = |t: &Ty| matches!(t.payload(), Ty::Tensor(_, Some(d)) if d.len() >= 2);
            if name == "sort" && argc == 1 && rows(&arg_type(self, 0)) {
                self.refuse(crow_runtime::core::SORT_ROWS.to_string());
            }
            if name == "contains" && argc == 2 && rows(&arg_type(self, 0)) && matches!(arg_type(self, 1).payload(), Ty::Int | Ty::Big | Ty::Float) {
                self.refuse(crow_runtime::core::NUMBER_IN_ROWS.to_string());
            }
            // (`join` shows each item as text, and a row is no number: it had
            // joined a matrix's numbers)
            if name == "join" && argc == 2 && rows(&arg_type(self, 0)) {
                self.refuse("join[] joins items that show as text — text, numbers, truths — and a matrix's items are its rows: join[flatten[m], sep] joins its numbers".to_string());
            }
        }

        // Maps are dictionaries: set[] must not change the map's value type
        // (spec Design Note "Maps Are Dictionaries, Not Records"), said in
        // Crow terms before either backend meets it.
        // a map's keys are one kind: a key of the other is refused — `m['2']`
        // of a map keyed by ints, `set[m, 2, v]` of one keyed by text
        if matches!(name, "set" | "del" | "contains") && argc >= 2 {
            let map_ty = arg_type(self, 0);
            let map_ty = if let Ty::Result(i) = map_ty { *i } else { map_ty };
            if let Ty::Map(key, _) = &map_ty {
                let given = arg_type(self, 1);
                let given = if let Ty::Result(i) = given { *i } else { given };
                let handed = crate::type_infer::key_kind_of(&given);
                if key.unify(handed).is_none() {
                    self.refuse(crate::table::wrong_key(*key, &given));
                }
            }
        }
        // a map's keys are text or ints: `map[[1.5, 2.5], …]` is refused
        if name == "__map_of" && argc == 2 {
            let keys = arg_type(self, 0);
            let item = match &keys {
                Ty::List(i) | Ty::Tensor(i, _) => Some(i.as_ref().clone()),
                _ => None,
            };
            if matches!(item, Some(Ty::Float)) {
                self.refuse("a map's keys are text or ints, not floats: `map[[1, 2], …]` keys by ints, `map[['1.5'], …]` by text".to_string());
            }
        }
        if name == "set" && argc == 3 {
            let map_ty = arg_type(self, 0);
            let map_ty = if let Ty::Result(i) = map_ty { *i } else { map_ty };
            if let Ty::Map(_, inner) = &map_ty {
                let val_ty = arg_type(self, 2);
                let val_ty = if let Ty::Result(i) = val_ty { *i } else { val_ty };
                if !matches!(inner.as_ref(), Ty::Unknown) && !matches!(val_ty, Ty::Unknown | Ty::Unit) && !inner.compatible(&val_ty) {
                    let (held, given) = (crate::table::Arg::of_type(inner).words(), crate::table::Arg::of_type(&val_ty).words());
                    match args.get(1) {
                        Some(A::Str(key)) => self.refuse(format!("a map holds values of one kind: the entry '{}' holds {}, and the map's other values are {} — a record (struct) holds fields of several kinds", key, given, held)),
                        _ => self.refuse(format!("a map holds values of one kind: the map's values are {}, and this is {} — a record (struct) holds fields of several kinds", held, given)),
                    }
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

        // ui elements exist only inside view blocks — and a standard module's
        // own function, inlined, keeps its names: llm's `embed` stacks
        // tensor rows in a `use ui` program too (its `stack` is math's)
        if crate::lower::UI_ELEMENTS.contains(&name) && self.v.ui_mode && !self.in_view() && !crate::modules::is_crow_fn(&self.current_fn) {
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
                // (the standard library's own source, tested on its own,
                // calls the function it defines for a row of that name —
                // math.co's `load` beside frame's — and the program's own
                // function is the call, as it is with one provider)
                let own_row = self.ctx.bare_modules.contains("sys")
                    && self.ctx.functions.contains_key(name)
                    && crate::table::ROWS.iter().any(|r| r.is_crow() && crate::table::crow_fn_base(r) == name);
                if own_row {
                    return;
                }
                // (two modules' rows of one name that the arguments' kinds
                // tell apart are no ambiguity — sys's `send` of a stream
                // beside a socket's — the kinds pick, as they pick among one
                // module's rows; the call is then checked as the row's)
                let kinds: Vec<crate::table::Arg> = types.iter().map(crate::table::Arg::of_type).collect();
                let taking = providers.iter().filter(|m| crate::table::row_taking(m, name, &kinds).is_some()).count();
                // (a kind not known here — a function not yet called with
                // its frame — settles nothing: the row is picked where the
                // kinds are known, or by the values as the program runs)
                // (the one resolver decides: where it names a module, or the
                // core's row takes the kinds, nothing is ambiguous)
                let decided = crate::table::core_row(name, &kinds).is_some() || crate::table::module_of_bare(name, &providers, &kinds).is_some();
                let unknown = kinds.iter().any(|k| matches!(k, crate::table::Arg::Unknown | crate::table::Arg::AnyList));
                if taking == 1 || decided || unknown {
                    // (decided by the first argument's kind while no row takes
                    // the arguments: refused in the table's words, as one
                    // module's call is — `load` of a tensor under `use frame,
                    // math` had reached math's row of text, stopped
                    // interpreted and not built compiled; crow check --self's
                    // gpu family)
                    if taking == 0 && decided && !unknown {
                        self.refuse_kinds(name, argc, types);
                    }
                    return;
                }
                let forms: Vec<String> = providers.iter().map(|m| format!("{}.{}[...]", m, name)).collect();
                self.refuse(format!("'{}' is ambiguous — provided by {}; use {}", name, providers.join(" and "), forms.join(" or ")));
                return;
            }
            if let Some(module) = self.ctx.registry.module_name_of(name) {
                if self.ctx.bare_modules.contains(module) {
                    // (a ui control named like a module's function — `select`
                    // of options, not of a frame — is the control's, as the
                    // lowering reads it: checked as frame's select, a page's
                    // dropdown had been refused once the view's facts were
                    // typed)
                    let first_is_frame = types.first().is_some_and(|t| matches!(t.payload(), Ty::Frame(_)));
                    if self.in_ui && crate::lower::is_ui_control(name, first_is_frame) {
                        return;
                    }
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

        // web's modules' names (web.mail's `mail` and `smtp`, web.ws's
        // `socket`, web.auth's `sign`) are theirs: under `use web` alone
        // neither mode had them (the interpreter stopped as it ran, "does
        // not run 'smtp' yet", and the compiled program did not build) —
        // refused where it is written, naming the line that brings them
        if !own && !self.declared(name) && !crate::modules::is_crow_fn(&self.current_fn) {
            let imported = |m: &str| self.v.bare_import_lines.keys().any(|p| p == m) || self.ctx.module_aliases.values().any(|a| a == m);
            let rows_of = |m: &str| crate::table::ROWS.iter().any(|r| r.module == m && r.name == name);
            let web_module = crate::table::ROWS.iter().find(|r| r.name == name && r.module.starts_with("web.")).map(|r| r.module);
            let answered = crate::table::has_core_rows(name) || self.v.bare_import_lines.keys().any(|m| rows_of(m));
            if let Some(m) = web_module.filter(|m| !imported(m)) {
                if !answered {
                    self.refuse_missing_use(name, m, "");
                    return;
                }
            }
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
                    Ty::Result(inner) => *inner,
                    other => other,
                };
                if !matches!(t, Ty::Map(..) | Ty::Unknown) {
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
                    Ty::Result(inner) => *inner,
                    other => other,
                };
                if !matches!(&t, Ty::Map(_, inner) if matches!(inner.as_ref(), Ty::Str | Ty::Unknown)) && !matches!(t, Ty::Unknown) {
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
            if crate::table::has_core_rows(name) || WEB_RESERVED.contains(&name) {
                self.refuse_kinds(name, argc, types);
            }
            if !crate::table::has_core_rows(name) && !crate::table::arity_ok(name, argc) {
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

        // a name an imported module provides, reaching the entry only
        // through a part's import
        if self.v.used.contains(name) && !self.scopes.iter().skip(1).any(|s| s.contains(name)) && !self.v.stored.contains(name) {
            let providers: Vec<String> = self.ctx.registry.providers_of(name).into_iter().map(String::from).collect();
            if !providers.is_empty() && providers.iter().all(|m| self.only_a_part_imports(m)) {
                self.refuse_missing_use(name, &providers[0], "");
                return;
            }
        }
        // A variable holding a function, a list or a map
        if self.defined(name) {
            if let Some(Ty::Fn(params, _)) = self.ctx.vars.get(name).cloned() {
                let expected = params.len();
                if argc != expected {
                    self.refuse(format!("'{}' expects {} argument{}, got {}", name, expected, if expected == 1 { "" } else { "s" }, argc));
                }
                // a function that takes text is handed text — a feed's put
                // among them (the compiled program did not build otherwise)
                for (i, p) in params.iter().enumerate() {
                    let given = match arg_type(self, i) {
                        Ty::Result(inner) => *inner,
                        other => other,
                    };
                    if matches!(p, Ty::Str) && !matches!(given, Ty::Str | Ty::Markup | Ty::Unknown | Ty::Unit) {
                        self.refuse(format!("{}[] takes text, and this is {}: write it into text, {}['`…`']", name, given.crow_name(), name));
                    }
                }
            }
            return;
        }

        // what the body cannot see, said as a read of it is (`xs[i]` of a
        // module variable inside a function had been "undefined function")
        if self.out_of_view(name) {
            return;
        }
        // a stdlib module's function the program does not import says which
        // — both, when two provide it, with what each answers; a name near
        // one in view, or near any module's, says that one
        let providers = stdlib_providers(name);
        match providers.as_slice() {
            [] => {
                let mut candidates: Vec<(String, Option<String>)> = BUILTINS.iter().map(|n| (n.to_string(), None)).collect();
                candidates.extend(self.v.used.iter().map(|n| (n.clone(), None)));
                candidates.extend(self.ctx.functions.keys().filter(|f| self.calls_fn(f)).map(|f| (f.clone(), None)));
                let registry = crate::modules::ModuleRegistry::with_stdlib();
                for module in crate::modules::MODULES.iter().filter(|m| m.registry).map(|m| m.path) {
                    if self.v.bare_import_lines.contains_key(module) {
                        continue;
                    }
                    candidates.extend(registry.functions_of(module).into_iter().map(|f| (f.to_string(), Some(module.to_string()))));
                }
                let near = self.near(name, candidates);
                self.refuse(format!("undefined function '{}'{}", name, near));
            }
            [m] => self.refuse_missing_use(name, m, ""),
            many => {
                let mut many = many.to_vec();
                many.sort();
                let each: Vec<String> = many.iter().map(|m| match answers(m, name) {
                    Some(a) => format!("{}'s ({})", m, a),
                    None => format!("{}'s", m),
                }).collect();
                let uses: Vec<String> = many.iter().map(|m| format!("`use {}`", m)).collect();
                self.refuse(format!("{}[] is {}: add {}", name, listed(&each, "or"), listed(&uses, "or")));
            }
        }
    }

    /// Whether the code being checked is read under imports of its own, not
    /// the entry's: a part's — a declaration or a function the expansion
    /// prefixed with the part's path — or a standard module's function,
    /// inlined, which is read in the module's scope. (A part's own mistakes
    /// are refused when it is validated alone, under its own imports; the
    /// ClickHouse driver's `frame.new[]` had been refused as the entry's, in
    /// an app whose parts alone import frame.)
    fn under_own_imports(&self) -> bool {
        let of = |n: &str| crate::part_prefix_of_declaration(n).is_some();
        self.v.decl.as_deref().is_some_and(of) || of(&self.current_fn) || crate::modules::is_crow_fn(&self.current_fn)
    }

    /// Whether a module's names reach this code only through a part's
    /// import: the entry does not import it, a part does, and this is the
    /// entry's own code — then they are refused for the import, as with no
    /// import at all (spec, Modules: each file's imports are its own).
    pub(super) fn only_a_part_imports(&self, module: &str) -> bool {
        !self.under_own_imports() && crate::PART_ONLY_IMPORTS.with(|p| p.borrow().iter().any(|m| m == module))
    }

    /// A function the program calls from a module it does not import: the
    /// import is asked for once per line, whatever the line calls of the
    /// module (`abs[] and max[] of numbers are math's`), and the language
    /// server offers it as a quick fix. `of` says the arguments, when the
    /// core holds the name for others.
    pub(super) fn refuse_missing_use(&mut self, written: &str, module: &str, of: &str) {
        // (a `use` one slip from the module said it already)
        if self.v.mute > 0 || self.v.meant.contains(module) {
            return;
        }
        let line = self.v.line;
        if let Some((first_line, at, names, first_of)) = self.v.missing_use.get_mut(module) {
            // (another line's call of the module is fixed by the same line)
            if *first_line == line && !names.iter().any(|n| n == written) {
                names.push(written.to_string());
                let calls: Vec<String> = names.iter().map(|n| format!("{}[]", n)).collect();
                let of = if first_of.as_str() == of { of } else { "" };
                let message = format!("line {}: {}{} are {}'s: add `use {}`", line, listed(&calls, "and"), of, module, module);
                if let Some(e) = self.v.errors.get_mut(*at) {
                    e.0 = message;
                }
            }
            return;
        }
        self.refuse(format!("{}[]{} is {}'s: add `use {}`", written, of, module, module));
        let at = self.v.errors.len() - 1;
        self.v.missing_use.insert(module.to_string(), (line, at, vec![written.to_string()], of.to_string()));
    }

    /// ` — did you mean 'len'?` for the name among `candidates` nearest the
    /// one written, when it is near enough to be a slip (one edit in a name
    /// of up to five characters, two in a longer one, a swapped pair one;
    /// none for a name of one or two); a module's name the program does not
    /// import says the import (` (math's: add `use math`)`). Empty
    /// otherwise. A name in view wins a tie with a module's.
    fn near(&self, name: &str, candidates: Vec<(String, Option<String>)>) -> String {
        let limit = match name.chars().count() {
            0..=2 => return String::new(),
            3..=5 => 1,
            _ => 2,
        };
        candidates
            .into_iter()
            .filter(|(c, _)| c != name && !c.starts_with("__"))
            .map(|(c, m)| (slips(name, &c), m.is_some(), c, m))
            .filter(|(d, ..)| *d <= limit)
            .min()
            .map(|(_, _, c, m)| match m {
                Some(m) => format!(" — did you mean '{}'? ({}'s: add `use {}`)", c, m, m),
                None => format!(" — did you mean '{}'?", c),
            })
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

/// The fields of a moment, read of one or over a column of them.
pub(crate) fn moment_field_name(field: &str) -> bool {
    matches!(field, "year" | "month" | "day" | "hour" | "minute" | "second" | "weekday" | "offset")
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

    /// A page's `select` of options and the fact it binds is the control,
    /// beside `use frame` too, where `select` of a frame picks columns (the
    /// dropdown had been checked as frame's select and refused).
    #[test]
    fn a_select_of_options_beside_frame_is_the_control() {
        let source = "use web, ui, frame\n\nranges = ['a', 'b']\nrandom_range = 'a'\n\nview home[]\n    return stack[select[ranges, random_range]]\n\npage['/', home]\nserve[3000]\n";
        assert!(check_source(source).is_ok(), "{:?}", check_source(source));
        let picked = "use frame\nf = frame.new[a = [1], b = [2]]\nlog[select[f, ['a']]]\n";
        assert!(check_source(picked).is_ok(), "{:?}", check_source(picked));
    }

    /// `use web.ui` makes a ui program as `use ui` does: the lowering had
    /// read both and this check `ui` alone, so an element outside a view
    /// and a name ui reserves passed under the longer import.
    #[test]
    fn a_web_ui_import_is_a_ui_program() {
        for import in ["ui", "web.ui"] {
            let outside = format!("use web, {import}\n\nfn home[]\n    return div['x']\n\npage['/', home]\nserve[3000]\n");
            let refused = check_source(&outside).expect_err("an element outside a view is refused");
            assert!(refused.contains("can only be used inside a view block"), "{import}: {refused}");
            let reserved = format!("use web, {import}\n\nfield = 3\nlog[field]\n");
            let refused = check_source(&reserved).expect_err("a name ui reserves is refused");
            assert!(refused.contains("is a ui builtin"), "{import}: {refused}");
        }
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
        assert!(err.contains("in band: 'cfg' is a derivation, which a function does not see — read it in a view, a handler or a route's handler"), "{}", err);
    }

    /// A list holds one kind (spec, Arrays): a literal with text beside a
    /// number is refused before the program runs, in both modes — the
    /// interpreter had made the list, and the compiled program had not
    /// built. A query's parameters are the exception: a list of what each
    /// `?` takes, sent one by one.
    #[test]
    fn a_list_literal_holds_one_kind() {
        let err = check_source("x = ['a', 1, 0.0]\nlog[x]\n").unwrap_err();
        assert!(err.contains("a list holds one kind: this one has text and a number"), "{}", err);
        assert!(check_source("x = [1, 2.5]\nlog[x]\n").is_ok());
        assert!(check_source("use db\nbook = db.connect['sqlite://:memory:']\nexec[book, 'CREATE TABLE t (k TEXT, v REAL)']\nexec[book, 'INSERT INTO t VALUES (?, ?)', ['a', 1.5]]\nlog[query[book, 'SELECT k FROM t WHERE v > ? AND k = ?', [1.0, 'a']]]\n").is_ok());
    }

    /// A frame is indexed by a mask alone, which picks its rows: `f['a']`
    /// and `f[0]` are refused before the program runs, naming the column
    /// read (the interpreter had stopped as it ran — `not indexable` — and
    /// the compiled program did not build)
    #[test]
    fn a_frame_is_indexed_by_a_mask_alone() {
        let err = check_source("use frame\nf = with[frame.new[], 'a', [1.0, 2.0]]\nx = f['a']\nlog[x]\n").unwrap_err();
        assert!(err.contains("a frame is indexed by a mask, which picks its rows (f[f.age gt 30]): read a column as `f.name`, its cell as `f.name[i]`"), "{}", err);
        let err = check_source("use frame\nf = with[frame.new[], 'a', [1.0, 2.0]]\nlog[f[0]]\n").unwrap_err();
        assert!(err.contains("a frame is indexed by a mask"), "{}", err);
        assert!(check_source("use frame\nf = with[frame.new[], 'a', [1.0, 2.0]]\nlog[f.a[0]]\n").is_ok());
        assert!(check_source("use frame\nf = with[frame.new[], 'a', [1.0, 2.0]]\nlog[f[f.a gt 1.0]]\nlog[f.a[f.a gt 1.0]]\n").is_ok());
    }

    /// The order of a list of numbers is a list of ints, and its running
    /// sum a vector, each indexed as one: inside a function, where the
    /// list is a parameter or a loop's items, the order's items had read
    /// as floats (indexing nothing) and the sum had no rank, so the index
    /// was refused before the program ran
    #[test]
    fn an_order_of_a_list_is_a_vector() {
        for src in [
            "use math\nfn first[vals]\n    order = argsort[vals]\n    return vals[order[0]]\nlog[first[[3.0, 1.0, 2.0]]]\n",
            "use math\nfn by_column[rows, f]\n    col = for r in rows\n        r[f]\n    order = argsort[col]\n    return col[order[0]]\nlog[by_column[[[1.0, 9.0], [2.0, 1.0]], 1]]\n",
            "use math\nfn total_of[vals]\n    c = cumsum[vals]\n    return c[len[vals] - 1]\nlog[total_of[[3.0, 1.0, 2.0]]]\n",
        ] {
            let checked = check_source(src);
            assert!(checked.is_ok(), "{:?}", checked);
        }
    }

    /// The greatest of a list of numbers computed inside a function is
    /// math's `max` of the whole list: the list had been left to the core's
    /// two-number `max`, and the call refused for its count (a literal
    /// list at the top level, a tensor, had always been math's)
    #[test]
    fn the_greatest_of_a_list_is_maths() {
        for src in [
            "use math\nfn top[xs]\n    best = for v in xs\n        v * 1.0\n    return max[best]\nlog[top[[4.0, 2.0, 9.0]]]\n",
            "use math\nfn widest[rows]\n    widths = for r in rows\n        len[r]\n    return max[widths]\nlog[widest[['ab', 'a']]]\n",
            "use math\nfn bottom[xs]\n    return min[xs]\nlog[bottom[[4, 2, 9]]]\n",
        ] {
            let checked = check_source(src);
            assert!(checked.is_ok(), "{:?}", checked);
        }
        assert!(check_source("use math\nx = min[1]\n").unwrap_err().contains("min[] takes"));
    }

    /// A nested literal of numbers is a matrix: a ragged one is refused
    /// before the program runs, in both modes — one with an empty row among
    /// rows of numbers had run interpreted as a list of lists and failed the
    /// compiled build; rows of two lengths had stopped as the program ran
    #[test]
    fn a_ragged_literal_of_numbers_is_refused() {
        for src in ["x = [[1, 2], [3]]\nlog[x]\n", "x = [[1, 2], [2], []]\nlog[x]\n", "x = [[1.5, 2.5], []]\nlog[x]\n"] {
            let err = check_source(src).unwrap_err();
            assert!(err.contains("all inner arrays must be the same length"), "{}", err);
        }
        for src in ["x = [[1, 2], [3, 4]]\nlog[x]\n", "x = [['a'], ['b', 'c']]\nlog[x]\n", "x = [[], []]\nlog[len[x]]\n"] {
            let checked = check_source(src);
            assert!(checked.is_ok(), "{:?}", checked);
        }
    }

    /// A tensor's item assigned in both branches of an `if` inside a loop,
    /// then pushed onto a list, reordered through a helper and read back as
    /// a tensor: the variable stays the tensor it was given (an element
    /// assignment types a list's unknown items, never a wholly unknown
    /// variable — a guess of a list had stuck to the helper the list was
    /// handed to, and a simplex's points read as lists of floats)
    #[test]
    fn a_tensor_takes_an_item() {
        let src = "use math\nfn reordered[xs, order]\n    out = []\n    for i in order\n        out = push[out, xs[i]]\n    return out\nfn simplex[x0]\n    n = len[x0]\n    sim = []\n    sim = push[sim, x0]\n    for k in range[n]\n        y = x0\n        if y[k] ne 0.0\n            y[k] = 1.05 * y[k]\n        else\n            y[k] = 0.00025\n        sim = push[sim, y]\n    order = argsort[[2.0, 1.0, 3.0]]\n    sim = reordered[sim, order]\n    rounds = 0\n    while rounds lt 2\n        xr = 2.0 * sim[0] - sim[n]\n        sim[n] = xr\n        rounds = rounds + 1\n    return sim[n] * 2.0\nlog[simplex[tensor[[1.0, 2.0]]]]\n";
        let checked = check_source(src);
        assert!(checked.is_ok(), "{:?}", checked);
    }

    /// A gradient handed as a value and handed on, with the function it is
    /// of called only through those hands, and only from a test: the
    /// parameters take the gradient's tensor type (a function only a test
    /// hands to `grad` is one `grad` differentiates, and a gradient of a
    /// function not typed yet is not a function of floats)
    #[test]
    fn a_gradient_handed_from_a_test_types_its_function() {
        let src = "use math\nfn booth[x]\n    e0 = tensor[[1.0, 0.0]]\n    e1 = tensor[[0.0, 1.0]]\n    a = sum[e0 .* x]\n    b = sum[e1 .* x]\n    return (a + 2.0 * b - 7.0) ^ 2 + (2.0 * a + b - 5.0) ^ 2\nfn probe[f, df, xk, pk, gfk]\n    derphi0 = dot[gfk, pk]\n    at = xk + 0.5 * pk\n    gval = df[at]\n    return dot[gval, pk] + derphi0 + f[at]\nfn steps[f, df, x0]\n    gfk = df[x0]\n    pk = 0.0 - gfk\n    d = probe[f, df, x0, pk, gfk]\n    gfk = df[x0 + 0.01 * pk]\n    return d + max[abs[gfk]]\ntest relays_a_gradient[]\n    assert[steps[booth, grad[booth], tensor[[0.0, 0.0]]] gt 0.0]\n";
        let checked = check_source(src);
        assert!(checked.is_ok(), "{:?}", checked);
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
        let src = "use frame\na = with[with[frame.new[], 'k', [0, 1]], 'p', [1.5, 2.5]]\nb = with[with[frame.new[], 'k', [0]], 'q', [7.5]]\nj = join[a, b, 'k', 'left']\nkeep = for i in range[len[j]]\n    j.q[i] gt 1.0\nlog[len[j[keep]]]\n";
        let err = check_source(src).unwrap_err();
        assert!(err.contains("'keep' holds values that can fail, and a mask decides every row: guard each value with .ok (cells of a column that can be missing: fill the column first, fill[frame, 'column', value])"), "{}", err);
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

    /// A cell of a loaded frame is what the program reads it as: multiplied
    /// by a float it is a float, and the frame is held to that where it is
    /// loaded (uses.rs; it had been a parsed value, read first by hand). A
    /// parsed value's member is read as its use implies: a number here.
    #[test]
    fn a_cell_is_what_the_program_reads_and_a_parsed_value_what_its_use_implies() {
        assert!(check_source("use frame\ndf = load['nowhere.csv']\nlog[df.px[0] * 2.0]\n").is_ok());
        assert!(check_source("use frame\ndf = load['nowhere.csv']\nlog[float[df.px[0]] * 2.0]\n").is_ok());
        assert!(check_source("d = json['{\"px\": 2}']\nlog[d.px * 2.0]\n").is_ok());
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
        // a parsed value assigned to a local of a kind is read as that kind
        assert!(check_source("fn f[s]\n    x = ''\n    x = json[s]\n    return x\nlog[f['1']]\n").is_ok());
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
        assert!(check_source("x = min['a', 'b']\n").unwrap_err().contains("min[] takes an int and an int, a number and a number, a decimal and a decimal, a fraction and a fraction or a moment and a moment, got text and text"));
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
        assert!(check_source("base = 10\nfn f[x]\n    return x + base\nr = f[1]\n").unwrap_err().contains("'base' is a module variable, which a function does not see — pass it as a parameter"));
    }

    #[test]
    fn test_fn_local_reusing_module_name_ok() {
        let result = check_source("count = 5\nfn f[n]\n    count = n * 2\n    return count\nr = f[3]\n");
        assert!(result.is_ok(), "local reusing module name should validate: {:?}", result);
    }

    #[test]
    fn test_map_hetero_set_rejected_at_compile_time() {
        // Maps are dictionaries: set[] changing value type is refused
        assert!(check_source("m = map[a = 1]\nm2 = set[m, 'b', 'x']\n").unwrap_err().contains("a map holds values of one kind: the entry 'b' holds text, and the map's other values are an int"), "{}", check_source("m = map[a = 1]\nm2 = set[m, 'b', 'x']\n").unwrap_err());
    }

    /// A collection builtin given a kind none of its rows takes is refused
    /// before either mode runs, naming the forms it takes (compiled, these
    /// did not build; interpreted, they stopped as they ran).
    /// A map's keys are one kind: a key of the other, a float key, and the
    /// dot form on a map keyed by ints are refused before the program runs.
    #[test]
    fn a_maps_keys_are_one_kind() {
        let e = check_source("m = map[[1, 2], ['a', 'b']]\nlog[m['2']]\n").unwrap_err();
        assert!(e.contains("the map's keys are ints, not text"), "{}", e);
        let e = check_source("t = map[a = 1]\nt = set[t, 2, 9]\n").unwrap_err();
        assert!(e.contains("the map's keys are text, not an int"), "{}", e);
        let e = check_source("m = map[[1, 2], ['a', 'b']]\nlog[m.a]\n").unwrap_err();
        assert!(e.contains("a map keyed by ints is read by `m[2]`"), "{}", e);
        let e = check_source("m = map[[1.5, 2.5], ['a', 'b']]\nlog[m]\n").unwrap_err();
        assert!(e.contains("not floats"), "{}", e);
        // an open map takes its first key's kind; a literal's int keys key it
        assert!(check_source("m = map[]\nm = set[m, 7, 'x']\nlog[m[7]]\nlog[2 in m]\n").is_ok());
        assert!(check_source("m = map[2 = 'b', 1 = 'a']\nlog[keys[m]]\nlog[m[1]]\n").is_ok());
        assert!(check_source("m = map[[1, 2], ['a', 'b']]\nlog[2 in m]\nlog['2' in m]\n").is_err());
    }

    #[test]
    fn a_collection_given_a_wrong_kind_is_refused() {
        // (a key of the other kind than the map's: refused for the kind, not the row)
        assert!(check_source("log[1 in map[a = 1]]\n").unwrap_err().contains("the map's keys are text, not an int"), "{}", check_source("log[1 in map[a = 1]]\n").unwrap_err());
        assert!(check_source("log[len[5]]\n").unwrap_err().contains("len[] takes a list, text, a map, numbers, a frame or bytes, got an int"));
        let sorted = check_source("struct P [x, y]\nlog[sort[[P[3, 4], P[1, 2]]]]\n").unwrap_err();
        assert!(sorted.contains("sort[] takes a list of ints, a list of numbers, a list of text or numbers, got a list of records"), "{}", sorted);
        assert!(check_source("log[range[2.5]]\n").unwrap_err().contains("range[] takes an int, got a number"));
        // a map's membership is the operator's (`has` retired into `in`)
        assert!(check_source("m = map[a = 1]\nlog['a' in m]\n").is_ok());
    }

    /// A parsed JSON value is read as one type before a builtin computes
    /// with it, in both modes; the builtins that navigate one take it, and a
    /// loop walks a parsed list. A module function given a kind none of its
    /// rows take is refused as a core builtin is (the interpreter stopped on
    /// the value, the compiled program did not build).
    /// A predicate handed to any[], all[] or find[] is a function: a frame
    /// a variable named like the predicate held is refused before either
    /// mode runs (the interpreter had stopped as it ran, the compiled
    /// program had not built — fuzz seed 506 of the widened generator).
    #[test]
    fn a_predicate_that_is_not_a_function_is_refused_alike() {
        let err = check_source("use frame\nfn p1[x]\n    return x % 2 eq 0\nfn case[]\n    p1 = with[frame.new[], 'a', [1, 2]]\n    log[all[[2, 4], p1]]\n    return 0\nr = case[]\n").unwrap_err();
        assert!(err.contains("all[] takes a list and a function, got numbers and a frame"), "{}", err);
        assert!(check_source("fn p1[x]\n    return x % 2 eq 0\nlog[all[[2, 4], p1]]\nlog[any[[1, 3], p1]]\n").is_ok());
    }

    #[test]
    fn a_parsed_value_is_read_before_a_builtin_computes_with_it() {
        // a parsed value handed to a builtin whose rows agree on text is read as text
        assert!(check_source("j = json['\"a\"']\nlog[upper[j]]\n").is_ok());
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
        assert!(err.contains("in show: 'amount' is a module variable, which a function does not see — pass it as a parameter"), "{}", err);
    }

    /// A route's handler is handed its request alone, so the advice for a
    /// module variable it reads is to open what it needs inside it — not a
    /// parameter no route can pass, nor wiring it is already given
    #[test]
    fn a_route_handler_reading_a_module_variable_is_told_to_open_it_itself() {
        let src = "use web, db\nconn = connect['sqlite://app.db']\nfn count[req]\n    q = query[conn, 'SELECT 1 AS one']\n    return text['one']\nroute['/count', count]\nroute['/n', 'GET', count, 3]\nserve[3000]\n";
        let err = check_source(src).unwrap_err();
        assert!(err.contains("in count: 'conn' is a module variable, which a route's handler does not see — the route hands it its request alone: open what it needs inside it"), "{}", err);
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
pub(crate) fn item_kind(t: &Ty) -> Option<(String, String)> {
    Some(match t.payload() {
        Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction => ("numbers".to_string(), "a number".to_string()),
        Ty::Str | Ty::Markup => ("text".to_string(), "text".to_string()),
        Ty::Bool => ("bools".to_string(), "a bool".to_string()),
        Ty::Record(n) | Ty::OptRecord(n) => (format!("{} records", crate::lower::shown(n)), format!("a {} record", crate::lower::shown(n))),
        // (a list's items say its kind, as `compatible` reads it: a tensor
        // and a list of numbers are one kind; a list of unknown items is
        // none, and a push onto it is not checked — a list of floats beside
        // a list of tensors had passed as "lists", and the verifier caught
        // the clone it mistyped: the gpt package's fit)
        Ty::Tensor(..) => ("lists of numbers".to_string(), "a list of numbers".to_string()),
        Ty::List(inner) => {
            let (many, _) = item_kind(inner)?;
            (format!("lists of {}", many), format!("a list of {}", many))
        }
        Ty::Map(..) => ("maps".to_string(), "a map".to_string()),
        Ty::Frame(_) => ("frames".to_string(), "a frame".to_string()),
        // (a moment and a frame's column are kinds of their own: pushed onto
        // a list of numbers, each had made a list of two kinds — crow check
        // --self)
        Ty::Moment => ("moments".to_string(), "a moment".to_string()),
        Ty::Column(_) => ("frame's columns".to_string(), "a frame's column".to_string()),
        // (the layer's bytes: pushed onto a list of ints, the compiled
        // program had not built — crow check --self)
        Ty::Bytes => ("bytes".to_string(), "bytes".to_string()),
        _ => return None,
    })
}

fn order_side(t: &Ty) -> Option<OrderSide> {
    Some(match t {
        Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction => OrderSide::One("a number"),
        Ty::Str | Ty::Markup => OrderSide::One("text"),
        Ty::Bool => OrderSide::One("a bool"),
        Ty::Moment => OrderSide::One("a moment"),
        Ty::Result(x) => return order_side(x),
        Ty::Tensor(..) => OrderSide::Many("a number"),
        Ty::List(item) | Ty::Column(item) => match order_side(item)? {
            OrderSide::One(k) => OrderSide::Many(k),
            OrderSide::Many(_) => OrderSide::Many("a list"),
        },
        Ty::Map(..) => OrderSide::One("a map"),
        Ty::Record(_) | Ty::OptRecord(_) => OrderSide::One("a record"),
        Ty::Frame(_) => OrderSide::One("a frame"),
        Ty::Node => OrderSide::One("markup"),
        _ => return None,
    })
}
