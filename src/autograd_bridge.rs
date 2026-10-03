//! The interpreter's side of `grad` over tensors (crow_runtime::autograd):
//! a function `grad` differentiates runs on traced values
//! (`Value::Traced`), and every operation on one — an operator, a
//! builtin — reaches the autograd kernel here, which records it on the
//! tape. A builtin that is not differentiable is the reason, in the
//! words the lowering refuses it with where it sees the call; the
//! interpreter meets it only where the lowering could not see.

use crate::ast::{BinOp, UnaryOp};
use crate::value::*;
use crow_runtime::autograd::{self, Traced};
use crow_runtime::tensor::CrowTensor;

/// A traced value as the interpreter holds it.
pub fn wrap(t: Traced) -> CrowValue {
    CrowValue::ok(Value::Traced(Box::new(t)))
}

/// Whether any of the values is traced.
pub fn any_traced(vals: &[CrowValue]) -> bool {
    vals.iter().any(|v| matches!(v.value, Value::Traced(_)))
}

/// A value as a traced one: a traced value itself, a tensor or a number
/// as a constant; anything else is the reason.
pub fn traced_of(v: &Value) -> Result<Traced, String> {
    match v {
        Value::Traced(t) => Ok(t.as_ref().clone()),
        Value::NumArray(t) => Ok(Traced::constant(t.as_ref().clone())),
        Value::Float(x) => Ok(Traced::number(*x)),
        Value::Int(n) => Ok(Traced::number(*n as f64)),
        Value::Dual(x, _) => Ok(Traced::number(*x)),
        other => Err(format!("grad: {} is not differentiable — a function grad differentiates computes with numbers and tensors", other.type_name())),
    }
}

/// The builtins a function `grad` differentiates may call with a traced
/// value (the lowering's list, crate::lower_ir::grad).
pub fn call(name: &str, vals: &[CrowValue]) -> Result<CrowValue, String> {
    let one = |i: usize| -> Result<Traced, String> {
        vals.get(i).ok_or_else(|| format!("{}[] is missing an argument", name)).and_then(|v| traced_of(&v.value))
    };
    let int = |i: usize| -> Result<i64, String> {
        match vals.get(i).map(|v| &v.value) {
            Some(Value::Int(n)) => Ok(*n),
            Some(Value::Float(x)) if x.fract() == 0.0 => Ok(*x as i64),
            Some(Value::Traced(t)) => t.as_number().filter(|x| x.fract() == 0.0).map(|x| x as i64).ok_or_else(|| format!("{}[] takes a whole number", name)),
            _ => Err(format!("{}[] takes a whole number", name)),
        }
    };
    let axis = |i: usize| -> Result<usize, String> { usize::try_from(int(i)?).map_err(|_| format!("{}: an axis is 0 or more", name)) };
    Ok(match (name, vals.len()) {
        ("sum", 1) => wrap(autograd::sum(&one(0)?)),
        ("sum", 2) => wrap(autograd::sum_axis(&one(0)?, axis(1)?)?),
        ("mean", 1) => wrap(autograd::mean(&one(0)?)),
        ("mean", 2) => wrap(autograd::mean_axis(&one(0)?, axis(1)?)?),
        ("exp", 1) => wrap(autograd::exp(&one(0)?)),
        ("ln", 1) => wrap(autograd::ln(&one(0)?)),
        ("sqrt", 1) => wrap(autograd::sqrt(&one(0)?)),
        ("tanh", 1) => wrap(autograd::tanh(&one(0)?)),
        ("abs", 1) => wrap(autograd::abs(&one(0)?)),
        ("relu", 1) => wrap(autograd::relu(&one(0)?)),
        ("sigmoid", 1) => wrap(autograd::sigmoid(&one(0)?)),
        ("softmax", 1) => wrap(autograd::softmax(&one(0)?)),
        ("mse", 2) => wrap(autograd::mse(&one(0)?, &one(1)?)?),
        ("bce", 2) => wrap(autograd::bce(&one(0)?, &one(1)?)?),
        ("crossentropy", 2) => wrap(autograd::crossentropy(&one(0)?, &one(1)?)?),
        ("linear", 3) => wrap(autograd::linear(&one(0)?, &one(1)?, &one(2)?)?),
        ("transpose", 1) => wrap(autograd::transpose(&one(0)?)),
        ("flatten", 1) => wrap(autograd::flatten(&one(0)?)),
        ("reshape", n) if n >= 2 => {
            let shape: Vec<usize> = (1..n).map(|i| usize::try_from(int(i)?).map_err(|_| "reshape: a size is 0 or more".to_string())).collect::<Result<_, _>>()?;
            wrap(autograd::reshape(&one(0)?, shape)?)
        }
        // read, never differentiated
        ("len", 1) => CrowValue::ok(Value::Int(crow_runtime::core::len_tensor(&one(0)?.value))),
        ("float", 1) | ("tensor", 1) => wrap(one(0)?),
        ("int", 1) => {
            let x = one(0)?.as_number().ok_or_else(|| "int[] takes a number".to_string())?;
            CrowValue::ok(Value::Int(x as i64))
        }
        _ => return Err(format!("grad: `{}` is not differentiable — a function grad differentiates may use {}", name, crate::lower_ir::grad::DIFFERENTIABLE.join(", "))),
    })
}

/// An operator on a traced value: the kernel's own, recorded on the tape.
/// `*` of two tensors is the matrix product; a comparison of two numbers
/// decides as it would, with no gradient through it.
pub fn binop(op: &BinOp, l: &Value, r: &Value) -> Result<CrowValue, String> {
    let (a, b) = (traced_of(l)?, traced_of(r)?);
    let both_tensors = a.value.ndim() >= 1 && b.value.ndim() >= 1;
    let number = |t: &Traced| t.as_number().ok_or_else(|| "grad: a comparison of tensors is not differentiable (compare numbers)".to_string());
    Ok(match op {
        BinOp::Add => wrap(autograd::add(&a, &b)?),
        BinOp::Sub => wrap(autograd::sub(&a, &b)?),
        BinOp::Mul if both_tensors => wrap(autograd::matmul(&a, &b)?),
        BinOp::Mul | BinOp::ElemMul => wrap(autograd::mul(&a, &b)?),
        BinOp::Div if both_tensors => return Err("use ./ for element-wise tensor division".to_string()),
        BinOp::Div | BinOp::ElemDiv => wrap(autograd::div(&a, &b)?),
        BinOp::Pow | BinOp::ElemPow => wrap(autograd::pow(&a, &b)?),
        BinOp::Mod => return Err("grad: `%` is not differentiable".to_string()),
        BinOp::Eq => CrowValue::ok(Value::Bool(number(&a)? == number(&b)?)),
        BinOp::Ne => CrowValue::ok(Value::Bool(number(&a)? != number(&b)?)),
        BinOp::Gt => CrowValue::ok(Value::Bool(number(&a)? > number(&b)?)),
        BinOp::Lt => CrowValue::ok(Value::Bool(number(&a)? < number(&b)?)),
        BinOp::Gte => CrowValue::ok(Value::Bool(number(&a)? >= number(&b)?)),
        BinOp::Lte => CrowValue::ok(Value::Bool(number(&a)? <= number(&b)?)),
        BinOp::And | BinOp::Or => return Err("grad: `and` and `or` decide on bools, which a tensor is not".to_string()),
    })
}

pub fn unaryop(op: &UnaryOp, v: &Value) -> Result<CrowValue, String> {
    match op {
        UnaryOp::Neg => Ok(wrap(autograd::neg(&traced_of(v)?))),
        UnaryOp::Not => Err("grad: `not` decides on a bool, which a tensor is not".to_string()),
    }
}

/// `grad[f][x, …]` over tensors: `f` run on traced leaves, the gradient of
/// its number with respect to each — one tensor, or a list of them.
pub fn gradient(inputs: Vec<CrowTensor>, run: impl FnOnce(&[Traced]) -> Result<Traced, String>) -> Result<CrowValue, String> {
    let grads = autograd::gradient(&inputs, run)?;
    Ok(if grads.len() == 1 {
        CrowValue::ok(Value::NumArray(Box::new(grads.into_iter().next().unwrap_or_else(|| CrowTensor::zeros(vec![0])))))
    } else {
        CrowValue::ok(Value::Array(grads.into_iter().map(|g| CrowValue::ok(Value::NumArray(Box::new(g)))).collect()))
    })
}
