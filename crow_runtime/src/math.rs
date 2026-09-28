//! The math module's builtins, once (docs/bug-classes.md §4). Each takes
//! what a Crow program hands it — sizes as ints, parameters as floats,
//! tensors — checks it, and answers the tensor or the reason. The
//! interpreter and the compiled program both call these functions through
//! the builtin table: a size the interpreter refused and the compiled
//! program cast to a huge `usize` (`zeros[-2]`: "capacity overflow") is one
//! check now. The tensor kernels below them assume what these checked.

use crate::stops;
use crate::tensor::CrowTensor;

/// The sizes a constructor was handed, each a count.
fn shape(what: &str, sizes: &[i64]) -> Result<Vec<usize>, String> {
    sizes.iter().map(|&n| stops::size(what, n)).collect()
}

/// A sample's shape: its sizes, or one value when none are given.
fn sample_shape(what: &str, sizes: &[i64]) -> Result<Vec<usize>, String> {
    let s = shape(what, sizes)?;
    Ok(if s.is_empty() { vec![1] } else { s })
}

/// `zeros[d1, d2, …]`.
pub fn zeros(sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::zeros(shape("zeros", sizes)?))
}

/// `ones[d1, d2, …]`.
pub fn ones(sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::ones(shape("ones", sizes)?))
}

/// `randn[d1, d2, …]`: standard normal draws.
pub fn randn(sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::randn(shape("randn", sizes)?))
}

/// `full[d1, d2, …, value]`.
pub fn full(sizes: &[i64], value: f64) -> Result<CrowTensor, String> {
    Ok(CrowTensor::full(shape("full", sizes)?, value))
}

/// `eye[n]`: the n×n identity.
pub fn eye(n: i64) -> Result<CrowTensor, String> {
    Ok(CrowTensor::eye(stops::size("eye", n)?))
}

/// `linspace[start, stop, n]`: n evenly spaced values, both ends included.
pub fn linspace(start: f64, stop: f64, n: i64) -> Result<CrowTensor, String> {
    Ok(CrowTensor::linspace(start, stop, stops::size("linspace", n)?))
}

/// `arange[start, stop, step]`: from start toward stop, stop excluded.
pub fn arange(start: f64, stop: f64, step: f64) -> Result<CrowTensor, String> {
    if step == 0.0 {
        return Err("arange: step cannot be zero".to_string());
    }
    Ok(CrowTensor::arange(start, stop, step))
}

/// `diag[t]`: a vector's diagonal matrix, or a matrix's diagonal.
pub fn diag(t: &CrowTensor) -> Result<CrowTensor, String> {
    CrowTensor::diag(t)
}

/// `normal[mean, std, d1, …]`.
pub fn normal(mean: f64, std: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    if !(std >= 0.0) {
        return Err(format!("normal: std must be 0 or more (got {})", std));
    }
    Ok(CrowTensor::normal(sample_shape("normal", sizes)?, mean, std))
}

/// `uniform[low, high, d1, …]`.
pub fn uniform(low: f64, high: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::rand_range(sample_shape("uniform", sizes)?, low, high))
}

/// `poisson[lambda, d1, …]`.
pub fn poisson(lambda: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::poisson(sample_shape("poisson", sizes)?, lambda)
}

/// `bernoulli[p, d1, …]`.
pub fn bernoulli(p: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::bernoulli(sample_shape("bernoulli", sizes)?, p)
}

/// `gamma_dist[alpha, beta, d1, …]`.
pub fn gamma_dist(alpha: f64, beta: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::gamma_dist(sample_shape("gamma_dist", sizes)?, alpha, beta)
}

/// `beta_dist[alpha, beta, d1, …]`.
pub fn beta_dist(alpha: f64, beta: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::beta_dist(sample_shape("beta_dist", sizes)?, alpha, beta)
}

/// `exponential[lambda, d1, …]`.
pub fn exponential(lambda: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::exponential(sample_shape("exponential", sizes)?, lambda)
}

// ── linear algebra ──────────────────────────────────────────────────
// The kernels take a matrix of the right shape; these check it, so a
// vector, a rank-3 tensor or a rectangle where a square is needed is the
// program's reason, never a kernel's panic.

/// The rows and columns of a matrix.
fn matrix(what: &str, t: &CrowTensor) -> Result<(usize, usize), String> {
    match t.shape() {
        [r, c] => Ok((*r, *c)),
        s => Err(format!("{}: expected a matrix (2D), got a {}D tensor", what, s.len())),
    }
}

/// The side of a square matrix.
fn square(what: &str, t: &CrowTensor) -> Result<usize, String> {
    let (r, c) = matrix(what, t)?;
    if r != c {
        return Err(format!("{}: the matrix must be square, got {}×{}", what, r, c));
    }
    Ok(r)
}

/// `det[a]`.
pub fn det(a: &CrowTensor) -> Result<f64, String> {
    square("det", a)?;
    Ok(a.det())
}

/// `inv[a]`.
pub fn inv(a: &CrowTensor) -> Result<CrowTensor, String> {
    square("inv", a)?;
    Ok(a.inv())
}

/// `svd[a]`: `[u, s, vt]`.
pub fn svd(a: &CrowTensor) -> Result<Vec<CrowTensor>, String> {
    matrix("svd", a)?;
    let (u, s, vt) = a.svd_decomp()?;
    Ok(vec![u, s, vt])
}

/// `qr[a]`: `[q, r]`.
pub fn qr(a: &CrowTensor) -> Result<Vec<CrowTensor>, String> {
    matrix("qr", a)?;
    let (q, r) = a.qr_decomp();
    Ok(vec![q, r])
}

/// `cholesky[a]`: the lower factor of a positive definite matrix.
pub fn cholesky(a: &CrowTensor) -> Result<CrowTensor, String> {
    square("cholesky", a)?;
    a.cholesky_decomp()
}

/// `eigen[a]`: `[real parts, imaginary parts]` of the eigenvalues.
pub fn eigen(a: &CrowTensor) -> Result<Vec<CrowTensor>, String> {
    square("eigen", a)?;
    let (re, im) = a.eigen_decomp()?;
    Ok(vec![re, im])
}

/// `norm[t]`: the Euclidean norm of every element.
pub fn norm(t: &CrowTensor) -> f64 {
    t.norm(2.0)
}

/// `norm[t, p]`.
pub fn norm_p(t: &CrowTensor, p: f64) -> f64 {
    t.norm(p)
}

/// `dot[a, b]`: of two vectors of one length.
pub fn dot(a: &CrowTensor, b: &CrowTensor) -> Result<f64, String> {
    if a.len() != b.len() {
        return Err(format!("dot: the vectors must be one length, got {} and {}", a.len(), b.len()));
    }
    Ok(a.dot(b))
}

/// `pinv[a]`: the pseudo-inverse.
pub fn pinv(a: &CrowTensor) -> Result<CrowTensor, String> {
    matrix("pinv", a)?;
    a.pinv()
}

/// `lstsq[a, b]`: the least-squares solution of `a x = b`.
pub fn lstsq(a: &CrowTensor, b: &CrowTensor) -> Result<CrowTensor, String> {
    let (rows, _) = matrix("lstsq", a)?;
    let b_rows = b.shape().first().copied().unwrap_or(0);
    if b.shape().len() > 2 || b_rows != rows {
        return Err(format!("lstsq: b must have the matrix's {} rows, got shape {:?}", rows, b.shape()));
    }
    a.lstsq(b)
}

/// `triu[a]`.
pub fn triu(a: &CrowTensor) -> Result<CrowTensor, String> {
    matrix("triu", a)?;
    Ok(a.triu())
}

/// `tril[a]`.
pub fn tril(a: &CrowTensor) -> Result<CrowTensor, String> {
    matrix("tril", a)?;
    Ok(a.tril())
}

/// `rank[a]`.
pub fn rank(a: &CrowTensor) -> Result<i64, String> {
    matrix("rank", a)?;
    Ok(a.rank()? as i64)
}

/// `cond[a]`.
pub fn cond(a: &CrowTensor) -> Result<f64, String> {
    matrix("cond", a)?;
    a.cond()
}


// ── reductions, shapes, selection ───────────────────────────────────
// An axis, a shape or a length a program gets wrong is the reason here;
// the kernels assert it, and panicked both modes (`sum[t, 5]`,
// `reshape[t, 7]`, `transpose` of a vector).

/// An axis of `t`: 0 up to its rank.
fn axis(what: &str, t: &CrowTensor, axis: i64) -> Result<usize, String> {
    match usize::try_from(axis) {
        Ok(a) if a < t.ndim() => Ok(a),
        _ => Err(format!("{}: axis {} is out of range for a {}D tensor", what, axis, t.ndim())),
    }
}

/// `sum[t]`.
pub fn sum(t: &CrowTensor) -> f64 {
    t.sum()
}

/// `sum[t, axis]`.
pub fn sum_axis(t: &CrowTensor, ax: i64) -> Result<CrowTensor, String> {
    Ok(t.sum_axis(axis("sum", t, ax)?))
}

/// `mean[t]`.
pub fn mean(t: &CrowTensor) -> f64 {
    t.mean()
}

/// `mean[t, axis]`.
pub fn mean_axis(t: &CrowTensor, ax: i64) -> Result<CrowTensor, String> {
    Ok(t.mean_axis(axis("mean", t, ax)?))
}

/// `min[t]` of a tensor.
pub fn min(t: &CrowTensor) -> f64 {
    t.min()
}

/// `min[t, axis]`.
pub fn min_axis(t: &CrowTensor, ax: i64) -> Result<CrowTensor, String> {
    Ok(t.min_axis(axis("min", t, ax)?))
}

/// `max[t]` of a tensor.
pub fn max(t: &CrowTensor) -> f64 {
    t.max()
}

/// `max[t, axis]`.
pub fn max_axis(t: &CrowTensor, ax: i64) -> Result<CrowTensor, String> {
    Ok(t.max_axis(axis("max", t, ax)?))
}

/// `prod[t]`.
pub fn prod(t: &CrowTensor) -> f64 {
    t.prod()
}

/// `prod[t, axis]`.
pub fn prod_axis(t: &CrowTensor, ax: i64) -> Result<CrowTensor, String> {
    Ok(t.prod_axis(axis("prod", t, ax)?))
}

/// `argmax[t]`: the flat index of the largest value, 0 of an empty tensor.
pub fn argmax(t: &CrowTensor) -> i64 {
    t.argmax() as i64
}

/// `argmin[t]`.
pub fn argmin(t: &CrowTensor) -> i64 {
    t.argmin() as i64
}

/// `cumsum[t]`.
pub fn cumsum(t: &CrowTensor) -> CrowTensor {
    t.cumsum()
}

/// `cumprod[t]`.
pub fn cumprod(t: &CrowTensor) -> CrowTensor {
    t.cumprod()
}

/// `argsort[t]`.
pub fn argsort(t: &CrowTensor) -> CrowTensor {
    t.argsort()
}

/// `topk[t, k]`: the k largest values, largest first; all of them when k
/// is past the length.
pub fn topk(t: &CrowTensor, k: i64) -> Result<CrowTensor, String> {
    Ok(t.topk(stops::size("topk", k)?))
}

/// `unique[t]`: the distinct values, sorted.
pub fn unique(t: &CrowTensor) -> CrowTensor {
    t.unique()
}

/// `where[cond, a, b]`: a's value where cond holds, b's elsewhere.
pub fn where_(cond: &CrowTensor, a: &CrowTensor, b: &CrowTensor) -> Result<CrowTensor, String> {
    if cond.len() != a.len() || cond.len() != b.len() {
        return Err(format!("where: cond, a and b must be one length, got {}, {} and {}", cond.len(), a.len(), b.len()));
    }
    Ok(cond.where_cond(a, b))
}

/// `reshape[t, d1, d2, …]`: the same values in a new shape of as many.
pub fn reshape(t: &CrowTensor, sizes: &[i64]) -> Result<CrowTensor, String> {
    let shape: Vec<usize> = sizes.iter().map(|&n| stops::size("reshape", n)).collect::<Result<_, _>>()?;
    let n: usize = shape.iter().product();
    if n != t.len() {
        return Err(format!("reshape: {} values cannot fill shape {:?} ({} values)", t.len(), shape, n));
    }
    Ok(t.reshape(shape))
}

/// `flatten[t]`.
pub fn flatten(t: &CrowTensor) -> CrowTensor {
    t.flatten()
}

/// `transpose[t]`: its last two dimensions swapped.
pub fn transpose(t: &CrowTensor) -> Result<CrowTensor, String> {
    if t.ndim() < 2 {
        return Err(format!("transpose: needs at least 2 dimensions, got {}", t.ndim()));
    }
    Ok(t.transpose())
}

/// `clip[t, lo, hi]`.
pub fn clip(t: &CrowTensor, lo: f64, hi: f64) -> CrowTensor {
    t.clip(lo, hi)
}

/// `atan2[y, x]`, element-wise, the shapes broadcasting.
pub fn atan2(y: &CrowTensor, x: &CrowTensor) -> Result<CrowTensor, String> {
    y.meets(x, "atan2")?;
    Ok(y.atan2(x))
}

// ── element-wise ────────────────────────────────────────────────────
// Each takes a number or a tensor, one rule for both: a number is the
// tensor's rule applied to one value, not a Rust method of its own
// (`sign[0.0]` was 1 compiled, from f64::signum, and 0 interpreted, from
// the tensor's rule). The tensor forms keep their GPU dispatch.

/// The sign of a value: 1, -1, or 0 for zero.
fn sign_of(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

macro_rules! elementwise {
    ($($number:ident, $tensor:ident, $rule:expr, $method:ident;)*) => {
        $(
            /// Of a number.
            pub fn $number(x: f64) -> f64 {
                let rule: fn(f64) -> f64 = $rule;
                rule(x)
            }
            /// Of a tensor, value by value.
            pub fn $tensor(t: &CrowTensor) -> CrowTensor {
                t.$method()
            }
        )*
    };
}

elementwise! {
    exp_n, exp_t, |x| x.exp(), exp;
    ln_n, ln_t, |x| x.ln(), log;
    log2_n, log2_t, |x| x.log2(), log2;
    log10_n, log10_t, |x| x.log10(), log10;
    sin_n, sin_t, |x| x.sin(), sin;
    cos_n, cos_t, |x| x.cos(), cos;
    tan_n, tan_t, |x| x.tan(), tan;
    asin_n, asin_t, |x| x.asin(), asin;
    acos_n, acos_t, |x| x.acos(), acos;
    atan_n, atan_t, |x| x.atan(), atan;
    sinh_n, sinh_t, |x| x.sinh(), sinh;
    cosh_n, cosh_t, |x| x.cosh(), cosh;
    tanh_n, tanh_t, |x| x.tanh(), tanh;
    sign_n, sign_t, sign_of, sign;
}

/// `abs`, `sqrt`, `floor`, `ceil`, `round` of a tensor; of a number they are
/// the core builtins.
pub fn abs_t(t: &CrowTensor) -> CrowTensor {
    t.abs()
}
pub fn sqrt_t(t: &CrowTensor) -> CrowTensor {
    t.sqrt()
}
pub fn floor_t(t: &CrowTensor) -> CrowTensor {
    t.floor()
}
pub fn ceil_t(t: &CrowTensor) -> CrowTensor {
    t.ceil()
}
pub fn round_t(t: &CrowTensor) -> CrowTensor {
    t.round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_checked_once_for_both_modes() {
        assert_eq!(zeros(&[2, 3]).map(|t| t.shape().to_vec()), Ok(vec![2, 3]));
        assert_eq!(zeros(&[-2]).map(|_| ()), Err("zeros: a size cannot be negative (got -2)".to_string()));
        assert_eq!(eye(-1).map(|_| ()), Err("eye: a size cannot be negative (got -1)".to_string()));
        assert_eq!(full(&[2], 7.0).map(|t| t.to_vec()), Ok(vec![7.0, 7.0]));
        assert_eq!(arange(0.0, 1.0, 0.0).map(|_| ()), Err("arange: step cannot be zero".to_string()));
        assert_eq!(linspace(0.0, 1.0, 3).map(|t| t.to_vec()), Ok(vec![0.0, 0.5, 1.0]));
    }

    #[test]
    fn a_distributions_parameters_are_checked() {
        assert_eq!(poisson(-1.0, &[]).map(|_| ()), Err("poisson: lambda must be > 0".to_string()));
        assert_eq!(bernoulli(2.0, &[3]).map(|_| ()), Err("bernoulli: p must be in [0, 1]".to_string()));
        assert_eq!(exponential(-1.0, &[]).map(|_| ()), Err("exponential: lambda must be 0 or more".to_string()));
        assert_eq!(normal(0.0, -1.0, &[]).map(|_| ()), Err("normal: std must be 0 or more (got -1)".to_string()));
        assert_eq!(poisson(2.0, &[]).map(|t| t.shape().to_vec()), Ok(vec![1]));
        assert_eq!(uniform(0.0, 1.0, &[4]).map(|t| t.shape().to_vec()), Ok(vec![4]));
    }

    #[test]
    fn a_matrix_is_the_shape_its_function_needs() {
        let v = CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
        let rect = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let sq = CrowTensor::from_vec(vec![4.0, 2.0, 2.0, 3.0], vec![2, 2]);
        assert_eq!(det(&v).map(|_| ()), Err("det: expected a matrix (2D), got a 1D tensor".to_string()));
        assert_eq!(inv(&rect).map(|_| ()), Err("inv: the matrix must be square, got 2×3".to_string()));
        assert!(det(&sq).is_ok());
        assert_eq!(dot(&v, &CrowTensor::from_vec(vec![1.0], vec![1])).map(|_| ()), Err("dot: the vectors must be one length, got 2 and 1".to_string()));
        assert_eq!(svd(&rect).map(|parts| parts.len()), Ok(3));
        assert_eq!(rank(&rect), Ok(2));
    }

    /// A matrix that is not positive definite is cholesky's reason: it
    /// panicked both modes.
    #[test]
    fn a_decomposition_that_fails_says_why() {
        let not_pd = CrowTensor::from_vec(vec![1.0, 2.0, 2.0, 1.0], vec![2, 2]);
        assert_eq!(cholesky(&not_pd).map(|_| ()), Err("cholesky: the matrix is not positive definite".to_string()));
        let empty = CrowTensor::zeros(vec![0, 0]);
        assert_eq!(pinv(&empty).map(|t| t.shape().to_vec()), Ok(vec![0, 0]));
    }

    #[test]
    fn an_axis_a_shape_or_a_length_is_checked() {
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        assert_eq!(sum_axis(&m, 1).map(|t| t.to_vec()), Ok(vec![3.0, 7.0]));
        assert_eq!(sum_axis(&m, 2).map(|_| ()), Err("sum: axis 2 is out of range for a 2D tensor".to_string()));
        assert_eq!(mean_axis(&m, -1).map(|_| ()), Err("mean: axis -1 is out of range for a 2D tensor".to_string()));
        assert_eq!(reshape(&m, &[4]).map(|t| t.shape().to_vec()), Ok(vec![4]));
        assert_eq!(reshape(&m, &[3]).map(|_| ()), Err("reshape: 4 values cannot fill shape [3] (3 values)".to_string()));
        let v = CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
        assert_eq!(transpose(&v).map(|_| ()), Err("transpose: needs at least 2 dimensions, got 1".to_string()));
        assert!(where_(&v, &v, &m).is_err());
        assert!(atan2(&v, &CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3])).is_err());
    }

    /// A number's rule is the tensor's: sign of zero is 0 in both.
    #[test]
    fn a_number_and_a_tensor_share_one_rule() {
        assert_eq!(sign_n(0.0), 0.0);
        assert_eq!(sign_t(&CrowTensor::from_vec(vec![0.0, -2.0], vec![2])).to_vec(), vec![0.0, -1.0]);
        assert_eq!(exp_n(0.0), 1.0);
        assert_eq!(ln_t(&CrowTensor::from_vec(vec![1.0], vec![1])).to_vec(), vec![0.0]);
    }

    #[test]
    fn a_diagonal_needs_one_or_two_dimensions() {
        assert!(diag(&CrowTensor::zeros(vec![2, 2, 2])).is_err());
        assert_eq!(diag(&CrowTensor::from_vec(vec![1.0, 2.0], vec![2])).map(|t| t.shape().to_vec()), Ok(vec![2, 2]));
    }
}
