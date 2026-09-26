use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;
use std::collections::BTreeMap;

pub struct IoModule;

/// io module functions: csv (parse CSV files), lines (read file as lines).
/// Note: read[] and write[] are already builtins — no duplication (rule 17).
pub(crate) const IO_FUNCTIONS: &[&str] = &["csv", "lines"];

impl CrowModule for IoModule {
    fn name(&self) -> &str { "io" }

    fn functions(&self) -> &[&str] { IO_FUNCTIONS }

    fn constants(&self) -> Vec<(&str, CrowValue)> { vec![] }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        for a in args {
            if !a.is_ok() {
                return Ok(a.clone());
            }
        }
        call_io(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        emit_io_rust(name, arg_strs)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        io_type_of(name)
    }
}

fn to_str<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        _ => Err(format!("{}: expected string argument", fname)),
    }
}

fn call_io(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "csv" => {
            if args.len() != 1 { return Err("csv: expected 1 argument (path)".into()); }
            let path = to_str(&args[0], name)?;
            match crow_runtime::io::parse_csv(path) {
                Ok(rows) => {
                    let arr: Vec<CrowValue> = rows.into_iter().map(|row| {
                        let map: BTreeMap<String, CrowValue> = row.into_iter()
                            .map(|(k, v)| (k, CrowValue::ok(Value::Str(v))))
                            .collect();
                        CrowValue::ok(Value::Map(map))
                    }).collect();
                    Ok(CrowValue::ok(Value::Array(arr)))
                }
                Err(e) => Ok(CrowValue::error_with_kind(&e, "io", "")),
            }
        }
        "lines" => {
            if args.len() != 1 { return Err("lines: expected 1 argument (path)".into()); }
            let path = to_str(&args[0], name)?;
            match crow_runtime::io::read_lines(path) {
                Ok(ls) => {
                    let arr: Vec<CrowValue> = ls.into_iter()
                        .map(|l| CrowValue::ok(Value::Str(l)))
                        .collect();
                    Ok(CrowValue::ok(Value::Array(arr)))
                }
                Err(e) => Ok(CrowValue::error_with_kind(&e, "io", "")),
            }
        }
        _ => Err(format!("{}: unknown io function", name)),
    }
}

fn emit_io_rust(name: &str, arg_strs: &[String]) -> Result<String, String> {
    let result = match name {
        "csv" => format!("{{ match crow_runtime::io::parse_csv(&{}) {{ Ok(rows) => {{ let arr: Vec<std::collections::BTreeMap<String, String>> = rows.into_iter().map(|r| r).collect(); CrowResult {{ value: arr, ok: true, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }} }}, Err(e) => CrowResult {{ value: vec![], ok: false, reason: e, kind: \"io\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0]),
        "lines" => format!("{{ match crow_runtime::io::read_lines(&{}) {{ Ok(ls) => CrowResult {{ value: ls, ok: true, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(e) => CrowResult {{ value: vec![], ok: false, reason: e, kind: \"io\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0]),
        _ => return Err(format!("{}: unknown io function for the Rust backend", name)),
    };
    Ok(result)
}

fn io_type_of(name: &str) -> CrowType {
    match name {
        "csv" => CrowType::Result(Box::new(CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))))),
        "lines" => CrowType::Result(Box::new(CrowType::Array(Box::new(CrowType::Str)))),
        _ => CrowType::Unknown,
    }
}

