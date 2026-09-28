//! The builtins that need no interpreter state, in one place: the
//! interpreter (`ir_interp.rs`, ir-design.md §5) calls nothing else for
//! these names, and each arm calls the runtime the compiled mode links
//! where it owns the operation. Each arm reads
//! its arguments as values and answers a value; `trace` is the caller's
//! position for the error values that carry one.

use crate::value::*;
use crow_runtime::tensor::CrowTensor;
use std::collections::BTreeMap;

/// The names answered here (a builtin with a row in crate::table is
/// answered by its row, before this).
pub const HANDLED: &[&str] = &["log", "type", "map", "abs", "sqrt", "floor", "ceil", "round", "pad", "str", "append", "args", "input", "time", "sleep", "env", "exit", "json", "encode", "now", "assert", "error"];

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
                    print!("error: {}", arg.reason());
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
        // --- Math builtins ---
        "abs" => {
            if args.len() != 1 { return Err("abs[] takes exactly 1 argument".into()); }
            match args[0].unwrap_or_propagate()? {
                Value::Int(n) => crow_runtime::stops::abs(*n).map(|n| CrowValue::ok(Value::Int(n))),
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
        // --- String builtins ---
        "pad" => {
            Err("pad was removed — use fmt[s, width = n]".into())
        }
        // fmt: value formatting via the shared display kernel. The lowering
        // pass guarantees exactly 7 positional args (sentinel defaults).
        // --- Conversion builtins ---
        "str" => {
            if args.len() != 1 { return Err("str[] takes exactly 1 argument".into()); }
            let val = args[0].unwrap_or_propagate()?;
            Ok(CrowValue::ok(Value::Str(format!("{}", val))))
        }
        "append" => {
            Err("append was removed — use concat[a, b] (one merge operation, see language-spec.md)".into())
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
        // --- Hashing builtin ---
        // --- Date/time builtins ---
        "now" => {
            if !args.is_empty() { return Err("now[] takes no arguments".into()); }
            Ok(CrowValue::ok(Value::Float(crow_runtime::moments::now())))
        }
        "assert" => {
            if args.is_empty() || args.len() > 2 {
                return Err("assert[] takes 1 or 2 arguments".to_string());
            }
            let cond = &args[0];
            // An assert is a decision — errors have no truth value
            if !cond.is_ok() {
                return Err(format!(
                    "cannot branch on an error value — {}",
                    cond.reason()
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
                if crate::rehearsal::unheld(format!("{}: {}", trace, msg)) {
                    return Ok(CrowValue::done());
                }
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
/// JSON null becomes an error-value with reason "null": the whole value's
/// carries the parse's place, an element's none (the compiled program
/// reads an element's null where it is indexed, with no place either).
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
                CrowValue::ok(Value::NumArray(Box::new(CrowTensor::from_vec(data, vec![len]))))
            } else {
                let items: Vec<CrowValue> = arr.iter().map(|v| json_to_crow("", v)).collect();
                CrowValue::ok(Value::Array(items))
            }
        }
        serde_json::Value::Object(obj) => {
            let mut map = BTreeMap::new();
            for (k, v) in obj {
                map.insert(k.clone(), json_to_crow("", v));
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
