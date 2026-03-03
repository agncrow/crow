use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;
use crow_runtime::tensor::CrowTensor;
use crow_runtime::sparse::CrowSparse;


pub struct MathModule;

const MATH_FUNCTIONS: &[&str] = &[
    // Tensor creation
    "zeros", "ones", "eye", "randn", "linspace", "arange", "full", "diag", "tensor",
    // Element-wise math (ln replaces log; abs/sqrt/floor/ceil/round overlap with core builtins)
    "exp", "ln", "sin", "cos", "sign", "clip", "mod",
    "tan", "asin", "acos", "atan", "atan2",
    "sinh", "cosh", "tanh",
    "abs", "sqrt", "floor", "ceil", "round",
    // Reductions (min/max overlap with core builtins)
    "sum", "mean", "min", "max", "prod",
    "argmax", "argmin",
    // Cumulative
    "cumsum", "cumprod",
    // Combining
    "concat", "stack",
    // Sorting & selection
    "argsort", "topk", "unique", "where",
    // Shape ops
    "reshape", "flatten", "transpose",
    // Linear algebra
    "solve", "inv", "det", "norm", "svd", "qr", "cholesky", "eigen",
    // Random & distributions
    "seed", "normal", "uniform", "poisson", "bernoulli",
    "shuffle", "choice", "permutation",
    // Signal processing
    "fft", "fft2", "ifft", "conv", "conv2d", "hamming", "hanning",
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

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        // Auto-propagation: if any argument is an error-value, return the first error.
        for a in args {
            if !a.ok {
                return Ok(a.clone());
            }
        }
        call_math(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        emit_math_rust(name, arg_strs, arg_types)
    }

    fn type_of(&self, name: &str, arg_count: usize) -> CrowType {
        math_type_of(name, arg_count)
    }
}

// ===== Helpers =====

fn to_f64(v: &CrowValue, fname: &str) -> Result<f64, String> {
    match &v.value {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(f) => Ok(*f),
        _ => Err(format!("{}: expected number, got {:?}", fname, v.value)),
    }
}

fn to_usize(v: &CrowValue, fname: &str) -> Result<usize, String> {
    match &v.value {
        Value::Int(n) if *n >= 0 => Ok(*n as usize),
        Value::Int(n) => Err(format!("{}: expected non-negative integer, got {}", fname, n)),
        Value::Float(f) => Ok(*f as usize),
        _ => Err(format!("{}: expected integer, got {:?}", fname, v.value)),
    }
}

fn to_tensor(v: &CrowValue, fname: &str) -> Result<CrowTensor, String> {
    match &v.value {
        Value::NumArray(t) => Ok(t.clone()),
        _ => Err(format!("{}: expected numarray, got {:?}", fname, v.value)),
    }
}

fn shape_from_args(args: &[CrowValue], fname: &str) -> Result<Vec<usize>, String> {
    args.iter().map(|a| match &a.value {
        Value::Int(n) if *n >= 0 => Ok(*n as usize),
        Value::Int(n) => Err(format!("{}: expected non-negative integer for shape, got {}", fname, n)),
        Value::Float(f) => Ok(*f as usize),
        _ => Err(format!("{}: expected integer for shape, got {:?}", fname, a.value)),
    }).collect()
}

// ===== Interpreter dispatch =====

fn call_math(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        // Tensor creation
        "zeros" => {
            let shape = shape_from_args(args, name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::zeros(shape))))
        }
        "ones" => {
            let shape = shape_from_args(args, name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::ones(shape))))
        }
        "eye" => {
            if args.len() != 1 { return Err(format!("eye: expected 1 argument, got {}", args.len())); }
            let n = to_usize(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::eye(n))))
        }
        "randn" => {
            let shape = shape_from_args(args, name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::randn(shape))))
        }
        "linspace" => {
            if args.len() != 3 { return Err(format!("linspace: expected 3 arguments (start, stop, n), got {}", args.len())); }
            let start = to_f64(&args[0], name)?;
            let stop = to_f64(&args[1], name)?;
            let n = to_usize(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::linspace(start, stop, n))))
        }
        "arange" => {
            if args.len() != 3 { return Err(format!("arange: expected 3 arguments (start, stop, step), got {}", args.len())); }
            let start = to_f64(&args[0], name)?;
            let stop = to_f64(&args[1], name)?;
            let step = to_f64(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::arange(start, stop, step))))
        }
        "full" => {
            if args.len() < 2 { return Err(format!("full: expected at least 2 arguments (dims..., value), got {}", args.len())); }
            let value = to_f64(args.last().unwrap(), name)?;
            let shape = shape_from_args(&args[..args.len()-1], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::full(shape, value))))
        }
        "diag" => {
            if args.len() != 1 { return Err(format!("diag: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::diag(&t))))
        }
        "tensor" => {
            if args.len() != 1 { return Err(format!("tensor: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::NumArray(t) => Ok(CrowValue::ok(Value::NumArray(t.clone()))),
                Value::Array(arr) => {
                    let data: Result<Vec<f64>, String> = arr.iter().map(|v| match &v.value {
                        Value::Int(n) => Ok(*n as f64),
                        Value::Float(f) => Ok(*f),
                        _ => Err("tensor: array elements must be numbers".into()),
                    }).collect();
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data?, vec![arr.len()]))))
                }
                _ => Err(format!("tensor: expected array, got {:?}", args[0].value)),
            }
        }

        // Element-wise math
        "exp" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.exp()))) }
        "ln" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.log()))) }
        "sqrt" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.sqrt()))) }
        "sin" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.sin()))) }
        "cos" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.cos()))) }
        "abs" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.abs()))) }
        "sign" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.sign()))) }
        "floor" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.floor()))) }
        "ceil" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.ceil()))) }
        "round" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.round()))) }
        "clip" => {
            if args.len() != 3 { return Err(format!("clip: expected 3 arguments (tensor, lo, hi), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let lo = to_f64(&args[1], name)?;
            let hi = to_f64(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.clip(lo, hi))))
        }
        "mod" => {
            if args.len() != 2 { return Err(format!("mod: expected 2 arguments, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            match &args[1].value {
                Value::NumArray(b) => Ok(CrowValue::ok(Value::NumArray(a.modulo(b)))),
                Value::Int(n) => Ok(CrowValue::ok(Value::NumArray(a.modulo_scalar(*n as f64)))),
                Value::Float(f) => Ok(CrowValue::ok(Value::NumArray(a.modulo_scalar(*f)))),
                _ => Err("mod: second argument must be a number or tensor".into()),
            }
        }

        // Trigonometry
        "tan" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.tan()))) }
        "asin" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.asin()))) }
        "acos" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.acos()))) }
        "atan" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.atan()))) }
        "atan2" => {
            if args.len() != 2 { return Err(format!("atan2: expected 2 arguments, got {}", args.len())); }
            let y = to_tensor(&args[0], name)?;
            let x = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(y.atan2(&x))))
        }
        "sinh" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.sinh()))) }
        "cosh" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.cosh()))) }
        "tanh" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.tanh()))) }

        _ => call_math_reductions(name, args),
    }
}

fn call_math_reductions(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        // Reductions
        "sum" => {
            let t = to_tensor(&args[0], name)?;
            if args.len() >= 2 { let axis = to_f64(&args[1], name)? as usize; Ok(CrowValue::ok(Value::NumArray(t.sum_axis(axis)))) }
            else { Ok(CrowValue::ok(Value::Float(t.sum()))) }
        }
        "mean" => {
            let t = to_tensor(&args[0], name)?;
            if args.len() >= 2 { let axis = to_f64(&args[1], name)? as usize; Ok(CrowValue::ok(Value::NumArray(t.mean_axis(axis)))) }
            else { Ok(CrowValue::ok(Value::Float(t.mean()))) }
        }
        "min" => {
            let t = to_tensor(&args[0], name)?;
            if args.len() >= 2 { let axis = to_f64(&args[1], name)? as usize; Ok(CrowValue::ok(Value::NumArray(t.min_axis(axis)))) }
            else { Ok(CrowValue::ok(Value::Float(t.min()))) }
        }
        "max" => {
            let t = to_tensor(&args[0], name)?;
            if args.len() >= 2 { let axis = to_f64(&args[1], name)? as usize; Ok(CrowValue::ok(Value::NumArray(t.max_axis(axis)))) }
            else { Ok(CrowValue::ok(Value::Float(t.max()))) }
        }
        "prod" => {
            let t = to_tensor(&args[0], name)?;
            if args.len() >= 2 { let axis = to_f64(&args[1], name)? as usize; Ok(CrowValue::ok(Value::NumArray(t.prod_axis(axis)))) }
            else { Ok(CrowValue::ok(Value::Float(t.prod()))) }
        }
        "argmax" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::Int(t.argmax() as i64))) }
        "argmin" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::Int(t.argmin() as i64))) }
        "cumsum" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.cumsum()))) }
        "cumprod" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.cumprod()))) }

        _ => call_math_ops(name, args),
    }
}

fn call_math_ops(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        // Combining ops
        "concat" => {
            if args.len() < 2 { return Err("concat: expected at least 2 arguments (a, b, [axis])".into()); }
            let a = to_tensor(&args[0], name)?;
            let b = to_tensor(&args[1], name)?;
            let axis = if args.len() >= 3 { to_f64(&args[2], name)? as usize } else { 0 };
            Ok(CrowValue::ok(Value::NumArray(a.concat(&b, axis))))
        }
        "stack" => {
            if args.is_empty() { return Err("stack: expected at least 1 argument (array of tensors, [axis])".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    let tensors: Result<Vec<CrowTensor>, String> = items.iter().map(|item| {
                        match &item.value {
                            Value::NumArray(t) => Ok(t.clone()),
                            _ => Err("stack: all elements must be tensors".into()),
                        }
                    }).collect();
                    let tensors = tensors?;
                    let axis = if args.len() >= 2 { to_f64(&args[1], name)? as usize } else { 0 };
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::stack(&tensors, axis))))
                }
                _ => Err("stack: first argument must be an array of tensors".into()),
            }
        }

        // Sorting & selection
        "argsort" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.argsort()))) }
        "topk" => {
            if args.len() < 2 { return Err("topk: expected 2 arguments (tensor, k)".into()); }
            let t = to_tensor(&args[0], name)?;
            let k = to_f64(&args[1], name)? as usize;
            Ok(CrowValue::ok(Value::NumArray(t.topk(k))))
        }
        "unique" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.unique()))) }
        "where" => {
            if args.len() < 3 { return Err("where: expected 3 arguments (cond, a, b)".into()); }
            let cond = to_tensor(&args[0], name)?;
            let a = to_tensor(&args[1], name)?;
            let b = to_tensor(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(cond.where_cond(&a, &b))))
        }

        // Shape ops
        "reshape" => {
            if args.len() < 2 { return Err(format!("reshape: expected at least 2 arguments (tensor, dims...), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let new_shape = shape_from_args(&args[1..], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.reshape(new_shape))))
        }
        "flatten" => {
            if args.len() != 1 { return Err(format!("flatten: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.flatten())))
        }
        "transpose" => {
            if args.len() != 1 { return Err(format!("transpose: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.transpose())))
        }

        _ => call_math_linalg(name, args),
    }
}

fn call_math_linalg(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "solve" => {
            if args.len() != 2 { return Err(format!("solve: expected 2 arguments, got {}", args.len())); }
            if let Value::Sparse(s) = &args[0].value {
                let b = to_tensor(&args[1], name)?;
                Ok(CrowValue::ok(Value::NumArray(s.solve(&b))))
            } else {
                let a = to_tensor(&args[0], name)?;
                let b = to_tensor(&args[1], name)?;
                Ok(CrowValue::ok(Value::NumArray(a.solve(&b))))
            }
        }
        "inv" => {
            if args.len() != 1 { return Err(format!("inv: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.inv())))
        }
        "det" => {
            if args.len() != 1 { return Err(format!("det: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::Float(a.det())))
        }
        "svd" => {
            if args.len() != 1 { return Err(format!("svd: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let (u, s, vt) = a.svd_decomp();
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(u)),
                CrowValue::ok(Value::NumArray(s)),
                CrowValue::ok(Value::NumArray(vt)),
            ])))
        }
        "qr" => {
            if args.len() != 1 { return Err(format!("qr: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let (q, r) = a.qr_decomp();
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(q)),
                CrowValue::ok(Value::NumArray(r)),
            ])))
        }
        "cholesky" => {
            if args.len() != 1 { return Err(format!("cholesky: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.cholesky_decomp())))
        }
        "eigen" => {
            if args.len() != 1 { return Err(format!("eigen: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let (real, imag) = a.eigen_decomp();
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(real)),
                CrowValue::ok(Value::NumArray(imag)),
            ])))
        }
        "norm" => {
            if args.is_empty() || args.len() > 2 { return Err(format!("norm: expected 1-2 arguments, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let p = if args.len() == 2 {
                match &args[1].value {
                    Value::Int(i) => *i as f64,
                    Value::Float(f) => *f,
                    _ => return Err("norm: p must be a number".into()),
                }
            } else { 2.0 };
            Ok(CrowValue::ok(Value::Float(t.norm(p))))
        }

        _ => call_math_random(name, args),
    }
}


fn call_math_random(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "seed" => {
            if args.len() != 1 { return Err(format!("seed: expected 1 argument, got {}", args.len())); }
            let n = match &args[0].value {
                Value::Int(i) => *i as u64,
                Value::Float(f) => *f as u64,
                _ => return Err("seed: expected integer".into()),
            };
            crow_runtime::rng::set_seed(n);
            Ok(CrowValue::done())
        }
        "normal" => {
            if args.len() < 2 { return Err(format!("normal: expected at least 2 arguments (mean, std, dims...), got {}", args.len())); }
            let mean = to_f64(&args[0], name)?;
            let std_val = to_f64(&args[1], name)?;
            let shape = shape_from_args(&args[2..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::normal(shape, mean, std_val))))
        }
        "uniform" => {
            if args.len() < 2 { return Err(format!("uniform: expected at least 2 arguments (low, high, dims...), got {}", args.len())); }
            let low = to_f64(&args[0], name)?;
            let high = to_f64(&args[1], name)?;
            let shape = shape_from_args(&args[2..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::rand_range(shape, low, high))))
        }
        "poisson" => {
            if args.is_empty() { return Err(format!("poisson: expected at least 1 argument (lambda, dims...), got 0")); }
            let lambda = to_f64(&args[0], name)?;
            let shape = shape_from_args(&args[1..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::poisson(shape, lambda))))
        }
        "bernoulli" => {
            if args.is_empty() { return Err(format!("bernoulli: expected at least 1 argument (p, dims...), got 0")); }
            let p = to_f64(&args[0], name)?;
            let shape = shape_from_args(&args[1..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::bernoulli(shape, p))))
        }
        "shuffle" => {
            if args.len() != 1 { return Err(format!("shuffle: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.shuffle())))
        }
        "choice" => {
            if args.len() != 2 { return Err(format!("choice: expected 2 arguments (tensor, n), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let n = to_usize(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.choice(n))))
        }
        "permutation" => {
            if args.len() != 1 { return Err(format!("permutation: expected 1 argument, got {}", args.len())); }
            let n = to_usize(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::permutation(n))))
        }
        _ => call_math_signal(name, args),
    }
}

fn call_math_signal(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "fft" => {
            if args.len() != 1 { return Err(format!("fft: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let (real, imag) = t.fft();
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(real)),
                CrowValue::ok(Value::NumArray(imag)),
            ])))
        }
        "ifft" => {
            if args.len() != 2 { return Err(format!("ifft: expected 2 arguments (real, imag), got {}", args.len())); }
            let real = to_tensor(&args[0], name)?;
            let imag = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::ifft(&real, &imag))))
        }
        "fft2" => {
            if args.len() != 1 { return Err(format!("fft2: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let (real, imag) = t.fft2();
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(real)),
                CrowValue::ok(Value::NumArray(imag)),
            ])))
        }
        "conv" => {
            if args.len() != 2 { return Err(format!("conv: expected 2 arguments, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let b = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.conv(&b))))
        }
        "conv2d" => {
            if args.len() != 2 { return Err(format!("conv2d: expected 2 arguments, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let b = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.conv2d(&b))))
        }
        "hamming" => {
            if args.len() != 1 { return Err(format!("hamming: expected 1 argument, got {}", args.len())); }
            let n = to_usize(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::hamming(n))))
        }
        "hanning" => {
            if args.len() != 1 { return Err(format!("hanning: expected 1 argument, got {}", args.len())); }
            let n = to_usize(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::hanning(n))))
        }
        _ => call_math_special(name, args),
    }
}

fn call_math_special(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        // Special functions
        "gamma" => {
            if args.len() != 1 { return Err(format!("gamma: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.gamma())))
        }
        "erf" => {
            if args.len() != 1 { return Err(format!("erf: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.erf())))
        }
        "bessel" => {
            if args.len() != 2 { return Err(format!("bessel: expected 2 arguments (x, n), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let n = to_f64(&args[1], name)? as i64;
            Ok(CrowValue::ok(Value::NumArray(t.bessel(n))))
        }
        "factorial" => {
            if args.len() != 1 { return Err(format!("factorial: expected 1 argument, got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.factorial())))
        }
        "choose" => {
            if args.len() != 2 { return Err(format!("choose: expected 2 arguments (n, k), got {}", args.len())); }
            let n = to_tensor(&args[0], name)?;
            let k = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(n.choose(&k))))
        }

        // Complex numbers
        "complex" => {
            if args.len() != 2 { return Err(format!("complex: expected 2 arguments (re, im), got {}", args.len())); }
            let re = to_f64(&args[0], name)?;
            let im = to_f64(&args[1], name)?;
            let (r, i) = CrowTensor::complex(re, im);
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(r)),
                CrowValue::ok(Value::NumArray(i)),
            ])))
        }
        "real" => {
            if args.len() != 1 { return Err(format!("real: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Array(arr) if arr.len() == 2 => Ok(arr[0].clone()),
                _ => Err("real: expected complex value [real, imag]".to_string()),
            }
        }
        "imag" => {
            if args.len() != 1 { return Err(format!("imag: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Array(arr) if arr.len() == 2 => Ok(arr[1].clone()),
                _ => Err("imag: expected complex value [real, imag]".to_string()),
            }
        }
        "magnitude" => {
            if args.len() != 1 { return Err(format!("magnitude: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Array(arr) if arr.len() == 2 => {
                    let re = to_tensor(&arr[0], name)?;
                    let im = to_tensor(&arr[1], name)?;
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::magnitude(&re, &im))))
                }
                _ => Err("magnitude: expected complex value [real, imag]".to_string()),
            }
        }
        "phase" => {
            if args.len() != 1 { return Err(format!("phase: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Array(arr) if arr.len() == 2 => {
                    let re = to_tensor(&arr[0], name)?;
                    let im = to_tensor(&arr[1], name)?;
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::phase(&re, &im))))
                }
                _ => Err("phase: expected complex value [real, imag]".to_string()),
            }
        }
        "conjugate" => {
            if args.len() != 1 { return Err(format!("conjugate: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Array(arr) if arr.len() == 2 => {
                    let re = to_tensor(&arr[0], name)?;
                    let im = to_tensor(&arr[1], name)?;
                    let (cr, ci) = CrowTensor::conjugate(&re, &im);
                    Ok(CrowValue::ok(Value::Array(vec![
                        CrowValue::ok(Value::NumArray(cr)),
                        CrowValue::ok(Value::NumArray(ci)),
                    ])))
                }
                _ => Err("conjugate: expected complex value [real, imag]".to_string()),
            }
        }

        // Sparse arrays
        "sparse" => {
            if args.len() == 1 {
                let t = to_tensor(&args[0], name)?;
                Ok(CrowValue::ok(Value::Sparse(CrowSparse::from_dense(&t))))
            } else if args.len() == 4 {
                let rows_t = to_tensor(&args[0], name)?;
                let cols_t = to_tensor(&args[1], name)?;
                let vals_t = to_tensor(&args[2], name)?;
                let shape_t = to_tensor(&args[3], name)?;
                let rows: Vec<usize> = rows_t.to_vec().iter().map(|v| *v as usize).collect();
                let cols: Vec<usize> = cols_t.to_vec().iter().map(|v| *v as usize).collect();
                let vals = vals_t.to_vec();
                let shape_data = shape_t.to_vec();
                if shape_data.len() != 2 {
                    return Err("sparse: shape must have 2 elements [nrows, ncols]".into());
                }
                let nrows = shape_data[0] as usize;
                let ncols = shape_data[1] as usize;
                Ok(CrowValue::ok(Value::Sparse(CrowSparse::from_coo(&rows, &cols, &vals, nrows, ncols))))
            } else {
                Err(format!("sparse: expected 1 or 4 arguments, got {}", args.len()))
            }
        }
        "dense" => {
            if args.len() != 1 { return Err(format!("dense: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Sparse(s) => Ok(CrowValue::ok(Value::NumArray(s.to_dense()))),
                Value::NumArray(t) => Ok(CrowValue::ok(Value::NumArray(t.clone()))),
                _ => Err(format!("dense: expected sparse or numarray, got {}", args[0].value.type_name())),
            }
        }
        "nnz" => {
            if args.len() != 1 { return Err(format!("nnz: expected 1 argument, got {}", args.len())); }
            match &args[0].value {
                Value::Sparse(s) => Ok(CrowValue::ok(Value::Int(s.nnz() as i64))),
                _ => Err(format!("nnz: expected sparse, got {}", args[0].value.type_name())),
            }
        }
        "spmv" => {
            if args.len() != 2 { return Err(format!("spmv: expected 2 arguments, got {}", args.len())); }
            match (&args[0].value, &args[1].value) {
                (Value::Sparse(s), Value::NumArray(v)) => {
                    Ok(CrowValue::ok(Value::NumArray(s.matmul_dense(v))))
                }
                _ => Err("spmv: expected (sparse, numarray)".into()),
            }
        }

        // I/O: load numeric CSV directly to tensor
        "load" => {
            if args.len() != 1 { return Err(format!("load: expected 1 argument (path), got {}", args.len())); }
            let path = match &args[0].value {
                Value::Str(s) => s.as_str(),
                _ => return Err("load: expected string path".into()),
            };
            match crow_runtime::io::load_csv_numeric(path) {
                Ok((data, nrows, ncols)) => {
                    let t = CrowTensor::from_vec(data, vec![nrows, ncols]);
                    Ok(CrowValue::ok(Value::NumArray(t)))
                }
                Err(e) => Ok(CrowValue::error(&e, "load")),
            }
        }

        _ => Err(format!("{}: unknown math function", name)),
    }
}

// ===== Transpiler emission =====

fn emit_math_rust(name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
    let result = match name {
        // Tensor creation
        "zeros" => format!("CrowTensor::zeros(vec![{}])", arg_strs.iter().map(|a| format!("{} as usize", a)).collect::<Vec<_>>().join(", ")),
        "ones" => format!("CrowTensor::ones(vec![{}])", arg_strs.iter().map(|a| format!("{} as usize", a)).collect::<Vec<_>>().join(", ")),
        "randn" => format!("CrowTensor::randn(vec![{}])", arg_strs.iter().map(|a| format!("{} as usize", a)).collect::<Vec<_>>().join(", ")),
        "eye" => format!("CrowTensor::eye({} as usize)", arg_strs[0]),
        "linspace" => format!("CrowTensor::linspace({} as f64, {} as f64, {} as usize)", arg_strs[0], arg_strs[1], arg_strs[2]),
        "arange" => format!("CrowTensor::arange({} as f64, {} as f64, {} as f64)", arg_strs[0], arg_strs[1], arg_strs[2]),
        "full" => {
            let shape_args: Vec<String> = arg_strs[..arg_strs.len()-1].iter().map(|a| format!("{} as usize", a)).collect();
            format!("CrowTensor::full(vec![{}], {} as f64)", shape_args.join(", "), arg_strs.last().unwrap())
        }
        "diag" => format!("CrowTensor::diag(&{})", arg_strs[0]),
        "tensor" => format!("{{ let _arr = {}; CrowTensor::from_vec(_arr.iter().map(|v| *v as f64).collect(), vec![_arr.len()]) }}", arg_strs[0]),
        // Element-wise math (ln replaces log)
        "exp" => format!("{}.exp()", arg_strs[0]),
        "ln" => format!("{}.log()", arg_strs[0]),
        "sqrt" => format!("{}.sqrt()", arg_strs[0]),
        "sin" => format!("{}.sin()", arg_strs[0]),
        "cos" => format!("{}.cos()", arg_strs[0]),
        "abs" => format!("{}.abs()", arg_strs[0]),
        "sign" => format!("{}.sign()", arg_strs[0]),
        "floor" => format!("{}.floor()", arg_strs[0]),
        "ceil" => format!("{}.ceil()", arg_strs[0]),
        "round" => format!("{}.round()", arg_strs[0]),
        "clip" => format!("{}.clip({} as f64, {} as f64)", arg_strs[0], arg_strs[1], arg_strs[2]),
        "mod" => {
            if arg_types.len() > 1 && matches!(arg_types[1], CrowType::NumArray(_)) {
                format!("{}.modulo(&{})", arg_strs[0], arg_strs[1])
            } else {
                format!("{}.modulo_scalar({} as f64)", arg_strs[0], arg_strs[1])
            }
        }
        // Trig
        "tan" => format!("{}.tan()", arg_strs[0]),
        "asin" => format!("{}.asin()", arg_strs[0]),
        "acos" => format!("{}.acos()", arg_strs[0]),
        "atan" => format!("{}.atan()", arg_strs[0]),
        "atan2" => format!("{}.atan2(&{})", arg_strs[0], arg_strs[1]),
        "sinh" => format!("{}.sinh()", arg_strs[0]),
        "cosh" => format!("{}.cosh()", arg_strs[0]),
        "tanh" => format!("{}.tanh()", arg_strs[0]),
        // Reductions
        "sum" if arg_strs.len() >= 2 => format!("{}.sum_axis({} as usize)", arg_strs[0], arg_strs[1]),
        "sum" => format!("{}.sum()", arg_strs[0]),
        "mean" if arg_strs.len() >= 2 => format!("{}.mean_axis({} as usize)", arg_strs[0], arg_strs[1]),
        "mean" => format!("{}.mean()", arg_strs[0]),
        "min" if arg_strs.len() >= 2 => format!("{}.min_axis({} as usize)", arg_strs[0], arg_strs[1]),
        "min" => format!("{}.min()", arg_strs[0]),
        "max" if arg_strs.len() >= 2 => format!("{}.max_axis({} as usize)", arg_strs[0], arg_strs[1]),
        "max" => format!("{}.max()", arg_strs[0]),
        "prod" if arg_strs.len() >= 2 => format!("{}.prod_axis({} as usize)", arg_strs[0], arg_strs[1]),
        "prod" => format!("{}.prod()", arg_strs[0]),
        "argmax" => format!("{}.argmax() as i64", arg_strs[0]),
        "argmin" => format!("{}.argmin() as i64", arg_strs[0]),
        // Cumulative
        "cumsum" => format!("{}.cumsum()", arg_strs[0]),
        "cumprod" => format!("{}.cumprod()", arg_strs[0]),
        // Combining
        "concat" if arg_strs.len() >= 3 => format!("{}.concat(&{}, {} as usize)", arg_strs[0], arg_strs[1], arg_strs[2]),
        "concat" => format!("{}.concat(&{}, 0)", arg_strs[0], arg_strs[1]),
        "stack" if arg_strs.len() >= 2 => format!("CrowTensor::stack(&{}, {} as usize)", arg_strs[0], arg_strs[1]),
        "stack" => format!("CrowTensor::stack(&{}, 0)", arg_strs[0]),
        // Sorting & selection
        "argsort" => format!("{}.argsort()", arg_strs[0]),
        "topk" => format!("{}.topk({} as usize)", arg_strs[0], arg_strs[1]),
        "unique" => format!("{}.unique()", arg_strs[0]),
        "where" => format!("{}.where_cond(&{}, &{})", arg_strs[0], arg_strs[1], arg_strs[2]),
        // Shape ops
        "reshape" => {
            let shape_args: Vec<String> = arg_strs[1..].iter().map(|a| format!("{} as usize", a)).collect();
            format!("{}.reshape(vec![{}])", arg_strs[0], shape_args.join(", "))
        }
        "flatten" => format!("{}.flatten()", arg_strs[0]),
        "transpose" => format!("{}.transpose()", arg_strs[0]),
        // Linear algebra
        "solve" => format!("{}.solve(&{})", arg_strs[0], arg_strs[1]),
        "inv" => format!("{}.inv()", arg_strs[0]),
        "det" => format!("{}.det()", arg_strs[0]),
        "svd" => format!("{{ let (_u, _s, _vt) = {}.svd_decomp(); vec![_u, _s, _vt] }}", arg_strs[0]),
        "qr" => format!("{{ let (_q, _r) = {}.qr_decomp(); vec![_q, _r] }}", arg_strs[0]),
        "cholesky" => format!("{}.cholesky_decomp()", arg_strs[0]),
        "eigen" => format!("{{ let (_re, _im) = {}.eigen_decomp(); vec![_re, _im] }}", arg_strs[0]),
        "norm" if arg_strs.len() >= 2 => format!("{}.norm({} as f64)", arg_strs[0], arg_strs[1]),
        "norm" => format!("{}.norm(2.0)", arg_strs[0]),
        _ => return emit_math_rust_continued(name, arg_strs),
    };
    Ok(result)
}

fn emit_math_rust_continued(name: &str, arg_strs: &[String]) -> Result<String, String> {
    let result = match name {
        // Random & Distributions
        "seed" => format!("crow_runtime::rng::set_seed({} as u64)", arg_strs[0]),
        "normal" => {
            let mean = &arg_strs[0];
            let std = &arg_strs[1];
            let shape_args: Vec<String> = arg_strs[2..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::normal(vec![{}], {} as f64, {} as f64)", shape, mean, std)
        }
        "uniform" => {
            let low = &arg_strs[0];
            let high = &arg_strs[1];
            let shape_args: Vec<String> = arg_strs[2..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::rand_range(vec![{}], {} as f64, {} as f64)", shape, low, high)
        }
        "poisson" => {
            let lambda = &arg_strs[0];
            let shape_args: Vec<String> = arg_strs[1..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::poisson(vec![{}], {} as f64)", shape, lambda)
        }
        "bernoulli" => {
            let p = &arg_strs[0];
            let shape_args: Vec<String> = arg_strs[1..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::bernoulli(vec![{}], {} as f64)", shape, p)
        }
        "shuffle" => format!("{}.shuffle()", arg_strs[0]),
        "choice" => format!("{}.choice({} as usize)", arg_strs[0], arg_strs[1]),
        "permutation" => format!("CrowTensor::permutation({} as usize)", arg_strs[0]),
        // Signal processing
        "fft" => format!("{{ let (_re, _im) = {}.fft(); vec![_re, _im] }}", arg_strs[0]),
        "ifft" => format!("CrowTensor::ifft(&{}, &{})", arg_strs[0], arg_strs[1]),
        "fft2" => format!("{{ let (_re, _im) = {}.fft2(); vec![_re, _im] }}", arg_strs[0]),
        "conv" => format!("{}.conv(&{})", arg_strs[0], arg_strs[1]),
        "conv2d" => format!("{}.conv2d(&{})", arg_strs[0], arg_strs[1]),
        "hamming" => format!("CrowTensor::hamming({} as usize)", arg_strs[0]),
        "hanning" => format!("CrowTensor::hanning({} as usize)", arg_strs[0]),
        // Special functions
        "gamma" => format!("{}.gamma()", arg_strs[0]),
        "erf" => format!("{}.erf()", arg_strs[0]),
        "bessel" => format!("{}.bessel({} as i64)", arg_strs[0], arg_strs[1]),
        "factorial" => format!("{}.factorial()", arg_strs[0]),
        "choose" => format!("{}.choose(&{})", arg_strs[0], arg_strs[1]),
        // Complex
        "complex" => format!("{{ let (_re, _im) = CrowTensor::complex({} as f64, {} as f64); vec![_re, _im] }}", arg_strs[0], arg_strs[1]),
        "real" => format!("{}[0].clone()", arg_strs[0]),
        "imag" => format!("{}[1].clone()", arg_strs[0]),
        "magnitude" => format!("CrowTensor::magnitude(&{}[0], &{}[1])", arg_strs[0], arg_strs[0]),
        "phase" => format!("CrowTensor::phase(&{}[0], &{}[1])", arg_strs[0], arg_strs[0]),
        "conjugate" => format!("{{ let (_re, _im) = CrowTensor::conjugate(&{}[0], &{}[1]); vec![_re, _im] }}", arg_strs[0], arg_strs[0]),
        // Sparse
        "sparse" if arg_strs.len() == 1 => format!("CrowSparse::from_dense(&{})", arg_strs[0]),
        "sparse" if arg_strs.len() == 4 => format!("{{ let _rows: Vec<usize> = {}.to_vec().iter().map(|v| *v as usize).collect(); let _cols: Vec<usize> = {}.to_vec().iter().map(|v| *v as usize).collect(); let _vals = {}.to_vec(); let _shape = {}.to_vec(); CrowSparse::from_coo(&_rows, &_cols, &_vals, _shape[0] as usize, _shape[1] as usize) }}", arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[3]),
        "sparse" => "/* sparse: unexpected arg count */".to_string(),
        "dense" => format!("{}.to_dense()", arg_strs[0]),
        "nnz" => format!("{}.nnz() as i64", arg_strs[0]),
        "spmv" => format!("{}.matmul_dense(&{})", arg_strs[0], arg_strs[1]),
        // I/O
        "load" => format!("{{ match crow_runtime::io::load_csv_numeric(&{}) {{ Ok((data, nrows, ncols)) => CrowResult {{ value: CrowTensor::from_vec(data, vec![nrows, ncols]), ok: true, reason: String::new(), kind: String::new(), trace: vec![] }}, Err(e) => CrowResult {{ value: CrowTensor::zeros(vec![0]), ok: false, reason: e, kind: \"load\".to_string(), trace: vec![] }} }} }}", arg_strs[0]),
        _ => return Err(format!("{}: unknown math function for transpiler", name)),
    };
    Ok(result)
}

// ===== Type inference =====

fn math_type_of(name: &str, arg_count: usize) -> CrowType {
    match name {
        // Tensor creation → NumArray(Float)
        "zeros" | "ones" | "eye" | "randn" | "linspace"
        | "arange" | "full" | "diag" | "tensor" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Element-wise → NumArray(Float)
        "exp" | "ln" | "sqrt" | "sin" | "cos" | "abs"
        | "sign" | "floor" | "ceil" | "round" | "clip" | "mod"
        | "tan" | "asin" | "acos" | "atan" | "atan2"
        | "sinh" | "cosh" | "tanh" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Shape ops → NumArray(Float)
        "reshape" | "flatten" | "transpose" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Linalg returning tensor
        "solve" | "inv" | "cholesky" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Linalg returning scalar
        "det" | "norm" => CrowType::Float,
        // Reductions: scalar if no axis, NumArray if axis given
        "sum" | "mean" | "min" | "max" | "prod" => {
            if arg_count >= 2 {
                CrowType::NumArray(Box::new(CrowType::Float))
            } else {
                CrowType::Float
            }
        }
        "argmax" | "argmin" => CrowType::Int,
        // Cumulative
        "cumsum" | "cumprod" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Combining
        "concat" | "stack" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Sorting & selection
        "argsort" | "topk" | "unique" | "where" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Decompositions returning arrays of tensors
        "svd" | "qr" | "eigen" => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
        // Random & Distributions
        "seed" => CrowType::Void,
        "normal" | "uniform" | "poisson" | "bernoulli"
        | "shuffle" | "choice" | "permutation" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Signal processing
        "fft" | "fft2" => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
        "ifft" | "conv" | "conv2d" | "hamming" | "hanning" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Special functions
        "gamma" | "erf" | "factorial" | "bessel" | "choose"
        | "magnitude" | "phase" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Complex (returns [real, imag] pair)
        "complex" | "conjugate" => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
        // Complex extraction
        "real" | "imag" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Sparse
        "sparse" => CrowType::Sparse,
        "dense" => CrowType::NumArray(Box::new(CrowType::Float)),
        "nnz" => CrowType::Int,
        "spmv" => CrowType::NumArray(Box::new(CrowType::Float)),
        // I/O
        "load" => CrowType::Result(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
        _ => CrowType::Unknown,
    }
}