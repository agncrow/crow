//! `use chart` — technical indicators for the chart element, computed by
//! the kernel (`crow_runtime::ta`) so the interpreter and compiled programs
//! draw the same numbers. Every function takes series (frame columns,
//! arrays) and returns a series the same length, NaN where the value is
//! not yet defined, ready for `with[frame, 'rsi', ...]` and a chart pane.
//!
//! `sma[v, n]`, `ema[v, n]`, `rsi[v, n]`, `rsi[ts, v, n, secs]` (on
//! clock-aligned buckets), `macd[v, fast, slow, signal]` (a frame: line,
//! signal, hist), `vwap[price, vol]` (cumulative), `vwap[ts, price, vol,
//! secs]` (trailing window), `rolling_sum[ts, v, secs]`, `cvd[close, vol]`.
use crate::modules::CrowModule;
use crate::type_infer::CrowType;
use crate::value::{CrowValue, Value};

pub struct ChartModule;

pub(crate) const CHART_FUNCTIONS: &[&str] =
    &["sma", "ema", "rsi", "macd", "vwap", "rolling_sum", "cvd"];

impl CrowModule for ChartModule {
    fn name(&self) -> &str { "chart" }

    fn functions(&self) -> &[&str] { CHART_FUNCTIONS }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        for a in args {
            if !a.is_ok() {
                return Ok(a.clone());
            }
        }
        call_chart(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        let n = arg_strs.len();
        let s = |i: usize| format!("&crow_runtime::ta::f64s(&{})", arg_strs[i]);
        let u = |i: usize| format!("({}) as usize", arg_strs[i]);
        let f = |i: usize| format!("({}) as f64", arg_strs[i]);
        let series = |body: String| format!("crow_runtime::ta::tensor({})", body);
        let out = match (name, n) {
            ("sma", 2) => series(format!("crow_runtime::ta::sma({}, {})", s(0), u(1))),
            ("ema", 2) => series(format!("crow_runtime::ta::ema({}, {})", s(0), u(1))),
            ("rsi", 2) => series(format!("crow_runtime::ta::rsi({}, {})", s(0), u(1))),
            ("rsi", 4) => series(format!("crow_runtime::ta::rsi_buckets({}, {}, {}, {})", s(0), s(1), u(2), f(3))),
            ("macd", 4) => format!("crow_runtime::ta::macd_frame({}, {}, {}, {})", s(0), u(1), u(2), u(3)),
            ("vwap", 2) => series(format!("crow_runtime::ta::vwap({}, {})", s(0), s(1))),
            ("vwap", 4) => series(format!("crow_runtime::ta::rolling_vwap({}, {}, {}, {})", s(0), s(1), s(2), f(3))),
            ("rolling_sum", 3) => series(format!("crow_runtime::ta::rolling_sum({}, {}, {})", s(0), s(1), f(2))),
            ("cvd", 2) => series(format!("crow_runtime::ta::cvd({}, {})", s(0), s(1))),
            _ => return Err(arity_error(name, n)),
        };
        Ok(out)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        match name {
            "macd" => CrowType::Frame,
            "sma" | "ema" | "rsi" | "vwap" | "rolling_sum" | "cvd" => CrowType::NumArray(Box::new(CrowType::Float)),
            _ => CrowType::Unknown,
        }
    }
}

fn arity_error(name: &str, n: usize) -> String {
    let forms = match name {
        "sma" => "sma[values, n]",
        "ema" => "ema[values, n]",
        "rsi" => "rsi[values, n] or rsi[ts, values, n, secs]",
        "macd" => "macd[values, fast, slow, signal]",
        "vwap" => "vwap[price, vol] or vwap[ts, price, vol, secs]",
        "rolling_sum" => "rolling_sum[ts, values, secs]",
        "cvd" => "cvd[close, vol]",
        _ => return format!("chart has no function '{}'", name),
    };
    format!("{}: got {} arguments — write {}", name, n, forms)
}

/// A series argument as floats: a numeric array or a frame column.
fn series(v: &CrowValue, fname: &str) -> Result<Vec<f64>, String> {
    match &v.value {
        Value::NumArray(t) => Ok(t.to_vec()),
        Value::Array(items) => items
            .iter()
            .map(|i| match &i.value {
                Value::Float(x) => Ok(*x),
                Value::Int(n) => Ok(*n as f64),
                Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
                other => Err(format!("{}: expected a series of numbers, got {}", fname, other.type_name())),
            })
            .collect(),
        other => Err(format!("{}: expected a series of numbers (a frame column or array), got {}", fname, other.type_name())),
    }
}

fn num(v: &CrowValue, fname: &str) -> Result<f64, String> {
    match &v.value {
        Value::Float(x) => Ok(*x),
        Value::Int(n) => Ok(*n as f64),
        other => Err(format!("{}: expected a number, got {}", fname, other.type_name())),
    }
}

fn out(v: Vec<f64>) -> Result<CrowValue, String> {
    Ok(CrowValue::ok(Value::NumArray(Box::new(crow_runtime::ta::tensor(v)))))
}

fn call_chart(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    use crow_runtime::ta;
    let n = args.len();
    match (name, n) {
        ("sma", 2) => out(ta::sma(&series(&args[0], name)?, num(&args[1], name)? as usize)),
        ("ema", 2) => out(ta::ema(&series(&args[0], name)?, num(&args[1], name)? as usize)),
        ("rsi", 2) => out(ta::rsi(&series(&args[0], name)?, num(&args[1], name)? as usize)),
        ("rsi", 4) => out(ta::rsi_buckets(
            &series(&args[0], name)?,
            &series(&args[1], name)?,
            num(&args[2], name)? as usize,
            num(&args[3], name)?,
        )),
        ("macd", 4) => Ok(CrowValue::ok(Value::Frame(Box::new(ta::macd_frame(
            &series(&args[0], name)?,
            num(&args[1], name)? as usize,
            num(&args[2], name)? as usize,
            num(&args[3], name)? as usize,
        ))))),
        ("vwap", 2) => out(ta::vwap(&series(&args[0], name)?, &series(&args[1], name)?)),
        ("vwap", 4) => out(ta::rolling_vwap(
            &series(&args[0], name)?,
            &series(&args[1], name)?,
            &series(&args[2], name)?,
            num(&args[3], name)?,
        )),
        ("rolling_sum", 3) => out(ta::rolling_sum(
            &series(&args[0], name)?,
            &series(&args[1], name)?,
            num(&args[2], name)?,
        )),
        ("cvd", 2) => out(ta::cvd(&series(&args[0], name)?, &series(&args[1], name)?)),
        _ => Err(arity_error(name, n)),
    }
}
