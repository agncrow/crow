//! The frame module's verbs beyond the first (spec §13, frame): aggregates
//! past sum, mean, min, max and count; windows over a span of time; sorts,
//! joins and distinct rows on several keys; buckets of moments; text over a
//! column; unpivot; fills by direction; complete rows; a conditional
//! column; casts; a sample. Each is one kernel both modes call, held to
//! pandas by tests/parity/185_frames_held_to_pandas.co. A misuse is the
//! reason the program stops with (docs/bug-classes.md §4).

use std::collections::HashMap;
use std::sync::Arc;


use crate::cell::Cell;
use crate::frame::{cells_equal, codes, gather_column, merged_keys, order_of, take_rows, take_some_rows, AggFn, Column, CrowColumn, CrowFrame, Roll, Validity};
use crate::frames::Cells;
use crate::moments::Moment;
use crate::textcol::{Text, TextBuilder};

// ── statistics the aggregates share ─────────────────────────────────

/// The median: the middle value, or the mean of the two middles (pandas's
/// `median_linear` and its rolling median agree on this form).
pub fn median_of(values: &[f64]) -> f64 {
    let mut s = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    median_sorted(&s)
}

pub(crate) fn median_sorted(s: &[f64]) -> f64 {
    let n = s.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// A percentile, interpolated between the two values it falls between
/// (numpy's and pandas's `linear`, Julia's default): the value at
/// `p / 100 · (n - 1)` along the sorted values.
pub fn percentile_of(values: &[f64], p: u8) -> f64 {
    let mut s = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile_sorted(&s, p as f64 / 100.0)
}

pub(crate) fn percentile_sorted(s: &[f64], q: f64) -> f64 {
    let n = s.len();
    if n == 0 {
        return f64::NAN;
    }
    let idx = q * (n - 1) as f64;
    let lo = idx as usize;
    let frac = idx - lo as f64;
    if frac == 0.0 || lo + 1 >= n {
        s[lo]
    } else {
        // (one rounding, as pandas's kernel computes it)
        (s[lo + 1] - s[lo]).mul_add(frac, s[lo])
    }
}

/// The population variance by Welford's recurrence, in the order pandas's
/// `group_var` adds the values and with its fused multiply-add, so a
/// group's variance is pandas's to the last digit; no value is a NaN of
/// its own.
pub fn welford_var(values: &[f64]) -> f64 {
    let mut nobs = 0.0f64;
    let mut mean = 0.0f64;
    let mut out = 0.0f64;
    for &val in values {
        nobs += 1.0;
        let oldmean = mean;
        mean += (val - oldmean) / nobs;
        out = (val - mean).mul_add(val - oldmean, out);
    }
    if nobs < 1.0 {
        f64::NAN
    } else {
        out / nobs
    }
}

/// A window's variance as pandas 3.0's `roll_var` keeps it: Welford's
/// mean and sum of squared deviations, a Kahan compensation for what is
/// added and another for what is removed, each value added as its row
/// comes and taken off as it leaves, the squared deviation added with one
/// rounding (a fused multiply-add, which pandas's kernel compiles to), and
/// a window whose sum collapsed by more than a thousand epsilons marked
/// unstable, to be computed afresh.
#[derive(Default)]
pub(crate) struct RollVar {
    nobs: f64,
    mean: f64,
    ssqdm: f64,
    comp_add: f64,
    comp_remove: f64,
    pub(crate) unstable: bool,
}

/// pandas's `InvCondTol`: a thousand times the machine epsilon.
const INV_COND_TOL: f64 = f64::EPSILON * 1e3;

impl RollVar {
    pub(crate) fn add(&mut self, val: f64) {
        if val.is_nan() {
            return;
        }
        let prev_m2 = self.ssqdm;
        self.nobs += 1.0;
        let prev_mean = self.mean - self.comp_add;
        let y = val - self.comp_add;
        let t = y - self.mean;
        self.comp_add = t + self.mean - y;
        self.mean += t / self.nobs;
        self.ssqdm = (val - prev_mean).mul_add(val - self.mean, self.ssqdm);
        if prev_m2 * INV_COND_TOL > self.ssqdm {
            self.unstable = true;
        }
    }

    pub(crate) fn remove(&mut self, val: f64) {
        if val.is_nan() {
            return;
        }
        let prev_m2 = self.ssqdm;
        self.nobs -= 1.0;
        if self.nobs > 0.0 {
            let prev_mean = self.mean - self.comp_remove;
            let y = val - self.comp_remove;
            let t = y - self.mean;
            self.comp_remove = t + self.mean - y;
            self.mean -= t / self.nobs;
            self.ssqdm = (-(val - prev_mean)).mul_add(val - self.mean, self.ssqdm);
            if prev_m2 * INV_COND_TOL > self.ssqdm {
                self.unstable = true;
            }
        } else {
            self.mean = 0.0;
            self.ssqdm = 0.0;
            self.unstable = false;
        }
    }

    /// The population variance of what is in the window (`NaN` of nothing).
    pub(crate) fn var(&self) -> f64 {
        if self.nobs < 1.0 {
            f64::NAN
        } else {
            self.ssqdm / self.nobs
        }
    }
}

// ── windows ─────────────────────────────────────────────────────────

/// What a window computes: an aggregate over the window's rows, or a cell
/// taken from another row of the partition.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Window {
    Agg(AggFn),
    Lag,
    Lead,
    Rank,
}

impl Window {
    fn named(how: &str) -> Option<Window> {
        Some(match how {
            "lag" => Window::Lag,
            "lead" => Window::Lead,
            "rank" => Window::Rank,
            other => Window::Agg(AggFn::from_str(other)?),
        })
    }

    fn word(self) -> String {
        match self {
            Window::Agg(a) => a.word(),
            Window::Lag => "lag".to_string(),
            Window::Lead => "lead".to_string(),
            Window::Rank => "rank".to_string(),
        }
    }
}

/// Where each row's window starts within its partition: `n` rows back (every
/// row so far when `n` is 0), or the rows whose clock is within `span` before
/// the row's own — the partition in ascending order of its clock, or the
/// reason it is not.
fn window_starts(rows: &[usize], n: usize, clock: Option<(&[f64], f64)>, on: &str) -> Result<Vec<usize>, String> {
    match clock {
        None => Ok((0..rows.len()).map(|k| if n == 0 { 0 } else { k.saturating_sub(n - 1) }).collect()),
        Some((t, span)) => {
            let mut start = 0;
            let mut out = Vec::with_capacity(rows.len());
            for k in 0..rows.len() {
                if k > 0 && t[rows[k]] < t[rows[k - 1]] {
                    return Err(format!(
                        "window: a span is measured along '{}' in ascending order, and row {} comes after row {} but is earlier — sort[df, '{}'] first",
                        on,
                        rows[k],
                        rows[k - 1],
                        on
                    ));
                }
                while t[rows[start]] <= t[rows[k]] - span {
                    start += 1;
                }
                out.push(start);
            }
            Ok(out)
        }
    }
}

impl CrowFrame {
    /// `window[frame, col, how, n = rows, by = keys, span = seconds, on =
    /// column]`: a column computed over each row's partition — the rows
    /// sharing the `by` columns' cells, the whole frame without — in the
    /// frame's order: an aggregate (`AggFn`) over the trailing `n` rows,
    /// over every row so far when `n` is 0, or over the rows whose `on`
    /// cell lies within `span` seconds before the row's own (a moment's
    /// column, or numbers); `'lag'` and `'lead'` the cell `n` rows back or
    /// ahead, missing where there is none; `'rank'` the cell's rank in its
    /// partition, equal cells sharing a rank and the next rank skipping.
    pub fn window_over(&self, col: &str, how: &str, n: i64, by: &[&str], span: f64, on: &str) -> Result<CrowColumn, String> {
        let ci = self.col_index(col).ok_or_else(|| format!("window: column '{}' not found", col))?;
        let how = Window::named(how).ok_or_else(|| format!("window: no window '{}' — write {}, 'lag', 'lead' or 'rank'", how, AggFn::WORDS))?;
        if n < 0 {
            return Err(format!("window: n is 0 or more (got {})", n));
        }
        if matches!(how, Window::Lag | Window::Lead) && n == 0 {
            return Err(format!("window: {} needs n of 1 or more, the rows between (got 0)", how.word()));
        }
        let timed = span != 0.0 || !on.is_empty();
        if timed {
            if !(span > 0.0) || !span.is_finite() {
                return Err(format!("window: a span is a positive number of seconds (got {})", span));
            }
            if on.is_empty() {
                return Err("window: a span is measured on a column — window[df, 'px', 'mean', span = 300, on = 'ts']".to_string());
            }
            if !matches!(how, Window::Agg(_)) {
                return Err(format!("window: {} counts rows, and a span is seconds — give n = rows instead", how.word()));
            }
            if n != 0 {
                return Err("window: a window is n rows or a span of seconds, not both".to_string());
            }
        }
        let n = n as usize;
        let by_idx: Vec<usize> = by.iter().map(|k| self.col_index(k).ok_or_else(|| format!("window: column '{}' not found", k))).collect::<Result<_, _>>()?;
        let nrows = self.nrows();
        let (mut ids, first_rows) = if by_idx.is_empty() { (vec![0u32; nrows], if nrows > 0 { vec![0] } else { vec![] }) } else { self.partitions(&by_idx) };
        // the clock a span is measured on: a moment's column in
        // milliseconds, numbers as they are; a row with no clock is in no
        // window
        let clock: Option<(Vec<f64>, f64)> = if timed {
            let oi = self.col_index(on).ok_or_else(|| format!("window: column '{}' not found", on))?;
            let valid = self.validity[oi].as_ref();
            let (values, scaled): (Vec<f64>, f64) = match self.data[oi].as_ref() {
                Column::DateTime(ms, _) => (ms.iter().map(|m| *m as f64).collect(), span * 1000.0),
                Column::Int(v) => (v.iter().map(|x| *x as f64).collect(), span),
                Column::Float(v) => (v.clone(), span),
                other => return Err(format!("window: on = '{}' names a column of {}, and a span is measured on moments or numbers", on, other.dtype_name())),
            };
            for (row, id) in ids.iter_mut().enumerate() {
                if !valid[row] || values[row].is_nan() {
                    *id = u32::MAX;
                }
            }
            Some((values, scaled))
        } else {
            None
        };
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); first_rows.len()];
        for (row, &id) in ids.iter().enumerate() {
            if id != u32::MAX {
                members[id as usize].push(row);
            }
        }
        let column = self.data[ci].as_ref();
        let valid = self.validity[ci].as_ref();
        let in_partition: Validity = (0..nrows).map(|r| ids[r] != u32::MAX).collect();
        match how {
            Window::Lag | Window::Lead => {
                let mut picks: Vec<Option<usize>> = vec![None; nrows];
                for rows in &members {
                    for (k, &row) in rows.iter().enumerate() {
                        let from = if matches!(how, Window::Lag) { k.checked_sub(n) } else { k.checked_add(n).filter(|&j| j < rows.len()) };
                        picks[row] = from.map(|j| rows[j]).filter(|&r| valid[r]);
                    }
                }
                let (data, ok) = take_some_rows(column, &picks);
                Ok(CrowColumn { data: Arc::new(data), valid: Arc::new(Validity::from(ok)) })
            }
            Window::Rank => {
                if matches!(column, Column::Bool(_)) {
                    return Err(format!("window: rank orders numbers or text, and column '{}' holds bools", col));
                }
                if matches!(column, Column::Vectors(..)) {
                    return Err(format!("window: rank orders numbers or text, and column '{}' holds vectors", col));
                }
                let mut out = vec![0i64; nrows];
                for rows in &members {
                    let present: Vec<usize> = rows.iter().copied().filter(|&r| valid[r]).collect();
                    let order = order_of(column, &present);
                    let mut rank = 0;
                    for (k, &p) in order.iter().enumerate() {
                        let row = present[p];
                        if k == 0 || !cells_equal(column, present[order[k - 1]], row) {
                            rank = k as i64 + 1;
                        }
                        out[row] = rank;
                    }
                }
                let present: Validity = (0..nrows).map(|r| ids[r] != u32::MAX && valid[r]).collect();
                Ok(CrowColumn { data: Arc::new(Column::Int(out)), valid: Arc::new(present) })
            }
            Window::Agg(agg) => {
                let starts: Vec<Vec<usize>> = members.iter().map(|rows| window_starts(rows, n, clock.as_ref().map(|(t, s)| (t.as_slice(), *s)), on)).collect::<Result<_, _>>()?;
                self.aggregate_windows(col, ci, agg, &members, &starts, &in_partition)
            }
        }
    }

    /// An aggregate over each row's window (`rows[starts[k]..=k]` of its
    /// partition), the partitions in parallel.
    fn aggregate_windows(&self, col: &str, ci: usize, agg: AggFn, members: &[Vec<usize>], starts: &[Vec<usize>], in_partition: &Validity) -> Result<CrowColumn, String> {
        let nrows = self.nrows();
        let column = self.data[ci].as_ref();
        let valid = self.validity[ci].as_ref();
        if matches!(column, Column::Vectors(..)) && !matches!(agg, AggFn::Count | AggFn::First | AggFn::Last | AggFn::Distinct) {
            return Err(format!("window: {} of column '{}', which holds vectors", agg.word(), col));
        }
        // a number per row where numbers are taken; a missing cell is the
        // reason there are none, as where numbers are handed to a function
        let numbers: Option<Vec<f64>> = if agg.numeric() || matches!(agg, AggFn::Min | AggFn::Max) {
            Some(self.col_native(col).ok_or_else(|| format!("window: column '{}' not found", col))?.numbers()?.to_vec())
        } else {
            None
        };
        let ints = matches!(column, Column::Int(_)) && matches!(agg, AggFn::Sum | AggFn::Min | AggFn::Max);
        match agg {
            AggFn::Sum | AggFn::Mean | AggFn::Min | AggFn::Max | AggFn::Median | AggFn::Std | AggFn::Var | AggFn::Percentile(_) => {
                let xs = numbers.as_deref().unwrap_or(&[]);
                let parts: Vec<Vec<(usize, f64)>> = crate::cores::map_range(members.len(), xs.len(), |g| {
                    let (rows, st) = (&members[g], &starts[g]);
                    match agg {
                        AggFn::Min | AggFn::Max => extremes(agg, xs, rows, st),
                        AggFn::Median | AggFn::Percentile(_) => quantiles(agg, xs, rows, st),
                        _ => totals(agg, xs, rows, st),
                    }
                });
                let mut out = vec![0.0; nrows];
                for (row, value) in parts.into_iter().flatten() {
                    out[row] = value;
                }
                let data = if ints { Column::Int(out.iter().map(|x| *x as i64).collect()) } else { Column::Float(out) };
                Ok(CrowColumn { data: Arc::new(data), valid: Arc::new(in_partition.clone()) })
            }
            AggFn::Count => {
                let mut out = vec![0i64; nrows];
                for (rows, st) in members.iter().zip(starts) {
                    let mut from = 0;
                    let mut count = 0i64;
                    for (k, &row) in rows.iter().enumerate() {
                        while from < st[k] {
                            count -= valid[rows[from]] as i64;
                            from += 1;
                        }
                        count += valid[row] as i64;
                        out[row] = count;
                    }
                }
                Ok(CrowColumn { data: Arc::new(Column::Int(out)), valid: Arc::new(in_partition.clone()) })
            }
            AggFn::Distinct => {
                let coded = codes(column, valid);
                let mut out = vec![0i64; nrows];
                for (rows, st) in members.iter().zip(starts) {
                    let mut from = 0;
                    let mut held: HashMap<u32, usize> = HashMap::new();
                    for (k, &row) in rows.iter().enumerate() {
                        while from < st[k] {
                            let gone = coded[rows[from]];
                            if gone != u32::MAX {
                                if let Some(n) = held.get_mut(&gone) {
                                    *n -= 1;
                                    if *n == 0 {
                                        held.remove(&gone);
                                    }
                                }
                            }
                            from += 1;
                        }
                        if coded[row] != u32::MAX {
                            *held.entry(coded[row]).or_insert(0) += 1;
                        }
                        out[row] = held.len() as i64;
                    }
                }
                Ok(CrowColumn { data: Arc::new(Column::Int(out)), valid: Arc::new(in_partition.clone()) })
            }
            AggFn::First | AggFn::Last => {
                let mut picks: Vec<Option<usize>> = vec![None; nrows];
                for (rows, st) in members.iter().zip(starts) {
                    for (k, &row) in rows.iter().enumerate() {
                        let window = &rows[st[k]..=k];
                        picks[row] = if agg == AggFn::First { window.iter().copied().find(|&r| valid[r]) } else { window.iter().rev().copied().find(|&r| valid[r]) };
                    }
                }
                let (data, ok) = take_some_rows(column, &picks);
                Ok(CrowColumn { data: Arc::new(data), valid: Arc::new(Validity::from(ok)) })
            }
        }
    }
}

/// A partition's sums, means and deviations over its windows (`rows[st[k]..=k]`
/// for each `k`): compensated running totals, each cell added as its row
/// comes and taken off as it leaves (pandas's order); a window that shares
/// no row with the one before starts afresh, as pandas's does, and a
/// deviation whose sum collapsed is computed afresh over the window.
fn totals(agg: AggFn, xs: &[f64], rows: &[usize], st: &[usize]) -> Vec<(usize, f64)> {
    let mut acc = Vec::with_capacity(rows.len());
    let mut from = 0;
    let mut roll = Roll::default();
    let mut var = RollVar::default();
    for (k, &row) in rows.iter().enumerate() {
        let x = xs[row];
        let afresh = st[k] >= k;
        if afresh {
            roll = Roll::default();
            var = RollVar::default();
            from = k;
        }
        while from < st[k] {
            let gone = xs[rows[from]];
            match agg {
                AggFn::Sum | AggFn::Mean => roll.remove(gone),
                _ => var.remove(gone),
            }
            from += 1;
        }
        let value = match agg {
            AggFn::Sum => {
                roll.add(x);
                roll.sum()
            }
            AggFn::Mean => {
                roll.add(x);
                roll.mean()
            }
            _ => {
                var.add(x);
                if var.unstable && !afresh {
                    var = RollVar::default();
                    for &r in &rows[st[k]..=k] {
                        var.add(xs[r]);
                    }
                    var.unstable = false;
                }
                if agg == AggFn::Var { var.var() } else { var.var().max(0.0).sqrt() }
            }
        };
        acc.push((row, value));
    }
    acc
}

/// A partition's minimum or maximum over its windows: the positions whose
/// cell could still be the answer, kept in a queue in the cell's order (a
/// window had been scanned, and a span of a day over a year of ticks took
/// its square).
fn extremes(agg: AggFn, xs: &[f64], rows: &[usize], st: &[usize]) -> Vec<(usize, f64)> {
    let mut acc = Vec::with_capacity(rows.len());
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let beaten = |held: f64, x: f64| if agg == AggFn::Min { held >= x } else { held <= x };
    for (k, &row) in rows.iter().enumerate() {
        let x = xs[row];
        while queue.front().is_some_and(|&p| p < st[k]) {
            queue.pop_front();
        }
        while queue.back().is_some_and(|&p| beaten(xs[rows[p]], x)) {
            queue.pop_back();
        }
        queue.push_back(k);
        acc.push((row, queue.front().map_or(x, |&p| xs[rows[p]])));
    }
    acc
}

/// Counts over the ranks of a partition's cells: how many of the window's
/// cells are at or below each rank, and the rank with `k` cells below it,
/// each in logarithmic time (a Fenwick tree).
struct Ranked {
    tree: Vec<u32>,
}

impl Ranked {
    fn new(n: usize) -> Ranked {
        Ranked { tree: vec![0; n + 1] }
    }

    fn add(&mut self, rank: usize, delta: i32) {
        let mut i = rank + 1;
        while i < self.tree.len() {
            self.tree[i] = (self.tree[i] as i32 + delta) as u32;
            i += i & i.wrapping_neg();
        }
    }

    /// The rank with `k` of the window's cells below it (the k-th smallest).
    fn kth(&self, k: usize) -> usize {
        let n = self.tree.len() - 1;
        let mut bit = 1usize;
        while bit * 2 <= n {
            bit *= 2;
        }
        let mut pos = 0usize;
        let mut remaining = k + 1;
        while bit > 0 {
            let next = pos + bit;
            if next <= n && (self.tree[next] as usize) < remaining {
                pos = next;
                remaining -= self.tree[next] as usize;
            }
            bit >>= 1;
        }
        pos
    }
}

/// A partition's medians or percentiles over its windows: the cells ranked
/// once, each window's cells counted over the ranks, the middle cells found
/// by rank (a sorted vector had been kept, and every row so far cost a
/// move of it).
fn quantiles(agg: AggFn, xs: &[f64], rows: &[usize], st: &[usize]) -> Vec<(usize, f64)> {
    let n = rows.len();
    let widest = (0..n).map(|k| k + 1 - st[k]).max().unwrap_or(0);
    // a window of a few cells is a small sorted vector, moved cell by
    // cell (ranking every cell of the partition costs more than it saves)
    if widest <= 64 {
        let mut acc = Vec::with_capacity(n);
        let mut sorted: Vec<f64> = Vec::with_capacity(widest);
        let mut from = 0;
        for (k, &row) in rows.iter().enumerate() {
            while from < st[k] {
                let gone = xs[rows[from]];
                let at = sorted.partition_point(|v| *v < gone);
                if at < sorted.len() {
                    sorted.remove(at);
                }
                from += 1;
            }
            let x = xs[row];
            let at = sorted.partition_point(|v| *v <= x);
            sorted.insert(at, x);
            let value = match agg {
                AggFn::Median => median_sorted(&sorted),
                AggFn::Percentile(p) => percentile_sorted(&sorted, p as f64 / 100.0),
                _ => f64::NAN,
            };
            acc.push((row, value));
        }
        return acc;
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| xs[rows[a]].partial_cmp(&xs[rows[b]]).unwrap_or_else(|| xs[rows[a]].is_nan().cmp(&xs[rows[b]].is_nan())));
    let mut rank_of = vec![0usize; n];
    let mut sorted = vec![0.0; n];
    for (r, &k) in order.iter().enumerate() {
        rank_of[k] = r;
        sorted[r] = xs[rows[k]];
    }
    let mut counts = Ranked::new(n);
    let mut acc = Vec::with_capacity(n);
    let mut from = 0;
    let mut held = 0usize;
    for (k, &row) in rows.iter().enumerate() {
        while from < st[k] {
            counts.add(rank_of[from], -1);
            held -= 1;
            from += 1;
        }
        counts.add(rank_of[k], 1);
        held += 1;
        let at = |i: usize| sorted[counts.kth(i)];
        let value = match agg {
            AggFn::Median => {
                if held % 2 == 1 {
                    at(held / 2)
                } else {
                    (at(held / 2 - 1) + at(held / 2)) / 2.0
                }
            }
            AggFn::Percentile(p) => {
                let idx = p as f64 / 100.0 * (held - 1) as f64;
                let lo = idx as usize;
                let frac = idx - lo as f64;
                if frac == 0.0 || lo + 1 >= held {
                    at(lo)
                } else {
                    let (a, b) = (at(lo), at(lo + 1));
                    (b - a).mul_add(frac, a)
                }
            }
            _ => f64::NAN,
        };
        acc.push((row, value));
    }
    acc
}

// ── sorting on several keys ─────────────────────────────────────────

impl CrowFrame {
    /// `sort[frame, ['a', 'b'], desc = [false, true]]`: the rows by the
    /// first key, ties by the second, and so on — stable, a missing cell
    /// after every key either way (`sort` on one key).
    pub fn sort_keys(&self, keys: &[&str], desc: &[bool]) -> Result<CrowFrame, String> {
        if keys.is_empty() {
            return Err("sort: a key is needed — sort[df, 'col'] or sort[df, ['a', 'b']]".to_string());
        }
        if desc.len() != keys.len() {
            return Err(format!("sort: desc names {} directions for {} keys — one bool, or one per key", desc.len(), keys.len()));
        }
        if keys.len() == 1 {
            return self.sort(keys[0], desc[0]);
        }
        let packed: Vec<Vec<(bool, u64)>> = keys.iter().zip(desc).map(|(k, &d)| self.sort_key(k, d)).collect::<Result<_, _>>()?;
        let mut indices: Vec<usize> = (0..self.nrows()).collect();
        crate::cores::sort_by(&mut indices, |&a, &b| {
            for key in &packed {
                let o = key[a].cmp(&key[b]);
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            a.cmp(&b)
        });
        Ok(self.reorder_rows(&indices))
    }

    /// A column as a sort key: (missing, key) per row, the key packed so
    /// that `u64` order is the cells' order, flipped for a descending sort.
    fn sort_key(&self, name: &str, desc: bool) -> Result<Vec<(bool, u64)>, String> {
        let idx = self.col_index(name).ok_or_else(|| format!("sort: column '{}' not found", name))?;
        let valid = self.validity[idx].as_ref();
        let n = self.nrows();
        let flip = |k: u64| if desc { u64::MAX - k } else { k };
        let keyed = |key: &dyn Fn(usize) -> Option<u64>| -> Vec<(bool, u64)> { (0..n).map(|i| match key(i) { Some(k) if valid[i] => (false, flip(k)), _ => (true, 0) }).collect() };
        Ok(match self.data[idx].as_ref() {
            Column::Vectors(..) => return Err(format!("sort: column '{}' holds vectors, which have no order", name)),
            Column::Int(v) => keyed(&|i| Some((v[i] as u64) ^ (1 << 63))),
            Column::DateTime(ms, _) => keyed(&|i| Some((ms[i] as u64) ^ (1 << 63))),
            Column::Bool(v) => keyed(&|i| Some(v[i] as u64)),
            Column::Float(v) => keyed(&|i| {
                let x = v[i];
                if x.is_nan() {
                    return None;
                }
                let bits = x.to_bits();
                Some(if x.is_sign_negative() { !bits } else { bits | (1 << 63) })
            }),
            Column::Str(t) => {
                let rank = text_ranks(t, valid);
                keyed(&|i| Some(rank[i]))
            }
        })
    }
}

/// Each cell's rank among a text column's distinct present values, by
/// character code (equal cells one rank).
fn text_ranks(t: &Text, valid: &Validity) -> Vec<u64> {
    let n = t.len();
    if let Some((codes, values)) = t.dictionary() {
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|&a, &b| values.get(a).cmp(values.get(b)));
        let mut rank = vec![0u64; values.len()];
        for (r, &code) in order.iter().enumerate() {
            rank[code] = r as u64;
        }
        return codes.iter().map(|&c| rank[c as usize]).collect();
    }
    let mut order: Vec<usize> = (0..n).filter(|&i| valid[i]).collect();
    order.sort_by(|&a, &b| t.get(a).cmp(t.get(b)));
    let mut rank = vec![0u64; n];
    let mut r = 0u64;
    for (p, &i) in order.iter().enumerate() {
        if p > 0 && t.get(order[p - 1]) != t.get(i) {
            r = p as u64;
        }
        rank[i] = r;
    }
    rank
}

// ── joins on several keys ───────────────────────────────────────────

/// A row's key across one or more key columns: the shared codes of its
/// cells, up to four packed, more in a vector.
#[derive(Hash, PartialEq, Eq, Clone)]
enum Key {
    Few([u32; 4]),
    Many(Vec<u32>),
}

/// The key of a row, or none where a key cell is missing.
fn key_of(codes: &[Vec<u32>], row: usize) -> Option<Key> {
    if codes.iter().any(|c| c[row] == u32::MAX) {
        return None;
    }
    if codes.len() <= 4 {
        let mut k = [0u32; 4];
        for (i, c) in codes.iter().enumerate() {
            k[i] = c[row];
        }
        Some(Key::Few(k))
    } else {
        Some(Key::Many(codes.iter().map(|c| c[row]).collect()))
    }
}

/// Both frames' cells of one key, coded alike: equal cells share a code,
/// a missing cell `u32::MAX`; cells of two kinds compare as they show.
fn shared_codes<'a>(l: &'a Column, lv: &Validity, r: &'a Column, rv: &Validity) -> (Vec<u32>, Vec<u32>) {
    fn both<K: std::hash::Hash + Eq>(lk: impl Iterator<Item = K>, lv: &Validity, rk: impl Iterator<Item = K>, rv: &Validity) -> (Vec<u32>, Vec<u32>) {
        let mut seen: HashMap<K, u32> = HashMap::new();
        let mut code = |k: K, present: bool| -> u32 {
            if !present {
                return u32::MAX;
            }
            let next = seen.len() as u32;
            *seen.entry(k).or_insert(next)
        };
        let left: Vec<u32> = lk.enumerate().map(|(i, k)| code(k, lv[i])).collect();
        let right: Vec<u32> = rk.enumerate().map(|(i, k)| code(k, rv[i])).collect();
        (left, right)
    }
    match (l, r) {
        (Column::Int(a), Column::Int(b)) => both(a.iter().copied(), lv, b.iter().copied(), rv),
        (Column::DateTime(a, _), Column::DateTime(b, _)) => both(a.iter().copied(), lv, b.iter().copied(), rv),
        (Column::Float(a), Column::Float(b)) => both(a.iter().map(|x| x.to_bits()), lv, b.iter().map(|x| x.to_bits()), rv),
        // (equal vectors share a code, by their numbers' bits)
        (Column::Vectors(wa, a), Column::Vectors(wb, b)) => both(
            (0..l.len()).map(|i| a[i * wa..(i + 1) * wa].iter().map(|x| (*x + 0.0).to_bits()).collect::<Vec<u64>>()),
            lv,
            (0..r.len()).map(|i| b[i * wb..(i + 1) * wb].iter().map(|x| (*x + 0.0).to_bits()).collect::<Vec<u64>>()),
            rv,
        ),
        // a number is a number: an int side and a float side match where
        // their values are equal (coded as text, they had not)
        (Column::Int(a), Column::Float(b)) => both(a.iter().map(|&x| (x as f64).to_bits()), lv, b.iter().map(|x| (*x + 0.0).to_bits()), rv),
        (Column::Float(a), Column::Int(b)) => both(a.iter().map(|x| (*x + 0.0).to_bits()), lv, b.iter().map(|&x| (x as f64).to_bits()), rv),
        (Column::Bool(a), Column::Bool(b)) => both(a.iter().copied(), lv, b.iter().copied(), rv),
        (Column::Str(a), Column::Str(b)) => {
            // a side held by a dictionary is coded once per distinct value
            fn code<'a>(seen: &mut HashMap<&'a str, u32>, k: &'a str) -> u32 {
                let next = seen.len() as u32;
                *seen.entry(k).or_insert(next)
            }
            fn side<'a>(t: &'a Text, valid: &Validity, seen: &mut HashMap<&'a str, u32>) -> Vec<u32> {
                match t.dictionary() {
                    Some((codes, values)) => {
                        let by_value: Vec<u32> = values.iter().map(|v| code(seen, v)).collect();
                        codes.iter().enumerate().map(|(i, &c)| if valid[i] { by_value[c as usize] } else { u32::MAX }).collect()
                    }
                    None => t.iter().enumerate().map(|(i, v)| if valid[i] { code(seen, v) } else { u32::MAX }).collect(),
                }
            }
            let mut seen: HashMap<&str, u32> = HashMap::new();
            let left = side(a, lv, &mut seen);
            let right = side(b, rv, &mut seen);
            (left, right)
        }
        (a, b) => {
            let shown = |c: &Column, v: &Validity| -> Vec<String> { (0..c.len()).map(|i| if v[i] { c.fmt_cell(i, true) } else { String::new() }).collect() };
            let (la, rb) = (shown(a, lv), shown(b, rv));
            both(la.iter(), lv, rb.iter(), rv)
        }
    }
}

impl CrowFrame {
    /// `join[a, b, keys, how]`: the rows of both whose keys match (`inner`),
    /// with every row of the left (`left`), of the right (`right`), of
    /// either (`outer`); the left's rows that have a match (`semi`) or none
    /// (`anti`); or, as of, each left row with the right row whose last key
    /// is the latest at or before the left's, the other keys matching
    /// exactly (`asof`) — a moment's column or numbers. The result has the
    /// left's columns, then the right's but its keys, one the left has too
    /// named `_right`; a semi or anti join has the left's alone.
    pub fn join_keys(&self, other: &CrowFrame, keys: &[&str], how: &str) -> Result<CrowFrame, String> {
        if keys.is_empty() {
            return Err("join: a key is needed — join[a, b, 'id'] or join[a, b, ['sym', 'day']]".to_string());
        }
        if let Some(twice) = keys.iter().enumerate().find(|(i, k)| keys[..*i].contains(k)).map(|(_, k)| k) {
            return Err(format!("join: key '{}' is named twice", twice));
        }
        if !matches!(how, "inner" | "left" | "right" | "outer" | "semi" | "anti" | "asof") {
            return Err(format!("join: no join '{}' — write 'inner', 'left', 'right', 'outer', 'semi', 'anti' or 'asof'", how));
        }
        let li: Vec<usize> = keys.iter().map(|k| self.col_index(k).ok_or_else(|| format!("join: column '{}' not found in left frame", k))).collect::<Result<_, _>>()?;
        let ri: Vec<usize> = keys.iter().map(|k| other.col_index(k).ok_or_else(|| format!("join: column '{}' not found in right frame", k))).collect::<Result<_, _>>()?;
        let asof = how == "asof";
        let exact = if asof { keys.len() - 1 } else { keys.len() };
        // one key of whole numbers (an id, an instant) on both sides: the
        // right side's rows by value, the left probed by value, no codes
        // built and no hash of the left side's rows beyond the probe —
        // a day of 871,527 rows joined to a 40-row universe had spent
        // most of its time coding both sides first
        // (a number is a number: an int and a float of one value match, as
        // two values do, so both sides are read as the bits of a float —
        // exact for every int below 2^53, and a NaN matches nothing)
        fn whole(c: &Column) -> Option<Vec<u64>> {
            match c {
                Column::Int(v) => Some(v.iter().map(|&x| (x as f64).to_bits()).collect()),
                Column::DateTime(ms, _) => Some(ms.iter().map(|&x| (x as f64).to_bits()).collect()),
                Column::Float(v) => Some(v.iter().map(|x| if x.is_nan() { u64::MAX } else { (*x + 0.0).to_bits() }).collect()),
                _ => None,
            }
        }
        let by_value = !asof && exact == 1 && matches!(self.data[li[0]].as_ref(), Column::Int(_) | Column::DateTime(..) | Column::Float(_)) && matches!(other.data[ri[0]].as_ref(), Column::Int(_) | Column::DateTime(..) | Column::Float(_));
        let coded: Vec<(Vec<u32>, Vec<u32>)> = if by_value { Vec::new() } else { (0..exact).map(|k| shared_codes(self.data[li[k]].as_ref(), self.validity[li[k]].as_ref(), other.data[ri[k]].as_ref(), other.validity[ri[k]].as_ref())).collect() };
        let lcodes: Vec<Vec<u32>> = coded.iter().map(|(l, _)| l.clone()).collect();
        let rcodes: Vec<Vec<u32>> = coded.iter().map(|(_, r)| r.clone()).collect();
        let (left_indices, right_indices): (Vec<Option<usize>>, Vec<Option<usize>>) = if asof {
            self.asof_pairs(other, li[exact], ri[exact], &lcodes, &rcodes)?
        } else {
            let mut right_map: HashMap<Key, Vec<usize>> = HashMap::new();
            let mut right_by_value: HashMap<u64, Vec<usize>> = HashMap::new();
            let (lvals, rvals) = if by_value { (whole(self.data[li[0]].as_ref()), whole(other.data[ri[0]].as_ref())) } else { (None, None) };
            let (lvalid, rvalid) = (self.validity[li[0]].as_ref(), other.validity[ri[0]].as_ref());
            if let Some(rvals) = &rvals {
                for (row, &v) in rvals.iter().enumerate() {
                    if rvalid[row] && v != u64::MAX {
                        right_by_value.entry(v).or_default().push(row);
                    }
                }
            } else {
                for row in 0..other.nrows() {
                    if let Some(k) = key_of(&rcodes, row) {
                        right_map.entry(k).or_default().push(row);
                    }
                }
            }
            let (is_left, is_right) = (matches!(how, "left" | "outer"), matches!(how, "right" | "outer"));
            let mut left_indices = Vec::new();
            let mut right_indices = Vec::new();
            let mut matched_right: Vec<bool> = vec![false; other.nrows()];
            for row in 0..self.nrows() {
                let hit = match &lvals {
                    Some(lvals) => if lvalid[row] && lvals[row] != u64::MAX { right_by_value.get(&lvals[row]) } else { None },
                    None => key_of(&lcodes, row).and_then(|k| right_map.get(&k)),
                };
                match how {
                    "semi" => {
                        if hit.is_some() {
                            left_indices.push(Some(row));
                            right_indices.push(None);
                        }
                    }
                    "anti" => {
                        if hit.is_none() {
                            left_indices.push(Some(row));
                            right_indices.push(None);
                        }
                    }
                    _ => match hit {
                        Some(rows) => {
                            for &rr in rows {
                                left_indices.push(Some(row));
                                right_indices.push(Some(rr));
                                matched_right[rr] = true;
                            }
                        }
                        None => {
                            if is_left {
                                left_indices.push(Some(row));
                                right_indices.push(None);
                            }
                        }
                    },
                }
            }
            if is_right {
                for (row, hit) in matched_right.iter().enumerate() {
                    let keyed = if rvals.is_some() { rvalid[row] } else { key_of(&rcodes, row).is_some() };
                    if !hit && keyed {
                        left_indices.push(None);
                        right_indices.push(Some(row));
                    }
                }
            }
            (left_indices, right_indices)
        };
        let right_columns = !matches!(how, "semi" | "anti");
        Ok(self.assemble_join(other, &left_indices, &right_indices, &li, &ri, right_columns))
    }

    /// The pairs of an as-of join: for each left row, the right row of the
    /// same exact keys whose clock is the latest at or before the left's.
    fn asof_pairs(&self, other: &CrowFrame, lc: usize, rc: usize, lcodes: &[Vec<u32>], rcodes: &[Vec<u32>]) -> Result<(Vec<Option<usize>>, Vec<Option<usize>>), String> {
        let clock = |frame: &CrowFrame, ci: usize, side: &str| -> Result<Vec<f64>, String> {
            Ok(match frame.data[ci].as_ref() {
                Column::DateTime(ms, _) => ms.iter().map(|m| *m as f64).collect(),
                Column::Int(v) => v.iter().map(|x| *x as f64).collect(),
                Column::Float(v) => v.clone(),
                other => return Err(format!("join: an as-of join matches its last key by time, and the {} frame's '{}' holds {}", side, frame.columns[ci], other.dtype_name())),
            })
        };
        let lt = clock(self, lc, "left")?;
        let rt = clock(other, rc, "right")?;
        let same_kind = matches!((self.data[lc].as_ref(), other.data[rc].as_ref()), (Column::DateTime(..), Column::DateTime(..)) | (Column::Int(_) | Column::Float(_), Column::Int(_) | Column::Float(_)));
        if !same_kind {
            return Err(format!("join: the as-of key '{}' holds {} on the left and {} on the right", self.columns[lc], self.data[lc].dtype_name(), other.data[rc].dtype_name()));
        }
        let (lv, rv) = (self.validity[lc].as_ref(), other.validity[rc].as_ref());
        let mut groups: HashMap<Key, Vec<(f64, usize)>> = HashMap::new();
        for row in 0..other.nrows() {
            if !rv[row] || rt[row].is_nan() {
                continue;
            }
            if let Some(k) = key_of(rcodes, row).or_else(|| (rcodes.is_empty()).then(|| Key::Few([0; 4]))) {
                groups.entry(k).or_default().push((rt[row], row));
            }
        }
        for rows in groups.values_mut() {
            rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
        }
        let mut left_indices = Vec::with_capacity(self.nrows());
        let mut right_indices = Vec::with_capacity(self.nrows());
        for row in 0..self.nrows() {
            left_indices.push(Some(row));
            let key = key_of(lcodes, row).or_else(|| (lcodes.is_empty()).then(|| Key::Few([0; 4])));
            let hit = match key {
                Some(k) if lv[row] && !lt[row].is_nan() => groups.get(&k).and_then(|rows| {
                    let at = rows.partition_point(|(t, _)| *t <= lt[row]);
                    at.checked_sub(1).map(|i| rows[i].1)
                }),
                _ => None,
            };
            right_indices.push(hit);
        }
        Ok((left_indices, right_indices))
    }

    /// The joined frame from the pairs of rows: the left's columns, then
    /// the right's but its key columns (`_right` on a duplicate); the
    /// left's key columns hold each row's key from whichever side has it.
    fn assemble_join(&self, other: &CrowFrame, left_indices: &[Option<usize>], right_indices: &[Option<usize>], lkeys: &[usize], rkeys: &[usize], right_columns: bool) -> CrowFrame {
        let mut result_cols = Vec::new();
        let mut specs: Vec<(bool, usize)> = Vec::new();
        for (ci, name) in self.columns.iter().enumerate() {
            result_cols.push(name.clone());
            specs.push((true, ci));
        }
        if right_columns {
            for (ci, name) in other.columns.iter().enumerate() {
                if rkeys.contains(&ci) {
                    continue;
                }
                result_cols.push(if self.col_index(name).is_some() { format!("{}_right", name) } else { name.clone() });
                specs.push((false, ci));
            }
        }
        let kept = left_indices.len() == self.nrows() && left_indices.iter().enumerate().all(|(i, l)| *l == Some(i));
        let left_holes = left_indices.iter().any(Option::is_none);
        let gathered: Vec<(Arc<Column>, Arc<Validity>)> = crate::cores::map_items(&specs, left_indices.len() * specs.len(), |&(is_left, ci)| {
                if is_left && kept {
                    return (Arc::clone(&self.data[ci]), Arc::clone(&self.validity[ci]));
                }
                let (col, valid) = if is_left && left_holes && lkeys.contains(&ci) {
                    let k = lkeys.iter().position(|&l| l == ci).unwrap_or(0);
                    let rk = rkeys[k];
                    let (col, valid) = merged_keys(self.data[ci].as_ref(), self.validity[ci].as_ref(), other.data[rk].as_ref(), other.validity[rk].as_ref(), left_indices, right_indices);
                    (col, Validity::from(valid))
                } else if is_left {
                    gather_column(self.data[ci].as_ref(), self.validity[ci].as_ref(), left_indices)
                } else {
                    gather_column(other.data[ci].as_ref(), other.validity[ci].as_ref(), right_indices)
                };
                (Arc::new(col), Arc::new(valid))
            });
        let (data, validity): (Vec<_>, Vec<_>) = gathered.into_iter().unzip();
        CrowFrame { columns: result_cols, data, validity }
    }

    /// `unique[frame, ['a', 'b']]`: the first row of each distinct
    /// combination of the columns' cells, in the order first seen, a
    /// missing cell a value of its own.
    pub fn unique_rows(&self, keys: &[&str]) -> Result<CrowFrame, String> {
        if keys.is_empty() {
            return Err("unique: a column is needed — unique[df, 'col'] or unique[df, ['a', 'b']]".to_string());
        }
        let idx: Vec<usize> = keys.iter().map(|k| self.col_index(k).ok_or_else(|| format!("unique: column '{}' not found", k))).collect::<Result<_, _>>()?;
        let coded: Vec<Vec<u32>> = idx.iter().map(|&i| codes(&self.data[i], &self.validity[i])).collect();
        let mut seen: HashMap<Key, ()> = HashMap::new();
        let mut first_rows = Vec::new();
        for row in 0..self.nrows() {
            let key = if coded.len() <= 4 {
                let mut k = [0u32; 4];
                for (i, c) in coded.iter().enumerate() {
                    k[i] = c[row];
                }
                Key::Few(k)
            } else {
                Key::Many(coded.iter().map(|c| c[row]).collect())
            };
            if seen.insert(key, ()).is_none() {
                first_rows.push(row);
            }
        }
        Ok(self.reorder_rows(&first_rows))
    }
}

// ── buckets of moments ──────────────────────────────────────────────

/// The start of the bucket a moment falls in: `span` seconds wide, the
/// buckets aligned to midnight of the moment's own offset (a day bucket
/// starts at the local midnight; with no offset, at the epoch's seconds).
pub(crate) fn bucket_ms(ms: i64, offset: i16, span_ms: i64) -> i64 {
    let shift = offset as i64 * 60_000;
    let local = ms + shift;
    local.div_euclid(span_ms) * span_ms - shift
}

/// A span in seconds as milliseconds, or the reason it is no span.
pub(crate) fn span_ms(what: &str, span: f64) -> Result<i64, String> {
    if !(span > 0.0) || !span.is_finite() {
        return Err(format!("{}: a span is a positive number of seconds (got {})", what, span));
    }
    Ok(((span * 1000.0).round() as i64).max(1))
}

/// `bucket[column, seconds]`: each moment's bucket start, a column of
/// moments; a missing cell stays missing.
pub fn bucket_column(c: &CrowColumn, span: f64) -> Result<CrowColumn, String> {
    let span_ms = span_ms("bucket", span)?;
    match c.data.as_ref() {
        Column::DateTime(ms, off) => {
            let out: Vec<i64> = ms.iter().zip(off).map(|(&m, &o)| bucket_ms(m, o, span_ms)).collect();
            Ok(CrowColumn { data: Arc::new(Column::DateTime(out, off.clone())), valid: Arc::clone(&c.valid) })
        }
        other => Err(format!("bucket: a column of {} holds no moments — moment[…] reads text or seconds as moments", other.dtype_name())),
    }
}

// ── text over a column ──────────────────────────────────────────────

fn text_of<'a>(what: &str, c: &'a CrowColumn) -> Result<&'a Text, String> {
    match c.data.as_ref() {
        Column::Str(t) => Ok(t),
        other => Err(format!("{}: a column of {} is not text — str[column] shows its cells as text", what, other.dtype_name())),
    }
}

/// Each present cell mapped; a missing cell stays missing. A column held
/// as a dictionary has its values mapped once, the cells keeping their
/// codes (two values one text now maps to share a code again).
fn mapped(what: &str, c: &CrowColumn, f: impl Fn(&str) -> String) -> Result<CrowColumn, String> {
    let t = text_of(what, c)?;
    let n = t.len();
    if let Some((codes, values)) = t.dictionary() {
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut distinct = TextBuilder::with_capacity(values.len());
        let remap: Vec<u32> = values
            .iter()
            .map(|v| {
                let m = f(v);
                let next = seen.len() as u32;
                *seen.entry(m.clone()).or_insert_with(|| {
                    distinct.push(&m);
                    next
                })
            })
            .collect();
        let data = Text::Dict { codes: codes.iter().map(|&code| remap[code as usize]).collect(), values: Box::new(distinct.finish().plain()) };
        return Ok(CrowColumn { data: Arc::new(Column::Str(data)), valid: Arc::clone(&c.valid) });
    }
    let mut b = TextBuilder::with_capacity(n);
    for i in 0..n {
        if c.valid[i] {
            b.push(&f(t.get(i)));
        } else {
            b.push("");
        }
    }
    Ok(CrowColumn { data: Arc::new(Column::Str(b.finish())), valid: Arc::clone(&c.valid) })
}

/// Each present cell tested; a missing cell is false (a mask). A
/// dictionary's values are tested once.
fn tested(what: &str, c: &CrowColumn, f: impl Fn(&str) -> bool) -> Result<Vec<bool>, String> {
    let t = text_of(what, c)?;
    if let Some((codes, values)) = t.dictionary() {
        let flags: Vec<bool> = values.iter().map(f).collect();
        return Ok(codes.iter().enumerate().map(|(i, &code)| c.valid[i] && flags[code as usize]).collect());
    }
    Ok((0..t.len()).map(|i| c.valid[i] && f(t.get(i))).collect())
}

pub fn lower_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    mapped("lower", c, crate::text::lower)
}

pub fn upper_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    mapped("upper", c, crate::text::upper)
}

pub fn trim_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    mapped("trim", c, crate::text::trim)
}

pub fn replace_column(c: &CrowColumn, old: &str, new: &str) -> Result<CrowColumn, String> {
    mapped("replace", c, |s| crate::text::replace(s, old, new))
}

/// `sub[column, pattern, replacement]`: the pattern read once; one that is
/// no pattern is the reason (a cell cannot carry it).
pub fn sub_column(c: &CrowColumn, pattern: &str, replacement: &str) -> Result<CrowColumn, String> {
    let re = crate::text::compile(pattern)?;
    mapped("sub", c, |cell| re.replace_all(cell, replacement).to_string())
}

pub fn contains_column(c: &CrowColumn, needle: &str) -> Result<Vec<bool>, String> {
    tested("contains", c, |s| s.contains(needle))
}

pub fn starts_column(c: &CrowColumn, prefix: &str) -> Result<Vec<bool>, String> {
    tested("starts", c, |s| s.starts_with(prefix))
}

pub fn ends_column(c: &CrowColumn, suffix: &str) -> Result<Vec<bool>, String> {
    tested("ends", c, |s| s.ends_with(suffix))
}

/// `column.hour` and the other fields of a moment, over a column of them:
/// a column of ints, a missing cell staying missing.
pub fn moment_field(c: &CrowColumn, field: &str) -> Result<CrowColumn, String> {
    match c.data.as_ref() {
        Column::DateTime(ms, off) => {
            let out: Vec<i64> = ms
                .iter()
                .zip(off)
                .map(|(&m, &o)| Moment { ms: m, offset: o }.field(field).ok_or_else(|| format!("a moment has no field '{}' — its fields are year, month, day, hour, minute, second, weekday and offset", field)))
                .collect::<Result<_, _>>()?;
            Ok(CrowColumn { data: Arc::new(Column::Int(out)), valid: Arc::clone(&c.valid) })
        }
        other => Err(format!("a column of {} has no field '{}' — a moment's fields are read of a column of moments", other.dtype_name(), field)),
    }
}

// ── unpivot ─────────────────────────────────────────────────────────

impl CrowFrame {
    /// `unpivot[frame, keys, cols]`: the wide frame made long — a row per
    /// key row and column, the key columns repeated, `variable` the
    /// column's name and `value` its cell, the columns' cells one kind
    /// (ints with floats are floats).
    pub fn unpivot(&self, keys: &[&str], cols: &[&str]) -> Result<CrowFrame, String> {
        if cols.is_empty() {
            return Err("unpivot: name the columns to unpivot — unpivot[df, 'id', ['q1', 'q2']]".to_string());
        }
        let ki: Vec<usize> = keys.iter().map(|k| self.col_index(k).ok_or_else(|| format!("unpivot: column '{}' not found", k))).collect::<Result<_, _>>()?;
        let ci: Vec<usize> = cols.iter().map(|c| self.col_index(c).ok_or_else(|| format!("unpivot: column '{}' not found", c))).collect::<Result<_, _>>()?;
        if let Some(both) = keys.iter().find(|k| cols.contains(k)) {
            return Err(format!("unpivot: '{}' is a key and a column to unpivot", both));
        }
        let kinds: Vec<&str> = ci.iter().map(|&i| self.data[i].dtype_name()).collect();
        if let Some(v) = kinds.iter().position(|k| *k == "vector") {
            return Err(format!("unpivot: column '{}' holds vectors, and the values are cells", cols[v]));
        }
        let kind = if kinds.iter().all(|k| *k == "int") {
            "int"
        } else if kinds.iter().all(|k| *k == "int" || *k == "float") {
            "float"
        } else if kinds.iter().all(|k| *k == kinds[0]) {
            kinds[0]
        } else {
            let other = kinds.iter().position(|k| *k != kinds[0]).unwrap_or(0);
            return Err(format!("unpivot: '{}' holds {} and '{}' holds {} — the values are one kind", cols[0], kinds[0], cols[other], kinds[other]));
        };
        let n = self.nrows();
        let repeated: Vec<usize> = (0..cols.len()).flat_map(|_| 0..n).collect();
        let mut columns: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        let mut data: Vec<Arc<Column>> = Vec::new();
        let mut validity: Vec<Arc<Validity>> = Vec::new();
        for &k in &ki {
            data.push(Arc::new(take_rows(&self.data[k], &repeated)));
            validity.push(Arc::new(self.validity[k].gather(&repeated)));
        }
        columns.push("variable".to_string());
        let mut names = TextBuilder::with_capacity(repeated.len());
        for c in cols {
            for _ in 0..n {
                names.push(c);
            }
        }
        data.push(Arc::new(Column::Str(names.finish())));
        validity.push(Arc::new(Validity::all(repeated.len())));
        columns.push("value".to_string());
        let value = match kind {
            "int" => Column::Int(ci.iter().flat_map(|&i| match self.data[i].as_ref() { Column::Int(v) => v.clone(), _ => vec![] }).collect()),
            "float" => Column::Float(
                ci.iter()
                    .flat_map(|&i| match self.data[i].as_ref() {
                        Column::Int(v) => v.iter().map(|x| *x as f64).collect(),
                        Column::Float(v) => v.clone(),
                        _ => vec![],
                    })
                    .collect(),
            ),
            "bool" => Column::Bool(ci.iter().flat_map(|&i| match self.data[i].as_ref() { Column::Bool(v) => v.clone(), _ => vec![] }).collect()),
            "datetime" => Column::DateTime(
                ci.iter().flat_map(|&i| match self.data[i].as_ref() { Column::DateTime(ms, _) => ms.clone(), _ => vec![] }).collect(),
                ci.iter().flat_map(|&i| match self.data[i].as_ref() { Column::DateTime(_, off) => off.clone(), _ => vec![] }).collect(),
            ),
            _ => {
                let parts: Vec<Text> = ci.iter().map(|&i| match self.data[i].as_ref() { Column::Str(t) => t.clone(), _ => Text::new() }).collect();
                Column::Str(Text::joined(&parts))
            }
        };
        data.push(Arc::new(value));
        let mut present = Validity::all(0);
        for &i in &ci {
            present = present.concat(&self.validity[i]);
        }
        validity.push(Arc::new(present));
        Ok(CrowFrame { columns, data, validity })
    }

    /// `fill[frame, col, how = 'forward']` or `'backward'`: each missing
    /// cell given the nearest present cell before it, or after it; one with
    /// none stays missing.
    pub fn fill_direction(&self, name: &str, how: &str) -> Result<CrowFrame, String> {
        let idx = self.col_index(name).ok_or_else(|| format!("fill: column '{}' not found", name))?;
        let valid = self.validity[idx].as_ref();
        let n = self.nrows();
        let mut picks: Vec<Option<usize>> = vec![None; n];
        match how {
            "forward" => {
                let mut last = None;
                for i in 0..n {
                    if valid[i] {
                        last = Some(i);
                    }
                    picks[i] = last;
                }
            }
            "backward" => {
                let mut next = None;
                for i in (0..n).rev() {
                    if valid[i] {
                        next = Some(i);
                    }
                    picks[i] = next;
                }
            }
            other => return Err(format!("fill: no direction '{}' — how = 'forward' or 'backward'", other)),
        }
        let (col, present) = gather_column(self.data[idx].as_ref(), valid, &picks);
        let mut data = self.data.clone();
        let mut validity = self.validity.clone();
        data[idx] = Arc::new(col);
        validity[idx] = Arc::new(present);
        Ok(CrowFrame { columns: self.columns.clone(), data, validity })
    }

    /// `complete[frame]`, `complete[frame, cols]`: the rows whose every
    /// cell, or every cell of the named columns, is present.
    pub fn complete_rows(&self, cols: &[&str]) -> Result<CrowFrame, String> {
        let idx: Vec<usize> = if cols.is_empty() {
            (0..self.ncols()).collect()
        } else {
            cols.iter().map(|c| self.col_index(c).ok_or_else(|| format!("complete: column '{}' not found", c))).collect::<Result<_, _>>()?
        };
        let n = self.nrows();
        let mask: Vec<bool> = (0..n).map(|row| idx.iter().all(|&i| self.validity[i][row])).collect();
        self.pick_rows(&mask)
    }

    /// `sample[frame, n]`: `n` of the rows drawn without replacement by the
    /// seeded generator, kept in the frame's order.
    pub fn sample_rows(&self, n: i64) -> Result<CrowFrame, String> {
        let n = crate::stops::size("sample", n)?;
        let rows = self.nrows();
        if n > rows {
            return Err(format!("sample: {} rows asked of a frame with {}", n, rows));
        }
        let mut indices: Vec<usize> = Vec::with_capacity(n);
        crate::rng::with_rng(|r| {
            indices = rand::seq::index::sample(r, rows, n).into_vec();
        });
        indices.sort_unstable();
        Ok(self.reorder_rows(&indices))
    }
}

// ── a conditional column, and casts ─────────────────────────────────

/// A cell as a column of `n` rows.
fn repeated(cell: &Cell, n: usize) -> Result<Column, String> {
    Ok(match cell {
        Cell::Int(v) => Column::Int(vec![*v; n]),
        Cell::Float(x) => Column::Float(vec![*x; n]),
        Cell::Bool(b) => Column::Bool(vec![*b; n]),
        Cell::Text(s) => Column::Str(Text::repeated(s, n)),
        Cell::Moment(m) => Column::DateTime(vec![m.ms; n], vec![m.offset; n]),
        other => Err(format!("where: a column cannot hold {}", other.kind()))?,
    })
}

/// `where[mask, a, b]`: a column that takes `a`'s cell where the mask is
/// true and `b`'s elsewhere — each a column, a list or one value for every
/// row, both of one kind (ints beside floats are floats).
pub fn choose(mask: &[bool], a: Cells, b: Cells) -> Result<CrowColumn, String> {
    let n = mask.len();
    let sided = |side: Cells, name: &str| -> Result<(Column, Validity), String> {
        match side {
            Cells::Column(col, valid) => {
                if col.len() != n {
                    return Err(format!("where: {} has {} values but the mask has {} rows", name, col.len(), n));
                }
                Ok((col, valid))
            }
            Cells::One(cell) => Ok((repeated(&cell, n)?, Validity::all(n))),
            Cells::Refused(why) => Err(format!("where: {}: {}", name, why)),
        }
    };
    let (a, av) = sided(a, "the first value")?;
    let (b, bv) = sided(b, "the second value")?;
    let floats = |c: &Column| -> Vec<f64> {
        match c {
            Column::Int(v) => v.iter().map(|x| *x as f64).collect(),
            Column::Float(v) => v.clone(),
            _ => vec![],
        }
    };
    let pick = |x: &Column, y: &Column| -> Result<Column, String> {
        Ok(match (x, y) {
            (Column::Int(p), Column::Int(q)) => Column::Int((0..n).map(|i| if mask[i] { p[i] } else { q[i] }).collect()),
            (Column::Int(_) | Column::Float(_), Column::Int(_) | Column::Float(_)) => {
                let (p, q) = (floats(x), floats(y));
                Column::Float((0..n).map(|i| if mask[i] { p[i] } else { q[i] }).collect())
            }
            (Column::Bool(p), Column::Bool(q)) => Column::Bool((0..n).map(|i| if mask[i] { p[i] } else { q[i] }).collect()),
            (Column::DateTime(pm, po), Column::DateTime(qm, qo)) => Column::DateTime((0..n).map(|i| if mask[i] { pm[i] } else { qm[i] }).collect(), (0..n).map(|i| if mask[i] { po[i] } else { qo[i] }).collect()),
            (Column::Str(p), Column::Str(q)) => {
                let mut t = TextBuilder::with_capacity(n);
                for i in 0..n {
                    t.push(if mask[i] { p.get(i) } else { q.get(i) });
                }
                Column::Str(t.finish())
            }
            (Column::Vectors(wp, p), Column::Vectors(wq, q)) => {
                if wp != wq && n > 0 {
                    return Err(format!("where: one value holds vectors of width {} and the other of width {}", wp, wq));
                }
                Column::Vectors(*wp, (0..n).flat_map(|i| if mask[i] { p[i * wp..(i + 1) * wp].iter().copied() } else { q[i * wq..(i + 1) * wq].iter().copied() }).collect())
            }
            (x, y) => return Err(format!("where: one value holds {} and the other {} — both are one kind", x.dtype_name(), y.dtype_name())),
        })
    };
    let data = pick(&a, &b)?;
    let valid: Validity = (0..n).map(|i| if mask[i] { av[i] } else { bv[i] }).collect();
    Ok(CrowColumn { data: Arc::new(data), valid: Arc::new(valid) })
}

/// `int[column]`, `float[column]`, `str[column]`, `moment[column]`: every
/// cell converted as the one-value form converts it; a cell that cannot be
/// is missing, the column's `.ok` saying which.
pub fn cast_column(c: &CrowColumn, to: &str) -> Result<CrowColumn, String> {
    let n = c.len();
    let valid = c.valid.as_ref();
    let col = c.data.as_ref();
    let (data, present): (Column, Validity) = match to {
        "int" => {
            let mut out = vec![0i64; n];
            let mut ok = Vec::with_capacity(n);
            for i in 0..n {
                let v: Option<i64> = if !valid[i] {
                    None
                } else {
                    match col {
                        Column::Int(v) => Some(v[i]),
                        Column::Float(v) => v[i].is_finite().then(|| v[i].trunc() as i64),
                        Column::Bool(v) => Some(v[i] as i64),
                        Column::Str(t) => t.get(i).trim().parse::<i64>().ok(),
                        Column::DateTime(ms, _) => Some(ms[i].div_euclid(1000)),
                        Column::Vectors(..) => None,
                    }
                };
                ok.push(v.is_some());
                out[i] = v.unwrap_or(0);
            }
            (Column::Int(out), Validity::from(ok))
        }
        "float" => {
            let mut out = vec![f64::NAN; n];
            let mut ok = Vec::with_capacity(n);
            for i in 0..n {
                let v: Option<f64> = if !valid[i] {
                    None
                } else {
                    match col {
                        Column::Int(v) => Some(v[i] as f64),
                        Column::Float(v) => Some(v[i]),
                        Column::Bool(v) => Some(v[i] as u8 as f64),
                        Column::Str(t) => t.get(i).trim().parse::<f64>().ok(),
                        Column::DateTime(ms, _) => Some(ms[i] as f64 / 1000.0),
                        Column::Vectors(..) => None,
                    }
                };
                ok.push(v.is_some());
                out[i] = v.unwrap_or(f64::NAN);
            }
            (Column::Float(out), Validity::from(ok))
        }
        "str" => {
            let mut b = TextBuilder::with_capacity(n);
            for i in 0..n {
                b.push(&if valid[i] { col.fmt_cell(i, true) } else { String::new() });
            }
            (Column::Str(b.finish()), valid.clone())
        }
        "moment" => {
            if matches!(col, Column::Bool(_)) {
                return Err("moment: a column of bools holds no moments".to_string());
            }
            if matches!(col, Column::Vectors(..)) {
                return Err("moment: a column of vectors holds no moments".to_string());
            }
            let mut ms = vec![0i64; n];
            let mut off = vec![0i16; n];
            let mut ok = Vec::with_capacity(n);
            for i in 0..n {
                let v: Option<(i64, i16)> = if !valid[i] {
                    None
                } else {
                    match col {
                        Column::DateTime(m, o) => Some((m[i], o[i])),
                        // (a number past the moments one holds is no moment:
                        // missing, as a cell that cannot be converted is)
                        Column::Int(v) => v[i].checked_mul(1000).filter(|ms| ms.abs() <= crate::moments::MOST_MS).map(|ms| (ms, 0)),
                        Column::Float(v) => crate::moments::ms_within(v[i]).map(|ms| (ms, 0)),
                        Column::Str(t) => crate::frame::parse_datetime(t.get(i)),
                        Column::Bool(_) | Column::Vectors(..) => None,
                    }
                };
                ok.push(v.is_some());
                if let Some((m, o)) = v {
                    ms[i] = m;
                    off[i] = o;
                }
            }
            (Column::DateTime(ms, off), Validity::from(ok))
        }
        other => return Err(format!("a bug in Crow: no cast to '{}'", other)),
    };
    Ok(CrowColumn { data: Arc::new(data), valid: Arc::new(present) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{new, with};

    fn frame(cols: &[(&str, Column, Option<Vec<bool>>)]) -> CrowFrame {
        let mut f = new();
        for (name, col, valid) in cols {
            let cells = match valid {
                Some(v) => Cells::Column(col.clone(), Validity::from(v.clone())),
                None => Cells::Column(col.clone(), Validity::all(col.len())),
            };
            f = with(&f, name, cells).expect("fits");
        }
        f
    }

    fn column(f: &CrowFrame, name: &str) -> (Column, Vec<bool>) {
        let c = f.col_native(name).expect("the column");
        ((*c.data).clone(), c.valid.to_vec())
    }

    /// The aggregates past the first five: a median and a percentile
    /// interpolated, the population deviation, the first and last present
    /// cell in the column's kind, and a count of distinct cells.
    #[test]
    fn the_other_aggregates_answer_as_pandas_does() {
        let f = frame(&[
            ("k", Column::strs(vec!["a".into(), "a".into(), "a".into(), "b".into(), "b".into()]), None),
            ("x", Column::Float(vec![1.0, 4.0, 2.0, 10.0, 0.0]), Some(vec![true, true, true, true, false])),
            ("s", Column::strs(vec!["p".into(), "q".into(), "p".into(), "r".into(), "r".into()]), Some(vec![false, true, true, true, true])),
        ]);
        let pair = |c: &str, how: &str| [c.to_string(), how.to_string()];
        let g = crate::frames::group(&f, "k", &[&pair("x", "median"), &pair("x", "p75"), &pair("x", "std"), &pair("x", "var"), &pair("x", "first"), &pair("x", "last"), &pair("s", "distinct"), &pair("s", "first")]).expect("a group");
        assert_eq!(g.columns, vec!["k", "x_median", "x_p75", "x_std", "x_var", "x_first", "x_last", "s_distinct", "s_first"]);
        assert_eq!(column(&g, "x_median"), (Column::Float(vec![2.0, 10.0]), vec![true, true]));
        assert_eq!(column(&g, "x_p75"), (Column::Float(vec![3.0, 10.0]), vec![true, true]));
        assert_eq!(column(&g, "x_var"), (Column::Float(vec![14.0 / 9.0, 0.0]), vec![true, true]));
        assert_eq!(column(&g, "x_std"), (Column::Float(vec![(14.0f64 / 9.0).sqrt(), 0.0]), vec![true, true]));
        assert_eq!(column(&g, "x_first"), (Column::Float(vec![1.0, 10.0]), vec![true, true]));
        assert_eq!(column(&g, "x_last"), (Column::Float(vec![2.0, 10.0]), vec![true, true]));
        assert_eq!(column(&g, "s_distinct"), (Column::Int(vec![2, 1]), vec![true, true]));
        assert_eq!(column(&g, "s_first"), (Column::strs(vec!["q".into(), "r".into()]), vec![true, true]));
        assert_eq!(AggFn::from_str("p0"), None);
        assert_eq!(AggFn::from_str("p100"), None);
        assert_eq!(AggFn::from_str("p5").map(|a| a.word()), Some("p5".to_string()));
        assert!(crate::frames::group(&f, "k", &[&pair("s", "median")]).unwrap_err().contains("median of column 's', which holds text"));
    }

    /// A window by a span of seconds takes the rows within the span before
    /// each row's own clock, and a partition out of order is the reason.
    #[test]
    fn a_window_over_a_span_takes_the_rows_within_it() {
        let f = frame(&[
            ("t", Column::DateTime(vec![0, 60_000, 150_000, 200_000, 500_000], vec![0; 5]), None),
            ("x", Column::Float(vec![1.0, 2.0, 3.0, 4.0, 5.0]), None),
        ]);
        let c = |r: Result<CrowColumn, String>| r.map(|c| ((*c.data).clone(), c.valid.to_vec()));
        // (t - 120 s, t]: the row at 150 s sees 60 s and 150 s, not 0 s
        assert_eq!(c(f.window_over("x", "sum", 0, &[], 120.0, "t")), Ok((Column::Float(vec![1.0, 3.0, 5.0, 7.0, 5.0]), vec![true; 5])));
        assert_eq!(c(f.window_over("x", "count", 0, &[], 120.0, "t")), Ok((Column::Int(vec![1, 2, 2, 2, 1]), vec![true; 5])));
        assert_eq!(c(f.window_over("x", "median", 3, &[], 0.0, "")), Ok((Column::Float(vec![1.0, 1.5, 2.0, 3.0, 4.0]), vec![true; 5])));
        assert_eq!(c(f.window_over("x", "var", 2, &[], 0.0, "")), Ok((Column::Float(vec![0.0, 0.25, 0.25, 0.25, 0.25]), vec![true; 5])));
        assert_eq!(c(f.window_over("x", "last", 2, &[], 0.0, "")), Ok((Column::Float(vec![1.0, 2.0, 3.0, 4.0, 5.0]), vec![true; 5])));
        assert!(f.window_over("x", "lag", 1, &[], 120.0, "t").unwrap_err().contains("lag counts rows"));
        let shuffled = frame(&[("t", Column::DateTime(vec![60_000, 0], vec![0; 2]), None), ("x", Column::Float(vec![1.0, 2.0]), None)]);
        assert!(shuffled.window_over("x", "sum", 0, &[], 120.0, "t").unwrap_err().contains("ascending order"));
    }

    /// A sort on several keys orders by the first, ties by the next, a
    /// direction per key, missing cells last; a join on several keys matches
    /// the cells of each; distinct rows keep the first of each combination.
    #[test]
    fn keys_sort_join_and_distinguish_rows() {
        let f = frame(&[
            ("sym", Column::strs(vec!["b".into(), "a".into(), "b".into(), "a".into()]), None),
            ("day", Column::Int(vec![1, 2, 2, 1]), Some(vec![true, true, false, true])),
            ("qty", Column::Int(vec![5, 6, 7, 8]), None),
        ]);
        let sorted = f.sort_keys(&["sym", "day"], &[false, true]).expect("sorted");
        assert_eq!(column(&sorted, "qty").0, Column::Int(vec![6, 8, 5, 7]));
        let lots = frame(&[("sym", Column::strs(vec!["a".into(), "b".into()]), None), ("day", Column::Int(vec![1, 1]), None), ("lot", Column::Int(vec![10, 20]), None)]);
        let j = f.join_keys(&lots, &["sym", "day"], "left").expect("joined");
        assert_eq!(j.columns, vec!["sym", "day", "qty", "lot"]);
        assert_eq!(column(&j, "lot"), (Column::Int(vec![20, 0, 0, 10]), vec![true, false, false, true]));
        let semi = f.join_keys(&lots, &["sym", "day"], "semi").expect("semi");
        assert_eq!(column(&semi, "qty").0, Column::Int(vec![5, 8]));
        let anti = f.join_keys(&lots, &["sym", "day"], "anti").expect("anti");
        assert_eq!(column(&anti, "qty").0, Column::Int(vec![6, 7]));
        let u = f.unique_rows(&["sym"]).expect("distinct rows");
        assert_eq!(column(&u, "qty").0, Column::Int(vec![5, 6]));
        assert!(f.join_keys(&lots, &["sym", "nope"], "inner").unwrap_err().contains("'nope' not found in left frame"));
        assert!(f.join_keys(&lots, &["sym"], "sideways").unwrap_err().contains("no join 'sideways'"));
    }

    /// An as-of join hands each left row the right row at or before its
    /// clock with the same other keys, and none where there is none.
    #[test]
    fn an_asof_join_takes_the_latest_row_at_or_before() {
        let bars = frame(&[
            ("sym", Column::strs(vec!["a".into(), "a".into(), "b".into()]), None),
            ("t", Column::DateTime(vec![100_000, 200_000, 150_000], vec![0; 3]), None),
        ]);
        let signals = frame(&[
            ("sym", Column::strs(vec!["a".into(), "a".into(), "b".into()]), None),
            ("t", Column::DateTime(vec![90_000, 200_000, 160_000], vec![0; 3]), None),
            ("sig", Column::Int(vec![1, 2, 3]), None),
        ]);
        let j = bars.join_keys(&signals, &["sym", "t"], "asof").expect("joined");
        assert_eq!(j.columns, vec!["sym", "t", "sig"]);
        assert_eq!(column(&j, "sig"), (Column::Int(vec![1, 2, 0]), vec![true, true, false]));
        assert!(bars.join_keys(&signals, &["t", "sym"], "asof").unwrap_err().contains("by time"));
    }

    /// Buckets start at the local midnight of the moment's offset; text
    /// functions map or test each present cell; a fill by direction carries
    /// the nearest present cell; `where` picks by the mask; casts make the
    /// cells that cannot convert missing.
    #[test]
    fn buckets_text_fills_where_and_casts() {
        // 2026-07-14T09:30:00-04:00 is 13:30Z; a day bucket in -04:00 starts at 04:00Z
        let c = CrowColumn { data: Arc::new(Column::DateTime(vec![1_784_035_800_000, 0], vec![-240, 0])), valid: Arc::new(Validity::from(vec![true, false])) };
        let b = bucket_column(&c, 86_400.0).expect("bucketed");
        assert_eq!((*b.data).clone(), Column::DateTime(vec![1_784_001_600_000, 0], vec![-240, 0]));
        assert_eq!(b.valid.to_vec(), vec![true, false]);
        assert!(bucket_column(&c, 0.0).unwrap_err().contains("positive"));
        let t = CrowColumn { data: Arc::new(Column::strs(vec!["Ab c".into(), "x".into(), "ab".into()])), valid: Arc::new(Validity::from(vec![true, false, true])) };
        assert_eq!((*lower_column(&t).expect("text").data).clone(), Column::strs(vec!["ab c".into(), "".into(), "ab".into()]));
        assert_eq!(contains_column(&t, "b"), Ok(vec![true, false, true]));
        assert_eq!(starts_column(&t, "A"), Ok(vec![true, false, false]));
        assert_eq!((*sub_column(&t, "[a-z]", "_").expect("subbed").data).clone(), Column::strs(vec!["A_ _".into(), "".into(), "__".into()]));
        assert!(sub_column(&t, "[", "_").is_err());
        let f = frame(&[("x", Column::Int(vec![1, 0, 0, 4, 0]), Some(vec![true, false, false, true, false]))]);
        assert_eq!(column(&f.fill_direction("x", "forward").expect("filled"), "x"), (Column::Int(vec![1, 1, 1, 4, 4]), vec![true; 5]));
        assert_eq!(column(&f.fill_direction("x", "backward").expect("filled"), "x"), (Column::Int(vec![1, 4, 4, 4, 0]), vec![true, true, true, true, false]));
        assert_eq!(f.complete_rows(&[]).expect("complete").nrows(), 2);
        let w = choose(&[true, false, true], Cells::from(vec![1_i64, 2, 3]), Cells::from(0.5)).expect("chosen");
        assert_eq!((*w.data).clone(), Column::Float(vec![1.0, 0.5, 3.0]));
        assert!(choose(&[true], Cells::from(1_i64), Cells::from("x".to_string())).unwrap_err().contains("one kind"));
        let s = CrowColumn { data: Arc::new(Column::strs(vec!["12".into(), "x".into(), " 7 ".into()])), valid: Arc::new(Validity::all(3)) };
        let i = cast_column(&s, "int").expect("ints");
        assert_eq!(((*i.data).clone(), i.valid.to_vec()), (Column::Int(vec![12, 0, 7]), vec![true, false, true]));
        let m = cast_column(&CrowColumn { data: Arc::new(Column::strs(vec!["2026-07-14".into(), "nope".into()])), valid: Arc::new(Validity::all(2)) }, "moment").expect("moments");
        assert_eq!(m.valid.to_vec(), vec![true, false]);
        let u = frame(&[("id", Column::Int(vec![1, 2]), None), ("a", Column::Int(vec![3, 4]), None), ("b", Column::Float(vec![0.5, 1.5]), None)]).unpivot(&["id"], &["a", "b"]).expect("long");
        assert_eq!(u.columns, vec!["id", "variable", "value"]);
        assert_eq!(column(&u, "value").0, Column::Float(vec![3.0, 4.0, 0.5, 1.5]));
        assert_eq!(column(&u, "variable").0, Column::strs(vec!["a".into(), "a".into(), "b".into(), "b".into()]));
    }
}
