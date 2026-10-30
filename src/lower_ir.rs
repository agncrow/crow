//! Lowering: the parser's tree to the IR (docs/ir-design.md §4).
//!
//! Two front-end passes have already run: named arguments are positional
//! (`lower.rs`) and the program's signatures are inferred
//! (`type_infer.rs`). This pass types each node from its children, refuses
//! what is not a program (`checks.rs`), and writes the decisions into the
//! IR, adding the ones the IR makes explicit: which calls are effects and
//! whether their answer is used, where a result is propagated, where an
//! int becomes a float, which read is a variable's last, which loops are
//! parallel, which functions are handlers and what each function can do.

use crate::ast::{BinOp, Expr as A, Stmt as S};
use crate::ir::*;
use crate::type_infer::{Ty, Dims, TypeContext};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod fuse;
pub(crate) mod grad;
mod checks;
mod named;
pub(crate) use checks::moment_field_name;
mod frames;
pub(crate) mod schema;
mod shapes;
pub(crate) mod uses;
#[allow(unused_imports)] // (the reserved lists are the self-check's and the docs check's)
pub(crate) use checks::{is_standard_module, names_of, slips, BUILTINS, CORE_RESERVED, UI_RESERVED, WEB_RESERVED};

/// Lower a whole (expanded, lowered, inferred) program.
pub fn lower(stmts: &[S], ctx: &mut TypeContext, part_name: &str) -> Result<Program, String> {
    crate::modules::ModuleRegistry::reset_scope();
    lower_checked(stmts, ctx, part_name, &HashSet::new()).map_err(|r| r.message())
}

/// A fragment of a session — the REPL's, a notebook's: a name an earlier
/// fragment bound is known (a read of one is a local, an assignment a
/// `set`), and the fragment's own top-level names outlive it, so none is
/// moved at its last read here, since a later fragment may read it. (A
/// notebook cell that defined a list and read it lost the list: the next
/// cell read `done`.) The functions an earlier fragment defined (`earlier`)
/// are the fragment's to call.
pub fn lower_fragment(stmts: &[S], ctx: &mut TypeContext, part_name: &str, known: &HashSet<String>, earlier: &[&Fn]) -> Result<Program, String> {
    lower_checked_as(stmts, ctx, part_name, known, earlier, true).map_err(|r| r.message())
}

/// Why a program was not lowered.
pub enum Refusal {
    /// What the program got wrong, each error with the top-level
    /// declaration it was found in: after module expansion a part's
    /// declarations carry its prefix, so a caller can say which file an
    /// error belongs to.
    Invalid(Vec<(String, Option<String>)>),
    /// A form the lowering has no IR for.
    Unlowered(String),
}

impl Refusal {
    /// The refusal as printed: every error, one per line, each variable
    /// named as the program wrote it.
    pub fn message(&self) -> String {
        match self {
            Refusal::Invalid(errors) => crate::lower::shown(&errors.iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>().join("\n")),
            Refusal::Unlowered(e) => e.clone(),
        }
    }
}

/// A program lowered, refused with each error and the declaration it was
/// found in when it is not one: the lowering a pipeline that runs or builds
/// the program shares with its check (F5 S10).
pub fn lower_refusing(stmts: &[S], ctx: &mut TypeContext, part_name: &str) -> Result<Program, Refusal> {
    lower_checked(stmts, ctx, part_name, &HashSet::new())
}

/// Whether a program is one: lowered in a copy of its context and thrown
/// away, for the callers that only ask (`crow check`, the harness, the
/// language server).
pub fn check(stmts: &[S], ctx: &TypeContext) -> Result<(), Refusal> {
    let mut ctx = ctx.clone();
    lower_checked(stmts, &mut ctx, "check", &HashSet::new()).map(|_| ())
}

fn lower_checked(stmts: &[S], ctx: &mut TypeContext, part_name: &str, known: &HashSet<String>) -> Result<Program, Refusal> {
    lower_checked_as(stmts, ctx, part_name, known, &[], false)
}

fn lower_checked_as(stmts: &[S], ctx: &mut TypeContext, part_name: &str, known: &HashSet<String>, earlier: &[&Fn], fragment: bool) -> Result<Program, Refusal> {
    // (a console cell is in a ui program when an earlier cell imported ui:
    // the session's imports are its context's — a view typed after `use ui`
    // in a cell of its own had been refused, `add use ui`)
    let in_ui = stmts.iter().any(|s| matches!(s, S::Use { path, .. } if path == "ui" || path == "web.ui")) || (fragment && (ctx.bare_modules.contains("ui") || ctx.bare_modules.contains("web.ui")));
    let mut l = Lowerer::new(ctx, in_ui);
    l.fragment = fragment;
    if fragment {
        // a console cell's last line is shown
        yields_of(stmts, &mut l.yields);
    }
    l.scopes[0].extend(known.iter().cloned());
    l.prescan(stmts, known);
    // a column read two ways is the one refusal then: the reads of it
    // would each be refused too, as reads of a parsed value, and say less
    if !l.ctx.shape_conflicts.is_empty() {
        for (line, conflict) in l.ctx.shape_conflicts.clone() {
            l.v.line = line;
            l.refuse(conflict);
        }
        return Err(Refusal::Invalid(std::mem::take(&mut l.v.errors)));
    }
    let part = l.lower_part(stmts, part_name);
    // what the program got wrong comes first: a form the lowering cannot
    // take is usually one of those errors seen from the other side
    if !l.v.errors.is_empty() {
        l.reattribute();
        return Err(Refusal::Invalid(checks::without_echoes(std::mem::take(&mut l.v.errors))));
    }
    let mut program = Program { parts: vec![part.map_err(Refusal::Unlowered)?] };
    decide_loops(&mut program);
    decide_answers(&mut program);
    written_names(&mut program);
    // the functions grad differentiates over tensors: what they may do,
    // their loops in order, and the set the backends trace
    let refusals = grad::settle(&mut program);
    if !refusals.is_empty() {
        return Err(Refusal::Invalid(refusals.into_iter().map(|(f, m)| (m, Some(f))).collect()));
    }
    // chains computed in one pass: element-wise tensors, a frame's mask
    fuse::chains(&mut program);
    // each connection opens with the tables the program declares on it
    schema::attach(&mut program);
    // an int the program may grow past 64 bits held as a growing one
    // (spec, Numbers That Keep Every Digit)
    crate::int_width::widen(&mut program);
    // the parts agree before a backend reads them (ir_verify): a lowering
    // that disagrees with itself is refused as the bug in Crow it is, in a
    // debug build and in every harness; a release build checks when asked
    match crate::ir_verify::mode() {
        crate::ir_verify::Mode::Off => {}
        crate::ir_verify::Mode::Report => {
            for f in crate::ir_verify::verify_in(&program, &known_types(ctx, known), earlier) {
                eprintln!("ir-verify: {}", f);
            }
        }
        crate::ir_verify::Mode::Strict => {
            let found = crate::ir_verify::verify_in(&program, &known_types(ctx, known), earlier);
            if let Some(first) = found.first() {
                let more = if found.len() > 1 { format!(" (and {} more)", found.len() - 1) } else { String::new() };
                return Err(Refusal::Unlowered(format!(
                    "internal error: the lowered program disagrees with itself — a bug in Crow, not in this program: {}{}",
                    first, more
                )));
            }
        }
    }
    Ok(program)
}

// ── effects: answered, or held to the commit ────────────────────────

/// Decides the effects functions return, now that every caller is known.
/// A function's last line is its value (the parser writes it as a
/// return), but a handler's value goes to nobody: the page, `later` and
/// the harness name a handler only to run it. Nor does the value of a
/// function every caller calls as a statement. An effect such a function
/// returns is one whose answer nobody uses, so it waits for the commit as
/// a statement's does (spec, "A handler is a transaction").
/// Each block's variable as the program wrote it (S11). The passes before
/// the IR keep a variable's type by its name, so the front end names a
/// binding apart when its name's earlier block has ended (`x__s2`,
/// lower::scoped); only one binding of a name is ever in view, so the IR
/// the backends read, and prints, says `x` again — Rust's blocks keep the
/// two apart as the program's do.
fn written_names(program: &mut Program) {
    fn name(v: &mut String) {
        let w = crate::lower::written(v);
        if w.len() != v.len() {
            *v = w.to_string();
        }
    }
    fn block(b: &mut Block) {
        for s in b.iter_mut() {
            stmt(s);
        }
    }
    fn stmt(s: &mut Stmt) {
        match s {
            Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } => {
                name(var);
                expr_(expr);
            }
            Stmt::SetPath { var, path, expr, .. } => {
                name(var);
                for p in path.iter_mut() {
                    if let PathStep::Index(i) = p {
                        expr_(i);
                    }
                }
                expr_(expr);
            }
            Stmt::Store { expr, .. } | Stmt::Do(expr) | Stmt::Return(Some(expr)) => expr_(expr),
            Stmt::If { cond, then, els } => {
                expr_(cond);
                block(then);
                if let Some(e) = els {
                    block(e);
                }
            }
            Stmt::While { cond, body } => {
                expr_(cond);
                block(body);
            }
            Stmt::For { var, iter, body, .. } => {
                name(var);
                expr_(iter);
                block(body);
            }
            Stmt::Propagate { values, .. } => values.iter_mut().for_each(name),
            _ => {}
        }
    }
    fn expr_(e: &mut Expr) {
        match e {
            Expr::Local(v, _) | Expr::Move(v, _) => name(v),
            Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => args.iter_mut().for_each(expr_),
            Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => expr_(expr),
            Expr::Binary { l, r, .. } => {
                expr_(l);
                expr_(r);
            }
            Expr::Index { base, indices, .. } => {
                expr_(base);
                indices.iter_mut().for_each(expr_);
            }
            Expr::Field { base, .. } | Expr::Meta { base, .. } => expr_(base),
            Expr::Interp(pieces) => {
                for p in pieces.iter_mut() {
                    if let Piece::Expr(x) = p {
                        expr_(x);
                    }
                }
            }
            Expr::ListLit(items, _) => items.iter_mut().for_each(expr_),
            Expr::MapLit(pairs, _) => pairs.iter_mut().for_each(|(_, v)| expr_(v)),
            Expr::RecordLit { fields, .. } => fields.iter_mut().for_each(expr_),
            Expr::Tree { attrs, children, .. } => {
                attrs.iter_mut().for_each(|(_, a)| expr_(a));
                children.iter_mut().for_each(expr_);
            }
            Expr::ForExpr { var, iter, body, .. } => {
                name(var);
                expr_(iter);
                block(body);
            }
            _ => {}
        }
    }
    for part in &mut program.parts {
        for f in &mut part.fns {
            block(&mut f.body);
        }
        for d in &mut part.derives {
            block(&mut d.body);
        }
        for t in &mut part.tests {
            block(&mut t.body);
        }
        block(&mut part.main);
    }
}

fn decide_answers(program: &mut Program) {
    for part in &mut program.parts {
        let used = values_used(part);
        for f in &mut part.fns {
            if !used.contains(&f.name) {
                hold_returned(&mut f.body);
            }
        }
    }
}

/// The functions whose value something reads: a call whose value is used,
/// a function handed on as a value anywhere but to run it, and a function
/// whose call a used function returns.
fn values_used(part: &Part) -> BTreeSet<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    // (caller, callee): the callee's value is the caller's
    let mut passes: Vec<(String, String)> = Vec::new();
    let mut refs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for f in &part.fns {
        used_in_block(&f.body, Some(&f.name), &mut used, &mut passes, &mut refs);
    }
    for d in &part.derives {
        used_in_block(&d.body, None, &mut used, &mut passes, &mut refs);
    }
    for t in &part.tests {
        used_in_block(&t.body, None, &mut used, &mut passes, &mut refs);
    }
    for fact in &part.facts {
        used_in_expr(&fact.default, &mut used, &mut refs);
    }
    used_in_block(&part.main, None, &mut used, &mut passes, &mut refs);
    // a function named somewhere other than to be run is a value a callee
    // may call and read
    used.extend(refs.into_iter().filter(|(_, (all, run))| all > run).map(|(n, _)| n));
    loop {
        let before = used.len();
        for (caller, callee) in &passes {
            if used.contains(caller) {
                used.insert(callee.clone());
            }
        }
        if used.len() == before {
            return used;
        }
    }
}

fn used_in_block(b: &Block, owner: Option<&str>, used: &mut BTreeSet<String>, passes: &mut Vec<(String, String)>, refs: &mut BTreeMap<String, (usize, usize)>) {
    for s in b {
        match s {
            // a call for what it does: its value goes nowhere
            Stmt::Do(Expr::Call { target: Target::User(_), args, .. }) => args.iter().for_each(|a| used_in_expr(a, used, refs)),
            // a call returned: its value is read when the function's is
            Stmt::Return(Some(Expr::Call { target: Target::User(n), args, .. })) if owner.is_some() => {
                passes.push((owner.unwrap_or_default().to_string(), n.clone()));
                args.iter().for_each(|a| used_in_expr(a, used, refs));
            }
            Stmt::If { cond, then, els } => {
                used_in_expr(cond, used, refs);
                used_in_block(then, owner, used, passes, refs);
                if let Some(e) = els {
                    used_in_block(e, owner, used, passes, refs);
                }
            }
            Stmt::While { cond: e, body } | Stmt::For { iter: e, body, .. } => {
                used_in_expr(e, used, refs);
                used_in_block(body, owner, used, passes, refs);
            }
            other => visit_stmt(other, &mut |e| note_used(e, used, refs)),
        }
    }
}

fn used_in_expr(e: &Expr, used: &mut BTreeSet<String>, refs: &mut BTreeMap<String, (usize, usize)>) {
    visit_expr(e, &mut |x| note_used(x, used, refs));
}

/// One node: a call's value is read where it is written (a collecting
/// loop's last line is what it yields), and each naming of a function is
/// counted, with the namings that only run it: a control's handler
/// (`click = save`, a widget's `on = map[pick = choose]`), `later[f]`,
/// `stream[feed]` and the harness's `click[a, save]`.
fn note_used(e: &Expr, used: &mut BTreeSet<String>, refs: &mut BTreeMap<String, (usize, usize)>) {
    let run = |x: &Expr, refs: &mut BTreeMap<String, (usize, usize)>| {
        if let Expr::FnRef(n, _) = x {
            refs.entry(n.clone()).or_default().1 += 1;
        }
    };
    match e {
        Expr::Call { target: Target::User(n), .. } => {
            used.insert(n.clone());
        }
        Expr::FnRef(n, _) => refs.entry(n.clone()).or_default().0 += 1,
        Expr::Tree { attrs, .. } => {
            for (_, a) in attrs {
                match a {
                    Expr::MapLit(pairs, _) => pairs.iter().for_each(|(_, v)| run(v, refs)),
                    other => run(other, refs),
                }
            }
        }
        Expr::Effect { args, .. } | Expr::Call { args, .. } if matches!(call_name(e).as_deref(), Some("later" | "click" | "stream")) => {
            args.iter().for_each(|a| run(a, refs));
        }
        _ => {}
    }
}

/// Every effect a function returns, held: its answer goes to nobody.
fn hold_returned(b: &mut Block) {
    for s in b {
        match s {
            Stmt::Return(Some(Expr::Effect { answered, .. })) => *answered = false,
            Stmt::If { then, els, .. } => {
                hold_returned(then);
                if let Some(e) = els {
                    hold_returned(e);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => hold_returned(body),
            _ => {}
        }
    }
}

// ── loops: in parallel or in order ──────────────────────────────────

/// Decides every loop of the program at once, now that each function's
/// kind and reach are known: a loop runs its iterations in parallel when
/// nothing its body does can show the order they ran in (spec §14, For
/// Loops). Both modes read the flag; neither decides for itself.
fn decide_loops(program: &mut Program) {
    for part in &mut program.parts {
        let order = LoopOrder::of(part);
        let decide = |var: &str, body: &Block| !order.keeps_order(var, body);
        for f in &mut part.fns {
            set_loops(&mut f.body, &decide);
        }
        for d in &mut part.derives {
            set_loops(&mut d.body, &decide);
        }
        for t in &mut part.tests {
            set_loops(&mut t.body, &decide);
        }
        for fact in &mut part.facts {
            set_loops_expr(&mut fact.default, &decide);
        }
        set_loops(&mut part.main, &decide);
    }
}

/// What keeps a loop in order, program-wide: the functions a call to
/// which does, and whether a call through a function value might.
struct LoopOrder {
    known: BTreeSet<String>,
    ordered: BTreeSet<String>,
    through_values: bool,
}

impl LoopOrder {
    fn of(part: &Part) -> LoopOrder {
        let known: BTreeSet<String> = part.fns.iter().map(|f| f.name.clone()).collect();
        let mut ordered: BTreeSet<String> = BTreeSet::new();
        // each function's callees (a function handed to a call counts: the
        // callee may call it) and whether it calls through a value
        let mut calls: BTreeMap<String, (BTreeSet<String>, bool)> = BTreeMap::new();
        for f in &part.fns {
            let mut here = false;
            let mut callees: BTreeSet<String> = BTreeSet::new();
            let mut through = false;
            visit_block(&f.body, &mut |e| {
                here |= shows_order(e);
                match e {
                    Expr::Call { target: Target::User(n), .. } => {
                        callees.insert(n.clone());
                    }
                    Expr::Index { base, .. } if matches!(base.ty(), Ty::Fn(..)) => through = true,
                    _ => {}
                }
                callees.extend(handed(e));
            });
            visit_stmts(&f.body, &mut |s| here |= matches!(s, Stmt::Store { .. }));
            if here || !f.effects.is_empty() || matches!(f.kind, FnKind::Handler | FnKind::Helper | FnKind::View) {
                ordered.insert(f.name.clone());
            }
            calls.insert(f.name.clone(), (callees, through));
        }
        // every function the program holds as a value: a call through one
        // may reach any of them — and the runtime's put, which stream[feed]
        // hands its feed, sends a chunk, so its order shows (a feed's loop
        // had run in parallel, its chunks in any order)
        let mut values: BTreeSet<String> = BTreeSet::new();
        let mut puts = false;
        let mut note = |e: &Expr| {
            if let Expr::FnRef(n, _) = e {
                values.insert(n.clone());
            }
            puts |= call_name(e).as_deref() == Some("stream");
        };
        for f in &part.fns {
            visit_block(&f.body, &mut note);
        }
        for d in &part.derives {
            visit_block(&d.body, &mut note);
        }
        for t in &part.tests {
            visit_block(&t.body, &mut note);
        }
        for fact in &part.facts {
            visit_expr(&fact.default, &mut note);
        }
        visit_block(&part.main, &mut note);
        loop {
            let through_values = puts || values.iter().any(|v| ordered.contains(v) || !known.contains(v));
            let mut grew = false;
            for (name, (callees, through)) in &calls {
                if ordered.contains(name) {
                    continue;
                }
                if callees.iter().any(|c| ordered.contains(c) || !known.contains(c)) || (*through && through_values) {
                    ordered.insert(name.clone());
                    grew = true;
                }
            }
            if !grew {
                return LoopOrder { known, ordered, through_values };
            }
        }
    }

    /// Whether a loop binding `var` over `body` must run in order: the body
    /// writes a variable from outside it or a fact, leaves the function (a
    /// return, a propagated error, a handler's stop: the first in input
    /// order decides), or reaches something whose order shows.
    fn keeps_order(&self, var: &str, body: &Block) -> bool {
        // the iteration's own names: the loop's variable, what the body
        // binds, and the variables of loops inside it
        let mut own: BTreeSet<String> = BTreeSet::new();
        own.insert(var.to_string());
        visit_stmts(body, &mut |s| match s {
            Stmt::Let { var, .. } | Stmt::For { var, .. } => {
                own.insert(var.clone());
            }
            _ => {}
        });
        visit_block(body, &mut |e| {
            if let Expr::ForExpr { var, .. } = e {
                own.insert(var.clone());
            }
        });
        let mut order = false;
        visit_stmts(body, &mut |s| {
            order |= match s {
                Stmt::Set { var, .. } | Stmt::SetPath { var, .. } => !own.contains(var),
                Stmt::Store { .. } | Stmt::Return(_) | Stmt::Propagate { .. } => true,
                _ => false,
            }
        });
        visit_block(body, &mut |e| {
            order |= shows_order(e)
                || match e {
                    Expr::Call { target: Target::User(n), .. } => self.ordered.contains(n) || !self.known.contains(n),
                    Expr::Index { base, .. } => matches!(base.ty(), Ty::Fn(..)) && self.through_values,
                    // a collecting loop over a list that can fail refuses it
                    // where it starts: it can leave the function
                    Expr::ForExpr { iter, .. } => iter.ty().is_result(),
                    _ => false,
                }
                || handed(e).iter().any(|n| self.ordered.contains(n) || !self.known.contains(n));
        });
        // a value that can fail, unwrapped where nothing around it holds the
        // failure, leaves the function with it — which a round run on
        // another thread cannot (the compiled program had returned the
        // function's result from the loop's closure, and did not build: fuzz
        // seed 1791422055, `bucket[unwrap moment[t]] eq …` into a bool)
        order || unwrap_leaves_block(body)
    }
}

/// Whether a statement of the block (nested ones too) unwraps a value that
/// can fail where no expression around it holds the failure: a call, an
/// operator or a text typed as a value that can fail answers it as its own
/// value (`occurs[unwrap seq, b]`, the round's item), and the round goes
/// on; anything else hands it out of the function.
fn unwrap_leaves_block(b: &[Stmt]) -> bool {
    let mut leaves = false;
    visit_stmts(b, &mut |s| {
        leaves |= match s {
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) | Stmt::Return(Some(expr)) => unwrap_leaves(expr, false),
            Stmt::SetPath { path, expr, .. } => unwrap_leaves(expr, false) || path.iter().any(|p| matches!(p, PathStep::Index(i) if unwrap_leaves(i, false))),
            Stmt::If { cond, .. } | Stmt::While { cond, .. } => unwrap_leaves(cond, false),
            Stmt::For { iter, .. } => unwrap_leaves(iter, false),
            _ => false,
        }
    });
    leaves
}

/// `unwrap_leaves_block` of one expression, `held` where an expression
/// around it holds a failure.
fn unwrap_leaves(e: &Expr, held: bool) -> bool {
    let inner = held || e.ty_ref().is_some_and(|t| t.is_result());
    match e {
        Expr::Convert { how: Convert::Unwrap, expr, .. } => (expr.ty().is_result() && !held) || unwrap_leaves(expr, held),
        // (a collecting loop's rounds: statements of their own)
        Expr::ForExpr { iter, body, .. } => unwrap_leaves(iter, held) || unwrap_leaves_block(body),
        Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => args.iter().any(|a| unwrap_leaves(a, inner)),
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } | Expr::Field { base: expr, .. } | Expr::Meta { base: expr, .. } => unwrap_leaves(expr, inner),
        Expr::Binary { l, r, .. } => unwrap_leaves(l, inner) || unwrap_leaves(r, inner),
        Expr::Index { base, indices, .. } => unwrap_leaves(base, inner) || indices.iter().any(|i| unwrap_leaves(i, inner)),
        Expr::Interp(pieces) => pieces.iter().any(|p| matches!(p, Piece::Expr(x) if unwrap_leaves(x, inner))),
        Expr::ListLit(items, _) => items.iter().any(|i| unwrap_leaves(i, inner)),
        Expr::MapLit(pairs, _) => pairs.iter().any(|(_, v)| unwrap_leaves(v, inner)),
        Expr::RecordLit { fields, .. } => fields.iter().any(|x| unwrap_leaves(x, inner)),
        Expr::Tree { attrs, children, .. } => attrs.iter().any(|(_, a)| unwrap_leaves(a, inner)) || children.iter().any(|c| unwrap_leaves(c, inner)),
        _ => false,
    }
}

/// `.ok`, `.reason`, `.kind` or `.trace` of a value its type says cannot
/// fail, decided here: true, empty text, no trace (typed-values-design.md,
/// decision 1) — nothing is looked at as the program runs. A value read
/// through something that can act or stop (a call, an index, arithmetic)
/// is left to the backends, which evaluate it and answer the same.
fn decided_meta(e: Expr) -> Expr {
    let Expr::Meta { base, which } = &e else { return e };
    if *which == Meta::Valid || !meta_decided(&base.ty()) || !plain_read(base) {
        return e;
    }
    match which {
        Meta::Ok => Expr::Const(Const::Bool(true), Ty::Bool),
        Meta::Trace => Expr::ListLit(Vec::new(), Ty::List(Box::new(Ty::Str))),
        _ => Expr::Const(Const::Str(String::new()), Ty::Str),
    }
}

/// Whether a value of this type cannot fail: not a result, not a parsed
/// value (JSON's null is an error value), and not unknown.
pub(crate) fn meta_decided(t: &Ty) -> bool {
    !t.is_result() && !matches!(t, Ty::Json | Ty::Unknown)
}

/// Whether reading an expression only reads: a constant, a local, a fact, a
/// derivation, a function, or a record's field of one.
/// The pieces' shapes where `e` is a decomposition's call (`svd[m]`), as
/// its row declares them.
fn pieces_of_call(e: &Expr) -> Option<Vec<Option<Dims>>> {
    let Expr::Call { target, args, .. } = e else { return None };
    let row = match target {
        Target::Row(row) => Some(*row),
        Target::Runtime(path) => crate::table::row(path),
        _ => None,
    };
    row.and_then(|r| shapes::declared_pieces(r, args))
}

/// What `grad` is handed, when an expression is a call of it.
fn grad_argument(e: &Expr) -> Option<&Expr> {
    match e {
        Expr::Call { target: Target::Runtime(n), args, .. } if n == "grad" || n.ends_with(".grad") => args.first(),
        _ => None,
    }
}

fn gradient_of_call(e: &Expr) -> Option<String> {
    // the function a gradient is of, when an expression is `grad[f]`
    match grad_argument(e) {
        Some(Expr::FnRef(f, _)) => Some(f.clone()),
        _ => None,
    }
}

fn plain_read(e: &Expr) -> bool {
    match e {
        Expr::Const(..) | Expr::Local(..) | Expr::Move(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) | Expr::FnRef(..) => true,
        Expr::Field { base, .. } => matches!(base.ty(), Ty::Record(_)) && plain_read(base),
        _ => false,
    }
}

/// A node whose order shows by itself: an effect of any kind (a draw from
/// the seeded generator among them), a builtin whose order shows (the
/// table's `ordered`: an assertion, a look at background work), or a paged
/// table (the render numbers them in order).
fn shows_order(e: &Expr) -> bool {
    match e {
        Expr::Effect { .. } => true,
        Expr::Call { target: Target::Runtime(_) | Target::Row(_) | Target::Module(..), .. } => {
            let n = call_name(e).unwrap_or_default();
            crate::table::ordered(&n)
        }
        Expr::Tree { element, attrs, .. } => element == "table" && attrs.iter().any(|(k, _)| k == "page" || k == "sort"),
        _ => false,
    }
}

/// The functions a call hands over as values (`any[xs, is_bad]`): the
/// callee may call them.
fn handed(e: &Expr) -> Vec<String> {
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } => args
            .iter()
            .filter_map(|a| match a {
                Expr::FnRef(n, _) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The items a row's list parameter takes: what an empty list handed to
/// it holds (`typed_empties`), and what a list whose items no type says
/// is read as there (`retype_unknown_lists`). A tensor's parameter takes
/// numbers.
fn listed_item(k: crate::table::Kind) -> Option<Ty> {
    use crate::table::Kind;
    Some(match k {
        Kind::Words | Kind::Strs | Kind::Texts => Ty::Str,
        Kind::Ints => Ty::Int,
        Kind::Floats => Ty::Float,
        Kind::Fractions => Ty::Fraction,
        Kind::Decimals => Ty::Decimal,
        Kind::Bools => Ty::Bool,
        Kind::Chunks => Ty::Bytes,
        Kind::Frames => Ty::Frame(None),
        Kind::Tensors => Ty::tensor(Ty::Float),
        Kind::Tensor => Ty::Float,
        _ => return None,
    })
}

/// A lowered value's type as inference's rules read a child's
/// (`infer_expr`): markup is text there.
fn read(x: &Expr) -> Ty {
    full(x).text()
}

/// What a loop walks, as inference reads it: a frame's column walks its
/// cells, as its type says them (a column's inferred type does not carry
/// them), anything else what its items are. (The collecting loop had read
/// a column as unknown in the scan, so a function called only inside one
/// took an unknown and was never emitted: the compiled program named a
/// function it did not hold.)
fn walked(it: &Expr) -> Ty {
    match it.ty().payload() {
        Ty::Column(cell) if **cell != Ty::Unknown => Ty::List(Box::new(as_inferred(cell))),
        _ => read(it),
    }
}

/// A collection written where it is used — `[]`, `map[]` — whose items
/// nothing later can say.
fn made_here(e: &Expr) -> bool {
    match e {
        Expr::ListLit(items, _) => items.is_empty(),
        Expr::MapLit(pairs, _) => pairs.is_empty(),
        Expr::Call { target: Target::Row(row), args, .. } => row.id == "core.map_new" && args.is_empty(),
        _ => false,
    }
}

/// A type with every "can fail" taken off, at any depth (a list of values
/// that can fail is a list of the values): what a kind check compares when
/// whether a value can fail is decided elsewhere.
fn sans_results(t: &Ty) -> Ty {
    match t {
        Ty::Result(inner) => sans_results(inner),
        Ty::List(item) => Ty::List(Box::new(sans_results(item))),
        Ty::Map(key, value) => Ty::Map(*key, Box::new(sans_results(value))),
        other => other.clone(),
    }
}

/// A lowered value's type as a variable keeps it: a column keeps its
/// cells' kind (`texts = str[t.at]` is a column of text, so `moment[texts]`
/// is one of moments and its cell a moment, not a parsed value — the
/// compiled program had compared the ISO text with a moment, and stopped
/// where the interpreter answered true).
fn kept(x: &Expr) -> Ty {
    match x.ty() {
        Ty::Column(cell) if !cell.has_unknown() => Ty::Column(cell),
        _ => full(x),
    }
}

/// A type as a variable keeps it: a column with its cells' kind, anything
/// else as inference reads it.
fn keep_cells(t: &Ty) -> Ty {
    match t {
        Ty::Column(cell) if !cell.has_unknown() => t.clone(),
        other => as_inferred(other),
    }
}

/// A lowered value's type with markup told apart (`infer_expr_full`): a
/// tree is markup.
fn full(x: &Expr) -> Ty {
    match x.ty() {
        Ty::Node => Ty::Markup,
        t => as_inferred(&t),
    }
}


/// The inference type an IR type came from: the Rust backend reads the
/// type tables (Rust names, defaults) through it.
/// A type as inference reads it: a column without its cells' type (the
/// lowering's types carry them; inference types a column by its frame's
/// columns where they are known), a view's node as nothing known. One
/// enum since 2026-10-07; this is what the conversion from the IR's
/// type to inference's used to forget.
pub fn as_inferred(t: &Ty) -> Ty {
    match t {
        Ty::Column(_) => Ty::Column(Box::new(Ty::Unknown)),
        Ty::Node => Ty::Unknown,
        Ty::Result(inner) => Ty::Result(Box::new(as_inferred(inner))),
        t => t.clone(),
    }
}

/// What a provisional lowering met (F5, signatures found by lowering
/// bodies): what inference reads from a body once the lowering types it.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Seen {
    /// Each call of a function or record the program declares, by the name
    /// written, with its arguments' types as lowered: markup told apart, a
    /// failure not yet unwrapped.
    pub calls: Vec<(String, Vec<Ty>)>,
    /// The type of each value the body returns, where it is returned (a
    /// collecting loop's body returns nothing of the function's).
    pub returns: Vec<Ty>,
    /// The local each of those returns hands back, where it is one.
    pub returned: Vec<Option<String>>,
    /// Each place an error leaves the body — a decision on a value that
    /// can fail, or an assignment refusing one — and whether it is inside
    /// a collecting loop.
    pub refusals: Vec<(Decision, bool)>,
}

/// How a body is lowered (F5).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pass {
    /// For the program: every check made, every decision kept.
    Final,
    /// To learn its locals' types: each variable settles across its
    /// assignments as the body runs, as inference's walk settles it.
    Settle,
    /// To learn what its calls say of their callees: each call notes its
    /// arguments' types, the environment kept as inference's call scan
    /// keeps it.
    Scan,
}

#[derive(Clone, Copy, PartialEq)]
enum Where {
    Top,
    In(FnKind),
    Derive,
    Test,
}

/// The names an earlier fragment bound, with the types inference holds for
/// them: the scope a fragment's main is checked in.
fn known_types(ctx: &TypeContext, known: &HashSet<String>) -> HashMap<String, Ty> {
    known.iter().map(|n| (n.clone(), ctx.vars.get(n).cloned().unwrap_or(Ty::Unknown))).collect()
}

struct Lowerer<'a> {
    ctx: &'a mut TypeContext,
    /// A session's fragment (`lower_fragment`): its top-level names
    /// outlive it.
    fragment: bool,
    /// The expressions whose value a block hands on, by address: a
    /// collecting loop's yield, a console cell's last line. Their value is
    /// not dropped, so a statement of one is never refused as dropping it.
    yields: HashSet<usize>,
    /// The locals that hold a gradient (`g = grad[f]`), by the function
    /// each is of: a call of one types that function's parameters.
    grad_of: HashMap<String, String>,
    /// A local pushed (or set) as an item while its own items were not
    /// known yet, by the function it is of: the holder's items are what
    /// the local settles on (`settled`).
    items_of: Vec<(String, String, String)>,
    /// Lowering `grad`'s arguments: the function it differentiates is typed
    /// by the run that traces it, as the verifier reads it.
    differentiating: bool,
    /// The function a row is being handed that the row calls with its own
    /// arguments (`revise[name, default, f, x]` calls `f[held, x]`): it is
    /// lowered at the copy those arguments reach, chosen once they are
    /// lowered, so it is no value of several kinds where it is named.
    handed_to_row: Option<String>,
    /// The variables each enclosing block grows with `push` (`v = push[v,
    /// …]`), innermost last: a list of whole numbers written down for one
    /// of them is a list of ints (`grown_by_push`).
    grown: Vec<HashSet<String>>,
    /// The locals holding a decomposition's pieces (`parts = svd[m]`), with
    /// each piece's shape by position (shapes::piece_dims).
    pieces: HashMap<String, Vec<Option<Dims>>>,
    /// Locals declared so far, innermost scope last.
    scopes: Vec<HashSet<String>>,
    /// Statements hoisted in front of the one being lowered (propagation).
    pre: Vec<Stmt>,
    /// Lowering a fact's declared value, which nothing can stand in front
    /// of: the facts are computed before the top level's statements run.
    in_fact_default: bool,
    /// The parameter list of the query or exec being lowered, as written
    /// (`{:?}` of its items): the one list of several kinds.
    params_items: Option<String>,
    temp: usize,
    at: Where,
    in_ui: bool,
    /// The program holds state: a ui program, or a web program that
    /// declares a derivation, which its routes read — its top level's
    /// values are facts, each given one value, that its derivations read.
    holds_state: bool,
    /// While a route's handler is lowered: the derivations it reads, each
    /// with its type — bound to a local of its name as the handler starts,
    /// and read as that local after (`lower_fn`).
    route_derived: Option<BTreeMap<String, Ty>>,
    /// Names every function in the program reaches, for `Fn.effects`.
    calls: BTreeMap<String, BTreeSet<String>>,
    /// The line each function first calls each callee on, for a refusal
    /// of what the caller reaches through it.
    call_lines: BTreeMap<(String, String), usize>,
    direct_effects: BTreeMap<String, BTreeSet<String>>,
    current_fn: String,
    /// What the checks keep (`checks.rs`).
    v: checks::Checks,
    /// How the body under way is lowered (F5, `provisional`).
    pass: Pass,
    /// While a body is lowered to learn its types, not to keep it: what
    /// the lowering met.
    seen: Option<Seen>,
    /// How many collecting loops the lowering is inside.
    collecting: usize,
    /// Above zero while the call scan lowers what inference's scan never
    /// reads (a callee lowered as a value).
    unscanned: usize,
    /// The collecting loops over a list of named functions the call scan
    /// is inside (`for f in [a, b]`): the loop's variable and the functions.
    fn_lists: Vec<(String, Vec<String>)>,
    /// The functions refused as undescribable, each once.
    undescribable: HashSet<String>,
    /// A provisional lowering's locals, each with its first value's type:
    /// with the type it settles on, what it is declared as (`declared_as`).
    first_types: HashMap<String, Ty>,
    /// A provisional lowering's assignments of a value that can fail to a
    /// variable declared before: the variable, the value's payload, and
    /// whether inside a collecting loop. Whether each refuses the failed
    /// value is known once the body's locals have settled.
    held: Vec<(String, Ty, bool)>,
    /// While a test is lowered for the handlers it clicks: each handler
    /// call a click holds, with its arguments' types (`type_clicks`).
    clicks: Option<Vec<(String, Vec<Ty>)>>,
    /// While a test is lowered for what only it types: each call of one of
    /// the program's functions it makes, with its arguments' types
    /// (`type_clicks`).
    tested: Option<Vec<(String, Vec<Ty>)>>,
    /// The kinds a function's returns answered so far in its final
    /// lowering, by function: a return of another kind is refused against
    /// them (`a function answers one kind`).
    returns_seen: HashMap<String, Vec<crate::table::Arg>>,
    /// The functions with a `return 0` — none, beside a record (spec, A
    /// Record or None) — kept apart from the kinds they return.
    returns_none: HashSet<String>,
    /// The names a guard knows have failed where the lowering is (`if not
    /// x.ok`'s body, `if x.ok`'s else): a return of one hands back its
    /// failure, never its value's type.
    failed: Vec<String>,
}

/// `map[keys, values]` of a tensor: the tensor as a list of the item kind,
/// in place of the argument. (Not inline, as `mask_pick`.)
#[inline(never)]
fn as_list(lowered: &mut [Expr], i: usize, item: Ty) {
    let taken = std::mem::replace(&mut lowered[i], Expr::Const(Const::Bool(false), Ty::Bool));
    lowered[i] = Expr::Convert { how: Convert::ToList, expr: Box::new(taken), ty: Ty::List(Box::new(item)) };
}

/// Whether a lowered value is a frame's column of bools (one that can fail
/// too: it is read, as any argument is, before the call) — by its IR type,
/// which keeps a column's cells (`read` forgets them). `or_unknown` takes a
/// column whose cells are not known yet as one: a pass before they settle,
/// a frame loaded with nothing describing it.
fn is_bool_column(x: &Expr, or_unknown: bool) -> bool {
    match x.ty().payload() {
        Ty::Column(cell) => cell.payload() == &Ty::Bool || (or_unknown && cell.payload() == &Ty::Unknown),
        _ => false,
    }
}

/// The mask a column of bools makes, as the program would write it: `c eq
/// true`.
fn as_mask(a: &A) -> A {
    A::BinOp { left: Box::new(a.clone()), op: BinOp::Eq, right: Box::new(A::Bool(true)) }
}

/// A pick's call (a mask's rows, items or cells): one that propagates
/// where an argument is read from a value that can fail — the backends read
/// such an argument through the lowering's mark only in a call that
/// propagates (the emitter's `raw`) — and is then typed as one that can
/// fail, as a builtin's call is (`xs[mask]` of a list read from a file had
/// not built compiled).
fn picked(row: &'static crate::table::Row, base: Expr, mask: Expr, ty: Ty) -> Expr {
    let fallible = |e: &Expr| matches!(e, Expr::Convert { how: Convert::Unwrap, .. }) || e.ty().is_result();
    let carried = fallible(&base) || fallible(&mask);
    let ty = if carried && !ty.is_result() { Ty::Result(Box::new(ty)) } else { ty };
    Expr::Call { target: Target::Row(row), args: vec![base, mask], ty, propagates: carried }
}

/// Whether a value can be a map's key: text or an int (or a value the
/// program reads as one as it runs).
fn key_like(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::Big | Ty::Str | Ty::Markup | Ty::Json | Ty::Unknown)
}

/// Whether a value can be a position: an int (or a value the program reads
/// as one as it runs); a mask is picked before this, and a list of
/// positions gathers.
fn position_like(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::Big | Ty::Json | Ty::Unknown | Ty::List(_) | Ty::Tensor(..) | Ty::Column(_))
}

/// Whether an expression reads a frame's column anywhere in it: a
/// comparison over one is a mask, one bool per row, even where the
/// column's cells are not typed yet.
fn reads_a_column(e: &Expr) -> bool {
    let mut found = false;
    crate::ir::visit_expr(e, &mut |x| found |= matches!(x.ty().payload(), Ty::Column(_)));
    found
}

/// A mask picks: the index's own row and the result's type, by the base —
/// `__pick_rows` of a tensor (its first dimension unknown after),
/// `__pick_items` of a list, `__pick_cells` of a frame's column — or none
/// where the index is not a mask, or either can fail. (A helper, not
/// inline: the lowering's recursive frame is measured against a 43-entry
/// map literal in the debug build, parity 48.)
#[inline(never)]
fn mask_pick(base: &Ty, index: &Ty) -> Option<(&'static crate::table::Row, Ty)> {
    if index.is_result() || base.is_result() {
        return None;
    }
    // a tensor of two axes or more by a mask of its own depth (what `t gt x`
    // answers): its numbers where the mask is true, as a vector
    if let Ty::Tensor(item, Some(dims)) = base.payload()
        && dims.len() >= 2
        && index.payload().truths_depth() == dims.len()
        && index.payload().is_truths()
    {
        return crate::table::row("core.__pick_numbers").map(|row| (row, Ty::Tensor(item.clone(), Some(vec![None]))));
    }
    if !matches!(index.payload(), Ty::List(item) if **item == Ty::Bool) {
        return None;
    }
    let base_ty = base.payload().clone();
    let (row_name, ty) = match &base_ty {
        Ty::Tensor(item, dims) => (
            "__pick_rows",
            Ty::Tensor(item.clone(), dims.as_ref().map(|d| { let mut d = d.clone(); if let Some(first) = d.first_mut() { *first = None; } d })),
        ),
        Ty::List(_) => ("__pick_items", base_ty.clone()),
        // (a frame's column: its cells where the mask is true, a column of
        // the same kind with its missing cells kept — `f.name[mask]`)
        Ty::Column(_) => ("__pick_cells", base_ty.clone()),
        _ => return None,
    };
    let kinds = [crate::table::Arg::of_type(&base_ty), crate::table::Arg::Truths];
    crate::table::core_row(row_name, &kinds).map(|row| (row, ty))
}

impl<'a> Lowerer<'a> {
    fn new(ctx: &'a mut TypeContext, in_ui: bool) -> Self {
        let web = ctx.bare_modules.iter().chain(ctx.module_aliases.values()).any(|m| m == "web" || m.starts_with("web."));
        let holds_state = in_ui || (web && !ctx.derives.is_empty());
        Lowerer {
            ctx,
            fragment: false,
            yields: HashSet::new(),
            grad_of: HashMap::new(),
            items_of: Vec::new(),
            differentiating: false,
            handed_to_row: None,
            grown: Vec::new(),
            pieces: HashMap::new(),
            scopes: vec![HashSet::new()],
            pre: Vec::new(),
            temp: 0,
            at: Where::Top,
            in_ui,
            holds_state,
            route_derived: None,
            in_fact_default: false,
            params_items: None,
            calls: BTreeMap::new(),
            call_lines: BTreeMap::new(),
            direct_effects: BTreeMap::new(),
            current_fn: String::new(),
            v: checks::Checks::default(),
            pass: Pass::Final,
            seen: None,
            collecting: 0,
            unscanned: 0,
            fn_lists: Vec::new(),
            undescribable: HashSet::new(),
            first_types: HashMap::new(),
            held: Vec::new(),
            clicks: None,
            tested: None,
            returns_seen: HashMap::new(),
            returns_none: HashSet::new(),
            failed: Vec::new(),
        }
    }

    /// A lowering that only infers (F5): what the checks read before the
    /// walk is read, and nothing it finds is kept but types.
    fn for_inference(ctx: &'a mut TypeContext, in_ui: bool, stmts: &[S]) -> Self {
        let mut l = Lowerer::new(ctx, in_ui);
        l.prescan(stmts, &HashSet::new());
        l
    }

    /// A body lowered to learn its types, keeping nothing of it (F5): the
    /// checks say nothing, what the lowering keeps for the program is left
    /// as it was, and each variable's type settles across its assignments
    /// as the body runs, the way inference's walk settles it, so the body's
    /// environment is left in the context. `own` is the body's first scope
    /// (its parameters). An error is the final lowering's to report.
    fn provisional(&mut self, pass: Pass, own: Option<HashSet<String>>, body: &[S]) -> Seen {
        self.provisional_block(pass, own, body).0
    }

    /// A provisional lowering, and the block it lowered to (read, and
    /// thrown away).
    fn provisional_block(&mut self, pass: Pass, own: Option<HashSet<String>>, body: &[S]) -> (Seen, Result<Block, String>) {
        let outer = self.seen.replace(Seen::default());
        let outer_pass = std::mem::replace(&mut self.pass, pass);
        let temp = self.temp;
        let pre = std::mem::take(&mut self.pre);
        let depth = self.scopes.len();
        let (mute, line, collecting, unscanned, fn_lists) = (self.v.mute, self.v.line, self.collecting, self.unscanned, self.fn_lists.len());
        let local_types = self.v.local_types.clone();
        let stored = self.v.stored.clone();
        let outer_first = std::mem::take(&mut self.first_types);
        let outer_held = std::mem::take(&mut self.held);
        self.v.mute += 1;
        if let Some(own) = own {
            self.scopes.push(own);
        }
        let lowered = self.lower_block(body);
        self.scopes.truncate(depth);
        // a variable holds no error when it is declared with a type that
        // cannot fail: an assignment of a failed value to it refuses the
        // value and hands the error out of the body (spec, Reassignment),
        // decided with the type the final lowering declares it with
        for (name, payload, collecting) in std::mem::take(&mut self.held) {
            let settled = self.ctx.vars.get(&name);
            let held = match self.first_types.get(&name) {
                Some(first) => as_inferred(&declared_as(&(first).clone(), settled)),
                None => settled.cloned().unwrap_or(Ty::Unknown),
            };
            if crate::type_infer::settle::refuses_failed(&held, &payload) {
                if let Some(seen) = &mut self.seen {
                    seen.refusals.push((Decision::Assign(name), collecting));
                }
            }
        }
        self.first_types = outer_first;
        self.held = outer_held;
        self.v.mute = mute;
        self.v.line = line;
        self.v.local_types = local_types;
        self.v.stored = stored;
        self.collecting = collecting;
        self.unscanned = unscanned;
        self.fn_lists.truncate(fn_lists);
        self.pre = pre;
        self.temp = temp;
        self.pass = outer_pass;
        (std::mem::replace(&mut self.seen, outer).unwrap_or_default(), lowered)
    }

    /// Whose the top-level statement being lowered is: the part its
    /// declaration's prefix names, or the entry's (a statement that
    /// declares nothing, `page[…]`, is the entry's). A part's top level
    /// reaches what it reaches under this name (`Part.main_effects`).
    fn main_of_part(&self) -> String {
        let prefix = self.v.decl.as_deref().and_then(crate::part_prefix_of_declaration).unwrap_or_default();
        format!("<main>{}", prefix)
    }

    /// The kind of function a declaration is, as the lowering writes it.
    /// The context the lowering stands in, where the language tells it
    /// apart for effects (crate::effects::Context): a view, a derivation.
    fn context(&self) -> Option<crate::effects::Context> {
        match self.at {
            Where::In(FnKind::View) => Some(crate::effects::Context::View),
            Where::Derive => Some(crate::effects::Context::Derive),
            _ => None,
        }
    }

    /// An effect called where the context cannot hold it — a view reaches
    /// nothing but the console, a derivation only reads — is refused where
    /// it is written, by its kind, in the one wording (crate::effects);
    /// one reached through a function the context calls is refused once
    /// every function's reach is known (lower_program).
    fn effect_in_context(&mut self, name: &str) {
        let kinds = crate::table::effects_of(name);
        self.effect_kinds_in_context(name, kinds);
    }

    /// The same, for a call written with its module: the module's row's kinds.
    fn effect_in_context_in(&mut self, module: &str, name: &str) {
        let kinds = crate::table::effects_of_in(module, name);
        self.effect_kinds_in_context(name, kinds);
    }

    fn effect_kinds_in_context(&mut self, name: &str, kinds: Vec<crate::effects::Effect>) {
        if let Some(ctx) = self.context() {
            if let Some(e) = ctx.refused(&kinds) {
                let bare = crate::table::shown_effect(name);
                self.refuse(ctx.refusal(bare, e, None));
            }
        }
    }

    fn kind_of(&self, name: &str, body: &[S], is_view: bool) -> FnKind {
        if is_view || self.ctx.views.contains(name) {
            FnKind::View
        } else if self.ctx.view_handlers.contains(name) {
            FnKind::Handler
        } else if self.in_ui && !crate::modules::is_crow_fn(name) && assigns_any(body, &own_facts(name, &self.ctx.facts)) {
            // (a script has no facts: its function's first assignment of a
            // name is a local, whatever the top level calls)
            FnKind::Helper
        } else {
            FnKind::Pure
        }
    }

    // ── inference by lowering (F5) ──────────────────────────────────

    /// The call scan: every body lowered in order, each call noting its
    /// arguments' types for its callee's signature — a function's body
    /// with its parameters bound, a derivation's like a parameterless
    /// function's, the top level's statements in the top level's
    /// environment. A test says nothing of the program's signatures.
    fn scan_program(&mut self, stmts: &[S]) {
        for s in stmts {
            if let S::FnDecl { name, defaults, .. } = s {
                self.ctx.fn_defaults.insert(name.clone(), defaults.clone());
            }
        }
        let top_grown = top_grown(stmts);
        for s in stmts {
            match s {
                S::FnDecl { name, params, body, is_view, .. } => {
                    let scan = self.ctx.enter_fn_scan(name);
                    self.at = Where::In(self.kind_of(name, body, *is_view));
                    self.current_fn = name.clone();
                    self.ctx.registry.enter(&self.current_fn);
                    self.provisional(Pass::Scan, Some(params.iter().cloned().collect()), body);
                    self.ctx.leave_fn_scan(name, scan);
                }
                S::Derive { name, body, .. } => {
                    let scan = self.ctx.enter_derive_scan(name);
                    self.at = Where::Derive;
                    self.current_fn = name.clone();
                    self.ctx.registry.enter(&self.current_fn);
                    self.provisional(Pass::Scan, Some(HashSet::new()), body);
                    self.ctx.leave_derive_scan(scan);
                }
                S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } => {}
                other => {
                    self.at = Where::Top;
                    self.current_fn = "<main>".to_string();
                    self.ctx.registry.enter(&self.current_fn);
                    self.top_statement(Pass::Scan, other, &top_grown);
                }
            }
        }
    }

    /// A function's body typed (F5): its locals, its return type and
    /// whether an error leaves it, all as a provisional lowering met them
    /// (`finish_body`).
    fn infer_body(&mut self, name: &str, body: &[S]) {
        let params: HashSet<String> = self.ctx.functions.get(name).map(|sig| sig.params.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default();
        let scope = self.ctx.enter_body(name);
        self.at = if self.ctx.derives.contains(name) { Where::Derive } else { Where::In(self.kind_of(name, body, false)) };
        self.current_fn = name.to_string();
        self.ctx.registry.enter(&self.current_fn);
        let mut seen = self.provisional(Pass::Settle, Some(params), body);
        // (a local handed back before the line that typed it — `return xs`
        // of `xs = []` inside a `while true`, the push after — answers what
        // the local settled on: typed where it was met, the answer had
        // been a list of nothing known, compiled as text; Rosetta Code,
        // Piprimes)
        for (t, local) in seen.returns.iter_mut().zip(&seen.returned) {
            if t.has_unknown()
                && let Some(v) = local
                && let Some(settled) = self.ctx.vars.get(v)
                && settled.is_concrete()
                && t.compatible(settled)
            {
                *t = settled.clone();
            }
        }
        self.ctx.finish_body(name, scope, &seen.returns, !seen.refusals.is_empty());
    }

    /// What only a test's click types (F5): a handler a test clicks with
    /// arguments (`click[a, h[4]]`) takes their types for the parameters
    /// nothing in the program types — the click may be the only call it
    /// has, and a compiled program dispatches it by them. A function only
    /// tests call takes the types their calls agree on, the same way (its
    /// calls had been run by name: nothing in the program typed it), and a
    /// part's function its own tests' calls, as the part alone typed them.
    /// Each test is lowered as it runs, its own variables in view where its
    /// clicks' and calls' arguments are typed. Whether any parameter moved.
    fn type_clicks(&mut self, stmts: &[S]) -> bool {
        let mut clicks: Vec<(String, Vec<Ty>)> = Vec::new();
        let mut tested: Vec<(String, Vec<Ty>)> = Vec::new();
        for s in stmts {
            if let S::TestBlock { name, params, body } = s {
                let saved = self.ctx.vars.clone();
                for p in params {
                    self.ctx.vars.insert(p.clone(), Ty::Unknown);
                }
                self.at = Where::Test;
                self.current_fn = name.clone().unwrap_or_default();
                self.ctx.registry.enter(&self.current_fn);
                self.clicks = Some(Vec::new());
                self.tested = Some(Vec::new());
                self.provisional(Pass::Settle, Some(params.iter().cloned().collect()), body);
                clicks.extend(self.clicks.take().unwrap_or_default());
                tested.extend(self.tested.take().unwrap_or_default());
                self.ctx.vars = saved;
            }
        }
        let clicked = self.ctx.type_clicked(&clicks);
        // the program's own tests' calls, kept for a program that imports
        // it; then its parts' tests', which are not its tests
        self.ctx.tested = tested.clone();
        tested.extend(crate::tested_by_parts());
        self.ctx.type_tested(&tested) || clicked
    }

    /// The F2 monomorphization pass (F5): each clone's calls read with its
    /// parameters bound, then its body typed; every clone's body typed
    /// again after, so return types settle whatever the order clones were
    /// made in; then every clone's calls read once more, since what a copy
    /// answers types the function it is handed to — `inner[may[m, false]]`
    /// in a copy of `outer`, where `may`'s copy had no answer yet when the
    /// copy was first read, so `inner` was never copied for it and the
    /// compiled program did not build (parity 259) — and a signature that
    /// only now shows is copied in the next round.
    fn specialize(&mut self) {
        for _round in 0..5 {
            let new_clones = self.ctx.new_clones();
            if new_clones.is_empty() {
                break;
            }
            for mangled in &new_clones {
                self.scan_clone(mangled);
                let body = self.ctx.functions.get(mangled).map(|sig| sig.body.clone()).unwrap_or_default();
                self.infer_body(mangled, &body);
            }
            for _ in 0..2 {
                for mangled in self.ctx.clone_names() {
                    if let Some(body) = self.ctx.reset_clone(&mangled) {
                        self.infer_body(&mangled, &body);
                    }
                }
            }
            for mangled in self.ctx.clone_names() {
                self.scan_clone(&mangled);
            }
        }
    }

    /// A clone's calls read with its parameters bound (the scan pass, which
    /// notes each call's signature for its callee).
    fn scan_clone(&mut self, mangled: &str) {
        let body = self.ctx.functions.get(mangled).map(|sig| sig.body.clone()).unwrap_or_default();
        let params: HashSet<String> = self.ctx.functions.get(mangled).map(|sig| sig.params.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default();
        // (the copy's scan enters the body's scope as the function's own
        // does: its first assignment to a name is a local of its own, and
        // inside a standard module's copy the program's names are
        // invisible — a copy of llm's `act` had read the program's `one`,
        // a record its first call answered, as its own local `one`, and
        // refused its own push of a schema onto a list)
        let scan = self.ctx.enter_fn_scan(mangled);
        self.at = Where::In(self.kind_of(mangled, &body, false));
        self.current_fn = mangled.to_string();
        self.ctx.registry.enter(&self.current_fn);
        self.provisional(Pass::Scan, Some(params), &body);
        self.ctx.leave_fn_scan(mangled, scan);
    }

    /// The top level's variables (F5): its statements lowered as they run,
    /// each variable settled across its assignments.
    fn settle_top(&mut self, stmts: &[S]) {
        self.at = Where::Top;
        self.current_fn = "<main>".to_string();
        self.ctx.registry.enter(&self.current_fn);
        let top_grown = top_grown(stmts);
        for s in stmts {
            match s {
                S::FnDecl { .. } | S::Derive { .. } | S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } => {}
                other => self.top_statement(Pass::Settle, other, &top_grown),
            }
        }
    }

    /// One top-level statement lowered provisionally, alone, with what the
    /// whole top level grows by `push` in view, as the final pass has it:
    /// rows of whole numbers written down and grown are a list of lists of
    /// ints in every pass (met alone, `decks = [[2, 4, 1, 3]]` had been a
    /// matrix the push made one of floats, and the final pass's list of
    /// lists was refused as a reassignment; Rosetta Code, Topswops).
    fn top_statement(&mut self, pass: Pass, stmt: &S, top_grown: &HashSet<String>) {
        self.lists_grown(top_grown);
        self.grown.push(top_grown.clone());
        self.provisional(pass, None, std::slice::from_ref(stmt));
        self.grown.pop();
    }

    /// A call's arguments' types noted for its callee's signature, in the
    /// call scan (F5).
    fn scan_call(&mut self, callee: &A, args: &[A], types: &[Ty]) {
        if self.pass != Pass::Scan || self.unscanned > 0 {
            return;
        }
        if self.ctx.call_says_of_callee(callee) {
            self.ctx.note_call(callee, args, types);
        }
        // (an item of a list whose items no type says, handed to a function:
        // held as text where it is compiled — `default_unknown_params`)
        if let A::Ident(n) = callee
            && self.ctx.functions.contains_key(n)
        {
            self.ctx.call_edges.insert((self.current_fn.clone(), n.clone()));
            for (i, a) in args.iter().enumerate() {
                if !matches!(types.get(i), Some(Ty::Unknown)) {
                    continue;
                }
                let item = match a {
                    A::Ident(v) => self.v.unknown_items.contains(v),
                    A::Call { callee: c, args: at } if at.len() == 1 => matches!(c.as_ref(), A::Ident(xs) if matches!(self.ctx.vars.get(xs).map(|t| t.payload().clone()), Some(Ty::List(e)) if *e == Ty::Unknown)),
                    _ => false,
                };
                if item {
                    self.ctx.text_items.insert((self.current_fn.clone(), n.clone(), i));
                }
            }
        }
        // (the program's first call of a module's function: where a refusal
        // inside the module's copy is placed)
        if let A::Ident(n) = callee {
            if self.calls_fn(n) && !crate::modules::is_crow_fn(&self.current_fn) && self.v.line > 0 {
                self.ctx.first_call_line.entry(n.clone()).or_insert(self.v.line);
            }
        }
        // a call through a loop's variable over a list of named functions
        // types every one of them (an uncalled function would keep its
        // parameters unknown and not be emitted)
        if let A::Ident(n) = callee {
            if let Some((_, fns)) = self.fn_lists.iter().rev().find(|(v, _)| v == n) {
                let text: Vec<Ty> = types.iter().map(|t| t.text()).collect();
                for f in fns.clone() {
                    self.ctx.note_through_list(&f, &text);
                }
            }
        }
    }

    /// A loop's body in the call scan (F5): read again when a variable from
    /// before the loop took a type in it, so its calls see that type.
    fn scan_loop_body(&mut self, body: &[S]) -> Result<Block, String> {
        let outer = self.ctx.vars.clone();
        let scope = self.scopes.last().cloned().unwrap_or_default();
        let b = self.lower_block(body)?;
        let moved = outer.iter().any(|(n, t)| self.ctx.vars.get(n).map_or(false, |now| now != t));
        if moved {
            if let Some(last) = self.scopes.last_mut() {
                *last = scope;
            }
            return self.lower_block(body);
        }
        Ok(b)
    }

    // ── declarations ────────────────────────────────────────────────

    fn lower_part(&mut self, stmts: &[S], name: &str) -> Result<Part, String> {
        let mut part = Part {
            name: name.to_string(),
            uses: vec![],
            cpu: None,
            gpu: None,
            seeds: vec![],
            structs: vec![],
            facts: vec![],
            derives: vec![],
            fns: vec![],
            tests: vec![],
            main: vec![],
            main_effects: BTreeMap::new(),
            traced: std::collections::BTreeSet::new(),
        };
        let fact_names: BTreeSet<String> = self.ctx.facts.clone();
        let written: BTreeSet<String> = facts_written_by_fns(stmts, &fact_names);
        // (the top level's own statements, as a block's: `grown_by_push`)
        let mut top_grown = HashSet::new();
        grown_by_push(stmts, &mut top_grown);
        self.lists_grown(&top_grown);
        self.grown = vec![top_grown];
        // (a name declared twice — two functions, two records, a function
        // and a derivation — means two things: the later had silently
        // replaced the earlier interpreted, and the compiled program had
        // not built for a record; Rosetta Code, Sealed classes and methods)
        let mut declared: HashMap<String, usize> = HashMap::new();
        for s in stmts {
            // an error is owned by the declaration it is found in; a line
            // marker belongs to the statement after it, and an attached test
            // (`test[p]` after a fn) to the fn it follows
            match s {
                S::Line(_) | S::TestBlock { name: None, .. } => {}
                _ => self.v.decl = checks::declared_name(s),
            }
            if let S::StructDecl { name, .. } | S::FnDecl { name, .. } | S::Derive { name, .. } = s
                && !crate::modules::is_crow_fn(name)
            {
                match declared.get(name) {
                    Some(first) => self.refuse(format!(
                        "'{}' is declared twice — first on line {}: a name means one thing (spec, Names Are Never Ambiguous), so rename one",
                        crate::lower::shown(name),
                        first
                    )),
                    None => {
                        declared.insert(name.clone(), self.v.line);
                    }
                }
            }
            match s {
                S::Use { path, alias } => {
                    part.uses.push(match alias {
                        Some(a) => format!("{} as {}", path, a),
                        None => path.clone(),
                    });
                }
                S::StructDecl { name, fields } => {
                    self.check_struct(name, fields);
                    let sig = self.ctx.structs.get(name).cloned();
                    let fields = match sig {
                        Some(sig) => sig.fields.iter().map(|(n, t)| (n.clone(), (t).clone())).collect(),
                        None => fields.iter().map(|f| (f.clone(), Ty::Unknown)).collect(),
                    };
                    part.structs.push(StructDef { name: name.clone(), shown: crate::declared_record_name(name), fields });
                }
                S::FnDecl { name, params, body, is_view, .. } => {
                    // (a standard module's function, inlined behind its
                    // prefix, was checked as a program of its own under its
                    // own imports: the program's imports and aliases reserve
                    // nothing in it — `use math.stats as s` beside a module
                    // whose parameter is `s`)
                    if !crate::modules::is_crow_fn(name) {
                        self.check_reserved(name, if *is_view { "view" } else { "function" });
                        for p in params {
                            self.check_reserved(p, "parameter");
                            self.check_not_a_function(p, "parameter");
                        }
                    }
                    // a view is drawn by the ui module: without `use ui` its
                    // elements are no names at all (the interpreter had
                    // stopped on the first, "does not run 'p' yet", and the
                    // compiled program had not built; Rosetta Code, Create an
                    // HTML table)
                    if *is_view && !self.in_ui && !crate::modules::is_crow_fn(name) {
                        self.refuse(format!("'{}' is a view, which `use ui` draws: add `use ui`", crate::lower::shown(name)));
                    }
                    // (a function copied per signature is called only through
                    // its copies: its own lowering, at every call's types
                    // joined, refuses nothing a copy does not — a matrix
                    // and a vector joined had no rank, and `x[0]` was refused
                    // where each copy reads its own — crow check --self)
                    let copied = self.ctx.specializations.get(name).is_some_and(|v| !v.is_empty());
                    let f = if copied { self.muted(|l| l.lower_fn(name, params, body, *is_view))? } else { self.lower_fn(name, params, body, *is_view)? };
                    part.fns.push(f);
                    // a function called at several signatures is one function
                    // per signature, each typed by its own (F2): the Rust
                    // backend compiles each. Each is checked at its own
                    // signature: a copy's types can refuse what the function's
                    // did not (`set[cols, …]` where one call hands a list and
                    // the recursive call the map set answers — the list's copy
                    // had resolved to the map's row, unrefused, and run
                    // interpreted only to stop, compiled not built); a refusal
                    // both make is printed once
                    let clones: Vec<String> = self.ctx.specializations.get(name).map(|v| v.iter().map(|(_, m)| m.clone()).collect()).unwrap_or_default();
                    for clone in clones {
                        let clone_body = self.ctx.functions.get(&clone).map(|sig| sig.body.clone()).unwrap_or_else(|| body.clone());
                        let f = self.lower_fn(&clone, params, &clone_body, *is_view)?;
                        part.fns.push(f);
                    }
                }
                S::Derive { name, fresh, body } => {
                    self.check_derive(name, body);
                    let d = self.lower_derive(name, *fresh, body)?;
                    part.derives.push(d);
                }
                S::TestBlock { name, params, body } => {
                    self.v.attached_test = name.is_none();
                    let t = self.lower_test(name.clone(), params, body)?;
                    self.v.attached_test = false;
                    part.tests.push(t);
                }
                S::Assign { name, value } if self.holds_state && fact_names.contains(name) && !self.declared(name) => {
                    self.at = Where::Top;
                    self.current_fn = self.main_of_part();
                    self.ctx.registry.enter(&self.current_fn);
                    if self.ctx.derives.contains(name) {
                        self.refuse(format!("'{}' is derived — it follows what it reads; change those facts instead of assigning it", name));
                    }
                    let line = self.v.line;
                    self.in_fact_default = true;
                    let default = self.lower_expr(value);
                    self.in_fact_default = false;
                    let default = default?;
                    // a loaded frame is held to the columns the program
                    // reads where it is bound; a page's data source carries
                    // them to the session, where its answer arrives
                    let default = self.held_to_shape(name, default);
                    let held = if is_source_call(&default) { self.held_of(name) } else { Vec::new() };
                    if !self.refuse_nothing_held(&format!("'{}'", crate::lower::shown(name)), &default) {
                        self.check_assign(name, &self.checked(&default));
                    }
                    self.flush_pre_into(&mut part.main);
                    // a fact holds no error: a declared value that can fail
                    // is refused where the facts are set, and the fact holds
                    // the payload (it had held the error, and a compiled
                    // program could not keep it in a session's state)
                    let refuses = default.ty().is_result();
                    let ty = self.ctx.vars.get(name).cloned().unwrap_or_else(|| default.ty());
                    let ty = if refuses { ty.payload().clone() } else { ty };
                    // a fact's tensor has the shape its declared value states
                    let ty = match default.ty().tensor_dims().cloned() {
                        Some(d) => shapes::shaped(ty, Some(d)),
                        None => ty,
                    };
                    // an int default of a fact that holds floats is a float
                    // (spec §1): the interpreter held the int until a handler
                    // stored a float, where the compiled program converted it
                    let default = if ty == Ty::Float && default.ty() == Ty::Int {
                        Expr::Convert { how: Convert::IntToFloat, expr: Box::new(default), ty: Ty::Float }
                    } else {
                        default
                    };
                    self.scopes[0].insert(name.clone());
                    self.v.fact_lines.insert(name.clone(), self.v.line);
                    part.facts.push(Fact { name: name.clone(), ty, default, session: written.contains(name), line, refuses, held });
                }
                other => {
                    self.at = Where::Top;
                    self.current_fn = self.main_of_part();
                    self.ctx.registry.enter(&self.current_fn);
                    let lowered = self.lower_stmt(other)?;
                    // (a page's top-level call that sets the draws — an
                    // effect on them answering nothing, `seed[n]` — runs
                    // before the declarations are set, so their draws are
                    // seeded; Vijay, 2026-10-29: Rosetta Code, Create an
                    // HTML table)
                    if self.holds_state && self.pre.is_empty() {
                        let mut steps = lowered.iter().filter(|s| !matches!(s, Stmt::Line(_)));
                        let sets_draws = |row: &crate::table::Row| matches!(row.ret, crate::table::Kind::Unit) && row.props.contains(&crate::table::Prop::Does(crate::effects::Effect::Random));
                        if let (Some(Stmt::Do(e)), None) = (steps.next(), steps.next())
                            && matches!(e, Expr::Effect { row: Some(row), .. } | Expr::Call { target: Target::Row(row), .. } if sets_draws(row))
                        {
                            part.seeds.push(e.clone());
                            continue;
                        }
                    }
                    self.flush_pre_into(&mut part.main);
                    part.main.extend(lowered);
                }
            }
        }
        // (a session's top-level names are read by the fragments after it)
        let outlive = if self.fragment { defined_in(&part.main) } else { HashSet::new() };
        place_moves_in(&mut part.main, &outlive, None);
        // what a derivation and each part's top level reach, transitively,
        // as a function's does below
        let reach = |start: &str, l: &Self| -> Vec<String> {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![start.to_string()];
            let mut effects: BTreeSet<String> = BTreeSet::new();
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if let Some(e) = l.direct_effects.get(&n) {
                    effects.extend(e.iter().cloned());
                }
                if let Some(c) = l.calls.get(&n) {
                    todo.extend(c.iter().cloned());
                }
            }
            effects.into_iter().collect()
        };
        for d in &mut part.derives {
            d.effects = reach(&d.name, self);
        }
        let mains: Vec<String> = self.direct_effects.keys().chain(self.calls.keys()).filter(|k| k.starts_with("<main>")).cloned().collect();
        for m in mains {
            let effects = reach(&m, self);
            let prefix = m.trim_start_matches("<main>").to_string();
            part.main_effects.entry(prefix).or_default().extend(effects);
        }
        for effects in part.main_effects.values_mut() {
            effects.sort();
            effects.dedup();
        }
        // what each function reaches, transitively
        let names: Vec<String> = part.fns.iter().map(|f| f.name.clone()).collect();
        for f in &mut part.fns {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![f.name.clone()];
            let mut effects: BTreeSet<String> = BTreeSet::new();
            let mut recursive = false;
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if let Some(e) = self.direct_effects.get(&n) {
                    effects.extend(e.iter().cloned());
                }
                if let Some(c) = self.calls.get(&n) {
                    for callee in c {
                        // reaching itself again: its calls can nest without end
                        recursive |= *callee == f.name;
                        if names.contains(callee) {
                            todo.push(callee.clone());
                        }
                    }
                }
            }
            f.effects = effects.into_iter().collect();
            f.recursive = recursive;
        }
        // a view or a derivation that reaches, through a function it calls,
        // an effect its context cannot hold (crate::effects::Context: a view
        // reaches nothing but the console, a derivation only reads) would
        // perform it at every render: refused by the kind, in the one
        // wording (a call of its own is refused where it is written)
        let renders: Vec<(String, crate::effects::Context)> = part
            .fns
            .iter()
            .filter(|f| f.kind == FnKind::View)
            .map(|f| (f.name.clone(), crate::effects::Context::View))
            .chain(part.derives.iter().map(|d| (d.name.clone(), crate::effects::Context::Derive)))
            .collect();
        for (r, ctx) in renders {
            let own: BTreeSet<String> = self.direct_effects.get(&r).cloned().unwrap_or_default();
            let mut found: Option<(String, String, crate::effects::Effect)> = None;
            for c in self.calls.get(&r).into_iter().flatten() {
                let Some(f) = part.fns.iter().find(|f| &f.name == c) else { continue };
                for e in &f.effects {
                    if own.contains(e) {
                        continue;
                    }
                    if let Some(k) = ctx.refused(&crate::table::effects_of(e)) {
                        found = Some((c.clone(), e.clone(), k));
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if let Some((via, e, k)) = found {
                let bare = e.rsplit('.').next().unwrap_or(&e).to_string();
                let line = self.call_lines.get(&(r.clone(), via.clone())).copied().unwrap_or(0);
                let msg = format!("line {}: {}", line, ctx.refusal(&bare, k, Some(&via)));
                self.v.errors.push((msg, Some(r.clone())));
            }
        }
        // what the live layer memoizes each derivation by, and the tables
        // each function writes, recorded here so the Rust backend reads the
        // same decisions the interpreter does
        {
            let derive_names = self.ctx.derives.clone();
            let bodies: BTreeMap<String, Vec<S>> = self
                .ctx
                .functions
                .iter()
                .filter(|(n, _)| !derive_names.contains(*n))
                .map(|(n, sig)| (n.clone(), sig.body.clone()))
                .collect();
            // (read once, before the walk: the checks the walk made read them)
            let specs = self.v.read_sets.clone();
            for d in &mut part.derives {
                if let Some(rs) = specs.get(&d.name) {
                    d.spec = rs.clone();
                }
            }
            for f in &mut part.fns {
                if let Some(body) = bodies.get(&f.name) {
                    let reach = crate::derive::called_fns(body, &bodies);
                    let reach_bodies: BTreeMap<String, Vec<S>> = reach.iter().filter_map(|n| bodies.get(n).map(|b| (n.clone(), b.clone()))).collect();
                    f.tables_written = crate::derive::owned_tables(body, &reach_bodies).into_iter().collect();
                }
            }
        }
        // a derivation reads the clock when anything it calls does
        for d in &mut part.derives {
            if d.reads.clock {
                continue;
            }
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![d.name.clone()];
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if self.direct_effects.get(&n).map_or(false, |e| e.contains("now")) {
                    d.reads.clock = true;
                    break;
                }
                if let Some(c) = self.calls.get(&n) {
                    todo.extend(c.iter().cloned());
                }
            }
        }
        Ok(part)
    }

    fn lower_fn(&mut self, name: &str, params: &[String], body: &[S], is_view: bool) -> Result<Fn, String> {
        let kind = self.kind_of(name, body, is_view);
        // the body typed as inference types it: parameters from the
        // signature, first assignments new bindings
        let scope = self.ctx.enter_body(name);
        let sig = self.ctx.functions.get(name).cloned();
        let mut ps: Vec<(String, Ty)> = Vec::new();
        for p in params {
            let t = sig
                .as_ref()
                .and_then(|s| s.params.iter().find(|(n, _)| n == p).map(|(_, t)| t.clone()))
                .unwrap_or(Ty::Unknown);
            ps.push((p.clone(), t.clone()));
        }
        let ret = sig.as_ref().map(|s| (&s.return_type).clone()).unwrap_or(Ty::Unknown);
        let ret = if kind == FnKind::View { Ty::Node } else { ret };
        self.at = Where::In(kind);
        self.current_fn = name.to_string();
        self.ctx.registry.enter(&self.current_fn);
        // a view or handler of a ui program sees its reactive state, the
        // sanctioned exception to function-scope isolation
        self.v.reactive = self.v.ui_mode && matches!(kind, FnKind::View | FnKind::Handler);
        self.v.local_types.clear();
        // (a parameter is a local holding what its calls hand: reassigned
        // another kind — `t = t + 1` of a parsed value — it is refused as a
        // local is, where its first reassignment had been taken for its
        // type and the lowered program disagreed with itself; crow check
        // --self, errors j84)
        for (p, t) in &ps {
            if !matches!(t, Ty::Unknown) {
                self.v.local_types.insert(p.clone(), t.clone());
            }
        }
        self.v.stored.clear();
        // a route's handler reads each derivation it names once, as its
        // request begins: the app's value then, bound to a local of the
        // derivation's name that the body reads (both backends run the one
        // read, and a read in a loop is a local's, not a copy each round)
        let route = self.v.route_handlers.contains(name);
        let saved_route = std::mem::replace(&mut self.route_derived, route.then(BTreeMap::new));
        // lowered once to learn its types: each local settled across its
        // assignments, which a first value of no known type is declared by
        self.provisional(Pass::Settle, Some(params.iter().cloned().collect()), body);
        self.record_env(name, params, body);
        if let Some(read) = self.route_derived.as_mut() {
            read.clear();
        }
        self.scopes.push(params.iter().cloned().collect());
        // (a function sees no top-level variable, nor what grows there)
        let enclosing = std::mem::take(&mut self.grown);
        let block = self.lower_block(body);
        self.grown = enclosing;
        self.scopes.pop();
        self.v.reactive = false;
        let mut block = block?;
        self.check_answered(name, kind, &ret, &block);
        if let Some(read) = std::mem::replace(&mut self.route_derived, saved_route) {
            let mut start: Vec<Stmt> = read.into_iter().map(|(d, ty)| Stmt::Let { var: d.clone(), ty: ty.clone(), expr: Expr::Derived(d, ty) }).collect();
            start.append(&mut block);
            block = start;
        }
        self.ctx.leave_body(scope);
        let param_set: HashSet<String> = params.iter().cloned().collect();
        place_moves_in(&mut block, &HashSet::new(), Some(&param_set));
        Ok(Fn { name: name.to_string(), params: ps, ret, body: block, kind, effects: vec![], tables_written: vec![], recursive: false })
    }

    /// A function that answers a value answers one on every path (spec,
    /// Functions): a `return` with no value, and a body that can end
    /// without one — a loop that finds nothing, an `if` with no `else` —
    /// are refused in both modes. The interpreter had answered `done` where
    /// the function's value was an int, the compiled program had not built,
    /// and a bare `return` had been "the lowered program disagrees with
    /// itself" (crow check --self, a route's handler that answered on one
    /// path).
    fn check_answered(&mut self, name: &str, kind: FnKind, ret: &Ty, body: &[Stmt]) {
        if self.pass != Pass::Final || matches!(kind, FnKind::View | FnKind::Handler) || matches!(ret.payload(), Ty::Unit | Ty::Unknown) {
            return;
        }
        let what = crate::table::Arg::of_type(ret.payload()).words();
        let shown = crate::lower::shown(name);
        let saved = self.v.line;
        if let Some(line) = bare_return(body, saved) {
            self.v.line = line;
            self.refuse(format!("{}[] returns {}, and this `return` returns nothing: return a value here too — a function answers on every path or on none", shown, what));
        } else if !ends_answered(body) {
            self.v.line = body.iter().rev().find_map(|s| if let Stmt::Line(n) = s { Some(*n) } else { None }).unwrap_or(saved);
            self.refuse(format!("{}[] returns {}, and its body can end here without returning one: end it with the value it answers then — a function answers on every path or on none", shown, what));
        }
        self.v.line = saved;
    }

    /// A body's parameters and locals as its lowering begins with them, kept
    /// when `crow build --types` asks (`type_infer::dump_types`).
    fn record_env(&mut self, key: &str, params: &[String], body: &[S]) {
        if !self.ctx.record_envs {
            return;
        }
        let mut own: HashSet<String> = params.iter().cloned().collect();
        own.extend(crate::lower::assigned(body));
        loop_vars(body, &mut own);
        let env: BTreeMap<String, Ty> = own.into_iter().filter_map(|n| self.ctx.vars.get(&n).map(|t| (n, t.clone()))).collect();
        self.ctx.body_envs.insert(key.to_string(), env);
    }

    fn lower_derive(&mut self, name: &str, fresh: f64, body: &[S]) -> Result<Derive, String> {
        // a derivation's locals are its own, as a function's are: typed in a
        // body of their own, first assignments new bindings (an empty list's
        // type is its own pushes', not another body's local of that name)
        let scope = self.ctx.enter_body(name);
        self.at = Where::Derive;
        self.current_fn = name.to_string();
        self.ctx.registry.enter(&self.current_fn);
        self.v.local_types.clear();
        self.v.stored.clear();
        self.provisional(Pass::Settle, Some(HashSet::new()), body);
        self.record_env(name, &[], body);
        self.scopes.push(HashSet::new());
        // (a function sees no top-level variable, nor what grows there)
        let enclosing = std::mem::take(&mut self.grown);
        let block = self.lower_block(body);
        self.grown = enclosing;
        let mut block = block?;
        self.scopes.pop();
        self.ctx.leave_body(scope);
        place_moves_in(&mut block, &HashSet::new(), Some(&HashSet::new()));
        let ty = self.ctx.vars.get(name).cloned().unwrap_or(Ty::Unknown);
        let mut reads = ReadSet::default();
        let mut names = BTreeSet::new();
        let mut clock = false;
        reads_of_stmts(body, &mut names, &mut clock);
        for n in names {
            if self.ctx.derives.contains(&n) && n != name {
                reads.derives.push(n);
            } else if self.ctx.facts.contains(&n) {
                reads.facts.push(n);
            }
        }
        reads.clock = clock;
        Ok(Derive { name: name.to_string(), ty, body: block, reads, fresh, spec: Default::default(), effects: vec![] })
    }

    fn lower_test(&mut self, name: Option<String>, params: &[String], body: &[S]) -> Result<Test, String> {
        let saved = self.ctx.vars.clone();
        for p in params {
            self.ctx.vars.insert(p.clone(), Ty::Unknown);
        }
        self.at = Where::Test;
        self.current_fn = name.clone().unwrap_or_default();
        self.ctx.registry.enter(&self.current_fn);
        let own: HashSet<String> = params.iter().cloned().collect();
        // its inputs are typed by their use, for the runner to generate: a
        // parameter handed to a function takes that parameter's type, one in
        // arithmetic with a number is a number — read twice, as an input
        // typed by the first reading can type another (`dt` in `a - dt`)
        for _ in 0..2 {
            if params.iter().all(|p| !matches!(self.ctx.vars.get(p), Some(Ty::Unknown) | None)) {
                break;
            }
            let Ok(lowered) = self.provisional_block(Pass::Settle, Some(own.clone()), body).1 else {
                break;
            };
            for p in params {
                if matches!(self.ctx.vars.get(p), Some(Ty::Unknown) | None) {
                    if let Some(t) = self.input_type_by_use(&lowered, p) {
                        self.ctx.vars.insert(p.clone(), t);
                    }
                }
            }
        }
        self.provisional(Pass::Settle, Some(own.clone()), body);
        self.record_env(&format!("test {}", name.clone().unwrap_or_default()), params, body);
        self.scopes.push(own.clone());
        let block = self.lower_test_body(body);
        let (mut block, truths) = block?;
        self.scopes.pop();
        let typed: Vec<(String, Ty)> = params.iter().map(|p| (p.clone(), self.ctx.vars.get(p).cloned().unwrap_or(Ty::Unknown))).collect();
        self.ctx.vars = saved;
        place_moves_in(&mut block, &HashSet::new(), Some(&own));
        Ok(Test { name, params: typed, body: block, truths })
    }

    /// A test's body, each bare line that is a truth (a bool, or a bool
    /// that can fail) lowered as an assertion, which reports its operands
    /// when it fails; and how many there are.
    /// Whether an expression is a truth by its syntax: a comparison, a
    /// conjunction or disjunction of expressions, or a negation.
    fn truth_syntax(e: &A) -> bool {
        match e {
            A::BinOp { op, .. } => matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or),
            A::UnaryOp { op, .. } => matches!(op, crate::ast::UnaryOp::Not),
            _ => false,
        }
    }

    /// A bare truth under a precondition — `if s gt 0.0` and, inside it,
    /// the comparison — is a truth too (spec: preconditions are plain
    /// ifs): each such line in the body's nested blocks is written as an
    /// assertion before the lowering sees it. Only the top level had been
    /// read as truths: a law whose two truths stood under its precondition
    /// had asserted nothing, and passed.
    fn truths_under_guards(block: &[S], nested: bool, truths: &mut usize) -> Vec<S> {
        block
            .iter()
            .map(|s| match s {
                S::ExprStmt(e) if nested && Self::truth_syntax(e) => {
                    *truths += 1;
                    S::ExprStmt(A::Call { callee: Box::new(A::Ident("assert".to_string())), args: vec![e.clone()] })
                }
                S::If { condition, then_body, else_body } => S::If {
                    condition: condition.clone(),
                    then_body: Self::truths_under_guards(then_body, true, truths),
                    else_body: else_body.as_ref().map(|b| Self::truths_under_guards(b, true, truths)),
                },
                S::For { var, iter, body } => S::For { var: var.clone(), iter: iter.clone(), body: Self::truths_under_guards(body, true, truths) },
                S::While { condition, body } => S::While { condition: condition.clone(), body: Self::truths_under_guards(body, true, truths) },
                other => other.clone(),
            })
            .collect()
    }

    /// A bare call answering a bool under a guard or in a loop of the body
    /// is a truth too, as it is at the top level: asserted by its type,
    /// once the lowering knows it.
    fn assert_bools_under(block: &mut [Stmt], truths: &mut usize) {
        for s in block.iter_mut() {
            match s {
                Stmt::If { then, els, .. } => {
                    Self::assert_bools_in(then, truths);
                    if let Some(e) = els {
                        Self::assert_bools_in(e, truths);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => Self::assert_bools_in(body, truths),
                _ => {}
            }
        }
    }

    fn assert_bools_in(block: &mut [Stmt], truths: &mut usize) {
        for s in block.iter_mut() {
            if let Stmt::Do(x) = s {
                let ty = x.ty();
                if matches!(ty, Ty::Bool) || matches!(&ty, Ty::Result(inner) if **inner == Ty::Bool) {
                    let taken = std::mem::replace(x, Expr::Const(Const::Int(0), Ty::Int));
                    *s = Stmt::Do(Expr::Call { target: Target::Runtime("assert".to_string()), args: vec![taken], ty: Ty::Unit, propagates: false });
                    *truths += 1;
                }
            }
        }
        Self::assert_bools_under(block, truths);
    }

    fn lower_test_body(&mut self, body: &[S]) -> Result<(Block, usize), String> {
        let mut grown = HashSet::new();
        grown_by_push(body, &mut grown);
        self.lists_grown(&grown);
        let enclosing = std::mem::replace(&mut self.grown, vec![grown]);
        let out = self.lower_test_stmts(body);
        self.grown = enclosing;
        out
    }

    fn lower_test_stmts(&mut self, body: &[S]) -> Result<(Block, usize), String> {
        let mut out = Vec::new();
        let mut truths = 0;
        let body = Self::truths_under_guards(body, false, &mut truths);
        for s in &body {
            let mut lowered = self.lower_stmt(s)?;
            self.flush_pre_into(&mut out);
            // a bare line is a truth by what it is written as — a
            // comparison, a conjunction, a negation — or, a call, by the
            // bool it answers (its operands' types alone had decided, and a
            // comparison over an input not yet typed was no truth)
            let written_as_truth = matches!(s, S::ExprStmt(e) if Self::truth_syntax(e));
            if let (S::ExprStmt(_), [Stmt::Do(x)]) = (s, lowered.as_slice()) {
                if written_as_truth || matches!(x.ty(), Ty::Bool) || matches!(x.ty(), Ty::Result(inner) if *inner == Ty::Bool) {
                    let Some(Stmt::Do(x)) = lowered.pop() else { unreachable!() };
                    truths += 1;
                    lowered.push(Stmt::Do(Expr::Call { target: Target::Runtime("assert".to_string()), args: vec![x], ty: Ty::Unit, propagates: false }));
                }
            }
            Self::assert_bools_under(&mut lowered, &mut truths);
            out.extend(lowered);
        }
        Ok((out, truths))
    }

    /// The type a test's input takes from its use in the lowered body: a
    /// function's parameter it is handed to, or the number on the other
    /// side of arithmetic it is in; the first use that says.
    fn input_type_by_use(&self, block: &[Stmt], param: &str) -> Option<Ty> {
        fn read_through(e: &Expr) -> &Expr {
            match e {
                Expr::Convert { expr, .. } => read_through(expr),
                other => other,
            }
        }
        let is_param = |e: &Expr| matches!(read_through(e), Expr::Local(v, _) | Expr::Move(v, _) if v == param);
        // what a function's parameter is handed where the test calls it with
        // a value of known type (`half[4]` beside `half[n]`): a function only
        // the test calls has no other signature
        let handed = |base: &str, i: usize| -> Option<Ty> {
            let mut seen: Option<Ty> = None;
            crate::ir::visit_block(block, &mut |e| {
                if let (None, Expr::Call { target: Target::User(g), args, .. }) = (&seen, e) {
                    if crate::ir::copy_base(g) == base {
                        if let Some(a) = args.get(i).filter(|a| !is_param(a)) {
                            let t = as_inferred(&read_through(a).ty());
                            if !matches!(t, Ty::Unknown) {
                                seen = Some(t);
                            }
                        }
                    }
                }
            });
            seen
        };
        let mut found: Option<Ty> = None;
        crate::ir::visit_block(block, &mut |e| {
            if found.is_some() {
                return;
            }
            match e {
                Expr::Call { target: Target::User(f), args, .. } => {
                    let base = crate::ir::copy_base(f);
                    for (i, a) in args.iter().enumerate() {
                        if is_param(a) {
                            let sig = self.ctx.functions.get(f.as_str()).or_else(|| self.ctx.functions.get(base));
                            let declared = sig.and_then(|s| s.params.get(i)).map(|(_, t)| t.clone()).filter(|t| !matches!(t, Ty::Unknown));
                            if let Some(t) = declared.or_else(|| handed(base, i)) {
                                found = Some(t);
                                return;
                            }
                        }
                    }
                }
                Expr::Binary { op, l, r, .. } if matches!(op.as_str(), "add" | "sub" | "mul" | "div" | "pow") => {
                    let other = if is_param(l) { Some(r) } else if is_param(r) { Some(l) } else { None };
                    if let Some(t) = other.map(|o| as_inferred(&o.ty())) {
                        if matches!(t, Ty::Int | Ty::Float) {
                            found = Some(t);
                        }
                    }
                }
                _ => {}
            }
        });
        found
    }

    // ── statements ──────────────────────────────────────────────────

    /// A variable grown by `push` that inference settled as a tensor of
    /// whole numbers of one axis (its literal's kind) is a list of ints:
    /// its literal is lowered as one (`S::Assign`), and its pushes and
    /// reads follow the variable's kind.
    fn lists_grown(&mut self, grown: &HashSet<String>) {
        for name in grown {
            // (through a failure: a push of a value that can fail had typed
            // the variable as one, the literal was lowered as a list and the
            // variable stayed a tensor — the lowered program disagreed with
            // itself; the push hands its failure on, and the variable holds
            // the list; Rosetta Code, colorful numbers)
            if let Some(t) = self.ctx.vars.get_mut(name)
                && matches!(t.payload(), Ty::Tensor(e, d) if **e == Ty::Int && d.as_ref().is_none_or(|d| d.len() <= 1))
            {
                *t = Ty::List(Box::new(Ty::Int));
            }
            // (and a matrix of them written down and grown is a list of
            // lists of ints: grown as a matrix, its rows of ints had been
            // pushed as floats — the lowered program disagreed with itself;
            // Rosetta Code, Lah numbers)
            if let Some(t) = self.ctx.vars.get_mut(name)
                && matches!(t.payload(), Ty::Tensor(e, Some(d)) if **e == Ty::Int && d.len() == 2)
            {
                *t = Ty::List(Box::new(Ty::List(Box::new(Ty::Int))));
            }
        }
    }

    fn lower_block(&mut self, stmts: &[S]) -> Result<Block, String> {
        let mut grown = HashSet::new();
        grown_by_push(stmts, &mut grown);
        self.lists_grown(&grown);
        self.grown.push(grown);
        let out = self.lower_stmts(stmts);
        self.grown.pop();
        out
    }

    fn lower_stmts(&mut self, stmts: &[S]) -> Result<Block, String> {
        let mut out = Vec::new();
        for s in stmts {
            let lowered = self.lower_stmt(s)?;
            self.flush_pre_into(&mut out);
            out.extend(lowered);
        }
        Ok(out)
    }

    fn flush_pre_into(&mut self, out: &mut Block) {
        if !self.pre.is_empty() {
            out.extend(std::mem::take(&mut self.pre));
        }
    }

    fn declared(&self, name: &str) -> bool {
        // a function sees its parameters and its own locals: the program's
        // top-level names are invisible inside it (spec, No Ambient
        // Mutability), so an assignment there binds a new local
        let own_only = matches!(self.at, Where::In(_) | Where::Derive);
        self.scopes.iter().enumerate().any(|(i, s)| !(own_only && i == 0) && s.contains(name))
    }

    /// Whether a name is a function the function being lowered can call:
    /// one of the program's — or, inside a standard module's function
    /// written in Crow and inlined, one of the module's own (prefixed by
    /// the inliner; its calls of a row stay bare and are the row's). The
    /// program's functions are invisible there: a local of the module's
    /// named like one (`lines`) is its local, indexed where it is written
    /// `lines[0]`, and a program is free to name its functions as it likes.
    /// A list a function is handed empty takes the kind of what the
    /// function pushes onto it: `collected[t, []]` with `push[acc, t.v]`
    /// in the body makes `acc` a list of numbers for every caller (the
    /// compiled program had typed the parameter a list of text, and did
    /// not build). Noted in the call scan, where a local's assignment
    /// settles its type too, and in the settling pass, which types a
    /// function's locals (a list a function started empty and pushed onto
    /// had stayed a list of nothing there, and the compiled program did
    /// not build; the program's top level was refined in the scan).
    fn refine_pushed(&mut self, row: &crate::table::Row, lowered: &[Expr]) {
        if self.pass == Pass::Final || row.id != "core.push" {
            return;
        }
        let (Some(Expr::Local(holder, _)), Some(item)) = (lowered.first(), lowered.get(1)) else { return };
        let held = full(&lowered[0]);
        let open = matches!(held, Ty::Unknown) || matches!(&held, Ty::List(inner) if matches!(inner.as_ref(), Ty::Unknown));
        let item_ty = read(item);
        if open && !matches!(item_ty, Ty::Unknown | Ty::Unit) {
            let refined = Ty::List(Box::new(item_ty));
            if self.pass == Pass::Scan {
                self.ctx.note_scanned_assign(holder, refined.clone());
            } else {
                self.ctx.vars.insert(holder.clone(), refined.clone());
            }
            self.rebind(holder, &refined);
        }
    }

    fn calls_fn(&self, name: &str) -> bool {
        self.ctx.functions.contains_key(name) && (crate::modules::is_crow_fn(name) || !crate::modules::is_crow_fn(&self.current_fn) || self.ctx.applying.iter().any(|f| f == name))
    }

    /// A write at a path — a field, an item, a key — of a local, or of a
    /// fact a handler moves: a fact is written only by `store` (the IR's one
    /// write to state), so its value is read into a temporary, written
    /// there, and stored back (a record fact's field had been written as a
    /// local's: the interpreter stopped, "undefined variable", and the
    /// compiled page changed it — crow check --self's state family).
    fn path_write(&mut self, var: String, path: Vec<PathStep>, expr: Expr) -> Vec<Stmt> {
        let (mut out, expr, taken) = self.grown_in_place(&var, &path, expr);
        if !(self.is_fact(&var) && self.sees_facts()) {
            out.push(Stmt::SetPath { var, path, expr, taken });
            return out;
        }
        let ty = self.ctx.vars.get(&var).cloned().unwrap_or(Ty::Unknown);
        let t = self.fresh();
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(t.clone());
        }
        out.extend([
            Stmt::Let { var: t.clone(), ty: ty.clone(), expr: Expr::Load(var.clone(), ty.clone()) },
            Stmt::SetPath { var: t.clone(), path, expr, taken },
            Stmt::Store { fact: var, expr: Expr::Local(t, ty) },
        ]);
        out
    }

    /// A place that grows where it lies: `xs[i] = push[xs[i], v]`, `m[k] =
    /// push[m[k], v]`, `r.items = push[r.items, v]` — a write at a path
    /// whose value is a call of an in-place row given that very place
    /// first (spec, Arrays). Marked `taken` for both backends
    /// (ir::Stmt::SetPath): the place's value is taken out, handed to the
    /// row moved and written back, where reading it had copied the list
    /// at every push — 200,000 pushes under eight keys took 650 ms
    /// compiled and 14 s interpreted, against 1 ms (found by the postings
    /// builder of magpie.file, 100× the kernel's time). The place holds a
    /// list, a map, text or bytes (a tensor has its own rows, `__set_at`);
    /// the path's positions are quiet, so computing each once is computing
    /// it as written; any other argument reading the variable is computed
    /// first, into a temporary, as `taken_in_place` does for a whole
    /// variable.
    fn grown_in_place(&mut self, var: &str, path: &[PathStep], expr: Expr) -> (Vec<Stmt>, Expr, bool) {
        let found = match &expr {
            Expr::Call { target: Target::Row(row), args, ty, propagates }
                if row.grows_in_place()
                    && args.first().is_some_and(|first| {
                        let place = first.ty();
                        !place.is_result()
                            && matches!(place.payload(), Ty::List(_) | Ty::Map(..) | Ty::Str | Ty::Bytes | Ty::Tensor(..))
                            && reads_place(first, var, path)
                    })
                    && path.iter().all(|p| match p {
                        PathStep::Index(i) => crate::ir::quiet(i),
                        PathStep::Field(_) => true,
                    }) =>
            {
                Some((*row, args.clone(), ty.clone(), *propagates))
            }
            _ => None,
        };
        let Some((row, args, ty, propagates)) = found else { return (Vec::new(), expr, false) };
        // (every other argument a plain read where the write runs: one that
        // computes — or reads the variable — is computed first, into a
        // temporary, so the place is taken only once nothing is left to run
        // and nothing reads it while it is out)
        let reads_var = |e: &Expr| {
            let mut found = false;
            crate::ir::visit_expr(e, &mut |x| found |= matches!(x, Expr::Local(n, _) | Expr::Move(n, _) if n == var));
            found
        };
        let mut out = Vec::new();
        let mut kept = vec![args[0].clone()];
        for a in &args[1..] {
            if crate::ir::quiet(a) && !reads_var(a) {
                kept.push(a.clone());
                continue;
            }
            let t = self.fresh();
            let at = a.ty();
            out.push(Stmt::Let { var: t.clone(), ty: at.clone(), expr: a.clone() });
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(t.clone());
            }
            kept.push(Expr::Local(t, at));
        }
        (out, Expr::Call { target: Target::Row(row), args: kept, ty, propagates }, true)
    }

    /// The frame a path holds — a variable's (`xs`), or a record's field's
    /// (`b.f`), each through its item where `item` says — made open: its
    /// columns stay known, and one it cannot name may be there (a column
    /// written through the path, read as the program runs).
    fn open_held_frame(&mut self, holder: &A, field: Option<&String>, item: bool) {
        fn open(t: &mut Ty, item: bool) {
            match t {
                Ty::Result(inner) => open(inner, item),
                Ty::List(inner) | Ty::Map(_, inner) if item => open(inner, false),
                Ty::Frame(Some(cols)) if !item && !cols.open => {
                    *t = Ty::Frame(Some(std::sync::Arc::new(crate::type_infer::Cols { cols: cols.cols.clone(), open: true })));
                }
                _ => {}
            }
        }
        match (holder, field) {
            (A::Ident(v), None) => {
                if let Some(mut t) = self.ctx.vars.get(v).cloned() {
                    open(&mut t, item);
                    self.ctx.vars.insert(v.clone(), t);
                }
            }
            (o, Some(f)) => {
                if let Ty::Record(rec) = self.target_type(o).payload().clone() {
                    if let Some(sig) = self.ctx.structs.get_mut(&rec) {
                        for (name, ty) in sig.fields.iter_mut() {
                            if name == f {
                                open(ty, item);
                            }
                        }
                    }
                }
            }
            // (a collection held in a record's field: the field's items)
            (A::FieldAccess { object: o, field: f }, None) => {
                if let Ty::Record(rec) = self.target_type(o).payload().clone() {
                    if let Some(sig) = self.ctx.structs.get_mut(&rec) {
                        for (name, ty) in sig.fields.iter_mut() {
                            if name == f {
                                open(ty, item);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn is_fact(&self, name: &str) -> bool {
        // (a standard module's function, inlined, has locals of its own:
        // one named like a fact of the program is still its local)
        self.holds_state && self.ctx.facts.contains(name) && !self.scopes.iter().skip(1).any(|s| s.contains(name)) && !crate::modules::is_crow_fn(&self.current_fn)
    }

    /// The type a variable takes from a value: a fact a ui program declares
    /// at its top level holds no error, so one declared from a value that
    /// can fail holds the payload (the failure stops the program where the
    /// facts are set).
    fn held_by_fact(&self, name: &str, ty: Ty) -> Ty {
        match ty {
            Ty::Result(inner) if self.holds_state && self.at == Where::Top && self.ctx.facts.contains(name) => *inner,
            other => other,
        }
    }

    fn sees_facts(&self) -> bool {
        matches!(self.at, Where::Top | Where::Derive | Where::Test | Where::In(FnKind::View) | Where::In(FnKind::Handler) | Where::In(FnKind::Helper))
    }

    fn lower_stmt(&mut self, s: &S) -> Result<Vec<Stmt>, String> {
        Ok(match s {
            S::Line(n) => {
                self.v.line = *n;
                vec![Stmt::Line(*n)]
            }
            S::Assign { name, value } => {
                // (a standard module's function, inlined, has locals of its
                // own: one named like a derivation of the program's is still
                // its local — the ClickHouse driver's `found`, which finch
                // derives)
                if self.ctx.derives.contains(name) && !crate::modules::is_crow_fn(&self.current_fn) {
                    self.refuse(format!("'{}' is derived — it follows what it reads; change those facts instead of assigning it", name));
                }
                if self.binds_anew(name) {
                    self.check_reserved(name, "variable");
                }
                self.check_not_a_function(name, "variable");
                self.check_top_write(name);
                let refused_before = self.v.errors.len();
                let expr = self.lower_expr(value)?;
                // a list of whole numbers written down for a variable the
                // program grows with `push` is a list of ints, which keeps
                // every digit: a tensor holds floats, and `fib = [0, 1]`
                // grown by its sums had rounded past 2^53 interpreted and
                // overflowed compiled (Rosetta Code, Abundant odd numbers)
                let expr = match expr {
                    Expr::ListLit(items, Ty::Tensor(e, d))
                        if matches!(value, A::Array(_)) && *e == Ty::Int && d.as_ref().is_none_or(|d| d.len() <= 1) && self.grown.iter().any(|g| g.contains(name)) =>
                    {
                        Expr::ListLit(items, Ty::List(Box::new(Ty::Int)))
                    }
                    // (a matrix of them, row by row — a row held in a
                    // variable converted where it is written: `decks =
                    // [cards]` grown by a pushed range had been a matrix of
                    // floats beside rows of ints; Rosetta Code, Topswops)
                    Expr::ListLit(rows, Ty::Tensor(e, Some(d)))
                        if matches!(value, A::Array(_))
                            && *e == Ty::Int
                            && d.len() == 2
                            && self.grown.iter().any(|g| g.contains(name))
                            && rows.iter().all(|r| matches!(r, Expr::ListLit(..)) || crate::type_infer::settle::is_ints(&r.ty())) =>
                    {
                        let rows = rows
                            .into_iter()
                            .map(|r| match r {
                                Expr::ListLit(items, _) => Expr::ListLit(items, Ty::List(Box::new(Ty::Int))),
                                other => listed(other, Ty::Int),
                            })
                            .collect();
                        Expr::ListLit(rows, Ty::List(Box::new(Ty::List(Box::new(Ty::Int)))))
                    }
                    other => other,
                };
                // a list of plain items given a literal whose items can fail
                // holds each item that is good, and the assignment refuses
                // one that failed, as a variable whose first value could not
                // fail refuses a later failure (spec, §1): the reassignment
                // had been refused as a change of kind — `parts = ['b`x`']`
                // of a text read from the environment; Rosetta Code,
                // Terminal control/Coloured text
                // (and a vector given numbers that can fail, as `[y, 3.0]`)
                let lands = |held: &Ty, item: &Ty| match held {
                    Ty::List(h) => !h.is_result() && h.compatible(item.payload()),
                    Ty::Tensor(e, d) => d.as_ref().is_none_or(|d| d.len() == 1) && (e.as_ref() == item.payload() || matches!((e.as_ref(), item.payload()), (Ty::Float, Ty::Int))),
                    _ => false,
                };
                let expr = match (self.declared(name).then(|| self.ctx.vars.get(name).cloned()).flatten(), expr) {
                    (Some(held), Expr::ListLit(items, Ty::List(item))) if item.is_result() && lands(&held, &item) => {
                        let landed_ty = match &held {
                            Ty::Tensor(e, _) => Ty::Tensor(e.clone(), Some(vec![Some(items.len())])),
                            _ => Ty::List(Box::new(item.payload().clone())),
                        };
                        let mut landed = Vec::with_capacity(items.len());
                        for x in items {
                            landed.push(if x.ty().is_result() {
                                self.decided(x, Decision::Assign(name.clone()))
                            } else if crate::ir::quiet(&x) {
                                x
                            } else {
                                // (in its place among the items hoisted)
                                let t = self.fresh();
                                if let Some(scope) = self.scopes.last_mut() {
                                    scope.insert(t.clone());
                                }
                                let ty = x.ty();
                                self.pre.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: x });
                                Expr::Local(t, ty)
                            });
                        }
                        Expr::ListLit(landed, landed_ty)
                    }
                    (_, e) => e,
                };
                // (a value whose making was refused: what the program reads
                // of it is that mistake's echo, not judged again)
                if self.v.errors.len() > refused_before {
                    self.v.contested.insert(name.clone());
                }
                // a loaded frame held to the columns the program's reads
                // say, before any pass types the variable by it (uses.rs)
                let expr = self.held_to_shape(name, expr);
                // a parsed value assigned to a variable of a kind is read as
                // that kind (a number as a float: the variable widens, as one
                // assigned a float does)
                let expr = match self.ctx.vars.get(name).map(|t| t.payload().clone()) {
                    Some(kind @ (Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Moment)) if self.declared(name) || self.is_fact(name) => {
                        let kind = if matches!(kind, Ty::Int) { Ty::Float } else { kind };
                        self.read_json(expr, &kind, &format!("assigned to {}", name))
                    }
                    _ => expr,
                };
                if self.pass == Pass::Final {
                    // (a call that answers nothing gives the variable no
                    // value: refused once, and what the variable was is not
                    // judged against it)
                    if self.refuse_nothing_held(&format!("'{}'", crate::lower::shown(name)), &expr) {
                        self.v.contested.insert(name.clone());
                    } else if self.declared(name) || self.in_fn().is_some() || self.is_fact(name) {
                        // (a top-level variable's first assignment is judged
                        // against nothing: what an earlier round guessed of it
                        // — a mask of a function's answer read as one number
                        // before the function was typed — had refused the one
                        // assignment it has; Rosetta Code, Statistics/Normal
                        // distribution)
                        self.check_assign(name, &self.checked(&expr));
                    }
                }
                if self.pass == Pass::Scan {
                    // the call scan settles the variable once its value's
                    // calls are read, with the value's type
                    let held = self.held_by_fact(name, kept(&expr));
                    self.ctx.note_scanned_assign(name, held.clone());
                    self.ctx.note_scanned_value(name, value);
                    self.rebind(name, &held);
                }
                let stmt = self.assign(name, expr);
                if matches!(stmt, Stmt::Store { .. }) {
                    self.v.stored.insert(name.clone());
                }
                if self.pass == Pass::Final {
                    return Ok(self.taken_in_place(stmt));
                }
                vec![stmt]
            }
            S::MultiAssign { names, values } if self.pass == Pass::Settle => self.settled_multi(names, values)?,
            S::MultiAssign { names, values } if self.pass == Pass::Scan => self.scanned_multi(names, values)?,
            S::MultiAssign { names, values } => {
                // each new name checked against the names taken, after the
                // values it is given
                let new: Vec<String> = names.iter().filter(|n| self.binds_anew(n)).cloned().collect();
                for n in names {
                    self.check_top_write(n);
                }
                let out = self.multi_assign(names, values)?;
                for st in &out {
                    if let Stmt::Store { fact, .. } = st {
                        self.v.stored.insert(fact.clone());
                    }
                }
                for n in &new {
                    self.check_reserved(n, "variable");
                }
                for n in names {
                    self.check_not_a_function(n, "variable");
                }
                out
            }
            S::FieldAssign { object, field, value } => {
                // a frame's column written where the frame lies — `df.name =
                // values`, `df.`name` = values` — is the frame assigned anew
                // with the column, through the column write's row (found by
                // its property, never by name): the statement is rewritten as
                // the assignment of what holds the frame — a variable, a
                // field, an item — and lowered as one, so the frame's type
                // gains the column at this line as a reassignment's does, a
                // fact's write is a store, and the value's own checks are the
                // assignment's (spec, Frames). A computed name is a frame's
                // alone: a record's fields are fixed before the program runs.
                let computed = field.contains('`');
                let held = self.target_type(object);
                // (a field the record does not declare is no place to write:
                // refused as a read of it is — interpreted it had stopped as
                // it ran, and the compiled program had not built; Rosetta
                // Code, Sealed classes and methods)
                if !computed && self.pass == Pass::Final {
                    match held.payload() {
                        t @ Ty::Record(_) => self.check_field(&t.clone(), field),
                        Ty::OptRecord(n) => self.check_field(&Ty::Record(n.clone()), field),
                        _ => {}
                    }
                }
                if computed && self.pass == Pass::Final && !matches!(held.payload(), Ty::Frame(_) | Ty::Unknown) {
                    self.refuse(format!(
                        "`.{}` names a frame's column by a computed name, and this is {}: a record's fields are fixed before the program runs, and a map's key is written as `m[key] = …`",
                        field,
                        crate::table::Arg::of_type(held.payload()).words()
                    ));
                    return Ok(Vec::new());
                }
                if computed || matches!(held.payload(), Ty::Frame(_)) {
                    let write = crate::table::column_write().name;
                    let call = A::Call {
                        callee: Box::new(A::Ident(write.to_string())),
                        args: vec![object.clone(), A::Str(field.clone()), value.clone()],
                    };
                    let rewritten = match object {
                        A::Ident(v) => S::Assign { name: v.clone(), value: call },
                        A::FieldAccess { object: o, field: f } => S::FieldAssign { object: (**o).clone(), field: f.clone(), value: call },
                        A::Call { callee, args } if args.len() == 1 => S::IndexAssign { object: (**callee).clone(), index: args[0].clone(), value: call },
                        _ => {
                            if self.pass == Pass::Final {
                                self.refuse(format!("a column is written on a frame a variable, a field or an item holds: hold the frame in one, then `.{} = …` it", field));
                            }
                            return Ok(Vec::new());
                        }
                    };
                    let out = self.lower_stmt(&rewritten)?;
                    // a column written through a record's field or a list's
                    // item: the frame's type there is every record's or every
                    // item's, and cannot carry the column, so it is open from
                    // here on and the column is read as the program runs
                    // (spec, Frames: a column only some add may be there)
                    match object {
                        A::FieldAccess { object: o, field: f } => self.open_held_frame(o, Some(f), false),
                        A::Call { callee, .. } => self.open_held_frame(callee, None, true),
                        _ => {}
                    }
                    return Ok(out);
                }
                // (a list, a tensor, text or a number has no field to write:
                // `xs.k = …` had stopped the interpreter as it ran, and the
                // compiled program had not built — refused as a read of the
                // field is, in its words)
                // (a value whose making was refused has no fields to judge
                // a write by: that mistake is told once, as its reads are)
                let contested = target_root(object).is_some_and(|root| self.v.contested.contains(&root));
                if self.pass == Pass::Final && !contested && !matches!(held.payload(), Ty::Record(_) | Ty::Map(..) | Ty::Json | Ty::Unknown) {
                    self.check_field(&held, field);
                }
                // the target is checked before its value, then lowered after
                self.check_target(object);
                if let Some(root) = target_root(object) {
                    self.check_top_write(&root);
                }
                let expr = self.lower_expr(value)?;
                if self.pass == Pass::Final && self.refuse_nothing_held(&format!("the field '{}'", field), &expr) {
                    return Ok(Vec::new());
                }
                if self.pass != Pass::Final {
                    self.settled_field(object, field, &expr);
                }
                // a list of numbers written down for a field that holds a
                // list is that list: a literal's tensor written as it was
                // had stopped interpreted ("expected a list") and not built
                // compiled (Rosetta Code, Blossom algorithm)
                let field_ty = match self.target_type(object).payload() {
                    Ty::Record(r) | Ty::OptRecord(r) => self.ctx.structs.get(r).and_then(|sig| sig.fields.iter().find(|(n, _)| n == field).map(|(_, t)| t.clone())),
                    _ => None,
                };
                let expr = match (field_ty, expr.ty()) {
                    (Some(Ty::List(item)), Ty::Tensor(e, d)) if matches!(*item, Ty::Int | Ty::Float) && (*e == *item || *item == Ty::Float) && d.as_ref().is_none_or(|d| d.len() <= 1) => {
                        listed(expr, *item)
                    }
                    _ => expr,
                };
                let (var, mut path, _) = self.muted_target(object)?;
                path.push(PathStep::Field(field.clone()));
                self.forget_shape(&var);
                self.path_write(var, path, expr)
            }
            S::IndexAssign { object, index, value } => {
                self.check_target(object);
                if let Some(root) = target_root(object) {
                    self.check_top_write(&root);
                }
                let expr = self.lower_expr(value)?;
                if self.pass == Pass::Final {
                    let place = match target_root(object) {
                        Some(root) => format!("an item of '{}'", crate::lower::shown(&root)),
                        None => "an item".to_string(),
                    };
                    if self.refuse_nothing_held(&place, &expr) {
                        return Ok(Vec::new());
                    }
                }
                let idx = self.lower_expr(index)?;
                let idx = self.decided_index(idx);
                // (in the scan too, as a field's write is: the top level is
                // scanned before it settles, and a call scanned after `m['a']
                // = 1` had typed its parameter by the open map — `Map(?)`,
                // which did not build compiled; Rosetta Code, Erdős–Selfridge)
                if self.pass != Pass::Final {
                    self.settled_entry(object, &expr, &idx);
                }
                let (var, mut path, held) = self.muted_target(object)?;
                // a map is written at a key of its keys' kind (text or an int),
                // a list, text or a tensor at an int: `m[2.5] = 3` into a map
                // of text keys had stopped interpreted and written the key
                // '2.5' compiled (crow check --self)
                if self.pass == Pass::Final {
                    let given = idx.ty().payload().clone();
                    match held.payload() {
                        // a parsed value and a frame are read, never written
                        // where they lie (the interpreter had stopped,
                        // "cannot index-assign into map", and the compiled
                        // program had not built — crow check --self)
                        Ty::Json => {
                            self.refuse(format!(
                                "'{}' is a parsed value, which the program reads, never writes: make a map or a list of what it holds and change that",
                                crate::lower::shown(&var)
                            ));
                        }
                        Ty::Frame(_) => {
                            self.refuse(format!(
                                "'{0}' is a frame, whose cells are not written by position: a column is written whole, {0}.name = values",
                                crate::lower::shown(&var)
                            ));
                        }
                        // (a column held in a variable is a frame's column:
                        // made anew, as the frame's are — the interpreter had
                        // written the cell, and the compiled program had not
                        // built)
                        Ty::Column(_) => {
                            self.refuse(format!(
                                "'{}' is a frame's column, whose cells are not written by position: a column is written whole, df.name = values",
                                crate::lower::shown(&var)
                            ));
                        }
                        // text is made, never written where it lies (the
                        // interpreter had stopped, "cannot index-assign into
                        // string", and the compiled program had not built)
                        Ty::Str | Ty::Markup => {
                            self.refuse(format!(
                                "'{}' is text, which is made anew, not written by position: concat[[slice[s, 0, i], 'x', slice[s, i + 1, len[s]]]]",
                                crate::lower::shown(&var)
                            ));
                        }
                        Ty::Map(key, _) => {
                            if !key_like(&given) {
                                self.refuse(format!("a map's key is text or an int, and this is {}", crate::table::Arg::of_type(&given).words()));
                            } else if key.unify(crate::type_infer::key_kind_of(&given)).is_none() {
                                self.refuse(crate::table::wrong_key(*key, &given));
                            }
                        }
                        Ty::List(_) | Ty::Tensor(..) if !position_like(&given) => {
                            self.refuse(format!("a position in '{}' is an int, and this is {}", crate::lower::shown(&var), crate::table::Arg::of_type(&given).words()));
                        }
                        // a mask picks what is read (`xs[mask]`), never where
                        // a write lands: the interpreter had stopped ("array
                        // index must be an integer") and the compiled program
                        // had not built (crow check --self)
                        Ty::List(_) | Ty::Tensor(..) if matches!(given, Ty::List(_) | Ty::Tensor(..) | Ty::Column(_)) => {
                            let v = crate::lower::shown(&var);
                            self.refuse(format!(
                                "`{0}[…] = …` writes at one position, an int, and this is {1}: a mask picks what `{0}[mask]` reads, not where a write lands",
                                v,
                                crate::table::Arg::of_type(&given).words()
                            ));
                        }
                        _ => {}
                    }
                }
                // an item written is of the items' kind: a value that can fail
                // is decided where it is written, as one assigned to a variable
                // is; a parsed value is read as the items' kind; another kind
                // is refused where it is written (text into a list of numbers,
                // a value that can fail into one that cannot: the lowered
                // program had disagreed with itself — crow check --self)
                // (a vector's number is a number: `b[0] = true` had been cast
                // to a float compiled and not built)
                let items = match held.payload().clone() {
                    Ty::Tensor(_, dims) if dims.as_ref().is_none_or(|d| d.len() <= 1) => Some(Ty::Float),
                    Ty::List(item) | Ty::Map(_, item) => Some(*item),
                    _ => None,
                };
                // (a list of numbers written down for an item that is a list
                // is that list: a literal's tensor written as it was had been
                // an internal error; Rosetta Code, Christofides algorithm)
                // (and whole numbers written for a list of floats are its
                // floats: `k[0] = [2, 3]` beside `k[0][1] = 4.25`)
                let expr = match (&items, expr.ty()) {
                    (Some(Ty::List(item)), Ty::Tensor(e, d)) if matches!(item.as_ref(), Ty::Int | Ty::Float) && (*e == **item || **item == Ty::Float) && d.as_ref().is_none_or(|d| d.len() <= 1) => {
                        listed(expr, (**item).clone())
                    }
                    _ => expr,
                };
                // (a provisional pass observes the refusal too, as an
                // assignment's does (`held_by`): inside a function the
                // failure is handed back, which the function's answer must
                // say — the item's write had halted the whole program;
                // Rosetta Code, Compiler/AST interpreter)
                if self.pass != Pass::Final && expr.ty().is_result() && items.as_ref().is_some_and(|item| !item.has_unknown() && !item.is_result()) {
                    self.saw_refusal(&Decision::Assign(var.clone()));
                }
                let expr = match items {
                    Some(item) if self.pass == Pass::Final && !item.has_unknown() && !item.is_result() => {
                        let expr = if expr.ty().is_result() { self.decided(expr, Decision::Assign(var.clone())) } else { expr };
                        let given = expr.ty().payload().clone();
                        if matches!(given, Ty::Json) {
                            self.read_json(expr, &item, &format!("written to an item of '{}'", crate::lower::shown(&var)))
                        } else {
                            let whole = |t: &Ty| matches!(t, Ty::Int | Ty::Big);
                            if !item.compatible(&given) && !(whole(&item) && whole(&given)) {
                                self.refuse(format!(
                                    "an item of '{}' is {}, and the value written to it is {}",
                                    crate::lower::shown(&var),
                                    crate::table::Arg::of_type(&item).words(),
                                    crate::table::Arg::of_type(&given).words()
                                ));
                            }
                            expr
                        }
                    }
                    _ => expr,
                };
                // an int assigned to an item of a list of floats is a float,
                // where it is written — as one handed to a float parameter
                // is (the compiled program had not built, and the
                // interpreter's list had held the int)
                let expr = match (held.payload(), expr.ty()) {
                    (Ty::List(item) | Ty::Map(_, item), Ty::Int) if **item == Ty::Float => Expr::Convert { how: Convert::IntToFloat, expr: Box::new(expr), ty: Ty::Float },
                    _ => expr,
                };
                // a tensor written at positions (spec, Arrays): a number at one
                // position per axis — `m[i][j] = x` — and a part at fewer — a
                // matrix's row, `m[i] = row` — of exactly the shape the
                // positions leave, each written where the tensor lies by one
                // row in both modes (the chained index had failed in both,
                // differently, and a row had been refused outright). A number
                // at fewer positions than axes is no part: the write had landed
                // on the flat number `i` while `m[i]` read the row. A vector's
                // `v[i] = x` is the item write below. The tensor is the
                // variable (`m[i] = row`) or the first field or item along the
                // target that holds one (`g.cells[i] = row`, `ms[k][i] = row`:
                // both modes had failed there, differently).
                let var_ty = self.ctx.vars.get(&var).cloned().unwrap_or(Ty::Unknown);
                // (a tensor whose making was refused — an undefined
                // function's answer handed to a function copied per
                // signature, typed as the copy a call with nothing known
                // reaches — has no shape to judge a write by: that mistake
                // is told once, as its reads are)
                let lead = match &var_ty {
                    _ if self.v.contested.contains(&var) => None,
                    Ty::Tensor(..) => Some((0, Vec::new())),
                    _ if path.is_empty() => None,
                    _ => self.tensor_lead(object, &var_ty),
                };
                if let Some((lead, step_types)) = lead
                    && let tensor_ty = step_types.last().cloned().unwrap_or_else(|| var_ty.clone())
                    && let Ty::Tensor(_, dims) = &tensor_ty
                {
                    let positions = path[lead..].iter().all(|p| matches!(p, PathStep::Index(_)));
                    let given = path.len() - lead + 1;
                    let rank = dims.as_ref().map(|d| d.len());
                    let number = matches!(expr.ty().payload(), Ty::Int | Ty::Float);
                    let part = matches!(expr.ty().payload(), Ty::Tensor(..))
                        || matches!(expr.ty().payload(), Ty::List(item) if matches!(item.payload(), Ty::Int | Ty::Float | Ty::List(_) | Ty::Tensor(..)));
                    let v = format!(
                        "{}{}",
                        crate::lower::shown(&var),
                        path[..lead].iter().map(|p| match p {
                            PathStep::Field(f) => format!(".{f}"),
                            PathStep::Index(_) => "[…]".to_string(),
                        }).collect::<String>()
                    );
                    // (a matrix holds numbers: text or a bool written as its
                    // row had been cast compiled, and not built)
                    if positions && !number && !part && !matches!(expr.ty().payload(), Ty::Unknown | Ty::Json) {
                        self.refuse(format!(
                            "`{}[…] = …` writes numbers into a tensor, and this is {}",
                            v,
                            crate::table::Arg::of_type(expr.ty().payload()).words()
                        ));
                    }
                    if positions && number && given == 1 && rank.is_some_and(|r| r >= 2) {
                        self.refuse(format!(
                            "`{0}[…] = …` writes one number, and {0} is a matrix, whose `{0}[i]` is its row i: write a number with {0}[i][j] = …, or a row of its own length with {0}[i] = […]",
                            v
                        ));
                    }
                    if positions && part {
                        // (both shapes known: one that disagrees is refused
                        // before the program runs, in the words it stops with)
                        let value_dims: Option<Vec<Option<usize>>> = match expr.ty().payload() {
                            Ty::Tensor(_, d) => d.clone(),
                            _ => shapes::stacked_dims(&expr.ty()),
                        };
                        if let (Some(d), Some(vd)) = (dims.as_ref(), value_dims.as_ref()) {
                            let rest: Vec<Option<usize>> = d.iter().skip(given).cloned().collect();
                            let known = |xs: &[Option<usize>]| xs.iter().map(|x| *x).collect::<Option<Vec<usize>>>();
                            if let (Some(r), Some(g)) = (known(&rest), known(vd)) {
                                if given > d.len() || r != g {
                                    self.refuse(format!("`{}[…] = …`: {}", v, crow_runtime::core::part_disagrees(&r, &g)));
                                }
                            }
                        }
                    }
                    let write = if number && given >= 2 { Some("core.__set_at") } else if part { Some("core.__set_part") } else { None };
                    if let (true, Some(id), Some(row)) = (positions, write, write.and_then(|id| crate::table::ROWS.iter().find(|r| r.id == id))) {
                        let mut at: Vec<Expr> = path.split_off(lead).into_iter().filter_map(|p| match p {
                            PathStep::Index(i) => Some(i),
                            _ => None,
                        }).collect();
                        at.push(idx);
                        // (a value that can fail is decided where it is
                        // written, as one assigned to a variable is)
                        let value = if expr.ty().is_result() { self.decided(expr, Decision::Assign(var.clone())) } else { expr };
                        let value = if id == "core.__set_at" {
                            match value.ty().payload() {
                                Ty::Float => value,
                                _ => Expr::Convert { how: Convert::IntToFloat, expr: Box::new(value), ty: Ty::Float },
                            }
                        } else if matches!(value.ty().payload(), Ty::List(_)) {
                            let ty = Ty::Tensor(Box::new(Ty::Float), shapes::stacked_dims(&value.ty()));
                            Expr::Convert { how: Convert::ToTensor, expr: Box::new(value), ty }
                        } else {
                            value
                        };
                        let at = Expr::ListLit(at, Ty::List(Box::new(Ty::Int)));
                        if lead == 0 {
                            let call = Expr::Call {
                                target: Target::Row(row),
                                args: vec![Expr::Local(var.clone(), var_ty.clone()), at, value],
                                ty: var_ty.clone(),
                                propagates: false,
                            };
                            let stmt = self.assign(&var, call);
                            self.forget_shape(&var);
                            if self.pass == Pass::Final {
                                return Ok(self.taken_in_place(stmt));
                            }
                            return Ok(vec![stmt]);
                        }
                        // through fields and items: the tensor read where it
                        // lies and written back whole, a position computed
                        // along the way computed once
                        let mut out = Vec::new();
                        let mut read = Expr::Local(var.clone(), var_ty.clone());
                        for (step, ty) in path.iter_mut().zip(step_types) {
                            read = match step {
                                PathStep::Field(name) => Expr::Field { base: Box::new(read), name: name.clone(), ty },
                                PathStep::Index(e) => {
                                    if !matches!(e, Expr::Const(..) | Expr::Local(..)) {
                                        let t = self.fresh();
                                        let et = e.ty();
                                        out.push(Stmt::Let { var: t.clone(), ty: et.clone(), expr: e.clone() });
                                        if let Some(scope) = self.scopes.last_mut() {
                                            scope.insert(t.clone());
                                        }
                                        *e = Expr::Local(t, et);
                                    }
                                    Expr::Index { base: Box::new(read), indices: vec![e.clone()], ty }
                                }
                            };
                        }
                        let call = Expr::Call { target: Target::Row(row), args: vec![read, at, value], ty: tensor_ty.clone(), propagates: false };
                        self.forget_shape(&var);
                        out.extend(self.path_write(var, path, call));
                        return Ok(out);
                    }
                }
                // an item assigned a value of a known kind types the list's
                // items where they were not known (a list of empty lists,
                // one of them given a list of records — a graph's arcs —
                // had kept its items unknown, and the compiled record's
                // field was a list of text)
                // (noted as the call scan and the settling pass note an
                // assignment: the settling pass is what the final lowering
                // declares the local from)
                // (a list whose items are not known yet — never a variable
                // whose kind is not known at all: a tensor takes an item too,
                // and a guess of a list stuck to a function it was handed to)
                if self.pass != Pass::Final && path.is_empty() {
                    let held = self.ctx.vars.get(&var).cloned().unwrap_or(Ty::Unknown);
                    let item = read(&expr);
                    let open = matches!(&held, Ty::List(inner) if inner.has_unknown());
                    if open && !item.has_unknown() && !matches!(item, Ty::Unit) {
                        let refined = Ty::List(Box::new(item));
                        self.ctx.note_scanned_assign(&var, refined.clone());
                        self.rebind(&var, &refined);
                    }
                }
                path.push(PathStep::Index(idx));
                self.forget_shape(&var);
                self.path_write(var, path, expr)
            }
            S::If { condition, then_body, else_body } => {
                let cond = self.lower_cond(condition)?;
                let mut prelude = std::mem::take(&mut self.pre);
                // a frame the branches bind anew has, after them, the columns
                // both ways give it
                let before = self.detailed_in_view();
                let (failed_then, failed_else) = frames::guarded_failure(condition);
                self.scopes.push(HashSet::new());
                self.failed.extend(failed_then.clone());
                let then = self.lower_block(then_body);
                self.failed.truncate(self.failed.len() - failed_then.len());
                let then = then?;
                self.scopes.pop();
                let after_then = self.detailed_now(&before);
                for (n, t) in &before {
                    self.ctx.vars.insert(n.clone(), t.clone());
                }
                let els = match else_body {
                    Some(b) => {
                        self.scopes.push(HashSet::new());
                        self.failed.extend(failed_else.clone());
                        let e = self.lower_block(b);
                        self.failed.truncate(self.failed.len() - failed_else.len());
                        let e = e?;
                        self.scopes.pop();
                        Some(e)
                    }
                    None => None,
                };
                let after_else = self.detailed_now(&before);
                self.join_detailed(&after_then, &after_else);
                prelude.push(Stmt::If { cond, then, els });
                prelude
            }
            S::While { condition, body } => {
                // a body that binds a frame anew reads, at its top, what the
                // loop's entry and its end both give
                let before = self.detailed_in_view();
                if self.reads_ahead(&before, body) {
                    self.pre_join_loop(&before, body);
                }
                let top = self.detailed_now(&before);
                let cond = self.lower_cond(condition)?;
                let prelude = std::mem::take(&mut self.pre);
                self.scopes.push(HashSet::new());
                let b = if self.pass == Pass::Scan { self.scan_loop_body(body)? } else { self.lower_block(body)? };
                self.scopes.pop();
                let end = self.detailed_now(&before);
                self.join_detailed(&top, &end);
                if prelude.is_empty() {
                    vec![Stmt::While { cond, body: b }]
                } else {
                    // a condition that had to be propagated first is
                    // evaluated again before each test, as it is written
                    let mut each = prelude;
                    each.push(Stmt::If {
                        cond: Expr::Unary { op: "not".to_string(), expr: Box::new(cond), ty: Ty::Bool },
                        then: vec![Stmt::Break],
                        els: None,
                    });
                    each.extend(b);
                    vec![Stmt::While { cond: Expr::Const(Const::Bool(true), Ty::Bool), body: each }]
                }
            }
            S::For { var, iter, body } => {
                self.check_reserved(var, "loop variable");
                self.check_not_a_function(var, "loop variable");
                // a list that can fail is refused like a condition
                let it = self.lower_expr(iter)?;
                // (`[]` written as what a loop walks walks no items, of any
                // kind: ints, for the compiled program to name one)
                let it = match it {
                    Expr::ListLit(items, Ty::List(e)) if items.is_empty() && *e == Ty::Unknown => Expr::ListLit(items, Ty::List(Box::new(Ty::Int))),
                    other => other,
                };
                let walks = walked(&it);
                let it = self.decided(it, Decision::Loop);
                let mut prelude = std::mem::take(&mut self.pre);
                self.scopes.push([var.clone()].into_iter().collect());
                let b = if self.pass != Pass::Final {
                    // inference binds the variable to an item of what the
                    // loop walks, read as text, and leaves it bound — but a
                    // name bound before the loop, which the variable hides
                    // inside it, keeps its own type after (a top-level
                    // record named like a later loop's variable had been
                    // declared a record or none, and did not build)
                    let shadowed = self.ctx.vars.get(var.as_str()).cloned();
                    self.ctx.scan_loop_var(var, walks);
                    let unknown_items = matches!(it.ty().payload(), Ty::List(e) if **e == Ty::Unknown);
                    if unknown_items {
                        self.v.unknown_items.insert(var.clone());
                    }
                    let before = self.detailed_in_view();
                    if self.reads_ahead(&before, body) {
                        self.pre_join_loop(&before, body);
                    }
                    let top = self.detailed_now(&before);
                    // a call through the variable of a loop over a literal
                    // list of named functions types every one of them, as a
                    // collecting loop's does (`for f in [double, triple]`
                    // calling `f[2]` had been an internal error)
                    let fn_list = match iter {
                        A::Array(elems) if !elems.is_empty() && elems.iter().all(|e| matches!(e, A::Ident(n) if self.calls_fn(n))) => {
                            Some(elems.iter().filter_map(|e| if let A::Ident(n) = e { Some(n.clone()) } else { None }).collect::<Vec<String>>())
                        }
                        _ => None,
                    };
                    let listed = self.pass == Pass::Scan && fn_list.is_some();
                    if let (true, Some(fns)) = (listed, fn_list) {
                        self.fn_lists.push((var.clone(), fns));
                    }
                    let b = if self.pass == Pass::Scan { self.scan_loop_body(body) } else { self.lower_block(body) };
                    if listed {
                        self.fn_lists.pop();
                    }
                    if unknown_items {
                        self.v.unknown_items.remove(var.as_str());
                    }
                    let b = b?;
                    let end = self.detailed_now(&before);
                    self.join_detailed(&top, &end);
                    if let Some(t) = shadowed {
                        self.ctx.vars.insert(var.clone(), t);
                    }
                    b
                } else {
                    self.check_rank_known(&it, "a loop over");
                    self.check_walkable(&it);
                    let saved = self.bind_loop_var(var, &it.ty());
                    // a body that binds a frame anew reads, at its top, what
                    // the loop's entry and its end both give
                    let before = self.detailed_in_view();
                    if self.reads_ahead(&before, body) {
                        self.pre_join_loop(&before, body);
                    }
                    let top = self.detailed_now(&before);
                    let b = self.lower_block(body)?;
                    let end = self.detailed_now(&before);
                    self.join_detailed(&top, &end);
                    self.unbind_loop_var(var, saved);
                    b
                };
                self.scopes.pop();
                // in parallel or in order: decided once every function is
                // known (decide_loops)
                prelude.push(Stmt::For { var: var.clone(), iter: it, body: b, parallel: false });
                prelude
            }
            S::Return(None) => {
                if let (Some(seen), 0) = (&mut self.seen, self.collecting) {
                    seen.returns.push(Ty::Unit);
                    seen.returned.push(None);
                }
                vec![Stmt::Return(None)]
            }
            S::Return(Some(e)) => {
                // a local nothing wrote a kind into, handed back, holds what
                // the function answers: `out = map[]` returned where a group
                // was pruned, beside a return of a map of records, had been
                // declared a map of text compiled (E0308; parity 319)
                if self.pass != Pass::Final {
                    self.settled_return(e);
                }
                let x = self.lower_expr(e)?;
                // a value a guard knows has failed hands back its failure:
                // its value's type (a frame's columns) is none of the
                // function's
                let failure_only = matches!(e, A::Ident(v) if self.failed.contains(v)) && x.ty().is_result();
                // (a call of the function itself answers what the function
                // answers, and says nothing of it: `countdown[n - 1]` on its
                // last line beside a bare `return` had left the answer
                // unknown, and compiled `return;` did not build; Rosetta
                // Code, AVL tree)
                let recursion = matches!(e, A::Call { callee, .. } if matches!(callee.as_ref(), A::Ident(f) if *f == self.current_fn || self.ctx.specializations.get(f).is_some_and(|v| v.iter().any(|(_, c)| *c == self.current_fn))));
                if let (Some(seen), 0, false) = (&mut self.seen, self.collecting, recursion) {
                    // (a column with its cells' kind, as a variable keeps it)
                    seen.returns.push(if failure_only { Ty::Result(Box::new(Ty::Unknown)) } else { kept(&x) });
                    // (a failure handed on hands back no local: its payload
                    // is not the answer — `return delta` of a failed read had
                    // joined a list of ints into a list of records, linked)
                    seen.returned.push(match e {
                        A::Ident(v) if !failure_only => Some(v.clone()),
                        _ => None,
                    });
                }
                // a function answers one kind: a return of another kind than
                // the returns before it answered is refused, naming both
                // (`return 'a'` beside `return x` of an int had been the
                // verifier's internal error, in both modes — crow check
                // --self); judged against the returns as this lowering types
                // them, not inference's first guess of the answer (a frame's
                // cell read before the frame's columns were known had been
                // guessed a parsed value, and kept); numbers meet as the
                // number rule says (an int returned where floats are is a
                // float, below), a list where a tensor is answered is that
                // tensor (below), and a record beside none is one that may
                // be none
                if self.pass == Pass::Final && !failure_only && self.v.mute == 0 {
                    use crate::table::Arg;
                    let given = Arg::of_type(x.ty().payload());
                    let number = |a: &Arg| matches!(a, Arg::Int | Arg::Float | Arg::Decimal | Arg::Fraction);
                    let listed = |a: &Arg| matches!(a, Arg::Ints | Arg::Floats | Arg::AnyList | Arg::Rows | Arg::Tensor | Arg::Tensors | Arg::FallibleNumbers | Arg::Fallible);
                    let unsettled = |a: &Arg| matches!(a, Arg::Unknown | Arg::AnyList | Arg::Fallible | Arg::FallibleNumbers);
                    // (`return 0` beside a record answers none: the function
                    // answers a record or none — refused as two kinds, a tree
                    // builder's empty branch had not run; Rosetta Code, K-d
                    // tree. Any other int beside a record is two kinds.)
                    let none = matches!(&x, Expr::Const(Const::Int(0), _));
                    let nones = self.returns_none.contains(&self.current_fn);
                    if !unsettled(&given) {
                        let seen = self.returns_seen.entry(self.current_fn.clone()).or_default();
                        let other = seen
                            .iter()
                            .find(|answer| {
                                let record_or_none = none && matches!(answer, Arg::Record);
                                !(**answer == given || (number(answer) && number(&given)) || (listed(answer) && listed(&given)) || record_or_none)
                            })
                            .cloned()
                            .or_else(|| (nones && !(number(&given) || matches!(given, Arg::Record))).then_some(Arg::Int));
                        if let Some(answer) = other {
                            self.refuse(format!("returns {} where the function answers {} — a function answers one kind", given.words(), answer.words()));
                        } else if none {
                            self.returns_none.insert(self.current_fn.clone());
                        } else {
                            seen.push(given);
                        }
                    }
                }
                // an int returned by a function whose value is a float is a
                // float (spec §1), as one assigned to a float variable is: the
                // interpreter handed the caller the int
                let floats = self.pass == Pass::Final
                    && self.ctx.functions.get(&self.current_fn).is_some_and(|sig| match &sig.return_type {
                        Ty::Result(inner) => matches!(inner.as_ref(), Ty::Float),
                        t => matches!(t, Ty::Float),
                    });
                // (an int that can fail returned from a function of floats
                // that can fail converts as its payload: a function handed as
                // a value beside one answering floats answers floats)
                let x = if floats && x.ty().payload() == &Ty::Int && !matches!(x, Expr::Convert { how: Convert::Unwrap, .. }) {
                    let ty = if x.ty().is_result() { Ty::Result(Box::new(Ty::Float)) } else { Ty::Float };
                    Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty }
                } else {
                    x
                };
                // a list of numbers returned by a function whose value is a
                // tensor — a literal list in one branch, a loop's list in
                // another — is that tensor, in both modes (the compiled
                // function had answered a Vec where it stated a tensor, and
                // did not build)
                let tensor = self.pass == Pass::Final
                    && self.ctx.functions.get(&self.current_fn).is_some_and(|sig| match &sig.return_type {
                        Ty::Result(inner) => matches!(inner.as_ref(), Ty::Tensor(..)),
                        t => matches!(t, Ty::Tensor(..)),
                    });
                // (the list's items, ints or floats, as the function answers
                // them)
                let list_item = if self.pass == Pass::Final {
                    self.ctx.functions.get(&self.current_fn).and_then(|sig| match sig.return_type.payload() {
                        Ty::List(i) if matches!(i.as_ref(), Ty::Int | Ty::Float) => Some(i.as_ref().clone()),
                        _ => None,
                    })
                } else {
                    None
                };
                let list = list_item.is_some();
                // (a literal list of numbers returned where the function
                // answers a list of numbers that can fail or grow: the
                // literal is that list, its numbers as written, the width
                // pass growing them where the items grow — it had stayed a
                // tensor beside `return [a, a]` of `a = int[text]`, and the
                // lowered program disagreed with itself; Rosetta Code,
                // Honeycombs)
                let held_item = if self.pass == Pass::Final {
                    self.ctx.functions.get(&self.current_fn).and_then(|sig| match sig.return_type.payload() {
                        Ty::List(i) if matches!(i.as_ref(), Ty::Big | Ty::Result(_)) && matches!(i.payload(), Ty::Int | Ty::Float | Ty::Big) => Some(i.as_ref().clone()),
                        _ => None,
                    })
                } else {
                    None
                };
                let x = match (held_item, x) {
                    (Some(item), Expr::ListLit(items, Ty::Tensor(_, d))) if d.as_ref().is_none_or(|d| d.len() <= 1) => {
                        let floats = matches!(item.payload(), Ty::Float);
                        let items = items
                            .into_iter()
                            .map(|i| match i.ty() {
                                Ty::Int if floats => Expr::Convert { how: Convert::IntToFloat, expr: Box::new(i), ty: Ty::Float },
                                _ => i,
                            })
                            .collect();
                        Expr::ListLit(items, Ty::List(Box::new(item)))
                    }
                    (_, x) => x,
                };
                let mut x = if tensor && !x.ty().is_result() && matches!(x.ty(), Ty::List(item) if matches!(item.payload(), Ty::Int | Ty::Float)) {
                    let ty = Ty::Tensor(Box::new(Ty::Float), shapes::stacked_dims(&x.ty()));
                    Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                } else if list && !x.ty().is_result() && matches!(x.ty(), Ty::Tensor(..)) {
                    // (and the other way: a literal list of numbers, a tensor,
                    // returned where the function answers a list — of ints
                    // where its items are ints: `return [0]` beside
                    // `out = push[out, x]` had been refused as a list of
                    // floats, "the lowered program disagrees with itself")
                    let ty = Ty::List(Box::new(list_item.clone().unwrap_or(Ty::Float)));
                    Expr::Convert { how: Convert::ToList, expr: Box::new(x), ty }
                } else {
                    x
                };
                // (a list of plain items returned where the function answers
                // a list whose items can fail is that list, each item the
                // success it is; one that can fail is returned as its
                // failure where it failed — the compiled return had
                // rewrapped a success's list as an empty one, a wrong
                // answer; Rosetta Code, Weiler-Atherton polygon clipping)
                let answers = self.ctx.functions.get(&self.current_fn).map(|s| s.return_type.payload().clone());
                if self.pass == Pass::Final
                    && let Some(Ty::List(item)) = &answers
                    && item.is_result()
                    && matches!(x.ty().payload(), Ty::List(i) if !i.is_result() && i.as_ref() == item.payload())
                {
                    if x.ty().is_result() {
                        let t = self.fresh();
                        let ty = x.ty();
                        self.pre.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: x });
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.insert(t.clone());
                        }
                        let failed = Expr::Unary { op: "not".to_string(), expr: Box::new(Expr::Meta { base: Box::new(Expr::Local(t.clone(), ty.clone())), which: Meta::Ok }), ty: Ty::Bool };
                        self.pre.push(Stmt::If { cond: failed, then: vec![Stmt::Return(Some(Expr::Local(t.clone(), ty.clone())))], els: None });
                        let known = Expr::Convert { how: Convert::Known, expr: Box::new(Expr::Local(t, ty.clone())), ty: ty.payload().clone() };
                        x = fallible_items(known, item);
                    } else {
                        x = fallible_items(x, item);
                    }
                }
                // a handler's value goes to nobody: a helper it returns is
                // called for what it writes, as a statement's is
                if self.at == Where::In(FnKind::Handler) {
                    self.refuse_before_helper(&mut x);
                }
                // a route's handler answers a response (spec, Responses): a
                // value of another kind is refused before either mode runs —
                // the interpreter had answered 500 as the request came, and
                // the compiled route an empty 200 (crow check --self: under
                // `use sys`, `text[bytes[…]]` is sys's text)
                // (in a web program: without `use web` the route itself is
                // what is refused)
                if self.pass == Pass::Final && self.v.route_handlers.contains(&self.current_fn) && self.web_program() {
                    let t = x.ty();
                    if !matches!(t.payload(), Ty::Unknown | Ty::Handle(_)) {
                        let what = crate::table::Arg::of_type(t.payload()).words();
                        self.refuse(crow_web::core::response::not_a_response(&self.current_fn, what));
                    }
                }
                vec![Stmt::Return(Some(x))]
            }
            S::Break => vec![Stmt::Break],
            S::Continue => vec![Stmt::Continue],
            S::ExprStmt(e) => {
                let mut x = self.lower_expr(e)?;
                if self.pass == Pass::Settle {
                    self.settled_set(e, &x);
                }
                // a builtin that answers a new value of the kind it was
                // handed — push, set, sort, upper — called as a statement:
                // the answer is dropped and the value is as it was (a
                // check's list of results had stayed empty, every test
                // passing)
                if self.pass == Pass::Final
                    && !self.yields.contains(&(e as *const A as usize))
                    && let Expr::Call { target: Target::Row(row), .. } = &x
                {
                    let answers_anew = row.params.first() == Some(&row.ret) && !matches!(row.ret, crate::table::Kind::Unit);
                    if answers_anew && !crate::ir::is_effect(row.name) {
                        // the call named as the program wrote it (table::form):
                        // a hidden row by its form — the pick behind
                        // `df[df.age gt 50]` is `f[mask]`, what it indexes kept
                        // — never by its own name (`__pick_frame[]` had been)
                        let called = if row.written.is_empty() { row.name } else { row.written };
                        let form = crate::table::form(row.name);
                        let (callee, first) = match e {
                            A::Call { callee, args } => (
                                match callee.as_ref() {
                                    A::Ident(c) => Some(c.clone()),
                                    _ => None,
                                },
                                match args.first() {
                                    Some(A::Ident(v)) => Some(v.clone()),
                                    _ => None,
                                },
                            ),
                            _ => (None, None),
                        };
                        let keep = match (form == format!("{}[]", called), callee, first) {
                            (true, _, Some(v)) => format!("{0} = {1}[{0}, …]", v, called),
                            (true, _, None) => format!("x = {}[…]", called),
                            (false, Some(c), _) => format!("{0} = {0}[…]", c),
                            (false, None, _) => format!("x = {}", form.trim_matches('`')),
                        };
                        self.refuse(format!("{} answers {}, and this line drops it — keep it: {}", form, row.ret.words(), keep));
                    }
                }
                // a value that can fail, dropped by its line, is decided on as
                // a read of one is: a function hands the failure back, a
                // handler stops, the top level halts (spec, Errors in
                // Conditions) — a failed write had gone by unseen in both
                // modes. What a line hands on (a collecting loop's last, a
                // console cell's) is no drop, and assigned it is a value; nor
                // is a truth — a line written as a comparison, or a test's
                // bool, which the test asserts (`len[v] eq 59` of a value
                // that can fail)
                let truth = Self::truth_syntax(e) || (matches!(self.at, Where::Test) && matches!(x.ty().payload(), Ty::Bool));
                // (a mail nothing reads is held for its handler's commit and
                // sent then, or dropped by a stop: reading its answer here
                // would send it at once — its failure is the commit's)
                let held = matches!(&x, Expr::Effect { name, .. } if name == "mail");
                // (a handler, or a helper that writes facts, called on its own
                // line: its stop is the handler's — the session stops and
                // nothing lands — and it answers nothing a line could drop;
                // compiled, it answers no value at all)
                let callee = match &x {
                    Expr::Convert { how: Convert::Unwrap, expr, .. } => expr.as_ref(),
                    other => other,
                };
                let handler_call = match callee {
                    Expr::Call { target: Target::User(g), .. } => {
                        self.ctx.functions.get(g).is_some_and(|sig| matches!(self.kind_of(g, &sig.body, false), FnKind::Handler | FnKind::Helper))
                    }
                    _ => false,
                };
                let dropped = !truth
                    && !held
                    && !handler_call
                    && !self.yields.contains(&(e as *const A as usize))
                    && !self.in_fact_default
                    && x.ty().is_result()
                    && !matches!(x.ty().payload(), Ty::Fn(..) | Ty::Unknown);
                if dropped {
                    if self.pass == Pass::Final && std::env::var_os("CROW_DROP_AUDIT").is_some() {
                        eprintln!("drop-audit: fn '{}' line {}", self.current_fn, self.v.line);
                    }
                    self.refuse_before_helper(&mut x);
                    let _ = self.decided(x, Decision::Dropped);
                    vec![]
                } else {
                    if let Expr::Effect { answered, .. } = &mut x {
                        *answered = false;
                    }
                    self.refuse_before_helper(&mut x);
                    vec![Stmt::Do(x)]
                }
            }
            // (a declaration inside a body is refused where it is written,
            // with where it goes: it had stopped the lowering with no line,
            // in words that read as Crow's own fault; Rosetta Code, Nested
            // function, Inner classes)
            S::FnDecl { name, .. } => {
                self.refuse(format!(
                    "'{}' is a function declared inside a body — functions are declared at the top of the file, and a function sees its parameters, not the body's variables: move it out and hand it what it reads",
                    crate::lower::shown(name)
                ));
                vec![]
            }
            S::StructDecl { name, .. } => {
                self.refuse(format!("'{}' is a record declared inside a body — records are declared at the top of the file: move it out", crate::lower::shown(name)));
                vec![]
            }
            S::Derive { name, .. } => {
                self.refuse(format!("'{}' is a derivation declared inside a body — derivations are declared at the top of the file: move it out", crate::lower::shown(name)));
                vec![]
            }
            S::TestBlock { .. } => {
                self.refuse("a test block is written at the top of the file, not inside a body: move it out".to_string());
                vec![]
            }
            S::Use { path, .. } => {
                self.refuse(format!("`use {}` is written at the top of the file, not inside a body: move it there", path));
                vec![]
            }
        })
    }

    /// Several names assigned at once, a value each (the parser refuses any
    /// other count: Crow takes no list apart into names), all read before
    /// any is written when they read each other.
    fn multi_assign(&mut self, names: &[String], values: &[A]) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        // (a value of a call that answers nothing is refused for its name,
        // as an assignment's is)
        let final_pass = self.pass == Pass::Final;
        let targets: BTreeSet<String> = names.iter().cloned().collect();
        let mut reads = BTreeSet::new();
        let mut clock = false;
        for v in values {
            let mut r = BTreeSet::new();
            reads_of_expr(v, &mut r, &mut clock);
            reads.extend(r);
        }
        if reads.is_disjoint(&targets) {
            for (n, v) in names.iter().zip(values) {
                let e = self.lower_expr(v)?;
                if final_pass {
                    self.refuse_nothing_held(&format!("'{}'", crate::lower::shown(n)), &e);
                }
                let stmt = self.assign(n, e);
                self.flush_pre_into(&mut out);
                out.push(stmt);
            }
        } else {
            let mut temps = Vec::new();
            for (i, v) in values.iter().enumerate() {
                let e = self.lower_expr(v)?;
                if final_pass && let Some(n) = names.get(i) {
                    self.refuse_nothing_held(&format!("'{}'", crate::lower::shown(n)), &e);
                }
                let t = self.fresh();
                let ty = e.ty();
                self.flush_pre_into(&mut out);
                out.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: e });
                self.scopes.last_mut().unwrap().insert(t.clone());
                temps.push((t, ty));
            }
            for (n, (t, ty)) in names.iter().zip(temps) {
                let stmt = self.assign(n, Expr::Local(t, ty));
                self.flush_pre_into(&mut out);
                out.push(stmt);
            }
        }
        Ok(out)
    }

    /// A condition: a fallible one is propagated first (spec §10, Errors
    /// in Conditions), then decided on its payload.
    fn lower_cond(&mut self, c: &A) -> Result<Expr, String> {
        let e = self.lower_expr(c)?;
        // a parsed value decided on is read as a bool
        let e = self.read_json(e, &Ty::Bool, "decided on");
        let e = self.decided(e, Decision::Branch);
        // a condition is true or false: a number, text or a list decided on
        // is refused as the program's, in both modes (`if 5` had reached the
        // verifier as "the lowered program disagrees with itself")
        let kind = e.ty().payload().clone();
        if self.pass == Pass::Final && !matches!(kind, Ty::Bool | Ty::Unknown | Ty::Json) && !self.contested(&e) {
            let words = crate::table::Arg::of_type(&kind).words();
            let what = match c {
                A::Ident(n) => format!("'{}' is {}", n, words),
                _ => format!("this is {}", words),
            };
            self.refuse(format!("{}, and a condition is true or false — compare it: x gt 0, len[xs] gt 0, s ne ''", what));
        }
        Ok(e)
    }

    /// What a decision reads: a value that can fail is refused first when it
    /// failed (a function hands it back, a handler stops, the top level
    /// halts), and the decision reads its payload.
    /// `l and r` (or `or`) whose right side needs statements first: the
    /// left into a temporary, then — only where it does not decide — the
    /// right side's statements and the right side into it (spec, Operators:
    /// the right side runs only where the left does not decide). A mask's
    /// `and`, which joins item by item, and every other operator compute
    /// both sides, the left first (`left_first`).
    fn right_side_waits(&mut self, joined: Expr, right_pre: Vec<Stmt>) -> Expr {
        let Expr::Binary { op, l, r, ty } = joined else { return joined };
        if !matches!(op.as_str(), "and" | "or") || !crate::ir::short_circuits(&l, &r) {
            return self.left_first(op, l, r, ty, right_pre);
        }
        let t = self.fresh();
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(t.clone());
        }
        let left_ty = l.ty();
        self.pre.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: *l });
        let held = Expr::Local(t.clone(), ty.clone());
        // (the left holds a truth, or an error: the right runs where it is
        // a truth that does not decide)
        let truth = if left_ty.is_result() || ty.is_result() {
            Expr::Convert { how: Convert::Known, expr: Box::new(held.clone()), ty: Ty::Bool }
        } else {
            held.clone()
        };
        let goes_on = if op == "and" { truth } else { Expr::Unary { op: "not".to_string(), expr: Box::new(truth), ty: Ty::Bool } };
        let cond = if left_ty.is_result() || ty.is_result() {
            Expr::Binary { op: "and".to_string(), l: Box::new(Expr::Meta { base: Box::new(held.clone()), which: Meta::Ok }), r: Box::new(goes_on), ty: Ty::Bool }
        } else {
            goes_on
        };
        let mut then = right_pre;
        then.push(Stmt::Set { var: t.clone(), expr: *r });
        self.pre.push(Stmt::If { cond, then, els: None });
        held
    }

    /// `l op r` whose right side needs statements first, where the operator
    /// computes both sides (a mask's `and`, a comparison, arithmetic): the
    /// left side runs first (spec, Operators) — into a temporary ahead of
    /// the right side's statements, unless it is quiet (`ir::quiet`: nothing
    /// it does could be observed, nothing could change what it reads).
    /// `put[d, f] eq 1 and latest[d].id eq ['a', 'b']` had read the log
    /// before the put: the read of the value that can fail was hoisted
    /// ahead of the statement, and the put stayed in the expression
    /// (parity 302).
    fn left_first(&mut self, op: String, l: Box<Expr>, r: Box<Expr>, ty: Ty, right_pre: Vec<Stmt>) -> Expr {
        if crate::ir::quiet(&l) {
            self.pre.extend(right_pre);
            return Expr::Binary { op, l, r, ty };
        }
        let t = self.fresh();
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(t.clone());
        }
        let left_ty = l.ty();
        self.pre.push(Stmt::Let { var: t.clone(), ty: left_ty.clone(), expr: *l });
        self.pre.extend(right_pre);
        Expr::Binary { op, l: Box::new(Expr::Local(t, left_ty)), r, ty }
    }

    /// A position or a key that can fail, refused first when it failed —
    /// a function hands the error back, a handler stops, the top level
    /// halts — as an item of a value that can fail is: one rule for
    /// reading into a value (Vijay, 2026-10-01). `xs[int[ch]]` had typed
    /// the item as able to fail where both backends stopped, and the
    /// compiled program did not build.
    /// An item of a local list nothing is put into — `g[0]` of `g = []`,
    /// or of a collecting loop whose last line is a `for` statement, which
    /// collects nothing — where its kind decides (an operator's side): the
    /// list is always empty, and the item has no kind to compute with.
    /// Refused, naming the list (the interpreter had stopped at the index,
    /// and the compiled program had not built; Rosetta Code, Inventory
    /// sequence). A parameter's list is its callers' to type.
    fn kindless_item(&mut self, x: &Expr) {
        if self.pass != Pass::Final {
            return;
        }
        let mut e = x;
        while let Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } = e {
            e = expr;
        }
        let Expr::Index { base, .. } = e else { return };
        let mut root = base.as_ref();
        while let Expr::Index { base, .. } = root {
            root = base.as_ref();
        }
        let mut t = root.ty().payload().clone();
        if !matches!(t, Ty::List(_)) {
            return;
        }
        while let Ty::List(i) = t {
            t = i.payload().clone();
        }
        let Some(name) = local_named(root) else { return };
        let param = self.ctx.functions.get(&self.current_fn).is_some_and(|sig| sig.params.iter().any(|(p, _)| *p == name));
        if t != Ty::Unknown || param {
            return;
        }
        // (a list pushed onto is given items, of a kind not known yet where
        // the function's parameters are not typed — a part checked alone,
        // a function no call types: refused there, `ch = push[ch, t *
        // secs]` had stopped every program importing the part)
        let grown = self.grown.iter().any(|g| g.contains(&name));
        let untyped = self.ctx.functions.get(&self.current_fn).is_some_and(|sig| sig.params.iter().any(|(_, t)| t.has_unknown()));
        if grown || untyped {
            return;
        }
        self.refuse(format!("'{}' is never given an item, so its items have no kind to compute with", crate::lower::shown(&name)));
    }

    fn decided_index(&mut self, i: Expr) -> Expr {
        if self.in_fact_default || !i.ty().is_result() || matches!(i.ty().payload(), Ty::Fn(..) | Ty::Unknown) {
            return i;
        }
        self.decided(i, Decision::Index)
    }

    fn decided(&mut self, e: Expr, at: Decision) -> Expr {
        if e.ty().is_result() {
            let ty = e.ty();
            let t = match &e {
                Expr::Local(v, _) => v.clone(),
                _ => {
                    let t = self.fresh();
                    self.pre.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: e });
                    self.scopes.last_mut().unwrap().insert(t.clone());
                    t
                }
            };
            self.saw_refusal(&at);
            self.pre.push(Stmt::Propagate { values: vec![t.clone()], at });
            return Expr::Convert {
                how: Convert::Unwrap,
                expr: Box::new(Expr::Local(t, ty.clone())),
                ty: ty.payload().clone(),
            };
        }
        e
    }

    /// A loaded frame bound where the program's reads say its columns
    /// (uses.rs): the value held to that shape as the program runs
    /// (`__expect_frame`), and typed by it from here on, in both modes.
    /// A parsed value read as the kind its use implies — a position an
    /// int, a key text, one met in arithmetic a float, one decided on a
    /// bool — by a row that stops with what the value holds and the read
    /// that asked, in both modes (spec, JSON). A value of any other type
    /// is itself. (Each such use had been refused until read by hand, and
    /// a parsed value decided on, negated or joined with and had run
    /// interpreted and not built compiled.)
    /// A parsed value handed to a parameter, or a parameter typed by
    /// nothing, read in the body as a kind: noted for the function's tool
    /// schema (`tool_schema`).
    fn note_param_read(&mut self, expr: &Expr, kind: &Ty) {
        let local = match expr {
            Expr::Convert { expr: inner, .. } => inner.as_ref(),
            other => other,
        };
        if let Expr::Local(p, lt) = local {
            let is_param = matches!(lt.payload(), Ty::Json | Ty::Unknown) && self.ctx.functions.get(&self.current_fn).is_some_and(|sig| sig.params.iter().any(|(n, _)| n == p));
            if is_param && !matches!(kind, Ty::Unknown) {
                let read = if matches!(kind, Ty::Markup) { Ty::Str } else { kind.clone() };
                self.ctx.param_reads.entry(self.current_fn.clone()).or_default().insert(p.clone(), read);
            }
        }
    }

    fn read_json(&mut self, expr: Expr, kind: &Ty, what: &str) -> Expr {
        if !matches!(expr.ty().payload(), Ty::Json) {
            return expr;
        }
        self.note_param_read(&expr, kind);
        let row_name = match kind {
            Ty::Int => "__read_int",
            Ty::Float => "__read_float",
            Ty::Str | Ty::Markup => "__read_str",
            Ty::Bool => "__read_bool",
            Ty::Moment => "__read_moment",
            _ => return expr,
        };
        let Some(row) = crate::table::core_row(row_name, &[crate::table::Arg::Json, crate::table::Arg::Text]) else { return expr };
        let failing = expr.ty().is_result();
        let arg = if failing { Expr::Convert { how: Convert::Unwrap, expr: Box::new(expr), ty: Ty::Json } } else { expr };
        // (inside a standard module's function the line is none of the
        // program's — its stop names the program's line of the call — so
        // the use is said alone: `handed to starts`, where magpie's `merge`
        // of a parsed value had said "at line 12" of an eleven-line program)
        let said = if crate::modules::is_crow_fn(&self.current_fn) { what.to_string() } else { format!("{} at line {}", what, self.v.line) };
        let how = Expr::Const(Const::Str(said), Ty::Str);
        let read = if matches!(kind, Ty::Markup) { Ty::Str } else { kind.clone() };
        let ty = if failing { Ty::Result(Box::new(read)) } else { read };
        Expr::Call { target: Target::Row(row), args: vec![arg, how], ty, propagates: failing }
    }

    fn held_to_shape(&mut self, name: &str, expr: Expr) -> Expr {
        if !uses::dynamic_frame(&expr.ty()) {
            return expr;
        }
        let body = if self.current_fn.starts_with(uses::TOP) { uses::TOP.to_string() } else { self.current_fn.clone() };
        let key = (body, name.to_string());
        let Some(cols) = self.ctx.frame_shapes.get(&key).cloned() else { return expr };
        // a page's data source arrives through the session, which holds it
        // to the columns its fact carries (`Fact::held`): the call is typed
        // by them here and wrapped in nothing
        if is_source_call(&expr) {
            let shaped = Ty::Frame(Some(cols));
            return match expr {
                Expr::Call { target, args, propagates, .. } => Expr::Call { target, args, ty: shaped, propagates },
                // (`fetch` and `poll` are effects: they run where the
                // session loads the source)
                Expr::Effect { name, row, args, answered, propagates, .. } => Expr::Effect { name, row, args, ty: shaped, answered, propagates },
                other => other,
            };
        }
        let reasons = self.ctx.frame_reasons.get(&key).cloned().unwrap_or_default();
        let Some(row) = crate::table::core_row("__expect_frame", &[crate::table::Arg::Frame, crate::table::Arg::Texts, crate::table::Arg::Texts, crate::table::Arg::Texts]) else { return expr };
        let failing = expr.ty().is_result();
        let lit = |items: Vec<String>| Expr::ListLit(items.into_iter().map(|s| Expr::Const(Const::Str(s), Ty::Str)).collect(), Ty::List(Box::new(Ty::Str)));
        let names: Vec<String> = cols.cols.keys().cloned().collect();
        // (the kind of each column's cells, which can be missing)
        let kinds: Vec<String> = cols.cols.values().map(|t| t.payload().crow_name()).collect();
        // the read that decided each column, for the stop's words
        let hows: Vec<String> = cols.cols.keys().map(|c| reasons.get(c).cloned().unwrap_or_default()).collect();
        let shaped = Ty::Frame(Some(cols));
        let arg = if failing { Expr::Convert { how: Convert::Unwrap, expr: Box::new(expr.clone()), ty: expr.ty().payload().clone() } } else { expr };
        let ty = if failing { Ty::Result(Box::new(shaped)) } else { shaped };
        Expr::Call { target: Target::Row(row), args: vec![arg, lit(names), lit(kinds), lit(hows)], ty, propagates: failing }
    }

    /// What a data source's fact carries to the session: each column the
    /// program reads, its kind and the read that decided it.
    fn held_of(&self, name: &str) -> Vec<(String, String, String)> {
        let key = (uses::TOP.to_string(), name.to_string());
        let Some(cols) = self.ctx.frame_shapes.get(&key) else { return Vec::new() };
        let reasons = self.ctx.frame_reasons.get(&key).cloned().unwrap_or_default();
        cols.cols.iter().map(|(c, t)| (c.clone(), t.payload().crow_name(), reasons.get(c).cloned().unwrap_or_default())).collect()
    }

    fn assign(&mut self, name: &str, expr: Expr) -> Stmt {
        let expr = self.held_to_shape(name, expr);
        if let Some(f) = gradient_of_call(&expr) {
            self.grad_of.insert(name.to_string(), f);
        }
        match pieces_of_call(&expr) {
            Some(p) => {
                self.pieces.insert(name.to_string(), p);
            }
            None => {
                self.pieces.remove(name);
            }
        }
        match self.pass {
            Pass::Settle => return self.settled(name, expr),
            Pass::Scan => return self.scanned(name, expr),
            Pass::Final => {}
        }
        let expr = if self.declared(name) && !(self.is_fact(name) && self.sees_facts()) { self.held_by(name, expr) } else { expr };
        let before = self.ctx.vars.get(name).cloned();
        self.rebind(name, &kept(&expr));
        // (converted before a local bound anew is, too: the conversions had
        // come after that early return, so a list that can fail bound anew
        // was given a tensor, which did not build)
        // an int assigned to a float variable is a float (spec §1): the
        // conversion is explicit in the form
        // (a variable holding a float that can fail takes an int as a
        // float too: `cur = loss[…]`, then `cur = 2`)
        let holds_float = matches!(self.ctx.vars.get(name), Some(Ty::Float)) || matches!(self.ctx.vars.get(name), Some(Ty::Result(inner)) if matches!(inner.as_ref(), Ty::Float));
        let expr = if holds_float && expr.ty() == Ty::Int {
            Expr::Convert { how: Convert::IntToFloat, expr: Box::new(expr), ty: Ty::Float }
        } else {
            expr
        };
        // numbers held as a tensor assigned to a variable that holds a list,
        // or the other way, are converted where they meet: `xs = [1.5]` into
        // a list the program grows (compiled, the tensor did not build into
        // the `Vec`, and the interpreter held a tensor where the IR said a
        // list)
        // (a value that can fail converts as its payload, its failure kept)
        let as_held = |held: &Ty, failing: bool| if failing { Ty::Result(Box::new((held).clone())) } else { (held).clone() };
        let failing = expr.ty().is_result();
        // (a variable that holds a value that can fail is read by what it
        // holds: a list that can fail given a tensor that can converts too)
        let held = self.ctx.vars.get(name).map(|t| match t {
            Ty::Result(inner) => inner.as_ref(),
            t => t,
        });
        let expr = match (held, expr.ty().payload().clone()) {
            (Some(var @ Ty::List(_)), Ty::Tensor(..)) => {
                let ty = as_held(var, failing);
                Expr::Convert { how: Convert::ToList, expr: Box::new(expr), ty }
            }
            (Some(var @ Ty::Tensor(..)), Ty::List(e)) if matches!(*e, Ty::Int | Ty::Float) => {
                let ty = as_held(var, failing);
                Expr::Convert { how: Convert::ToTensor, expr: Box::new(expr), ty }
            }
            // (ints held as a list assigned to a variable holding a list of
            // floats — `t = [1.0, 2.0]`, then `t = range[3]` — are floats,
            // as one int assigned to a float variable is; and a map of ints
            // held as one of floats — written a float later, `b['a'] = 2.5` —
            // is one of floats from its declaration: it had been declared a
            // map of ints, and not built — crow check --self; at any depth: a
            // grid of ints one of whose numbers is written a float, `g[0][0]
            // = 2.5`, holds rows of floats — Rosetta Code, Abelian sandpile)
            (Some(var @ (Ty::List(_) | Ty::Map(..))), given) if crate::ir::floats_of_ints(var, &given, false) => {
                let ty = as_held(var, failing);
                Expr::Convert { how: Convert::IntsToFloats, expr: Box::new(expr), ty }
            }
            // (ints where records or none are held: each 0 none)
            (Some(var @ Ty::List(i)), Ty::List(e)) if !failing && matches!(i.as_ref(), Ty::OptRecord(_)) && *e == Ty::Int => nones_of(expr, var),
            _ => expr,
        };
        // a frame given other columns in the block it was bound in is bound
        // anew there (a Rust `let` shadows it the same way), when it is held
        // the same way — a frame, or one that can fail
        let rebinding = match (&before, self.ctx.vars.get(name)) {
            (Some(was), Some(now)) => {
                self.scopes.last().is_some_and(|s| s.contains(name))
                    && !(self.is_fact(name) && self.sees_facts())
                    && was.detail().is_some()
                    && was.detail() != now.detail()
                    && expr.ty().is_result() == matches!(now, Ty::Result(_))
            }
            _ => false,
        };
        if rebinding {
            let ty = (self.ctx.vars.get(name).unwrap_or(&Ty::Unknown)).clone();
            return Stmt::Let { var: name.to_string(), ty, expr };
        }
        if self.is_fact(name) && self.sees_facts() {
            return Stmt::Store { fact: name.to_string(), expr };
        }
        if self.declared(name) {
            let held = self.ctx.vars.get(name).cloned();
            let expr = match held {
                Some(held) => settled_yield(expr, &held),
                None => expr,
            };
            return Stmt::Set { var: name.to_string(), expr };
        }
        self.scopes.last_mut().unwrap().insert(name.to_string());
        // what reads the local from here on is typed by the local as
        // declared: its first value's type, or the type it settles on
        let ty = declared_as(&expr.ty(), self.ctx.vars.get(name));
        let expr = match (&ty, expr.ty()) {
            (Ty::List(i), Ty::List(e)) if matches!(i.as_ref(), Ty::OptRecord(_)) && *e == Ty::Int => nones_of(expr, &ty),
            _ => expr,
        };
        if !matches!(ty, Ty::Unknown) {
            // (a column keeps its cells' kind: `texts = str[t.at]` is read
            // as a column of text from here on, not of cells unknown)
            self.ctx.vars.insert(name.to_string(), keep_cells(&ty));
        }
        let expr = settled_yield(expr, &ty);
        Stmt::Let { var: name.to_string(), ty, expr }
    }

    /// A value that can fail, assigned to a variable whose first value
    /// could not: the variable holds no error (spec, Reassignment), so a
    /// failed value is refused where it is assigned, as a decision refuses
    /// one, and its payload lands.
    fn held_by(&mut self, name: &str, expr: Expr) -> Expr {
        let payload = match expr.ty() {
            Ty::Result(p) => *p,
            _ => return expr,
        };
        let held = match self.ctx.vars.get(name) {
            Some(t) => t.clone(),
            None => return expr,
        };
        if !crate::type_infer::settle::refuses_failed(&held, &as_inferred(&payload)) {
            return expr;
        }
        let at = Decision::Assign(name.to_string());
        // `xs = push[xs, float[s]]`: the list holds no error, so only an item
        // can fail; the item is refused before the call, and the list still
        // grows where it lies
        if let Expr::Call { target: target @ (Target::Runtime(_) | Target::Row(_)), args, ty: _, propagates } = &expr {
            let own = matches!(args.first(), Some(Expr::Local(v, _)) if v == name);
            let in_place = match target {
                Target::Row(row) => row.takes_in_place() || (row.module.is_empty() && in_place_builtin(row.name)),
                Target::Runtime(path) => in_place_builtin(path),
                _ => false,
            };
            if own && in_place {
                let mut out = vec![args[0].clone()];
                let mut failing = Vec::new();
                for a in &args[1..] {
                    match a {
                        Expr::Convert { how: Convert::Unwrap, expr: inner, ty } if inner.ty().is_result() => {
                            let t = self.fresh();
                            self.pre.push(Stmt::Let { var: t.clone(), ty: inner.ty(), expr: (**inner).clone() });
                            self.scopes.last_mut().unwrap().insert(t.clone());
                            // known good once refused: nothing propagates from it
                            out.push(Expr::Convert { how: Convert::Known, expr: Box::new(Expr::Local(t.clone(), inner.ty())), ty: ty.clone() });
                            failing.push(t);
                        }
                        other => out.push(other.clone()),
                    }
                }
                if !failing.is_empty() {
                    self.saw_refusal(&at);
                    self.pre.push(Stmt::Propagate { values: failing, at });
                    return Expr::Call { target: target.clone(), args: out, ty: payload, propagates: *propagates };
                }
            }
        }
        self.decided(expr, at)
    }

    /// An error leaving the body, for a provisional lowering's observer.
    /// A helper called for what it writes — its value dropped — handed a
    /// value that can fail: the value is refused before the call, so a
    /// handler stops with its reason, as it does for an effect given one
    /// (docs/bug-classes.md, decision 1). By auto-propagation the error was
    /// the call's value, which nobody read: the click went on without the
    /// helper's writes and said nothing (and the modes told the click's
    /// own result apart).
    fn refuse_before_helper(&mut self, x: &mut Expr) {
        if !matches!(self.at, Where::In(FnKind::Handler | FnKind::Helper)) {
            return;
        }
        let Expr::Call { target: Target::User(callee), args, propagates: true, .. } = x else {
            return;
        };
        let Some(body) = self.ctx.functions.get(callee.as_str()).map(|sig| sig.body.clone()) else {
            return;
        };
        if !matches!(self.kind_of(callee, &body, false), FnKind::Handler | FnKind::Helper) {
            return;
        }
        let mut failing = Vec::new();
        for a in args.iter_mut() {
            if let Expr::Convert { how: Convert::Unwrap, expr: inner, ty } = a {
                if inner.ty().is_result() {
                    let t = self.fresh();
                    self.pre.push(Stmt::Let { var: t.clone(), ty: inner.ty(), expr: (**inner).clone() });
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(t.clone());
                    }
                    // known good once refused: nothing propagates from it
                    *a = Expr::Convert { how: Convert::Known, expr: Box::new(Expr::Local(t.clone(), inner.ty())), ty: ty.clone() };
                    failing.push(t);
                }
            }
        }
        if !failing.is_empty() {
            self.saw_refusal(&Decision::Branch);
            self.pre.push(Stmt::Propagate { values: failing, at: Decision::Branch });
        }
    }

    fn saw_refusal(&mut self, at: &Decision) {
        let collecting = self.collecting > 0;
        if let Some(seen) = &mut self.seen {
            seen.refusals.push((at.clone(), collecting));
        }
    }

    /// A provisional lowering's assignment (F5): the variable's type
    /// settles with its value's, as inference's walk settles it — a
    /// function's first assignment of a name binds it with the value's type
    /// (spec §7), a later one joins it — and the scopes are kept as the
    /// final lowering keeps them. A failed value's refusal is observed
    /// (`held_by`), the value's own type settling.
    fn settled(&mut self, name: &str, expr: Expr) -> Stmt {
        let ty = self.held_by_fact(name, kept(&expr));
        let store = self.is_fact(name) && self.sees_facts();
        self.note_held(name, &ty, store);
        let settled = if self.ctx.fresh_binding(name) {
            ty.clone()
        } else {
            let existing = self.ctx.vars.get(name).cloned().unwrap_or(Ty::Unknown);
            crate::type_infer::settle::settle(&existing, ty.clone(), self.ctx.body_assigned.is_some())
        };
        self.ctx.vars.insert(name.to_string(), settled.clone());
        // (a holder this local was pushed onto before its items were known
        // holds what it settles on)
        if settled.is_concrete() {
            let holders: Vec<String> = self.items_of.iter().filter(|(f, _, i)| *f == self.current_fn && i == name).map(|(_, h, _)| h.clone()).collect();
            for h in holders {
                let joined = match self.ctx.vars.get(&h) {
                    Some(Ty::List(x)) if x.has_unknown() && x.compatible(&settled) => Some(Ty::List(Box::new(x.unify(&settled)))),
                    Some(Ty::Map(k, x)) if x.has_unknown() && x.compatible(&settled) => Some(Ty::Map(*k, Box::new(x.unify(&settled)))),
                    _ => None,
                };
                if let Some(t) = joined {
                    self.ctx.vars.insert(h, t);
                }
            }
        }
        self.rebind(name, &ty);
        if !store && !self.declared(name) {
            self.scopes.last_mut().unwrap().insert(name.to_string());
        }
        Stmt::Set { var: name.to_string(), expr }
    }

    /// What a provisional lowering's assignment says of the variable's
    /// declaration: a new local's first value's type is kept, and a value
    /// that can fail, given to a variable declared before, is held to be
    /// judged once the body has settled (`provisional`).
    fn note_held(&mut self, name: &str, ty: &Ty, store: bool) {
        if store {
            return;
        }
        if self.declared(name) {
            if let Ty::Result(payload) = ty {
                self.held.push((name.to_string(), payload.as_ref().clone(), self.collecting > 0));
            }
        } else {
            self.first_types.entry(name.to_string()).or_insert_with(|| ty.clone());
        }
    }

    /// A provisional lowering's `a, b = x, y` (F5): each name settles with
    /// its own value's type read as text, as inference's walk pairs them,
    /// and no name is a new binding there.
    fn settled_multi(&mut self, names: &[String], values: &[A]) -> Result<Vec<Stmt>, String> {
        let mut types = Vec::with_capacity(values.len());
        let mut fulls = Vec::with_capacity(values.len());
        for v in values {
            let x = self.lower_expr(v)?;
            types.push(read(&x));
            fulls.push(full(&x));
        }
        if names.len() == values.len() {
            for (n, t) in names.iter().zip(&fulls) {
                let store = self.is_fact(n) && self.sees_facts();
                self.note_held(n, t, store);
            }
        }
        let in_body = self.ctx.body_assigned.is_some();
        for (n, t) in names.iter().zip(types) {
            let existing = self.ctx.vars.get(n).cloned().unwrap_or(Ty::Unknown);
            self.ctx.vars.insert(n.clone(), crate::type_infer::settle::settle(&existing, t, in_body));
        }
        for n in names {
            let store = self.is_fact(n) && self.sees_facts();
            if !store && !self.declared(n) {
                self.scopes.last_mut().unwrap().insert(n.clone());
            }
        }
        Ok(vec![])
    }

    /// The call scan's assignment (F5): the variable settled before its
    /// value was lowered (`note_scanned_assign`); the scopes are kept as the
    /// final lowering keeps them.
    fn scanned(&mut self, name: &str, expr: Expr) -> Stmt {
        let store = self.is_fact(name) && self.sees_facts();
        if !store && !self.declared(name) {
            self.scopes.last_mut().unwrap().insert(name.to_string());
        }
        Stmt::Set { var: name.to_string(), expr }
    }

    /// The call scan's `a, b = x, y` (F5): once the values' calls are
    /// read, each name settles with its own value's type read as text, as
    /// the scan pairs them.
    fn scanned_multi(&mut self, names: &[String], values: &[A]) -> Result<Vec<Stmt>, String> {
        let mut types = Vec::with_capacity(values.len());
        for v in values {
            let x = self.lower_expr(v)?;
            types.push(read(&x));
        }
        let in_body = self.ctx.body_assigned.is_some();
        for (n, t) in names.iter().zip(types) {
            let existing = self.ctx.vars.get(n).cloned().unwrap_or(Ty::Unknown);
            self.ctx.vars.insert(n.clone(), crate::type_infer::settle::settle(&existing, t, in_body));
        }
        for n in names {
            let store = self.is_fact(n) && self.sees_facts();
            if !store && !self.declared(n) {
                self.scopes.last_mut().unwrap().insert(n.clone());
            }
        }
        Ok(vec![])
    }

    /// A provisional lowering's `m[k] = v` (F5): a map's values take the
    /// value's type, as inference's walk has it.
    fn settled_entry(&mut self, object: &A, value: &Expr, index: &Expr) {
        // (a float written at a nested position — `g[0][0] = 2.5` into a
        // list of lists of ints — widens the innermost items, as a list's
        // own item does: the compiled program had not built, and the
        // interpreter kept the float among ints; Rosetta Code, Abelian
        // sandpile model)
        if let A::Call { callee, args } = object
            && args.len() == 1
            && read(value).payload() == &Ty::Float
        {
            let mut depth = 2;
            let mut root = callee.as_ref();
            while let A::Call { callee, args } = root
                && args.len() == 1
            {
                depth += 1;
                root = callee.as_ref();
            }
            fn widened(t: &Ty, depth: usize) -> Option<Ty> {
                match (t, depth) {
                    (Ty::Int, 0) => Some(Ty::Float),
                    (Ty::List(item), d) if d > 0 => widened(item, d - 1).map(|w| Ty::List(Box::new(w))),
                    _ => None,
                }
            }
            if let A::Ident(var) = root
                && let Some(w) = widened(&self.ctx.ident_type(var).text(), depth)
            {
                self.ctx.vars.insert(var.clone(), w);
            }
            return;
        }
        // (an item written at a nested position into collections whose
        // innermost items nothing typed yet — `m['a'] = map[]` then
        // `m['a']['n'] = 1` — types those items, and an open map's keys, as
        // a write one level down does: the map of empty maps had stayed one
        // of maps of nothing known, compiled as text, and the program did
        // not build; Rosetta Code, Random sentence from book)
        if let A::Call { args, .. } = object
            && args.len() == 1
        {
            let mut depth = 1;
            let mut root = object;
            while let A::Call { callee, args } = root
                && args.len() == 1
            {
                root = callee.as_ref();
                depth += 1;
            }
            fn filled(t: &Ty, depth: usize, leaf: &Ty, key: crate::ir::Key) -> Option<Ty> {
                match (t, depth) {
                    (Ty::Map(k, item), 1) if item.has_unknown() && item.compatible(leaf) => Some(Ty::Map(k.unify(key).unwrap_or(*k), Box::new(item.unify(leaf)))),
                    (Ty::List(item), 1) if item.has_unknown() && item.compatible(leaf) => Some(Ty::List(Box::new(item.unify(leaf)))),
                    (Ty::Map(k, inner), d) if d > 1 => filled(inner, d - 1, leaf, key).map(|i| Ty::Map(*k, Box::new(i))),
                    (Ty::List(inner), d) if d > 1 => filled(inner, d - 1, leaf, key).map(|i| Ty::List(Box::new(i))),
                    _ => None,
                }
            }
            if let A::Ident(var) = root
                && let Some(t) = filled(&self.ctx.ident_type(var).text(), depth, &read(value), crate::type_infer::key_kind_of(read(index).payload()))
            {
                self.ctx.vars.insert(var.clone(), t);
                return;
            }
        }
        // (an item written through a record's field — `s.lists[1] = xs`
        // into a field made a list of `[]`s — types the field's items, as
        // a local's item write does: the field had stayed a list of lists
        // of nothing, compiled as lists of text; Rosetta Code, Christofides
        // algorithm)
        if let A::FieldAccess { object: record, field } = object {
            let rec = match self.target_type(record).payload() {
                Ty::Record(r) | Ty::OptRecord(r) => r.clone(),
                _ => return,
            };
            let given = read(value).payload().clone();
            if let Some(sig) = self.ctx.structs.get_mut(&rec)
                && let Some((_, held)) = sig.fields.iter_mut().find(|(n, _)| n == field)
            {
                let typed = match &*held {
                    Ty::List(item) if item.has_unknown() && item.compatible(&given) => Some(Ty::List(Box::new(item.unify(&given)))),
                    Ty::Map(k, item) if item.has_unknown() && item.compatible(&given) => Some(Ty::Map(*k, Box::new(item.unify(&given)))),
                    _ => None,
                };
                if let Some(t) = typed {
                    *held = t;
                }
            }
            return;
        }
        if let A::Ident(var) = object {
            match self.ctx.ident_type(var).text() {
                Ty::Map(key, inner) => {
                    // (a value that can fail, written into a map of plain
                    // values, is decided where it is written — the map
                    // holds its payload, the failure handed back or refused
                    // (spec, Reassignment): joined, the values had become
                    // ones that can fail, a function answering the map was
                    // typed so, and the top level refused the map's first
                    // line as a reassignment; Rosetta Code, Terminal
                    // control/Coloured text)
                    let given = read(value);
                    let given = if !inner.is_result() && given.is_result() { given.payload().clone() } else { given };
                    let joined = inner.unify(&given);
                    // (an open map's first key decides its keys' kind, as
                    // `set` decides it: `m = map[]` then `m[1] = 'one'` had
                    // kept the map keyed by text — the interpreter stopped
                    // and the compiled program did not build; Rosetta Code,
                    // Achilles numbers)
                    let key = key.unify(crate::type_infer::key_kind_of(read(index).payload())).unwrap_or(key);
                    self.ctx.vars.insert(var.clone(), Ty::Map(key, Box::new(joined)));
                }
                // a list of ints an item of which is written a float holds
                // floats, as a map's values do (spec §1; `c[0] = 2.5` had
                // been a list of ints holding a float — crow check --self)
                Ty::List(inner) if *inner == Ty::Int && read(value).payload() == &Ty::Float => {
                    self.ctx.vars.insert(var.clone(), Ty::List(Box::new(Ty::Float)));
                }
                // (and one an item of which is written a record holds records
                // or none, its 0s none — empty buckets made of zeros, as a
                // variable given 0 and a record does; Vijay, 2026-10-29:
                // Rosetta Code, Monads/List monad)
                Ty::List(inner) if *inner == Ty::Int && matches!(read(value).payload(), Ty::Record(_) | Ty::OptRecord(_)) => {
                    if let Ty::Record(r) | Ty::OptRecord(r) = read(value).payload() {
                        self.ctx.vars.insert(var.clone(), Ty::List(Box::new(Ty::OptRecord(r.clone()))));
                    }
                }
                // (and so does whole numbers written down, as a vector, at
                // the top level, whose lines are scanned one at a time: the
                // final pass made the literal a list of ints, written by
                // position, and the float met it unwidened — an internal
                // error; in a function the body settles whole; Rosetta Code,
                // Gotchas)
                Ty::Tensor(e, d) if *e == Ty::Int && d.as_ref().is_none_or(|d| d.len() <= 1) && read(value).payload() == &Ty::Float => {
                    self.ctx.vars.insert(var.clone(), Ty::Tensor(Box::new(Ty::Float), d));
                }
                _ => {}
            }
        }
    }

    /// A provisional lowering's `r.f = v`: a record's field that holds
    /// something not known yet — one given `[]` or `map[]` where the record
    /// was made — takes what the value written knows, as a variable first
    /// given `[]` takes its pushes' kind; every later pass reads it, and the
    /// record is declared so (`b.items = push[b.items, i]` after `Bag[[],
    /// 0]` had left the field a list of nothing, declared a list of text
    /// compiled, and the program did not build; interpreted it ran).
    fn settled_field(&mut self, object: &A, field: &str, value: &Expr) {
        let rec = match self.target_type(object).payload() {
            Ty::Record(r) | Ty::OptRecord(r) => r.clone(),
            _ => return,
        };
        let given = read(value).payload().clone();
        // (a field every constructor gave 0, none, written a record of its
        // own kind is a record or none: typed an int by the constructors
        // alone, `n.next = m` had not built compiled; Rosetta Code, Deepcopy)
        if let (Ty::Record(given_rec), Some(sig)) = (&given, self.ctx.structs.get_mut(&rec))
            && *given_rec == rec
            && let Some((_, held)) = sig.fields.iter_mut().find(|(n, _)| n == field)
            && *held == Ty::Int
        {
            *held = Ty::OptRecord(rec.clone());
            return;
        }
        if let Some(sig) = self.ctx.structs.get_mut(&rec)
            && let Some((_, held)) = sig.fields.iter_mut().find(|(n, _)| n == field)
            && held.has_unknown()
            && held.compatible(&given)
        {
            let joined = held.unify(&given);
            if joined != *held {
                *held = joined;
            }
        }
        // (a field holds what it is given, a failed value included (spec,
        // Structs: `b.speed.ok` asks the field), written after the record
        // was made as where it was made: a field made plain had stayed so,
        // `c.v.ok` was true of a failure interpreted, and the compiled
        // program did not build; Rosetta Code, Compiler/AST interpreter)
        if read(value).is_result()
            && let Some(sig) = self.ctx.structs.get_mut(&rec)
            && let Some((_, held)) = sig.fields.iter_mut().find(|(n, _)| n == field)
            && !held.is_result()
            && !held.has_unknown()
            && held.compatible(&given)
        {
            *held = Ty::Result(Box::new(held.unify(&given)));
        }
    }

    /// A provisional lowering's `return x` of a local whose type holds
    /// something not known yet: it takes what the function answers, as a
    /// variable first given `[]` takes its pushes' kind — the local is
    /// empty of the kind nothing wrote, so the answer's kind is its own.
    fn settled_return(&mut self, e: &A) {
        let A::Ident(v) = e else { return };
        if self.is_fact(v) {
            return;
        }
        // (a parameter is typed by its calls and by how the body reads it,
        // never by what the function answers: `return points` beside
        // `return singles[points]` had made the parameter a list of the
        // lists `singles` answers, and its items' field reads were refused
        // in another function; Rosetta Code, Weiler-Atherton polygon
        // clipping)
        if self.ctx.functions.get(&self.current_fn).is_some_and(|sig| sig.params.iter().any(|(p, _)| p == v)) {
            return;
        }
        let Some(held) = self.ctx.vars.get(v).cloned() else { return };
        if !held.has_unknown() {
            return;
        }
        let Some(answer) = self.ctx.functions.get(&self.current_fn).map(|sig| sig.return_type.payload().clone()) else { return };
        // (an answer of no value — an early round's, before the body's
        // returns were typed — says nothing of the local: joined, it had made
        // magpie's `by` no value, written back to the parameter, and the
        // lowered program disagreed with itself)
        if answer.has_unknown() || matches!(answer, Ty::Unit) || !held.payload().compatible(&answer) {
            return;
        }
        let joined = match &held {
            Ty::Result(inner) => Ty::Result(Box::new(inner.unify(&answer))),
            other => other.unify(&answer),
        };
        self.ctx.vars.insert(v.clone(), joined);
    }

    /// A provisional lowering's `set[m, k, v]` as a statement (F5): the
    /// map's values take v's type, as inference's walk has it.
    fn settled_set(&mut self, e: &A, x: &Expr) {
        let A::Call { callee, args } = e else { return };
        let (true, [A::Ident(var), _, _]) = (matches!(callee.as_ref(), A::Ident(n) if n == "set"), args.as_slice()) else { return };
        let value = match x {
            Expr::Call { args, .. } | Expr::Effect { args, .. } => match args.get(2) {
                // the value as written, before its failure was unwrapped
                Some(Expr::Convert { how: Convert::Unwrap, expr, .. }) => read(expr),
                Some(v) => read(v),
                None => return,
            },
            _ => return,
        };
        let existing = self.ctx.vars.get(var).cloned().unwrap_or(Ty::Unknown);
        // (open: the variable's own key kind stays, a value settles none)
        self.ctx.vars.insert(var.clone(), existing.unify(&Ty::Map(crate::ir::Key::Open, Box::new(value))));
    }

    /// While a loop's body is lowered, its variable has the element type
    /// of what it iterates (inference typed the body in a copy of its
    /// context, so the lowering's context has not seen the variable) — a
    /// binding of its own even where that element is not known, hiding a
    /// name bound outside the loop (`scan_loop_var` says why).
    fn bind_loop_var(&mut self, var: &str, iter_ty: &Ty) -> Option<Ty> {
        let saved = self.ctx.vars.get(var).cloned();
        if matches!(iter_ty.payload(), Ty::List(e) if **e == Ty::Unknown) {
            self.v.unknown_items.insert(var.to_string());
        } else {
            self.v.unknown_items.remove(var);
        }
        let elem = match iter_ty.payload() {
            // a frame's column walks its cells, as its type says them (the
            // variable had been unknown here and a float cell in the IR, so
            // `n = n + cell` kept `n` an int that a float was given): the
            // cells read before the column's are forgotten (as_inferred)
            t @ Ty::Column(_) => as_inferred(&crate::type_infer::TypeContext::loop_item(t.clone())),
            t => crate::type_infer::TypeContext::loop_item(as_inferred(t)),
        };
        self.ctx.vars.insert(var.to_string(), elem);
        saved
    }

    fn unbind_loop_var(&mut self, var: &str, saved: Option<Ty>) {
        self.v.unknown_items.remove(var);
        match saved {
            Some(t) => {
                self.ctx.vars.insert(var.to_string(), t);
            }
            None => {
                self.ctx.vars.remove(var);
            }
        }
    }

    /// `x = set[x, k, v]` (or another builtin that answers the collection
    /// it takes, `Prop::InPlace`) whose later arguments read `x`
    /// (`counts = set[counts, w, counts[w] + 1]`): those arguments computed
    /// first, in order, into temporaries, so the call takes `x` where it
    /// lies in both backends — reading a variable has no effect, so the
    /// order is the program's. The map had been copied at every update of
    /// an entry it held (k-nucleotide's counts, four times per window).
    fn taken_in_place(&mut self, stmt: Stmt) -> Vec<Stmt> {
        let Stmt::Set { var, expr: Expr::Call { target: Target::Row(row), args, ty, propagates } } = &stmt else { return vec![stmt] };
        let reads_var = |e: &Expr| {
            let mut found = false;
            crate::ir::visit_expr(e, &mut |x| found |= matches!(x, Expr::Local(n, _) | Expr::Move(n, _) if n == var));
            found
        };
        let takes_var = matches!(args.first(), Some(Expr::Local(n, _) | Expr::Move(n, _)) if n == var);
        // (a tensor too: `d[i][j] = d[i - 1][j] + 1.0` reads `d` in the
        // value, which had made every write a copy of the whole matrix)
        // (any module's row: a bytes buffer's `set` and `push` are sys's,
        // and `b = set[b, i, byte[b, j]]` reads `b` in the value as a
        // tensor's write does)
        let taken_kind = |k: &crate::table::Kind| k.taken_whole() || matches!(k, crate::table::Kind::Tensor);
        if !(row.props.contains(&crate::table::Prop::InPlace) && row.params.first().is_some_and(taken_kind) && takes_var && args[1..].iter().any(reads_var)) {
            return vec![stmt];
        }
        let mut out = Vec::with_capacity(args.len() + 1);
        let mut taken = vec![args[0].clone()];
        for a in &args[1..] {
            // (a variable or a constant has no effect to keep in order, and
            // stays: copying it would cost what the hoist saves)
            if matches!(a, Expr::Const(..)) || matches!(a, Expr::Local(n, _) | Expr::Move(n, _) if n != var) {
                taken.push(a.clone());
                continue;
            }
            let t = self.fresh();
            let at = a.ty();
            out.push(Stmt::Let { var: t.clone(), ty: at.clone(), expr: a.clone() });
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(t.clone());
            }
            taken.push(Expr::Local(t, at));
        }
        out.push(Stmt::Set { var: var.clone(), expr: Expr::Call { target: Target::Row(row), args: taken, ty: ty.clone(), propagates: *propagates } });
        out
    }

    /// An int handed where a record or none is held — a field only ever
    /// given 0 (`t.right` of `Node[5, leaf, 0]`) handed to a function whose
    /// parameter holds a record or none: checked first, 0 being none and
    /// another int stopping, then handed as the 0 that writes none, in both
    /// modes (the lowered program had handed the int, and disagreed with
    /// itself).
    fn none_of(&mut self, x: Expr, slot: &Ty) -> Expr {
        // (an int that can fail has no place there: its failure would be
        // the call's, and the check is made before it — refused where it is
        // handed; compiled, it had not built — crow check --self)
        if x.ty().is_result() || matches!(&x, Expr::Convert { how: Convert::Unwrap, expr, .. } if expr.ty().is_result()) {
            self.refuse("a record or 0 (none) is held here, and this is an int that can fail: decide it first (`if v.ok`)".to_string());
            return Expr::Const(Const::Int(0), slot.clone());
        }
        let Some(row) = crate::table::ROWS.iter().find(|r| r.id == "core.__none") else { return x };
        self.pre.push(Stmt::Do(Expr::Call { target: Target::Row(row), args: vec![x], ty: Ty::Int, propagates: false }));
        Expr::Const(Const::Int(0), slot.clone())
    }

    fn fresh(&mut self) -> String {
        self.temp += 1;
        // (named as no function of the program's is: a program with a
        // function `t1` had its value shadowed by the temporary, compiled)
        format!("_t{}", self.temp)
    }

    /// An assignment target's path, its variable and fields already
    /// checked (`check_target`); an index's expressions are checked as they
    /// are lowered.
    /// An assignment target's variable, its path, and the type of what the
    /// path names (`path_of`), its checks made where the target is read.
    fn muted_target(&mut self, object: &A) -> Result<(String, Vec<PathStep>, Ty), String> {
        match object {
            A::Ident(_) | A::FieldAccess { .. } => self.muted(|l| l.path_of(object)),
            _ => self.path_of(object),
        }
    }

    /// Where along an assignment's target a tensor starts, past a variable
    /// that is none: the first field of a record, item of a list or value of
    /// a map that holds one (`g.cells` of `g.cells[i] = row`, `ms[k]` of
    /// `ms[k][i] = row`, `mats['a']` of `mats['a'][i] = row`), as the type
    /// each step up to it reads — the last the tensor's. A target through
    /// anything else (a parsed value, an index of several positions) has
    /// none.
    fn tensor_lead(&mut self, object: &A, root: &Ty) -> Option<(usize, Vec<Ty>)> {
        fn chain<'b>(o: &'b A, out: &mut Vec<&'b A>) -> bool {
            let ok = match o {
                A::Ident(_) => true,
                A::FieldAccess { object, .. } => chain(object, out),
                A::Call { callee, args } if args.len() == 1 => chain(callee, out),
                _ => false,
            };
            out.push(o);
            ok
        }
        let mut nodes = Vec::new();
        if !chain(object, &mut nodes) {
            return None;
        }
        let mut types = Vec::new();
        let mut before = root.clone();
        for node in nodes.into_iter().skip(1) {
            let through = match node {
                A::FieldAccess { .. } => matches!(before, Ty::Record(_)),
                _ => matches!(before, Ty::List(_) | Ty::Map(..)),
            };
            if !through {
                return None;
            }
            let ty = self.target_type(node);
            types.push(ty.clone());
            if matches!(ty, Ty::Tensor(..)) {
                return Some((types.len(), types));
            }
            before = ty;
        }
        None
    }

    /// An assignment target's variable, its path, and the type of what the
    /// path names, by inference's rules along it (read as text).
    fn path_of(&mut self, object: &A) -> Result<(String, Vec<PathStep>, Ty), String> {
        match object {
            A::Ident(n) => {
                self.check_ident(n);
                Ok((n.clone(), vec![], self.ctx.ident_type(n).text()))
            }
            A::FieldAccess { object, field } => {
                let (v, mut p, object_type) = self.path_of(object)?;
                self.check_field(&object_type, field);
                let ty = self.ctx.field_type(object, object_type, field).text();
                p.push(PathStep::Field(field.clone()));
                Ok((v, p, ty))
            }
            A::Call { callee, args } => {
                // a name indexed is checked as the call it is written as
                let (v, mut p, callee_type) = match callee.as_ref() {
                    A::Ident(_) => self.muted(|l| l.path_of(callee))?,
                    _ => self.path_of(callee)?,
                };
                let mut arg_types = Vec::with_capacity(args.len());
                let mut types = Vec::with_capacity(args.len());
                let mut checks = Vec::with_capacity(args.len());
                for a in args {
                    let i = self.lower_expr(a)?;
                    let i = self.decided_index(i);
                    arg_types.push(read(&i));
                    types.push(full(&i));
                    checks.push(self.checked(&i));
                    p.push(PathStep::Index(i));
                }
                self.check_callee(callee, args, &checks);
                self.scan_call(callee, args, &types);
                let ty = as_inferred(&self.call_type(callee, args, &arg_types, &types, Some(callee_type))).text();
                Ok((v, p, ty))
            }
            _ => Err("lowering: an assignment target must be a variable, a field or an index".into()),
        }
    }

    /// What an assignment's target names, before its value is lowered:
    /// inference's rules along the path, an index typed by a lowering of it
    /// that keeps nothing (its own checks are made when it is lowered).
    fn target_type(&mut self, e: &A) -> Ty {
        match e {
            A::Ident(n) => self.ctx.ident_type(n).text(),
            A::FieldAccess { object, field } => {
                let object_type = self.target_type(object);
                self.ctx.field_type(object, object_type, field).text()
            }
            _ => {
                let (pre, temp) = (std::mem::take(&mut self.pre), self.temp);
                self.unscanned += 1;
                let t = self.muted(|l| l.lower_expr(e)).map(|x| read(&x)).unwrap_or(Ty::Unknown);
                self.unscanned -= 1;
                self.pre = pre;
                self.temp = temp;
                t
            }
        }
    }

    // ── expressions ─────────────────────────────────────────────────

    /// An expression lowered and typed: each node is typed from its
    /// lowered children by inference's rules, markup told apart from text (a
    /// view places the one as markup and the other as text, as the
    /// interpreter's values do).
    fn lower_expr(&mut self, e: &A) -> Result<Expr, String> {
        Ok(match e {
            A::Int(n) => Expr::Const(Const::Int(*n), Ty::Int),
            A::Float(f) => Expr::Const(Const::Float(*f), Ty::Float),
            A::Bool(b) => Expr::Const(Const::Bool(*b), Ty::Bool),
            A::Str(s) => {
                if s.contains('`') {
                    Expr::Interp(self.interpolation(s)?)
                } else {
                    Expr::Const(Const::Str(s.replace(crate::lexer::BACKSLASH_MARK, "")), Ty::Str)
                }
            }
            A::Ident(name) => {
                self.check_ident(name);
                let ty = (&self.ctx.ident_type(name)).clone();
                self.ident(name, ty)
            }
            A::BinOp { left, op, right } => {
                let joins = matches!(op, BinOp::And | BinOp::Or);
                let mut l = self.lower_expr(left)?;
                // (a frame's column of bools joined by and/or is the mask it
                // makes, `c eq true`, as wherever a mask is taken: `f.on and
                // f.n gt 1` had joined a column with a mask, which ran
                // interpreted and did not build compiled)
                if joins && is_bool_column(&l, false) {
                    l = self.lower_expr(&as_mask(left))?;
                }
                // (what the right side needs done first — a failed value's
                // refusal — waits for the left: under its decision for `and`/
                // `or` of two truths (`r.ok and r.x gt 0` must not refuse `r`
                // when it failed), behind its value for an operator that
                // computes both sides)
                let outer = std::mem::take(&mut self.pre);
                let mut r = self.lower_expr(right)?;
                if joins && is_bool_column(&r, false) {
                    r = self.lower_expr(&as_mask(right))?;
                }
                let right_pre = std::mem::replace(&mut self.pre, outer);
                // `[]` compared is a list of the other side's kind, or of ints
                // where nothing says (`[] eq []` had left the compiled program
                // no type to give it — crow check --self)
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    let empty = |x: &Expr| matches!(x, Expr::ListLit(items, Ty::List(e)) if items.is_empty() && **e == Ty::Unknown);
                    let kind_of = |x: &Expr| match x.ty().payload() {
                        Ty::List(e) if **e != Ty::Unknown => Ty::List(e.clone()),
                        _ => Ty::List(Box::new(Ty::Int)),
                    };
                    if empty(&l) {
                        let t = kind_of(&r);
                        l = Expr::ListLit(Vec::new(), t);
                    }
                    if empty(&r) {
                        let t = kind_of(&l);
                        r = Expr::ListLit(Vec::new(), t);
                    }
                }
                // a parsed value met in an operator is read as the kind the
                // other side says — a number as a float, `%` an int — and as
                // a bool by and/or; two parsed values compare as the program
                // runs
                let (lj, rj) = (matches!(l.ty().payload(), Ty::Json), matches!(r.ty().payload(), Ty::Json));
                if lj != rj {
                    let other = if lj { r.ty().payload().clone() } else { l.ty().payload().clone() };
                    let arithmetic = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow);
                    let kind = match op {
                        BinOp::And | BinOp::Or => Some(Ty::Bool),
                        BinOp::Mod => Some(Ty::Int),
                        _ => match other {
                            Ty::Int | Ty::Float => Some(Ty::Float),
                            Ty::Str | Ty::Markup => Some(Ty::Str),
                            Ty::Bool => Some(Ty::Bool),
                            Ty::Moment => Some(Ty::Moment),
                            _ if arithmetic => Some(Ty::Float),
                            _ => None,
                        },
                    };
                    if let Some(kind) = kind {
                        let how = format!("met in {:?}", op).to_lowercase();
                        if lj {
                            l = self.read_json(l, &kind, &how);
                        } else {
                            r = self.read_json(r, &kind, &how);
                        }
                    }
                } else if lj && rj && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow | BinOp::Mod | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow | BinOp::And | BinOp::Or) {
                    // two parsed values met in arithmetic are both numbers
                    // (`%` ints), joined by and/or both bools; two compared,
                    // whole or in order, compare as the program runs
                    let kind = match op {
                        BinOp::And | BinOp::Or => Ty::Bool,
                        BinOp::Mod => Ty::Int,
                        _ => Ty::Float,
                    };
                    let how = format!("met in {:?}", op).to_lowercase();
                    l = self.read_json(l, &kind, &how);
                    r = self.read_json(r, &kind, &how);
                }
                // (a column with its cells' kind: `f.a and f.a` of ints had
                // passed as a column of cells not known, and not built)
                let with_cells = |s: &Self, x: &Expr| match x.ty() {
                    Ty::Column(cell) if !cell.has_unknown() => Ty::Column(cell),
                    _ => s.checked(x),
                };
                self.kindless_item(&l);
                self.kindless_item(&r);
                self.check_binop(op, with_cells(self, &l), with_cells(self, &r));
                // a record or none compared with the 0 that writes none is
                // the none test (spec, A Record or None): the 0 is typed as
                // the link it is compared with; any other record beside a
                // number is refused
                let none_test = matches!(op, BinOp::Eq | BinOp::Ne)
                    && ((matches!(l.ty(), Ty::Record(_) | Ty::OptRecord(_)) && matches!(r, Expr::Const(Const::Int(0), _)))
                        || (matches!(r.ty(), Ty::Record(_) | Ty::OptRecord(_)) && matches!(l, Expr::Const(Const::Int(0), _))));
                if none_test {
                    let link = |t: Ty| match t {
                        Ty::Record(n) | Ty::OptRecord(n) => Ty::OptRecord(n),
                        other => other,
                    };
                    if let Expr::Const(Const::Int(0), _) = &r {
                        r = Expr::Const(Const::Int(0), link(l.ty()));
                    } else {
                        l = Expr::Const(Const::Int(0), link(r.ty()));
                    }
                } else {
                    self.check_order(op, &l.ty(), &r.ty());
                }
                // a frame's column in arithmetic is its numbers, as it is
                // handed to a function that takes them: converted the same
                // way in both modes, a missing cell the reason it has none
                // (the interpreter refused `df.a + df.b`, the compiled
                // program read a missing cell as 0); a comparison is a
                // mask, whose missing cells are false (spec, Boolean Masks)
                if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow) {
                    // (a column of vectors is its matrix, a row per cell)
                    let numbers = |x: Expr| match x.ty() {
                        Ty::Column(_) => {
                            let ty = Ty::Tensor(Box::new(Ty::Float), shapes::stacked_dims(&x.ty()));
                            Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                        }
                        _ => x,
                    };
                    l = numbers(l);
                    r = numbers(r);
                }
                // `.*`, `./` and `.^` work on a tensor's numbers: a list of
                // numbers or of truths (1 and 0) is its tensor, and an int past
                // 64 bits its nearest float, as a tensor's numbers are floats
                // (written as they stood, they had not built — crow check
                // --self)
                let operand = |x: Expr| match x.ty() {
                    // (a list of numbers is a vector: one axis — of no known
                    // rank, `range[3] * 2.0` had been refused where a loop
                    // walked it; Rosetta Code, Map range)
                    Ty::List(item) if matches!(*item, Ty::Int | Ty::Float | Ty::Bool) => {
                        let ty = Ty::Tensor(Box::new(Ty::Float), Some(vec![None]));
                        Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                    }
                    Ty::Big => match crate::table::row("core.__float_of_big") {
                        Some(row) => Expr::Call { target: Target::Row(row), args: vec![x], ty: Ty::Float, propagates: false },
                        None => x,
                    },
                    _ => x,
                };
                if matches!(op, BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow) {
                    l = operand(l);
                    r = operand(r);
                }
                // `+`, `-`, `*` and `/` of a list of numbers (or of truths, 1
                // and 0) are its tensor's too: an all-numeric array is a
                // tensor (spec, Tensor Arithmetic — `[1, 2, 3] + 10`), and a
                // list grown by `push` had been refused (`cannot use '+' with
                // array[int] and int`) where `.*` took it — crow check
                // --self's reference found it
                if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
                    let listed = |x: &Expr| matches!(x.ty(), Ty::List(item) if matches!(*item, Ty::Int | Ty::Float | Ty::Bool));
                    let numbers = |x: &Expr| listed(x) || matches!(x.ty().payload(), Ty::Tensor(..) | Ty::Int | Ty::Float | Ty::Big);
                    if (listed(&l) && numbers(&r)) || (listed(&r) && numbers(&l)) {
                        l = operand(l);
                        r = operand(r);
                    }
                }
                // a list of numbers that can fail — each a map's read, a
                // function's answer that can — compared whole with a tensor
                // or a list of numbers is its numbers, as it is handed to a
                // function that takes them: a failed item stops the program
                // with its reason, in both modes (the interpreter had
                // compared the items, and the compiled program had not built)
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    let fallible_numbers = |x: &Expr| matches!(x.ty(), Ty::List(item) if item.is_result() && matches!(item.payload(), Ty::Int | Ty::Float));
                    // (the other side decided first where it can fail: a
                    // loaded column's numbers)
                    let numbers_whole = |x: &Expr| matches!(x.ty().payload(), Ty::Tensor(..)) || matches!(x.ty().payload(), Ty::List(item) if matches!(item.payload(), Ty::Int | Ty::Float));
                    if (fallible_numbers(&l) && numbers_whole(&r)) || (fallible_numbers(&r) && numbers_whole(&l)) {
                        let to_tensor = |x: Expr| match x.ty() {
                            Ty::List(_) => Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: Ty::vector(Ty::Float) },
                            _ => x,
                        };
                        l = to_tensor(l);
                        r = to_tensor(r);
                    }
                }
                self.check_shapes(op, &l, &r);
                // `%` is the ints' remainder (spec, Operators): a float on
                // either side is refused before the program runs (the
                // interpreter had stopped on it where the compiled program
                // computed a float's remainder)
                if matches!(op, BinOp::Mod) && self.pass == Pass::Final && (matches!(l.ty().payload(), Ty::Float) || matches!(r.ty().payload(), Ty::Float)) {
                    self.refuse("`%` takes ints and answers the remainder — round a float first (int[x], floor[x], round[x]), or use mod[t, d] on a tensor".to_string());
                }
                let ty = shapes::shaped(self.ctx.binop_type(op, read(&l), read(&r)), shapes::binary_dims(op, &l, &r));
                // a tensor compared with a number is a mask of its shape, by
                // its rank: one whose rank the program does not show is
                // refused, as a loop over one is, so both modes build the
                // mask the type says
                // (in a function lowered with a parameter nothing types — one
                // this program or test does not call — a rank is unknown for
                // that alone: its called copies are lowered at their types,
                // and checked there)
                let untyped_params = self.ctx.functions.get(&self.current_fn).is_some_and(|sig| sig.params.iter().any(|(_, t)| t.has_unknown()));
                if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte) && !untyped_params {
                    for (x, other) in [(&l, &r), (&r, &l)] {
                        if matches!(x.ty().payload(), Ty::Tensor(_, None)) && matches!(other.ty().payload(), Ty::Int | Ty::Float) {
                            self.check_rank_known(x, "a comparison with a number of");
                        }
                    }
                }
                let arithmetic = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow | BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte);
                if arithmetic && (exact_kind(&l.ty()) || exact_kind(&r.ty())) {
                    let (el, er) = self.exact_operands(op, l, r);
                    l = el;
                    r = er;
                } else if arithmetic {
                    match (l.ty().payload(), r.ty().payload()) {
                        (Ty::Int, Ty::Float) => l = Expr::Convert { how: Convert::IntToFloat, expr: Box::new(l), ty: Ty::Float },
                        (Ty::Float, Ty::Int) => r = Expr::Convert { how: Convert::IntToFloat, expr: Box::new(r), ty: Ty::Float },
                        _ => {}
                    }
                }
                let joined = Expr::Binary { op: format!("{:?}", op).to_lowercase(), l: Box::new(l), r: Box::new(r), ty };
                if right_pre.is_empty() {
                    joined
                } else {
                    self.right_side_waits(joined, right_pre)
                }
            }
            A::UnaryOp { op, expr } => {
                let mut x = self.lower_expr(expr)?;
                self.kindless_item(&x);
                // a frame's column of bools negated is the mask it makes,
                // negated: `not f.on` is `not (f.on eq true)`, one bool per
                // row (it had been typed one bool — refused where a mask is
                // taken, and `xs[not f.on]` had stopped the interpreter and
                // not built compiled)
                if matches!(op, crate::ast::UnaryOp::Not) && is_bool_column(&x, false) {
                    x = self.lower_expr(&as_mask(expr))?;
                }
                // a parsed value negated is read as a bool, one made negative
                // as a number
                let x = match op {
                    crate::ast::UnaryOp::Not => self.read_json(x, &Ty::Bool, "negated"),
                    crate::ast::UnaryOp::Neg => self.read_json(x, &Ty::Float, "made negative"),
                };
                // a frame's column made negative is its numbers, as one in
                // arithmetic is (the compiled program had not built `-f.a`),
                // and so is a list of numbers (a tensor's numbers made
                // negative); `not []` is the empty mask
                let x = match (op, x.ty().payload()) {
                    (crate::ast::UnaryOp::Neg, Ty::Column(_)) => {
                        let ty = if x.ty().is_result() { Ty::Result(Box::new(Ty::vector(Ty::Float))) } else { Ty::vector(Ty::Float) };
                        Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                    }
                    (crate::ast::UnaryOp::Neg, Ty::List(e)) if matches!(e.payload(), Ty::Int | Ty::Float) => {
                        let ty = if x.ty().is_result() { Ty::Result(Box::new(Ty::vector(Ty::Float))) } else { Ty::vector(Ty::Float) };
                        Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                    }
                    (crate::ast::UnaryOp::Not, Ty::List(e)) if **e == Ty::Unknown && matches!(&x, Expr::ListLit(items, _) if items.is_empty()) => {
                        Expr::ListLit(Vec::new(), Ty::List(Box::new(Ty::Bool)))
                    }
                    // a list whose items no type says, turned over, is read
                    // as a mask: a parameter handed `[]` where a truth was
                    // meant (`not every`) had been a list of text compiled,
                    // which `!` does not build (crow check --self; errors
                    // j82) — as a mask, its use as a condition is refused
                    // in both modes
                    (crate::ast::UnaryOp::Not, Ty::List(e)) if **e == Ty::Unknown => {
                        self.read_as_list_of(&x, Ty::Bool);
                        x
                    }
                    _ => x,
                };
                // `not` turns a truth over (or a mask, item by item), and a
                // minus sign makes a number negative (or a tensor's numbers):
                // `not 3` had been false interpreted and -4 compiled, and a
                // minus before text, a bool or a map had not built (crow
                // check --self)
                if self.pass == Pass::Final {
                    let t = x.ty().payload().clone();
                    let fits = match op {
                        crate::ast::UnaryOp::Not => matches!(t, Ty::Bool | Ty::Json | Ty::Unknown) || t.is_truths() || matches!(&t, Ty::List(i) | Ty::Column(i) if matches!(i.payload(), Ty::Bool | Ty::Unknown)),
                        crate::ast::UnaryOp::Neg => matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction | Ty::Json | Ty::Unknown | Ty::Tensor(..) | Ty::Column(_)) || matches!(&t, Ty::List(i) if matches!(i.payload(), Ty::Int | Ty::Float)),
                    };
                    if !fits {
                        match op {
                            crate::ast::UnaryOp::Not => self.refuse(format!("`not` turns a truth over — a bool, or a mask item by item — and this is {}: compare it first (not (x eq 0))", crate::table::Arg::of_type(&t).words())),
                            crate::ast::UnaryOp::Neg => self.refuse(format!("a minus sign makes a number negative, and this is {}", crate::table::Arg::of_type(&t).words())),
                        }
                    }
                }
                let ty = shapes::shaped(self.ctx.unary_type(op, read(&x)), x.ty().tensor_dims().cloned());
                Expr::Unary { op: format!("{:?}", op).to_lowercase(), expr: Box::new(x), ty }
            }
            A::Call { callee, args } => made_failure_answers(self.call(callee, args)?),
            // a column read by a name the program computes — `df.`name``,
            // `df.lag_`k`` — through the column read's row (found by its
            // property): the column `df.name` reads, its cells as they show
            // when it runs, as an open frame's unnamed column reads (a written
            // piece names no column: the whole name is the pieces' text)
            A::FieldAccess { object, field } if field.contains('`') => {
                let base = self.lower_expr(object)?;
                let base = if !self.in_fact_default && base.ty().is_result() && matches!(base.ty().payload(), Ty::Frame(_)) {
                    self.decided(base, Decision::Read(field.clone()))
                } else {
                    base
                };
                if self.pass == Pass::Final && !matches!(base.ty().payload(), Ty::Frame(_) | Ty::Unknown) {
                    self.refuse(format!(
                        "`.{}` names a frame's column by a computed name, and {} is {}: a record's fields are fixed before the program runs, and a map's key is read as `m[key]`",
                        field,
                        local_named(&base).map(|n| format!("'{}'", n)).unwrap_or_else(|| "this".to_string()),
                        crate::table::Arg::of_type(base.ty().payload()).words()
                    ));
                }
                let name = Expr::Interp(self.interpolation(field)?);
                // (a frame that can fail, read as its payload, is an unwrap
                // mark the emitter reads only in a call that propagates, as a
                // pick's is: typed by the mark's payload, the call had not
                // propagated, and `g.`c`` of a summary's frame did not build
                // compiled — parity 381)
                let carried = matches!(base, Expr::Convert { how: Convert::Unwrap, .. }) || base.ty().is_result();
                let ty = Ty::Column(Box::new(Ty::Json));
                let ty = if carried { Ty::Result(Box::new(ty)) } else { ty };
                Expr::Call { target: Target::Row(crate::table::column_read()), args: vec![base, name], ty, propagates: carried }
            }
            A::FieldAccess { object, field } => {
                // `m.x` where `m` is no variable names what a part declares:
                // `m` is not read as one
                let member = matches!(object.as_ref(), A::Ident(m) if self.names_module_member(m, field));
                match field.as_str() {
                    "ok" | "reason" | "kind" | "trace" => {
                        let base = if member { self.muted(|l| l.lower_expr(object))? } else { self.lower_expr(object)? };
                        // a frame that holds a column of the name: the dot is
                        // the frame's status, never the column — a misread
                        // refused where it is written (a chart's `kind`, a
                        // log's `reason`, a file's `ok` are names data has;
                        // `describe`'s had been `kind`, read as nothing)
                        if self.pass == Pass::Final && matches!(base.ty().payload(), Ty::Frame(Some(c)) if c.cols.contains_key(field.as_str())) {
                            let name = local_named(&base).unwrap_or_else(|| "the frame".to_string());
                            self.refuse(format!(
                                "{0} holds a column named '{1}', and `{0}.{1}` is the frame's status, not that column (.ok, .reason, .kind and .trace are every value's) — rename the column to read it: rename[{0}, '{1}', 'another']",
                                name, field
                            ));
                        }
                        let which = match field.as_str() {
                            // a frame column's `.ok` is its rows' validity;
                            // any other value's is its own status — a column
                            // that can fail included: `.ok` of `may[f.a]` asks
                            // whether it failed (read as its cells' validity,
                            // the interpreter stopped on the failure and the
                            // compiled program answered false)
                            "ok" if matches!(base.ty(), Ty::Column(_)) => Meta::Valid,
                            "ok" => Meta::Ok,
                            "reason" => Meta::Reason,
                            "kind" => Meta::Kind,
                            _ => Meta::Trace,
                        };
                        decided_meta(Expr::Meta { base: Box::new(base), which })
                    }
                    _ => {
                        if let A::Ident(m) = object.as_ref() {
                            if self.is_module(m) && !self.declared(m) {
                                let ty = (&self.qualified_type(object, field)).clone();
                                return Ok(Expr::Constant(format!("{}.{}", m, field), ty));
                            }
                        }
                        // `x.len` is `len[x]`: one length whichever way it
                        // is asked — a list's items, text's characters, a
                        // matrix's rows, a column's cells. (Text's had been
                        // its bytes in both modes and a matrix's every
                        // number.) A record's field, a map's key and a
                        // frame's column named `len` stay what they are.
                        if field == "len" && !member {
                            // (typed without lowering it twice: `f[x].len`
                            // calls `f` once)
                            let measured = match self.target_type(object) {
                                Ty::Result(inner) => *inner,
                                t => t,
                            };
                            if matches!(measured, Ty::List(_) | Ty::Tensor(..) | Ty::Str | Ty::Column(_)) {
                                return self.call(&A::Ident("len".to_string()), std::slice::from_ref(object.as_ref()));
                            }
                        }
                        let base = if member { self.muted(|l| l.lower_expr(object))? } else { self.lower_expr(object)? };
                        // a parameter read by its fields, and typed by nothing
                        // else, is a parsed value (spec, Tools are the
                        // functions): what a call by name hands a function
                        // — a gate's call, a tool's object — and the one
                        // kind a field read says of a value no call types.
                        // (Such a function had been refused as typed by
                        // nothing, with advice to read the parameter, which
                        // the body did — Vijay, 2026-10-20.)
                        self.note_param_read(&base, &Ty::Json);
                        // a frame, a record, a map or a parsed value that can
                        // fail is read from its payload, refused first when
                        // it failed —
                        // a function hands the error back, a handler stops,
                        // the top level halts — as a decision on it is.
                        // (Compiled, the read had found the failed value's
                        // placeholder: `frame has no property or column
                        // 'price'` for a load that found no file, and JSON's
                        // `null` for a parse that failed, where the
                        // interpreter went on with the load's or the parse's
                        // error.)
                        // (a fact's declared value has no statement to stand
                        // in front of: the read itself refuses the value, in
                        // both backends, with the same reason)
                        let base = if !member && !self.in_fact_default && base.ty().is_result() && matches!(base.ty().payload(), Ty::Frame(_) | Ty::Record(_) | Ty::Map(..) | Ty::Json) {
                            self.decided(base, Decision::Read(field.clone()))
                        } else {
                            base
                        };
                        if !member && !self.contested(&base) {
                            self.check_field(&read(&base), field);
                        }
                        let ty = self.ctx.field_type(object, read(&base), field);
                        // a frame's column is typed by its cells as the frame's
                        // type says them; one it does not name shows as the
                        // program runs; and of a frame whose columns are not
                        // worked out yet, nothing is known yet (inference
                        // reads it again once they are)
                        if matches!(ty, Ty::Column(_)) {
                            self.refuse_column_read(&base, field);
                        }
                        let ty = match (&ty, read(&base).frame_cols()) {
                            (Ty::Column(_), Some(Some(cols))) => Ty::Column(Box::new((cols.cols.get(field).unwrap_or(&Ty::Json)).clone())),
                            (Ty::Column(_), Some(None)) if self.pass != Pass::Final => Ty::Unknown,
                            (Ty::Column(_), Some(None)) => Ty::Column(Box::new(Ty::Json)),
                            _ => ty,
                        };
                        // a moment's field read over a column of moments
                        // (`df.at.hour`) is a column of ints, missing where
                        // the moment is
                        let ty = match base.ty().payload() {
                            Ty::Column(cell) if crate::lower_ir::checks::moment_field_name(field) && matches!(cell.payload(), Ty::Moment | Ty::Json) => {
                                let int = if cell.is_result() { Ty::Result(Box::new(Ty::Int)) } else { Ty::Int };
                                Ty::Column(Box::new(int))
                            }
                            _ => ty,
                        };
                        // a map keyed by ints has no named members: `m[2]`
                        if let Ty::Map(crate::ir::Key::Int, _) = base.ty().payload() {
                            self.refuse(format!("a map keyed by ints is read by `m[2]`: `.{}` names a text key", field));
                        }
                        Expr::Field { base: Box::new(base), name: field.clone(), ty }
                    }
                }
            }
            A::Array(items) => {
                // a literal of functions written is a group: each described
                // and called by name (`describe`, `apply`), a positional call
                // through one a dispatch over them — compiled, each value is
                // its name (section 7 of docs/extending-crow.md; a list of
                // function values had needed each one's parameters typed)
                let written: Vec<String> = items.iter().filter_map(|i| match i { A::Ident(n) if self.calls_fn(n) && !self.declared(n) => Some(n.clone()), _ => None }).collect();
                if !written.is_empty() && written.len() == items.len() {
                    // (a member whose parameter nothing types — not a call of
                    // its own, not its body's read, not a default — cannot be
                    // described or called by name: refused where the list is
                    // written, with the fix; the dispatch that meets it later
                    // says nothing more)
                    if self.pass == Pass::Final {
                        for f in &written {
                            self.check_describable(f);
                        }
                    }
                    let group = Ty::Fns(written.clone());
                    let out: Vec<Expr> = written.iter().map(|n| Expr::FnRef(n.clone(), group.clone())).collect();
                    return Ok(Expr::ListLit(out, Ty::List(Box::new(group))));
                }
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.lower_expr(i)?);
                }
                // a list holds one kind (spec, Arrays: `[1, 'hello', true]` is
                // an error) — refused where two items' kinds are known and
                // differ, in both modes (the interpreter had made the list,
                // and the compiled program had not built)
                let params = self.params_items.as_ref().is_some_and(|p| *p == format!("{:?}", items));
                let kinds: Vec<(String, String)> = if params { Vec::new() } else { out.iter().filter_map(|e| crate::lower_ir::checks::item_kind(&e.ty())).collect() };
                if let Some((first_many, first_one)) = kinds.first() {
                    if let Some((_, other)) = kinds.iter().find(|(many, _)| many != first_many) {
                        self.refuse(format!("a list holds one kind: this one has {} and {} — a record (struct) holds fields of several kinds", first_one, other));
                    }
                }
                // a nested literal of numbers is a matrix, and a matrix's rows
                // are of one length: a ragged one is refused before the
                // program runs, in both modes, with the runtime's words (one
                // with an empty row among rows of numbers had run interpreted
                // as a list of lists, and the compiled program had not built;
                // one with rows of two lengths had stopped as it ran)
                if self.pass == Pass::Final {
                    let numeric_row = |e: &Expr| match e {
                        Expr::ListLit(inner, _) => inner.is_empty() || inner.iter().all(|x| matches!(x.ty().payload(), Ty::Int | Ty::Float)),
                        _ => matches!(e.ty().payload(), Ty::Tensor(..)),
                    };
                    let row_len = |e: &Expr| match e {
                        Expr::ListLit(inner, _) => Some(inner.len()),
                        _ => match e.ty().payload() {
                            Ty::Tensor(_, Some(dims)) => dims.first().copied().flatten(),
                            _ => None,
                        },
                    };
                    if out.len() >= 2 && out.iter().all(numeric_row) {
                        let lens: Vec<usize> = out.iter().filter_map(row_len).collect();
                        if lens.windows(2).any(|w| w[0] != w[1]) {
                            self.refuse("all inner arrays must be the same length: a nested literal of numbers is a matrix — a list of lists of other lengths is built with push[…]".to_string());
                        }
                    }
                }
                // (a literal whose rows' sizes show only as the program runs
                // keeps the rank its nesting states)
                let ty = TypeContext::array_type(out.iter().map(full));
                let ty = match shapes::literal_dims(&out) {
                    Some(dims) => shapes::shaped(ty, Some(dims)),
                    None => ty,
                };
                // a tensor holds its numbers as floats, and a float holds a
                // whole number exactly only up to 2^53: ints past it make a
                // list of ints, which holds them as they are (a tensor had
                // held 9007199254740993 as …992, in both modes); a matrix of
                // them is refused, since a matrix is a tensor
                // (and a numeral past 64 bits, and a power of two literals
                // past 2^53 — `[2 ^ 65, 3]` — are whole numbers a float would
                // round or a 64-bit int refuse: each had stopped where the
                // literal was made; Rosetta Code, Abundant odd numbers. A
                // computed int that grows there is known only after the
                // lowering: push it, which keeps every digit)
                let past = |e: &Expr| match e {
                    Expr::Const(Const::Int(v), _) => v.unsigned_abs() > 1u64 << 53,
                    Expr::Binary { op, l, r, .. } if op == "pow" => match (l.as_ref(), r.as_ref()) {
                        (Expr::Const(Const::Int(b), _), Expr::Const(Const::Int(x), _)) if *x >= 0 => (*b as f64).abs().powf(*x as f64) > (1u64 << 53) as f64,
                        _ => false,
                    },
                    // (a row that answers a growing int: a numeral past 64
                    // bits is one, `__whole`)
                    Expr::Call { target: Target::Row(row), .. } => matches!(row.ret, crate::table::Kind::Big),
                    other => matches!(other.ty().payload(), Ty::Big),
                };
                let ty = match &ty {
                    Ty::Tensor(e, dims) if matches!(e.as_ref(), Ty::Int) && dims.as_ref().is_none_or(|d| d.len() <= 1) && out.iter().any(past) => Ty::List(Box::new(Ty::Int)),
                    // (whatever the outer type came out as: the row with the
                    // big int had been typed a list of ints by the rule
                    // above, the other rows tensors, and the verifier found
                    // "a list of List(Int) holds an item of type Tensor")
                    _ if out.iter().any(|row| matches!(row, Expr::ListLit(inner, _) if inner.iter().any(past))) => {
                        self.refuse("a matrix holds its numbers as floats, which hold a whole number exactly only up to 2^53 (9007199254740992) — a list of ints holds a larger one, built with push[…]".to_string());
                        ty
                    }
                    _ => ty,
                };
                // (each item in the form the literal holds them: numbers held
                // as a tensor beside numbers held as a list are the list, a
                // list beside a matrix's rows a row, whole numbers beside
                // floats floats — held as they came, `[ys * 2, ys]` had been
                // an internal error, and handed to `log` it had not built
                // compiled; Rosetta Code, Fast Fourier transform)
                let held = match &ty {
                    Ty::List(item) => Some((**item).clone()),
                    Ty::Tensor(e, Some(dims)) if dims.len() >= 2 => Some(Ty::Tensor(e.clone(), Some(dims[1..].to_vec()))),
                    _ => None,
                };
                let out = match held {
                    Some(item) if !item.has_unknown() => out
                        .into_iter()
                        .map(|x| {
                            let x = failure_as(x, &item);
                            let x = held_as(x, &item);
                            if crate::ir::floats_of_ints(&item, x.ty().payload(), false) {
                                let ty = if x.ty().is_result() { Ty::Result(Box::new(item.clone())) } else { item.clone() };
                                Expr::Convert { how: Convert::IntsToFloats, expr: Box::new(x), ty }
                            } else {
                                x
                            }
                        })
                        .collect(),
                    _ => out,
                };
                Expr::ListLit(out, ty)
            }
            A::Map(pairs) => {
                let mut out = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    if let A::Ident(n) = k {
                        self.check_ident(n);
                    }
                    let key = match k {
                        A::Ident(n) | A::Str(n) => n.clone(),
                        A::Int(n) => n.to_string(),
                        other => format!("{:?}", other),
                    };
                    out.push((key, self.lower_expr(v)?));
                }
                // (keys written as ints key the map by ints)
                let int_keys = !pairs.is_empty() && pairs.iter().all(|(k, _)| matches!(k, A::Int(_)));
                let ty = match TypeContext::map_type(out.iter().map(|(_, v)| read(v))) {
                    Ty::Map(_, v) if int_keys => Ty::Map(crate::ir::Key::Int, v),
                    t => t,
                };
                // (each value in the form the map holds its values: a tensor
                // literal beside an empty list, or a list, is the list)
                let out = match &ty {
                    Ty::Map(_, item) => out.into_iter().map(|(k, v)| (k, held_as(v, item))).collect(),
                    _ => out,
                };
                Expr::MapLit(out, ty)
            }
            A::Named { name, .. } => return Err(format!("lowering: a named argument ('{}') survived the lowering of named arguments", name)),
            A::ForExpr { var, iter, body } => {
                // a list that can fail is refused where the loop starts, like
                // a condition: an expression has no statement before it to
                // hold the refusal (a fact's default has none), so both
                // backends refuse a list typed as able to fail right there.
                // What the iterable hoisted waits while the body is lowered,
                // or the body's first statement would take it.
                let it = self.lower_expr(iter)?;
                // a list that can fail is refused where the loop starts (by
                // the backends, not a propagation): an error leaves there
                if it.ty().is_result() {
                    self.saw_refusal(&Decision::Loop);
                }
                let before = std::mem::take(&mut self.pre);
                self.scopes.push([var.clone()].into_iter().collect());
                // inference's walk reads a collecting loop's yield, never its
                // body: a provisional lowering's body leaves nothing behind
                let outer = (self.pass == Pass::Settle).then(|| (self.ctx.vars.clone(), self.ctx.body_assigned.clone()));
                // the call scan reads the body with the variable bound as
                // an item, what it assigns left behind
                let fn_list = match iter.as_ref() {
                    A::Array(elems) if !elems.is_empty() && elems.iter().all(|e| matches!(e, A::Ident(n) if self.calls_fn(n))) => {
                        Some(elems.iter().filter_map(|e| if let A::Ident(n) = e { Some(n.clone()) } else { None }).collect::<Vec<String>>())
                    }
                    _ => None,
                };
                // (a name the variable hides keeps its type after the loop)
                let shadowed = (self.pass == Pass::Scan).then(|| self.ctx.vars.get(var.as_str()).cloned()).flatten();
                let unknown_items = self.pass == Pass::Scan && matches!(it.ty().payload(), Ty::List(e) if **e == Ty::Unknown);
                if unknown_items {
                    self.v.unknown_items.insert(var.clone());
                }
                let saved = if self.pass == Pass::Scan {
                    self.ctx.scan_loop_var(var, walked(&it));
                    None
                } else {
                    self.check_rank_known(&it, "a loop over");
                    self.check_walkable(&it);
                    Some(self.bind_loop_var(var, &it.ty()))
                };
                let listed = self.pass == Pass::Scan && fn_list.is_some();
                if let (true, Some(fns)) = (listed, fn_list) {
                    self.fn_lists.push((var.clone(), fns));
                }
                self.collecting += 1;
                yields_of(body, &mut self.yields);
                let lowered = self.lower_block(body);
                if unknown_items {
                    self.v.unknown_items.remove(var.as_str());
                }
                let mut b = lowered?;
                self.collecting -= 1;
                if listed {
                    self.fn_lists.pop();
                }
                // a list of what the body yields, as the lowering typed it
                let ty = TypeContext::for_type(yield_of(&b));
                if let Some(saved) = saved {
                    self.unbind_loop_var(var, saved);
                }
                if let Some(t) = shadowed {
                    self.ctx.vars.insert(var.clone(), t);
                }
                if let Some((vars, assigned)) = outer {
                    self.ctx.vars = vars;
                    self.ctx.body_assigned = assigned;
                }
                self.scopes.pop();
                let left = std::mem::replace(&mut self.pre, before);
                self.pre.extend(left);
                place_moves_in(&mut b, &HashSet::new(), Some(&[var.clone()].into_iter().collect()));
                // decided with the statement form's rule (decide_loops)
                Expr::ForExpr { var: var.clone(), iter: Box::new(it), body: b, ty, parallel: false }
            }
        })
    }

    fn is_module(&self, name: &str) -> bool {
        self.ctx.bare_modules.contains(name) || self.ctx.module_aliases.contains_key(name) || self.ctx.registry.is_loaded(name)
    }

    fn ident(&mut self, name: &str, ty: Ty) -> Expr {
        if self.declared(name) && !(self.is_fact(name) && self.sees_facts() && !self.scopes.iter().skip(1).any(|s| s.contains(name))) {
            return Expr::Local(name.to_string(), ty);
        }
        if self.is_fact(name) && self.sees_facts() {
            return Expr::Load(name.to_string(), ty);
        }
        if self.ctx.derives.contains(name) {
            // a route's handler reads it as the local it bound as it started
            if let Some(read) = self.route_derived.as_mut() {
                read.entry(name.to_string()).or_insert_with(|| ty.clone());
                return Expr::Local(name.to_string(), ty);
            }
            return Expr::Derived(name.to_string(), ty);
        }
        if self.calls_fn(name) {
            self.check_fn_value(name);
            return Expr::FnRef(name.to_string(), ty);
        }
        if self.ctx.registry.has_constant(name) && !self.declared(name) {
            return Expr::Constant(name.to_string(), ty);
        }
        Expr::Local(name.to_string(), ty)
    }

    /// A call, with the parameter list of a statement form noted while
    /// its arguments are lowered: the one list of several kinds (a list
    /// of what each `?` takes, sent one by one), exempt from the rule
    /// that a list holds one kind, whichever path lowers the call
    fn call(&mut self, callee: &A, args: &[A]) -> Result<Expr, String> {
        let saved = self.params_items.take();
        if let (A::Ident(name), Some(A::Array(items))) = (callee, args.get(2)) {
            if crate::table::statement_form(None, name).is_some() {
                self.params_items = Some(format!("{:?}", items));
            }
        }
        let out = self.call_inner(callee, args);
        self.params_items = saved;
        // a gradient differentiated again — `grad[grad[f]]`, or `grad[g]` of
        // a variable that holds one — is a second derivative, which is not
        // built (spec, Automatic Differentiation): refused before either
        // mode runs, as `grad` inside a function `grad` differentiates is
        // (the interpreter had stopped, "grad[] takes a function, got fn",
        // and the compiled program had not built)
        if self.pass == Pass::Final
            && let Ok(x) = &out
            && let Some(handed) = grad_argument(x)
            && self.gradient_of(handed).is_some()
        {
            self.refuse("grad[] takes a function the program writes, and this is a gradient — a gradient is not differentiated again: a second derivative is not built".to_string());
        }
        out
    }

    fn call_inner(&mut self, callee: &A, args: &[A]) -> Result<Expr, String> {
        if let A::Ident(n) = callee {
            if let Some(done) = self.described(n, args)? {
                return Ok(done);
            }
        }
        // a variable indexed: xs[0], m['k'], r.symbol[0] — except that
        // inside a view an element name is the element (the vocabulary
        // boundary, spec §12): a session named `a` outside does not turn
        // the view's anchors into lookups
        let element_in_view = |n: &str| self.in_ui && self.at == Where::In(FnKind::View) && crate::lower::UI_ELEMENTS.contains(&n);
        let indexes_value = match callee {
            // (a derivation is read like a fact, and indexed like one: the
            // book's dashboard picks rows of a derived frame, `trades[mask]`)
            A::Ident(n) => (self.declared(n) || self.is_fact(n) || self.ctx.derives.contains(n)) && (!self.calls_fn(n) || self.ctx.derives.contains(n)) && !element_in_view(n),
            A::FieldAccess { object, .. } => match object.as_ref() {
                A::Ident(m) => !self.is_module(m) || self.declared(m),
                _ => true,
            },
            _ => true,
        };
        if indexes_value {
            // a name or `m.x` indexed is checked as the call it is written
            // as, after its arguments; a call indexed (`f[x][0]`) is a call,
            // its arguments saying of its callee what any call's do (it had
            // been lowered unscanned, and a function only ever called so
            // kept its parameters unknown)
            let base = match callee {
                A::Ident(_) | A::FieldAccess { .. } if !matches!(callee, A::FieldAccess { object, .. } if !matches!(object.as_ref(), A::Ident(_))) => {
                    self.unscanned += 1;
                    let base = self.muted(|l| l.lower_expr(callee));
                    self.unscanned -= 1;
                    base?
                }
                _ => self.lower_expr(callee)?,
            };
            // an item of a value that can fail is read from its payload,
            // refused first when it failed — a function hands the error
            // back, a handler stops, the top level halts — as a field read
            // is (Vijay, 2026-10-01: one rule for reading into a value; the
            // item had been the error, carried on). A fact's declared value
            // has no statement to stand in front of: its own refusal, where
            // the facts are set, takes the error the read carries.
            let refused = !self.in_fact_default && base.ty().is_result() && !matches!(base.ty().payload(), Ty::Fn(..) | Ty::Unknown);
            let base = if refused { self.decided(base, Decision::Item) } else { base };
            // a number or a bool has no items (`x[0]` of an int had stopped
            // the interpreter as it ran — `not indexable` — and the compiled
            // program had not built); a builtin's name read as a method,
            // `xs.len[]`, is the call it means, the value first
            // (a growing number, a moment, a record and a response neither:
            // crow check --self)
            if self.pass == Pass::Final
                && matches!(
                    base.ty().payload(),
                    Ty::Int
                        | Ty::Float
                        | Ty::Bool
                        | Ty::Big
                        | Ty::Decimal
                        | Ty::Fraction
                        | Ty::Moment
                        | Ty::Record(_)
                        | Ty::OptRecord(_)
                        | Ty::Handle(_)
                        | Ty::Sparse
                        | Ty::Bytes
                        | Ty::Unit
                )
                && !self.contested(&base)
                && !of_unknown_rank(&base)
            {
                let words = match base.ty().payload() {
                    Ty::Record(_) | Ty::OptRecord(_) => "a record, read by its fields' names (p.x)",
                    Ty::Moment => "a moment, read by its fields' names (t.year)",
                    Ty::Handle(h) => h.word,
                    Ty::Sparse => "a sparse matrix, read whole",
                    // (the layer's bytes are read by `byte[b, i]` and
                    // `slice[b, i, j]`; indexed, the interpreter had stopped,
                    // "not indexable", and the compiled program had not built)
                    Ty::Bytes => "bytes, read by byte[b, i] and slice[b, i, j]",
                    t => crate::table::Arg::of_type(t).words(),
                };
                match callee {
                    A::FieldAccess { object, field } if checks::BUILTINS.contains(&field.as_str()) => {
                        let of = match object.as_ref() {
                            A::Ident(n) => n.clone(),
                            _ => "…".to_string(),
                        };
                        self.refuse(format!("{0}.{1}[] is {1}[{0}]: a function takes its value first", of, field));
                    }
                    // (what a function that answers nothing gives, held for
                    // whether it failed: indexed, the interpreter had stopped,
                    // "not indexable", and the compiled program had not built;
                    // Rosetta Code, Elementary cellular automaton)
                    _ if matches!(base.ty().payload(), Ty::Unit) => {
                        let name = local_named(&base).map(|n| format!("'{}'", n)).unwrap_or_else(|| "this".to_string());
                        self.refuse(format!("{} holds no value — the function it came from answers nothing, only whether it failed (`.ok`) — and `[…]` reads an item of a list, a map, text or a column", name));
                    }
                    _ => {
                        let name = local_named(&base).map(|n| format!("'{}'", n)).unwrap_or_else(|| "this".to_string());
                        self.refuse(format!("{} is {}, which has no items — `[…]` reads an item of a list, a map, text or a column", name, words));
                    }
                }
            }
            // (a function value applied is a call: its arguments propagate
            // as any call's do, and are no positions)
            let applied = matches!(base.ty().payload(), Ty::Fn(..) | Ty::Unknown);
            let mut idx = Vec::new();
            for a in args {
                let mut i = self.lower_expr(a)?;
                // (a frame's column of bools as the index picks as the mask
                // it makes: `names[f.on]` is `names[f.on eq true]`)
                if !applied && args.len() == 1 && is_bool_column(&i, false) {
                    i = self.lower_expr(&as_mask(a))?;
                }
                idx.push(if applied { i } else { self.decided_index(i) });
            }
            // a position or a key the program does not know the type of — a
            // parsed value — is read as what the place takes: a position an
            // int, a key text, in both modes, stopping with what it holds
            // (the interpreter had read it as it ran and the compiled
            // program had not built; then both had refused it until read)
            // (a function value applied takes its arguments as its own body
            // reads them, as a call by name does: read as a position, a
            // parsed value had typed the function's parameter an int, and
            // stopped — crow check --self)
            // a function value has one type: an argument of another kind than
            // its parameter's — `pass[3]` called by name, then handed as `g`
            // and called `g['abc']` — is refused where it is handed (the
            // compiled program had not built it — crow check --self)
            // a call through a function of a group (a list literal of
            // functions): a dispatch over the members — each must take the
            // arguments, and all answer one kind (section 7 of
            // docs/extending-crow.md: called by name, never positionally,
            // where they disagree)
            // (in every pass where the members answer one known kind: typed
            // as nothing before the final pass, a function answering the
            // call through a group handed in had been declared as text, and
            // the compiled program did not build; Rosetta Code, Deming's
            // funnel)
            if let Ty::Fns(names) = base.ty().payload().clone()
                && (self.pass == Pass::Final || {
                    let answers: Vec<Ty> = names.iter().filter_map(|n| self.ctx.functions.get(n).map(|s| s.return_type.clone())).collect();
                    let first = answers.first().cloned().unwrap_or(Ty::Unknown);
                    !answers.is_empty() && answers.windows(2).all(|w| w[0] == w[1]) && !first.has_unknown() && Self::zero_of(&first).is_some()
                })
            {
                let written: &[A] = args;
                let answers: Vec<Ty> = names.iter().filter_map(|n| self.ctx.functions.get(n).map(|s| s.return_type.clone())).collect();
                let arities: std::collections::BTreeSet<usize> = names.iter().filter_map(|n| self.ctx.functions.get(n).map(|s| s.params.len())).collect();
                let one_kind = answers.windows(2).all(|w| w[0] == w[1]);
                let answer = answers.first().cloned().unwrap_or(Ty::Unknown);
                let zero = Self::zero_of(&answer);
                if arities.len() != 1 || !one_kind || answer.has_unknown() || zero.is_none() {
                    self.refuse(format!(
                        "`{}[…]` calls one of {} by position: they take or answer different kinds, or answer a kind no position can hold — call it by name (apply[f, args]), or hand functions of one kind that answer an int, a number, a bool, text, a list or a map",
                        local_named(&base).unwrap_or_else(|| "f".to_string()),
                        names.join(", ")
                    ));
                    return Ok(Expr::Const(Const::Str(String::new()), Ty::Str));
                }
                let written: Vec<A> = written.to_vec();
                let init = zero.expect("a zero of the answer's kind");
                let out = self.dispatch(base, &names, answer, init, &|me, cand| me.lower_expr(&A::Call { callee: Box::new(A::Ident(cand.to_string())), args: written.clone() }))?;
                return Ok(out);
            }
            // (before the final pass, a call through a group is a call of each
            // member, as the dispatch will be: the spec's own `ops = [double,
            // square]` then `ops[0][3]` had left the members typed by nothing,
            // and the dispatch was refused; Rosetta Code, Call a function)
            if self.pass != Pass::Final
                && let Ty::Fns(names) = base.ty().payload().clone()
            {
                let written: Vec<A> = args.to_vec();
                for cand in &names {
                    let call = A::Call { callee: Box::new(A::Ident(cand.clone())), args: written.clone() };
                    let _typed = self.muted(|me| me.lower_expr(&call));
                }
            }
            if self.pass == Pass::Final
                && let Ty::Fn(params, _) = base.ty().payload().clone()
            {
                let whole = |t: &Ty| matches!(t, Ty::Int | Ty::Big);
                let misfit = params.iter().zip(idx.iter()).find(|(p, x)| {
                    let (held, given) = (sans_results(p.payload()), sans_results(x.ty().payload()));
                    !held.has_unknown()
                        && !matches!(held, Ty::Json | Ty::Fn(..))
                        && !matches!(given, Ty::Unknown | Ty::Json)
                        && !held.compatible(&given)
                        && !(whole(&held) && whole(&given))
                        && !held.compatible(&as_inferred(&given))
                });
                if let Some((p, x)) = misfit {
                    let words = |t: &Ty| crate::table::Arg::of_type(t.payload()).words();
                    let msg = format!(
                        "`{}[…]` hands {} to a function value whose parameter is {} — a function value has one type, the one its other calls give it",
                        local_named(&base).unwrap_or_else(|| "f".to_string()),
                        words(&x.ty()),
                        words(p)
                    );
                    self.refuse(msg);
                }
            }
            if !applied && !matches!(base.ty().payload(), Ty::Json) && idx.iter().any(|x| matches!(x.ty().payload(), Ty::Json)) {
                let what = local_named(&base).unwrap_or_else(|| "the value".to_string());
                let (kind, how) = if let Ty::Map(key, _) = base.ty().payload() { (key.ty(), format!("a key of {}", what)) } else { (Ty::Int, format!("a position in {}", what)) };
                let mut read_idx = Vec::with_capacity(idx.len());
                for x in idx {
                    read_idx.push(self.read_json(x, &kind, &how));
                }
                idx = read_idx;
            }
            // a mask picks a frame's rows (spec, Frames; Vijay, 2026-10-16:
            // the index picks a frame's rows as it picks a list's items and a
            // tensor's rows): `f[f.n gt 1]` is the frame of the rows where it
            // is true, every column as it was, fused where the mask compares
            // the frame's own columns into one pass over them (fuse.rs). Any other index of a frame is refused: a column is
            // read by its name, a cell by the column's index (`f['a']` had
            // stopped the interpreter as it ran and not built compiled).
            if matches!(base.ty().payload(), Ty::Frame(_)) {
                let masked = match idx.as_slice() {
                    [index] => match index.ty().payload() {
                        Ty::List(item) => matches!(item.payload(), Ty::Bool),
                        // (a comparison over columns not typed yet is typed
                        // a truth until they are, and a mask still: it reads a
                        // column)
                        Ty::Bool | Ty::Unknown => self.pass != Pass::Final || reads_a_column(index),
                        _ => false,
                    },
                    _ => false,
                };
                let name = local_named(&base).unwrap_or_else(|| "f".to_string());
                if masked && let Some(row) = crate::table::row("core.__pick_frame") {
                    // a mask decides each row, and an error decides nothing: a
                    // list of values that can fail (a cell of a column that can
                    // be missing) is not a mask, in either mode
                    if let (Some(Ty::List(item)), Some(a)) = (idx.first().map(|m| m.ty().payload().clone()), args.first())
                        && item.is_result()
                    {
                        let what = match a {
                            A::Ident(v) => format!("'{}'", crate::lower::shown(v)),
                            _ => "the mask".to_string(),
                        };
                        self.refuse(format!("{} holds values that can fail, and a mask decides every row: guard each value with .ok (cells of a column that can be missing: fill the column first, fill[frame, 'column', value])", what));
                    }
                    let frame = base.ty().payload().clone();
                    let mask = idx.pop().unwrap_or(Expr::Const(Const::Bool(false), Ty::Bool));
                    return Ok(picked(row, base, mask, frame));
                }
                if self.pass == Pass::Final {
                    self.refuse(format!(
                        "a frame is indexed by a mask, which picks its rows ({0}[{0}.age gt 30]): read a column as `{0}.name`, its cell as `{0}.name[i]`, and columns with select[{0}, ['name']]",
                        name
                    ));
                }
            }
            // `[]` written and indexed holds nothing to read (its items'
            // kind unknown, the compiled program had not built)
            if self.pass == Pass::Final && matches!(&base, Expr::ListLit(items, _) if items.is_empty()) {
                self.refuse("`[][…]` reads from a list that holds nothing".to_string());
            }
            // a mask picks: `t[labels eq j]` is the rows (or the numbers)
            // where it is true, `xs[mask]` the items (spec, Arrays) — the
            // index's own rows, typed by the base as a position's read is
            if let [index] = idx.as_slice()
                && let Some((row, ty)) = mask_pick(&base.ty(), &index.ty())
            {
                let mut index = idx.pop().unwrap_or(Expr::Const(Const::Bool(false), Ty::Bool));
                // (a mask of a matrix's shape reaches its row as its numbers,
                // true 1 and false 0, in both modes)
                if row.id == "core.__pick_numbers" {
                    let dims = base.ty().tensor_dims().cloned();
                    index = Expr::Convert { how: Convert::ToTensor, expr: Box::new(index), ty: Ty::Tensor(Box::new(Ty::Float), dims) };
                }
                return Ok(picked(row, base, index, ty));
            }
            // a map's keys are one kind: an index of the other kind is
            // refused before the program runs, in both modes — and a key is
            // text or an int
            if let (Ty::Map(key, _), [index]) = (base.ty().payload(), idx.as_slice()) {
                let given = index.ty().payload().clone();
                if self.pass == Pass::Final && !key_like(&given) {
                    self.refuse(format!("a map's key is text or an int, and this is {}", crate::table::Arg::of_type(&given).words()));
                } else if key.unify(crate::type_infer::key_kind_of(&given)).is_none() {
                    self.refuse(crate::table::wrong_key(*key, &given));
                }
            }
            // a position is an int: text, a list, a tensor, a column read by
            // a bool or a float had stopped interpreted and been read compiled
            // as the number the value casts to (`'abc'[true]` was 'b')
            // (a mask was picked above: what is left of a list is one int —
            // a list of ints or of text as positions had stopped interpreted,
            // "array index must be an integer", and not built compiled; a
            // tensor gathers at a list of ints)
            if self.pass == Pass::Final
                && let [index] = idx.as_slice()
                && matches!(base.ty().payload(), Ty::Str | Ty::List(_) | Ty::Tensor(..) | Ty::Column(_))
            {
                let given = index.ty().payload().clone();
                let gathers = matches!(base.ty().payload(), Ty::Tensor(..))
                    && matches!(&given, Ty::Tensor(e, _) | Ty::List(e) if matches!(e.payload(), Ty::Int | Ty::Float | Ty::Unknown));
                let unsettled = matches!(&given, Ty::List(e) | Ty::Column(e) | Ty::Tensor(e, _) if e.has_unknown());
                if !position_like(&given) || (matches!(given, Ty::List(_) | Ty::Tensor(..) | Ty::Column(_)) && !gathers && !unsettled) {
                    let what = local_named(&base).map(|n| format!("'{}'", n)).unwrap_or_else(|| crate::table::Arg::of_type(base.ty().payload()).words().to_string());
                    let given = crate::table::Arg::of_type(&given).words();
                    self.refuse(format!("a position in {} is an int, and this is {}", what, given));
                }
            }
            if self.pass == Pass::Final && idx.len() == 1 {
                // a parsed value is read at a key or a position
                if matches!(base.ty().payload(), Ty::Json) && !key_like(idx[0].ty().payload()) {
                    self.refuse(format!("a parsed value is read at a key (text) or a position (an int), and this is {}", crate::table::Arg::of_type(idx[0].ty().payload()).words()));
                }
            }
            let arg_types: Vec<Ty> = idx.iter().map(read).collect();
            let full_types: Vec<Ty> = idx.iter().map(full).collect();
            // a gradient applied (`dloss[w, xs]`, or `grad[f][w, xs]`) hands
            // the function it is of what a call of it would: a value that can
            // fail by its payload, which is all the gradient takes, as
            // `grad[f]`'s type says (a function of f's parameters) — `xs`
            // from `load` had made the variable holding the gradient a
            // function of a value that can fail in the scan and of a tensor
            // after, a type it "cannot change"
            let gradient = self.gradient_of(&base);
            let scanned: Vec<Ty> = match &gradient {
                Some(_) => full_types.iter().map(|t| t.payload().clone()).collect(),
                None => full_types.clone(),
            };
            self.scan_call(callee, args, &scanned);
            let checks: Vec<Ty> = idx.iter().map(|x| self.checked(x)).collect();
            self.check_callee(callee, args, &checks);
            // (an indexed column is read as it is lowered here, not where
            // it was lowered muted)
            if let Expr::Field { base: frame, name, ty: Ty::Column(_) } = &base {
                self.refuse_column_read(frame, name);
            }
            // a cell of a column its frame's type knows, not a gather by a
            // list of indices: frame.col[i]
            if let (Ty::Column(cell), [index]) = (base.ty().payload(), arg_types.as_slice()) {
                if **cell != Ty::Unknown && !matches!(index, Ty::Tensor(..) | Ty::List(_)) {
                    let ty = (**cell).clone();
                    return Ok(Expr::Index { base: Box::new(base), indices: idx, ty });
                }
            }
            // a call of a gradient types the function it is of, as a direct
            // call would: the tensors it hands retype parameters the seek
            // guessed floats (a function grad differentiates is of numbers,
            // or of tensors), and every call after unifies
            if let Some(f) = gradient.clone() {
                // (the call typed wherever it is lowered: as an index's base
                // too, `grad[f][x][1]` — unifying twice changes nothing)
                if self.pass == Pass::Scan {
                    // (an int handed to a function of numbers is a float)
                    let handed: Vec<Ty> = scanned.iter().map(|t| if *t == Ty::Int { Ty::Float } else { t.clone() }).collect();
                    self.ctx.grad_forced.remove(&f);
                    self.ctx.note_call(&A::Ident(f), args, &handed);
                }
            }
            // a row or a number, by a rank the lowering knows
            if let ([index], Ty::Tensor(_, None)) = (arg_types.as_slice(), base.ty().payload())
                && matches!(index, Ty::Int | Ty::Unknown)
            {
                self.check_rank_known(&base, "an index into");
            }
            // (a variable of the program's named like a form — `box`, the
            // test harness's — is indexed by its own items: the form's rule
            // had typed `box[0]` of `box = [5, 6]` as a list; Rosetta Code,
            // Facial detection in a photo)
            let named_like_a_form = matches!(callee, A::Ident(n) if self.declared(n) && crate::table::rows_named(n).next().is_some());
            let ty = if named_like_a_form {
                self.ctx.index_type(read(&base), arg_types.first().cloned())
            } else {
                self.call_type(callee, args, &arg_types, &full_types, Some(read(&base)))
            };
            // a decomposition's piece, read by its literal position, has its
            // own shape (the list's item type has none of them)
            let position = if idx.len() == 1 { shapes::size(&idx[0]) } else { None };
            let ty = match (self.pieces_of(&base), position) {
                (Some(pieces), Some(i)) => match pieces.get(i) {
                    Some(Some(d)) => shapes::shaped(ty, Some(d.clone())),
                    _ => ty,
                },
                _ => ty,
            };
            // (an item of a refused value is the payload's item: the call's
            // type, read from the variable, still said it could fail — but
            // a parsed value's item can fail itself, JSON's null where it
            // has none, and so can an item of a list of values that can
            // fail: stripped, `one = names[1]` of such a list was typed
            // plain, its `.ok` true where it held an error, and an int's
            // "disagreed with itself")
            let ty = if refused && !matches!(base.ty(), Ty::Json) && !self.ctx.index_type(base.ty(), arg_types.first().cloned()).is_result() { ty.payload().clone() } else { ty };
            // a gradient applied to a value that failed answers that failure,
            // the leftmost, and never runs, as any call does (spec,
            // Auto-Propagation): the interpreter had handed the failure in
            // ("takes numbers or tensors, got done"), and the compiled call
            // had stopped or answered it by whether it was the first argument
            let ty = if gradient.is_some() && !matches!(ty, Ty::Unknown) && !ty.is_result() && idx.iter().any(|i| i.ty().is_result()) {
                Ty::Result(Box::new(ty))
            } else {
                ty
            };
            return Ok(Expr::Index { base: Box::new(base), indices: idx, ty });
        }
        let (module, name) = match callee {
            A::Ident(n) => (None, n.clone()),
            A::FieldAccess { object, field } => match object.as_ref() {
                A::Ident(m) => (Some(m.clone()), field.clone()),
                _ => return Err("lowering: a call through an expression".into()),
            },
            _ => return Err("lowering: a call through an expression".into()),
        };
        // a read a guard narrowed (lower.rs): the value, never its failure
        if module.is_none() && name == crate::lower::KNOWN && args.len() == 1 {
            let x = self.lower_expr(&args[0])?;
            self.scan_call(callee, args, &[full(&x)]);
            self.check_call(&name, args, &[self.checked(&x)]);
            if x.ty().is_result() {
                let payload = x.ty().payload().clone();
                return Ok(Expr::Convert { how: Convert::Known, expr: Box::new(x), ty: payload });
            }
            return Ok(x);
        }
        // a struct constructor
        if module.is_none() && self.ctx.structs.contains_key(&name) {
            let declared: Vec<Ty> = self.ctx.structs.get(&name).map(|s| s.fields.iter().map(|(_, t)| (t).clone()).collect()).unwrap_or_default();
            let mut fields = Vec::new();
            let mut types = Vec::new();
            let mut checks = Vec::new();
            for (i, a) in args.iter().enumerate() {
                let value = self.lower_expr(a)?;
                // a list whose items are not known yet, handed to a field
                // whose are (a list of empty lists, a graph's arcs, grown a
                // record at a time elsewhere): the local takes the field's
                // type (compiled, it had been a list of text, and the
                // record did not build)
                if self.pass != Pass::Final {
                    let field_ty = self.ctx.structs.get(&name).and_then(|s| s.fields.get(i)).map(|(_, t)| t.clone());
                    if let (Expr::Local(v, _), Some(field_ty)) = (&value, field_ty) {
                        let open = matches!(full(&value), Ty::List(_)) && full(&value).has_unknown();
                        let known = matches!(field_ty, Ty::List(_)) && !field_ty.has_unknown();
                        if open && known {
                            let v = v.clone();
                            self.ctx.note_scanned_assign(&v, field_ty.clone());
                            self.rebind(&v, &field_ty);
                        }
                    }
                }
                types.push(full(&value));
                checks.push(self.checked(&value));
                // a parsed value given to a field of a kind is read as that
                // kind, as one assigned to a variable of a kind is (spec, a
                // parsed value is read as its use implies; a field of text
                // given one had held the parsed value compiled, and not built)
                let value = match declared.get(i).map(|t| t.payload().clone()) {
                    Some(kind @ (Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Moment)) if self.pass == Pass::Final && matches!(value.ty().payload(), Ty::Json) => {
                        let fname = self.ctx.structs.get(&name).and_then(|s| s.fields.get(i)).map(|(f, _)| f.clone()).unwrap_or_default();
                        self.read_json(value, &kind, &format!("given to {}'s field '{}'", crate::lower::shown(&name), fname))
                    }
                    _ => value,
                };
                // a field holds one kind, as a variable does: a value of
                // another is refused where the record is made (`Box[2.5]`
                // beside `Box['a']` had run interpreted and not built
                // compiled — crow check --self)
                if self.pass == Pass::Final
                    && let Some(field) = declared.get(i).map(|t| t.payload().clone())
                    && !matches!(field, Ty::Unknown)
                {
                    let given = value.ty().payload().clone();
                    let whole = |t: &Ty| matches!(t, Ty::Int | Ty::Big);
                    // (whether items can fail is no kind of its own: a field's
                    // kind joins every record made, and a list of values that
                    // cannot fail is held where some can — bm25's index)
                    let (field_k, given_k) = (sans_results(&field), sans_results(&given));
                    // (a parsed value given a field of a simple kind was read
                    // as it above; one given a field of another kind fits no
                    // more than another value would — a column's field given
                    // one had not built)
                    let fits = matches!(given, Ty::Unknown)
                        || (matches!(given, Ty::Json) && matches!(field_k, Ty::Json))
                        || field_k.compatible(&given_k)
                        || (whole(&field_k) && whole(&given_k))
                        || field_k.compatible(&as_inferred(&given_k));
                    if !fits {
                        let fname = self.ctx.structs.get(&name).and_then(|s| s.fields.get(i)).map(|(f, _)| f.clone()).unwrap_or_default();
                        // (a list named by its items' kind: `holds a list, and
                        // this is a list` had named neither; Rosetta Code,
                        // Gaussian elimination)
                        let words = |t: &Ty| crate::table::Arg::of_type(t).said(t);
                        self.refuse(format!(
                            "{}'s field '{}' holds {}, and this is {}: a field holds one kind, as a variable does",
                            crate::lower::shown(&name),
                            fname,
                            words(&field),
                            words(&given)
                        ));
                    }
                }
                // a list of numbers is held as a list or as a tensor: a
                // field holds the form its record was settled on, so a value
                // in the other is converted (`[h]` beside a list pushed to);
                // an int given a float field is a float, as one handed to a
                // float parameter is (compiled, `P[1, 2]` beside `P[1, 2.5]`
                // did not build, and the interpreter kept the int)
                // (an int that can fail — `len` of a frame that can — is a
                // float that can fail: the payload converts, the failure
                // stays; it had been left an int in a field of floats, and
                // the compiled program did not build)
                // (an int where the field holds a record or none is none only
                // as 0: checked where the record is made, as a parameter's is
                // — `Box[3]` beside `Box[P[…]]` had not built compiled)
                let value = match (declared.get(i).map(|t| t.payload().clone()), value.ty().payload()) {
                    (Some(slot @ Ty::OptRecord(_)), Ty::Int) if self.pass == Pass::Final && !matches!(value, Expr::Const(Const::Int(0), _)) => self.none_of(value, &slot),
                    // (0 written down is none, typed as the link: where the
                    // field can also hold a failure — `left` given a record
                    // that can fail elsewhere — it had been a success holding
                    // the int, and the compiled program did not build; Rosetta
                    // Code, Rational calculator)
                    (Some(slot @ Ty::OptRecord(_)), Ty::Int) if matches!(value, Expr::Const(Const::Int(0), _)) => Expr::Const(Const::Int(0), slot),
                    _ => value,
                };
                let how = match (declared.get(i).map(|t| t.payload()), value.ty().payload()) {
                    (Some(Ty::List(_)), Ty::Tensor(..)) => Some(Convert::ToList),
                    (Some(Ty::Tensor(..)), Ty::List(_)) => Some(Convert::ToTensor),
                    (Some(Ty::Float), Ty::Int) if !matches!(value, Expr::Convert { how: Convert::Unwrap, .. }) => Some(Convert::IntToFloat),
                    // (a list of ints where the field holds floats — a vector
                    // given at another record — is a list of floats)
                    (Some(Ty::List(f)), Ty::List(e)) if **f == Ty::Float && **e == Ty::Int => Some(Convert::IntsToFloats),
                    _ => None,
                };
                fields.push(match how {
                    Some(how) => {
                        let payload = declared[i].payload().clone();
                        let ty = if value.ty().is_result() { Ty::Result(Box::new(payload)) } else { payload };
                        Expr::Convert { how, expr: Box::new(value), ty }
                    }
                    None => value,
                });
            }
            self.check_call(&name, args, &checks);
            self.scan_call(callee, args, &types);
            // a test's record, for what only tests type (`type_clicks`)
            if let Some(tested) = &mut self.tested {
                tested.push((name.clone(), types.iter().map(|t| t.text()).collect()));
            }
            if let Some(seen) = &mut self.seen {
                seen.calls.push((name.clone(), types));
            }
            return Ok(Expr::RecordLit { name, fields });
        }
        // a ui element with its attributes first (lower.rs put them there);
        // a standard module's inlined function keeps its names (llm's
        // `embed` stacks tensor rows: math's `stack`, not the element)
        if module.is_none() && self.in_ui && crate::lower::UI_ELEMENTS.contains(&name.as_str()) && !crate::modules::is_crow_fn(&self.current_fn) {
            let mut attrs = Vec::new();
            let mut children = Vec::new();
            // what the call scan reads of the arguments: the attributes as
            // one map, each held handler call as the call it holds
            let mut types: Vec<Ty> = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                if i == 0 {
                    if let A::Map(pairs) = a {
                        for (k, v) in pairs {
                            if let A::Ident(n) = k {
                                self.check_ident(n);
                            }
                            let key = match k {
                                A::Ident(n) | A::Str(n) => n.clone(),
                                other => format!("{:?}", other),
                            };
                            // an event attribute holds a handler, or a
                            // handler with its arguments, to call later
                            let is_event = matches!(key.as_str(), "click" | "input" | "submit" | "change" | "seek");
                            if is_event {
                                if let Some(d) = self.deferred(v)? {
                                    attrs.push((key, d));
                                    continue;
                                }
                            }
                            attrs.push((key, self.lower_expr(v)?));
                        }
                        let values: Vec<Ty> = pairs.iter().zip(&attrs).map(|((_, v), (_, x))| if matches!(x, Expr::Deferred { .. }) { self.held_type(v, x).text() } else { read(x) }).collect();
                        types.push(TypeContext::map_type(values.into_iter()));
                        continue;
                    }
                }
                let child = self.lower_expr(a)?;
                // a response is what a route returns, not something a page
                // shows (the interpreter showed `text[s]` as its text; the
                // compiled page did not build)
                if matches!(child.ty().payload(), Ty::Handle(h) if h.name == crate::table::RESPONSE.name) {
                    let what = match a {
                        A::Call { callee, .. } => match callee.as_ref() {
                            A::Ident(f) => format!("{}[…]", f),
                            A::FieldAccess { field, .. } => format!("{}[…]", field),
                            _ => "this".to_string(),
                        },
                        _ => "this".to_string(),
                    };
                    self.refuse(format!("{}: {} is a web response, which a route returns — a page shows elements and text: place the text itself, or p[…] or span[…]", name, what));
                }
                // (a function is no value a page shows, as `log` shows none:
                // a control is given its handler by name, `click = f`)
                if matches!(child.ty().payload(), Ty::Fn(..)) {
                    self.refuse(format!("{}: a function is no child — a page shows elements and text, and an element is given its handler by name: click = …", name));
                }
                types.push(full(&child));
                children.push(child);
            }
            let checks: Vec<Ty> = types.iter().map(|t| t.text()).collect();
            self.check_call(&name, args, &checks);
            self.scan_call(callee, args, &types);
            self.check_tree_columns(&name, &attrs, &children);
            return Ok(Expr::Tree { element: name, attrs, children });
        }
        // arguments: a fallible one reaches the callee as its payload, and
        // an error among them is the call's value (`propagates`); a view
        // is the exception, like the elements it is made of: it renders
        // what it is given (spec §10), so its arguments arrive as they are
        let propagates = !(module.is_none() && (matches!(name.as_str(), "log" | "type" | "error" | "assert") || self.ctx.views.contains(&name)));
        let mut lowered = Vec::with_capacity(args.len());
        let mut arg_types = Vec::with_capacity(args.len());
        let mut full_types = Vec::with_capacity(args.len());
        let mut checks = Vec::with_capacity(args.len());
        // (a function the row calls with what it is handed: `revise[name,
        // default, f, x]` is f[held, x], `later[f, a]` f[a] — inference
        // copies f per signature though the row's call is its only call
        // site, and the copy is chosen once the arguments are lowered)
        // (a predicate a row decides by — `all`, `any`, `find` — answers a
        // truth: one whose answer can fail had been read as not passing
        // interpreted, and the compiled program had not built; refused, with
        // the fix; Rosetta Code, Roman numerals/Encode)
        if self.pass == Pass::Final && !self.ctx.functions.contains_key(&name) {
            for (i, sig) in crate::table::fn_params(&name, args.len()) {
                if sig.ret != crate::table::Kind::Bool {
                    continue;
                }
                if let Some(A::Ident(f)) = args.get(i)
                    && self.calls_fn(f)
                    && self.ctx.functions.get(f).is_some_and(|fsig| fsig.return_type.is_result())
                {
                    self.refuse(format!(
                        "{}[] decides by what {}[] answers, a truth, and {}[] answers one that can fail — decide the failure inside it (`if not v.ok`, then `return false`)",
                        crate::lower::shown(&name),
                        crate::lower::shown(f),
                        crate::lower::shown(f)
                    ));
                }
            }
        }
        let row_fns: Vec<(usize, &'static crate::table::Sig)> = if self.ctx.functions.contains_key(&name) {
            Vec::new()
        } else {
            crate::table::fn_params(&name, args.len())
                .into_iter()
                .filter(|(i, _)| matches!(args.get(*i), Some(A::Ident(f)) if self.calls_fn(f) && self.ctx.specializations.get(f).is_some_and(|v| !v.is_empty())))
                .collect()
        };
        for (i, a) in args.iter().enumerate() {
            // the harness's click[a, h[x]] holds the handler call for the click
            if module.is_none() && name == "click" && i == 1 {
                if let Some(d) = self.deferred(a)? {
                    if let (Some(clicks), Expr::Deferred { name: h, args: held_args }) = (&mut self.clicks, &d) {
                        clicks.push((h.clone(), held_args.iter().map(read).collect()));
                    }
                    // typed as the call it holds
                    let held = self.held_type(a, &d);
                    arg_types.push(held.text());
                    checks.push(held.text());
                    full_types.push(held);
                    lowered.push(d);
                    continue;
                }
            }
            // a frame's column of bools where the call takes a mask is the
            // mask it makes — `where[f.on, a, b]` is `where[f.on eq true,
            // a, b]`, a missing cell false as the comparison answers it
            // (spec, Frames): lowered as that comparison, in both modes
            let now = self.differentiating || name == "grad";
            let was = std::mem::replace(&mut self.differentiating, now);
            let handed = match a {
                A::Ident(f) if row_fns.iter().any(|(at, _)| *at == i) => Some(f.clone()),
                _ => None,
            };
            let held = std::mem::replace(&mut self.handed_to_row, handed);
            let x = self.lower_expr(a);
            self.handed_to_row = held;
            self.differentiating = was;
            let mut x = x?;
            if !self.ctx.functions.contains_key(&name) && is_bool_column(&x, true) && crate::table::takes_mask_at(&name, i, args.len()) {
                x = self.lower_expr(&as_mask(a))?;
            }
            // (a failure made in an item's place — `push[xs, error['none']]`
            // — is a failure of what the collection before it holds: of no
            // value, no row was picked by it, and the item handed on had
            // been `()` compiled, which did not build)
            if i > 0
                && i + 1 == args.len()
                && !self.ctx.functions.contains_key(&name)
                && let Some(Ty::List(item) | Ty::Map(_, item)) = lowered.first().map(|f: &Expr| f.ty().payload().clone())
                && item.is_concrete()
            {
                x = failure_as(x, &Ty::Result(Box::new(item.payload().clone())));
            }
            arg_types.push(read(&x));
            // a column keeps its cells' kind into the parameter it is handed
            // to: `for x in may[f.a, …]` walked a column of nothing known,
            // which the compiled program walked as text
            full_types.push(kept(&x));
            checks.push(self.checked(&x));
            if propagates && x.ty().is_result() {
                let rty = x.ty();
                lowered.push(Expr::Convert { how: Convert::Unwrap, expr: Box::new(x), ty: rty.payload().clone() });
            } else {
                lowered.push(x);
            }
        }
        // (a list of whole numbers written down beside a list of ints, in
        // one builtin's call, is a list of ints as the other is: `concat[[x],
        // p]` had made the literal a tensor, and the answer one, against
        // the list a sibling return made — an internal error; Rosetta Code,
        // Determinant and permanent)
        if !self.ctx.functions.contains_key(&name) && lowered.iter().any(|x| matches!(x.ty().payload(), Ty::List(e) if matches!(e.as_ref(), Ty::Int | Ty::Big))) {
            for i in 0..lowered.len() {
                let literal = matches!(args.get(i), Some(A::Array(_))) && matches!(&lowered[i], Expr::ListLit(_, Ty::Tensor(e, d)) if **e == Ty::Int && d.as_ref().is_none_or(|d| d.len() <= 1));
                if literal && let Expr::ListLit(items, _) = std::mem::replace(&mut lowered[i], Expr::Const(Const::Int(0), Ty::Int)) {
                    lowered[i] = Expr::ListLit(items, Ty::List(Box::new(Ty::Int)));
                    arg_types[i] = read(&lowered[i]);
                    full_types[i] = kept(&lowered[i]);
                    checks[i] = self.checked(&lowered[i]);
                }
            }
        }
        for (i, sig) in row_fns {
            let Some(A::Ident(f)) = args.get(i) else { continue };
            match self.row_copy(&name, f, i, sig, &full_types) {
                Some(copy) => {
                    let x = Expr::FnRef(copy.clone(), self.ctx.ident_type(&copy));
                    arg_types[i] = read(&x);
                    full_types[i] = kept(&x);
                    checks[i] = self.checked(&x);
                    lowered[i] = x;
                }
                // (no copy takes what the row hands it: refused as any
                // value of several kinds is)
                None => self.check_fn_value(f),
            }
        }
        // a parameter typed by nothing (a function no call types: a tool only
        // handed to a model) handed to a builtin whose rows agree on the
        // kind they take there is read as that kind, noted for the tool's
        // schema as a parsed value's read is: `upper[city]` types `city` as
        // text (spec, Tools are the functions — "read it in the body"; the
        // refusal had said so, and only a call had typed it)
        if !self.ctx.functions.contains_key(&name) {
            let argc = lowered.len();
            for (i, x) in lowered.iter().enumerate() {
                if matches!(x, Expr::Local(_, Ty::Unknown))
                    && let Some(k) = self.rows_say(&name, module.as_deref(), i, argc)
                {
                    self.note_param_read(x, &k);
                }
            }
        }
        // a parsed value handed to a builtin whose rows agree on the kind
        // they take there is read as that kind (`upper[j.name]` text,
        // `abs[j.n]` a number); a function's parameter takes it as it is,
        // and the function's own body reads it
        if !self.ctx.functions.contains_key(&name) && lowered.iter().any(|x| matches!(x.ty().payload(), Ty::Json)) {
            let argc = lowered.len();
            // an item handed to a collection is read as its items are
            // (`push[ids, j.id]` into ints an int, `j.name in names` text):
            // read as the number the push rows take, it had made a list of
            // ints floats, and the lowered program disagreed with itself
            let item_at = match name.as_str() {
                "push" | "contains" if argc == 2 => Some(1),
                "set" if argc == 3 => Some(2),
                _ => None,
            };
            let items = item_at.and_then(|_| lowered.first()).and_then(|c| Self::items_kind(&c.ty(), name == "contains"));
            // (a collection whose items nothing says yet — `[]`, `map[]` —
            // takes a parsed value as it is: read as the number push's rows
            // take, `push[[], j]` had stopped on a list or a map — crow check
            // --self)
            // (one made here only: a variable's items are what its later
            // uses make them — a frame's cell pushed onto `hi` is a number
            // for the `max` after it)
            // (and one whose items are parsed values holds another as it is)
            let open = item_at.is_some()
                && lowered.first().is_some_and(|c| {
                    matches!(c.ty().payload(), Ty::List(e) | Ty::Map(_, e) if **e == Ty::Json)
                        || (made_here(c) && matches!(c.ty().payload(), Ty::List(e) | Ty::Map(_, e) if **e == Ty::Unknown))
                });
            // (a parsed value pushed onto a variable's list whose items
            // nothing says yet is read as push's rows take it, a number, so a
            // later `max` reads numbers — packages/market's bars; where it
            // holds text the stop says how to keep parsed values: Vijay,
            // 2026-10-29: Rosetta Code, Associative array/Merging)
            let held_in = match lowered.first() {
                Some(Expr::Local(v, _) | Expr::Move(v, _)) if item_at.is_some() => Some((self.current_fn.clone(), v.clone())),
                _ => None,
            };
            if let Some(at) = &held_in
                && !open
                && lowered.first().is_some_and(|c| matches!(c.ty().payload(), Ty::List(e) if **e == Ty::Unknown))
            {
                self.ctx.unsaid_pushes.insert(at.clone());
            }
            let unsaid = held_in.as_ref().is_some_and(|at| self.ctx.unsaid_pushes.contains(at));
            let mut read_args = Vec::with_capacity(argc);
            for (i, x) in lowered.into_iter().enumerate() {
                let kind = match (matches!(x.ty().payload(), Ty::Json), &items) {
                    (false, _) => None,
                    (true, _) if open && item_at == Some(i) => None,
                    (true, Some(k)) if item_at == Some(i) => Some(k.clone()),
                    // (a response's body is text, `respond`'s status an int:
                    // forms of web's own, which no row says; and `json` of
                    // a parsed value in a web program answers it as it is,
                    // where read as text it had been parsed again)
                    (true, _) if module.is_none() && name == "json" && argc == 1 && self.web_form(&name) => None,
                    (true, _) if module.is_none() && Self::web_form_reads(&name, i).is_some() && self.web_form(&name) => Self::web_form_reads(&name, i),
                    (true, _) => self.rows_say(&name, module.as_deref(), i, argc),
                };
                match kind {
                    Some(k) => {
                        // an argument the call marked (`unwrap x`, x can fail):
                        // the read takes the value that can fail, and the mark
                        // moves onto the read — handed the mark, which only a
                        // call that hands a failure on reads through, the read
                        // had been given the failed value itself (compiled,
                        // `upper[j.names[1]]` had not built)
                        // (typed, as every argument is, before the mark)
                        let (x, marked) = match x {
                            Expr::Convert { how: Convert::Unwrap, expr: inner, .. } if inner.ty().is_result() => (self.read_json(*inner, &k, &format!("handed to {}", name)), true),
                            x => (self.read_json(x, &k, &format!("handed to {}", name)), false),
                        };
                        // (the stop says why it was read so, and how to keep
                        // parsed values)
                        let x = match x {
                            Expr::Call { target, mut args, ty, propagates } if unsaid && item_at == Some(i) && args.len() == 2 => {
                                if let Expr::Const(Const::Str(said), _) = &mut args[1] {
                                    said.push_str(", onto a list whose items nothing said yet, so read as a number — push it onto a list made in the call, `push[[], …]`, to keep parsed values");
                                }
                                Expr::Call { target, args, ty, propagates }
                            }
                            x => x,
                        };
                        arg_types[i] = read(&x);
                        full_types[i] = full(&x);
                        checks[i] = self.checked(&x);
                        let x = if marked && x.ty().is_result() {
                            let ty = x.ty().payload().clone();
                            Expr::Convert { how: Convert::Unwrap, expr: Box::new(x), ty }
                        } else {
                            x
                        };
                        read_args.push(x);
                    }
                    None => read_args.push(x),
                }
            }
            lowered = read_args;
        }
        // `len[sub[s, pattern, r]]` of text: the length the substitution
        // makes, counted without making the text (regex-redux counted nine
        // patterns of a 50 MB sequence so, each a copy, nine at once)
        if name == "len" && module.is_none() && lowered.len() == 1
            && let Some(fused) = Self::sub_len_of(&lowered[0])
        {
            return Ok(fused);
        }
        // `map[keys, values]` of a tensor of values: its numbers as a list,
        // for a map of floats (the row takes a list of values of one kind)
        if name == "__map_of" && lowered.len() == 2 {
            // (a tensor of keys is a list of ints; a tensor of values a list of floats)
            for (i, item) in [(0, Ty::Int), (1, Ty::Float)] {
                let tensor_of_ints = i == 1 || matches!(lowered[0].ty().payload(), Ty::Tensor(e, _) if **e == Ty::Int);
                if tensor_of_ints && matches!(lowered[i].ty().payload(), Ty::Tensor(..)) && !lowered[i].ty().is_result() {
                    as_list(&mut lowered, i, item);
                    arg_types[i] = read(&lowered[i]);
                    full_types[i] = full(&lowered[i]);
                    checks[i] = self.checked(&lowered[i]);
                }
            }
        }
        // `[]` asked whether it holds an item is a list of the item's kind:
        // typed before the call is (the rows of `contains` answer
        // differently, so a list of nothing known had typed the call as
        // nothing known — crow check --self)
        if name == "contains"
            && module.is_none()
            && lowered.len() == 2
            && matches!(&lowered[0], Expr::ListLit(items, Ty::List(e)) if items.is_empty() && **e == Ty::Unknown)
            && !matches!(lowered[1].ty().payload(), Ty::Unknown)
        {
            let item = lowered[1].ty().payload().clone();
            lowered[0] = Expr::ListLit(Vec::new(), Ty::List(Box::new(item)));
            arg_types[0] = read(&lowered[0]);
            full_types[0] = full(&lowered[0]);
            checks[0] = self.checked(&lowered[0]);
        }
        self.check_callee(callee, args, &checks);
        self.scan_call(callee, args, &full_types);
        // a list handed to a `?` is sent as (?, ?, …), which only `IN` reads:
        // finch's `n.symbol = ANY(?)` handed its symbols failed at the
        // database every time (Postgres wants an array there, not a row)
        if crate::table::statement_form(module.as_deref(), &name).is_some() {
            let items = match lowered.get(2) {
                Some(Expr::ListLit(items, _)) => Some(items),
                Some(Expr::Convert { how: Convert::Unwrap, expr, .. }) => match expr.as_ref() {
                    Expr::ListLit(items, _) => Some(items),
                    _ => None,
                },
                _ => None,
            };
            if let (Some(A::Str(sql)), Some(A::Array(_)), Some(items)) = (args.get(1), args.get(2), items) {
                let before = words_before_placeholders(sql);
                for (i, item) in items.iter().enumerate() {
                    let t = read(item);
                    let t = if let Ty::Result(inner) = t { *inner } else { t };
                    let is_list = matches!(t, Ty::List(_) | Ty::Tensor(..));
                    if is_list && before.get(i).is_some_and(|w| !w.eq_ignore_ascii_case("IN")) {
                        self.refuse(format!("{}: parameter {} is a list, which is sent as (?, ?, …) — only `IN ?` reads one: write `x IN ?`", name, i + 1));
                    }
                }
            }
        }
        let ty = self.call_type(callee, args, &arg_types, &full_types, None);
        let ty = self.frame_typed(callee, args, &arg_types, &lowered, ty);
        let ty = self.column_typed(callee, args, &arg_types, &lowered, ty);
        if name == "chart" && matches!(module.as_deref(), None | Some("chart")) {
            self.check_chart_columns(&lowered);
        }
        let current = self.current_fn.clone();
        // a form that takes a statement: its provider says whether the
        // literal writes, and a write, or a statement built as the program
        // runs, takes the form's writing twin (table::Provider)
        let twin = self.statement_twin(module.as_deref(), &name, &lowered);
        let refused_as_computed = twin.is_some_and(|(_, refused)| refused);
        // (the module stays as the program wrote it: a bare call keeps its
        // bare path through both backends, a qualified one its module's)
        let name = match &twin {
            Some((t, _)) => t.name.to_string(),
            None => name,
        };
        if let Some(m) = module {
            let m = self.ctx.module_aliases.get(&m).cloned().unwrap_or(m);
            self.refuse_module_call(&m, &name, &lowered);
            if crate::table::is_effect_in(&m, &name) {
                // a qualified effect keeps its module: frame.load, db.query
                // (the module's own row says: ui's `input` is an element,
                // the core's waits on the reader)
                if self.pass == Pass::Final {
                    if !refused_as_computed {
                        self.effect_in_context_in(&m, &name);
                    }
                    self.direct_effects.entry(current).or_default().insert(name.clone());
                }
                let lowered = self.row_arguments(&m, &name, lowered);
                let row = self.builtin_row(Some(&m), &name, &lowered);
                self.retype_unknown_lists(row, &lowered);
                self.kindless_args(row, &lowered);
                let ty = Self::shaped_by_row(Some(&m), row, &lowered, ty, &name);
                let lowered = Self::crow_call_args(row, lowered);
                return Ok(Expr::Effect { name: format!("{}.{}", m, name), row, args: lowered, ty, answered: true, propagates });
            }
            let lowered = self.row_arguments(&m, &name, lowered);
            let row = self.builtin_row(Some(&m), &name, &lowered);
            self.retype_unknown_lists(row, &lowered);
            self.kindless_args(row, &lowered);
            let ty = Self::typed_by_row(ty, row);
            let ty = Self::shaped_by_row(Some(&m), row, &lowered, ty, &name);
            let lowered = Self::crow_call_args(row, lowered);
            let target = match row {
                Some(row) if row.is_crow() => self.crow_target(row),
                Some(row) => Target::Row(row),
                None => Target::Module(m, name),
            };
            return Ok(Expr::Call { target, args: lowered, ty, propagates });
        }
        // (a program's function, or a copy of one, is invisible inside a
        // standard module's function: sys's `answer[…]` is its row, not a
        // program's `fn answer` — but one a dispatch there is applying by
        // name, a group's member handed in, is the program's function it
        // names: `applying`)
        let visible = crate::modules::is_crow_fn(&name) || !crate::modules::is_crow_fn(&self.current_fn) || self.ctx.applying.iter().any(|f| *f == name);
        if (self.ctx.functions.contains_key(&name) || self.ctx.specializations.contains_key(&name)) && visible {
            if let Some(seen) = &mut self.seen {
                seen.calls.push((name.clone(), full_types.clone()));
            }
            // a render reads state and never moves it (spec, Reactive State):
            // the interpreter wrote the fact mid-render, and the compiled
            // program did not build
            if self.pass == Pass::Final && matches!(self.at, Where::In(FnKind::View) | Where::Derive) {
                if let Some(body) = self.ctx.functions.get(&name).map(|sig| sig.body.clone()) {
                    if matches!(self.kind_of(&name, &body, false), FnKind::Handler | FnKind::Helper) {
                        let from = if self.at == Where::Derive { "a derivation" } else { "a view" };
                        self.refuse(format!(
                            "{} calls '{}', which assigns reactive state — {} reads state and never changes it: move the assignment into a handler, or give the local a name that is not a fact's",
                            from, name, from
                        ));
                    }
                }
            }
            // a test's call, for what only tests type (`type_clicks`) — and
            // the functions it hands as values, typed by the parameters they
            // are handed to as a call outside a test hands them
            if let Some(tested) = &mut self.tested {
                tested.push((name.clone(), lowered.iter().map(read).collect()));
                self.ctx.hand_functions(&name, args);
            }
            let resolved = self.specialised(&name, &full_types);
            // an int handed to a float parameter is a float (spec §1), as one
            // assigned to a float variable is: the conversion is explicit
            // — and numbers held as a tensor handed to a parameter that holds
            // them as a list, or the other way, are converted where they meet
            // (a literal handed where a call elsewhere made the parameter a
            // list did not build)
            if let Some(sig) = self.ctx.functions.get(&resolved).cloned() {
                let params: Vec<Ty> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                // a function taking a function is one function for every call
                // (spec, Specialization: it is not copied per kind), so an
                // argument of another kind than its other calls give is
                // refused where it is handed — it had been an internal error
                // ("hands 'apply' a column for its parameter 'x', which is
                // a number"; crow check --self)
                if self.pass == Pass::Final && params.iter().any(|p| matches!(p, Ty::Fn(..))) {
                    let names: Vec<String> = sig.params.iter().map(|(n, _)| n.clone()).collect();
                    let whole = |t: &Ty| matches!(t, Ty::Int | Ty::Big);
                    // (and a list of ints is no list of records: an int is
                    // a record's none only handed alone)
                    let alike = |held: &Ty, given: &Ty| held.compatible(given) && crate::type_infer::TypeContext::items_alike_strictly(held, given);
                    let misfit = params.iter().zip(lowered.iter()).enumerate().find(|(_, (p, x))| {
                        let (held, given) = (sans_results(p.payload()), sans_results(x.ty().payload()));
                        // (a parsed value is a kind too: the one function
                        // cannot read it as each call's kind)
                        !matches!(held, Ty::Fn(..) | Ty::Unknown)
                            && !matches!(given, Ty::Unknown)
                            && !alike(&held, &given)
                            && !(whole(&held) && whole(&given))
                            && !alike(&held, &as_inferred(&given))
                    });
                    // (and a function handed where another call hands one
                    // answering another kind: the one parameter cannot
                    // read both answers — it had been typed by one, and
                    // the call handing the other did not build compiled;
                    // Rosetta Code, Monads/List monad)
                    let answers = |t: &Ty| match t {
                        Ty::Fn(_, r) if r.is_concrete() => Some(r.payload().clone()),
                        _ => None,
                    };
                    // (a member of a list of functions is its name, called
                    // through the list's dispatch: where another call hands a
                    // function itself, the parameter is that function's, and
                    // the member had reached it as text — an internal error;
                    // Rosetta Code, Deming's funnel)
                    if let Some((i, _)) = params.iter().zip(lowered.iter()).enumerate().find(|(_, (p, x))| matches!(p, Ty::Fn(..)) && matches!(x.ty().payload(), Ty::Fns(_))) {
                        self.refuse(format!(
                            "`{0}`'s '{1}' is a function another call hands by name, and this is a member of a list of functions — hand the function by name (`{0}[double, …]`), or call the member where it is (`fs[i][x]`)",
                            crate::lower::shown(&name),
                            names.get(i).cloned().unwrap_or_default()
                        ));
                    }
                    let fn_misfit = params.iter().zip(lowered.iter()).enumerate().find(|(_, (p, x))| matches!((answers(p), answers(&x.ty())), (Some(a), Some(b)) if !a.compatible(&b)));
                    if misfit.is_none()
                        && let Some((i, (p, x))) = fn_misfit
                    {
                        let words = |t: &Ty| answers(t).map(|a| crate::table::Arg::of_type(&a).words()).unwrap_or_default();
                        let msg = format!(
                            "`{0}` takes a function, so one `{0}` serves every call, and its '{1}' answers {2} here where another call's answers {3} — a function taking a function is not copied per kind (spec, Specialization): write one for each kind",
                            crate::lower::shown(&name),
                            names.get(i).cloned().unwrap_or_default(),
                            words(&x.ty()),
                            words(p)
                        );
                        self.refuse(msg);
                    }
                    if let Some((i, (p, x))) = misfit {
                        // (records named by their kind: `a list of records
                        // here where another call makes it a list of records`
                        // had named two kinds alike; Rosetta Code, Hash join)
                        let words = |t: &Ty| match t.payload() {
                            Ty::Record(n) | Ty::OptRecord(n) => format!("a {}", crate::lower::shown(n)),
                            Ty::List(item) => match item.payload() {
                                Ty::Record(n) | Ty::OptRecord(n) => format!("a list of {}", crate::lower::shown(n)),
                                _ => crate::table::Arg::of_type(t.payload()).said(t.payload()),
                            },
                            other => crate::table::Arg::of_type(other).said(other),
                        };
                        let msg = format!(
                            "`{0}` takes a function, so one `{0}` serves every call, and its '{1}' is {2} here where another call makes it {3} — a function taking a function is not copied per kind (spec, Specialization): write one for each kind",
                            crate::lower::shown(&name),
                            names.get(i).cloned().unwrap_or_default(),
                            words(&x.ty()),
                            words(p)
                        );
                        self.refuse(msg);
                    }
                }
                let mut handed = Vec::with_capacity(lowered.len());
                for (i, x) in lowered.into_iter().enumerate() {
                    handed.push(match (params.get(i), x.ty()) {
                        (Some(Ty::Float), Ty::Int) => Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty: Ty::Float },
                        // (a map of ints where the parameter holds floats —
                        // a map of floats handed at another call — is a map of
                        // floats; Rosetta Code, Runtime evaluation/In an
                        // environment)
                        (Some(p @ Ty::Map(_, pe)), Ty::Map(_, e)) if **pe == Ty::Float && *e == Ty::Int => Expr::Convert { how: Convert::IntsToFloats, expr: Box::new(x), ty: (p).clone() },
                        (Some(p @ Ty::List(_)), Ty::Tensor(..)) => Expr::Convert { how: Convert::ToList, expr: Box::new(x), ty: (p).clone() },
                        // (a list of ints where the parameter holds floats —
                        // a vector handed at another call — is a list of
                        // floats, as one assigned is)
                        (Some(p @ Ty::List(pe)), Ty::List(e)) if **pe == Ty::Float && *e == Ty::Int => Expr::Convert { how: Convert::IntsToFloats, expr: Box::new(x), ty: (p).clone() },
                        // (an empty literal, whose items are not known, is
                        // any list the backends need: parity 71)
                        (Some(p @ Ty::Tensor(..)), Ty::List(e)) if *e != Ty::Unknown => Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: (p).clone() },
                        // (0 handed where a record or none is held is none,
                        // typed as the parameter holds it)
                        (Some(p @ Ty::OptRecord(_)), Ty::Int) if matches!(x, Expr::Const(Const::Int(0), _)) => Expr::Const(Const::Int(0), p.clone()),
                        (Some(p @ Ty::OptRecord(_)), Ty::Int) => self.none_of(x, p),
                        // (and a list of ints where it holds a list of them —
                        // a list of 0s the function writes records into — is
                        // a list of nones, each int checked so)
                        (Some(p @ Ty::List(pe)), Ty::List(e)) if matches!(pe.as_ref(), Ty::OptRecord(_)) && *e == Ty::Int => nones_of(x, p),
                        _ => x,
                    });
                }
                lowered = handed;
            }
            // (every pass records the call under the names it resolved, so a
            // refusal in a copy lowered before its caller's body in the
            // final pass still finds the chain the settle pass drew)
            if self.v.line > 0 {
                self.ctx.module_calls.entry((current.clone(), resolved.clone())).or_insert(self.v.line);
            }
            if self.pass == Pass::Final {
                self.call_lines.entry((current.clone(), resolved.clone())).or_insert(self.v.line);
                self.calls.entry(current).or_default().insert(resolved.clone());
            }
            return Ok(Expr::Call { target: Target::User(resolved), args: lowered, ty, propagates });
        }
        // str[city], int[n], float[x] of a parsed value a parameter holds:
        // the kind the body reads it as, for the function's tool schema
        if module.is_none() && lowered.len() == 1 && matches!(name.as_str(), "str" | "int" | "float") {
            let kind = match name.as_str() {
                "str" => Ty::Str,
                "int" => Ty::Int,
                _ => Ty::Float,
            };
            self.note_param_read(&lowered[0], &kind);
        }
        if is_effect(&name) {
            if self.pass == Pass::Final {
                if !refused_as_computed {
                    self.effect_in_context(&name);
                }
                self.direct_effects.entry(current).or_default().insert(name.clone());
            }
            // (an effect with a row is handed its arguments as any row's
            // call is — the core's, or the module's that answers the name:
            // a request's or a reader's options left out are maps of text)
            let lowered = match self.module_for(&name, &lowered) {
                Some(m) => {
                    // (a count none of the module's rows take is refused, as
                    // a call's is: `mail` with three arguments)
                    if let Some(e) = crate::table::miscounted(&m, &name, lowered.len()) {
                        self.refuse(e);
                    }
                    self.row_arguments(&m, &name, lowered)
                }
                None => self.core_row_arguments(&name, lowered),
            };
            let row = self.builtin_row(None, &name, &lowered);
            self.retype_unknown_lists(row, &lowered);
            self.kindless_args(row, &lowered);
            let lowered = Self::concrete_empties(&name, row, lowered);
            let ty = Self::shaped_by_row(None, row, &lowered, ty, &name);
            let lowered = Self::crow_call_args(row, lowered);
            return Ok(Expr::Effect { name, row, args: lowered, ty, answered: true, propagates });
        }
        if RUNTIME_TABLE.iter().any(|(b, _)| *b == name) {
            // a list holds one kind: two lists of two kinds are not joined
            // (the interpreter had made a list of both, `[true, 3]`, and the
            // compiled program had not built — crow check --self)
            if name == "concat" && lowered.len() >= 2 && self.pass == Pass::Final {
                let item = |x: &Expr| match x.ty().payload() {
                    Ty::List(e) => Some(sans_results(e)),
                    // (a tensor's are numbers: beside a mask, two kinds)
                    Ty::Tensor(..) => Some(Ty::Float),
                    _ => None,
                };
                if let (Some(a), Some(b)) = (item(&lowered[0]), item(&lowered[1]))
                    && !matches!(a, Ty::Unknown)
                    && !matches!(b, Ty::Unknown)
                    && !a.compatible(&b)
                    && !(matches!(a, Ty::Int | Ty::Big) && matches!(b, Ty::Int | Ty::Big))
                {
                    // (a matrix named so, and the way to join one: it had
                    // been called "numbers" beside a list of lists; Rosetta
                    // Code, Maximum triangle path sum)
                    let matrix = |x: &Expr| matches!(x.ty().payload(), Ty::Tensor(_, Some(d)) if d.len() >= 2);
                    let words = |x: &Expr| if matrix(x) { "a matrix".to_string() } else { crate::table::Arg::of_type(x.ty().payload()).said(&x.ty()) };
                    let hint = if matrix(&lowered[0]) || matrix(&lowered[1]) { " — join a matrix with a matrix: math's tensor[rows] makes one of a list of rows" } else { " — a record (struct) holds fields of several kinds" };
                    self.refuse(format!(
                        "concat[] joins lists of one kind, and these are {} and {}: a list holds one kind{}",
                        words(&lowered[0]),
                        words(&lowered[1]),
                        hint
                    ));
                }
            }
            // (whole numbers written down beside a list of ints are that
            // list's kind: `concat[[1], xs]` of growing ints had been joined
            // as a tensor — the digits past 2^53 lost compiled, a stop
            // interpreted; Rosetta Code, Write float arrays to a text file)
            let ty = match (name.as_str(), lowered.first(), lowered.get(1).map(|x| x.ty())) {
                ("concat", Some(Expr::ListLit(items, Ty::Tensor(e, d))), Some(second))
                    if **e == Ty::Int && d.as_ref().is_none_or(|d| d.len() <= 1) && matches!(second.payload(), Ty::List(i) if matches!(i.payload(), Ty::Int | Ty::Big)) =>
                {
                    lowered[0] = Expr::ListLit(items.clone(), Ty::List(Box::new(Ty::Int)));
                    let list = second.payload().clone();
                    if ty.is_result() { Ty::Result(Box::new(list)) } else { list }
                }
                _ => ty,
            };
            // a list of numbers is held as a list or as a tensor: `concat`
            // joins them in the first's form, so the second is converted
            if name == "concat" && lowered.len() >= 2 {
                let how = match (lowered[0].ty().payload(), lowered[1].ty().payload(), ty.payload()) {
                    (Ty::List(_), Ty::Tensor(..), Ty::List(_)) => Some(Convert::ToList),
                    (Ty::Tensor(..), Ty::List(_), Ty::Tensor(..)) => Some(Convert::ToTensor),
                    _ => None,
                };
                if let Some(how) = how {
                    let second = lowered.remove(1);
                    // (typed by what it converts: a tensor of floats beside a
                    // mask is a list of floats, not the answer's bools —
                    // which the conversion after it had read as truths)
                    let converted = match (&how, second.ty().payload()) {
                        (Convert::ToList, Ty::Tensor(e, _)) => Ty::List(e.clone()),
                        (Convert::ToTensor, _) => Ty::vector(Ty::Float),
                        _ => ty.payload().clone(),
                    };
                    lowered.insert(1, Expr::Convert { how, expr: Box::new(second), ty: converted });
                }
            }
            // (lists whose items differ only in whether they can fail are
            // joined as lists of items that can fail, the plain one's items
            // each the success it is: `concat[[f[x]], ['b']]` had joined a
            // list of text that can fail with one of text, and the compiled
            // program did not build; Rosetta Code, Facial detection)
            if name == "concat" && lowered.len() >= 2 {
                let fallible = lowered.iter().find_map(|x| match x.ty() {
                    Ty::List(i) if i.is_result() => Some((*i).clone()),
                    _ => None,
                });
                if let Some(item) = fallible {
                    lowered = lowered
                        .into_iter()
                        .map(|x| match x.ty() {
                            Ty::List(i) if !i.is_result() && i.as_ref() == item.payload() => fallible_items(x, &item),
                            _ => x,
                        })
                        .collect();
                }
            }
            let lowered = self.core_row_arguments(&name, lowered);
            // the collection's form is the answer's (`held_as` converted the
            // item or the value to it): a tensor set into a map of lists
            // answers a map of lists, a list pushed onto a list of rows a
            // list of rows (the answer had been in the join's form, which
            // the variable's declaration then disagreed with)
            let ty = in_holders_form(&name, &lowered, ty);
            // (a local pushed before its items are known — `groups =
            // push[groups, group]` while `group` is still `[]` — makes the
            // holder's items what the local settles on: met in order, the
            // holder had stayed a list of lists of nothing known, compiled
            // as lists of text; Rosetta Code, Brilliant numbers)
            if self.pass == Pass::Settle
                && matches!(name.as_str(), "push" | "set")
                && let (Some(Expr::Local(holder, _) | Expr::Move(holder, _)), Some(Expr::Local(item, t) | Expr::Move(item, t))) = (lowered.first(), lowered.last())
                && lowered.len() >= 2
                && t.has_unknown()
                && holder != item
            {
                let link = (self.current_fn.clone(), holder.clone(), item.clone());
                if !self.items_of.contains(&link) {
                    self.items_of.push(link);
                }
            }
            let row = self.builtin_row(None, &name, &lowered);
        self.retype_unknown_lists(row, &lowered);
        self.kindless_args(row, &lowered);
            // (a row answering a tensor answers one, whatever the call's rule
            // read from a list given it: `concat[xs, ys, 0]` of lists joins
            // their tensors — typed a list, the compiled program had not
            // built — crow check --self)
            let ty = match row {
                Some(r) if r.ret == crate::table::Kind::Tensor && matches!(ty.payload(), Ty::List(_)) => {
                    let t = Ty::tensor(Ty::Float);
                    if ty.is_result() { Ty::Result(Box::new(t)) } else { t }
                }
                _ => ty,
            };
            let lowered = Self::concrete_empties(&name, row, lowered);
            let ty = Self::answered_as_empties(ty, row, &lowered);
            let (lowered, ty) = match row {
                Some(row) => Self::widen_items(row, lowered, ty),
                None => (lowered, ty),
            };
            let ty = Self::shaped_by_row(None, row, &lowered, ty, &name);
            let target = match row {
                Some(row) => {
                    self.check_item(row, &lowered);
                    self.refine_pushed(row, &lowered);
                    Target::Row(row)
                }
                None => Target::Runtime(runtime_name(&name)),
            };
            return Ok(Expr::Call { target, args: lowered, ty, propagates });
        }
        // a bare name a loaded module provides reaches it by the registry's
        // rule (`builtin_row`)
        let lowered = match self.module_for(&name, &lowered) {
            Some(m) => {
                // a name only a module answers — not the core's, not an
                // element's — reaches it, so a count none of its rows take
                // is refused as a qualified call's is
                // (a ui control named like a module's function — `select` of
                // options, not of a frame — is the control's)
                let first_is_frame = lowered.first().is_some_and(|x| matches!(x.ty().payload(), Ty::Frame(_)));
                // (never inside a standard module's body: no module imports
                // ui, and its `select[whole, names]` is frame's)
                let control = self.in_ui && !crate::modules::is_crow_fn(&self.current_fn) && crate::lower::is_ui_control(&name, first_is_frame);
                // (an element's name is an element's in a ui program only:
                // elsewhere `stack[rows]` is math's, checked as math's —
                // unchecked, a list no row takes had reached the verifier)
                let element = self.in_ui && crate::lower::UI_ELEMENTS.contains(&name.as_str());
                if !CORE_RESERVED.contains(&name.as_str()) && !element && !control {
                    self.refuse_module_call(&m, &name, &lowered);
                }
                self.row_arguments(&m, &name, lowered)
            }
            None => self.core_row_arguments(&name, lowered),
        };
        let row = self.builtin_row(None, &name, &lowered);
        self.retype_unknown_lists(row, &lowered);
        self.kindless_args(row, &lowered);
        let ty = Self::typed_by_row(ty, row);
        let lowered = Self::concrete_empties(&name, row, lowered);
        let ty = Self::answered_as_empties(ty, row, &lowered);
        let (lowered, ty) = match row {
            Some(row) => Self::widen_items(row, lowered, ty),
            None => (lowered, ty),
        };
        let ty = Self::shaped_by_row(None, row, &lowered, ty, &name);
        let lowered = Self::crow_call_args(row, lowered);
        let target = match row {
            Some(row) if row.is_crow() => self.crow_target(row),
            Some(row) => {
                self.check_item(row, &lowered);
                self.refine_pushed(row, &lowered);
                Target::Row(row)
            }
            None => Target::Runtime(name),
        };
        Ok(Expr::Call { target, args: lowered, ty, propagates })
    }

    /// A call nothing typed answers what its row does, where the row's answer
    /// is no collection of its arguments' items: `unique[f, []]` — the rows
    /// of the name answering a column or a frame, `[]` saying neither — had
    /// been typed as nothing known, and the compiled program had not built
    /// (crow check --self).
    /// A statement form's writing twin, where the call takes it: the form's
    /// provider says whether the literal statement writes, and a statement
    /// built as the program runs writes for all the compiler can tell — in
    /// a context that may read but not write, refused with that said, the
    /// fix being the literal.
    fn statement_twin(&mut self, module: Option<&str>, name: &str, lowered: &[Expr]) -> Option<(&'static crate::table::Row, bool)> {
        if self.ctx.functions.contains_key(name) && !crate::modules::is_crow_fn(&self.current_fn) {
            return None;
        }
        let m = match module {
            Some(m) => Some(self.ctx.module_aliases.get(m).cloned().unwrap_or_else(|| m.to_string())),
            None => self.module_for(name, lowered),
        };
        let row = crate::table::statement_form(m.as_deref(), name)?;
        let provider = row.provider?;
        let twin = crate::table::writing_twin(row)?;
        fn plain(e: &Expr) -> &Expr {
            match e {
                Expr::Convert { expr, .. } => plain(expr),
                other => other,
            }
        }
        let literal = lowered.get(provider.statement).map(plain).and_then(|x| match x {
            Expr::Const(Const::Str(s), _) => Some(crate::type_infer::scrubbed(s)),
            _ => None,
        });
        if !(provider.writes)(literal.as_deref()) {
            return None;
        }
        let mut refused = false;
        if literal.is_none() && self.pass == Pass::Final {
            if let Some(ctx) = self.context() {
                let reads = crate::table::effects_of_in(row.module, row.name).into_iter().all(|e| ctx.allows(e));
                let writes = crate::table::effects_of_in(twin.module, twin.name).into_iter().any(|e| !ctx.allows(e));
                if reads && writes {
                    self.refuse(format!(
                        "{}[]'s statement is built as the program runs, so whether it reads or writes is known only then — {} reads: write the statement as a literal, with ? for its parameters",
                        name,
                        ctx.word()
                    ));
                    refused = true;
                }
            }
        }
        Some((twin, refused))
    }

    /// A builtin's tensor has the shape its row declares (`table::Shape`),
    /// read from what the program states in the call: the rank every
    /// answer has, and the sizes that follow.
    fn shaped_by_row(module: Option<&str>, row: Option<&'static crate::table::Row>, args: &[Expr], ty: Ty, name: &str) -> Ty {
        let dims = match row {
            Some(r) => shapes::declared(r, args),
            None => shapes::agreed(module, name, args),
        };
        match dims {
            Some(d) => shapes::shaped(ty, Some(d)),
            None => ty,
        }
    }

    fn typed_by_row(ty: Ty, row: Option<&'static crate::table::Row>) -> Ty {
        match row {
            Some(r) if matches!(ty.payload(), Ty::Unknown) && !r.ret.generic() && !matches!(r.ret.ty(), Ty::Unknown) => {
                let t = r.answer();
                if ty.is_result() || matches!(r.fails, crate::table::Fails::Value(_)) { Ty::Result(Box::new(t)) } else { t }
            }
            _ => ty,
        }
    }

    /// `[]` written where a row's answer says nothing of its items — `len[[]]`,
    /// `type[[]]`, `encode[[]]`, `log[[]]` — is a list of ints: every kind
    /// answers alike, and with none the compiled program had had no type to
    /// give it (crow check --self). A row whose answer carries the items
    /// (`push`, `concat`, `slice`) leaves the list to its other uses.
    fn concrete_empties(name: &str, row: Option<&'static crate::table::Row>, lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::Kind;
        // (`type` is a form of its own, whose answer is a word; a row of
        // the list alone — `pop[[]]`, `reverse[[]]` — answers a list of
        // the same nothing)
        // (or whose other parameters say nothing of the items — `slice[[],
        // 3, 3]`, its positions ints)
        let alone = row.is_some_and(|r| {
            r.params.iter().filter(|k| matches!(k, Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef | Kind::MapOf | Kind::MapRef | Kind::Key | Kind::Keys)).count() == 1
        });
        // (or whose every list is `[]` and every item none — `concat[[],
        // []]`: nothing says the items, and the compiled program had had no
        // type for them — crow check --self's pairs)
        let unknown_empty = |x: &Expr| matches!(x, Expr::ListLit(items, Ty::List(i)) if items.is_empty() && **i == Ty::Unknown);
        let only_empties = lowered.iter().any(unknown_empty)
            && lowered.iter().all(|x| unknown_empty(x) || !matches!(x.ty().payload(), Ty::List(_) | Ty::Map(..) | Ty::Tensor(..) | Ty::Column(_) | Ty::Unknown | Ty::Json))
            && row.is_some_and(|r| !r.params.iter().any(|k| matches!(k, Kind::Item | Kind::ItemRef)));
        // (`json`'s response is a form of its own, which writes `[]` as
        // JSON writes an empty array: its items any kind — compiled, they
        // had had none, and the program did not build)
        if !matches!(name, "type" | "json") && !alone && !only_empties && row.is_none_or(|r| r.ret.generic()) {
            return lowered;
        }
        // (each list of the kind its parameter takes: `sort[f, []]` a list
        // of text, which a list of ints had been handed as)
        let kinds = row.and_then(|r| crate::table::kinds_of(r.params, lowered.len())).unwrap_or_default();
        let item_of = |i: usize| match kinds.get(i) {
            Some(Kind::Words | Kind::Strs | Kind::Texts) => Ty::Str,
            Some(Kind::Floats) => Ty::Float,
            Some(Kind::Bools) => Ty::Bool,
            Some(Kind::Chunks) => Ty::Bytes,
            Some(Kind::Frames) => Ty::Frame(None),
            Some(Kind::Tensors) => Ty::tensor(Ty::Float),
            _ => Ty::Int,
        };
        lowered
            .into_iter()
            .enumerate()
            .map(|(i, x)| match x {
                Expr::ListLit(items, Ty::List(item)) if items.is_empty() && *item == Ty::Unknown => Expr::ListLit(items, Ty::List(Box::new(item_of(i)))),
                // (and `map[]`, written or made by its call: a map of ints)
                Expr::MapLit(pairs, Ty::Map(_, v)) if pairs.is_empty() && *v == Ty::Unknown => Expr::MapLit(pairs, Ty::Map(crate::ir::Key::Text, Box::new(Ty::Int))),
                Expr::Call { target: Target::Row(row), args, ty: Ty::Map(_, v), .. } if row.id == "core.map_new" && args.is_empty() && *v == Ty::Unknown => {
                    Expr::MapLit(Vec::new(), Ty::Map(crate::ir::Key::Text, Box::new(Ty::Int)))
                }
                other => other,
            })
            .collect()
    }

    /// The answer of a row that answers its list's items — `slice[[], i, j]`,
    /// `concat[[], []]`, `reverse[[]]` — has the items `concrete_empties`
    /// gave its `[]` (the answer had kept items of no kind, which the
    /// compiled program wrote as text beside the list's ints, and a call
    /// whose argument could fail did not build — crow check --self's pairs).
    fn answered_as_empties(ty: Ty, row: Option<&'static crate::table::Row>, lowered: &[Expr]) -> Ty {
        let Some(row) = row else { return ty };
        if !row.ret.generic() || !matches!(ty.payload(), Ty::List(i) if **i == Ty::Unknown) {
            return ty;
        }
        let given = lowered.iter().find_map(|x| match x {
            Expr::ListLit(items, t @ Ty::List(i)) if items.is_empty() && **i != Ty::Unknown => Some(t.clone()),
            _ => None,
        });
        match given {
            Some(t) if ty.is_result() => Ty::Result(Box::new(t)),
            Some(t) => t,
            None => ty,
        }
    }

    /// A list whose items no type says, read where a row takes a list of a
    /// kind, is a list of that kind from then on, as a variable first given
    /// `[]` takes its pushes' kind: a parameter a call handed `[]`
    /// (`tensor[pred]` in `mse` reads an empty list of floats, converted
    /// where it meets the row — the row's `[]` had been an empty tensor),
    /// a local given `[]` (`stack[parts]` reads an empty list of frames),
    /// and a record's field given `[]` where the record was made (a tail's
    /// `parts`, read by `stack` in a program that writes no frame into
    /// them). Typed by nothing, the compiled program had held the list as
    /// text and not built (crow check --self; parity 322). A parameter
    /// settles in the scan, which the rounds read again; a local and a
    /// field in every provisional pass, as a field written does.
    /// An item of a list nothing is put into, handed where a row takes a
    /// number, text or a truth (`max[g[0], 1]`): its kind is none to compute
    /// with, refused as an operator's side is (`kindless_item`); a row that
    /// shows any value (`log`) reaches the index, which stops (Rosetta Code,
    /// Longest common subsequence).
    fn kindless_args(&mut self, row: Option<&'static crate::table::Row>, lowered: &[Expr]) {
        use crate::table::Kind;
        // (no row resolved: the item's lack of a kind is why — `max` has no
        // row for it)
        let Some(row) = row else {
            for x in lowered {
                self.kindless_item(x);
            }
            return;
        };
        let Some(kinds) = crate::table::kinds_of(row.params, lowered.len()) else { return };
        for (k, x) in kinds.iter().zip(lowered) {
            if matches!(k, Kind::Int | Kind::Whole | Kind::Float | Kind::Bool | Kind::Str) {
                self.kindless_item(x);
            }
        }
    }

    fn retype_unknown_lists(&mut self, row: Option<&'static crate::table::Row>, lowered: &[Expr]) {
        let Some(row) = row else { return };
        if self.pass == Pass::Final {
            return;
        }
        let Some(kinds) = crate::table::kinds_of(row.params, lowered.len()) else { return };
        // (only where every row of the name takes that list there: a list
        // of nothing is any list to the row's choice, so `sort[ready]` had
        // reached the core's first `sort` and typed `ready` numbers, which
        // the text pushed onto it then contradicted — packages/graph)
        let rows: Vec<Vec<crate::table::Kind>> = crate::table::ROWS
            .iter()
            .filter(|r| r.module == row.module && r.name == row.name)
            .filter_map(|r| crate::table::kinds_of(r.params, lowered.len()))
            .collect();
        for (i, (k, x)) in kinds.iter().zip(lowered).enumerate() {
            if !matches!(read(x).payload(), Ty::List(e) if **e == Ty::Unknown) {
                continue;
            }
            // (`join`'s list takes text, numbers and bools as they show: it
            // says nothing of the items — at the top level a list met it
            // before the push that typed it, and the int pushed was refused,
            // `its items are text`; a list nothing else types is text where
            // the compiled program holds it; Rosetta Code, Generator/
            // Exponential)
            if *k == crate::table::Kind::Words {
                continue;
            }
            let Some(item) = listed_item(*k) else { continue };
            // (a tensor's parameter takes numbers whatever the name's other
            // rows take: the rule a parameter handed `[]` has always had —
            // `tensor[x]` in math.stats's `corr` — which the agreement must
            // not undo)
            if *k != crate::table::Kind::Tensor && rows.iter().any(|ks| ks.get(i).and_then(|k| listed_item(*k)).as_ref() != Some(&item)) {
                continue;
            }
            self.read_as_list_of(x, item);
        }
    }

    /// A list whose items no type says — a parameter a call handed `[]`, a
    /// local given `[]`, a record's field given `[]` where the record was
    /// made — read as a list of `item` from here on (see
    /// `retype_unknown_lists`; `not xs` reads a mask, a list of bools). A
    /// provisional pass's: a parameter in the scan, which the rounds read
    /// again, a local and a field in every provisional pass.
    fn read_as_list_of(&mut self, x: &Expr, item: Ty) {
        if self.pass == Pass::Final || !matches!(read(x).payload(), Ty::List(e) if **e == Ty::Unknown) {
            return;
        }
        let list = Ty::List(Box::new(item));
        match x {
            Expr::Local(n, _) | Expr::Move(n, _) => {
                let scan = self.pass == Pass::Scan;
                match self.ctx.functions.get_mut(&self.current_fn).and_then(|sig| sig.params.iter_mut().find(|(p, _)| p == n)) {
                    Some(p) => {
                        if scan {
                            p.1 = list.clone();
                            self.ctx.vars.insert(n.clone(), list);
                        }
                    }
                    None => {
                        if self.ctx.vars.contains_key(n) && !self.is_fact(n) {
                            self.ctx.vars.insert(n.clone(), list);
                        }
                    }
                }
            }
            Expr::Field { base, name, .. } => {
                let rec = match base.ty().payload() {
                    Ty::Record(r) | Ty::OptRecord(r) => r.clone(),
                    _ => return,
                };
                if let Some(sig) = self.ctx.structs.get_mut(&rec)
                    && let Some((_, held)) = sig.fields.iter_mut().find(|(f, _)| f == name)
                    && matches!(held.payload(), Ty::List(e) if **e == Ty::Unknown)
                {
                    *held = if held.is_result() { Ty::Result(Box::new(list)) } else { list };
                }
            }
            _ => {}
        }
    }

    /// A Crow row's arguments as its parameters take them: an int where the
    /// row takes a number is a float, as the user-function path converts
    /// it (a Rust row's printing casts; the Crow body is a function of
    /// floats, and `timeout = 5` did not build compiled).
    fn crow_call_args(row: Option<&'static crate::table::Row>, lowered: Vec<Expr>) -> Vec<Expr> {
        let Some(row) = row.filter(|r| r.is_crow()) else { return lowered };
        // an argument as the parameter's kind takes it: an int handed to a
        // float, a value JSON can hold as its JSON text
        // (crate::table::crow_param_type)
        let read_as = |k: crate::table::Kind, x: Expr| -> Expr {
            match (k, x.ty()) {
                (crate::table::Kind::Float, Ty::Int) => Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty: Ty::Float },
                // a number where an int is taken: its whole part, as a row
                // written in Rust reads one (`randn[2.5]` had been a list of
                // ints holding a float)
                (crate::table::Kind::Int, Ty::Float) => match crate::table::ROWS.iter().find(|r| r.id == "core.__truncated") {
                    Some(row) => Expr::Call { target: Target::Row(row), args: vec![x], ty: Ty::Int, propagates: false },
                    None => x,
                },
                // a map of other values where a map of text is taken: each as
                // it shows, as a Rust row's parameter makes it (`load[path,
                // map[a = 1], …]` had not built — crow check --self)
                (crate::table::Kind::TextMap, Ty::Map(_, v)) if !matches!(v.as_ref(), Ty::Str | Ty::Unknown) => match crate::table::ROWS.iter().find(|r| r.id == "core.__text_map") {
                    Some(row) => Expr::Call { target: Target::Row(row), args: vec![x], ty: Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)), propagates: false },
                    None => x,
                },
                (crate::table::Kind::Encoded, ty) => {
                    let fails = ty.is_result();
                    let Some(encode) = crate::table::core_row("encode", &[crate::table::Arg::Unknown]) else { return x };
                    // an argument the call marked (`unwrap x`, x can fail):
                    // the encode reads the mark and hands the failure on, and
                    // the call reads its answer through a mark of its own —
                    // left inside an encode that hands nothing on, the mark
                    // was read by no one, and the compiled program encoded
                    // the failure's record where the call answered a failure
                    // (`msgpack[decimal['1.25']]` had not built — crow check
                    // --self)
                    if matches!(&x, Expr::Convert { how: Convert::Unwrap, expr, .. } if expr.ty().is_result()) {
                        let call = Expr::Call { target: Target::Row(encode), args: vec![x], ty: Ty::Result(Box::new(Ty::Str)), propagates: true };
                        return Expr::Convert { how: Convert::Unwrap, expr: Box::new(call), ty: Ty::Str };
                    }
                    Expr::Call { target: Target::Row(encode), args: vec![x], ty: if fails { Ty::Result(Box::new(Ty::Str)) } else { Ty::Str }, propagates: fails }
                }
                _ => x,
            }
        };
        // a parameter that takes the rest of the arguments (`anova[a, b,
        // c]`, `normal[0, 1, 3, 4]`) is handed them as one list, as the
        // function takes it; the parameters before it read theirs as above
        if row.params.iter().any(|k| k.is_rest()) {
            let Ok(spans) = crate::table::spans(row.params, lowered.len(), row.name) else { return lowered };
            let mut rest: Vec<Option<Expr>> = lowered.into_iter().map(Some).collect();
            let mut bundled = Vec::with_capacity(row.params.len());
            for (k, (a, b)) in row.params.iter().zip(spans) {
                if k.is_rest() {
                    let items: Vec<Expr> = (a..b).filter_map(|i| rest[i].take()).map(|x| read_as(k.each(), x)).collect();
                    let item = (&k.each().ty()).clone();
                    // one that can fail is gathered by a row's call, which
                    // hands its failure on as a call does: in a list
                    // literal the interpreter had handed the failed value to
                    // the body, which stopped on it, and the compiled
                    // program its default (`randn[may[3, true]]` was `[]`)
                    let fails = |x: &Expr| x.ty().is_result() || matches!(x, Expr::Convert { how: Convert::Unwrap, expr, .. } if expr.ty().is_result());
                    let gather = match k {
                        crate::table::Kind::Sizes => crate::table::ROWS.iter().find(|r| r.id == "core.__sizes"),
                        crate::table::Kind::Groups => crate::table::ROWS.iter().find(|r| r.id == "core.__groups"),
                        _ => None,
                    };
                    match gather {
                        Some(row) if items.iter().any(fails) => {
                            let list = Ty::List(Box::new(item));
                            let call = Expr::Call { target: Target::Row(row), args: items, ty: Ty::Result(Box::new(list.clone())), propagates: true };
                            bundled.push(Expr::Convert { how: Convert::Unwrap, expr: Box::new(call), ty: list });
                        }
                        _ => bundled.push(Expr::ListLit(items, Ty::List(Box::new(item)))),
                    }
                } else if let Some(x) = rest[a].take() {
                    bundled.push(read_as(*k, x));
                }
            }
            return bundled;
        }
        lowered
            .into_iter()
            .zip(row.params.iter().copied().map(Some).chain(std::iter::repeat(None)))
            .map(|(x, k)| match k {
                Some(k) => read_as(k, x),
                None => x,
            })
            .collect()
    }

    /// A builtin written in Crow (crate::table, `crow!`): its call is a call
    /// of the program's function its body is inlined as, the arguments read
    /// as the row's parameters take them (`row_arguments`), the answer the
    /// row's. Its stops name the line of this call (crate::modules).
    fn crow_target(&mut self, row: &'static crate::table::Row) -> Target {
        // (the copy for the row's own signature, were the function
        // specialised by another call)
        let given: Vec<Ty> = row.params.iter().map(|k| crate::table::crow_param_type(*k)).collect();
        let f = self.specialised(&crate::table::crow_fn_name(row), &given);
        if self.pass == Pass::Final {
            let current = self.current_fn.clone();
            self.calls.entry(current).or_default().insert(f.clone());
        }
        Target::User(f)
    }

    /// What the rows a call reaches say of its argument at a position
    /// (`uses::row_says`): a bare call's are the core's and those of the
    /// modules in scope that provide the name.
    fn rows_say(&self, name: &str, module: Option<&str>, at: usize, argc: usize) -> Option<Ty> {
        match module {
            Some(m) => uses::row_says(name, Some(m), at, argc),
            None => uses::row_says_within(name, &self.ctx.registry.providers_of(name), at, argc),
        }
    }

    /// The row a builtin's call runs, decided once, here, from its
    /// arguments' types, and named by the IR: both backends call it, where
    /// each had picked one of its own — the interpreter by the values, the
    /// emitter by the types (docs/bug-classes.md §4). The core's row the
    /// kinds pick comes first; else, where a loaded module answers the name,
    /// the module's row: a name the core and math both answer (`abs`,
    /// `max`) is math's only with a tensor among the arguments, a frame
    /// function the core answers too (`sort`, `join`) is the frame's only
    /// given a frame, and a ui control, or an element in a view, is a form
    /// of its own. A qualified call to a module the registry holds is its
    /// bare call, pinned to the module. None when an argument's kind is not
    /// known here (a harness's answer, a value nothing typed): its values
    /// pick the row as the program runs. A parsed value is JSON in both
    /// modes, and names the rows that take JSON (it had been left by name:
    /// the interpreter held it as the value it parsed to).
    /// The module a bare name reaches: the one that provides it — or, where
    /// several do (a standard module's function sees its own rows beside
    /// its imports': sys's `close` of a stream beside a socket's), the one
    /// whose row takes the arguments' kinds, as the kinds pick among one
    /// module's rows; none where they tell nothing apart.
    fn module_for(&self, name: &str, lowered: &[Expr]) -> Option<String> {
        let providers = self.ctx.registry.providers_of(name);
        let kinds: Vec<crate::table::Arg> = lowered.iter().map(|x| crate::table::Arg::of_type(&read(x))).collect();
        crate::table::module_of_bare(name, &providers, &kinds)
    }

    fn builtin_row(&self, module: Option<&str>, name: &str, lowered: &[Expr]) -> Option<&'static crate::table::Row> {
        use crate::table::Arg;
        // (a qualified call to a standard module the registry knows — web's
        // modules written in Crow among them — is pinned to it)
        let pinned = match module {
            Some(m) if crate::modules::module_by_path(m).is_some() && self.ctx.registry.has_function(name) => Some(m),
            Some(_) => return None,
            None => None,
        };
        let types: Vec<Ty> = lowered.iter().map(read).collect();
        let kinds: Vec<Arg> = types.iter().map(Arg::of_type).collect();
        if kinds.contains(&Arg::Unknown) {
            return None;
        }
        if let Some(row) = crate::table::core_row(name, &kinds) {
            return Some(row);
        }
        let registry = &self.ctx.registry;
        if !registry.has_function(name) {
            return None;
        }
        // a select in a page is the widget, an element in a view the
        // element: forms of their own, not a module's row. Which module a
        // bare name reaches is the one rule's (table::module_of_bare, in
        // module_for): a name the core also answers goes to the module
        // when the core's rows do not take the kinds — `max` of a tensor,
        // or of a list of numbers, is math's of the whole; `sort` of a list
        // with frame imported is the core's
        let control = name == "select" && self.in_ui && lowered.len() == 2 && registry.providers_of(name).contains(&"frame");
        let element = matches!(self.at, Where::In(FnKind::View)) && crate::lower::UI_ELEMENTS.contains(&name);
        if control || element {
            return None;
        }
        let chosen;
        let m = match pinned {
            Some(m) => m,
            None => {
                chosen = self.module_for(name, lowered)?;
                chosen.as_str()
            }
        };
        if crate::table::miscounted(m, name, lowered.len()).is_some() {
            return None;
        }
        crate::table::module(m, name, &kinds).map(|row| crate::table::exact_form(row, &kinds))
    }

    /// A builtin row's arguments as its parameters take them (docs/
    /// bug-classes.md §4): numbers held as a list, or a frame's column,
    /// handed to a parameter that takes a tensor are converted where they
    /// meet, the same way in both modes — compiled, a list did not build
    /// and a column's missing cells were read as 0, where the interpreter
    /// read the list and stopped at the missing cell. A name the core also
    /// answers (`max`, `abs`) picks its builtin by the values at run time —
    /// a list goes to the core's, a tensor to the module's — so a list
    /// handed to it is left as it is, and a column, numbers either way, is
    /// converted. An argument that can fail is handed on as its error.
    /// An empty list literal handed to a row's list parameter is a list of
    /// what the parameter takes: its items' type is not known from the
    /// literal, and the compiled program must name it (`join[[], ',']` did
    /// not build).
    fn typed_empties(kinds: &[crate::table::Kind], lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::Kind;
        lowered
            .into_iter()
            .zip(kinds.iter().copied().map(Some).chain(std::iter::repeat(None)))
            .map(|(x, k)| {
                // an empty map handed to a map of text is one (a request's
                // headers left out)
                if k == Some(Kind::TextMap) {
                    return match x {
                        Expr::MapLit(pairs, Ty::Map(_, e)) if pairs.is_empty() && *e == Ty::Unknown => Expr::MapLit(pairs, Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str))),
                        // (`map[]` written out is the call that makes one: a
                        // map of text where the row takes one, as the literal
                        // is — compiled, its type had nothing to be read from)
                        Expr::Call { target: Target::Row(row), args, ty: Ty::Map(_, e), .. } if row.id == "core.map_new" && args.is_empty() && *e == Ty::Unknown => Expr::MapLit(Vec::new(), Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str))),
                        other => other,
                    };
                }
                let Some(item) = k.and_then(listed_item) else { return x };
                match x {
                    Expr::ListLit(items, Ty::List(e)) if items.is_empty() && *e == Ty::Unknown => {
                        let list = Expr::ListLit(items, Ty::List(Box::new(item)));
                        if k == Some(Kind::Tensor) {
                            Expr::Convert { how: Convert::ToTensor, expr: Box::new(list), ty: Ty::vector(Ty::Float) }
                        } else {
                            list
                        }
                    }
                    other => other,
                }
            })
            .collect()
    }

    /// A core builtin's row's arguments, as a module row's are given theirs:
    /// an empty list literal is typed by the parameter it is handed to, and
    /// numbers held as a list handed to a tensor's parameter are converted
    /// (`push[m, range[3]]`). An int kept in or compared with a collection
    /// of floats is a float, as one assigned to a float variable is
    /// (compiled, `push[xs, 2]` onto floats did not build).
    fn core_row_arguments(&self, name: &str, lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::Kind;
        let args: Vec<crate::table::Arg> = lowered.iter().map(|x| crate::table::Arg::of_type(&read(x))).collect();
        let Some(kinds) = crate::table::core_row(name, &args).and_then(|row| crate::table::kinds_of(row.params, lowered.len())) else {
            return lowered;
        };
        let mut lowered = Self::typed_empties(&kinds, lowered);
        // the item a list is pushed, and the value a map is set, in the form
        // the collection holds its items (`held_as`)
        // (`x in xs`: the item asked about is read as the list's items are —
        // an int in a list of floats a float; compiled, it had not built)
        let (slot, item) = match (name, lowered.len()) {
            ("push", 2) | ("contains", 2) => (1, match lowered[0].ty().payload() {
                Ty::List(item) => Some(item.as_ref().clone()),
                _ => None,
            }),
            ("set", 3) => (2, match lowered[0].ty().payload() {
                Ty::Map(_, item) => Some(item.as_ref().clone()),
                _ => None,
            }),
            _ => (0, None),
        };
        // (`[]` pushed or set as an item is a list of the items' kind where
        // the collection says it, or of ints where nothing does: nested in
        // a collection made here, its type had been nothing known, and the
        // compiled program had not built — crow check --self)
        // (into a collection made here: a variable's — a graph's arcs, each
        // `[]` until an arc is pushed — is typed by its later pushes)
        if matches!(name, "push" | "set") && lowered.len() == slot + 1 && slot > 0 {
            let empty = matches!(&lowered[slot], Expr::ListLit(items, Ty::List(e)) if items.is_empty() && **e == Ty::Unknown);
            // (of text where nothing says, into a collection made here: a
            // list of items nothing typed is held as one of text, and a
            // parameter given it is too; one whose items are lists of a
            // kind says it, however it was made)
            let as_item = match &item {
                Some(t @ Ty::List(e)) if **e != Ty::Unknown => Some(t.clone()),
                _ if made_here(&lowered[0]) => Some(Ty::List(Box::new(Ty::Str))),
                _ => None,
            };
            if let (true, Some(as_item)) = (empty, as_item) {
                lowered[slot] = Expr::ListLit(Vec::new(), as_item);
            }
        }
        let item = match (&item, name) {
            // (and the collection holds what it was handed, once typed)
            (Some(Ty::Unknown), "push" | "set") | (None, "push" | "set") => lowered.get(slot).map(|x| x.ty().payload().clone()),
            _ => item,
        };
        if let Some(item) = item {
            let value = lowered.remove(slot);
            let value = held_as(value, &item);
            // (an int asked about in a list of floats is the float it is,
            // through the mark of one that can fail)
            let value = match (name, item.payload(), value.ty().payload()) {
                ("contains", Ty::Float, Ty::Int) => {
                    let ty = if value.ty().is_result() { Ty::Result(Box::new(Ty::Float)) } else { Ty::Float };
                    Expr::Convert { how: Convert::IntToFloat, expr: Box::new(value), ty }
                }
                _ => value,
            };
            lowered.insert(slot, value);
        }
        let floats = kinds.iter().zip(&lowered).any(|(k, x)| match (k, x.ty().payload()) {
            (Kind::List | Kind::Seq, Ty::List(e)) | (Kind::MapOf | Kind::MapRef, Ty::Map(_, e)) => **e == Ty::Float,
            _ => false,
        });
        lowered
            .into_iter()
            .zip(kinds)
            .map(|(x, k)| match (k, x.ty()) {
                // (a mask too: where numbers are asked it is numbers, true 1
                // and false 0 — `sort[mask]` had been an internal error)
                (Kind::Tensor, Ty::List(e)) if matches!(e.payload(), Ty::Int | Ty::Float | Ty::Bool) => Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: Ty::vector(Ty::Float) },
                // a list of rows is a matrix where a tensor is taken (spec,
                // Arrays: a list of rows and a matrix hold numbers alike) —
                // rows that are tensors, or lists of numbers
                (Kind::Tensor, Ty::List(e)) if matches!(e.payload(), Ty::Tensor(..)) || matches!(e.as_ref(), Ty::List(i) if matches!(i.as_ref(), Ty::Int | Ty::Float | Ty::Big)) || x.ty().truths_depth() >= 2 => {
                    let ty = Ty::Tensor(Box::new(Ty::Float), shapes::stacked_dims(&x.ty()));
                    Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                }
                // (a plain int: one that can fail is handed on as its error,
                // through the mark the backends read)
                (Kind::Item | Kind::ItemRef, Ty::Int) if floats && !matches!(x, Expr::Convert { how: Convert::Unwrap, .. }) => {
                    Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty: Ty::Float }
                }
                // ints held as a tensor (`[1, 2]`) written as a column's
                // values are the column's ints, as a module row's cells take
                // them (`row_arguments`): the column write is a core row, and
                // the literal's int columns had become floats through it
                (Kind::Cells, t) if matches!(t.payload(), Ty::Tensor(e, dims) if **e == Ty::Int && dims.as_ref().is_none_or(|d| d.len() == 1)) => {
                    let ty = match x.ty() {
                        Ty::Result(_) => Ty::Result(Box::new(Ty::List(Box::new(Ty::Int)))),
                        _ => Ty::List(Box::new(Ty::Int)),
                    };
                    Expr::Convert { how: Convert::ToList, expr: Box::new(x), ty }
                }
                // a whole number of any size where the row takes one
                // (`decimal[12]`, `fraction[1, 3]`): the int as it grows
                (Kind::Big, t) if matches!(t.payload(), Ty::Int) => {
                    let ty = if t.is_result() { Ty::Result(Box::new(Ty::Big)) } else { Ty::Big };
                    Expr::Convert { how: Convert::ToBig, expr: Box::new(x), ty }
                }
                _ => x,
            })
            .collect()
    }

    /// A module function's call that no row of its name takes, by count or
    /// by kind, is refused before either mode runs (the interpreter stopped
    /// on the value, the compiled program did not build: `mean['a']`); a
    /// name the core answers too takes what the core's rows take as well.
    fn refuse_module_call(&mut self, m: &str, name: &str, lowered: &[Expr]) {
        let core_too = CORE_RESERVED.contains(&name);
        // (a module whose rows lack the name has a form of its own for it —
        // `ui.input` is the element, not the core's input)
        if core_too && !crate::table::has_rows(m, name) {
            return;
        }
        if !core_too {
            if let Some(e) = crate::table::miscounted(m, name, lowered.len()) {
                self.refuse(e);
                return;
            }
        }
        // (a name the core answers by a form of its own — `sqrt` of a number,
        // which grad differentiates — takes what that form takes)
        if core_too && !crate::table::has_core_rows(name) {
            return;
        }
        let types: Vec<Ty> = lowered.iter().map(read).collect();
        let kinds: Vec<crate::table::Arg> = types.iter().map(crate::table::Arg::of_type).collect();
        let modules: Vec<&str> = if core_too { vec!["", m] } else { vec![m] };
        if let Some(e) = crate::table::refusal_typed(&modules, name, &kinds, &types) {
            self.refuse(e);
        }
    }

    fn row_arguments(&self, module: &str, name: &str, lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::{self, Kind};
        let core = CORE_RESERVED.contains(&name);
        let args: Vec<crate::table::Arg> = lowered.iter().map(|x| crate::table::Arg::of_type(&read(x))).collect();
        // (the row the call is answered by: an exact form takes a list of
        // ints as it is)
        let Some(kinds) = table::module(module, name, &args).and_then(|row| table::kinds_of(table::exact_form(row, &args).params, lowered.len())) else {
            return lowered;
        };
        let lowered = Self::typed_empties(&kinds, lowered);
        lowered
            .into_iter()
            .zip(kinds)
            .map(|(x, k)| {
                let numbers = match x.ty() {
                    // (a list that can fail, marked: converted over the mark,
                    // which both backends read through — the call hands its
                    // failure on, and a list of ints that could fail had
                    // reached a tensor's parameter as it was)
                    // (a column that can fail too — `sum[where[m, c, 0.0]]`
                    // had handed the marked column to math's sum as it was,
                    // and the verifier refused it)
                    _ if matches!(&x, Expr::Convert { how: Convert::Unwrap, expr, .. } if !matches!(expr.ty().payload(), Ty::List(_) | Ty::Column(_))) => false,
                    // numbers that can fail are numbers too, a failed one
                    // stopping where they are read (as a column's missing
                    // cell does)
                    // (a list of numbers handed to a core name's module row
                    // — `max` of a list a function computed, math's — is
                    // converted as the row takes it)
                    Ty::List(e) => (!core || k == Kind::Tensor) && *e != Ty::Unknown && (!e.is_result() || matches!(e.payload(), Ty::Int | Ty::Float)),
                    Ty::Column(_) => true,
                    _ => false,
                };
                if k == Kind::Tensor && numbers {
                    let ty = Ty::Tensor(Box::new(Ty::Float), shapes::stacked_dims(&x.ty()));
                    Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty }
                } else if k == Kind::Cells && matches!(x.ty().payload(), Ty::Tensor(e, dims) if **e == Ty::Int && dims.as_ref().map_or(true, |d| d.len() == 1)) {
                    // ints held as a tensor (`[1, 2]`) handed as a column's
                    // values are the column's ints (a tensor's numbers had
                    // made a column of floats)
                    let ty = match x.ty() {
                        Ty::Result(_) => Ty::Result(Box::new(Ty::List(Box::new(Ty::Int)))),
                        _ => Ty::List(Box::new(Ty::Int)),
                    };
                    Expr::Convert { how: Convert::ToList, expr: Box::new(x), ty }
                } else {
                    x
                }
            })
            .collect()
    }

    /// A call's type by inference's rules, from its arguments' types (each
    /// as lowered, before a failure among them is unwrapped) and, when the
    /// callee was lowered as a value, the callee's. The rule is picked by the
    /// callee's form, as the walk picks it.
    fn call_type(&self, callee: &A, args: &[A], arg_types: &[Ty], full_types: &[Ty], callee_type: Option<Ty>) -> Ty {
        let t = match callee {
            A::Ident(name) => self.ctx.named_call_type(name, args, callee, arg_types, full_types),
            _ => {
                let module = match callee {
                    A::FieldAccess { object, field } => match object.as_ref() {
                        A::Ident(m) => self.ctx.module_call_type(m, field, arg_types),
                        _ => None,
                    },
                    _ => None,
                };
                match module {
                    Some(t) => t,
                    None => {
                        // indexing, or a function value applied: the callee
                        // as lowered, or a module's value it names (m.f[x])
                        let callee_type = match (callee_type, callee) {
                            (Some(t), _) => t,
                            (None, A::FieldAccess { object, field }) => self.qualified_type(object, field),
                            (None, _) => Ty::Unknown,
                        };
                        self.ctx.index_type(callee_type, arg_types.first().cloned())
                    }
                }
            }
        };
        t.clone()
    }

    /// A call's own checks, made after its arguments': a name called is
    /// checked as a call by name, `m.f[…]` as a module's.
    fn check_callee(&mut self, callee: &A, args: &[A], types: &[Ty]) {
        match callee {
            A::Ident(n) => self.check_call(n, args, types),
            A::FieldAccess { object, field } => {
                if let A::Ident(m) = object.as_ref() {
                    self.check_module_call(m, field);
                }
            }
            _ => {}
        }
    }

    /// The type of a value a module names, `m.x`.
    fn qualified_type(&self, object: &A, field: &str) -> Ty {
        let object_type = match object {
            A::Ident(m) => self.ctx.ident_type(m).text(),
            _ => Ty::Unknown,
        };
        self.ctx.field_type(object, object_type, field)
    }

    /// A user function called with arguments, as a call held for later,
    /// when the expression is one.
    fn deferred(&mut self, e: &A) -> Result<Option<Expr>, String> {
        if let A::Call { callee, args } = e {
            if let A::Ident(f) = callee.as_ref() {
                if self.ctx.functions.contains_key(f) && !self.declared(f) {
                    let mut lowered = Vec::with_capacity(args.len());
                    for a in args {
                        lowered.push(self.lower_expr(a)?);
                    }
                    let checks: Vec<Ty> = lowered.iter().map(|x| self.checked(x)).collect();
                    self.check_call(f, args, &checks);
                    let types: Vec<Ty> = lowered.iter().map(full).collect();
                    self.scan_call(callee, args, &types);
                    return Ok(Some(Expr::Deferred { name: f.clone(), args: lowered }));
                }
            }
        }
        Ok(None)
    }

    /// The type of the call a deferred call holds (`h[x]` in `click[a,
    /// h[x]]`), from its lowered arguments, markup told apart.
    fn held_type(&self, a: &A, d: &Expr) -> Ty {
        match (a, d) {
            (A::Call { callee, args }, Expr::Deferred { args: lowered, .. }) => {
                let arg_types: Vec<Ty> = lowered.iter().map(read).collect();
                let full_types: Vec<Ty> = lowered.iter().map(full).collect();
                match self.call_type(callee, args, &arg_types, &full_types, None) {
                    Ty::Node => Ty::Markup,
                    t => as_inferred(&t),
                }
            }
            _ => Ty::Unknown,
        }
    }

    /// The specialised name of a user function called at these argument
    /// types, when it has one.
    /// The function a gradient being called is of: `grad[f][…]`, or a
    /// local holding one (`g = grad[f]`, then `g[…]`).
    /// A function handed as a value to a function of the program's is
    /// typed by what the parameter it is handed to is applied with; one
    /// nothing types cannot be built (the compiled program had left it
    /// out, and the call of it did not build), so it is refused with the
    /// fix. A function handed to a builtin — a page's view, a route's
    /// handler, `grad`'s function — is the runtime's to type.
    /// (wherever it is handed: to a function, compared, held in a list, put
    /// into text — each had been an internal error, crow check --self; a
    /// view, a handler or a helper is typed by the runtime that calls it,
    /// a test's by the interpreter that runs it, and `grad`'s function by
    /// the run that traces it, as the verifier reads them)
    fn check_fn_value(&mut self, f: &str) {
        if self.pass != Pass::Final || self.at == Where::Test || self.differentiating {
            return;
        }
        // (inside a standard module's function the program does not call —
        // its own parameters typed by nothing — a function it hands on is
        // typed by nothing either, and neither is compiled: a program
        // calling magpie's `backup` alone had been refused for the
        // sequencer's `put_rows`, which hands `revise` its `_appended`)
        if crate::modules::is_crow_fn(&self.current_fn) && self.ctx.functions.get(&self.current_fn).is_some_and(|sig| sig.params.iter().any(|(_, t)| *t == Ty::Unknown)) {
            return;
        }
        // a function copied per signature has no one type to be held at
        // (spec, Specialization): refused where it is handed — the
        // compiled program had refused it as it built — unless a row calls
        // it with arguments of its own, which choose the copy
        // (`row_copies`, after the row's arguments are lowered)
        if self.ctx.specializations.get(f).is_some_and(|v| !v.is_empty()) && self.handed_to_row.as_deref() != Some(f) {
            self.refuse(format!(
                "'{}' is used at several kinds, and is handed as a value here — a function value has one type: hand a function of one kind (fn {}_text[x], calling {}[x])",
                crate::lower::shown(f),
                crate::lower::shown(f),
                crate::lower::shown(f)
            ));
            return;
        }
        let Some(sig) = self.ctx.functions.get(f) else { return };
        if !sig.params.iter().any(|(_, t)| *t == Ty::Unknown) {
            return;
        }
        let body = sig.body.clone();
        if matches!(self.kind_of(f, &body, false), FnKind::View | FnKind::Handler | FnKind::Helper) {
            return;
        }
        self.refuse(format!(
            "`{}` is handed as a value, and nothing types its parameters — apply it in the function it is handed to, or call it directly once",
            crate::lower::shown(f)
        ));
    }

    /// A list or a map of ints handed a float holds floats from then on, as
    /// a variable that is ever assigned a float is a float (spec §1): the
    /// holder is converted where it meets the float, and the answer holds
    /// floats. (The interpreter had kept a mixed list — `push[[1, 2], 2.5]`
    /// was `[1, 2, 2.5]`, ints among floats — and the compiled program had
    /// not built.)
    fn widen_items(row: &'static crate::table::Row, mut args: Vec<Expr>, ty: Ty) -> (Vec<Expr>, Ty) {
        fn ints(t: &Ty) -> bool {
            matches!(t.payload(), Ty::List(e) | Ty::Map(_, e) | Ty::Tensor(e, _) if **e == Ty::Int)
        }
        // (whole numbers written down and pushed as a list's row are a list
        // of ints, as whole numbers written down and grown are: held as a
        // vector, the first row had made the list's rows vectors, and every
        // later row of ints was held as floats — a silent wrong answer past
        // 2^53, past 64 bits a stop or a clamp; Rosetta Code, Lah numbers)
        let mut ty = ty;
        if row.id == "core.push"
            && args.len() == 2
            && matches!(args[0].ty().payload(), Ty::List(i) if matches!(i.as_ref(), Ty::Unknown) || matches!(i.as_ref(), Ty::List(e) if matches!(e.as_ref(), Ty::Int | Ty::Big)))
            && let Expr::ListLit(items, Ty::Tensor(e, d)) = &args[1]
            && **e == Ty::Int
            && d.as_ref().is_none_or(|d| d.len() <= 1)
        {
            args[1] = Expr::ListLit(items.clone(), Ty::List(Box::new(Ty::Int)));
            let rows_of = |t: &Ty| match t {
                Ty::List(i) if matches!(i.as_ref(), Ty::Tensor(..)) => Ty::List(Box::new(Ty::List(Box::new(Ty::Int)))),
                other => other.clone(),
            };
            ty = match &ty {
                Ty::Result(inner) => Ty::Result(Box::new(rows_of(inner))),
                other => rows_of(other),
            };
        }
        fn floats(t: &Ty) -> bool {
            matches!(t.payload(), Ty::List(e) | Ty::Map(_, e) | Ty::Tensor(e, _) if **e == Ty::Float)
        }
        fn of_floats(t: Ty) -> Ty {
            match t {
                Ty::List(e) if *e == Ty::Int => Ty::List(Box::new(Ty::Float)),
                Ty::Map(k, e) if *e == Ty::Int => Ty::Map(k, Box::new(Ty::Float)),
                // (numbers held as a tensor: its numbers are floats already;
                // what changes is what the type says of them)
                Ty::Tensor(e, d) if *e == Ty::Int => Ty::Tensor(Box::new(Ty::Float), d),
                Ty::Result(inner) => Ty::Result(Box::new(of_floats(*inner))),
                other => other,
            }
        }
        fn widen(args: &mut [Expr], at: usize) {
            let x = std::mem::replace(&mut args[at], Expr::Const(Const::Int(0), Ty::Int));
            let ty = of_floats(x.ty());
            args[at] = Expr::Convert { how: Convert::IntsToFloats, expr: Box::new(x), ty };
        }
        let item = match row.id {
            "core.push" | "core.push_number" | "core.contains" => 1,
            "core.set" => 2,
            "core.concat" if args.len() == 2 => {
                if ints(&args[0].ty()) && floats(&args[1].ty()) {
                    widen(&mut args, 0);
                    return (args, of_floats(ty));
                }
                if floats(&args[0].ty()) && ints(&args[1].ty()) {
                    widen(&mut args, 1);
                    return (args, of_floats(ty));
                }
                return (args, ty);
            }
            _ => return (args, ty),
        };
        if args.len() > item && ints(&args[0].ty()) && matches!(args[item].ty().payload(), Ty::Float) {
            widen(&mut args, 0);
            return (args, of_floats(ty));
        }
        // (a holder of ints given to a holder of holders of floats — a
        // grid's row of ints pushed into a grid one of whose numbers was
        // written a float — holds floats as the grid's rows do: the
        // compiled program had pushed a Vec<i64> into a Vec<Vec<f64>>;
        // Rosetta Code, Abelian sandpile)
        let held = match args.first().map(|a| a.ty()) {
            Some(Ty::List(e) | Ty::Map(_, e)) => Some(*e),
            Some(Ty::Result(inner)) => match *inner {
                Ty::List(e) | Ty::Map(_, e) => Some(*e),
                _ => None,
            },
            _ => None,
        };
        if args.len() > item
            && let Some(held) = held
            && crate::ir::floats_of_ints(&held, args[item].ty().payload(), false)
        {
            let x = std::mem::replace(&mut args[item], Expr::Const(Const::Int(0), Ty::Int));
            let ty = if x.ty().is_result() { Ty::Result(Box::new(held)) } else { held };
            args[item] = Expr::Convert { how: Convert::IntsToFloats, expr: Box::new(x), ty };
        }
        (args, ty)
    }

        /// `len` of a `sub` of text, as the call that counts the length the
    /// substitution makes without making it (`text.__sub_len`): the same
    /// arguments, and a failure where `sub`'s would be.
    fn sub_len_of(arg: &Expr) -> Option<Expr> {
        let inner = match arg {
            Expr::Convert { how: Convert::Unwrap, expr, .. } => expr.as_ref(),
            other => other,
        };
        let Expr::Call { target: Target::Row(row), args, propagates, ty } = inner else { return None };
        if row.id != "text.sub" || args.len() != 3 {
            return None;
        }
        let fused = crate::table::ROWS.iter().find(|r| r.id == "text.__sub_len")?;
        let ty = if ty.is_result() { Ty::Result(Box::new(Ty::Int)) } else { Ty::Int };
        Some(Expr::Call { target: Target::Row(fused), args: args.clone(), ty, propagates: *propagates })
    }

    /// The kind of a collection's items, as a value handed to it is read: a
    /// list's or a tensor's items, a map's values — or its keys, for `k in
    /// m` — when they are numbers, text or bools.
    fn items_kind(t: &Ty, keys: bool) -> Option<Ty> {
        let kind = match t.payload() {
            Ty::List(e) | Ty::Tensor(e, _) => (**e).clone(),
            Ty::Map(Key::Text, _) if keys => Ty::Str,
            Ty::Map(Key::Int, _) if keys => Ty::Int,
            Ty::Map(_, v) if !keys => (**v).clone(),
            _ => return None,
        };
        matches!(kind, Ty::Int | Ty::Float | Ty::Str | Ty::Bool).then_some(kind)
    }

    /// The pieces a value indexed holds: a decomposition called here, or a
    /// local one was assigned to.
    fn pieces_of(&self, base: &Expr) -> Option<Vec<Option<Dims>>> {
        match base {
            Expr::Local(v, _) | Expr::Move(v, _) => self.pieces.get(v).cloned(),
            Expr::Convert { expr, .. } => self.pieces_of(expr),
            e => pieces_of_call(e),
        }
    }

    fn gradient_of(&self, base: &Expr) -> Option<String> {
        match base {
            Expr::Local(g, _) | Expr::Move(g, _) => self.grad_of.get(g).cloned(),
            other => gradient_of_call(other),
        }
    }

    fn specialised(&self, name: &str, full_types: &[Ty]) -> String {
        self.ctx.specialization_for(name, full_types).unwrap_or_else(|| name.to_string())
    }

    /// The copy of `f` a row reaches when it calls `f` with what it is
    /// handed (the row's function signature `sig`, at argument `at`): the
    /// arguments as inference hands them (`hand_to_row`) — a walked item,
    /// the record held, each declared kind, then the row's remaining
    /// arguments where the signature takes them — matched exactly, never
    /// the first copy for want of one.
    fn row_copy(&self, row: &str, f: &str, at: usize, sig: &crate::table::Sig, full_types: &[Ty]) -> Option<String> {
        let argc = full_types.len();
        let walked = crate::table::walked_param(row, argc).and_then(|w| full_types.get(w)).map(|t| t.payload().clone());
        let record = crate::table::record_param(row, argc).and_then(|w| full_types.get(w)).map(|t| t.payload().clone());
        let mut given: Vec<Ty> = sig
            .params
            .iter()
            .map(|k| match k {
                crate::table::Kind::Item => match &walked {
                    Some(Ty::List(inner)) => inner.as_ref().clone(),
                    Some(Ty::Tensor(inner, dims)) => crate::type_infer::TypeContext::tensor_item(inner, dims),
                    Some(Ty::Column(cell)) => cell.as_ref().clone(),
                    _ => Ty::Unknown,
                },
                crate::table::Kind::Record => record.clone().unwrap_or(Ty::Unknown),
                k => k.ty(),
            })
            .collect();
        if sig.rest {
            given.extend(full_types.get(at + 1..).unwrap_or(&[]).iter().map(|t| match t {
                Ty::Result(inner) => inner.as_ref().clone(),
                other => other.clone(),
            }));
        }
        let call_sig: Vec<Ty> = given
            .into_iter()
            .map(|t| match t {
                Ty::OptRecord(n) => Ty::Record(n),
                other => other,
            })
            .collect();
        self.ctx.specializations.get(f)?.iter().find(|(csig, _)| crate::type_infer::TypeContext::same_signature(csig, &call_sig)).map(|(_, copy)| copy.clone())
    }

    /// An interpolated string's pieces, each expression parsed as the
    /// interpreter parses it.
    fn interpolation(&mut self, s: &str) -> Result<Vec<Piece>, String> {
        let chars: Vec<char> = s.chars().collect();
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == crate::lexer::BACKSLASH_MARK {
                text.push('\\');
                i += 2;
                continue;
            }
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                text.push('`');
                i += 2;
                continue;
            }
            if chars[i] == '`' {
                let start = i + 1;
                i += 1;
                let mut in_str = false;
                while i < chars.len() {
                    let c = chars[i];
                    if in_str {
                        if c == '\\' && i + 1 < chars.len() {
                            i += 2;
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
                    i += 1;
                }
                // (the interpolation is read on its own, so what its reading
                // refuses is placed at the string's line, not at the
                // fragment's first: a bracket closing nothing in a string on
                // line 20 had been shown on line 1; Rosetta Code, Unicode
                // strings)
                let above = chars[..start].iter().filter(|c| **c == '\n').count();
                let line = self.v.line;
                let placed = |e: String| in_string_at(e, line, above);
                if i >= chars.len() {
                    return Err(placed("unterminated interpolation — close it with ` or write \\` for a literal backtick".to_string()));
                }
                let src: String = chars[start..i].iter().collect();
                i += 1;
                if src.trim().is_empty() {
                    return Err(placed("empty interpolation — backticks must contain an expression".to_string()));
                }
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                let mut lexer = crate::lexer::Lexer::new(&src);
                let tokens = lexer.tokenize().map_err(placed)?;
                let mut parser = crate::parser::Parser::for_fragment(tokens);
                let stmts = parser.parse().map_err(placed)?;
                let stmts = crate::lower::lower_fragment(&stmts).unwrap_or(stmts);
                let expr = stmts.iter().find_map(|st| if let S::ExprStmt(e) = st { Some(e.clone()) } else { None });
                match expr {
                    Some(e) => pieces.push(Piece::Expr(self.lower_expr(&e)?)),
                    None => return Err(placed(format!("interpolation is not an expression: `{}`", src))),
                }
            } else {
                text.push(chars[i]);
                i += 1;
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        Ok(pieces)
    }
}

/// A refusal of an interpolation's text, read on its own (its lines count
/// from 1), placed at the line of the string that holds it: the statement's
/// line, the string's line breaks before the backtick, and the fragment's
/// own. The fragment's column is not the source's, so the message keeps
/// what it quotes to point with.
fn in_string_at(e: String, line: usize, above: usize) -> String {
    if line == 0 {
        return e;
    }
    let (inner, message) = match e.strip_prefix("line ") {
        Some(after) => {
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            let tail = &after[digits.len()..];
            let tail = match tail.strip_prefix(", col ") {
                Some(c) => &c[c.chars().take_while(|c| c.is_ascii_digit()).count()..],
                None => tail,
            };
            match (digits.parse::<usize>(), tail.strip_prefix(": ")) {
                (Ok(n), Some(m)) => (n.saturating_sub(1), m.to_string()),
                _ => (0, e.clone()),
            }
        }
        None => (0, e.clone()),
    };
    // (the fragment's end is the interpolation's, not the file's)
    let message = message.replace("the end of the file", "the end of the interpolation");
    format!("line {}: {}", line + above + inner, message)
}

// ── inference by lowering (F5) ───────────────────────────────────────

/// Signatures found by lowering bodies (F5, core-rewrite-notes §10):
/// inference's rounds, where each round's call scan lowers every body in
/// order, each call noting its arguments' types for its callee, and each
/// function's locals are what a provisional lowering of its body leaves.
/// A body's return type and whether an error leaves it are still decided
/// from that environment (`finish_body`).
pub(crate) fn infer_by_lowering(ctx: &mut TypeContext, stmts: &[S]) {
    // what the program reads of the frames it cannot see (uses.rs), read
    // off the declarations before anything is typed, so a cell's reader
    // takes the cell's kind and not a parsed value (a parameter once typed
    // as one stays one); then a second look with the top level typed, for
    // a cell met beside a typed variable
    ctx.begin_inference(stmts);
    let mut applied = apply_shapes(ctx, stmts);
    infer_once(ctx, stmts);
    if apply_shapes(ctx, stmts) > applied {
        applied += 1;
        infer_once(ctx, stmts);
    }
    let _ = applied;
}

/// The shapes the program's reads say (uses.rs), kept in the context for
/// the lowering to hold each loaded frame to; how many columns are known.
/// A page's data source: `fetch[url]` or `poll[url, seconds]`, as the
/// emitter tells them too.
fn is_source_call(e: &Expr) -> bool {
    crate::ir::call_name(e).is_some_and(|n| crate::table::source(&n, crate::ir::call_args(e).len()))
}

fn apply_shapes(ctx: &mut TypeContext, stmts: &[S]) -> usize {
    let shapes = uses::shapes_from_use(stmts, ctx);
    ctx.shape_conflicts = shapes.conflicts;
    for (key, cols) in shapes.frames {
        // a file's cell can be empty and a record's field null: a column the
        // reads type holds cells that can be missing, each an error value of
        // reason `missing` read one at a time (spec, Frames) — typed plain,
        // a missing cell had been read as '' or NaN with `.ok` true
        let cols: BTreeMap<String, Ty> = cols.into_iter().map(|(c, t)| (c, Ty::Result(Box::new(t)))).collect();
        let known: BTreeMap<String, Ty> = match ctx.frame_shapes.get(&key) {
            Some(had) => had.cols.iter().map(|(k, v)| (k.clone(), v.clone())).chain(cols).collect(),
            None => cols,
        };
        ctx.frame_shapes.insert(key, std::sync::Arc::new(crate::type_infer::Cols { cols: known, open: true }));
    }
    for (key, reasons) in shapes.reasons {
        ctx.frame_reasons.entry(key).or_default().extend(reasons);
    }
    ctx.frame_shapes.values().map(|c| c.cols.len()).sum()
}

fn infer_once(ctx: &mut TypeContext, stmts: &[S]) {
    let in_ui = stmts.iter().any(|s| matches!(s, S::Use { path, .. } if path == "ui" || path == "web.ui"));
    ctx.begin_inference(stmts);
    // five rounds at least, as ever; then more while a signature still
    // moves — a chain of derivations learns one link a round
    for round in 0..16 {
        let before = ctx.signatures();
        ctx.sync_derive_types();
        Lowerer::for_inference(ctx, in_ui, stmts).scan_program(stmts);
        // types from indirect calls back to the functions named
        ctx.propagate_indirect_fn_types(stmts);
        infer_bodies(ctx, in_ui, stmts);
        if round >= 4 && before == ctx.signatures() {
            break;
        }
    }
    // what the last round learned, where the top level reads it
    ctx.sync_derive_types();
    // what only a test's clicks and calls type, last, and then the program's
    // calls again: a body those types reach calls other functions with
    // them (a helper only tests call hands another its float). The tests
    // are read again once those rounds settle, since what they hand a
    // function may come from another that only now has: a frame another
    // function builds, whose columns were not worked out the first time
    for _ in 0..4 {
        // (a test's call hands functions as values too, which moves their
        // signatures without a parameter of the callee's moving)
        let handed = ctx.signatures();
        if !Lowerer::for_inference(ctx, in_ui, stmts).type_clicks(stmts) && handed == ctx.signatures() {
            break;
        }
        for _ in 0..8 {
            let before = ctx.signatures();
            ctx.sync_derive_types();
            Lowerer::for_inference(ctx, in_ui, stmts).scan_program(stmts);
            ctx.propagate_indirect_fn_types(stmts);
            infer_bodies(ctx, in_ui, stmts);
            if before == ctx.signatures() {
                break;
            }
        }
    }
    // F2: a function called at several signatures is one per signature;
    // then the calls are read again, since what a copy answers can type
    // the function it is handed to (`two[may[1, …], 2]` beside `may[f.a,
    // …]`: `two`'s parameter had kept the first rounds' guess, the other
    // copy's column, and the compiled program did not build), and a
    // signature that only now shows is copied too
    for _ in 0..8 {
        Lowerer::for_inference(ctx, in_ui, stmts).specialize();
        if ctx.clone_names().is_empty() {
            break;
        }
        let before = ctx.signatures();
        for _ in 0..8 {
            let round = ctx.signatures();
            ctx.sync_derive_types();
            Lowerer::for_inference(ctx, in_ui, stmts).scan_program(stmts);
            ctx.propagate_indirect_fn_types(stmts);
            infer_bodies(ctx, in_ui, stmts);
            if round == ctx.signatures() {
                break;
            }
        }
        // (and while a function waits for its copies: a signature that
        // shows only once another function's copies answer moves no
        // function's types, and stopping there had left the function
        // uncopied, each call of it answering nothing known — magpie's
        // `group_bytes`, called by the manifest's writer once `columns_of`
        // was copied, read as a field of a value that can fail compiled;
        // parity 312)
        if before == ctx.signatures() && !ctx.awaiting_clones() {
            break;
        }
    }
    // a parameter every call hands a value of no known kind is text, as it
    // is compiled (`default_unknown_params`), and the bodies are read again
    // with it so — a value walked out of it reaches the functions it is
    // handed to as text too, which the next look types in turn
    for _ in 0..4 {
        if !ctx.default_unknown_params() {
            break;
        }
        for _ in 0..8 {
            let round = ctx.signatures();
            ctx.sync_derive_types();
            Lowerer::for_inference(ctx, in_ui, stmts).scan_program(stmts);
            ctx.propagate_indirect_fn_types(stmts);
            infer_bodies(ctx, in_ui, stmts);
            if round == ctx.signatures() {
                break;
            }
        }
    }
    ctx.sync_derive_types();
    Lowerer::for_inference(ctx, in_ui, stmts).settle_top(stmts);
}

/// Every function's body typed, in name order (`Lowerer::infer_body`).
fn infer_bodies(ctx: &mut TypeContext, in_ui: bool, stmts: &[S]) {
    let fns: Vec<(String, Vec<S>)> = ctx.functions.iter().map(|(n, sig)| (n.clone(), sig.body.clone())).collect();
    let mut l = Lowerer::for_inference(ctx, in_ui, stmts);
    for (name, body) in &fns {
        l.infer_body(name, body);
    }
}

// ── analyses over the parser's tree ─────────────────────────────────

/// The word before each `?` of a statement, outside quotes, an opening
/// parenthesis passed over (`ANY(?)` is `ANY`): where a list parameter
/// may go (only after `IN`).
pub(crate) fn words_before_placeholders(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '?' => {
                let mut j = i;
                while j > 0 && (chars[j - 1].is_whitespace() || chars[j - 1] == '(') {
                    j -= 1;
                }
                let end = j;
                while j > 0 && (chars[j - 1].is_ascii_alphanumeric() || chars[j - 1] == '_') {
                    j -= 1;
                }
                out.push(chars[j..end].iter().collect());
            }
            None => {}
        }
    }
    out
}

/// The type a local is declared with, from its first value's type and the
/// type inference settled on across its assignments: the first value's (a
/// node's value is its markup, an int given to a float variable is a
/// float, spec §1), unless it is not known yet, when the settled type is
/// taken — its payload when the first value cannot fail, which makes a
/// variable that holds no error (spec, Reassignment), as the Rust backend
/// declares it (`xs = []`, then a push of a value that can fail, refused
/// where it is assigned).
/// The variable an expression reads, through the conversions a read is
/// wrapped in (a value that can fail is decided before it is indexed).
/// Whether a value was read out of a tensor whose rank is not known: its
/// kind, a number, is the lowering's guess, not the program's (an index
/// into it is the rank's refusal, not a number's).
fn of_unknown_rank(e: &Expr) -> bool {
    match e {
        Expr::Index { base, .. } => matches!(base.ty().payload(), Ty::Tensor(_, None)) || of_unknown_rank(base),
        _ => false,
    }
}

/// The expressions a block hands its value on through: its last statement,
/// and the last of each branch of an `if` that ends it — what a collecting
/// loop yields, what a console shows.
fn yields_of(body: &[S], out: &mut HashSet<usize>) {
    match body.iter().rev().find(|s| !matches!(s, S::Line(_))) {
        Some(S::ExprStmt(e)) => {
            out.insert(e as *const A as usize);
        }
        Some(S::If { then_body, else_body, .. }) => {
            yields_of(then_body, out);
            if let Some(b) = else_body {
                yields_of(b, out);
            }
        }
        _ => {}
    }
}

impl Lowerer<'_> {
    /// A value read from a variable given two kinds, whose reassignment was
    /// refused: its kind here is neither's to judge.
    fn contested(&self, e: &Expr) -> bool {
        local_named(e).is_some_and(|n| self.v.contested.contains(&n))
    }

    /// `contested` of a variable a value is read from, through its fields
    /// and items (`g21.t` of a `g21` whose making was refused).
    pub(super) fn rooted_in_contested(&self, e: &Expr) -> bool {
        match e {
            Expr::Field { base, .. } | Expr::Index { base, .. } => self.rooted_in_contested(base),
            other => self.contested(other),
        }
    }
}

fn local_named(e: &Expr) -> Option<String> {
    match e {
        // (a name the lowering made — `_t1` — is no name the program wrote)
        Expr::Local(v, _) | Expr::Move(v, _) | Expr::Load(v, _) if v.starts_with("_t") && v[2..].bytes().all(|b| b.is_ascii_digit()) => None,
        Expr::Local(v, _) | Expr::Move(v, _) | Expr::Load(v, _) => Some(v.clone()),
        Expr::Convert { expr, .. } => local_named(expr),
        _ => None,
    }
}

/// A collecting loop whose yield says nothing of its items — an empty list
/// each round (`preds = for i in range[n]` yielding `[]`, a graph's arcs)
/// — is typed by the local it is assigned to, as the local settled (an
/// item assigned later typed the items): the loop had kept its unknown
/// items, which the compiled program read as text.
fn settled_yield(expr: Expr, declared: &Ty) -> Expr {
    if !expr.ty().has_unknown() || declared.has_unknown() {
        return expr;
    }
    match expr {
        Expr::ForExpr { var, iter, mut body, ty: _, parallel } if matches!(declared, Ty::List(_)) => {
            if let (Ty::List(item), Some(Stmt::Do(Expr::ListLit(items, lt)))) = (declared, body.iter_mut().rev().find(|s| !matches!(s, Stmt::Line(_)))) {
                if items.is_empty() && lt.has_unknown() {
                    *lt = (**item).clone();
                }
            }
            // (and a local yielded — `none = []` then `none` — is declared
            // as the items are: it had stayed a list of nothing known,
            // compiled as one of text; Rosetta Code, Hopcroft-Karp)
            if let (Ty::List(item), Some(Stmt::Do(Expr::Local(v, t) | Expr::Move(v, t)))) = (declared, body.iter_mut().rev().find(|s| !matches!(s, Stmt::Line(_))))
                && t.has_unknown()
                && t.compatible(item)
            {
                *t = (**item).clone();
                let v = v.clone();
                for s in body.iter_mut() {
                    if let Stmt::Let { var, ty, expr } = s
                        && *var == v
                        && ty.has_unknown()
                    {
                        *ty = (**item).clone();
                        match expr {
                            Expr::ListLit(items, lt) if items.is_empty() => *lt = (**item).clone(),
                            Expr::MapLit(pairs, lt) if pairs.is_empty() => *lt = (**item).clone(),
                            _ => {}
                        }
                    }
                }
            }
            Expr::ForExpr { var, iter, body, ty: declared.clone(), parallel }
        }
        other => other,
    }
}

/// A value in the form a collection holds its items: numbers held as a
/// tensor set into a map of lists, or pushed onto a list of lists, are the
/// list; numbers held as a list, or an empty list, where the items are
/// tensors are the tensor — converted where they meet, as they are where a
/// variable or a record's field takes them (`map[a = [1, 2], c = []]` had
/// stopped the interpreter at the tensor — `expected a list, got array` —
/// and the compiled program had not built).
/// Numbers held as a tensor, as the list of `item` a place holds: whole
/// numbers written down are that list as they are written — never made
/// floats and back (`list([n])` had held 2^53 + 1 as 2^53: Rosetta Code,
/// Lah numbers) — and any other tensor is converted where it is handed.
fn listed(value: Expr, item: Ty) -> Expr {
    match value {
        Expr::ListLit(items, Ty::Tensor(e, d)) if *e == Ty::Int && item == Ty::Int && d.as_ref().is_none_or(|d| d.len() <= 1) => Expr::ListLit(items, Ty::List(Box::new(Ty::Int))),
        other => {
            let ty = Ty::List(Box::new(item));
            let ty = if other.ty().is_result() { Ty::Result(Box::new(ty)) } else { ty };
            Expr::Convert { how: Convert::ToList, expr: Box::new(other), ty }
        }
    }
}

/// A failure made where it is held (`error[…]`, a form answering a
/// failure of no value) is a failure of what the place holds: an item of a
/// list whose items can fail. Typed as no value, it had been an item of
/// another kind than the list's, and the lowered program disagreed with
/// itself (Rosetta Code, Monads/Maybe monad).
fn failure_as(value: Expr, item: &Ty) -> Expr {
    match value {
        Expr::Call { target: Target::Runtime(name), args, ty, propagates } if made_failure_named(&name, &ty) && item.is_result() && !matches!(item.payload(), Ty::Unit | Ty::Unknown) => {
            Expr::Call { target: Target::Runtime(name), args, ty: item.clone(), propagates }
        }
        other => other,
    }
}

/// Whether a call of `name` typed `ty` is a failure made where it is
/// written: a core form answering a failure of no value.
fn made_failure_named(name: &str, ty: &Ty) -> bool {
    matches!(ty, Ty::Result(inner) if matches!(inner.as_ref(), Ty::Unit | Ty::Unknown)) && crate::table::rows_named(name).any(|r| r.path == crate::table::IN_FORM && r.module.is_empty())
}

/// A call handed a failure made in its argument (`encode[error['why']]`)
/// answers that failure (spec, Auto-Propagation): its value of no kind had
/// been handed on compiled — `!` where the row takes a value — and the
/// program did not build, where the interpreter answered the failure.
/// Folded where every other argument is quiet, so nothing beside it is
/// lost.
fn made_failure_answers(x: Expr) -> Expr {
    let (args, ty) = match &x {
        Expr::Call { args, ty, propagates: true, .. } | Expr::Effect { args, ty, propagates: true, .. } => (args, ty),
        _ => return x,
    };
    if matches!(ty.payload(), Ty::Unit) {
        return x;
    }
    let made = |a: &Expr| matches!(a, Expr::Convert { how: Convert::Unwrap, expr, .. } if matches!(expr.as_ref(), Expr::Call { target: Target::Runtime(n), ty, .. } if made_failure_named(n, ty)));
    let Some(i) = args.iter().position(made) else { return x };
    if args.iter().enumerate().any(|(j, a)| j != i && !crate::ir::quiet(a)) {
        return x;
    }
    // (the failure the call answers: of its kind, where the call was typed
    // as never failing — a row that shows any value)
    let ty = Ty::Result(Box::new(ty.payload().clone()));
    match args[i].clone() {
        Expr::Convert { expr, .. } => match *expr {
            Expr::Call { target, args, propagates, .. } => Expr::Call { target, args, ty, propagates },
            other => other,
        },
        other => other,
    }
}

fn held_as(value: Expr, item: &Ty) -> Expr {
    let failing = value.ty().is_result();
    let wrap = |t: Ty| if failing { Ty::Result(Box::new(t)) } else { t };
    match (item.payload(), value.ty().payload().clone()) {
        (Ty::List(e), Ty::Tensor(..)) if matches!(e.payload(), Ty::Int | Ty::Float) => listed(value, e.payload().clone()),
        (Ty::Tensor(..), Ty::List(e)) if matches!(e.payload(), Ty::Int | Ty::Float | Ty::Unknown) => {
            let ty = wrap(Ty::Tensor(Box::new(Ty::Float), shapes::stacked_dims(&value.ty())));
            Expr::Convert { how: Convert::ToTensor, expr: Box::new(value), ty }
        }
        // (a list whose items hold their numbers the other way — lists of
        // numbers where vectors are held, vectors where lists are — each
        // item converted by a collecting loop: a list of lists pushed onto
        // a list holding lists of vectors had been an internal error;
        // Rosetta Code, Numbers with equal rises and falls)
        (Ty::List(want), Ty::List(have)) if !failing && forms_differ(&want, &have) => {
            let var = held_name();
            let one = Expr::Local(var.clone(), (*have).clone());
            let each = match (want.as_ref(), have.as_ref()) {
                // (a list of whole numbers is a vector of whole numbers)
                (Ty::Tensor(_, d), Ty::List(e)) => {
                    let ty = Ty::Tensor(Box::new(e.payload().clone()), d.clone().or(Some(vec![None])));
                    Expr::Convert { how: Convert::ToTensor, expr: Box::new(one), ty }
                }
                _ => held_as(one, &want),
            };
            let ty = Ty::List(Box::new(each.ty()));
            Expr::ForExpr { var, iter: Box::new(value), body: vec![Stmt::Do(each)], ty, parallel: false }
        }
        _ => value,
    }
}

/// A plain list where a list whose items can fail is held: each item the
/// success it is — a literal holds plain items as such, any other list is
/// walked by a collecting loop, whose plain rounds the backends hold as
/// successes.
fn fallible_items(value: Expr, item: &Ty) -> Expr {
    match value {
        Expr::ListLit(items, _) => Expr::ListLit(items, Ty::List(Box::new(item.clone()))),
        other => {
            let var = held_name();
            let one = Expr::Local(var.clone(), item.payload().clone());
            Expr::ForExpr { var, iter: Box::new(other), body: vec![Stmt::Do(one)], ty: Ty::List(Box::new(item.clone())), parallel: false }
        }
    }
}

/// A list of ints where a list of records or none is held — empty buckets
/// made of zeros, written records later: each item checked as one int
/// handed alone is (0 none, another stopping), by a collecting loop that
/// answers the list of nones (Vijay, 2026-10-29).
fn nones_of(x: Expr, slot: &Ty) -> Expr {
    let Ty::List(item) = slot else { return x };
    let Some(row) = crate::table::ROWS.iter().find(|r| r.id == "core.__none") else { return x };
    let var = held_name();
    let each = vec![
        Stmt::Do(Expr::Call { target: Target::Row(row), args: vec![Expr::Local(var.clone(), Ty::Int)], ty: Ty::Int, propagates: false }),
        Stmt::Do(Expr::Const(Const::Int(0), item.as_ref().clone())),
    ];
    Expr::ForExpr { var, iter: Box::new(x), body: each, ty: slot.clone(), parallel: false }
}

/// A hidden name for a converting loop's item, one per loop.
fn held_name() -> String {
    static HELD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    format!("_held{}", HELD.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Whether two item types hold numbers two ways at some depth: a list of
/// numbers against a vector.
fn forms_differ(want: &Ty, have: &Ty) -> bool {
    let vector = |d: &Option<crate::type_infer::Dims>| d.as_ref().is_none_or(|d| d.len() == 1);
    match (want, have) {
        (Ty::Tensor(_, d), Ty::List(e)) | (Ty::List(e), Ty::Tensor(_, d)) => matches!(e.as_ref(), Ty::Int | Ty::Float) && vector(d),
        (Ty::List(a), Ty::List(b)) => forms_differ(a, b),
        _ => false,
    }
}

/// A `set` or `push` answer in the form its collection holds its items:
/// the holder's form where the answer's items differ only in form (see
/// `held_as`).
fn in_holders_form(name: &str, lowered: &[Expr], ty: Ty) -> Ty {
    let holder = match (name, lowered.first()) {
        ("set" | "push", Some(h)) => h.ty().payload().clone(),
        _ => return ty,
    };
    let failing = ty.is_result();
    let (held, answered) = match (&holder, ty.payload()) {
        (Ty::Map(_, h), Ty::Map(_, a)) | (Ty::List(h), Ty::List(a)) => (h.as_ref().clone(), a.as_ref().clone()),
        _ => return ty,
    };
    fn holders(held: &Ty, answered: &Ty) -> Option<Ty> {
        match (held.payload(), answered.payload()) {
            (Ty::Tensor(e, d), Ty::List(a)) if matches!(a.payload(), Ty::Int | Ty::Float | Ty::Unknown) => Some(Ty::Tensor(e.clone(), d.clone())),
            (Ty::List(h), Ty::Tensor(..)) if matches!(h.payload(), Ty::Int | Ty::Float) => Some(Ty::List(h.clone())),
            // (at any depth: a list of lists of numbers where the items are
            // lists of vectors)
            (Ty::List(h), Ty::List(a)) => holders(h, a).map(|i| Ty::List(Box::new(i))),
            _ => None,
        }
    }
    let Some(item) = holders(&held, &answered) else { return ty };
    let inner = match holder {
        Ty::Map(k, _) => Ty::Map(k, Box::new(item)),
        _ => Ty::List(Box::new(item)),
    };
    if failing {
        Ty::Result(Box::new(inner))
    } else {
        inner
    }
}

fn declared_as(first: &Ty, settled: Option<&Ty>) -> Ty {
    if matches!(first, Ty::Node) {
        return Ty::Markup;
    }
    if *first == Ty::Int && matches!(settled, Some(Ty::Float)) {
        return Ty::Float;
    }
    // whole numbers first held as a tensor, a list of ints later: a list of
    // ints from the declaration (type_infer::settle)
    if crate::type_infer::settle::is_ints(first) && matches!(settled, Some(Ty::List(item)) if matches!(item.as_ref(), Ty::Int)) {
        return Ty::List(Box::new(Ty::Int));
    }
    // (and rows of them, a list of lists of ints)
    if crate::type_infer::settle::is_int_rows(first) && matches!(settled, Some(t) if crate::type_infer::settle::is_int_lists(t)) {
        return Ty::List(Box::new(Ty::List(Box::new(Ty::Int))));
    }
    // a list of ints that settles on records or none holds them from its
    // declaration, its 0s none (`nones_of`)
    if matches!(first, Ty::List(i) if **i == Ty::Int)
        && let Some(t @ Ty::List(i)) = settled
        && matches!(i.as_ref(), Ty::OptRecord(_))
    {
        return t.clone();
    }
    // a variable that settles on a record or none — first 0, the empty
    // sentinel, or a record, and the other later — holds a record or none
    // from its declaration (the Rust backend declares it so)
    if let Some(Ty::OptRecord(n)) = settled {
        if *first == Ty::Int || matches!(first, Ty::Record(_)) {
            return Ty::OptRecord(n.clone());
        }
    }
    if !first.has_unknown() {
        return first.clone();
    }
    match settled {
        Some(t) if !matches!(t, Ty::Unknown) => {
            let settled = (t).clone();
            if first.is_result() {
                settled
            } else {
                settled.payload().clone()
            }
        }
        _ => first.clone(),
    }
}

/// What a collecting loop's lowered body yields: its last expression, or
/// what the branches of a trailing `if` yield (the filter pattern);
/// unknown when the body ends otherwise.
fn yield_of(body: &Block) -> Ty {
    match body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
        Some(Stmt::Do(x)) => full(x),
        Some(Stmt::If { then, els, .. }) => {
            let t = yield_of(then);
            match els {
                Some(e) => t.unify(&yield_of(e)),
                None => t,
            }
        }
        _ => Ty::Unknown,
    }
}

/// The facts a function can write: a part's function any part's, whose
/// names carry that part's prefix — an unprefixed name there is a local.
fn own_facts(f: &str, facts: &BTreeSet<String>) -> BTreeSet<String> {
    match crate::part_prefix_of_declaration(f) {
        // any part's (a part writes one it uses), never the entry's
        Some(_) => facts.iter().filter(|m| crate::part_prefix_of_declaration(m).is_some()).cloned().collect(),
        None => facts.clone(),
    }
}

/// The loop variables of a body, at any depth.
/// The line of the first `return` in a body that returns no value.
fn bare_return(body: &[Stmt], mut line: usize) -> Option<usize> {
    for s in body {
        match s {
            Stmt::Line(n) => line = *n,
            Stmt::Return(None) => return Some(line),
            Stmt::If { then, els, .. } => {
                if let Some(l) = bare_return(then, line).or_else(|| els.as_ref().and_then(|b| bare_return(b, line))) {
                    return Some(l);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                if let Some(l) = bare_return(body, line) {
                    return Some(l);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a body answers where it ends: its last statement returns a
/// value (an implicit return of nothing does not), is an `if` whose every
/// arm ends answered, or is a `while true` that only a `return` leaves.
fn ends_answered(body: &[Stmt]) -> bool {
    // (a body ending where the program does — `halt[…]` — needs no answer
    // there: it had been refused as able to end without one; Rosetta Code,
    // Sequence: smallest number greater than previous term with exactly n
    // divisors)
    match body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
        Some(Stmt::Return(Some(e))) => e.ty() != Ty::Unit || crate::ir::ends_program(e),
        Some(Stmt::Do(e)) => crate::ir::ends_program(e),
        Some(Stmt::If { then, els: Some(els), .. }) => ends_answered(then) && ends_answered(els),
        Some(Stmt::While { cond: Expr::Const(Const::Bool(true), _), body }) => !breaks(body),
        _ => false,
    }
}

/// Whether a loop's body breaks out of that loop (a `break` of a loop
/// inside it is that loop's).
fn breaks(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Break => true,
        Stmt::If { then, els, .. } => breaks(then) || els.as_ref().is_some_and(|b| breaks(b)),
        _ => false,
    })
}

fn loop_vars(body: &[S], out: &mut HashSet<String>) {
    for s in body {
        match s {
            S::For { var, body, .. } => {
                out.insert(var.clone());
                loop_vars(body, out);
            }
            S::While { body, .. } => loop_vars(body, out),
            S::If { then_body, else_body, .. } => {
                loop_vars(then_body, out);
                if let Some(b) = else_body {
                    loop_vars(b, out);
                }
            }
            _ => {}
        }
    }
}

fn assigns_any(body: &[S], names: &BTreeSet<String>) -> bool {
    body.iter().any(|s| match s {
        S::Assign { name, .. } => names.contains(name),
        S::MultiAssign { names: ns, .. } => ns.iter().any(|n| names.contains(n)),
        S::FieldAssign { object, .. } | S::IndexAssign { object, .. } => root_ident(object).map_or(false, |n| names.contains(n)),
        S::If { then_body, else_body, .. } => assigns_any(then_body, names) || else_body.as_ref().map_or(false, |b| assigns_any(b, names)),
        S::For { body, .. } | S::While { body, .. } => assigns_any(body, names),
        _ => false,
    })
}


fn root_ident(e: &A) -> Option<&str> {
    match e {
        A::Ident(n) => Some(n),
        A::FieldAccess { object, .. } => root_ident(object),
        A::Call { callee, .. } => root_ident(callee),
        _ => None,
    }
}

/// The facts any function's body writes: those are per session.
/// The variable a field or index assignment writes into: `a` of `a.b[0]`.
fn target_root(object: &A) -> Option<String> {
    match object {
        A::Ident(n) => Some(n.clone()),
        A::FieldAccess { object, .. } => target_root(object),
        A::Call { callee, .. } => target_root(callee),
        _ => None,
    }
}

fn facts_written_by_fns(stmts: &[S], facts: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(body: &[S], facts: &BTreeSet<String>, out: &mut BTreeSet<String>) {
        for s in body {
            match s {
                S::Assign { name, .. } if facts.contains(name) => {
                    out.insert(name.clone());
                }
                S::MultiAssign { names, .. } => {
                    for n in names {
                        if facts.contains(n) {
                            out.insert(n.clone());
                        }
                    }
                }
                S::FieldAssign { object, .. } | S::IndexAssign { object, .. } => {
                    if let Some(n) = root_ident(object) {
                        if facts.contains(n) {
                            out.insert(n.to_string());
                        }
                    }
                }
                S::If { then_body, else_body, .. } => {
                    walk(then_body, facts, out);
                    if let Some(b) = else_body {
                        walk(b, facts, out);
                    }
                }
                S::For { body, .. } | S::While { body, .. } => walk(body, facts, out),
                _ => {}
            }
        }
    }
    for s in stmts {
        if let S::FnDecl { body, .. } = s {
            walk(body, facts, &mut out);
        }
    }
    out
}

fn reads_of_expr(e: &A, names: &mut BTreeSet<String>, clock: &mut bool) {
    reads_of_stmts(&[S::ExprStmt(e.clone())], names, clock);
}

fn reads_of_stmts(body: &[S], names: &mut BTreeSet<String>, clock: &mut bool) {
    fn in_expr(e: &A, names: &mut BTreeSet<String>, clock: &mut bool) {
        match e {
            A::Ident(n) => {
                names.insert(n.clone());
            }
            A::Call { callee, args } => {
                if let A::Ident(n) = callee.as_ref() {
                    if n == "now" {
                        *clock = true;
                    }
                    names.insert(n.clone());
                } else {
                    in_expr(callee, names, clock);
                }
                for a in args {
                    in_expr(a, names, clock);
                }
            }
            A::BinOp { left, right, .. } => {
                in_expr(left, names, clock);
                in_expr(right, names, clock);
            }
            A::UnaryOp { expr, .. } => in_expr(expr, names, clock),
            A::FieldAccess { object, .. } => in_expr(object, names, clock),
            A::Array(items) => items.iter().for_each(|i| in_expr(i, names, clock)),
            A::Map(pairs) => pairs.iter().for_each(|(k, v)| {
                in_expr(k, names, clock);
                in_expr(v, names, clock);
            }),
            A::Named { value, .. } => in_expr(value, names, clock),
            A::ForExpr { iter, body, .. } => {
                in_expr(iter, names, clock);
                reads_of_stmts(body, names, clock);
            }
            A::Str(s) => {
                for n in crate::parallel::str_interpolation_reads(s) {
                    names.insert(n);
                }
                if s.contains("now[") {
                    *clock = true;
                }
            }
            _ => {}
        }
    }
    for s in body {
        match s {
            S::Assign { value, .. } => in_expr(value, names, clock),
            S::MultiAssign { values, .. } => values.iter().for_each(|v| in_expr(v, names, clock)),
            S::FieldAssign { value, .. } => in_expr(value, names, clock),
            S::IndexAssign { index, value, .. } => {
                in_expr(index, names, clock);
                in_expr(value, names, clock);
            }
            S::ExprStmt(e) | S::Return(Some(e)) => in_expr(e, names, clock),
            S::If { condition, then_body, else_body } => {
                in_expr(condition, names, clock);
                reads_of_stmts(then_body, names, clock);
                if let Some(b) = else_body {
                    reads_of_stmts(b, names, clock);
                }
            }
            S::For { iter, body, .. } => {
                in_expr(iter, names, clock);
                reads_of_stmts(body, names, clock);
            }
            S::While { condition, body } => {
                in_expr(condition, names, clock);
                reads_of_stmts(body, names, clock);
            }
            _ => {}
        }
    }
}

// ── moves: a local's last read ──────────────────────────────────────

/// Turn the last read of each local into a move. `live_after` are the
/// locals read after this block; `allowed` restricts moves to these
/// locals (inside a loop body, the ones the body itself defines, since
/// the body repeats), `None` allows every local the block defines.
fn place_moves_in(block: &mut Block, live_after: &HashSet<String>, allowed: Option<&HashSet<String>>) {
    // (`CROW_NO_MOVES=1`: every read a copy and nothing let go early — the
    // program `crow check --self` holds the move pass to)
    static NO_MOVES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *NO_MOVES.get_or_init(|| std::env::var("CROW_NO_MOVES").is_ok_and(|v| !v.is_empty() && v != "0")) {
        return;
    }
    let mut leaving: HashSet<String> = defined_in(block);
    if let Some(a) = allowed {
        leaving.extend(a.iter().cloned());
    }
    place_moves_leaving(block, live_after, allowed, &leaving);
}

/// `place_moves_in`, where `leaving` are the names a `return` may move:
/// what the outermost block may, however deep the return lies — a loop's
/// body moves nothing defined outside it, since the body repeats, but a
/// return leaves the function and nothing reads the name again (`return
/// m` inside a `while` had copied the record it handed back, at every
/// call; Rosetta Code, Universal lambda machine).
fn place_moves_leaving(block: &mut Block, live_after: &HashSet<String>, allowed: Option<&HashSet<String>>, leaving: &HashSet<String>) {
    let defined: HashSet<String> = defined_in(block);
    let mut allowed_set: HashSet<String> = defined.clone();
    if let Some(a) = allowed {
        allowed_set.extend(a.iter().cloned());
    }
    // a name an inner block defines is dead after it (spec, Scope: a
    // variable is scoped to the block it is defined in, and one set again
    // after the block is another variable): its last read in the block
    // moves, even where the block is a loop's body and the next round reads
    // the name — that round binds it afresh (`out = push[out, piece]` had
    // copied `piece` in both modes). The outermost block's names are what
    // its caller says is live after it (a notebook's next cell reads them).
    let mut live: HashSet<String> = if allowed.is_some() { live_after.iter().filter(|n| !defined.contains(*n)).cloned().collect() } else { live_after.clone() };
    // the block's own locals a statement reads for the last time and does
    // not move, freed after it (`move x` alone, which both backends drop
    // there): a value read last as an indexed base, or by a loop, had lived
    // to the block's end — revcomp's input slice and its reversed lines, a
    // section of 185 MB each, beside the lines made from them. Never after
    // the block's last statement (its value, or the end that frees them),
    // nor after one that leaves.
    let last = block.iter().rposition(|s| !matches!(s, Stmt::Line(_)));
    // `x = f[x, …]` whose call can fail is lowered `let _t = f[x, …]`,
    // `propagate _t (into x)`, `x = unwrap move _t`: x is assigned anew
    // after the call on its good path and read by no one after a failure
    // the assignment refuses, so the call takes x as `x = f[x]` does, in a
    // loop too (a list grown by a call handed a value that can fail beside
    // it had been copied at every call; Rosetta Code, Universal lambda
    // machine)
    let reassigned: HashMap<usize, String> = (0..block.len())
        .filter_map(|at| {
            let Stmt::Let { var: t, .. } = &block[at] else { return None };
            let mut next = block[at + 1..].iter().filter(|s| !matches!(s, Stmt::Line(_)));
            let Some(Stmt::Propagate { values, at: Decision::Assign(x) }) = next.next() else { return None };
            let Some(Stmt::Set { var, expr: Expr::Convert { how: Convert::Unwrap, expr: taken, .. } }) = next.next() else { return None };
            let takes_t = matches!(taken.as_ref(), Expr::Move(v, _) | Expr::Local(v, _) if v == t);
            (values.len() == 1 && values[0] == *t && var == x && takes_t).then(|| (at, x.clone()))
        })
        .collect();
    let mut frees: Vec<(usize, String)> = Vec::new();
    for (at, s) in block.iter_mut().enumerate().rev() {
        // a return leaves the function: nothing after it is read on its path
        // (`return slice[pieces, 0, n - 1]` in a branch, `return pieces`
        // after the branch, had copied every line of a file to drop one)
        if matches!(s, Stmt::Return(_)) {
            live.clear();
        }
        let mut reads = HashSet::new();
        stmt_reads(s, &mut reads);
        // nested blocks first, with what is live after them
        match s {
            Stmt::If { then, els, .. } => {
                let after = live.clone();
                place_moves_leaving(then, &after, Some(&allowed_set), leaving);
                if let Some(e) = els {
                    place_moves_leaving(e, &after, Some(&allowed_set), leaving);
                }
            }
            Stmt::While { cond, body } => {
                // the body repeats: nothing defined outside it moves inside
                let mut body_reads = HashSet::new();
                for b in body.iter() {
                    stmt_reads(b, &mut body_reads);
                }
                let mut after = live.clone();
                after.extend(body_reads);
                // (and the condition is read again after every round)
                expr_reads(cond, &mut after);
                let inner_allowed = defined_in(body);
                place_moves_leaving(body, &after, Some(&inner_allowed), leaving);
            }
            Stmt::For { var, iter, body, .. } => {
                let mut body_reads = HashSet::new();
                for b in body.iter() {
                    stmt_reads(b, &mut body_reads);
                }
                let mut after = live.clone();
                after.extend(body_reads);
                // (the loop's variable is bound afresh each round too)
                after.remove(var);
                let mut inner_allowed = defined_in(body);
                inner_allowed.insert(var.clone());
                // (what the loop walks is lent to it for the whole loop: a
                // return inside does not move it — `return push[rs, r]` of
                // a loop over `rs` moved the list while its item was read)
                let mut walked = HashSet::new();
                expr_reads(iter, &mut walked);
                let inner_leaving: HashSet<String> = leaving.difference(&walked).cloned().collect();
                place_moves_leaving(body, &after, Some(&inner_allowed), &inner_leaving);
            }
            _ => {}
        }
        // this statement's own expressions, last read first; `x = f[x]`
        // redefines x before anyone reads it again, so the old x moves
        // even when the statement sits in a loop; the head of an if, a
        // while or a for is read before its blocks, so what they read is
        // still to come
        let mut seen: HashSet<String> = HashSet::new();
        let (live_here, allowed_here) = match s {
            Stmt::Set { var, .. } => {
                let mut l = live.clone();
                l.remove(var);
                let mut a = allowed_set.clone();
                a.insert(var.clone());
                (l, a)
            }
            Stmt::If { then, els, .. } => {
                let mut l = live.clone();
                for b in then.iter() {
                    stmt_reads(b, &mut l);
                }
                if let Some(e) = els {
                    for b in e.iter() {
                        stmt_reads(b, &mut l);
                    }
                }
                (l, allowed_set.clone())
            }
            // a while loop's condition is read again after every round: what
            // it reads is still to come, and does not move in it (`while i lt
            // len[s]` had moved `s` on the first round — the interpreter
            // then measured nothing, the compiled program had read it)
            Stmt::While { cond, body } => {
                let mut l = live.clone();
                expr_reads(cond, &mut l);
                for b in body.iter() {
                    stmt_reads(b, &mut l);
                }
                (l, allowed_set.clone())
            }
            Stmt::For { body, .. } => {
                let mut l = live.clone();
                for b in body.iter() {
                    stmt_reads(b, &mut l);
                }
                (l, allowed_set.clone())
            }
            // (a return leaves: what the function may move, it moves)
            Stmt::Return(_) => (live.clone(), allowed_set.union(leaving).cloned().collect()),
            Stmt::Let { .. } if reassigned.contains_key(&at) => {
                let x = &reassigned[&at];
                let mut l = live.clone();
                l.remove(x);
                let mut a = allowed_set.clone();
                a.insert(x.clone());
                (l, a)
            }
            _ => (live.clone(), allowed_set.clone()),
        };
        // (a name two of the statement's expressions read — an index
        // assignment's key and its value, `m[k] = m[k] + 1` — moves in
        // neither: the backends read the value first, then the key)
        let mut readers: HashMap<String, usize> = HashMap::new();
        for e in stmt_exprs_mut(s) {
            let mut names = HashSet::new();
            expr_reads(e, &mut names);
            for n in names {
                *readers.entry(n).or_insert(0) += 1;
            }
        }
        seen.extend(readers.into_iter().filter(|(_, c)| *c > 1).map(|(n, _)| n));
        for e in stmt_exprs_mut(s).into_iter().rev() {
            move_last_reads(e, &live_here, &allowed_here, &mut seen);
        }
        if Some(at) != last && !matches!(s, Stmt::Return(_) | Stmt::Break | Stmt::Continue) {
            let mut moved = HashSet::new();
            stmt_moves(s, &mut moved);
            // (a record read by its fields may have handed them on: the
            // compiled program moves a field read for the last time, and
            // what is left of the record goes with its block)
            stmt_field_bases(s, &mut moved);
            let target = match s {
                Stmt::Let { var, .. } | Stmt::Set { var, .. } => Some(var.as_str()),
                _ => None,
            };
            for v in &reads {
                if defined.contains(v) && !live.contains(v) && !moved.contains(v) && target != Some(v.as_str()) {
                    frees.push((at, v.clone()));
                }
            }
        }
        match s {
            Stmt::Let { var, .. } | Stmt::Set { var, .. } => {
                live.remove(var);
            }
            _ => {}
        }
        live.extend(reads);
    }
    // each after its statement, of the binding it reads there: one this
    // block made before it (a name read before the block binds it anew is
    // an outer binding, which is not this block's to free)
    frees.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (at, v) in frees {
        let ty = block[..=at].iter().rev().find_map(|s| match s {
            Stmt::Let { var, ty, .. } if *var == v => Some(ty.clone()),
            _ => None,
        });
        if let Some(ty) = ty.filter(movable) {
            block.insert(at + 1, Stmt::Do(Expr::Move(v, ty)));
        }
    }
}

/// Whether an expression reads exactly the place a path write names: the
/// local at the path's root, then each step — an index by the same
/// expression, a field of the same name (`grown_in_place`).
fn reads_place(e: &Expr, var: &str, path: &[PathStep]) -> bool {
    match (path.split_last(), e) {
        (None, Expr::Local(v, _) | Expr::Move(v, _)) => v == var,
        (Some((PathStep::Index(k), front)), Expr::Index { base, indices, .. }) => indices.len() == 1 && same_expr(&indices[0], k) && reads_place(base, var, front),
        (Some((PathStep::Field(f), front)), Expr::Field { base, name, .. }) => name == f && reads_place(base, var, front),
        _ => false,
    }
}

/// Two quiet expressions written the same — a local, a constant, a field
/// of those — which both backends evaluate to one value.
fn same_expr(a: &Expr, b: &Expr) -> bool {
    crate::ir::quiet(a) && crate::ir::quiet(b) && format!("{:?}", a) == format!("{:?}", b)
}

/// The names a statement reads a field of, anywhere in it.
fn stmt_field_bases(s: &Stmt, out: &mut HashSet<String>) {
    crate::ir::visit_block(std::slice::from_ref(s), &mut |e| {
        if let Expr::Field { base, .. } = e
            && let Some(v) = named(base)
        {
            out.insert(v.clone());
        }
    });
    fn named(e: &Expr) -> Option<&String> {
        match e {
            Expr::Local(v, _) | Expr::Move(v, _) => Some(v),
            Expr::Convert { expr, .. } => named(expr),
            _ => None,
        }
    }
}

/// The names a statement moves, anywhere in it.
fn stmt_moves(s: &Stmt, out: &mut HashSet<String>) {
    crate::ir::visit_block(std::slice::from_ref(s), &mut |e| {
        if let Expr::Move(v, _) = e {
            out.insert(v.clone());
        }
    });
}

fn defined_in(block: &Block) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in block {
        if let Stmt::Let { var, .. } = s {
            out.insert(var.clone());
        }
    }
    out
}

/// The top-level expressions of a statement, in evaluation order.
fn stmt_exprs_mut(s: &mut Stmt) -> Vec<&mut Expr> {
    match s {
        Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => vec![expr],
        Stmt::SetPath { path, expr, .. } => {
            let mut v: Vec<&mut Expr> = Vec::new();
            for p in path.iter_mut() {
                if let PathStep::Index(i) = p {
                    v.push(i);
                }
            }
            v.push(expr);
            v
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => vec![cond],
        Stmt::For { iter, .. } => vec![iter],
        Stmt::Return(Some(e)) => vec![e],
        _ => vec![],
    }
}

/// Walk an expression in reverse evaluation order; the first `Local`
/// met for a variable is its last read.
fn move_last_reads(e: &mut Expr, live: &HashSet<String>, allowed: &HashSet<String>, seen: &mut HashSet<String>) {
    match e {
        // a move placed before (a collecting loop's body places its own) is
        // the last read: nothing before it moves again (two moves of one
        // name in an expression — `label[move t, len[move t]]` — had read
        // nothing the second time)
        Expr::Move(v, _) => {
            seen.insert(v.clone());
        }
        Expr::Local(v, ty) => {
            if movable(ty) && !live.contains(v) && allowed.contains(v) && !seen.contains(v) {
                let name = v.clone();
                let t = ty.clone();
                *e = Expr::Move(name, t);
            }
            if let Expr::Move(v, _) | Expr::Local(v, _) = e {
                seen.insert(v.clone());
            }
        }
        Expr::Call { args, .. } | Expr::Effect { args, .. } => {
            // a name two arguments read moves in neither: the backends do not
            // evaluate a call's arguments in one order (the emitter computes
            // one that can fail first, to hand its failure on), so the last
            // read in one is not the last in the other (`map[path = p, words
            // = read[p]]` had moved `p` into the read before the map's key
            // read it, compiled)
            let mut readers: HashMap<String, usize> = HashMap::new();
            for a in args.iter() {
                let mut names = HashSet::new();
                expr_reads(a, &mut names);
                for n in names {
                    *readers.entry(n).or_insert(0) += 1;
                }
            }
            seen.extend(readers.into_iter().filter(|(_, c)| *c > 1).map(|(n, _)| n));
            for a in args.iter_mut().rev() {
                move_last_reads(a, live, allowed, seen);
            }
        }
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => move_last_reads(expr, live, allowed, seen),
        Expr::Binary { l, r, .. } => {
            // a name both sides read moves in neither: the compiled program
            // holds the left side lent while it computes the right (`v1 *
            // transpose[v1]`, v1 able to fail, had moved v1 into the right
            // side while the left still held it, and did not build — fuzz
            // seed 1791669691)
            let mut left = HashSet::new();
            expr_reads(l, &mut left);
            let mut right = HashSet::new();
            expr_reads(r, &mut right);
            seen.extend(left.intersection(&right).cloned());
            move_last_reads(r, live, allowed, seen);
            move_last_reads(l, live, allowed, seen);
        }
        Expr::Index { base, indices, .. } => {
            // an indexed base is borrowed, never moved, and both backends read
            // it after its indices: an index reading it is no last read
            // (`segments[len[move segments] - 1]` had handed the list away
            // before it was indexed, compiled)
            note_reads(base, seen);
            for i in indices.iter_mut().rev() {
                move_last_reads(i, live, allowed, seen);
            }
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => note_reads(base, seen),
        Expr::Interp(pieces) => {
            // the pieces of a text are held together while it is written: a
            // name two pieces read moves in neither (`'`t.feature` is
            // `classify[t, x]`'` had moved `t` into the call while its field
            // was still lent to the text, compiled)
            let mut readers: HashMap<String, usize> = HashMap::new();
            for p in pieces.iter() {
                if let Piece::Expr(x) = p {
                    let mut names = HashSet::new();
                    expr_reads(x, &mut names);
                    for n in names {
                        *readers.entry(n).or_insert(0) += 1;
                    }
                }
            }
            seen.extend(readers.into_iter().filter(|(_, c)| *c > 1).map(|(n, _)| n));
            for p in pieces.iter_mut().rev() {
                if let Piece::Expr(x) = p {
                    move_last_reads(x, live, allowed, seen);
                }
            }
        }
        Expr::ListLit(items, _) => {
            for i in items.iter_mut().rev() {
                move_last_reads(i, live, allowed, seen);
            }
        }
        Expr::MapLit(pairs, _) => {
            for (_, v) in pairs.iter_mut().rev() {
                move_last_reads(v, live, allowed, seen);
            }
        }
        Expr::RecordLit { fields, .. } => {
            for f in fields.iter_mut().rev() {
                move_last_reads(f, live, allowed, seen);
            }
        }
        Expr::Tree { attrs, children, .. } => {
            for c in children.iter_mut().rev() {
                move_last_reads(c, live, allowed, seen);
            }
            for (_, a) in attrs.iter_mut().rev() {
                move_last_reads(a, live, allowed, seen);
            }
        }
        // (a collecting loop's body placed its own moves where it was lowered)
        Expr::ForExpr { iter, body, .. } => {
            let mut body_reads = HashSet::new();
            for b in body.iter() {
                stmt_reads(b, &mut body_reads);
            }
            seen.extend(body_reads);
            move_last_reads(iter, live, allowed, seen);
        }
        Expr::Deferred { args, .. } => {
            for a in args.iter_mut().rev() {
                move_last_reads(a, live, allowed, seen);
            }
        }
        _ => {}
    }
}

/// A decimal or a fraction (spec, Numbers That Keep Every Digit).
fn exact_kind(t: &Ty) -> bool {
    matches!(t.payload(), Ty::Decimal | Ty::Fraction)
}

impl<'a> Lowerer<'a> {
    /// The operands of an operator meeting an exact number, each as the
    /// exact kind the operator computes in: an int meeting a decimal is the
    /// decimal it is, an int or a decimal meeting a fraction the fraction it
    /// is; a float written as a literal is the decimal it writes, and any
    /// other float is refused before the program runs — the exact answer
    /// would be a guess. `%` takes ints. A power's exponent stays an int.
    fn exact_operands(&mut self, op: &BinOp, l: Expr, r: Expr) -> (Expr, Expr) {
        let (lt, rt) = (l.ty().payload().clone(), r.ty().payload().clone());
        let fraction = matches!(lt, Ty::Fraction) || matches!(rt, Ty::Fraction);
        let want = if fraction { Ty::Fraction } else { Ty::Decimal };
        let final_pass = self.pass == Pass::Final;
        if matches!(op, BinOp::Mod) {
            if final_pass {
                self.refuse(format!("`%` takes ints — a {}'s remainder is written out: d - n * int[d / n]", want.crow_name()));
            }
            return (l, r);
        }
        let literal = |e: &Expr| match e {
            Expr::Const(Const::Float(_), _) => true,
            Expr::Unary { op, expr, .. } if op == "neg" => matches!(expr.as_ref(), Expr::Const(Const::Float(_), _)),
            _ => false,
        };
        // a float beside an exact number: a literal is the number it writes
        let l_float = matches!(lt, Ty::Float) && !literal(&l);
        let r_float = matches!(rt, Ty::Float) && !literal(&r);
        if (l_float || r_float) && final_pass {
            let name = |e: &Expr| match e {
                Expr::Local(n, _) | Expr::Move(n, _) => n.clone(),
                _ => "…".to_string(),
            };
            let (exact, float) = if l_float { (&r, &l) } else { (&l, &r) };
            let k = want.crow_name();
            self.refuse(format!("a {k} and a float: {k}[{}] reads the float as the {k} it shows, float[{}] reads the {k} as a float", name(float), name(exact)));
        }
        let to = |e: Expr, me: &Self| -> Expr {
            let _ = me;
            let have = e.ty().payload().clone();
            let fails = e.ty().is_result();
            let keep = |t: Ty| if fails { Ty::Result(Box::new(t)) } else { t };
            match (&have, &want) {
                (Ty::Int | Ty::Big | Ty::Float, Ty::Decimal) => Expr::Convert { how: Convert::ToDecimal, ty: keep(Ty::Decimal), expr: Box::new(e) },
                (Ty::Int | Ty::Big | Ty::Decimal, Ty::Fraction) => Expr::Convert { how: Convert::ToFraction, ty: keep(Ty::Fraction), expr: Box::new(e) },
                (Ty::Float, Ty::Fraction) => {
                    let d = Expr::Convert { how: Convert::ToDecimal, ty: keep(Ty::Decimal), expr: Box::new(e) };
                    Expr::Convert { how: Convert::ToFraction, ty: keep(Ty::Fraction), expr: Box::new(d) }
                }
                _ => e,
            }
        };
        // a power's exponent: an int as it is (`d ^ 2`, `f ^ -1`)
        if matches!(op, BinOp::Pow) && matches!(rt, Ty::Int | Ty::Big) && exact_kind(&lt) {
            return (l, r);
        }
        (to(l, self), to(r, self))
    }
}

/// A value worth moving: anything that owns memory. A scalar copies.
fn movable(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Unit | Ty::Fn(_, _) | Ty::Handle(_) | Ty::Sparse | Ty::WebRequest => false,
        Ty::Result(inner) => movable(inner),
        _ => true,
    }
}

fn note_reads(e: &Expr, seen: &mut HashSet<String>) {
    let mut r = HashSet::new();
    expr_reads(e, &mut r);
    seen.extend(r);
}

fn stmt_reads(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => expr_reads(expr, out),
        Stmt::SetPath { var, path, expr, .. } => {
            out.insert(var.clone());
            for p in path {
                if let PathStep::Index(i) = p {
                    expr_reads(i, out);
                }
            }
            expr_reads(expr, out);
        }
        Stmt::If { cond, then, els } => {
            expr_reads(cond, out);
            then.iter().for_each(|x| stmt_reads(x, out));
            if let Some(e) = els {
                e.iter().for_each(|x| stmt_reads(x, out));
            }
        }
        Stmt::While { cond, body } => {
            expr_reads(cond, out);
            body.iter().for_each(|x| stmt_reads(x, out));
        }
        Stmt::For { iter, body, .. } => {
            expr_reads(iter, out);
            body.iter().for_each(|x| stmt_reads(x, out));
        }
        Stmt::Return(Some(e)) => expr_reads(e, out),
        Stmt::Propagate { values, .. } => out.extend(values.iter().cloned()),
        _ => {}
    }
}

fn expr_reads(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Local(v, _) | Expr::Move(v, _) => {
            out.insert(v.clone());
        }
        Expr::Call { args, .. } | Expr::Effect { args, .. } => args.iter().for_each(|a| expr_reads(a, out)),
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => expr_reads(expr, out),
        Expr::Binary { l, r, .. } => {
            expr_reads(l, out);
            expr_reads(r, out);
        }
        Expr::Index { base, indices, .. } => {
            expr_reads(base, out);
            indices.iter().for_each(|i| expr_reads(i, out));
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => expr_reads(base, out),
        Expr::Interp(pieces) => pieces.iter().for_each(|p| {
            if let Piece::Expr(x) = p {
                expr_reads(x, out);
            }
        }),
        Expr::ListLit(items, _) => items.iter().for_each(|i| expr_reads(i, out)),
        Expr::MapLit(pairs, _) => pairs.iter().for_each(|(_, v)| expr_reads(v, out)),
        Expr::RecordLit { fields, .. } => fields.iter().for_each(|f| expr_reads(f, out)),
        Expr::Tree { attrs, children, .. } => {
            attrs.iter().for_each(|(_, a)| expr_reads(a, out));
            children.iter().for_each(|c| expr_reads(c, out));
        }
        Expr::ForExpr { iter, body, .. } => {
            expr_reads(iter, out);
            body.iter().for_each(|s| stmt_reads(s, out));
        }
        Expr::Deferred { args, .. } => args.iter().for_each(|a| expr_reads(a, out)),
        _ => {}
    }
}


/// The variables a block binds to a list written down and grows with
/// `push` or writes by position — `v = [...]`, then `v = push[v, …]` or
/// `v[i] = …`, anywhere in it, nested blocks included: a list of whole
/// numbers written down for one is a list of ints, as a tensor's float
/// would round what is pushed or written past 2^53. A parameter is no such
/// variable: its kind is its callers'.
/// What a block's statements grow by `push`, the block read whole.
fn top_grown(stmts: &[S]) -> HashSet<String> {
    let mut grown = HashSet::new();
    grown_by_push(stmts, &mut grown);
    grown
}

fn grown_by_push(stmts: &[S], out: &mut HashSet<String>) {
    fn walk(stmts: &[S], grown: &mut HashSet<String>, written: &mut HashSet<String>) {
        for s in stmts {
            match s {
                S::Assign { name, value: A::Array(_) } => {
                    written.insert(name.clone());
                }
                S::Assign { name, value: A::Call { callee, args } } if matches!(callee.as_ref(), A::Ident(f) if f == "push") && matches!(args.first(), Some(A::Ident(v)) if v == name) => {
                    grown.insert(name.clone());
                }
                S::IndexAssign { object: A::Ident(v), .. } => {
                    grown.insert(v.clone());
                }
                S::If { then_body, else_body, .. } => {
                    walk(then_body, grown, written);
                    if let Some(e) = else_body {
                        walk(e, grown, written);
                    }
                }
                S::For { body, .. } | S::While { body, .. } => walk(body, grown, written),
                _ => {}
            }
        }
    }
    let (mut grown, mut written) = (HashSet::new(), HashSet::new());
    walk(stmts, &mut grown, &mut written);
    out.extend(grown.intersection(&written).cloned());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tools = [price, listed, far, weather]` handed to a model is a group:
    /// each held by its name, and the module describes each through
    /// `describe` — its name, the comment above it, its parameters' kinds
    /// as the words errors use (a parameter handed a parsed value is the
    /// kind the body reads it as; a record's is a record), what it answers
    #[test]
    fn a_tools_list_of_functions_is_a_group_each_described() {
        let src = "use llm

--- the price of n units at a rate
fn price[n, rate]
    return n * rate

--- whether a name is on the list
fn listed[name, names, strict]
    if strict
        return name in names
    return lower[name] in names

struct Point [x, y]

--- how far a point is from home
fn far[p]
    return p.x + p.y

--- the weather in a city
fn weather[city]
    return 'sunny in `upper[city]`'

m = reach['openai://127.0.0.1:1/x']
r = chat[m, ask['hi'], tools = [price, listed, far, weather]]
log[price[2, 1.5]]
log[listed['a', ['a'], true]]
log[far[Point[1.0, 2.0]]]
log[r.ok]
if r.ok
    log[weather[r.calls[0].args.city]]
";
        crate::note_fn_docs("", src);
        let ir = ir_of(src);
        assert!(ir.contains("List(Fns(price, listed, far, weather))"), "a group: {}", ir);
        assert!(ir.contains(r#"{\"name\": \"price\", \"doc\": \"the price of n units at a rate\", \"params\": [{\"name\": \"n\", \"takes\": \"an int\", \"default\": null}, {\"name\": \"rate\", \"takes\": \"a number\", \"default\": null}], \"answers\": \"a number\"}"#), "{}", ir);
        assert!(ir.contains(r#"{\"name\": \"listed\", \"doc\": \"whether a name is on the list\", \"params\": [{\"name\": \"name\", \"takes\": \"text\", \"default\": null}, {\"name\": \"names\", \"takes\": \"a list of text\", \"default\": null}, {\"name\": \"strict\", \"takes\": \"a bool\", \"default\": null}], \"answers\": \"a bool\"}"#), "{}", ir);
        assert!(ir.contains(r#"{\"name\": \"far\", \"doc\": \"how far a point is from home\", \"params\": [{\"name\": \"p\", \"takes\": \"a record\", \"default\": null}], \"answers\": \"a number\"}"#), "{}", ir);
        // (handed a parsed value, read as text by upper[])
        assert!(ir.contains(r#"{\"name\": \"weather\", \"doc\": \"the weather in a city\", \"params\": [{\"name\": \"city\", \"takes\": \"text\", \"default\": null}], \"answers\": \"text\"}"#), "{}", ir);
    }

    /// A comment alone on its line opens and closes no block, whatever its
    /// indentation (spec, Blocks), and the comment written above a function
    /// is still its description: `describe`'s `doc`, the sentence a tool's
    /// schema quotes.
    #[test]
    fn a_comment_at_any_indentation_leaves_a_functions_description() {
        let src = "use llm

--- the weather in a city
fn weather[city]
    sky = 'sunny in `upper[city]`'       --- the sky, as the
                                         --- model is told it
--- at the margin, inside the body
            --- and deeper than it
    return sky

m = reach['openai://127.0.0.1:1/x']
r = chat[m, ask['hi'], tools = [weather]]
  --- under nothing
log[r.ok]
if r.ok
--- before the branch's first line
    log[weather[r.calls[0].args.city]]
";
        crate::note_fn_docs("", src);
        let ir = ir_of(src);
        assert!(ir.contains(r#"{\"name\": \"weather\", \"doc\": \"the weather in a city\", \"params\": [{\"name\": \"city\", \"takes\": \"text\", \"default\": null}], \"answers\": \"text\"}"#), "{}", ir);
    }

    fn ir_of(src: &str) -> String {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let stmts = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let program = lower(&stmts, &mut ctx, "t").unwrap();
        print(&program)
    }

    /// `counts = set[counts, w, counts[w] + 1]`: the value read first, into
    /// a temporary, so the call takes the map where it lies in both
    /// backends (it had been copied at every update of an entry it held).
    #[test]
    fn an_update_from_its_own_entry_takes_the_map_where_it_lies() {
        let ir = ir_of("fn counts_of[t]\n    counts = map[]\n    for i in range[len[t]]\n        w = slice[t, i, i + 1]\n        if w in counts\n            counts = set[counts, w, counts[w] + 1]\n        else\n            counts = set[counts, w, 1]\n    return counts\nlog[counts_of['abca']]\n");
        assert!(ir.contains(": Int = proven((counts[w] add 1))"), "the value first, into a temporary (a count of a text's characters, within 64 bits): {}", ir);
        assert!(ir.contains("counts = core.set(move counts, move w, _t"), "the map taken where it lies, the key read for the last time: {}", ir);
        assert!(!ir.contains("core.set(counts,"), "{}", ir);
    }

    /// A while loop's condition runs every round: a variable only it reads
    /// is never moved there (`while i lt len[s]` had moved `s` on the first
    /// round, and the interpreter stopped on the second).
    #[test]
    fn a_while_condition_moves_nothing_it_reads() {
        let ir = ir_of("fn text_len[s]\n    n = 0\n    i = 0\n    while i lt len[s]\n        n = n + 1\n        i = i + 1\n    return n\nlog[text_len['hello']]\n");
        assert!(ir.contains("core.len_text(s)"), "{}", ir);
        assert!(!ir.contains("move s"), "{}", ir);
    }

    /// A function's last line is its value, but a handler's value goes to
    /// nobody: an effect it returns waits for the commit, as does one a
    /// helper returns to a caller that calls it as a statement; an effect
    /// whose answer is read runs when asked.
    #[test]
    fn an_effect_whose_answer_nobody_reads_waits_for_the_commit() {
        let ir = ir_of("use web, ui\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'noreply@example.com', 'secret']\nn = 0\nfn notify[m, to]\n    mail[m, to, 'Hello', 'From a helper']\nfn welcome[]\n    n = n + 1\n    mail[mailer, 'ada@example.com', 'Hello', 'Welcome, number `n`']\nfn broken[]\n    mail[mailer, 'bob@example.com', 'Lost', 'never sent']\n    x = float['no']\n    if x gt 1.0\n        n = 1\nfn asked[]\n    r = mail[mailer, 'cy@example.com', 'Asked', 'sent at once']\n    if r.ok\n        n = 10\nfn helped[]\n    notify[mailer, 'di@example.com']\nview v[]\n    return stack[p['`n`'], button['Welcome', click = welcome], button['Broken', click = broken], button['Asked', click = asked], button['Helped', click = helped]]\n");
        assert!(ir.contains("return web.mail.mail!held(load mailer, \"ada@example.com\""), "{}", ir);
        assert!(ir.contains("return web.mail.mail!held(m, move to"), "{}", ir);
        assert!(ir.contains("let r: Result(Bool) = web.mail.mail!now(load mailer"), "{}", ir);
        // read by its caller, the helper's effect answers
        let ir = ir_of("use web, ui\nuse web.mail\nmailer = smtp['h', 587, 'u', 'p']\nsent = false\nfn notify[m, to]\n    mail[m, to, 'Hello', 'x']\nfn go[]\n    r = notify[mailer, 'a@b.c']\n    sent = r.ok\nview v[]\n    return button['Go', click = go]\n");
        assert!(ir.contains("return web.mail.mail!now(m, move to"), "{}", ir);
    }

    /// A function's body lowered provisionally (F5): what it met, the
    /// environment it leaves, and the lowering's state after it, which
    /// must be as before.
    fn provisional_of(src: &str, f: &str) -> (Seen, BTreeMap<String, Ty>) {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let stmts = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let body = ctx.functions[f].body.clone();
        let params: HashSet<String> = ctx.functions[f].params.iter().map(|(p, _)| p.clone()).collect();
        let mut l = Lowerer::new(&mut ctx, false);
        l.prescan(&stmts, &HashSet::new());
        let scope = l.ctx.enter_body(f);
        l.at = Where::In(FnKind::Pure);
        l.current_fn = f.to_string();
        let seen = l.provisional(Pass::Settle, Some(params), &body);
        let lowered = l.ctx.vars.clone();
        assert_eq!((l.temp, l.pre.len(), l.scopes.len(), l.v.mute, l.pass), (0, 0, 1, 0, Pass::Final));
        assert!(l.calls.is_empty() && l.direct_effects.is_empty() && l.v.errors.is_empty() && l.seen.is_none());
        l.ctx.leave_body(scope);
        (seen, lowered)
    }

    /// Lowered to learn its types, a body leaves the environment inference
    /// reads — a variable settled across its assignments, a loop's variable
    /// left bound, a collecting loop's locals left out, a map's values taken
    /// from `set[]` and `m[k] = v` — and says what it met: the calls it
    /// makes, the values it returns, the errors it hands back.
    #[test]
    fn a_body_lowered_to_learn_its_types_settles_as_inference_walks() {
        let src = "fn half[x]\n    return x / 2.0\nfn f[n, s]\n    xs = []\n    for i in range[n]\n        xs = push[xs, i * 1.5]\n    ys = for x in xs\n        y = half[x]\n        y\n    m = map[a = 1]\n    set[m, 'b', 2]\n    m['c'] = 3\n    a, b = 1, 'x'\n    k = int[s]\n    if k gt 5\n        return half[2]\n    return len[ys] * 1.0\nlog[f[3, '7']]\n";
        let (seen, lowered) = provisional_of(src, "f");
        assert_eq!(lowered.get("xs"), Some(&Ty::List(Box::new(Ty::Float))));
        assert_eq!(lowered.get("i"), Some(&Ty::Int));
        assert!(!lowered.contains_key("y") && !lowered.contains_key("x"), "{:?}", lowered);
        assert_eq!(lowered.get("m"), Some(&Ty::Map(crate::ir::Key::Text, Box::new(Ty::Int))));
        assert_eq!((lowered.get("a"), lowered.get("b")), (Some(&Ty::Int), Some(&Ty::Str)));
        assert_eq!(seen.calls, vec![("half".to_string(), vec![Ty::Float]), ("half".to_string(), vec![Ty::Int])]);
        assert_eq!(seen.returns, vec![Ty::Float, Ty::Float]);
        assert_eq!(seen.refusals, vec![(Decision::Branch, false)]);
    }

    /// A loop body's own local, and the loop's variable, move at their last
    /// read in a round — the next round binds them afresh — in a statement
    /// loop and a collecting one; what the body reads from outside never
    /// moves inside it (spec, Scope)
    #[test]
    fn a_loop_bodys_own_names_move_at_their_last_read() {
        let ir = ir_of("fn keep[xs, tag]\n    out = []\n    for x in xs\n        y = upper[x]\n        out = push[out, y]\n        log[tag]\n    ys = for x in xs\n        z = lower[x]\n        push[[tag], z]\n    return [len[out], len[ys]]\nlog[keep[['a', 'b'], 't']]\n");
        assert!(ir.contains("out = core.push(move out, move y)"), "{}", ir);
        assert!(ir.contains("text.upper(move x)"), "{}", ir);
        assert!(ir.contains("text.lower(move x)"), "{}", ir);
        assert!(!ir.contains("move tag"), "tag is read again next round: {}", ir);
        assert!(ir.contains("    for x in xs\n"), "xs is walked again by the second loop: {}", ir);
        assert!(ir.contains("for x in move xs yield"), "the second loop's walk is its last read: {}", ir);
    }

    /// A `return` inside a loop moves what the function may move: nothing
    /// reads the name after it, though the body's next round would — the
    /// record handed back from inside a `while` had been copied at every
    /// call (Rosetta Code, Universal lambda machine: 41.8 s compiled
    /// against 1.1 s interpreted); a name the body reads again still does
    /// not move in it
    #[test]
    fn a_return_inside_a_loop_moves_what_it_hands_back() {
        let ir = ir_of("struct M [term, head]
fn run[m, c]
    k = 0
    while true
        if k lt 10
            m.term = push[m.term, k + c]
            k = k + 1
        else
            m.head = len[m.term]
            return m
fn drive[n]
    m = M[[], 0]
    r = 0
    while r lt n
        m = run[m, r]
        r = r + 1
    return m.head
log[drive[3]]
");
        assert!(ir.contains("return move m"), "{}", ir);
        assert!(ir.contains("m.term = core.push(m.term"), "{}", ir);
    }

    /// `xs = grow[xs, n]` whose call can fail (n can) takes `xs` as the
    /// call that cannot fail does, in a loop too: the good path assigns it
    /// anew and a failure leaves the function (a list grown by such calls
    /// had been copied at every call: 40,000 calls 5.2 s interpreted)
    #[test]
    fn a_call_that_can_fail_takes_the_variable_it_reassigns() {
        let ir = ir_of("fn grow[xs, n]\n    xs = push[xs, n]\n    return xs\nfn many[k]\n    n = int[str[k]]\n    xs = []\n    r = 0\n    while r lt 10\n        xs = grow[xs, n]\n        r = r + 1\n    return len[xs]\nlog[many[1]]\n");
        assert!(ir.contains("call grow(move xs, unwrap n)"), "{}", ir);
    }

    /// A counter stepped both ways in a loop, its step down under the
    /// condition that it is not 0, is held in 64 bits: the narrowed write
    /// had been taken as no sum, its range joined the other step's drift
    /// into where the sums start each round, and n-queens' `k` became a
    /// growing int (4.4× slower than Rust's).
    #[test]
    fn a_counter_stepped_both_ways_under_a_condition_stays_in_64_bits() {
        let ir = ir_of("fn f[n]\n    a = slice[[0], 0, 0]\n    i = 0\n    while i lt n + 1\n        a = push[a, -1]\n        i = i + 1\n    k = 0\n    count = 0\n    done = false\n    while not done\n        c = a[k] + 1\n        if c lt n\n            a[k] = c\n            if k eq n - 1\n                count = count + 1\n            else\n                k = k + 1\n                a[k] = -1\n        else\n            a[k] = -1\n            if k eq 0\n                done = true\n            else\n                k = k - 1\n    return count\nlog[f[5]]\n");
        assert!(ir.contains("let k: Int = 0"), "{}", ir);
        assert!(!ir.contains(": Big"), "{}", ir);
    }

    /// A recursive function that adds its own answers into a local and
    /// returns it is a sum over its calls, as one that returns the sum is:
    /// the local and the answer are held in 64 bits (they had grown each
    /// pass until held as growing ints: n-queens' count).
    #[test]
    fn a_recursive_counter_summed_in_a_local_is_a_plain_int() {
        let ir = ir_of("fn count[n]\n    if n eq 0\n        return 1\n    total = 0\n    i = 0\n    while i lt 2\n        total = total + count[n - 1]\n        i = i + 1\n    return total\nlog[count[10]]\n");
        assert!(ir.contains("pure count(n: Int) -> Int") && ir.contains("let total: Int = 0"), "{}", ir);
        assert!(!ir.contains(": Big") && !ir.contains("-> Big"), "{}", ir);
    }

    /// A value that can fail, dropped by its line, is decided on (a function
    /// hands the failure back); kept in a variable, or handed on by a
    /// collecting loop's last line, or a test's truth, it is not.
    #[test]
    fn a_failed_value_its_line_drops_is_decided_on() {
        let ir = ir_of("fn save[path]\n    write[path, 'hi']\n    return 1\nkept = write['/x/a.txt', 'a']\nnums = for s in ['1', 'x']\n    int[s]\nlog[save['/x/b.txt']]\nlog[kept.ok]\nlog[nums]\n");
        assert!(ir.contains("(dropped by its line)"), "the dropped write is decided on: {}", ir);
        assert_eq!(ir.matches("(dropped by its line)").count(), 1, "the kept write and the loop's last line are not: {}", ir);
    }

    /// A frame's column of bools where a mask is taken is the mask `c eq
    /// true` makes: a frame's index, where's, a list's index.
    #[test]
    fn a_column_of_bools_where_a_mask_is_taken_is_its_mask() {
        let ir = ir_of("use frame\nf = frame.new[n = [1, 2], on = [true, false]]\nlog[f[f.on]]\nlog[where[f.on, f.n, 0]]\n");
        // (a frame's mask is fused into the frame's own kernel, as `f[f.on
        // eq true]`'s is)
        assert!(ir.contains("core.__pick_where(f, \"c0 v0 eq\", [\"on\"], true)"), "{}", ir);
        assert!(ir.contains("frame.where((f.on eq true)"), "{}", ir);
    }

    /// A list of lists of numbers is a matrix where a tensor is taken:
    /// `stack[rows]` is the matrix's rows restacked, never math.stack of a
    /// list of tensors handed a list of lists.
    #[test]
    fn a_list_of_number_lists_stacks_as_a_matrix() {
        let ir = ir_of("use math\nxs = push[push[[], 1.5], 2.5]\nrows = push[push[[], xs], xs]\nlog[stack[rows]]\nlog[tensor[rows]]\n");
        assert!(ir.contains("math.stack_rows(") && !ir.contains("math.stack("), "{}", ir);
    }

    /// A counter a condition bounds on the side it steps toward stays
    /// within the bound, also where a branch gives it a value of its own
    /// (a brainfuck interpreter's `pc`, jumping): its step is proven.
    #[test]
    fn a_counter_under_its_bound_steps_without_a_check() {
        let ir = ir_of("fn run[ops, jump, most]\n    n = len[ops]\n    pc = 0\n    steps = 0\n    while pc lt n and steps lt most\n        op = ops[pc]\n        if op eq 5\n            pc = jump[pc]\n        pc = pc + 1\n        steps = steps + 1\n    return pc\nlog[run[[5, 1, 2], [2, 0, 1], 10]]\n");
        assert!(ir.contains("pc = proven((pc add 1))"), "{}", ir);
    }

    /// The right side of `and` runs only where the left does not decide:
    /// what it needs done first — the refusal of a failed record whose
    /// field it reads — waits under the left's decision.
    #[test]
    fn a_right_side_waits_for_the_left() {
        let ir = ir_of("struct P [x]\nfn maybe[s]\n    if s eq 'bad'\n        return error['no', 'p']\n    return P[1]\nfn guard[s]\n    r = maybe[s]\n    if r.ok and r.x gt 0\n        return 'yes'\n    return 'no'\nlog[guard['bad']]\n");
        let left = ir.find("= r.ok").unwrap_or_else(|| panic!("the left first: {}", ir));
        let refusal = ir.find("propagate r (reading x)").unwrap_or_else(|| panic!("the refusal: {}", ir));
        let decision = ir[left..refusal].find("if ");
        assert!(decision.is_some(), "the refusal waits under the left's decision: {}", ir);
    }

    /// An operator that computes both sides runs the left first: what the
    /// right side needs done ahead of the statement — the refusal of a
    /// failed value whose field it reads — comes after the left side's
    /// value is taken. `put[d, f] eq 1 and latest[d].id eq ['a', 'b']` had
    /// read the log before the put (parity 302).
    #[test]
    fn the_left_side_runs_before_the_right_sides_statements() {
        let maybe = "struct P [x]\nfn maybe[s]\n    if s eq 'bad'\n        return error['no', 'p']\n    return P[1]\nfn tally[]\n    return 1\n";
        let ir = ir_of(&format!("{}n = tally[] + maybe['ok'].x\nlog[n]\n", maybe));
        let left = ir.find("= call tally()").unwrap_or_else(|| panic!("the left into a temporary: {}", ir));
        let refusal = ir.find("propagate").unwrap_or_else(|| panic!("the refusal: {}", ir));
        assert!(left < refusal, "the left side first: {}", ir);
        // a quiet left side needs no temporary
        let ir = ir_of(&format!("{}k = 2\nn = k + maybe['ok'].x\nlog[n]\n", maybe));
        assert!(ir.contains("(k add unwrap"), "{}", ir);
        // a mask's `and` computes both sides, the left first
        let ir = ir_of("use frame\nfn table_of[s]\n    if s eq 'bad'\n        return error['no', 'p']\n    return frame.new[v = [1, 2]]\nfn tally[]\n    return 1\nm = tally[] eq 1 and table_of['ok'].v gt 0\nlog[m]\n");
        let left = ir.find("= (call tally() eq 1)").unwrap_or_else(|| panic!("the left into a temporary: {}", ir));
        let refusal = ir.find("propagate").unwrap_or_else(|| panic!("the refusal: {}", ir));
        assert!(left < refusal, "the left side first: {}", ir);
        assert!(ir.contains("let m: List(Bool)"), "a mask: {}", ir);
    }

    /// A column compared with one value is a mask; with a list, a column
    /// or a tensor it is one truth, as two lists compare — so `true and
    /// df.id eq ['a', 'b']` decides from the left and builds compiled.
    #[test]
    fn a_column_compared_with_a_list_is_one_truth() {
        let ir = ir_of("use frame\ndf = frame.new[id = ['a', 'b'], v = [1, 2]]\na = df.id eq 'a'\nb = df.id eq ['a', 'b']\nc = df.v eq df.v\nd = df.v eq [1, 2]\ne = ['a', 'b'] eq ['a', 'b']\nf = (df.v gt 1) eq (df.v gt 1)\ng = true and df.id eq ['a', 'b']\nh = (df.v gt 1) eq true\nlog[a]\nlog[b]\nlog[c]\nlog[d]\nlog[e]\nlog[f]\nlog[g]\nlog[h]\n");
        for v in ["a", "h"] {
            assert!(ir.contains(&format!("let {}: List(Bool)", v)), "{} is a mask: {}", v, ir);
        }
        for v in ["b", "c", "d", "e", "f", "g"] {
            assert!(ir.contains(&format!("let {}: Bool", v)), "{} is one truth: {}", v, ir);
        }
    }

    #[test]
    fn a_collecting_loop_moves_its_list_into_push() {
        let ir = ir_of("xs = [1]\ni = 0\nwhile i lt 3\n    xs = push[xs, i]\n    i = i + 1\nlog[xs]\n");
        // (whole numbers written down and grown are a list of ints: the IR
        // names the list's row)
        assert!(ir.contains("xs = core.push(move xs, i)"), "{}", ir);
        assert!(ir.contains("log!held(move xs)"), "{}", ir);
        assert!(ir.contains("i = proven((i add 1))"), "{}", ir);
    }

    #[test]
    fn a_fact_and_a_wired_handler_lower_to_store_and_handler() {
        let ir = ir_of("use web, ui\ncount = 0\nfn inc[]\n    count = count + 1\nview render[]\n    return button['+', click = inc]\n");
        assert!(ir.contains("fact count: Int = 0            session"), "{}", ir);
        assert!(ir.contains("handler inc() -> Unit"), "{}", ir);
        assert!(ir.contains("store count ← (load count add 1)"), "{}", ir);
        assert!(ir.contains("view render() -> Node"), "{}", ir);
        assert!(ir.contains("tree button(click: fn inc)[\"+\"]"), "{}", ir);
    }

    /// What the lowering refuses of a program.
    fn refusal_of(src: &str) -> String {
        let stmts = crate::lower::lower_program(&crate::build(src).expect("parses")).expect("lowers to the tree");
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        lower(&stmts, &mut ctx, "t").err().unwrap_or_default()
    }

    /// An element-wise chain of tensors whose operands only read is one call
    /// of the fused kernel (crow_runtime::fused); a single operation, a chain
    /// over a call (which acts where the program calls it), and the matrix
    /// product are the operators'
    #[test]
    fn an_elementwise_chain_is_one_call() {
        let ir = ir_of("x = [1.0, 2.0]\ny = x * 2.0 + x .* x - 1.0\nlog[y]\n");
        // (a variable read three times is one operand)
        assert!(ir.contains("tensor.fused(\"#0 #1 mul #0 #0 elemmul add #2 sub\""), "{}", ir);
        let one = ir_of("x = [1.0, 2.0]\ny = x * 2.0\nlog[y]\n");
        assert!(!one.contains("tensor.fused"), "one operation: {}", one);
        let call = ir_of("use math\ny = zeros[2] * 2.0 + 1.0\nlog[y]\n");
        assert!(!call.contains("tensor.fused"), "a chain over a call: {}", call);
        let product = ir_of("m = [[1.0, 2.0], [3.0, 4.0]]\ny = m * m + 1.0\nlog[y]\n");
        assert!(!product.contains("tensor.fused"), "the matrix product: {}", product);
        let inner = ir_of("m = [[1.0, 2.0], [3.0, 4.0]]\ny = m * (m .* 2.0 - 1.0)\nlog[y]\n");
        assert!(inner.contains("tensor.fused(\"#0 #1 elemmul #2 sub\""), "a chain inside a product: {}", inner);
    }

    /// A frame's mask that compares the frame's own columns with values
    /// that only read, asks a column's `.ok`, and combines them with `and`,
    /// `or` and `not` is one call of the fused kernel
    /// (crow_runtime::mask::pick_where): a column read twice is one, a
    /// value written first turns the operator round. A value that calls
    /// something, another frame's column, and a mask held in a variable
    /// are the operators'
    #[test]
    fn a_frames_mask_is_one_call() {
        let df = "use frame\ndf = frame.new[name = ['ada', 'bob'], age = [30, 50]]\n";
        let ir = ir_of(&format!("{}k = 40\nx = df[df.age gt 40 and df.name ne 'bob' or not (k lt df.age) and df.age.ok]\nlog[len[x]]\n", df));
        assert!(ir.contains("core.__pick_where(df, \"c0 v0 gt c1 v1 ne and c0 v2 gt not c0 ok and or\", [\"age\", \"name\"], 40, \"bob\", k)"), "{}", ir);
        let call = ir_of(&format!("{}fn limit[]\n    40\nx = df[df.age gt limit[]]\nlog[len[x]]\n", df));
        assert!(!call.contains("__pick_where"), "a value that calls: {}", call);
        let other = ir_of(&format!("{}other = df\nx = df[other.age gt 40]\nlog[len[x]]\n", df));
        assert!(!other.contains("__pick_where"), "another frame's column: {}", other);
        let held = ir_of(&format!("{}m = df.age gt 40\nx = df[m]\nlog[len[x]]\n", df));
        assert!(!held.contains("__pick_where"), "a mask in a variable: {}", held);
    }

    /// Values of two kinds are not compared, and an order is asked only of
    /// numbers or of text: where the lowering knows both kinds the
    /// comparison is refused before the program runs, in the words the
    /// runtime stops with (crow_runtime::mask::incomparable) — a number with
    /// text, a bool with either, bools ordered, a column of text with a
    /// number, two lists of two kinds; a parsed value is read as one type
    /// before an order; an item put in or looked for in a collection of
    /// another kind; a list of values that can fail as a mask; arithmetic on
    /// a list of text
    #[test]
    fn values_of_two_kinds_are_not_compared() {
        let refused = |src: &str, words: &str| {
            let r = refusal_of(src);
            assert!(r.contains(words), "{}\nrefused with: {}", src, r);
        };
        refused("log['a' eq 1]\n", "eq compares values of one kind, not text with a number");
        refused("log[1 ne 'a']\n", "ne compares values of one kind, not a number with text");
        refused("log[true eq 1]\n", "eq compares values of one kind, not a bool with a number");
        // a moment compares with a moment, adds or subtracts seconds, and subtracts a moment
        refused("log[now[] gt 5.0]\n", "gt compares numbers with numbers or text with text, not a moment with a number");
        refused("log[now[] eq 'x']\n", "eq compares values of one kind, not a moment with text");
        refused("log[now[] * 2]\n", "a moment adds or subtracts seconds (t + 60), subtracts a moment (t2 - t1, the seconds between), and compares with a moment");
        assert_eq!(refusal_of("t = now[]\nlog[t gt t - 5]\nlog[(t - 60) - t]\nlog[t + 1.5]\nlog[min[t, t + 1] eq t]\n"), "");
        // (a list of numbers beside one number is a mask, so a condition
        // of it is refused as a tensor's is: it had run interpreted and
        // not built compiled)
        refused("ys = []\nys = push[ys, 1.0]\nif 3 gte ys\n    log[1]\n", "this is a list of bools, and a condition is true or false");
        refused("log[true gt false]\n", "gt compares numbers with numbers or text with text, not a bool with a bool");
        refused("log['a' lte 1]\n", "lte compares numbers with numbers or text with text, not text with a number");
        let df = "use frame\ndf = frame.new[name = ['ada', 'bob']]\n";
        refused(&format!("{}log[df.name gt 2]\n", df), "gt compares numbers with numbers or text with text, not text with a number");
        refused(&format!("{}log[df.name eq 2]\n", df), "eq compares values of one kind, not text with a number");
        refused(&format!("{}log[2 lt df.name]\n", df), "lt compares numbers with numbers or text with text, not a number with text");
        refused("log[['a'] gt ['b']]\n", "gt compares numbers with numbers or text with text, not a list of text with a list of text");
        refused("log[['a'] eq [1]]\n", "eq compares values of one kind, not a list of text with a list of numbers");
        refused("xs = ['1', '2']\nlog[xs + 1]\n", "cannot use '+' with array[str] and int");
        refused("a = float['1.5']\nxs = [a, a]\nlog[xs eq 1.5]\n", "eq of a list of values that can fail is a mask, and a mask decides every row: guard each value with .ok");
        refused("log[1 in ['a']]\n", "`x in xs` takes a list and an item of its kind: its items are text, not a number");
        refused("xs = push[['a'], 1]\nlog[xs]\n", "push[] takes a list and an item of its kind: its items are text, not a number");
        // (one mistake, one message: the map's own check, not the holder's too)
        refused("m = set[map[a = 'x'], 'b', 1]\nlog[m]\n", "a map holds values of one kind: the entry 'b' holds an int, and the map's other values are text");
        // of one kind, they are compared: text ordered, an int beside a
        // float, two lists whole, a parsed value as the program runs
        assert_eq!(refusal_of(&format!("{}log['b' gt 'a']\nlog[df.name gte 'b']\nlog[1 eq 1.0]\nlog[['a'] eq ['a']]\nj = json['1']\nlog[j eq 'x']\na = float['1.5']\nlog[[a] eq [a]]\nlog['b' in ['a']]\n", df)), "");
    }

    /// A group by several keys keeps each key's type; a pivot's rows column
    /// is known and its others show as it runs; a window's column is typed
    /// by what it computes — a float, an int, the source's cells that can
    /// be missing for a lag — and a column it names that the frame lacks
    /// is refused, as a key the frame lacks is
    #[test]
    fn a_group_by_keys_a_pivot_and_a_window_are_typed() {
        let t = "use frame\nt = frame.new[sym = ['b', 'a'], day = [2, 1], px = [1.5, 2.0]]\n";
        let ir = ir_of(&format!("{}g = group[t, ['sym', 'day'], ['px', 'max']]\nlog[g.day[0]]\np = pivot[t, 'sym', 'day', 'px']\nlog[p.sym[0]]\nw = t\nw.prev = window[t, 'px', 'lag', n = 1, by = 'sym']\nlog[w.prev[0].ok]\nr = t\nr.r = window[t, 'px', 'rank']\nlog[r.r[0]]\nm = t\nm.m = window[t, 'px', 'mean', n = 2]\nlog[m.m[0]]\n", t));
        // (`[2, 1]` is a column of ints, so the key is one; the max over floats a float)
        assert!(ir.contains("let g: Frame{day: Int, px_max: Float, sym: Str} = frame.group_keys("), "{}", ir);
        assert!(ir.contains("let p: Frame{sym: Str, …} = frame.pivot("), "{}", ir);
        assert!(ir.contains("prev: Result(Float)") && ir.contains("frame.window_one(t, \"px\", \"lag\", 1, \"sym\", 0.0, \"\")"), "{}", ir);
        assert!(ir.contains("r: Int") && ir.contains("frame.window(t, \"px\", \"rank\", 0, [], 0.0, \"\")"), "{}", ir);
        assert!(ir.contains("m: Float"), "{}", ir);
        assert!(refusal_of(&format!("{}log[len[group[t, ['sym', 'nope'], ['px', 'max']]]]\n", t)).contains("group: column 'nope' not found — the frame's columns are 'day', 'px', 'sym'"));
        assert!(refusal_of(&format!("{}log[len[pivot[t, 'sym', 'nope', 'px']]]\n", t)).contains("pivot: column 'nope' not found"));
        assert!(refusal_of(&format!("{}log[window[t, 'nope', 'sum']]\n", t)).contains("window: column 'nope' not found"));
        assert!(refusal_of(&format!("{}log[window[t, 'px', 'sum', by = 'nope']]\n", t)).contains("window: column 'nope' not found"));
        // (an option it does not have, and a count it does not take, are
        // refused as the call is read, before the lowering)
        let front = |src: &str| crate::lower::lower_program(&crate::build(src).expect("parses")).err().map(|e| format!("{}", e)).unwrap_or_default();
        assert!(front(&format!("{}log[window[t, 'px', 'sum', rows = 2]]\n", t)).contains("window[] has no option 'rows' — its options are n, by, span, on"));
        assert!(front(&format!("{}log[window[t, 'px']]\n", t)).contains("window[] takes 3 arguments, then n = 0, by = [], span = 0.0, on = '' by name"));
    }

    /// A function named like an element (`h3`, `p`) is a call outside a
    /// view: handed a value that can fail for a plain parameter it is typed
    /// as able to fail, as any call is — auto-propagation had skipped it as
    /// the element, so `.ok` of its answer was decided true and the compiled
    /// program did not build (found by the fuzzer, whose helpers are h1, h2)
    #[test]
    fn a_function_named_like_an_element_hands_a_failure_on() {
        let ir = ir_of("fn h3[x]\n    return x + 1\nt = float['x']\nw = h3[t]\nlog[w.ok]\nu = h3[float['y']]\nlog[u]\n");
        assert!(ir.contains("let w: Result(Float) = call h3(unwrap t)"), "{}", ir);
        assert!(ir.contains("display.log!held(w.ok)"), "asked, not decided: {}", ir);
        assert!(ir.contains("let u: Result(Float) = call h3(unwrap core.float_of(\"y\"))"), "{}", ir);
    }

    /// A call whose argument's kind shows only as the program runs is typed
    /// by what every row of its name answers, else not known: `abs` of a
    /// map's item nothing typed is no int (the first row's answer had been
    /// taken, and a float came); `stack` of a list of rows, and `tensor` of
    /// a list, state their shapes, so a stacked matrix's columns walk as
    /// rows; a list of rows is a matrix where a tensor is taken
    #[test]
    fn a_call_on_an_unknown_value_is_typed_by_what_its_rows_agree_on() {
        let ir = ir_of("d = map[]\nv = abs[d['x']]\nlog[abs[v - 1.0]]\nlog[len[d['x']]]\n");
        assert!(ir.contains("let v: ? = abs("), "{}", ir);
        assert!(!ir.contains("core.abs_int"), "no row picked before the value comes: {}", ir);
        assert!(ir.contains("len("), "a len of an unknown value is left to its values, typed as every len row answers: {}", ir);
        let ir = ir_of("use math\nm = [[1.0, 2.0], [3.0, 4.0]]\nrows = []\nfor r in m\n    rows = push[rows, r * 2.0]\ns = stack[rows]\nt = transpose[rows]\ncols = transpose[s]\nmeans = for c in cols\n    mean[c]\nlog[means]\nv = tensor[[1.0, 2.0]]\nlog[v]\n");
        assert!(ir.contains("let s: Tensor(Float, [?, 2]) = math.stack("), "{}", ir);
        assert!(ir.contains("let t: Tensor(Float, [2, ?]) = math.transpose(tensor(move rows))") || ir.contains("math.transpose(tensor(rows))"), "{}", ir);
        assert!(ir.contains("let means: Tensor(Float) = ") || ir.contains("let means: List(Float)"), "{}", ir);
        assert!(ir.contains("let v: Tensor(Float, [2])") || ir.contains("let v: Tensor(Float, [?])"), "{}", ir);
    }

    /// A status the type settles is decided before the program runs: `.ok`
    /// of a value that cannot fail is true and its reason empty
    /// (typed-values decision 1); one that can fail, and a parsed value's,
    /// are asked as it runs; one read through a call or an index keeps the
    /// read, which acts or stops
    #[test]
    fn a_status_its_type_settles_is_decided() {
        let ir = ir_of("fn loud[x]\n    log['called']\n    return x\nn = 3\nlog[n.ok]\nlog[n.reason]\nr = float['x']\nlog[r.ok]\nj = json['[1]']\nlog[j[0].ok]\nlog[loud[4].ok]\nxs = [1, 2]\nlog[xs[1].ok]\n");
        assert!(ir.contains("display.log!held(true)"), "{}", ir);
        assert!(ir.contains("display.log!held(\"\")"), "{}", ir);
        assert!(ir.contains("display.log!held(r.ok)"), "{}", ir);
        assert!(ir.contains("[0].ok)"), "a parsed value's status is asked: {}", ir);
        assert!(ir.contains("4).ok)") && ir.contains("[1].ok)"), "what acts or stops keeps its read: {}", ir);
    }

    /// A module asked for what it does not have is told so: `io.read` is
    /// the core's `read` (it had said that io was no module or variable,
    /// with `use io` written above it)
    #[test]
    fn a_module_without_the_name_says_so() {
        assert_eq!(refusal_of("use io\nt = io.read['notes.txt']\nlog[t]\n"), "line 2: io has no 'read' — read is the core's: read[…]");
        assert_eq!(refusal_of("use io\nt = io.slurp['notes.txt']\nlog[t]\n"), "line 2: io has no 'slurp'");
        assert_eq!(refusal_of("t = nowhere.read['notes.txt']\nlog[t]\n"), "line 1: undefined module or variable 'nowhere'");
    }

    /// A page's address holds facts (spec, Core (Server)): a path's `:name`
    /// or a `url` name that names none is refused before the program runs,
    /// in both modes (the compiled program had not built, and `run -i` had
    /// stopped as it served); a part's fact is named by its last name.
    #[test]
    fn a_page_s_address_names_facts() {
        let src = "use web, ui\ncard = 1\nview v[]\n    return stack[title['Card `card`'], p['`card`']]\nserve[3999]\npage['/card/:card', v]\npage['/x/:nope', v]\npage['/', v, url = ['gone']]\n";
        let refused = refusal_of(src);
        assert!(refused.contains("page['/x/:nope', …]: ':nope' names no fact — a page's path holds facts, as url = […] does"), "{}", refused);
        assert!(refused.contains("page[…, url = …]: no reactive fact named 'gone'"), "{}", refused);
        assert!(!refused.contains("':card'"), "{}", refused);
        assert_eq!(refusal_of("use web, ui\ncard = 1\nview v[]\n    return p['`card`']\nserve[3999]\npage['/card/:card', v]\n"), "");
    }

    /// The tables a program declares on each connection: every literal
    /// `CREATE TABLE` handed to it, directly or through the functions the
    /// connection is handed to (the task board's `setup[book]`), but one the
    /// program drops there too, remaking it each run; a connection handed
    /// none declares nothing, and its `connect` is as written.
    #[test]
    fn a_program_declares_the_tables_it_creates_on_each_connection() {
        let src = "use db\nbook = db.connect['sqlite://app.db']\nother = db.connect['sqlite://:memory:']\nfn setup[c]\n    query[c, 'CREATE TABLE IF NOT EXISTS people (name TEXT PRIMARY KEY)']\n    query[c, 'INSERT INTO people VALUES (?)', ['ada']]\nfn more[c]\n    setup[c]\n    query[c, 'create table if not exists boards (id INTEGER PRIMARY KEY)']\n    query[c, 'DROP TABLE IF EXISTS scratch']\n    query[c, 'CREATE TABLE scratch (n INTEGER)']\nmore[book]\nlog[query[other, 'SELECT 1 AS one']]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).expect("parses")).expect("lowers to the tree");
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let program = lower(&stmts, &mut ctx, "t").expect("lowers");
        let declared = schema::declared(&program);
        let people = "CREATE TABLE IF NOT EXISTS people (name TEXT PRIMARY KEY)".to_string();
        let boards = "create table if not exists boards (id INTEGER PRIMARY KEY)".to_string();
        assert_eq!(declared.get("book").cloned().unwrap_or_default(), vec![people.clone(), boards.clone()], "{:?}", declared);
        assert!(!declared.contains_key("other"), "{:?}", declared);
        assert_eq!(schema::plan(&program), vec![("book".to_string(), Some("sqlite://app.db".to_string()), vec![people, boards])]);
        // each connection opens with what is declared on it
        let ir = crate::ir::print(&program);
        assert!(ir.contains("db.connect!now(\"sqlite://app.db\", 0, [\"CREATE TABLE IF NOT EXISTS people (name TEXT PRIMARY KEY)\", \"create table if not exists boards"), "{}", ir);
        assert!(ir.contains("db.connect!now(\"sqlite://:memory:\")"), "{}", ir);
    }

    /// A frame's type is its columns (second-half-design.md §2, columns in
    /// the type), and a reassignment binds it anew: in the block it was
    /// bound in, a new `let` of the frame the value is, whose columns every
    /// later read has.
    #[test]
    fn a_frame_reassigned_is_bound_anew() {
        let ir = ir_of("use frame\ndf = frame.new[a = [1.5]]\ndf.b = ['x']\nlog[df.b[0]]\n");
        assert!(ir.contains("let df: Frame{a: Float} = core.__set_col(frame.new()"), "{}", ir);
        assert!(ir.contains("let df: Frame{a: Float, b: Str} = core.__set_col(df, \"b\""), "{}", ir);
    }

    /// A column written by the dot (spec, Frames) is the frame assigned
    /// anew through the column write's row: a name the program writes adds
    /// the column to the type; a name with a written piece, or one wholly
    /// computed, is read as the program runs and leaves the frame open; a
    /// computed read is the read's row, its cells as they show; through a
    /// record's field the frame is read and written back whole; and a
    /// scalar repeats down the rows.
    #[test]
    fn a_column_written_by_the_dot_is_the_frame_assigned_anew() {
        let ir = ir_of("use frame\ndf = frame.new[a = [1.5]]\ndf.side = 'buy'\nk = 1\ndf.lag_`k` = [2.0]\nlog[df.`'a'`[0]]\n");
        assert!(ir.contains("let df: Frame{a: Float, side: Str} = core.__set_col(df, \"side\", \"buy\")"), "{}", ir);
        assert!(ir.contains("let df: Frame{a: Float, side: Str, …} = core.__set_col(df, interp(\"lag_\", k), [2.0])"), "{}", ir);
        assert!(ir.contains("core.__col(df, interp(\"a\"))"), "{}", ir);
        let ir = ir_of("use frame\nstruct B [f]\nb = B[frame.new[px = [1.0]]]\nb.f.d = b.f.px\nlog[b.f.d[0]]\n");
        assert!(ir.contains("b.f = core.__set_col(b.f, \"d\", b.f.px)"), "{}", ir);
    }

    /// A computed name is a frame's alone, and a field written on what
    /// has no fields is refused before the program runs, in both modes
    #[test]
    fn a_computed_column_name_is_a_frames_alone() {
        let e = refusal_of("struct P [x]\np = P[1]\np.`'x'` = 3\nlog[p.x]\n");
        assert!(e.contains("names a frame's column by a computed name, and this is a record"), "{}", e);
        let e = refusal_of("struct P [x]\np = P[1]\nlog[p.`'x'`]\n");
        assert!(e.contains("names a frame's column by a computed name, and 'p' is a record"), "{}", e);
        let e = refusal_of("xs = ['a']\nxs.k = [3]\nlog[xs]\n");
        assert!(e.contains("a list has no field 'k'"), "{}", e);
    }

    /// A tensor local reassigned another shape is bound anew in the block
    /// it was bound in, as a frame given other columns is; where branches
    /// meet it has the dimensions they agree on; and a function's return
    /// has the shape its returns agree on.
    #[test]
    fn a_tensor_local_is_bound_anew_to_its_shape() {
        let ir = ir_of("use math\nx = zeros[2, 3]\nx = zeros[3, 3]\nlog[x]\n");
        assert!(ir.contains("let x: Tensor(Float, [2, 3]) = math.zeros(2, 3)"), "{}", ir);
        assert!(ir.contains("let x: Tensor(Float, [3, 3]) = math.zeros(3, 3)"), "{}", ir);
        let ir = ir_of("use math\nfn pick[k]\n    a = zeros[2, 3]\n    if k gt 1\n        a = zeros[2, 4]\n    return a\nlog[pick[2]]\n");
        assert!(ir.contains("pick(k: Int) -> Tensor(Float, [2, ?])"), "{}", ir);
    }

    /// A route's handler is handed the request: its parameter is typed so,
    /// the request's params, query and headers being maps of text and its
    /// method, path and body text (untyped, the compiled program had read
    /// `req.params.name` as a record's field and not built)
    #[test]
    fn a_routes_handler_is_handed_the_request() {
        let ir = ir_of("use web\nfn hello[req]\n    return text['hello `req.params.name` `req.body`']\nroute['/hello/:name', hello]\nroute['/made', 'POST', hello]\nserve[3999]\n");
        assert!(ir.contains("pure hello(req: Handle(web.request)) -> Handle(response)"), "{}", ir);
    }

    /// stream[feed, a, …] calls feed[put, a, …]: put is a function of text
    /// that answers whether the reader still listens, the rest are typed by
    /// what the call hands, and the feed's loop keeps its order — a call
    /// through put sends a chunk (untyped, put's loop had been marked
    /// parallel, its chunks going out in any order); the responses are
    /// responses
    #[test]
    fn a_streams_feed_is_handed_put_and_keeps_its_order() {
        let ir = ir_of("use web\nfn lines[put, n]\n    for i in range[n]\n        put['line `i`']\nfn flow[req]\n    return stream[lines, 3]\nfn packed[req]\n    return msgpack[map[a = 1]]\nroute['/flow', flow]\nroute['/packed', packed]\nserve[3999]\n");
        assert!(ir.contains("pure lines(put: Fn(Str) -> Bool, n: Int) -> Unit"), "{}", ir);
        assert!(ir.contains("for i in core.range_to(n)\n"), "{}", ir);
        assert!(!ir.contains("parallel"), "{}", ir);
        assert!(ir.contains("pure flow(req: Handle(web.request)) -> Handle(response)"), "{}", ir);
        assert!(ir.contains("pure packed(req: Handle(web.request)) -> Handle(response)"), "{}", ir);
    }

    /// What stream is handed is checked before either mode runs: a function
    /// of the program taking put and then what the call hands it, whose put
    /// is handed text; and the responses are web's (the interpreter had
    /// answered msgpack, arrow, bytes and stream with a map that only looked
    /// like a response)
    #[test]
    fn a_stream_and_the_responses_are_checked() {
        let refused = refusal_of("use web\nfn lines[put]\n    put['a']\nfn flow[req]\n    return stream[lines, 3]\nroute['/flow', flow]\nserve[3999]\n");
        assert!(refused.contains("stream[] calls lines[put, …], and lines takes 1 parameter: its first is put, and the rest are what stream[lines, …] hands it"), "{}", refused);
        let refused = refusal_of("use web\nfn lines[put]\n    put[42]\nfn flow[req]\n    return stream[lines]\nroute['/flow', flow]\nserve[3999]\n");
        assert!(refused.contains("put[] takes text, and this is int: write it into text, put['`…`']"), "{}", refused);
        let refused = refusal_of("use web\nx = 3\nfn flow[req]\n    return stream[x]\nroute['/flow', flow]\nserve[3999]\n");
        assert!(refused.contains("stream[] takes a function of the program, which it calls with put"), "{}", refused);
        assert!(refusal_of("x = msgpack[map[a = 1]]\n").contains("msgpack[] is web's: add `use web`"));
        assert!(refusal_of("x = respond[200, 'ok']\n").contains("respond[] is web's: add `use web`"));
        // a program's own function of that name is its own
        assert_eq!(refusal_of("fn stream[x]\n    return x + 1\nlog[stream[2]]\n"), "");
        assert_eq!(refusal_of("use web\nfn lines[put, n]\n    put['`n`']\nfn flow[req]\n    return stream[lines, 3]\nroute['/flow', flow]\nserve[3999]\n"), "");
    }

    /// A tensor whose shape is in its type is walked and indexed by its
    /// rows, as a literal's nested rows are: `transpose[m]`'s items and
    /// `zeros[2, 3][1]` are rows (they had been numbers, and the compiled
    /// program walked the numbers)
    #[test]
    fn a_shaped_matrix_is_walked_by_its_rows() {
        let ir = ir_of("use math\nm = [[1.0, 2.0], [3.0, 4.0]]\nfor c in transpose[m]\n    log[mean[c]]\nz = zeros[2, 3]\nr = z[1]\nlog[r]\n");
        assert!(ir.contains("math.mean(move c)"), "{}", ir);
        assert!(ir.contains("let r: Tensor(Float, [3]) = z[1]"), "{}", ir);
    }

    /// A variable bound again after its name's block has ended is a new
    /// variable, and the IR names it as the program wrote it (S11): it had
    /// said `t__s2`, the front end's name for the passes that key a type by
    /// a name
    #[test]
    fn a_blocks_variable_keeps_its_written_name() {
        let ir = ir_of("fn f[c]\n    if c\n        t = 1\n        log[t]\n    for i in range[2]\n        t = 'x'\n        log[t]\n    return 0\nlog[f[true]]\n");
        assert!(ir.contains("let t: Int = 1"), "{}", ir);
        assert!(ir.contains("let t: Str = \"x\""), "{}", ir);
        assert!(!ir.contains("__s"), "{}", ir);
    }

    /// A function handed to grad is a function of floats, however grad is
    /// written (`grad[f][3]` had left its parameters untyped where
    /// `df = grad[f]` did not), and its body's builtins are the ones for
    /// floats; a direct call's int is converted
    #[test]
    fn a_function_grad_differentiates_takes_floats() {
        let ir = ir_of("fn root[x]\n    sqrt[x]\nfn sq[x]\n    x * x\nlog[grad[root][4]]\ndf = grad[sq]\nlog[df[3]]\nlog[sq[2]]\n");
        assert!(ir.contains("pure root(x: Float) -> Float"), "{}", ir);
        assert!(ir.contains("return core.sqrt(x)"), "{}", ir);
        assert!(ir.contains("pure sq(x: Float) -> Float"), "{}", ir);
        assert!(ir.contains("call sq(float(2))"), "{}", ir);
    }

    /// A function `grad` differentiates over tensors is typed by the calls
    /// of its gradient: its parameters the tensors handed, its answer a
    /// number; the gradient of one parameter is a tensor, of several a list
    /// of tensors; and the function and the ones it reaches are traced —
    /// their chains unfused, since a fused kernel is opaque to the tape
    #[test]
    fn a_function_grad_differentiates_over_tensors() {
        let ir = ir_of("use math, math.nn\nfn loss[w, x, y]\n    return mse[flatten[x * w], y]\nfn helper[w]\n    return sum[w .* w + w]\nfn outer[w]\n    return helper[w] * 2.0\ng = grad[loss]\ngs = g[zeros[2, 1], [[1.0, 0.0], [0.0, 1.0]], [1.0, 2.0]]\nlog[gs[0].shape]\nh = grad[outer][[1.0, 2.0]]\nlog[h.shape]\n");
        assert!(ir.contains("loss(w: Tensor(Float") && ir.contains(") -> Float"), "{}", ir);
        assert!(ir.contains("helper(w: Tensor(Float"), "{}", ir);
        assert!(ir.contains("let gs: List(Tensor(Float))"), "{}", ir);
        assert!(ir.contains("let h: Tensor(Float, [2])"), "{}", ir);
        assert!(!ir.contains("fused"), "{}", ir);
    }

    /// A function handed a matrix literal by one call, a tensor of unknown
    /// shape by another and a list of rows grown by `push` by a third keeps
    /// the rank the literal states: its rows are rows (the join had dropped
    /// to unknown, and `len[x[0]]` was refused as the length of a number)
    #[test]
    fn a_parameter_keeps_the_rank_one_call_states() {
        let ir = ir_of("use math\nstruct S[xs]\nfn first[x]\n    return len[x[0]]\ns = S[zeros[2, 2]]\nrows = []\nrows = push[rows, [1.0, 2.0]]\nlog[first[[[1.0, 2.0], [3.0, 4.0]]]]\nlog[first[s.xs]]\nlog[first[rows]]\n");
        assert!(ir.contains("first(x: Tensor(Float, [?, 2])) -> Int") || ir.contains("first(x: Tensor(Float, [?, ?])) -> Int"), "{}", ir);
    }

    /// A fact keeps the form it was declared in — a tensor, `ep_start =
    /// [0.0]` — through a handler's assignments of a list: the list is
    /// converted where it is stored, and the fact is a list where a record's
    /// list field takes it (the join had turned the fact into the value's
    /// form while the compiled state kept the declared one: finch's clock
    /// did not build)
    #[test]
    fn a_fact_keeps_its_declared_form() {
        let ir = ir_of("use web, ui\nstruct Episodes [symbol, start, end]\nfn grown[e]\n    s = []\n    s = push[s, 1.5]\n    return Episodes[e.symbol, s, s]\nsym = 'a'\nep_start = [0.0]\nep_end = [0.0]\nfn step[]\n    act = Episodes[sym, ep_start, ep_end]\n    act = grown[act]\n    sym, ep_start, ep_end = act.symbol, act.start, act.end\nview home[]\n    return stack[p['`sym` `len[ep_start]`'], button['step', click = step]]\nserve[3402]\npage['/', home]\n");
        assert!(ir.contains("fact ep_start: Tensor(Float, [1])"), "{}", ir);
        assert!(ir.contains("Episodes{load sym, list(load ep_start), list(load ep_end)}"), "{}", ir);
        assert!(ir.contains("store ep_start ← tensor(act.start)"), "{}", ir);
    }

    /// A tensor that can fail pushed to is a tensor (that can fail): its
    /// holder is read by what it holds (it had been typed a list)
    #[test]
    fn a_failing_tensor_pushed_to_stays_a_tensor() {
        let ir = ir_of("use frame\ndf = frame.new[a = int['1']]\nv = push[push[[3, 1], len[frame.stack[df, df]]], 10]\nlog[v]\n");
        assert!(ir.contains("let v: Result(Tensor(Int"), "{}", ir);
    }

    /// `%` takes ints: a float on either side is refused before the program
    /// runs (the interpreter had stopped on it, the compiled program went on)
    #[test]
    fn a_remainder_of_a_float_is_refused() {
        let refused = refusal_of("x = 2.5\nlog[x % 3]\nlog[7 % 2.0]\n");
        assert!(refused.contains("line 2: `%` takes ints and answers the remainder"), "{}", refused);
        assert!(refused.contains("line 3: `%` takes ints"), "{}", refused);
        let ir = ir_of("log[7 % 2]\n");
        assert!(ir.contains("(7 mod 2)"), "{}", ir);
    }

    /// A holder of ints given a float holds floats; a record's float field
    /// given an int that can fail holds a float that can fail
    #[test]
    fn a_holder_of_ints_given_a_float_holds_floats() {
        let ir = ir_of("use frame\nxs = push[range[3], 2.5]\nm = set[map[a = 1], 'b', 2.5]\nc = concat[range[2], xs]\nstruct P [x, y]\na = P[1.5, 2.0]\nb = P[int['12'], 3.0]\nlog[xs, m, c, a, b]\n");
        assert!(ir.contains("let xs: List(Float) = core.push(floats(") && ir.contains(", 2.5)"), "{}", ir);
        assert!(ir.contains("let m: Map(Float) = core.set(floats(core.set(core.map_new(), \"a\", 1)), \"b\", 2.5)"), "{}", ir);
        assert!(ir.contains("let c: List(Float) = core.concat(floats("), "{}", ir);
        assert!(ir.contains("struct P { x: Result(Float), y: Float }"), "{}", ir);
        assert!(ir.contains("P{float(core.__int_big(\"12\")), 3.0}"), "an int read from text keeps every digit: {}", ir);
    }

    /// A tensor's rank is in its type: every builtin's answer has one, so a
    /// loop over it walks rows or numbers and an index reads a row or a
    /// number as the type decides, in both modes; where the rank is not
    /// known at a loop or an index — a piece of `svd`, whose ranks differ —
    /// the program is refused with the fix; a row assigned into a tensor
    /// is refused too (the interpreter had stopped, the compiled program
    /// had not built)
    #[test]
    fn a_loop_and_an_index_go_by_the_rank_the_type_states() {
        let ir = ir_of("use math, math.nn\nstruct Net[w]\nnet = Net[zeros[2, 3] + 1.0]\nfor r in relu[net.w]\n    a = r\n    log[a]\nfor x in sum[net.w, 1]\n    b = x\n    log[b]\nfor x in flatten[net.w]\n    c = x\n    log[c]\nfor r in reshape[linspace[0.0, 5.0, 6], 3, 2]\n    d = r\n    log[d]\nrow0 = concat[net.w, net.w][0]\nfn fn_loss[w]\n    return sum[w .* w]\ng = grad[fn_loss][net.w]\nf = g[1]\nv = [1.0, 2.0]\nv[0] = 3.0\nfor x in v\n    h = x\n    log[h]\nlog[row0, f]\n");
        assert!(ir.contains("let a: Tensor(Float, [3]) = "), "{}", ir);
        assert!(ir.contains("let b: Float = "), "{}", ir);
        assert!(ir.contains("let c: Float = "), "{}", ir);
        assert!(ir.contains("let d: Tensor(Float, [2]) = "), "{}", ir);
        assert!(ir.contains("let row0: Tensor(Float, [3]) = core.concat_tensors("), "{}", ir);
        assert!(ir.contains("pure fn_loss(w: Tensor(Float, [2, 3])) -> Float"), "{}", ir);
        assert!(ir.contains("let g: Tensor(Float, [2, 3]) = grad(fn fn_loss)[net.w]"), "{}", ir);
        assert!(ir.contains("let f: Tensor(Float, [3]) = g[1]"), "{}", ir);
        assert!(ir.contains("let h: Float = "), "{}", ir);
        // (a piece read by a position the program computes has no shape)
        let refused = refusal_of("use math\nm = [[2.0, 0.0], [0.0, 3.0]]\nparts = svd[m]\ni = 0\nu = parts[i]\nfor r in u\n    log[r]\nlog[parts[i][0]]\n");
        assert!(refused.contains("line 6: a loop over `u` walks rows or numbers by its rank, and the rank of `u` is not known here — state it: reshape[t, rows, columns] for a matrix, flatten[t] for a vector"), "{}", refused);
        assert!(refused.contains("line 8: an index into the tensor reads a row or a number by its rank"), "{}", refused);
        // a row of a matrix is written as one of its own length, in place;
        // one of another length is refused before the program runs
        let ir = ir_of("use math\nm = eye[2]\nm[1] = [7.0, 8.0]\nlog[m]\n");
        assert!(ir.contains("core.__set_part("), "{}", ir);
        let refused = refusal_of("use math\nm = eye[2]\nm[1] = [7.0, 8.0, 9.0]\nlog[m]\n");
        assert!(refused.contains("`m[…] = …`: a row here holds 2 numbers, and the value written holds 3 numbers"), "{}", refused);
        // a function handed as a value: typed however far it is handed on,
        // and refused where nothing types it
        let ir = ir_of("fn double[x]\n    return x * 2\nfn apply[f, x]\n    return f[x]\nfn relay[f, x]\n    return apply[f, x]\nlog[relay[double, 21]]\n");
        assert!(ir.contains("pure double(x: Int) -> Int"), "{}", ir);
        let refused = refusal_of("fn scaled[v]\n    return v * 1.5\nfn with_both[g, x, y]\n    held = g\n    return apply[held, x] + apply[held, y]\nfn apply[f, x]\n    return f[x]\nlog[with_both[scaled, 2.0, 4.0]]\n");
        assert!(refused.contains("`scaled` is handed as a value, and nothing types its parameters"), "{}", refused);
        // a decomposition's pieces, each its own shape by position
        let ir = ir_of("use math\nm = [[2.0, 0.0, 1.0], [0.0, 3.0, 1.0]]\nparts = svd[m]\ns = parts[1]\nvt = parts[2]\nfor x in s\n    log[x]\nfor r in vt\n    log[r]\nlog[svd[m][0][0]]\nq = qr[m]\nlog[q[0][0]]\n");
        assert!(ir.contains("let s: Tensor(Float, [?]) = parts[1]"), "{}", ir);
        assert!(ir.contains("let vt: Tensor(Float, [?, 3]) = parts[2]"), "{}", ir);
        assert!(ir.contains("let q: List(Tensor(Float, [?, ?])) = math.qr("), "{}", ir);
    }

    /// What a function `grad` differentiates over tensors may not do is
    /// refused before the program runs, with the function and the line
    #[test]
    fn a_function_grad_differentiates_over_tensors_is_checked() {
        let refuses = |src: &str, words: &str| {
            let r = refusal_of(src);
            assert!(r.contains(words), "{}\n--- wanted: {}", r, words);
        };
        refuses("use math\nfn f[w]\n    return prod[w]\nlog[grad[f][[3.0, 1.0]]]\n", "grad: f calls `prod`, which is not differentiable");
        refuses("use math\nfn f[w]\n    return w[0] * w[1]\nlog[grad[f][[3.0, 1.0]]]\n", "grad: f indexes a tensor");
        refuses("use math\nfn f[w]\n    t = 0.0\n    for x in w\n        t = t + x\n    return t\nlog[grad[f][[3.0, 1.0]]]\n", "grad: f walks a tensor");
        refuses("use math\nfn f[w, k]\n    return sum[w] * k\nlog[grad[f][[1.0, 2.0], 3.0]]\n", "a function grad differentiates takes tensors, or numbers alone — f takes a tensor, a number");
        refuses("use math\nfn inner[x]\n    return x * x\nfn outer[w]\n    return sum[w] * grad[inner][2.0]\nlog[grad[outer][[1.0]]]\n", "grad: outer calls grad");
        refuses("use math\nfn f[w]\n    n = len[w gt 0.0]\n    return sum[w] * float[n]\nlog[grad[f][[3.0, 1.0]]]\n", "grad: f compares tensors with `gt`");
    }

    /// A second derivative is not built: `grad` inside a function `grad`
    /// differentiates over numbers, or one it reaches, and `grad` handed a
    /// gradient, are refused before the program runs, in the words the
    /// tensors' refusal has (the first had answered 0 in both modes, the
    /// second stopped interpreted and did not build compiled); a function
    /// that computes a derivative, which nothing differentiates, runs
    #[test]
    fn a_second_derivative_is_refused() {
        let refuses = |src: &str, words: &str| {
            let r = refusal_of(src);
            assert!(r.contains(words), "{}\n--- wanted: {}", r, words);
        };
        refuses("fn f[x]\n    return x * x * x\nfn df[x]\n    return grad[f][x]\nlog[grad[df][2.0]]\n", "line 4: grad: df calls grad, and a function grad differentiates cannot");
        refuses(
            "fn cube[x]\n    return x * x * x\nfn slope[x]\n    if x gt 0.0\n        return grad[cube][x]\n    return 0.0\nfn tangent[x, at]\n    return cube[at] + slope[at] * (x - at)\nlog[grad[tangent][3.0, 2.0]]\n",
            "line 5: grad: slope calls grad, and a function grad differentiates cannot",
        );
        refuses("fn f[x]\n    return x * x\nlog[grad[grad[f]][2.0]]\n", "line 3: grad[] takes a function the program writes, and this is a gradient — a gradient is not differentiated again");
        refuses("fn f[x]\n    return x * x\ng = grad[f]\nlog[grad[g][2.0]]\n", "line 4: grad[] takes a function the program writes, and this is a gradient");
        assert_eq!(refusal_of("fn f[x]\n    return x * x * x\nfn df[x]\n    return grad[f][x]\nlog[df[2.0]]\n"), "");
    }

    /// A gradient held in a variable and applied to a value that can fail
    /// (a table `load` read) is a function of the payload in every pass, as
    /// `grad[f]`'s type says: the call scan had typed it a function of a
    /// value that can fail, and the final pass refused its one assignment
    /// as a change of type
    #[test]
    fn a_held_gradient_takes_a_loaded_tables_payload() {
        let src = "use math\nfn loss[w, x]\n    return sum[x * w]\nxs = load['xs.csv']\ndloss = grad[loss]\nlog[dloss[ones[3, 1], xs]]\nn = float['2.5']\nfn square[x]\n    return x * x\nd = grad[square]\nlog[d[n]]\n";
        assert_eq!(refusal_of(src), "", "accepted");
        let ir = ir_of(src);
        assert!(ir.contains("let dloss: Fn(Tensor(Float, [3, 1]), Tensor(Float, [?, ?])) -> List(Tensor(Float, [?, ?])) = grad(fn loss)"), "{}", ir);
        assert!(ir.contains("pure loss(w: Tensor(Float, [3, 1]), x: Tensor(Float, [?, ?])) -> Float"), "{}", ir);
        assert!(ir.contains("let d: Fn(Float) -> Float = grad(fn square)"), "{}", ir);
    }

    /// A builtin's answer a line drops is refused naming the call as the
    /// program wrote it: an index by a mask by its form, what it indexes
    /// kept — never the hidden row behind it (`__pick_frame[]` had been)
    #[test]
    fn a_dropped_pick_is_named_by_its_form() {
        let refused = refusal_of("use frame\ndf = frame.new[age = [61, 34]]\ndf[df.age gt 50]\nlog[df]\n");
        assert!(refused.contains("`f[mask]` answers a frame, and this line drops it — keep it: df = df[…]"), "{}", refused);
        assert!(!refused.contains("__pick"), "{}", refused);
        let refused = refusal_of("xs = ['c', 'a']\npush[xs, 'b']\nlog[xs]\n");
        assert!(refused.contains("push[] answers a list, and this line drops it — keep it: xs = push[xs, …]"), "{}", refused);
    }

    /// A call that answers nothing — a row that declares no answer, or a
    /// function whose returns hand on nothing — gives a variable, a field
    /// or an item no value: refused where it is written, in one wording
    /// (after `y = 1.0`, `y = log[3]` had been an internal error: the
    /// lowered program disagreed with itself)
    #[test]
    fn what_answers_nothing_is_held_nowhere() {
        let refused = refusal_of("y = 1.0\ny = log[3]\nlog[y]\n");
        assert!(refused.contains("line 2: log[] answers nothing, which 'y' cannot hold — call it on a line of its own"), "{}", refused);
        assert_eq!(refused.lines().count(), 1, "one mistake, one message: {}", refused);
        let refused = refusal_of("struct P [x]\np = P[1.0]\np.x = log[2]\nnames = ['a']\nnames[0] = log[4]\na, b = log[5], 2\nlog[p.x, names, a, b]\n");
        assert!(refused.contains("line 3: log[] answers nothing, which the field 'x' cannot hold"), "{}", refused);
        assert!(refused.contains("line 5: log[] answers nothing, which an item of 'names' cannot hold"), "{}", refused);
        assert!(refused.contains("line 6: log[] answers nothing, which 'a' cannot hold"), "{}", refused);
        // (through the program's functions: the body's last call is its
        // implicit return, of nothing no value)
        let refused = refusal_of("fn shout[x]\n    log[x]\nfn twice[x]\n    shout[x]\n    return shout[x]\ny = twice[1]\nlog[y]\n");
        assert!(refused.contains("line 6: twice[] answers nothing, which 'y' cannot hold"), "{}", refused);
        // a function that answers a value is held, and one that answers
        // nothing is called on its own line
        let ir = ir_of("fn shout[x]\n    log[x]\n    return x\nfn quiet[x]\n    log[x]\ny = shout[1]\nquiet[2]\nlog[y]\n");
        assert!(ir.contains("let y: Int = call shout(1)"), "{}", ir);
    }

    /// A test that builds a record the program only receives types the
    /// record's fields where the program leaves them untyped, as its calls
    /// type a function's parameters (the layout engine's node, which Rust
    /// builds, had fields of no type, and the engine's calls on them were
    /// left by name)
    #[test]
    fn a_tests_record_types_the_fields_nothing_else_types() {
        let ir = ir_of("struct Node [tag, kids]\nfn tag_of[n]\n    return upper[n.tag]\ntest t[]\n    n = Node['div', []]\n    assert[tag_of[n] eq 'DIV']\n");
        assert!(ir.contains("struct Node { tag: Str"), "{}", ir);
        assert!(ir.contains("text.upper(n.tag)"), "{}", ir);
    }

    /// A route's handler whose value can fail keeps its Result (the Rust
    /// backend's route answers the failure: emit_rs tests)
    #[test]
    fn a_fallible_routes_handler_keeps_its_result() {
        let ir = ir_of("use web, frame\nfn table[req]\n    return arrow[frame.load['data.csv']]\nroute['/table', table]\nserve[3999]\n");
        assert!(ir.contains("pure table(req: Handle(web.request)) -> Result(Handle(response))"), "{}", ir);
    }

    /// A frame a handler stores into a fact joins the fact's columns: the
    /// fact holds its default's or the handler's (it had kept its default's,
    /// no columns, and a table naming the handler's column was refused).
    #[test]
    fn a_frame_fact_holds_what_its_handlers_store() {
        let src = "use web, ui, frame\nuniverse = frame.new[]\nfn keep[u]\n    universe = u\nfn fill_in[]\n    keep[frame.new[symbol = ['a']]]\nview v[]\n    return stack[table[universe, key = 'symbol'], button['Load', click = fill_in]]\npage['/', v]\n";
        assert_eq!(refusal_of(src), "");
        assert!(ir_of(src).contains("fact universe: Frame{…} = frame.new()"), "{}", ir_of(src));
    }

    /// A column handed on whole keeps its cells' type (`with` copying one
    /// had typed it as text, so arithmetic on the copy was refused), and a
    /// renamed column keeps its type under its new name.
    #[test]
    fn a_copied_or_renamed_column_keeps_its_type() {
        let ir = ir_of("use frame\ndf = frame.new[a = [1.5, 2.5]]\ndf.b = df.a\nlog[df.b[0] * 2.0]\nr = rename[df, 'a', 'z']\nlog[r.z[0]]\n");
        assert!(ir.contains("let df: Frame{a: Float, b: Float}"), "{}", ir);
        assert!(ir.contains("let r: Frame{b: Float, z: Float}") || ir.contains("let r: Result(Frame{b: Float, z: Float})"), "{}", ir);
    }

    /// Where ways through meet, a frame has the columns they share: one a
    /// branch adds, or a loop's body, is one it may not have (open); what
    /// they share keeps its type.
    #[test]
    fn a_frame_after_branches_has_the_columns_they_share() {
        let ir = ir_of("use frame\ndf = frame.new[a = [1.5]]\nif len[df] gt 0\n    df.b = ['x']\nafter = df\nlog[after.a[0]]\n");
        assert!(ir.contains("let after: Frame{a: Float, …} = move df"), "{}", ir);
        let ir = ir_of("use frame\ndf = frame.new[a = [1.5]]\nfor i in range[2]\n    df.c`i` = [2.0]\nafter = df\nlog[after.a[0]]\n");
        assert!(ir.contains("let after: Frame{a: Float, …} = move df"), "{}", ir);
    }

    /// A function given frames of other columns is a copy per set of them,
    /// as for any other type, so each reads the columns it is handed.
    #[test]
    fn a_function_is_specialized_per_frame_columns() {
        let ir = ir_of("use frame, math\nfn total[f]\n    return sum[f.amount]\nx = frame.new[amount = [1.5]]\ny = x\ny.who = ['a']\nlog[total[x]]\nlog[total[y]]\n");
        assert!(ir.contains("(f: Frame{amount: Float}) -> Float"), "{}", ir);
        assert!(ir.contains("(f: Frame{amount: Float, who: Str}) -> Float"), "{}", ir);
    }

    /// A column a frame whose columns are all known does not have is
    /// refused before the program runs, in the runtime's words: read, or
    /// named where an operation takes a column, or read by a function the
    /// frame is handed to.
    #[test]
    fn a_column_a_known_frame_lacks_is_refused() {
        let frame = "use frame, math\ndf = frame.new[price = [1.5, 2.5]]\n";
        for (line, words) in [
            ("log[df.cost[0]]", "frame has no property or column 'cost' — its columns are 'price'"),
            ("log[sort[df, 'cost']]", "sort: column 'cost' not found — the frame's columns are 'price'"),
            ("log[select[df, ['cost']]]", "select: column 'cost' not found"),
            ("log[fill[df, 'cost', 0.0]]", "fill: column 'cost' not found"),
            ("log[group[df, 'cost', ['price', 'sum']]]", "group: column 'cost' not found"),
            ("log[join[df, df, 'cost']]", "join: column 'cost' not found"),
        ] {
            let refused = refusal_of(&format!("{}{}\n", frame, line));
            assert!(refused.contains(words), "{}: {}", line, refused);
        }
        let refused = refusal_of("use frame, math\nfn total[f]\n    return sum[f.amount]\nprices = frame.new[price = [1.5]]\nlog[total[prices]]\n");
        assert!(refused.contains("frame has no property or column 'amount' — its columns are 'price' (the frame total[] is handed as `f`)"), "{}", refused);
        let refused = refusal_of(&format!("{}log[rename[df, 'cost', 'c']]\n", frame));
        assert!(refused.contains("rename: column 'cost' not found"), "{}", refused);
        // what a table or a chart names of the frame it draws
        let view = "use web, ui, frame\ndf = frame.new[price = [1.5, 2.5]]\nview v[]\n    return ";
        for (tree, words) in [
            ("table[df, key = 'id']", "table: column 'id' not found — the frame's columns are 'price'"),
            ("table[df, hide = ['id']]", "table: column 'id' not found"),
            ("chart[df, lines = ['close']]", "chart: column 'close' not found"),
        ] {
            let refused = refusal_of(&format!("{}{}\npage['/', v]\n", view, tree));
            assert!(refused.contains(words), "{}: {}", tree, refused);
        }
        // a frame whose columns show only as it runs is checked as it runs
        assert_eq!(refusal_of("use frame\ndf = load['x.csv']\nlog[df.cost[0]]\n"), "");
    }

    /// A value a guard knows has failed, handed back, hands back its
    /// failure: the frame it would have held is none of the function's
    /// columns.
    #[test]
    fn a_guarded_failure_hands_back_no_columns() {
        let ir = ir_of("use frame\nfn pick[n]\n    a = frame.new[k = float['1.5']]\n    if not a.ok\n        return a\n    return frame.new[v = ['s']]\nr = pick[1]\nlog[r.v[0]]\n");
        assert!(ir.contains("pick(n: Int) -> Result(Frame{v: Str})"), "{}", ir);
    }

    /// Each builtin call names the row it runs, picked once from its
    /// arguments' types: both backends call that function, where each had
    /// picked a row of its own (docs/bug-classes.md §4). An argument whose
    /// kind is not known leaves the call by name, its values picking.
    #[test]
    fn a_builtin_call_names_the_row_it_runs() {
        let ir = ir_of(
            "use math\ns = 'héllo'\nxs = ['a', 'b']\nm = map[a = 1]\nt = [1.0, -2.0]\nj = json['[1, 2, 3]']\nlog[len[s]]\nlog[len[xs]]\nlog[len[m]]\nlog[len[j]]\nlog[abs[-2]]\nlog[abs[t]]\nlog[max[1, 2]]\nlog[mean[t]]\nlog[math.sqrt[16.0]]\nr = read['nothing.txt']\nlog[r.ok]\nfn f[x]\n    return len[x]\n",
        );
        for named in [
            "core.len_text(move s)",
            "core.len(move xs)",
            "core.len_map(move m)",
            "core.abs_int(proven((neg 2)))",
            // a name the core and math both answer is math's for a tensor
            "math.abs_t(t)",
            "core.max_ints(1, 2)",
            "math.mean(move t)",
            // a qualified call to a module the registry holds is its bare call
            "core.sqrt(16.0)",
            // an effect's call is its row too, inside the session layer
            "io.read!now(\"nothing.txt\")",
            "display.log!held(",
        ] {
            assert!(ir.contains(named), "{}\n{}", named, ir);
        }
        assert!(ir.contains("return len(move x)"), "{}", ir);
        // a parsed value is JSON in both modes, and names JSON's row (it had
        // been left by name, its kind showing as it ran)
        assert!(ir.contains("core.len_json(unwrap move j)"), "{}", ir);
    }

    /// A test's inputs are typed by their use in the lowered body — a
    /// function's parameter they are handed to, the number on the other
    /// side of arithmetic — and its bare boolean lines are truths, each
    /// lowered as an assertion: decisions the test runner had made on the
    /// syntax tree, with a second inference of its own.
    #[test]
    fn a_test_types_its_inputs_and_asserts_its_truths() {
        let src = "fn area[w, h]\n    return w * h\nlog[area[2.0, 3.0]]\ntest[a, dt]\n    t = 1.5 - dt\n    area[a, t] gte 0.0 or true\n    assert[a eq a]\n    t lt 100.0 or true\n    log[t]\n";
        let stmts = crate::build(src).expect("parses");
        let (program, _) = crate::ir_interp::prepare(&stmts, "t").expect("lowers");
        let test = &program.parts[0].tests[0];
        assert_eq!(test.params, vec![("a".to_string(), Ty::Float), ("dt".to_string(), Ty::Float)], "{:?}", test.params);
        // two bare boolean lines; an assert written as one is not a truth
        assert_eq!(test.truths, 2);
        let asserts = test.body.iter().filter(|s| matches!(s, Stmt::Do(Expr::Call { target: Target::Runtime(n), .. }) if n == "assert")).count();
        assert_eq!(asserts, 3);
    }

    /// A loop walks what has items. A map, text or a frame has none, and
    /// the loop is refused before the program runs, in both modes, naming
    /// what to walk instead (the interpreter had refused the map as it
    /// ran; the loop over text had compiled to Rust that did not build).
    #[test]
    fn a_loop_over_what_has_no_items_is_refused() {
        let refused = |src: &str| crate::build(src).ok().and_then(|stmts| crate::ir_interp::prepare(&stmts, "t").err()).unwrap_or_default();
        assert!(refused("counts = map[a = 1, b = 2]\nfor k in counts\n    log[k]\n").contains("a loop walks map[int], which has no items — walk keys[m] or values[m]"));
        assert!(refused("for c in 'abc'\n    log[c]\n").contains("a loop walks str, which has no items — walk split[s, '']"));
        assert!(refused("use frame\ndf = frame.new[a = [1, 2]]\nfor r in df\n    log[r]\n").contains("a loop walks frame, which has no items — walk one of its columns"));
        assert!(refused("for n in 3\n    log[n]\n").contains("a loop walks int, which has no items"));
        // what has items walks
        assert!(crate::build("for x in [1, 2]\n    log[x]\nfor r in [[1.0, 2.0], [3.0, 4.0]]\n    log[r]\n").ok().and_then(|s| crate::ir_interp::prepare(&s, "t").ok()).is_some());
    }

    /// A truth under a precondition is a truth: a law's bare lines inside
    /// its `if` are asserted by their syntax, a bool a call answers there
    /// by its type, and a line in a loop the same (only the top level had
    /// been read; a law whose two truths stood under its precondition had
    /// asserted nothing, and passed).
    #[test]
    fn a_truth_under_a_precondition_is_asserted() {
        // (positive is called by the program, so its bool is known where
        // the test calls it; a function only a test calls answers an unknown)
        let src = "fn area[w, h]\n    return w * h\nfn positive[x]\n    return x gt 0.0\nlog[area[2.0, 3.0]]\nlog[positive[2.0]]\ntest[a, dt]\n    if a gt 0.0 and dt gte 0.0\n        area[a, dt] gte 0.0\n        positive[a]\n    for k in [1, 2]\n        k lt 3\n";
        let stmts = crate::build(src).expect("parses");
        let (program, _) = crate::ir_interp::prepare(&stmts, "t").expect("lowers");
        let test = &program.parts[0].tests[0];
        assert_eq!(test.truths, 3);
        let mut asserts = 0;
        visit_block(&test.body, &mut |e| {
            if let Expr::Call { target: Target::Runtime(n), .. } = e {
                if n == "assert" {
                    asserts += 1;
                }
            }
        });
        assert_eq!(asserts, 3);
        // and a false one under its guard fails the block
        let src = "test nested_false[]\n    y = 1.0\n    if y gt 0.0\n        y eq 2.0\n";
        let stmts = crate::build(src).expect("parses");
        let (issues, counts) = crate::checker::run_test_blocks(&stmts);
        assert_eq!(counts.truths, 1);
        assert_eq!(issues.len(), 1, "{:?}", issues);
        assert!(format!("{:?}", issues[0]).contains("assert failed"), "{:?}", issues);
    }

    /// A parsed value — a document's member — is read as the kind its use
    /// implies, in both modes: a position an int, a key text, one met in
    /// arithmetic or an order a float, one decided on, negated or joined
    /// with and/or a bool, one handed to a builtin what its rows take,
    /// one assigned to a variable of a kind that kind; the program stops
    /// where the value is not that, with what it holds and the read that
    /// asked. (Each had been refused until read by hand; a parsed value
    /// decided on, negated or joined with and had run interpreted and not
    /// built compiled.) Two parsed values compare as the program runs, and
    /// a loaded frame's cell as a position or a key is typed by that read.
    /// The map's type carries its keys' kind: from two lists by the keys,
    /// from a literal's int keys, from the first `set` of an open map; its
    /// keys read back as that kind.
    #[test]
    fn a_maps_type_carries_its_key_kind() {
        let ir = ir_of("m = map[[1, 2], ['a', 'b']]\nks = keys[m]\nt = map[a = 1]\no = map[]\no = set[o, 3, 'x']\nl = map[5 = 'q']\nlog[m[1], ks, t, o, l]\n");
        assert!(ir.contains("let m: Map(Int, Str)"), "{}", ir);
        assert!(ir.contains("let ks: List(Int)"), "{}", ir);
        assert!(ir.contains("let t: Map(Int)"), "{}", ir);
        assert!(ir.contains("let o: Map(Int, Str)"), "{}", ir);
        assert!(ir.contains("let l: Map(Int, Str)"), "{}", ir);
    }

    #[test]
    fn a_parsed_value_is_read_as_its_use_implies() {
        let ir = ir_of("d = json[args[][0]]\nxs = [10, 20, 30]\nm = map[a = 1]\nlog[xs[d['i']]]\nlog[m[d['k']]]\nlog[d['n'] gt 1]\nlog[d['x'] * 2.0]\ntotal = 0\ntotal = d['n']\nlog[total]\nlog[upper[d['k']]]\nif d['flag']\n    log['yes']\nlog[not d['flag']]\nlog[d['flag'] and true]\n");
        for needle in ["core.__read_int(", "\"a position in xs at line 4\"", "core.__read_str(", "\"a key of m at line 5\"", "\"met in gt at line 6\"", "\"met in mul at line 7\"", "\"assigned to total at line 9\"", "\"handed to upper at line 11\"", "core.__read_bool(", "\"decided on at line 12\"", "\"negated at line 14\"", "\"met in and at line 15\""] {
            assert!(ir.contains(needle), "{} missing in\n{}", needle, ir);
        }
        // two parsed values compared, whole or in order, compare as the
        // program runs; two met in arithmetic are both read as numbers
        let dynamic = ir_of("d = json[args[][0]]\nlog[d['a'] eq d['b']]\nlog[d['a'] gt d['b']]\n");
        assert!(!dynamic.contains("__read_"), "{}", dynamic);
        let both = ir_of("d = json[args[][0]]\nlog[d['a'] + d['b']]\n");
        assert_eq!(both.matches("core.__read_float(").count(), 2, "{}", both);
        let refused = |src: &str| crate::build(src).ok().and_then(|stmts| crate::ir_interp::prepare(&stmts, "t").err()).unwrap_or_default();
        let by_cell = "use frame\nname = args[][0]\nt = frame.load[name]\nxs = [10, 20, 30]\nm = map[a = 1]\nlog[xs[t.at[0]]]\nlog[m[t.k[0]]]\n";
        assert_eq!(refused(by_cell), "");
        let ir = ir_of(by_cell);
        assert!(ir.contains("[\"at\", \"k\"], [\"int\", \"str\"], [\"a position in xs at line 6\", \"a key of m at line 7\"]"), "{}", ir);
    }

    #[test]
    fn a_fallible_argument_reaches_the_callee_as_its_payload() {
        // the call propagates an error among its arguments (spec §10); the
        // IR says so on the call and hoists nothing
        let ir = ir_of("r = read['x.txt']\nn = len[r]\nlog[n]\n");
        assert!(ir.contains("let r: Result(Str) = io.read!now(\"x.txt\")"), "{}", ir);
        assert!(!ir.contains("propagate r"), "{}", ir);
        // len[] of an error is the error, as every call's is: the value can fail
        assert!(ir.contains("let n: Result(Int) = core.len_text(unwrap move r)"), "{}", ir);
    }

    #[test]
    fn a_fallible_condition_is_propagated_first() {
        let ir = ir_of("fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nlog[check['7']]\n");
        assert!(ir.contains("propagate"), "{}", ir);
        assert!(ir.contains("propagate _t1\n    if unwrap _t1"), "{}", ir);
    }

    #[test]
    fn ints_meet_floats_through_an_explicit_conversion() {
        let ir = ir_of("x = 2\ny = x * 1.5\nlog[y]\n");
        assert!(ir.contains("(float(x) mul 1.5)"), "{}", ir);
    }

    #[test]
    fn a_derivation_lists_what_it_reads() {
        let ir = ir_of("use web, ui\nbase = 0.0\nspeed = 1.0\nderive head\n    return base + (float[now[]] - 0.0) * speed\nview v[]\n    return p['`head`']\n");
        assert!(ir.contains("derive head: Float  reads {facts: base speed, clock}"), "{}", ir);
    }

    #[test]
    fn a_parallel_loop_is_marked_and_a_writing_one_is_not() {
        let ir = ir_of("total = 0\nfor x in [1, 2, 3]\n    total = total + x\nfor y in [1, 2]\n    z = y * 2\nlog[total]\n");
        assert!(ir.contains("for x in [1, 2, 3]\n"), "{}", ir);
        assert!(ir.contains("for y in [1, 2]  parallel"), "{}", ir);
    }

    /// The printed line of the loop whose body mentions `needle`.
    fn loop_line<'a>(ir: &'a str, needle: &str) -> &'a str {
        ir.lines().find(|l| l.contains(" for ") && l.contains(needle)).unwrap_or_else(|| panic!("no loop over {} in\n{}", needle, ir))
    }

    /// One rule for both modes (spec §14, For Loops): a loop over a
    /// function runs in parallel when the function has no side effects,
    /// however deep they are, and in order when it has one
    #[test]
    fn a_loop_over_a_function_is_parallel_only_without_side_effects() {
        let ir = ir_of("fn sq[x]\n    return x * x\nfn say[x]\n    log[x]\n    return x\nfn relay[x]\n    return say[x]\nxs = for i in [1, 2, 3]\n    sq[i]\nys = for i in [1, 2, 3]\n    relay[i]\nlog[xs]\nlog[ys]\n");
        assert!(loop_line(&ir, "call sq(").ends_with("parallel"), "{}", ir);
        assert!(!loop_line(&ir, "call relay(").ends_with("parallel"), "{}", ir);
    }

    /// A seeded program's draws repeat only in the order written: a loop
    /// that draws runs in order (it ran in parallel, and three runs of a
    /// seeded script gave three answers)
    #[test]
    fn a_loop_that_draws_runs_in_order() {
        let ir = ir_of("use math\nseed[1]\nxs = for i in [1, 2, 3]\n    rand[]\nys = for i in [1, 2]\n    randn[2]\nlog[xs]\nlog[ys]\n");
        assert!(!loop_line(&ir, "rand()").ends_with("parallel"), "{}", ir);
        assert!(!loop_line(&ir, "randn").ends_with("parallel"), "{}", ir);
    }

    /// A view renders in order (the page's stale marks and paged tables
    /// are kept in render order); plain elements are values
    #[test]
    fn a_loop_that_renders_a_view_runs_in_order() {
        let ir = ir_of("use web, ui\n\nitems = ['a', 'b']\n\nview item[x]\n    return p[x]\n\nview render[]\n    plain = for x in items\n        span[x]\n    viewed = for x in items\n        item[x]\n    return stack[plain, viewed]\n\nserve[3996]\npage['/', render]\n");
        assert!(loop_line(&ir, "span").ends_with("parallel"), "{}", ir);
        assert!(!loop_line(&ir, "call item(").ends_with("parallel"), "{}", ir);
    }

    /// A call through a function value reaches any function the program
    /// holds as a value; handing one to a builtin is a call of it too
    #[test]
    fn a_function_value_counts_by_what_it_could_be() {
        let pure = ir_of("fn keep[items, f]\n    out = for x in items\n        if f[x]\n            x\n    return out\nfn even[x]\n    return x % 2 eq 0\nlog[keep[[1, 2, 3], even]]\nrows = for r in [[1, 2], [3, 4]]\n    any[r, even]\nlog[rows]\n");
        assert!(loop_line(&pure, "f[").ends_with("parallel"), "{}", pure);
        assert!(loop_line(&pure, "any(").ends_with("parallel"), "{}", pure);
        let loud = ir_of("fn keep[items, f]\n    out = for x in items\n        if f[x]\n            x\n    return out\nfn loud[x]\n    log[x]\n    return true\nlog[keep[[1, 2, 3], loud]]\nrows = for r in [[1, 2], [3, 4]]\n    any[r, loud]\nlog[rows]\n");
        assert!(!loop_line(&loud, "f[").ends_with("parallel"), "{}", loud);
        assert!(!loop_line(&loud, "any(").ends_with("parallel"), "{}", loud);
    }

    /// A loop's list that can fail is refused first, like a condition: the
    /// statement form's refusal comes before it, and the loop walks the
    /// value; a collecting loop, an expression, keeps the list and is
    /// refused where it starts, and a loop around it runs in order
    #[test]
    fn a_loop_over_a_list_that_can_fail_is_refused_first() {
        let ir = ir_of("fn f[s]\n    for i in range[int[s]]\n        log[i]\n    out = for j in range[int[s]]\n        j * 2\n    return out\nlog[f['2']]\nys = for k in [1, 2]\n    zs = for m in range[int['3']]\n        m\n    len[zs]\nlog[ys]\n");
        assert_eq!(ir.matches("(a loop's list)").count(), 1, "{}", ir);
        assert!(ir.contains("for i in unwrap"), "{}", ir);
        let collecting = loop_line(&ir, "for j in");
        assert!(!collecting.contains("propagate") && !collecting.contains("unwrap t"), "refused where it starts: {}", collecting);
        assert!(!loop_line(&ir, "for k in").ends_with("parallel"), "a loop around one runs in order: {}", ir);
    }

    /// A body that can leave the function (an error handed back from a
    /// condition) runs in order: the first failure in input order decides
    #[test]
    fn a_loop_that_can_return_an_error_runs_in_order() {
        let ir = ir_of("fn f[xs]\n    out = for x in xs\n        if int[x] gt 0\n            x\n    return len[out]\nlog[f[['1', '2']]]\n");
        assert!(!loop_line(&ir, "for x in").ends_with("parallel"), "{}", ir);
    }

    /// A standard module's function written in Crow (math.nn,
    /// src/modules/nn.co) is the program's call of the function inlined:
    /// typed by its use, a copy per signature (a vector and a matrix are
    /// two), the answer its body's; a misuse is refused before the program
    /// runs where the body first cannot take the kind, the refusal naming
    /// the function the program called and the program's line.
    #[test]
    fn a_modules_function_written_in_crow_is_the_programs_call() {
        let ir = ir_of("use math, math.nn\nlog[relu[[-1.0, 2.0]]]\nlog[mse[[1.0, 2.0], [1.0, 4.0]]]\nlog[sum[relu[[[1.0, -2.0], [3.0, 0.5]]]]]\n");
        assert!(ir.contains("pure _math_nn_relu"), "{}", ir);
        assert!(ir.contains("call _math_nn_mse"), "{}", ir);
        assert!(ir.contains("math.sum(call _math_nn_relu"), "{}", ir);
        assert!(!ir.contains("_math_nn__through_all"), "a function only the module's tests use is not inlined: {}", ir);
        let r = refusal_of("use math, math.nn\nlog[relu[5]]\n");
        assert!(r.contains("in relu:") && r.contains("got an int") && !r.contains("_math_nn"), "{}", r);
        let r = refusal_of("use math, math.nn\nlog[mse[[1.0], 'x']]\n");
        assert!(r.contains("in mse:") && !r.contains("_math_nn"), "{}", r);
    }

    /// A call indexed (`f[x][0]`) types its callee as any call does; a
    /// function whose parameter nothing has typed answers nothing typed
    /// (not a number guessed from `x - max[x]`); and a gradient's list has
    /// its items' rank only where every parameter shares it.
    #[test]
    fn an_indexed_call_and_an_untyped_parameter_type_as_calls_do() {
        let ir = ir_of("use math\nfn f[x]\n    return x .* 2.0\nlog[f[[1.0]][0]]\n");
        assert!(ir.contains("pure f(x: Tensor(Float, [1])) -> Tensor(Float, [1])"), "{}", ir);
        let ir = ir_of("use math\nfn g[x]\n    ex = exp[x - max[x]]\n    return ex / sum[ex]\nlog[1]\n");
        assert!(ir.contains("pure g(x: ?) -> ?"), "{}", ir);
        let r = refusal_of("use math, math.nn\nfn loss[w, y]\n    return mse[flatten[w], y]\ngrads = grad[loss][[[1.0], [2.0]], [1.0, 2.0]]\nlog[grads[0][0][0]]\n");
        assert!(r.contains("the rank of the tensor is not known here"), "{}", r);
    }

    /// A parameter named like an effect, indexed, is the parameter; a call
    /// of a name a module in scope also answers is typed by the core's row
    /// where one takes the arguments (`slice` of text beside sys's of
    /// bytes); and `map[]` handed where a row takes a map of text is one.
    /// A column made from its parts is a column of its kind (`ints_column`
    /// answers ints, as its row declares), and written onto a frame it is
    /// the frame's column of that kind: typed a column of nothing known, it
    /// had been the frame's text, and `df.v eq 1` was refused before the
    /// program ran (found writing Magpie's read verbs in Crow)
    #[test]
    fn a_column_made_from_its_parts_keeps_its_kind() {
        let ir = ir_of("use sys, frame\nvals = []\nfor i in range[3]\n    vals = push[vals, i]\nc = ints_column[vals, bytes[0]]\ndf = frame.new[id = [1, 2, 3]]\ndf.v = c\nlog[len[df[df.v eq 1]]]\n");
        assert!(ir.contains("let c: Column(Int)"), "{}", ir);
        assert!(ir.contains("v: Int"), "{}", ir);
    }

    #[test]
    fn a_parameter_named_like_an_effect_and_a_name_two_tables_answer() {
        let ir = ir_of("fn f[query]\n    return query[0]\nlog[f[[1, 2]]]\n");
        assert!(ir.contains("return query[0]"), "{}", ir);
        let ir = ir_of("use sys\nfn parse[head]\n    lines = split[head, '|']\n    out = ''\n    for line in slice[lines, 1, len[lines]]\n        out = trim[slice[line, 0, 1]]\n    return out\nlog[parse['ab|cd|ef']]\n");
        assert!(ir.contains("core.slice_str(") && ir.contains("let lines: List(Str)"), "{}", ir);
        // (`map[]` typed as a map of text: the IR shows the literal, the
        // book's sys page builds it compiled)
        let ir = ir_of("use sys\nr = reply[404, 'no', map[]]\nlog[r.status]\n");
        assert!(ir.contains("sys.reply(404, \"no\", {})"), "{}", ir);
    }
}
