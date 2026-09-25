//! The operators, indexing and field access, in one place:
//! what `+` means between an int and a tensor, what `xs[-1]` is, what
//! `df.price` reads. The interpreter (`ir_interp.rs`, ir-design.md §5)
//! calls nothing else for them; the compiled mode's runtime is built from
//! the same kernels.

use crate::ast::{BinOp, UnaryOp};
use crate::value::*;
use crow_runtime::tensor::CrowTensor;

pub fn binop(
    left: &CrowValue,
    op: &BinOp,
    right: &CrowValue,
    trace: &str,
) -> Result<CrowValue, String> {
    // soft error propagation: if either operand is an error, propagate it
    if left.is_error() {
        return Ok(left.clone());
    }
    if right.is_error() {
        return Ok(right.clone());
    }
    // Note: trace is appended at the call site (eval_call / call_builtin), not here

    let l = &left.value;
    let r = &right.value;

    match op {
        BinOp::Add => match (l, r) {
            // Dual number rules: dual(a,a') + dual(b,b') = dual(a+b, a'+b')
            (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a + b, da + db))),
            (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a + b, *da))),
            (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a + b, *db))),
            (Value::Dual(a, da), Value::Int(b)) => Ok(CrowValue::ok(Value::Dual(a + *b as f64, *da))),
            (Value::Int(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(*a as f64 + b, *db))),
            (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a + b))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a + b))),
            (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 + b))),
            (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a + *b as f64))),
            (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.add(b)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(*b as f64)))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(*b)))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.add_scalar(*a as f64)))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.add_scalar(*a)))),
            // sparse + sparse
            (Value::Sparse(a), Value::Sparse(b)) => Ok(CrowValue::ok(Value::Sparse(a.add(b)))),
            (Value::Str(_), Value::Str(_)) => Err("cannot use '+' with strings — use string interpolation instead".to_string()),
            // Vectorized: Array + scalar
            (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) |
            (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                array_scalar_arithmetic(items, scalar, |a, b| a + b)
            }
            _ => Err("cannot add these types".to_string()),
        },
        BinOp::Sub => match (l, r) {
            // Dual: dual(a,a') - dual(b,b') = dual(a-b, a'-b')
            (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a - b, da - db))),
            (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a - b, *da))),
            (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a - b, -db))),
            (Value::Dual(a, da), Value::Int(b)) => Ok(CrowValue::ok(Value::Dual(a - *b as f64, *da))),
            (Value::Int(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(*a as f64 - b, -db))),
            (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a - b))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a - b))),
            (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 - b))),
            (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a - *b as f64))),
            (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.sub(b)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(-(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(-*b)))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.neg().add_scalar(*a as f64)))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.neg().add_scalar(*a)))),
            // Vectorized: Array - scalar
            (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) => {
                array_scalar_arithmetic(items, scalar, |a, b| a - b)
            }
            (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                array_scalar_arithmetic(items, scalar, |a, b| b - a)
            }
            _ => Err("cannot subtract these types".to_string()),
        },
        BinOp::Mul => match (l, r) {
            // Dual: dual(a,a') * dual(b,b') = dual(a*b, a'*b + a*b')
            (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a * b, da * b + a * db))),
            (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a * b, da * b))),
            (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a * b, a * db))),
            (Value::Dual(a, da), Value::Int(b)) => Ok(CrowValue::ok(Value::Dual(a * *b as f64, da * *b as f64))),
            (Value::Int(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(*a as f64 * b, *a as f64 * db))),
            (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a * b))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a * b))),
            (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 * b))),
            (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a * *b as f64))),
            // tensor * tensor → matmul; tensor * scalar → scale
            (Value::NumArray(a), Value::NumArray(b)) => a.try_matmul(b).map(|t| CrowValue::ok(Value::NumArray(t))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b as f64)))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b)))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a as f64)))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a)))),
            // sparse * dense → matmul
            (Value::Sparse(s), Value::NumArray(t)) => Ok(CrowValue::ok(Value::NumArray(s.matmul_dense(t)))),
            // Vectorized: Array * scalar
            (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) |
            (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                array_scalar_arithmetic(items, scalar, |a, b| a * b)
            }
            _ => Err("cannot multiply these types".to_string()),
        },
        BinOp::Div => match (l, r) {
            // Dual: dual(a,a') / dual(b,b') = dual(a/b, (a'*b - a*b') / b^2)
            (Value::Dual(a, da), Value::Dual(b, db)) => {
                Ok(CrowValue::ok(Value::Dual(a / b, (da * b - a * db) / (b * b))))
            }
            (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a / b, da / b))),
            (Value::Float(a), Value::Dual(b, db)) => {
                Ok(CrowValue::ok(Value::Dual(a / b, (-a * db) / (b * b))))
            }
            (Value::Dual(a, da), Value::Int(b)) => {
                let bf = *b as f64;
                Ok(CrowValue::ok(Value::Dual(a / bf, da / bf)))
            }
            (Value::Int(a), Value::Dual(b, db)) => {
                let af = *a as f64;
                Ok(CrowValue::ok(Value::Dual(af / b, (-af * db) / (b * b))))
            }
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Ok(CrowValue::error_with_kind("division by zero", "math", trace))
                } else {
                    Ok(CrowValue::ok(Value::Int(a / b)))
                }
            }
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a / b))),
            (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 / b))),
            (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a / *b as f64))),
            // tensor / tensor → error (use ./ for element-wise)
            (Value::NumArray(_), Value::NumArray(_)) => Err("use ./ for element-wise tensor division".to_string()),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b as f64)))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b)))),
            (Value::Int(a), Value::NumArray(b)) => {
                let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a as f64);
                Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
            }
            (Value::Float(a), Value::NumArray(b)) => {
                let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a);
                Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
            }
            // Vectorized: Array / scalar
            (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) => {
                array_scalar_arithmetic(items, scalar, |a, b| a / b)
            }
            (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                array_scalar_arithmetic(items, scalar, |a, b| b / a)
            }
            _ => Err("cannot divide these types".to_string()),
        },
        BinOp::Mod => match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Ok(CrowValue::error_with_kind("modulo by zero", "math", trace))
                } else {
                    Ok(CrowValue::ok(Value::Int(a % b)))
                }
            }
            _ => Err("modulo only works on integers".to_string()),
        },
        BinOp::Pow => match (l, r) {
            // Dual: d/dx(x^n) = n * x^(n-1) * x'
            (Value::Dual(a, da), Value::Int(b)) => {
                let bf = *b as f64;
                Ok(CrowValue::ok(Value::Dual(a.powf(bf), da * bf * a.powf(bf - 1.0))))
            }
            (Value::Dual(a, da), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Dual(a.powf(*b), da * b * a.powf(b - 1.0))))
            }
            (Value::Dual(a, da), Value::Dual(b, db)) => {
                let val = a.powf(*b);
                let deriv = val * (db * a.ln() + b * da / a);
                Ok(CrowValue::ok(Value::Dual(val, deriv)))
            }
            (Value::Int(a), Value::Int(b)) => {
                if *b >= 0 {
                    Ok(CrowValue::ok(Value::Int(a.pow(*b as u32))))
                } else {
                    Ok(CrowValue::ok(Value::Float((*a as f64).powf(*b as f64))))
                }
            }
            (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b as f64)))),
            (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).powf(*b)))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b)))),
            _ => Err("^ requires numbers".to_string()),
        },
        BinOp::ElemMul => match (&coerce_numeric_array(l), &coerce_numeric_array(r)) {
            (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul(b)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b as f64)))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b)))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a as f64)))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a)))),
            _ => Err(".* requires tensors or tensor and scalar".to_string()),
        },
        BinOp::ElemDiv => match (&coerce_numeric_array(l), &coerce_numeric_array(r)) {
            (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.div(b)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b as f64)))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b)))),
            (Value::Int(a), Value::NumArray(b)) => {
                let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a as f64);
                Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
            }
            (Value::Float(a), Value::NumArray(b)) => {
                let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a);
                Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
            }
            _ => Err("./ requires tensors or tensor and scalar".to_string()),
        },
        BinOp::ElemPow => match (&coerce_numeric_array(l), &coerce_numeric_array(r)) {
            (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.pow(b)))),
            (Value::NumArray(a), Value::Int(b)) => {
                let s = *b as f64;
                Ok(CrowValue::ok(Value::NumArray(a.map(|x| x.powf(s)))))
            }
            (Value::NumArray(a), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::NumArray(a.map(|x| x.powf(*b)))))
            }
            _ => Err(".^ requires tensors or tensor and scalar".to_string()),
        },
        BinOp::Eq => equality_values(left, right, false),
        BinOp::Ne => equality_values(left, right, true),
        BinOp::Gt => compare_values(l, r, |a, b| a > b),
        BinOp::Lt => compare_values(l, r, |a, b| a < b),
        BinOp::Gte => compare_values(l, r, |a, b| a >= b),
        BinOp::Lte => compare_values(l, r, |a, b| a <= b),
        BinOp::And => logical_values(left, right, |a, b| a && b),
        BinOp::Or => logical_values(left, right, |a, b| a || b),
    }
}

/// eq/ne. Array vs scalar broadcasts element-wise into a boolean array
/// (mask semantics, matching gt/lt); everything else is whole-value equality.
pub fn equality_values(
    left: &CrowValue,
    right: &CrowValue,
    negate: bool,
) -> Result<CrowValue, String> {
    let is_scalar = |v: &Value| matches!(v, Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_));
    let broadcast = |items: &[CrowValue], scalar: &CrowValue| -> CrowValue {
        let results: Vec<CrowValue> = items.iter().map(|item| {
            let eq = !item.is_error() && values_equal(item, scalar);
            CrowValue::ok(Value::Bool(if negate { !eq } else { eq }))
        }).collect();
        CrowValue::ok(Value::Array(results))
    };
    match (&left.value, &right.value) {
        (Value::Array(items), r) if is_scalar(r) => Ok(broadcast(items, right)),
        (l, Value::Array(items)) if is_scalar(l) => Ok(broadcast(items, left)),
        // Tensor vs numeric scalar → element-wise mask (like gt/lt)
        (Value::NumArray(t), Value::Int(_) | Value::Float(_))
        | (Value::Int(_) | Value::Float(_), Value::NumArray(t)) => {
            let s = match (&left.value, &right.value) {
                (_, Value::Int(n)) | (Value::Int(n), _) => *n as f64,
                (_, Value::Float(f)) | (Value::Float(f), _) => *f,
                _ => unreachable!(),
            };
            let results: Vec<CrowValue> = t.to_vec().iter().map(|&v| {
                let eq = v == s;
                CrowValue::ok(Value::Bool(if negate { !eq } else { eq }))
            }).collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        _ => {
            let eq = values_equal(left, right);
            Ok(CrowValue::ok(Value::Bool(if negate { !eq } else { eq })))
        }
    }
}

/// and/or. Boolean arrays combine element-wise (mask semantics);
/// array vs scalar broadcasts; scalars short-circuit as before.
pub fn logical_values(
    left: &CrowValue,
    right: &CrowValue,
    op: impl Fn(bool, bool) -> bool,
) -> Result<CrowValue, String> {
    match (&left.value, &right.value) {
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!(
                    "boolean arrays have different lengths ({} vs {})",
                    a.len(), b.len()
                ));
            }
            let results: Vec<CrowValue> = a.iter().zip(b.iter()).map(|(x, y)| {
                CrowValue::ok(Value::Bool(op(is_truthy(x), is_truthy(y))))
            }).collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        (Value::Array(a), _) => {
            let s = is_truthy(right);
            let results: Vec<CrowValue> = a.iter().map(|x| {
                CrowValue::ok(Value::Bool(op(is_truthy(x), s)))
            }).collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        (_, Value::Array(b)) => {
            let s = is_truthy(left);
            let results: Vec<CrowValue> = b.iter().map(|x| {
                CrowValue::ok(Value::Bool(op(s, is_truthy(x))))
            }).collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        _ => Ok(CrowValue::ok(Value::Bool(op(
            is_truthy(left),
            is_truthy(right),
        )))),
    }
}

pub fn compare_values(
    l: &Value,
    r: &Value,
    cmp: impl Fn(f64, f64) -> bool,
) -> Result<CrowValue, String> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b as f64))))
        }
        (Value::Float(a), Value::Float(b)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
        }
        (Value::Int(a), Value::Float(b)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b))))
        }
        (Value::Float(a), Value::Int(b)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a, *b as f64))))
        }
        (Value::Dual(a, _), Value::Dual(b, _)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
        }
        (Value::Dual(a, _), Value::Float(b)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
        }
        (Value::Float(a), Value::Dual(b, _)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
        }
        (Value::Dual(a, _), Value::Int(b)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a, *b as f64))))
        }
        (Value::Int(a), Value::Dual(b, _)) => {
            Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b))))
        }
        // Vectorized: tensor compared to scalar → boolean mask array
        (Value::NumArray(t), Value::Int(b)) => {
            let s = *b as f64;
            let results: Vec<CrowValue> = t.to_vec().iter()
                .map(|&v| CrowValue::ok(Value::Bool(cmp(v, s))))
                .collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        (Value::NumArray(t), Value::Float(b)) => {
            let s = *b;
            let results: Vec<CrowValue> = t.to_vec().iter()
                .map(|&v| CrowValue::ok(Value::Bool(cmp(v, s))))
                .collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        (Value::Int(a), Value::NumArray(t)) => {
            let s = *a as f64;
            let results: Vec<CrowValue> = t.to_vec().iter()
                .map(|&v| CrowValue::ok(Value::Bool(cmp(s, v))))
                .collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        (Value::Float(a), Value::NumArray(t)) => {
            let s = *a;
            let results: Vec<CrowValue> = t.to_vec().iter()
                .map(|&v| CrowValue::ok(Value::Bool(cmp(s, v))))
                .collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        // Vectorized: Array compared to scalar → boolean array
        (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) => {
            let results: Vec<CrowValue> = items.iter().map(|item| {
                if item.is_error() {
                    CrowValue::ok(Value::Bool(false))
                } else {
                    let val = match (&item.value, scalar) {
                        (Value::Int(a), Value::Int(b)) => cmp(*a as f64, *b as f64),
                        (Value::Float(a), Value::Float(b)) => cmp(*a, *b),
                        (Value::Int(a), Value::Float(b)) => cmp(*a as f64, *b),
                        (Value::Float(a), Value::Int(b)) => cmp(*a, *b as f64),
                        _ => false,
                    };
                    CrowValue::ok(Value::Bool(val))
                }
            }).collect();
            Ok(CrowValue::ok(Value::Array(results)))
        }
        _ => Err("cannot compare these types".to_string()),
    }
}

/// Apply a scalar arithmetic operation element-wise to an Array of CrowValues.
/// Error elements are propagated (preserved as errors).
fn array_scalar_arithmetic(
    items: &[CrowValue],
    scalar: &Value,
    op: fn(f64, f64) -> f64,
) -> Result<CrowValue, String> {
    let scalar_f = match scalar {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        _ => return Err("non-numeric scalar in array arithmetic".to_string()),
    };
    let results: Vec<CrowValue> = items.iter().map(|item| {
        if item.is_error() {
            item.clone()
        } else {
            let val = match &item.value {
                Value::Int(a) => op(*a as f64, scalar_f),
                Value::Float(a) => op(*a, scalar_f),
                _ => return CrowValue::error("non-numeric value in array", "type"),
            };
            CrowValue::ok(Value::Float(val))
        }
    }).collect();
    Ok(CrowValue::ok(Value::Array(results)))
}

pub fn unaryop(
    op: &UnaryOp,
    val: &CrowValue,
) -> Result<CrowValue, String> {
    // soft error propagation
    if val.is_error() {
        return Ok(val.clone());
    }

    let v = &val.value;
    match op {
        UnaryOp::Neg => match v {
            Value::Int(n) => Ok(CrowValue::ok(Value::Int(-n))),
            Value::Float(n) => Ok(CrowValue::ok(Value::Float(-n))),
            Value::Dual(a, da) => Ok(CrowValue::ok(Value::Dual(-a, -da))),
            Value::NumArray(t) => Ok(CrowValue::ok(Value::NumArray(t.neg()))),
            _ => Err("cannot negate this type".to_string()),
        },
        UnaryOp::Not => match v {
            // Boolean arrays negate element-wise (mask semantics)
            Value::Array(items) => {
                let results: Vec<CrowValue> = items.iter().map(|x| {
                    CrowValue::ok(Value::Bool(!is_truthy(x)))
                }).collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            _ => Ok(CrowValue::ok(Value::Bool(!is_truthy(val)))),
        },
    }
}

pub fn is_truthy(val: &CrowValue) -> bool {
    if val.is_error() {
        return false;
    }
    match &val.value {
        Value::Bool(b) => *b,
        Value::Int(n) => *n != 0,
        Value::Float(n) => *n != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Array(l) => !l.is_empty(),
        Value::NumArray(t) => !t.is_empty(),
        Value::Done => false,
        _ => true,
    }
}

/// Collect view names and event-handler names (bare fn refs inside view
/// bodies + page[] registration args) — these frames keep reactive state
/// visible under `use ui`.
/// Coerce an all-numeric Array (how frame columns arrive) into a tensor
/// value for element-wise operators; other values pass through unchanged.
fn coerce_numeric_array(v: &Value) -> Value {
    if let Value::Array(items) = v {
        if !items.is_empty() {
            let mut data = Vec::with_capacity(items.len());
            for item in items {
                match &item.value {
                    Value::Int(n) => data.push(*n as f64),
                    Value::Float(f) => data.push(*f),
                    _ => return v.clone(),
                }
            }
            let n = data.len();
            return Value::NumArray(CrowTensor::from_vec(data, vec![n]));
        }
    }
    v.clone()
}

pub fn values_equal(a: &CrowValue, b: &CrowValue) -> bool {
    match (&a.value, &b.value) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
        (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
        (Value::Dual(x, _), Value::Dual(y, _)) => x == y,
        (Value::Dual(x, _), Value::Float(y)) => x == y,
        (Value::Float(x), Value::Dual(y, _)) => x == y,
        (Value::Dual(x, _), Value::Int(y)) => *x == (*y as f64),
        (Value::Int(x), Value::Dual(y, _)) => (*x as f64) == *y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::NumArray(x), Value::NumArray(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len()
                && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b))
        }
        // a list of numbers and a tensor hold numbers alike: equal when the
        // numbers and the shape are (a vector's numbers, a matrix's rows)
        (Value::Array(x), Value::NumArray(t)) | (Value::NumArray(t), Value::Array(x)) => {
            if t.ndim() == 0 {
                return false;
            }
            let ys = iter_items(&CrowValue::ok(Value::NumArray(t.clone()))).unwrap_or_default();
            x.len() == ys.len() && x.iter().zip(ys.iter()).all(|(a, b)| values_equal(a, b))
        }
        _ => false,
    }
}

/// A value indexed, or a struct constructed: `xs[i]`, `t[i]` (a row of a
/// matrix, a gathered tensor), `s[i]` (a character), `m['k']`,
/// `Point[1, 2]`.
pub fn index(callee: &CrowValue, arg_vals: Vec<CrowValue>) -> Result<CrowValue, String> {
    match &callee.value {
    Value::Array(items) => {
        // array[index] - indexing
        if arg_vals.len() != 1 {
            return Err("array indexing requires exactly one argument".to_string());
        }
        Ok(items[list_position(&arg_vals[0], items.len())?].clone())
    }
    Value::NumArray(tensor) => {
        // NumArray[index] - indexing
        if arg_vals.len() != 1 {
            return Err("array indexing requires exactly one argument".to_string());
        }
        let index_val = arg_vals[0].unwrap_or_propagate()?;
        match index_val {
            Value::Int(idx) => {
                let idx = *idx;
                // Rank >= 2: the element at [i] is a row (rank-1-lower tensor)
                if tensor.ndim() >= 2 {
                    let rows = tensor.shape()[0] as i64;
                    let resolved = if idx < 0 { rows + idx } else { idx };
                    if resolved < 0 || resolved >= rows {
                        return Err(format!(
                            "index {} out of bounds for tensor with {} rows",
                            idx, rows
                        ));
                    }
                    return Ok(CrowValue::ok(Value::NumArray(tensor.index0(resolved as usize))));
                }
                let len = tensor.len() as i64;
                let resolved = if idx < 0 { len + idx } else { idx };
                if resolved < 0 || resolved >= len {
                    return Err(format!(
                        "index {} out of bounds for array of length {}",
                        idx, tensor.len()
                    ));
                }
                let v = tensor.get_flat(resolved as usize);
                // Return as int if it's a whole number
                if v == (v as i64) as f64 && v.is_finite() {
                    Ok(CrowValue::ok(Value::Int(v as i64)))
                } else {
                    Ok(CrowValue::ok(Value::Float(v)))
                }
            }
            Value::NumArray(idx_tensor) => {
                // Advanced indexing: gather by index array
                Ok(CrowValue::ok(Value::NumArray(tensor.gather(idx_tensor))))
            }
            _ => Err("array index must be an integer or tensor".to_string()),
        }
    }
    Value::Str(s) => {
        // string[index] - character access
        if arg_vals.len() != 1 {
            return Err("string indexing requires exactly one argument".to_string());
        }
        let index_val = arg_vals[0].unwrap_or_propagate()?;
        match index_val {
            Value::Int(idx) => {
                let idx = *idx;
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let resolved = if idx < 0 { len + idx } else { idx };
                if resolved < 0 || resolved >= len {
                    return Err(format!(
                        "index {} out of bounds for string of length {}",
                        idx,
                        chars.len()
                    ));
                }
                Ok(CrowValue::ok(Value::Str(chars[resolved as usize].to_string())))
            }
            _ => Err("string index must be an integer".to_string()),
        }
    }
    Value::Map(map) => {
        // map['key'] - access
        if arg_vals.len() != 1 {
            return Err("map access requires exactly one argument".to_string());
        }
        let key_val = arg_vals[0].unwrap_or_propagate()?;
        let key_str = match key_val {
            Value::Str(s) => s.clone(),
            other => format!("{}", other),
        };
        match map.get(&key_str) {
            Some(val) => Ok(val.clone()),
            None => Err(format!("key '{}' not found in map", key_str)),
        }
    }
        _ => Err("not indexable".to_string()),
    }
}

/// `.ok` of a frame column: each row's validity, a missing cell false — the
/// mask `filter[df, df.col.ok]` keeps the complete rows with. A column that
/// failed to read is its error, as any field of an error value is.
pub fn valid(obj: CrowValue) -> Result<CrowValue, String> {
    if obj.is_error() {
        return Ok(obj);
    }
    match &obj.value {
        Value::Array(items) => Ok(CrowValue::ok(Value::Array(
            items.iter().map(|item| CrowValue::ok(Value::Bool(item.ok))).collect(),
        ))),
        _ => Ok(CrowValue::ok(Value::Bool(obj.ok))),
    }
}

/// A field of a value: the status fields on anything, `len`, the datetime
/// parts of a text, a tensor's shape, a frame's properties and columns, a
/// struct's or a map's member, a module's binding.
/// A frame's cell: a missing one is an error of kind `missing` (the
/// column's `.ok` reads it false, a read by index gets its placeholder).
pub fn frame_cell(frame: &crow_runtime::frame::CrowFrame, col: usize, row: usize) -> CrowValue {
    if !frame.validity[col][row] {
        let mut missing = CrowValue::error_with_kind("nan", "missing", "");
        missing.trace.clear();
        return missing;
    }
    CrowValue::ok(match frame.data[col].as_ref() {
        crow_runtime::frame::Column::Int(v) => Value::Int(v[row]),
        crow_runtime::frame::Column::Float(v) => Value::Float(v[row]),
        crow_runtime::frame::Column::Str(v) => Value::Str(v[row].clone()),
        crow_runtime::frame::Column::Bool(v) => Value::Bool(v[row]),
        crow_runtime::frame::Column::DateTime(millis, offsets) => {
            Value::Str(crow_runtime::frame::format_datetime(millis[row], offsets[row]))
        }
    })
}

/// Where `xs[i]` reads in a list of `len` items: a negative index counts
/// from the end.
pub fn list_position(index: &CrowValue, len: usize) -> Result<usize, String> {
    match index.unwrap_or_propagate()? {
        Value::Int(idx) => {
            let idx = *idx;
            let n = len as i64;
            let resolved = if idx < 0 { n + idx } else { idx };
            if resolved < 0 || resolved >= n {
                return Err(format!("index {} out of bounds for array of length {}", idx, len));
            }
            Ok(resolved as usize)
        }
        _ => Err("array index must be an integer".to_string()),
    }
}

pub fn field(obj: CrowValue, field: &str) -> Result<CrowValue, String> {

    // special fields: .ok, .reason, .kind, .trace — always work, even on errors.
    // `.ok` is the value's own status: a list that did not fail is ok,
    // whatever it holds (a frame column's rows are `valid`).
    match field {
        "ok" => {
            return Ok(CrowValue::ok(Value::Bool(obj.ok)));
        }
        "reason" => {
            return Ok(CrowValue::ok(Value::Str(obj.reason.clone())));
        }
        "kind" => {
            return Ok(CrowValue::ok(Value::Str(obj.kind.clone())));
        }
        "trace" => {
            let trace_list: Vec<CrowValue> = obj.trace.iter()
                .map(|s| CrowValue::ok(Value::Str(s.clone())))
                .collect();
            return Ok(CrowValue::ok(Value::Array(trace_list)));
        }
        _ => {}
    }

    // soft error propagation for other field access
    if obj.is_error() {
        return Ok(obj);
    }

    // .length property for lists and strings
    if field == "len" {
        match &obj.value {
            Value::Array(items) => {
                return Ok(CrowValue::ok(Value::Int(items.len() as i64)));
            }
            Value::NumArray(t) => {
                return Ok(CrowValue::ok(Value::Int(t.len() as i64)));
            }
            Value::Str(s) => {
                return Ok(CrowValue::ok(Value::Int(s.len() as i64)));
            }
            _ => return Err("only arrays and strings have .len".to_string()),
        }
    }

    // DateTime properties on strings: .year, .month, .day, .hour, .minute, .second, .offset
    if let Value::Str(s) = &obj.value {
        match field {
            "year" | "month" | "day" | "hour" | "minute" | "second" | "offset" => {
                if let Some(val) = crow_runtime::frame::datetime_field(s, field) {
                    return Ok(CrowValue::ok(Value::Int(val)));
                }
                return Err(format!("'{}' is not a valid datetime string", s));
            }
            _ => {}
        }
    }

    // NumArray properties: .shape, .dtype, .device, .grad
    if let Value::NumArray(t) = &obj.value {
        match field {
            "shape" => {
                let shape_vals: Vec<CrowValue> = t.shape().iter()
                    .map(|&s| CrowValue::ok(Value::Int(s as i64)))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(shape_vals)));
            }
            "dtype" => {
                return Ok(CrowValue::ok(Value::Str("float".to_string())));
            }
            "device" => {
                let device_str = if t.is_gpu() { "gpu" } else { "cpu" };
                return Ok(CrowValue::ok(Value::Str(device_str.to_string())));
            }
            "grad" => {
                // For now, gradient is not stored on the tensor itself
                return Ok(CrowValue::ok(Value::Str("none".to_string())));
            }
            _ => return Err(format!("numarray has no property '{}'", field)),
        }
    }

    // Frame properties: .shape, .columns, .dtypes, .column_name
    if let Value::Frame(frame) = &obj.value {
        match field {
            "shape" => {
                let shape = vec![
                    CrowValue::ok(Value::Int(frame.nrows() as i64)),
                    CrowValue::ok(Value::Int(frame.ncols() as i64)),
                ];
                return Ok(CrowValue::ok(Value::Array(shape)));
            }
            "columns" => {
                let cols: Vec<CrowValue> = frame.columns.iter()
                    .map(|c| CrowValue::ok(Value::Str(c.clone())))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(cols)));
            }
            "dtypes" => {
                let dtypes: Vec<CrowValue> = frame.dtypes().iter()
                    .map(|d| CrowValue::ok(Value::Str(d.to_string())))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(dtypes)));
            }
            _ => {
                // Try column access by name: df.age returns the column as an array
                if let Some(idx) = frame.col_index(field) {
                    let vals: Vec<CrowValue> = (0..frame.nrows()).map(|i| frame_cell(frame, idx, i)).collect();
                    return Ok(CrowValue::ok(Value::Array(vals)));
                }
                return Err(format!("frame has no property or column '{}'", field));
            }
        }
    }

    // struct and module field access
    match &obj.value {
        Value::StructInstance(si) => {
            match si.fields.get(field) {
                Some(val) => Ok(val.clone()),
                None => Err(format!(
                    "struct '{}' has no field '{}'",
                    si.name, field
                )),
            }
        }
        Value::Map(m) => {
            match m.get(field) {
                Some(val) => Ok(val.clone()),
                None => Err(format!("map has no key '{}'", field)),
            }
        }
        _ => Err(format!("cannot access field '{}' on this value", field)),
    }
}

/// An array literal's value: all numbers make a tensor, all tensors of one
/// shape make a shaped tensor (spec: unified arrays), anything else a list.
pub fn array_literal(vals: Vec<CrowValue>) -> Result<CrowValue, String> {
    let all_numeric = !vals.is_empty() && vals.iter().all(|v| v.ok && matches!(v.value, Value::Int(_) | Value::Float(_)));
    if all_numeric {
        let data: Vec<f64> = vals.iter().map(|v| match &v.value {
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            _ => unreachable!(),
        }).collect();
        let len = data.len();
        return Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))));
    }
    let all_tensors = !vals.is_empty() && vals.iter().all(|v| v.ok && matches!(v.value, Value::NumArray(_)));
    if all_tensors {
        let tensors: Vec<CrowTensor> = vals.iter().map(|v| match &v.value {
            Value::NumArray(t) => t.clone(),
            _ => unreachable!(),
        }).collect();
        let stacked = CrowTensor::from_elements(&tensors)?;
        return Ok(CrowValue::ok(Value::NumArray(stacked)));
    }
    Ok(CrowValue::ok(Value::Array(vals)))
}

/// What a collecting loop yields: iterations that produced nothing are
/// skipped, all numbers make a tensor, anything else a list.
pub fn collect_loop(results: Vec<CrowValue>) -> CrowValue {
    // an iteration that yielded nothing is skipped; one that yielded an
    // error keeps it as its item (an error's value is `Done` too)
    let filtered: Vec<CrowValue> = results.into_iter().filter(|v| !(v.ok && matches!(v.value, Value::Done))).collect();
    let all_numeric = !filtered.is_empty() && filtered.iter().all(|v| v.ok && matches!(v.value, Value::Int(_) | Value::Float(_)));
    if all_numeric {
        let data: Vec<f64> = filtered.iter().map(|v| match &v.value {
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            _ => unreachable!(),
        }).collect();
        let len = data.len();
        return CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len])));
    }
    CrowValue::ok(Value::Array(filtered))
}

/// What a `for` walks: a list's items, a matrix's rows, a vector's numbers
/// (whole ones as ints).
pub fn iter_items(iter_val: &CrowValue) -> Result<Vec<CrowValue>, String> {
    match &iter_val.value {
        Value::Array(items) => Ok(items.clone()),
        Value::NumArray(tensor) => {
            if tensor.ndim() >= 2 {
                Ok(tensor.rows().into_iter().map(|t| CrowValue::ok(Value::NumArray(t))).collect())
            } else {
                Ok(tensor.to_vec().into_iter().map(|v| {
                    if v == (v as i64) as f64 && v.is_finite() {
                        CrowValue::ok(Value::Int(v as i64))
                    } else {
                        CrowValue::ok(Value::Float(v))
                    }
                }).collect())
            }
        }
        _ => Err("for loop requires an array".to_string()),
    }
}

/// A view element rendered to markup from its evaluated arguments: an
/// attribute map (scalar entries are attributes, event keys are wiring),
/// markup children that pass through, text children escaped, a list that
/// contributes each item, a frame that is a table's rows; layout
/// attributes go through the kernel's table (crow_runtime::layout).
/// `widget[name, props = value, on = map[event = handler]]`: the element the
/// program's own module draws — one shape for both modes.
fn render_widget(vals: &[CrowValue]) -> String {
    let mut module = String::new();
    let mut props = serde_json::Value::Null;
    let mut on: Vec<(String, String)> = Vec::new();
    for v in vals {
        match &v.value {
            Value::Map(m) => {
                for (k, val) in m {
                    match (k.as_str(), &val.value) {
                        ("props", _) => props = crate::builtins::crow_to_json("", val).unwrap_or(serde_json::Value::Null),
                        ("on", Value::Map(evs)) => {
                            for (event, h) in evs {
                                if let Value::Fn(f) = &h.value {
                                    on.push((event.clone(), f.name.clone()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Value::Str(s) => module = s.clone(),
            _ => {}
        }
    }
    let pairs: Vec<(&str, &str)> = on.iter().map(|(e, h)| (e.as_str(), h.as_str())).collect();
    crow_web::view::CrowVNode::widget(&module, &props, &pairs).to_html()
}

pub fn render_element(name: &str, vals: &[CrowValue]) -> Result<String, String> {
    if name == "widget" {
        return Ok(render_widget(vals));
    }
    // markup a view, an element or `doc` made is placed as markup; every
    // other string is text, escaped, whatever it holds
    fn splice(inner: &mut String, items: &[CrowValue]) {
        for it in items {
            match &it.value {
                Value::Str(s) if it.markup => inner.push_str(s),
                Value::Array(nested) => splice(inner, nested),
                other => inner.push_str(&crate::harness::html_escape_text(&format!("{}", other))),
            }
        }
    }
    let mut inner = String::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let (mut table_click, mut table_key, mut table_mark): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    let mut table_delta: Vec<String> = Vec::new();
    let mut table_actions: Vec<(String, String)> = Vec::new();
    let mut table_hide: Vec<String> = Vec::new();
    let mut table_confirm: Vec<(String, String)> = Vec::new();
    // table[frame, page = 50]: the page size, and — from the session the
    // interpreter renders for — which paged table this is and its page
    let (mut table_page, mut page_index, mut page_at): (Option<i64>, usize, usize) = (None, 0, 0);
    // the wiring, as the compiled page writes it: the handler a click, an
    // input or a form calls, and the arguments a click froze
    let mut events: Vec<(String, String)> = Vec::new();
    let mut confirm: Option<String> = None;
    for v in vals {
        if v.is_error() {
            // a view renders what it is given: a failed value shows its reason
            inner.push_str(&format!("error: {}", v.reason));
            continue;
        }
        match &v.value {
            Value::Map(m) => {
                for (k, val) in m.iter() {
                    if name == "table" {
                        match (k.as_str(), &val.value) {
                            ("click", Value::Fn(f)) => {
                                table_click = Some(f.name.clone());
                                continue;
                            }
                            ("key", Value::Str(s)) => {
                                table_key = Some(s.clone());
                                continue;
                            }
                            ("mark", Value::Str(s)) => {
                                table_mark = Some(s.clone());
                                continue;
                            }
                            ("delta", Value::Array(items)) => {
                                table_delta = items.iter().map(|i| format!("{}", i.value)).collect();
                                continue;
                            }
                            ("actions", Value::Map(acts)) => {
                                table_actions = acts
                                    .iter()
                                    .filter_map(|(label, h)| if let Value::Fn(hf) = &h.value { Some((label.clone(), hf.name.clone())) } else { None })
                                    .collect();
                                continue;
                            }
                            ("hide", Value::Array(items)) => {
                                table_hide = items.iter().map(|i| format!("{}", i.value)).collect();
                                continue;
                            }
                            ("confirm", Value::Map(texts)) => {
                                table_confirm = texts.iter().map(|(label, t)| (label.clone(), format!("{}", t.value))).collect();
                                continue;
                            }
                            ("page", Value::Int(n)) => {
                                table_page = Some(*n);
                                continue;
                            }
                            ("__page_index", Value::Int(n)) => {
                                page_index = (*n).max(0) as usize;
                                continue;
                            }
                            ("__page_at", Value::Int(n)) => {
                                page_at = (*n).max(0) as usize;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    if k == "confirm" {
                        confirm = Some(format!("{}", val.value));
                        continue;
                    }
                    if matches!(k.as_str(), "click" | "input" | "submit" | "change") {
                        let attr = format!("data-crow-{}", k);
                        match &val.value {
                            Value::Fn(f) => events.push((attr, f.name.clone())),
                            Value::Array(items) if matches!(items.first().map(|i| &i.value), Some(Value::Fn(_))) => {
                                if let Value::Fn(f) = &items[0].value {
                                    let frozen: Vec<serde_json::Value> = items[1..]
                                        .iter()
                                        .map(|i| crate::builtins::crow_to_json("", i).unwrap_or(serde_json::Value::Null))
                                        .collect();
                                    events.push((attr, f.name.clone()));
                                    events.push(("data-crow-args".to_string(), serde_json::Value::Array(frozen).to_string()));
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if k == "seek" {
                        continue;
                    }
                    let s = match &val.value {
                        Value::Str(s) => s.clone(),
                        Value::Int(i) => i.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    attrs.push((k.clone(), s));
                }
            }
            Value::Str(s) if v.markup => inner.push_str(s),
            Value::Frame(f) if name == "table" => inner.push_str(&crow_runtime::layout::frame_table_html(
                f,
                table_click.as_deref(),
                table_key.as_deref(),
                table_mark.as_deref(),
                &table_delta,
                &table_actions,
                &table_hide,
                &table_confirm,
                table_page.map(|size| crow_runtime::layout::Paging { size: size.max(1) as usize, index: page_index, at: page_at }),
            )),
            Value::Array(items) => splice(&mut inner, items),
            other => inner.push_str(&crate::harness::html_escape_text(&format!("{}", other))),
        }
    }
    if name == "nav" {
        if let Some((_, active)) = attrs.iter().find(|(k, _)| k == "active") {
            inner = crow_runtime::layout::nav_active_html(&inner, active);
        }
    }
    let rendered = crow_runtime::layout::render(name, &attrs);
    if let Some(e) = rendered.errors.first() {
        return Err(format!("{}: {}", name, e));
    }
    let mut out_attrs: Vec<(String, String)> = rendered.passthrough.clone();
    if !rendered.class.is_empty() {
        out_attrs.push(("class".to_string(), rendered.class.clone()));
    }
    if !rendered.style.is_empty() {
        out_attrs.push(("style".to_string(), rendered.style.clone()));
    }
    out_attrs.extend(rendered.data.iter().cloned());
    out_attrs.extend(crow_runtime::layout::confirm_wiring(events, confirm.as_deref()));
    out_attrs.sort();
    let attr_str: String = out_attrs.iter().map(|(k, v)| format!(" {}=\"{}\"", k, crate::harness::html_escape_attr(v))).collect();
    Ok(format!("<{0}{2}>{1}</{0}>", rendered.tag, inner, attr_str))
}

/// A bound input rendered to markup: `field`, `slider`, `checkbox`,
/// `toggle`, `radio`, `tabs`, `select`, given the variable's name (`key`)
/// and the evaluated arguments.
pub fn render_input(name: &str, key: &str, vals: &[CrowValue]) -> Result<String, String> {
    use crow_web::view::CrowVNode;
    let s = |v: &CrowValue| format!("{}", v.value);
    let num = |v: &CrowValue| match &v.value {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    };
    let options = |v: &CrowValue| -> Result<Vec<String>, String> {
        match &v.value {
            Value::Array(items) => Ok(items.iter().map(|i| format!("{}", i.value)).collect()),
            other => Err(format!("{}[] takes a list of options first, got {}", name, other.type_name())),
        }
    };
    let node = match name {
        "field" => {
            let kind = match &vals[2].value {
                Value::Str(k) if !k.is_empty() => k.clone(),
                _ => match &vals[1].value {
                    Value::Int(_) | Value::Float(_) => "number".to_string(),
                    _ => "text".to_string(),
                },
            };
            // a file field shows the name of the file picked
            let current = match (&vals[1].value, kind.as_str()) {
                (Value::Map(m), "file") => m.get("name").map(|v| format!("{}", v.value)).unwrap_or_default(),
                _ => s(&vals[1]),
            };
            CrowVNode::field_typed(&s(&vals[0]), key, &current, &kind, num(&vals[3]) as i64)
        }
        "slider" => CrowVNode::slider(&s(&vals[0]), key, &s(&vals[1]), num(&vals[2]), num(&vals[3]), num(&vals[4])),
        "checkbox" | "toggle" => {
            let on = match &vals[1].value {
                Value::Bool(b) => *b,
                Value::Int(n) => *n != 0,
                Value::Str(t) => t == "true",
                _ => false,
            };
            CrowVNode::check_input(&s(&vals[0]), key, on, name == "toggle")
        }
        "radio" | "tabs" => CrowVNode::choice_input(&options(&vals[0])?, key, &s(&vals[1]), name == "tabs"),
        _ => CrowVNode::select_input(&options(&vals[0])?, key, &s(&vals[1])),
    };
    Ok(node.to_html())
}
