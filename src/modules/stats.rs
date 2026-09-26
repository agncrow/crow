use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;
use std::collections::BTreeMap;

pub struct StatsModule;

pub(crate) const STATS_FUNCTIONS: &[&str] = &[
    "median", "std", "var", "percentile", "zscore", "corr", "cov", "ttest", "linreg",
    "chisq", "anova", "kstest", "fit",
];

impl CrowModule for StatsModule {
    fn name(&self) -> &str { "math.stats" }

    fn functions(&self) -> &[&str] { STATS_FUNCTIONS }

    fn constants(&self) -> Vec<(&str, CrowValue)> { vec![] }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        for a in args {
            if !a.is_ok() {
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
        "linreg" => {
            if args.len() != 2 { return Err("linreg: expected 2 arguments (x, y)".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(x), Value::NumArray(y)) => {
                    let (slope, intercept, r2) = crow_runtime::stats::linreg(x, y);
                    let mut map = BTreeMap::new();
                    map.insert("slope".to_string(), CrowValue::ok(Value::Float(slope)));
                    map.insert("intercept".to_string(), CrowValue::ok(Value::Float(intercept)));
                    map.insert("r2".to_string(), CrowValue::ok(Value::Float(r2)));
                    Ok(CrowValue::ok(Value::Map(map)))
                }
                _ => Err("linreg: expected two numarrays".into()),
            }
        }
        "chisq" => {
            if args.len() != 2 { return Err("chisq: expected 2 arguments (observed, expected)".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(obs), Value::NumArray(exp)) => {
                    let (chi2, p) = crow_runtime::stats::chisq(obs, exp);
                    let mut map = BTreeMap::new();
                    map.insert("chi2".to_string(), CrowValue::ok(Value::Float(chi2)));
                    map.insert("p".to_string(), CrowValue::ok(Value::Float(p)));
                    Ok(CrowValue::ok(Value::Map(map)))
                }
                _ => Err("chisq: expected two numarrays".into()),
            }
        }
        "anova" => {
            if args.len() < 2 { return Err("anova: expected at least 2 arguments (groups)".into()); }
            let groups: Result<Vec<&crow_runtime::tensor::CrowTensor>, String> = args.iter().map(|a| {
                match &a.value {
                    Value::NumArray(t) => Ok(t),
                    _ => Err("anova: all arguments must be numarrays".into()),
                }
            }).collect();
            let groups = groups?;
            let (f, p) = crow_runtime::stats::anova(&groups);
            let mut map = BTreeMap::new();
            map.insert("f".to_string(), CrowValue::ok(Value::Float(f)));
            map.insert("p".to_string(), CrowValue::ok(Value::Float(p)));
            Ok(CrowValue::ok(Value::Map(map)))
        }
        "kstest" => {
            if args.len() != 2 { return Err("kstest: expected 2 arguments (data, distribution_name)".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(data), Value::Str(dist)) => {
                    let (d, p) = crow_runtime::stats::kstest(data, dist);
                    let mut map = BTreeMap::new();
                    map.insert("d".to_string(), CrowValue::ok(Value::Float(d)));
                    map.insert("p".to_string(), CrowValue::ok(Value::Float(p)));
                    Ok(CrowValue::ok(Value::Map(map)))
                }
                _ => Err("kstest: expected (numarray, string)".into()),
            }
        }
        "fit" => {
            if args.len() != 2 { return Err("fit: expected 2 arguments (data, distribution_name)".into()); }
            match (&args[0].value, &args[1].value) {
                (Value::NumArray(data), Value::Str(dist)) => {
                    let params = crow_runtime::stats::fit(data, dist);
                    let mut map = BTreeMap::new();
                    for (key, val) in params {
                        map.insert(key, CrowValue::ok(Value::Float(val)));
                    }
                    Ok(CrowValue::ok(Value::Map(map)))
                }
                _ => Err("fit: expected (numarray, string)".into()),
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
        "linreg" => format!("{{ let (_s, _i, _r) = crow_runtime::stats::linreg(&{}, &{}); let mut _m = std::collections::BTreeMap::new(); _m.insert(\"slope\".to_string(), _s); _m.insert(\"intercept\".to_string(), _i); _m.insert(\"r2\".to_string(), _r); _m }}", arg_strs[0], arg_strs[1]),
        "chisq" => format!("{{ let (_c, _p) = crow_runtime::stats::chisq(&{}, &{}); let mut _m = std::collections::BTreeMap::new(); _m.insert(\"chi2\".to_string(), _c); _m.insert(\"p\".to_string(), _p); _m }}", arg_strs[0], arg_strs[1]),
        "anova" => {
            let refs: Vec<String> = arg_strs.iter().map(|a| format!("&{}", a)).collect();
            format!("{{ let (_f, _p) = crow_runtime::stats::anova(&[{}]); let mut _m = std::collections::BTreeMap::new(); _m.insert(\"f\".to_string(), _f); _m.insert(\"p\".to_string(), _p); _m }}", refs.join(", "))
        }
        "kstest" => format!("{{ let (_d, _p) = crow_runtime::stats::kstest(&{}, &{}); let mut _m = std::collections::BTreeMap::new(); _m.insert(\"d\".to_string(), _d); _m.insert(\"p\".to_string(), _p); _m }}", arg_strs[0], arg_strs[1]),
        "fit" => format!("{{ let _params = crow_runtime::stats::fit(&{}, &{}); let mut _m = std::collections::BTreeMap::new(); for (_k, _v) in _params {{ _m.insert(_k, _v); }} _m }}", arg_strs[0], arg_strs[1]),
        _ => return Err(format!("{}: unknown stats function for the Rust backend", name)),
    };
    Ok(result)
}

fn stats_type_of(name: &str) -> CrowType {
    match name {
        "median" | "std" | "var" | "percentile" | "corr" | "cov" => CrowType::Float,
        "zscore" => CrowType::NumArray(Box::new(CrowType::Float)),
        "ttest" | "linreg" | "chisq" | "anova" | "kstest" | "fit" => CrowType::Map(Box::new(CrowType::Float)),
        _ => CrowType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{CrowValue, Value};
    use crow_runtime::tensor::CrowTensor;

    fn tensor_1d(data: Vec<f64>) -> CrowValue {
        let len = data.len();
        CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len])))
    }

    fn unwrap_map(r: Result<CrowValue, String>) -> BTreeMap<String, CrowValue> {
        let cv = r.expect("call_stats failed");
        assert!(cv.is_ok(), "CrowValue is error: {}", cv.reason());
        match cv.value {
            Value::Map(m) => m,
            other => panic!("expected Map, got {:?}", other),
        }
    }

    fn unwrap_float(r: Result<CrowValue, String>) -> f64 {
        let cv = r.expect("call_stats failed");
        assert!(cv.is_ok(), "CrowValue is error: {}", cv.reason());
        match cv.value {
            Value::Float(f) => f,
            other => panic!("expected Float, got {:?}", other),
        }
    }

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn map_float(m: &BTreeMap<String, CrowValue>, key: &str) -> f64 {
        match &m[key].value {
            Value::Float(f) => *f,
            other => panic!("expected Float for key '{}', got {:?}", key, other),
        }
    }

    #[test]
    fn test_linreg_dispatch() {
        let x = tensor_1d(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor_1d(vec![3.0, 5.0, 7.0, 9.0, 11.0]); // y = 2x + 1
        let m = unwrap_map(call_stats("linreg", &[x, y]));
        assert!(approx_eq(map_float(&m, "slope"), 2.0));
        assert!(approx_eq(map_float(&m, "intercept"), 1.0));
        assert!(approx_eq(map_float(&m, "r2"), 1.0));
    }

    #[test]
    fn test_linreg_wrong_args() {
        let x = tensor_1d(vec![1.0, 2.0]);
        assert!(call_stats("linreg", &[x]).is_err());
    }

    #[test]
    fn test_median_dispatch() {
        let x = tensor_1d(vec![3.0, 1.0, 2.0]);
        let v = unwrap_float(call_stats("median", &[x]));
        assert!(approx_eq(v, 2.0));
    }

    #[test]
    fn test_ttest_dispatch() {
        let x = tensor_1d(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor_1d(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let m = unwrap_map(call_stats("ttest", &[x, y]));
        assert!(approx_eq(map_float(&m, "t"), 0.0));
    }
}
