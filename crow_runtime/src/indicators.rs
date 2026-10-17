//! The functions `use chart` gives a program (docs/bug-classes.md §4): each
//! takes its series as a tensor — a list of numbers or a frame's column is
//! converted where it is handed over, the same way in both modes — checks
//! what the kernels in [`crate::ta`] assume, and answers a series the same
//! length. A window of no rows, series of different lengths, a bucket of
//! no seconds: the reason, where a negative window was a huge one compiled
//! and a window of one interpreted, and a shorter series cut the others.

use crate::frame::CrowFrame;
use crate::ta;
use crate::tensor::CrowTensor;

/// A window of `n` rows: 1 or more.
fn rows(what: &str, n: i64) -> Result<usize, String> {
    match usize::try_from(n) {
        Ok(w) if w >= 1 => Ok(w),
        _ => Err(format!("{}: a window is 1 row or more (got {})", what, n)),
    }
}

/// A trailing window of `secs` seconds: 0 or more.
fn trailing(what: &str, secs: f64) -> Result<f64, String> {
    if secs >= 0.0 {
        Ok(secs)
    } else {
        Err(format!("{}: a window is 0 seconds or more (got {})", what, secs))
    }
}

/// Times a trailing window reads: in order, each at least the one before.
/// A window over times that go back answered what no reading of "the
/// trailing seconds" gives (crow check --self's reference: over 3, 1, 2,
/// the last row had summed every row); now it stops, saying where.
fn in_order(what: &str, ts: &CrowTensor) -> Result<(), String> {
    let t = ts.to_vec();
    match t.windows(2).position(|w| w[1] < w[0]) {
        None => Ok(()),
        Some(i) => Err(format!("{}: the times go back at row {} ({} after {}) — a trailing window reads times in order", what, i + 1, crate::display::CrowDisplay::crow_fmt(&t[i + 1]), crate::display::CrowDisplay::crow_fmt(&t[i]))),
    }
}

/// Series an indicator reads row by row: of one length.
fn aligned(what: &str, names: &str, series: &[&CrowTensor]) -> Result<(), String> {
    let lens: Vec<usize> = series.iter().map(|s| s.len()).collect();
    if lens.windows(2).all(|w| w[0] == w[1]) {
        return Ok(());
    }
    let words: Vec<String> = lens.iter().map(|n| n.to_string()).collect();
    let got = match words.split_last() {
        Some((last, rest)) => format!("{} and {}", rest.join(", "), last),
        None => String::new(),
    };
    Err(format!("{}: {} must be one length, got {}", what, names, got))
}

/// `sma[values, n]`: the mean of the last `n` rows, NaN until there are `n`.
pub fn sma(v: &CrowTensor, n: i64) -> Result<CrowTensor, String> {
    Ok(ta::tensor(ta::sma(&v.to_vec(), rows("sma", n)?)))
}

/// `ema[values, n]`: smoothing 2/(n+1), seeded by the mean of the first `n`.
pub fn ema(v: &CrowTensor, n: i64) -> Result<CrowTensor, String> {
    Ok(ta::tensor(ta::ema(&v.to_vec(), rows("ema", n)?)))
}

/// `rsi[values, n]`: Wilder's RSI over `n` changes.
pub fn rsi(v: &CrowTensor, n: i64) -> Result<CrowTensor, String> {
    Ok(ta::tensor(ta::rsi(&v.to_vec(), rows("rsi", n)?)))
}

/// `rsi[ts, values, n, secs]`: the RSI of clock-aligned buckets of `secs`
/// seconds, `ts` in seconds.
pub fn rsi_buckets(ts: &CrowTensor, v: &CrowTensor, n: i64, secs: f64) -> Result<CrowTensor, String> {
    aligned("rsi", "ts and values", &[ts, v])?;
    in_order("rsi", ts)?;
    let n = rows("rsi", n)?;
    if secs.is_nan() || secs <= 0.0 {
        return Err(format!("rsi: a bucket is more than 0 seconds (got {})", secs));
    }
    Ok(ta::tensor(ta::rsi_buckets(&ts.to_vec(), &v.to_vec(), n, secs)))
}

/// `macd[values, fast, slow, signal]`: a frame of `line`, `signal`, `hist`.
pub fn macd(v: &CrowTensor, fast: i64, slow: i64, signal: i64) -> Result<CrowFrame, String> {
    let (fast, slow, signal) = (rows("macd", fast)?, rows("macd", slow)?, rows("macd", signal)?);
    Ok(ta::macd_frame(&v.to_vec(), fast, slow, signal))
}

/// `vwap[price, vol]`: from the first row.
pub fn vwap(price: &CrowTensor, vol: &CrowTensor) -> Result<CrowTensor, String> {
    aligned("vwap", "price and vol", &[price, vol])?;
    Ok(ta::tensor(ta::vwap(&price.to_vec(), &vol.to_vec())))
}

/// `vwap[ts, price, vol, secs]`: over the trailing `secs` seconds.
pub fn vwap_trailing(ts: &CrowTensor, price: &CrowTensor, vol: &CrowTensor, secs: f64) -> Result<CrowTensor, String> {
    aligned("vwap", "ts, price and vol", &[ts, price, vol])?;
    in_order("vwap", ts)?;
    let secs = trailing("vwap", secs)?;
    Ok(ta::tensor(ta::rolling_vwap(&ts.to_vec(), &price.to_vec(), &vol.to_vec(), secs)))
}

/// `rolling_sum[ts, values, secs]`: over the trailing `secs` seconds.
pub fn rolling_sum(ts: &CrowTensor, v: &CrowTensor, secs: f64) -> Result<CrowTensor, String> {
    aligned("rolling_sum", "ts and values", &[ts, v])?;
    in_order("rolling_sum", ts)?;
    let secs = trailing("rolling_sum", secs)?;
    Ok(ta::tensor(ta::rolling_sum(&ts.to_vec(), &v.to_vec(), secs)))
}

/// `cvd[close, vol]`: volume signed by the close's change, summed.
pub fn cvd(close: &CrowTensor, vol: &CrowTensor) -> Result<CrowTensor, String> {
    aligned("cvd", "close and vol", &[close, vol])?;
    Ok(ta::tensor(ta::cvd(&close.to_vec(), &vol.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(v: &[f64]) -> CrowTensor {
        CrowTensor::from_vec(v.to_vec(), vec![v.len()])
    }

    #[test]
    fn an_indicator_is_its_kernel() {
        let v = series(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(sma(&v, 2).map(|t| t.to_vec()).expect("a window").get(3).copied(), Some(3.5));
        assert_eq!(macd(&v, 2, 3, 2).map(|f| f.columns.clone()).expect("windows"), vec!["line".to_string(), "signal".to_string(), "hist".to_string()]);
    }

    /// A trailing window reads times in order: one that goes back stops,
    /// saying where (it had summed rows a window holds by no reading).
    #[test]
    fn a_trailing_window_reads_times_in_order() {
        let (ts, v) = (series(&[3.0, 1.0, 2.0]), series(&[3.0, 1.0, 2.0]));
        assert_eq!(rolling_sum(&ts, &v, 0.0).map(|t| t.to_vec()).unwrap_err(), "rolling_sum: the times go back at row 1 (1 after 3) — a trailing window reads times in order");
        assert!(vwap_trailing(&ts, &v, &v, 1.0).is_err() && rsi_buckets(&ts, &v, 2, 1.0).is_err());
        let ts = series(&[1.0, 2.0, 2.0, 5.0]);
        let v = series(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rolling_sum(&ts, &v, 1.0).map(|t| t.to_vec()).expect("in order"), vec![1.0, 3.0, 6.0, 4.0]);
    }

    /// A misuse is the reason: a negative window was a huge one compiled
    /// (all NaN) and a window of one interpreted (the values themselves).
    #[test]
    fn a_misuse_is_the_reason() {
        let (v, w) = (series(&[1.0, 2.0, 3.0]), series(&[1.0, 2.0]));
        assert_eq!(sma(&v, -1).map(|_| ()), Err("sma: a window is 1 row or more (got -1)".to_string()));
        assert_eq!(ema(&v, 0).map(|_| ()), Err("ema: a window is 1 row or more (got 0)".to_string()));
        assert_eq!(vwap(&v, &w).map(|_| ()), Err("vwap: price and vol must be one length, got 3 and 2".to_string()));
        assert_eq!(vwap_trailing(&v, &v, &w, 60.0).map(|_| ()), Err("vwap: ts, price and vol must be one length, got 3, 3 and 2".to_string()));
        assert_eq!(rsi_buckets(&v, &v, 14, 0.0).map(|_| ()), Err("rsi: a bucket is more than 0 seconds (got 0)".to_string()));
        assert_eq!(rolling_sum(&v, &v, -1.0).map(|_| ()), Err("rolling_sum: a window is 0 seconds or more (got -1)".to_string()));
    }
}
