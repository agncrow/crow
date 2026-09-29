//! AST lowering: resolves named arguments into plain positional calls.
//!
//! Runs once, after parsing (and module expansion), before type inference,
//! validation, interpretation, or compilation. Downstream code never sees
//! `Expr::Named` — this is the single implementation of named-argument
//! semantics for both execution modes (shared-kernel principle, see
//! docs/design-review.md pushback 6).
//!
//! What it does per call:
//! - user functions / structs: named args are placed into their parameter
//!   slots; omitted parameters with defaults are filled with the default
//!   literal. Errors on unknown names, duplicates, and missing parameters.
//! - builtins with named parameters (axis, desc, ...): named args are
//!   reordered to their positional slots. No default filling — builtin arity
//!   is meaningful (e.g. `sum[x]` vs `sum[x, axis]` return different shapes).
//! - `map[...]`: the variadic named constructor. `map[a = 1, b = 2]` desugars
//!   to `set[set[map[], 'a', 1], 'b', 2]` — existing primitives, zero new
//!   runtime code.
//! - UI elements (`use ui`): named args become the attribute map:
//!   `button['+', click = inc]` → `button[[click = inc], '+']`.

use crate::ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

/// HTML element constructors that accept trailing named attributes.
pub const UI_ELEMENTS: &[&str] = &[
    "div", "span", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "button", "ul", "ol", "li", "form", "a", "nav",
    "header", "footer", "section", "tr", "td", "th",
    "thead", "tbody", "option", "textarea", "label",
    "pre", "code", "input", "img", "table",
    // the layout vocabulary's containers (render as div; see crow_runtime::layout)
    "stack", "row", "grid", "overlay", "scroll",
    // revealing: disclosure, a dropdown panel, and the three layers
    "details", "summary", "dropdown", "modal", "drawer", "toast",
    // a rule between siblings
    "divider",
    // the one way out of the vocabulary: the program's own module draws it
    "widget",
    // a page's head: what a served page carries in its head, not its body
    "title", "meta",
];

/// The controls a view draws that are builtins, not elements: like an
/// element or a view, each renders what it is given, and a failed argument
/// is its reason in place of the control (spec, Auto-Propagation).
pub const UI_CONTROLS: &[&str] = &["field", "select", "slider", "checkbox", "toggle", "radio", "tabs", "progress", "icon"];

/// Whether a call in a view is one of the controls: `select` of a frame
/// is the frame module's column selection, not the control.
pub(crate) fn is_ui_control(name: &str, first_is_frame: bool) -> bool {
    UI_CONTROLS.contains(&name) && !(name == "select" && first_is_frame)
}

/// The pairs of a map literal as the lowering leaves it: `map[k = v, …]`
/// becomes a chain of `set[…]` calls over `map[]`; a bare `Expr::Map` is
/// read too. `None` for anything else (a variable, a call with dynamic keys).
pub fn map_literal_pairs(e: &Expr) -> Option<Vec<(String, Expr)>> {
    if let Expr::Map(pairs) = e {
        return Some(pairs.iter().filter_map(|(k, v)| match k {
            Expr::Ident(s) | Expr::Str(s) => Some((s.clone(), v.clone())),
            _ => None,
        }).collect());
    }
    let mut out: Vec<(String, Expr)> = Vec::new();
    let mut cur = e;
    loop {
        match cur {
            Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(n) if n == "set") && args.len() == 3 => {
                match &args[1] {
                    Expr::Str(k) => out.push((k.clone(), args[2].clone())),
                    _ => return None,
                }
                cur = &args[0];
            }
            Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(n) if n == "map") && args.is_empty() => {
                out.reverse();
                return Some(out);
            }
            _ => return None,
        }
    }
}

/// `chart[..., seek = handler]` after lowering: twelve positional args with
/// the handler's name in the last slot. Every pass that discovers event
/// handlers asks this so the seek target counts as one.
pub(crate) fn chart_seek_handler<'a>(callee: &Expr, args: &'a [Expr]) -> Option<&'a str> {
    match callee {
        Expr::Ident(n) if n == "chart" && args.len() == 12 => match &args[11] {
            Expr::Ident(f) => Some(f.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Builtins with named parameters: name → ordered named-parameter list.
/// Position of a named parameter = (call arity without it) + its index here.
fn builtin_named_params(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "sum" | "mean" | "min" | "max" | "prod" | "stack" | "concat" => Some(&["axis"]),
        "sort" => Some(&["desc"]),
        // test harness: app[view, width = 1280, wide = true]
        "app" => Some(&["width", "wide"]),
        _ => None,
    }
}

#[derive(Clone)]
struct FnSig {
    params: Vec<String>,
    defaults: Vec<Option<Expr>>,
}

#[derive(Clone)]
pub struct Lowerer {
    fns: BTreeMap<String, FnSig>,
    structs: BTreeMap<String, Vec<String>>,
    current_line: usize,
    /// Lowering inside a `view` body — ui element named-args resolve here
    in_view: bool,
    /// The program imports web: `reader` is the page's signed-in reader
    web: bool,
}

thread_local! {
    /// Whether the program lowered last on this thread uses web: the
    /// interpolations parsed from its strings later are lowered as it was.
    static PROGRAM_WEB: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The functions and structs of the program lowered last on this
    /// thread: a call in one of its interpolations takes its named
    /// arguments and defaults as a call outside the string does.
    static PROGRAM_SIGS: std::cell::RefCell<(BTreeMap<String, FnSig>, BTreeMap<String, Vec<String>>)> = const { std::cell::RefCell::new((BTreeMap::new(), BTreeMap::new())) };
}

pub fn lower_program(stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
    let web = stmts.iter().any(|s| matches!(s, Stmt::Use { path, alias: None } if path == "web" || path.starts_with("web.")));
    PROGRAM_WEB.with(|w| w.set(web));
    let mut l = Lowerer::new(web);
    l.collect_sigs(stmts);
    PROGRAM_SIGS.with(|p| *p.borrow_mut() = (l.fns.clone(), l.structs.clone()));
    l.finish(stmts)
}

/// An expression parsed from a string's interpolation, lowered as the
/// program around it was — its `reader` is the page's when the program
/// uses web, as it is outside the string, and its calls know the
/// program's functions.
pub fn lower_fragment(stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
    let mut l = Lowerer::new(PROGRAM_WEB.with(|w| w.get()));
    PROGRAM_SIGS.with(|p| {
        let (fns, structs) = &*p.borrow();
        l.fns = fns.clone();
        l.structs = structs.clone();
    });
    l.collect_sigs(stmts);
    l.finish(stmts)
}

impl Lowerer {
    fn new(web: bool) -> Self {
        Lowerer { fns: BTreeMap::new(), structs: BTreeMap::new(), current_line: 0, in_view: false, web }
    }

    fn finish(mut self, stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
        let lowered = self.lower_stmts(stmts)?;
        let scoped = scoped(lowered)?;
        Ok(narrowed(scoped, &BTreeSet::new()))
    }
}

// ── names, scoped ───────────────────────────────────────────────────

/// Variables are scoped to the block they are bound in (spec §1, Scope):
/// a name bound again after its block has ended is a new variable, and a
/// read of it there is refused. The passes after this one keep a
/// variable's type by its name, so each later binding of a name whose
/// earlier binding's block has ended gets a name of its own (`x__s2`);
/// messages print the name the program wrote (`shown`). Names visible
/// beyond a block (the top level's, functions, structs, modules) are never
/// renamed.
fn scoped(stmts: Vec<Stmt>) -> Result<Vec<Stmt>, String> {
    let mut globals: BTreeSet<String> = BTreeSet::new();
    for s in &stmts {
        match s {
            Stmt::Assign { name, .. } => {
                globals.insert(name.clone());
            }
            Stmt::MultiAssign { names, .. } => globals.extend(names.iter().cloned()),
            Stmt::FnDecl { name, .. } | Stmt::StructDecl { name, .. } | Stmt::Derive { name, .. } => {
                globals.insert(name.clone());
            }
            Stmt::Use { path, alias } => {
                globals.insert(alias.clone().unwrap_or_else(|| path.clone()));
            }
            _ => {}
        }
    }
    let mut namer = Namer::new(&globals, &[]);
    namer.block(stmts, false)
}

/// The suffix a renamed binding carries, and the name a message prints.
const SCOPED: &str = "__s";

/// A name as the program wrote it: `x__s2` is `x`, in every message.
pub fn shown(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;
    while let Some(i) = rest.find(SCOPED) {
        let after = &rest[i + SCOPED.len()..];
        let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
        let before_ok = i > 0 && rest[..i].chars().last().map_or(false, |c| c.is_alphanumeric() || c == '_');
        if digits > 0 && before_ok && !after[digits..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            out.push_str(&rest[..i]);
            rest = &after[digits..];
        } else {
            out.push_str(&rest[..i + SCOPED.len()]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

struct Namer<'a> {
    globals: &'a BTreeSet<String>,
    /// per open block: the name written → the variable it is now
    scopes: Vec<BTreeMap<String, String>>,
    /// how many variables of each written name this body has had
    seen: BTreeMap<String, usize>,
    /// names whose variable's block has ended (a read of one is refused)
    closed: BTreeSet<String>,
    line: usize,
}

impl<'a> Namer<'a> {
    fn new(globals: &'a BTreeSet<String>, params: &[String]) -> Self {
        let mut top = BTreeMap::new();
        let mut seen = BTreeMap::new();
        for p in params {
            top.insert(p.clone(), p.clone());
            seen.insert(p.clone(), 1);
        }
        Namer { globals, scopes: vec![top], seen, closed: BTreeSet::new(), line: 0 }
    }

    fn visible(&self, name: &str) -> Option<&String> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// A binding: the variable already in scope, or a new one, named apart
    /// when an earlier one of the same name has ended.
    fn bind(&mut self, name: &str) -> String {
        if self.globals.contains(name) {
            return name.to_string();
        }
        if let Some(v) = self.visible(name) {
            return v.clone();
        }
        let n = self.seen.entry(name.to_string()).or_insert(0);
        *n += 1;
        let v = if *n == 1 { name.to_string() } else { format!("{}{}{}", name, SCOPED, n) };
        self.scopes.last_mut().unwrap().insert(name.to_string(), v.clone());
        v
    }

    fn read(&self, name: &str) -> Result<String, String> {
        if let Some(v) = self.visible(name) {
            return Ok(v.clone());
        }
        if self.closed.contains(name) && !self.globals.contains(name) {
            return Err(format!(
                "line {}: '{}' is not in scope here: it was set inside a block that has ended. A variable belongs to the block it is set in: set it before the block ({} = …) to use it after",
                self.line, name, name
            ));
        }
        Ok(name.to_string())
    }

    fn open(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn close(&mut self) {
        if let Some(ended) = self.scopes.pop() {
            for name in ended.into_keys() {
                if self.visible(&name).is_none() {
                    self.closed.insert(name);
                }
            }
        }
    }

    /// A block's statements; `own` opens a scope of its own for them.
    fn block(&mut self, stmts: Vec<Stmt>, own: bool) -> Result<Vec<Stmt>, String> {
        if own {
            self.open();
        }
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            out.push(self.stmt(s)?);
        }
        if own {
            self.close();
        }
        Ok(out)
    }

    fn stmt(&mut self, s: Stmt) -> Result<Stmt, String> {
        Ok(match s {
            Stmt::Line(n) => {
                self.line = n;
                Stmt::Line(n)
            }
            Stmt::Assign { name, value } => {
                let value = self.expr(value)?;
                Stmt::Assign { name: self.bind(&name), value }
            }
            Stmt::MultiAssign { names, values } => {
                let values = values.into_iter().map(|v| self.expr(v)).collect::<Result<Vec<_>, _>>()?;
                Stmt::MultiAssign { names: names.iter().map(|n| self.bind(n)).collect(), values }
            }
            // a function, a derivation and a test block are bodies of their own
            Stmt::FnDecl { name, params, defaults, body, is_view } => {
                let mut inner = Namer::new(self.globals, &params);
                Stmt::FnDecl { name, params, defaults, body: inner.block(body, false)?, is_view }
            }
            Stmt::Derive { name, fresh, body } => {
                let mut inner = Namer::new(self.globals, &[]);
                Stmt::Derive { name, fresh, body: inner.block(body, false)? }
            }
            Stmt::TestBlock { name, params, body } => {
                let mut inner = Namer::new(self.globals, &params);
                Stmt::TestBlock { name, params, body: inner.block(body, false)? }
            }
            Stmt::Return(v) => Stmt::Return(v.map(|x| self.expr(x)).transpose()?),
            Stmt::ExprStmt(x) => Stmt::ExprStmt(self.expr(x)?),
            Stmt::If { condition, then_body, else_body } => Stmt::If {
                condition: self.expr(condition)?,
                then_body: self.block(then_body, true)?,
                else_body: else_body.map(|b| self.block(b, true)).transpose()?,
            },
            Stmt::While { condition, body } => Stmt::While { condition: self.expr(condition)?, body: self.block(body, true)? },
            Stmt::For { var, iter, body } => {
                let iter = self.expr(iter)?;
                self.open();
                let var = self.bind(&var);
                let body = self.block(body, false)?;
                self.close();
                Stmt::For { var, iter, body }
            }
            // the object of an assignment is the variable in scope
            Stmt::FieldAssign { object, field, value } => {
                let value = self.expr(value)?;
                Stmt::FieldAssign { object: self.expr(object)?, field, value }
            }
            Stmt::IndexAssign { object, index, value } => {
                let index = self.expr(index)?;
                let value = self.expr(value)?;
                Stmt::IndexAssign { object: self.expr(object)?, index, value }
            }
            Stmt::Resource { kind, count } => Stmt::Resource { kind, count: self.expr(count)? },
            other => other,
        })
    }

    fn expr(&mut self, e: Expr) -> Result<Expr, String> {
        Ok(match e {
            Expr::Ident(n) => Expr::Ident(self.read(&n)?),
            Expr::Str(s) if s.contains('`') => Expr::Str(self.spans(&s)?),
            Expr::BinOp { op, left, right } => Expr::BinOp { op, left: Box::new(self.expr(*left)?), right: Box::new(self.expr(*right)?) },
            Expr::UnaryOp { op, expr } => Expr::UnaryOp { op, expr: Box::new(self.expr(*expr)?) },
            // a function's name is not a variable
            Expr::Call { callee, args } => {
                let callee = match *callee {
                    Expr::Ident(n) => Expr::Ident(self.visible(&n).cloned().unwrap_or(n)),
                    other => self.expr(other)?,
                };
                Expr::Call { callee: Box::new(callee), args: args.into_iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()? }
            }
            Expr::FieldAccess { object, field } => Expr::FieldAccess { object: Box::new(self.expr(*object)?), field },
            Expr::Array(items) => Expr::Array(items.into_iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?),
            // a key is a name (an attribute's, a field's), not a read
            Expr::Map(pairs) => Expr::Map(pairs.into_iter().map(|(k, v)| Ok((k, self.expr(v)?))).collect::<Result<Vec<_>, String>>()?),
            Expr::Named { name, value } => Expr::Named { name, value: Box::new(self.expr(*value)?) },
            Expr::ForExpr { var, iter, body } => {
                let iter = self.expr(*iter)?;
                self.open();
                let var = self.bind(&var);
                let body = self.block(body, false)?;
                self.close();
                Expr::ForExpr { var, iter: Box::new(iter), body }
            }
            other => other,
        })
    }

    /// A string's interpolation spans, their variables renamed where the
    /// variable in scope was: each span is lexed, and a name token that is
    /// a read (not a field after `.`, nor a named argument before `=`) is
    /// spliced by its column.
    fn spans(&self, s: &str) -> Result<String, String> {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                out.push(chars[i]);
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if chars[i] != '`' {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            // a span: up to its closing backtick, quote-aware
            let start = i + 1;
            let mut j = start;
            let mut in_str = false;
            while j < chars.len() {
                let c = chars[j];
                if in_str {
                    if c == '\\' && j + 1 < chars.len() {
                        j += 2;
                        continue;
                    }
                    if c == '\'' {
                        in_str = false;
                    }
                } else if c == '\'' {
                    in_str = true;
                } else if c == '`' {
                    break;
                }
                j += 1;
            }
            let span: String = chars[start..j.min(chars.len())].iter().collect();
            out.push('`');
            out.push_str(&self.span(&span)?);
            if j < chars.len() {
                out.push('`');
            }
            i = j + 1;
        }
        Ok(out)
    }

    fn span(&self, span: &str) -> Result<String, String> {
        let tokens = match crate::lexer::Lexer::new(span).tokenize() {
            Ok(t) => t,
            Err(_) => return Ok(span.to_string()),
        };
        let chars: Vec<char> = span.chars().collect();
        // (column, length, replacement), spliced from the end
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for (k, t) in tokens.iter().enumerate() {
            if let crate::token::Token::Ident(n) = &t.token {
                let after_dot = k > 0 && matches!(tokens[k - 1].token, crate::token::Token::Dot);
                let named = matches!(tokens.get(k + 1).map(|x| &x.token), Some(crate::token::Token::Assign));
                if after_dot || named {
                    continue;
                }
                let v = self.read(n)?;
                if &v != n && t.col >= 1 {
                    edits.push((t.col - 1, n.chars().count(), v));
                }
            }
        }
        let mut text: Vec<char> = chars;
        for (col, len, v) in edits.into_iter().rev() {
            if col + len <= text.len() && text[col..col + len].iter().collect::<String>() != v {
                text.splice(col..col + len, v.chars());
            }
        }
        Ok(text.into_iter().collect())
    }
}

// ── a guard narrows ─────────────────────────────────────────────────

/// The hidden builtin a narrowed read becomes: `__known[x]` is `x`'s value,
/// typed as its payload (the IR's unwrap).
pub const KNOWN: &str = "__known";

/// `fmt[t, time = style, tz = zone]` once its named arguments are lowered:
/// a moment's time style, a row of its own that can fail (crate::table).
pub const FMT_TIME: &str = "__fmt_time";

/// A guard narrows (spec, Errors in Conditions): after `if not x.ok` with
/// no `else`, whose branch always leaves (`return`, `break`, `continue`),
/// `x` is known good for the rest of its block, and `if not a.ok or not
/// b.ok` narrows both. Its reads there become `__known[x]`, here, once, so
/// every pass after this one (inference, the checker, both backends) reads
/// the same type. An assignment to `x` ends it, and a loop does not carry
/// a name its body assigns into the body (the last pass may have made it
/// fail again).
fn narrowed(stmts: Vec<Stmt>, outer: &BTreeSet<String>) -> Vec<Stmt> {
    let mut known = outer.clone();
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        let s = narrow_stmt(s, &known);
        for n in assigned(std::slice::from_ref(&s)) {
            known.remove(&n);
        }
        known.extend(guarded(&s));
        out.push(s);
    }
    out
}

/// The names a guard statement makes known good for the rest of its block.
fn guarded(s: &Stmt) -> Vec<String> {
    if let Stmt::If { condition, then_body, else_body: None } = s {
        let leaves = matches!(then_body.iter().rev().find(|x| !matches!(x, Stmt::Line(_))), Some(Stmt::Return(_) | Stmt::Break | Stmt::Continue));
        let mut names = Vec::new();
        if leaves && failed_checks(condition, &mut names) {
            return names;
        }
    }
    Vec::new()
}

/// `not x.ok`, or an `or` of them: the names each checks.
fn failed_checks(e: &Expr, out: &mut Vec<String>) -> bool {
    match e {
        Expr::UnaryOp { op: crate::ast::UnaryOp::Not, expr } => match expr.as_ref() {
            Expr::FieldAccess { object, field } if field == "ok" => match object.as_ref() {
                Expr::Ident(n) => {
                    out.push(n.clone());
                    true
                }
                // already known good: nothing more to narrow
                Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(k) if k == KNOWN) && args.len() == 1 => true,
                _ => false,
            },
            _ => false,
        },
        Expr::BinOp { op: crate::ast::BinOp::Or, left, right } => failed_checks(left, out) && failed_checks(right, out),
        _ => false,
    }
}

/// What a loop's body inherits: not the names the body assigns, nor the
/// loop's own variable.
fn loop_known(known: &BTreeSet<String>, body: &[Stmt], var: Option<&str>) -> BTreeSet<String> {
    let written = assigned(body);
    known.iter().filter(|n| !written.contains(*n) && Some(n.as_str()) != var).cloned().collect()
}

fn narrow_stmt(s: Stmt, known: &BTreeSet<String>) -> Stmt {
    let e = |x: Expr| narrow_expr(x, known);
    match s {
        Stmt::Assign { name, value } => Stmt::Assign { name, value: e(value) },
        Stmt::MultiAssign { names, values } => Stmt::MultiAssign { names, values: values.into_iter().map(e).collect() },
        // a function, a derivation and a test block start with nothing known
        Stmt::FnDecl { name, params, defaults, body, is_view } => Stmt::FnDecl { name, params, defaults, body: narrowed(body, &BTreeSet::new()), is_view },
        Stmt::Derive { name, fresh, body } => Stmt::Derive { name, fresh, body: narrowed(body, &BTreeSet::new()) },
        Stmt::TestBlock { name, params, body } => Stmt::TestBlock { name, params, body: narrowed(body, &BTreeSet::new()) },
        Stmt::Return(v) => Stmt::Return(v.map(e)),
        Stmt::ExprStmt(x) => Stmt::ExprStmt(e(x)),
        Stmt::If { condition, then_body, else_body } => Stmt::If {
            condition: e(condition),
            then_body: narrowed(then_body, known),
            else_body: else_body.map(|b| narrowed(b, known)),
        },
        Stmt::For { var, iter, body } => {
            let inner = loop_known(known, &body, Some(&var));
            Stmt::For { var, iter: e(iter), body: narrowed(body, &inner) }
        }
        // the condition is tested again after every pass through the body
        Stmt::While { condition, body } => {
            let inner = loop_known(known, &body, None);
            Stmt::While { condition: narrow_expr(condition, &inner), body: narrowed(body, &inner) }
        }
        // the object of an assignment is a place, not a read
        Stmt::FieldAssign { object, field, value } => Stmt::FieldAssign { object, field, value: e(value) },
        Stmt::IndexAssign { object, index, value } => Stmt::IndexAssign { object, index: e(index), value: e(value) },
        Stmt::Resource { kind, count } => Stmt::Resource { kind, count: e(count) },
        other => other,
    }
}

fn narrow_expr(e: Expr, known: &BTreeSet<String>) -> Expr {
    let n = |x: Box<Expr>| Box::new(narrow_expr(*x, known));
    match e {
        Expr::Ident(name) if known.contains(&name) => Expr::Call { callee: Box::new(Expr::Ident(KNOWN.to_string())), args: vec![Expr::Ident(name)] },
        Expr::BinOp { op, left, right } => Expr::BinOp { op, left: n(left), right: n(right) },
        Expr::UnaryOp { op, expr } => Expr::UnaryOp { op, expr: n(expr) },
        // a function's name is not a value to narrow
        Expr::Call { callee, args } => {
            let callee = if matches!(callee.as_ref(), Expr::Ident(_)) { callee } else { n(callee) };
            Expr::Call { callee, args: args.into_iter().map(|a| narrow_expr(a, known)).collect() }
        }
        Expr::FieldAccess { object, field } => Expr::FieldAccess { object: n(object), field },
        Expr::Array(items) => Expr::Array(items.into_iter().map(|a| narrow_expr(a, known)).collect()),
        // a key is a name (an attribute's, a field's), not a read
        Expr::Map(pairs) => Expr::Map(pairs.into_iter().map(|(k, v)| (k, narrow_expr(v, known))).collect()),
        Expr::Named { name, value } => Expr::Named { name, value: n(value) },
        Expr::ForExpr { var, iter, body } => {
            let inner = loop_known(known, &body, Some(&var));
            Expr::ForExpr { var, iter: n(iter), body: narrowed(body, &inner) }
        }
        other => other,
    }
}

/// Every name a block assigns, inside its nested blocks and collecting
/// loops too (a function's body is its own).
pub(crate) fn assigned(stmts: &[Stmt]) -> BTreeSet<String> {
    fn in_expr(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::BinOp { left, right, .. } => {
                in_expr(left, out);
                in_expr(right, out);
            }
            Expr::UnaryOp { expr, .. } => in_expr(expr, out),
            Expr::Call { callee, args } => {
                in_expr(callee, out);
                args.iter().for_each(|a| in_expr(a, out));
            }
            Expr::FieldAccess { object, .. } => in_expr(object, out),
            Expr::Array(items) => items.iter().for_each(|a| in_expr(a, out)),
            Expr::Map(pairs) => pairs.iter().for_each(|(_, v)| in_expr(v, out)),
            Expr::Named { value, .. } => in_expr(value, out),
            Expr::ForExpr { iter, body, .. } => {
                in_expr(iter, out);
                in_stmts(body, out);
            }
            _ => {}
        }
    }
    fn in_stmts(stmts: &[Stmt], out: &mut BTreeSet<String>) {
        for s in stmts {
            match s {
                Stmt::Assign { name, value } => {
                    out.insert(name.clone());
                    in_expr(value, out);
                }
                Stmt::MultiAssign { names, values } => {
                    out.extend(names.iter().cloned());
                    values.iter().for_each(|v| in_expr(v, out));
                }
                Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => in_expr(e, out),
                Stmt::If { condition, then_body, else_body } => {
                    in_expr(condition, out);
                    in_stmts(then_body, out);
                    if let Some(b) = else_body {
                        in_stmts(b, out);
                    }
                }
                Stmt::For { iter, body, .. } => {
                    in_expr(iter, out);
                    in_stmts(body, out);
                }
                Stmt::While { condition, body } => {
                    in_expr(condition, out);
                    in_stmts(body, out);
                }
                Stmt::FieldAssign { value, .. } => in_expr(value, out),
                Stmt::IndexAssign { index, value, .. } => {
                    in_expr(index, out);
                    in_expr(value, out);
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    in_stmts(stmts, &mut out);
    out
}

impl Lowerer {
    fn collect_sigs(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::FnDecl { name, params, defaults, body, .. } => {
                    self.fns.insert(
                        name.clone(),
                        FnSig { params: params.clone(), defaults: defaults.clone() },
                    );
                    self.collect_sigs(body);
                }
                Stmt::StructDecl { name, fields } => {
                    self.structs.insert(name.clone(), fields.clone());
                }
                Stmt::If { then_body, else_body, .. } => {
                    self.collect_sigs(then_body);
                    if let Some(eb) = else_body {
                        self.collect_sigs(eb);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    self.collect_sigs(body);
                }
                _ => {}
            }
        }
    }

    fn err(&self, msg: String) -> String {
        if self.current_line > 0 {
            format!("line {}: {}", self.current_line, msg)
        } else {
            msg
        }
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
        stmts.iter().map(|s| self.lower_stmt(s)).collect()
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<Stmt, String> {
        Ok(match stmt {
            Stmt::Line(n) => {
                self.current_line = *n;
                Stmt::Line(*n)
            }
            Stmt::Assign { name, value } => Stmt::Assign {
                name: name.clone(),
                value: self.lower_expr(value)?,
            },
            Stmt::MultiAssign { names, values } => Stmt::MultiAssign {
                names: names.clone(),
                values: values.iter().map(|v| self.lower_expr(v)).collect::<Result<_, _>>()?,
            },
            Stmt::FnDecl { name, params, defaults, body, is_view } => {
                // Element vocabulary is active only inside views
                let saved = self.in_view;
                self.in_view = *is_view;
                let lowered = Stmt::FnDecl {
                    name: name.clone(),
                    params: params.clone(),
                    defaults: defaults.clone(),
                    body: self.lower_stmts(body)?,
                    is_view: *is_view,
                };
                self.in_view = saved;
                lowered
            }
            Stmt::Return(e) => Stmt::Return(match e {
                Some(e) => Some(self.lower_expr(e)?),
                None => None,
            }),
            Stmt::ExprStmt(e) => Stmt::ExprStmt(self.lower_expr(e)?),
            Stmt::If { condition, then_body, else_body } => Stmt::If {
                condition: self.lower_expr(condition)?,
                then_body: self.lower_stmts(then_body)?,
                else_body: match else_body {
                    Some(eb) => Some(self.lower_stmts(eb)?),
                    None => None,
                },
            },
            Stmt::For { var, iter, body } => Stmt::For {
                var: var.clone(),
                iter: self.lower_expr(iter)?,
                body: self.lower_stmts(body)?,
            },
            Stmt::While { condition, body } => Stmt::While {
                condition: self.lower_expr(condition)?,
                body: self.lower_stmts(body)?,
            },
            Stmt::TestBlock { name, params, body } => Stmt::TestBlock {
                name: name.clone(),
                params: params.clone(),
                body: self.lower_stmts(body)?,
            },
            Stmt::Derive { name, fresh, body } => Stmt::Derive {
                name: name.clone(),
                fresh: *fresh,
                body: self.lower_stmts(body)?,
            },
            Stmt::FieldAssign { object, field, value } => Stmt::FieldAssign {
                object: self.lower_expr(object)?,
                field: field.clone(),
                value: self.lower_expr(value)?,
            },
            Stmt::IndexAssign { object, index, value } => Stmt::IndexAssign {
                object: self.lower_expr(object)?,
                index: self.lower_expr(index)?,
                value: self.lower_expr(value)?,
            },
            other => other.clone(),
        })
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<Expr, String> {
        Ok(match expr {
            // a for expression's body is a block like any other: its map
            // literals and named arguments lower the same way
            Expr::ForExpr { var, iter, body } => Expr::ForExpr {
                var: var.clone(),
                iter: Box::new(self.lower_expr(iter)?),
                body: self.lower_stmts(body)?,
            },
            Expr::Call { callee, args } => {
                let callee = self.lower_expr(callee)?;
                let args: Vec<Expr> = args.iter().map(|a| self.lower_expr(a)).collect::<Result<_, _>>()?;
                let has_named = args.iter().any(|a| matches!(a, Expr::Named { .. }));
                let callee_name = if let Expr::Ident(n) = &callee { Some(n.clone()) } else { None };
                // Module-prefixed ui elements (ui.input[...]) are ui elements too
                let ui_name = match &callee {
                    Expr::Ident(n) => Some(n.clone()),
                    Expr::FieldAccess { object, field } => {
                        if matches!(object.as_ref(), Expr::Ident(m) if m == "ui") {
                            Some(field.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                // Qualified user-module call (scene.mesh[...]): the import
                // expander registered the prefixed fn/struct — resolve named
                // args against it and canonicalize the callee to the prefixed
                // name so both modes see an ordinary call
                if let Expr::FieldAccess { object, field } = &callee {
                    if let Expr::Ident(m) = object.as_ref() {
                        let prefixed = format!("{}_{}", m, field);
                        if let Some(sig) = self.fns.get(&prefixed).cloned() {
                            let resolved =
                                self.resolve_args(&prefixed, &sig.params, &sig.defaults, args)?;
                            return Ok(Expr::Call {
                                callee: Box::new(Expr::Ident(prefixed)),
                                args: resolved,
                            });
                        }
                        if let Some(fields) = self.structs.get(&prefixed).cloned() {
                            let no_defaults = vec![None; fields.len()];
                            let resolved =
                                self.resolve_args(&prefixed, &fields, &no_defaults, args)?;
                            return Ok(Expr::Call {
                                callee: Box::new(Expr::Ident(prefixed)),
                                args: resolved,
                            });
                        }
                    }
                }

                match callee_name.as_deref() {
                    Some("map") => self.lower_map_call(args)?,
                    // slider[label, var, min = 0, max = 100, step = 1]
                    // field[label, var, type = '', lines = 0] — '' means "from
                    // the variable's type"; lines > 0 is a textarea
                    Some("field") if !self.fns.contains_key("field") => {
                        let params: Vec<String> = ["label", "var", "type", "lines"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, None, Some(Expr::Str(String::new())), Some(Expr::Int(0))];
                        let resolved = self.resolve_args("field", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // progress[value, max = 1.0]
                    Some("progress") if !self.fns.contains_key("progress") => {
                        let params: Vec<String> = ["value", "max"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, Some(Expr::Float(1.0))];
                        let resolved = self.resolve_args("progress", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // icon[name, size = 'm', color = '']
                    Some("icon") if !self.fns.contains_key("icon") => {
                        let params: Vec<String> = ["name", "size", "color"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, Some(Expr::Str("m".to_string())), Some(Expr::Str(String::new()))];
                        let resolved = self.resolve_args("icon", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    Some("slider") if !self.fns.contains_key("slider") => {
                        let params: Vec<String> = ["label", "var", "min", "max", "step"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, None, Some(Expr::Int(0)), Some(Expr::Int(100)), Some(Expr::Int(1))];
                        let resolved = self.resolve_args("slider", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // fmt has a full named signature with sentinel defaults
                    // (decimals < 0 / width 0 / align '' mean "unset"): both
                    // modes receive the 7 of a number's or a text's options,
                    // or the 3 of a moment's time style
                    Some("fmt") if !self.fns.contains_key("fmt") => {
                        let params: Vec<String> = ["value", "decimals", "commas", "width", "align", "fill", "percent", "time", "tz"]
                            .iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![
                            None,
                            Some(Expr::Int(-1)),
                            Some(Expr::Bool(false)),
                            Some(Expr::Int(0)),
                            Some(Expr::Str(String::new())),
                            Some(Expr::Str(" ".to_string())),
                            Some(Expr::Bool(false)),
                            Some(Expr::Str(String::new())),
                            Some(Expr::Str("UTC".to_string())),
                        ];
                        let resolved = self.resolve_args("fmt", &params, &defaults, args)?;
                        // a moment's time style is a row of its own, which
                        // can fail (kind 'time'); a number's and a text's
                        // options are the other's (crate::table)
                        // (an internal name: lowered again, as a derivation's
                        // body is, the call is left as it is)
                        let timed = !matches!(resolved.get(7), Some(Expr::Str(s)) if s.is_empty());
                        if timed {
                            Expr::Call { callee: Box::new(Expr::Ident(crate::lower::FMT_TIME.to_string())), args: vec![resolved[0].clone(), resolved[7].clone(), resolved[8].clone()] }
                        } else {
                            Expr::Call { callee: Box::new(callee), args: resolved[..7].to_vec() }
                        }
                    }
                    // moment[year, month, day, hour = 0, minute = 0,
                    // second = 0, tz = 'UTC'] — the tz-aware constructor.
                    // A moment is a number (epoch seconds).
                    Some("moment") if !self.fns.contains_key("moment") => {
                        let params: Vec<String> =
                            ["year", "month", "day", "hour", "minute", "second", "tz"]
                                .iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![
                            None, None, None,
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                            Some(Expr::Str("UTC".to_string())),
                        ];
                        let resolved = self.resolve_args("moment", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // parts[t, tz = 'UTC'] — calendar components + weekday.
                    Some("calendar") if !self.fns.contains_key("calendar") => {
                        let params: Vec<String> =
                            ["t", "tz"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> =
                            vec![None, Some(Expr::Str("UTC".to_string()))];
                        let resolved = self.resolve_args("calendar", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // parse_time[text, tz = 'UTC'] — string → moment.
                    Some("parse_time") if !self.fns.contains_key("parse_time") => {
                        let params: Vec<String> =
                            ["text", "tz"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> =
                            vec![None, Some(Expr::Str("UTC".to_string()))];
                        let resolved = self.resolve_args("parse_time", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // chart[frame, x = 'ts', candles = [], lines = [],
                    // bars_y = '', marker = 0, tz = 'UTC', height = 320,
                    // panes = [], levels = 0, points = 0, seek = 0] —
                    // a chart is a view over a frame (chart-module-design).
                    // panes: lists of column names, each drawn in its own
                    // strip; levels: map[col = [values]] of reference lines
                    // for the pane that draws col; points: a frame of markers
                    // (ts, y, label, kind); seek: a handler that receives the
                    // clicked bucket's moment.
                    Some("chart") if !self.fns.contains_key("chart") => {
                        let params: Vec<String> = [
                            "frame", "x", "candles", "lines", "bars_y",
                            "marker", "tz", "height", "panes", "levels",
                            "points", "seek",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                        let defaults: Vec<Option<Expr>> = vec![
                            None,
                            Some(Expr::Str("ts".to_string())),
                            Some(Expr::Array(vec![])),
                            Some(Expr::Array(vec![])),
                            Some(Expr::Str(String::new())),
                            Some(Expr::Int(0)),
                            Some(Expr::Str("UTC".to_string())),
                            Some(Expr::Int(320)),
                            Some(Expr::Array(vec![])),
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                            Some(Expr::Int(0)),
                        ];
                        let resolved = self.resolve_args("chart", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // serve[port, refresh = secs, budget = ms] — 0 means unset.
                    // refresh: the runtime re-renders every session on this
                    // interval; budget: handlers/renders over this many ms are
                    // logged. Both modes receive exactly 3 positional args.
                    // Plus the look: theme ('' = system), density ('' =
                    // comfortable), accent ('' = the theme's), wide (false).
                    Some("serve") if !self.fns.contains_key("serve") => {
                        let params: Vec<String> =
                            ["port", "refresh", "budget", "theme", "density", "accent", "wide", "keep", "uploads"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> =
                            vec![None, Some(Expr::Int(0)), Some(Expr::Int(0)), Some(Expr::Str(String::new())), Some(Expr::Str(String::new())), Some(Expr::Str(String::new())), Some(Expr::Bool(false)), Some(Expr::Bool(false)), Some(Expr::Int(10))];
                        let resolved = self.resolve_args("serve", &params, &defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    // page[path, view, url = ['day', 'sym']]: the facts the page's
                    // address carries, lowered as a third argument that is a list
                    // (a name there is page's older handler form)
                    Some("page") if !self.fns.contains_key("page") && has_named => {
                        let params: Vec<String> = ["path", "view", "url"].iter().map(|s| s.to_string()).collect();
                        let defaults: Vec<Option<Expr>> = vec![None, None, Some(Expr::Array(vec![]))];
                        let resolved = self.resolve_args("page", &params, &defaults, args)?;
                        if !matches!(resolved.get(2), Some(Expr::Array(_))) {
                            return Err(self.err("page[…, url = …] takes a list of fact names: url = ['day', 'sym']".to_string()));
                        }
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    Some(name) if self.fns.contains_key(name) => {
                        let sig = self.fns.get(name).unwrap().clone();
                        let resolved = self.resolve_args(name, &sig.params, &sig.defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    Some(name) if self.structs.contains_key(name) => {
                        let fields = self.structs.get(name).unwrap().clone();
                        let no_defaults = vec![None; fields.len()];
                        let resolved = self.resolve_args(name, &fields, &no_defaults, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    _ if self.in_view
                        && ui_name.as_deref().map_or(false, |n| UI_ELEMENTS.contains(&n))
                        && has_named =>
                    {
                        self.lower_ui_call(callee, args)
                    }
                    Some(name) if builtin_named_params(name).is_some() && has_named => {
                        let named_params = builtin_named_params(name).unwrap();
                        let resolved = self.resolve_builtin_args(name, named_params, args)?;
                        Expr::Call { callee: Box::new(callee), args: resolved }
                    }
                    _ => {
                        if has_named {
                            let target = callee_name.unwrap_or_else(|| "<expression>".to_string());
                            return Err(self.err(format!(
                                "'{}' does not accept named arguments (named arguments require a known function, struct, builtin with named parameters, or ui element)",
                                target
                            )));
                        }
                        Expr::Call { callee: Box::new(callee), args }
                    }
                }
            }
            Expr::Named { name, value } => Expr::Named {
                name: name.clone(),
                value: Box::new(self.lower_expr(value)?),
            },
            Expr::BinOp { left, op, right } => Expr::BinOp {
                left: Box::new(self.lower_expr(left)?),
                op: op.clone(),
                right: Box::new(self.lower_expr(right)?),
            },
            Expr::UnaryOp { op, expr: e } => Expr::UnaryOp {
                op: op.clone(),
                expr: Box::new(self.lower_expr(e)?),
            },
            // the session's notice reads as a record the runtime provides:
            // `notice.text` and `notice.at` (core-design.md §1.4)
            Expr::FieldAccess { object, field }
                if matches!(object.as_ref(), Expr::Ident(n) if n == "notice") && (field == "text" || field == "at") =>
            {
                Expr::Call { callee: Box::new(Expr::Ident(format!("__notice_{}", field))), args: vec![] }
            }
            Expr::FieldAccess { object, field } => Expr::FieldAccess {
                object: Box::new(self.lower_expr(object)?),
                field: field.clone(),
            },
            // the page's signed-in reader reads as a record the runtime
            // provides; nothing in the program declares or assigns it
            Expr::Ident(n) if self.web && n == "reader" => Expr::Call { callee: Box::new(Expr::Ident("__reader".to_string())), args: vec![] },
            Expr::Array(items) => Expr::Array(
                items.iter().map(|i| self.lower_expr(i)).collect::<Result<_, _>>()?,
            ),
            Expr::Map(pairs) => Expr::Map(
                pairs
                    .iter()
                    .map(|(k, v)| Ok((self.lower_expr(k)?, self.lower_expr(v)?)))
                    .collect::<Result<Vec<_>, String>>()?,
            ),
            other => other.clone(),
        })
    }

    /// Place named args into parameter slots and fill omitted defaults.
    fn resolve_args(
        &self,
        fn_name: &str,
        params: &[String],
        defaults: &[Option<Expr>],
        args: Vec<Expr>,
    ) -> Result<Vec<Expr>, String> {
        let positional_count = args.iter().take_while(|a| !matches!(a, Expr::Named { .. })).count();
        if positional_count > params.len() {
            // Too many positionals — leave as-is; the validator reports arity
            // errors with its usual message.
            return Ok(args);
        }
        let mut slots: Vec<Option<Expr>> = vec![None; params.len()];
        for (i, a) in args.iter().take(positional_count).enumerate() {
            slots[i] = Some(a.clone());
        }
        for a in args.into_iter().skip(positional_count) {
            let (name, value) = match a {
                Expr::Named { name, value } => (name, *value),
                _ => unreachable!("parser rejects positional after named"),
            };
            let idx = params.iter().position(|p| p == &name).ok_or_else(|| {
                self.err(format!(
                    "'{}' has no parameter named '{}' (parameters: {})",
                    fn_name, name, params.join(", ")
                ))
            })?;
            if slots[idx].is_some() {
                return Err(self.err(format!(
                    "argument '{}' given more than once in call to '{}'",
                    name, fn_name
                )));
            }
            slots[idx] = Some(value);
        }
        // Fill remaining slots with defaults
        let mut resolved = Vec::with_capacity(params.len());
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(e) => resolved.push(e),
                None => match &defaults[i] {
                    Some(d) => resolved.push(d.clone()),
                    None => {
                        return Err(self.err(format!(
                            "missing argument '{}' in call to '{}'",
                            params[i], fn_name
                        )))
                    }
                },
            }
        }
        Ok(resolved)
    }

    /// Builtins: reorder named args after the positionals in declared order.
    /// No default filling — arity is meaningful to the builtin itself.
    fn resolve_builtin_args(
        &self,
        fn_name: &str,
        named_params: &[&str],
        args: Vec<Expr>,
    ) -> Result<Vec<Expr>, String> {
        let mut positional = Vec::new();
        let mut named: Vec<(String, Expr)> = Vec::new();
        for a in args {
            match a {
                Expr::Named { name, value } => {
                    if !named_params.contains(&name.as_str()) {
                        return Err(self.err(format!(
                            "'{}' has no named parameter '{}' (named parameters: {})",
                            fn_name, name, named_params.join(", ")
                        )));
                    }
                    if named.iter().any(|(n, _)| n == &name) {
                        return Err(self.err(format!(
                            "argument '{}' given more than once in call to '{}'",
                            name, fn_name
                        )));
                    }
                    named.push((name, *value));
                }
                other => positional.push(other),
            }
        }
        for p in named_params {
            if let Some(pos) = named.iter().position(|(n, _)| n == p) {
                positional.push(named.remove(pos).1);
            }
        }
        Ok(positional)
    }

    /// UI elements: named args become entries of the attribute map (first
    /// argument by convention). `button['+', click = inc]` →
    /// `button[[click = inc], '+']`. Merges into an explicit attribute map if
    /// one was passed.
    fn lower_ui_call(&self, callee: Expr, args: Vec<Expr>) -> Expr {
        let mut attrs: Vec<(Expr, Expr)> = Vec::new();
        let mut children = Vec::new();
        for a in args {
            match a {
                Expr::Named { name, value } => attrs.push((Expr::Str(name), *value)),
                Expr::Map(pairs) => attrs.extend(pairs),
                other => children.push(other),
            }
        }
        let mut new_args = Vec::with_capacity(children.len() + 1);
        if !attrs.is_empty() {
            new_args.push(Expr::Map(attrs));
        }
        new_args.extend(children);
        Expr::Call { callee: Box::new(callee), args: new_args }
    }

    /// map[...] — the variadic named constructor. Desugars to a set[] chain.
    fn lower_map_call(&self, args: Vec<Expr>) -> Result<Expr, String> {
        let empty = Expr::Call {
            callee: Box::new(Expr::Ident("map".to_string())),
            args: vec![],
        };
        let mut result = empty;
        for a in args {
            match a {
                Expr::Named { name, value } => {
                    result = Expr::Call {
                        callee: Box::new(Expr::Ident("set".to_string())),
                        args: vec![result, Expr::Str(name), *value],
                    };
                }
                _ => {
                    return Err(self.err(
                        "map[] takes named arguments only: map[key = value, ...]".to_string(),
                    ))
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn lower_src(src: &str) -> Result<Vec<Stmt>, String> {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex failed");
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().expect("parse failed");
        lower_program(&stmts)
    }

    fn has_named(stmts: &[Stmt]) -> bool {
        fn expr_has(e: &Expr) -> bool {
            match e {
                Expr::Named { .. } => true,
                Expr::Call { callee, args } => expr_has(callee) || args.iter().any(expr_has),
                Expr::BinOp { left, right, .. } => expr_has(left) || expr_has(right),
                Expr::UnaryOp { expr, .. } => expr_has(expr),
                Expr::FieldAccess { object, .. } => expr_has(object),
                Expr::Array(items) => items.iter().any(expr_has),
                Expr::Map(pairs) => pairs.iter().any(|(k, v)| expr_has(k) || expr_has(v)),
                _ => false,
            }
        }
        fn stmt_has(s: &Stmt) -> bool {
            match s {
                Stmt::Assign { value, .. } => expr_has(value),
                Stmt::ExprStmt(e) => expr_has(e),
                Stmt::Return(Some(e)) => expr_has(e),
                Stmt::FnDecl { body, .. } => body.iter().any(stmt_has),
                Stmt::If { condition, then_body, else_body } => {
                    expr_has(condition)
                        || then_body.iter().any(stmt_has)
                        || else_body.as_ref().map_or(false, |b| b.iter().any(stmt_has))
                }
                Stmt::For { iter, body, .. } => expr_has(iter) || body.iter().any(stmt_has),
                Stmt::While { condition, body } => expr_has(condition) || body.iter().any(stmt_has),
                _ => false,
            }
        }
        stmts.iter().any(stmt_has)
    }

    #[test]
    fn test_named_args_resolved_and_defaults_filled() {
        let stmts = lower_src("fn train[data, lr = 0.5, epochs = 10]\n    data\nr = train[1, epochs = 3]\n").unwrap();
        assert!(!has_named(&stmts));
        // r = train[1, 0.5, 3]
        let assign = stmts.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "r" => Some(value.clone()),
            _ => None,
        }).unwrap();
        match assign {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 3);
                assert!(matches!(args[0], Expr::Int(1)));
                assert!(matches!(args[1], Expr::Float(f) if f == 0.5));
                assert!(matches!(args[2], Expr::Int(3)));
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_named_arg_errors() {
        let err = lower_src("fn f[a, b = 1]\n    a\nr = f[1, zzz = 2]\n").unwrap_err();
        assert!(err.contains("no parameter named 'zzz'"), "{}", err);
    }

    #[test]
    fn test_duplicate_named_arg_errors() {
        let err = lower_src("fn f[a, b = 1]\n    a\nr = f[1, 2, b = 3]\n").unwrap_err();
        assert!(err.contains("more than once"), "{}", err);
    }

    #[test]
    fn test_map_constructor_desugars_to_set_chain() {
        let stmts = lower_src("m = map[a = 1, b = 2]\n").unwrap();
        assert!(!has_named(&stmts));
        let value = stmts.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "m" => Some(value.clone()),
            _ => None,
        }).unwrap();
        // set[set[map[], 'a', 1], 'b', 2]
        match &value {
            Expr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), Expr::Ident(n) if n == "set"));
                assert_eq!(args.len(), 3);
                assert!(matches!(&args[1], Expr::Str(s) if s == "b"));
            }
            other => panic!("expected set chain, got {:?}", other),
        }
    }

    #[test]
    fn test_builtin_axis_reordered() {
        let stmts = lower_src("s = sum[x, axis = 0]\n").unwrap();
        assert!(!has_named(&stmts));
        let value = stmts.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "s" => Some(value.clone()),
            _ => None,
        }).unwrap();
        match value {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(matches!(args[1], Expr::Int(0)));
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_ui_named_attrs_become_attr_map() {
        let stmts = lower_src("fn inc[]\n    1\nview v[]\n    b = button['+', click = inc]\n    return b\n").unwrap();
        assert!(!has_named(&stmts));
        // the assignment now lives inside the view body
        let view_body = stmts.iter().find_map(|s| match s {
            Stmt::FnDecl { name, body, is_view: true, .. } if name == "v" => Some(body.clone()),
            _ => None,
        }).unwrap();
        let value = view_body.iter().find_map(|s| match s {
            Stmt::Assign { name, value } if name == "b" => Some(value.clone()),
            _ => None,
        }).unwrap();
        match value {
            Expr::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(matches!(&args[0], Expr::Map(pairs) if pairs.len() == 1));
                assert!(matches!(&args[1], Expr::Str(s) if s == "+"));
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_indirect_call_named_arg_errors() {
        let err = lower_src("r = unknown_fn[1, a = 2]\n").unwrap_err();
        assert!(err.contains("does not accept named arguments"), "{}", err);
    }

    /// The narrowed reads of a lowered body, as the names they read.
    fn known_reads(stmts: &[Stmt]) -> Vec<String> {
        fn in_expr(e: &Expr, out: &mut Vec<String>) {
            match e {
                Expr::Call { callee, args } => {
                    if matches!(callee.as_ref(), Expr::Ident(k) if k == KNOWN) {
                        if let Some(Expr::Ident(n)) = args.first() {
                            out.push(n.clone());
                        }
                    }
                    args.iter().for_each(|a| in_expr(a, out));
                }
                Expr::BinOp { left, right, .. } => {
                    in_expr(left, out);
                    in_expr(right, out);
                }
                Expr::UnaryOp { expr, .. } => in_expr(expr, out),
                Expr::FieldAccess { object, .. } => in_expr(object, out),
                _ => {}
            }
        }
        fn in_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
            for s in stmts {
                match s {
                    Stmt::Assign { value, .. } | Stmt::ExprStmt(value) | Stmt::Return(Some(value)) => in_expr(value, out),
                    Stmt::If { condition, then_body, else_body } => {
                        in_expr(condition, out);
                        in_stmts(then_body, out);
                        if let Some(b) = else_body {
                            in_stmts(b, out);
                        }
                    }
                    Stmt::While { condition, body } => {
                        in_expr(condition, out);
                        in_stmts(body, out);
                    }
                    Stmt::For { iter, body, .. } => {
                        in_expr(iter, out);
                        in_stmts(body, out);
                    }
                    Stmt::FnDecl { body, .. } => in_stmts(body, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        in_stmts(stmts, &mut out);
        out
    }

    /// A guard narrows (spec, Errors in Conditions): reads after it are
    /// the value; the guard's own check, a read before it, and every read
    /// after an assignment are not
    #[test]
    fn a_guard_narrows_the_rest_of_its_block() {
        let stmts = lower_src("fn f[s]\n    x = int[s]\n    a = x\n    if not x.ok\n        return 0\n    b = x + 1\n    x = int['q']\n    c = x\n    return b\n").unwrap();
        assert_eq!(known_reads(&stmts), vec!["x".to_string()], "only `b = x + 1` reads a narrowed x");
    }

    /// `or` of checks narrows each name; a branch that does not leave
    /// narrows nothing, nor does an `else`
    #[test]
    fn only_a_guard_that_leaves_narrows() {
        let both = lower_src("fn f[a, b]\n    if not a.ok or not b.ok\n        return 0\n    return a + b\n").unwrap();
        assert_eq!(known_reads(&both), vec!["a".to_string(), "b".to_string()]);
        let stays = lower_src("fn f[a]\n    if not a.ok\n        log['bad']\n    return a\n").unwrap();
        assert!(known_reads(&stays).is_empty());
        let with_else = lower_src("fn f[a]\n    if not a.ok\n        return 0\n    else\n        log['ok']\n    return a\n").unwrap();
        assert!(known_reads(&with_else).is_empty());
    }

    /// Variables belong to their block (spec §1, Scope): a name set again
    /// after its block ended is a variable of its own, renamed apart for
    /// the passes after, inside interpolation too; a name still in scope is
    /// the same variable, and top-level names and parameters are never
    /// renamed
    #[test]
    fn a_block_ends_its_variables() {
        let stmts = lower_src("fn f[c]\n    if c\n        x = 1\n    else\n        x = 'one'\n        log['`x`!']\n    n = 0\n    while n lt 2\n        n = n + 1\n    return c\n").unwrap();
        let body = stmts.iter().find_map(|s| match s {
            Stmt::FnDecl { body, .. } => Some(body.clone()),
            _ => None,
        }).expect("the function");
        let text = format!("{:?}", body);
        assert!(text.contains("name: \"x\"") && text.contains("name: \"x__s2\""), "{}", text);
        assert!(text.contains("`x__s2`!"), "the span reads the renamed variable: {}", text);
        assert!(!text.contains("n__s") && !text.contains("c__s"), "an update and a parameter keep their names: {}", text);
    }

    /// A read after the block that set the variable ended is refused, with
    /// the line and what to write instead
    #[test]
    fn a_read_after_its_block_is_refused() {
        let err = lower_src("x = 10\nif true\n    y = 20\nlog[y]\n").unwrap_err();
        assert!(err.contains("line 4") && err.contains("'y' is not in scope here") && err.contains("y = …"), "{}", err);
    }

    /// A message names a variable as the program wrote it
    #[test]
    fn a_renamed_variable_is_shown_as_written() {
        assert_eq!(shown("cannot reassign 'x__s2' from int to str"), "cannot reassign 'x' from int to str");
        assert_eq!(shown("keep __s2 and a__sb"), "keep __s2 and a__sb");
    }

    /// A loop whose body assigns a name does not inherit its narrowing,
    /// even for reads before the assignment (an earlier pass made it fail);
    /// the end of a block ends a narrowing made in it
    #[test]
    fn a_narrowing_stays_in_its_block() {
        let looped = lower_src("fn f[s]\n    x = int[s]\n    if not x.ok\n        return 0\n    while x gt 0\n        y = x\n        x = int['q']\n    return 1\n").unwrap();
        assert!(known_reads(&looped).is_empty(), "{:?}", known_reads(&looped));
        let nested = lower_src("fn f[s, c]\n    x = int[s]\n    if c\n        if not x.ok\n            return 0\n        y = x\n    return x\n").unwrap();
        assert_eq!(known_reads(&nested), vec!["x".to_string()], "only the read inside the inner block");
    }

    /// Lowering a lowered program changes nothing: a derivation's body is
    /// lowered again, and `fmt`'s time style, lowered to three arguments,
    /// was read the second time as a number's decimals and commas.
    #[test]
    fn lowering_twice_is_lowering_once() {
        let src = "t = now[]\na = fmt[t, time = 'time', tz = 'America/New_York']\nb = fmt[2.5, decimals = 1]\n";
        let once = lower_src(src).unwrap();
        let twice = lower_program(&once).unwrap();
        assert_eq!(format!("{:?}", once), format!("{:?}", twice));
    }
}
