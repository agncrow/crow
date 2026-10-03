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

/// `frame.load[path]`: a table as a frame — a file, a URL or an S3 object
/// (crate::sources) — in the format its extension names: CSV, each
/// column's type read from its cells; JSON records; Parquet. A URL with no
/// extension is read as what its server says it answered.
pub fn load(path: &str) -> Result<CrowFrame, String> {
    load_from(path, &crate::sources::Ask::default())
}

/// `frame.load[url, headers = map[…], query = map[…], timeout = s]`: a
/// URL's table, asked with the options (crate::sources::Ask).
pub fn load_asked(path: &str, headers: &std::collections::BTreeMap<String, String>, query: &std::collections::BTreeMap<String, String>, timeout: f64) -> Result<CrowFrame, String> {
    load_from(path, &crate::sources::Ask::for_path(path, headers, query, timeout)?)
}

/// A table, from wherever its path says, a URL asked with the options.
pub fn load_from(path: &str, ask: &crate::sources::Ask) -> Result<CrowFrame, String> {
    use crate::sources::{self, Format};
    let named = sources::format_of(path);
    if !sources::is_remote(path) {
        return match named {
            Some(Format::Json) => crate::frame::from_json(&std::fs::read_to_string(path).map_err(|e| format!("load: {}", e))?),
            Some(Format::Parquet) => parquet_frame(std::fs::read(path).map_err(|e| format!("load: {}", e))?),
            _ => crate::frame::load_csv(path),
        };
    }
    let fetched = sources::fetch_asked(path, ask)?;
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).map_err(|_| format!("cannot read '{}': it is not text (UTF-8)", path));
    match named.unwrap_or_else(|| sources::format_sniffed(fetched.media.as_deref(), &fetched.bytes)) {
        Format::Parquet => parquet_frame(fetched.bytes),
        Format::Json => crate::frame::from_json(&text(fetched.bytes)?),
        Format::Csv => crate::frame::csv_frame(&text(fetched.bytes)?),
    }
}

/// `frame.save[frame, path]`: a frame as a file or an S3 object, in the
/// format its extension names (CSV otherwise). A failure is an error value
/// of kind 'frame', as `load`'s is (compiled, it stopped the program while
/// the interpreter answered the error).
pub fn save(f: &CrowFrame, path: &str) -> Result<String, String> {
    use crate::sources::{self, Format};
    let format = sources::format_of(path);
    if !sources::is_remote(path) && !matches!(format, Some(Format::Json | Format::Parquet)) {
        return f.save(path).map(|_| String::new());
    }
    let bytes = match format {
        Some(Format::Json) => crate::frame::to_json(f).into_bytes(),
        Some(Format::Parquet) => parquet_bytes(f)?,
        _ => f.to_csv()?.into_bytes(),
    };
    if sources::is_remote(path) {
        sources::put(path, &bytes)?;
    } else {
        std::fs::write(path, bytes).map_err(|e| format!("save: {}", e))?;
    }
    Ok(String::new())
}

/// A Parquet file's bytes as a frame, where the program was built to read
/// Parquet (the interpreter always is; a compiled program whose paths may
/// name one).
fn parquet_frame(bytes: Vec<u8>) -> Result<CrowFrame, String> {
    #[cfg(feature = "parquet")]
    {
        crate::frame::from_parquet(bytes)
    }
    #[cfg(not(feature = "parquet"))]
    {
        let _ = bytes;
        Err("load: this program was built without Parquet".to_string())
    }
}

/// A frame as a Parquet file's bytes, likewise.
fn parquet_bytes(f: &CrowFrame) -> Result<Vec<u8>, String> {
    #[cfg(feature = "parquet")]
    {
        crate::frame::to_parquet(f)
    }
    #[cfg(not(feature = "parquet"))]
    {
        let _ = f;
        Err("save: this program was built without Parquet".to_string())
    }
}

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

/// The aggregations a `group` is given, each `[column, how]`.
fn aggregations<'a>(aggs: &'a [&'a [String]]) -> Result<Vec<(&'a str, AggFn)>, String> {
    let mut pairs = Vec::with_capacity(aggs.len());
    for a in aggs {
        let [column, how] = a else {
            return Err(format!("group: an aggregation is [column, how], got {} values", a.len()));
        };
        let how = AggFn::from_str(how).ok_or_else(|| format!("group: no aggregation '{}' — write 'sum', 'mean', 'min', 'max' or 'count'", how))?;
        pairs.push((column.as_str(), how));
    }
    Ok(pairs)
}

/// `group[frame, name, [column, how], …]`: one row per value of `name`,
/// each `[column, how]` an aggregation of `column`.
pub fn group(f: &CrowFrame, by: &str, aggs: &[&[String]]) -> Result<CrowFrame, String> {
    f.group_by(&[by], &aggregations(aggs)?)
}

/// `group[frame, ['a', 'b'], [column, how], …]`: one row per distinct
/// combination of the keys' cells.
pub fn group_keys(f: &CrowFrame, keys: &[String], aggs: &[&[String]]) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    f.group_by(&keys, &aggregations(aggs)?)
}

/// `pivot[frame, rows, cols, values]`: the long frame made wide — a row per
/// cell of `rows`, a column per cell of `cols`, the `values` cell of each
/// pair in between (`CrowFrame::pivot`).
pub fn pivot(f: &CrowFrame, rows: &str, cols: &str, values: &str) -> Result<CrowFrame, String> {
    f.pivot(rows, cols, values)
}

/// `window[frame, col, how, n = rows, by = [keys]]`: a column computed over
/// each row's partition (`CrowFrame::window`).
pub fn window(f: &CrowFrame, col: &str, how: &str, n: i64, by: &[String]) -> Result<CrowColumn, String> {
    let by: Vec<&str> = by.iter().map(String::as_str).collect();
    f.window(col, how, n, &by)
}

/// `window[frame, col, how, n = rows, by = 'key']`: one key names the
/// partitions.
pub fn window_one(f: &CrowFrame, col: &str, how: &str, n: i64, by: &str) -> Result<CrowColumn, String> {
    let by: Vec<&str> = if by.is_empty() { Vec::new() } else { vec![by] };
    f.window(col, how, n, &by)
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

    /// Trades over two days and symbols: the frame the groups, pivots and
    /// windows below are read from.
    fn trades() -> CrowFrame {
        let f = with(&new(), "sym", Cells::from(vec!["b".to_string(), "a".to_string(), "b".to_string(), "a".to_string(), "b".to_string()])).expect("five rows");
        let f = with(&f, "day", Cells::from(vec![2_i64, 1, 1, 2, 1])).expect("five rows");
        let f = with(&f, "qty", Cells::from(vec![10_i64, 1, 20, 2, 30])).expect("five rows");
        with(&f, "px", Cells::from(vec![1.5, 2.0, 0.5, 4.0, 1.0])).expect("five rows")
    }

    fn column(f: &CrowFrame, name: &str) -> (Column, Vec<bool>) {
        let c = f.col_native(name).expect("the column");
        ((*c.data).clone(), (*c.valid).clone())
    }

    /// A group's keys keep their kinds and come in the order first seen,
    /// one key or several; an aggregate is a float named column_how.
    #[test]
    fn a_group_keeps_its_keys_kinds() {
        let f = trades();
        let sum = ["qty".to_string(), "sum".to_string()];
        let g = group(&f, "day", &[&sum]).expect("a group");
        assert_eq!(g.columns, vec!["day", "qty_sum"]);
        assert_eq!(column(&g, "day").0, Column::Int(vec![2, 1]));
        assert_eq!(column(&g, "qty_sum").0, Column::Float(vec![12.0, 51.0]));
        let g = group_keys(&f, &["sym".to_string(), "day".to_string()], &[&sum]).expect("a group");
        assert_eq!(g.columns, vec!["sym", "day", "qty_sum"]);
        assert_eq!(column(&g, "sym").0, Column::Str(vec!["b".into(), "a".into(), "b".into(), "a".into()]));
        assert_eq!(column(&g, "day").0, Column::Int(vec![2, 1, 1, 2]));
        assert_eq!(column(&g, "qty_sum").0, Column::Float(vec![10.0, 1.0, 50.0, 2.0]));
        assert_eq!(group_keys(&f, &[], &[&sum]).map(|_| ()), Err("group: a key is needed — group[frame, 'col', …] or group[frame, ['a', 'b'], …]".to_string()));
        assert_eq!(group_keys(&f, &["sym".to_string(), "sym".to_string()], &[&sum]).map(|_| ()), Err("group: key 'sym' is named twice".to_string()));
        assert_eq!(group_keys(&f, &["sym".to_string(), "nope".to_string()], &[&sum]).map(|_| ()), Err("group: column 'nope' not found".to_string()));
    }

    /// A pivot is one row per cell of `rows`, a column per cell of `cols`,
    /// both ascending, the values between and a missing cell where no row
    /// has the pair; a pair two rows share is the reason.
    #[test]
    fn a_pivot_makes_the_long_frame_wide() {
        let f = trades();
        let one_per_pair = filter(&f, &[true, true, false, true, true]).expect("four rows");
        let p = pivot(&one_per_pair, "sym", "day", "qty").expect("a pivot");
        assert_eq!(p.columns, vec!["sym", "1", "2"]);
        assert_eq!(column(&p, "sym").0, Column::Str(vec!["a".into(), "b".into()]));
        assert_eq!(column(&p, "1"), (Column::Int(vec![1, 30]), vec![true, true]));
        assert_eq!(column(&p, "2"), (Column::Int(vec![2, 10]), vec![true, true]));
        let sparse = filter(&f, &[true, true, false, false, true]).expect("three rows");
        let p = pivot(&sparse, "day", "sym", "px").expect("a pivot");
        assert_eq!(p.columns, vec!["day", "a", "b"]);
        assert_eq!(column(&p, "day").0, Column::Int(vec![1, 2]));
        let (a, present) = column(&p, "a");
        assert_eq!(present, vec![true, false]);
        assert!(matches!(a, Column::Float(v) if v[0] == 2.0 && v[1].is_nan()));
        assert_eq!(column(&p, "b"), (Column::Float(vec![1.0, 1.5]), vec![true, true]));
        assert_eq!(
            pivot(&f, "sym", "day", "qty").map(|_| ()),
            Err("pivot: 'b' and '1' appear 2 times — one row per pair: aggregate first, group[frame, ['sym', 'day'], ['qty', 'mean']]".to_string())
        );
        assert_eq!(pivot(&f, "sym", "sym", "qty").map(|_| ()), Err("pivot: the rows and the columns are one column, 'sym'".to_string()));
        assert_eq!(pivot(&f, "sym", "day", "nope").map(|_| ()), Err("pivot: column 'nope' not found".to_string()));
    }

    /// A window is computed over each row's partition in the frame's order:
    /// running and trailing aggregates, counts, lags and leads with missing
    /// cells at a partition's edge, and ranks with ties.
    #[test]
    fn a_window_is_computed_over_each_rows_partition() {
        let f = trades();
        let by = ["sym".to_string()];
        let c = |r: Result<CrowColumn, String>| r.map(|c| ((*c.data).clone(), (*c.valid).clone()));
        assert_eq!(c(window(&f, "qty", "sum", 0, &by)), Ok((Column::Float(vec![10.0, 1.0, 30.0, 3.0, 60.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "mean", 2, &by)), Ok((Column::Float(vec![10.0, 1.0, 15.0, 1.5, 25.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "px", "max", 0, &[])), Ok((Column::Float(vec![1.5, 2.0, 2.0, 4.0, 4.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "px", "min", 2, &[])), Ok((Column::Float(vec![1.5, 1.5, 0.5, 0.5, 1.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "count", 0, &by)), Ok((Column::Int(vec![1, 1, 2, 2, 3]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "lag", 1, &by)), Ok((Column::Int(vec![0, 0, 10, 1, 20]), vec![false, false, true, true, true])));
        let (lead, present) = c(window(&f, "px", "lead", 1, &by)).expect("a column");
        assert_eq!(present, vec![true, true, true, false, false]);
        assert!(matches!(lead, Column::Float(v) if v[..3] == [0.5, 4.0, 1.0] && v[3].is_nan() && v[4].is_nan()));
        assert_eq!(c(window(&f, "px", "rank", 0, &[])), Ok((Column::Int(vec![3, 4, 1, 5, 2]), vec![true; 5])));
        let tied = with(&f, "t", Cells::from(vec![1_i64, 1, 2, 2, 3])).expect("five rows");
        assert_eq!(c(window(&tied, "t", "rank", 0, &[])), Ok((Column::Int(vec![1, 1, 3, 3, 5]), vec![true; 5])));
        assert_eq!(c(window(&f, "sym", "rank", 0, &by)), Ok((Column::Int(vec![1, 1, 1, 1, 1]), vec![true; 5])));
        assert_eq!(window(&f, "qty", "median", 0, &[]).map(|_| ()), Err("window: no window 'median' — write 'sum', 'mean', 'min', 'max', 'count', 'lag', 'lead' or 'rank'".to_string()));
        assert_eq!(window(&f, "qty", "lag", 0, &[]).map(|_| ()), Err("window: lag needs n of 1 or more, the rows between (got 0)".to_string()));
        assert_eq!(window(&f, "qty", "sum", -1, &[]).map(|_| ()), Err("window: n is 0 or more (got -1)".to_string()));
        assert!(window(&f, "sym", "sum", 0, &[]).is_err(), "a column of text has no sum");
        assert_eq!(window(&f, "qty", "sum", 0, &["nope".to_string()]).map(|_| ()), Err("window: column 'nope' not found".to_string()));
        // a missing key cell: a cell in no partition, missing
        let keyed = CrowFrame::new(vec!["k".into(), "x".into()], vec![Column::Int(vec![1, 1, 1]), Column::Float(vec![1.0, 2.0, 3.0])], vec![vec![true, false, true], vec![true; 3]]);
        assert_eq!(c(window(&keyed, "x", "sum", 0, &["k".to_string()])), Ok((Column::Float(vec![1.0, 0.0, 4.0]), vec![true, false, true])));
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
