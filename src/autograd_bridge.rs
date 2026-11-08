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
use crate::table::{Row, Tape};

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

/// A builtin a function `grad` differentiates calls with a traced value:
/// the kernel its row declares (`table::Tape`), recorded on the tape. A row
/// with none is the reason, in the words the lowering refuses it with where
/// it sees the call; the interpreter meets it only where the lowering could
/// not see.
pub fn call(row: &'static Row, vals: &[CrowValue]) -> Result<CrowValue, String> {
    let name = crate::table::written_name(row);
    match crate::table::tape_of(row, vals.len()) {
        Some(tape) => run(tape, row, name, vals),
        None => Err(not_differentiable(name)),
    }
}

/// The same call of a builtin the IR left by name.
pub fn call_named(name: &str, vals: &[CrowValue]) -> Result<CrowValue, String> {
    match crate::table::tape_named(name, vals.len()) {
        Some((row, tape)) => run(tape, row, name, vals),
        None => Err(not_differentiable(name)),
    }
}

fn not_differentiable(name: &str) -> String {
    format!("grad: `{}` is not differentiable — a function grad differentiates may use {}", name, crate::table::taped().join(", "))
}

fn run(tape: Tape, row: &Row, name: &str, vals: &[CrowValue]) -> Result<CrowValue, String> {
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
    Ok(match tape {
        Tape::Map(_, f) => wrap(f(&one(0)?)),
        Tape::Axis(_, f) => wrap(f(&one(0)?, axis(1)?)?),
        Tape::Bounds(_, f) => wrap(f(&one(0)?, &one(1)?, &one(2)?)?),
        Tape::Join(_, f) => wrap(f(&one(0)?, &one(1)?, if vals.len() == 3 { int(2)? } else { 0 })?),
        Tape::Span(_, f) => wrap(f(&one(0)?, int(1)?, int(2)?)),
        Tape::Choose(_, f) => wrap(f(&condition(&vals[0].value)?, &one(1)?, &one(2)?)?),
        Tape::Shape(_, f) => {
            let shape: Vec<usize> = (1..vals.len()).map(|i| usize::try_from(int(i)?).map_err(|_| format!("{}: a size is 0 or more", name))).collect::<Result<_, _>>()?;
            wrap(f(&one(0)?, shape)?)
        }
        // the row's own function of the values; a tensor it answers is a
        // constant of the tape
        Tape::Read => {
            let plain: Vec<CrowValue> = vals
                .iter()
                .map(|v| match &v.value {
                    Value::Traced(t) if t.value.ndim() == 0 => CrowValue::ok(Value::Float(t.as_number().unwrap_or(f64::NAN))),
                    Value::Traced(t) => CrowValue::ok(Value::NumArray(Box::new(t.value.clone()))),
                    _ => v.clone(),
                })
                .collect();
            let answer = (row.call)(&plain, "")?;
            match answer.value {
                Value::NumArray(t) if answer.is_ok() => wrap(Traced::constant(*t)),
                _ => answer,
            }
        }
        Tape::Same => wrap(one(0)?),
        Tape::Whole => {
            let x = one(0)?.as_number().ok_or_else(|| format!("{}[] takes a number", name))?;
            CrowValue::ok(Value::Int(x as i64))
        }
    })
}

/// A condition `where` reads: a mask (true 1, false 0, a matrix's rows of
/// them), or a tensor's numbers — a constant of the tape either way.
fn condition(v: &Value) -> Result<CrowTensor, String> {
    match v {
        Value::Array(items) => {
            let mut flat = Vec::new();
            let shape = crate::ops::truths_flat(items, &mut flat)?;
            Ok(CrowTensor::from_vec(flat, shape))
        }
        other => Ok(traced_of(other)?.value),
    }
}

/// `t[i]` and `t[positions]` of a traced tensor: a row or a number by a
/// whole number, the numbers at a tensor of positions (spec, Advanced
/// Indexing), each recorded on the tape.
pub fn index(t: &Traced, idx: &[CrowValue]) -> Result<CrowValue, String> {
    let [i] = idx else { return Err("a tensor is indexed one position at a time — `m[i][j]`".to_string()) };
    match i.unwrap_or_propagate()? {
        Value::Int(n) => Ok(wrap(autograd::at(t, *n)?)),
        Value::NumArray(p) => Ok(wrap(autograd::gather(t, p)?)),
        Value::Traced(p) => Ok(wrap(autograd::gather(t, &p.value)?)),
        other => Err(format!("a tensor's position is a whole number, or a tensor of them — got {}", other.type_name())),
    }
}

/// An operator on a traced value: the kernel's own, recorded on the tape.
/// `*` of two tensors is the matrix product; a comparison of two numbers
/// decides as it would, with no gradient through it.
pub fn binop(op: &BinOp, l: &Value, r: &Value) -> Result<CrowValue, String> {
    let (a, b) = (traced_of(l)?, traced_of(r)?);
    let both_tensors = a.value.ndim() >= 1 && b.value.ndim() >= 1;
    // a tensor compared with a number answers its mask, as it does
    // untraced: a constant of the tape, no gradient through the decision
    let compares = matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte);
    if compares && (a.value.ndim() >= 1) != (b.value.ndim() >= 1) {
        let plain = |t: &Traced| {
            CrowValue::ok(match t.as_number().filter(|_| t.value.ndim() == 0) {
                Some(x) => Value::Float(x),
                None => Value::NumArray(Box::new(t.value.clone())),
            })
        };
        return crate::ops::binop(&plain(&a), op, &plain(&b));
    }
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
        CrowValue::ok(Value::Array(std::sync::Arc::new(grads.into_iter().map(|g| CrowValue::ok(Value::NumArray(Box::new(g)))).collect())))
    })
}
