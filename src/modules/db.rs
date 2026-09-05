use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;

pub struct DbModule;

pub(crate) const DB_FUNCTIONS: &[&str] = &[
    "connect", "query", "exec", "insert", "migrate", "transact",
];

impl CrowModule for DbModule {
    fn name(&self) -> &str { "db" }

    fn functions(&self) -> &[&str] { DB_FUNCTIONS }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        // Propagate errors
        for a in args {
            if !a.ok { return Ok(a.clone()); }
        }
        call_db(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        emit_db_rust(name, arg_strs, arg_types)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        db_type_of(name)
    }
}

fn to_str<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        _ => Err(format!("{}: expected string argument", fname)),
    }
}

fn to_db<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a crow_db::DbHandle, String> {
    match &v.value {
        Value::DbConn(h) => Ok(h),
        _ => Err(format!("{}: expected db connection, got {}", fname, v.value.type_name())),
    }
}

fn call_db(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "connect" => {
            // connect[url] or connect[url, pool_size]
            if args.is_empty() || args.len() > 2 {
                return Err("connect[] takes 1 or 2 arguments".into());
            }
            let url = to_str(&args[0], "connect")?;
            let pool_size = if args.len() == 2 {
                match &args[1].value {
                    Value::Int(n) => Some(*n as usize),
                    _ => return Err("connect[] pool_size must be an integer".into()),
                }
            } else {
                None
            };
            let handle = crow_db::connect(url, pool_size)?;
            Ok(CrowValue::ok(Value::DbConn(handle)))
        }
        "query" => {
            // query[conn, sql] or query[conn, sql, params]
            if args.len() < 2 || args.len() > 3 {
                return Err("query[] takes 2 or 3 arguments".into());
            }
            let conn = to_db(&args[0], "query")?;
            let sql = to_str(&args[1], "query")?;
            let params = if args.len() == 3 {
                crow_value_to_json_params(&args[2])?
            } else {
                vec![]
            };
            match conn.query(sql, &params) {
                Ok(frame) => Ok(CrowValue::ok(Value::Frame(frame))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "db", "")),
            }
        }
        "exec" => {
            // exec[conn, sql] or exec[conn, sql, params]
            if args.len() < 2 || args.len() > 3 {
                return Err("exec[] takes 2 or 3 arguments".into());
            }
            let conn = to_db(&args[0], "exec")?;
            let sql = to_str(&args[1], "exec")?;
            let params = if args.len() == 3 {
                crow_value_to_json_params(&args[2])?
            } else {
                vec![]
            };
            match conn.exec(sql, &params) {
                Ok(count) => Ok(CrowValue::ok(Value::Int(count))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "db", "")),
            }
        }
        "insert" => {
            // insert[conn, table, frame]
            if args.len() != 3 {
                return Err("insert[] takes 3 arguments".into());
            }
            let conn = to_db(&args[0], "insert")?;
            let table = to_str(&args[1], "insert")?;
            let frame = match &args[2].value {
                Value::Frame(f) => f,
                _ => return Err("insert[] data must be a frame".into()),
            };
            let count = conn.insert(table, frame)?;
            Ok(CrowValue::ok(Value::Int(count)))
        }
        "migrate" => {
            // migrate[conn, dir]
            if args.len() != 2 {
                return Err("migrate[] takes 2 arguments".into());
            }
            let conn = to_db(&args[0], "migrate")?;
            let dir = to_str(&args[1], "migrate")?;
            let count = conn.migrate(dir)?;
            Ok(CrowValue::ok(Value::Int(count)))
        }
        "transact" => {
            // transact[conn, fn] — handled specially in interpreter
            // because we need to call a Crow function, not a Rust closure
            Err("transact[] must be called at top level — see interpreter".into())
        }
        _ => Err(format!("unknown db function: {}", name)),
    }
}

/// Convert a Crow array of values to JSON params for SQL binding.
fn crow_value_to_json_params(v: &CrowValue) -> Result<Vec<serde_json::Value>, String> {
    match &v.value {
        Value::Array(items) => {
            let mut params = Vec::with_capacity(items.len());
            for item in items {
                let json = match &item.value {
                    Value::Int(n) => serde_json::json!(*n),
                    Value::Float(f) => serde_json::json!(*f),
                    Value::Str(s) => serde_json::json!(s),
                    Value::Bool(b) => serde_json::json!(*b),
                    _ => return Err(format!("cannot bind {} as SQL param", item.value.type_name())),
                };
                params.push(json);
            }
            Ok(params)
        }
        // An all-numeric literal like [18.95] or [100, 50] is a tensor in the
        // interpreter; bind its elements as numbers (whole values as ints,
        // matching what compiled mode emits for the same literal).
        Value::NumArray(t) => Ok(t
            .to_vec()
            .into_iter()
            .map(|f| {
                if f.is_finite() && f == (f as i64) as f64 {
                    serde_json::json!(f as i64)
                } else {
                    serde_json::json!(f)
                }
            })
            .collect()),
        _ => Err("SQL params must be an array".into()),
    }
}

fn emit_db_rust(_name: &str, _arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
    // TODO: implement transpiler codegen in Step 7
    Err("db transpiler codegen not yet implemented".into())
}

fn db_type_of(name: &str) -> CrowType {
    match name {
        "connect" => CrowType::DbHandle,
        "query" => CrowType::Result(Box::new(CrowType::Frame)),
        "exec" => CrowType::Result(Box::new(CrowType::Int)),
        "insert" => CrowType::Int,
        "migrate" => CrowType::Int,
        "transact" => CrowType::Unknown,
        _ => CrowType::Unknown,
    }
}

