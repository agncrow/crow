//! Technical indicators — the `use chart` module's kernel.
//!
//! Pure functions over slices: no I/O, no frames, no vnodes. Every
//! indicator returns one value per input row and writes NaN where the
//! value is not yet defined (a 14-period RSI has no value on row 3), so
//! charts draw gaps instead of lies. Both execution modes call these —
//! the interpreter through `src/modules/chart.rs`, compiled programs
//! directly — so the numbers agree by construction.

use crate::frame::CrowColumn;
use crate::tensor::CrowTensor;

/// Anything that can be read as a series of floats: a tensor, a frame
/// column, a plain vector. Compiled code goes through this so a module
/// call compiles whatever the argument's static type turned out to be.
pub trait Series {
    fn f64s(&self) -> Vec<f64>;
}

impl Series for CrowTensor {
    fn f64s(&self) -> Vec<f64> {
        self.to_vec()
    }
}
impl Series for CrowColumn {
    fn f64s(&self) -> Vec<f64> {
        self.to_f64_vec()
    }
}
impl Series for Vec<f64> {
    fn f64s(&self) -> Vec<f64> {
        self.clone()
    }
}
impl Series for Vec<i64> {
    fn f64s(&self) -> Vec<f64> {
        self.iter().map(|x| *x as f64).collect()
    }
}
impl Series for [f64] {
    fn f64s(&self) -> Vec<f64> {
        self.to_vec()
    }
}
impl<T: Series + ?Sized> Series for &T {
    fn f64s(&self) -> Vec<f64> {
        (**self).f64s()
    }
}

/// Read any series as floats.
pub fn f64s<T: Series + ?Sized>(s: &T) -> Vec<f64> {
    s.f64s()
}

/// Wrap a result as a one-dimensional tensor (what Crow arrays are).
pub fn tensor(v: Vec<f64>) -> CrowTensor {
    let n = v.len();
    CrowTensor::from_vec(v, vec![n])
}

/// Simple moving average over the last `n` rows; NaN until `n` rows exist.
pub fn sma(v: &[f64], n: usize) -> Vec<f64> {
    let n = n.max(1);
    let mut out = vec![f64::NAN; v.len()];
    let mut sum = 0.0;
    let mut count = 0usize;
    for i in 0..v.len() {
        if v[i].is_finite() {
            sum += v[i];
            count += 1;
        }
        if i >= n && v[i - n].is_finite() {
            sum -= v[i - n];
            count -= 1;
        }
        if i + 1 >= n && count > 0 {
            out[i] = sum / count as f64;
        }
    }
    out
}

/// Exponential moving average with smoothing 2/(n+1), seeded by the SMA of
/// the first `n` finite rows; NaN before the seed.
pub fn ema(v: &[f64], n: usize) -> Vec<f64> {
    let n = n.max(1);
    let k = 2.0 / (n as f64 + 1.0);
    let mut out = vec![f64::NAN; v.len()];
    let mut seed_sum = 0.0;
    let mut seeded = 0usize;
    let mut prev = f64::NAN;
    for i in 0..v.len() {
        let x = v[i];
        if !x.is_finite() {
            out[i] = prev;
            continue;
        }
        if seeded < n {
            seed_sum += x;
            seeded += 1;
            if seeded == n {
                prev = seed_sum / n as f64;
                out[i] = prev;
            }
            continue;
        }
        prev = x * k + prev * (1.0 - k);
        out[i] = prev;
    }
    out
}

/// Wilder's RSI over `n` deltas: the first `n` deltas seed the average
/// gain and loss, then each step smooths by (n-1)/n. A simple average
/// during warmup, so the line appears from the second row on.
pub fn rsi(v: &[f64], n: usize) -> Vec<f64> {
    let n = n.max(1);
    let mut out = vec![f64::NAN; v.len()];
    let mut state = WilderState::new(n);
    let mut prev = f64::NAN;
    for i in 0..v.len() {
        let x = v[i];
        if !x.is_finite() {
            continue;
        }
        if prev.is_finite() {
            state.commit(x - prev);
        }
        prev = x;
        out[i] = state.value();
    }
    out
}

/// RSI on clock-aligned buckets of `secs` seconds — the way a 14-period
/// RSI on 3-minute candles is drawn over 1-second bars: completed buckets
/// feed Wilder's smoothing once, and every row inside the forming bucket
/// sees a provisional value from its latest close, so history never
/// changes and the newest value moves with the tape. `ts` are seconds.
pub fn rsi_buckets(ts: &[f64], close: &[f64], n: usize, secs: f64) -> Vec<f64> {
    let n = n.max(1);
    let secs = if secs > 0.0 { secs } else { 60.0 };
    let m = ts.len().min(close.len());
    let mut out = vec![f64::NAN; m];
    let mut state = WilderState::new(n);
    let mut last_committed_close = f64::NAN;
    let mut bucket_id = f64::NAN;
    let mut bucket_close = f64::NAN;
    for i in 0..m {
        let (t, c) = (ts[i], close[i]);
        if !t.is_finite() || !c.is_finite() {
            continue;
        }
        let id = (t / secs).floor();
        if bucket_id.is_finite() && id != bucket_id {
            // the previous bucket is complete: commit its close
            if last_committed_close.is_finite() {
                state.commit(bucket_close - last_committed_close);
            }
            last_committed_close = bucket_close;
        }
        bucket_id = id;
        bucket_close = c;
        // provisional: the forming bucket's close applied as one extra step
        out[i] = if last_committed_close.is_finite() {
            state.peek(c - last_committed_close)
        } else {
            f64::NAN
        };
    }
    out
}

#[derive(Clone)]
struct WilderState {
    n: usize,
    seeded: usize,
    gain_sum: f64,
    loss_sum: f64,
    avg_gain: f64,
    avg_loss: f64,
}

impl WilderState {
    fn new(n: usize) -> Self {
        WilderState { n, seeded: 0, gain_sum: 0.0, loss_sum: 0.0, avg_gain: f64::NAN, avg_loss: f64::NAN }
    }

    fn commit(&mut self, delta: f64) {
        let (g, l) = if delta >= 0.0 { (delta, 0.0) } else { (0.0, -delta) };
        if self.seeded < self.n {
            self.gain_sum += g;
            self.loss_sum += l;
            self.seeded += 1;
            if self.seeded == self.n {
                self.avg_gain = self.gain_sum / self.n as f64;
                self.avg_loss = self.loss_sum / self.n as f64;
            }
            return;
        }
        let n = self.n as f64;
        self.avg_gain = (self.avg_gain * (n - 1.0) + g) / n;
        self.avg_loss = (self.avg_loss * (n - 1.0) + l) / n;
    }

    fn value(&self) -> f64 {
        if self.seeded == 0 {
            return f64::NAN;
        }
        if self.seeded < self.n {
            return rsi_of(self.gain_sum / self.seeded as f64, self.loss_sum / self.seeded as f64);
        }
        rsi_of(self.avg_gain, self.avg_loss)
    }

    /// The RSI if `delta` were committed next, without committing it.
    fn peek(&self, delta: f64) -> f64 {
        let mut trial = self.clone();
        trial.commit(delta);
        trial.value()
    }
}

fn rsi_of(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss <= 0.0 {
        return if avg_gain > 0.0 { 100.0 } else { 50.0 };
    }
    100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
}

/// MACD line (fast EMA − slow EMA), its signal EMA, and the histogram.
pub fn macd(v: &[f64], fast: usize, slow: usize, signal: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let ef = ema(v, fast);
    let es = ema(v, slow);
    let line: Vec<f64> = ef.iter().zip(&es).map(|(a, b)| a - b).collect();
    let sig = ema(&line, signal);
    let hist: Vec<f64> = line.iter().zip(&sig).map(|(a, b)| a - b).collect();
    (line, sig, hist)
}

/// MACD as a three-column frame (`line`, `signal`, `hist`), the shape a
/// program adds to its bars column by column.
pub fn macd_frame(v: &[f64], fast: usize, slow: usize, signal: usize) -> crate::frame::CrowFrame {
    let (line, sig, hist) = macd(v, fast, slow, signal);
    let n = line.len();
    crate::frame::CrowFrame::new(
        vec!["line".into(), "signal".into(), "hist".into()],
        vec![
            crate::frame::Column::Float(line),
            crate::frame::Column::Float(sig),
            crate::frame::Column::Float(hist),
        ],
        vec![vec![true; n], vec![true; n], vec![true; n]],
    )
}

/// Cumulative volume-weighted average price from the first row.
pub fn vwap(price: &[f64], vol: &[f64]) -> Vec<f64> {
    let m = price.len().min(vol.len());
    let mut out = vec![f64::NAN; m];
    let (mut pv, mut v) = (0.0, 0.0);
    for i in 0..m {
        if price[i].is_finite() && vol[i].is_finite() {
            pv += price[i] * vol[i];
            v += vol[i];
        }
        if v > 0.0 {
            out[i] = pv / v;
        }
    }
    out
}

/// VWAP over the trailing `secs` seconds (rows with ts ≥ t − secs).
pub fn rolling_vwap(ts: &[f64], price: &[f64], vol: &[f64], secs: f64) -> Vec<f64> {
    let m = ts.len().min(price.len()).min(vol.len());
    let mut out = vec![f64::NAN; m];
    let (mut pv, mut v) = (0.0, 0.0);
    let mut head = 0usize;
    for i in 0..m {
        if price[i].is_finite() && vol[i].is_finite() {
            pv += price[i] * vol[i];
            v += vol[i];
        }
        while head < i && ts[head] < ts[i] - secs {
            if price[head].is_finite() && vol[head].is_finite() {
                pv -= price[head] * vol[head];
                v -= vol[head];
            }
            head += 1;
        }
        if v > 1e-9 {
            out[i] = pv / v;
        }
    }
    out
}

/// Sum over the trailing `secs` seconds (rows with ts ≥ t − secs).
pub fn rolling_sum(ts: &[f64], v: &[f64], secs: f64) -> Vec<f64> {
    let m = ts.len().min(v.len());
    let mut out = vec![f64::NAN; m];
    let mut sum = 0.0;
    let mut head = 0usize;
    for i in 0..m {
        if v[i].is_finite() {
            sum += v[i];
        }
        while head < i && ts[head] < ts[i] - secs {
            if v[head].is_finite() {
                sum -= v[head];
            }
            head += 1;
        }
        out[i] = sum;
    }
    out
}

/// Cumulative volume delta: volume counted positive when the close rose,
/// negative when it fell, ignored when unchanged.
pub fn cvd(close: &[f64], vol: &[f64]) -> Vec<f64> {
    let m = close.len().min(vol.len());
    let mut out = vec![f64::NAN; m];
    let mut acc = 0.0;
    let mut prev = f64::NAN;
    for i in 0..m {
        let c = close[i];
        if c.is_finite() && prev.is_finite() && vol[i].is_finite() {
            if c > prev {
                acc += vol[i];
            } else if c < prev {
                acc -= vol[i];
            }
        }
        if c.is_finite() {
            prev = c;
        }
        out[i] = acc;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn sma_waits_for_a_full_window() {
        let s = sma(&[1.0, 2.0, 3.0, 4.0], 2);
        assert!(s[0].is_nan());
        assert!(close(s[1], 1.5) && close(s[2], 2.5) && close(s[3], 3.5));
    }

    #[test]
    fn ema_seeds_with_the_sma() {
        let e = ema(&[1.0, 2.0, 3.0, 4.0], 2);
        assert!(e[0].is_nan());
        assert!(close(e[1], 1.5));
        assert!(close(e[2], 2.5));
    }

    #[test]
    fn rsi_is_100_on_a_straight_climb_and_50_when_flat() {
        let up: Vec<f64> = (0..20).map(|i| i as f64).collect();
        assert!(rsi(&up, 14)[19] > 99.9);
        let flat = vec![5.0; 20];
        assert!(close(rsi(&flat, 14)[19], 50.0));
        let zig: Vec<f64> = (0..40).map(|i| if i % 2 == 0 { 10.0 } else { 11.0 }).collect();
        let z = rsi(&zig, 14);
        assert!((z[39] - 50.0).abs() < 5.0, "{}", z[39]);
    }

    #[test]
    fn bucketed_rsi_commits_once_per_bucket() {
        let ts: Vec<f64> = (0..30).map(|i| i as f64).collect();
        let cl: Vec<f64> = ts.iter().map(|t| 10.0 + (t / 3.0).floor()).collect();
        let r = rsi_buckets(&ts, &cl, 2, 3.0);
        assert!(close(r[7], r[8]));
        assert!(r[29] > 99.0);
        assert!(r[0].is_nan());
    }

    #[test]
    fn macd_and_vwap_shapes() {
        let v: Vec<f64> = (0..50).map(|i| (i as f64 * 0.3).sin() + 10.0).collect();
        let (l, s, h) = macd(&v, 3, 6, 2);
        assert_eq!(l.len(), 50);
        assert!(h.iter().zip(l.iter().zip(&s)).all(|(h, (l, s))| h.is_nan() || close(*h, l - s)));
        let w = vwap(&[10.0, 12.0], &[1.0, 3.0]);
        assert!(close(w[1], (10.0 + 36.0) / 4.0));
    }

    #[test]
    fn rolling_windows_drop_old_rows() {
        let ts = [0.0, 10.0, 20.0, 30.0];
        let v = [1.0, 1.0, 1.0, 1.0];
        assert_eq!(rolling_sum(&ts, &v, 15.0), vec![1.0, 2.0, 2.0, 2.0]);
        let p = [1.0, 2.0, 3.0, 4.0];
        assert!(close(rolling_vwap(&ts, &p, &v, 15.0)[3], 3.5));
    }

    #[test]
    fn cvd_signs_volume_by_the_close_change() {
        assert_eq!(cvd(&[1.0, 2.0, 2.0, 1.0], &[5.0; 4]), vec![0.0, 5.0, 5.0, 0.0]);
    }
}
