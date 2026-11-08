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

use std::collections::BTreeSet;

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
        let mut traced = BTreeSet::new();
        for t in &tensor_targets {
            traced.insert(t.clone());
            traced.extend(reached_from(part, t));
        }
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
            let mut line = first_line(&f.body);
            check_block(&f.name, &mut f.body, &mut line, &mut refusals);
            in_order(&mut f.body);
        }
        part.traced = traced;
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

fn traced_value(e: &Expr) -> bool {
    matches!(e.ty().payload(), Ty::Tensor(..) | Ty::Float)
}

/// A traced function's body checked, its loops made to run in order.
fn check_block(fname: &str, block: &mut Block, line: &mut usize, refusals: &mut Vec<(String, String)>) {
    for s in block.iter_mut() {
        match s {
            Stmt::Line(n) => *line = *n,
            Stmt::For { iter, body, parallel, .. } => {
                if matches!(iter.ty().payload(), Ty::Tensor(..)) {
                    refusals.push((fname.to_string(), format!("line {}: grad: {} walks a tensor, which is not differentiable — compute with the tensor whole", line, fname)));
                }
                *parallel = false;
                check_expr(fname, iter, *line, refusals);
                check_block(fname, body, line, refusals);
            }
            Stmt::If { cond, then, els } => {
                check_expr(fname, cond, *line, refusals);
                check_block(fname, then, line, refusals);
                if let Some(e) = els {
                    check_block(fname, e, line, refusals);
                }
            }
            Stmt::While { cond, body } => {
                check_expr(fname, cond, *line, refusals);
                check_block(fname, body, line, refusals);
            }
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Do(expr) | Stmt::Return(Some(expr)) => check_expr(fname, expr, *line, refusals),
            Stmt::Store { .. } => refusals.push((fname.to_string(), format!("line {}: grad: {} writes a fact, and a function grad differentiates only computes", line, fname))),
            _ => {}
        }
    }
}

fn check_expr(fname: &str, e: &Expr, line: usize, refusals: &mut Vec<(String, String)>) {
    let mut refuse = |what: String| refusals.push((fname.to_string(), format!("line {}: grad: {} {}", line, fname, what)));
    match e {
        Expr::Call { target, args, .. } => {
            let name = call_name(e).unwrap_or_default();
            let traced_arg = args.iter().any(traced_value);
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
                check_expr(fname, a, line, refusals);
            }
        }
        Expr::Effect { name, args, .. } => {
            if name != "log" {
                refuse(format!("does `{}`, and a function grad differentiates only computes", name));
            }
            for a in args {
                check_expr(fname, a, line, refusals);
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
            check_expr(fname, base, line, refusals);
            for i in indices {
                check_expr(fname, i, line, refusals);
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
            if op == "mod" && (traced_value(l) || traced_value(r)) {
                refuse("uses `%`, which is not differentiable".to_string());
            }
            check_expr(fname, l, line, refusals);
            check_expr(fname, r, line, refusals);
        }
        Expr::Unary { expr, .. } | Expr::Convert { expr, .. } | Expr::Field { base: expr, .. } | Expr::Meta { base: expr, .. } => check_expr(fname, expr, line, refusals),
        Expr::ListLit(items, _) => items.iter().for_each(|i| check_expr(fname, i, line, refusals)),
        Expr::MapLit(pairs, _) => pairs.iter().for_each(|(_, v)| check_expr(fname, v, line, refusals)),
        Expr::RecordLit { fields, .. } => fields.iter().for_each(|f| check_expr(fname, f, line, refusals)),
        Expr::Interp(pieces) => {
            for p in pieces {
                if let Piece::Expr(x) = p {
                    check_expr(fname, x, line, refusals);
                }
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            if matches!(iter.ty().payload(), Ty::Tensor(..)) {
                refuse("walks a tensor, which is not differentiable — compute with the tensor whole".to_string());
            }
            check_expr(fname, iter, line, refusals);
            let mut l = line;
            // (a collecting loop's body: checked, not made sequential here —
            // its parallel flag is on the ForExpr)
            let mut body = body.clone();
            check_block(fname, &mut body, &mut l, refusals);
        }
        _ => {}
    }
}
