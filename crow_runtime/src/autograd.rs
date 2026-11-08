//! Reverse-mode automatic differentiation over tensors (spec, Automatic
//! Differentiation): `grad[f]` of a function whose parameters are tensors.
//! A function handed to `grad` runs once on *traced* values — each a
//! tensor (a number is a tensor of no dimensions) and, where it was
//! computed from a parameter, a node on a tape recording how — and the
//! tape is walked backwards from the function's answer, a number, to the
//! gradient with respect to each parameter. Every operation here computes
//! its value with the operators' own functions, so a traced function's
//! answer is the untraced function's to the bit; what it records beside
//! the value is how the answer moves with each input. The tape lives on
//! the thread that runs the function, so a traced function runs its loops
//! in order. Both modes call these functions: the interpreter on its
//! traced values, the compiled program in the copy of the function it
//! differentiates.

use std::cell::RefCell;

use crate::tensor::CrowTensor;

/// A value a differentiated function computes with: its tensor, and its
/// node on the tape where it depends on a parameter.
#[derive(Clone, Debug)]
pub struct Traced {
    pub value: CrowTensor,
    node: Option<usize>,
}

/// A node's backward rule: the gradient of the number with respect to
/// the node's value, answered as the gradient with respect to each input.
type Backward = Box<dyn Fn(&CrowTensor) -> Vec<CrowTensor>>;

/// How a node's gradient reaches its parents: each parent's node and the
/// function of the node's gradient that gives the parent's share.
struct Node {
    parents: Vec<usize>,
    back: Backward,
}

thread_local! {
    static TAPE: RefCell<Option<Vec<Node>>> = const { RefCell::new(None) };
}

/// A node recorded on the tape, when one is open: none where no input
/// depends on a parameter.
fn record(parents: Vec<Option<usize>>, back: impl Fn(&CrowTensor) -> Vec<CrowTensor> + 'static) -> Option<usize> {
    if parents.iter().all(Option::is_none) {
        return None;
    }
    TAPE.with(|t| {
        let mut tape = t.borrow_mut();
        let tape = tape.as_mut()?;
        // a constant parent (no node) gets no gradient: its share is dropped
        let parents: Vec<Option<usize>> = parents;
        tape.push(Node {
            parents: parents.iter().map(|p| p.unwrap_or(usize::MAX)).collect(),
            back: Box::new(back),
        });
        Some(tape.len() - 1)
    })
}

impl Traced {
    /// A constant: a value that depends on no parameter.
    pub fn constant(value: CrowTensor) -> Traced {
        Traced { value, node: None }
    }

    /// A number as a constant.
    pub fn number(x: f64) -> Traced {
        Traced::constant(CrowTensor::scalar(x))
    }

    /// The value as a number, when it is one (a tensor of no dimensions or
    /// of one element).
    pub fn as_number(&self) -> Option<f64> {
        (self.value.len() == 1).then(|| self.value.to_vec()[0])
    }

    fn from(value: CrowTensor, node: Option<usize>) -> Traced {
        Traced { value, node }
    }
}

impl crate::display::CrowDisplay for Traced {
    fn crow_repr(&self) -> String {
        match self.as_number().filter(|_| self.value.ndim() == 0) {
            Some(x) => x.crow_repr(),
            None => self.value.crow_repr(),
        }
    }
}

impl std::fmt::Display for Traced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.as_number().filter(|_| self.value.ndim() == 0) {
            Some(x) => write!(f, "{}", x),
            None => write!(f, "{}", self.value),
        }
    }
}

/// A value as a traced operand: a traced value as it is (borrowed), a
/// number or a tensor as a constant. The compiled copy of a function `grad`
/// differentiates reads every operand through this, so a value that is no
/// parameter's — a global's tensor, a list's number — meets the kernel as
/// the constant it is.
pub trait AsTraced {
    fn traced(&self) -> std::borrow::Cow<'_, Traced>;
}

impl AsTraced for Traced {
    fn traced(&self) -> std::borrow::Cow<'_, Traced> {
        std::borrow::Cow::Borrowed(self)
    }
}

impl AsTraced for f64 {
    fn traced(&self) -> std::borrow::Cow<'_, Traced> {
        std::borrow::Cow::Owned(Traced::number(*self))
    }
}

impl AsTraced for i64 {
    fn traced(&self) -> std::borrow::Cow<'_, Traced> {
        std::borrow::Cow::Owned(Traced::number(*self as f64))
    }
}

impl AsTraced for CrowTensor {
    fn traced(&self) -> std::borrow::Cow<'_, Traced> {
        std::borrow::Cow::Owned(Traced::constant(self.clone()))
    }
}

/// A tensor's numbers where they lie, in either form a traced copy holds a
/// tensor in: a traced value's numbers, a plain tensor itself. The copy
/// reads its numbers through this where they are wanted as they are — a
/// whole number by position, a list's items — whichever form holds them
/// (`wd[0]` of a literal of sizes, a parameter handed where a list of ints
/// is taken: parity 515), and with no copy of a plain one, which `AsTraced`
/// would make a constant of the tape first.
pub trait TensorOf {
    fn tensor_of(&self) -> &CrowTensor;
}

impl TensorOf for Traced {
    fn tensor_of(&self) -> &CrowTensor {
        &self.value
    }
}

impl TensorOf for CrowTensor {
    fn tensor_of(&self) -> &CrowTensor {
        self
    }
}

impl From<f64> for Traced {
    fn from(x: f64) -> Traced {
        Traced::number(x)
    }
}

impl From<i64> for Traced {
    fn from(n: i64) -> Traced {
        Traced::number(n as f64)
    }
}

impl From<CrowTensor> for Traced {
    fn from(t: CrowTensor) -> Traced {
        Traced::constant(t)
    }
}

impl From<&CrowTensor> for Traced {
    fn from(t: &CrowTensor) -> Traced {
        Traced::constant(t.clone())
    }
}

/// `grad[f][x, …]`: the gradient of `f` with respect to each of its
/// parameters, `f` run once on traced leaves — the tape opened for the
/// run and closed after it (a function that calls `grad` inside one is
/// refused before the program runs). `f` answers a number: a tensor of
/// one element is read as one; anything else is the reason.
pub fn gradient(inputs: &[CrowTensor], f: impl FnOnce(&[Traced]) -> Result<Traced, String>) -> Result<Vec<CrowTensor>, String> {
    let already = TAPE.with(|t| t.borrow().is_some());
    if already {
        return Err("grad: a function grad differentiates cannot call grad".to_string());
    }
    TAPE.with(|t| *t.borrow_mut() = Some(Vec::new()));
    let leaves: Vec<Traced> = inputs
        .iter()
        .map(|x| {
            let node = record(vec![Some(usize::MAX)], |_| Vec::new());
            Traced::from(x.clone(), node)
        })
        .collect();
    let out = f(&leaves);
    let tape = TAPE.with(|t| t.borrow_mut().take()).unwrap_or_default();
    let out = out?;
    if out.value.len() != 1 {
        return Err(format!("grad: the function answers a tensor of shape {:?}, and a gradient is of a function that answers a number", out.value.shape()));
    }
    let mut grads: Vec<Option<CrowTensor>> = vec![None; tape.len()];
    if let Some(n) = out.node {
        grads[n] = Some(CrowTensor::full(out.value.shape().to_vec(), 1.0));
    }
    for i in (0..tape.len()).rev() {
        let Some(g) = grads[i].take() else { continue };
        let shares = (tape[i].back)(&g);
        for (p, share) in tape[i].parents.iter().zip(shares) {
            if *p == usize::MAX {
                continue;
            }
            grads[*p] = Some(match grads[*p].take() {
                Some(so_far) => so_far.add(&share),
                None => share,
            });
        }
        // (a leaf's node keeps its gradient for the answer below)
        if i < inputs.len() {
            grads[i] = Some(g);
        }
    }
    Ok(leaves
        .iter()
        .zip(inputs)
        .map(|(leaf, x)| leaf.node.and_then(|n| grads[n].clone()).unwrap_or_else(|| CrowTensor::zeros(x.shape().to_vec())))
        .collect())
}

/// A gradient brought back to the shape of the input it belongs to, where
/// the operation broadcast the input: summed over the dimensions
/// broadcasting added or stretched.
fn unbroadcast(g: &CrowTensor, shape: &[usize]) -> CrowTensor {
    let mut g = g.clone();
    while g.ndim() > shape.len() {
        g = g.sum_axis(0);
    }
    if shape.is_empty() {
        // (left on the GPU where g lies: a number beside a tensor, a
        // temperature dividing the logits, had read the gradient back)
        return g.sum_tensor();
    }
    for d in 0..shape.len() {
        if shape[d] == 1 && g.shape()[d] != 1 {
            let mut kept = g.shape().to_vec();
            kept[d] = 1;
            g = g.sum_axis(d).reshape(kept);
        }
    }
    g
}

/// The shapes of two tensors an element-wise operation meets, or the
/// operator's reason.
fn meets(a: &Traced, b: &Traced, sign: &str) -> Result<(), String> {
    a.value.meets(&b.value, sign)
}

pub fn add(a: &Traced, b: &Traced) -> Result<Traced, String> {
    meets(a, b, "+")?;
    let (sa, sb) = (a.value.shape().to_vec(), b.value.shape().to_vec());
    let node = record(vec![a.node, b.node], move |g| vec![unbroadcast(g, &sa), unbroadcast(g, &sb)]);
    Ok(Traced::from(a.value.add(&b.value), node))
}

pub fn sub(a: &Traced, b: &Traced) -> Result<Traced, String> {
    meets(a, b, "-")?;
    let (sa, sb) = (a.value.shape().to_vec(), b.value.shape().to_vec());
    let node = record(vec![a.node, b.node], move |g| vec![unbroadcast(g, &sa), unbroadcast(&g.neg(), &sb)]);
    Ok(Traced::from(a.value.sub(&b.value), node))
}

/// `.*`, and `*` of a tensor and a number.
pub fn mul(a: &Traced, b: &Traced) -> Result<Traced, String> {
    meets(a, b, ".*")?;
    let (sa, sb) = (a.value.shape().to_vec(), b.value.shape().to_vec());
    let (av, bv) = (a.value.clone(), b.value.clone());
    let node = record(vec![a.node, b.node], move |g| vec![unbroadcast(&g.mul(&bv), &sa), unbroadcast(&g.mul(&av), &sb)]);
    Ok(Traced::from(a.value.mul(&b.value), node))
}

/// `./`, and `/` of a tensor by a number.
pub fn div(a: &Traced, b: &Traced) -> Result<Traced, String> {
    meets(a, b, "./")?;
    let (sa, sb) = (a.value.shape().to_vec(), b.value.shape().to_vec());
    let (av, bv) = (a.value.clone(), b.value.clone());
    let node = record(vec![a.node, b.node], move |g| {
        let db = g.mul(&av).div(&bv.mul(&bv)).neg();
        vec![unbroadcast(&g.div(&bv), &sa), unbroadcast(&db, &sb)]
    });
    Ok(Traced::from(a.value.div(&b.value), node))
}

/// `.^`, and `^` of a tensor by a number: a .^ b.
pub fn pow(a: &Traced, b: &Traced) -> Result<Traced, String> {
    meets(a, b, ".^")?;
    let (sa, sb) = (a.value.shape().to_vec(), b.value.shape().to_vec());
    let (av, bv) = (a.value.clone(), b.value.clone());
    let value = a.value.pow(&b.value);
    let out = value.clone();
    let node = record(vec![a.node, b.node], move |g| {
        // d/da = b a^(b-1); d/db = a^b ln a
        let da = g.mul(&bv).mul(&av.pow(&bv.sub_scalar(1.0)));
        let db = g.mul(&out).mul(&local(&av, f64::ln, CrowTensor::log));
        vec![unbroadcast(&da, &sa), unbroadcast(&db, &sb)]
    });
    Ok(Traced::from(value, node))
}

pub fn neg(a: &Traced) -> Traced {
    let node = record(vec![a.node], |g| vec![g.neg()]);
    Traced::from(a.value.neg(), node)
}

/// `*` of two matrices.
pub fn matmul(a: &Traced, b: &Traced) -> Result<Traced, String> {
    let value = a.value.try_matmul(&b.value)?;
    let (av, bv) = (a.value.clone(), b.value.clone());
    let node = record(vec![a.node, b.node], move |g| vec![g.matmul(&bv.transpose()), av.transpose().matmul(g)]);
    Ok(Traced::from(value, node))
}

pub fn transpose(a: &Traced) -> Traced {
    let node = record(vec![a.node], |g| vec![g.transpose()]);
    Traced::from(a.value.transpose(), node)
}

pub fn reshape(a: &Traced, shape: Vec<usize>) -> Result<Traced, String> {
    if CrowTensor::numel(&shape) != a.value.len() {
        return Err(format!("reshape: a tensor of {} elements cannot take shape {:?}", a.value.len(), shape));
    }
    let back_to = a.value.shape().to_vec();
    let node = record(vec![a.node], move |g| vec![g.reshape(back_to.clone())]);
    Ok(Traced::from(a.value.reshape(shape), node))
}

pub fn flatten(a: &Traced) -> Traced {
    let n = a.value.len();
    reshape(a, vec![n]).unwrap_or_else(|_| a.clone())
}

/// `sum[t]`: a number.
pub fn sum(a: &Traced) -> Traced {
    let shape = a.value.shape().to_vec();
    let on_gpu = a.value.is_gpu();
    let node = record(vec![a.node], move |g| vec![filled(g, &shape, None, on_gpu)]);
    Traced::from(a.value.sum_tensor(), node)
}

/// `mean[t]`: a number.
pub fn mean(a: &Traced) -> Traced {
    let shape = a.value.shape().to_vec();
    let n = a.value.len().max(1) as f64;
    let on_gpu = a.value.is_gpu();
    let node = record(vec![a.node], move |g| vec![filled(g, &shape, Some(n), on_gpu)]);
    Traced::from(a.value.mean_tensor(), node)
}

/// The gradient of a sum over a tensor of `shape` (of a mean, divided by
/// its `n`): the answer's gradient `g`, one number, at every place. Where
/// `g` lies on the GPU it stays there, read with strides of 0 (read back as
/// a number, it had been a round trip to the device at every step of a
/// network's training); where the tensor summed lies there, the number is
/// sent once (a tensor of the input's size had been made on the CPU and
/// sent at the next operation).
fn filled(g: &CrowTensor, shape: &[usize], n: Option<f64>, on_gpu: bool) -> CrowTensor {
    if g.is_gpu() && g.len() == 1 {
        let one = g.reshape(vec![]);
        let one = match n {
            Some(n) => one.div_scalar(n),
            None => one,
        };
        return one.broadcast_to(shape);
    }
    let x = match n {
        Some(n) => g.sum() / n,
        None => g.sum(),
    };
    if on_gpu { CrowTensor::scalar(x).to_gpu().broadcast_to(shape) } else { CrowTensor::full(shape.to_vec(), x) }
}

/// `sum[t, axis = k]`.
pub fn sum_axis(a: &Traced, axis: usize) -> Result<Traced, String> {
    if axis >= a.value.ndim() {
        return Err(format!("sum: axis {} is outside a tensor of {} dimensions", axis, a.value.ndim()));
    }
    let shape = a.value.shape().to_vec();
    let node = record(vec![a.node], move |g| vec![spread(g, &shape, axis)]);
    Ok(Traced::from(a.value.sum_axis(axis), node))
}

/// `mean[t, axis = k]`.
pub fn mean_axis(a: &Traced, axis: usize) -> Result<Traced, String> {
    if axis >= a.value.ndim() {
        return Err(format!("mean: axis {} is outside a tensor of {} dimensions", axis, a.value.ndim()));
    }
    let shape = a.value.shape().to_vec();
    let n = shape[axis].max(1) as f64;
    let node = record(vec![a.node], move |g| vec![spread(g, &shape, axis).div_scalar(n)]);
    Ok(Traced::from(a.value.mean_axis(axis), node))
}

/// A gradient of a reduction along `axis` spread back over the input's
/// shape: each input element gets its slice's gradient.
fn spread(g: &CrowTensor, shape: &[usize], axis: usize) -> CrowTensor {
    let mut kept = shape.to_vec();
    kept[axis] = 1;
    let g = if g.ndim() == 0 { g.reshape(kept.clone()) } else { g.reshape(kept) };
    if g.is_gpu() {
        return g.broadcast_to(shape);
    }
    CrowTensor::zeros(shape.to_vec()).add(&g)
}

fn unary(a: &Traced, value: CrowTensor, local: impl Fn() -> CrowTensor + 'static) -> Traced {
    let node = record(vec![a.node], move |g| vec![g.mul(&local())]);
    Traced::from(value, node)
}

/// A local derivative of each number of `t`: by `cpu`, number by number,
/// where the tensor lies on the CPU (the digits it always had), and by the
/// tensor operations `gpu` names where it lies on the GPU — a closure over
/// numbers brought each step of a backward pass back to the CPU.
fn local(t: &CrowTensor, cpu: impl Fn(f64) -> f64, gpu: impl Fn(&CrowTensor) -> CrowTensor) -> CrowTensor {
    if t.is_gpu() { gpu(t) } else { t.map(cpu) }
}

/// 1 where `x` lies strictly between the bounds, 0 elsewhere, on the GPU
/// (`clip`'s derivative there): a bound no f32 holds, an infinity, is no
/// bound a number there passes.
fn inside(t: &CrowTensor, lo: f64, hi: f64) -> CrowTensor {
    use crate::tensor::Compare;
    let above = (lo > f64::NEG_INFINITY).then(|| t.compared(Compare::Gt, lo));
    let below = (hi < f64::INFINITY).then(|| t.compared(Compare::Lt, hi));
    match (above, below) {
        (Some(a), Some(b)) => a.mul(&b),
        (Some(m), None) | (None, Some(m)) => m,
        (None, None) => t.abs().compared(Compare::Ge, 0.0),
    }
}

pub fn exp(a: &Traced) -> Traced {
    let value = a.value.exp();
    let out = value.clone();
    unary(a, value, move || out.clone())
}

pub fn ln(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.log(), move || local(&av, |x| 1.0 / x, |t| t.scalar_div(1.0)))
}

pub fn sqrt(a: &Traced) -> Traced {
    let value = a.value.sqrt();
    let out = value.clone();
    unary(a, value, move || local(&out, |s| 0.5 / s, |t| t.scalar_div(0.5)))
}

pub fn tanh(a: &Traced) -> Traced {
    let value = a.value.tanh();
    let out = value.clone();
    unary(a, value, move || local(&out, |t| 1.0 - t * t, |t| t.mul(t).neg().add_scalar(1.0)))
}

pub fn abs(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.abs(), move || local(&av, |x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }, CrowTensor::sign))
}

/// `sin[t]`, `cos[t]`, `tan[t]` and the inverse and hyperbolic ones, each
/// by its derivative: cos, −sin, 1 + tan², 1/√(1−x²), −1/√(1−x²),
/// 1/(1+x²), cosh, sinh; `log2` and `log10` by 1/(x ln 2), 1/(x ln 10).
pub fn sin(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.sin(), move || local(&av, crate::math::cosine, CrowTensor::cos))
}

pub fn cos(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.cos(), move || local(&av, |x| -crate::math::sine(x), |t| t.sin().neg()))
}

pub fn tan(a: &Traced) -> Traced {
    let value = a.value.tan();
    let out = value.clone();
    unary(a, value, move || local(&out, |t| 1.0 + t * t, |t| t.mul(t).add_scalar(1.0)))
}

pub fn asin(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.asin(), move || local(&av, |x| 1.0 / (1.0 - x * x).sqrt(), |t| t.mul(t).neg().add_scalar(1.0).sqrt().scalar_div(1.0)))
}

pub fn acos(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.acos(), move || local(&av, |x| -1.0 / (1.0 - x * x).sqrt(), |t| t.mul(t).neg().add_scalar(1.0).sqrt().scalar_div(-1.0)))
}

pub fn atan(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.atan(), move || local(&av, |x| 1.0 / (1.0 + x * x), |t| t.mul(t).add_scalar(1.0).scalar_div(1.0)))
}

pub fn sinh(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.sinh(), move || local(&av, f64::cosh, CrowTensor::cosh))
}

pub fn cosh(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.cosh(), move || local(&av, f64::sinh, CrowTensor::sinh))
}

pub fn log2(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.log2(), move || local(&av, |x| 1.0 / (x * std::f64::consts::LN_2), |t| t.mul_scalar(std::f64::consts::LN_2).scalar_div(1.0)))
}

pub fn log10(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.log10(), move || local(&av, |x| 1.0 / (x * std::f64::consts::LN_10), |t| t.mul_scalar(std::f64::consts::LN_10).scalar_div(1.0)))
}

/// `log1p[t]`: d/dx ln(1 + x) = 1/(1 + x).
pub fn log1p(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.log1p(), move || local(&av, |x| 1.0 / (1.0 + x), |t| t.add_scalar(1.0).scalar_div(1.0)))
}

/// `expm1[t]`: d/dx (exp(x) − 1) = exp(x).
pub fn expm1(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.expm1(), move || local(&av, f64::exp, CrowTensor::exp))
}

/// `sign[t]`: piecewise constant, a constant of the tape (derivative 0).
pub fn sign(a: &Traced) -> Traced {
    Traced::constant(a.value.sign())
}

/// `clip[t, lo, hi]`: each element held within the bounds; the gradient
/// passes where the element lies strictly inside them and is 0 at and
/// beyond a bound (so `clip[x, 0.0, inf]` is relu, its gradient 0 at 0).
/// The bounds are numbers and take no gradient.
pub fn clip(a: &Traced, lo: &Traced, hi: &Traced) -> Result<Traced, String> {
    let (lo, hi) = match (lo.as_number(), hi.as_number()) {
        (Some(lo), Some(hi)) => (lo, hi),
        _ => return Err("clip: the bounds are numbers".to_string()),
    };
    let av = a.value.clone();
    Ok(unary(a, a.value.clip(lo, hi), move || local(&av, |x| if x > lo && x < hi { 1.0 } else { 0.0 }, |t| inside(t, lo, hi))))
}

/// `erf[t]`: d/dx erf(x) = 2/√π · exp(−x²).
pub fn erf(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.erf(), move || local(&av, |x| std::f64::consts::FRAC_2_SQRT_PI * (-x * x).exp(), |t| t.mul(t).neg().exp().mul_scalar(std::f64::consts::FRAC_2_SQRT_PI)))
}

/// `floor[t]`, `ceil[t]`, `round[t]`: piecewise constant, so the answer is
/// a constant of the tape (derivative 0, as for numbers).
pub fn floor(a: &Traced) -> Traced {
    Traced::constant(a.value.floor())
}

pub fn ceil(a: &Traced) -> Traced {
    Traced::constant(a.value.ceil())
}

pub fn round(a: &Traced) -> Traced {
    Traced::constant(a.value.round())
}

/// Where the extreme of each slice along `axis` lies, in the flat data:
/// one position per element of the reduced tensor, in its row-major order
/// (the first of equal extremes), with the reduced shape.
fn extreme_positions(t: &CrowTensor, axis: usize, better: impl Fn(f64, f64) -> bool) -> (Vec<usize>, Vec<usize>) {
    let mut out_shape = t.shape().to_vec();
    out_shape.remove(axis);
    let out_n = CrowTensor::numel(&out_shape);
    let out_strides = CrowTensor::row_major_strides(&out_shape);
    let src_strides = CrowTensor::row_major_strides(t.shape());
    let src = t.to_vec();
    let mut best: Vec<Option<(f64, usize)>> = vec![None; out_n];
    for (flat, &x) in src.iter().enumerate() {
        let mut remaining = flat;
        let mut out_flat = 0usize;
        let mut out_dim = 0;
        for d in 0..t.ndim() {
            let idx = remaining / src_strides[d];
            remaining %= src_strides[d];
            if d != axis {
                out_flat += idx * out_strides[out_dim];
                out_dim += 1;
            }
        }
        if let Some(slot) = best.get_mut(out_flat) {
            match slot {
                Some((v, _)) if !better(x, *v) => {}
                _ => *slot = Some((x, flat)),
            }
        }
    }
    (best.into_iter().map(|b| b.map_or(0, |(_, i)| i)).collect(), out_shape)
}

/// A reduction to the extreme along an axis: the gradient reaches the
/// element each slice's extreme was read from.
fn extreme_axis(a: &Traced, axis: usize, value: CrowTensor, op: crate::gpu::Reduce, better: impl Fn(f64, f64) -> bool + 'static) -> Traced {
    let shape = a.value.shape().to_vec();
    // (found on the GPU where the tensor lies there, and the gradient laid
    // there: every softmax's max had read its scores back to find them)
    if let Some(at) = a.value.gpu_extreme_places(axis, op) {
        let node = record(vec![a.node], move |g| match CrowTensor::gpu_scattered_once(g, &at, &shape) {
            Some(d) => vec![d],
            None => vec![laid_at(g, &at.places(), &shape)],
        });
        return Traced::from(value, node);
    }
    let (positions, _) = extreme_positions(&a.value, axis, better);
    let node = record(vec![a.node], move |g| vec![laid_at(g, &positions, &shape)]);
    Traced::from(value, node)
}

/// Zeros of `shape` with g's numbers added at the places `positions`
/// names, one each, on the CPU.
fn laid_at(g: &CrowTensor, positions: &[usize], shape: &[usize]) -> CrowTensor {
    let mut d = vec![0.0; CrowTensor::numel(shape)];
    for (slot, g_slot) in g.to_vec().into_iter().enumerate() {
        if let Some(cell) = positions.get(slot).and_then(|&pos| d.get_mut(pos)) {
            *cell += g_slot;
        }
    }
    CrowTensor::from_vec(d, shape.to_vec())
}

/// A reduction to the extreme of the whole tensor: a number, its gradient
/// reaching the first element that holds it.
fn extreme(a: &Traced, value: f64, better: impl Fn(f64, f64) -> bool) -> Traced {
    let data = a.value.to_vec();
    let mut at = 0usize;
    for (i, &x) in data.iter().enumerate() {
        if i == 0 || better(x, data[at]) {
            at = i;
        }
    }
    let shape = a.value.shape().to_vec();
    let node = record(vec![a.node], move |g| {
        let mut d = vec![0.0; CrowTensor::numel(&shape)];
        if let Some(cell) = d.get_mut(at) {
            *cell = g.sum();
        }
        vec![CrowTensor::from_vec(d, shape.clone())]
    });
    Traced::from(CrowTensor::scalar(value), node)
}

/// `max[t]`: a number.
pub fn max(a: &Traced) -> Traced {
    extreme(a, a.value.max(), |x, best| x > best)
}

/// `min[t]`: a number.
pub fn min(a: &Traced) -> Traced {
    extreme(a, a.value.min(), |x, best| x < best)
}

/// `max[t, axis = k]`.
pub fn max_axis(a: &Traced, axis: usize) -> Result<Traced, String> {
    if axis >= a.value.ndim() {
        return Err(format!("max: axis {} is outside a tensor of {} dimensions", axis, a.value.ndim()));
    }
    Ok(extreme_axis(a, axis, a.value.max_axis(axis), crate::gpu::Reduce::Max, |x, best| x > best))
}

/// `min[t, axis = k]`.
pub fn min_axis(a: &Traced, axis: usize) -> Result<Traced, String> {
    if axis >= a.value.ndim() {
        return Err(format!("min: axis {} is outside a tensor of {} dimensions", axis, a.value.ndim()));
    }
    Ok(extreme_axis(a, axis, a.value.min_axis(axis), crate::gpu::Reduce::Min, |x, best| x < best))
}

/// `t[positions]` of a tensor by a tensor of whole-number positions: the
/// numbers of the flattened tensor at the positions (a negative one counted
/// from the end), shaped like the positions (spec, Advanced Indexing). The
/// gradient reaches each number read, summed where a position is read
/// more than once: the answer's gradient scattered back over the tensor.
/// A layer reads its weights out of one vector of parameters so, a
/// convolution its windows, an embedding its rows.
pub fn gather(a: &Traced, positions: &CrowTensor) -> Result<Traced, String> {
    // (where each read lies, checked once and kept for the backward pass:
    // positions on the GPU are read back once)
    let at = a.value.positions_read(positions)?;
    let value = a.value.gathered_at(positions, &at);
    let n = a.value.len();
    let shape = a.value.shape().to_vec();
    let node = record(vec![a.node], move |g| {
        if g.is_gpu()
            && let Some(d) = CrowTensor::gpu_scattered(g, &at, &shape)
        {
            return vec![d];
        }
        let mut d = vec![0.0; n];
        for (k, gk) in g.numbers().iter().enumerate() {
            if let Some(cell) = at.get(k).and_then(|&p| d.get_mut(p)) {
                *cell += gk;
            }
        }
        vec![CrowTensor::from_vec(d, shape.clone())]
    });
    Ok(Traced::from(value, node))
}

/// `t[i]`: a matrix's row (a cube's matrix), a vector's number — a
/// negative position counted from the end, as the untraced read counts;
/// the gradient reaches what was read.
pub fn at(a: &Traced, i: i64) -> Result<Traced, String> {
    let shape = a.value.shape().to_vec();
    let n = a.value.len();
    if a.value.ndim() >= 2 {
        let value = crate::core::row_at(&a.value, i)?;
        let r = crate::stops::row(i, shape[0])?;
        let width = value.len();
        let node = record(vec![a.node], move |g| {
            if g.is_gpu() {
                // (laid into the zeros of the whole where it lies)
                let mut row = shape.clone();
                row[0] = 1;
                let mut place = vec![0; shape.len()];
                place[0] = r;
                return vec![CrowTensor::laid_into(&shape, &place, &g.reshape(row))];
            }
            let mut d = vec![0.0; n];
            if let Some(cells) = d.get_mut(r * width..(r + 1) * width) {
                cells.copy_from_slice(&g.numbers());
            }
            vec![CrowTensor::from_vec(d, shape.clone())]
        });
        return Ok(Traced::from(value, node));
    }
    let value = crate::core::number_at(&a.value, i)?;
    let k = crate::stops::item(i, n)?;
    let node = record(vec![a.node], move |g| {
        let mut d = vec![0.0; n];
        if let Some(cell) = d.get_mut(k) {
            *cell = g.sum();
        }
        vec![CrowTensor::from_vec(d, shape.clone())]
    });
    Ok(Traced::from(CrowTensor::scalar(value), node))
}

/// `slice[t, start, end]`: a vector's numbers, a matrix's rows, from
/// `start` to `end` (crate::core::slice_tensor's positions); the gradient
/// reaches the items kept, and nothing the slice left out.
pub fn slice(a: &Traced, start: i64, end: i64) -> Traced {
    let value = crate::core::slice_tensor(&a.value, start, end);
    if a.value.ndim() == 0 {
        return Traced::constant(value);
    }
    let shape = a.value.shape().to_vec();
    let (s, e) = crate::core::slice_bounds(start, end, shape[0]);
    let e = e.max(s);
    let width = CrowTensor::numel(&shape[1..]);
    let n = a.value.len();
    let node = record(vec![a.node], move |g| {
        if g.is_gpu() {
            // (a layer's weights read out of one vector of parameters: the
            // gradient laid into the vector's zeros on the device)
            let mut part = shape.clone();
            part[0] = e - s;
            let mut place = vec![0; shape.len()];
            place[0] = s;
            return vec![CrowTensor::laid_into(&shape, &place, &g.reshape(part))];
        }
        let mut d = vec![0.0; n];
        if let Some(cells) = d.get_mut(s * width..e * width) {
            cells.copy_from_slice(&g.numbers());
        }
        vec![CrowTensor::from_vec(d, shape.clone())]
    });
    Traced::from(value, node)
}

/// `concat[a, b]` and `concat[a, b, axis = k]` of tensors: the two joined
/// along the axis (0 unless named); the gradient is the answer's, split
/// where the two met.
pub fn concat(a: &Traced, b: &Traced, axis: i64) -> Result<Traced, String> {
    let value = a.value.try_concat(&b.value, axis)?;
    let ax = usize::try_from(axis).unwrap_or(0);
    let first = a.value.shape().get(ax).copied().unwrap_or(0);
    let whole = value.shape().get(ax).copied().unwrap_or(0);
    let node = record(vec![a.node, b.node], move |g| vec![g.slice(ax, 0, first), g.slice(ax, first, whole)]);
    Ok(Traced::from(value, node))
}

/// A node whose gradient is split along `axis` among the items laid side
/// by side there: each item's share is its slice of the axis, one position
/// wide, in its own shape.
fn split_back(ts: &[Traced], axis: usize) -> Option<usize> {
    let shapes: Vec<Vec<usize>> = ts.iter().map(|t| t.value.shape().to_vec()).collect();
    record(ts.iter().map(|t| t.node).collect(), move |g| {
        shapes.iter().enumerate().map(|(i, s)| g.slice(axis, i, i + 1).reshape(s.clone())).collect()
    })
}

/// `stack[ts]` and `stack[ts, axis = k]` of traced tensors — PyTorch's
/// `torch.stack`: the tensors side by side along a new axis (0 unless
/// named), as `math::stack_axis` lays them (its words where the shapes
/// differ); each tensor's gradient is its slice of the new axis. A
/// recurrent layer's outputs, one a step, are stacked so.
pub fn stack(ts: &[Traced], axis: i64) -> Result<Traced, String> {
    let values: Vec<CrowTensor> = ts.iter().map(|t| t.value.clone()).collect();
    let value = crate::math::stack_axis(&values, axis)?;
    let node = split_back(ts, usize::try_from(axis).unwrap_or(0));
    Ok(Traced::from(value, node))
}

/// A list of traced numbers or tensors handed where a tensor is taken
/// (`tensor[items]`): the tensor of its items along a first axis, as
/// `CrowTensor::from_elements` makes one of plain items (its words where
/// they differ in shape); each item's gradient is its slice.
pub fn from_elements(ts: &[Traced]) -> Result<Traced, String> {
    let values: Vec<CrowTensor> = ts.iter().map(|t| t.value.clone()).collect();
    let value = CrowTensor::from_elements(&values)?;
    let node = split_back(ts, 0);
    Ok(Traced::from(value, node))
}

/// `where[cond, a, b]`: `a`'s number where the condition's is not 0 (a
/// mask's true), `b`'s elsewhere — PyTorch's `torch.where`; the condition
/// is a constant of the tape (a mask a comparison answers inside the
/// function is one), and the gradient reaches `a` where the condition
/// held and `b` where it did not, each in its own shape.
pub fn where_(cond: &CrowTensor, a: &Traced, b: &Traced) -> Result<Traced, String> {
    let value = crate::math::where_(cond, &a.value, &b.value)?;
    let held: Vec<f64> = cond.numbers().iter().map(|&c| if c != 0.0 { 1.0 } else { 0.0 }).collect();
    let (sa, sb) = (a.value.shape().to_vec(), b.value.shape().to_vec());
    // (the condition's truths on the device beside a value there, once)
    let mask = value.is_gpu().then(|| CrowTensor::from_vec(held.clone(), value.shape().to_vec()).to_gpu());
    let node = record(vec![a.node, b.node], move |g| {
        if let Some(mask) = mask.as_ref().filter(|_| g.is_gpu()) {
            return vec![g.mul(mask).reshape(sa.clone()), g.mul(&mask.neg().add_scalar(1.0)).reshape(sb.clone())];
        }
        let g = g.numbers();
        let ga: Vec<f64> = g.iter().zip(&held).map(|(x, h)| x * h).collect();
        let gb: Vec<f64> = g.iter().zip(&held).map(|(x, h)| x * (1.0 - h)).collect();
        vec![CrowTensor::from_vec(ga, sa.clone()), CrowTensor::from_vec(gb, sb.clone())]
    });
    Ok(Traced::from(value, node))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small generator (SplitMix64): the same cases on every run.
    struct Gen(u64);
    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn unit(&mut self) -> f64 {
            (self.next() % 20_000) as f64 / 10_000.0 - 1.0
        }
        fn tensor(&mut self, shape: &[usize], lo: f64, hi: f64) -> CrowTensor {
            let n: usize = shape.iter().product();
            let data = (0..n).map(|_| lo + (self.unit() + 1.0) / 2.0 * (hi - lo)).collect();
            CrowTensor::from_vec(data, shape.to_vec())
        }
    }

    /// The gradient by central differences, for every element of every
    /// input, against the one the tape gives.
    fn check(name: &str, inputs: &[CrowTensor], f: impl Fn(&[Traced]) -> Result<Traced, String> + Copy) {
        let grads = gradient(inputs, |xs| f(xs)).unwrap_or_else(|e| panic!("{}: {}", name, e));
        let value = |xs: &[CrowTensor]| f(&xs.iter().map(|x| Traced::constant(x.clone())).collect::<Vec<_>>()).expect(name).as_number().expect("a number");
        for (i, x) in inputs.iter().enumerate() {
            assert_eq!(grads[i].shape(), x.shape(), "{}: input {}'s gradient shape", name, i);
            let data = x.to_vec();
            let got = grads[i].to_vec();
            for k in 0..data.len() {
                let h = 1e-5 * data[k].abs().max(1.0);
                let mut plus = inputs.to_vec();
                let mut minus = inputs.to_vec();
                let mut up = data.clone();
                up[k] += h;
                let mut down = data.clone();
                down[k] -= h;
                plus[i] = CrowTensor::from_vec(up, x.shape().to_vec());
                minus[i] = CrowTensor::from_vec(down, x.shape().to_vec());
                let numeric = (value(&plus) - value(&minus)) / (2.0 * h);
                let tolerance = 1e-4 * numeric.abs().max(1.0);
                assert!((got[k] - numeric).abs() <= tolerance, "{}: input {} element {}: tape {} finite differences {}", name, i, k, got[k], numeric);
            }
        }
    }

    #[test]
    fn every_operation_agrees_with_finite_differences() {
        let mut g = Gen(20261002);
        for round in 0..20 {
            let a = g.tensor(&[3, 4], -2.0, 2.0);
            let b = g.tensor(&[3, 4], -2.0, 2.0);
            let row = g.tensor(&[4], -2.0, 2.0);
            let col = g.tensor(&[3, 1], -2.0, 2.0);
            let positive = g.tensor(&[3, 4], 0.5, 3.0);
            let m = g.tensor(&[4, 2], -1.0, 1.0);
            let bias = g.tensor(&[2], -1.0, 1.0);
            let s = Traced::number(0.7 + round as f64 / 50.0);
            check("add", &[a.clone(), b.clone()], |x| Ok(sum(&add(&x[0], &x[1])?)));
            check("add broadcast row", &[a.clone(), row.clone()], |x| Ok(sum(&mul(&add(&x[0], &x[1])?, &x[0])?)));
            check("add broadcast column", &[a.clone(), col.clone()], |x| Ok(sum(&mul(&add(&x[0], &x[1])?, &x[0])?)));
            check("sub", &[a.clone(), b.clone()], |x| Ok(sum(&mul(&sub(&x[0], &x[1])?, &x[1])?)));
            check("mul", &[a.clone(), b.clone()], |x| Ok(sum(&mul(&x[0], &x[1])?)));
            check("div", &[a.clone(), positive.clone()], |x| Ok(sum(&div(&x[0], &x[1])?)));
            check("pow", &[positive.clone(), b.clone()], |x| Ok(sum(&pow(&x[0], &x[1])?)));
            check("scalar", &[a.clone()], |x| Ok(sum(&div(&mul(&x[0], &s)?, &Traced::number(3.0))?)));
            check("neg", &[a.clone()], |x| Ok(sum(&mul(&neg(&x[0]), &x[0])?)));
            check("matmul", &[a.clone(), m.clone()], |x| Ok(sum(&mul(&matmul(&x[0], &x[1])?, &matmul(&x[0], &x[1])?)?)));
            check("transpose", &[a.clone()], |x| Ok(sum(&matmul(&transpose(&x[0]), &x[0])?)));
            check("reshape", &[a.clone()], |x| Ok(sum(&mul(&reshape(&x[0], vec![4, 3])?, &transpose(&x[0]))?)));
            check("flatten", &[a.clone()], |x| Ok(sum(&mul(&flatten(&x[0]), &flatten(&x[0]))?)));
            check("mean", &[a.clone()], |x| Ok(mean(&mul(&x[0], &x[0])?)));
            check("sum axis 0", &[a.clone()], |x| Ok(sum(&mul(&sum_axis(&x[0], 0)?, &sum_axis(&x[0], 0)?)?)));
            check("mean axis 1", &[a.clone()], |x| Ok(sum(&mul(&mean_axis(&x[0], 1)?, &mean_axis(&x[0], 1)?)?)));
            check("exp", &[a.clone()], |x| Ok(sum(&exp(&x[0]))));
            check("ln", &[positive.clone()], |x| Ok(sum(&ln(&x[0]))));
            check("log1p", &[positive.clone()], |x| Ok(sum(&mul(&log1p(&x[0]), &x[0])?)));
            check("expm1", &[a.clone()], |x| Ok(sum(&mul(&expm1(&x[0]), &x[0])?)));
            check("sqrt", &[positive.clone()], |x| Ok(sum(&sqrt(&x[0]))));
            check("tanh", &[a.clone()], |x| Ok(sum(&tanh(&x[0]))));
            check("abs", &[a.clone()], |x| Ok(sum(&abs(&x[0]))));
            check("clip", &[a.clone()], |x| Ok(sum(&mul(&clip(&x[0], &Traced::number(-1.00005), &Traced::number(0.99995))?, &x[0])?)));
            check("erf", &[a.clone()], |x| Ok(sum(&erf(&x[0]))));
            check("max", &[a.clone()], |x| mul(&max(&x[0]), &sum(&x[0])));
            check("min", &[a.clone()], |x| mul(&min(&x[0]), &sum(&x[0])));
            check("max axis 0", &[a.clone()], |x| Ok(sum(&mul(&max_axis(&x[0], 0)?, &max_axis(&x[0], 0)?)?)));
            check("min axis 1", &[a.clone()], |x| Ok(sum(&mul(&min_axis(&x[0], 1)?, &sum_axis(&x[0], 1)?)?)));
            // (a position read twice sums its two shares; one counted from
            // the end is the number it names)
            let positions = CrowTensor::from_vec(vec![0.0, 5.0, 5.0, 11.0, -1.0, 7.0], vec![2, 3]);
            check("gather", &[a.clone()], |x| Ok(sum(&mul(&gather(&x[0], &positions)?, &gather(&x[0], &positions)?)?)));
            check("row", &[a.clone()], |x| Ok(sum(&mul(&at(&x[0], 1)?, &at(&x[0], -1)?)?)));
            check("number", &[row.clone()], |x| mul(&at(&x[0], 2)?, &at(&x[0], -4)?));
            check("slice", &[a.clone()], |x| Ok(sum(&mul(&slice(&x[0], 1, 3), &slice(&x[0], 0, 2))?)));
            check("slice past the end", &[row.clone()], |x| Ok(sum(&mul(&slice(&x[0], -2, 9), &slice(&x[0], -2, 9))?)));
            check("concat", &[a.clone(), b.clone()], |x| Ok(sum(&mul(&concat(&x[0], &x[1], 0)?, &concat(&x[1], &x[0], 0)?)?)));
            check("concat axis 1", &[a.clone(), col.clone()], |x| Ok(sum(&mul(&concat(&x[0], &x[1], 1)?, &concat(&x[1], &x[0], 1)?)?)));
            // (a stack along each axis, read against itself reversed, so
            // each item's share differs; and a list made a tensor)
            for axis in 0..3 {
                check("stack", &[a.clone(), b.clone()], move |x| Ok(sum(&mul(&stack(&[x[0].clone(), x[1].clone()], axis)?, &stack(&[x[1].clone(), x[0].clone()], axis)?)?)));
            }
            check("elements", &[a.clone(), b.clone()], |x| {
                let m = from_elements(&[x[0].clone(), x[1].clone(), x[0].clone()])?;
                Ok(sum(&mul(&m, &from_elements(&[x[1].clone(), x[0].clone(), x[1].clone()])?)?))
            });
            check("elements of numbers", &[row.clone()], |x| {
                let v = from_elements(&[at(&x[0], 0)?, at(&x[0], 1)?, at(&x[0], 0)?])?;
                Ok(sum(&mul(&v, &v)?))
            });
            let cond = a.map(|v| if v > 0.3 { 1.0 } else { 0.0 });
            check("where", &[a.clone(), b.clone()], |x| Ok(sum(&mul(&where_(&cond, &x[0], &x[1])?, &x[0])?)));
        }
    }

    /// The reads a layer makes of one vector of parameters: a gather's
    /// answer is the untraced index's, and a read of no parameter is a
    /// constant (no node, no gradient).
    #[test]
    fn a_read_by_positions_is_the_index_and_reaches_what_it_read() {
        let t = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0], vec![2, 2]);
        let p = CrowTensor::from_vec(vec![3.0, 0.0, 3.0], vec![3]);
        let grads = gradient(std::slice::from_ref(&t), |x| Ok(sum(&gather(&x[0], &p)?))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![1.0, 0.0, 0.0, 2.0]);
        assert_eq!(gather(&Traced::constant(t.clone()), &p).map(|g| g.value.to_vec()), Ok(vec![40.0, 10.0, 40.0]));
        assert_eq!(at(&Traced::constant(t.clone()), -1).map(|r| r.value.to_vec()), Ok(vec![30.0, 40.0]));
        assert!(at(&Traced::constant(t.clone()), 2).is_err());
        assert!(gather(&Traced::constant(t), &CrowTensor::from_vec(vec![4.0], vec![1])).is_err());
    }

    /// A small network's loss, differentiated with respect to every weight,
    /// against finite differences: the chain through matmul, a bias
    /// broadcast, and relu, sigmoid and the binary cross-entropy written as
    /// the formulas pie's nn states them in Crow.
    #[test]
    fn a_networks_loss_differentiates_to_the_weights() {
        let mut g = Gen(7);
        let x = g.tensor(&[6, 3], -1.0, 1.0);
        let y = CrowTensor::from_vec(vec![1.0, 0.0, 1.0, 1.0, 0.0, 0.0], vec![6]);
        let w1 = g.tensor(&[3, 5], -0.5, 0.5);
        let b1 = g.tensor(&[1, 5], -0.5, 0.5);
        let w2 = g.tensor(&[5, 1], -0.5, 0.5);
        let b2 = g.tensor(&[1, 1], -0.5, 0.5);
        let one = Traced::number(1.0);
        let relu = |z: &Traced| clip(z, &Traced::number(0.0), &Traced::number(f64::INFINITY));
        let sigmoid = |z: &Traced| div(&Traced::number(1.0), &add(&Traced::number(1.0), &exp(&neg(z)))?);
        let bce = |p: &Traced, t: &Traced| -> Result<Traced, String> {
            let p = clip(p, &Traced::number(1e-12), &Traced::number(1.0 - 1e-12))?;
            let hit = mul(t, &ln(&p))?;
            let miss = mul(&sub(&one, t)?, &ln(&sub(&one, &p)?))?;
            Ok(neg(&mean(&add(&hit, &miss)?)))
        };
        check("network", &[w1, b1, w2, b2], |p| {
            let h = relu(&add(&matmul(&Traced::constant(x.clone()), &p[0])?, &p[1])?)?;
            let out = sigmoid(&add(&matmul(&h, &p[2])?, &p[3])?)?;
            bce(&flatten(&out), &Traced::constant(y.clone()))
        });
    }

    /// `floor`, `ceil` and `round` are constants of the tape: the gradient
    /// through `sum[floor[x] .* x]` is floor[x] itself; and an extreme's
    /// gradient reaches the first element holding it.
    #[test]
    fn a_step_is_a_constant_and_an_extreme_reaches_its_element() {
        let a = CrowTensor::from_vec(vec![0.5, 1.5, -0.5, 2.5], vec![4]);
        let grads = gradient(&[a.clone()], |x| Ok(sum(&mul(&floor(&x[0]), &x[0])?))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![0.0, 1.0, -1.0, 2.0]);
        let grads = gradient(&[a.clone()], |x| Ok(sum(&mul(&round(&x[0]), &ceil(&x[0]))?))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![0.0; 4]);
        let tie = CrowTensor::from_vec(vec![1.0, 3.0, 3.0, 2.0], vec![4]);
        let grads = gradient(&[tie.clone()], |x| Ok(max(&x[0]))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![0.0, 1.0, 0.0, 0.0]);
        let grads = gradient(&[tie.clone()], |x| Ok(min(&x[0]))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![1.0, 0.0, 0.0, 0.0]);
        let m = CrowTensor::from_vec(vec![1.0, 5.0, 3.0, 4.0, 2.0, 6.0], vec![2, 3]);
        let grads = gradient(&[m.clone()], |x| Ok(sum(&max_axis(&x[0], 1)?))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let grads = gradient(&[m.clone()], |x| Ok(sum(&min_axis(&x[0], 0)?))).expect("a gradient");
        assert_eq!(grads[0].to_vec(), vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
        assert_eq!(max_axis(&Traced::constant(m.clone()), 2).map(|_| ()), Err("max: axis 2 is outside a tensor of 2 dimensions".to_string()));
        assert_eq!(clip(&Traced::constant(m), &Traced::constant(CrowTensor::zeros(vec![2])), &Traced::number(1.0)).map(|_| ()), Err("clip: the bounds are numbers".to_string()));
    }

    /// A traced function's answer is the untraced function's to the bit,
    /// and a parameter the answer does not depend on has a zero gradient;
    /// a function that answers a tensor, and grad inside grad, are the
    /// reason.
    #[test]
    fn a_traced_answer_is_the_operators_answer() {
        let a = CrowTensor::from_vec(vec![0.1, 0.2, 0.3], vec![3]);
        let b = CrowTensor::from_vec(vec![0.4, 0.5, 0.6], vec![3]);
        let plain = a.mul(&b).add_scalar(0.7).sum();
        let traced = gradient(&[a.clone(), b.clone()], |x| Ok(sum(&add(&mul(&x[0], &x[1])?, &Traced::number(0.7))?))).map(|g| g.len());
        assert_eq!(traced, Ok(2));
        let out = {
            let xs = [Traced::constant(a.clone()), Traced::constant(b.clone())];
            sum(&add(&mul(&xs[0], &xs[1]).expect("meet"), &Traced::number(0.7)).expect("meet")).as_number().expect("a number")
        };
        assert_eq!(out.to_bits(), plain.to_bits());
        let unused = gradient(&[a.clone(), b.clone()], |x| Ok(sum(&x[0]))).expect("a gradient");
        assert_eq!(unused[1].to_vec(), vec![0.0; 3]);
        assert_eq!(gradient(&[a.clone()], |x| Ok(x[0].clone())).map(|_| ()), Err("grad: the function answers a tensor of shape [3], and a gradient is of a function that answers a number".to_string()));
        let nested = gradient(&[a.clone()], |x| {
            let inner = gradient(&[a.clone()], |y| Ok(sum(&y[0])));
            inner.map(|_| sum(&x[0]))
        });
        assert_eq!(nested.map(|_| ()), Err("grad: a function grad differentiates cannot call grad".to_string()));
        assert!(TAPE.with(|t| t.borrow().is_none()), "the tape is closed after a gradient");
    }
}
