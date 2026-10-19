use crate::value::{CrowValue, Value};
use crate::type_infer::Ty;
use crate::modules::CrowModule;

pub struct DbModule;

/// The database's verbs: forms of their own (crate::table::FORMS), which
/// the compiler answers.
pub(crate) static DB_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_forms("db"));

/// The modules a program that imports `db` carries: the drivers written
/// in Crow its connection URLs name — every driver when a URL is not a
/// literal the source spells, or no connection is made (the tooling's
/// `use db` alone).
pub fn drivers_for(urls: &[String]) -> Vec<&'static str> {
    // (the drivers and their schemes are the manifest's, crate::modules::MODULES)
    let all: Vec<(&'static str, &'static str)> = crate::modules::MODULES
        .iter()
        .filter_map(|m| m.driver.as_ref().map(|d| (m.path, d.schemes)))
        .flat_map(|(path, schemes)| schemes.iter().map(move |s| (*s, path)))
        .collect();
    let spelled = |u: &String| all.iter().any(|(s, _)| u.starts_with(s)) || u.starts_with("sqlite://");
    let mut out: Vec<&'static str> = Vec::new();
    let wanted: Vec<&'static str> = if urls.is_empty() || urls.iter().any(|u| !spelled(u)) {
        all.iter().map(|(_, m)| *m).collect()
    } else {
        all.iter().filter(|(s, _)| urls.iter().any(|u| u.starts_with(s))).map(|(_, m)| *m).collect()
    };
    for m in wanted {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

impl CrowModule for DbModule {
    fn name(&self) -> &str { "db" }

    fn functions(&self) -> &[&str] { &DB_FUNCTIONS }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        // Propagate errors
        for a in args {
            if !a.is_ok() { return Ok(a.clone()); }
        }
        call_db(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[Ty]) -> Result<String, String> {
        emit_db_rust(name, arg_strs, arg_types)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> Ty {
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
            // connect[url] or connect[url, pool_size]; the lowering hands a
            // connection the program declares tables on the statements that
            // declare them, third (a pool of 0: the backend's)
            if args.is_empty() || args.len() > 3 {
                return Err("connect[] takes 1 or 2 arguments".into());
            }
            let url = to_str(&args[0], "connect")?;
            let pool_size = match args.get(1).map(|a| &a.value) {
                None => None,
                Some(Value::Int(n)) => Some(*n as usize),
                Some(_) => return Err("connect[] pool_size must be an integer".into()),
            };
            // in a rehearsal nothing connects: the rehearsal's own database
            // answers (rehearsal.rs)
            if crate::rehearsal::active().is_some() {
                return Ok(CrowValue::ok(Value::DbConn(crate::rehearsal::connect(url))));
            }
            let declared: Vec<&str> = match args.get(2).map(|a| &a.value) {
                Some(Value::Array(items)) => items.iter().filter_map(|i| if let Value::Str(s) = &i.value { Some(s.as_str()) } else { None }).collect(),
                _ => Vec::new(),
            };
            let handle = crow_db::schema::connect(url, pool_size, &declared)?;
            Ok(CrowValue::ok(Value::DbConn(handle)))
        }
        "query" | "__query_write" => {
            // query[conn, sql] or query[conn, sql, params]: every statement,
            // its rows as a frame (an empty one where it answers none)
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
                Ok(frame) => Ok(CrowValue::ok(Value::Frame(Box::new(frame)))),
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
        "describe" => {
            // describe[conn, sql]: the columns a statement answers, as a
            // frame of name and kind, without running it
            if args.len() != 2 {
                return Err("describe[] takes 2 arguments".into());
            }
            let conn = to_db(&args[0], "describe")?;
            let sql = to_str(&args[1], "describe")?;
            match conn.describe(sql) {
                Ok(cols) => Ok(CrowValue::ok(Value::Frame(Box::new(crow_db::driven::describe_frame(&cols))))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "db", "")),
            }
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
            for item in items.iter() {
                let json = match &item.value {
                    Value::Int(n) => serde_json::json!(*n),
                    Value::Float(f) => serde_json::json!(*f),
                    Value::Str(s) => serde_json::json!(s),
                    Value::Bool(b) => serde_json::json!(*b),
                    // a list parameter: `IN ?` expands to one placeholder per element
                    Value::Array(_) | Value::NumArray(_) => serde_json::Value::Array(crow_value_to_json_params(item)?),
                    _ => return Err(format!("cannot bind {} as SQL param", item.value.type_name())),
                };
                params.push(json);
            }
            Ok(params)
        }
        // An all-numeric literal like [18.95] or [100, 50] is a tensor in the
        // interpreter; bind its elements as numbers (whole values as ints,
        // matching what compiled mode emits for the same literal).
        Value::NumArray(t) => {
            let num = |f: f64| if f.is_finite() && f == (f as i64) as f64 { serde_json::json!(f as i64) } else { serde_json::json!(f) };
            let shape = t.shape();
            // a 2-D literal like [[1, 3]] is a list of list parameters
            if shape.len() == 2 {
                let (rows, cols) = (shape[0], shape[1]);
                let flat = t.to_vec();
                return Ok((0..rows).map(|r| serde_json::Value::Array((0..cols).map(|c| num(flat[r * cols + c])).collect())).collect());
            }
            Ok(t.to_vec().into_iter().map(num).collect())
        }
        _ => Err("SQL params must be an array".into()),
    }
}

fn emit_db_rust(_name: &str, _arg_strs: &[String], _arg_types: &[Ty]) -> Result<String, String> {
    // the Rust backend prints database calls itself (emit_rs/builtins.rs:
    // the parameter guard, the typed cells); one reaching the registry is
    // a backend gap, reported where it is printed
    Err(format!("db.{}: the Rust backend prints database calls itself, and missed this one", _name))
}

fn db_type_of(name: &str) -> Ty {
    match name {
        "connect" => Ty::Handle(&crate::table::DB),
        "query" | "__query_write" => Ty::Result(Box::new(Ty::Frame(None))),
        "describe" => Ty::Result(Box::new(Ty::Frame(None))),
        "insert" => Ty::Int,
        "transact" => Ty::Unknown,
        _ => Ty::Unknown,
    }
}

