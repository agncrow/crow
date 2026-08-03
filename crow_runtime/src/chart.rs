//! Chart geometry kernel — pure math, no I/O, no vnodes.
//!
//! The chart module's shared kernel (see docs/chart-module-design.md §4):
//! scales, tick selection, path building, candle layout, and the point
//! budget live here so both modes and unit tests reach them. SVG assembly
//! happens in crow_web; this file never sees an element.
//!
//! Time ticks are timezone-aware by construction — tick logic is where
//! naive charts betray timezones, and machine-local time does not exist
//! in this language.

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// Hard ceiling per series. Over budget is a teaching error naming the
/// fix (`bucket` the frame first), never a silent downsample.
pub const POINT_BUDGET: usize = 1000;

/// Refuse oversized input with the fix spelled out.
pub fn check_budget(points: usize, series: &str) -> Result<(), String> {
    if points > POINT_BUDGET {
        return Err(format!(
            "chart got {} points for '{}' (budget {}) — bucket first: bars_1m = bucket[bars, 'ts', 60.0, ...]",
            points, series, POINT_BUDGET
        ));
    }
    Ok(())
}

/// Linear scale: domain [d0, d1] → range [r0, r1].
#[derive(Debug, Clone, Copy)]
pub struct Scale {
    pub d0: f64,
    pub d1: f64,
    pub r0: f64,
    pub r1: f64,
}

impl Scale {
    pub fn new(d0: f64, d1: f64, r0: f64, r1: f64) -> Self {
        Scale { d0, d1, r0, r1 }
    }

    pub fn map(&self, v: f64) -> f64 {
        if (self.d1 - self.d0).abs() < f64::EPSILON {
            return (self.r0 + self.r1) / 2.0;
        }
        self.r0 + (v - self.d0) / (self.d1 - self.d0) * (self.r1 - self.r0)
    }
}

/// A "nice" step from the 1/2/5 ladder, at or above `rough`.
pub fn nice_step(rough: f64) -> f64 {
    if rough <= 0.0 {
        return 1.0;
    }
    let mag = 10f64.powf(rough.log10().floor());
    let frac = rough / mag;
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Y-axis ticks: ~`target` nice values covering [min, max].
pub fn y_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) || target == 0 {
        return vec![];
    }
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    let span = (max - min).max(f64::MIN_POSITIVE);
    let step = nice_step(span / target as f64);
    let first = (min / step).ceil() * step;
    let mut out = Vec::new();
    let mut v = first;
    // f64 accumulation drift guard: bound the count, snap to step grid
    let mut i = 0u32;
    while v <= max + step * 1e-9 && i < 1000 {
        out.push((v / step).round() * step);
        v += step;
        i += 1;
    }
    out
}

/// Pad a y-domain so lines don't kiss the frame; degenerate domains
/// (flat series) get a symmetric band.
pub fn pad_domain(min: f64, max: f64) -> (f64, f64) {
    if (max - min).abs() < f64::EPSILON {
        let pad = if min == 0.0 { 1.0 } else { min.abs() * 0.05 };
        return (min - pad, max + pad);
    }
    let pad = (max - min) * 0.05;
    (min - pad, max + pad)
}

/// Step ladder for time axes, in seconds.
const TIME_STEPS: &[i64] = &[
    1, 5, 15, 30,                 // seconds
    60, 300, 900, 1800,           // minutes
    3600, 10800, 21600, 43200,    // hours
    86400,                        // day
];

/// Time-axis ticks between two moments, aligned to boundaries **in the
/// named timezone**, labeled for humans. Returns (moment, label) pairs.
pub fn time_ticks(t0: f64, t1: f64, tz: &str, target: usize) -> Result<Vec<(f64, String)>, String> {
    let zone: Tz = tz
        .parse()
        .map_err(|_| format!("unknown timezone '{}' — use an IANA name like 'America/New_York' or 'UTC'", tz))?;
    if !(t0.is_finite() && t1.is_finite()) || t1 <= t0 || target == 0 {
        return Ok(vec![]);
    }
    let span = (t1 - t0) as i64;
    let rough = (span / target.max(1) as i64).max(1);
    let step = *TIME_STEPS
        .iter()
        .find(|&&s| s >= rough)
        .unwrap_or(&86400);

    // Align the first tick to a step boundary in the chart's zone: local
    // seconds-of-day decide the boundary, so a 30m step lands on :00/:30
    // as the viewer's clock reads them.
    let start = t0.floor() as i64;
    let local = local_datetime(start, &zone)?;
    let sod = i64::from(local.hour()) * 3600 + i64::from(local.minute()) * 60 + i64::from(local.second());
    let rem = sod % step;
    let mut t = if rem == 0 { start } else { start + (step - rem) };

    let mut out = Vec::new();
    let end = t1.ceil() as i64;
    let mut guard = 0u32;
    while t <= end && guard < 2000 {
        // A boundary that lands inside a DST gap simply doesn't exist as a
        // wall-clock time; the instant is still real — label what the zone
        // actually shows for it.
        let dt = local_datetime(t, &zone)?;
        let label = if step >= 86400 {
            format!("{:02}-{:02}", dt.month(), dt.day())
        } else {
            format!("{:02}:{:02}", dt.hour(), dt.minute())
        };
        out.push((t as f64, label));
        t += step;
        guard += 1;
    }
    Ok(out)
}

fn local_datetime(epoch: i64, zone: &Tz) -> Result<DateTime<Tz>, String> {
    match Utc.timestamp_opt(epoch, 0) {
        chrono::LocalResult::Single(u) => Ok(u.with_timezone(zone)),
        _ => Err(format!("moment {} is out of range", epoch)),
    }
}

/// SVG path for a polyline through (xs, ys), mapped through the scales.
/// Non-finite points break the line (gap), matching how price series
/// with holes should render.
pub fn path_line(xs: &[f64], ys: &[f64], sx: &Scale, sy: &Scale) -> String {
    let n = xs.len().min(ys.len());
    let mut out = String::with_capacity(n * 14);
    let mut pen_down = false;
    for i in 0..n {
        if !(xs[i].is_finite() && ys[i].is_finite()) {
            pen_down = false;
            continue;
        }
        let x = sx.map(xs[i]);
        let y = sy.map(ys[i]);
        if pen_down {
            out.push_str(&format!("L{:.2} {:.2}", x, y));
        } else {
            out.push_str(&format!("M{:.2} {:.2}", x, y));
            pen_down = true;
        }
    }
    out
}

/// One candle's geometry in range coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    pub cx: f64,      // wick x (center)
    pub body_x: f64,  // body left edge
    pub body_w: f64,
    pub body_top: f64,
    pub body_h: f64,
    pub wick_top: f64,
    pub wick_bot: f64,
    pub rising: bool, // close >= open
}

/// Lay out candles across the x-range: even bands, 20% gap, minimum
/// 1px body. o/h/l/c slices must be equal length.
pub fn candles(
    opens: &[f64],
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    sx_r0: f64,
    sx_r1: f64,
    sy: &Scale,
) -> Vec<Candle> {
    let n = opens.len().min(highs.len()).min(lows.len()).min(closes.len());
    if n == 0 {
        return vec![];
    }
    let band = (sx_r1 - sx_r0) / n as f64;
    let body_w = (band * 0.8).max(1.0);
    (0..n)
        .map(|i| {
            let cx = sx_r0 + band * (i as f64 + 0.5);
            let (o, c) = (opens[i], closes[i]);
            let y_o = sy.map(o);
            let y_c = sy.map(c);
            Candle {
                cx,
                body_x: cx - body_w / 2.0,
                body_w,
                body_top: y_o.min(y_c),
                body_h: (y_o - y_c).abs().max(1.0),
                wick_top: sy.map(highs[i]),
                wick_bot: sy.map(lows[i]),
                rising: c >= o,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_maps_endpoints_and_midpoint() {
        let s = Scale::new(0.0, 10.0, 0.0, 100.0);
        assert_eq!(s.map(0.0), 0.0);
        assert_eq!(s.map(10.0), 100.0);
        assert_eq!(s.map(5.0), 50.0);
        // Inverted range (SVG y grows downward)
        let sy = Scale::new(0.0, 10.0, 200.0, 0.0);
        assert_eq!(sy.map(0.0), 200.0);
        assert_eq!(sy.map(10.0), 0.0);
    }

    #[test]
    fn nice_steps_follow_the_ladder() {
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(1.2), 2.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert_eq!(nice_step(0.03), 0.05);
    }

    #[test]
    fn y_ticks_cover_a_price_range() {
        // The VIVK day: 0 → 3.92 wants steps of 1
        let t = y_ticks(0.0, 3.92, 5);
        assert_eq!(t, vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn time_ticks_align_to_ny_half_hours() {
        // The practice window: 06:55 → 09:30 ET on 2026-07-14
        let t0 = crate::moments::moment(2026, 7, 14, 6, 55, 0.0, "America/New_York").unwrap();
        let t1 = crate::moments::moment(2026, 7, 14, 9, 30, 0.0, "America/New_York").unwrap();
        let ticks = time_ticks(t0, t1, "America/New_York", 6).unwrap();
        let labels: Vec<&str> = ticks.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, vec!["07:00", "07:30", "08:00", "08:30", "09:00", "09:30"]);
    }

    #[test]
    fn time_ticks_survive_a_dst_gap() {
        // Spring forward 2026-03-08 in New York: 02:00–03:00 does not exist.
        let t0 = crate::moments::moment(2026, 3, 8, 1, 0, 0.0, "America/New_York").unwrap();
        let t1 = crate::moments::moment(2026, 3, 8, 4, 0, 0.0, "America/New_York").unwrap();
        let ticks = time_ticks(t0, t1, "America/New_York", 6).unwrap();
        assert!(!ticks.is_empty());
        // No tick may claim a wall-clock label inside the gap
        for (_, label) in &ticks {
            assert!(!label.starts_with("02:"), "impossible NY time {}", label);
        }
        // Unknown zones are loud
        assert!(time_ticks(t0, t1, "Mars/Olympus", 6).is_err());
    }

    #[test]
    fn line_path_breaks_at_gaps() {
        let sx = Scale::new(0.0, 3.0, 0.0, 300.0);
        let sy = Scale::new(0.0, 10.0, 100.0, 0.0);
        let p = path_line(&[0.0, 1.0, 2.0, 3.0], &[0.0, 5.0, f64::NAN, 10.0], &sx, &sy);
        assert_eq!(p, "M0.00 100.00L100.00 50.00M300.00 0.00");
    }

    #[test]
    fn candle_layout_fills_bands() {
        let sy = Scale::new(0.0, 10.0, 100.0, 0.0);
        let cs = candles(&[2.0, 6.0], &[5.0, 8.0], &[1.0, 4.0], &[4.0, 5.0], 0.0, 100.0, &sy);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].cx, 25.0);
        assert_eq!(cs[1].cx, 75.0);
        assert!(cs[0].rising);
        assert!(!cs[1].rising);
        assert!(cs[0].body_w > 0.0 && cs[0].body_w <= 50.0);
        // wick spans body
        assert!(cs[0].wick_top <= cs[0].body_top);
        assert!(cs[0].wick_bot >= cs[0].body_top + cs[0].body_h);
    }

    #[test]
    fn budget_refuses_and_teaches() {
        assert!(check_budget(900, "close").is_ok());
        let err = check_budget(9360, "close").unwrap_err();
        assert!(err.contains("9360") && err.contains("bucket["), "{}", err);
    }
}

/// Assemble the full chart SVG. Both modes call this — the interpreter
/// for its html strings, crow_web for its raw vnode — so the markup is
/// identical by construction. Colors reference the style tokens
/// (var(--crow-…)), which makes every chart theme-correct with zero
/// color plumbing.
pub struct ChartSpec<'a> {
    pub ts: &'a [f64],
    /// (open, high, low, close) column slices; empty slices = no candles.
    pub candles: (&'a [f64], &'a [f64], &'a [f64], &'a [f64]),
    /// Named line series: (label, values).
    pub lines: Vec<(&'a str, &'a [f64])>,
    /// Volume bars; empty = no volume pane.
    pub volume: &'a [f64],
    /// Play-head moment; NaN = none.
    pub marker: f64,
    pub tz: &'a str,
    pub width: f64,
    pub height: f64,
}

pub fn render_svg(spec: &ChartSpec) -> Result<String, String> {
    let n = spec.ts.len();
    check_budget(n, "chart")?;
    if n == 0 {
        return Ok(format!(
            "<svg class=\"crow-chart\" viewBox=\"0 0 {w} {h}\" width=\"100%\" height=\"{h}\"><text x=\"{cx}\" y=\"{cy}\" text-anchor=\"middle\" fill=\"var(--crow-muted)\" font-size=\"13\">no data yet</text></svg>",
            w = spec.width, h = spec.height, cx = spec.width / 2.0, cy = spec.height / 2.0
        ));
    }

    // Layout: main pane + optional volume strip, margins for axes.
    let ml = 54.0;
    let mr = 10.0;
    let mt = 8.0;
    let mb = 22.0;
    let has_vol = !spec.volume.is_empty();
    let vol_h = if has_vol { (spec.height * 0.18).floor() } else { 0.0 };
    let main_bot = spec.height - mb - if has_vol { vol_h + 6.0 } else { 0.0 };

    // Domains
    let t0 = spec.ts[0];
    let t1 = spec.ts[n - 1].max(t0 + 1.0);
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    let (o, h, l, c) = spec.candles;
    for v in l.iter().chain(h.iter()) {
        if v.is_finite() {
            ymin = ymin.min(*v);
            ymax = ymax.max(*v);
        }
    }
    for (_, vs) in &spec.lines {
        for v in vs.iter() {
            if v.is_finite() {
                ymin = ymin.min(*v);
                ymax = ymax.max(*v);
            }
        }
    }
    if !ymin.is_finite() {
        ymin = 0.0;
        ymax = 1.0;
    }
    let (ymin, ymax) = pad_domain(ymin, ymax);
    let sx = Scale::new(t0, t1, ml, spec.width - mr);
    let sy = Scale::new(ymin, ymax, main_bot, mt);

    let mut svg = String::with_capacity(16 * 1024);
    svg.push_str(&format!(
        "<svg class=\"crow-chart\" viewBox=\"0 0 {w} {h}\" width=\"100%\" height=\"{h}\" font-family=\"ui-monospace,monospace\">",
        w = spec.width, h = spec.height
    ));

    // Gridlines + y ticks
    for yt in y_ticks(ymin, ymax, 4) {
        let y = sy.map(yt);
        svg.push_str(&format!(
            "<line x1=\"{ml}\" y1=\"{y:.1}\" x2=\"{x2}\" y2=\"{y:.1}\" stroke=\"var(--crow-border)\" stroke-width=\"1\"/><text x=\"{tx}\" y=\"{ty:.1}\" text-anchor=\"end\" fill=\"var(--crow-muted)\" font-size=\"11\">{lab}</text>",
            ml = ml, x2 = spec.width - mr, tx = ml - 6.0, ty = y + 4.0,
            lab = trim_num(yt)
        ));
    }
    // Time ticks
    for (tt, lab) in time_ticks(t0, t1, spec.tz, 6)? {
        let x = sx.map(tt);
        svg.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y}\" text-anchor=\"middle\" fill=\"var(--crow-muted)\" font-size=\"11\">{lab}</text>",
            y = spec.height - 6.0
        ));
    }

    // Candles
    if !o.is_empty() {
        for cd in candles(o, h, l, c, ml, spec.width - mr, &sy) {
            let color = if cd.rising { "var(--crow-up)" } else { "var(--crow-down)" };
            svg.push_str(&format!(
                "<line x1=\"{cx:.1}\" y1=\"{wt:.1}\" x2=\"{cx:.1}\" y2=\"{wb:.1}\" stroke=\"{color}\" stroke-width=\"1\"/><rect x=\"{bx:.1}\" y=\"{bt:.1}\" width=\"{bw:.1}\" height=\"{bh:.1}\" fill=\"{color}\"/>",
                cx = cd.cx, wt = cd.wick_top, wb = cd.wick_bot,
                bx = cd.body_x, bt = cd.body_top, bw = cd.body_w, bh = cd.body_h
            ));
        }
    }

    // Lines
    for (label, vs) in &spec.lines {
        let d = path_line(spec.ts, vs, &sx, &sy);
        if !d.is_empty() {
            svg.push_str(&format!(
                "<path class=\"crow-series-{label}\" d=\"{d}\" fill=\"none\" stroke=\"var(--crow-accent)\" stroke-width=\"1.5\"/>"
            ));
        }
    }

    // Volume pane
    if has_vol {
        let vmax = spec
            .volume
            .iter()
            .cloned()
            .filter(|v| v.is_finite())
            .fold(0.0_f64, f64::max)
            .max(1.0);
        let v_top = main_bot + 6.0;
        let svy = Scale::new(0.0, vmax, spec.height - mb, v_top);
        let band = (spec.width - ml - mr) / n as f64;
        let bw = (band * 0.8).max(1.0);
        for (i, v) in spec.volume.iter().enumerate() {
            if !v.is_finite() || *v <= 0.0 {
                continue;
            }
            let x = ml + band * (i as f64 + 0.5) - bw / 2.0;
            let y = svy.map(*v);
            svg.push_str(&format!(
                "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{bh:.1}\" fill=\"var(--crow-muted)\" opacity=\"0.45\"/>",
                bh = (spec.height - mb - y).max(0.5)
            ));
        }
    }

    // Play-head marker
    if spec.marker.is_finite() && spec.marker >= t0 && spec.marker <= t1 {
        let x = sx.map(spec.marker);
        svg.push_str(&format!(
            "<line class=\"crow-marker\" x1=\"{x:.1}\" y1=\"{mt}\" x2=\"{x:.1}\" y2=\"{yb}\" stroke=\"var(--crow-accent)\" stroke-width=\"1.5\" stroke-dasharray=\"4 3\"/>",
            yb = spec.height - mb
        ));
    }

    svg.push_str("</svg>");
    Ok(svg)
}

fn trim_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{:.2}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
