//! The builtins that need no interpreter state, in one place: the
//! interpreter (`ir_interp.rs`, ir-design.md §5) calls nothing else for
//! these names, and each arm calls the runtime the compiled mode links
//! where it owns the operation. Each arm reads
//! its arguments as values and answers a value; `trace` is the caller's
//! position for the error values that carry one.

use crate::value::*;
use crow_runtime::tensor::CrowTensor;
use std::collections::BTreeMap;

/// A state-free builtin by name, or `None` when the name is one the
/// walker itself must handle.
/// The names answered here.
pub const HANDLED: &[&str] = &["log", "type", "map", "range", "abs", "sqrt", "floor", "ceil", "round", "min", "max", "split", "trim", "replace", "upper", "lower", "join", "starts", "ends", "repeat", "pad", "fmt", "int", "float", "str", "len", "contains", "slice", "keys", "values", "has", "push", "pop", "sort", "reverse", "append", "concat", "set", "del", "args", "input", "rand", "time", "sleep", "env", "exit", "read", "write", "exists", "mkdir", "ls", "rm", "mv", "cp", "json", "encode", "shell", "hash", "matches", "sub", "now", "moment", "calendar", "parse_time", "assert", "error"];

/// A state-free builtin by name, or `None` when the name is one the
/// walker itself must handle.
pub fn call(name: &str, args: &[CrowValue], trace: &str) -> Option<Result<CrowValue, String>> {
    if !HANDLED.contains(&name) {
        return None;
    }
    Some(dispatch(name, args, trace))
}

fn dispatch(name: &str, args: &[CrowValue], trace: &str) -> Result<CrowValue, String> {
    match name {
        "log" => {
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                if arg.is_error() {
                    print!("error: {}", arg.reason);
                } else {
                    print!("{}", arg.value);
                }
            }
            println!();
            Ok(CrowValue::done())
        }
        "type" => {
            if args.len() != 1 { return Err("type[] takes exactly 1 argument".into()); }
            let type_name = if args[0].is_error() {
                "error".to_string()
            } else {
                match &args[0].value {
                    Value::App(..) => "app".to_string(),
                    Value::Int(_) => "int".to_string(),
                    Value::Float(_) => "float".to_string(),
                    Value::Str(_) => "str".to_string(),
                    Value::Bool(_) => "bool".to_string(),
                    Value::Array(_) | Value::NumArray(_) => "array".to_string(),
                    Value::Sparse(_) => "sparse".to_string(),
                    Value::Map(_) => "map".to_string(),
                    Value::Fn(_) => "fn".to_string(),
                    Value::StructInstance(si) => si.name.clone(),
                    Value::Dual(_, _) => "float".to_string(),
                    Value::GradFn(_) => "fn".to_string(),
                    Value::Frame(_) => "frame".to_string(),
                    Value::DbConn(_) => "db".to_string(),
                    Value::Mailer(..) => "mailer".to_string(),
                    Value::Done => "none".to_string(),
                }
            };
            Ok(CrowValue::ok(Value::Str(type_name)))
        }
        "map" => {
            // map[] creates an empty map
            if args.is_empty() {
                Ok(CrowValue::ok(Value::Map(BTreeMap::new())))
            } else {
                Err("map[] takes no arguments".to_string())
            }
        }
        "range" => {
            // range[stop], range[start, stop], range[start, stop, step]
            let (start, stop, step) = match args.len() {
                1 => {
                    match args[0].unwrap_or_propagate()? {
                        Value::Int(n) => (0i64, *n, 1i64),
                        _ => return Err("range[] requires integer arguments".to_string()),
                    }
                }
                2 => {
                    match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                        (Value::Int(a), Value::Int(b)) => (*a, *b, 1i64),
                        _ => return Err("range[] requires integer arguments".to_string()),
                    }
                }
                3 => {
                    match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                        (Value::Int(a), Value::Int(b), Value::Int(c)) => {
                            if *c == 0 {
                                return Err("range[] step cannot be zero".to_string());
                            }
                            (*a, *b, *c)
                        }
                        _ => return Err("range[] requires integer arguments".to_string()),
                    }
                }
                _ => return Err("range[] takes 1, 2, or 3 arguments".to_string()),
            };
            let result = crow_runtime::core::range(start, stop, step)?
                .into_iter()
                .map(|i| CrowValue::ok(Value::Int(i)))
                .collect();
            Ok(CrowValue::ok(Value::Array(result)))
        }
        // --- Math builtins ---
        "abs" => {
            if args.len() != 1 { return Err("abs[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(n.abs()))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Float(n.abs()))),
                Value::Dual(a, da) => {
                    // abs'(x) = sign(x) * x'
                    let sign = if *a > 0.0 { 1.0 } else if *a < 0.0 { -1.0 } else { 0.0 };
                    Ok(CrowValue::ok(Value::Dual(a.abs(), da * sign)))
                }
                _ => Err("abs[] requires a number".into()),
            }
        }
        "sqrt" => {
            if args.len() != 1 { return Err("sqrt[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Float((*n as f64).sqrt()))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Float(n.sqrt()))),
                Value::Dual(a, da) => {
                    // sqrt'(x) = x' / (2*sqrt(x))
                    let s = a.sqrt();
                    Ok(CrowValue::ok(Value::Dual(s, da / (2.0 * s))))
                }
                _ => Err("sqrt[] requires a number".into()),
            }
        }
        "floor" => {
            if args.len() != 1 { return Err("floor[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.floor() as i64))),
                Value::Dual(a, _) => {
                    // floor is a step function, derivative is 0
                    Ok(CrowValue::ok(Value::Dual(a.floor(), 0.0)))
                }
                _ => Err("floor[] requires a number".into()),
            }
        }
        "ceil" => {
            if args.len() != 1 { return Err("ceil[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.ceil() as i64))),
                Value::Dual(a, _) => {
                    Ok(CrowValue::ok(Value::Dual(a.ceil(), 0.0)))
                }
                _ => Err("ceil[] requires a number".into()),
            }
        }
        "round" => {
            if args.len() != 1 { return Err("round[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.round() as i64))),
                Value::Dual(a, _) => {
                    Ok(CrowValue::ok(Value::Dual(a.round(), 0.0)))
                }
                _ => Err("round[] requires a number".into()),
            }
        }
        // pow removed — use ^ operator instead
        "min" => {
            if args.len() != 2 { return Err("min[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(*a.min(b)))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.min(*b)))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).min(*b)))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.min(*b as f64)))),
                _ => Err("min[] requires numbers".into()),
            }
        }
        "max" => {
            if args.len() != 2 { return Err("max[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(*a.max(b)))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.max(*b)))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).max(*b)))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.max(*b as f64)))),
                _ => Err("max[] requires numbers".into()),
            }
        }
        // --- String builtins ---
        "split" => {
            if args.len() != 2 { return Err("split[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(s), Value::Str(delim)) => {
                    let parts: Vec<CrowValue> = crow_runtime::text::split(s, delim).into_iter().map(|p| CrowValue::ok(Value::Str(p))).collect();
                    Ok(CrowValue::ok(Value::Array(parts)))
                }
                _ => Err("split[] requires two strings".into()),
            }
        }
        "trim" => {
            if args.len() != 1 { return Err("trim[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(s) => Ok(CrowValue::ok(Value::Str(crow_runtime::text::trim(s)))),
                _ => Err("trim[] requires a string".into()),
            }
        }
        "replace" => {
            if args.len() != 3 { return Err("replace[] takes exactly 3 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                (Value::Str(s), Value::Str(old), Value::Str(new)) => Ok(CrowValue::ok(Value::Str(crow_runtime::text::replace(s, old, new)))),
                _ => Err("replace[] requires three strings".into()),
            }
        }
        "upper" => {
            if args.len() != 1 { return Err("upper[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(s) => Ok(CrowValue::ok(Value::Str(crow_runtime::text::upper(s)))),
                _ => Err("to_upper[] requires a string".into()),
            }
        }
        "lower" => {
            if args.len() != 1 { return Err("lower[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(s) => Ok(CrowValue::ok(Value::Str(crow_runtime::text::lower(s)))),
                _ => Err("to_lower[] requires a string".into()),
            }
        }
        "join" => {
            if args.len() != 2 { return Err("join[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Array(items), Value::Str(sep)) => {
                    let parts: Vec<String> = items.iter()
                        .map(|item| match item.unwrap_or_propagate() {
                            Ok(Value::Str(s)) => s.clone(),
                            Ok(Value::Int(n)) => format!("{}", n),
                            Ok(Value::Float(f)) => format!("{}", f),
                            Ok(Value::Bool(b)) => format!("{}", b),
                            _ => "?".to_string(),
                        })
                        .collect();
                    Ok(CrowValue::ok(Value::Str(parts.join(sep.as_str()))))
                }
                (Value::NumArray(t), Value::Str(sep)) => {
                    Ok(CrowValue::ok(Value::Str(crow_runtime::core::join_floats(&t.to_vec(), sep))))
                }
                _ => Err("join[] requires an array and a string separator".into()),
            }
        }
        "starts" => {
            if args.len() != 2 { return Err("starts[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(s), Value::Str(prefix)) => Ok(CrowValue::ok(Value::Bool(crow_runtime::text::starts(s, prefix)))),
                _ => Err("starts_with[] requires two strings".into()),
            }
        }
        "ends" => {
            if args.len() != 2 { return Err("ends[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(s), Value::Str(suffix)) => Ok(CrowValue::ok(Value::Bool(crow_runtime::text::ends(s, suffix)))),
                _ => Err("ends_with[] requires two strings".into()),
            }
        }
        "repeat" => {
            if args.len() != 2 { return Err("repeat[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(s), Value::Int(n)) => Ok(CrowValue::ok(Value::Str(crow_runtime::text::repeat(s, *n)?))),
                _ => Err("repeat[] requires a string and an int".into()),
            }
        }
        "pad" => {
            Err("pad was removed — use fmt[s, width = n]".into())
        }
        // fmt: value formatting via the shared display kernel. The lowering
        // pass guarantees exactly 7 positional args (sentinel defaults).
        "fmt" => {
            if args.len() != 9 { return Err("fmt[] takes a value plus named arguments (decimals, commas, width, align, fill, percent, time, tz)".into()); }
            let time_style = match &args[7].value { Value::Str(s) => s.clone(), _ => String::new() };
            if !time_style.is_empty() {
                let tz = match &args[8].value { Value::Str(s) => s.clone(), _ => "UTC".to_string() };
                let t = match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => *n as f64,
                    Value::Float(f) => *f,
                    other => return Err(format!("fmt: time display needs a moment (a number), got {}", other.type_name())),
                };
                return match crow_runtime::moments::fmt_time(t, &time_style, &tz) {
                    Ok(s) => Ok(CrowValue::ok(Value::Str(s))),
                    Err(e) => Ok(CrowValue::error_with_kind(&e, "time", trace)),
                };
            }
            let decimals = match &args[1].value { Value::Int(n) => *n, _ => return Err("fmt: decimals must be an int".into()) };
            let commas = matches!(&args[2].value, Value::Bool(true));
            let width = match &args[3].value { Value::Int(n) => *n, _ => return Err("fmt: width must be an int".into()) };
            let align = match &args[4].value { Value::Str(s) => s.clone(), _ => return Err("fmt: align must be a string".into()) };
            if !align.is_empty() && align != "left" && align != "right" {
                return Err(format!("fmt: align must be 'left' or 'right', got '{}'", align));
            }
            let fill = match &args[5].value { Value::Str(s) => s.clone(), _ => return Err("fmt: fill must be a string".into()) };
            let percent = matches!(&args[6].value, Value::Bool(true));
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Str(
                    crow_runtime::display::crow_fmt_f64(*n as f64, decimals, commas, width, &align, &fill, percent),
                ))),
                Value::Float(f) => Ok(CrowValue::ok(Value::Str(
                    crow_runtime::display::crow_fmt_f64(*f, decimals, commas, width, &align, &fill, percent),
                ))),
                Value::Str(s) => {
                    if decimals >= 0 { return Err("fmt: decimals applies to numbers, not strings".into()); }
                    if percent { return Err("fmt: percent applies to numbers, not strings".into()); }
                    Ok(CrowValue::ok(Value::Str(
                        crow_runtime::display::crow_fmt_str(s, -1, width, &align, &fill, false),
                    )))
                }
                other => Err(format!("fmt: cannot format {}", other.type_name())),
            }
        }
        // --- Conversion builtins ---
        "int" => {
            if args.len() != 1 { return Err("int[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Int(*n as i64))),
                Value::Str(s) => match s.parse::<i64>() {
                    Ok(n) => Ok(CrowValue::ok(Value::Int(n))),
                    Err(_) => Ok(CrowValue::error_with_kind(&format!("cannot convert '{}' to int", s), "parse", trace)),
                },
                Value::Bool(b) => Ok(CrowValue::ok(Value::Int(if *b { 1 } else { 0 }))),
                _ => Ok(CrowValue::error_with_kind("cannot convert to int", "parse", trace)),
            }
        }
        "float" => {
            if args.len() != 1 { return Err("float[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => Ok(CrowValue::ok(Value::Float(*n as f64))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Float(*n))),
                Value::Str(s) => match s.parse::<f64>() {
                    Ok(n) => Ok(CrowValue::ok(Value::Float(n))),
                    Err(_) => Ok(CrowValue::error_with_kind(&format!("cannot convert '{}' to float", s), "parse", trace)),
                },
                Value::Bool(b) => Ok(CrowValue::ok(Value::Float(if *b { 1.0 } else { 0.0 }))),
                _ => Ok(CrowValue::error_with_kind("cannot convert to float", "parse", trace)),
            }
        }
        "str" => {
            if args.len() != 1 { return Err("str[] takes exactly 1 argument".into()); }
            let val = args[0].unwrap_or_propagate()?;
            Ok(CrowValue::ok(Value::Str(format!("{}", val))))
        }
        // --- Collection builtins (mutating: push, pop, sort, reverse, append) ---
        "len" => {
            if args.len() != 1 { return Err("len[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => Ok(CrowValue::ok(Value::Int(items.len() as i64))),
                Value::NumArray(t) => Ok(CrowValue::ok(Value::Int(t.len() as i64))),
                Value::Str(s) => Ok(CrowValue::ok(Value::Int(s.chars().count() as i64))),
                Value::Map(m) => Ok(CrowValue::ok(Value::Int(m.len() as i64))),
                Value::Frame(f) => Ok(CrowValue::ok(Value::Int(f.nrows() as i64))),
                _ => Err("len[] requires an array, string, map, or frame".into()),
            }
        }

        "contains" => {
            if args.len() != 2 { return Err("contains[] takes exactly 2 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    let needle = args[1].unwrap_or_propagate()?;
                    let found = items.iter().any(|item| {
                        if let Ok(v) = item.unwrap_or_propagate() {
                            match (v, needle) {
                                (Value::Int(a), Value::Int(b)) => a == b,
                                (Value::Float(a), Value::Float(b)) => a == b,
                                (Value::Str(a), Value::Str(b)) => a == b,
                                (Value::Bool(a), Value::Bool(b)) => a == b,
                                _ => false,
                            }
                        } else {
                            false
                        }
                    });
                    Ok(CrowValue::ok(Value::Bool(found)))
                }
                Value::NumArray(t) => {
                    let needle = match args[1].unwrap_or_propagate()? {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => return Err("contains[] on numeric array requires a number".into()),
                    };
                    let found = t.to_vec().iter().any(|v| *v == needle);
                    Ok(CrowValue::ok(Value::Bool(found)))
                }
                Value::Str(s) => {
                    match args[1].unwrap_or_propagate()? {
                        Value::Str(sub) => Ok(CrowValue::ok(Value::Bool(s.contains(sub.as_str())))),
                        _ => Err("contains[] with string requires a string needle".into()),
                    }
                }
                _ => Err("contains[] requires an array or string".into()),
            }
        }
        "slice" => {
            if args.len() != 3 { return Err("slice[] takes exactly 3 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                        (Value::Int(start), Value::Int(end)) => {
                            Ok(CrowValue::ok(Value::Array(crow_runtime::core::slice(items, *start, *end))))
                        }
                        _ => Err("slice[] requires integer start and end".into()),
                    }
                }
                Value::NumArray(t) => {
                    match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                        (Value::Int(start), Value::Int(end)) => {
                            Ok(CrowValue::ok(Value::NumArray(crow_runtime::core::slice_tensor(t, *start, *end))))
                        }
                        _ => Err("slice[] requires integer start and end".into()),
                    }
                }
                Value::Str(s) => {
                    match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                        (Value::Int(start), Value::Int(end)) => {
                            Ok(CrowValue::ok(Value::Str(crow_runtime::core::slice_str(s, *start, *end))))
                        }
                        _ => Err("slice[] requires integer start and end".into()),
                    }
                }
                _ => Err("slice[] requires an array or string".into()),
            }
        }
        // --- Map builtins (non-mutating) ---
        "keys" => {
            if args.len() != 1 { return Err("keys[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Map(m) => {
                    let key_list: Vec<CrowValue> = m.keys()
                        .map(|k| CrowValue::ok(Value::Str(k.clone())))
                        .collect();
                    Ok(CrowValue::ok(Value::Array(key_list)))
                }
                _ => Err("keys[] requires a map".into()),
            }
        }
        "values" => {
            if args.len() != 1 { return Err("values[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Map(m) => {
                    let val_list: Vec<CrowValue> = m.values().cloned().collect();
                    Ok(CrowValue::ok(Value::Array(val_list)))
                }
                _ => Err("values[] requires a map".into()),
            }
        }
        "has" => {
            if args.len() != 2 { return Err("has[] takes exactly 2 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Map(m) => {
                    let key_str = match args[1].unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        _ => return Err("has[] key must be a string".into()),
                    };
                    Ok(CrowValue::ok(Value::Bool(m.contains_key(&key_str))))
                }
                _ => Err("has[] requires a map".into()),
            }
        }
        // --- Immutable collection builtins (return new values) ---
        "push" => {
            if args.len() != 2 { return Err("push[] takes exactly 2 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    Ok(CrowValue::ok(Value::Array(crow_runtime::core::push(items.clone(), args[1].clone()))))
                }
                Value::NumArray(t) => {
                    // The element of a rank-N tensor is a rank-(N-1) slice:
                    // scalar for 1-D, row for 2-D. Shape and type checked.
                    match args[1].unwrap_or_propagate()? {
                        Value::Int(n) => Ok(CrowValue::ok(Value::NumArray(t.push_scalar(*n as f64)?))),
                        Value::Float(f) => Ok(CrowValue::ok(Value::NumArray(t.push_scalar(*f)?))),
                        Value::NumArray(row) => Ok(CrowValue::ok(Value::NumArray(t.push_row(row)?))),
                        other => Err(format!(
                            "arrays are homogeneous — cannot add {} to a numeric array",
                            other.type_name()
                        )),
                    }
                }
                _ => Err("push[] requires an array".into()),
            }
        }
        "pop" => {
            if args.len() != 1 { return Err("pop[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    Ok(CrowValue::ok(Value::Array(crow_runtime::core::pop(items.clone()))))
                }
                Value::NumArray(t) => {
                    let mut data = t.to_vec();
                    data.pop();
                    let len = data.len();
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                }
                _ => Err("pop[] requires an array".into()),
            }
        }
        "sort" => {
            if args.len() < 1 || args.len() > 2 { return Err("sort[] takes 1 or 2 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    Ok(CrowValue::ok(Value::Array(crow_runtime::core::sort_by(items.clone(), value_cmp))))
                }
                Value::NumArray(t) => {
                    let mut data = t.to_vec();
                    data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let len = data.len();
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                }
                _ => Err("sort[] requires an array".into()),
            }
        }
        "reverse" => {
            if args.len() != 1 { return Err("reverse[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Array(items) => {
                    Ok(CrowValue::ok(Value::Array(crow_runtime::core::reverse(items.clone()))))
                }
                Value::NumArray(t) => {
                    let mut data = t.to_vec();
                    data.reverse();
                    let len = data.len();
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                }
                Value::Str(s) => {
                    Ok(CrowValue::ok(Value::Str(s.chars().rev().collect())))
                }
                _ => Err("reverse[] requires an array or string".into()),
            }
        }
        "append" => {
            Err("append was removed — use concat[a, b] (one merge operation, see language-spec.md)".into())
        }
        // concat is the single collection-merge operation: plain arrays
        // (axis 0 implied) and tensors (optional axis). Replaces append
        // and the former math-module concat.
        "concat" => {
            if args.len() < 2 || args.len() > 3 { return Err("concat[] takes 2-3 arguments (a, b[, axis])".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(a), Value::Str(b)) => {
                    if args.len() == 3 {
                        return Err("concat: axis applies to tensors only".into());
                    }
                    Ok(CrowValue::ok(Value::Str(format!("{}{}", a, b))))
                }
                (Value::Array(items), Value::Array(other)) => {
                    if args.len() == 3 {
                        return Err("concat: axis applies to tensors only".into());
                    }
                    Ok(CrowValue::ok(Value::Array(crow_runtime::core::concat(items.clone(), other))))
                }
                (Value::NumArray(a), Value::NumArray(b)) => {
                    let axis = if args.len() == 3 {
                        match args[2].unwrap_or_propagate()? {
                            Value::Int(n) => *n as usize,
                            _ => return Err("concat: axis must be an int".into()),
                        }
                    } else { 0 };
                    Ok(CrowValue::ok(Value::NumArray(a.concat(b, axis))))
                }
                _ => Err("concat[] requires two arrays of the same kind".into()),
            }
        }
        "set" => {
            if args.len() != 3 { return Err("set[] takes exactly 3 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Map(m) => {
                    let key_str = match args[1].unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        _ => return Err("set[] key must be a string".into()),
                    };
                    check_map_homogeneous(m, &args[2])?;
                    Ok(CrowValue::ok(Value::Map(crow_runtime::core::map_set(m.clone(), key_str, args[2].clone()))))
                }
                _ => Err("set[] requires a map".into()),
            }
        }
        "del" => {
            if args.len() != 2 { return Err("del[] takes exactly 2 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Map(m) => {
                    let key_str = match args[1].unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        _ => return Err("del[] key must be a string".into()),
                    };
                    Ok(CrowValue::ok(Value::Map(crow_runtime::core::map_del(m.clone(), &key_str))))
                }
                _ => Err("del[] requires a map".into()),
            }
        }
        // --- CLI / IO builtins ---
        "args" => {
            if !args.is_empty() { return Err("args[] takes no arguments".into()); }
            let cli_args: Vec<CrowValue> = std::env::args()
                .skip(1) // skip binary name
                .skip_while(|a| a == "run" || a == "-i" || a.ends_with(".co") || a.ends_with(".coc") || a.ends_with(".crow") || a.ends_with(".crowc"))
                .map(|a| CrowValue::ok(Value::Str(a)))
                .collect();
            Ok(CrowValue::ok(Value::Array(cli_args)))
        }
        "input" => {
            if args.len() > 1 { return Err("input[] takes at most one argument (a prompt)".into()); }
            let prompt = args.first().map(|p| format!("{}", p.value));
            match crow_runtime::core::read_line(prompt.as_deref()) {
                Ok(line) => Ok(CrowValue::ok(Value::Str(line))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
            }
        }
        // --- System builtins ---
        "rand" => {
            if args.len() > 2 { return Err("rand[] takes 0, 1, or 2 arguments".into()); }
            // the program's RNG, the one seed[n] seeds (core.rs)
            if args.is_empty() {
                Ok(CrowValue::ok(Value::Float(crow_runtime::core::rand_float())))
            } else if args.len() == 1 {
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(crow_runtime::core::rand_int(*n)?))),
                    _ => Err("rand[n] requires an integer".into()),
                }
            } else {
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Int(min), Value::Int(max)) => Ok(CrowValue::ok(Value::Int(crow_runtime::core::rand_between(*min, *max)?))),
                    _ => Err("rand[min, max] requires two integers".into()),
                }
            }
        }
        "time" => {
            if !args.is_empty() { return Err("time[] takes no arguments".into()); }
            match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => Ok(CrowValue::ok(Value::Float(d.as_secs_f64()))),
                Err(_) => Ok(CrowValue::error("system time before unix epoch", trace)),
            }
        }
        "sleep" => {
            if args.len() != 1 { return Err("sleep[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(ms) => {
                    std::thread::sleep(std::time::Duration::from_millis(*ms as u64));
                    Ok(CrowValue::done())
                }
                Value::Float(s) => {
                    std::thread::sleep(std::time::Duration::from_secs_f64(*s));
                    Ok(CrowValue::done())
                }
                _ => Err("sleep[] requires a number (milliseconds as int, or seconds as float)".into()),
            }
        }
        "env" => {
            if args.is_empty() || args.len() > 2 { return Err("env[] takes 1 or 2 arguments".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(name) => {
                    match std::env::var(name) {
                        Ok(val) => Ok(CrowValue::ok(Value::Str(val))),
                        Err(_) => {
                            if args.len() == 2 {
                                // env[key, default] — return default
                                Ok(args[1].clone())
                            } else {
                                // env[key] — return error-value
                                Ok(CrowValue::error_with_kind(
                                    &format!("environment variable '{}' not set", name),
                                    "env",
                                    trace,
                                ))
                            }
                        }
                    }
                }
                _ => Err("env[] requires a string key".into()),
            }
        }
        "exit" => {
            if args.len() > 1 { return Err("exit[] takes 0 or 1 arguments".into()); }
            let code = if args.is_empty() {
                0
            } else {
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => *n as i32,
                    _ => return Err("exit[] requires an integer".into()),
                }
            };
            std::process::exit(code);
        }
        // --- File I/O builtins ---
        "read" => {
            if args.len() != 1 { return Err("read[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(path) => {
                    match crow_runtime::io::fs_read(path) {
                        Ok(content) => Ok(CrowValue::ok(Value::Str(content))),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("read[] requires a string path".into()),
            }
        }
        "write" => {
            if args.len() != 2 { return Err("write[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(path), Value::Str(content)) => {
                    match crow_runtime::io::fs_write(path, content) {
                        Ok(_) => Ok(CrowValue::done()),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("write[] requires a string path and string content".into()),
            }
        }
        // --- File system builtins ---
        "exists" => {
            if args.len() != 1 { return Err("exists[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(path) => Ok(CrowValue::ok(Value::Bool(crow_runtime::io::fs_exists(path)))),
                _ => Err("exists[] requires a string path".into()),
            }
        }
        "mkdir" => {
            if args.len() != 1 { return Err("mkdir[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(path) => {
                    match crow_runtime::io::fs_mkdir(path) {
                        Ok(_) => Ok(CrowValue::done()),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("mkdir[] requires a string path".into()),
            }
        }
        "ls" => {
            if args.len() != 1 { return Err("ls[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(path) => {
                    match crow_runtime::io::fs_ls(path) {
                        Ok(names) => Ok(CrowValue::ok(Value::Array(
                            names.into_iter().map(|n| CrowValue::ok(Value::Str(n))).collect(),
                        ))),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("ls[] requires a string path".into()),
            }
        }
        "rm" => {
            if args.len() != 1 { return Err("rm[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(path) => {
                    match crow_runtime::io::fs_rm(path) {
                        Ok(_) => Ok(CrowValue::done()),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("rm[] requires a string path".into()),
            }
        }
        "mv" => {
            if args.len() != 2 { return Err("mv[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(src), Value::Str(dst)) => {
                    match crow_runtime::io::fs_mv(src, dst) {
                        Ok(_) => Ok(CrowValue::done()),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("mv[] requires two string paths".into()),
            }
        }
        "cp" => {
            if args.len() != 2 { return Err("cp[] takes exactly 2 arguments".into()); }
            match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                (Value::Str(src), Value::Str(dst)) => {
                    match crow_runtime::io::fs_cp(src, dst) {
                        Ok(_) => Ok(CrowValue::done()),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "io", trace)),
                    }
                }
                _ => Err("cp[] requires two string paths".into()),
            }
        }
        // --- JSON builtins ---
        "json" => {
            if args.len() != 1 { return Err("json[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(s) => {
                    match serde_json::from_str::<serde_json::Value>(s) {
                        Ok(jval) => Ok(json_to_crow(trace, &jval)),
                        Err(e) => Ok(CrowValue::error_with_kind(&format!("invalid JSON: {}", e), "parse", trace)),
                    }
                }
                _ => Err("json[] requires a string argument".into()),
            }
        }
        "encode" => {
            if args.len() != 1 { return Err("encode[] takes exactly 1 argument".into()); }
            let jval = crow_to_json(trace, &args[0]);
            match jval {
                Ok(v) => Ok(CrowValue::ok(Value::Str(v.to_string()))),
                Err(e) => Ok(CrowValue::error(&e, trace)),
            }
        }
        // --- Shell builtin ---
        "shell" => {
            if args.len() != 1 { return Err("shell[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Str(cmd) => {
                    match crow_runtime::core::shell(cmd) {
                        Ok(sh) => {
                            let mut map = std::collections::BTreeMap::new();
                            map.insert("status".to_string(), CrowValue::ok(Value::Int(sh.status)));
                            map.insert("stdout".to_string(), CrowValue::ok(Value::Str(sh.stdout)));
                            map.insert("stderr".to_string(), CrowValue::ok(Value::Str(sh.stderr)));
                            Ok(CrowValue::ok(Value::Map(map)))
                        }
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "shell", trace)),
                    }
                }
                _ => Err("shell[] requires a string command".into()),
            }
        }
        // --- Hashing builtin ---
        "hash" => {
            if args.len() != 2 { return Err("hash[] takes exactly 2 arguments (string, algorithm)".into()); }
            let s = match args[0].unwrap_or_propagate()? {
                Value::Str(s) => s.clone(),
                _ => return Err("hash[] first argument must be a string".into()),
            };
            let algo = match args[1].unwrap_or_propagate()? {
                Value::Str(a) => a.clone(),
                _ => return Err("hash[] second argument must be a string algorithm".into()),
            };
            // one implementation for both modes (crow_runtime::hash)
            match crow_runtime::hash::hash(&s, &algo) {
                Ok(h) => Ok(CrowValue::ok(Value::Str(h))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "hash", trace)),
            }
        }
        "matches" => {
            if args.len() != 2 { return Err("matches[] takes exactly 2 arguments (string, pattern)".into()); }
            let s = match args[0].unwrap_or_propagate()? {
                Value::Str(s) => s.clone(),
                _ => return Err("matches[] first argument must be a string".into()),
            };
            let pattern = match args[1].unwrap_or_propagate()? {
                Value::Str(p) => p.clone(),
                _ => return Err("matches[] second argument must be a string pattern".into()),
            };
            match crow_runtime::text::matches(&s, &pattern) {
                Ok(b) => Ok(CrowValue::ok(Value::Bool(b))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "regex", trace)),
            }
        }
        "sub" => {
            if args.len() != 3 { return Err("sub[] takes exactly 3 arguments (string, pattern, replacement)".into()); }
            let s = match args[0].unwrap_or_propagate()? {
                Value::Str(s) => s.clone(),
                _ => return Err("sub[] first argument must be a string".into()),
            };
            let pattern = match args[1].unwrap_or_propagate()? {
                Value::Str(p) => p.clone(),
                _ => return Err("sub[] second argument must be a string pattern".into()),
            };
            let replacement = match args[2].unwrap_or_propagate()? {
                Value::Str(r) => r.clone(),
                _ => return Err("sub[] third argument must be a string replacement".into()),
            };
            match crow_runtime::text::sub(&s, &pattern, &replacement) {
                Ok(out) => Ok(CrowValue::ok(Value::Str(out))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "regex", trace)),
            }
        }
        // --- Date/time builtins ---
        "now" => {
            if !args.is_empty() { return Err("now[] takes no arguments".into()); }
            Ok(CrowValue::ok(Value::Float(crow_runtime::moments::now())))
        }
        "moment" => {
            if args.len() != 7 { return Err("moment[] takes year, month, day + named hour, minute, second, tz".into()); }
            let geti = |i: usize| -> Result<i64, String> {
                match &args[i].value { Value::Int(n) => Ok(*n), Value::Float(f) => Ok(*f as i64), _ => Err("moment[]: components must be numbers".into()) }
            };
            let second = match &args[5].value { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => return Err("moment[]: second must be a number".into()) };
            let tz = match &args[6].value { Value::Str(s) => s.clone(), _ => return Err("moment[]: tz must be a string".into()) };
            match crow_runtime::moments::moment(geti(0)?, geti(1)?, geti(2)?, geti(3)?, geti(4)?, second, &tz) {
                Ok(t) => Ok(CrowValue::ok(Value::Float(t))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "time", trace)),
            }
        }
        "calendar" => {
            if args.len() != 2 { return Err("calendar[] takes a moment + named tz".into()); }
            let t = match &args[0].value { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => return Err("calendar[]: takes a moment (a number)".into()) };
            let tz = match &args[1].value { Value::Str(s) => s.clone(), _ => return Err("calendar[]: tz must be a string".into()) };
            match crow_runtime::moments::calendar(t, &tz) {
                Ok(fields) => {
                    let mut map = std::collections::BTreeMap::new();
                    for (k, v) in fields { map.insert(k.to_string(), CrowValue::ok(Value::Int(v))); }
                    Ok(CrowValue::ok(Value::Map(map)))
                }
                Err(e) => Ok(CrowValue::error_with_kind(&e, "time", trace)),
            }
        }
        "parse_time" => {
            if args.len() != 2 { return Err("parse_time[] takes a string + named tz".into()); }
            let text = match &args[0].value { Value::Str(s) => s.clone(), _ => return Err("parse_time[]: takes a string".into()) };
            let tz = match &args[1].value { Value::Str(s) => s.clone(), _ => return Err("parse_time[]: tz must be a string".into()) };
            match crow_runtime::moments::parse_time(&text, &tz) {
                Ok(t) => Ok(CrowValue::ok(Value::Float(t))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "time", trace)),
            }
        }
        "assert" => {
            if args.is_empty() || args.len() > 2 {
                return Err("assert[] takes 1 or 2 arguments".to_string());
            }
            let cond = &args[0];
            // An assert is a decision — errors have no truth value
            if !cond.ok {
                return Err(format!(
                    "cannot branch on an error value — {}",
                    cond.reason
                ));
            }
            let passed = match &cond.value {
                Value::Bool(b) => *b,
                Value::Int(n) => *n != 0,
                Value::Str(s) => !s.is_empty(),
                Value::Array(l) => !l.is_empty(),
                Value::Map(m) => !m.is_empty(),
                _ => true,
            };
            if !passed {
                let msg = if args.len() == 2 {
                    format!("assertion failed: {}", args[1].value)
                } else {
                    "assertion failed".to_string()
                };
                return Err(msg);
            }
            Ok(CrowValue::done())
        }
        "error" => {
            // error[reason] or error[reason, kind]
            if args.is_empty() || args.len() > 2 {
                return Err("error[] takes 1 or 2 arguments".to_string());
            }
            let reason = match args[0].unwrap_or_propagate()? {
                Value::Str(s) => s.clone(),
                other => format!("{}", other),
            };
            let kind = if args.len() == 2 {
                match args[1].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    other => format!("{}", other),
                }
            } else {
                String::new()
            };
            Ok(CrowValue::error_with_kind(&reason, &kind, trace))
        }
        _ => Err(format!("{} is not a builtin", name)),
    }
}

/// How `sort[]` orders the interpreter's values: ints, floats and strings
/// by value, anything else as equal (the runtime does the sorting,
/// core.rs; this is the ordering it is handed).
pub fn value_cmp(a: &CrowValue, b: &CrowValue) -> std::cmp::Ordering {
    match (a.unwrap_or_propagate().ok(), b.unwrap_or_propagate().ok()) {
        (Some(Value::Int(x)), Some(Value::Int(y))) => x.cmp(y),
        (Some(Value::Float(x)), Some(Value::Float(y))) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(Value::Str(x)), Some(Value::Str(y))) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

/// Maps are dictionaries: every value must have the same type
/// (spec Design Note "Maps Are Dictionaries, Not Records"). This is the
/// runtime check matching what the compiled program's typed maps enforce.
pub fn check_map_homogeneous(
    map: &std::collections::BTreeMap<String, CrowValue>,
    new_val: &CrowValue,
) -> Result<(), String> {
    if let Some(existing) = map.values().next() {
        let (have_key, have_name) = map_value_kind(&existing.value);
        let (add_key, add_name) = map_value_kind(&new_val.value);
        if have_key != add_key {
            return Err(format!(
                "map values must all be the same type — this map holds {} values, cannot add {} (use a struct for mixed named data)",
                have_name, add_name
            ));
        }
    }
    Ok(())
}

/// Convert a serde_json::Value to a CrowValue.
/// JSON null becomes an error-value with reason "null".
pub fn json_to_crow(trace: &str, jval: &serde_json::Value) -> CrowValue {
    match jval {
        serde_json::Value::Null => CrowValue::error_with_kind("null", "parse", trace),
        serde_json::Value::Bool(b) => CrowValue::ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CrowValue::ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                CrowValue::ok(Value::Float(f))
            } else {
                CrowValue::ok(Value::Int(0))
            }
        }
        serde_json::Value::String(s) => CrowValue::ok(Value::Str(s.clone())),
        serde_json::Value::Array(arr) => {
            // Check if all elements are numeric
            let all_numeric = !arr.is_empty() && arr.iter().all(|v| v.is_number());
            if all_numeric {
                let data: Vec<f64> = arr.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
                let len = data.len();
                CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len])))
            } else {
                let items: Vec<CrowValue> = arr.iter().map(|v| json_to_crow(trace, v)).collect();
                CrowValue::ok(Value::Array(items))
            }
        }
        serde_json::Value::Object(obj) => {
            let mut map = BTreeMap::new();
            for (k, v) in obj {
                map.insert(k.clone(), json_to_crow(trace, v));
            }
            CrowValue::ok(Value::Map(map))
        }
    }
}

/// Convert a CrowValue to a serde_json::Value for encoding.
pub fn crow_to_json(trace: &str, val: &CrowValue) -> Result<serde_json::Value, String> {
    if val.is_error() {
        // Error-values encode as JSON null (round-trip with json[])
        return Ok(serde_json::Value::Null);
    }
    match &val.value {
        Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
        // JSON has no NaN or infinity: those encode as null (json.rs)
        Value::Float(f) => Ok(crow_runtime::json::float_to_json(*f)),
        Value::Str(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Array(items) => {
            let arr: Result<Vec<serde_json::Value>, String> = items.iter().map(|v| crow_to_json(trace, v)).collect();
            Ok(serde_json::Value::Array(arr?))
        }
        Value::NumArray(t) => Ok(crow_runtime::json::tensor_to_json(t)),
        Value::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                obj.insert(k.clone(), crow_to_json(trace, v)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        Value::Done => Ok(serde_json::Value::Null),
        _ => Err(format!("cannot encode {} to JSON", val.value.type_name())),
    }
}

/// Assign to a struct field, navigating nested field access chains
/// Coarse dynamic type key for map-homogeneity checks (comparison) and a
/// user-facing name (display). int/float unify (like arrays); structs are
/// distinct per name.
pub fn map_value_kind(v: &Value) -> (String, String) {
    match v {
        Value::Int(_) => ("number".to_string(), "int".to_string()),
        Value::Float(_) | Value::Dual(_, _) => ("number".to_string(), "float".to_string()),
        Value::Str(_) => ("str".to_string(), "str".to_string()),
        Value::Bool(_) => ("bool".to_string(), "bool".to_string()),
        Value::Array(_) | Value::NumArray(_) => ("array".to_string(), "array".to_string()),
        Value::Map(_) => ("map".to_string(), "map".to_string()),
        Value::StructInstance(si) => (format!("struct {}", si.name), format!("struct {}", si.name)),
        Value::Fn(_) | Value::GradFn(_) => ("fn".to_string(), "fn".to_string()),
        other => (other.type_name().to_string(), other.type_name().to_string()),
    }
}
