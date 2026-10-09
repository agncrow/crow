//! A function's vectors kept as plain `Vec<f64>` in the compiled program.
//!
//! A one-axis tensor of floats or ints — `[0.0, 4.84, 8.34]` — is a
//! `CrowTensor`: shared storage behind a reference count, a shape and
//! strides, so a value is a value however many hold it. Reading one number
//! goes through all of that, and writing one checks that nothing else holds
//! the storage. A local the body only reads and sets by index and measures
//! needs none of it: it is kept as a plain vector for the function, made
//! from the tensor where the local is made (a parameter as the body starts),
//! and made back into a tensor only where the body uses it as a value — a
//! call's argument, the answer, a loop's list — which the rule below allows
//! only outside every loop, so the pass over it is never repeated. n-body,
//! written over lists, had run 13× slower than the same steps over
//! variables; this is what closes it.

use super::*;
use crate::ir::{Expr, Fn, Piece, Stmt, Target};
use std::collections::{BTreeSet, HashMap};

/// A type the rule keeps as a plain vector: a tensor of one axis whose
/// numbers are floats or ints (a tensor stores both as `f64`).
pub(super) fn is_vector(t: &Ty) -> bool {
    matches!(as_inferred(t), Ty::Tensor(el, Some(dims)) if dims.len() == 1 && matches!(*el, Ty::Float | Ty::Int))
}

/// The locals of a function kept as plain vectors: parameters and bindings
/// of a vector type whose every use is a read or a write of one number by
/// index, its length, or — outside every loop — a use as a value.
pub(super) fn plain_vectors(f: &Fn) -> BTreeSet<String> {
    let mut ok: HashMap<String, bool> = HashMap::new();
    for (p, t) in &f.params {
        ok.insert(p.clone(), is_vector(t));
    }
    bindings(&f.body, &mut ok);
    let mut w = Walk { ok };
    w.block(&f.body, 0);
    w.ok.into_iter().filter(|(_, keep)| *keep).map(|(n, _)| n).collect()
}

/// Every name a body binds, kept only while each of its bindings is of a
/// vector type (a name bound twice to two types is left alone).
fn bindings(b: &[Stmt], ok: &mut HashMap<String, bool>) {
    for s in b {
        match s {
            Stmt::Let { var, ty, .. } => {
                let v = is_vector(ty);
                let e = ok.entry(var.clone()).or_insert(v);
                *e &= v;
            }
            Stmt::If { then, els, .. } => {
                bindings(then, ok);
                if let Some(e) = els {
                    bindings(e, ok);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => bindings(body, ok),
            _ => {}
        }
    }
}

struct Walk {
    ok: HashMap<String, bool>,
}

impl Walk {
    fn kept(&self, v: &str) -> bool {
        self.ok.get(v).copied().unwrap_or(false)
    }

    /// A use of the local as a value: allowed outside every loop.
    fn as_value(&mut self, v: &str, depth: usize) {
        if depth > 0
            && let Some(k) = self.ok.get_mut(v)
        {
            *k = false;
        }
    }

    fn block(&mut self, b: &[Stmt], depth: usize) {
        for s in b {
            self.stmt(s, depth);
        }
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) {
        match s {
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => self.expr(expr, depth),
            Stmt::SetPath { var, path, expr } => {
                if self.kept(var) && !matches!(path.as_slice(), [PathStep::Index(_)]) {
                    self.ok.insert(var.clone(), false);
                }
                for p in path {
                    if let PathStep::Index(i) = p {
                        self.expr(i, depth);
                    }
                }
                self.expr(expr, depth);
            }
            Stmt::If { cond, then, els } => {
                self.expr(cond, depth);
                self.block(then, depth);
                if let Some(e) = els {
                    self.block(e, depth);
                }
            }
            // a while loop's condition runs every round
            Stmt::While { cond, body } => {
                self.expr(cond, depth + 1);
                self.block(body, depth + 1);
            }
            // a for loop's list is made once, before its rounds
            Stmt::For { iter, body, .. } => {
                self.expr(iter, depth);
                self.block(body, depth + 1);
            }
            Stmt::Return(Some(e)) => self.expr(e, depth),
            Stmt::Propagate { values, .. } => {
                for v in values {
                    self.as_value(v, depth);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr, depth: usize) {
        match e {
            Expr::Local(v, _) | Expr::Move(v, _) => {
                if self.kept(v) {
                    self.as_value(v, depth);
                }
            }
            // a number read by index: the vector read where it lies
            Expr::Index { base, indices, .. } if indices.len() == 1 && matches!(unwrapped(base), Expr::Local(v, _) | Expr::Move(v, _) if self.kept(v)) => {
                self.expr(&indices[0], depth);
            }
            // its length, measured where it lies
            Expr::Call { target: Target::Row(row), args, .. } if row.id == "core.len_tensor" && args.len() == 1 && matches!(unwrapped(&args[0]), Expr::Local(v, _) | Expr::Move(v, _) if self.kept(v)) => {}
            Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => {
                for a in args {
                    self.expr(a, depth);
                }
            }
            Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => self.expr(expr, depth),
            Expr::Binary { l, r, .. } => {
                self.expr(l, depth);
                self.expr(r, depth);
            }
            Expr::Index { base, indices, .. } => {
                self.expr(base, depth);
                for i in indices {
                    self.expr(i, depth);
                }
            }
            Expr::Field { base, .. } | Expr::Meta { base, .. } => self.expr(base, depth),
            Expr::Interp(pieces) => {
                for p in pieces {
                    if let Piece::Expr(x) = p {
                        self.expr(x, depth);
                    }
                }
            }
            Expr::ListLit(items, _) => {
                for x in items {
                    self.expr(x, depth);
                }
            }
            Expr::MapLit(pairs, _) => {
                for (_, x) in pairs {
                    self.expr(x, depth);
                }
            }
            Expr::RecordLit { fields, .. } => {
                for x in fields {
                    self.expr(x, depth);
                }
            }
            Expr::Tree { attrs, children, .. } => {
                for (_, x) in attrs {
                    self.expr(x, depth);
                }
                for x in children {
                    self.expr(x, depth);
                }
            }
            Expr::ForExpr { iter, body, .. } => {
                self.expr(iter, depth);
                self.block(body, depth + 1);
            }
            Expr::Const(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) | Expr::FnRef(..) => {}
        }
    }
}

/// An expression with an unwrap of a checked value looked through.
fn unwrapped(e: &Expr) -> &Expr {
    match e {
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => expr,
        other => other,
    }
}

impl Emitter {
    /// The plain vector an expression names, if it names one.
    pub(super) fn plain_vector<'e>(&self, e: &'e Expr) -> Option<&'e str> {
        match unwrapped(e) {
            Expr::Local(v, _) | Expr::Move(v, _) if self.plain_vecs.contains(v.as_str()) => Some(v.as_str()),
            _ => None,
        }
    }

    /// A plain vector used as a value: the tensor made from it.
    pub(super) fn vector_as_tensor(v: &str) -> String {
        format!("crow_runtime::tensor::CrowTensor::from_vec({v}.clone(), vec![{v}.len()])")
    }
}
