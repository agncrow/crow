//! The functions `grad` differentiates over tensors (spec, Automatic
//! Differentiation; crow_runtime::autograd): which they are, what they
//! may do, and what both backends make of them. A function handed to
//! `grad` whose parameters are tensors, and every function of the
//! program's it reaches, is *traced*: the compiled program makes a copy of
//! each over traced values, the interpreter runs them on traced values,
//! and the fusion pass leaves them alone (a fused kernel would be opaque
//! to the tape). What a traced function may do is the list of
//! differentiable operations; anything else handed a traced value is
//! refused before the program runs, in both modes, so nothing is silently
//! a constant. Its loops run in order: one tape records the run.
//!
//! What is traced is what carries a derivative (`active_parameters`): a
//! target's tensors, and in a function it reaches, a parameter a call
//! hands such a value, with every local computed from one. A function no
//! call hands one — a table of positions built from a tensor's sizes, a
//! count — runs as it does outside, and a value that carries none (a
//! position made a float, a scale read from a layer) may meet any builtin:
//! the derivative cannot reach it, so nothing is lost. Both had been held
//! to the list of differentiable operations by their kind alone, and a
//! convolution's table of positions, built with `push`, was refused.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::*;

/// The functions `grad` differentiates, with those they reach, settled:
/// `program.traced` names them, their loops run in order, and what they
/// may not do is the refusals answered (each with its line).
pub(super) fn settle(program: &mut Program) -> Vec<(String, String)> {
    let mut refusals = Vec::new();
    for part in &mut program.parts {
        let targets = targets_of(part);
        if targets.is_empty() {
            continue;
        }
        // the tensor-differentiated ones: a parameter typed a tensor
        let tensor_targets: BTreeSet<String> = targets
            .iter()
            .filter(|t| part.fns.iter().any(|f| &f.name == *t && f.params.iter().any(|(_, ty)| matches!(ty.payload(), Ty::Tensor(..)))))
            .cloned()
            .collect();
        // the traced: the targets, and the functions a call hands a value
        // that carries a derivative (a function reached only with sizes,
        // positions and constants is not)
        let active = active_parameters(part, &tensor_targets);
        let traced: BTreeSet<String> = active.keys().cloned().collect();
        let mut carried = BTreeMap::new();
        // a function differentiated over numbers, and every function it
        // reaches (each run on the dual numbers), calls no `grad` either,
        // as the spec refuses it for both kinds: the inner gradient had run
        // on the dual's value alone, and the outer one answered 0 — no
        // second derivative, silently (a traced one is refused below)
        let mut duals = BTreeSet::new();
        for t in targets.iter().filter(|t| !tensor_targets.contains(*t)) {
            duals.insert(t.clone());
            duals.extend(reached_from(part, t));
        }
        for f in part.fns.iter().filter(|f| duals.contains(&f.name) && !traced.contains(&f.name)) {
            let mut line = first_line(&f.body);
            if let Some(at) = grad_called_in(&f.body, &mut line) {
                refusals.push((f.name.clone(), format!("line {}: grad: {} calls grad, and a function grad differentiates cannot", at, f.name)));
            }
        }
        for f in &mut part.fns {
            if !traced.contains(&f.name) {
                continue;
            }
            // parameters: the tensors, differentiated, and beside them
            // constants of the run — a number, a bool, text, a list, a map,
            // a record (a model's layers, a rate) — which take no gradient
            if tensor_targets.contains(&f.name) {
                for (p, t) in &f.params {
                    if !constant_kind(t) {
                        let line = first_line(&f.body);
                        refusals.push((f.name.clone(), format!(
                            "line {}: grad[{}]: a parameter beside the tensors is a constant of the run — a number, a bool, text, a list, a map or a record — and '{}' is {}",
                            line, f.name, p, kind_word(t)
                        )));
                    }
                }
            }
            let names = active_names(f, &active[&f.name]);
            let mut line = first_line(&f.body);
            check_block(&f.name, &mut f.body, &mut line, &names, &mut refusals);
            in_order(&mut f.body);
            carried.insert(f.name.clone(), names);
        }
        part.traced = traced;
        part.active = carried;
    }
    refusals
}

/// Every loop of a traced body runs in order — a collecting loop's, and a
/// loop inside one, included: the tape is the thread's own, so a round run
/// on another thread records nothing, and its share of the gradient is
/// lost with no word (a collecting loop's rounds had been shared across
/// the cores, and the compiled copy did not build).
fn in_order(block: &mut Block) {
    for s in block.iter_mut() {
        match s {
            Stmt::For { body, parallel, .. } => {
                *parallel = false;
                in_order(body);
            }
            Stmt::While { body, .. } => in_order(body),
            Stmt::If { then, els, .. } => {
                in_order(then);
                if let Some(e) = els {
                    in_order(e);
                }
            }
            _ => {}
        }
    }
    visit_block_mut(block, &mut |e| {
        if let Expr::ForExpr { parallel, body, .. } = e {
            *parallel = false;
            in_order(body);
        }
    });
}

/// A parameter a function differentiated over tensors may take beside them:
/// a tensor, differentiated, or a value of a kind that is a constant of the
/// run (spec, Automatic Differentiation).
fn constant_kind(t: &Ty) -> bool {
    matches!(
        t.payload(),
        Ty::Tensor(..) | Ty::Int | Ty::Big | Ty::Float | Ty::Bool | Ty::Str | Ty::List(_) | Ty::Map(..) | Ty::Record(_) | Ty::OptRecord(_)
    )
}

fn kind_word(t: &Ty) -> &'static str {
    match t.payload() {
        Ty::Tensor(..) => "a tensor",
        Ty::Int | Ty::Float => "a number",
        Ty::Str | Ty::Markup => "text",
        Ty::Bool => "a bool",
        Ty::List(_) => "a list",
        Ty::Map(..) => "a map",
        Ty::Record(_) | Ty::OptRecord(_) => "a record",
        Ty::Frame(_) => "a frame",
        Ty::Fn(..) | Ty::Fns(_) => "a function",
        Ty::Handle(_) => "a handle",
        _ => "a value of a kind grad does not differentiate",
    }
}

/// Whether an expression is a call of `grad`.
fn is_grad(e: &Expr) -> bool {
    matches!(e, Expr::Call { .. }) && call_name(e).as_deref() == Some("grad")
}

/// The line of the first call of `grad` in a body, where it has one.
fn grad_called_in(block: &[Stmt], line: &mut usize) -> Option<usize> {
    for s in block {
        if let Stmt::Line(n) = s {
            *line = *n;
            continue;
        }
        // (a nested block's own lines place the call inside it; the
        // statement's own expressions are on its line)
        let here = *line;
        let inner = match s {
            Stmt::If { then, els, .. } => grad_called_in(then, line).or_else(|| els.as_ref().and_then(|e| grad_called_in(e, line))),
            Stmt::While { body, .. } | Stmt::For { body, .. } => grad_called_in(body, line),
            _ => None,
        };
        if inner.is_some() {
            return inner;
        }
        let mut found = false;
        visit_stmt(s, &mut |e| found |= is_grad(e));
        if found {
            return Some(here);
        }
    }
    None
}

/// The functions `grad` is handed, anywhere in the part.
fn targets_of(part: &Part) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut note = |e: &Expr| {
        if let Expr::Call { args, .. } = e {
            if is_grad(e) {
                for a in args {
                    if let Expr::FnRef(f, _) = a {
                        out.insert(f.clone());
                    }
                }
            }
        }
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
    visit_block(&part.main, &mut note);
    out
}

/// The program's functions a function reaches by calling.
fn reached_from(part: &Part, name: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut todo = vec![name.to_string()];
    while let Some(n) = todo.pop() {
        let Some(f) = part.fns.iter().find(|f| f.name == n) else { continue };
        let mut called = Vec::new();
        visit_block(&f.body, &mut |e| {
            if let Expr::Call { target: Target::User(c), .. } = e {
                called.push(c.clone());
            }
        });
        for c in called {
            if c != name && seen.insert(c.clone()) {
                todo.push(c);
            }
        }
    }
    seen
}

/// Whether the call runs on the tape: its row declares a kernel for that
/// many arguments (`table::tape_of`), or a row of its name does.
fn taped(target: &Target, name: &str, n: usize) -> bool {
    match target {
        Target::Row(row) => crate::table::tape_of(row, n).is_some(),
        _ => crate::table::tape_named(name, n).is_some(),
    }
}

/// Which parameters carry a derivative, by function: a target's tensors,
/// and in a function reached, each one a call hands a value that carries
/// one (`is_active`), settled over the calls. A function every call hands
/// only sizes, positions and constants has no entry: it is not traced.
fn active_parameters(part: &Part, targets: &BTreeSet<String>) -> BTreeMap<String, BTreeSet<String>> {
    let find = |n: &str| part.fns.iter().find(|f| f.name == n);
    let mut active: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in targets {
        if let Some(f) = find(t) {
            active.insert(t.clone(), f.params.iter().filter(|(_, ty)| matches!(ty.payload(), Ty::Tensor(..))).map(|(p, _)| p.clone()).collect());
        }
    }
    loop {
        let mut grew = false;
        let now: Vec<(String, BTreeSet<String>)> = active.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (name, params) in now {
            let Some(f) = find(&name) else { continue };
            let names = active_names(f, &params);
            let mut handed: Vec<(String, usize)> = Vec::new();
            visit_block(&f.body, &mut |e| {
                if let Expr::Call { target: Target::User(c), args, .. } = e {
                    for (i, a) in args.iter().enumerate() {
                        if is_active(a, &names) {
                            handed.push((c.clone(), i));
                        }
                    }
                }
            });
            for (c, i) in handed {
                let Some(p) = find(&c).and_then(|g| g.params.get(i)).map(|(p, _)| p.clone()) else { continue };
                grew |= active.entry(c).or_default().insert(p);
            }
        }
        if !grew {
            return active;
        }
    }
}

/// The names of a body that carry a derivative, given the parameters that
/// do: every local a statement computes from one (`is_active`), through
/// any operator, call, field, index or literal whose value can hold a
/// number. A whole number, a truth and text carry none: `len[x]`, `x.shape`
/// and `x gt 0.0` are the tape's constants, and so is what is made of them.
fn active_names(f: &Fn, params: &BTreeSet<String>) -> BTreeSet<String> {
    let mut names = params.clone();
    loop {
        let before = names.len();
        bind_block(&f.body, &mut names);
        if names.len() == before {
            return names;
        }
    }
}

fn bind_block(block: &[Stmt], names: &mut BTreeSet<String>) {
    for s in block {
        match s {
            Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } | Stmt::SetPath { var, expr, .. } => {
                if is_active(expr, names) {
                    names.insert(var.clone());
                }
                bind_expr(expr, names);
            }
            Stmt::For { var, iter, body, .. } => {
                if is_active(iter, names) {
                    names.insert(var.clone());
                }
                bind_expr(iter, names);
                bind_block(body, names);
            }
            Stmt::If { cond, then, els } => {
                bind_expr(cond, names);
                bind_block(then, names);
                if let Some(e) = els {
                    bind_block(e, names);
                }
            }
            Stmt::While { cond, body } => {
                bind_expr(cond, names);
                bind_block(body, names);
            }
            Stmt::Do(e) | Stmt::Return(Some(e)) | Stmt::Store { expr: e, .. } => bind_expr(e, names),
            _ => {}
        }
    }
}

/// The names a collecting loop inside an expression binds.
fn bind_expr(e: &Expr, names: &mut BTreeSet<String>) {
    let mut loops = Vec::new();
    visit_expr(e, &mut |x| {
        if let Expr::ForExpr { var, iter, body, .. } = x {
            loops.push((var.clone(), (**iter).clone(), body.clone()));
        }
    });
    for (var, iter, body) in loops {
        if is_active(&iter, names) {
            names.insert(var);
        }
        bind_block(&body, names);
    }
}

fn is_active(e: &Expr, names: &BTreeSet<String>) -> bool {
    carries_derivative(e, names)
}

/// A traced function's body checked, its loops made to run in order;
/// `names` are those that carry a derivative.
fn check_block(fname: &str, block: &mut Block, line: &mut usize, names: &BTreeSet<String>, refusals: &mut Vec<(String, String)>) {
    for s in block.iter_mut() {
        match s {
            Stmt::Line(n) => *line = *n,
            Stmt::For { iter, body, parallel, .. } => {
                if matches!(iter.ty().payload(), Ty::Tensor(..)) {
                    refusals.push((fname.to_string(), format!("line {}: grad: {} walks a tensor, which is not differentiable — compute with the tensor whole", line, fname)));
                }
                *parallel = false;
                check_expr(fname, iter, *line, names, refusals);
                check_block(fname, body, line, names, refusals);
            }
            Stmt::If { cond, then, els } => {
                check_expr(fname, cond, *line, names, refusals);
                check_block(fname, then, line, names, refusals);
                if let Some(e) = els {
                    check_block(fname, e, line, names, refusals);
                }
            }
            Stmt::While { cond, body } => {
                check_expr(fname, cond, *line, names, refusals);
                check_block(fname, body, line, names, refusals);
            }
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Do(expr) | Stmt::Return(Some(expr)) => check_expr(fname, expr, *line, names, refusals),
            Stmt::Store { .. } => refusals.push((fname.to_string(), format!("line {}: grad: {} writes a fact, and a function grad differentiates only computes", line, fname))),
            _ => {}
        }
    }
}

fn check_expr(fname: &str, e: &Expr, line: usize, names: &BTreeSet<String>, refusals: &mut Vec<(String, String)>) {
    let mut refuse = |what: String| refusals.push((fname.to_string(), format!("line {}: grad: {} {}", line, fname, what)));
    match e {
        Expr::Call { target, args, .. } => {
            let name = call_name(e).unwrap_or_default();
            // (a value that carries no derivative meets any builtin: a
            // position made a float, pushed onto a table of them)
            let traced_arg = args.iter().any(|a| is_active(a, names));
            match target {
                Target::User(_) => {}
                _ if is_grad(e) => refuse("calls grad, and a function grad differentiates cannot".to_string()),
                // (the rows say which builtins a traced value may reach: a
                // row that declares its kernel on the tape — differentiated,
                // or read and never differentiated — and the constructors of
                // the tape's constants)
                _ if traced_arg && !taped(target, &name, args.len()) && !crate::table::has(&name, crate::table::Prop::Constructs) => {
                    // (a name differentiated at another count of arguments
                    // says the count: `max` of a tensor is, of two numbers not)
                    let taped = crate::table::taped();
                    let what = match args.len() {
                        _ if !taped.contains(&name.as_str()) => format!("`{}`", name),
                        1 => format!("`{}` with one argument", name),
                        n => format!("`{}` with {} arguments", name, n),
                    };
                    refuse(format!("calls {}, which is not differentiable — a function grad differentiates may use {}", what, taped.join(", ")))
                }
                _ => {}
            }
            for a in args {
                check_expr(fname, a, line, names, refusals);
            }
        }
        Expr::Effect { name, args, .. } => {
            if name != "log" {
                refuse(format!("does `{}`, and a function grad differentiates only computes", name));
            }
            for a in args {
                check_expr(fname, a, line, names, refusals);
            }
        }
        Expr::Index { base, indices, .. } => {
            // a tensor's index is differentiated: a row or a number by a
            // whole number, the numbers at a tensor of positions
            // (crow_runtime::autograd's `at` and `gather`); a position of
            // any other kind is no position of a tensor's
            if matches!(base.ty().payload(), Ty::Tensor(..)) {
                for i in indices {
                    if !matches!(i.ty().payload(), Ty::Int | Ty::Tensor(..)) {
                        refuse(format!("indexes a tensor by {}, and a tensor's position is a whole number or a tensor of them", kind_word(&i.ty())));
                    }
                }
            }
            check_expr(fname, base, line, names, refusals);
            for i in indices {
                check_expr(fname, i, line, names, refusals);
            }
        }
        Expr::Binary { op, l, r, .. } => {
            // a tensor compared with a number is its mask, a constant of
            // the tape (`where` reads it, the gradient reaches neither
            // side through it); two tensors compared are one truth of
            // their numbers, which no gradient reads
            if matches!(op.as_str(), "eq" | "ne" | "gt" | "lt" | "gte" | "lte") && matches!(l.ty().payload(), Ty::Tensor(..)) && matches!(r.ty().payload(), Ty::Tensor(..)) {
                refuse(format!("compares two tensors with `{}`, which is not differentiable (compare a tensor with a number for its mask)", op));
            }
            if op == "mod" && (is_active(l, names) || is_active(r, names)) {
                refuse("uses `%`, which is not differentiable".to_string());
            }
            check_expr(fname, l, line, names, refusals);
            check_expr(fname, r, line, names, refusals);
        }
        Expr::Unary { expr, .. } | Expr::Convert { expr, .. } | Expr::Field { base: expr, .. } | Expr::Meta { base: expr, .. } => check_expr(fname, expr, line, names, refusals),
        Expr::ListLit(items, _) => items.iter().for_each(|i| check_expr(fname, i, line, names, refusals)),
        Expr::MapLit(pairs, _) => pairs.iter().for_each(|(_, v)| check_expr(fname, v, line, names, refusals)),
        Expr::RecordLit { fields, .. } => fields.iter().for_each(|f| check_expr(fname, f, line, names, refusals)),
        Expr::Interp(pieces) => {
            for p in pieces {
                if let Piece::Expr(x) = p {
                    check_expr(fname, x, line, names, refusals);
                }
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            if matches!(iter.ty().payload(), Ty::Tensor(..)) {
                refuse("walks a tensor, which is not differentiable — compute with the tensor whole".to_string());
            }
            check_expr(fname, iter, line, names, refusals);
            let mut l = line;
            // (a collecting loop's body: checked, not made sequential here —
            // its parallel flag is on the ForExpr)
            let mut body = body.clone();
            check_block(fname, &mut body, &mut l, names, refusals);
        }
        _ => {}
    }
}
