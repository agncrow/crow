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
//! dimensions is refused. A tensor's rank is always in its type: every
//! builtin's answer has one (declared on its row, `table::Shape`, read by `declared`), and a loop over a tensor walks
//! rows or numbers, an index reads a row or a number, by it — refused
//! where the lowering does not know it (`check_rank_known`), so both
//! backends decide alike (spec, Arrays, Design Note "rank is in the type").

use super::*;
use crate::table::{Dim, Kind, Row, Shape};
use crate::type_infer::{shown_dims, Dims};

/// The shape a row declares for its answer (`table::Shape`), read from
/// what the program states in the call's arguments: a literal size, an
/// argument's own shape. None where the row states nothing, or what it
/// reads is not there.
pub(super) fn declared(row: &Row, args: &[Expr]) -> Option<Dims> {
    let n = args.len();
    match row.shape {
        Shape::Open | Shape::Pieces(_) => None,
        Shape::Same(i) => dims(args, i),
        Shape::Broadcast(is) => {
            let mut out: Option<Dims> = None;
            for &i in is {
                let d = dims(args, i)?;
                out = Some(match out {
                    Some(o) => broadcast(&o, &d).ok()?,
                    None => d,
                });
            }
            out
        }
        Shape::Sizes(from, but) => (n > from + but).then(|| args[from..n - but].iter().map(size).collect()),
        Shape::Dims(ds) => Some(ds.iter().map(|d| dim(args, *d)).collect()),
        Shape::Reduce(i, axis) => dims(args, i).map(|mut d| match args.get(axis).and_then(size) {
            Some(ax) if ax < d.len() => {
                d.remove(ax);
                d
            }
            _ => vec![None; d.len().saturating_sub(1)],
        }),
        Shape::Like(i, first) => dims(args, i).map(|mut d| {
            if let Some(x) = d.first_mut() {
                *x = dim(args, first);
            }
            d
        }),
        Shape::Along(i, axis) => dims(args, i).map(|mut d| {
            let ax = axis.and_then(|j| args.get(j)).and_then(size).unwrap_or(0);
            if ax < d.len() {
                d[ax] = None;
            }
            d
        }),
        Shape::Flat(i) => Some(vec![dims(args, i).and_then(|d| d.iter().copied().try_fold(1usize, |p, x| x.map(|x| p * x)))]),
        Shape::Transposed(i) => dims(args, i).or_else(|| stacked(args, i)).filter(|d| d.len() >= 2).map(|mut d| {
            let n = d.len();
            d.swap(n - 2, n - 1);
            d
        }),
        Shape::Stacked(i) => stacked(args, i).or_else(|| dims(args, i)),
        Shape::Restacked(i) => stacked(args, i).or_else(|| dims(args, i)).map(|d| vec![None; d.len()]),
        Shape::Diag(i) => match dims(args, i) {
            Some(d) if d.len() == 1 => Some(vec![d[0], d[0]]),
            Some(d) if d.len() == 2 => Some(vec![None]),
            _ => None,
        },
        Shape::Items(i) => items(args, i),
    }
}

/// Where the lowering picked no row — one its values pick as the program
/// runs — the shape the rows of the name that answer a tensor agree on
/// (`abs` of a list its items unknown: every tensor row of `abs` keeps its
/// argument's shape); none where they differ.
pub(super) fn agreed(module: Option<&str>, name: &str, args: &[Expr]) -> Option<Dims> {
    let mut answer: Option<Option<Dims>> = None;
    for row in crate::table::ROWS.iter().filter(|r| r.name == name && module.is_none_or(|m| r.module == m) && matches!(r.ret, Kind::Tensor | Kind::Tensors)) {
        let d = declared(row, args);
        match &answer {
            None => answer = Some(d),
            Some(a) if *a == d => {}
            Some(_) => return None,
        }
    }
    answer.flatten()
}

/// The shapes of the pieces a row's answer holds, by position (`svd`'s
/// U, singular values and Vt): a list holds items of one type, so the
/// list's item says nothing of them; an index by a literal position reads
/// the piece's own shape.
pub(super) fn declared_pieces(row: &Row, args: &[Expr]) -> Option<Vec<Option<Dims>>> {
    match row.shape {
        Shape::Pieces(pieces) => Some(pieces.iter().map(|p| Some(p.iter().map(|d| dim(args, *d)).collect())).collect()),
        _ => None,
    }
}

/// One declared dimension, read from the arguments.
fn dim(args: &[Expr], d: Dim) -> Option<usize> {
    match d {
        Dim::Open => None,
        Dim::Size(i) => args.get(i).and_then(size),
        Dim::Of(i, j) => dims(args, i).and_then(|d| d.get(j).copied()).flatten(),
    }
}

/// The expression whose type says an argument's shape: a list converted
/// to a tensor where it is handed (`tensor(move xs)`) says its shape as
/// the list it was.
fn seen(a: &Expr) -> &Expr {
    match a {
        Expr::Convert { expr, ty, .. } if matches!(ty.payload(), Ty::Tensor(_, None)) => seen(expr),
        _ => a,
    }
}

/// Argument i's shape where a tensor is taken: a tensor's own; a list of
/// numbers is a vector (the order of a list computed in a function —
/// `argsort[vals]` — is one, and an index into it reads a number), a
/// list of lists of numbers a matrix, a column a vector, a frame and a
/// sparse matrix matrices.
fn dims(args: &[Expr], i: usize) -> Option<Dims> {
    args.get(i).map(seen).and_then(|a| match a.ty().payload() {
        Ty::List(item) => match item.payload() {
            Ty::Int | Ty::Float => Some(vec![None]),
            Ty::List(inner) if matches!(inner.payload(), Ty::Int | Ty::Float) => Some(vec![None, None]),
            _ => None,
        },
        Ty::Column(_) => Some(vec![None]),
        Ty::Frame(_) | Ty::Sparse => Some(vec![None, None]),
        _ => a.ty().tensor_dims().cloned(),
    })
}

/// The shape of argument i's items, a list of tensors.
fn items(args: &[Expr], i: usize) -> Option<Dims> {
    args.get(i).map(seen).and_then(|a| match a.ty().payload() {
        Ty::List(item) => item.payload().tensor_dims().cloned(),
        _ => None,
    })
}

/// Argument i stacked: one dimension more than its rows.
fn stacked(args: &[Expr], i: usize) -> Option<Dims> {
    args.get(i).map(seen).and_then(|a| stacked_dims(&a.ty()))
}


/// A literal size: an int the program wrote.
pub(super) fn size(x: &Expr) -> Option<usize> {
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
        // a list of tensors shaped alike (`qr`'s two matrices, `fft`'s two)
        Ty::List(item) if matches!(item.payload(), Ty::Tensor(..)) => Ty::List(Box::new(shaped(*item, dims))),
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

/// The shape a list of rows has stacked: one dimension more than each row
/// has, where the rows' shape is known (a list of lists of numbers is a
/// matrix); none where the rows' rank shows only as the program runs.
pub(super) fn stacked_dims(list: &Ty) -> Option<Dims> {
    match list.payload() {
        // (a mask of a matrix's shape: a matrix of 1 and 0, deeper for more
        // axes)
        t @ Ty::List(_) if t.truths_depth() >= 2 => Some(vec![None; t.truths_depth()]),
        Ty::List(item) => match item.payload() {
            Ty::Tensor(_, Some(d)) => Some(std::iter::once(None).chain(d.iter().cloned()).collect()),
            Ty::List(inner) if matches!(inner.payload(), Ty::Int | Ty::Float) => Some(vec![None, None]),
            _ => None,
        },
        // a column of vectors is its matrix, a row per cell; another
        // column its vector of numbers
        Ty::Column(cell) => match cell.payload() {
            Ty::Tensor(_, Some(d)) => Some(std::iter::once(None).chain(d.iter().cloned()).collect()),
            Ty::Tensor(_, None) => Some(vec![None, None]),
            _ => Some(vec![None]),
        },
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
        // tensor × tensor is the matrix product: a matrix (`check_shapes`
        // refuses two known shapes that are not matrices)
        BinOp::Mul if tensor(l) && tensor(r) => match (ls, rs) {
            (Some(a), Some(b)) if a.len() == 2 && b.len() == 2 => Some(vec![a[0], b[1]]),
            (Some(a), None) if a.len() == 2 => Some(vec![a[0], None]),
            (None, Some(b)) if b.len() == 2 => Some(vec![None, b[1]]),
            (None, None) => Some(vec![None, None]),
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
    /// A loop over a tensor walks rows or numbers, and an index into one
    /// reads a row or a number, by the tensor's rank — which its type
    /// states, so both backends decide alike. Where the lowering does not
    /// know it, the decision is refused rather than guessed, and the
    /// program states the rank.
    /// What a loop walks has items: a list, a tensor (its rows, or its
    /// numbers), a frame's column (its cells) or a parsed list. Anything
    /// else is refused here, the same in both modes: the interpreter had
    /// refused a map as it ran, and a loop over text had not built.
    pub(super) fn check_walkable(&mut self, x: &Expr) {
        if self.pass != Pass::Final {
            return;
        }
        // (an item of a list no item is ever put into has no kind, and a
        // loop over it none to walk: the interpreter had run no rounds and
        // the compiled program, holding the item as text, did not build;
        // Rosetta Code, Tropical algebra overloading)
        if let Expr::Local(v, Ty::Unknown) | Expr::Move(v, Ty::Unknown) = x
            && self.v.unknown_items.contains(v)
        {
            self.refuse(format!("'{}' is an item of a list never given one, so it has no kind to walk", crate::lower::shown(v)));
            return;
        }
        let walk = match x.ty().payload() {
            Ty::List(_) | Ty::Tensor(..) | Ty::Column(_) | Ty::Json | Ty::Unknown => return,
            Ty::Map(..) => " — walk keys[m] or values[m]",
            Ty::Str | Ty::Markup | Ty::Node => " — walk split[s, '']",
            Ty::Frame(_) => " — walk one of its columns",
            Ty::Unit => {
                self.refuse("a loop walks what a function that answers nothing gave, which holds no value, only whether it failed (`.ok`)".to_string());
                return;
            }
            _ => "",
        };
        self.refuse(format!("a loop walks {}, which has no items{}", x.ty().payload().crow_name(), walk));
    }

    pub(super) fn check_rank_known(&mut self, x: &Expr, what: &str) {
        // (a value whose making was refused — an undefined function's — has
        // no rank to tell: that mistake is told once)
        if self.pass != Pass::Final || !matches!(x.ty().payload(), Ty::Tensor(_, None)) || self.rooted_in_contested(x) {
            return;
        }
        // (on a line refused for something else it is that mistake's echo,
        // and goes: checks::without_echoes)
        let name = match x {
            Expr::Local(v, _) | Expr::Move(v, _) => format!("`{}`", v),
            Expr::Field { base, name, .. } => match base.as_ref() {
                Expr::Local(v, _) | Expr::Move(v, _) => format!("`{}.{}`", v, name),
                _ => format!("`.{}`", name),
            },
            _ => "the tensor".to_string(),
        };
        let read = if what.starts_with("a loop") {
            "walks rows or numbers"
        } else if what.starts_with("a comparison") {
            "makes a mask of its shape"
        } else {
            "reads a row or a number"
        };
        self.refuse(format!(
            "{} {} {} by its rank, and the rank of {} is not known here — state it: reshape[t, rows, columns] for a matrix, flatten[t] for a vector",
            what, name, read, name
        ));
    }

    /// A tensor local a part of which is assigned: its sizes are no longer
    /// stated from there on, so nothing is refused of them; its rank stays.
    pub(super) fn forget_shape(&mut self, var: &str) {
        fn forget(t: &Ty) -> Ty {
            match t {
                Ty::Tensor(elem, dims) => Ty::Tensor(elem.clone(), dims.as_ref().map(|d| vec![None; d.len()])),
                Ty::Result(inner) => Ty::Result(Box::new(forget(inner))),
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
            [Some((f, p)), Some((_, q))] if p != q => format!(" (the tensors {}[] is handed as `{}` and `{}`)", super::checks::plain_name(&f), super::checks::plain_name(&p), super::checks::plain_name(&q)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn int(n: i64) -> Expr {
        Expr::Const(Const::Int(n), Ty::Int)
    }
    fn tensor(dims: &[Option<usize>]) -> Expr {
        Expr::Local("t".into(), Ty::Tensor(Box::new(Ty::Float), Some(dims.to_vec())))
    }
    fn row(id: &str) -> &'static Row {
        crate::table::row(id).expect("a row")
    }

    /// A row's declared shape is read from what the call states: a literal
    /// size, an argument's shape; nothing where the row states nothing or
    /// the call does not say.
    #[test]
    fn a_rows_declared_shape_is_read_from_the_call() {
        assert_eq!(declared(row("math.eye"), &[int(3)]), Some(vec![Some(3), Some(3)]));
        assert_eq!(declared(row("math.zeros"), &[int(2), int(3)]), Some(vec![Some(2), Some(3)]));
        assert_eq!(declared(row("math.zeros"), &[]), None);
        assert_eq!(declared(row("math.full"), &[int(2), int(3), Expr::Const(Const::Float(1.0), Ty::Float)]), Some(vec![Some(2), Some(3)]));
        assert_eq!(declared(row("math.sum_axis"), &[tensor(&[Some(2), Some(3)]), int(0)]), Some(vec![Some(3)]));
        assert_eq!(declared(row("math.sum_axis"), &[tensor(&[Some(2), Some(3)]), Expr::Local("ax".into(), Ty::Int)]), Some(vec![None]));
        assert_eq!(declared(row("math.lstsq"), &[tensor(&[Some(4), Some(2)]), tensor(&[Some(4)])]), Some(vec![Some(2)]));
        assert_eq!(declared(row("core.pop_tensor"), &[tensor(&[Some(4), Some(2)])]), Some(vec![None, Some(2)]));
        assert_eq!(declared(row("core.concat_axis"), &[tensor(&[Some(2), Some(3)]), tensor(&[Some(2), Some(1)]), int(1)]), Some(vec![Some(2), None]));
        assert_eq!(declared(row("core.concat_tensors"), &[tensor(&[Some(2), Some(3)]), tensor(&[Some(1), Some(3)])]), Some(vec![None, Some(3)]));
        assert_eq!(declared(row("math.flatten"), &[tensor(&[Some(2), Some(3)])]), Some(vec![Some(6)]));
        assert_eq!(declared(row("math.transpose"), &[tensor(&[Some(2), Some(3)])]), Some(vec![Some(3), Some(2)]));
        assert_eq!(declared(row("math.transpose"), &[tensor(&[Some(3)])]), None);
        assert_eq!(declared(row("math.diag"), &[tensor(&[Some(3)])]), Some(vec![Some(3), Some(3)]));
        assert_eq!(declared(row("math.diag"), &[tensor(&[Some(3), Some(3)])]), Some(vec![None]));
        assert_eq!(declared(row("math.where"), &[tensor(&[Some(2), Some(3)]), tensor(&[Some(3)]), tensor(&[Some(1), Some(3)])]), Some(vec![Some(2), Some(3)]));
        assert_eq!(declared(row("math.where"), &[tensor(&[Some(2), Some(3)]), tensor(&[Some(2)]), tensor(&[Some(3)])]), None);
        assert_eq!(declared(row("chart.sma"), &[Expr::Local("p".into(), Ty::Unknown), int(5)]), Some(vec![None]));
        assert_eq!(declared(row("math.svd"), &[tensor(&[Some(4), Some(2)])]), None);
        assert_eq!(declared_pieces(row("math.svd"), &[tensor(&[Some(4), Some(2)])]), Some(vec![Some(vec![Some(4), None]), Some(vec![None]), Some(vec![None, Some(2)])]));
        assert_eq!(declared_pieces(row("math.eye"), &[int(3)]), None);
        assert_eq!(declared(row("text.split"), &[]), None);
    }
}
