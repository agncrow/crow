//! The functions `use frame` gives a program (docs/bug-classes.md §4),
//! each one runtime function both modes call, over the kernels in
//! [`crate::frame`]. A misuse — a column the frame lacks, an aggregation
//! or a join it does not know, a mask or a count that does not fit — is
//! the reason the program stops with, as `df.col` of a missing column is.
//! Values `with` cannot fit to the rows, and text that is no time, are
//! error values. (The interpreter had answered every failure an error
//! value, where the type, a frame, said none could happen, and the
//! compiled program stopped or panicked.)

use crate::frame::{format_datetime, parse_datetime, AggFn, Column, CrowColumn, CrowFrame};
use crate::tensor::CrowTensor;

pub use crate::cell::Cell;

/// A cell as a column of one row, for `with` to repeat.
fn one_row(cell: Cell) -> Result<Column, String> {
    match cell {
        Cell::Int(n) => Ok(Column::Int(vec![n])),
        Cell::Float(x) => Ok(Column::Float(vec![x])),
        Cell::Bool(b) => Ok(Column::Bool(vec![b])),
        Cell::Text(s) => Ok(Column::Str(vec![s])),
        other => Err(format!("with: a column cannot hold {}", other.kind())),
    }
}

/// The values a program hands `with` for a column: a list of one kind, a
/// tensor's numbers, another frame's column with its missing cells, or a
/// cell repeated for every row.
#[derive(Debug, Clone)]
pub enum Cells {
    /// The column's values and, row by row, whether each is present.
    Column(Column, Vec<bool>),
    One(Cell),
}

impl Cells {
    fn all(c: Column) -> Cells {
        let n = c.len();
        Cells::Column(c, vec![true; n])
    }
}

impl From<Vec<i64>> for Cells {
    fn from(v: Vec<i64>) -> Self {
        Cells::all(Column::Int(v))
    }
}
impl From<Vec<f64>> for Cells {
    fn from(v: Vec<f64>) -> Self {
        Cells::all(Column::Float(v))
    }
}
impl From<Vec<bool>> for Cells {
    fn from(v: Vec<bool>) -> Self {
        Cells::all(Column::Bool(v))
    }
}
impl From<Vec<String>> for Cells {
    fn from(v: Vec<String>) -> Self {
        Cells::all(Column::Str(v))
    }
}
impl From<CrowTensor> for Cells {
    fn from(t: CrowTensor) -> Self {
        Cells::all(Column::Float(t.to_vec()))
    }
}
impl From<CrowColumn> for Cells {
    fn from(c: CrowColumn) -> Self {
        Cells::Column((*c.data).clone(), (*c.valid).clone())
    }
}
impl From<i64> for Cells {
    fn from(n: i64) -> Self {
        Cells::One(Cell::Int(n))
    }
}
impl From<f64> for Cells {
    fn from(x: f64) -> Self {
        Cells::One(Cell::Float(x))
    }
}
impl From<bool> for Cells {
    fn from(b: bool) -> Self {
        Cells::One(Cell::Bool(b))
    }
}
impl From<String> for Cells {
    fn from(s: String) -> Self {
        Cells::One(Cell::Text(s))
    }
}

/// `frame.new[]`: the empty frame.
pub fn new() -> CrowFrame {
    CrowFrame::new(vec![], vec![], vec![])
}

/// `with[frame, name, values]`: the frame with the column added or
/// replaced; values of another length than the rows are an error value.
pub fn with(f: &CrowFrame, name: &str, values: Cells) -> Result<CrowFrame, String> {
    match values {
        Cells::Column(data, valid) => f.with_valid(name, data, valid),
        Cells::One(cell) => f.with_scalar(name, one_row(cell)?),
    }
}

/// `select[frame, names]`.
pub fn select(f: &CrowFrame, names: &[String]) -> Result<CrowFrame, String> {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    f.select(&names)
}

/// `filter[frame, mask]`: the rows the mask keeps, a mask per row.
pub fn filter(f: &CrowFrame, mask: &[bool]) -> Result<CrowFrame, String> {
    f.filter(mask)
}

/// `sort[frame, name]`, ascending.
pub fn sort(f: &CrowFrame, name: &str) -> Result<CrowFrame, String> {
    f.sort(name, false)
}

/// `sort[frame, name, desc = true]`.
pub fn sort_by(f: &CrowFrame, name: &str, desc: bool) -> Result<CrowFrame, String> {
    f.sort(name, desc)
}

/// `group[frame, name, [column, how], …]`: one row per value of `name`,
/// each `[column, how]` an aggregation of `column`.
pub fn group(f: &CrowFrame, by: &str, aggs: &[&[String]]) -> Result<CrowFrame, String> {
    let mut pairs = Vec::with_capacity(aggs.len());
    for a in aggs {
        let [column, how] = a else {
            return Err(format!("group: an aggregation is [column, how], got {} values", a.len()));
        };
        let how = AggFn::from_str(how).ok_or_else(|| format!("group: no aggregation '{}' — write 'sum', 'mean', 'min', 'max' or 'count'", how))?;
        pairs.push((column.as_str(), how));
    }
    f.group(by, &pairs)
}

/// `join[left, right, key]`: the rows whose keys match.
pub fn join(left: &CrowFrame, right: &CrowFrame, key: &str) -> Result<CrowFrame, String> {
    left.join(right, key, "inner")
}

/// `join[left, right, key, how]`: `'inner'`, `'left'`, `'right'` or
/// `'outer'`. (Another word was an inner join.)
pub fn join_how(left: &CrowFrame, right: &CrowFrame, key: &str, how: &str) -> Result<CrowFrame, String> {
    if !matches!(how, "inner" | "left" | "right" | "outer") {
        return Err(format!("join: no join '{}' — write 'inner', 'left', 'right' or 'outer'", how));
    }
    left.join(right, key, how)
}

/// `stack[top, bottom]`: the rows of both, the columns alike.
pub fn stack(top: &CrowFrame, bottom: &CrowFrame) -> Result<CrowFrame, String> {
    top.stack(bottom)
}

/// `head[frame, n]`: the first `n` rows, all of them when there are fewer.
pub fn head(f: &CrowFrame, n: i64) -> Result<CrowFrame, String> {
    Ok(f.head(crate::stops::size("head", n)?))
}

/// `tail[frame, n]`: the last `n` rows.
pub fn tail(f: &CrowFrame, n: i64) -> Result<CrowFrame, String> {
    Ok(f.tail(crate::stops::size("tail", n)?))
}

/// `fill[frame, name, value]`: the column's missing cells given `value`,
/// of the column's kind (an int fills a float column).
pub fn fill(f: &CrowFrame, name: &str, value: &Cell) -> Result<CrowFrame, String> {
    let column = f.col(name).ok_or_else(|| format!("fill: column '{}' not found", name))?;
    let text = match (column, value) {
        (Column::Int(_), Cell::Int(n)) => n.to_string(),
        (Column::Float(_), Cell::Float(x)) => x.to_string(),
        (Column::Float(_), Cell::Int(n)) => n.to_string(),
        (Column::Bool(_), Cell::Bool(b)) => b.to_string(),
        (Column::Str(_), Cell::Text(s)) => s.clone(),
        (Column::DateTime(..), _) => return Err(format!("fill: column '{}' holds moments, which fill does not give yet", name)),
        (c, v) => return Err(format!("fill: column '{}' is {}, and {} is not", name, c.dtype_name(), v.shown())),
    };
    f.fill_column(name, &text)
}

/// `unique[frame, name]`: the column's distinct values, as text.
pub fn unique(f: &CrowFrame, name: &str) -> Result<Vec<String>, String> {
    f.unique_col(name)
}

/// `rename[frame, old, new]`.
pub fn rename(f: &CrowFrame, old: &str, new: &str) -> Result<CrowFrame, String> {
    f.rename(old, new)
}

/// `stats[frame]`: a summary of each column, as text.
pub fn stats(f: &CrowFrame) -> String {
    f.stats_summary()
}

/// `datetime[s]`: ISO 8601 text as the moment it names, written back in
/// its own offset; text that is no time is an error value.
pub fn datetime(s: &str) -> Result<String, String> {
    let (millis, offset) = parse_datetime(s).ok_or_else(|| format!("datetime: cannot parse '{}'", s))?;
    Ok(format_datetime(millis, offset))
}

/// `tz[s, offset]`: the moment `s` names, written in `offset` — `'UTC'`
/// or `'+HH:MM'` / `'-HH:MM'`.
pub fn tz(s: &str, offset: &str) -> Result<String, String> {
    let (millis, _) = parse_datetime(s).ok_or_else(|| format!("tz: cannot parse '{}'", s))?;
    let minutes = offset_minutes(offset).ok_or_else(|| format!("tz: invalid offset '{}', use 'UTC' or '+HH:MM'/'-HH:MM'", offset))?;
    Ok(format_datetime(millis, minutes))
}

/// `'UTC'` → 0, `'+05:30'` → 330, `'-08:00'` → -480.
fn offset_minutes(offset: &str) -> Option<i16> {
    if offset == "UTC" {
        return Some(0);
    }
    let bytes = offset.as_bytes();
    let sign: i16 = match bytes.first() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return None,
    };
    if bytes.len() != 6 || bytes.get(3) != Some(&b':') {
        return None;
    }
    let h: i16 = offset.get(1..3)?.parse().ok()?;
    let m: i16 = offset.get(4..6)?.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h * 60 + m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> CrowFrame {
        let f = with(&new(), "name", Cells::from(vec!["a".to_string(), "b".to_string(), "c".to_string()])).expect("three rows");
        with(&f, "age", Cells::from(vec![30_i64, 40, 50])).expect("three rows")
    }

    #[test]
    fn a_misuse_is_the_reason() {
        let f = people();
        assert_eq!(select(&f, &["nope".to_string()]).map(|_| ()), Err("select: column 'nope' not found".to_string()));
        assert_eq!(head(&f, -1).map(|_| ()), Err("head: a size cannot be negative (got -1)".to_string()));
        let median = ["age".to_string(), "median".to_string()];
        assert_eq!(group(&f, "name", &[&median]).map(|_| ()), Err("group: no aggregation 'median' — write 'sum', 'mean', 'min', 'max' or 'count'".to_string()));
        assert_eq!(join_how(&f, &f, "name", "lefty").map(|_| ()), Err("join: no join 'lefty' — write 'inner', 'left', 'right' or 'outer'".to_string()));
        assert_eq!(fill(&f, "age", &Cell::Float(2.5)).map(|_| ()), Err("fill: column 'age' is int, and 2.5 is not".to_string()));
        assert_eq!(with(&f, "x", Cells::from(vec![1_i64])).map(|_| ()), Err("with: column 'x' has 1 values but the frame has 3 rows".to_string()));
    }

    #[test]
    fn a_time_is_parsed_once() {
        assert_eq!(tz("2026-07-14T13:30:00Z", "+09:00"), Ok("2026-07-14T22:30:00+09:00".to_string()));
        assert_eq!(tz("2026-07-14T13:30:00Z", "+5:30"), Err("tz: invalid offset '+5:30', use 'UTC' or '+HH:MM'/'-HH:MM'".to_string()));
        assert_eq!(datetime("not a time"), Err("datetime: cannot parse 'not a time'".to_string()));
    }

    /// A column copied with `with` keeps its missing cells (both modes had
    /// made them a guess: NaN, 0 or false).
    #[test]
    fn a_copied_column_keeps_its_missing_cells() {
        let f = CrowFrame::new(vec!["n".into()], vec![Column::Int(vec![1, 0])], vec![vec![true, false]]);
        let copied = with(&f, "m", Cells::from(f.col_native("n").expect("the column"))).expect("two rows");
        assert_eq!(copied.col_native("m").map(|c| (*c.valid).clone()), Some(vec![true, false]));
    }
}
