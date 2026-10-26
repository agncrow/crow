//! The functions `use frame` gives a program (docs/bug-classes.md §4),
//! each one runtime function both modes call, over the kernels in
//! [`crate::frame`]. A misuse — a column the frame lacks, an aggregation
//! or a join it does not know, a mask or a count that does not fit — is
//! the reason the program stops with, as `df.col` of a missing column is.
//! Values `with` cannot fit to the rows, and text that is no time, are
//! error values. (The interpreter had answered every failure an error
//! value, where the type, a frame, said none could happen, and the
//! compiled program stopped or panicked.)

use crate::frame::{AggFn, Column, CrowColumn, CrowFrame, Validity};
use crate::tensor::CrowTensor;

pub use crate::cell::Cell;

/// A Parquet file's bytes as a frame, where the program was built to read
/// Parquet (the interpreter always is; a compiled program whose paths may
/// name one).
pub fn parquet_frame(bytes: Vec<u8>) -> Result<CrowFrame, String> {
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
pub fn parquet_bytes(f: &CrowFrame) -> Result<Vec<u8>, String> {
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
pub(crate) fn one_row(cell: Cell) -> Result<Column, String> {
    match cell {
        Cell::Int(n) => Ok(Column::Int(vec![n])),
        Cell::Float(x) => Ok(Column::Float(vec![x])),
        Cell::Bool(b) => Ok(Column::Bool(vec![b])),
        Cell::Text(s) => Ok(Column::strs(vec![s])),
        Cell::Moment(m) => Ok(Column::DateTime(vec![m.ms], vec![m.offset])),
        other => Err(format!("with: a column cannot hold {}", other.kind())),
    }
}

/// The values a program hands `with` for a column: a list of one kind, a
/// tensor's numbers, another frame's column with its missing cells, or a
/// cell repeated for every row.
#[derive(Debug, Clone)]
pub enum Cells {
    /// The column's values and, row by row, whether each is present.
    Column(Column, Validity),
    One(Cell),
}

impl Cells {
    fn all(c: Column) -> Cells {
        let n = c.len();
        Cells::Column(c, Validity::all(n))
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
        Cells::all(Column::strs(v))
    }
}
impl From<Vec<crate::moments::Moment>> for Cells {
    fn from(v: Vec<crate::moments::Moment>) -> Self {
        Cells::all(Column::DateTime(v.iter().map(|m| m.ms).collect(), v.iter().map(|m| m.offset).collect()))
    }
}
impl From<crate::moments::Moment> for Cells {
    fn from(m: crate::moments::Moment) -> Self {
        Cells::One(Cell::Moment(m))
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
        Cells::Column(data, valid) => f.with_validity(name, data, valid),
        Cells::One(cell) => f.with_scalar(name, one_row(cell)?),
    }
}

/// `df.`name``: a column read by a name the program computes, the column
/// `df.name` reads; where the frame has no such column the program stops
/// in the words a written name's read stops with.
pub fn column(f: &CrowFrame, name: &str) -> Result<CrowColumn, String> {
    f.col_native(name).ok_or_else(|| crate::stops::no_column(name))
}

/// `select[frame, names]`.
pub fn select(f: &CrowFrame, names: &[String]) -> Result<CrowFrame, String> {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    f.select(&names)
}

/// `f[mask]`: the rows the mask keeps, a mask per row (the lowering's
/// fused form, `mask::pick_where`, where the mask compares the frame's
/// own columns).
pub fn pick_rows(f: &CrowFrame, mask: &[bool]) -> Result<CrowFrame, String> {
    f.pick_rows(mask)
}

/// `f.name[mask]`: the cells the mask keeps, a column of the same kind with
/// its missing cells kept — what `f[mask].name` reads, one column picked
/// rather than every one. A mask of another length stops in the words a
/// list's mask does.
pub fn pick_cells(c: &CrowColumn, mask: &[bool]) -> Result<CrowColumn, String> {
    if mask.len() != c.len() {
        return Err(format!("a mask of {} picks from a column of {}: they must be as long as each other", mask.len(), c.len()));
    }
    let (data, valid) = crate::frame::pick_column(&c.data, &c.valid, mask);
    Ok(CrowColumn { data: std::sync::Arc::new(data), valid: std::sync::Arc::new(valid) })
}

/// `sort[frame, name]`, ascending.
pub fn sort(f: &CrowFrame, name: &str) -> Result<CrowFrame, String> {
    f.sort(name, false)
}

/// `sort[frame, name, desc = true]`.
pub fn sort_by(f: &CrowFrame, name: &str, desc: bool) -> Result<CrowFrame, String> {
    f.sort(name, desc)
}

/// `sort[frame, ['a', 'b']]`: by the first key, ties by the next.
pub fn sort_keys(f: &CrowFrame, keys: &[String]) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    f.sort_keys(&keys, &vec![false; keys.len()])
}

/// `sort[frame, ['a', 'b'], desc = true]`: every key descending.
pub fn sort_keys_by(f: &CrowFrame, keys: &[String], desc: bool) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    f.sort_keys(&keys, &vec![desc; keys.len()])
}

/// `sort[frame, ['a', 'b'], desc = [false, true]]`: a direction per key.
pub fn sort_keys_each(f: &CrowFrame, keys: &[String], desc: &[bool]) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    f.sort_keys(&keys, desc)
}

/// The aggregations a `group` is given, each `[column, how]`.
fn aggregations<'a>(aggs: &'a [&'a [String]]) -> Result<Vec<(&'a str, AggFn)>, String> {
    let mut pairs = Vec::with_capacity(aggs.len());
    for a in aggs {
        let [column, how] = a else {
            return Err(format!("group: an aggregation is [column, how], got {} values", a.len()));
        };
        let how = AggFn::from_str(how).ok_or_else(|| format!("group: no aggregation '{}' — write {}", how, AggFn::WORDS))?;
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

/// `window[frame, col, how, n = rows, by = [keys], span = seconds, on =
/// column]`: a column computed over each row's partition
/// (`CrowFrame::window_over`).
pub fn window(f: &CrowFrame, col: &str, how: &str, n: i64, by: &[String], span: f64, on: &str) -> Result<CrowColumn, String> {
    let by: Vec<&str> = by.iter().map(String::as_str).collect();
    f.window_over(col, how, n, &by, span, on)
}

/// `window[frame, col, how, n = rows, by = 'key', …]`: one key names the
/// partitions.
pub fn window_one(f: &CrowFrame, col: &str, how: &str, n: i64, by: &str, span: f64, on: &str) -> Result<CrowColumn, String> {
    let by: Vec<&str> = if by.is_empty() { Vec::new() } else { vec![by] };
    f.window_over(col, how, n, &by, span, on)
}

/// `join[left, right, key]`: the rows whose keys match.
pub fn join(left: &CrowFrame, right: &CrowFrame, key: &str) -> Result<CrowFrame, String> {
    left.join(right, key, "inner")
}

/// `join[left, right, key, how]`: `'inner'`, `'left'`, `'right'`,
/// `'outer'`, `'semi'`, `'anti'` or `'asof'`. (Another word was an inner
/// join.)
pub fn join_how(left: &CrowFrame, right: &CrowFrame, key: &str, how: &str) -> Result<CrowFrame, String> {
    left.join_keys(right, &[key], how)
}

/// `join[left, right, ['a', 'b']]`: the rows whose every key matches.
pub fn join_keys(left: &CrowFrame, right: &CrowFrame, keys: &[String]) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    left.join_keys(right, &keys, "inner")
}

/// `join[left, right, ['a', 'b'], how]`.
pub fn join_keys_how(left: &CrowFrame, right: &CrowFrame, keys: &[String], how: &str) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    left.join_keys(right, &keys, how)
}

/// `unique[frame, ['a', 'b']]`: the first row of each distinct combination.
pub fn unique_rows(f: &CrowFrame, keys: &[String]) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    f.unique_rows(&keys)
}

/// `unpivot[frame, ['id'], ['a', 'b']]`: the wide frame made long.
pub fn unpivot(f: &CrowFrame, keys: &[String], cols: &[String]) -> Result<CrowFrame, String> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
    f.unpivot(&keys, &cols)
}

/// `unpivot[frame, 'id', ['a', 'b']]`: one key.
pub fn unpivot_one(f: &CrowFrame, key: &str, cols: &[String]) -> Result<CrowFrame, String> {
    let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
    f.unpivot(&[key], &cols)
}

/// `complete[frame]`: the rows with every cell present.
pub fn complete(f: &CrowFrame) -> Result<CrowFrame, String> {
    f.complete_rows(&[])
}

/// `complete[frame, 'col']`: the rows present in that column.
pub fn complete_one(f: &CrowFrame, col: &str) -> Result<CrowFrame, String> {
    f.complete_rows(&[col])
}

/// `complete[frame, ['a', 'b']]`: the rows present in those columns.
pub fn complete_in(f: &CrowFrame, cols: &[String]) -> Result<CrowFrame, String> {
    let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
    f.complete_rows(&cols)
}

/// `sample[frame, n]`: `n` rows drawn by the seeded generator.
pub fn sample(f: &CrowFrame, n: i64) -> Result<CrowFrame, String> {
    f.sample_rows(n)
}

/// `bucket[column, seconds]`: each moment's bucket start.
pub fn bucket(c: &CrowColumn, span: f64) -> Result<CrowColumn, String> {
    crate::verbs::bucket_column(c, span)
}

/// `where[mask, a, b]`: `a`'s cell where the mask is true, `b`'s elsewhere.
pub fn choose(mask: &[bool], a: Cells, b: Cells) -> Result<CrowColumn, String> {
    crate::verbs::choose(mask, a, b)
}

/// `int[column]`: every cell as an int, one that cannot be missing.
pub fn int_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    crate::verbs::cast_column(c, "int")
}

/// `float[column]`.
pub fn float_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    crate::verbs::cast_column(c, "float")
}

/// `str[column]`: each cell as it shows.
pub fn str_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    crate::verbs::cast_column(c, "str")
}

/// `moment[column]`: text read as moments, numbers as seconds.
pub fn moment_column(c: &CrowColumn) -> Result<CrowColumn, String> {
    crate::verbs::cast_column(c, "moment")
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
/// of the column's kind (an int fills a float column); `fill[frame, name,
/// how = 'forward']` or `'backward'` gives each the nearest present cell.
pub fn fill(f: &CrowFrame, name: &str, value: &Cell, how: &str) -> Result<CrowFrame, String> {
    if !how.is_empty() {
        return f.fill_direction(name, how);
    }
    let column = f.col(name).ok_or_else(|| format!("fill: column '{}' not found", name))?;
    let text = match (column, value) {
        (Column::Int(_), Cell::Int(n)) => n.to_string(),
        (Column::Float(_), Cell::Float(x)) => x.to_string(),
        (Column::Float(_), Cell::Int(n)) => n.to_string(),
        (Column::Bool(_), Cell::Bool(b)) => b.to_string(),
        (Column::Str(_), Cell::Text(s)) => s.clone(),
        (Column::DateTime(..), Cell::Moment(m)) => m.to_string(),
        (c, v) => return Err(format!("fill: column '{}' is {}, and {} is not", name, c.dtype_name(), v.shown())),
    };
    f.fill_column(name, &text)
}

/// `unique[frame, name]`: the column's distinct values, as text.
pub fn unique(f: &CrowFrame, name: &str) -> Result<CrowColumn, String> {
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
        let median = ["age".to_string(), "mediam".to_string()];
        assert_eq!(group(&f, "name", &[&median]).map(|_| ()), Err(format!("group: no aggregation 'mediam' — write {}", AggFn::WORDS)));
        assert_eq!(join_how(&f, &f, "name", "lefty").map(|_| ()), Err("join: no join 'lefty' — write 'inner', 'left', 'right', 'outer', 'semi', 'anti' or 'asof'".to_string()));
        assert_eq!(fill(&f, "age", &Cell::Float(2.5), "").map(|_| ()), Err("fill: column 'age' is int, and 2.5 is not".to_string()));
        assert_eq!(fill(&f, "age", &Cell::Int(0), "sideways").map(|_| ()), Err("fill: no direction 'sideways' — how = 'forward' or 'backward'".to_string()));
        assert_eq!(with(&f, "x", Cells::from(vec![1_i64])).map(|_| ()), Err("column 'x' is given 1 values, and the frame has 3 rows".to_string()));
    }


    /// A mask picks a frame's rows and a column's cells alike: the column
    /// keeps its kind and its missing cells, and a mask of another length
    /// stops in the words a list's mask does.
    #[test]
    fn a_mask_picks_rows_and_cells() {
        let f = people();
        let kept = pick_rows(&f, &[true, false, true]).expect("a mask per row");
        assert_eq!(kept.nrows(), 2);
        assert_eq!(column(&kept, "age").0, Column::Int(vec![30, 50]));
        assert_eq!(pick_rows(&f, &[true]).map(|_| ()), Err("a mask of 1 picks from a frame of 3 rows: they must be as long as each other".to_string()));
        let gappy = CrowFrame::new(vec!["n".to_string()], vec![Column::Int(vec![1, 0, 3])], vec![vec![true, false, true]]);
        let n = gappy.col_native("n").expect("the column");
        let picked = pick_cells(&n, &[false, true, true]).expect("a mask per cell");
        assert_eq!((*picked.data).clone(), Column::Int(vec![0, 3]));
        assert_eq!(picked.valid.to_vec(), vec![false, true]);
        assert_eq!(pick_cells(&n, &[true, true]).map(|_| ()), Err("a mask of 2 picks from a column of 3: they must be as long as each other".to_string()));
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
        ((*c.data).clone(), c.valid.to_vec())
    }

    /// A group's keys keep their kinds and come in the order first seen,
    /// one key or several; an aggregate named column_how is in the kind it
    /// has: a sum over ints an int, a count an int, a mean a float, a min
    /// or max a cell of the column's kind — missing where a group has no
    /// present cell — and a sum over text is refused.
    #[test]
    fn a_group_keeps_its_keys_kinds() {
        let f = trades();
        let sum = ["qty".to_string(), "sum".to_string()];
        let g = group(&f, "day", &[&sum]).expect("a group");
        assert_eq!(g.columns, vec!["day", "qty_sum"]);
        assert_eq!(column(&g, "day").0, Column::Int(vec![2, 1]));
        assert_eq!(column(&g, "qty_sum").0, Column::Int(vec![12, 51]));
        let g = group_keys(&f, &["sym".to_string(), "day".to_string()], &[&sum]).expect("a group");
        assert_eq!(g.columns, vec!["sym", "day", "qty_sum"]);
        assert_eq!(column(&g, "sym").0, Column::strs(vec!["b".into(), "a".into(), "b".into(), "a".into()]));
        assert_eq!(column(&g, "day").0, Column::Int(vec![2, 1, 1, 2]));
        assert_eq!(column(&g, "qty_sum").0, Column::Int(vec![10, 1, 50, 2]));
        let pair = |c: &str, how: &str| [c.to_string(), how.to_string()];
        let (count, mean, lo, hi, best) = (pair("qty", "count"), pair("px", "mean"), pair("px", "min"), pair("sym", "max"), pair("day", "max"));
        let g = group(&f, "sym", &[&count, &mean, &lo, &hi, &best]).expect("a group");
        assert_eq!(g.columns, vec!["sym", "qty_count", "px_mean", "px_min", "sym_max", "day_max"]);
        assert_eq!(column(&g, "qty_count"), (Column::Int(vec![3, 2]), vec![true, true]));
        assert_eq!(column(&g, "px_mean"), (Column::Float(vec![1.0, 3.0]), vec![true, true]));
        assert_eq!(column(&g, "px_min"), (Column::Float(vec![0.5, 2.0]), vec![true, true]));
        assert_eq!(column(&g, "sym_max"), (Column::strs(vec!["b".into(), "a".into()]), vec![true, true]));
        assert_eq!(column(&g, "day_max"), (Column::Int(vec![2, 2]), vec![true, true]));
        // a group with no present cell: its min, max and mean are missing,
        // its sum 0 and its count 0
        let holes = CrowFrame::new(vec!["k".into(), "x".into()], vec![Column::strs(vec!["a".into(), "b".into(), "b".into()]), Column::Int(vec![0, 4, 6])], vec![vec![true; 3], vec![false, true, true]]);
        let g = group(&holes, "k", &[&pair("x", "min"), &pair("x", "mean"), &pair("x", "sum"), &pair("x", "count")]).expect("a group");
        assert_eq!(column(&g, "x_min"), (Column::Int(vec![0, 4]), vec![false, true]));
        let (means, present) = column(&g, "x_mean");
        assert!(matches!(means, Column::Float(v) if v[0].is_nan() && v[1] == 5.0) && present == vec![false, true]);
        assert_eq!(column(&g, "x_sum"), (Column::Int(vec![0, 10]), vec![true, true]));
        assert_eq!(column(&g, "x_count"), (Column::Int(vec![0, 2]), vec![true, true]));
        assert_eq!(group(&f, "day", &[&pair("sym", "sum")]).map(|_| ()), Err("group: sum of column 'sym', which holds text".to_string()));
        assert_eq!(group(&f, "day", &[&pair("sym", "mean")]).map(|_| ()), Err("group: mean of column 'sym', which holds text".to_string()));
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
        let one_per_pair = pick_rows(&f, &[true, true, false, true, true]).expect("four rows");
        let p = pivot(&one_per_pair, "sym", "day", "qty").expect("a pivot");
        assert_eq!(p.columns, vec!["sym", "1", "2"]);
        assert_eq!(column(&p, "sym").0, Column::strs(vec!["a".into(), "b".into()]));
        assert_eq!(column(&p, "1"), (Column::Int(vec![1, 30]), vec![true, true]));
        assert_eq!(column(&p, "2"), (Column::Int(vec![2, 10]), vec![true, true]));
        let sparse = pick_rows(&f, &[true, true, false, false, true]).expect("three rows");
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
        let c = |r: Result<CrowColumn, String>| r.map(|c| ((*c.data).clone(), c.valid.to_vec()));
        assert_eq!(c(window(&f, "qty", "sum", 0, &by, 0.0, "")), Ok((Column::Int(vec![10, 1, 30, 3, 60]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "max", 2, &[], 0.0, "")), Ok((Column::Int(vec![10, 10, 20, 20, 30]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "mean", 2, &by, 0.0, "")), Ok((Column::Float(vec![10.0, 1.0, 15.0, 1.5, 25.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "px", "max", 0, &[], 0.0, "")), Ok((Column::Float(vec![1.5, 2.0, 2.0, 4.0, 4.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "px", "min", 2, &[], 0.0, "")), Ok((Column::Float(vec![1.5, 1.5, 0.5, 0.5, 1.0]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "count", 0, &by, 0.0, "")), Ok((Column::Int(vec![1, 1, 2, 2, 3]), vec![true; 5])));
        assert_eq!(c(window(&f, "qty", "lag", 1, &by, 0.0, "")), Ok((Column::Int(vec![0, 0, 10, 1, 20]), vec![false, false, true, true, true])));
        let (lead, present) = c(window(&f, "px", "lead", 1, &by, 0.0, "")).expect("a column");
        assert_eq!(present, vec![true, true, true, false, false]);
        assert!(matches!(lead, Column::Float(v) if v[..3] == [0.5, 4.0, 1.0] && v[3].is_nan() && v[4].is_nan()));
        assert_eq!(c(window(&f, "px", "rank", 0, &[], 0.0, "")), Ok((Column::Int(vec![3, 4, 1, 5, 2]), vec![true; 5])));
        let tied = with(&f, "t", Cells::from(vec![1_i64, 1, 2, 2, 3])).expect("five rows");
        assert_eq!(c(window(&tied, "t", "rank", 0, &[], 0.0, "")), Ok((Column::Int(vec![1, 1, 3, 3, 5]), vec![true; 5])));
        assert_eq!(c(window(&f, "sym", "rank", 0, &by, 0.0, "")), Ok((Column::Int(vec![1, 1, 1, 1, 1]), vec![true; 5])));
        assert_eq!(window(&f, "qty", "mediam", 0, &[], 0.0, "").map(|_| ()), Err(format!("window: no window 'mediam' — write {}, 'lag', 'lead' or 'rank'", AggFn::WORDS)));
        assert_eq!(window(&f, "qty", "lag", 0, &[], 0.0, "").map(|_| ()), Err("window: lag needs n of 1 or more, the rows between (got 0)".to_string()));
        assert_eq!(window(&f, "qty", "sum", -1, &[], 0.0, "").map(|_| ()), Err("window: n is 0 or more (got -1)".to_string()));
        assert!(window(&f, "sym", "sum", 0, &[], 0.0, "").is_err(), "a column of text has no sum");
        assert_eq!(window(&f, "qty", "sum", 0, &["nope".to_string()], 0.0, "").map(|_| ()), Err("window: column 'nope' not found".to_string()));
        // a missing key cell: a cell in no partition, missing
        let keyed = CrowFrame::new(vec!["k".into(), "x".into()], vec![Column::Int(vec![1, 1, 1]), Column::Float(vec![1.0, 2.0, 3.0])], vec![vec![true, false, true], vec![true; 3]]);
        assert_eq!(c(window(&keyed, "x", "sum", 0, &["k".to_string()], 0.0, "")), Ok((Column::Float(vec![1.0, 0.0, 4.0]), vec![true, false, true])));
    }

    /// A column copied with `with` keeps its missing cells (both modes had
    /// made them a guess: NaN, 0 or false).
    #[test]
    fn a_copied_column_keeps_its_missing_cells() {
        let f = CrowFrame::new(vec!["n".into()], vec![Column::Int(vec![1, 0])], vec![vec![true, false]]);
        let copied = with(&f, "m", Cells::from(f.col_native("n").expect("the column"))).expect("two rows");
        assert_eq!(copied.col_native("m").map(|c| c.valid.to_vec()), Some(vec![true, false]));
    }

    /// A sort is stable — equal cells keep their order — with a missing
    /// cell, or a float that is not a number, last whichever way it goes;
    /// text orders by character code.
    #[test]
    fn a_sort_is_stable_with_missing_cells_last() {
        let f = CrowFrame::new(
            vec!["k".into(), "x".into(), "s".into()],
            vec![Column::Int(vec![2, 1, 2, 1, 3]), Column::Float(vec![1.0, f64::NAN, 2.0, 3.0, 0.5]), Column::strs(vec!["b".into(), "a".into(), "B".into(), String::new(), "c".into()])],
            vec![vec![true, true, true, false, true], vec![true; 5], vec![true, true, true, false, true]],
        );
        let s = |g: &CrowFrame| column(g, "s");
        let text = |cells: &[&str]| Column::Str(cells.iter().map(|c| c.to_string()).collect());
        assert_eq!(s(&sort(&f, "k").expect("sorted")), (text(&["a", "b", "B", "c", ""]), vec![true, true, true, true, false]));
        assert_eq!(s(&sort_by(&f, "k", true).expect("sorted")), (text(&["c", "b", "B", "a", ""]), vec![true, true, true, true, false]));
        assert_eq!(s(&sort(&f, "x").expect("sorted")), (text(&["c", "b", "B", "", "a"]), vec![true, true, true, false, true]));
        assert_eq!(s(&sort_by(&f, "x", true).expect("sorted")), (text(&["", "B", "b", "c", "a"]), vec![false, true, true, true, true]));
        assert_eq!(column(&sort(&f, "s").expect("sorted"), "k"), (Column::Int(vec![2, 1, 2, 3, 1]), vec![true, true, true, true, false]));
    }

    /// `unique` answers a column's distinct present cells, first seen
    /// first, in the column's own kind.
    #[test]
    fn unique_keeps_the_columns_kind() {
        let f = trades();
        let cells = |c: CrowColumn| ((*c.data).clone(), c.valid.to_vec());
        assert_eq!(unique(&f, "day").map(cells), Ok((Column::Int(vec![2, 1]), vec![true, true])));
        assert_eq!(unique(&f, "sym").map(cells), Ok((Column::strs(vec!["b".into(), "a".into()]), vec![true, true])));
        let holes = CrowFrame::new(vec!["x".into()], vec![Column::Float(vec![1.5, 0.0, 1.5, 2.5])], vec![vec![true, false, true, true]]);
        assert_eq!(unique(&holes, "x").map(cells), Ok((Column::Float(vec![1.5, 2.5]), vec![true, true])));
        assert_eq!(unique(&f, "nope").map(|_| ()), Err("unique: column 'nope' not found".to_string()));
    }
}

/// The table with no rows yet, in the columns a program reads: what a
/// page's data source is before its answer arrives, in both modes, and
/// what an answer of no records is.
pub fn empty_with(names: &[String], kinds: &[String]) -> CrowFrame {
    let data: Vec<crate::frame::Column> = kinds
        .iter()
        .map(|k| match k.as_str() {
            "int" => crate::frame::Column::Int(Vec::new()),
            "float" => crate::frame::Column::Float(Vec::new()),
            "bool" => crate::frame::Column::Bool(Vec::new()),
            "moment" => crate::frame::Column::DateTime(Vec::new(), Vec::new()),
            _ => crate::frame::Column::strs(Vec::new()),
        })
        .collect();
    CrowFrame::new(names.to_vec(), data, vec![Vec::new(); names.len()])
}

/// A frame held to the columns a program reads (the lowering's
/// `__expect_frame`; spec, Frames: a frame's type is its columns): each
/// named column is there and holds the kind the program reads it as —
/// text, ints, numbers, bools or moments — or the program stops with the
/// column, what it holds, what the program reads and the read that asked
/// (`hows`, one per column). A column of whole numbers the program reads
/// as numbers is answered as floats: the program compares, adds and
/// divides numbers, and which kind the file wrote is the file's business,
/// so `q gt 20` holds for 20.5 and `q / 2` of 7 is 3.5 either way. A whole
/// number a float cannot hold exactly (beyond 2^53) stops the load rather
/// than lose digits.
pub fn expect(f: &CrowFrame, names: &[String], kinds: &[String], hows: &[String]) -> Result<CrowFrame, String> {
    fn reads(kind: &str) -> &str {
        match kind {
            "str" => "text",
            "int" => "ints",
            "float" => "numbers",
            "bool" => "bools",
            "moment" => "moments",
            other => other,
        }
    }
    fn holds(dtype: &str) -> &str {
        match dtype {
            "str" => "text",
            "int" => "ints",
            "float" => "floats",
            "bool" => "bools",
            "datetime" => "moments",
            other => other,
        }
    }
    // a frame of no rows and no columns — a source that answered `[]` — is
    // the table with no rows yet: it is given the columns it is held to
    if f.columns.is_empty() && f.nrows() == 0 && !names.is_empty() {
        return Ok(empty_with(names, kinds));
    }
    let mut out = f.clone();
    for (i, (name, kind)) in names.iter().zip(kinds).enumerate() {
        let how = hows.get(i).filter(|h| !h.is_empty()).map(|h| format!(" ({})", h)).unwrap_or_default();
        let Some(col) = f.col_native(name) else {
            return Err(format!("the frame has no column '{}', which the program reads as {}{} — its columns are {}", name, reads(kind), how, f.columns.join(", ")));
        };
        let dtype = col.data.dtype_name();
        if kind == "float" && dtype == "int" {
            if let crate::frame::Column::Int(v) = col.data.as_ref() {
                const EXACT: u64 = 1 << 53;
                let present = |i: usize| col.valid.get(i).copied().unwrap_or(true);
                if let Some(x) = v.iter().enumerate().find(|(i, x)| present(*i) && x.unsigned_abs() > EXACT).map(|(_, x)| *x) {
                    return Err(format!("column '{}' holds {}, a whole number a float cannot hold exactly, where the program reads numbers{} — read it with int[…] where it is compared", name, x, how));
                }
                let floats: Vec<f64> = v.iter().map(|x| *x as f64).collect();
                out = out.with_validity(name, crate::frame::Column::Float(floats), col.valid.as_ref().clone())?;
            }
            continue;
        }
        let fits = match kind.as_str() {
            "str" => dtype == "str",
            "int" => dtype == "int",
            "float" => dtype == "float",
            "bool" => dtype == "bool",
            "moment" => dtype == "datetime",
            _ => true,
        };
        if !fits {
            return Err(format!("column '{}' holds {} where the program reads {}{}", name, holds(dtype), reads(kind), how));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod expect_tests {
    use super::*;
    use crate::frame::Column;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A column of whole numbers the program reads as numbers is widened
    /// to floats at the load, its missing cells kept; one it reads as ints
    /// is answered as it is; one of floats where ints are asked stops with
    /// the read that asked; a whole number beyond 2^53 stops rather than
    /// lose digits.
    #[test]
    fn a_column_of_whole_numbers_read_as_numbers_is_floats_exactly() {
        let f = CrowFrame::new(
            strs(&["qty", "px", "id"]),
            vec![Column::Int(vec![7, 0, 50]), Column::Float(vec![1.5, 2.5, 3.5]), Column::Int(vec![1, (1 << 53) + 1, 2])],
            vec![vec![true, false, true], vec![true, true, true], vec![true, true, true]],
        );
        let held = expect(&f, &strs(&["qty"]), &strs(&["float"]), &strs(&["met in gt at line 9"])).expect("widened");
        let qty = held.col_native("qty").expect("qty");
        assert!(matches!(qty.data.as_ref(), Column::Float(v) if v == &vec![7.0, 0.0, 50.0]), "{:?}", qty.data.dtype_name());
        assert_eq!(qty.valid.to_vec(), vec![true, false, true]);
        assert_eq!(held.columns, strs(&["qty", "px", "id"]), "the columns keep their order");
        let as_ints = expect(&f, &strs(&["qty"]), &strs(&["int"]), &strs(&["a position in names at line 3"])).expect("ints where ints are asked");
        assert!(matches!(as_ints.col_native("qty").expect("qty").data.as_ref(), Column::Int(_)));
        let err = expect(&f, &strs(&["px"]), &strs(&["int"]), &strs(&["a position in names at line 3"])).expect_err("floats where ints are asked");
        assert_eq!(err, "column 'px' holds floats where the program reads ints (a position in names at line 3)");
        let err = expect(&f, &strs(&["id"]), &strs(&["float"]), &strs(&["met in eq at line 4"])).expect_err("beyond 2^53");
        assert_eq!(err, "column 'id' holds 9007199254740993, a whole number a float cannot hold exactly, where the program reads numbers (met in eq at line 4) — read it with int[…] where it is compared");
        let err = expect(&f, &strs(&["sym"]), &strs(&["str"]), &strs(&["handed to upper at line 2"])).expect_err("missing");
        assert_eq!(err, "the frame has no column 'sym', which the program reads as text (handed to upper at line 2) — its columns are qty, px, id");
        // no rows and no columns: the table with no rows yet, in the shape held
        let none = CrowFrame::new(Vec::new(), Vec::new(), Vec::new());
        let held = expect(&none, &strs(&["sym", "qty"]), &strs(&["str", "float"]), &strs(&["", ""])).expect("empty, shaped");
        assert_eq!(held.columns, strs(&["sym", "qty"]));
        assert_eq!(held.nrows(), 0);
        assert!(matches!(held.col_native("qty").expect("qty").data.as_ref(), Column::Float(v) if v.is_empty()));
    }
}

// ── batch files: the log's unit (crate::batch) ──────────────────────────

/// `sys.batch_write[dir, number, first, frame, keys, texts]` where the
/// program was built with Arrow (the interpreter always is; a compiled
/// program that writes batches).
pub fn batch_write(dir: &str, number: i64, first: i64, f: &CrowFrame, keys: &[String], texts: &[String], flush: bool) -> Result<String, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::write(dir, number, first, f, keys, texts, flush)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = (dir, number, first, f, keys, texts, flush);
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_read[path]`, likewise.
pub fn batch_read(path: &str, wanted: &[String]) -> Result<CrowFrame, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::read(path, wanted)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = (path, wanted);
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_stats[path]`, likewise.
pub fn batch_stats(path: &str) -> Result<String, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::stats(path)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = path;
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_admits[path, column, value, token]`, likewise.
pub fn batch_admits(path: &str, column: &str, value: &str, token: bool) -> Result<bool, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::admits(path, column, value, token)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = (path, column, value, token);
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_merge[dir, paths, keys]`, where the program was built with
/// Parquet (the interpreter always is; a compiled program that merges).
pub fn batch_merge(dir: &str, paths: &[String], keys: &[String], every: bool) -> Result<String, String> {
    #[cfg(feature = "parquet")]
    {
        crate::batch::merge(dir, paths, keys, every)
    }
    #[cfg(not(feature = "parquet"))]
    {
        let _ = (dir, paths, keys, every);
        Err("batch: this program was built without Parquet".to_string())
    }
}

/// `sys.batch_search[paths, column, words, k]`, likewise.
pub fn batch_search(paths: &[String], column: &str, words: &[String], k: i64) -> Result<CrowFrame, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::search(paths, column, words, k)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = (paths, column, words, k);
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_since[paths, after, wanted]`, likewise.
pub fn batch_since(paths: &[String], after: i64, wanted: &[String]) -> Result<CrowFrame, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::since(paths, after, wanted)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = (paths, after, wanted);
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_sync[dir]`, likewise.
pub fn batch_sync(dir: &str) -> Result<i64, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::sync(dir)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = dir;
        Err("batch: this program was built without Arrow".to_string())
    }
}

/// `sys.batch_find[path, columns, values, wanted]`, likewise.
pub fn batch_find(paths: &[String], columns: &[String], values: &[String], wanted: &[String]) -> Result<CrowFrame, String> {
    #[cfg(feature = "arrow")]
    {
        crate::batch::find_in(paths, columns, values, wanted)
    }
    #[cfg(not(feature = "arrow"))]
    {
        let _ = (paths, columns, values, wanted);
        Err("batch: this program was built without Arrow".to_string())
    }
}
