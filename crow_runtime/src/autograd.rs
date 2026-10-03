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
        return CrowTensor::scalar(g.sum());
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
        let db = g.mul(&out).mul(&av.map(f64::ln));
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
    let node = record(vec![a.node], move |g| vec![CrowTensor::full(shape.clone(), g.sum())]);
    Traced::from(CrowTensor::scalar(a.value.sum()), node)
}

/// `mean[t]`: a number.
pub fn mean(a: &Traced) -> Traced {
    let shape = a.value.shape().to_vec();
    let n = a.value.len().max(1) as f64;
    let node = record(vec![a.node], move |g| vec![CrowTensor::full(shape.clone(), g.sum() / n)]);
    Traced::from(CrowTensor::scalar(a.value.mean()), node)
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
    let node = record(vec![a.node], move |g| vec![spread(g, &shape, axis).mul_scalar(1.0 / n)]);
    Ok(Traced::from(a.value.mean_axis(axis), node))
}

/// A gradient of a reduction along `axis` spread back over the input's
/// shape: each input element gets its slice's gradient.
fn spread(g: &CrowTensor, shape: &[usize], axis: usize) -> CrowTensor {
    let mut kept = shape.to_vec();
    kept[axis] = 1;
    let g = if g.ndim() == 0 { g.reshape(kept.clone()) } else { g.reshape(kept) };
    CrowTensor::zeros(shape.to_vec()).add(&g)
}

fn unary(a: &Traced, value: CrowTensor, local: impl Fn() -> CrowTensor + 'static) -> Traced {
    let node = record(vec![a.node], move |g| vec![g.mul(&local())]);
    Traced::from(value, node)
}

pub fn exp(a: &Traced) -> Traced {
    let value = a.value.exp();
    let out = value.clone();
    unary(a, value, move || out.clone())
}

pub fn ln(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.map(f64::ln), move || av.map(|x| 1.0 / x))
}

pub fn sqrt(a: &Traced) -> Traced {
    let value = a.value.sqrt();
    let out = value.clone();
    unary(a, value, move || out.map(|s| 0.5 / s))
}

pub fn tanh(a: &Traced) -> Traced {
    let value = a.value.tanh();
    let out = value.clone();
    unary(a, value, move || out.map(|t| 1.0 - t * t))
}

pub fn abs(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, a.value.map(f64::abs), move || av.map(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }))
}

pub fn relu(a: &Traced) -> Traced {
    let av = a.value.clone();
    unary(a, crate::nn::relu(&a.value), move || av.map(|x| if x > 0.0 { 1.0 } else { 0.0 }))
}

pub fn sigmoid(a: &Traced) -> Traced {
    let value = crate::nn::sigmoid(&a.value);
    let out = value.clone();
    unary(a, value, move || out.map(|s| s * (1.0 - s)))
}

/// `softmax[t]`: the tensor's elements to one, as the runtime's softmax
/// normalizes (one row of logits).
pub fn softmax(a: &Traced) -> Traced {
    let value = crate::nn::softmax(&a.value);
    let out = value.clone();
    let node = record(vec![a.node], move |g| {
        // d = s .* (g - sum(g .* s))
        let dot = g.mul(&out).sum();
        vec![out.mul(&g.sub_scalar(dot))]
    });
    Traced::from(value, node)
}

/// `mse[pred, target]`: a number.
pub fn mse(p: &Traced, t: &Traced) -> Result<Traced, String> {
    let value = crate::nn::mse(&p.value, &t.value)?;
    let (pv, tv) = (p.value.clone(), t.value.clone());
    let n = p.value.len().max(1) as f64;
    let node = record(vec![p.node, t.node], move |g| {
        let d = pv.sub(&tv).mul_scalar(2.0 * g.sum() / n);
        vec![d.clone(), d.neg()]
    });
    Ok(Traced::from(CrowTensor::scalar(value), node))
}

/// `bce[pred, target]`: a number, each prediction read within
/// [eps, 1 - eps] as the loss reads it (no gradient past the clamp).
pub fn bce(p: &Traced, t: &Traced) -> Result<Traced, String> {
    let value = crate::nn::bce(&p.value, &t.value)?;
    let (pv, tv) = (p.value.to_vec(), t.value.to_vec());
    let shape = p.value.shape().to_vec();
    let n = pv.len().max(1) as f64;
    let node = record(vec![p.node, t.node], move |g| {
        let eps = crate::nn::BCE_EPS;
        let scale = g.sum() / n;
        let dp: Vec<f64> = pv
            .iter()
            .zip(&tv)
            .map(|(&p, &t)| {
                if !(eps..=1.0 - eps).contains(&p) {
                    return 0.0;
                }
                (p - t) / (p * (1.0 - p)) * scale
            })
            .collect();
        let dt: Vec<f64> = pv.iter().map(|&p| { let c = p.max(eps).min(1.0 - eps); -(c.ln() - (1.0 - c).ln()) * scale }).collect();
        vec![CrowTensor::from_vec(dp, shape.clone()), CrowTensor::from_vec(dt, shape.clone())]
    });
    Ok(Traced::from(CrowTensor::scalar(value), node))
}

/// `crossentropy[logits, targets]`: a number; the targets are classes
/// and get no gradient.
pub fn crossentropy(logits: &Traced, targets: &Traced) -> Result<Traced, String> {
    let value = crate::nn::crossentropy(&logits.value, &targets.value)?;
    let (lv, tv) = (logits.value.clone(), targets.value.to_vec());
    let node = record(vec![logits.node, targets.node], move |g| {
        // each row's softmax less one at its class, over the batch
        let rows = if lv.ndim() == 1 { 1 } else { lv.shape()[0] };
        let classes = lv.len() / rows.max(1);
        let data = lv.to_vec();
        let scale = g.sum() / rows.max(1) as f64;
        let mut d = Vec::with_capacity(data.len());
        for (r, &t) in tv.iter().enumerate() {
            let row = &data[r * classes..(r + 1) * classes];
            let max_val = row.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let exps: Vec<f64> = row.iter().map(|&x| (x - max_val).exp()).collect();
            let sum: f64 = exps.iter().sum();
            for (k, e) in exps.iter().enumerate() {
                let s = e / sum - if k == t as usize { 1.0 } else { 0.0 };
                d.push(s * scale);
            }
        }
        vec![CrowTensor::from_vec(d, lv.shape().to_vec()), CrowTensor::zeros(vec![tv.len()])]
    });
    Ok(Traced::from(CrowTensor::scalar(value), node))
}

/// `linear[x, w, b]`: x * w + b.
pub fn linear(x: &Traced, w: &Traced, b: &Traced) -> Result<Traced, String> {
    let xw = matmul(x, w)?;
    if xw.value.meets(&b.value, "+").is_err() {
        return Err(format!("linear: a bias of shape {:?} does not meet x * w of shape {:?}", b.value.shape(), xw.value.shape()));
    }
    add(&xw, b)
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
            check("sqrt", &[positive.clone()], |x| Ok(sum(&sqrt(&x[0]))));
            check("tanh", &[a.clone()], |x| Ok(sum(&tanh(&x[0]))));
            check("abs", &[a.clone()], |x| Ok(sum(&abs(&x[0]))));
            check("relu", &[a.clone()], |x| Ok(sum(&mul(&relu(&x[0]), &x[0])?)));
            check("sigmoid", &[a.clone()], |x| Ok(sum(&mul(&sigmoid(&x[0]), &x[0])?)));
            check("softmax", &[a.clone()], |x| Ok(sum(&mul(&softmax(&x[0]), &x[1.min(0)])?)));
            check("mse", &[a.clone(), b.clone()], |x| mse(&x[0], &x[1]));
            let probs = g.tensor(&[5], 0.1, 0.9);
            let truth = CrowTensor::from_vec(vec![1.0, 0.0, 1.0, 1.0, 0.0], vec![5]);
            check("bce", &[probs.clone(), truth.clone()], |x| bce(&x[0], &x[1]));
            let logits = g.tensor(&[3, 4], -2.0, 2.0);
            let classes = CrowTensor::from_vec(vec![0.0, 3.0, 1.0], vec![3]);
            check("crossentropy", &[logits.clone()], |x| crossentropy(&x[0], &Traced::constant(classes.clone())));
            check("linear", &[a.clone(), m.clone(), bias.clone()], |x| Ok(sum(&mul(&linear(&x[0], &x[1], &x[2])?, &linear(&x[0], &x[1], &x[2])?)?)));
        }
    }

    /// A small network's loss, differentiated with respect to every weight,
    /// against finite differences: the chain through matmul, a bias
    /// broadcast, relu, sigmoid and the loss.
    #[test]
    fn a_networks_loss_differentiates_to_the_weights() {
        let mut g = Gen(7);
        let x = g.tensor(&[6, 3], -1.0, 1.0);
        let y = CrowTensor::from_vec(vec![1.0, 0.0, 1.0, 1.0, 0.0, 0.0], vec![6, 1]);
        let w1 = g.tensor(&[3, 5], -0.5, 0.5);
        let b1 = g.tensor(&[1, 5], -0.5, 0.5);
        let w2 = g.tensor(&[5, 1], -0.5, 0.5);
        let b2 = g.tensor(&[1, 1], -0.5, 0.5);
        check("network", &[w1, b1, w2, b2], |p| {
            let h = relu(&add(&matmul(&Traced::constant(x.clone()), &p[0])?, &p[1])?);
            let out = sigmoid(&add(&matmul(&h, &p[2])?, &p[3])?);
            bce(&flatten(&out), &Traced::constant(CrowTensor::from_vec(y.to_vec(), vec![6])))
        });
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
