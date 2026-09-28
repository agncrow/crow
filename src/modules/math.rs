//! `use math`: every function is a row of the builtin table
//! (crate::table), its runtime function in `crow_runtime::math`, which both
//! modes call — `load` too, whose call is an effect the session layer
//! performs.
use crate::value::{CrowValue, Value};
use crate::modules::CrowModule;

pub struct MathModule;

pub(crate) const MATH_FUNCTIONS: &[&str] = &[
    // Tensor creation
    "zeros", "ones", "eye", "randn", "linspace", "arange", "full", "diag", "tensor",
    // Element-wise math (ln replaces log; abs/sqrt/floor/ceil/round overlap with core builtins)
    "exp", "ln", "log2", "log10",
    "sin", "cos", "sign", "clip", "mod",
    "tan", "asin", "acos", "atan", "atan2",
    "sinh", "cosh", "tanh",
    "abs", "sqrt", "floor", "ceil", "round",
    // Reductions (min/max overlap with core builtins)
    "sum", "mean", "min", "max", "prod",
    "argmax", "argmin",
    // Cumulative
    "cumsum", "cumprod",
    // Combining
    "stack",
    // Sorting & selection
    "argsort", "topk", "unique", "where",
    // Shape ops
    "reshape", "flatten", "transpose",
    // Linear algebra
    "solve", "inv", "det", "norm", "svd", "qr", "cholesky", "eigen",
    "dot", "pinv", "lstsq", "triu", "tril", "rank", "cond",
    // Random & distributions
    "seed", "normal", "uniform", "poisson", "bernoulli",
    "gamma_dist", "beta_dist", "exponential",
    "shuffle", "choice", "permutation",
    // Signal processing
    "fft", "fft2", "ifft", "conv", "conv2d", "hamming", "hanning",
    "stft", "spectrogram", "lowpass", "highpass", "bandpass",
    // Special functions
    "gamma", "erf", "bessel", "factorial", "choose",
    // Complex numbers
    "complex", "real", "imag", "magnitude", "phase", "conjugate",
    // Sparse
    "sparse", "dense", "nnz", "spmv",
    // I/O
    "load",
];

impl CrowModule for MathModule {
    fn name(&self) -> &str { "math" }

    fn functions(&self) -> &[&str] { MATH_FUNCTIONS }

    fn constants(&self) -> Vec<(&str, CrowValue)> {
        vec![
            ("pi", CrowValue::ok(Value::Float(std::f64::consts::PI))),
            ("e", CrowValue::ok(Value::Float(std::f64::consts::E))),
            ("inf", CrowValue::ok(Value::Float(f64::INFINITY))),
            ("nan", CrowValue::ok(Value::Float(f64::NAN))),
        ]
    }
}

/// Element-wise functions that also take a plain number.
pub(crate) const SCALAR_UNARY: &[&str] = &[
    "exp", "ln", "log2", "log10", "sign", "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh",
];

#[cfg(test)]
mod tests {
    /// A math function as a program calls it: through the registry, so a
    /// function with a row in the builtin table is its runtime function.
    fn call_math(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        crate::modules::ModuleRegistry::with_stdlib().call_in("math", name, args)
    }
    use crate::value::{CrowValue, Value};
    use crow_runtime::tensor::CrowTensor;

    // Helper: make CrowValue args for call_math
    fn int(n: i64) -> CrowValue { CrowValue::ok(Value::Int(n)) }
    fn float(f: f64) -> CrowValue { CrowValue::ok(Value::Float(f)) }
    fn tensor(data: Vec<f64>, shape: Vec<usize>) -> CrowValue {
        CrowValue::ok(Value::NumArray(Box::new(CrowTensor::from_vec(data, shape))))
    }
    fn tensor_1d(data: Vec<f64>) -> CrowValue {
        let len = data.len();
        CrowValue::ok(Value::NumArray(Box::new(CrowTensor::from_vec(data, vec![len]))))
    }

    fn unwrap_tensor(r: Result<CrowValue, String>) -> CrowTensor {
        let cv = r.expect("call_math failed");
        assert!(cv.is_ok(), "CrowValue is error: {}", cv.reason());
        match cv.value {
            Value::NumArray(t) => *t,
            other => panic!("expected NumArray, got {:?}", other),
        }
    }
    fn unwrap_float(r: Result<CrowValue, String>) -> f64 {
        let cv = r.expect("call_math failed");
        assert!(cv.is_ok(), "CrowValue is error: {}", cv.reason());
        match cv.value {
            Value::Float(f) => f,
            Value::Int(n) => n as f64,
            other => panic!("expected Float, got {:?}", other),
        }
    }
    fn unwrap_int(r: Result<CrowValue, String>) -> i64 {
        let cv = r.expect("call_math failed");
        assert!(cv.is_ok(), "CrowValue is error: {}", cv.reason());
        match cv.value {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }
    fn unwrap_array(r: Result<CrowValue, String>) -> Vec<CrowValue> {
        let cv = r.expect("call_math failed");
        assert!(cv.is_ok(), "CrowValue is error: {}", cv.reason());
        match cv.value {
            Value::Array(a) => a,
            other => panic!("expected Array, got {:?}", other),
        }
    }

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 || (a.is_nan() && b.is_nan())
    }
    fn assert_tensor_eq(t: &CrowTensor, expected: &[f64]) {
        let data = t.to_vec();
        assert_eq!(data.len(), expected.len(), "length mismatch: {} vs {}", data.len(), expected.len());
        for (i, (a, b)) in data.iter().zip(expected).enumerate() {
            assert!(approx_eq(*a, *b), "element {} mismatch: {} vs {}", i, a, b);
        }
    }

    // ===== Tensor Creation =====

    #[test]
    fn test_zeros() {
        let t = unwrap_tensor(call_math("zeros", &[int(3)]));
        assert_tensor_eq(&t, &[0.0, 0.0, 0.0]);
        assert_eq!(t.shape(), &[3]);
    }

    #[test]
    fn test_zeros_2d() {
        let t = unwrap_tensor(call_math("zeros", &[int(2), int(3)]));
        assert_tensor_eq(&t, &[0.0; 6]);
        assert_eq!(t.shape(), &[2, 3]);
    }

    #[test]
    fn test_ones() {
        let t = unwrap_tensor(call_math("ones", &[int(4)]));
        assert_tensor_eq(&t, &[1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_ones_2d() {
        let t = unwrap_tensor(call_math("ones", &[int(2), int(2)]));
        assert_tensor_eq(&t, &[1.0, 1.0, 1.0, 1.0]);
        assert_eq!(t.shape(), &[2, 2]);
    }

    #[test]
    fn test_eye() {
        let t = unwrap_tensor(call_math("eye", &[int(3)]));
        assert_tensor_eq(&t, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(t.shape(), &[3, 3]);
    }

    #[test]
    fn test_eye_wrong_args() {
        assert!(call_math("eye", &[]).is_err());
        assert!(call_math("eye", &[int(2), int(3)]).is_err());
    }

    #[test]
    fn test_randn() {
        let t = unwrap_tensor(call_math("randn", &[int(100)]));
        assert_eq!(t.to_vec().len(), 100);
        // Mean should be roughly 0 for large samples
    }

    #[test]
    fn test_linspace() {
        let t = unwrap_tensor(call_math("linspace", &[float(0.0), float(1.0), int(5)]));
        assert_tensor_eq(&t, &[0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn test_linspace_wrong_args() {
        assert!(call_math("linspace", &[float(0.0), float(1.0)]).is_err());
    }

    #[test]
    fn test_arange() {
        let t = unwrap_tensor(call_math("arange", &[float(0.0), float(5.0), float(1.0)]));
        assert_tensor_eq(&t, &[0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_arange_step() {
        let t = unwrap_tensor(call_math("arange", &[float(0.0), float(1.0), float(0.5)]));
        assert_tensor_eq(&t, &[0.0, 0.5]);
    }

    #[test]
    fn test_full() {
        let t = unwrap_tensor(call_math("full", &[int(2), int(3), float(7.0)]));
        assert_tensor_eq(&t, &[7.0; 6]);
        assert_eq!(t.shape(), &[2, 3]);
    }

    #[test]
    fn test_diag() {
        let t = unwrap_tensor(call_math("diag", &[tensor_1d(vec![1.0, 2.0, 3.0])]));
        assert_tensor_eq(&t, &[1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0]);
        assert_eq!(t.shape(), &[3, 3]);
    }

    #[test]
    fn test_tensor_from_numarray() {
        let input = tensor_1d(vec![1.0, 2.0, 3.0]);
        let t = unwrap_tensor(call_math("tensor", &[input]));
        assert_tensor_eq(&t, &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_tensor_from_array() {
        let arr = CrowValue::ok(Value::Array(vec![int(1), int(2), int(3)]));
        let t = unwrap_tensor(call_math("tensor", &[arr]));
        assert_tensor_eq(&t, &[1.0, 2.0, 3.0]);
    }

    // ===== Element-wise Math =====

    #[test]
    fn test_exp() {
        let t = unwrap_tensor(call_math("exp", &[tensor_1d(vec![0.0, 1.0])]));
        assert!(approx_eq(t.to_vec()[0], 1.0));
        assert!(approx_eq(t.to_vec()[1], std::f64::consts::E));
    }

    #[test]
    fn test_ln() {
        let t = unwrap_tensor(call_math("ln", &[tensor_1d(vec![1.0, std::f64::consts::E])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
        assert!(approx_eq(t.to_vec()[1], 1.0));
    }

    #[test]
    fn test_sqrt() {
        let t = unwrap_tensor(call_math("sqrt", &[tensor_1d(vec![4.0, 9.0, 16.0])]));
        assert_tensor_eq(&t, &[2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_sin() {
        let t = unwrap_tensor(call_math("sin", &[tensor_1d(vec![0.0, std::f64::consts::FRAC_PI_2])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
        assert!(approx_eq(t.to_vec()[1], 1.0));
    }

    #[test]
    fn test_cos() {
        let t = unwrap_tensor(call_math("cos", &[tensor_1d(vec![0.0, std::f64::consts::PI])]));
        assert!(approx_eq(t.to_vec()[0], 1.0));
        assert!(approx_eq(t.to_vec()[1], -1.0));
    }

    #[test]
    fn test_abs() {
        let t = unwrap_tensor(call_math("abs", &[tensor_1d(vec![-3.0, 0.0, 5.0])]));
        assert_tensor_eq(&t, &[3.0, 0.0, 5.0]);
    }

    #[test]
    fn test_sign() {
        let t = unwrap_tensor(call_math("sign", &[tensor_1d(vec![-3.0, 0.0, 5.0])]));
        assert_tensor_eq(&t, &[-1.0, 0.0, 1.0]);
    }

    #[test]
    fn test_floor() {
        let t = unwrap_tensor(call_math("floor", &[tensor_1d(vec![1.7, -1.7, 0.0])]));
        assert_tensor_eq(&t, &[1.0, -2.0, 0.0]);
    }

    #[test]
    fn test_ceil() {
        let t = unwrap_tensor(call_math("ceil", &[tensor_1d(vec![1.2, -1.2, 0.0])]));
        assert_tensor_eq(&t, &[2.0, -1.0, 0.0]);
    }

    #[test]
    fn test_round() {
        let t = unwrap_tensor(call_math("round", &[tensor_1d(vec![1.4, 1.5, 1.6, -0.5])]));
        let v = t.to_vec();
        assert!(approx_eq(v[0], 1.0));
        assert!(approx_eq(v[2], 2.0));
    }

    #[test]
    fn test_clip() {
        let t = unwrap_tensor(call_math("clip", &[tensor_1d(vec![-5.0, 0.0, 5.0, 10.0]), float(0.0), float(5.0)]));
        assert_tensor_eq(&t, &[0.0, 0.0, 5.0, 5.0]);
    }

    #[test]
    fn test_mod() {
        let t = unwrap_tensor(call_math("mod", &[tensor_1d(vec![7.0, 8.0, 9.0]), float(3.0)]));
        assert_tensor_eq(&t, &[1.0, 2.0, 0.0]);
    }

    #[test]
    fn test_tan() {
        let t = unwrap_tensor(call_math("tan", &[tensor_1d(vec![0.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
    }

    #[test]
    fn test_asin() {
        let t = unwrap_tensor(call_math("asin", &[tensor_1d(vec![0.0, 1.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
        assert!(approx_eq(t.to_vec()[1], std::f64::consts::FRAC_PI_2));
    }

    #[test]
    fn test_acos() {
        let t = unwrap_tensor(call_math("acos", &[tensor_1d(vec![1.0, 0.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
        assert!(approx_eq(t.to_vec()[1], std::f64::consts::FRAC_PI_2));
    }

    #[test]
    fn test_atan() {
        let t = unwrap_tensor(call_math("atan", &[tensor_1d(vec![0.0, 1.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
        assert!(approx_eq(t.to_vec()[1], std::f64::consts::FRAC_PI_4));
    }

    #[test]
    fn test_atan2() {
        let t = unwrap_tensor(call_math("atan2", &[tensor_1d(vec![1.0, 0.0]), tensor_1d(vec![0.0, 1.0])]));
        assert!(approx_eq(t.to_vec()[0], std::f64::consts::FRAC_PI_2));
        assert!(approx_eq(t.to_vec()[1], 0.0));
    }

    #[test]
    fn test_sinh() {
        let t = unwrap_tensor(call_math("sinh", &[tensor_1d(vec![0.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
    }

    #[test]
    fn test_cosh() {
        let t = unwrap_tensor(call_math("cosh", &[tensor_1d(vec![0.0])]));
        assert!(approx_eq(t.to_vec()[0], 1.0));
    }

    #[test]
    fn test_tanh() {
        let t = unwrap_tensor(call_math("tanh", &[tensor_1d(vec![0.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
    }

    // ===== Reductions =====

    #[test]
    fn test_sum() {
        let v = unwrap_float(call_math("sum", &[tensor_1d(vec![1.0, 2.0, 3.0])]));
        assert!(approx_eq(v, 6.0));
    }

    #[test]
    fn test_sum_axis() {
        let t = unwrap_tensor(call_math("sum", &[tensor(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]), int(0)]));
        assert_tensor_eq(&t, &[4.0, 6.0]);
    }

    #[test]
    fn test_mean() {
        let v = unwrap_float(call_math("mean", &[tensor_1d(vec![2.0, 4.0, 6.0])]));
        assert!(approx_eq(v, 4.0));
    }

    #[test]
    fn test_min() {
        let v = unwrap_float(call_math("min", &[tensor_1d(vec![3.0, 1.0, 2.0])]));
        assert!(approx_eq(v, 1.0));
    }

    #[test]
    fn test_max() {
        let v = unwrap_float(call_math("max", &[tensor_1d(vec![3.0, 1.0, 2.0])]));
        assert!(approx_eq(v, 3.0));
    }

    #[test]
    fn test_prod() {
        let v = unwrap_float(call_math("prod", &[tensor_1d(vec![2.0, 3.0, 4.0])]));
        assert!(approx_eq(v, 24.0));
    }

    #[test]
    fn test_argmax() {
        let v = unwrap_int(call_math("argmax", &[tensor_1d(vec![1.0, 5.0, 3.0])]));
        assert_eq!(v, 1);
    }

    #[test]
    fn test_argmin() {
        let v = unwrap_int(call_math("argmin", &[tensor_1d(vec![3.0, 1.0, 5.0])]));
        assert_eq!(v, 1);
    }

    #[test]
    fn test_cumsum() {
        let t = unwrap_tensor(call_math("cumsum", &[tensor_1d(vec![1.0, 2.0, 3.0])]));
        assert_tensor_eq(&t, &[1.0, 3.0, 6.0]);
    }

    #[test]
    fn test_cumprod() {
        let t = unwrap_tensor(call_math("cumprod", &[tensor_1d(vec![1.0, 2.0, 3.0])]));
        assert_tensor_eq(&t, &[1.0, 2.0, 6.0]);
    }

    // ===== Combining & Sorting =====

    #[test]
    fn test_argsort() {
        let t = unwrap_tensor(call_math("argsort", &[tensor_1d(vec![3.0, 1.0, 2.0])]));
        assert_tensor_eq(&t, &[1.0, 2.0, 0.0]);
    }

    #[test]
    fn test_topk() {
        let t = unwrap_tensor(call_math("topk", &[tensor_1d(vec![1.0, 5.0, 3.0, 4.0, 2.0]), int(3)]));
        assert_eq!(t.to_vec().len(), 3);
        assert!(approx_eq(t.to_vec()[0], 5.0));
    }

    #[test]
    fn test_unique() {
        let t = unwrap_tensor(call_math("unique", &[tensor_1d(vec![1.0, 2.0, 1.0, 3.0, 2.0])]));
        let v = t.to_vec();
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn test_where() {
        let t = unwrap_tensor(call_math("where", &[
            tensor_1d(vec![1.0, 0.0, 1.0]),
            tensor_1d(vec![10.0, 20.0, 30.0]),
            tensor_1d(vec![-1.0, -2.0, -3.0]),
        ]));
        assert_tensor_eq(&t, &[10.0, -2.0, 30.0]);
    }

    // ===== Shape Ops =====

    #[test]
    fn test_reshape() {
        let t = unwrap_tensor(call_math("reshape", &[tensor_1d(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), int(2), int(3)]));
        assert_eq!(t.shape(), &[2, 3]);
        assert_tensor_eq(&t, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_flatten() {
        let input = tensor(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let t = unwrap_tensor(call_math("flatten", &[input]));
        assert_eq!(t.shape(), &[4]);
        assert_tensor_eq(&t, &[1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_transpose() {
        let input = tensor(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let t = unwrap_tensor(call_math("transpose", &[input]));
        assert_eq!(t.shape(), &[3, 2]);
        assert_tensor_eq(&t, &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    }

    // ===== Linear Algebra =====

    #[test]
    fn test_inv() {
        // Identity matrix inverse = identity
        let eye = tensor(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let t = unwrap_tensor(call_math("inv", &[eye]));
        assert_tensor_eq(&t, &[1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_det() {
        let m = tensor(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let v = unwrap_float(call_math("det", &[m]));
        assert!(approx_eq(v, -2.0));
    }

    #[test]
    fn test_solve() {
        // Solve Ax = b where A = [[2,1],[1,3]], b = [[5],[10]]
        let a = tensor(vec![2.0, 1.0, 1.0, 3.0], vec![2, 2]);
        let b = tensor(vec![5.0, 10.0], vec![2, 1]);
        let x = unwrap_tensor(call_math("solve", &[a, b]));
        assert!(approx_eq(x.to_vec()[0], 1.0));
        assert!(approx_eq(x.to_vec()[1], 3.0));
    }

    #[test]
    fn test_norm_l2() {
        let v = unwrap_float(call_math("norm", &[tensor_1d(vec![3.0, 4.0])]));
        assert!(approx_eq(v, 5.0));
    }

    #[test]
    fn test_norm_l1() {
        let v = unwrap_float(call_math("norm", &[tensor_1d(vec![3.0, -4.0]), float(1.0)]));
        assert!(approx_eq(v, 7.0));
    }

    #[test]
    fn test_svd() {
        let m = tensor(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let result = unwrap_array(call_math("svd", &[m]));
        assert_eq!(result.len(), 3); // U, S, Vt
    }

    #[test]
    fn test_qr() {
        let m = tensor(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let result = unwrap_array(call_math("qr", &[m]));
        assert_eq!(result.len(), 2); // Q, R
    }

    #[test]
    fn test_cholesky() {
        // Positive definite matrix: [[4,2],[2,3]]
        let m = tensor(vec![4.0, 2.0, 2.0, 3.0], vec![2, 2]);
        let t = unwrap_tensor(call_math("cholesky", &[m]));
        assert_eq!(t.shape(), &[2, 2]);
    }

    #[test]
    fn test_eigen() {
        let m = tensor(vec![2.0, 0.0, 0.0, 3.0], vec![2, 2]);
        let result = unwrap_array(call_math("eigen", &[m]));
        assert_eq!(result.len(), 2); // real, imag eigenvalues
    }

    // ===== Random & Distributions =====

    #[test]
    fn test_seed() {
        let r = call_math("seed", &[int(42)]);
        let cv = r.expect("seed failed");
        assert!(cv.is_ok());
    }

    #[test]
    fn test_normal() {
        let t = unwrap_tensor(call_math("normal", &[float(0.0), float(1.0), int(100)]));
        assert_eq!(t.to_vec().len(), 100);
    }

    #[test]
    fn test_uniform() {
        let t = unwrap_tensor(call_math("uniform", &[float(0.0), float(1.0), int(50)]));
        let v = t.to_vec();
        assert_eq!(v.len(), 50);
        for &x in &v { assert!(x >= 0.0 && x <= 1.0); }
    }

    #[test]
    fn test_poisson() {
        let t = unwrap_tensor(call_math("poisson", &[float(5.0), int(100)]));
        assert_eq!(t.to_vec().len(), 100);
        for &x in &t.to_vec() { assert!(x >= 0.0); }
    }

    #[test]
    fn test_bernoulli() {
        let t = unwrap_tensor(call_math("bernoulli", &[float(0.5), int(100)]));
        assert_eq!(t.to_vec().len(), 100);
        for &x in &t.to_vec() { assert!(x == 0.0 || x == 1.0); }
    }

    #[test]
    fn test_shuffle() {
        let t = unwrap_tensor(call_math("shuffle", &[tensor_1d(vec![1.0, 2.0, 3.0, 4.0, 5.0])]));
        assert_eq!(t.to_vec().len(), 5);
        let mut sorted = t.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(sorted, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn test_choice() {
        let t = unwrap_tensor(call_math("choice", &[tensor_1d(vec![10.0, 20.0, 30.0, 40.0]), int(2)]));
        assert_eq!(t.to_vec().len(), 2);
    }

    #[test]
    fn test_permutation() {
        let t = unwrap_tensor(call_math("permutation", &[int(5)]));
        assert_eq!(t.to_vec().len(), 5);
        let mut sorted = t.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(sorted, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    // ===== Signal Processing =====

    #[test]
    fn test_fft() {
        let result = unwrap_array(call_math("fft", &[tensor_1d(vec![1.0, 0.0, 1.0, 0.0])]));
        assert_eq!(result.len(), 2); // real, imag
    }

    #[test]
    fn test_ifft() {
        // FFT then IFFT should recover original
        let original = tensor_1d(vec![1.0, 2.0, 3.0, 4.0]);
        let fft_result = unwrap_array(call_math("fft", &[original.clone()]));
        let recovered = unwrap_tensor(call_math("ifft", &[
            fft_result[0].clone(),
            fft_result[1].clone(),
        ]));
        let orig_v = vec![1.0, 2.0, 3.0, 4.0];
        let rec_v = recovered.to_vec();
        for i in 0..4 { assert!(approx_eq(rec_v[i], orig_v[i])); }
    }

    #[test]
    fn test_fft2() {
        let m = tensor(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let result = unwrap_array(call_math("fft2", &[m]));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_conv() {
        let signal = tensor_1d(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let kernel = tensor_1d(vec![1.0, 0.0, -1.0]);
        let t = unwrap_tensor(call_math("conv", &[signal, kernel]));
        assert!(t.to_vec().len() > 0);
    }

    #[test]
    fn test_conv2d() {
        let img = tensor(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let kern = tensor(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let t = unwrap_tensor(call_math("conv2d", &[img, kern]));
        assert!(t.to_vec().len() > 0);
    }

    #[test]
    fn test_hamming() {
        let t = unwrap_tensor(call_math("hamming", &[int(8)]));
        assert_eq!(t.to_vec().len(), 8);
    }

    #[test]
    fn test_hanning() {
        let t = unwrap_tensor(call_math("hanning", &[int(8)]));
        assert_eq!(t.to_vec().len(), 8);
    }

    // ===== Special Functions =====

    #[test]
    fn test_gamma() {
        // gamma(1) = 0! = 1, gamma(5) = 4! = 24
        let t = unwrap_tensor(call_math("gamma", &[tensor_1d(vec![1.0, 5.0])]));
        assert!(approx_eq(t.to_vec()[0], 1.0));
        assert!(approx_eq(t.to_vec()[1], 24.0));
    }

    #[test]
    fn test_erf() {
        // erf(0) = 0
        let t = unwrap_tensor(call_math("erf", &[tensor_1d(vec![0.0])]));
        assert!(approx_eq(t.to_vec()[0], 0.0));
    }

    #[test]
    fn test_bessel() {
        // J_0(0) = 1
        let t = unwrap_tensor(call_math("bessel", &[tensor_1d(vec![0.0]), int(0)]));
        assert!(approx_eq(t.to_vec()[0], 1.0));
    }

    #[test]
    fn test_factorial() {
        let t = unwrap_tensor(call_math("factorial", &[tensor_1d(vec![0.0, 1.0, 5.0])]));
        assert!(approx_eq(t.to_vec()[0], 1.0));
        assert!(approx_eq(t.to_vec()[1], 1.0));
        assert!(approx_eq(t.to_vec()[2], 120.0));
    }

    #[test]
    fn test_choose() {
        // C(5,2) = 10, C(4,0) = 1
        let t = unwrap_tensor(call_math("choose", &[tensor_1d(vec![5.0, 4.0]), tensor_1d(vec![2.0, 0.0])]));
        assert!(approx_eq(t.to_vec()[0], 10.0));
        assert!(approx_eq(t.to_vec()[1], 1.0));
    }

    // ===== Complex Numbers =====

    #[test]
    fn test_complex() {
        let result = unwrap_array(call_math("complex", &[float(3.0), float(4.0)]));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_real() {
        let c = call_math("complex", &[float(3.0), float(4.0)]).unwrap();
        let re = unwrap_tensor(call_math("real", &[c]));
        assert!(approx_eq(re.to_vec()[0], 3.0));
    }

    #[test]
    fn test_imag() {
        let c = call_math("complex", &[float(3.0), float(4.0)]).unwrap();
        let im = unwrap_tensor(call_math("imag", &[c]));
        assert!(approx_eq(im.to_vec()[0], 4.0));
    }

    #[test]
    fn test_magnitude() {
        let c = call_math("complex", &[float(3.0), float(4.0)]).unwrap();
        let m = unwrap_tensor(call_math("magnitude", &[c]));
        assert!(approx_eq(m.to_vec()[0], 5.0));
    }

    #[test]
    fn test_phase() {
        let c = call_math("complex", &[float(1.0), float(1.0)]).unwrap();
        let p = unwrap_tensor(call_math("phase", &[c]));
        assert!(approx_eq(p.to_vec()[0], std::f64::consts::FRAC_PI_4));
    }

    #[test]
    fn test_conjugate() {
        let c = call_math("complex", &[float(3.0), float(4.0)]).unwrap();
        let result = unwrap_array(call_math("conjugate", &[c]));
        assert_eq!(result.len(), 2);
    }

    // ===== Sparse =====

    #[test]
    fn test_sparse_from_dense() {
        let t = tensor(vec![1.0, 0.0, 0.0, 2.0], vec![2, 2]);
        let r = call_math("sparse", &[t]).expect("sparse failed");
        assert!(r.is_ok());
        match &r.value {
            Value::Sparse(_) => {},
            other => panic!("expected Sparse, got {:?}", other),
        }
    }

    #[test]
    fn test_dense() {
        let t = tensor(vec![1.0, 0.0, 0.0, 2.0], vec![2, 2]);
        let sp = call_math("sparse", &[t]).unwrap();
        let dense = unwrap_tensor(call_math("dense", &[sp]));
        assert_tensor_eq(&dense, &[1.0, 0.0, 0.0, 2.0]);
    }

    #[test]
    fn test_nnz() {
        let t = tensor(vec![1.0, 0.0, 0.0, 2.0], vec![2, 2]);
        let sp = call_math("sparse", &[t]).unwrap();
        let n = unwrap_int(call_math("nnz", &[sp]));
        assert_eq!(n, 2);
    }

    #[test]
    fn test_spmv() {
        let t = tensor(vec![2.0, 0.0, 0.0, 3.0], vec![2, 2]);
        let sp = call_math("sparse", &[t]).unwrap();
        let v = tensor_1d(vec![1.0, 1.0]);
        let result = unwrap_tensor(call_math("spmv", &[sp, v]));
        assert_tensor_eq(&result, &[2.0, 3.0]);
    }

    // ===== New: log2, log10 =====

    #[test]
    fn test_log2_dispatch() {
        let t = unwrap_tensor(call_math("log2", &[tensor_1d(vec![1.0, 2.0, 4.0, 8.0])]));
        assert_tensor_eq(&t, &[0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_log10_dispatch() {
        let t = unwrap_tensor(call_math("log10", &[tensor_1d(vec![1.0, 10.0, 100.0])]));
        assert_tensor_eq(&t, &[0.0, 1.0, 2.0]);
    }

    // ===== New: dot, pinv, lstsq, triu, tril =====

    #[test]
    fn test_dot_dispatch() {
        let v = unwrap_float(call_math("dot", &[
            tensor_1d(vec![1.0, 2.0, 3.0]),
            tensor_1d(vec![4.0, 5.0, 6.0]),
        ]));
        assert!(approx_eq(v, 32.0));
    }

    #[test]
    fn test_pinv_dispatch() {
        let m = tensor(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let t = unwrap_tensor(call_math("pinv", &[m]));
        assert_tensor_eq(&t, &[1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_lstsq_dispatch() {
        let a = tensor(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let b = tensor(vec![3.0, 5.0], vec![2, 1]);
        let x = unwrap_tensor(call_math("lstsq", &[a, b]));
        let v = x.to_vec();
        assert!(approx_eq(v[0], 3.0));
        assert!(approx_eq(v[1], 5.0));
    }

    #[test]
    fn test_triu_dispatch() {
        let m = tensor(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let t = unwrap_tensor(call_math("triu", &[m]));
        assert_tensor_eq(&t, &[1.0, 2.0, 3.0, 0.0, 5.0, 6.0, 0.0, 0.0, 9.0]);
    }

    #[test]
    fn test_tril_dispatch() {
        let m = tensor(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let t = unwrap_tensor(call_math("tril", &[m]));
        assert_tensor_eq(&t, &[1.0, 0.0, 0.0, 4.0, 5.0, 0.0, 7.0, 8.0, 9.0]);
    }

    // ===== New: gamma_dist, beta_dist, exponential =====

    #[test]
    fn test_gamma_dist_dispatch() {
        crow_runtime::rng::set_seed(42);
        let t = unwrap_tensor(call_math("gamma_dist", &[float(2.0), float(1.0), int(10)]));
        assert_eq!(t.shape(), &[10]);
        assert!(t.to_vec().iter().all(|&x| x > 0.0));
        crow_runtime::rng::clear_seed();
    }

    #[test]
    fn test_beta_dist_dispatch() {
        crow_runtime::rng::set_seed(42);
        let t = unwrap_tensor(call_math("beta_dist", &[float(2.0), float(5.0), int(10)]));
        assert_eq!(t.shape(), &[10]);
        assert!(t.to_vec().iter().all(|&x| x > 0.0 && x < 1.0));
        crow_runtime::rng::clear_seed();
    }

    #[test]
    fn test_exponential_dispatch() {
        crow_runtime::rng::set_seed(42);
        let t = unwrap_tensor(call_math("exponential", &[float(1.5), int(10)]));
        assert_eq!(t.shape(), &[10]);
        assert!(t.to_vec().iter().all(|&x| x > 0.0));
        crow_runtime::rng::clear_seed();
    }
}