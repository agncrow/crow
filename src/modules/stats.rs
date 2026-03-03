use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;
use std::collections::BTreeMap;

pub struct StatsModule;

const STATS_FUNCTIONS: &[&str] = &[
    "median", "std", "var", "percentile", "zscore", "corr", "cov", "ttest",
];

impl CrowModule for StatsModule {
    fn name(&self) -> &str { "math.stats" }

    fn functions(&self) -> &[&str] { STATS_FUNCTIONS }

    fn constants(&self) -> Vec<(&str, CrowValue)> { vec![] }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        for a in args {
            if !a.ok {
                return Ok(a.clone());
            }
        }
        call_stats(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        emit_stats_rust(name, arg_strs)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        stats_type_of(name)
    }
}

fn call_stats(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "median" => {
            if args.len() != 1 { return Err("median: expected 1 argument".into()); }
            match &args[0].value {
                Value::NumArray(t) => Ok(CrowValue::ok(Value::Float(crow_runtime::stats::median(t)))),
                _ => Err("median: expected numarray".into()),
            }
        }
        "std" => {
            if args.len() != 1 { return Err("std: expected 1 argument".into()); }
            match &args[0].value {
                Value::NumArray(t) => Ok(CrowValue::ok(Value::Float(crow_runtime::stats::std(t)))),
                _ => Err("std: expected numarray".into()),
            }
        }
        "var" => {
            if args.len() != 1 { return Err("var: expected 1 argument".into()); }
            match &args[0].value {
                Value::NumArray(t) => Ok(CrowValue::ok(Value::Float(crow_runtime::stats::var(t)))),
                _ => Err("var: expected numarray".into()),
            }
        }
        "percentile" => {
            if args.len() != 2 { return Err("percentile: expected 2 arguments".into()); }
            match &args[0].value {
                Value::NumArray(t) => {
                    let p = match &args[1].value {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => return Err("percentile: second argument must be a number".into()),
                    };
                    Ok(CrowValue::ok(Value::Float(crow_runtime::stats::percentile(t, p))))
                }
                _ => Err("percentile: expected numarray".into()),
            }
        }
        "zscore" => {
            if args.len() != 1 { return Err("zscore: expected 1 argument".into()); }
            match &args[0].value {
                Value::NumArray(t) => Ok(CrowValue::ok(Value::NumArray(crow_runtime::stats::zscore(t)))),
                _ => Err("zscore: expected numarray".into()),
            }
        }
        "corr" => {
            if args.len() != 2 { return Err("corr: expected 2 arguments".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(x), Value::NumArray(y)) => Ok(CrowValue::ok(Value::Float(crow_runtime::stats::corr(x, y)))),
                _ => Err("corr: expected two numarrays".into()),
            }
        }
        "cov" => {
            if args.len() != 2 { return Err("cov: expected 2 arguments".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(x), Value::NumArray(y)) => Ok(CrowValue::ok(Value::Float(crow_runtime::stats::cov(x, y)))),
                _ => Err("cov: expected two numarrays".into()),
            }
        }
        "ttest" => {
            if args.len() != 2 { return Err("ttest: expected 2 arguments".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(x), Value::NumArray(y)) => {
                    let (t, p) = crow_runtime::stats::ttest(x, y);
                    let mut map = BTreeMap::new();
                    map.insert("t".to_string(), CrowValue::ok(Value::Float(t)));
                    map.insert("p".to_string(), CrowValue::ok(Value::Float(p)));
                    Ok(CrowValue::ok(Value::Map(map)))
                }
                _ => Err("ttest: expected two numarrays".into()),
            }
        }
        _ => Err(format!("{}: unknown stats function", name)),
    }
}

fn emit_stats_rust(name: &str, arg_strs: &[String]) -> Result<String, String> {
    let result = match name {
        "median" => format!("crow_runtime::stats::median(&{})", arg_strs[0]),
        "std" => format!("crow_runtime::stats::std(&{})", arg_strs[0]),
        "var" => format!("crow_runtime::stats::var(&{})", arg_strs[0]),
        "percentile" => format!("crow_runtime::stats::percentile(&{}, {} as f64)", arg_strs[0], arg_strs[1]),
        "zscore" => format!("crow_runtime::stats::zscore(&{})", arg_strs[0]),
        "corr" => format!("crow_runtime::stats::corr(&{}, &{})", arg_strs[0], arg_strs[1]),
        "cov" => format!("crow_runtime::stats::cov(&{}, &{})", arg_strs[0], arg_strs[1]),
        "ttest" => format!("{{ let (_t, _p) = crow_runtime::stats::ttest(&{}, &{}); let mut _m = std::collections::BTreeMap::new(); _m.insert(\"t\".to_string(), _t); _m.insert(\"p\".to_string(), _p); _m }}", arg_strs[0], arg_strs[1]),
        _ => return Err(format!("{}: unknown stats function for transpiler", name)),
    };
    Ok(result)
}

fn stats_type_of(name: &str) -> CrowType {
    match name {
        "median" | "std" | "var" | "percentile" | "corr" | "cov" => CrowType::Float,
        "zscore" => CrowType::NumArray(Box::new(CrowType::Float)),
        "ttest" => CrowType::Map(Box::new(CrowType::Float)),
        _ => CrowType::Unknown,
    }
}

