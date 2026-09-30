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
/// fix (aggregate the frame first), never a silent downsample.
pub const POINT_BUDGET: usize = 1000;

/// Refuse oversized input with the fix spelled out.
pub fn check_budget(points: usize, series: &str) -> Result<(), String> {
    if points > POINT_BUDGET {
        return Err(format!(
            "chart got {} points for '{}' (budget {}) — aggregate first: fewer rows per query (GROUP BY a coarser interval in SQL) or head[frame, n]",
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
        assert!(err.contains("9360") && err.contains("aggregate first"), "{}", err);
    }
}

/// One strip under the main pane with its own y scale: line series plus
/// dashed reference levels (70 for an RSI pane, 0 for a delta).
pub struct Pane<'a> {
    pub lines: Vec<(&'a str, &'a [f64])>,
    pub levels: Vec<f64>,
}

/// A marker on the main pane: a moment, a price, a label, and a kind
/// (`buy` draws an up triangle in the up color, `sell` a down triangle in
/// the down color, anything else a dot in the accent color).
#[derive(Debug, Clone)]
pub struct Point {
    pub ts: f64,
    pub y: f64,
    pub label: String,
    pub kind: String,
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
    /// Named line series on the main pane: (label, values).
    pub lines: Vec<(&'a str, &'a [f64])>,
    /// Volume bars; empty = no volume strip.
    pub volume: &'a [f64],
    /// Play-head moment; NaN = none.
    pub marker: f64,
    pub tz: &'a str,
    pub width: f64,
    pub height: f64,
    /// Extra strips under the volume, each with its own scale.
    pub panes: Vec<Pane<'a>>,
    /// Markers on the main pane.
    pub points: Vec<Point>,
    /// A handler name: every bucket becomes a click target that sends
    /// its moment (`data-crow-args="[ts]"`) to the handler.
    pub seek: Option<&'a str>,
}

/// Reference levels keyed by column: `levels = map[rsi = [30, 70], cvd = [0]]`.
/// A pane draws the levels of every column it holds. Implemented for the
/// map shapes a program can produce and for `()` (no levels).
pub trait LevelMap {
    fn levels_for(&self, col: &str) -> Vec<f64>;
}
impl LevelMap for () {
    fn levels_for(&self, _col: &str) -> Vec<f64> {
        vec![]
    }
}
impl LevelMap for std::collections::BTreeMap<String, Vec<f64>> {
    fn levels_for(&self, col: &str) -> Vec<f64> {
        self.get(col).cloned().unwrap_or_default()
    }
}
impl LevelMap for std::collections::BTreeMap<String, Vec<i64>> {
    fn levels_for(&self, col: &str) -> Vec<f64> {
        self.get(col).map(|v| v.iter().map(|x| *x as f64).collect()).unwrap_or_default()
    }
}
impl LevelMap for std::collections::BTreeMap<String, crate::tensor::CrowTensor> {
    fn levels_for(&self, col: &str) -> Vec<f64> {
        self.get(col).map(|t| t.to_vec()).unwrap_or_default()
    }
}
impl LevelMap for std::collections::BTreeMap<String, f64> {
    fn levels_for(&self, col: &str) -> Vec<f64> {
        self.get(col).map(|x| vec![*x]).unwrap_or_default()
    }
}
impl LevelMap for std::collections::BTreeMap<String, i64> {
    fn levels_for(&self, col: &str) -> Vec<f64> {
        self.get(col).map(|x| vec![*x as f64]).unwrap_or_default()
    }
}
impl<T: LevelMap + ?Sized> LevelMap for &T {
    fn levels_for(&self, col: &str) -> Vec<f64> {
        (**self).levels_for(col)
    }
}

/// Series colors, by order of appearance across the whole chart.
pub const SERIES_COLORS: [&str; 6] = [
    "var(--crow-accent)",
    "var(--crow-series-2)",
    "var(--crow-series-3)",
    "var(--crow-series-4)",
    "var(--crow-series-5)",
    "var(--crow-series-6)",
];

fn series_color(i: usize) -> &'static str {
    SERIES_COLORS[i % SERIES_COLORS.len()]
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Joint finite min/max over several series.
fn extent(series: &[&[f64]]) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in series {
        for v in s.iter() {
            if v.is_finite() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
    }
    if !lo.is_finite() {
        (0.0, 1.0)
    } else {
        (lo, hi)
    }
}

/// Text inside an attribute of the drawing.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// What a chart shows, for a reader who cannot see it: its marks and how
/// many moments they span (`a chart of candles, vwap and volume, 390
/// points`).
fn chart_label(spec: &ChartSpec) -> String {
    let mut marks: Vec<String> = Vec::new();
    if !spec.candles.0.is_empty() {
        marks.push("candles".to_string());
    }
    marks.extend(spec.lines.iter().map(|(name, _)| name.to_string()));
    for pane in &spec.panes {
        marks.extend(pane.lines.iter().map(|(name, _)| name.to_string()));
    }
    if !spec.volume.is_empty() {
        marks.push("volume".to_string());
    }
    let listed = match marks.len() {
        0 => String::new(),
        1 => format!(" of {}", marks[0]),
        n => format!(" of {} and {}", marks[..n - 1].join(", "), marks[n - 1]),
    };
    format!("a chart{}, {} points", listed, spec.ts.len())
}

pub fn render_svg(spec: &ChartSpec) -> Result<String, String> {
    let n = spec.ts.len();
    check_budget(n, "chart")?;
    if n == 0 {
        return Ok(format!(
            "<svg class=\"crow-chart\" role=\"img\" aria-label=\"a chart, no data yet\" viewBox=\"0 0 {w} {h}\" width=\"100%\" height=\"{h}\"><text x=\"{cx}\" y=\"{cy}\" text-anchor=\"middle\" fill=\"var(--crow-muted)\" font-size=\"13\">no data yet</text></svg>",
            w = spec.width, h = spec.height, cx = spec.width / 2.0, cy = spec.height / 2.0
        ));
    }

    // Layout: main pane, optional volume strip, extra panes, time axis.
    let ml = 54.0;
    let mr = 10.0;
    let mt = 8.0;
    let mb = 22.0;
    let gap = 6.0;
    let has_vol = !spec.volume.is_empty();
    let vol_h = if has_vol { (spec.height * 0.16).floor() } else { 0.0 };
    let pane_h = (spec.height * 0.18).floor();
    let strips = vol_h + if has_vol { gap } else { 0.0 } + spec.panes.len() as f64 * (pane_h + gap);
    let main_bot = (spec.height - mb - strips).max(mt + 40.0);
    let x0 = ml;
    let x1 = spec.width - mr;

    // Domains
    let t0 = spec.ts[0];
    let t1 = spec.ts[n - 1].max(t0 + 1.0);
    let (o, h, l, c) = spec.candles;
    let mut main_series: Vec<&[f64]> = vec![l, h];
    for (_, vs) in &spec.lines {
        main_series.push(vs);
    }
    let (ymin, ymax) = extent(&main_series);
    let (ymin, ymax) = pad_domain(ymin, ymax);
    let sx = Scale::new(t0, t1, x0, x1);
    let sy = Scale::new(ymin, ymax, main_bot, mt);
    let band = (x1 - x0) / n as f64;

    let mut svg = String::with_capacity(32 * 1024);
    svg.push_str(&format!(
        "<svg class=\"crow-chart\" role=\"img\" aria-label=\"{label}\" viewBox=\"0 0 {w} {h}\" width=\"100%\" height=\"{h}\" font-family=\"ui-monospace,monospace\">",
        w = spec.width, h = spec.height, label = xml_escape(&chart_label(spec))
    ));

    // Gridlines + y ticks
    for yt in y_ticks(ymin, ymax, 4) {
        let y = sy.map(yt);
        svg.push_str(&format!(
            "<line x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\" stroke=\"var(--crow-border)\" stroke-width=\"1\"/><text x=\"{tx}\" y=\"{ty:.1}\" text-anchor=\"end\" fill=\"var(--crow-muted)\" font-size=\"11\">{lab}</text>",
            tx = x0 - 6.0, ty = y + 4.0, lab = trim_num(yt)
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
        for cd in candles(o, h, l, c, x0, x1, &sy) {
            let color = if cd.rising { "var(--crow-up)" } else { "var(--crow-down)" };
            svg.push_str(&format!(
                "<line x1=\"{cx:.1}\" y1=\"{wt:.1}\" x2=\"{cx:.1}\" y2=\"{wb:.1}\" stroke=\"{color}\" stroke-width=\"1\"/><rect x=\"{bx:.1}\" y=\"{bt:.1}\" width=\"{bw:.1}\" height=\"{bh:.1}\" fill=\"{color}\"/>",
                cx = cd.cx, wt = cd.wick_top, wb = cd.wick_bot,
                bx = cd.body_x, bt = cd.body_top, bw = cd.body_w, bh = cd.body_h
            ));
        }
    }

    // Main-pane lines, with a legend
    let mut series_i = 0usize;
    let mut legend_x = x0 + 4.0;
    for (label, vs) in &spec.lines {
        let color = series_color(series_i);
        let d = path_line(spec.ts, vs, &sx, &sy);
        if !d.is_empty() {
            svg.push_str(&format!(
                "<path class=\"crow-series-{lab}\" d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\"/>",
                lab = esc(label)
            ));
        }
        svg.push_str(&format!(
            "<text class=\"crow-legend\" x=\"{legend_x:.1}\" y=\"{y}\" fill=\"{color}\" font-size=\"10\">{lab}</text>",
            y = mt + 10.0, lab = esc(label)
        ));
        legend_x += label.chars().count() as f64 * 6.2 + 10.0;
        series_i += 1;
    }

    // Points on the main pane
    for p in &spec.points {
        if !p.ts.is_finite() || p.ts < t0 || p.ts > t1 {
            continue;
        }
        let x = sx.map(p.ts);
        let y = if p.y.is_finite() { sy.map(p.y) } else { mt + 6.0 };
        let (shape, color) = match p.kind.as_str() {
            "buy" => (
                format!("<path d=\"M{x:.1} {y0:.1}L{xl:.1} {y1:.1}L{xr:.1} {y1:.1}Z\"", y0 = y + 3.0, y1 = y + 11.0, xl = x - 5.0, xr = x + 5.0),
                "var(--crow-up)",
            ),
            "sell" => (
                format!("<path d=\"M{x:.1} {y0:.1}L{xl:.1} {y1:.1}L{xr:.1} {y1:.1}Z\"", y0 = y - 3.0, y1 = y - 11.0, xl = x - 5.0, xr = x + 5.0),
                "var(--crow-down)",
            ),
            _ => (format!("<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3.5\""), "var(--crow-accent)"),
        };
        svg.push_str(&format!(
            "{shape} class=\"crow-point crow-point-{kind}\" fill=\"{color}\"/>",
            kind = esc(&p.kind)
        ));
        if !p.label.is_empty() {
            svg.push_str(&format!(
                "<text x=\"{tx:.1}\" y=\"{ty:.1}\" fill=\"{color}\" font-size=\"10\">{lab}</text>",
                tx = x + 7.0, ty = y + 3.5, lab = esc(&p.label)
            ));
        }
    }

    // Volume strip
    let mut strip_top = main_bot + gap;
    if has_vol {
        let vmax = spec
            .volume
            .iter()
            .cloned()
            .filter(|v| v.is_finite())
            .fold(0.0_f64, f64::max)
            .max(1.0);
        let bot = strip_top + vol_h;
        let svy = Scale::new(0.0, vmax, bot, strip_top);
        let bw = (band * 0.8).max(1.0);
        svg.push_str(&format!(
            "<line x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\" stroke=\"var(--crow-border)\" stroke-width=\"1\"/>",
            y = strip_top - gap / 2.0
        ));
        for (i, v) in spec.volume.iter().enumerate() {
            if !v.is_finite() || *v <= 0.0 {
                continue;
            }
            let x = x0 + band * (i as f64 + 0.5) - bw / 2.0;
            let y = svy.map(*v);
            svg.push_str(&format!(
                "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{bh:.1}\" fill=\"var(--crow-muted)\" opacity=\"0.45\"/>",
                bh = (bot - y).max(0.5)
            ));
        }
        strip_top = bot + gap;
    }

    // Extra panes
    for pane in &spec.panes {
        let top = strip_top;
        let bot = top + pane_h;
        let mut all: Vec<&[f64]> = pane.lines.iter().map(|(_, v)| *v).collect();
        let levels_slice: &[f64] = &pane.levels;
        all.push(levels_slice);
        let (lo, hi) = extent(&all);
        let (lo, hi) = pad_domain(lo, hi);
        let psy = Scale::new(lo, hi, bot, top);
        svg.push_str(&format!(
            "<g class=\"crow-pane\"><line x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\" stroke=\"var(--crow-border)\" stroke-width=\"1\"/>",
            y = top - gap / 2.0
        ));
        for yt in y_ticks(lo, hi, 2) {
            let y = psy.map(yt);
            svg.push_str(&format!(
                "<text x=\"{tx}\" y=\"{ty:.1}\" text-anchor=\"end\" fill=\"var(--crow-muted)\" font-size=\"10\">{lab}</text>",
                tx = x0 - 6.0, ty = y + 3.5, lab = trim_num(yt)
            ));
        }
        for lv in &pane.levels {
            if !lv.is_finite() {
                continue;
            }
            let y = psy.map(*lv);
            svg.push_str(&format!(
                "<line class=\"crow-level\" x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\" stroke=\"var(--crow-muted)\" stroke-width=\"1\" stroke-dasharray=\"3 3\"/>"
            ));
        }
        let mut lx = x0 + 4.0;
        for (label, vs) in &pane.lines {
            let color = series_color(series_i);
            let d = path_line(spec.ts, vs, &sx, &psy);
            if !d.is_empty() {
                svg.push_str(&format!(
                    "<path class=\"crow-series-{lab}\" d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.2\"/>",
                    lab = esc(label)
                ));
            }
            svg.push_str(&format!(
                "<text class=\"crow-legend\" x=\"{lx:.1}\" y=\"{y:.1}\" fill=\"{color}\" font-size=\"10\">{lab}</text>",
                y = top + 9.0, lab = esc(label)
            ));
            lx += label.chars().count() as f64 * 6.2 + 10.0;
            series_i += 1;
        }
        svg.push_str("</g>");
        strip_top = bot + gap;
    }

    // Play-head marker, across every pane
    if spec.marker.is_finite() && spec.marker >= t0 && spec.marker <= t1 {
        let x = sx.map(spec.marker);
        svg.push_str(&format!(
            "<line class=\"crow-marker\" x1=\"{x:.1}\" y1=\"{mt}\" x2=\"{x:.1}\" y2=\"{yb}\" stroke=\"var(--crow-accent)\" stroke-width=\"1.5\" stroke-dasharray=\"4 3\"/>",
            yb = spec.height - mb
        ));
    }

    // Seek targets: one transparent band per bucket, on top of everything
    if let Some(handler) = spec.seek {
        let yb = spec.height - mb;
        for (i, t) in spec.ts.iter().enumerate() {
            if !t.is_finite() {
                continue;
            }
            let x = x0 + band * i as f64;
            svg.push_str(&format!(
                "<rect class=\"crow-seek\" x=\"{x:.1}\" y=\"{mt}\" width=\"{bw:.2}\" height=\"{bh:.1}\" fill=\"transparent\" style=\"cursor:pointer\" data-crow-click=\"{h}\" data-crow-args=\"[{t}]\"/>",
                bw = band, bh = yb - mt, h = esc(handler), t = trim_ts(*t)
            ));
        }
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

/// A moment as a JSON number: whole seconds stay integers, fractions keep
/// three places.
fn trim_ts(t: f64) -> String {
    if (t - t.round()).abs() < 1e-6 {
        format!("{}", t.round() as i64)
    } else {
        let s = format!("{:.3}", t);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    fn spec<'a>(ts: &'a [f64], close: &'a [f64], rsi: &'a [f64]) -> ChartSpec<'a> {
        ChartSpec {
            ts,
            candles: (&[], &[], &[], &[]),
            lines: vec![("close", close)],
            volume: &[],
            marker: f64::NAN,
            tz: "UTC",
            width: 400.0,
            height: 200.0,
            panes: vec![Pane { lines: vec![("rsi", rsi)], levels: vec![70.0] }],
            points: vec![Point { ts: ts[1], y: close[1], label: "B".into(), kind: "buy".into() }],
            seek: Some("jump"),
        }
    }

    #[test]
    fn panes_points_and_seek_targets_render() {
        let ts = [0.0, 60.0, 120.0];
        let close = [1.0, 2.0, 1.5];
        let rsi = [f64::NAN, 80.0, 40.0];
        let svg = render_svg(&spec(&ts, &close, &rsi)).unwrap();
        assert!(svg.contains("class=\"crow-pane\""));
        assert!(svg.contains("crow-level"));
        assert!(svg.contains("crow-series-rsi"));
        assert!(svg.contains("crow-point-buy"));
        assert!(svg.contains("data-crow-click=\"jump\" data-crow-args=\"[60]\""));
        assert_eq!(svg.matches("crow-seek").count(), 3);
        // the pane's own series takes the next palette color
        assert!(svg.contains("var(--crow-series-2)"));
    }

    /// A chart is an image whose name says what it draws, for a reader who
    /// cannot see it — an empty one too
    #[test]
    fn a_chart_names_what_it_draws() {
        let ts = [0.0, 60.0, 120.0];
        let close = [1.0, 2.0, 1.5];
        let rsi = [f64::NAN, 80.0, 40.0];
        let svg = render_svg(&spec(&ts, &close, &rsi)).unwrap();
        assert!(svg.starts_with("<svg class=\"crow-chart\" role=\"img\" aria-label=\"a chart of close and rsi, 3 points\""), "{}", &svg[..160]);
        let none = ChartSpec { ts: &[], candles: (&[], &[], &[], &[]), lines: vec![], volume: &[], marker: f64::NAN, tz: "UTC", width: 400.0, height: 200.0, panes: vec![], points: vec![], seek: None };
        let empty = render_svg(&none).unwrap_or_default();
        assert!(empty.contains("role=\"img\" aria-label=\"a chart, no data yet\""), "{}", empty);
    }
}
