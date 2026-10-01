//! The app baseline: what a program does, derived from its source in a
//! canonical form, kept under `.crow/baseline/<entry>/` and diffed on
//! every `crow check` (docs/app-baseline-design.md).
//!
//! A refactor leaves the artifacts unchanged — locals are substituted by
//! their definitions, parameters are numbered, paths are sorted by their
//! conditions — and a change in behaviour or composition shows as a diff
//! with its cause (the file) and its reach (the views and pages it can
//! affect). Nobody writes a rule; intent is the accepted baseline.

use crate::ast::{BinOp, Expr, Stmt, UnaryOp};
use crate::type_infer::TypeContext;
use std::collections::{BTreeMap, BTreeSet};

/// Builtins whose call is an effect on the world outside the program.
const EFFECTS: &[&str] = &[
    "exec", "insert", "transact", "later", "fetch", "shell", "write", "save", "mail", "send", "post", "put", "delete", "remove",
    "notice", "sign_in", "sign_out", "download",
];
/// Events a control declares in a view.
const EVENTS: &[&str] = &["click", "input", "submit", "change", "seek"];
/// Elements whose second argument is the fact they bind.
const BOUND: &[&str] = &["field", "slider", "checkbox", "toggle", "radio", "tabs", "select"];
/// Containers that show under an `open` attribute.
const OPENERS: &[&str] = &["modal", "drawer", "toast", "details"];
const INLINE_DEPTH: usize = 4;

/// The derived artifacts: one canonical text per declaration, keyed
/// `kind name` (`handler sim_session_start_session`).
#[derive(Debug, Default, Clone)]
pub struct Artifacts {
    pub entries: BTreeMap<String, String>,
    /// declaration → project-relative file
    pub files: BTreeMap<String, String>,
    /// declaration → the views and pages it reaches
    pub reach: BTreeMap<String, Vec<String>>,
    /// the app as a tree: pages, composition under guards, controls
    pub structure: Structure,
}

/// One node of a view's tree, kept beside its text so a wireframe can be
/// laid out from it: the element, its literal attributes, its text with
/// placeholders, its children; `@` names a sub-view, `for` a repeated body.
#[derive(Debug, Default, Clone)]
pub struct WNode {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub kids: Vec<WNode>,
    pub view: String,
}

/// The shape of the app, for `crow check --show`.
#[derive(Debug, Default, Clone)]
pub struct Structure {
    pub pages: BTreeMap<String, String>,
    /// view → (guard, child view), in order of appearance
    pub composes: BTreeMap<String, Vec<(String, String)>>,
    /// view → the guard of every path through it
    pub view_paths: BTreeMap<String, BTreeSet<String>>,
    /// view → (guard, tree) per path, for the wireframes
    pub wires: BTreeMap<String, Vec<(String, WNode)>>,
    /// view → (control, handler)
    pub controls: BTreeMap<String, Vec<(String, String)>>,
    /// view → facts its inputs bind
    pub binds: BTreeMap<String, BTreeSet<String>>,
    /// handler → facts it writes (certainly or maybe)
    pub writes: BTreeMap<String, BTreeSet<String>>,
    /// handler → effects on any path
    pub effects: BTreeMap<String, BTreeSet<String>>,
    /// fact → (bound by, written by, read by)
    pub facts: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>)>,
    pub derives: BTreeSet<String>,
    pub parts: BTreeSet<String>,
}

/// One difference between the accepted baseline and the program.
#[derive(Debug, Clone)]
pub struct Change {
    pub key: String,
    pub file: String,
    pub was: String,
    pub now: String,
    pub reach: Vec<String>,
}

/// What a check found beside the changes.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub first: bool,
    pub dir: String,
    pub recorded: usize,
    pub unchanged: usize,
    pub changed: usize,
    pub removed: usize,
    pub added: usize,
}

/// A kernel rule over the wiring: not a change, a shape nobody wants.
#[derive(Debug, Clone)]
pub struct Rule {
    pub subject: String,
    pub file: String,
    pub detail: String,
    pub fix: String,
}

// ---------------------------------------------------------------- program model

struct Model<'a> {
    stmts: &'a [Stmt],
    facts: BTreeSet<String>,
    derives: BTreeSet<String>,
    views: BTreeSet<String>,
    handlers: BTreeSet<String>,
    fns: BTreeMap<String, (Vec<String>, Vec<Stmt>)>,
    parts: BTreeMap<String, String>,
    entry: String,
}

impl<'a> Model<'a> {
    fn file_of(&self, name: &str) -> String {
        let mut best: Option<(&String, &String)> = None;
        for (prefix, path) in &self.parts {
            if name.starts_with(prefix.as_str()) && best.map_or(true, |(p, _)| prefix.len() > p.len()) {
                best = Some((prefix, path));
            }
        }
        best.map(|(_, p)| p.clone()).unwrap_or_else(|| self.entry.clone())
    }

    fn is_fact(&self, n: &str) -> bool { self.facts.contains(n) }
    fn is_view(&self, n: &str) -> bool { self.views.contains(n) }
    fn writes_state(&self, n: &str) -> bool { self.handlers.contains(n) && !self.views.contains(n) }
}

// ---------------------------------------------------------------- canonical form

fn op_str(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*", BinOp::Div => "/", BinOp::Mod => "%", BinOp::Pow => "^",
        BinOp::ElemMul => ".*", BinOp::ElemDiv => "./", BinOp::ElemPow => ".^",
        BinOp::Eq => "eq", BinOp::Ne => "neq", BinOp::Gt => "gt", BinOp::Lt => "lt", BinOp::Gte => "gte", BinOp::Lte => "lte",
        BinOp::And => "and", BinOp::Or => "or",
    }
}

/// The environment of a walk: a local's canonical text, a parameter's
/// number. Facts and derivations print as themselves.
#[derive(Clone, Default)]
struct Env {
    locals: BTreeMap<String, String>,
    /// a local holding a tree, as a node, on this path (a path's own: the
    /// branch that reassigns `body` does not reassign its sibling's)
    nodes: BTreeMap<String, WNode>,
}

fn fmt_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 { format!("{:.1}", f) } else { format!("{}", f) }
}

/// A string literal with its interpolation spans canonicalised: a local
/// read inside a span prints as what it holds.
fn canon_str(s: &str, env: &Env) -> String {
    if !s.contains('`') { return format!("'{}'", s); }
    let mut out = String::new();
    let mut in_span = false;
    let mut span = String::new();
    for c in s.chars() {
        if c == '`' {
            if in_span {
                out.push('`');
                out.push_str(&canon_span(&span, env));
                out.push('`');
                span.clear();
            }
            in_span = !in_span;
            continue;
        }
        if in_span { span.push(c) } else { out.push(c) }
    }
    format!("'{}'", out)
}

/// Identifiers inside a span that name locals are replaced by their text.
fn canon_span(span: &str, env: &Env) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() { return; }
        match env.locals.get(word.as_str()) {
            Some(v) => out.push_str(v),
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in span.chars() {
        if c.is_alphanumeric() || c == '_' { word.push(c); } else { flush(&mut word, &mut out); out.push(c); }
    }
    flush(&mut word, &mut out);
    out
}

fn canon(e: &Expr, env: &Env) -> String {
    match e {
        Expr::Int(n) => n.to_string(),
        Expr::Float(f) => fmt_float(*f),
        Expr::Bool(b) => b.to_string(),
        Expr::Str(s) => canon_str(s, env),
        Expr::Ident(n) => env.locals.get(n).cloned().unwrap_or_else(|| n.clone()),
        Expr::BinOp { left, op, right } => format!("({} {} {})", canon(left, env), op_str(op), canon(right, env)),
        Expr::UnaryOp { op, expr } => match op {
            UnaryOp::Not => format!("(not {})", canon(expr, env)),
            UnaryOp::Neg => format!("(-{})", canon(expr, env)),
        },
        // a read a guard narrowed reads as the name the program wrote
        Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(k) if k == crate::lower::KNOWN) && args.len() == 1 => canon(&args[0], env),
        Expr::Call { callee, args } => {
            let c = canon(callee, env);
            let a: Vec<String> = args.iter().map(|x| canon(x, env)).collect();
            format!("{}[{}]", c, a.join(", "))
        }
        Expr::FieldAccess { object, field } => format!("{}.{}", canon(object, env), field),
        Expr::Array(items) => format!("[{}]", items.iter().map(|x| canon(x, env)).collect::<Vec<_>>().join(", ")),
        Expr::Map(pairs) => {
            let mut ps: Vec<String> = pairs.iter().map(|(k, v)| format!("{} = {}", canon(k, env), canon(v, env))).collect();
            ps.sort();
            format!("map[{}]", ps.join(", "))
        }
        Expr::Named { name, value } => format!("{} = {}", name, canon(value, env)),
        Expr::ForExpr { var, iter, body } => {
            let mut inner = env.clone();
            inner.locals.insert(var.clone(), format!("each[{}]", canon(iter, env)));
            let last = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
            let value = match last {
                Some(Stmt::ExprStmt(e)) | Some(Stmt::Return(Some(e))) => canon(e, &inner),
                _ => "...".to_string(),
            };
            format!("for[each[{}] -> {}]", canon(iter, env), value)
        }
    }
}

fn callee_name(callee: &Expr) -> Option<String> {
    match callee {
        Expr::Ident(n) => Some(n.clone()),
        Expr::FieldAccess { object, field } if matches!(object.as_ref(), Expr::Ident(_)) => Some(field.clone()),
        _ => None,
    }
}

/// The element, control or chart a call draws: its name written bare
/// (`stack[…]`), or under `ui` — never a value's field, whose name may be
/// an element's (`card.title[0]` reads a column, since `title` is one).
fn drawn_name(callee: &Expr) -> Option<String> {
    let name = match callee {
        Expr::Ident(n) => n.clone(),
        Expr::FieldAccess { object, field } if matches!(object.as_ref(), Expr::Ident(m) if m == "ui") => field.clone(),
        _ => return None,
    };
    (crate::lower::UI_ELEMENTS.contains(&name.as_str()) || BOUND.contains(&name.as_str()) || name == "chart" || name == "table").then_some(name)
}

// ---------------------------------------------------------------- signatures

/// Calls whose value is a read of the world: named `q1`, `q2`, ... on the
/// path so a condition on a query's cell stays readable.
const READS: &[&str] = &["query", "exec", "insert", "fetch", "shell", "request", "transact", "now", "rand", "random", "uuid", "input", "moment"];

#[derive(Clone, Default, Debug)]
struct Path {
    conds: Vec<String>,
    reads: Vec<String>,
    writes: BTreeMap<String, String>,
    effects: Vec<String>,
    calls: BTreeSet<String>,
    may: BTreeSet<String>,
    returns: Option<String>,
    next_q: usize,
}

impl Path {
    /// This path continued by another's: conditions and reads accumulate,
    /// later writes win, effects keep their order.
    fn then(&self, sub: Path) -> Path {
        let mut out = self.clone();
        out.conds.extend(sub.conds);
        out.reads.extend(sub.reads);
        for (f, v) in sub.writes { out.writes.insert(f, v); }
        out.effects.extend(sub.effects);
        out.calls.extend(sub.calls);
        out.may.extend(sub.may);
        out.next_q = sub.next_q.max(out.next_q);
        out
    }
}

fn negate(c: &str) -> String {
    if let Some(inner) = c.strip_prefix("(not ").and_then(|r| r.strip_suffix(')')) {
        inner.to_string()
    } else {
        format!("(not {})", c)
    }
}

struct Walker<'m, 'a> {
    m: &'m Model<'a>,
    out: Vec<(Path, Env)>,
    in_loop: usize,
    inlined: Vec<String>,
}

impl<'m, 'a> Walker<'m, 'a> {
    fn sub(&self) -> Walker<'m, 'a> {
        Walker { m: self.m, out: Vec::new(), in_loop: self.in_loop, inlined: self.inlined.clone() }
    }

    fn effect_of(&self, name: &str, args: &[Expr], env: &Env) -> Option<String> {
        if !EFFECTS.contains(&name) { return None; }
        let a: Vec<String> = args.iter().map(|x| canon(x, env)).collect();
        Some(format!("effect {} {}", name, a.join(" ")))
    }

    /// Walk a statement list on one path; forks at `if` and at a call into
    /// a helper with several paths; ends at `return`.
    fn walk(&mut self, stmts: &[Stmt], mut env: Env, mut path: Path) {
        let mut i = 0;
        while i < stmts.len() {
            let st = &stmts[i];
            i += 1;
            let rest = &stmts[i..];
            match st {
                Stmt::Line(_) | Stmt::Use { .. } | Stmt::StructDecl { .. } | Stmt::Resource { .. } | Stmt::TestBlock { .. } | Stmt::Derive { .. } | Stmt::FnDecl { .. } => {}
                Stmt::Assign { name, value } => self.assign(name, value, &mut env, &mut path),
                Stmt::MultiAssign { names, values } => {
                    // every right side reads the state before any left side moves
                    let texts: Vec<String> = values.iter().map(|v| canon(v, &env)).collect();
                    for (n, t) in names.iter().zip(texts) { self.bind(n, t, &mut env, &mut path); }
                }
                Stmt::FieldAssign { object, field, value } => {
                    if let Expr::Ident(n) = object {
                        let t = format!("{}.{} := {}", canon(object, &env), field, canon(value, &env));
                        if self.m.is_fact(n) { self.record_write(n, t, &mut path); }
                    }
                }
                Stmt::IndexAssign { object, index, value } => {
                    if let Expr::Ident(n) = object {
                        let t = format!("{}[{}] := {}", canon(object, &env), canon(index, &env), canon(value, &env));
                        if self.m.is_fact(n) { self.record_write(n, t, &mut path); }
                    }
                }
                Stmt::ExprStmt(e) => {
                    if let Some(forks) = self.inline_call(e, &env, &path) {
                        for (p, en) in forks { self.walk(rest, en, p); }
                        return;
                    }
                    self.expr_stmt(e, &env, &mut path);
                }
                Stmt::Return(v) => {
                    // a helper called as the last statement is a call for effect
                    if let Some(e) = v {
                        if let Some(forks) = self.inline_call(e, &env, &path) {
                            for (p, en) in forks { self.out.push((p, en)); }
                            return;
                        }
                        if let Expr::Call { .. } = e {
                            let mut p = path.clone();
                            self.expr_stmt(e, &env, &mut p);
                            if p.effects.len() != path.effects.len() || p.calls.len() != path.calls.len() {
                                self.out.push((p, env));
                                return;
                            }
                        }
                    }
                    path.returns = v.as_ref().map(|e| canon(e, &env));
                    self.out.push((path, env));
                    return;
                }
                Stmt::Break | Stmt::Continue => {}
                Stmt::If { condition, then_body, else_body } => {
                    let c = canon(condition, &env);
                    // a guard the path already decides (a fact a helper just
                    // set) is no fork: one branch is dead
                    if c == "true" || c == "false" {
                        let mut s: Vec<Stmt> = if c == "true" { then_body.clone() } else { else_body.clone().unwrap_or_default() };
                        s.extend(rest.iter().cloned());
                        self.walk(&s, env, path);
                        return;
                    }
                    if self.in_loop > 0 {
                        // inside a loop every branch may run: take both, no fork
                        let mut all: Vec<Stmt> = then_body.clone();
                        if let Some(eb) = else_body { all.extend(eb.iter().cloned()); }
                        all.extend(rest.iter().cloned());
                        self.walk(&all, env, path);
                        return;
                    }
                    let mut p1 = path.clone();
                    p1.conds.push(c.clone());
                    let mut s1: Vec<Stmt> = then_body.clone();
                    s1.extend(rest.iter().cloned());
                    self.walk(&s1, env.clone(), p1);
                    let mut p2 = path;
                    p2.conds.push(negate(&c));
                    let mut s2: Vec<Stmt> = else_body.clone().unwrap_or_default();
                    s2.extend(rest.iter().cloned());
                    self.walk(&s2, env, p2);
                    return;
                }
                Stmt::For { var, iter, body } => {
                    let mut inner = env.clone();
                    inner.locals.insert(var.clone(), format!("each[{}]", canon(iter, &env)));
                    self.in_loop += 1;
                    let mut sub = self.sub();
                    let mut start = Path::default();
                    start.next_q = path.next_q;
                    sub.walk(body, inner, start);
                    self.in_loop -= 1;
                    self.absorb_loop(&mut path, sub.out);
                }
                Stmt::While { condition, body } => {
                    let c = canon(condition, &env);
                    self.in_loop += 1;
                    let mut sub = self.sub();
                    let mut start = Path::default();
                    start.next_q = path.next_q;
                    sub.walk(body, env.clone(), start);
                    self.in_loop -= 1;
                    path.may.insert(format!("while {}", c));
                    self.absorb_loop(&mut path, sub.out);
                }
            }
        }
        self.out.push((path, env));
    }

    /// A loop body's writes and effects may happen any number of times.
    fn absorb_loop(&mut self, path: &mut Path, paths: Vec<(Path, Env)>) {
        for (p, _) in paths {
            for r in p.reads { path.may.insert(format!("read {}", r)); }
            for (f, v) in p.writes { path.may.insert(format!("{} := {}", f, v)); }
            for e in p.effects { path.may.insert(e); }
            for c in p.calls { path.calls.insert(c); }
            for m in p.may { path.may.insert(m); }
            path.next_q = path.next_q.max(p.next_q);
        }
    }

    /// A call into a helper that writes facts: its paths continue this one.
    /// None when the call is not such a helper (or inlining is exhausted).
    fn inline_call(&mut self, e: &Expr, env: &Env, path: &Path) -> Option<Vec<(Path, Env)>> {
        let Expr::Call { callee, args } = e else { return None };
        let name = callee_name(callee)?;
        if EFFECTS.contains(&name.as_str()) || self.m.is_view(&name) || !self.m.writes_state(&name) { return None; }
        if self.inlined.len() >= INLINE_DEPTH || self.inlined.contains(&name) { return None; }
        let (params, body) = self.m.fns.get(&name).cloned()?;
        // the helper reads the facts as this path left them; its parameters
        // are the arguments; its own locals stay its own
        let mut inner = Env::default();
        for (k, v) in &env.locals { if self.m.is_fact(k) { inner.locals.insert(k.clone(), v.clone()); } }
        for (p, a) in params.iter().zip(args.iter()) { inner.locals.insert(p.clone(), canon(a, env)); }
        let mut sub = self.sub();
        sub.inlined.push(name.clone());
        let mut start = Path::default();
        start.next_q = path.next_q;
        sub.walk(&body, inner, start);
        let mut forks = Vec::new();
        for (sp, senv) in sub.out {
            let merged = path.then(sp);
            let mut en = env.clone();
            for (k, v) in senv.locals { if self.m.is_fact(&k) { en.locals.insert(k, v); } }
            forks.push((merged, en));
        }
        Some(forks)
    }

    fn assign(&mut self, name: &str, value: &Expr, env: &mut Env, path: &mut Path) {
        if let Expr::Call { callee, args } = value {
            if let Some(n) = callee_name(callee) {
                if let Some(e) = self.effect_of(&n, args, env) { self.record_effect(e, path); }
                if READS.contains(&n.as_str()) || (self.m.fns.contains_key(&n) && !self.m.is_view(&n)) {
                    // the value is a symbol; the path lists what it stands for
                    path.next_q += 1;
                    let sym = format!("v{}", path.next_q);
                    let line = format!("{} = {}", sym, canon(value, env));
                    if self.in_loop > 0 { path.may.insert(format!("read {}", line)); } else { path.reads.push(line); }
                    self.bind(name, sym, env, path);
                    return;
                }
            }
        }
        let t = canon(value, env);
        self.bind(name, t, env, path);
    }

    fn bind(&mut self, name: &str, text: String, env: &mut Env, path: &mut Path) {
        if self.m.is_fact(name) {
            self.record_write(name, text.clone(), path);
        }
        // a later read of the name sees what was assigned, fact or local
        env.locals.insert(name.to_string(), text);
    }

    fn record_write(&mut self, fact: &str, text: String, path: &mut Path) {
        if self.in_loop > 0 { path.may.insert(format!("{} := {}", fact, text)); } else { path.writes.insert(fact.to_string(), text); }
    }

    fn record_effect(&mut self, text: String, path: &mut Path) {
        if self.in_loop > 0 { path.may.insert(text); } else { path.effects.push(text); }
    }

    fn expr_stmt(&mut self, e: &Expr, env: &Env, path: &mut Path) {
        let Expr::Call { callee, args } = e else { return };
        let Some(name) = callee_name(callee) else { return };
        if let Some(eff) = self.effect_of(&name, args, env) {
            self.record_effect(eff, path);
            // later[f]: the deferred half runs with the state it finds
            if name == "later" {
                if let Some(Expr::Ident(f)) = args.first() { path.calls.insert(format!("later {}", f)); }
            }
            return;
        }
        if self.m.fns.contains_key(&name) && !self.m.is_view(&name) {
            path.calls.insert(format!("{}[{}]", name, args.iter().map(|a| canon(a, env)).collect::<Vec<_>>().join(", ")));
        }
    }
}

/// Every path through a handler, canonical and sorted.
fn signature(m: &Model, name: &str, params: &[String], body: &[Stmt]) -> Vec<Path> {
    let mut env = Env::default();
    for (i, p) in params.iter().enumerate() {
        env.locals.insert(p.clone(), format!("${}", i + 1));
    }
    let mut w = Walker { m, out: Vec::new(), in_loop: 0, inlined: vec![name.to_string()] };
    w.walk(body, env, Path::default());
    let mut paths: Vec<Path> = w.out.into_iter().map(|(p, _)| p).collect();
    for p in &mut paths { p.conds.sort(); p.conds.dedup(); }
    paths.sort_by(|a, b| a.conds.join(" and ").cmp(&b.conds.join(" and ")));
    paths.dedup_by(|a, b| a.conds == b.conds && a.writes == b.writes && a.effects == b.effects && a.may == b.may && a.calls == b.calls && a.reads == b.reads);
    paths
}

fn render_paths(paths: &[Path], is_view: bool) -> String {
    let mut out = String::new();
    // reads every path makes are listed once
    let common: Vec<String> = match paths.first() {
        Some(first) => first.reads.iter().filter(|r| paths.iter().all(|p| p.reads.contains(r))).cloned().collect(),
        None => Vec::new(),
    };
    for r in &common { out.push_str(&format!("  reads {}\n", r)); }
    for p in paths {
        let head = if p.conds.is_empty() { "always".to_string() } else { format!("when {}", p.conds.join(" and ")) };
        out.push_str(&format!("  {}:\n", head));
        if is_view {
            if let Some(r) = &p.returns { out.push_str(&format!("    {}\n", r)); }
        } else {
            for r in &p.reads { if !common.contains(r) { out.push_str(&format!("    reads {}\n", r)); } }
            let writes: Vec<String> = p.writes.iter().map(|(f, v)| format!("{} := {}", f, v)).collect();
            if !writes.is_empty() { out.push_str(&format!("    {}\n", writes.join(" · "))); }
            for e in &p.effects { out.push_str(&format!("    {}\n", e)); }
            if !p.calls.is_empty() { out.push_str(&format!("    calls {}\n", p.calls.iter().cloned().collect::<Vec<_>>().join(" · "))); }
            for m in &p.may { out.push_str(&format!("    may {}\n", m)); }
            if writes.is_empty() && p.effects.is_empty() && p.calls.is_empty() && p.may.is_empty() { out.push_str("    nothing\n"); }
        }
    }
    out
}

// ---------------------------------------------------------------- view trees

#[derive(Default, Debug, Clone)]
struct Wiring {
    /// view → (control description, handler)
    controls: BTreeMap<String, Vec<(String, String)>>,
    /// view → facts bound by inputs
    binds: BTreeMap<String, BTreeSet<String>>,
    /// view → views it composes
    composes: BTreeMap<String, BTreeSet<String>>,
    /// view → (guard, child view) in order
    composed_under: BTreeMap<String, Vec<(String, String)>>,
    /// view → the guard of every path through it
    view_paths: BTreeMap<String, BTreeSet<String>>,
    /// view → (element, attr, fact) for open/disabled attributes
    gates: BTreeMap<String, Vec<(String, String, String)>>,
    /// view → (guard, tree) per path
    wires: BTreeMap<String, Vec<(String, WNode)>>,
    /// view → (guard conditions, tree text) per path, for the states
    path_trees: BTreeMap<String, Vec<(Vec<String>, String)>>,
}

/// A value's short name for a placeholder: the last identifier of the
/// expression with any part prefix removed — `sim_session_status` reads
/// as ‹status›, `v1.symbol[0]` as ‹symbol›.
fn short_name(expr: &str, m: &Model) -> String {
    let mut best = String::new();
    let mut word = String::new();
    let mut words: Vec<String> = Vec::new();
    for c in expr.chars() {
        if c.is_alphanumeric() || c == '_' { word.push(c); } else if !word.is_empty() { words.push(std::mem::take(&mut word)); }
    }
    if !word.is_empty() { words.push(word); }
    for w in words.iter().rev() {
        if w.chars().all(|c| c.is_ascii_digit()) || w.starts_with('v') && w[1..].chars().all(|c| c.is_ascii_digit()) && w.len() > 1 { continue; }
        best = w.clone();
        break;
    }
    if best.is_empty() { best = "value".into(); }
    for prefix in m.parts.keys() {
        if let Some(rest) = best.strip_prefix(prefix.as_str()) { best = rest.to_string(); break; }
    }
    format!("‹{}›", best)
}

/// Text for a wireframe: literals as they are, spans and expressions as
/// placeholders.
fn placeholder(canon_text: &str, m: &Model) -> String {
    if canon_text.starts_with('\'') && canon_text.ends_with('\'') && canon_text.len() >= 2 {
        let inner = &canon_text[1..canon_text.len() - 1];
        if !inner.contains('`') { return inner.to_string(); }
        let mut out = String::new();
        let mut in_span = false;
        let mut span = String::new();
        for c in inner.chars() {
            if c == '`' {
                if in_span { out.push_str(&short_name(&span, m)); span.clear(); }
                in_span = !in_span;
                continue;
            }
            if in_span { span.push(c) } else { out.push(c) }
        }
        return out;
    }
    short_name(canon_text, m)
}

/// Print an element tree with guards resolved for one path: elements one
/// per line, attributes in name order, controls and bindings named — and
/// build the node the wireframe lays out.
fn tree(m: &Model, e: &Expr, env: &Env, depth: usize, view: &str, w: &mut Wiring, out: &mut String) -> WNode {
    let pad = "  ".repeat(depth);
    match e {
        Expr::Call { callee, args } => {
            let name = callee_name(callee);
            if let Some(n) = &name {
                if drawn_name(callee).is_some() {
                    let mut attrs: Vec<String> = Vec::new();
                    let mut children: Vec<&Expr> = Vec::new();
                    for (i, a) in args.iter().enumerate() {
                        match a {
                            Expr::Map(pairs) if i == 0 => {
                                for (k, v) in pairs {
                                    let key = match k { Expr::Ident(s) | Expr::Str(s) => s.clone(), other => canon(other, env) };
                                    if EVENTS.contains(&key.as_str()) {
                                        let handler = match v {
                                            Expr::Ident(h) => h.clone(),
                                            Expr::Call { callee, args } => format!("{}[{}]", canon(callee, env), args.iter().map(|x| canon(x, env)).collect::<Vec<_>>().join(", ")),
                                            other => canon(other, env),
                                        };
                                        attrs.push(format!("{} → {}", key, handler));
                                        let control = (format!("{} {}", n, label_of(args, env)), handler.split('[').next().unwrap_or("").to_string());
                                        let list = w.controls.entry(view.to_string()).or_default();
                                        if !list.contains(&control) { list.push(control); }
                                    } else if key == "actions" {
                                        if let Some(pairs) = crate::lower::map_literal_pairs(v) {
                                            for (label, h) in pairs {
                                                let hn = canon(&h, env);
                                                attrs.push(format!("action '{}' → {}", label, hn));
                                                let control = (format!("{} action '{}'", n, label), hn);
                                                let list = w.controls.entry(view.to_string()).or_default();
                                                if !list.contains(&control) { list.push(control); }
                                            }
                                        } else { attrs.push(format!("actions = {}", canon(v, env))); }
                                    } else if key == "open" || key == "disabled" {
                                        if let Expr::Ident(f) = v { w.gates.entry(view.to_string()).or_default().push((n.clone(), key.clone(), f.clone())); }
                                        attrs.push(format!("{} = {}", key, canon(v, env)));
                                    } else {
                                        attrs.push(format!("{} = {}", key, canon(v, env)));
                                    }
                                }
                            }
                            _ => children.push(a),
                        }
                    }
                    let mut inline: Vec<String> = Vec::new();
                    let mut nested: Vec<&Expr> = Vec::new();
                    if BOUND.contains(&n.as_str()) {
                        // a bound input: its label, the fact it binds, its settings;
                        // the lowered argument list stays out of the artifact
                        let (label_at, fact_at) = if matches!(n.as_str(), "select" | "radio" | "tabs") { (None, 1) } else { (Some(0), 1) };
                        if let Some(Expr::Ident(f)) = children.get(fact_at) {
                            if m.is_fact(f) {
                                attrs.push(format!("binds {}", f));
                                w.binds.entry(view.to_string()).or_default().insert(f.clone());
                            } else {
                                attrs.push(format!("binds {}", canon(children[fact_at], env)));
                            }
                        }
                        match n.as_str() {
                            "field" => {
                                if let Some(Expr::Str(t)) = children.get(2) { if !t.is_empty() { attrs.push(format!("type = '{}'", t)); } }
                                if let Some(Expr::Int(l)) = children.get(3) { if *l > 0 { attrs.push(format!("lines = {}", l)); } }
                            }
                            "slider" => {
                                for (i, k) in ["min", "max", "step"].iter().enumerate() {
                                    if let Some(v) = children.get(2 + i) { attrs.push(format!("{} = {}", k, canon(v, env))); }
                                }
                            }
                            "select" | "radio" | "tabs" => {
                                if let Some(o) = children.first() { attrs.push(format!("options = {}", canon(o, env))); }
                            }
                            _ => {}
                        }
                        if let Some(i) = label_at { if let Some(l) = children.get(i) { inline.push(canon(l, env)); } }
                    } else {
                        for c in children {
                            if is_node(m, c, env) { nested.push(c); } else { inline.push(canon(c, env)); }
                        }
                    }
                    attrs.sort();
                    out.push_str(&format!("{}{}[{}]{}\n", pad, n, attrs.join(", "), if inline.is_empty() { String::new() } else { format!(" {}", inline.join(" ")) }));
                    // the node: literal attributes, placeholder text, the kids
                    let mut node = WNode { tag: n.clone(), ..Default::default() };
                    for a in &attrs {
                        if let Some((k, v)) = a.split_once(" = ") { node.attrs.push((k.to_string(), v.to_string())); }
                        else if let Some(rest) = a.strip_prefix("binds ") { node.attrs.push(("binds".into(), rest.to_string())); }
                        else if let Some((k, _)) = a.split_once(" → ") { node.attrs.push((k.to_string(), "handler".into())); }
                    }
                    node.text = inline.iter().map(|t| placeholder(t, m)).collect::<Vec<_>>().join(" ");
                    for c in nested { let k = tree(m, c, env, depth + 1, view, w, out); node.kids.push(k); }
                    return node;
                }
                if m.is_view(n) {
                    w.composes.entry(view.to_string()).or_default().insert(n.clone());
                    out.push_str(&format!("{}@{}[{}]\n", pad, n, args.iter().map(|a| canon(a, env)).collect::<Vec<_>>().join(", ")));
                    return WNode { tag: "@".into(), view: n.clone(), ..Default::default() };
                }
            }
            let t = canon(e, env);
            out.push_str(&format!("{}{}\n", pad, t));
            WNode { tag: "text".into(), text: placeholder(&t, m), ..Default::default() }
        }
        Expr::Array(items) => {
            let mut node = WNode { tag: "list".into(), ..Default::default() };
            for i in items { node.kids.push(tree(m, i, env, depth, view, w, out)); }
            node
        }
        Expr::Ident(n) => {
            if let Some(t) = env.locals.get(n) {
                // a local holding a tree prints as that tree
                if t.starts_with('\u{1}') {
                    out.push_str(&t[1..].lines().map(|l| format!("{}{}\n", pad, l)).collect::<String>());
                    return env.nodes.get(n).cloned().unwrap_or_default();
                }
            }
            let t = canon(e, env);
            out.push_str(&format!("{}{}\n", pad, t));
            WNode { tag: "text".into(), text: placeholder(&t, m), ..Default::default() }
        }
        Expr::ForExpr { var, iter, body } => {
            let mut inner = env.clone();
            inner.locals.insert(var.clone(), format!("each[{}]", canon(iter, env)));
            out.push_str(&format!("{}for each[{}]\n", pad, canon(iter, env)));
            let mut node = WNode { tag: "for".into(), ..Default::default() };
            if let Some(Stmt::ExprStmt(e)) | Some(Stmt::Return(Some(e))) = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
                node.kids.push(tree(m, e, &inner, depth + 1, view, w, out));
            }
            node
        }
        other => {
            let t = canon(other, env);
            out.push_str(&format!("{}{}\n", pad, t));
            WNode { tag: "text".into(), text: placeholder(&t, m), ..Default::default() }
        }
    }
}

fn label_of(args: &[Expr], env: &Env) -> String {
    for a in args.iter().skip(1) {
        match a {
            Expr::Str(s) => return canon_str(s, env),
            Expr::Map(_) => continue,
            other => return format!("‹{}›", canon(other, env)),
        }
    }
    String::new()
}

fn is_node(m: &Model, e: &Expr, env: &Env) -> bool {
    match e {
        Expr::Call { callee, .. } => drawn_name(callee).is_some() || callee_name(callee).is_some_and(|n| m.is_view(&n)),
        Expr::Array(items) => items.iter().any(|i| is_node(m, i, env)),
        Expr::ForExpr { .. } => true,
        Expr::Ident(n) => env.locals.get(n).map_or(false, |t| t.starts_with('\u{1}')),
        _ => false,
    }
}

/// The sub-views a path's tree shows, recorded under the path's guard —
/// a sub-view held in a local sits under the guard of the path that
/// returns it, not of the statement that built it.
fn record_composition(w: &mut Wiring, view: &str, conds: &[String], tree_text: &str) {
    let guard = conds.join(" and ");
    w.view_paths.entry(view.to_string()).or_default().insert(guard.clone());
    for line in tree_text.lines() {
        let l = line.trim_start();
        if let Some(rest) = l.strip_prefix('@') {
            let child: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if child.is_empty() { continue; }
            let pair = (guard.clone(), child);
            let under = w.composed_under.entry(view.to_string()).or_default();
            if !under.contains(&pair) { under.push(pair); }
        }
    }
}

/// A view's conditional tree: the walk forks at guards like a handler's,
/// and each path ends in the tree it returns. Locals that hold trees are
/// printed in place.
fn view_tree(m: &Model, name: &str, params: &[String], body: &[Stmt], w: &mut Wiring) -> String {
    let mut env = Env::default();
    for (i, p) in params.iter().enumerate() { env.locals.insert(p.clone(), format!("${}", i + 1)); }
    let mut paths: Vec<(Vec<String>, String)> = Vec::new();
    walk_view(m, name, body, env, Vec::new(), w, &mut paths);
    paths.sort();
    paths.dedup();
    w.path_trees.insert(name.to_string(), paths.clone());
    let mut out = String::new();
    for (conds, t) in paths {
        let head = if conds.is_empty() { "always".to_string() } else { format!("when {}", conds.join(" and ")) };
        out.push_str(&format!("  {}:\n", head));
        for l in t.lines() { out.push_str(&format!("    {}\n", l)); }
    }
    out
}

fn walk_view(m: &Model, view: &str, stmts: &[Stmt], mut env: Env, conds: Vec<String>, w: &mut Wiring, out: &mut Vec<(Vec<String>, String)>) {
    let mut i = 0;
    while i < stmts.len() {
        let st = &stmts[i];
        i += 1;
        match st {
            Stmt::Assign { name, value } => {
                if is_node(m, value, &env) {
                    let mut t = String::new();
                    let node = tree(m, value, &env, 0, view, w, &mut t);
                    env.nodes.insert(name.clone(), node);
                    env.locals.insert(name.clone(), format!("\u{1}{}", t));
                } else {
                    let t = canon(value, &env);
                    env.locals.insert(name.clone(), t);
                }
            }
            Stmt::MultiAssign { names, values } => {
                let texts: Vec<String> = values.iter().map(|v| canon(v, &env)).collect();
                for (n, t) in names.iter().zip(texts) { env.locals.insert(n.clone(), t); }
            }
            Stmt::Return(Some(e)) => {
                let mut t = String::new();
                let mut c = conds.clone(); c.sort();
                let node = tree(m, e, &env, 0, view, w, &mut t);
                record_composition(w, view, &c, &t);
                w.wires.entry(view.to_string()).or_default().push((c.join(" and "), node));
                out.push((c, t));
                return;
            }
            Stmt::Return(None) => { let mut c = conds.clone(); c.sort(); record_composition(w, view, &c, ""); out.push((c, "nothing".into())); return; }
            Stmt::If { condition, then_body, else_body } => {
                let c = canon(condition, &env);
                let rest = &stmts[i..];
                let mut s1: Vec<Stmt> = then_body.clone(); s1.extend(rest.iter().cloned());
                let mut c1 = conds.clone(); c1.push(c.clone());
                walk_view(m, view, &s1, env.clone(), c1, w, out);
                let mut s2: Vec<Stmt> = else_body.clone().unwrap_or_default(); s2.extend(rest.iter().cloned());
                let mut c2 = conds.clone(); c2.push(negate(&c));
                walk_view(m, view, &s2, env, c2, w, out);
                return;
            }
            Stmt::ExprStmt(e) => {
                // the last statement of a view is its tree
                if i == stmts.len() && is_node(m, e, &env) {
                    let mut t = String::new();
                    let mut c = conds.clone(); c.sort();
                    let node = tree(m, e, &env, 0, view, w, &mut t);
                    record_composition(w, view, &c, &t);
                    w.wires.entry(view.to_string()).or_default().push((c.join(" and "), node));
                    out.push((c, t));
                    return;
                }
            }
            _ => {}
        }
    }
    let mut c = conds; c.sort();
    record_composition(w, view, &c, "");
    out.push((c, "nothing".into()));
}

// ---------------------------------------------------------------- derivation

fn read_names(m: &Model, body: &[Stmt]) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>, bool) {
    let bodies: BTreeMap<String, Vec<Stmt>> = m.fns.iter().filter(|(n, _)| !m.derives.contains(*n)).map(|(n, (_, b))| (n.clone(), b.clone())).collect();
    let params: BTreeMap<String, Vec<String>> = m.fns.iter().map(|(n, (ps, _))| (n.clone(), ps.clone())).collect();
    let owned = crate::derive::owned_tables(m.stmts, &bodies);
    let rs = crate::derive::read_set(body, &m.facts, &m.derives, &bodies, &params, &owned);
    (rs.facts, rs.derives, rs.tables, rs.now)
}

/// Derive every artifact of the program.
pub fn derive_artifacts(stmts: &[Stmt], ctx: &TypeContext, parts: &BTreeMap<String, String>, entry: &str) -> (Artifacts, Vec<Rule>) {
    let facts = crate::derive::fact_names(stmts);
    let derives: BTreeSet<String> = ctx.derives.clone();
    let views: BTreeSet<String> = ctx.views.clone();
    let mut handlers: BTreeSet<String> = ctx.view_handlers.clone();
    let mut fns: BTreeMap<String, (Vec<String>, Vec<Stmt>)> = BTreeMap::new();
    for s in stmts {
        if let Stmt::FnDecl { name, params, body, .. } = s { fns.insert(name.clone(), (params.clone(), body.clone())); }
    }
    // a helper that writes a fact belongs to the handler class for the walk
    let bodies: BTreeMap<String, Vec<Stmt>> = fns.iter().map(|(n, (_, b))| (n.clone(), b.clone())).collect();
    for (n, b) in &bodies {
        let one: BTreeMap<String, Vec<Stmt>> = [(n.clone(), b.clone())].into_iter().collect();
        if !crate::derive::written_facts(&one, &facts).is_empty() && !views.contains(n) { handlers.insert(n.clone()); }
    }
    let m = Model { stmts, facts: facts.clone(), derives: derives.clone(), views: views.clone(), handlers: handlers.clone(), fns: fns.clone(), parts: parts.clone(), entry: entry.to_string() };

    let mut art = Artifacts::default();
    let mut wiring = Wiring::default();
    let mut sigs: BTreeMap<String, Vec<Path>> = BTreeMap::new();

    // views first: they tell who is wired
    let mut view_reads: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut view_trees: BTreeMap<String, String> = BTreeMap::new();
    for v in &views {
        if let Some((params, body)) = fns.get(v) {
            view_trees.insert(v.clone(), view_tree(&m, v, params, body, &mut wiring));
            let (f, d, _, _) = read_names(&m, body);
            view_reads.insert(v.clone(), f.into_iter().chain(d).collect());
        }
    }
    // pages
    let mut pages: BTreeMap<String, String> = BTreeMap::new();
    for s in stmts {
        if let Stmt::ExprStmt(Expr::Call { callee, args }) = s {
            if matches!(callee.as_ref(), Expr::Ident(n) if n == "page") {
                if let (Some(Expr::Str(path)), Some(Expr::Ident(v))) = (args.first(), args.get(1)) { pages.insert(path.clone(), v.clone()); }
            }
        }
    }
    // test blocks wire handlers too
    let mut test_wired: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in stmts {
        if let Stmt::TestBlock { name, body, .. } = s {
            let mut clicked = BTreeSet::new();
            collect_clicks(body, &mut clicked);
            for h in clicked { test_wired.entry(h).or_default().insert(name.clone().unwrap_or_else(|| "test".into())); }
        }
    }
    // composition closure: view → pages composing it
    let mut composed_on: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (path, v) in &pages {
        let mut stack = vec![v.clone()];
        let mut seen = BTreeSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) { continue; }
            composed_on.entry(cur.clone()).or_default().insert(format!("page {}", path));
            if let Some(subs) = wiring.composes.get(&cur) { for s in subs { stack.push(s.clone()); } }
        }
    }
    let wired_in = |h: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (v, controls) in &wiring.controls {
            for (c, handler) in controls { if handler == h { out.push(format!("{} {}", v, c)); } }
        }
        if let Some(tests) = test_wired.get(h) { for t in tests { out.push(format!("test {}", t)); } }
        out.sort(); out.dedup();
        out
    };
    // who calls a helper: the functions whose bodies call it directly
    let mut callers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (n, (_, b)) in &fns {
        let mut called = BTreeSet::new();
        direct_calls(b, &mut called);
        for c in called { if c != *n && fns.contains_key(&c) { callers.entry(c).or_default().insert(n.clone()); } }
    }

    // signatures
    for h in &handlers {
        if views.contains(h) { continue; }
        if let Some((params, body)) = fns.get(h) {
            sigs.insert(h.clone(), signature(&m, h, params, body));
        }
    }
    // who reads a fact or derivation
    let mut readers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (v, reads) in &view_reads { for r in reads { readers.entry(r.clone()).or_default().insert(v.clone()); } }
    let mut derive_lines: BTreeMap<String, String> = BTreeMap::new();
    for s in stmts {
        if let Stmt::Derive { name, fresh, body } = s {
            let (f, d, tables, now) = read_names(&m, body);
            for r in f.iter().chain(d.iter()) { readers.entry(r.clone()).or_default().insert(name.clone()); }
            let mut reads: Vec<String> = f.iter().chain(d.iter()).cloned().collect();
            if now { reads.push("now[]".into()); }
            let mut text = format!("derive {} ({})\n", name, m.file_of(name));
            text.push_str(&format!("  reads: {}\n", if reads.is_empty() { "nothing".to_string() } else { reads.join(", ") }));
            if !tables.is_empty() { text.push_str(&format!("  tables: {}{}\n", tables.iter().cloned().collect::<Vec<_>>().join(", "), if *fresh > 0.0 { format!(" (fresh {}s)", fresh) } else { String::new() })); }
            derive_lines.insert(name.clone(), text);
        }
    }
    for (h, paths) in &sigs {
        for p in paths {
            let (f, d, _, _) = read_names(&m, &fns[h].1);
            for r in f.iter().chain(d.iter()) { readers.entry(r.clone()).or_default().insert(h.clone()); }
            let _ = p;
        }
    }
    let readers_of = |n: &str| -> Vec<String> { readers.get(n).map(|s| s.iter().cloned().collect()).unwrap_or_default() };
    // writers of each fact, with literal values where written literally
    let mut writers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut written_values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (h, paths) in &sigs {
        for p in paths {
            for (f, v) in &p.writes { writers.entry(f.clone()).or_default().insert(h.clone()); written_values.entry(f.clone()).or_default().insert(v.clone()); }
            for mw in &p.may { if let Some((f, v)) = mw.split_once(" := ") { if facts.contains(f) { writers.entry(f.to_string()).or_default().insert(h.clone()); written_values.entry(f.to_string()).or_default().insert(v.to_string()); } } }
        }
    }
    let mut bound_by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (v, fs) in &wiring.binds { for f in fs { bound_by.entry(f.clone()).or_default().insert(v.clone()); } }

    // reach: through readers of what a handler writes, to views and pages
    let pages_of = |v: &str| -> Vec<String> { composed_on.get(v).map(|s| s.iter().cloned().collect()).unwrap_or_default() };
    let reach_of_views = |vs: &BTreeSet<String>| -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for v in vs { out.insert(format!("view {}", v)); for p in pages_of(v) { out.insert(p); } }
        out.into_iter().collect()
    };
    let views_reading = |names: &BTreeSet<String>| -> BTreeSet<String> {
        // a derivation's readers are readers too
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = names.iter().cloned().collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(n) = stack.pop() {
            if !seen.insert(n.clone()) { continue; }
            for r in readers_of(&n) {
                if views.contains(&r) { out.insert(r.clone()); } else if derives.contains(&r) { stack.push(r.clone()); }
            }
        }
        out
    };

    // facts
    for f in &facts {
        let ty = ctx.vars.get(f).map(|t| format!("{:?}", t).to_lowercase()).unwrap_or_else(|| "unknown".into());
        let mut text = format!("fact {} ({})\n  type: {}\n", f, m.file_of(f), ty);
        let ws = writers.get(f).map(|s| s.iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
        text.push_str(&format!("  written by: {}\n", if ws.is_empty() { "nobody".to_string() } else { ws.join(", ") }));
        let bs = bound_by.get(f).map(|s| s.iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
        if !bs.is_empty() { text.push_str(&format!("  bound by: {}\n", bs.join(", "))); }
        let rs = readers_of(f);
        text.push_str(&format!("  read by: {}\n", if rs.is_empty() { "nobody".to_string() } else { rs.join(", ") }));
        let key = format!("fact {}", f);
        art.files.insert(key.clone(), m.file_of(f));
        art.reach.insert(key.clone(), reach_of_views(&views_reading(&[f.clone()].into_iter().collect())));
        art.entries.insert(key, text);
    }
    // derivations
    for (d, mut text) in derive_lines {
        let rs = readers_of(&d);
        text.push_str(&format!("  read by: {}\n", if rs.is_empty() { "nobody".to_string() } else { rs.join(", ") }));
        let key = format!("derive {}", d);
        art.files.insert(key.clone(), m.file_of(&d));
        art.reach.insert(key.clone(), reach_of_views(&views_reading(&[d.clone()].into_iter().collect())));
        art.entries.insert(key, text);
    }
    // handlers
    for (h, paths) in &sigs {
        let params = &fns[h].0;
        let mut text = format!("handler {} ({})\n", h, m.file_of(h));
        if !params.is_empty() { text.push_str(&format!("  inputs: {}\n", params.iter().enumerate().map(|(i, _)| format!("${}", i + 1)).collect::<Vec<_>>().join(", "))); }
        let wired = wired_in(h);
        if wired.is_empty() {
            let by: Vec<String> = callers.get(h).map(|s| s.iter().cloned().collect()).unwrap_or_default();
            text.push_str(&format!("  called by: {}\n", if by.is_empty() { "nobody".to_string() } else { by.join(", ") }));
        } else {
            text.push_str(&format!("  wired: {}\n", wired.join(" · ")));
        }
        text.push_str(&render_paths(paths, false));
        let mut written: BTreeSet<String> = BTreeSet::new();
        for p in paths { for f in p.writes.keys() { written.insert(f.clone()); } for mw in &p.may { if let Some((f, _)) = mw.split_once(" := ") { if facts.contains(f) { written.insert(f.to_string()); } } } }
        let mut reach_views: BTreeSet<String> = views_reading(&written);
        for w in &wired { if let Some(v) = w.split(' ').next() { if views.contains(v) { reach_views.insert(v.to_string()); } } }
        let key = format!("handler {}", h);
        art.files.insert(key.clone(), m.file_of(h));
        art.reach.insert(key.clone(), reach_of_views(&reach_views));
        art.entries.insert(key, text);
    }
    // views
    for (v, t) in &view_trees {
        let mut text = format!("view {} ({})\n", v, m.file_of(v));
        let on = pages_of(v);
        text.push_str(&format!("  composed on: {}\n", if on.is_empty() { "nothing".to_string() } else { on.join(", ") }));
        let reads: Vec<String> = view_reads.get(v).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        if !reads.is_empty() { text.push_str(&format!("  reads: {}\n", reads.join(", "))); }
        text.push_str(t);
        let key = format!("view {}", v);
        art.files.insert(key.clone(), m.file_of(v));
        let mut r: BTreeSet<String> = BTreeSet::new();
        for p in on { r.insert(p); }
        for (pv, subs) in &wiring.composes { if subs.contains(v) { r.insert(format!("view {}", pv)); } }
        art.reach.insert(key.clone(), r.into_iter().collect());
        art.entries.insert(key, text);
    }
    // pages
    for (path, v) in &pages {
        let key = format!("page {}", path);
        art.files.insert(key.clone(), entry.to_string());
        art.reach.insert(key.clone(), vec![format!("view {}", v)]);
        art.entries.insert(key, format!("page {} → {}\n", path, v));
    }
    // each page's reachable states, and what each offers (B2)
    let mut initial: BTreeMap<String, String> = BTreeMap::new();
    for s in stmts {
        if let Stmt::Assign { name, value } = s {
            if facts.contains(name) {
                initial.insert(name.clone(), canon(value, &Env::default()));
            }
        }
    }
    // the page's reader: nobody until a handler signs someone in
    initial.insert("reader".into(), "'nobody'".into());
    let machine_paths: BTreeMap<String, Vec<crate::states::HandlerPath>> = sigs
        .iter()
        .map(|(h, paths)| {
            let ps = paths
                .iter()
                .map(|p| crate::states::HandlerPath {
                    conds: p.conds.clone(),
                    writes: p
                        .writes
                        .iter()
                        .map(|(f, v)| (f.clone(), v.clone()))
                        .chain(p.effects.iter().filter_map(|e| {
                            let verb = e.strip_prefix("effect ")?.split_whitespace().next()?;
                            match verb {
                                "sign_in" => Some(("reader".to_string(), "'someone'".to_string())),
                                "sign_out" => Some(("reader".to_string(), "'nobody'".to_string())),
                                _ => None,
                            }
                        }))
                        .collect(),
                    may: p.may.iter().filter_map(|mw| mw.split_once(" := ").map(|(f, v)| (f.to_string(), v.to_string()))).collect(),
                })
                .collect();
            (h.clone(), ps)
        })
        .collect();
    let program = crate::states::Program { initial: &initial, handlers: &machine_paths, views: &wiring.path_trees };
    for (path, v) in &pages {
        let key = format!("states {}", path);
        art.files.insert(key.clone(), entry.to_string());
        art.reach.insert(key.clone(), vec![format!("page {}", path), format!("view {}", v)]);
        art.entries.insert(key, crate::states::machine(&program, path, v));
    }

    // kernel rules
    let mut rules: Vec<Rule> = Vec::new();
    for (v, gates) in &wiring.gates {
        for (el, attr, fact) in gates {
            if !facts.contains(fact) { continue; }
            let vals = written_values.get(fact).cloned().unwrap_or_default();
            if attr == "open" && OPENERS.contains(&el.as_str()) {
                if vals.is_empty() {
                    rules.push(Rule { subject: v.clone(), file: m.file_of(v), detail: format!("{} opens on '{}', which no handler writes — it never opens and never closes", el, fact), fix: format!("write '{}' from a handler, or derive it", fact) });
                } else if !vals.iter().any(|x| x == "false") && vals.iter().all(|x| x == "true") {
                    rules.push(Rule { subject: v.clone(), file: m.file_of(v), detail: format!("{} opens on '{}', which handlers only ever set true — there is no way to close it", el, fact), fix: format!("a handler that sets '{}' false", fact) });
                }
            }
            if attr == "disabled" && vals.is_empty() {
                rules.push(Rule { subject: v.clone(), file: m.file_of(v), detail: format!("{} is disabled on '{}', which no handler writes — it is never disabled, or always", el, fact), fix: format!("write '{}' from a handler, or drop the attribute", fact) });
            }
        }
    }
    for (v, controls) in &wiring.controls {
        for (c, h) in controls {
            let Some(paths) = sigs.get(h) else { continue };
            let idle = paths.iter().all(|p| p.writes.is_empty() && p.effects.is_empty() && p.calls.is_empty() && p.may.is_empty());
            if idle {
                rules.push(Rule { subject: v.clone(), file: m.file_of(v), detail: format!("{} fires '{}', which writes nothing and performs nothing on any path — a dead control", c, h), fix: "make the handler move a fact or perform an effect, or drop the control".into() });
            }
        }
    }
    let mut called: BTreeSet<String> = BTreeSet::new();
    for (_, b) in bodies.iter() { called.extend(crate::derive::called_fns(b, &bodies)); }
    for s in stmts {
        if let Stmt::Derive { body, .. } = s { called.extend(crate::derive::called_fns(body, &bodies)); }
        if let Stmt::TestBlock { body, .. } = s { called.extend(crate::derive::called_fns(body, &bodies)); }
    }
    for h in &handlers {
        if views.contains(h) || ctx.view_handlers.contains(h) { continue; }
        if wired_in(h).is_empty() && !called.contains(h) {
            rules.push(Rule { subject: h.clone(), file: m.file_of(h), detail: format!("'{}' assigns a fact but nothing wires it and nothing calls it — a handler nobody can reach", h), fix: "wire it from a view, call it from a handler, or remove it".into() });
        }
    }
    rules.sort_by(|a, b| (a.subject.clone(), a.detail.clone()).cmp(&(b.subject.clone(), b.detail.clone())));

    // the shape of the app, for --show
    art.structure.pages = pages.clone();
    art.structure.composes = wiring.composed_under.clone();
    art.structure.view_paths = wiring.view_paths.clone();
    art.structure.wires = wiring.wires.clone();
    art.structure.controls = wiring.controls.clone();
    art.structure.binds = wiring.binds.clone();
    for (h, paths) in &sigs {
        let mut ws: BTreeSet<String> = BTreeSet::new();
        let mut es: BTreeSet<String> = BTreeSet::new();
        for p in paths {
            for f in p.writes.keys() { ws.insert(f.clone()); }
            for mw in &p.may { if let Some((f, _)) = mw.split_once(" := ") { if facts.contains(f) { ws.insert(f.to_string()); } } }
            for e in &p.effects { es.insert(e.split_whitespace().take(3).collect::<Vec<_>>().join(" ")); }
        }
        art.structure.writes.insert(h.clone(), ws);
        art.structure.effects.insert(h.clone(), es);
    }
    for f in &facts {
        let b = bound_by.get(f).cloned().unwrap_or_default();
        let w = writers.get(f).cloned().unwrap_or_default();
        let r: BTreeSet<String> = readers_of(f).into_iter().collect();
        art.structure.facts.insert(f.clone(), (b, w, r));
    }
    art.structure.derives = derives.clone();
    art.structure.parts = parts.values().cloned().collect();
    (art, rules)
}

/// The app as a tree: each page, the views it composes under their
/// guards, the controls and the handlers they fire; then the handlers
/// with what they write and do; then the facts with who binds, writes
/// and reads them.
pub fn render_structure(art: &Artifacts, entry: &str) -> String {
    let s = &art.structure;
    let mut out = String::new();
    let count = |kind: &str| art.entries.keys().filter(|k| k.starts_with(&format!("{} ", kind))).count();
    out.push_str(&format!(
        "{} — {} part{} · {} fact{} · {} derivation{} · {} handler{} · {} view{} · {} page{}\n",
        entry.trim_end_matches(".crow").trim_end_matches(".co"),
        s.parts.len().max(1), if s.parts.len() == 1 { "" } else { "s" },
        count("fact"), if count("fact") == 1 { "" } else { "s" },
        count("derive"), if count("derive") == 1 { "" } else { "s" },
        count("handler"), if count("handler") == 1 { "" } else { "s" },
        count("view"), if count("view") == 1 { "" } else { "s" },
        count("page"), if count("page") == 1 { "" } else { "s" },
    ));
    fn view_line(art: &Artifacts, v: &str) -> String {
        let s = &art.structure;
        let file = art.files.get(&format!("view {}", v)).cloned().unwrap_or_default();
        let mut bits: Vec<String> = Vec::new();
        if let Some(b) = s.binds.get(v) { for f in b { bits.push(format!("binds {}", f)); } }
        if let Some(c) = s.controls.get(v) { for (control, h) in c { bits.push(format!("{} → {}", control, h)); } }
        let tail = if bits.is_empty() { String::new() } else { format!("   {}", bits.join(" · ")) };
        format!("@{} ({}){}", v, file, tail)
    }
    fn walk(art: &Artifacts, v: &str, depth: usize, seen: &mut Vec<String>, out: &mut String) {
        if seen.contains(&v.to_string()) {
            out.push_str(&format!("{}@{} (above)\n", "  ".repeat(depth), v));
            return;
        }
        seen.push(v.to_string());
        if let Some(children) = art.structure.composes.get(v) {
            // a child on every path is common; the rest sit under their guards
            let all_paths = art.structure.view_paths.get(v).cloned().unwrap_or_default();
            let mut under: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            let mut order: Vec<&str> = Vec::new();
            for (g, c) in children {
                under.entry(c.as_str()).or_default().insert(g.as_str());
                if !order.contains(&c.as_str()) { order.push(c.as_str()); }
            }
            let mut guarded: Vec<(&str, &str)> = Vec::new();
            for c in &order {
                let gs = &under[c];
                let common = all_paths.is_empty() || all_paths.iter().all(|p| gs.contains(p.as_str()));
                if common {
                    out.push_str(&format!("{}{}\n", "  ".repeat(depth), view_line(art, c)));
                    walk(art, c, depth + 1, seen, out);
                } else {
                    for g in gs { guarded.push((g, c)); }
                }
            }
            guarded.sort();
            let mut last: Option<&str> = None;
            for (g, c) in guarded {
                if last != Some(g) { out.push_str(&format!("{}when {}:\n", "  ".repeat(depth), g)); last = Some(g); }
                out.push_str(&format!("{}{}\n", "  ".repeat(depth + 1), view_line(art, c)));
                walk(art, c, depth + 2, seen, out);
            }
        }
        seen.pop();
    }
    for (path, v) in &s.pages {
        out.push_str(&format!("\npage {} → {}\n", path, view_line(art, v)));
        let mut seen = Vec::new();
        walk(art, v, 1, &mut seen, &mut out);
    }
    if !s.writes.is_empty() {
        out.push_str("\nhandlers\n");
        for (h, ws) in &s.writes {
            let file = art.files.get(&format!("handler {}", h)).cloned().unwrap_or_default();
            let mut bits: Vec<String> = Vec::new();
            if !ws.is_empty() { bits.push(format!("writes {}", ws.iter().cloned().collect::<Vec<_>>().join(", "))); }
            if let Some(es) = s.effects.get(h) { if !es.is_empty() { bits.push(es.iter().cloned().collect::<Vec<_>>().join(" · ")); } }
            if bits.is_empty() { bits.push("nothing".into()); }
            out.push_str(&format!("  {} ({})   {}\n", h, file, bits.join("   ")));
        }
    }
    if !s.facts.is_empty() {
        out.push_str("\nfacts\n");
        for (f, (b, w, r)) in &s.facts {
            let file = art.files.get(&format!("fact {}", f)).cloned().unwrap_or_default();
            let mut bits: Vec<String> = Vec::new();
            if !b.is_empty() { bits.push(format!("bound by {}", b.iter().cloned().collect::<Vec<_>>().join(", "))); }
            if !w.is_empty() { bits.push(format!("written by {}", w.iter().cloned().collect::<Vec<_>>().join(", "))); }
            bits.push(format!("read by {}", if r.is_empty() { "nobody".to_string() } else if r.len() > 4 { format!("{} readers", r.len()) } else { r.iter().cloned().collect::<Vec<_>>().join(", ") }));
            out.push_str(&format!("  {} ({})   {}\n", f, file, bits.join(" · ")));
        }
    }
    if !s.derives.is_empty() {
        out.push_str("\nderivations\n");
        for d in &s.derives {
            if let Some(text) = art.entries.get(&format!("derive {}", d)) {
                let reads = text.lines().find(|l| l.trim_start().starts_with("reads:")).map(|l| l.trim().to_string()).unwrap_or_default();
                let file = art.files.get(&format!("derive {}", d)).cloned().unwrap_or_default();
                out.push_str(&format!("  {} ({})   {}\n", d, file, reads));
            }
        }
    }
    out
}

/// The names a body calls directly, in any expression.
fn direct_calls(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    fn walk(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Some(n) = callee_name(callee) { out.insert(n); }
                walk(callee, out);
                for a in args { walk(a, out); }
            }
            Expr::BinOp { left, right, .. } => { walk(left, out); walk(right, out); }
            Expr::UnaryOp { expr, .. } => walk(expr, out),
            Expr::FieldAccess { object, .. } => walk(object, out),
            Expr::Array(items) => for i in items { walk(i, out); },
            Expr::Map(pairs) => for (k, v) in pairs { walk(k, out); walk(v, out); },
            Expr::Named { value, .. } => walk(value, out),
            Expr::ForExpr { iter, body, .. } => { walk(iter, out); direct_calls(body, out); }
            _ => {}
        }
    }
    for st in stmts {
        match st {
            Stmt::Assign { value, .. } => walk(value, out),
            Stmt::MultiAssign { values, .. } => for v in values { walk(v, out); },
            Stmt::FieldAssign { object, value, .. } | Stmt::IndexAssign { object, value, .. } => { walk(object, out); walk(value, out); }
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk(e, out),
            Stmt::If { condition, then_body, else_body } => { walk(condition, out); direct_calls(then_body, out); if let Some(eb) = else_body { direct_calls(eb, out); } }
            Stmt::For { iter, body, .. } => { walk(iter, out); direct_calls(body, out); }
            Stmt::While { condition, body } => { walk(condition, out); direct_calls(body, out); }
            _ => {}
        }
    }
}

fn collect_clicks(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    fn walk(e: &Expr, out: &mut BTreeSet<String>) {
        if let Expr::Call { callee, args } = e {
            if matches!(callee.as_ref(), Expr::Ident(n) if n == "click") && args.len() == 2 {
                match &args[1] {
                    Expr::Ident(h) => { out.insert(h.clone()); }
                    Expr::Call { callee, .. } => { if let Expr::Ident(h) = callee.as_ref() { out.insert(h.clone()); } }
                    _ => {}
                }
            }
            for a in args { walk(a, out); }
        }
    }
    for st in stmts {
        match st {
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk(e, out),
            Stmt::Assign { value, .. } => walk(value, out),
            Stmt::If { then_body, else_body, .. } => { collect_clicks(then_body, out); if let Some(eb) = else_body { collect_clicks(eb, out); } }
            Stmt::For { body, .. } | Stmt::While { body, .. } => collect_clicks(body, out),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- the store

// ---------------------------------------------------------------- reach

/// The capability an effect is one of: what it lets a part touch
/// (second-half-design.md §1). `log` touches nothing outside the program.
pub fn capability_of(effect: &str) -> Option<&'static str> {
    Some(match effect {
        "query" | "exec" | "insert" | "transact" | "migrate" | "connect" => "database",
        "read" | "write" | "mkdir" | "ls" | "rm" | "mv" | "cp" | "csv" | "lines" | "load" | "save" => "files",
        "fetch" | "poll" | "send" | "recv" | "request" => "network",
        "serve" | "page" | "route" | "listen" => "serving",
        "shell" => "shell",
        "mail" => "mail",
        "now" | "time" | "sleep" => "clock",
        "input" | "args" | "env" | "exit" => "process",
        "later" | "notice" | "sign_in" | "sign_out" | "download" => "session",
        _ => return None,
    })
}

/// The capabilities, in the order a reach names them.
const CAPABILITIES: &[&str] = &["database", "files", "network", "serving", "shell", "mail", "clock", "process", "session"];

/// Each file's reach, from the lowered program: every effect its
/// functions, derivations and top level reach, transitively, by the
/// capability it is one of (second-half-design.md §1). One entry per file,
/// the entry's own and each part's, keyed `reach <file>`; a file that
/// reaches nothing says so, so its first reach is a change to accept.
pub fn reach_entries(program: &crate::ir::Program, parts: &BTreeMap<String, String>, entry: &str) -> BTreeMap<String, (String, String)> {
    let file_of = |name: &str| -> String {
        parts
            .iter()
            .filter(|(prefix, _)| name.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, f)| f.clone())
            .unwrap_or_else(|| entry.to_string())
    };
    let mut by_file: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    by_file.entry(entry.to_string()).or_default();
    for f in parts.values() {
        by_file.entry(f.clone()).or_default();
    }
    for part in &program.parts {
        for f in &part.fns {
            by_file.entry(file_of(&f.name)).or_default().extend(f.effects.iter().cloned());
        }
        for d in &part.derives {
            by_file.entry(file_of(&d.name)).or_default().extend(d.effects.iter().cloned());
        }
        for (prefix, effects) in &part.main_effects {
            let file = parts.get(prefix).cloned().unwrap_or_else(|| entry.to_string());
            by_file.entry(file).or_default().extend(effects.iter().cloned());
        }
    }
    by_file
        .into_iter()
        .map(|(file, effects)| {
            let key = format!("reach {}", file);
            let mut text = format!("{}\n", key);
            for cap in CAPABILITIES {
                let mine: Vec<&str> = effects.iter().filter(|e| capability_of(e) == Some(*cap)).map(|e| e.as_str()).collect();
                if !mine.is_empty() {
                    text.push_str(&format!("  {}: {}\n", cap, mine.join(", ")));
                }
            }
            if !text.contains(':') {
                text.push_str("  nothing\n");
            }
            (key, (file, text))
        })
        .collect()
}

const BASELINE_DIR: &str = ".crow/baseline";

/// The store for an entry file: `.crow/baseline/<entry>/`, the entry's
/// path with separators folded so two programs checked from one root
/// keep their own baselines.
pub fn store_dir(entry: &str) -> std::path::PathBuf {
    let key: String = entry.trim_end_matches(".crow").trim_end_matches(".co").chars().map(|c| if c == '/' || c == '\\' || c == '.' { '-' } else { c }).collect();
    let key = key.trim_matches('-').to_string();
    std::path::Path::new(BASELINE_DIR).join(if key.is_empty() { "main".to_string() } else { key })
}

fn file_name(key: &str) -> String {
    let mut s: String = key.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '.' }).collect();
    while s.contains("..") { s = s.replace("..", "."); }
    format!("{}.txt", s.trim_matches('.'))
}

pub fn load(entry: &str) -> Option<BTreeMap<String, String>> {
    let dir = store_dir(entry);
    if !dir.exists() { return None; }
    let mut out = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map_or(false, |x| x == "txt") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    if let Some(first) = text.lines().next() {
                        let key = first.split(" (").next().unwrap_or(first).trim_end_matches(" →").to_string();
                        // pages: "page / → view" — key is "page /"
                        let key = if first.starts_with("page ") { format!("page {}", first.split(' ').nth(1).unwrap_or("")) } else { key };
                        out.insert(key, text);
                    }
                }
            }
        }
    }
    Some(out)
}

pub fn save_all(entry: &str, artifacts: &Artifacts) {
    let dir = store_dir(entry);
    std::fs::create_dir_all(&dir).ok();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() { let _ = std::fs::remove_file(e.path()); }
    }
    for (key, text) in &artifacts.entries {
        std::fs::write(dir.join(file_name(key)), text).ok();
    }
}

/// Accept one declaration's new text (or its removal).
pub fn accept(entry: &str, key: &str, now: &str) {
    let dir = store_dir(entry);
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(file_name(key));
    if now.is_empty() { let _ = std::fs::remove_file(path); } else { std::fs::write(path, now).ok(); }
}

// ---------------------------------------------------------------- the diff

/// The changed lines of two texts, paired by likeness, each pair shown as
/// the part that differs with a little of what surrounds it.
pub fn compact_diff(was: &str, now: &str) -> Vec<String> {
    let old_set: BTreeSet<&str> = was.lines().collect();
    let new_set: BTreeSet<&str> = now.lines().collect();
    let removed: Vec<&str> = was.lines().filter(|l| !new_set.contains(l)).collect();
    let mut added: Vec<&str> = now.lines().filter(|l| !old_set.contains(l)).collect();
    let mut out = Vec::new();
    for r in removed {
        // the added line sharing the most with this one is its new form
        let best = added.iter().enumerate().map(|(i, a)| (common_prefix(r, a) + common_suffix(r, a), i)).max();
        match best {
            Some((score, i)) if score > 0 && score * 2 >= r.len().min(added[i].len()) => {
                let a = added.remove(i);
                let mut pre = common_prefix(r, a);
                let mut suf = common_suffix(r, a).min(r.len() - pre).min(a.len() - pre);
                // widen to whole words, so `lte` → `lt` reads as such
                let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
                while pre > 0 && is_word(r.as_bytes()[pre - 1]) && (r.len() > pre && is_word(r.as_bytes()[pre]) || a.len() > pre && is_word(a.as_bytes()[pre])) { pre -= 1; }
                while suf > 0 && is_word(r.as_bytes()[r.len() - suf]) && (r.len() - suf > pre && is_word(r.as_bytes()[r.len() - suf - 1]) || a.len() - suf > pre && is_word(a.as_bytes()[a.len() - suf - 1])) { suf -= 1; }
                let (r_mid, a_mid) = (&r[pre..r.len() - suf], &a[pre..a.len() - suf]);
                let lead: String = r[..pre].chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
                let tail: String = r[r.len() - suf..].chars().take(30).collect();
                let ell_l = if pre > 40 { "…" } else { "" };
                let ell_r = if suf > 30 { "…" } else { "" };
                out.push(format!("~ {}{}«{}» → «{}»{}{}", ell_l, lead.trim_start(), r_mid, a_mid, tail, ell_r));
            }
            _ => out.push(format!("- {}", shorten(r))),
        }
    }
    for a in added { out.push(format!("+ {}", shorten(a))); }
    out
}

fn shorten(l: &str) -> String {
    let t = l.trim_end();
    if t.chars().count() > 160 { format!("{}…", t.chars().take(160).collect::<String>()) } else { t.to_string() }
}

fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.bytes().zip(b.bytes()) { if x == y { n += 1; } else { break; } }
    while n > 0 && !a.is_char_boundary(n) { n -= 1; }
    n
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.bytes().rev().zip(b.bytes().rev()) { if x == y { n += 1; } else { break; } }
    while n > 0 && !a.is_char_boundary(a.len() - n) { n -= 1; }
    n
}

/// Lines removed and added between two texts, in order.
#[cfg(test)]
pub fn line_diff(was: &str, now: &str) -> String {
    let old: Vec<&str> = was.lines().collect();
    let new: Vec<&str> = now.lines().collect();
    let old_set: BTreeSet<&str> = old.iter().cloned().collect();
    let new_set: BTreeSet<&str> = new.iter().cloned().collect();
    let mut out = String::new();
    for l in &old { if !new_set.contains(l) { out.push_str(&format!("- {}\n", l.trim_end())); } }
    for l in &new { if !old_set.contains(l) { out.push_str(&format!("+ {}\n", l.trim_end())); } }
    out
}

/// Compare the accepted baseline with the program's artifacts.
pub fn compare(old: &BTreeMap<String, String>, art: &Artifacts) -> (Vec<Change>, Summary) {
    let mut changes = Vec::new();
    let mut summary = Summary::default();
    for (key, now) in &art.entries {
        match old.get(key) {
            Some(was) if was == now => summary.unchanged += 1,
            Some(was) => {
                summary.changed += 1;
                changes.push(Change { key: key.clone(), file: art.files.get(key).cloned().unwrap_or_default(), was: was.clone(), now: now.clone(), reach: art.reach.get(key).cloned().unwrap_or_default() });
            }
            None => summary.added += 1,
        }
    }
    for (key, was) in old {
        if !art.entries.contains_key(key) {
            summary.removed += 1;
            changes.push(Change { key: key.clone(), file: String::new(), was: was.clone(), now: String::new(), reach: Vec::new() });
        }
    }
    (changes, summary)
}

/// The files git says changed since the last commit, relative to the
/// repository root; empty outside a repository.
pub fn git_changed_files(dir: &std::path::Path) -> Vec<String> {
    // an entry in the current directory has an empty parent: that is here
    let dir = if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir };
    let mut out = Vec::new();
    for args in [vec!["diff", "--name-only", "HEAD"], vec!["ls-files", "--others", "--exclude-standard"]] {
        if let Ok(o) = std::process::Command::new("git").args(&args).current_dir(dir).output() {
            if o.status.success() {
                for l in String::from_utf8_lossy(&o.stdout).lines() { if !l.trim().is_empty() { out.push(l.trim().to_string()); } }
            }
        }
    }
    out
}

/// Whether a declaration's file is among the changed ones (by suffix, so
/// repository-relative and project-relative paths agree).
pub fn is_expected(file: &str, changed: &[String]) -> bool {
    // outside a repository, or with nothing changed since the commit,
    // there is no edit to attribute to: nothing is claimed as reached
    if changed.is_empty() { return true; }
    !file.is_empty() && changed.iter().any(|c| c == file || c.ends_with(&format!("/{}", file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests that write into the repository's tree, or read it through
    /// git, take turns: a store written while git is asked shows up in one
    /// answer and not the other.
    static TREE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn artifacts(src: &str) -> (Artifacts, Vec<Rule>) {
        let stmts = crate::build(src).expect("parses");
        let lowered = crate::lower::lower_program(&stmts).expect("lowers");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&lowered);
        derive_artifacts(&lowered, &ctx, &BTreeMap::new(), "main.co")
    }

    const APP: &str = "use web, ui\nsession_id = 0\nending = false\nstatus = ''\nwin_from = '06:55'\nwin_to = '09:30'\nfn say[msg]\n    status = msg\nfn start[]\n    t0 = win_from\n    t1 = win_to\n    if t1 lte t0\n        say['the window ends before it starts']\n        return\n    session_id = 1\n    say['started']\nfn ask_end[]\n    ending = true\nfn cancel_end[]\n    ending = false\nfn end_session[]\n    ending = false\n    session_id = 0\nview setup[]\n    return row[field['From', win_from], button['Start', click = start]]\nview session_view[]\n    return stack[p['`status`'], button['End session', click = ask_end], modal[button['End', click = end_session], button['Keep going', click = cancel_end], open = ending]]\nview render[]\n    if session_id gt 0\n        return session_view[]\n    return setup[]\npage['/', render]\n";

    /// Each file's reach is what its functions, derivations and top level
    /// can touch, by capability, transitively through what they call; a
    /// file that touches nothing says so (second-half-design.md §1).
    #[test]
    fn a_files_reach_is_what_it_can_touch() {
        let src = "fn keep[s]\n    write['out.txt', s]\nfn stamp[s]\n    t = now[]\n    keep['`s` `t`']\n    return t\nfn pure[x]\n    return x + 1\nlog[stamp['x'], pure[1]]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).expect("parses")).expect("lowers");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let program = crate::lower_ir::lower(&stmts, &mut ctx, "main").expect("lowers to the IR");
        let reach = reach_entries(&program, &BTreeMap::new(), "main.co");
        assert_eq!(reach["reach main.co"].1, "reach main.co\n  files: write\n  clock: now\n");
        let quiet = "fn pure[x]\n    return x + 1\nlog[pure[1]]\n";
        let stmts = crate::lower::lower_program(&crate::build(quiet).expect("parses")).expect("lowers");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let program = crate::lower_ir::lower(&stmts, &mut ctx, "main").expect("lowers to the IR");
        assert_eq!(reach_entries(&program, &BTreeMap::new(), "main.co")["reach main.co"].1, "reach main.co\n  nothing\n");
    }

    /// A local holding a tree is the path's own: the branch that reassigns
    /// it does not reassign it for its sibling (the app's book drew one
    /// state of such a page twice)
    #[test]
    fn a_local_tree_is_each_paths_own() {
        let src = "use web, ui\nn = 0\nfn go[]\n    n = n + 1\nview first[]\n    return p['first']\nview second[]\n    return button['second', click = go]\nview render[]\n    body = first[]\n    if n gt 0\n        body = second[]\n    return stack[body]\npage['/', render]\n";
        let (art, _) = artifacts(src);
        let shown: Vec<String> = art.structure.wires["render"].iter().map(|(_, node)| node.kids.iter().map(|k| k.view.clone()).collect::<Vec<_>>().join(",")).collect();
        assert!(shown.contains(&"first".to_string()) && shown.contains(&"second".to_string()), "{:?}", shown);
    }

    /// A value's field read in a view is a read, whatever its name: a
    /// frame's `title` column (`card.title[0]`) is not the `title` element,
    /// which is drawn by its bare name (the task board's page changed its
    /// baseline when `title` joined the vocabulary).
    #[test]
    fn a_field_named_like_an_element_is_read_not_drawn() {
        let src = "use web, ui, frame\ncards = with[frame.new[], 'title', ['ship']]\nview render[]\n    return stack[h2[cards.title[0]], title['Board']]\npage['/', render]\n";
        let (art, _) = artifacts(src);
        let tree = &art.entries["view render"];
        assert!(tree.contains("h2[] cards.title[0]"), "the column is read: {}", tree);
        assert_eq!(tree.matches("title[]").count(), 1, "the element alone is drawn: {}", tree);
    }

    /// Each page's reachable states: every control with when it is offered,
    /// over the facts the guards read; a control moved out of its guard is
    /// its line changing (B2)
    #[test]
    fn each_control_is_said_with_when_it_is_offered() {
        let (a, _) = artifacts(APP);
        let s = &a.entries["states /"];
        // the facts a guard reads, a handler's condition included (`t1 lte t0`)
        assert!(s.contains("  facts: ending ∈ {false, true} · session_id ∈ {0, 1} · win_from ∈ {'06:55', *} · win_to ∈ {'09:30'}\n"), "{}", s);
        assert!(s.contains("  button 'Start' → start: when session_id = 0\n"), "{}", s);
        assert!(s.contains("  button 'End session' → ask_end: when session_id = 1\n"), "{}", s);
        // the modal's buttons only while it is open, which only a session opens
        assert!(s.contains("  button 'End' → end_session: when ending = true\n"), "{}", s);
        assert!(s.contains("  button 'Keep going' → cancel_end: when ending = true\n"), "{}", s);
        assert!(s.contains("  field 'From' binds win_from: when session_id = 0\n"), "{}", s);
        let moved = APP.replace("row[field['From', win_from], button['Start', click = start]]", "row[field['From', win_from], button['Start', click = start], button['End session', click = ask_end]]");
        let (b, _) = artifacts(&moved);
        assert!(b.entries["states /"].contains("  button 'End session' → ask_end: always\n"), "{}", b.entries["states /"]);
        let (changes, _) = compare(&a.entries, &b);
        assert!(changes.iter().any(|c| c.key == "states /"), "{:?}", changes.iter().map(|c| &c.key).collect::<Vec<_>>());
    }

    /// Locals are substituted, so a renamed temporary or an inlined helper
    /// leaves the signature unchanged; a flipped guard changes it.
    #[test]
    fn a_refactor_is_silent_and_a_guard_change_is_not() {
        let (a, _) = artifacts(APP);
        let renamed = APP.replace("t0 = win_from\n    t1 = win_to\n    if t1 lte t0", "from_ = win_from\n    upto = win_to\n    if upto lte from_");
        let (b, _) = artifacts(&renamed);
        assert_eq!(a.entries["handler start"], b.entries["handler start"], "{}", a.entries["handler start"]);
        let inlined = APP.replace("    say['started']\n", "    status = 'started'\n");
        let (c, _) = artifacts(&inlined);
        assert_eq!(a.entries["handler start"], c.entries["handler start"], "a helper's write is the handler's write\n{}", a.entries["handler start"]);
        let flipped = APP.replace("if t1 lte t0", "if t1 lt t0");
        let (d, _) = artifacts(&flipped);
        assert_ne!(a.entries["handler start"], d.entries["handler start"]);
        let diff = line_diff(&a.entries["handler start"], &d.entries["handler start"]);
        assert!(diff.contains("- ") && diff.contains("+ ") && diff.contains("lt"), "{}", diff);
    }

    /// The signature reads as paths with their writes, and the wiring names
    /// where the handler fires and what it reaches.
    #[test]
    fn a_signature_is_paths_with_writes_and_reach() {
        let (a, _) = artifacts(APP);
        let s = &a.entries["handler start"];
        assert!(s.contains("when (win_to lte win_from):"), "{}", s);
        assert!(s.contains("status := 'the window ends before it starts'"), "{}", s);
        assert!(s.contains("session_id := 1 · status := 'started'"), "{}", s);
        assert!(s.contains("wired: setup button 'Start'"), "{}", s);
        let reach = &a.reach["handler start"];
        assert!(reach.iter().any(|r| r == "page /") && reach.iter().any(|r| r == "view render"), "{:?}", reach);
        let v = &a.entries["view render"];
        assert!(v.contains("when (session_id gt 0):\n    @session_view[]"), "{}", v);
        let sv = &a.entries["view session_view"];
        assert!(sv.contains("modal[open = ending]") && sv.contains("button[click → end_session] 'End'"), "{}", sv);
        let f = &a.entries["fact ending"];
        assert!(f.contains("written by: ask_end, cancel_end, end_session"), "{}", f);
        let setup = &a.entries["view setup"];
        assert!(setup.contains("field[binds win_from] 'From'"), "a bound input shows its binding, not its lowered arguments\n{}", setup);
        let say = &a.entries["handler say"];
        assert!(say.contains("called by: start"), "a helper says who calls it\n{}", say);
    }

    /// A draw from chance or the clock held in a local is one value, named,
    /// however many times the local is read.
    #[test]
    fn a_random_draw_is_one_value() {
        let src = "use web, ui\nsym = ''\nstatus = ''\nnames = ['a', 'b']\nfn pick[]\n    i = rand[len[names]]\n    sym = names[i]\n    status = 'picked `names[i]`'\nview render[]\n    return button['Pick', click = pick]\npage['/', render]\n";
        let (a, _) = artifacts(src);
        let s = &a.entries["handler pick"];
        assert!(s.contains("reads v1 = rand[len[names]]") && s.contains("sym := names[v1]") && !s.contains("names[rand"), "{}", s);
    }

    /// The kernel rules: a modal nobody can close, a dead control.
    #[test]
    fn kernel_rules_find_an_unclosable_modal_and_a_dead_control() {
        let (_, rules) = artifacts(APP);
        assert!(rules.is_empty(), "{:?}", rules);
        let unclosable = APP.replace("fn cancel_end[]\n    ending = false\n", "fn cancel_end[]\n    ending = true\n").replace("fn end_session[]\n    ending = false\n    session_id = 0\n", "fn end_session[]\n    session_id = 0\n");
        let (_, rules) = artifacts(&unclosable);
        assert!(rules.iter().any(|r| r.detail.contains("no way to close")), "{:?}", rules);
        let dead = APP.replace("fn ask_end[]\n    ending = true\n", "fn ask_end[]\n    x = 1\n");
        let (_, rules) = artifacts(&dead);
        assert!(rules.iter().any(|r| r.detail.contains("dead control")), "{:?}", rules);
    }

    /// A guard a helper already decided is no fork, a value from a call is
    /// named, and the compact diff shows the token that moved.
    #[test]
    fn dead_branches_are_pruned_and_call_values_are_named() {
        let src = "use web, ui\nplaying = false\nbase = 0.0\nstatus = ''\nfn begin[]\n    playing = false\nfn toggle[]\n    if playing\n        playing = false\n    else\n        playing = true\nfn moment[d]\n    return d * 2.0\nfn start[]\n    begin[]\n    t = moment[base]\n    if t gt 1.0\n        status = 'late'\n    toggle[]\nview render[]\n    return button['Start', click = start]\npage['/', render]\n";
        let (a, _) = artifacts(src);
        let s = &a.entries["handler start"];
        assert!(!s.contains("when false") && !s.contains("(not false)"), "the helper set playing false, so toggle's other branch is dead\n{}", s);
        assert!(s.contains("reads v1 = moment[base]") && s.contains("when (v1 gt 1.0):"), "{}", s);
        assert!(s.contains("playing := true"), "{}", s);
        let flipped = src.replace("if t gt 1.0", "if t gte 1.0");
        let (b, _) = artifacts(&flipped);
        let lines = compact_diff(&a.entries["handler start"], &b.entries["handler start"]);
        assert!(lines.iter().any(|l| l.contains("«gt» → «gte»")), "{:?}", lines);
    }

    /// The structure reads as a tree from the page down, with guards,
    /// controls and bindings in place.
    #[test]
    fn the_structure_is_the_app_as_a_tree() {
        let (a, _) = artifacts(APP);
        let s = render_structure(&a, "main.co");
        assert!(s.contains("page / → @render (main.co)"), "{}", s);
        assert!(s.contains("when (session_id gt 0):\n    @session_view (main.co)   button 'End session' → ask_end · button 'End' → end_session · button 'Keep going' → cancel_end"), "{}", s);
        assert!(s.contains("@setup (main.co)   binds win_from · button 'Start' → start"), "{}", s);
        assert!(s.contains("start (main.co)   writes session_id, status"), "{}", s);
        assert!(s.contains("ending (main.co)   written by ask_end, cancel_end, end_session · read by render, session_view"), "{}", s);
    }

    /// The entry's own directory is an empty path; git is asked from here.
    #[test]
    fn an_empty_directory_is_the_current_one() {
        let _tree = TREE.lock().unwrap_or_else(|e| e.into_inner());
        // (another module's test can write into the tree between the two
        // questions: the same answer twice in a row is the property)
        let same = (0..3).any(|_| git_changed_files(std::path::Path::new(".")) == git_changed_files(std::path::Path::new("")));
        assert!(same, "'.' and '' name different directories");
    }

    /// The store round-trips and the diff attributes removals.
    #[test]
    fn the_store_round_trips_and_compare_reports_changes() {
        let _tree = TREE.lock().unwrap_or_else(|e| e.into_inner());
        let (a, _) = artifacts(APP);
        let entry = format!("scratch_baseline_{}.co", std::process::id());
        let dir = store_dir(&entry);
        let _ = std::fs::remove_dir_all(&dir);
        save_all(&entry, &a);
        let loaded = load(&entry).expect("saved");
        assert_eq!(loaded.len(), a.entries.len(), "{:?}", loaded.keys());
        let (changes, summary) = compare(&loaded, &a);
        assert!(changes.is_empty() && summary.unchanged == a.entries.len(), "{:?} {:?}", changes, summary);
        let flipped = APP.replace("if t1 lte t0", "if t1 lt t0");
        let (b, _) = artifacts(&flipped);
        let (changes, summary) = compare(&loaded, &b);
        assert_eq!(summary.changed, 1);
        assert_eq!(changes[0].key, "handler start");
        accept(&entry, "handler start", &b.entries["handler start"]);
        let reloaded = load(&entry).unwrap();
        assert_eq!(reloaded["handler start"], b.entries["handler start"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

