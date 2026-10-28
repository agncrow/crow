//! The operators, indexing and field access, in one place:
//! what `+` means between an int and a tensor, what `xs[-1]` is, what
//! `df.price` reads. The interpreter (`ir_interp.rs`, ir-design.md §5)
//! calls nothing else for them; the compiled mode's runtime is built from
//! the same kernels.

use crate::ast::{BinOp, UnaryOp};
use crate::value::*;
use crow_runtime::mask::Cmp;
use crow_runtime::stops;
use crow_runtime::tensor::CrowTensor;

pub fn binop(
    left: &CrowValue,
    op: &BinOp,
    right: &CrowValue,
) -> Result<CrowValue, String> {
    // soft error propagation: if either operand is an error, propagate it
    if left.is_error() {
        return Ok(left.clone());
    }
    if right.is_error() {
        return Ok(right.clone());
    }

    let l = &left.value;
    let r = &right.value;

    // two whole numbers: the exact answer, however large it grows (spec,
    // Numbers That Keep Every Digit) — past 64 bits a `Big`, back in them an
    // `Int`; a zero divisor and a negative power still stop
    if let (Some(a), Some(b)) = (Value::int_of(l), Value::int_of(r)) {
        let n = match op {
            BinOp::Add => a.add(&b),
            BinOp::Sub => a.sub(&b),
            BinOp::Mul => a.mul(&b),
            BinOp::Div => a.div(&b)?,
            BinOp::Mod => a.rem(&b)?,
            BinOp::Pow => a.pow(&b)?,
            BinOp::Eq => return Ok(CrowValue::ok(Value::Bool(a == b))),
            BinOp::Ne => return Ok(CrowValue::ok(Value::Bool(a != b))),
            BinOp::Lt => return Ok(CrowValue::ok(Value::Bool(a < b))),
            BinOp::Gt => return Ok(CrowValue::ok(Value::Bool(a > b))),
            BinOp::Lte => return Ok(CrowValue::ok(Value::Bool(a <= b))),
            BinOp::Gte => return Ok(CrowValue::ok(Value::Bool(a >= b))),
            _ => return binop_rest(left, op, right),
        };
        return Ok(CrowValue::ok(Value::of_int(n)));
    }
    // exact numbers: an int meeting a decimal is the decimal it is, an int or
    // a decimal meeting a fraction the fraction it is (the lowering converts
    // them the same way; a float beside one it refused, a literal it read)
    if matches!(l, Value::Decimal(_) | Value::Fraction(_)) || matches!(r, Value::Decimal(_) | Value::Fraction(_)) {
        return exact_binop(left, op, right);
    }
    // a whole number past 64 bits beside anything else is read as the
    // nearest float there (a tensor's numbers are floats)
    if matches!(l, Value::Big(_)) || matches!(r, Value::Big(_)) {
        let as_float = |v: &CrowValue| match &v.value {
            Value::Big(b) => CrowValue::ok(Value::Float(b.to_f64().value())),
            _ => v.clone(),
        };
        return binop(&as_float(left), op, &as_float(right));
    }
    binop_rest(left, op, right)
}

/// `binop` where a decimal or a fraction is an operand: both read as the
/// kind the operator computes in, then its exact operation — a quotient's
/// digits and a zero divisor as the runtime says (crow_runtime::decimal,
/// crow_runtime::fraction).
fn exact_binop(left: &CrowValue, op: &BinOp, right: &CrowValue) -> Result<CrowValue, String> {
    use crow_runtime::decimal::Dec;
    use crow_runtime::fraction::Frac;
    let (l, r) = (&left.value, &right.value);
    let fraction = matches!(l, Value::Fraction(_)) || matches!(r, Value::Fraction(_));
    let cmp = |o: std::cmp::Ordering| -> Option<bool> {
        Some(match op {
            BinOp::Eq => o.is_eq(),
            BinOp::Ne => o.is_ne(),
            BinOp::Lt => o.is_lt(),
            BinOp::Gt => o.is_gt(),
            BinOp::Lte => o.is_le(),
            BinOp::Gte => o.is_ge(),
            _ => return None,
        })
    };
    // a power's exponent is an int
    if matches!(op, BinOp::Pow) {
        if let Some(e) = Value::int_of(r) {
            return Ok(match l {
                Value::Fraction(f) => crate::table::write_fraction(f.pow(&e)?),
                _ => crate::table::write_decimal(crate::table::read_decimal(left)?.pow(&e)?),
            });
        }
        return Err(format!("{} ^ {}: an exact number's power is by an int", l, r));
    }
    if fraction {
        let (a, b): (Frac, Frac) = (crate::table::read_fraction(left)?, crate::table::read_fraction(right)?);
        if let Some(t) = cmp(a.cmp(&b)) {
            return Ok(CrowValue::ok(Value::Bool(t)));
        }
        let q = match op {
            BinOp::Add => a.add(&b),
            BinOp::Sub => a.sub(&b),
            BinOp::Mul => a.mul(&b),
            BinOp::Div => a.div(&b)?,
            _ => return Err(format!("cannot use '{}' with fractions", format!("{:?}", op).to_lowercase())),
        };
        return Ok(crate::table::write_fraction(q));
    }
    let (a, b): (Dec, Dec) = (crate::table::read_decimal(left)?, crate::table::read_decimal(right)?);
    if let Some(t) = cmp(a.cmp(&b)) {
        return Ok(CrowValue::ok(Value::Bool(t)));
    }
    let d = match op {
        BinOp::Add => a.add(&b),
        BinOp::Sub => a.sub(&b),
        BinOp::Mul => a.mul(&b),
        BinOp::Div => a.div(&b)?,
        _ => return Err(format!("cannot use '{}' with decimals", format!("{:?}", op).to_lowercase())),
    };
    Ok(crate::table::write_decimal(d))
}

/// A value handed where 64-bit ints are taken (`Convert::ToSmall`): an int
/// past 64 bits is refused, naming what took it (spec, Numbers That Keep
/// Every Digit), and so is a list or a map holding one; anything else is
/// itself.
pub fn small_value(v: CrowValue, who: crate::ir::Who) -> Result<CrowValue, String> {
    small_check(&v.value, who)?;
    Ok(v)
}

/// Whether a value holds an int past 64 bits, refused as `small_value`
/// refuses it.
pub fn small_check(v: &Value, who: crate::ir::Who) -> Result<(), String> {
    match v {
        Value::Big(b) => Err(crow_runtime::int::past_64_bits(&who.text(), &crow_runtime::int::Int::Big(b.clone()).brief())),
        Value::Array(items) => items.iter().try_for_each(|i| small_check(&i.value, who)),
        Value::Map(m) => m.values().try_for_each(|i| small_check(&i.value, who)),
        Value::IntMap(m) => m.values().try_for_each(|i| small_check(&i.value, who)),
        _ => Ok(()),
    }
}

/// `binop` of operands that are not both whole numbers.
fn binop_rest(left: &CrowValue, op: &BinOp, right: &CrowValue) -> Result<CrowValue, String> {
    let l = &left.value;
    let r = &right.value;

    // an int meets a float in arithmetic only through the lowering's
    // conversion (spec §1), and the verifier holds the IR to it: a pair here
    // is a bug in Crow, said as one. (The arms that had computed around it
    // hid a lowering slip the compiled program would have refused.)
    if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow)
        && matches!((l, r), (Value::Int(_), Value::Float(_)) | (Value::Float(_), Value::Int(_)) | (Value::Int(_), Value::Dual(..)) | (Value::Dual(..), Value::Int(_)))
    {
        return Err(format!("internal error: an int met a float in '{}' without a conversion — a bug in Crow, not in this program", format!("{:?}", op).to_lowercase()));
    }

    // a value a function grad differentiates computes with: the autograd
    // kernel's own operation, recorded on the tape
    if matches!(l, Value::Traced(_)) || matches!(r, Value::Traced(_)) {
        return crate::autograd_bridge::binop(op, l, r);
    }

    match op {
        BinOp::Add => match (l, r) {
            // Dual number rules: dual(a,a') + dual(b,b') = dual(a+b, a'+b')
            (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a + b, da + db))),
            (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a + b, *da))),
            (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a + b, *db))),
            (Value::Int(a), Value::Int(b)) => stops::add(*a, *b).map(|n| CrowValue::ok(Value::Int(n))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a + b))),
            // a moment and seconds: the moment on
            (Value::Moment(t), Value::Int(s)) | (Value::Int(s), Value::Moment(t)) => crow_runtime::moments::plus(t, *s as f64).map(|m| CrowValue::ok(Value::Moment(m))),
            (Value::Moment(t), Value::Float(s)) | (Value::Float(s), Value::Moment(t)) => crow_runtime::moments::plus(t, *s).map(|m| CrowValue::ok(Value::Moment(m))),
            (Value::NumArray(a), Value::NumArray(b)) => a.try_elementwise(b, "+").map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.add_scalar(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.add_scalar(*b))))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.add_scalar(*a as f64))))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.add_scalar(*a))))),
            // sparse + sparse
            (Value::Sparse(a), Value::Sparse(b)) => Ok(CrowValue::ok(Value::Sparse(Box::new(a.add(b))))),
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
            (Value::Int(a), Value::Int(b)) => stops::sub(*a, *b).map(|n| CrowValue::ok(Value::Int(n))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a - b))),
            // a moment back by seconds, and the seconds between two moments
            (Value::Moment(t), Value::Int(s)) => crow_runtime::moments::less(t, *s as f64).map(|m| CrowValue::ok(Value::Moment(m))),
            (Value::Moment(t), Value::Float(s)) => crow_runtime::moments::less(t, *s).map(|m| CrowValue::ok(Value::Moment(m))),
            (Value::Moment(a), Value::Moment(b)) => Ok(CrowValue::ok(Value::Float(*a - *b))),
            (Value::NumArray(a), Value::NumArray(b)) => a.try_elementwise(b, "-").map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.sub_scalar(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.sub_scalar(*b))))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.neg().add_scalar(*a as f64))))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.neg().add_scalar(*a))))),
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
            (Value::Int(a), Value::Int(b)) => stops::mul(*a, *b).map(|n| CrowValue::ok(Value::Int(n))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a * b))),
            // tensor * tensor → matmul; tensor * scalar → scale
            (Value::NumArray(a), Value::NumArray(b)) => a.try_matmul(b).map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.mul_scalar(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.mul_scalar(*b))))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.mul_scalar(*a as f64))))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.mul_scalar(*a))))),
            // sparse * dense → matmul
            (Value::Sparse(s), Value::NumArray(t)) => s.matmul_dense(t).map(|m| CrowValue::ok(Value::NumArray(Box::new(m)))),
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
            // an int divided by zero stops the program, as in the compiled
            // one: the quotient is typed int, and no int says it failed
            (Value::Int(a), Value::Int(b)) => stops::div(*a, *b).map(|n| CrowValue::ok(Value::Int(n))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a / b))),
            // tensor / tensor → error (use ./ for element-wise)
            (Value::NumArray(_), Value::NumArray(_)) => Err("use ./ for element-wise tensor division".to_string()),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.div_scalar(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.div_scalar(*b))))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.scalar_div(*a as f64))))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.scalar_div(*a))))),
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
            (Value::Int(a), Value::Int(b)) => stops::rem(*a, *b).map(|n| CrowValue::ok(Value::Int(n))),
            _ => Err("modulo only works on integers".to_string()),
        },
        BinOp::Pow => match (l, r) {
            // Dual: d/dx(x^n) = n * x^(n-1) * x'
            (Value::Dual(a, da), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Dual(a.powf(*b), da * b * a.powf(b - 1.0))))
            }
            (Value::Dual(a, da), Value::Dual(b, db)) => {
                let val = a.powf(*b);
                let deriv = val * (db * a.ln() + b * da / a);
                Ok(CrowValue::ok(Value::Dual(val, deriv)))
            }
            // int ^ int is an int: exact, or the program stops
            (Value::Int(a), Value::Int(b)) => stops::pow(*a, *b).map(|n| CrowValue::ok(Value::Int(n))),
            (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b)))),
            _ => Err("^ requires numbers".to_string()),
        },
        BinOp::ElemMul => match (&coerce_numeric_array(l), &coerce_numeric_array(r)) {
            (Value::NumArray(a), Value::NumArray(b)) => a.try_elementwise(b, ".*").map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.mul_scalar(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.mul_scalar(*b))))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.mul_scalar(*a as f64))))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.mul_scalar(*a))))),
            _ => Err(".* requires tensors or tensor and scalar".to_string()),
        },
        BinOp::ElemDiv => match (&coerce_numeric_array(l), &coerce_numeric_array(r)) {
            (Value::NumArray(a), Value::NumArray(b)) => a.try_elementwise(b, "./").map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))),
            (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.div_scalar(*b as f64))))),
            (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(a.div_scalar(*b))))),
            (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.scalar_div(*a as f64))))),
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.scalar_div(*a))))),
            _ => Err("./ requires tensors or tensor and scalar".to_string()),
        },
        BinOp::ElemPow => match (&coerce_numeric_array(l), &coerce_numeric_array(r)) {
            (Value::NumArray(a), Value::NumArray(b)) => a.try_elementwise(b, ".^").map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))),
            (Value::NumArray(a), Value::Int(b)) => {
                let s = *b as f64;
                Ok(CrowValue::ok(Value::NumArray(Box::new(a.map(|x| x.powf(s))))))
            }
            (Value::NumArray(a), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::NumArray(Box::new(a.map(|x| x.powf(*b))))))
            }
            // a number raised to each of a tensor's numbers, as `.*` and `./`
            // take a number on either side (the compiled program had not built)
            (Value::Int(a), Value::NumArray(b)) => {
                let s = *a as f64;
                Ok(CrowValue::ok(Value::NumArray(Box::new(b.map(|x| s.powf(x))))))
            }
            (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(Box::new(b.map(|x| a.powf(x)))))),
            _ => Err(".^ requires tensors or tensor and scalar".to_string()),
        },
        BinOp::Eq => equality_values(left, right, false),
        BinOp::Ne => equality_values(left, right, true),
        BinOp::Gt => compare_values(l, r, Cmp::Gt),
        BinOp::Lt => compare_values(l, r, Cmp::Lt),
        BinOp::Gte => compare_values(l, r, Cmp::Gte),
        BinOp::Lte => compare_values(l, r, Cmp::Lte),
        BinOp::And => logical_values(left, right, |a, b| a && b),
        BinOp::Or => logical_values(left, right, |a, b| a || b),
    }
}

/// eq/ne. Two values are compared when they are of one kind — a number is
/// not text (crow_runtime::mask): a list compared with one value is a mask
/// (`mask`), a tensor with a number each element by value, a parsed value
/// with a value of its kind, two values of one kind whole; two kinds are the
/// reason, which the lowering refuses where it knows both.
pub fn equality_values(
    left: &CrowValue,
    right: &CrowValue,
    negate: bool,
) -> Result<CrowValue, String> {
    use crow_runtime::mask;
    let cmp = if negate { Cmp::Ne } else { Cmp::Eq };
    let answer = |b: bool| CrowValue::ok(Value::Bool(b));
    match (&left.value, &right.value) {
        (Value::Array(items), r) if is_scalar(r) => mask(items, cmp, r),
        (l, Value::Array(items)) if is_scalar(l) => mask(items, cmp, l),
        (Value::NumArray(t), s) | (s, Value::NumArray(t)) if is_scalar(s) => tensor_mask(t, cmp, s),
        (Value::Json(a), Value::Json(b)) => mask::json_pair(cmp, a, b).map(answer),
        (Value::Json(j), v) | (v, Value::Json(j)) if is_scalar(v) => mask::json_holds(cmp, j, mask_cell(v)).map(answer),
        // two ints exactly, as the compiled program compares them (a cell
        // is a float: past 2^53 two ints had compared equal)
        (Value::Int(a), Value::Int(b)) => Ok(answer((a == b) != negate)),
        (a, b) if is_scalar(a) && is_scalar(b) => mask::holds(cmp, mask_cell(a), true, mask_cell(b)).map(answer),
        _ => {
            whole_kinds(&left.value, &right.value, cmp)?;
            Ok(answer(values_equal(left, right) != negate))
        }
    }
}

/// Two lists compared whole — a column's cells among them — whose items'
/// kinds show only as the program runs (a file's column) are of one kind,
/// read from their first items, as the compiled program's columns read it
/// (crow_runtime::mask, `CrowColumn::whole`).
fn whole_kinds(a: &Value, b: &Value, cmp: Cmp) -> Result<(), String> {
    use crow_runtime::mask::Cell;
    fn first(v: &Value) -> Option<(usize, Cell<'_>)> {
        match v {
            Value::Array(items) => items.first().map(|i| (items.len(), mask_cell(&i.value))),
            Value::NumArray(t) if t.ndim() == 1 => t.shape().first().filter(|&&n| n > 0).map(|&n| (n, Cell::Number(0.0))),
            _ => None,
        }
    }
    let cell = |c: &Cell| matches!(c, Cell::Number(_) | Cell::Text(_) | Cell::Bool(_));
    if let (Some((n, x)), Some((m, y))) = (first(a), first(b))
        && n == m
        && cell(&x)
        && cell(&y)
    {
        crow_runtime::mask::holds(cmp, x, false, y)?;
    }
    Ok(())
}

/// A value a mask compares with: a number, text or a bool.
fn is_scalar(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_) | Value::Moment(_) | Value::Dual(..))
}

/// A value as a comparison reads it (crow_runtime::mask): a number, text
/// or a bool, or what else it is, named for the reason.
fn mask_cell(v: &Value) -> crow_runtime::mask::Cell<'_> {
    use crow_runtime::mask::Cell;
    match v {
        Value::Int(n) => Cell::Number(*n as f64),
        Value::Big(n) => Cell::Number(n.to_f64().value()),
        Value::Decimal(_) => Cell::Other("a decimal"),
        Value::Fraction(_) => Cell::Other("a fraction"),
        Value::Float(x) | Value::Dual(x, _) => Cell::Number(*x),
        Value::Str(s) => Cell::Text(s),
        Value::Bool(b) => Cell::Bool(*b),
        Value::Moment(m) => Cell::Moment(m.ms),
        Value::Array(_) | Value::NumArray(_) => Cell::Other("a list"),
        Value::Map(_) | Value::IntMap(_) => Cell::Other("a map"),
        Value::StructInstance(_) => Cell::Other("a record"),
        Value::Json(_) => Cell::Other("a JSON value"),
        Value::Frame(_) => Cell::Other("a frame"),
        Value::Fn(_) | Value::GradFn(_) => Cell::Other("a function"),
        Value::Sparse(_) => Cell::Other("a sparse matrix"),
        Value::DbConn(_) => Cell::Other("a database connection"),
        Value::Mailer(_) => Cell::Other("a mailer"),
        Value::Socket(_) => Cell::Other("a socket"),
        Value::Bytes(_) => Cell::Other("bytes"),
        Value::Stream(_) => Cell::Other("a stream"),
        Value::Response(_) => Cell::Other("a response"),
        Value::Traced(_) => Cell::Other("a tensor under grad"),
        Value::App(_) => Cell::Other("an app session"),
        Value::Done => Cell::Other("nothing"),
    }
}

/// A list compared with one value — a frame's column as its cells — is a
/// mask: each item compared as the compiled program compares its column's
/// cells, by the one function both call (crow_runtime::mask::holds), a
/// missing cell false for every operator, `ne` included.
fn mask(items: &[CrowValue], cmp: Cmp, value: &Value) -> Result<CrowValue, String> {
    let value = mask_cell(value);
    // a column's cells are of one kind: numbers, moments or text against
    // a value of their kind compare in one loop, each item's kind checked
    // by a match and nothing else (the general path asks `holds` per item,
    // which checks the kinds compare and builds a result each time — a
    // day's 871,527 timestamps against an instant had taken twice the
    // kernel's time). A kind the loop does not expect falls to the general
    // path, which refuses it with the reason.
    let truth = |b: bool| CrowValue::ok(Value::Bool(b));
    let fast: Option<Vec<CrowValue>> = match value {
        crow_runtime::mask::Cell::Number(y) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let x = match item.value {
                    Value::Float(x) => x,
                    Value::Int(n) => n as f64,
                    _ => return mask_slow(items, cmp, value),
                };
                out.push(truth(item.is_ok() && cmp.of(x.partial_cmp(&y))));
            }
            Some(out)
        }
        crow_runtime::mask::Cell::Moment(y) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Value::Moment(m) = item.value else { return mask_slow(items, cmp, value) };
                out.push(truth(item.is_ok() && cmp.of(Some(m.ms.cmp(&y)))));
            }
            Some(out)
        }
        crow_runtime::mask::Cell::Text(y) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Value::Str(t) = &item.value else { return mask_slow(items, cmp, value) };
                out.push(truth(item.is_ok() && cmp.of(Some(t.as_str().cmp(y)))));
            }
            Some(out)
        }
        _ => None,
    };
    match fast {
        Some(out) => Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(out)))),
        None => mask_slow(items, cmp, value),
    }
}

/// `mask`, item by item through the one function both backends call.
fn mask_slow(items: &[CrowValue], cmp: Cmp, value: crow_runtime::mask::Cell) -> Result<CrowValue, String> {
    let out = items
        .iter()
        .map(|item| crow_runtime::mask::holds(cmp, mask_cell(&item.value), item.is_ok(), value).map(|b| CrowValue::ok(Value::Bool(b))))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(out))))
}

/// and/or. Boolean arrays combine element-wise (mask semantics);
/// array vs scalar broadcasts; scalars short-circuit as before.
pub fn logical_values(
    left: &CrowValue,
    right: &CrowValue,
    op: impl Fn(bool, bool) -> bool,
) -> Result<CrowValue, String> {
    match (&left.value, &right.value) {
        // (masks of any depth: a matrix's are its rows of bools)
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() && !a.iter().chain(b.iter()).any(|x| matches!(x.value, Value::Array(_))) {
                return Err(format!(
                    "boolean arrays have different lengths ({} vs {})",
                    a.len(), b.len()
                ));
            }
            truths_with(a, b, &op).map(CrowValue::ok)
        }
        (Value::Array(a), _) => {
            let s = is_truthy(right);
            Ok(CrowValue::ok(truths_each(a, &|x| op(x, s))))
        }
        (_, Value::Array(b)) => {
            let s = is_truthy(left);
            Ok(CrowValue::ok(truths_each(b, &|x| op(s, x))))
        }
        _ => Ok(CrowValue::ok(Value::Bool(op(
            is_truthy(left),
            is_truthy(right),
        )))),
    }
}

pub fn compare_values(l: &Value, r: &Value, cmp: Cmp) -> Result<CrowValue, String> {
    match (l, r) {
        // Vectorized: tensor compared to scalar → boolean mask array
        (Value::NumArray(t), s @ (Value::Int(_) | Value::Float(_))) => tensor_mask(t, cmp, s),
        (s @ (Value::Int(_) | Value::Float(_)), Value::NumArray(t)) => tensor_mask(t, cmp.flipped(), s),
        // a list (a frame's column) compared with one value: a mask
        (Value::Array(items), v) if is_scalar(v) => mask(items, cmp, v),
        (v, Value::Array(items)) if is_scalar(v) => mask(items, cmp.flipped(), v),
        // two parsed values, or one beside a value, order as equality reads
        // them: two numbers or two texts, another pair the reason (the
        // compiled program had answered where this stopped)
        (Value::Json(a), Value::Json(b)) => crow_runtime::mask::json_pair(cmp, a, b).map(|b| CrowValue::ok(Value::Bool(b))),
        (Value::Json(j), v) if is_scalar(v) => crow_runtime::mask::json_holds(cmp, j, mask_cell(v)).map(|b| CrowValue::ok(Value::Bool(b))),
        (v, Value::Json(j)) if is_scalar(v) => crow_runtime::mask::json_holds(cmp.flipped(), j, mask_cell(v)).map(|b| CrowValue::ok(Value::Bool(b))),
        // two ints exactly, as the compiled program orders them (a cell is
        // a float: past 2^53 two ints had ordered as equal)
        (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Bool(match cmp {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Gt => a > b,
            Cmp::Lt => a < b,
            Cmp::Gte => a >= b,
            Cmp::Lte => a <= b,
        }))),
        // two ints, one past 64 bits: exactly
        (Value::Int(_) | Value::Big(_), Value::Int(_) | Value::Big(_)) => {
            let (Some(a), Some(b)) = (Value::int_of(l), Value::int_of(r)) else { return Err("cannot compare these types".to_string()) };
            let o = a.cmp(&b);
            Ok(CrowValue::ok(Value::Bool(match cmp {
                Cmp::Eq => o.is_eq(),
                Cmp::Ne => o.is_ne(),
                Cmp::Gt => o.is_gt(),
                Cmp::Lt => o.is_lt(),
                Cmp::Gte => o.is_ge(),
                Cmp::Lte => o.is_le(),
            })))
        }
        // two values: numbers by value, text by character code; an order
        // between kinds with none is the reason, as the lowering refuses it
        // where it knows both
        (a, b) if is_scalar(a) && is_scalar(b) => crow_runtime::mask::holds(cmp, mask_cell(a), true, mask_cell(b)).map(|b| CrowValue::ok(Value::Bool(b))),
        _ => Err("cannot compare these types".to_string()),
    }
}

/// A tensor compared with a number: each element by value.
fn tensor_mask(t: &CrowTensor, cmp: Cmp, s: &Value) -> Result<CrowValue, String> {
    let s = mask_cell(s);
    let flat = t
        .to_vec()
        .iter()
        .map(|&v| crow_runtime::mask::holds(cmp, crow_runtime::mask::Cell::Number(v), true, s))
        .collect::<Result<Vec<bool>, _>>()?;
    // (of the tensor's shape: a matrix's mask is its rows of bools, as the
    // compiled program's `crow_runtime::mask::shaped` makes it)
    Ok(CrowValue::ok(truths_shaped(&flat, t.shape())))
}

/// A mask of a shape, its truths in row order: a list of bools for one
/// axis (or none), a list of rows of bools for two, and deeper.
fn truths_shaped(flat: &[bool], shape: &[usize]) -> Value {
    if shape.len() <= 1 {
        return Value::Array(std::sync::Arc::new(flat.iter().map(|b| CrowValue::ok(Value::Bool(*b))).collect()));
    }
    let step: usize = shape[1..].iter().product();
    let rows = (0..shape[0]).map(|i| CrowValue::ok(truths_shaped(flat.get(i * step..(i + 1) * step).unwrap_or(&[]), &shape[1..]))).collect();
    Value::Array(std::sync::Arc::new(rows))
}

/// A mask of any depth as its numbers in row order (true 1, false 0) and
/// its shape, or the reason its rows are not one length.
pub fn truths_flat(items: &[CrowValue], out: &mut Vec<f64>) -> Result<Vec<usize>, String> {
    let mut inner: Option<Vec<usize>> = None;
    for x in items {
        let shape = match &x.value {
            Value::Array(xs) if x.is_ok() => truths_flat(xs, out)?,
            _ => {
                out.push(if is_truthy(x) { 1.0 } else { 0.0 });
                Vec::new()
            }
        };
        match &inner {
            Some(s) if *s != shape => return Err("a mask's rows are of different lengths".to_string()),
            Some(_) => {}
            None => inner = Some(shape),
        }
    }
    let mut shape = vec![items.len()];
    shape.extend(inner.unwrap_or_default());
    Ok(shape)
}

/// Each truth of a mask of any depth turned: a list's items, a list of
/// rows' rows (a matrix's comparison keeps its shape).
fn truths_each(items: &[CrowValue], f: &dyn Fn(bool) -> bool) -> Value {
    Value::Array(std::sync::Arc::new(
        items
            .iter()
            .map(|x| match &x.value {
                Value::Array(inner) if x.is_ok() => CrowValue::ok(truths_each(inner, f)),
                _ => CrowValue::ok(Value::Bool(f(is_truthy(x)))),
            })
            .collect(),
    ))
}

/// Two masks of one shape, truth by truth, or the reason (the compiled
/// program's `crow_runtime::mask::joined` words for one of another shape).
fn truths_with(a: &[CrowValue], b: &[CrowValue], f: &dyn Fn(bool, bool) -> bool) -> Result<Value, String> {
    if a.len() != b.len() {
        return Err(format!("masks of {} and {} items are joined truth by truth: they must be one shape", a.len(), b.len()));
    }
    let mut out = Vec::with_capacity(a.len());
    for (x, y) in a.iter().zip(b.iter()) {
        out.push(match (&x.value, &y.value) {
            (Value::Array(xs), Value::Array(ys)) if x.is_ok() && y.is_ok() => CrowValue::ok(truths_with(xs, ys, f)?),
            _ => CrowValue::ok(Value::Bool(f(is_truthy(x), is_truthy(y)))),
        });
    }
    Ok(Value::Array(std::sync::Arc::new(out)))
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
    Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(results))))
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
    if matches!(v, Value::Traced(_)) {
        return crate::autograd_bridge::unaryop(op, v);
    }
    match op {
        UnaryOp::Neg => match v {
            Value::Int(n) => Ok(CrowValue::ok(Value::of_int(crow_runtime::int::Int::Small(*n).neg()))),
            Value::Big(n) => Ok(CrowValue::ok(Value::of_int(crow_runtime::int::Int::Big(n.clone()).neg()))),
            Value::Decimal(d) => Ok(crate::table::write_decimal(d.neg())),
            Value::Fraction(f) => Ok(crate::table::write_fraction(f.neg())),
            Value::Float(n) => Ok(CrowValue::ok(Value::Float(-n))),
            Value::Dual(a, da) => Ok(CrowValue::ok(Value::Dual(-a, -da))),
            Value::NumArray(t) => Ok(CrowValue::ok(Value::NumArray(Box::new(t.neg())))),
            _ => Err("cannot negate this type".to_string()),
        },
        UnaryOp::Not => match v {
            // Boolean arrays negate element-wise (mask semantics), a
            // matrix's rows each
            Value::Array(items) => Ok(CrowValue::ok(truths_each(items, &|x| !x))),
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
        Value::Traced(t) => t.as_number().is_none_or(|x| x != 0.0),
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
            for item in items.iter() {
                match &item.value {
                    Value::Int(n) => data.push(*n as f64),
                    Value::Float(f) => data.push(*f),
                    _ => return v.clone(),
                }
            }
            let n = data.len();
            return Value::NumArray(Box::new(CrowTensor::from_vec(data, vec![n])));
        }
    }
    v.clone()
}

/// `a eq b` of two values, as the compiled program's `==` compares its
/// typed values: numbers by value (an int beside a float as a float), a
/// list item by item, a map entry by entry, a record field by field; an
/// error equals nothing, as a result that failed does (a missing cell holds
/// a placeholder, never a value to compare).
pub fn values_equal(a: &CrowValue, b: &CrowValue) -> bool {
    if a.is_error() || b.is_error() {
        return false;
    }
    match (&a.value, &b.value) {
        // parsed values compare as JSON, and one with a value the program
        // built compares with that value's JSON
        (Value::Json(x), Value::Json(y)) => x == y,
        (Value::Json(x), _) => crate::builtins::crow_to_json("", b).is_ok_and(|y| **x == y),
        (_, Value::Json(y)) => crate::builtins::crow_to_json("", a).is_ok_and(|x| x == **y),
        (Value::Int(x), Value::Int(y)) => x == y,
        // (an int in 64 bits is never `Big`: one of each are unequal)
        (Value::Big(x), Value::Big(y)) => x == y,
        // exact numbers by value, with each other and with ints
        (Value::Decimal(_) | Value::Fraction(_), _) | (_, Value::Decimal(_) | Value::Fraction(_)) => {
            exact_binop(a, &BinOp::Eq, b).is_ok_and(|v| matches!(v.value, Value::Bool(true)))
        }
        (Value::Big(x), Value::Float(y)) | (Value::Float(y), Value::Big(x)) => x.to_f64().value() == *y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
        (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
        (Value::Dual(x, _), Value::Dual(y, _)) => x == y,
        (Value::Bytes(x), Value::Bytes(y)) => x == y,
        (Value::Dual(x, _), Value::Float(y)) => x == y,
        (Value::Float(x), Value::Dual(y, _)) => x == y,
        (Value::Dual(x, _), Value::Int(y)) => *x == (*y as f64),
        (Value::Int(x), Value::Dual(y, _)) => (*x as f64) == *y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        // (two moments are equal by instant; a list of them had compared
        // false item by item, where the compiled column compared true)
        (Value::Moment(x), Value::Moment(y)) => x == y,
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
        (Value::Map(x), Value::Map(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|((ka, va), (kb, vb))| ka == kb && values_equal(va, vb)),
        (Value::IntMap(x), Value::IntMap(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|((ka, va), (kb, vb))| ka == kb && values_equal(va, vb)),
        (Value::StructInstance(x), Value::StructInstance(y)) => {
            std::sync::Arc::ptr_eq(x, y) || (x.name() == y.name() && x.len() == y.len() && x.fields().zip(y.fields()).all(|((ka, va), (kb, vb))| ka == kb && values_equal(va, vb)))
        }
        (Value::Frame(x), Value::Frame(y)) => x == y,
        (Value::Sparse(x), Value::Sparse(y)) => x == y,
        (Value::Traced(x), Value::Traced(y)) => x.value == y.value,
        (Value::Done, Value::Done) => true,
        // a function, a connection, a mailer, a socket, a stream, a
        // response and a session are equal to nothing, themselves included:
        // each is a handle, not a value (listed so that a kind added is
        // decided here, not by the arm below)
        (Value::Fn(_), Value::Fn(_)) | (Value::GradFn(_), Value::GradFn(_)) | (Value::DbConn(_), Value::DbConn(_)) | (Value::Mailer(_), Value::Mailer(_)) | (Value::Socket(_), Value::Socket(_)) | (Value::Stream(_), Value::Stream(_)) | (Value::Response(_), Value::Response(_)) | (Value::App(_), Value::App(_)) => false,
        // two values of two kinds
        _ => false,
    }
}

/// A value indexed, or a struct constructed: `xs[i]`, `t[i]` (a row of a
/// matrix, a gathered tensor), `s[i]` (a character), `m['k']`,
/// `Point[1, 2]`.
pub fn index(callee: &CrowValue, arg_vals: Vec<CrowValue>) -> Result<CrowValue, String> {
    match &callee.value {
    // (the lowering refuses an index into a tensor a function grad
    // differentiates holds; this is one it could not see)
    Value::Traced(_) => Err("grad: an index into a tensor is not differentiable — compute with the tensor whole".to_string()),
    // a parsed value's item or key, as the compiled program reads its JSON:
    // one it lacks (or a negative position) is JSON's null, an error value
    // of reason `null`
    Value::Json(j) => {
        if arg_vals.len() != 1 {
            return Err("a JSON value is indexed by one position or key".to_string());
        }
        let found = match &arg_vals[0].unwrap_or_propagate()? {
            Value::Int(i) => usize::try_from(*i).ok().and_then(|i| j.get(i)),
            Value::Str(k) => j.get(k.as_str()),
            other => return Err(format!("a JSON value is indexed by a position or a key, got {}", other.type_name())),
        };
        Ok(match found {
            Some(x) if !x.is_null() => CrowValue::ok(Value::Json(std::sync::Arc::new(x.clone()))),
            _ => CrowValue::error_with_kind("null", "parse", ""),
        })
    }
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
                    let at = stops::row(idx, tensor.shape()[0])?;
                    return Ok(CrowValue::ok(Value::NumArray(Box::new(tensor.index0(at)))));
                }
                let v = tensor.get_flat(stops::item(idx, tensor.len())?);
                // Return as int if it's a whole number
                if v == (v as i64) as f64 && v.is_finite() {
                    Ok(CrowValue::ok(Value::Int(v as i64)))
                } else {
                    Ok(CrowValue::ok(Value::Float(v)))
                }
            }
            Value::NumArray(idx_tensor) => {
                // Advanced indexing: gather by index array
                Ok(CrowValue::ok(Value::NumArray(Box::new(tensor.try_gather(idx_tensor)?))))
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
            // (read with the runtime's one walk: every character had been
            // collected first, for each position read)
            Value::Int(idx) => Ok(CrowValue::ok(Value::Str(crow_runtime::text::char_at_known(s, &crow_runtime::text::Known::new(), *idx)?))),
            _ => Err("string index must be an integer".to_string()),
        }
    }
    // a map's key is of the map's kind: text, or an int (spec, Maps)
    Value::Map(map) => {
        if arg_vals.len() != 1 {
            return Err("map access requires exactly one argument".to_string());
        }
        match arg_vals[0].unwrap_or_propagate()? {
            Value::Str(s) => stops::key(map, s).cloned(),
            Value::Int(n) => Err(format!("the map's keys are text, not ints: {}", n)),
            other => Err(format!("a map's key is text or an int, got {}", other.type_name())),
        }
    }
    Value::IntMap(map) => {
        if arg_vals.len() != 1 {
            return Err("map access requires exactly one argument".to_string());
        }
        match arg_vals[0].unwrap_or_propagate()? {
            Value::Int(n) => stops::key(map, n).cloned(),
            Value::Str(s) => Err(format!("the map's keys are ints, not text: '{}'", s)),
            other => Err(format!("a map's key is text or an int, got {}", other.type_name())),
        }
    }
        _ => Err("not indexable".to_string()),
    }
}

/// `.ok` of a frame column: each row's validity, a missing cell false — the
/// mask `df[df.col.ok]` keeps the complete rows with. A column that
/// failed to read is its error, as any field of an error value is.
pub fn valid(obj: CrowValue) -> Result<CrowValue, String> {
    if obj.is_error() {
        return Ok(obj);
    }
    match &obj.value {
        Value::Array(items) => Ok(CrowValue::ok(Value::Array(
            items.iter().map(|item| CrowValue::ok(Value::Bool(item.is_ok()))).collect::<Vec<_>>().into(),
        ))),
        _ => Ok(CrowValue::ok(Value::Bool(obj.is_ok()))),
    }
}

/// A field of a value: the status fields on anything, `len`, the datetime
/// parts of a text, a tensor's shape, a frame's properties and columns, a
/// struct's or a map's member, a module's binding.
/// A frame's cell: a missing one is an error of kind `missing` (the
/// column's `.ok` reads it false, a read by index gets its placeholder).
pub fn frame_cell(frame: &crow_runtime::frame::CrowFrame, col: usize, row: usize) -> CrowValue {
    column_cell(frame.data[col].as_ref(), frame.validity[col][row], row)
}

/// A column's cell, `valid` saying whether it is there: a missing one is
/// an error of kind `missing` holding the column's placeholder.
pub fn column_cell(column: &crow_runtime::frame::Column, valid: bool, row: usize) -> CrowValue {
    use crow_runtime::frame::Column;
    if !valid {
        let mut missing = CrowValue::error_with_kind("missing", "missing", "");
        missing.status_mut().trace.clear();
        // the safe default a failed value holds (spec §10), of the column's
        // kind: a column copied from its cells keeps its kind even when every
        // cell is missing
        missing.value = match column {
            Column::Int(_) => Value::Int(0),
            Column::Float(_) => Value::Float(f64::NAN),
            Column::Bool(_) => Value::Bool(false),
            Column::Str(_) => Value::Str(String::new()),
            Column::DateTime(..) => Value::Moment(crow_runtime::moments::Moment::default()),
            // (a vector of zeros of the column's width, as the compiled
            // `get_vector` answers a missing cell)
            Column::Vectors(w, _) => Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::zeros(vec![(*w).max(1)]))),
        };
        return missing;
    }
    CrowValue::ok(match column {
        Column::Int(v) => Value::Int(v[row]),
        Column::Float(v) => Value::Float(v[row]),
        Column::Str(v) => Value::Str(v[row].to_string()),
        Column::Bool(v) => Value::Bool(v[row]),
        Column::DateTime(millis, offsets) => Value::Moment(crow_runtime::moments::Moment { ms: millis[row], offset: offsets[row] }),
        Column::Vectors(..) => Value::NumArray(Box::new(column.vector_at(row))),
    })
}

/// Where `xs[i]` reads in a list of `len` items: a negative index counts
/// from the end.
pub fn list_position(index: &CrowValue, len: usize) -> Result<usize, String> {
    match index.unwrap_or_propagate()? {
        Value::Int(idx) => stops::item(*idx, len),
        _ => Err("array index must be an integer".to_string()),
    }
}

pub fn field(obj: CrowValue, field: &str) -> Result<CrowValue, String> {
    // a parsed value, as the compiled program reads its JSON: JSON's null
    // is an error value of reason `null`, and a field it lacks is null
    if let (Value::Json(j), true) = (&obj.value, obj.is_ok()) {
        let null = j.is_null();
        return Ok(match field {
            "ok" => CrowValue::ok(Value::Bool(!null)),
            "reason" => CrowValue::ok(Value::Str(if null { "null" } else { "" }.to_string())),
            "kind" => CrowValue::ok(Value::Str(if null { "parse" } else { "" }.to_string())),
            "trace" => CrowValue::ok(Value::Array(std::sync::Arc::new(Vec::new()))),
            name => CrowValue::ok(Value::Json(std::sync::Arc::new(j.get(name).cloned().unwrap_or(serde_json::Value::Null)))),
        });
    }

    // special fields: .ok, .reason, .kind, .trace — always work, even on errors.
    // `.ok` is the value's own status: a list that did not fail is ok,
    // whatever it holds (a frame column's rows are `valid`).
    match field {
        "ok" => {
            return Ok(CrowValue::ok(Value::Bool(obj.is_ok())));
        }
        "reason" => {
            return Ok(CrowValue::ok(Value::Str(obj.reason().to_string())));
        }
        "kind" => {
            return Ok(CrowValue::ok(Value::Str(obj.kind().to_string())));
        }
        "trace" => {
            let trace_list: Vec<CrowValue> = obj.trace().iter()
                .map(|s| CrowValue::ok(Value::Str(s.clone())))
                .collect();
            return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(trace_list))));
        }
        _ => {}
    }

    // soft error propagation for other field access
    if obj.is_error() {
        return Ok(obj);
    }

    // `x.len` is `len[x]` (the lowering calls the row wherever it knows x
    // is measured); a record's field, a map's key or a frame's column named
    // `len` is its own
    if field == "len" {
        match &obj.value {
            Value::Array(items) => return Ok(CrowValue::ok(Value::Int(items.len() as i64))),
            Value::NumArray(t) => return Ok(CrowValue::ok(Value::Int(crow_runtime::core::len_tensor(t)))),
            Value::Str(s) => return Ok(CrowValue::ok(Value::Int(crow_runtime::core::len_text(s)))),
            _ => {}
        }
    }

    // a moment's fields, read in its own zone
    if let Value::Moment(m) = &obj.value {
        if let Some(val) = m.field(field) {
            return Ok(CrowValue::ok(Value::Int(val)));
        }
    }
    // the same field over a column of moments (the interpreter holds a
    // column as the list of its cells, a missing one an error value): a
    // list of ints, a missing cell staying missing
    if let Value::Array(items) = &obj.value {
        if crate::lower_ir::moment_field_name(field) && items.iter().all(|i| i.is_error() || matches!(i.value, Value::Moment(_))) {
            let out: Vec<CrowValue> = items
                .iter()
                .map(|i| match &i.value {
                    Value::Moment(m) if i.is_ok() => CrowValue::ok(Value::Int(m.field(field).unwrap_or(0))),
                    _ => i.clone(),
                })
                .collect();
            return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(out))));
        }
    }

    // a traced tensor's properties are its value's
    if let Value::Traced(tr) = &obj.value {
        return crate::ops::field(CrowValue::ok(Value::NumArray(Box::new(tr.value.clone()))), field);
    }
    // NumArray properties: .shape, .dtype, .device, .grad
    if let Value::NumArray(t) = &obj.value {
        match field {
            "shape" => {
                let shape_vals: Vec<CrowValue> = t.shape().iter()
                    .map(|&s| CrowValue::ok(Value::Int(s as i64)))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(shape_vals))));
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
                return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(shape))));
            }
            "columns" => {
                let cols: Vec<CrowValue> = frame.columns.iter()
                    .map(|c| CrowValue::ok(Value::Str(c.clone())))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(cols))));
            }
            "dtypes" => {
                let dtypes: Vec<CrowValue> = frame.dtypes().iter()
                    .map(|d| CrowValue::ok(Value::Str(d.to_string())))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(dtypes))));
            }
            _ => {
                // Try column access by name: df.age returns the column as an array
                if let Some(idx) = frame.col_index(field) {
                    // (a column of no rows keeps its kind, which no cell says)
                    if frame.nrows() == 0 {
                        return Ok(CrowValue::empty_column(frame.data[idx].as_ref()));
                    }
                    let vals: Vec<CrowValue> = (0..frame.nrows()).map(|i| frame_cell(frame, idx, i)).collect();
                    return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(vals))));
                }
                return Err(stops::no_column(field));
            }
        }
    }

    // struct and module field access
    match &obj.value {
        Value::StructInstance(si) => {
            match si.get(field) {
                Some(val) => Ok(val.clone()),
                None => Err(format!(
                    "struct '{}' has no field '{}'",
                    si.shown(), field
                )),
            }
        }
        Value::Map(m) => stops::key(m, &field.to_string()).cloned(),
        Value::IntMap(_) => Err(format!("a map keyed by ints is read by `m[{}]`: `.{}` names a text key", field, field)),
        _ => Err(format!("cannot access field '{}' on this value", field)),
    }
}

/// An array literal's value: all numbers make a tensor, all tensors of one
/// shape make a shaped tensor (spec: unified arrays), anything else a list.
pub fn array_literal(vals: Vec<CrowValue>) -> Result<CrowValue, String> {
    let all_numeric = !vals.is_empty() && vals.iter().all(|v| v.is_ok() && matches!(v.value, Value::Int(_) | Value::Float(_)));
    if all_numeric {
        let data: Vec<f64> = vals.iter().map(|v| match &v.value {
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            _ => unreachable!(),
        }).collect();
        let len = data.len();
        return Ok(CrowValue::ok(Value::NumArray(Box::new(CrowTensor::from_vec(data, vec![len])))));
    }
    let all_tensors = !vals.is_empty() && vals.iter().all(|v| v.is_ok() && matches!(v.value, Value::NumArray(_)));
    if all_tensors {
        let tensors: Vec<CrowTensor> = vals.iter().map(|v| match &v.value {
            Value::NumArray(t) => t.as_ref().clone(),
            _ => unreachable!(),
        }).collect();
        let stacked = CrowTensor::from_elements(&tensors)?;
        return Ok(CrowValue::ok(Value::NumArray(Box::new(stacked))));
    }
    Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(vals))))
}

/// What a collecting loop gathers, held as the IR types it: numbers it
/// holds as a tensor are one, and anything else is the list of what was
/// yielded, ints kept ints — the compiled program's `Vec` (a loop of ints
/// had been a tensor of floats, whose own rows the values then picked).
pub fn collect_loop(results: Vec<CrowValue>, as_tensor: bool) -> CrowValue {
    // an iteration that yielded nothing is skipped; one that yielded an
    // error keeps it as its item (an error's value is `Done` too)
    let filtered: Vec<CrowValue> = results.into_iter().filter(|v| !(v.is_ok() && matches!(v.value, Value::Done))).collect();
    let all_numeric = !filtered.is_empty() && filtered.iter().all(|v| v.is_ok() && matches!(v.value, Value::Int(_) | Value::Float(_)));
    if as_tensor && all_numeric {
        let data: Vec<f64> = filtered.iter().map(|v| match &v.value {
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            _ => unreachable!(),
        }).collect();
        let len = data.len();
        return CrowValue::ok(Value::NumArray(Box::new(CrowTensor::from_vec(data, vec![len]))));
    }
    CrowValue::ok(Value::Array(std::sync::Arc::new(filtered)))
}

/// What a `for` walks: a list's items, a matrix's rows, a vector's numbers
/// (whole ones as ints).
pub fn iter_items(iter_val: &CrowValue) -> Result<Vec<CrowValue>, String> {
    match &iter_val.value {
        Value::Array(items) => Ok(items.as_ref().clone()),
        // a parsed list's items, each JSON, as the compiled program walks it
        Value::Json(j) => Ok(crow_runtime::json::items(j)?.into_iter().map(|x| CrowValue::ok(Value::Json(std::sync::Arc::new(x)))).collect()),
        Value::NumArray(tensor) => {
            if tensor.ndim() >= 2 {
                Ok(tensor.rows().into_iter().map(|t| CrowValue::ok(Value::NumArray(Box::new(t)))).collect())
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
                for (k, val) in m.iter() {
                    match (k.as_str(), &val.value) {
                        // props that failed: the widget shows the reason in
                        // its place, as a control shows a failed argument
                        ("props", _) if val.is_error() => {
                            return crate::harness::html_escape_text(&crow_web::core::response::failed_in_view(&val.reason()));
                        }
                        ("props", _) => props = crate::builtins::crow_to_json("", val).unwrap_or(serde_json::Value::Null),
                        ("on", Value::Map(evs)) => {
                            for (event, h) in evs.iter() {
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
            // (an item that failed — a column's missing cell — shows as a
            // failed value does: it had shown its placeholder, `0`)
            if it.is_error() {
                inner.push_str(&crow_web::core::response::failed_in_view(&it.reason()));
                continue;
            }
            match &it.value {
                Value::Str(s) if it.is_markup() => inner.push_str(s),
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
    // table[frame, sort = true]: which sorted table this is, and its order
    let (mut table_sort, mut sort_index, mut sort_by, mut sort_desc): (bool, usize, Option<String>, bool) = (false, 0, None, false);
    // the wiring, as the compiled page writes it: the handler a click, an
    // input or a form calls, and the arguments a click froze
    let mut events: Vec<(String, String)> = Vec::new();
    let mut confirm: Option<String> = None;
    for (at, v) in vals.iter().enumerate() {
        if v.is_error() {
            // a view renders what it is given: a failed value shows an id
            // the server's log has its reason under
            inner.push_str(&crow_web::core::response::failed_in_view(&v.reason()));
            continue;
        }
        match &v.value {
            // (the element's own attributes come first; a map among its
            // children is text, as any value is — it had been read as
            // attributes, which the compiled program did not build)
            Value::Map(m) if at == 0 => {
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
                            ("sort", Value::Bool(b)) => {
                                table_sort = *b;
                                continue;
                            }
                            ("__sort_index", Value::Int(n)) => {
                                sort_index = (*n).max(0) as usize;
                                continue;
                            }
                            ("__sort_by", Value::Str(s)) => {
                                sort_by = Some(s.clone());
                                continue;
                            }
                            ("__sort_desc", Value::Bool(b)) => {
                                sort_desc = *b;
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
            Value::Str(s) if v.is_markup() => inner.push_str(s),
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
                table_sort.then(|| crow_runtime::layout::Sorting { index: sort_index, by: sort_by.clone().map(|c| (c, sort_desc)) }),
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
    // an element with a click the browser does not press gets a tab stop
    if out_attrs.iter().any(|(k, _)| k == "data-crow-click") {
        let extra = crow_runtime::layout::press_attrs(&rendered.tag, &out_attrs);
        out_attrs.extend(extra);
    }
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
