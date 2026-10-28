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

/// A vector of whole numbers.
fn of_ints(t: &Ty) -> bool {
    matches!(as_inferred(t), Ty::Tensor(el, Some(dims)) if dims.len() == 1 && *el == Ty::Int)
}

/// The plain vectors of whole numbers among them, kept as `Vec<i64>`: each
/// binding a vector of ints and every number written to it an int (a
/// float pushed onto one keeps it a vector of floats). Held as floats, a
/// brainfuck interpreter's tape had turned each cell into a float and back
/// at every step, and run at a third of Rust's speed.
pub(super) fn int_vectors(f: &Fn, plain: &BTreeSet<String>) -> BTreeSet<String> {
    let mut ints: HashMap<String, bool> = HashMap::new();
    for (p, t) in &f.params {
        if plain.contains(p) {
            ints.insert(p.clone(), of_ints(t));
        }
    }
    fn walk(b: &[Stmt], plain: &BTreeSet<String>, ints: &mut HashMap<String, bool>) {
        for s in b {
            match s {
                Stmt::Let { var, ty, .. } if plain.contains(var) => {
                    let e = ints.entry(var.clone()).or_insert(true);
                    *e &= of_ints(ty);
                }
                Stmt::Set { var, expr } if plain.contains(var) => {
                    let whole = match pushed(var, expr) {
                        Some(n) => *n.ty().payload() == Ty::Int,
                        None => of_ints(&expr.ty()),
                    };
                    let e = ints.entry(var.clone()).or_insert(true);
                    *e &= whole;
                }
                Stmt::SetPath { var, expr, .. } if plain.contains(var) => {
                    let e = ints.entry(var.clone()).or_insert(true);
                    *e &= *expr.ty().payload() == Ty::Int;
                }
                Stmt::If { then, els, .. } => {
                    walk(then, plain, ints);
                    if let Some(e) = els {
                        walk(e, plain, ints);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => walk(body, plain, ints),
                _ => {}
            }
        }
    }
    walk(&f.body, plain, &mut ints);
    ints.into_iter().filter(|(_, whole)| *whole).map(|(n, _)| n).collect()
}

/// The vector parameters the body only reads — each use a number read by
/// index or its length, the parameter never bound anew — read through a
/// view of its numbers made as the body starts (borrowed where the tensor
/// is one contiguous run): a read through the tensor checks its shape,
/// strides and storage at every number (a brainfuck interpreter's program
/// and jump table, a binary search's sorted list).
pub(super) fn read_views(f: &Fn, plain: &BTreeSet<String>) -> BTreeSet<String> {
    let mut ok: HashMap<String, bool> = HashMap::new();
    for (p, t) in &f.params {
        if is_vector(t) && !plain.contains(p) {
            ok.insert(p.clone(), true);
        }
    }
    if ok.is_empty() {
        return BTreeSet::new();
    }
    let mut w = Walk { ok, written: BTreeSet::new() };
    // (as deep as a loop's body: a use as a value anywhere disqualifies)
    w.block(&f.body, 1);
    let mut rebound = BTreeSet::new();
    crate::ir::visit_stmts(&f.body, &mut |s| match s {
        Stmt::Let { var, .. } | Stmt::Set { var, .. } | Stmt::SetPath { var, .. } | Stmt::For { var, .. } => {
            rebound.insert(var.clone());
        }
        _ => {}
    });
    crate::ir::visit_block(&f.body, &mut |e| {
        if let Expr::ForExpr { var, .. } = e {
            rebound.insert(var.clone());
        }
    });
    w.ok.into_iter().filter(|(n, keep)| *keep && !w.written.contains(n) && !rebound.contains(n)).map(|(n, _)| n).collect()
}

/// The locals of a function kept as plain vectors: parameters and bindings
/// of a vector type the body writes — a number set by index, a number
/// pushed (`x = push[x, n]`) — whose every use is a read or a write of one
/// number by index, a push, its length, or — outside every loop — a use as
/// a value. (One the body only reads stays the tensor it is: making the
/// vector had copied it at every call, and pidigits' comparisons read one
/// or two limbs of numbers thousands long.)
pub(super) fn plain_vectors(f: &Fn) -> BTreeSet<String> {
    let mut ok: HashMap<String, bool> = HashMap::new();
    for (p, t) in &f.params {
        ok.insert(p.clone(), is_vector(t));
    }
    bindings(&f.body, &mut ok);
    let mut w = Walk { ok, written: BTreeSet::new() };
    w.block(&f.body, 0);
    let written = w.written;
    w.ok.into_iter().filter(|(n, keep)| *keep && written.contains(n)).map(|(n, _)| n).collect()
}

/// `push[x, n]` of the local `x` onto its numbers: the number.
pub(super) fn pushed<'e>(x: &str, e: &'e Expr) -> Option<&'e Expr> {
    match unwrapped(e) {
        Expr::Call { target: Target::Row(row), args, .. } if row.id == "core.push_number" && args.len() == 2 && matches!(unwrapped(&args[0]), Expr::Local(v, _) | Expr::Move(v, _) if v == x) => Some(&args[1]),
        _ => None,
    }
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
    /// The locals the body writes: by index, or by a push.
    written: BTreeSet<String>,
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
            // a number pushed onto the local: a push onto the vector
            Stmt::Set { var, expr } if self.kept(var) && pushed(var, expr).is_some() => {
                self.written.insert(var.clone());
                if let Some(n) = pushed(var, expr) {
                    self.expr(n, depth);
                }
            }
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => self.expr(expr, depth),
            Stmt::SetPath { var, path, expr, .. } => {
                if self.kept(var) && !matches!(path.as_slice(), [PathStep::Index(_)]) {
                    self.ok.insert(var.clone(), false);
                }
                self.written.insert(var.clone());
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

    /// The view of a parameter's numbers an expression reads through, if
    /// it names a parameter read so (`read_views`).
    pub(super) fn read_view(&self, e: &Expr) -> Option<String> {
        match unwrapped(e) {
            Expr::Local(v, _) | Expr::Move(v, _) if self.read_vecs.contains(v.as_str()) => Some(format!("{}__v", v)),
            _ => None,
        }
    }

    /// A plain vector used as a value: the tensor made from it.
    pub(super) fn vector_as_tensor(&self, v: &str) -> String {
        if self.int_vecs.contains(v) {
            return format!("crow_runtime::core::tensor_of_ints(&{v})");
        }
        format!("crow_runtime::tensor::CrowTensor::from_vec({v}.clone(), vec![{v}.len()])")
    }

    /// A plain vector's Rust type, and a tensor's numbers as it holds them.
    pub(super) fn vector_of(&self, v: &str, tensor: &str) -> (&'static str, String) {
        if self.int_vecs.contains(v) {
            ("Vec<i64>", format!("crow_runtime::core::ints_of(&({}))", tensor))
        } else {
            ("Vec<f64>", format!("({}).to_vec()", tensor))
        }
    }

    /// A number as a plain vector holds it: a float, or a whole number as a
    /// tensor holds it (through its float).
    pub(super) fn vector_number(&self, v: &str, number: &str) -> String {
        if self.int_vecs.contains(v) {
            format!("crow_runtime::core::held_int({})", number)
        } else {
            format!("({}) as f64", number)
        }
    }
}
