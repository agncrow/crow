use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;

pub struct NnModule;

pub(crate) const NN_FUNCTIONS: &[&str] = &[
    "relu", "sigmoid", "softmax", "gelu",
    "mse", "crossentropy", "bce",
    "linear",
];

impl CrowModule for NnModule {
    fn name(&self) -> &str { "math.nn" }

    fn functions(&self) -> &[&str] { NN_FUNCTIONS }

    fn constants(&self) -> Vec<(&str, CrowValue)> { vec![] }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        for a in args {
            if !a.is_ok() {
                return Ok(a.clone());
            }
        }
        call_nn(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        emit_nn_rust(name, arg_strs)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        nn_type_of(name)
    }
}

fn to_tensor<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a crow_runtime::tensor::CrowTensor, String> {
    match &v.value {
        Value::NumArray(t) => Ok(t),
        _ => Err(format!("{}: expected numarray", fname)),
    }
}

fn call_nn(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "relu" => {
            if args.len() != 1 { return Err("relu: expected 1 argument".into()); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(crow_runtime::nn::relu(t))))
        }
        "sigmoid" => {
            if args.len() != 1 { return Err("sigmoid: expected 1 argument".into()); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(crow_runtime::nn::sigmoid(t))))
        }
        "softmax" => {
            if args.len() != 1 { return Err("softmax: expected 1 argument".into()); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(crow_runtime::nn::softmax(t))))
        }
        "gelu" => {
            if args.len() != 1 { return Err("gelu: expected 1 argument".into()); }
            let t = to_tensor(&args[0], name)?;
            Ok(CrowValue::ok(Value::NumArray(crow_runtime::nn::gelu(t))))
        }
        "mse" => {
            if args.len() != 2 { return Err("mse: expected 2 arguments".into()); }
            let pred = to_tensor(&args[0], name)?;
            let target = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::Float(crow_runtime::nn::mse(pred, target))))
        }
        "crossentropy" => {
            if args.len() != 2 { return Err("crossentropy: expected 2 arguments".into()); }
            let logits = to_tensor(&args[0], name)?;
            let targets = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::Float(crow_runtime::nn::crossentropy(logits, targets))))
        }
        "bce" => {
            if args.len() != 2 { return Err("bce: expected 2 arguments".into()); }
            let pred = to_tensor(&args[0], name)?;
            let target = to_tensor(&args[1], name)?;
            Ok(CrowValue::ok(Value::Float(crow_runtime::nn::bce(pred, target))))
        }
        "linear" => {
            if args.len() != 3 { return Err("linear: expected 3 arguments (x, w, b)".into()); }
            let x = to_tensor(&args[0], name)?;
            let w = to_tensor(&args[1], name)?;
            let b = to_tensor(&args[2], name)?;
            Ok(CrowValue::ok(Value::NumArray(crow_runtime::nn::linear(x, w, b))))
        }
        _ => Err(format!("{}: unknown nn function", name)),
    }
}

fn emit_nn_rust(name: &str, arg_strs: &[String]) -> Result<String, String> {
    let result = match name {
        "relu" => format!("crow_runtime::nn::relu(&{})", arg_strs[0]),
        "sigmoid" => format!("crow_runtime::nn::sigmoid(&{})", arg_strs[0]),
        "softmax" => format!("crow_runtime::nn::softmax(&{})", arg_strs[0]),
        "gelu" => format!("crow_runtime::nn::gelu(&{})", arg_strs[0]),
        "mse" => format!("crow_runtime::nn::mse(&{}, &{})", arg_strs[0], arg_strs[1]),
        "crossentropy" => format!("crow_runtime::nn::crossentropy(&{}, &{})", arg_strs[0], arg_strs[1]),
        "bce" => format!("crow_runtime::nn::bce(&{}, &{})", arg_strs[0], arg_strs[1]),
        "linear" => format!("crow_runtime::nn::linear(&{}, &{}, &{})", arg_strs[0], arg_strs[1], arg_strs[2]),
        _ => return Err(format!("{}: unknown nn function for the Rust backend", name)),
    };
    Ok(result)
}

fn nn_type_of(name: &str) -> CrowType {
    match name {
        "relu" | "sigmoid" | "softmax" | "gelu" | "linear" => CrowType::NumArray(Box::new(CrowType::Float)),
        "mse" | "crossentropy" | "bce" => CrowType::Float,
        _ => CrowType::Unknown,
    }
}

