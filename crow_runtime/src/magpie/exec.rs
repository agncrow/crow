//! The executor (docs/magpie-design.md, phase 8): aggregates pushed below
//! the reader, as operators over row groups — scan (prune by statistics,
//! decode the columns named), filter (predicates over column vectors),
//! group (by key bytes), reduce (partials), combine — each row group on
//! its own core, the rows never materialised as one frame. And the as-of
//! join over the log: for each label's keys and moment, the newest row
//! at or before it. What `summary` answers is what the frame module's
//! `group` answers over `scan` of the same rows, within the last digits
//! a sum in another order moves.

use super::file::{Bound, Entry, Reader};
use super::tail::key_bytes;
use super::{parsed, parsed_bytes, Parsed, POSITION};
use crate::frame::{Column, CrowFrame};
use crate::textcol::Text;
use std::collections::HashMap;

/// A predicate on a column: the operator and the value as the column's kind.
struct Predicate {
    column: String,
    op: Op,
    value: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

fn op_of(s: &str) -> Result<Op, String> {
    Ok(match s {
        "eq" => Op::Eq,
        "ne" => Op::Ne,
        "lt" => Op::Lt,
        "le" | "lte" => Op::Le,
        "gt" => Op::Gt,
        "ge" | "gte" => Op::Ge,
        other => Err(format!("batch: no operator '{}' — write eq, ne, lt, le, gt or ge", other))?,
    })
}

fn cmp_parsed(a: &Parsed, b: &Parsed) -> std::cmp::Ordering {
    use std::cmp::Ordering::Equal;
    match (a, b) {
        (Parsed::Int(x), Parsed::Int(y)) => x.cmp(y),
        (Parsed::Float(x), Parsed::Float(y)) => x.partial_cmp(y).unwrap_or(Equal),
        (Parsed::Bool(x), Parsed::Bool(y)) => x.cmp(y),
        (Parsed::Text(x), Parsed::Text(y)) => x.cmp(y),
        _ => Equal,
    }
}

fn bound_parsed(b: &Bound) -> Parsed {
    match b {
        Bound::Int(v) => Parsed::Int(*v),
        Bound::Float(v) => Parsed::Float(*v),
        Bound::Bool(v) => Parsed::Bool(*v),
        Bound::Text(v) => Parsed::Text(v.clone()),
    }
}

fn cell_parsed(col: &Column, i: usize) -> Parsed {
    match col {
        Column::Int(v) => Parsed::Int(v[i]),
        Column::DateTime(ms, _) => Parsed::Int(ms[i]),
        Column::Float(v) => Parsed::Float(v[i]),
        Column::Bool(v) => Parsed::Bool(v[i]),
        Column::Str(t) => Parsed::Text(t.get(i).to_string()),
    }
}

/// Whether a group's bounds can hold a cell the predicate admits.
fn group_may(e: &Entry, op: Op, want: &Parsed) -> bool {
    use std::cmp::Ordering::*;
    if e.rows > 0 && e.missing == e.rows {
        return false;
    }
    let (min, max) = (e.min.as_ref().map(bound_parsed), e.max.as_ref().map(bound_parsed));
    match op {
        Op::Eq => min.as_ref().is_none_or(|m| cmp_parsed(m, want) != Greater) && max.as_ref().is_none_or(|m| cmp_parsed(m, want) != Less),
        Op::Ne => true,
        Op::Lt => min.as_ref().is_none_or(|m| cmp_parsed(m, want) == Less),
        Op::Le => min.as_ref().is_none_or(|m| cmp_parsed(m, want) != Greater),
        Op::Gt => max.as_ref().is_none_or(|m| cmp_parsed(m, want) == Greater),
        Op::Ge => max.as_ref().is_none_or(|m| cmp_parsed(m, want) != Less),
    }
}

fn cell_passes(col: &Column, i: usize, op: Op, want: &Parsed) -> bool {
    use std::cmp::Ordering::*;
    let o = cmp_parsed(&cell_parsed(col, i), want);
    match op {
        Op::Eq => o == Equal,
        Op::Ne => o != Equal,
        Op::Lt => o == Less,
        Op::Le => o != Greater,
        Op::Gt => o == Greater,
        Op::Ge => o != Less,
    }
}

/// One cell held by a partial (a min, a max, a first, a last).
#[derive(Clone, Debug)]
enum Cell {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Moment(i64, i16),
}

fn cell_of(col: &Column, i: usize) -> Cell {
    match col {
        Column::Int(v) => Cell::Int(v[i]),
        Column::Float(v) => Cell::Float(v[i]),
        Column::Str(t) => Cell::Text(t.get(i).to_string()),
        Column::Bool(v) => Cell::Bool(v[i]),
        Column::DateTime(ms, off) => Cell::Moment(ms[i], off[i]),
    }
}

fn cell_cmp(a: &Cell, b: &Cell) -> std::cmp::Ordering {
    use std::cmp::Ordering::Equal;
    match (a, b) {
        (Cell::Int(x), Cell::Int(y)) => x.cmp(y),
        (Cell::Float(x), Cell::Float(y)) => x.partial_cmp(y).unwrap_or(Equal),
        (Cell::Text(x), Cell::Text(y)) => x.cmp(y),
        (Cell::Bool(x), Cell::Bool(y)) => x.cmp(y),
        (Cell::Moment(x, _), Cell::Moment(y, _)) => x.cmp(y),
        _ => Equal,
    }
}

/// How a column is reduced.
#[derive(Clone, Copy, PartialEq)]
enum How {
    Count,
    Sum,
    Min,
    Max,
    Mean,
    Var,
    Std,
    First,
    Last,
}

fn how_of(s: &str) -> Result<How, String> {
    Ok(match s {
        "count" => How::Count,
        "sum" => How::Sum,
        "min" => How::Min,
        "max" => How::Max,
        "mean" => How::Mean,
        "var" => How::Var,
        "std" => How::Std,
        "first" => How::First,
        "last" => How::Last,
        other => Err(format!("batch: no aggregate '{}' — write count, sum, min, max, mean, var, std, first or last", other))?,
    })
}

/// A partial of one aggregate over one key: enough to combine.
#[derive(Clone, Debug)]
struct Partial {
    count: i64,
    int_sum: i64,
    sum: f64,
    /// Welford's running mean and squared deviations, for var and std.
    mean: f64,
    m2: f64,
    least: Option<Cell>,
    most: Option<Cell>,
    first: Option<(i64, Cell)>,
    last: Option<(i64, Cell)>,
}

impl Partial {
    fn new() -> Partial {
        Partial { count: 0, int_sum: 0, sum: 0.0, mean: 0.0, m2: 0.0, least: None, most: None, first: None, last: None }
    }

    fn add(&mut self, col: &Column, i: usize, pos: i64) {
        if let Column::Float(v) = col
            && v[i].is_nan()
        {
            return;
        }
        let cell = cell_of(col, i);
        self.count += 1;
        let x = match &cell {
            Cell::Int(v) => {
                self.int_sum = self.int_sum.wrapping_add(*v);
                *v as f64
            }
            Cell::Float(v) => *v,
            Cell::Bool(b) => {
                self.int_sum += *b as i64;
                *b as u8 as f64
            }
            _ => f64::NAN,
        };
        if !x.is_nan() {
            self.sum += x;
            let delta = x - self.mean;
            self.mean += delta / self.count as f64;
            self.m2 += delta * (x - self.mean);
        }
        if self.least.as_ref().is_none_or(|l| cell_cmp(&cell, l) == std::cmp::Ordering::Less) {
            self.least = Some(cell.clone());
        }
        if self.most.as_ref().is_none_or(|m| cell_cmp(&cell, m) == std::cmp::Ordering::Greater) {
            self.most = Some(cell.clone());
        }
        if self.first.as_ref().is_none_or(|(p, _)| pos < *p) {
            self.first = Some((pos, cell.clone()));
        }
        if self.last.as_ref().is_none_or(|(p, _)| pos > *p) {
            self.last = Some((pos, cell));
        }
    }

    /// Another partial folded in (Chan's merge for the variance).
    fn merge(&mut self, o: &Partial) {
        if o.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = o.clone();
            return;
        }
        let (na, nb) = (self.count as f64, o.count as f64);
        let delta = o.mean - self.mean;
        let n = na + nb;
        self.mean += delta * nb / n;
        self.m2 += o.m2 + delta * delta * na * nb / n;
        self.count += o.count;
        self.int_sum = self.int_sum.wrapping_add(o.int_sum);
        self.sum += o.sum;
        if let Some(l) = &o.least
            && self.least.as_ref().is_none_or(|m| cell_cmp(l, m) == std::cmp::Ordering::Less)
        {
            self.least = Some(l.clone());
        }
        if let Some(h) = &o.most
            && self.most.as_ref().is_none_or(|m| cell_cmp(h, m) == std::cmp::Ordering::Greater)
        {
            self.most = Some(h.clone());
        }
        if let Some((p, c)) = &o.first
            && self.first.as_ref().is_none_or(|(q, _)| p < q)
        {
            self.first = Some((*p, c.clone()));
        }
        if let Some((p, c)) = &o.last
            && self.last.as_ref().is_none_or(|(q, _)| p > q)
        {
            self.last = Some((*p, c.clone()));
        }
    }
}

/// What a key's row holds in the answer: its key cells and its partials.
struct Group {
    keys: Vec<Option<Cell>>,
    partials: Vec<Partial>,
}

/// `sys.batch_summary[paths, by, of, how, on, ops, equal, lo, hi, upto]`:
/// the rows of the files at or before `upto`, whose first key lies
/// within `lo` and `hi` and which every predicate admits (`on[i] ops[i]
/// equal[i]`; one column named twice is any of its values, columns are
/// all of them), grouped by the `by` columns and reduced: `of[i]` by
/// `how[i]` into a column `of_how`. A row per key in the keys' order,
/// the key columns as they were; `count` an int, `sum` an int of ints and
/// a float of floats, `mean`, `var` (the population's) and `std` floats,
/// `min`, `max`, `first` and `last` (by position) of the column's kind.
#[allow(clippy::too_many_arguments)] // the row's arguments, as the program writes them
pub fn summary(paths: &[String], by: &[String], of: &[String], how: &[String], on: &[String], ops: &[String], equal: &[String], lo: &str, hi: &str, upto: i64) -> Result<CrowFrame, String> {
    if of.len() != how.len() {
        return Err(format!("batch: a summary takes one aggregate per column: {} columns, {} aggregates", of.len(), how.len()));
    }
    if on.len() != ops.len() || on.len() != equal.len() {
        return Err("batch: the predicates are three lists of one length: on, ops and equal".to_string());
    }
    let hows: Vec<How> = how.iter().map(|h| how_of(h)).collect::<Result<_, _>>()?;
    let predicates: Vec<Predicate> = on.iter().zip(ops).zip(equal).map(|((c, o), v)| Ok(Predicate { column: c.clone(), op: op_of(o)?, value: v.clone() })).collect::<Result<_, String>>()?;
    if paths.is_empty() {
        return Ok(CrowFrame::new(Vec::new(), Vec::new(), Vec::new()));
    }
    // the groups of every file, each on its own core
    let mut units: Vec<(usize, usize)> = Vec::new();
    let mut readers = Vec::with_capacity(paths.len());
    for (fi, p) in paths.iter().enumerate() {
        let r = Reader::open(p)?;
        for g in 0..r.footer().groups.len() {
            units.push((fi, g));
        }
        readers.push(r);
    }
    let rows_total: usize = readers.iter().map(|r| r.footer().rows as usize).sum();
    let parts: Vec<Result<HashMap<Vec<u8>, Group>, String>> = crate::cores::map_items(&units, rows_total.max(1), |&(fi, g)| {
        let r = &readers[fi];
        let f = r.footer();
        let keys0 = f.keys.first().cloned();
        // prune: the positions, the first key's range, each predicate's bounds
        if upto >= 0
            && let Some(pc) = f.column(POSITION)
            && let Some(e) = r.entries(pc)?.get(g)
            && matches!(e.min, Some(Bound::Int(m)) if m > upto)
        {
            return Ok(HashMap::new());
        }
        if (!lo.is_empty() || !hi.is_empty())
            && let Some(k) = &keys0
            && let Some(kc) = f.column(k)
            && let Some(e) = r.entries(kc)?.get(g)
        {
            let kind = f.columns[kc].kind;
            if !lo.is_empty() && !group_may(e, Op::Ge, &parsed(kind, lo).map_err(|m| format!("batch: column '{}': {}", k, m))?) {
                return Ok(HashMap::new());
            }
            if !hi.is_empty() && !group_may(e, Op::Le, &parsed(kind, hi).map_err(|m| format!("batch: column '{}': {}", k, m))?) {
                return Ok(HashMap::new());
            }
        }
        // a column named twice is any of its values: the group may hold a row where any admits
        let mut by_column: HashMap<&str, Vec<&Predicate>> = HashMap::new();
        for p in &predicates {
            by_column.entry(p.column.as_str()).or_default().push(p);
        }
        for (column, ps) in &by_column {
            let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", r.path, column))?;
            let kind = f.columns[c].kind;
            let e = r.entries(c)?.get(g).cloned().ok_or_else(|| "no group".to_string())?;
            let mut any = false;
            for p in ps {
                let want = parsed(kind, &p.value).map_err(|m| format!("batch: column '{}': {}", column, m))?;
                if group_may(&e, p.op, &want) {
                    if p.op == Op::Eq
                        && let Some(bloom) = r.bloom(&e)?
                        && !bloom.has(&parsed_bytes(&want))
                    {
                        continue;
                    }
                    any = true;
                    break;
                }
            }
            if !any {
                return Ok(HashMap::new());
            }
        }
        // the columns read: the keys, the aggregated, the predicates', the first key, the position
        let mut names: Vec<String> = Vec::new();
        for n in by.iter().chain(of).chain(on) {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        if let Some(k) = &keys0
            && (!lo.is_empty() || !hi.is_empty())
            && !names.contains(k)
        {
            names.push(k.clone());
        }
        if !names.iter().any(|n| n == POSITION) {
            names.push(POSITION.to_string());
        }
        let cols = r.wanted(&names)?;
        let frame = r.group(g, &cols)?;
        let n = frame.nrows();
        let at = |name: &str| frame.col_index(name).ok_or_else(|| format!("batch: '{}' holds no column '{}'", r.path, name));
        let pos_c = at(POSITION)?;
        let Column::Int(positions) = frame.data[pos_c].as_ref() else {
            return Err(format!("batch: '{}' holds no positions", r.path));
        };
        // the mask: positions, the key range, the predicates
        let mut mask = vec![true; n];
        if upto >= 0 {
            for i in 0..n {
                mask[i] &= positions[i] <= upto;
            }
        }
        if let Some(k) = &keys0
            && (!lo.is_empty() || !hi.is_empty())
        {
            let kc = at(k)?;
            let kind = f.columns[f.column(k).unwrap_or(0)].kind;
            let low = if lo.is_empty() { None } else { Some(parsed(kind, lo).map_err(|m| format!("batch: column '{}': {}", k, m))?) };
            let high = if hi.is_empty() { None } else { Some(parsed(kind, hi).map_err(|m| format!("batch: column '{}': {}", k, m))?) };
            let col = frame.data[kc].as_ref();
            let valid = frame.validity[kc].as_ref();
            for i in 0..n {
                mask[i] &= valid[i] && low.as_ref().is_none_or(|l| cell_passes(col, i, Op::Ge, l)) && high.as_ref().is_none_or(|h| cell_passes(col, i, Op::Le, h));
            }
        }
        for (column, ps) in &by_column {
            let c = at(column)?;
            let kind = f.columns[f.column(column).unwrap_or(0)].kind;
            let wants: Vec<(Op, Parsed)> = ps.iter().map(|p| Ok((p.op, parsed(kind, &p.value).map_err(|m| format!("batch: column '{}': {}", column, m))?))).collect::<Result<_, String>>()?;
            let col = frame.data[c].as_ref();
            let valid = frame.validity[c].as_ref();
            for i in 0..n {
                if mask[i] {
                    mask[i] = valid[i] && wants.iter().any(|(op, w)| cell_passes(col, i, *op, w));
                }
            }
        }
        // group and reduce
        let key_cols: Vec<usize> = by.iter().map(|k| at(k)).collect::<Result<_, _>>()?;
        let of_cols: Vec<usize> = of.iter().map(|k| at(k)).collect::<Result<_, _>>()?;
        let mut groups: HashMap<Vec<u8>, Group> = HashMap::new();
        for i in 0..n {
            if !mask[i] {
                continue;
            }
            // a row with a missing key cell is in no group
            if key_cols.iter().any(|&c| !frame.validity[c][i]) {
                continue;
            }
            let kb = key_bytes(&frame, &key_cols, i);
            let entry = groups.entry(kb).or_insert_with(|| Group { keys: key_cols.iter().map(|&c| Some(cell_of(frame.data[c].as_ref(), i))).collect(), partials: vec![Partial::new(); of_cols.len()] });
            for (a, &c) in of_cols.iter().enumerate() {
                if frame.validity[c][i] {
                    entry.partials[a].add(frame.data[c].as_ref(), i, positions[i]);
                }
            }
        }
        Ok(groups)
    });
    // combine
    let mut all: HashMap<Vec<u8>, Group> = HashMap::new();
    for part in parts {
        for (kb, g) in part? {
            match all.get_mut(&kb) {
                Some(have) => {
                    for (a, p) in g.partials.iter().enumerate() {
                        have.partials[a].merge(p);
                    }
                }
                None => {
                    all.insert(kb, g);
                }
            }
        }
    }
    // the answer in the keys' order
    let mut rows: Vec<Group> = all.into_values().collect();
    rows.sort_by(|a, b| {
        for (x, y) in a.keys.iter().zip(&b.keys) {
            let o = match (x, y) {
                (Some(x), Some(y)) => cell_cmp(x, y),
                _ => std::cmp::Ordering::Equal,
            };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    });
    let kinds: Vec<super::encode::Kind> = {
        let r = &readers[0];
        let f = r.footer();
        of.iter().map(|c| f.column(c).map(|i| f.columns[i].kind).ok_or_else(|| format!("batch: '{}' holds no column '{}'", r.path, c))).collect::<Result<_, _>>()?
    };
    let key_kinds: Vec<super::encode::Kind> = {
        let f = readers[0].footer();
        by.iter().map(|c| f.column(c).map(|i| f.columns[i].kind).ok_or_else(|| format!("batch: '{}' holds no column '{}'", readers[0].path, c))).collect::<Result<_, _>>()?
    };
    let nrows = rows.len();
    let mut names: Vec<String> = by.to_vec();
    let mut data: Vec<Column> = Vec::new();
    let mut validity: Vec<Vec<bool>> = Vec::new();
    for (k, kind) in key_kinds.iter().enumerate() {
        let cells: Vec<Option<Cell>> = rows.iter().map(|g| g.keys[k].clone()).collect();
        let (col, valid) = column_of(&cells, *kind);
        data.push(col);
        validity.push(valid);
    }
    for (a, (name, h)) in of.iter().zip(&hows).enumerate() {
        names.push(format!("{}_{}", name, how[a]));
        let kind = kinds[a];
        match h {
            How::Count => {
                data.push(Column::Int(rows.iter().map(|g| g.partials[a].count).collect()));
                validity.push(vec![true; nrows]);
            }
            How::Sum => match kind {
                super::encode::Kind::Int | super::encode::Kind::Bool => {
                    data.push(Column::Int(rows.iter().map(|g| g.partials[a].int_sum).collect()));
                    validity.push(vec![true; nrows]);
                }
                super::encode::Kind::Float => {
                    data.push(Column::Float(rows.iter().map(|g| g.partials[a].sum).collect()));
                    validity.push(vec![true; nrows]);
                }
                _ => return Err(format!("batch: sum of column '{}', which holds {}", name, kind.word())),
            },
            How::Mean | How::Var | How::Std => {
                if !matches!(kind, super::encode::Kind::Int | super::encode::Kind::Float | super::encode::Kind::Bool) {
                    return Err(format!("batch: {} of column '{}', which holds {}", how[a], name, kind.word()));
                }
                let values: Vec<f64> = rows
                    .iter()
                    .map(|g| {
                        let p = &g.partials[a];
                        if p.count == 0 {
                            return f64::NAN;
                        }
                        match h {
                            How::Mean => p.sum / p.count as f64,
                            How::Var => p.m2 / p.count as f64,
                            _ => (p.m2 / p.count as f64).sqrt(),
                        }
                    })
                    .collect();
                validity.push(rows.iter().map(|g| g.partials[a].count > 0).collect());
                data.push(Column::Float(values));
            }
            How::Min | How::Max | How::First | How::Last => {
                let cells: Vec<Option<Cell>> = rows
                    .iter()
                    .map(|g| {
                        let p = &g.partials[a];
                        match h {
                            How::Min => p.least.clone(),
                            How::Max => p.most.clone(),
                            How::First => p.first.as_ref().map(|(_, c)| c.clone()),
                            _ => p.last.as_ref().map(|(_, c)| c.clone()),
                        }
                    })
                    .collect();
                let (col, valid) = column_of(&cells, kind);
                data.push(col);
                validity.push(valid);
            }
        }
    }
    Ok(CrowFrame::new(names, data, validity))
}

/// A column of the kind from cells, a missing one where none.
fn column_of(cells: &[Option<Cell>], kind: super::encode::Kind) -> (Column, Vec<bool>) {
    let valid: Vec<bool> = cells.iter().map(|c| c.is_some()).collect();
    let col = match kind {
        super::encode::Kind::Int => Column::Int(cells.iter().map(|c| match c { Some(Cell::Int(v)) => *v, _ => 0 }).collect()),
        super::encode::Kind::Float => Column::Float(cells.iter().map(|c| match c { Some(Cell::Float(v)) => *v, Some(Cell::Int(v)) => *v as f64, _ => 0.0 }).collect()),
        super::encode::Kind::Bool => Column::Bool(cells.iter().map(|c| matches!(c, Some(Cell::Bool(true)))).collect()),
        super::encode::Kind::Moment => {
            let ms = cells.iter().map(|c| match c { Some(Cell::Moment(m, _)) => *m, _ => 0 }).collect();
            let off = cells.iter().map(|c| match c { Some(Cell::Moment(_, o)) => *o, _ => 0 }).collect();
            Column::DateTime(ms, off)
        }
        super::encode::Kind::Text => {
            let mut t = Text::with_capacity(cells.len(), cells.len() * 8);
            for c in cells {
                t.push(match c {
                    Some(Cell::Text(s)) => s.as_str(),
                    _ => "",
                });
            }
            Column::Str(t)
        }
    };
    (col, valid)
}

/// The log's rows a label's key reaches: per key, each row's moment, its
/// position, the row itself and a spare.
type Candidates = HashMap<Vec<u8>, Vec<(i64, i64, CrowFrame, usize)>>;

/// `sys.batch_asof[paths, labels, keys, time, wanted]`: for each row of
/// `labels` — the log's keys and a moment in its time column — the
/// newest row of the files with the same keys at or before the moment
/// (by time, then by position), its `wanted` columns (every column of
/// the log not among the labels' when none is named) beside the label's;
/// missing cells where none precedes. The labels' columns come first,
/// in their order.
pub fn asof(paths: &[String], labels: &CrowFrame, keys: &[String], time: &str, wanted: &[String]) -> Result<CrowFrame, String> {
    if keys.is_empty() {
        return Err("batch: an as-of join needs the log's keys".to_string());
    }
    if time.is_empty() {
        return Err("batch: an as-of join needs the log's time column — declare it at the first batch (time = 'ts')".to_string());
    }
    let label_keys: Vec<usize> = keys.iter().map(|k| labels.col_index(k).ok_or_else(|| format!("batch: the labels lack the key column '{}'", k))).collect::<Result<_, _>>()?;
    let label_time = labels.col_index(time).ok_or_else(|| format!("batch: the labels lack the time column '{}'", time))?;
    let nl = labels.nrows();
    let label_moment = |i: usize| -> Option<i64> {
        if !labels.validity[label_time][i] {
            return None;
        }
        match labels.data[label_time].as_ref() {
            Column::Int(v) => Some(v[i]),
            Column::DateTime(ms, _) => Some(ms[i]),
            Column::Float(v) => Some(v[i] as i64),
            _ => None,
        }
    };
    // the labels by key: each key's labels, and the latest moment asked of it
    let mut by_key: HashMap<Vec<u8>, (Vec<usize>, i64)> = HashMap::new();
    for i in 0..nl {
        if label_keys.iter().any(|&c| !labels.validity[c][i]) {
            continue;
        }
        let Some(m) = label_moment(i) else { continue };
        let kb = key_bytes(labels, &label_keys, i);
        let e = by_key.entry(kb).or_insert((Vec::new(), i64::MIN));
        e.0.push(i);
        e.1 = e.1.max(m);
    }
    if paths.is_empty() || by_key.is_empty() {
        return joined(labels, &HashMap::new(), &[], wanted, keys, time, None);
    }
    // the log's columns to bring, from the first file
    let first = Reader::open(&paths[0])?;
    let footer = first.footer().clone();
    let bring: Vec<String> = if wanted.is_empty() {
        footer.columns.iter().map(|c| c.name.clone()).filter(|c| !labels.columns.contains(c) && c != POSITION).collect()
    } else {
        wanted.to_vec()
    };
    let mut needed: Vec<String> = keys.to_vec();
    needed.push(time.to_string());
    needed.push(POSITION.to_string());
    for b in &bring {
        if !needed.contains(b) {
            needed.push(b.clone());
        }
    }
    // every group whose first key's bounds overlap the labels' keys, read
    // and its rows for the labels' keys kept as candidates
    let mut candidates: Candidates = HashMap::new();
    for p in paths {
        let r = Reader::open(p)?;
        let f = r.footer();
        let cols = r.wanted(&needed)?;
        let kc = f.column(&keys[0]).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, keys[0]))?;
        let entries = r.entries(kc)?;
        for g in 0..f.groups.len() {
            let frame = r.group(g, &cols)?;
            let _ = &entries;
            let kcols: Vec<usize> = keys.iter().map(|k| frame.col_index(k).ok_or_else(|| format!("batch: no key column '{}'", k))).collect::<Result<_, _>>()?;
            let tc = frame.col_index(time).ok_or_else(|| format!("batch: '{}' holds no time column '{}'", p, time))?;
            let pc = frame.col_index(POSITION).ok_or_else(|| "batch: no positions".to_string())?;
            let Column::Int(positions) = frame.data[pc].as_ref() else { return Err("batch: no positions".to_string()) };
            for i in 0..frame.nrows() {
                if kcols.iter().any(|&c| !frame.validity[c][i]) || !frame.validity[tc][i] {
                    continue;
                }
                let kb = key_bytes(&frame, &kcols, i);
                let Some((_, latest)) = by_key.get(&kb) else { continue };
                let moment = match frame.data[tc].as_ref() {
                    Column::Int(v) => v[i],
                    Column::DateTime(ms, _) => ms[i],
                    Column::Float(v) => v[i] as i64,
                    _ => continue,
                };
                if moment > *latest {
                    continue;
                }
                let mask: Vec<bool> = (0..frame.nrows()).map(|j| j == i).collect();
                candidates.entry(kb).or_default().push((moment, positions[i], frame.pick_rows(&mask)?, 0));
            }
        }
    }
    joined(labels, &candidates, &by_key.iter().flat_map(|(k, (ls, _))| ls.iter().map(move |&l| (l, k.clone()))).collect::<Vec<_>>(), &bring, keys, time, Some(&label_moment))
}

/// The labels with the newest candidate at or before each label's
/// moment beside them.
fn joined(labels: &CrowFrame, candidates: &Candidates, label_of: &[(usize, Vec<u8>)], bring: &[String], _keys: &[String], _time: &str, moment: Option<&dyn Fn(usize) -> Option<i64>>) -> Result<CrowFrame, String> {
    let nl = labels.nrows();
    let mut pick: Vec<Option<(&CrowFrame, usize)>> = vec![None; nl];
    for (l, kb) in label_of {
        let Some(m) = moment.and_then(|f| f(*l)) else { continue };
        if let Some(cs) = candidates.get(kb) {
            let mut best: Option<&(i64, i64, CrowFrame, usize)> = None;
            for c in cs {
                if c.0 <= m && best.is_none_or(|b| (c.0, c.1) > (b.0, b.1)) {
                    best = Some(c);
                }
            }
            if let Some(b) = best {
                pick[*l] = Some((&b.2, 0));
            }
        }
    }
    let mut names = labels.columns.clone();
    let mut data: Vec<Column> = labels.data.iter().map(|c| c.as_ref().clone()).collect();
    let mut validity: Vec<Vec<bool>> = labels.validity.iter().map(|v| v.to_vec()).collect();
    for b in bring {
        let mut cells: Vec<Option<Cell>> = Vec::with_capacity(nl);
        let mut kind: Option<super::encode::Kind> = None;
        for p in &pick {
            match p {
                Some((f, i)) => match f.col_index(b) {
                    Some(c) => {
                        kind = Some(super::encode::Kind::of(f.data[c].as_ref()));
                        cells.push(if f.validity[c][*i] { Some(cell_of(f.data[c].as_ref(), *i)) } else { None });
                    }
                    None => cells.push(None),
                },
                None => cells.push(None),
            }
        }
        let (col, valid) = column_of(&cells, kind.unwrap_or(super::encode::Kind::Float));
        names.push(b.clone());
        data.push(col);
        validity.push(valid);
    }
    Ok(CrowFrame::new(names, data, validity))
}
