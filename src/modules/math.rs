use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;
use crow_runtime::tensor::CrowTensor;
use crow_runtime::sparse::CrowSparse;


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

fn to_float(v: &CrowValue, fname: &str) -> Result<f64, String> {
    match &v.value {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        _ => Err(format!("{}: expected number, got {:?}", fname, v.value)),
    }
}

fn to_tensor(v: &CrowValue, fname: &str) -> Result<CrowTensor, String> {
    match &v.value {
        Value::NumArray(t) => Ok(t.clone()),
        // All-numeric plain arrays coerce (frame columns arrive this way in
        // the interpreter — compiled mode already accepts columns)
        Value::Array(items) => {
            let mut data = Vec::with_capacity(items.len());
            for item in items {
                match &item.value {
                    Value::Int(n) => data.push(*n as f64),
                    Value::Float(f) => data.push(*f),
                    other => {
                        return Err(format!(
                            "{}: expected numeric array, got element {:?}",
                            fname, other
                        ))
                    }
                }
            }
            let n = data.len();
            Ok(CrowTensor::from_vec(data, vec![n]))
        }
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
            if args.is_empty() || args.len() > 2 { return Err(format!("tensor: expected 1-2 arguments, got {}", args.len())); }
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
                Value::Frame(frame) => {
                    // tensor[df] or tensor[df, [col1, col2]]
                    let col_names: Option<Vec<String>> = if args.len() == 2 {
                        match &args[1].value {
                            Value::Array(arr) => {
                                let names: Result<Vec<String>, String> = arr.iter().map(|v| match &v.value {
                                    Value::Str(s) => Ok(s.clone()),
                                    _ => Err("tensor: column names must be strings".into()),
                                }).collect();
                                Some(names?)
                            }
                            _ => return Err("tensor: second argument must be an array of column names".into()),
                        }
                    } else { None };
                    let (data, nrows, ncols) = crow_runtime::frame::frame_to_tensor_data(
                        frame, col_names.as_deref(),
                    )?;
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![nrows, ncols]))))
                }
                _ => Err(format!("tensor: expected array or frame, got {:?}", args[0].value)),
            }
        }

        // Element-wise math
        "exp" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.exp()))) }
        "ln" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.log()))) }
        "log2" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.log2()))) }
        "log10" => { let t = to_tensor(&args[0], name)?; Ok(CrowValue::ok(Value::NumArray(t.log10()))) }
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
                // Array-of-tensor literals auto-build a stacked tensor now;
                // accept it and restack along the requested axis
                Value::NumArray(t) => {
                    let tensors = t.rows();
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
        "dot" => {
            if args.len() != 2 { return Err(format!("dot: expected 2 arguments, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let b = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::Float(a.dot(&b))))
        }
        "pinv" => {
            if args.len() != 1 { return Err(format!("pinv: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.pinv())))
        }
        "lstsq" => {
            if args.len() != 2 { return Err(format!("lstsq: expected 2 arguments, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            let b = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.lstsq(&b))))
        }
        "triu" => {
            if args.len() != 1 { return Err(format!("triu: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.triu())))
        }
        "tril" => {
            if args.len() != 1 { return Err(format!("tril: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(a.tril())))
        }
        "rank" => {
            if args.len() != 1 { return Err(format!("rank: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::Int(a.rank() as i64)))
        }
        "cond" => {
            if args.len() != 1 { return Err(format!("cond: expected 1 argument, got {}", args.len())); }
            let a = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::Float(a.cond())))
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
        "gamma_dist" => {
            if args.len() < 2 { return Err(format!("gamma_dist: expected at least 2 arguments (alpha, beta, dims...), got {}", args.len())); }
            let alpha = to_f64(&args[0], name)?;
            let beta = to_f64(&args[1], name)?;
            let shape = shape_from_args(&args[2..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::gamma_dist(shape, alpha, beta))))
        }
        "beta_dist" => {
            if args.len() < 2 { return Err(format!("beta_dist: expected at least 2 arguments (alpha, beta, dims...), got {}", args.len())); }
            let alpha = to_f64(&args[0], name)?;
            let beta = to_f64(&args[1], name)?;
            let shape = shape_from_args(&args[2..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::beta_dist(shape, alpha, beta))))
        }
        "exponential" => {
            if args.is_empty() { return Err(format!("exponential: expected at least 1 argument (lambda, dims...), got 0")); }
            let lambda = to_f64(&args[0], name)?;
            let shape = shape_from_args(&args[1..], name)?;
            let shape = if shape.is_empty() { vec![1] } else { shape };
            Ok(CrowValue::ok(Value::NumArray(CrowTensor::exponential(shape, lambda))))
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
        "stft" => {
            if args.len() != 3 { return Err(format!("stft: expected 3 arguments (signal, window_size, hop_size), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let win = to_usize(&args[1], name)?;
            let hop = to_usize(&args[2], name)?;
            let (real, imag) = t.stft(win, hop);
            Ok(CrowValue::ok(Value::Array(vec![
                CrowValue::ok(Value::NumArray(real)),
                CrowValue::ok(Value::NumArray(imag)),
            ])))
        }
        "spectrogram" => {
            if args.len() != 3 { return Err(format!("spectrogram: expected 3 arguments (signal, window_size, hop_size), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let win = to_usize(&args[1], name)?;
            let hop = to_usize(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.spectrogram(win, hop))))
        }
        "lowpass" => {
            if args.len() != 3 { return Err(format!("lowpass: expected 3 arguments (signal, cutoff_hz, sample_rate), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let cutoff = to_float(&args[1], name)?;
            let sr = to_float(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.lowpass(cutoff, sr))))
        }
        "highpass" => {
            if args.len() != 3 { return Err(format!("highpass: expected 3 arguments (signal, cutoff_hz, sample_rate), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let cutoff = to_float(&args[1], name)?;
            let sr = to_float(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.highpass(cutoff, sr))))
        }
        "bandpass" => {
            if args.len() != 4 { return Err(format!("bandpass: expected 4 arguments (signal, low_hz, high_hz, sample_rate), got {}", args.len())); }
            let t = to_tensor(&args[0], name)?;
            let low = to_float(&args[1], name)?;
            let high = to_float(&args[2], name)?;
            let sr = to_float(&args[3], name)?;
            Ok(CrowValue::ok(Value::NumArray(t.bandpass(low, high, sr))))
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
                Err(e) => Ok(CrowValue::error_with_kind(&e, "load", "")),
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
        "tensor" => {
            // Check if argument is a Frame type
            let is_frame = !arg_types.is_empty() && (matches!(arg_types[0], CrowType::Frame) || matches!(&arg_types[0], CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Frame)));
            if is_frame {
                if arg_strs.len() == 2 {
                    format!("{{ let (_d, _r, _c) = crow_runtime::frame::frame_to_tensor_data(&{}, Some(&{})).unwrap(); CrowTensor::from_vec(_d, vec![_r, _c]) }}", arg_strs[0], arg_strs[1])
                } else {
                    format!("{{ let (_d, _r, _c) = crow_runtime::frame::frame_to_tensor_data(&{}, None).unwrap(); CrowTensor::from_vec(_d, vec![_r, _c]) }}", arg_strs[0])
                }
            } else if !arg_types.is_empty() && matches!(arg_types[0], CrowType::NumArray(_)) {
                // an all-numeric array literal is already a tensor
                arg_strs[0].clone()
            } else {
                format!("{{ let _arr = {}; CrowTensor::from_vec(_arr.iter().map(|v| *v as f64).collect(), vec![_arr.len()]) }}", arg_strs[0])
            }
        }
        // Element-wise math (ln replaces log)
        "exp" => format!("{}.exp()", arg_strs[0]),
        "ln" => format!("{}.log()", arg_strs[0]),
        "log2" => format!("{}.log2()", arg_strs[0]),
        "log10" => format!("{}.log10()", arg_strs[0]),
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
        // Combining. Array-of-tensor literals now build a stacked tensor,
        // so stack accepts either a Vec of tensors or a tensor (restack rows).
        "stack" => {
            let axis = if arg_strs.len() >= 2 { format!("{} as usize", arg_strs[1]) } else { "0".to_string() };
            if matches!(arg_types.first(), Some(CrowType::NumArray(_))) {
                format!("CrowTensor::stack(&{}.rows(), {})", arg_strs[0], axis)
            } else {
                format!("CrowTensor::stack(&{}, {})", arg_strs[0], axis)
            }
        }
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
        "dot" => format!("{}.dot(&{})", arg_strs[0], arg_strs[1]),
        "pinv" => format!("{}.pinv()", arg_strs[0]),
        "lstsq" => format!("{}.lstsq(&{})", arg_strs[0], arg_strs[1]),
        "triu" => format!("{}.triu()", arg_strs[0]),
        "tril" => format!("{}.tril()", arg_strs[0]),
        "rank" => format!("{}.rank() as i64", arg_strs[0]),
        "cond" => format!("{}.cond()", arg_strs[0]),
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
        "gamma_dist" => {
            let alpha = &arg_strs[0];
            let beta = &arg_strs[1];
            let shape_args: Vec<String> = arg_strs[2..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::gamma_dist(vec![{}], {} as f64, {} as f64)", shape, alpha, beta)
        }
        "beta_dist" => {
            let alpha = &arg_strs[0];
            let beta = &arg_strs[1];
            let shape_args: Vec<String> = arg_strs[2..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::beta_dist(vec![{}], {} as f64, {} as f64)", shape, alpha, beta)
        }
        "exponential" => {
            let lambda = &arg_strs[0];
            let shape_args: Vec<String> = arg_strs[1..].iter().map(|a| format!("{} as usize", a)).collect();
            let shape = if shape_args.is_empty() { "1usize".to_string() } else { shape_args.join(", ") };
            format!("CrowTensor::exponential(vec![{}], {} as f64)", shape, lambda)
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
        "stft" => format!("{{ let (_re, _im) = {}.stft({} as usize, {} as usize); vec![_re, _im] }}", arg_strs[0], arg_strs[1], arg_strs[2]),
        "spectrogram" => format!("{}.spectrogram({} as usize, {} as usize)", arg_strs[0], arg_strs[1], arg_strs[2]),
        "lowpass" => format!("{}.lowpass({}, {})", arg_strs[0], arg_strs[1], arg_strs[2]),
        "highpass" => format!("{}.highpass({}, {})", arg_strs[0], arg_strs[1], arg_strs[2]),
        "bandpass" => format!("{}.bandpass({}, {}, {})", arg_strs[0], arg_strs[1], arg_strs[2], arg_strs[3]),
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
        "load" => format!("{{ match crow_runtime::io::load_csv_numeric(&{}) {{ Ok((data, nrows, ncols)) => CrowResult {{ value: CrowTensor::from_vec(data, vec![nrows, ncols]), ok: true, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(e) => CrowResult {{ value: CrowTensor::zeros(vec![0]), ok: false, reason: e, kind: \"load\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0]),
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
        "exp" | "ln" | "log2" | "log10" | "sqrt" | "sin" | "cos" | "abs"
        | "sign" | "floor" | "ceil" | "round" | "clip" | "mod"
        | "tan" | "asin" | "acos" | "atan" | "atan2"
        | "sinh" | "cosh" | "tanh" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Shape ops → NumArray(Float)
        "reshape" | "flatten" | "transpose" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Linalg returning tensor
        "solve" | "inv" | "cholesky" | "pinv" | "lstsq" | "triu" | "tril" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Linalg returning scalar
        "det" | "norm" | "dot" | "cond" => CrowType::Float,
        "rank" => CrowType::Int,
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
        | "gamma_dist" | "beta_dist" | "exponential"
        | "shuffle" | "choice" | "permutation" => CrowType::NumArray(Box::new(CrowType::Float)),
        // Signal processing
        "fft" | "fft2" | "stft" => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
        "ifft" | "conv" | "conv2d" | "hamming" | "hanning"
        | "spectrogram" | "lowpass" | "highpass" | "bandpass" => CrowType::NumArray(Box::new(CrowType::Float)),
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{CrowValue, Value};
    use crow_runtime::tensor::CrowTensor;

    // Helper: make CrowValue args for call_math
    fn int(n: i64) -> CrowValue { CrowValue::ok(Value::Int(n)) }
    fn float(f: f64) -> CrowValue { CrowValue::ok(Value::Float(f)) }
    fn tensor(data: Vec<f64>, shape: Vec<usize>) -> CrowValue {
        CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, shape)))
    }
    fn tensor_1d(data: Vec<f64>) -> CrowValue {
        let len = data.len();
        CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len])))
    }
    fn str_val(s: &str) -> CrowValue { CrowValue::ok(Value::Str(s.to_string())) }

    fn unwrap_tensor(r: Result<CrowValue, String>) -> CrowTensor {
        let cv = r.expect("call_math failed");
        assert!(cv.ok, "CrowValue is error: {}", cv.reason);
        match cv.value {
            Value::NumArray(t) => t,
            other => panic!("expected NumArray, got {:?}", other),
        }
    }
    fn unwrap_float(r: Result<CrowValue, String>) -> f64 {
        let cv = r.expect("call_math failed");
        assert!(cv.ok, "CrowValue is error: {}", cv.reason);
        match cv.value {
            Value::Float(f) => f,
            Value::Int(n) => n as f64,
            other => panic!("expected Float, got {:?}", other),
        }
    }
    fn unwrap_int(r: Result<CrowValue, String>) -> i64 {
        let cv = r.expect("call_math failed");
        assert!(cv.ok, "CrowValue is error: {}", cv.reason);
        match cv.value {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }
    fn unwrap_array(r: Result<CrowValue, String>) -> Vec<CrowValue> {
        let cv = r.expect("call_math failed");
        assert!(cv.ok, "CrowValue is error: {}", cv.reason);
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
        assert!(cv.ok);
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
        assert!(r.ok);
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