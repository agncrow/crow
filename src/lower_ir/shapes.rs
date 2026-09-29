//! Shapes in the type (second-half-design.md §3): a tensor's rank and
//! dimensions where the program states them — a literal, a constructor
//! given literal sizes — carried in its type through the element-wise
//! operators, `*`, `transpose` and `reshape`, a local reassigned (bound anew,
//! as a frame's columns are), and the functions a tensor is handed to and
//! returned from, so an operation two known dimensions make certainly wrong
//! is refused before the program runs, in both modes. A dimension the
//! program computes stays unknown; a function's parameter has the
//! dimensions its calls agree on (a shape is no reason for a copy of the
//! function: every tensor compiles alike); and only a mismatch of two known
//! dimensions is refused.

use super::*;
use crate::type_infer::{shown_dims, Dims};

/// A literal size: an int the program wrote.
fn size(x: &Expr) -> Option<usize> {
    match x {
        Expr::Const(Const::Int(n), _) if *n >= 0 => Some(*n as usize),
        Expr::Convert { expr, .. } => size(expr),
        _ => None,
    }
}

/// A type with a tensor's shape set: a tensor's, or a failing tensor's,
/// dimensions; any other type as it is.
pub(super) fn shaped(ty: Ty, dims: Option<Dims>) -> Ty {
    match ty {
        Ty::Tensor(elem, _) => Ty::Tensor(elem, dims),
        Ty::Result(inner) => Ty::Result(Box::new(shaped(*inner, dims))),
        other => other,
    }
}

/// A list literal's shape: its length, then its first item's (a nested
/// literal is rectangular — a ragged one is refused — so its first row
/// stands for every row). Unknown where the first item's is.
pub(super) fn literal_dims(items: &[Expr]) -> Option<Dims> {
    let mut dims = vec![Some(items.len())];
    if let Some(first) = items.first() {
        dims.extend(item_dims(first)?);
    }
    Some(dims)
}

/// An item's own dimensions: none for a number, a literal's or a tensor's,
/// unknown for anything else.
fn item_dims(x: &Expr) -> Option<Dims> {
    match x {
        Expr::ListLit(items, _) => literal_dims(items),
        Expr::Convert { expr, .. } => item_dims(expr),
        x => match x.ty().payload() {
            Ty::Int | Ty::Float => Some(Vec::new()),
            Ty::Tensor(_, dims) => dims.clone(),
            _ => None,
        },
    }
}

/// The shape a builtin's tensor has from what the program states: the
/// constructors given literal sizes, and what `transpose` and `reshape`
/// make of a known shape.
pub(super) fn call_dims(name: &str, args: &[Expr]) -> Option<Dims> {
    let sizes = |xs: &[Expr]| -> Dims { xs.iter().map(size).collect() };
    match name {
        "zeros" | "ones" | "randn" if !args.is_empty() => Some(sizes(args)),
        // the last argument is the value every cell holds
        "full" if args.len() >= 2 => Some(sizes(&args[..args.len() - 1])),
        "eye" if args.len() == 1 => Some(vec![size(&args[0]), size(&args[0])]),
        "linspace" if args.len() == 3 => Some(vec![size(&args[2])]),
        // the last two axes swapped (the runtime's CrowTensor::transpose)
        "transpose" if args.len() == 1 => args[0].ty().tensor_dims().filter(|d| d.len() >= 2).map(|d| {
            let mut d = d.clone();
            let n = d.len();
            d.swap(n - 2, n - 1);
            d
        }),
        "reshape" if args.len() >= 2 => Some(sizes(&args[1..])),
        _ => None,
    }
}

/// Whether an operand is a tensor.
fn tensor(x: &Expr) -> bool {
    matches!(x.ty().payload(), Ty::Tensor(..))
}

/// The shape of an operation on tensors: the matrix product's, the
/// element-wise operators' by broadcasting, and a tensor's own with a
/// number.
pub(super) fn binary_dims(op: &BinOp, l: &Expr, r: &Expr) -> Option<Dims> {
    let (ls, rs) = (l.ty().tensor_dims().cloned(), r.ty().tensor_dims().cloned());
    match op {
        // tensor × tensor is the matrix product
        BinOp::Mul if tensor(l) && tensor(r) => match (ls?, rs?) {
            (a, b) if a.len() == 2 && b.len() == 2 => Some(vec![a[0], b[1]]),
            _ => None,
        },
        BinOp::Add | BinOp::Sub | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow if tensor(l) && tensor(r) => broadcast(&ls?, &rs?).ok(),
        // with a number, the tensor's own shape
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow => {
            if tensor(l) {
                ls
            } else {
                rs
            }
        }
        _ => None,
    }
}

impl<'a> Lowerer<'a> {
    /// A tensor local a part of which is assigned: its shape is no longer
    /// stated from there on, so nothing is refused of it.
    pub(super) fn forget_shape(&mut self, var: &str) {
        fn forget(t: &CrowType) -> CrowType {
            match t {
                CrowType::NumArray(elem, _) => CrowType::NumArray(elem.clone(), None),
                CrowType::Result(inner) => CrowType::Result(Box::new(forget(inner))),
                other => other.clone(),
            }
        }
        if let Some(t) = self.ctx.vars.get(var) {
            let t = forget(t);
            self.ctx.vars.insert(var.to_string(), t);
        }
    }

    /// An operation between two tensors whose known dimensions cannot meet,
    /// refused before the program runs with the message it would stop with
    /// — once the program's calls are all read, since a parameter's shape
    /// is what they agree on.
    pub(super) fn check_shapes(&mut self, op: &BinOp, l: &Expr, r: &Expr) {
        if self.pass != Pass::Final || !(tensor(l) && tensor(r)) {
            return;
        }
        let (Some(a), Some(b)) = (l.ty().tensor_dims().cloned(), r.ty().tensor_dims().cloned()) else { return };
        // where a parameter's shape came from: the calls handing it
        let handed = match [self.handed_as(l), self.handed_as(r)] {
            [Some((f, p)), Some((_, q))] if p != q => format!(" (the tensors {}[] is handed as `{}` and `{}`)", f, p, q),
            [Some((f, p)), _] | [None, Some((f, p))] => format!(" (the tensor {}[] is handed as `{}`)", f, p),
            [None, None] => String::new(),
        };
        match op {
            BinOp::Mul => {
                if a.len() != 2 || b.len() != 2 {
                    self.refuse(format!(
                        "`*` between tensors is matrix multiplication and needs two 2-D tensors, got shapes {} and {} (use .* for an element-wise product){}",
                        shown_dims(&a),
                        shown_dims(&b),
                        handed
                    ));
                } else if let (Some(k1), Some(k2)) = (a[1], b[0]) {
                    if k1 != k2 {
                        self.refuse(format!(
                            "matmul: incompatible shapes {} × {} — the first's columns ({}) must be the second's rows ({}){}",
                            shown_dims(&a),
                            shown_dims(&b),
                            k1,
                            k2,
                            handed
                        ));
                    }
                }
            }
            BinOp::Add | BinOp::Sub | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow => {
                if let Err(i) = broadcast(&a, &b) {
                    let sign = match op {
                        BinOp::Add => "+",
                        BinOp::Sub => "-",
                        BinOp::ElemMul => ".*",
                        BinOp::ElemDiv => "./",
                        _ => ".^",
                    };
                    self.refuse(format!(
                        "`{}` of tensors of shapes {} and {}: dimension {} is {} in one and {} in the other — an element-wise operation needs each pair equal, or one of them 1{}",
                        sign,
                        shown_dims(&a),
                        shown_dims(&b),
                        i + 1,
                        a[a.len() + i - a.len().max(b.len())].map_or(0, |d| d),
                        b[b.len() + i - a.len().max(b.len())].map_or(0, |d| d),
                        handed
                    ));
                }
            }
            _ => {}
        }
    }
}

/// Two shapes broadcast as the element-wise operators do, aligned from the
/// innermost dimension: each pair equal, or one of them 1. The result, or
/// the dimension (of the longer shape, 0-based) where two known ones meet
/// neither way. An unknown dimension meets anything, and stays unknown.
fn broadcast(a: &Dims, b: &Dims) -> Result<Dims, usize> {
    let n = a.len().max(b.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let da = if i < n - a.len() { Some(1) } else { a[i - (n - a.len())] };
        let db = if i < n - b.len() { Some(1) } else { b[i - (n - b.len())] };
        out.push(match (da, db) {
            (Some(x), Some(y)) if x == y => Some(x),
            (Some(1), other) | (other, Some(1)) => other,
            (Some(_), Some(_)) => return Err(i),
            _ => None,
        });
    }
    Ok(out)
}
