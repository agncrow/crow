//! `use math.stats`: every function is a row of the builtin table
//! (crate::table), its runtime function in `crow_runtime::stats`, which
//! both modes call.
use crate::modules::CrowModule;

pub struct StatsModule;

pub(crate) const STATS_FUNCTIONS: &[&str] = &[
    "median", "std", "var", "percentile", "zscore", "corr", "cov", "ttest", "linreg",
    "chisq", "anova", "kstest", "fit",
];

impl CrowModule for StatsModule {
    fn name(&self) -> &str { "math.stats" }

    fn functions(&self) -> &[&str] { STATS_FUNCTIONS }
}

#[cfg(test)]
mod tests {
    use crate::value::{CrowValue, Value};
    use crow_runtime::tensor::CrowTensor;
    use std::collections::BTreeMap;

    /// A stats function as a program calls it: through the registry, so
    /// the function is its row.
    fn call_stats(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        crate::modules::ModuleRegistry::with_stdlib().call_in("math.stats", name, args)
    }

    fn tensor_1d(data: Vec<f64>) -> CrowValue {
        let len = data.len();
        CrowValue::ok(Value::NumArray(Box::new(CrowTensor::from_vec(data, vec![len]))))
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
        assert_eq!(call_stats("linreg", &[x.clone()]).map(|_| ()), Err("linreg[] takes 2 arguments, got 1".to_string()));
        let y = tensor_1d(vec![1.0, 2.0, 3.0]);
        assert_eq!(call_stats("linreg", &[x, y]).map(|_| ()), Err("linreg: x and y must be one length, got 2 and 3".to_string()));
    }

    /// anova takes the rest of the arguments, each a group; one group is
    /// the reason (it was NaN compiled and refused interpreted).
    #[test]
    fn anova_takes_its_groups() {
        let (a, b, c) = (tensor_1d(vec![1.0, 2.0, 3.0]), tensor_1d(vec![10.0, 11.0, 12.0]), tensor_1d(vec![20.0, 21.0, 22.0]));
        let m = unwrap_map(call_stats("anova", &[a.clone(), b, c]));
        assert!(map_float(&m, "f") > 10.0);
        assert_eq!(call_stats("anova", &[a]).map(|_| ()), Err("anova: compares 2 groups or more, got 1".to_string()));
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
