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
//! builtin's answer has one (`row_dims`), and a loop over a tensor walks
//! rows or numbers, an index reads a row or a number, by it — refused
//! where the lowering does not know it (`check_rank_known`), so both
//! backends decide alike (spec, Arrays, Design Note "rank is in the type").

use super::*;
use crate::type_infer::{shown_dims, Dims};

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
        // a row per target, a column per class
        "onehot" if args.len() == 2 => Some(vec![None, size(&args[1])]),
        "linspace" if args.len() == 3 => Some(vec![size(&args[2])]),
        // the last two axes swapped (the runtime's CrowTensor::transpose);
        // a list of rows is the matrix they stack to
        "transpose" if args.len() == 1 => args[0].ty().tensor_dims().cloned().or_else(|| stacked_dims(&args[0].ty())).filter(|d| d.len() >= 2).map(|mut d| {
            let n = d.len();
            d.swap(n - 2, n - 1);
            d
        }),
        "reshape" if args.len() >= 2 => Some(sizes(&args[1..])),
        // a list of rows stacked along a new axis: one dimension more than
        // its rows have; a list of lists of numbers is a matrix
        "stack" if args.len() == 1 => stacked_dims(&args[0].ty()),
        // numbers as a tensor: a list's a vector, a list of lists' a matrix,
        // a tensor's its own
        "tensor" if args.len() == 1 => match args[0].ty().payload() {
            Ty::List(item) => match item.payload() {
                Ty::Int | Ty::Float => Some(vec![None]),
                Ty::List(inner) if matches!(inner.payload(), Ty::Int | Ty::Float) => Some(vec![None, None]),
                _ => None,
            },
            Ty::Tensor(_, d) => d.clone(),
            Ty::Column(_) => Some(vec![None]),
            _ => None,
        },
        _ => None,
    }
}

/// The shape a list of rows has stacked: one dimension more than each row
/// has, where the rows' shape is known (a list of lists of numbers is a
/// matrix); none where the rows' rank shows only as the program runs.
pub(super) fn stacked_dims(list: &Ty) -> Option<Dims> {
    match list.payload() {
        Ty::List(item) => match item.payload() {
            Ty::Tensor(_, Some(d)) => Some(std::iter::once(None).chain(d.iter().cloned()).collect()),
            Ty::List(inner) if matches!(inner.payload(), Ty::Int | Ty::Float) => Some(vec![None, None]),
            _ => None,
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

/// The shape of a builtin's tensor where `call_dims` states no sizes: by
/// the rank every builtin's answer has (spec, Arrays: rank is in the type)
/// — an element-wise map keeps its argument's shape, a reduction along an
/// axis drops that axis, a constructor has a dimension per size, a loaded
/// table is a matrix, and so on — with the sizes that follow. None where
/// the rank depends on a value (`transpose` of a tensor of unknown rank,
/// `svd`'s pieces, whose ranks differ).
pub(super) fn row_dims(name: &str, args: &[Expr]) -> Option<Dims> {
    // (a list of numbers handed where a tensor is taken is a vector: the
    // order of a list computed in a function — `argsort[vals]` — is one,
    // and an index into it reads a number; it had no rank there, and the
    // index was refused before the program ran)
    let dims = |i: usize| -> Option<Dims> {
        args.get(i).and_then(|a| match a.ty().payload() {
            Ty::List(item) if matches!(item.payload(), Ty::Int | Ty::Float) => Some(vec![None]),
            _ => a.ty().tensor_dims().cloned(),
        })
    };
    let items = |i: usize| -> Option<Dims> {
        args.get(i).and_then(|a| match a.ty().payload() {
            Ty::List(item) => item.payload().tensor_dims().cloned(),
            _ => None,
        })
    };
    let unknown = |n: usize| -> Option<Dims> { Some(vec![None; n]) };
    let sizes = |xs: &[Expr]| -> Dims { xs.iter().map(size).collect() };
    let first_unknown = |d: Option<Dims>| -> Option<Dims> {
        d.map(|mut d| {
            if let Some(x) = d.first_mut() {
                *x = None;
            }
            d
        })
    };
    // the element-wise shape of two or three operands, where every one's
    // shape is known
    let broadcast_of = |is: &[usize]| -> Option<Dims> {
        let mut out: Option<Dims> = None;
        for &i in is {
            let d = dims(i)?;
            out = Some(match out {
                Some(o) => broadcast(&o, &d).ok()?,
                None => d,
            });
        }
        out
    };
    let n = args.len();
    match name {
        // element-wise maps, and the transforms that keep a shape
        "exp" | "ln" | "log2" | "log10" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "sign" | "abs" | "sqrt" | "floor" | "ceil" | "round" | "gamma" | "erf" | "factorial" | "bessel" | "clip" | "zscore" | "relu" | "sigmoid" | "softmax" | "gelu" | "cumsum" | "cumprod" | "sort" | "reverse" | "shuffle" | "triu" | "tril" | "inv" | "pinv" | "cholesky" | "ifft" | "lowpass" | "highpass" | "bandpass" => dims(0),
        // an indicator of a series is a series, whatever held the prices
        "sma" | "ema" | "rsi" => unknown(1),
        "atan2" | "choose" => broadcast_of(&[0, 1]),
        "where" => broadcast_of(&[0, 1, 2]),
        "mod" if n == 2 && tensor(&args[1]) => broadcast_of(&[0, 1]),
        "mod" => dims(0),
        // `tensor[t]` is its own, `tensor[df]` a matrix; `stack[t]` restacks a
        // tensor's rows, `stack[rows, axis]` one dimension more than a row
        "tensor" | "dense" if n == 1 => match args[0].ty().payload() {
            Ty::Tensor(_, d) => d.clone(),
            Ty::Frame(_) => unknown(2),
            Ty::Sparse => unknown(2),
            _ => None,
        },
        "tensor" if n == 2 => unknown(2),
        "stack" if n == 1 => dims(0),
        "stack" if n == 2 => stacked_dims(&args[0].ty()).or_else(|| dims(0)).map(|d| vec![None; d.len()]),
        // a reduction along an axis drops it
        "sum" | "mean" | "min" | "max" | "prod" if n == 2 => dims(0).map(|mut d| match size(&args[1]) {
            Some(ax) if ax < d.len() => {
                d.remove(ax);
                d
            }
            _ => vec![None; d.len().saturating_sub(1)],
        }),
        // vectors: a flattened tensor's numbers, a range, a sample, a signal
        "flatten" => Some(vec![dims(0).and_then(|d| d.iter().copied().try_fold(1usize, |p, x| x.map(|x| p * x)))]),
        "arange" | "unique" | "vwap" | "rolling_sum" | "cvd" | "rsi_buckets" | "conv" | "spmv" => unknown(1),
        "hamming" | "hanning" | "permutation" if n == 1 => Some(vec![size(&args[0])]),
        "topk" | "choice" if n == 2 => Some(vec![size(&args[1])]),
        // matrices
        "conv2d" | "spectrogram" | "load" | "tensor_of_frame" => unknown(2),
        "diag" => match dims(0) {
            Some(d) if d.len() == 1 => Some(vec![d[0], d[0]]),
            Some(d) if d.len() == 2 => unknown(1),
            _ => None,
        },
        // random draws: a dimension per size after the distribution's numbers
        "normal" | "uniform" | "gamma_dist" | "beta_dist" if n >= 3 => Some(sizes(&args[2..])),
        "poisson" | "bernoulli" | "exponential" if n >= 2 => Some(sizes(&args[1..])),
        // one row more or fewer, a slice of rows, a join along an axis
        "pop" | "slice" => first_unknown(dims(0)),
        "push" if n == 2 && tensor(&args[1]) => first_unknown(dims(0)),
        "push" if n == 2 && tensor(&args[0]) => unknown(1),
        "concat" if n >= 2 && tensor(&args[0]) => dims(0).map(|mut d| {
            let ax = args.get(2).and_then(size).unwrap_or(0);
            if ax < d.len() {
                d[ax] = None;
            }
            d
        }),
        // a system's solution is shaped like its right-hand side, its rows
        // the matrix's columns
        "solve" if n == 2 => dims(1),
        "lstsq" if n == 2 => dims(1).map(|mut d| {
            if let Some(x) = d.first_mut() {
                *x = dims(0).and_then(|a| a.get(1).copied()).flatten();
            }
            d
        }),
        // a layer's rows are its input's, its columns its weights'
        "linear" if n == 3 => Some(vec![dims(0).and_then(|d| d.first().copied()).flatten(), dims(1).and_then(|d| d.get(1).copied()).flatten()]),
        // lists of tensors shaped alike: `qr`'s matrices, a transform's real
        // and imaginary parts (`svd` and `eigen`, whose pieces differ, are
        // typed piece by piece: `piece_dims`)
        "qr" | "fft2" | "stft" => unknown(2),
        "fft" => dims(0),
        "conjugate" => items(0),
        // a complex tensor's parts are shaped like its pieces
        "real" | "imag" | "magnitude" | "phase" => items(0),
        // a complex number is two vectors of one number each
        "complex" => unknown(1),
        _ => None,
    }
}

/// The shapes of the pieces a decomposition answers, by position: `svd`'s
/// U (the rows of its matrix), singular values (a vector) and Vt (its
/// columns); `eigen`'s values (a vector) and vectors (a square matrix). A
/// list holds items of one type, so the list's item says nothing of them;
/// an index by a literal position reads the piece's own shape.
pub(super) fn piece_dims(name: &str, arg: Option<&Dims>) -> Option<Vec<Option<Dims>>> {
    let at = |i: usize| arg.and_then(|d| d.get(i).copied()).flatten();
    match name {
        "svd" => Some(vec![Some(vec![at(0), None]), Some(vec![None]), Some(vec![None, at(1)])]),
        "eigen" => Some(vec![Some(vec![at(0)]), Some(vec![at(0), at(0)])]),
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
        let walk = match x.ty().payload() {
            Ty::List(_) | Ty::Tensor(..) | Ty::Column(_) | Ty::Json | Ty::Unknown => return,
            Ty::Map(_) => " — walk keys[m] or values[m]",
            Ty::Str | Ty::Markup | Ty::Node => " — walk split[s, '']",
            Ty::Frame(_) => " — walk one of its columns",
            _ => "",
        };
        self.refuse(format!("a loop walks {}, which has no items{}", x.ty().payload().crow_name(), walk));
    }

    pub(super) fn check_rank_known(&mut self, x: &Expr, what: &str) {
        if self.pass != Pass::Final || !matches!(x.ty().payload(), Ty::Tensor(_, None)) {
            return;
        }
        let name = match x {
            Expr::Local(v, _) | Expr::Move(v, _) => format!("`{}`", v),
            Expr::Field { base, name, .. } => match base.as_ref() {
                Expr::Local(v, _) | Expr::Move(v, _) => format!("`{}.{}`", v, name),
                _ => format!("`.{}`", name),
            },
            _ => "the tensor".to_string(),
        };
        let read = if what.starts_with("a loop") { "walks rows or numbers" } else { "reads a row or a number" };
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
