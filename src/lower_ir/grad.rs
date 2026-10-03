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

/// The builtins a traced function may call with a traced value.
pub(crate) const DIFFERENTIABLE: &[&str] = &[
    "sum", "mean", "exp", "ln", "sqrt", "tanh", "abs", "relu", "sigmoid", "softmax", "mse", "bce", "crossentropy", "linear", "transpose", "reshape", "flatten",
    // read, never differentiated: a length, a conversion of a number
    "len", "float", "int", "log",
];

/// The builtins that make constants: their arguments are sizes or lists.
pub(crate) const CONSTRUCTORS: &[&str] = &["zeros", "ones", "full", "eye", "randn", "tensor", "linspace", "range"];

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
        for f in &mut part.fns {
            if !traced.contains(&f.name) {
                continue;
            }
            // parameters: all tensors, none a record or a list; a number
            // beside a tensor is two kinds
            if tensor_targets.contains(&f.name) {
                let kinds: Vec<&str> = f.params.iter().map(|(_, t)| kind_word(t)).collect();
                if kinds.iter().any(|k| *k != "a tensor") {
                    let line = first_line(&f.body);
                    refusals.push((f.name.clone(), format!(
                        "line {}: grad[{}]: a function grad differentiates takes tensors, or numbers alone — {} takes {}",
                        line, f.name, f.name, kinds.join(", ")
                    )));
                }
            }
            let mut line = first_line(&f.body);
            check_block(&f.name, &mut f.body, &mut line, &mut refusals);
        }
        part.traced = traced;
    }
    refusals
}

fn kind_word(t: &Ty) -> &'static str {
    match t.payload() {
        Ty::Tensor(..) => "a tensor",
        Ty::Int | Ty::Float => "a number",
        Ty::Str | Ty::Markup => "text",
        Ty::Bool => "a bool",
        Ty::List(_) => "a list",
        Ty::Map(_) => "a map",
        Ty::Record(_) | Ty::OptRecord(_) => "a record",
        Ty::Frame(_) => "a frame",
        _ => "a value of a kind grad does not differentiate",
    }
}

/// The functions `grad` is handed, anywhere in the part.
fn targets_of(part: &Part) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut note = |e: &Expr| {
        if let Expr::Call { args, .. } = e {
            if call_name(e).as_deref() == Some("grad") {
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
                _ if name == "grad" => refuse("calls grad, and a function grad differentiates cannot".to_string()),
                _ if traced_arg && !DIFFERENTIABLE.contains(&name.as_str()) && !CONSTRUCTORS.contains(&name.as_str()) => {
                    refuse(format!("calls `{}`, which is not differentiable — a function grad differentiates may use {}", name, DIFFERENTIABLE.join(", ")))
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
            if matches!(base.ty().payload(), Ty::Tensor(..)) {
                refuse("indexes a tensor, which is not differentiable — compute with the tensor whole".to_string());
            }
            check_expr(fname, base, line, refusals);
            for i in indices {
                check_expr(fname, i, line, refusals);
            }
        }
        Expr::Binary { op, l, r, .. } => {
            if matches!(op.as_str(), "eq" | "ne" | "gt" | "lt" | "gte" | "lte") && (matches!(l.ty().payload(), Ty::Tensor(..)) || matches!(r.ty().payload(), Ty::Tensor(..))) {
                refuse(format!("compares tensors with `{}`, which is not differentiable (a comparison of two numbers decides, with no gradient through it)", op));
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
