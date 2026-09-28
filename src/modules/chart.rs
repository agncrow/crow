//! `use chart` — technical indicators for the chart element. Every
//! function is a row of the builtin table (crate::table), its runtime
//! function in `crow_runtime::indicators` over the kernels in
//! `crow_runtime::ta`, so the interpreter and compiled programs draw the
//! same numbers. Every function takes series (frame columns, arrays) and
//! returns a series the same length, NaN where the value is not yet
//! defined, ready for `with[frame, 'rsi', ...]` and a chart pane.
//!
//! `sma[v, n]`, `ema[v, n]`, `rsi[v, n]`, `rsi[ts, v, n, secs]` (on
//! clock-aligned buckets), `macd[v, fast, slow, signal]` (a frame: line,
//! signal, hist), `vwap[price, vol]` (cumulative), `vwap[ts, price, vol,
//! secs]` (trailing window), `rolling_sum[ts, v, secs]`, `cvd[close, vol]`.
use crate::modules::CrowModule;

pub struct ChartModule;

pub(crate) const CHART_FUNCTIONS: &[&str] =
    &["sma", "ema", "rsi", "macd", "vwap", "rolling_sum", "cvd"];

impl CrowModule for ChartModule {
    fn name(&self) -> &str { "chart" }

    fn functions(&self) -> &[&str] { CHART_FUNCTIONS }
}
