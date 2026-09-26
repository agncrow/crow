//! Shapes: a tensor's rank and dimensions where the program states them
//! (a literal, a constructor given literal sizes), carried through the
//! element-wise operators and `*`, so an operation two known dimensions
//! make certainly wrong is refused before the program runs, in both modes
//! (second-half-design.md §3). A dimension the program computes stays
//! unknown, and only a mismatch of two known ones is refused.

use super::*;

/// A tensor's dimensions, outermost first; `None` where a dimension is
/// not known before the program runs.
pub(super) type Shape = Vec<Option<usize>>;

/// A shape as a message prints it: `[2, 3]`, `?` for an unknown dimension.
fn shown(s: &Shape) -> String {
    let dims: Vec<String> = s.iter().map(|d| d.map_or("?".to_string(), |n| n.to_string())).collect();
    format!("[{}]", dims.join(", "))
}

/// A literal size: an int the program wrote.
fn size(x: &Expr) -> Option<usize> {
    match x {
        Expr::Const(Const::Int(n), _) if *n >= 0 => Some(*n as usize),
        Expr::Convert { expr, .. } => size(expr),
        _ => None,
    }
}

/// The name a call reaches, a runtime builtin or a module's function.
fn callee(t: &Target) -> Option<&str> {
    match t {
        Target::Runtime(n) | Target::Module(_, n) => Some(n.rsplit('.').next().unwrap_or(n)),
        Target::User(_) => None,
    }
}

impl<'a> Lowerer<'a> {
    /// The shape of a lowered tensor value, when the program states it.
    pub(super) fn shape_of(&self, x: &Expr) -> Option<Shape> {
        if !matches!(x.ty().payload(), Ty::Tensor(_)) {
            return None;
        }
        match x {
            Expr::ListLit(items, _) => {
                // a nested literal is rectangular (a ragged one is refused):
                // its first row stands for every row
                let mut dims = vec![Some(items.len())];
                if let Some(first) = items.first() {
                    if let Some(inner) = self.shape_of(first) {
                        dims.extend(inner);
                    }
                }
                Some(dims)
            }
            Expr::Local(n, _) | Expr::Move(n, _) => self.shapes.get(n).cloned(),
            Expr::Convert { expr, .. } => self.shape_of(expr),
            Expr::Unary { expr, .. } => self.shape_of(expr),
            Expr::Call { target, args, .. } => {
                let dims = |xs: &[Expr]| -> Shape { xs.iter().map(size).collect() };
                match callee(target)? {
                    "zeros" | "ones" | "rand" | "randn" if !args.is_empty() => Some(dims(args)),
                    // the last argument is the value every cell holds
                    "full" if args.len() >= 2 => Some(dims(&args[..args.len() - 1])),
                    "eye" if args.len() == 1 => Some(vec![size(&args[0]), size(&args[0])]),
                    "linspace" if args.len() == 3 => Some(vec![size(&args[2])]),
                    "transpose" if args.len() == 1 => self.shape_of(&args[0]).map(|mut s| {
                        s.reverse();
                        s
                    }),
                    _ => None,
                }
            }
            Expr::Binary { op, l, r, .. } => {
                let (ls, rs) = (self.shape_of(l), self.shape_of(r));
                let both = matches!(l.ty().payload(), Ty::Tensor(_)) && matches!(r.ty().payload(), Ty::Tensor(_));
                match op.as_str() {
                    // tensor × tensor is the matrix product
                    "mul" if both => match (ls?, rs?) {
                        (a, b) if a.len() == 2 && b.len() == 2 => Some(vec![a[0], b[1]]),
                        _ => None,
                    },
                    "add" | "sub" | "elemmul" | "elemdiv" | "elempow" if both => broadcast(&ls?, &rs?).ok(),
                    // with a number, the tensor's own shape
                    "add" | "sub" | "mul" | "div" | "pow" | "elemmul" | "elemdiv" | "elempow" => ls.or(rs),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// An operation between two tensors whose known dimensions cannot meet,
    /// refused before the program runs with the message it would stop with.
    pub(super) fn check_shapes(&mut self, op: &BinOp, l: &Expr, r: &Expr) {
        let both = matches!(l.ty().payload(), Ty::Tensor(_)) && matches!(r.ty().payload(), Ty::Tensor(_));
        if !both {
            return;
        }
        let (Some(a), Some(b)) = (self.shape_of(l), self.shape_of(r)) else { return };
        match op {
            BinOp::Mul => {
                if a.len() != 2 || b.len() != 2 {
                    self.refuse(format!(
                        "`*` between tensors is matrix multiplication and needs two 2-D tensors, got shapes {} and {} (use .* for an element-wise product)",
                        shown(&a),
                        shown(&b)
                    ));
                } else if let (Some(k1), Some(k2)) = (a[1], b[0]) {
                    if k1 != k2 {
                        self.refuse(format!(
                            "matmul: incompatible shapes {} × {} — the first's columns ({}) must be the second's rows ({})",
                            shown(&a),
                            shown(&b),
                            k1,
                            k2
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
                        "`{}` of tensors of shapes {} and {}: dimension {} is {} in one and {} in the other — an element-wise operation needs each pair equal, or one of them 1",
                        sign,
                        shown(&a),
                        shown(&b),
                        i + 1,
                        a[a.len() + i - a.len().max(b.len())].map_or(0, |d| d),
                        b[b.len() + i - a.len().max(b.len())].map_or(0, |d| d)
                    ));
                }
            }
            _ => {}
        }
    }

    /// What an assignment says of a local's shape: its value's, where
    /// known; a later value of another shape (or an unknown one) leaves it
    /// unknown from there on.
    pub(super) fn note_shape(&mut self, name: &str, value: &Expr, first: bool) {
        match self.shape_of(value) {
            Some(s) if first || self.shapes.get(name) == Some(&s) => {
                self.shapes.insert(name.to_string(), s);
            }
            _ => {
                self.shapes.remove(name);
            }
        }
    }
}

/// Two shapes broadcast as the element-wise operators do, aligned from the
/// innermost dimension: each pair equal, or one of them 1. The result, or
/// the dimension (of the longer shape, 0-based) where two known ones meet
/// neither way. An unknown dimension meets anything, and stays unknown.
fn broadcast(a: &Shape, b: &Shape) -> Result<Shape, usize> {
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
