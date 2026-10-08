/// Crow frame module runtime — column-oriented data frames with typed vectors.

use std::fmt;
use std::sync::Arc;
use rayon::prelude::*;

pub use crate::textcol::Text;
pub use crate::validity::Validity;

/// Column data types — each column stores a single typed vector.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Column {
    Int(Vec<i64>),
    Float(Vec<f64>),
    /// Text in one buffer, or codes into a dictionary (text.rs).
    Str(Text),
    Bool(Vec<bool>),
    /// DateTime: UTC epoch milliseconds + timezone offset in minutes
    DateTime(Vec<i64>, Vec<i16>),
}

impl Column {
    /// A column of text from strings, held in one buffer.
    pub fn strs(v: Vec<String>) -> Column {
        Column::Str(Text::from(v))
    }

    /// A cell read as a condition: non-zero numbers, `true`, and the strings
    /// `true`/`1`/`t` are true.
    pub fn truthy(&self, idx: usize) -> bool {
        match self {
            Column::Bool(v) => v[idx],
            Column::Int(v) => v[idx] != 0,
            Column::Float(v) => v[idx] != 0.0,
            Column::Str(v) => matches!(v[idx].trim(), "true" | "1" | "t"),
            Column::DateTime(ms, _) => ms[idx] != 0,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Column::Int(v) => v.len(),
            Column::Float(v) => v.len(),
            Column::Str(v) => v.len(),
            Column::Bool(v) => v.len(),
            Column::DateTime(v, _) => v.len(),
        }
    }

    pub fn dtype_name(&self) -> &'static str {
        match self {
            Column::Int(_) => "int",
            Column::Float(_) => "float",
            Column::Str(_) => "str",
            Column::Bool(_) => "bool",
            Column::DateTime(_, _) => "datetime",
        }
    }

    /// Format a single cell value as string (for display)
    pub fn fmt_cell(&self, row: usize, valid: bool) -> String {
        if !valid {
            return "nan".to_string();
        }
        match self {
            Column::Int(v) => format!("{}", v[row]),
            Column::Float(v) => {
                let val = v[row];
                if val.is_nan() { "nan".to_string() }
                else if val == (val as i64) as f64 && val.is_finite() {
                    format!("{}", val as i64)
                } else {
                    format!("{}", val)
                }
            }
            Column::Str(v) => v[row].to_string(),
            Column::Bool(v) => format!("{}", v[row]),
            Column::DateTime(millis, offsets) => format_datetime(millis[row], offsets[row]),
        }
    }
}

/// A column-oriented data frame. Two frames are equal when their column
/// names, cells and missing cells are.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CrowFrame {
    pub columns: Vec<String>,              // column names (order preserved)
    pub data: Vec<Arc<Column>>,            // column data (Arc for cheap sharing)
    pub validity: Vec<Arc<Validity>>,      // which cells are there, a bit per cell
}

impl CrowFrame {
    /// A new frame with one more (or one replaced) column. Every row is
    /// valid; the length must match the frame's rows unless the frame is
    /// empty, in which case the column sets the row count.
    pub fn with_column(&self, name: &str, data: Column) -> Result<CrowFrame, String> {
        let n = data.len();
        self.with_valid(name, data, vec![true; n])
    }

    /// `with_column` with each row's presence given: a column copied from
    /// another frame keeps its missing cells.
    pub fn with_valid(&self, name: &str, data: Column, valid: Vec<bool>) -> Result<CrowFrame, String> {
        self.with_validity(name, data, Validity::from(valid))
    }

    /// A column added or replaced, with which of its cells are there.
    pub fn with_validity(&self, name: &str, data: Column, valid: Validity) -> Result<CrowFrame, String> {
        let n = data.len();
        if !self.columns.is_empty() && n != self.nrows() {
            return Err(format!("with: column '{}' has {} values but the frame has {} rows", name, n, self.nrows()));
        }
        if valid.len() != n {
            return Err(format!("with: column '{}' has {} values but {} rows of presence", name, n, valid.len()));
        }
        let mut out = self.clone();
        let validity = Arc::new(valid);
        if let Some(i) = out.columns.iter().position(|c| c == name) {
            out.data[i] = Arc::new(data);
            out.validity[i] = validity;
        } else {
            out.columns.push(name.to_string());
            out.data.push(Arc::new(data));
            out.validity.push(validity);
        }
        Ok(out)
    }

    /// `with_column` from a scalar repeated for every row.
    pub fn with_scalar(&self, name: &str, data: Column) -> Result<CrowFrame, String> {
        let n = self.nrows();
        let repeated = match data {
            Column::Int(v) => Column::Int(vec![v[0]; n]),
            Column::Float(v) => Column::Float(vec![v[0]; n]),
            Column::Str(v) => Column::Str(Text::repeated(&v[0], n)),
            Column::Bool(v) => Column::Bool(vec![v[0]; n]),
            Column::DateTime(v, z) => Column::DateTime(vec![v[0]; n], vec![z[0]; n]),
        };
        self.with_column(name, repeated)
    }

    /// Create from raw (unshared) columns — wraps each in Arc.
    pub fn new(columns: Vec<String>, data: Vec<Column>, validity: Vec<Vec<bool>>) -> Self {
        CrowFrame {
            columns,
            data: data.into_iter().map(Arc::new).collect(),
            validity: validity.into_iter().map(|v| Arc::new(Validity::from(v))).collect(),
        }
    }

    /// A frame from columns and their bitmaps.
    pub fn assembled(columns: Vec<String>, data: Vec<Column>, validity: Vec<Validity>) -> Self {
        CrowFrame { columns, data: data.into_iter().map(Arc::new).collect(), validity: validity.into_iter().map(Arc::new).collect() }
    }

    pub fn nrows(&self) -> usize {
        if self.data.is_empty() { 0 } else { self.data[0].len() }
    }

    pub fn ncols(&self) -> usize {
        self.columns.len()
    }

    /// Get column index by name
    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// Get column reference by name
    pub fn col(&self, name: &str) -> Option<&Column> {
        self.col_index(name).map(|i| self.data[i].as_ref())
    }

    /// Get validity bitmap for a column by name
    pub fn col_validity(&self, name: &str) -> Option<&Validity> {
        self.col_index(name).map(|i| self.validity[i].as_ref())
    }

    /// Get column dtypes as strings
    pub fn dtypes(&self) -> Vec<&'static str> {
        self.data.iter().map(|c| c.dtype_name()).collect()
    }

    /// Get column values as formatted strings (column access in generated code).
    /// Missing values are represented as "nan".
    pub fn col_as_strings(&self, name: &str) -> Option<Vec<String>> {
        let idx = self.col_index(name)?;
        let col = &self.data[idx];
        let valid = &self.validity[idx];
        let nrows = self.nrows();
        let mut result = Vec::with_capacity(nrows);
        for i in 0..nrows {
            result.push(col.fmt_cell(i, valid[i]));
        }
        Some(result)
    }

    /// Get a CrowColumn wrapper for a named column (native typed access).
    pub fn col_native(&self, name: &str) -> Option<CrowColumn> {
        let idx = self.col_index(name)?;
        Some(CrowColumn {
            data: Arc::clone(&self.data[idx]),
            valid: Arc::clone(&self.validity[idx]),
        })
    }
}

// ===== CrowColumn: typed column wrapper for zero-parse operations =====

/// A typed column wrapper that avoids string conversion for comparisons/arithmetic.
#[derive(Debug, Clone, PartialEq)]
pub struct CrowColumn {
    pub data: Arc<Column>,
    pub valid: Arc<Validity>,
}

impl CrowColumn {
    pub fn len(&self) -> usize { self.data.len() }

    /// Numeric view of the column: floats pass through, ints cast,
    /// numeric strings parse (stringly-typed backends), everything else
    /// (and invalid rows) becomes NaN — which charts render as gaps.
    pub fn to_f64_vec(&self) -> Vec<f64> {
        let n = self.data.len();
        (0..n)
            .map(|i| {
                if !self.valid.get(i).copied().unwrap_or(false) {
                    return f64::NAN;
                }
                match self.data.as_ref() {
                    Column::Float(v) => v[i],
                    Column::Int(v) => v[i] as f64,
                    Column::Str(v) => v[i].trim().parse::<f64>().unwrap_or(f64::NAN),
                    Column::Bool(v) => {
                        if v[i] {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    Column::DateTime(ms, _) => ms[i] as f64 / 1000.0,
                }
            })
            .collect()
    }

    /// A cell as a moment: a moment column's instant and offset; a
    /// missing cell, or another column's, the epoch.
    pub fn get_moment(&self, idx: usize) -> crate::moments::Moment {
        if !self.valid.get(idx).copied().unwrap_or(false) { return crate::moments::Moment::default(); }
        match self.data.as_ref() {
            Column::DateTime(ms, offsets) => crate::moments::Moment { ms: ms[idx], offset: offsets[idx] },
            Column::Int(v) => crate::moments::Moment::of_seconds(v[idx] as f64),
            Column::Float(v) => crate::moments::Moment::of_seconds(v[idx]),
            Column::Str(v) => crate::moments::Moment::parse(&v[idx]).unwrap_or_default(),
            Column::Bool(_) => crate::moments::Moment::default(),
        }
    }

    /// Get a single cell as a formatted string.
    pub fn get_str(&self, idx: usize) -> String {
        self.data.fmt_cell(idx, self.valid[idx])
    }

    /// A cell as a float: ints cast, numeric strings parse, moments are
    /// seconds; a missing or unparsable cell is NaN.
    pub fn get_f64(&self, idx: usize) -> f64 {
        if !self.valid.get(idx).copied().unwrap_or(false) { return f64::NAN; }
        match self.data.as_ref() {
            Column::Float(v) => v[idx],
            Column::Int(v) => v[idx] as f64,
            Column::Str(v) => v[idx].trim().parse::<f64>().unwrap_or(f64::NAN),
            Column::Bool(v) => if v[idx] { 1.0 } else { 0.0 },
            Column::DateTime(ms, _) => ms[idx] as f64 / 1000.0,
        }
    }

    /// A cell as an int: floats truncate, numeric strings parse, moments
    /// are whole seconds; a missing or unparsable cell is 0.
    pub fn get_i64(&self, idx: usize) -> i64 {
        if !self.valid.get(idx).copied().unwrap_or(false) { return 0; }
        match self.data.as_ref() {
            Column::Int(v) => v[idx],
            Column::Float(v) => v[idx] as i64,
            Column::Str(v) => { let s = v[idx].trim(); s.parse::<i64>().ok().or_else(|| s.parse::<f64>().ok().map(|f| f as i64)).unwrap_or(0) }
            Column::Bool(v) => if v[idx] { 1 } else { 0 },
            Column::DateTime(ms, _) => ms[idx] / 1000,
        }
    }

    /// A cell as the dynamic value it is, for a column whose type nothing
    /// told the compiler: a number, text or a flag; a moment in seconds; a
    /// missing cell (or a float that is not finite) null.
    pub fn get_json(&self, idx: usize) -> serde_json::Value {
        if !self.valid.get(idx).copied().unwrap_or(false) {
            return serde_json::Value::Null;
        }
        match self.data.as_ref() {
            Column::Int(v) => serde_json::json!(v[idx]),
            Column::Float(v) => crate::json::float_to_json(v[idx]),
            Column::Str(v) => serde_json::Value::String(v[idx].to_string()),
            Column::Bool(v) => serde_json::Value::Bool(v[idx]),
            // (the interpreter reads the cell as its text; compiled, it had
            // been seconds, so `str` of one differed between the modes)
            Column::DateTime(ms, offsets) => serde_json::Value::String(format_datetime(ms[idx], offsets[idx])),
        }
    }

    /// A cell as a bool: numbers are true when non-zero, strings when
    /// `true` or `1`; a missing cell is false.
    pub fn get_bool(&self, idx: usize) -> bool {
        if !self.valid.get(idx).copied().unwrap_or(false) { return false; }
        match self.data.as_ref() {
            Column::Bool(v) => v[idx],
            Column::Int(v) => v[idx] != 0,
            Column::Float(v) => v[idx] != 0.0,
            Column::Str(v) => matches!(v[idx].trim(), "true" | "1" | "t"),
            Column::DateTime(ms, _) => ms[idx] != 0,
        }
    }

    /// Convert to Vec<String> (for display, interpolation, etc.)
    pub fn to_strings(&self) -> Vec<String> {
        let n = self.len();
        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            result.push(self.data.fmt_cell(i, self.valid[i]));
        }
        result
    }

    /// Sum of numeric column values (skips nulls). Returns 0.0 for non-numeric or empty columns.
    pub fn sum(&self) -> f64 {
        let n = self.len();
        let mut total = 0.0;
        match self.data.as_ref() {
            Column::Int(v) => {
                for i in 0..n {
                    if self.valid[i] { total += v[i] as f64; }
                }
            }
            Column::Float(v) => {
                for i in 0..n {
                    if self.valid[i] && !v[i].is_nan() { total += v[i]; }
                }
            }
            _ => {}
        }
        total
    }

    /// The column's numbers where a builtin takes numbers (docs/
    /// bug-classes.md §4): its ints and floats, or the reason it has none —
    /// a missing cell, a cell of text or a bool — found row by row, as the
    /// interpreter reads the column's cells. (Compiled, a missing cell was
    /// read as 0 where the interpreter stopped.)
    pub fn numbers(&self) -> Result<crate::tensor::CrowTensor, String> {
        let n = self.len();
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            if !self.valid.get(i).copied().unwrap_or(false) {
                return Err(crate::stops::missing_cell(i));
            }
            data.push(match self.data.as_ref() {
                Column::Int(v) => v[i] as f64,
                Column::Float(v) => v[i],
                Column::Bool(_) => return Err(crate::stops::not_numbers("bools")),
                Column::Str(_) | Column::DateTime(..) => return Err(crate::stops::not_numbers("text")),
            });
        }
        Ok(crate::tensor::CrowTensor::from_vec(data, vec![n]))
    }

    /// Numeric values as a tensor (invalid cells become 0.0) — the bridge
    /// from named columns into tensor math (mean, element-wise ops, ...).
    pub fn to_tensor(&self) -> crate::tensor::CrowTensor {
        let n = self.len();
        let mut data = Vec::with_capacity(n);
        match self.data.as_ref() {
            Column::Int(v) => {
                for i in 0..n {
                    data.push(if self.valid[i] { v[i] as f64 } else { 0.0 });
                }
            }
            Column::Float(v) => {
                for i in 0..n {
                    data.push(if self.valid[i] { v[i] } else { 0.0 });
                }
            }
            Column::Bool(v) => {
                for i in 0..n {
                    data.push(if self.valid[i] && v[i] { 1.0 } else { 0.0 });
                }
            }
            _ => {
                // Str / DateTime: no numeric interpretation
                for _ in 0..n {
                    data.push(0.0);
                }
            }
        }
        crate::tensor::CrowTensor::from_vec(data, vec![n])
    }

    pub fn unique(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        let n = self.len();
        match self.data.as_ref() {
            Column::Str(v) => {
                for i in 0..n {
                    if !self.valid[i] { continue; }
                    if seen.insert(v[i].to_string()) {
                        result.push(v[i].to_string());
                    }
                }
            }
            Column::Int(v) => {
                for i in 0..n {
                    if !self.valid[i] { continue; }
                    let s = v[i].to_string();
                    if seen.insert(s.clone()) {
                        result.push(s);
                    }
                }
            }
            Column::Float(v) => {
                for i in 0..n {
                    if !self.valid[i] || v[i].is_nan() { continue; }
                    let s = v[i].to_string();
                    if seen.insert(s.clone()) {
                        result.push(s);
                    }
                }
            }
            _ => {}
        }
        result
    }

    /// Validity bitmap — true for each non-missing value.
    pub fn ok(&self) -> Vec<bool> {
        self.valid.to_vec()
    }
}

impl fmt::Display for CrowColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let strings = self.to_strings();
        write!(f, "[{}]", strings.join(", "))
    }
}

// ===== CSV Loading =====

/// Load a CSV file into a CrowFrame with auto type inference.
pub fn load_csv(path: &str) -> Result<CrowFrame, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("load: {}", e))?;
    csv_frame(&content)
}

/// CSV text as a frame, each column's type read from its cells: a file's,
/// or what a URL answered.
pub fn csv_frame(content: &str) -> Result<CrowFrame, String> {
    // the header: the column names
    let newline_pos = content.find('\n').ok_or_else(|| "load: empty file".to_string())?;
    let header = content[..newline_pos].trim();
    let col_names: Vec<String> = if header.contains('"') { split_quoted(header) } else { header.split(',').map(|s| s.trim().to_string()).collect() };
    let ncols = col_names.len();
    if ncols == 0 {
        return Err("load: no columns found".to_string());
    }
    let body = &content[newline_pos + 1..];
    // a file with a quoted cell is split by the quoting rules, line by
    // line; one without takes the byte search
    let quoted = memchr::memchr(b'"', body.as_bytes()).is_some();

    // the kinds, from a sample of the first rows: ints, floats, bools,
    // moments, or text — held by a dictionary where the sample shows few
    // distinct values (a symbol, a side)
    let mut sample: Vec<Vec<String>> = Vec::new();
    for line in lines_of(body).take(SAMPLE_ROWS) {
        sample.push(fields_of(line, quoted, ncols));
    }
    let kinds: Vec<Kind> = (0..ncols)
        .map(|ci| {
            let present: Vec<&str> = sample.iter().map(|f| f[ci].as_str()).filter(|s| !s.is_empty()).collect();
            kind_of(&present)
        })
        .collect();

    // the body cut into pieces at line ends, one per core and more, each
    // parsed on its own into parts of the columns
    let pieces = pieces_of(body);
    let parts: Vec<Part> = pieces.par_iter().map(|piece| parse_piece(piece, quoted, &kinds)).collect();
    let nrows: usize = parts.iter().map(|p| p.rows).sum();

    // the parts joined into columns; a column a part could not read in
    // its sampled kind is read again, whole, as the kind every cell has
    let mut columns: Vec<Column> = Vec::with_capacity(ncols);
    let mut validity: Vec<Vec<bool>> = Vec::with_capacity(ncols);
    for ci in 0..ncols {
        if parts.iter().any(|p| p.failed[ci]) {
            let (col, valid) = column_read_whole(body, quoted, ci, nrows);
            columns.push(col);
            validity.push(valid);
            continue;
        }
        let mut valid = Vec::with_capacity(nrows);
        for p in &parts {
            valid.extend_from_slice(&p.valid[ci]);
        }
        validity.push(valid);
        columns.push(join_parts(parts.iter().map(|p| &p.cols[ci]).collect(), nrows));
    }
    Ok(CrowFrame::new(col_names, columns, validity))
}

/// How many rows decide the columns' kinds.
const SAMPLE_ROWS: usize = 100;

/// A column's kind, as the sample of its present cells decides it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Int,
    Float,
    Bool,
    DateTime,
    Text,
    /// Text of few distinct values: a dictionary, a code per cell.
    Dict,
}

fn kind_of(present: &[&str]) -> Kind {
    if present.is_empty() {
        return Kind::Text;
    }
    if present.iter().all(|s| s.parse::<i64>().is_ok()) {
        return Kind::Int;
    }
    if present.iter().all(|s| s.parse::<f64>().is_ok()) {
        return Kind::Float;
    }
    if present.iter().all(|s| bool_text(s).is_some()) {
        return Kind::Bool;
    }
    if present.iter().all(|s| parse_datetime(s).is_some()) {
        return Kind::DateTime;
    }
    let distinct: std::collections::HashSet<&str> = present.iter().copied().collect();
    if distinct.len() * 2 <= present.len() {
        Kind::Dict
    } else {
        Kind::Text
    }
}

/// The body's lines, each without its line break (a `\r` before it too)
/// and with empty lines left out.
fn lines_of(body: &str) -> impl Iterator<Item = &str> {
    let bytes = body.as_bytes();
    let mut start = 0;
    memchr::memchr_iter(b'\n', bytes)
        .map(Some)
        .chain(std::iter::once(None))
        .filter_map(move |nl| {
            let end = nl.unwrap_or(bytes.len());
            let line = &body[start..end];
            start = end + 1;
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.trim().is_empty() {
                None
            } else {
                Some(line)
            }
        })
}

/// A line's fields, `ncols` of them (a short line's rest empty, a long
/// line's extra fields dropped), each trimmed.
fn fields_of(line: &str, quoted: bool, ncols: usize) -> Vec<String> {
    let mut fields: Vec<String> = if quoted && line.contains('"') {
        split_quoted(line)
    } else {
        line.split(',').map(|f| f.trim().to_string()).collect()
    };
    fields.resize(ncols, String::new());
    fields
}

/// The body in pieces, each ending at a line end, one per core and more
/// for a large file.
fn pieces_of(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let want = (rayon::current_num_threads() * 4).max(1);
    let size = (bytes.len() / want).max(1 << 20);
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let nominal = (start + size).min(bytes.len());
        let end = if nominal >= bytes.len() { bytes.len() } else { memchr::memchr(b'\n', &bytes[nominal..]).map_or(bytes.len(), |p| nominal + p + 1) };
        pieces.push(&body[start..end]);
        start = end;
    }
    pieces
}

/// One piece's rows: a part of each column, the presence of each cell,
/// and which columns held a cell the sampled kind could not read.
struct Part {
    rows: usize,
    cols: Vec<ColPart>,
    valid: Vec<Vec<bool>>,
    failed: Vec<bool>,
}

enum ColPart {
    Int(Vec<i64>),
    Float(Vec<f64>),
    Bool(Vec<bool>),
    DateTime(Vec<i64>, Vec<i16>),
    Text(Text),
    Dict(crate::textcol::DictBuilder),
}

impl ColPart {
    fn of(kind: Kind, cells: usize) -> ColPart {
        match kind {
            Kind::Int => ColPart::Int(Vec::with_capacity(cells)),
            Kind::Float => ColPart::Float(Vec::with_capacity(cells)),
            Kind::Bool => ColPart::Bool(Vec::with_capacity(cells)),
            Kind::DateTime => ColPart::DateTime(Vec::with_capacity(cells), Vec::with_capacity(cells)),
            Kind::Text => ColPart::Text(Text::with_capacity(cells, cells * 8)),
            Kind::Dict => ColPart::Dict(crate::textcol::DictBuilder::with_capacity(cells)),
        }
    }

    /// A cell pushed as the part's kind; whether it could be read as one
    /// (an empty cell is a missing one, pushed as the kind's placeholder).
    fn push(&mut self, s: &str) -> bool {
        match self {
            ColPart::Int(v) => match s.parse::<i64>() {
                Ok(n) => v.push(n),
                Err(_) => {
                    v.push(0);
                    return s.is_empty();
                }
            },
            ColPart::Float(v) => match s.parse::<f64>() {
                Ok(x) => v.push(x),
                Err(_) => {
                    v.push(f64::NAN);
                    return s.is_empty();
                }
            },
            ColPart::Bool(v) => match bool_text(s) {
                Some(b) => v.push(b),
                None => {
                    v.push(false);
                    return s.is_empty();
                }
            },
            ColPart::DateTime(ms, off) => match parse_datetime(s) {
                Some((m, o)) => {
                    ms.push(m);
                    off.push(o);
                }
                None => {
                    ms.push(0);
                    off.push(0);
                    return s.is_empty();
                }
            },
            ColPart::Text(t) => t.push(s),
            ColPart::Dict(b) => {
                b.push(s);
                if b.over() {
                    // past the dictionary's limit the part is held plain
                    let plain = std::mem::replace(b, crate::textcol::DictBuilder::with_capacity(0)).finish();
                    *self = ColPart::Text(plain);
                }
            }
        }
        true
    }
}

/// A piece parsed: each line split once, each field pushed into its
/// column's part.
fn parse_piece(piece: &str, quoted: bool, kinds: &[Kind]) -> Part {
    let ncols = kinds.len();
    // the rows there are, counted first, so each part is allocated once
    let guess = memchr::memchr_iter(b'\n', piece.as_bytes()).count() + 1;
    let mut part = Part {
        rows: 0,
        cols: kinds.iter().map(|&k| ColPart::of(k, guess)).collect(),
        valid: (0..ncols).map(|_| Vec::with_capacity(guess)).collect(),
        failed: vec![false; ncols],
    };
    let push = |part: &mut Part, ci: usize, field: &str| {
        let present = !field.is_empty();
        part.valid[ci].push(present);
        if !part.cols[ci].push(field) {
            part.failed[ci] = true;
        }
    };
    for line in lines_of(piece) {
        part.rows += 1;
        if quoted && line.contains('"') {
            let fields = split_quoted(line);
            for ci in 0..ncols {
                push(&mut part, ci, fields.get(ci).map_or("", String::as_str));
            }
            continue;
        }
        let bytes = line.as_bytes();
        let mut start = 0;
        let mut ci = 0;
        for comma in memchr::memchr_iter(b',', bytes).chain(std::iter::once(bytes.len())) {
            if ci < ncols {
                push(&mut part, ci, line[start..comma].trim());
            }
            ci += 1;
            start = comma + 1;
        }
        while ci < ncols {
            push(&mut part, ci, "");
            ci += 1;
        }
    }
    part
}

/// A column's parts joined, in order.
fn join_parts(parts: Vec<&ColPart>, nrows: usize) -> Column {
    match parts.first() {
        Some(ColPart::Int(_)) => {
            let mut v = Vec::with_capacity(nrows);
            for p in parts {
                if let ColPart::Int(x) = p {
                    v.extend_from_slice(x);
                }
            }
            Column::Int(v)
        }
        Some(ColPart::Float(_)) => {
            let mut v = Vec::with_capacity(nrows);
            for p in parts {
                if let ColPart::Float(x) = p {
                    v.extend_from_slice(x);
                }
            }
            Column::Float(v)
        }
        Some(ColPart::Bool(_)) => {
            let mut v = Vec::with_capacity(nrows);
            for p in parts {
                if let ColPart::Bool(x) = p {
                    v.extend_from_slice(x);
                }
            }
            Column::Bool(v)
        }
        Some(ColPart::DateTime(..)) => {
            let (mut ms, mut off) = (Vec::with_capacity(nrows), Vec::with_capacity(nrows));
            for p in parts {
                if let ColPart::DateTime(m, o) = p {
                    ms.extend_from_slice(m);
                    off.extend_from_slice(o);
                }
            }
            Column::DateTime(ms, off)
        }
        Some(ColPart::Text(_)) | Some(ColPart::Dict(_)) => {
            // every part coded: one dictionary over them all, each part's
            // codes renumbered; a part held plain makes the column plain
            let texts: Vec<Text> = parts
                .iter()
                .map(|p| match p {
                    ColPart::Dict(b) => b.snapshot(),
                    ColPart::Text(t) => t.clone(),
                    _ => Text::new(),
                })
                .collect();
            Column::Str(Text::joined(&texts))
        }
        None => Column::Str(Text::new()),
    }
}

/// One column read whole, every cell's text kept until its kind is known
/// (the slow way, for a column whose sample misled: an int column with
/// `n/a` far down is text).
fn column_read_whole(body: &str, quoted: bool, ci: usize, nrows: usize) -> (Column, Vec<bool>) {
    let mut cells: Vec<String> = Vec::with_capacity(nrows);
    for line in lines_of(body) {
        let field = if quoted && line.contains('"') {
            split_quoted(line).into_iter().nth(ci).unwrap_or_default()
        } else {
            line.split(',').nth(ci).map_or(String::new(), |f| f.trim().to_string())
        };
        cells.push(field);
    }
    let raw: Vec<&str> = cells.iter().map(String::as_str).collect();
    let valid: Vec<bool> = raw.iter().map(|s| !s.is_empty()).collect();
    (infer_and_convert_refs(&raw, &valid), valid)
}

/// A line's fields by the quoting rules (RFC 4180): a field in quotes may
/// hold commas and quotes, a quote inside it written twice; a field not in
/// quotes is trimmed, as the fast path trims one. A line break inside
/// quotes is not read: the file is split into lines first.
fn split_quoted(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    let mut was_quoted = false;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.trim().is_empty() {
            field.clear();
            in_quotes = true;
            was_quoted = true;
        } else if c == ',' {
            fields.push(if was_quoted { std::mem::take(&mut field) } else { field.trim().to_string() });
            field.clear();
            was_quoted = false;
        } else {
            field.push(c);
        }
    }
    fields.push(if was_quoted { field } else { field.trim().to_string() });
    fields
}

/// The bool a cell's text writes, as `save` writes one (`true`, `false`)
/// and as pandas does (`True`, `False`, and in capitals).
fn bool_text(s: &str) -> Option<bool> {
    match s {
        "true" | "True" | "TRUE" => Some(true),
        "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

/// The text of a cell as a CSV field: quoted where it holds a comma, a
/// quote (written twice inside) or a line break, as it is read back.
fn csv_field(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains([',', '"', '\n', '\r']) {
        std::borrow::Cow::Owned(format!("\"{}\"", text.replace('"', "\"\"")))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Infer column type from &str slices (avoids String allocation during parsing).
fn infer_and_convert_refs(raw: &[&str], valid: &[bool]) -> Column {
    // Sample first few valid values for type inference (avoid scanning all values)
    let sample_size = 100;
    let mut sampled = Vec::with_capacity(sample_size);
    for (s, &v) in raw.iter().zip(valid.iter()) {
        if v {
            sampled.push(*s);
            if sampled.len() >= sample_size { break; }
        }
    }

    if sampled.is_empty() {
        return Column::Str(raw.iter().copied().collect());
    }

    // The sample rules a type out quickly; the whole column decides it. A
    // value past the sample that does not parse makes the column the next
    // type: an int column read "n/a" as 0, a float column as NaN, and a
    // datetime column panicked.
    if sampled.iter().all(|s| s.parse::<i64>().is_ok()) {
        let data: Option<Vec<i64>> = raw.iter().zip(valid.iter()).map(|(s, &v)| if v { s.parse().ok() } else { Some(0) }).collect();
        if let Some(data) = data {
            return Column::Int(data);
        }
    }

    if sampled.iter().all(|s| s.parse::<f64>().is_ok()) {
        let data: Option<Vec<f64>> = raw.iter().zip(valid.iter()).map(|(s, &v)| if v { s.parse().ok() } else { Some(f64::NAN) }).collect();
        if let Some(data) = data {
            return Column::Float(data);
        }
    }

    // true and false make a column of bools, as `save` writes one and as
    // pandas reads one (it had loaded as text, so `save` then `load` lost
    // the kind)
    if sampled.iter().all(|s| bool_text(s).is_some()) {
        let data: Option<Vec<bool>> = raw.iter().zip(valid.iter()).map(|(s, &v)| if v { bool_text(s) } else { Some(false) }).collect();
        if let Some(data) = data {
            return Column::Bool(data);
        }
    }

    if sampled.iter().all(|s| parse_datetime(s).is_some()) {
        let parsed: Option<Vec<(i64, i16)>> = raw.iter().zip(valid.iter()).map(|(s, &v)| if v { parse_datetime(s) } else { Some((0, 0)) }).collect();
        if let Some(parsed) = parsed {
            let (millis, offsets) = parsed.into_iter().unzip();
            return Column::DateTime(millis, offsets);
        }
    }

    // Fall back to text: a column whose sample shows few distinct values
    // (a symbol, a side) is held by a dictionary, a code a cell; one that
    // grows past the dictionary's limit is held plain
    let distinct: std::collections::HashSet<&str> = sampled.iter().copied().collect();
    if distinct.len() * 2 <= sampled.len() {
        let mut b = crate::textcol::DictBuilder::with_capacity(raw.len());
        for s in raw {
            b.push(s);
            if b.over() {
                return Column::Str(raw.iter().copied().collect());
            }
        }
        return Column::Str(b.finish());
    }
    Column::Str(raw.iter().copied().collect())
}


/// Parse ISO 8601 datetime string → (epoch_millis, offset_minutes).
/// Supports: 2024-01-15, 2024-01-15T10:30:00, 2024-01-15T10:30:00Z,
///           2024-01-15T10:30:00+05:30, 2024-01-15T10:30:00-08:00,
/// a fraction of a second (`10:30:00.123`, kept to the millisecond) and
/// an offset of hours alone (`-08`, as Postgres writes one) or `-0800`.
pub fn parse_datetime(s: &str) -> Option<(i64, i16)> {
    let s = s.trim();

    // Date-only: YYYY-MM-DD
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        let year: i32 = s[0..4].parse().ok()?;
        let month: u32 = s[5..7].parse().ok()?;
        let day: u32 = s[8..10].parse().ok()?;
        let millis = date_to_epoch_millis(year, month, day, 0, 0, 0)?;
        return Some((millis, 0));
    }

    // DateTime: YYYY-MM-DDTHH:MM:SS[offset]
    if s.len() >= 19 && (s.as_bytes()[10] == b'T' || s.as_bytes()[10] == b' ') {
        let year: i32 = s[0..4].parse().ok()?;
        let month: u32 = s[5..7].parse().ok()?;
        let day: u32 = s[8..10].parse().ok()?;
        let hour: u32 = s[11..13].parse().ok()?;
        let min: u32 = s[14..16].parse().ok()?;
        let sec: u32 = s[17..19].parse().ok()?;

        let mut rest = &s[19..];
        // a fraction of a second, kept to the millisecond
        let mut frac_millis: i64 = 0;
        if let Some(after_dot) = rest.strip_prefix('.') {
            let digits = after_dot.len() - after_dot.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits == 0 {
                return None;
            }
            let frac = &after_dot[..digits];
            let padded = format!("{:0<3}", frac);
            frac_millis = padded[..3].parse().ok()?;
            rest = &after_dot[digits..];
        }
        let offset_minutes: i16 = if rest.is_empty() {
            0 // No timezone → assume UTC
        } else if rest == "Z" {
            0
        } else if (rest.starts_with('+') || rest.starts_with('-')) && matches!(rest.len(), 3 | 5 | 6) {
            // +HH:MM, -HHMM or -HH
            let sign: i16 = if rest.starts_with('-') { -1 } else { 1 };
            let oh: i16 = rest[1..3].parse().ok()?;
            let om: i16 = match rest.len() {
                3 => 0,
                5 => rest[3..5].parse().ok()?,
                _ => {
                    if &rest[3..4] != ":" {
                        return None;
                    }
                    rest[4..6].parse().ok()?
                }
            };
            sign * (oh * 60 + om)
        } else {
            return None;
        };

        // Convert to UTC epoch millis: subtract offset to get UTC
        let local_millis = date_to_epoch_millis(year, month, day, hour, min, sec)? + frac_millis;
        let utc_millis = local_millis - (offset_minutes as i64) * 60_000;
        return Some((utc_millis, offset_minutes));
    }

    None
}

/// Convert date/time components to epoch milliseconds (UTC).
fn date_to_epoch_millis(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> Option<i64> {
    if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    // Days from epoch (1970-01-01) using a simplified algorithm
    let y = year as i64;
    let m = month as i64;
    let d = day as i64;

    // Adjust for months Jan, Feb
    let (y_adj, m_adj) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let days = 365 * y_adj + y_adj / 4 - y_adj / 100 + y_adj / 400
        + (m_adj * 306 + 5) / 10 + d - 1
        - 719468; // offset to Unix epoch

    Some(days * 86_400_000 + hour as i64 * 3_600_000 + min as i64 * 60_000 + sec as i64 * 1_000)
}

/// Format epoch millis + offset to ISO 8601 string.
pub fn format_datetime(epoch_millis: i64, offset_minutes: i16) -> String {
    // the instant in its zone: the offset added
    let local_millis = epoch_millis + (offset_minutes as i64) * 60_000;
    let total_secs = local_millis.div_euclid(1000);
    // a fraction of a second, to the millisecond, where there is one
    let fraction = match local_millis.rem_euclid(1000) {
        0 => String::new(),
        ms => format!(".{:03}", ms),
    };
    let days = total_secs.div_euclid(86400);
    let day_secs = total_secs.rem_euclid(86400) as u32;

    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;

    // Civil date from days since epoch
    let (year, month, day) = civil_from_days(days);

    if offset_minutes == 0 {
        if hour == 0 && min == 0 && sec == 0 && fraction.is_empty() {
            format!("{:04}-{:02}-{:02}", year, month, day)
        } else {
            format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}Z", year, month, day, hour, min, sec, fraction)
        }
    } else {
        let sign = if offset_minutes >= 0 { '+' } else { '-' };
        let abs_off = offset_minutes.unsigned_abs();
        let oh = abs_off / 60;
        let om = abs_off % 60;
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{}{:02}:{:02}",
            year, month, day, hour, min, sec, fraction, sign, oh, om)
    }
}

/// Convert days since Unix epoch to (year, month, day).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

/// Convert frame columns to a flat f64 vector in row-major order.
/// Only numeric (Int, Float) columns are accepted. Returns (data, nrows, ncols).
pub fn frame_to_tensor_data(frame: &CrowFrame, col_names: Option<&[String]>) -> Result<(Vec<f64>, usize, usize), String> {
    let indices: Vec<usize> = if let Some(names) = col_names {
        names.iter().map(|n| {
            frame.col_index(n).ok_or_else(|| format!("tensor: column '{}' not found", n))
        }).collect::<Result<_, _>>()?
    } else {
        (0..frame.ncols()).collect()
    };
    let nrows = frame.nrows();
    let ncols = indices.len();
    if ncols == 0 { return Err("tensor: frame has no columns".into()); }
    // Verify all selected columns are numeric
    for &ci in &indices {
        match frame.data[ci].as_ref() {
            Column::Int(_) | Column::Float(_) => {}
            other => return Err(format!("tensor: column '{}' has non-numeric type '{}'", frame.columns[ci], other.dtype_name())),
        }
    }
    let mut data = vec![0.0f64; nrows * ncols];
    for (j, &ci) in indices.iter().enumerate() {
        let valid = frame.validity[ci].as_ref();
        match frame.data[ci].as_ref() {
            Column::Int(v) => {
                for i in 0..nrows {
                    data[i * ncols + j] = if valid[i] { v[i] as f64 } else { f64::NAN };
                }
            }
            Column::Float(v) => {
                for i in 0..nrows {
                    data[i * ncols + j] = if valid[i] { v[i] } else { f64::NAN };
                }
            }
            // every column was checked numeric above
            _ => {}
        }
    }
    Ok((data, nrows, ncols))
}

// ===== Frame Operations =====

impl CrowFrame {
    /// Select specific columns by name.
    pub fn select(&self, col_names: &[&str]) -> Result<CrowFrame, String> {
        let mut new_cols = Vec::new();
        let mut new_data = Vec::new();
        let mut new_valid = Vec::new();
        for name in col_names {
            let idx = self.col_index(name)
                .ok_or_else(|| format!("select: column '{}' not found", name))?;
            new_cols.push(name.to_string());
            new_data.push(Arc::clone(&self.data[idx]));
            new_valid.push(Arc::clone(&self.validity[idx]));
        }
        Ok(CrowFrame { columns: new_cols, data: new_data, validity: new_valid })
    }

    /// Filter rows by a boolean mask.
    pub fn filter(&self, mask: &[bool]) -> Result<CrowFrame, String> {
        if mask.len() != self.nrows() {
            return Err(format!("filter: mask length {} != row count {}", mask.len(), self.nrows()));
        }
        let results: Vec<(Column, Validity)> = self.data.par_iter().zip(self.validity.par_iter())
            .map(|(col, valid)| filter_column(col, valid, mask))
            .collect();
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        Ok(CrowFrame::assembled(self.columns.clone(), new_data, new_valid))
    }

    /// Sort by a column, ascending or descending: stable — rows whose cells
    /// are equal keep their order, so a sort by one column then another
    /// orders by both — with a missing cell (or a float that is not a
    /// number) last either way, as pandas orders one. Each row's cell is
    /// packed into a key that orders as the cells do, and the (key, row)
    /// pairs are sorted across the cores: the row breaks every tie, which
    /// keeps the order. Text held by a dictionary is keyed by its value's
    /// rank among the dictionary's; plain text is compared as it is.
    pub fn sort(&self, col_name: &str, descending: bool) -> Result<CrowFrame, String> {
        let idx = self.col_index(col_name)
            .ok_or_else(|| format!("sort: column '{}' not found", col_name))?;
        let valid = self.validity[idx].as_ref();
        let column = self.data[idx].as_ref();
        let n = self.nrows();
        // (missing last, key, row): a missing cell sorts after every key,
        // and a descending sort flips the key, never the missing flag
        let keyed = |key: &(dyn Fn(usize) -> Option<u64> + Sync)| -> Vec<usize> {
            let mut pairs: Vec<(bool, u64, u32)> = (0..n)
                .into_par_iter()
                .map(|i| match key(i) {
                    Some(k) if valid[i] => (false, if descending { u64::MAX - k } else { k }, i as u32),
                    _ => (true, 0, i as u32),
                })
                .collect();
            pairs.par_sort_unstable();
            pairs.iter().map(|p| p.2 as usize).collect()
        };
        let indices: Vec<usize> = match column {
            Column::Int(v) => keyed(&|i| Some((v[i] as u64) ^ (1 << 63))),
            Column::DateTime(millis, _) => keyed(&|i| Some((millis[i] as u64) ^ (1 << 63))),
            Column::Bool(v) => keyed(&|i| Some(v[i] as u64)),
            Column::Float(v) => keyed(&|i| {
                let x = v[i];
                if x.is_nan() {
                    return None;
                }
                let bits = x.to_bits();
                Some(if x.is_sign_negative() { !bits } else { bits | (1 << 63) })
            }),
            Column::Str(t) => match t.dictionary() {
                Some((codes, values)) => {
                    let mut order: Vec<usize> = (0..values.len()).collect();
                    order.sort_by(|&a, &b| values.get(a).cmp(values.get(b)));
                    let mut rank = vec![0u64; values.len()];
                    for (r, &code) in order.iter().enumerate() {
                        rank[code] = r as u64;
                    }
                    keyed(&|i| Some(rank[codes[i] as usize]))
                }
                None => {
                    let mut indices: Vec<usize> = (0..n).collect();
                    indices.par_sort_by(|&a, &b| match (valid[a], valid[b]) {
                        (false, false) => std::cmp::Ordering::Equal,
                        (false, true) => std::cmp::Ordering::Greater,
                        (true, false) => std::cmp::Ordering::Less,
                        (true, true) => {
                            let o = t.get(a).cmp(t.get(b));
                            if descending { o.reverse() } else { o }
                        }
                    });
                    indices
                }
            },
        };
        Ok(self.reorder_rows(&indices))
    }

    /// Head: first N rows.
    pub fn head(&self, n: usize) -> CrowFrame {
        let n = n.min(self.nrows());
        self.slice_rows(0, n)
    }

    /// Tail: last N rows.
    pub fn tail(&self, n: usize) -> CrowFrame {
        let nrows = self.nrows();
        let n = n.min(nrows);
        self.slice_rows(nrows - n, nrows)
    }

    /// Rename a column. O(ncols) — shares all column data via Arc.
    pub fn rename(&self, old_name: &str, new_name: &str) -> Result<CrowFrame, String> {
        if self.col_index(old_name).is_none() {
            return Err(format!("rename: column '{}' not found", old_name));
        }
        let new_cols: Vec<String> = self.columns.iter()
            .map(|c| if c == old_name { new_name.to_string() } else { c.clone() })
            .collect();
        // Arc::clone is O(1) per column — no data copying
        Ok(CrowFrame {
            columns: new_cols,
            data: self.data.iter().map(Arc::clone).collect(),
            validity: self.validity.iter().map(Arc::clone).collect(),
        })
    }

    /// Fill missing values in a column with a default value string.
    /// Only deep-clones the target column via Arc::make_mut; other columns are shared.
    pub fn fill_column(&self, col_name: &str, fill_val: &str) -> Result<CrowFrame, String> {
        let idx = self.col_index(col_name)
            .ok_or_else(|| format!("fill: column '{}' not found", col_name))?;
        // Arc::clone is O(1) — only target column will be deep-cloned via make_mut
        let mut new_data: Vec<Arc<Column>> = self.data.iter().map(Arc::clone).collect();
        let mut new_valid: Vec<Arc<Validity>> = self.validity.iter().map(Arc::clone).collect();

        let col_mut = Arc::make_mut(&mut new_data[idx]);
        let valid_mut = Arc::make_mut(&mut new_valid[idx]);

        match col_mut {
            Column::Int(v) => {
                let fv: i64 = fill_val.parse().map_err(|_| format!("fill: cannot parse '{}' as int", fill_val))?;
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fv; valid_mut.set(i, true); }
                }
            }
            Column::Float(v) => {
                let fv: f64 = fill_val.parse().map_err(|_| format!("fill: cannot parse '{}' as float", fill_val))?;
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fv; valid_mut.set(i, true); }
                }
            }
            Column::Str(v) => {
                *v = v.filled(&valid_mut.to_vec(), fill_val);
                *valid_mut = Validity::all(v.len());
            }
            Column::Bool(v) => {
                let fv: bool = fill_val.parse().map_err(|_| format!("fill: cannot parse '{}' as bool", fill_val))?;
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fv; valid_mut.set(i, true); }
                }
            }
            Column::DateTime(ms, offsets) => {
                let (fm, fo) = parse_datetime(fill_val).ok_or_else(|| format!("fill: cannot read '{}' as a moment", fill_val))?;
                for i in 0..ms.len() {
                    if !valid_mut[i] { ms[i] = fm; offsets[i] = fo; valid_mut.set(i, true); }
                }
            }
        }

        Ok(CrowFrame { columns: self.columns.clone(), data: new_data, validity: new_valid })
    }

    /// Stack two frames vertically. Columns processed in parallel.
    pub fn stack(&self, other: &CrowFrame) -> Result<CrowFrame, String> {
        if self.columns != other.columns {
            return Err("stack: column names don't match".to_string());
        }
        let results: Result<Vec<(Column, Validity)>, String> = (0..self.ncols()).into_par_iter().map(|i| {
            stack_columns(&self.data[i], &self.validity[i], &other.data[i], &other.validity[i])
        }).collect();
        let results = results?;
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        Ok(CrowFrame::assembled(self.columns.clone(), new_data, new_valid))
    }
}

// ===== Helper functions =====

fn filter_column(col: &Column, valid: &Validity, mask: &[bool]) -> (Column, Validity) {
    let new_valid = valid.filtered(mask);
    let new_col = match col {
        Column::Int(v) => Column::Int(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect()),
        Column::Float(v) => Column::Float(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect()),
        Column::Str(v) => Column::Str(v.filtered(mask)),
        Column::Bool(v) => Column::Bool(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect()),
        Column::DateTime(millis, offsets) => {
            let new_m: Vec<i64> = millis.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect();
            let new_o: Vec<i16> = offsets.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect();
            Column::DateTime(new_m, new_o)
        }
    };
    (new_col, new_valid)
}

fn stack_columns(a: &Column, av: &Validity, b: &Column, bv: &Validity) -> Result<(Column, Validity), String> {
    let new_valid = av.concat(bv);
    let col = match (a, b) {
        (Column::Int(va), Column::Int(vb)) => { let mut v = va.clone(); v.extend_from_slice(vb); Column::Int(v) }
        (Column::Float(va), Column::Float(vb)) => { let mut v = va.clone(); v.extend_from_slice(vb); Column::Float(v) }
        (Column::Str(va), Column::Str(vb)) => Column::Str(va.concat(vb)),
        (Column::Bool(va), Column::Bool(vb)) => { let mut v = va.clone(); v.extend_from_slice(vb); Column::Bool(v) }
        (Column::DateTime(ma, oa), Column::DateTime(mb, ob)) => {
            let mut m = ma.clone(); m.extend_from_slice(mb);
            let mut o = oa.clone(); o.extend_from_slice(ob);
            Column::DateTime(m, o)
        }
        _ => return Err("stack: column type mismatch".to_string()),
    };
    Ok((col, new_valid))
}

impl CrowFrame {
    /// Reorder rows by given indices (columns processed in parallel).
    pub(crate) fn reorder_rows(&self, indices: &[usize]) -> CrowFrame {
        let results: Vec<(Column, Validity)> = self.data.par_iter().zip(self.validity.par_iter())
            .map(|(col, valid)| {
                let nv = valid.gather(indices);
                let nc = match col.as_ref() {
                    Column::Int(v) => Column::Int(indices.iter().map(|&i| v[i]).collect()),
                    Column::Float(v) => Column::Float(indices.iter().map(|&i| v[i]).collect()),
                    Column::Str(v) => Column::Str(v.gather(indices)),
                    Column::Bool(v) => Column::Bool(indices.iter().map(|&i| v[i]).collect()),
                    Column::DateTime(millis, offsets) => Column::DateTime(
                        indices.iter().map(|&i| millis[i]).collect(),
                        indices.iter().map(|&i| offsets[i]).collect(),
                    ),
                };
                (nc, nv)
            })
            .collect();
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        CrowFrame::assembled(self.columns.clone(), new_data, new_valid)
    }

    /// Slice rows [start..end). Parallel across columns.
    fn slice_rows(&self, start: usize, end: usize) -> CrowFrame {
        let results: Vec<(Column, Validity)> = self.data.par_iter().zip(self.validity.par_iter())
            .map(|(col, valid)| {
                let nv = valid.sliced(start, end);
                let nc = match col.as_ref() {
                    Column::Int(v) => Column::Int(v[start..end].to_vec()),
                    Column::Float(v) => Column::Float(v[start..end].to_vec()),
                    Column::Str(v) => Column::Str(v.slice(start, end)),
                    Column::Bool(v) => Column::Bool(v[start..end].to_vec()),
                    Column::DateTime(millis, offsets) => Column::DateTime(
                        millis[start..end].to_vec(), offsets[start..end].to_vec(),
                    ),
                };
                (nc, nv)
            })
            .collect();
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        CrowFrame::assembled(self.columns.clone(), new_data, new_valid)
    }
}

// ===== Group =====

/// Aggregation function type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggFn {
    Mean,
    Sum,
    Min,
    Max,
    Count,
    /// The middle of the present cells (the mean of the two middles).
    Median,
    /// The population standard deviation and variance (spec: the frame
    /// module's deviation is the population's, as `stats` and `std` of a
    /// tensor answer it).
    Std,
    Var,
    /// The first and the last present cell, in the frame's order.
    First,
    Last,
    /// How many distinct present cells.
    Distinct,
    /// A percentile, `'p90'`: the value a fraction of the present cells
    /// lie at or below, interpolated between two cells (as pandas, numpy
    /// and Julia interpolate; `'median'` is `'p50'`).
    Percentile(u8),
}

impl AggFn {
    pub fn from_str(s: &str) -> Option<AggFn> {
        match s {
            "mean" => Some(AggFn::Mean),
            "sum" => Some(AggFn::Sum),
            "min" => Some(AggFn::Min),
            "max" => Some(AggFn::Max),
            "count" => Some(AggFn::Count),
            "median" => Some(AggFn::Median),
            "std" => Some(AggFn::Std),
            "var" => Some(AggFn::Var),
            "first" => Some(AggFn::First),
            "last" => Some(AggFn::Last),
            "distinct" => Some(AggFn::Distinct),
            p => p.strip_prefix('p').filter(|d| !d.is_empty() && d.len() <= 2 && d.bytes().all(|b| b.is_ascii_digit())).and_then(|d| d.parse::<u8>().ok()).filter(|p| (1..=99).contains(p)).map(AggFn::Percentile),
        }
    }

    /// The word a program writes for it, which names the aggregate's
    /// column (`price_median`, `price_p90`).
    pub fn word(&self) -> String {
        match self {
            AggFn::Mean => "mean".to_string(),
            AggFn::Sum => "sum".to_string(),
            AggFn::Min => "min".to_string(),
            AggFn::Max => "max".to_string(),
            AggFn::Count => "count".to_string(),
            AggFn::Median => "median".to_string(),
            AggFn::Std => "std".to_string(),
            AggFn::Var => "var".to_string(),
            AggFn::First => "first".to_string(),
            AggFn::Last => "last".to_string(),
            AggFn::Distinct => "distinct".to_string(),
            AggFn::Percentile(p) => format!("p{}", p),
        }
    }

    /// The words there are, for the reason a misspelled one stops with.
    pub const WORDS: &'static str = "'sum', 'mean', 'min', 'max', 'count', 'median', 'std', 'var', 'first', 'last', 'distinct' or a percentile ('p90')";

    /// Whether it is computed over numbers alone (text and moments stop).
    pub fn numeric(&self) -> bool {
        matches!(self, AggFn::Sum | AggFn::Mean | AggFn::Median | AggFn::Std | AggFn::Var | AggFn::Percentile(_))
    }
}

impl CrowFrame {
    /// Group by a column and aggregate multiple (col, agg) pairs.
    pub fn group(&self, group_col: &str, aggs: &[(&str, AggFn)]) -> Result<CrowFrame, String> {
        self.group_by(&[group_col], aggs)
    }

    /// `group[frame, ['a', 'b'], [column, how], …]`: one row per distinct
    /// combination of the keys' cells, in the order each is first seen,
    /// the key columns as they were (their kinds kept) and a float per
    /// aggregate named `column_how`. A row with a missing key cell is in
    /// no group.
    pub fn group_by(&self, keys: &[&str], aggs: &[(&str, AggFn)]) -> Result<CrowFrame, String> {
        if keys.is_empty() {
            return Err("group: a key is needed — group[frame, 'col', …] or group[frame, ['a', 'b'], …]".to_string());
        }
        let key_idx: Vec<usize> = keys
            .iter()
            .map(|k| self.col_index(k).ok_or_else(|| format!("group: column '{}' not found", k)))
            .collect::<Result<_, _>>()?;
        if let Some(twice) = keys.iter().enumerate().find(|(i, k)| keys[..*i].contains(k)).map(|(_, k)| k) {
            return Err(format!("group: key '{}' is named twice", twice));
        }
        // every aggregated column exists before the work is shared out (a
        // misspelled one panicked a worker thread)
        let agg_cols: Vec<usize> = aggs
            .iter()
            .map(|(c, _)| self.col_index(c).ok_or_else(|| format!("group: column '{}' not found", c)))
            .collect::<Result<_, _>>()?;
        let (group_ids, first_rows) = self.partitions(&key_idx);
        let ngroups = first_rows.len();

        let mut result_cols: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        let mut result_data: Vec<Column> = key_idx.iter().map(|&ki| take_rows(&self.data[ki], &first_rows)).collect();
        let mut result_valid: Vec<Vec<bool>> = vec![vec![true; ngroups]; keys.len()];

        // Aggregate columns — process each agg column in parallel
        let agg_results: Result<Vec<(String, Column, Vec<bool>)>, String> = aggs.par_iter().zip(agg_cols.par_iter()).map(|((col_name, agg_fn), &ci)| {
            let agg_name = format!("{}_{}", col_name, agg_fn.word());
            let col_valid = self.validity[ci].as_ref();
            let (data, valid) = aggregate_by_groups(col_name, self.data[ci].as_ref(), col_valid, &group_ids, ngroups, *agg_fn)?;
            Ok((agg_name, data, valid))
        }).collect();

        for (agg_name, data, valid) in agg_results? {
            result_cols.push(agg_name);
            result_valid.push(valid);
            result_data.push(data);
        }

        Ok(CrowFrame::new(result_cols, result_data, result_valid))
    }

    /// Each row's partition by the cells of the key columns — a number per
    /// distinct combination, first seen first, `u32::MAX` for a row with a
    /// missing key cell — and the first row of each partition.
    pub(crate) fn partitions(&self, key_idx: &[usize]) -> (Vec<u32>, Vec<usize>) {
        let nrows = self.nrows();
        let per_key: Vec<Vec<u32>> = key_idx.iter().map(|&ki| codes(&self.data[ki], &self.validity[ki])).collect();
        if per_key.len() == 1 {
            let ids = per_key.into_iter().next().unwrap_or_default();
            let mut first_rows = Vec::new();
            for (row, &id) in ids.iter().enumerate() {
                if id != u32::MAX && id as usize == first_rows.len() {
                    first_rows.push(row);
                }
            }
            return (ids, first_rows);
        }
        let mut ids = vec![u32::MAX; nrows];
        let mut first_rows = Vec::new();
        // up to four keys' codes packed into one array (a vector per row
        // had been allocated); more keys take a vector
        if per_key.len() <= 4 {
            let mut seen: std::collections::HashMap<[u32; 4], u32> = std::collections::HashMap::new();
            for row in 0..nrows {
                let mut key = [0u32; 4];
                let mut missing = false;
                for (k, c) in per_key.iter().enumerate() {
                    key[k] = c[row];
                    missing |= c[row] == u32::MAX;
                }
                if missing {
                    continue;
                }
                let next = first_rows.len() as u32;
                let id = *seen.entry(key).or_insert_with(|| {
                    first_rows.push(row);
                    next
                });
                ids[row] = id;
            }
            return (ids, first_rows);
        }
        let mut seen: std::collections::HashMap<Vec<u32>, u32> = std::collections::HashMap::new();
        for row in 0..nrows {
            let key: Vec<u32> = per_key.iter().map(|c| c[row]).collect();
            if key.contains(&u32::MAX) {
                continue;
            }
            let next = first_rows.len() as u32;
            let id = *seen.entry(key).or_insert_with(|| {
                first_rows.push(row);
                next
            });
            ids[row] = id;
        }
        (ids, first_rows)
    }

    /// `pivot[frame, rows, cols, values]`: a wide frame — one row per
    /// distinct cell of `rows`, a column per distinct cell of `cols`, named
    /// by the cell's text, each holding the `values` cell of the row that
    /// has that pair, and a missing cell where no row does. Rows and columns
    /// come in ascending order of their cells. A pair two rows share is the
    /// reason: pivot reshapes, and `group` aggregates.
    pub fn pivot(&self, rows: &str, cols: &str, values: &str) -> Result<CrowFrame, String> {
        let at = |name: &str| self.col_index(name).ok_or_else(|| format!("pivot: column '{}' not found", name));
        let (ri, ci, vi) = (at(rows)?, at(cols)?, at(values)?);
        if ri == ci {
            return Err(format!("pivot: the rows and the columns are one column, '{}'", rows));
        }
        let (row_codes, row_first) = self.partitions(&[ri]);
        let (col_codes, col_first) = self.partitions(&[ci]);
        // each distinct cell's place in ascending order
        let row_order = order_of(&self.data[ri], &row_first);
        let col_order = order_of(&self.data[ci], &col_first);
        let place = |order: &[usize]| {
            let mut place = vec![0usize; order.len()];
            for (p, &code) in order.iter().enumerate() {
                place[code] = p;
            }
            place
        };
        let (row_place, col_place) = (place(&row_order), place(&col_order));
        let (nr, nc) = (row_first.len(), col_first.len());
        let mut source: Vec<Option<usize>> = vec![None; nr * nc];
        let mut twice: Option<(usize, usize)> = None;
        for row in 0..self.nrows() {
            let (rc, cc) = (row_codes[row], col_codes[row]);
            if rc == u32::MAX || cc == u32::MAX {
                continue;
            }
            let slot = row_place[rc as usize] * nc + col_place[cc as usize];
            if source[slot].is_some() {
                twice.get_or_insert((rc as usize, cc as usize));
            } else {
                source[slot] = Some(row);
            }
        }
        if let Some((rc, cc)) = twice {
            let count = (0..self.nrows()).filter(|&r| row_codes[r] as usize == rc && col_codes[r] as usize == cc).count();
            return Err(format!(
                "pivot: '{}' and '{}' appear {} times — one row per pair: aggregate first, group[frame, ['{}', '{}'], ['{}', 'mean']]",
                self.data[ri].fmt_cell(row_first[rc], true),
                self.data[ci].fmt_cell(col_first[cc], true),
                count, rows, cols, values
            ));
        }
        let names: Vec<String> = col_order.iter().map(|&code| self.data[ci].fmt_cell(col_first[code], true)).collect();
        if let Some(name) = names.iter().find(|n| **n == rows) {
            return Err(format!("pivot: a cell of '{}' is '{}', the name of the rows column", cols, name));
        }
        let mut out_cols = vec![rows.to_string()];
        out_cols.extend(names);
        let row_cells: Vec<usize> = row_order.iter().map(|&code| row_first[code]).collect();
        let mut data = vec![take_rows(&self.data[ri], &row_cells)];
        let mut valid = vec![vec![true; nr]];
        let value_valid = self.validity[vi].as_ref();
        for c in 0..nc {
            let picks: Vec<Option<usize>> = (0..nr).map(|r| source[r * nc + c].filter(|&row| value_valid[row])).collect();
            let (col, ok) = take_some_rows(&self.data[vi], &picks);
            data.push(col);
            valid.push(ok);
        }
        Ok(CrowFrame::new(out_cols, data, valid))
    }

    /// `window[…]` (verbs.rs: `window_over`).
    pub fn window(&self, col: &str, how: &str, n: i64, by: &[&str]) -> Result<CrowColumn, String> {
        self.window_over(col, how, n, by, 0.0, "")
    }

    /// `join[a, b, key, how]`: a join on one key (verbs.rs: `join_keys`).
    pub fn join(&self, other: &CrowFrame, key: &str, join_type: &str) -> Result<CrowFrame, String> {
        self.join_keys(other, &[key], join_type)
    }
}

/// Aggregate using group_ids array (avoids building per-group Vec<usize> row lists).
/// A column's cells as codes: each distinct present cell a number, first
/// seen first; a missing cell `u32::MAX`.
pub(crate) fn codes(col: &Column, valid: &Validity) -> Vec<u32> {
    fn coded<K: std::hash::Hash + Eq>(keys: impl Iterator<Item = K>, valid: &Validity) -> Vec<u32> {
        let mut seen: std::collections::HashMap<K, u32> = std::collections::HashMap::new();
        keys.enumerate()
            .map(|(row, k)| {
                if !valid[row] {
                    return u32::MAX;
                }
                let next = seen.len() as u32;
                *seen.entry(k).or_insert(next)
            })
            .collect()
    }
    match col {
        Column::Int(v) => coded(v.iter().copied(), valid),
        Column::Float(v) => coded(v.iter().map(|x| x.to_bits()), valid),
        Column::Str(v) => match v.dictionary() {
            // a dictionary column's codes, renumbered in the order the rows
            // first show them, with no hashing
            Some((codes, values)) => {
                let mut seen = vec![u32::MAX; values.len()];
                let mut next = 0u32;
                codes
                    .iter()
                    .enumerate()
                    .map(|(row, &c)| {
                        if !valid[row] {
                            return u32::MAX;
                        }
                        let slot = &mut seen[c as usize];
                        if *slot == u32::MAX {
                            *slot = next;
                            next += 1;
                        }
                        *slot
                    })
                    .collect()
            }
            None => coded(v.iter(), valid),
        },
        Column::Bool(v) => coded(v.iter().copied(), valid),
        Column::DateTime(ms, _) => coded(ms.iter().copied(), valid),
    }
}

/// The named rows of a column, as a column.
pub(crate) fn take_rows(col: &Column, rows: &[usize]) -> Column {
    match col {
        Column::Int(v) => Column::Int(rows.iter().map(|&r| v[r]).collect()),
        Column::Float(v) => Column::Float(rows.iter().map(|&r| v[r]).collect()),
        Column::Str(v) => Column::Str(v.gather(rows)),
        Column::Bool(v) => Column::Bool(rows.iter().map(|&r| v[r]).collect()),
        Column::DateTime(ms, off) => Column::DateTime(rows.iter().map(|&r| ms[r]).collect(), rows.iter().map(|&r| off[r]).collect()),
    }
}

/// The named rows of a column, a missing cell where no row is named.
pub(crate) fn take_some_rows(col: &Column, rows: &[Option<usize>]) -> (Column, Vec<bool>) {
    let valid: Vec<bool> = rows.iter().map(Option::is_some).collect();
    let data = match col {
        Column::Int(v) => Column::Int(rows.iter().map(|r| r.map_or(0, |r| v[r])).collect()),
        Column::Float(v) => Column::Float(rows.iter().map(|r| r.map_or(f64::NAN, |r| v[r])).collect()),
        Column::Str(v) => Column::Str(v.gather_some(rows)),
        Column::Bool(v) => Column::Bool(rows.iter().map(|r| r.is_some_and(|r| v[r])).collect()),
        Column::DateTime(ms, off) => Column::DateTime(rows.iter().map(|r| r.map_or(0, |r| ms[r])).collect(), rows.iter().map(|r| r.map_or(0, |r| off[r])).collect()),
    };
    (data, valid)
}

/// The positions of `rows` in ascending order of their cells: numbers by
/// value (NaN last), text by character code, bools false first, moments
/// by their instant; equal cells keep their order.
pub(crate) fn order_of(col: &Column, rows: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    match col {
        Column::Int(v) => order.sort_by_key(|&p| v[rows[p]]),
        Column::Float(v) => order.sort_by(|&a, &b| v[rows[a]].partial_cmp(&v[rows[b]]).unwrap_or_else(|| v[rows[a]].is_nan().cmp(&v[rows[b]].is_nan()))),
        Column::Str(v) => order.sort_by(|&a, &b| v[rows[a]].cmp(&v[rows[b]])),
        Column::Bool(v) => order.sort_by_key(|&p| v[rows[p]]),
        Column::DateTime(ms, _) => order.sort_by_key(|&p| ms[rows[p]]),
    }
    order
}

/// Whether two of a column's cells are equal, as two values are.
pub(crate) fn cells_equal(col: &Column, a: usize, b: usize) -> bool {
    match col {
        Column::Int(v) => v[a] == v[b],
        Column::Float(v) => v[a] == v[b],
        Column::Str(v) => v[a] == v[b],
        Column::Bool(v) => v[a] == v[b],
        Column::DateTime(ms, _) => ms[a] == ms[b],
    }
}

/// A running total with Kahan compensation, added to as pandas adds a
/// group's cells (its `group_sum` and `group_mean`), so a sum or a mean
/// over floats is pandas's to the last digit — and nearer the true sum
/// than a plain running total.
#[derive(Clone, Copy, Default)]
pub(crate) struct Kahan {
    total: f64,
    compensation: f64,
}

impl Kahan {
    pub(crate) fn add(&mut self, x: f64) {
        let y = x - self.compensation;
        let t = self.total + y;
        self.compensation = t - self.total - y;
        self.total = t;
    }
}

/// A window's running total as pandas keeps one (`roll_sum`, `roll_mean`):
/// a compensated total with one compensation for the cells added and
/// another for those taken off; a count of the negatives, so a sum of
/// nothing negative is never below 0; and a count of the equal cells in a
/// row, so a window whose cells are all one value answers that value
/// exactly (the compensated total could be off by a bit).
#[derive(Clone, Copy, Default)]
pub(crate) struct Roll {
    pub(crate) total: f64,
    added: f64,
    removed: f64,
    pub(crate) count: usize,
    negatives: usize,
    same: usize,
    last: f64,
}

impl Roll {
    pub(crate) fn add(&mut self, x: f64) {
        self.count += 1;
        let y = x - self.added;
        let t = self.total + y;
        self.added = t - self.total - y;
        self.total = t;
        if x < 0.0 {
            self.negatives += 1;
        }
        if self.count > 1 && x == self.last {
            self.same += 1;
        } else {
            self.same = 1;
        }
        self.last = x;
    }

    pub(crate) fn remove(&mut self, x: f64) {
        self.count -= 1;
        let y = -x - self.removed;
        let t = self.total + y;
        self.removed = t - self.total - y;
        self.total = t;
        if x < 0.0 {
            self.negatives -= 1;
        }
    }

    pub(crate) fn sum(&self) -> f64 {
        if self.same >= self.count {
            return self.last * self.count as f64;
        }
        let total = self.total;
        if (self.negatives == 0 && total < 0.0) || (self.negatives == self.count && total > 0.0) {
            0.0
        } else {
            total
        }
    }

    pub(crate) fn mean(&self) -> f64 {
        if self.same >= self.count {
            return self.last;
        }
        let mean = self.total / self.count as f64;
        if (self.negatives == 0 && mean < 0.0) || (self.negatives == self.count && mean > 0.0) {
            0.0
        } else {
            mean
        }
    }
}

/// A column aggregated by group, each aggregate in the kind it has: a
/// `count` the present cells, an int; a `sum` an int over ints or bools
/// and a float over floats, 0 where a group has no present cell; a `mean`
/// a float; a `min` or `max` a cell of the column's own kind — numbers by
/// value, text by character code, bools false before true, moments by
/// their instant — and a missing cell where a group has none present. A
/// missing cell is skipped, as SQL's aggregates skip NULL. (Every
/// aggregate had been a float, and a sum over text had silently been 0.)
fn aggregate_by_groups(name: &str, col: &Column, valid: &Validity, group_ids: &[u32], ngroups: usize, agg: AggFn) -> Result<(Column, Vec<bool>), String> {
    use std::cmp::Ordering;
    let rows = group_ids.iter().enumerate().filter_map(|(row, &g)| (g != u32::MAX && valid[row]).then_some((row, g as usize)));
    // a float that is not a number is no cell to aggregate
    let present = |row: usize| match col {
        Column::Float(d) => !d[row].is_nan(),
        _ => true,
    };
    let holds = match col {
        Column::Str(_) => Some("text"),
        Column::DateTime(..) => Some("moments"),
        _ => None,
    };
    match agg {
        AggFn::Count => {
            let mut counts = vec![0i64; ngroups];
            for (_, g) in rows {
                counts[g] += 1;
            }
            Ok((Column::Int(counts), vec![true; ngroups]))
        }
        AggFn::Sum => match col {
            Column::Int(d) => {
                let mut sums = vec![0i64; ngroups];
                for (row, g) in rows {
                    sums[g] = sums[g].wrapping_add(d[row]);
                }
                Ok((Column::Int(sums), vec![true; ngroups]))
            }
            Column::Bool(d) => {
                let mut sums = vec![0i64; ngroups];
                for (row, g) in rows {
                    sums[g] += d[row] as i64;
                }
                Ok((Column::Int(sums), vec![true; ngroups]))
            }
            Column::Float(d) => {
                let mut sums = vec![Kahan::default(); ngroups];
                for (row, g) in rows {
                    if !d[row].is_nan() {
                        sums[g].add(d[row]);
                    }
                }
                Ok((Column::Float(sums.iter().map(|k| k.total).collect()), vec![true; ngroups]))
            }
            Column::Str(_) | Column::DateTime(..) => Err(format!("group: sum of column '{}', which holds {}", name, holds.unwrap_or("no numbers"))),
        },
        AggFn::Mean => {
            if let Some(what) = holds {
                return Err(format!("group: mean of column '{}', which holds {}", name, what));
            }
            let value = |row: usize| match col {
                Column::Int(d) => d[row] as f64,
                Column::Float(d) => d[row],
                Column::Bool(d) => d[row] as u8 as f64,
                _ => f64::NAN,
            };
            let mut sums = vec![Kahan::default(); ngroups];
            let mut counts = vec![0u64; ngroups];
            for (row, g) in rows {
                if present(row) {
                    sums[g].add(value(row));
                    counts[g] += 1;
                }
            }
            let valid: Vec<bool> = counts.iter().map(|&c| c > 0).collect();
            let means: Vec<f64> = sums.iter().zip(&counts).map(|(s, &c)| if c > 0 { s.total / c as f64 } else { f64::NAN }).collect();
            Ok((Column::Float(means), valid))
        }
        AggFn::First | AggFn::Last => {
            // the first, or the last, present cell of each group
            let mut pick: Vec<Option<usize>> = vec![None; ngroups];
            for (row, g) in rows {
                if !present(row) {
                    continue;
                }
                if agg == AggFn::Last || pick[g].is_none() {
                    pick[g] = Some(row);
                }
            }
            Ok(take_some_rows(col, &pick))
        }
        AggFn::Distinct => {
            // (group, code) pairs sorted and deduplicated: a count per group
            let coded = codes(col, valid);
            let mut pairs: Vec<(u32, u32)> = rows.filter(|&(row, _)| present(row)).map(|(row, g)| (g as u32, coded[row])).collect();
            pairs.par_sort_unstable();
            pairs.dedup();
            let mut counts = vec![0i64; ngroups];
            for (g, _) in pairs {
                counts[g as usize] += 1;
            }
            Ok((Column::Int(counts), vec![true; ngroups]))
        }
        AggFn::Median | AggFn::Std | AggFn::Var | AggFn::Percentile(_) => {
            if let Some(what) = holds {
                return Err(format!("group: {} of column '{}', which holds {}", agg.word(), name, what));
            }
            let value = |row: usize| match col {
                Column::Int(d) => d[row] as f64,
                Column::Float(d) => d[row],
                Column::Bool(d) => d[row] as u8 as f64,
                _ => f64::NAN,
            };
            let mut per: Vec<Vec<f64>> = vec![Vec::new(); ngroups];
            for (row, g) in rows {
                if present(row) {
                    per[g].push(value(row));
                }
            }
            let out: Vec<f64> = per
                .par_iter()
                .map(|values| match agg {
                    AggFn::Median => crate::verbs::median_of(values),
                    AggFn::Percentile(p) => crate::verbs::percentile_of(values, p),
                    AggFn::Var => crate::verbs::welford_var(values),
                    _ => crate::verbs::welford_var(values).sqrt(),
                })
                .collect();
            let valid: Vec<bool> = per.iter().map(|v| !v.is_empty()).collect();
            Ok((Column::Float(out), valid))
        }
        AggFn::Min | AggFn::Max => {
            // the row holding each group's least, or greatest, cell so far
            let better = |candidate: usize, held: usize| -> bool {
                let o = match col {
                    Column::Int(d) => d[candidate].cmp(&d[held]),
                    Column::Float(d) => d[candidate].partial_cmp(&d[held]).unwrap_or(Ordering::Equal),
                    Column::Str(d) => d[candidate].cmp(&d[held]),
                    Column::Bool(d) => d[candidate].cmp(&d[held]),
                    Column::DateTime(ms, _) => ms[candidate].cmp(&ms[held]),
                };
                matches!((agg, o), (AggFn::Min, Ordering::Less) | (AggFn::Max, Ordering::Greater))
            };
            let mut best: Vec<Option<usize>> = vec![None; ngroups];
            for (row, g) in rows {
                if !present(row) {
                    continue;
                }
                match best[g] {
                    Some(held) if !better(row, held) => {}
                    _ => best[g] = Some(row),
                }
            }
            Ok(take_some_rows(col, &best))
        }
    }
}

/// A join's key column where rows have no match on the left (a right or
/// outer join): each row's key from the side that has the row.
pub(crate) fn merged_keys(left: &Column, lvalid: &Validity, right: &Column, rvalid: &Validity, li: &[Option<usize>], ri: &[Option<usize>]) -> (Column, Vec<bool>) {
    // (from the left, at this row) for each output row
    let picks: Vec<(bool, usize)> = li.iter().zip(ri).map(|(l, r)| match (l, r) { (Some(i), _) => (true, *i), (None, Some(j)) => (false, *j), _ => (true, 0) }).collect();
    let valid: Vec<bool> = li.iter().zip(ri).map(|(l, r)| match (l, r) { (Some(i), _) => lvalid[*i], (None, Some(j)) => rvalid[*j], _ => false }).collect();
    let data = match (left, right) {
        (Column::Int(a), Column::Int(b)) => Column::Int(picks.iter().map(|&(l, i)| if l { a[i] } else { b[i] }).collect()),
        (Column::Float(a), Column::Float(b)) => Column::Float(picks.iter().map(|&(l, i)| if l { a[i] } else { b[i] }).collect()),
        (Column::Bool(a), Column::Bool(b)) => Column::Bool(picks.iter().map(|&(l, i)| if l { a[i] } else { b[i] }).collect()),
        (Column::DateTime(am, ao), Column::DateTime(bm, bo)) => Column::DateTime(
            picks.iter().map(|&(l, i)| if l { am[i] } else { bm[i] }).collect(),
            picks.iter().map(|&(l, i)| if l { ao[i] } else { bo[i] }).collect(),
        ),
        (Column::Str(a), Column::Str(b)) => Column::Str(picks.iter().map(|&(l, i)| if l { &a[i] } else { &b[i] }).collect()),
        (a, b) => Column::Str(picks.iter().map(|&(l, i)| if l { a.fmt_cell(i, true) } else { b.fmt_cell(i, true) }).collect()),
    };
    (data, valid)
}

pub(crate) fn gather_column(col: &Column, valid: &Validity, indices: &[Option<usize>]) -> (Column, Validity) {
    let new_valid = valid.gather_some(indices);
    let nc = match col {
        Column::Int(v) => Column::Int(indices.iter().map(|opt| opt.map_or(0, |i| v[i])).collect()),
        Column::Float(v) => Column::Float(indices.iter().map(|opt| opt.map_or(f64::NAN, |i| v[i])).collect()),
        Column::Str(v) => Column::Str(v.gather_some(indices)),
        Column::Bool(v) => Column::Bool(indices.iter().map(|opt| opt.map_or(false, |i| v[i])).collect()),
        Column::DateTime(millis, offsets) => Column::DateTime(
            indices.iter().map(|opt| opt.map_or(0, |i| millis[i])).collect(),
            indices.iter().map(|opt| opt.map_or(0, |i| offsets[i])).collect(),
        ),
    };
    (nc, new_valid)
}

// ===== Save, Unique, Stats =====

impl CrowFrame {
    /// Save frame to CSV. Parallel per-column string building, then interleave rows.
    pub fn save(&self, path: &str) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("save: {}", e))?;
        self.write_csv(file)
    }

    /// The frame as CSV text, as `save` writes it — what `download` hands
    /// the reader for a frame.
    pub fn to_csv(&self) -> Result<String, String> {
        let mut out = Vec::new();
        self.write_csv(&mut out)?;
        String::from_utf8(out).map_err(|e| format!("save: {}", e))
    }

    /// The frame as CSV into any writer.
    pub fn write_csv<W: std::io::Write>(&self, out: W) -> Result<(), String> {
        use std::io::{Write, BufWriter};
        let mut w = BufWriter::with_capacity(8 * 1024 * 1024, out);
        // Header
        let header: Vec<String> = self.columns.iter().map(|c| csv_field(c).into_owned()).collect();
        writeln!(w, "{}", header.join(",")).map_err(|e| format!("save: {}", e))?;

        let nrows = self.nrows();
        let ncols = self.ncols();
        if nrows == 0 { return Ok(()); }

        // Phase 1: parallel per-column string building
        // Each column produces a Vec<u8> buffer with values separated by \0 sentinel,
        // plus a Vec<u32> of offsets for O(1) row lookup.
        struct ColStrings {
            buf: Vec<u8>,
            offsets: Vec<u32>, // offsets[i] = start of row i in buf; offsets[nrows] = end
        }
        let col_strings: Vec<ColStrings> = (0..ncols).into_par_iter().map(|ci| {
            let valid = self.validity[ci].as_ref();
            let col = self.data[ci].as_ref();
            // Estimate capacity: ~10 bytes per cell
            let mut buf = Vec::with_capacity(nrows * 10);
            let mut offsets = Vec::with_capacity(nrows + 1);
            match col {
                Column::Int(v) => {
                    for row in 0..nrows {
                        offsets.push(buf.len() as u32);
                        if valid[row] {
                            let mut tmp = itoa::Buffer::new();
                            buf.extend_from_slice(tmp.format(v[row]).as_bytes());
                        }
                    }
                }
                Column::Float(v) => {
                    for row in 0..nrows {
                        offsets.push(buf.len() as u32);
                        if valid[row] {
                            let mut tmp = ryu::Buffer::new();
                            buf.extend_from_slice(tmp.format(v[row]).as_bytes());
                        }
                    }
                }
                Column::Str(v) => {
                    for row in 0..nrows {
                        offsets.push(buf.len() as u32);
                        if valid[row] {
                            buf.extend_from_slice(csv_field(&v[row]).as_bytes());
                        }
                    }
                }
                Column::Bool(v) => {
                    for row in 0..nrows {
                        offsets.push(buf.len() as u32);
                        if valid[row] {
                            buf.extend_from_slice(if v[row] { b"true" } else { b"false" });
                        }
                    }
                }
                Column::DateTime(_, _) => {
                    for row in 0..nrows {
                        offsets.push(buf.len() as u32);
                        if valid[row] {
                            buf.extend_from_slice(col.fmt_cell(row, true).as_bytes());
                        }
                    }
                }
            }
            offsets.push(buf.len() as u32);
            ColStrings { buf, offsets }
        }).collect();

        // Phase 2: parallel chunk assembly + sequential write
        // Divide rows into chunks, build each chunk's bytes in parallel, write in order.
        const CHUNK_SIZE: usize = 65536;
        let nchunks = (nrows + CHUNK_SIZE - 1) / CHUNK_SIZE;
        let chunk_ranges: Vec<(usize, usize)> = (0..nchunks)
            .map(|i| (i * CHUNK_SIZE, std::cmp::min((i + 1) * CHUNK_SIZE, nrows)))
            .collect();

        let chunks: Vec<Vec<u8>> = chunk_ranges.par_iter().map(|&(start, end)| {
            let chunk_len = end - start;
            let mut out = Vec::with_capacity(chunk_len * 50);
            for row in start..end {
                for ci in 0..ncols {
                    if ci > 0 { out.push(b','); }
                    let cs = &col_strings[ci];
                    let s = cs.offsets[row] as usize;
                    let e = cs.offsets[row + 1] as usize;
                    out.extend_from_slice(&cs.buf[s..e]);
                }
                out.push(b'\n');
            }
            out
        }).collect();

        for chunk in &chunks {
            w.write_all(chunk).map_err(|e| format!("save: {}", e))?;
        }
        w.flush().map_err(|e| format!("save: {}", e))?;
        Ok(())
    }

    /// The column's distinct present cells, first seen first, in the
    /// column's own kind. (They had come out as text, so an int column's
    /// could not be added to; and a scan that gave up after a long stretch
    /// without a new cell could miss one further on.)
    pub fn unique_col(&self, col_name: &str) -> Result<CrowColumn, String> {
        let idx = self.col_index(col_name)
            .ok_or_else(|| format!("unique: column '{}' not found", col_name))?;
        let codes = codes(&self.data[idx], &self.validity[idx]);
        let mut first_rows = Vec::new();
        for (row, &code) in codes.iter().enumerate() {
            if code != u32::MAX && code as usize == first_rows.len() {
                first_rows.push(row);
            }
        }
        let n = first_rows.len();
        Ok(CrowColumn { data: Arc::new(take_rows(&self.data[idx], &first_rows)), valid: Arc::new(Validity::all(n)) })
    }

    /// `stats[frame]`: each numeric column's count, mean, population
    /// standard deviation, min, lower quartile, median, upper quartile and
    /// max — what pandas's `describe` shows, in Crow's words.
    pub fn stats_summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("{:<15} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}", "column", "count", "mean", "std", "min", "p25", "median", "p75", "max"));
        lines.push("-".repeat(98));
        for (ci, col_name) in self.columns.iter().enumerate() {
            let vals: Vec<f64> = match self.data[ci].as_ref() {
                Column::Int(v) => v.iter().enumerate().filter(|(i, _)| self.validity[ci][*i]).map(|(_, &x)| x as f64).collect(),
                Column::Float(v) => v.iter().enumerate().filter(|(i, _)| self.validity[ci][*i]).map(|(_, &x)| x).filter(|x| !x.is_nan()).collect(),
                _ => continue,
            };
            if vals.is_empty() {
                continue;
            }
            let count = vals.len();
            let mean = vals.iter().sum::<f64>() / count as f64;
            let std = crate::verbs::welford_var(&vals).sqrt();
            let mut sorted = vals.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let (min, max) = (sorted[0], sorted[count - 1]);
            let (p25, median, p75) = (crate::verbs::percentile_sorted(&sorted, 0.25), crate::verbs::median_sorted(&sorted), crate::verbs::percentile_sorted(&sorted, 0.75));
            lines.push(format!("{:<15} {:>8} {:>10.4} {:>10.4} {:>10.4} {:>10.4} {:>10.4} {:>10.4} {:>10.4}", col_name, count, mean, std, min, p25, median, p75, max));
        }
        lines.join("\n")
    }
}

// ===== Display =====

impl fmt::Display for CrowFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nrows = self.nrows();
        let ncols = self.ncols();
        if ncols == 0 {
            return write!(f, "Frame(0 rows, 0 cols)");
        }

        // Compute column widths
        let max_display = 10; // show first 5 and last 5 if > 10 rows
        let show_rows: Vec<usize> = if nrows <= max_display {
            (0..nrows).collect()
        } else {
            let mut rows: Vec<usize> = (0..5).collect();
            rows.push(usize::MAX); // sentinel for "..."
            rows.extend(nrows-5..nrows);
            rows
        };

        // Build cell strings: header + data rows
        let mut col_widths: Vec<usize> = self.columns.iter().map(|c| c.len()).collect();
        let mut row_cells: Vec<Vec<String>> = Vec::new();
        for &row in &show_rows {
            let mut cells = Vec::new();
            for ci in 0..ncols {
                let cell = if row == usize::MAX {
                    "...".to_string()
                } else if self.validity[ci][row] {
                    self.data[ci].fmt_cell(row, true)
                } else {
                    "nan".to_string()
                };
                if cell.len() > col_widths[ci] {
                    col_widths[ci] = cell.len();
                }
                cells.push(cell);
            }
            row_cells.push(cells);
        }

        // Cap column widths at 20
        for w in col_widths.iter_mut() {
            if *w > 20 { *w = 20; }
        }

        // Print header
        let header: Vec<String> = self.columns.iter().enumerate()
            .map(|(i, c)| format!("{:>width$}", c, width = col_widths[i]))
            .collect();
        writeln!(f, "{}", header.join("  "))?;

        // Print dtype row
        let dtypes: Vec<String> = self.data.iter().enumerate()
            .map(|(i, c)| format!("{:>width$}", c.dtype_name(), width = col_widths[i]))
            .collect();
        writeln!(f, "{}", dtypes.join("  "))?;

        // Separator
        let sep: Vec<String> = col_widths.iter().map(|&w| "-".repeat(w)).collect();
        writeln!(f, "{}", sep.join("  "))?;

        // Data rows
        for cells in &row_cells {
            let row_str: Vec<String> = cells.iter().enumerate()
                .map(|(i, c)| {
                    let truncated = if c.len() > col_widths[i] {
                        format!("{}...", &c[..col_widths[i]-3])
                    } else {
                        c.clone()
                    };
                    format!("{:>width$}", truncated, width = col_widths[i])
                })
                .collect();
            writeln!(f, "{}", row_str.join("  "))?;
        }

        write!(f, "\n[{} rows x {} cols]", nrows, ncols)
    }
}


/// Convert a CrowFrame to Arrow IPC stream bytes.
///
/// Each Column maps to an Arrow array:
/// - Column::Int → Int64Array
/// - Column::Float → Float64Array
/// - Column::Str → StringArray (Utf8)
/// - Column::Bool → BooleanArray
/// - Column::DateTime → TimestampMillisecondArray
///
/// The validity bitmap is applied as null masks.
#[cfg(feature = "arrow")]
pub fn to_arrow_ipc(frame: &CrowFrame) -> Result<Vec<u8>, String> {
    use arrow::ipc::writer::StreamWriter;
    let batch = record_batch(frame)?;
    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, &batch.schema()).map_err(|e| format!("arrow: {}", e))?;
        writer.write(&batch).map_err(|e| format!("arrow: {}", e))?;
        writer.finish().map_err(|e| format!("arrow: {}", e))?;
    }
    Ok(buf)
}

/// A frame as one Arrow record batch, as the IPC stream and a Parquet file
/// carry it (the column mapping above).
#[cfg(feature = "arrow")]
pub fn record_batch(frame: &CrowFrame) -> Result<arrow::record_batch::RecordBatch, String> {
    use arrow::array::*;
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;

    let mut fields = Vec::new();
    let mut arrays: Vec<Arc<dyn Array>> = Vec::new();

    for (i, col_name) in frame.columns.iter().enumerate() {
        let col = frame.data[i].as_ref();
        let validity = frame.validity[i].as_ref();

        match col {
            Column::Int(vals) => {
                fields.push(Field::new(col_name, DataType::Int64, true));
                let nulls: Vec<bool> = validity.iter().copied().collect();
                let arr = Int64Array::from(
                    vals.iter().zip(nulls.iter())
                        .map(|(v, &valid)| if valid { Some(*v) } else { None })
                        .collect::<Vec<Option<i64>>>()
                );
                arrays.push(Arc::new(arr));
            }
            Column::Float(vals) => {
                fields.push(Field::new(col_name, DataType::Float64, true));
                let nulls: Vec<bool> = validity.iter().copied().collect();
                let arr = Float64Array::from(
                    vals.iter().zip(nulls.iter())
                        .map(|(v, &valid)| if valid { Some(*v) } else { None })
                        .collect::<Vec<Option<f64>>>()
                );
                arrays.push(Arc::new(arr));
            }
            Column::Str(vals) => {
                fields.push(Field::new(col_name, DataType::Utf8, true));
                let nulls: Vec<bool> = validity.iter().copied().collect();
                let arr = StringArray::from(
                    vals.iter().zip(nulls.iter())
                        .map(|(v, &valid)| if valid { Some(v) } else { None })
                        .collect::<Vec<Option<&str>>>()
                );
                arrays.push(Arc::new(arr));
            }
            Column::Bool(vals) => {
                fields.push(Field::new(col_name, DataType::Boolean, true));
                let nulls: Vec<bool> = validity.iter().copied().collect();
                let arr = BooleanArray::from(
                    vals.iter().zip(nulls.iter())
                        .map(|(v, &valid)| if valid { Some(*v) } else { None })
                        .collect::<Vec<Option<bool>>>()
                );
                arrays.push(Arc::new(arr));
            }
            Column::DateTime(millis, _offsets) => {
                fields.push(Field::new(col_name, DataType::Timestamp(TimeUnit::Millisecond, None), true));
                let nulls: Vec<bool> = validity.iter().copied().collect();
                let arr = TimestampMillisecondArray::from(
                    millis.iter().zip(nulls.iter())
                        .map(|(v, &valid)| if valid { Some(*v) } else { None })
                        .collect::<Vec<Option<i64>>>()
                );
                arrays.push(Arc::new(arr));
            }
        }
    }

    // a frame's columns are one length, so this fails only for a frame
    // built wrongly: its reason, not a panic
    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, arrays).map_err(|e| format!("arrow: {}", e))
}

// ===== JSON and Parquet tables (spec, frame: load and save) =====

/// A frame from JSON: a list of records, a row each, or an object holding
/// exactly one such list (an API's `{"results": [...]}`). A column is every
/// key a record holds, in name order (as a map's keys are); its cells are ints, numbers
/// (an int among floats is a float), bools or text, and a record that
/// lacks the key, or holds `null`, has a missing cell there. A column whose
/// values are of mixed kinds, or lists or objects, is their JSON text.
pub fn from_json(text: &str) -> Result<CrowFrame, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("load: not JSON: {}", e))?;
    of_records(&value)
}

/// A frame from JSON records already parsed, by `from_json`'s rules: what
/// a page's data source answers arrives this way, through the session.
pub fn of_records(value: &serde_json::Value) -> Result<CrowFrame, String> {
    use serde_json::Value;
    let records: &Vec<Value> = match value {
        Value::Array(items) => items,
        Value::Object(map) => {
            let lists: Vec<(&String, &Vec<Value>)> = map
                .iter()
                .filter_map(|(k, v)| match v {
                    Value::Array(items) if items.iter().all(Value::is_object) => Some((k, items)),
                    _ => None,
                })
                .collect();
            match lists.as_slice() {
                [(_, items)] => items,
                [] => return Err("load: the JSON holds no list of records — a table is a list of objects, or an object holding one".to_string()),
                many => {
                    let names: Vec<String> = many.iter().map(|(k, _)| format!("'{}'", k)).collect();
                    return Err(format!("load: the JSON holds more than one list of records ({}) — read it with json[read[path]] and take the one wanted", names.join(", ")));
                }
            }
        }
        _ => return Err("load: the JSON is not a list of records".to_string()),
    };
    let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (i, record) in records.iter().enumerate() {
        let Value::Object(fields) = record else {
            return Err(format!("load: item {} of the JSON's list is not a record", i));
        };
        keys.extend(fields.keys().cloned());
    }
    let names: Vec<String> = keys.into_iter().collect();
    let mut columns = Vec::with_capacity(names.len());
    let mut validity = Vec::with_capacity(names.len());
    for name in &names {
        let cells: Vec<Option<&Value>> = records.iter().map(|r| r.get(name).filter(|v| !v.is_null())).collect();
        let valid: Vec<bool> = cells.iter().map(Option::is_some).collect();
        let present = || cells.iter().flatten();
        let column = if present().all(|v| v.is_i64()) {
            Column::Int(cells.iter().map(|c| c.and_then(Value::as_i64).unwrap_or(0)).collect())
        } else if present().all(|v| v.is_number()) {
            Column::Float(cells.iter().map(|c| c.and_then(Value::as_f64).unwrap_or(f64::NAN)).collect())
        } else if present().all(|v| v.is_boolean()) {
            Column::Bool(cells.iter().map(|c| c.and_then(Value::as_bool).unwrap_or(false)).collect())
        } else {
            Column::Str(
                cells
                    .iter()
                    .map(|c| match c {
                        Some(Value::String(s)) => s.clone(),
                        Some(v) => v.to_string(),
                        None => String::new(),
                    })
                    .collect(),
            )
        };
        columns.push(column);
        validity.push(valid);
    }
    Ok(CrowFrame::new(names, columns, validity))
}

/// A frame as JSON: a list of records, a missing cell `null`, a moment its
/// ISO text, a float that is not finite `null`.
pub fn to_json(frame: &CrowFrame) -> String {
    records(frame).to_string()
}

/// A frame's records as a JSON value (`to_json`'s form): what a route
/// answers for `json[frame]`, in both modes.
pub fn records(frame: &CrowFrame) -> serde_json::Value {
    use serde_json::{Map, Value};
    let rows: Vec<Value> = (0..frame.nrows())
        .map(|r| {
            let mut record = Map::new();
            for (i, name) in frame.columns.iter().enumerate() {
                let cell = if !frame.validity[i][r] {
                    Value::Null
                } else {
                    match frame.data[i].as_ref() {
                        Column::Int(v) => Value::from(v[r]),
                        Column::Float(v) => crate::json::float_to_json(v[r]),
                        Column::Bool(v) => Value::Bool(v[r]),
                        Column::Str(v) => Value::String(v[r].to_string()),
                        Column::DateTime(ms, offsets) => Value::String(format_datetime(ms[r], offsets[r])),
                    }
                };
                record.insert(name.clone(), cell);
            }
            Value::Object(record)
        })
        .collect();
    Value::Array(rows)
}

/// A frame from a Parquet file's bytes: integers are ints, floating and
/// decimal numbers floats, text (and dictionary-encoded text) text, bools
/// bools, timestamps and dates moments, and a null a missing cell. A
/// column of another kind (a list, a struct, bytes) is refused, named.
#[cfg(feature = "parquet")]
pub fn from_parquet(bytes: Vec<u8>) -> Result<CrowFrame, String> {
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{DataType, Float64Type, Int64Type, TimeUnit, TimestampMillisecondType};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes)).map_err(|e| format!("load: not a Parquet file: {}", e))?;
    let schema = builder.schema().clone();
    // each column's kind, and the Arrow type it is read as
    let mut kinds: Vec<(&'static str, DataType)> = Vec::new();
    for field in schema.fields() {
        let kind = match field.data_type() {
            DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 | DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => ("int", DataType::Int64),
            DataType::Float16 | DataType::Float32 | DataType::Float64 | DataType::Decimal128(..) | DataType::Decimal256(..) => ("float", DataType::Float64),
            DataType::Boolean => ("bool", DataType::Boolean),
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => ("text", DataType::Utf8),
            DataType::Dictionary(_, v) if matches!(v.as_ref(), DataType::Utf8 | DataType::LargeUtf8) => ("text", DataType::Utf8),
            // an instant stays the instant it is: a zoned timestamp keeps
            // its zone and only its unit changes (a cast to no zone reads
            // the zone's wall clock), and a date is its midnight in UTC
            DataType::Timestamp(_, tz) => ("moment", DataType::Timestamp(TimeUnit::Millisecond, tz.clone())),
            DataType::Date32 | DataType::Date64 => ("moment", DataType::Timestamp(TimeUnit::Millisecond, None)),
            other => return Err(format!("load: column '{}' is {} — a frame's cells are numbers, text, bools and moments", field.name(), other)),
        };
        kinds.push(kind);
    }
    let reader = builder.build().map_err(|e| format!("load: {}", e))?;
    let mut columns: Vec<Column> = kinds
        .iter()
        .map(|(k, _)| match *k {
            "int" => Column::Int(Vec::new()),
            "float" => Column::Float(Vec::new()),
            "bool" => Column::Bool(Vec::new()),
            "text" => Column::Str(Text::new()),
            _ => Column::DateTime(Vec::new(), Vec::new()),
        })
        .collect();
    let mut validity: Vec<Vec<bool>> = vec![Vec::new(); kinds.len()];
    let options = arrow::compute::CastOptions { safe: false, ..Default::default() };
    for batch in reader {
        let batch = batch.map_err(|e| format!("load: {}", e))?;
        for (i, array) in batch.columns().iter().enumerate() {
            let name = schema.field(i).name();
            let cast = arrow::compute::cast_with_options(array, &kinds[i].1, &options).map_err(|e| format!("load: column '{}': {}", name, e))?;
            validity[i].extend((0..cast.len()).map(|r| cast.is_valid(r)));
            match &mut columns[i] {
                Column::Int(v) => v.extend(cast.as_primitive::<Int64Type>().values().iter().copied()),
                Column::Float(v) => v.extend(cast.as_primitive::<Float64Type>().values().iter().copied()),
                Column::Bool(v) => {
                    let b = cast.as_boolean();
                    v.extend((0..b.len()).map(|r| b.value(r)));
                }
                Column::Str(v) => {
                    let s = cast.as_string::<i32>();
                    for r in 0..s.len() {
                        v.push(if s.is_valid(r) { s.value(r) } else { "" });
                    }
                }
                Column::DateTime(ms, offsets) => {
                    let t = cast.as_primitive::<TimestampMillisecondType>();
                    ms.extend(t.values().iter().copied());
                    offsets.extend(std::iter::repeat(0).take(t.len()));
                }
            }
        }
    }
    let names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    Ok(CrowFrame::new(names, columns, validity))
}

/// A frame as a Parquet file's bytes, compressed with Snappy (what pandas
/// and Spark write by default, so every reader reads it).
#[cfg(feature = "parquet")]
pub fn to_parquet(frame: &CrowFrame) -> Result<Vec<u8>, String> {
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    let batch = record_batch(frame)?;
    let props = WriterProperties::builder().set_compression(Compression::SNAPPY).build();
    let mut out = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut out, batch.schema(), Some(props)).map_err(|e| format!("save: {}", e))?;
    writer.write(&batch).map_err(|e| format!("save: {}", e))?;
    writer.close().map_err(|e| format!("save: {}", e))?;
    Ok(out)
}

#[cfg(test)]
mod format_tests {
    use super::*;

    /// JSON records are rows: columns in name order, an int among floats a
    /// float, a key a record lacks (or its null) a missing cell, a nested
    /// value its JSON text; an API's object holding one list of records is
    /// that list, and one holding two is refused, naming them
    #[test]
    fn json_records_are_rows() {
        let f = from_json(r#"{"status": "OK", "results": [{"t": 1, "c": 10.5, "v": 100, "x": [1]}, {"t": 2, "c": 11, "x": {"a": 1}, "v": null}]}"#).unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(f.columns, vec!["c", "t", "v", "x"]);
        assert!(matches!(f.data[0].as_ref(), Column::Float(v) if v == &vec![10.5, 11.0]));
        assert!(matches!(f.data[1].as_ref(), Column::Int(v) if v == &vec![1, 2]));
        assert_eq!(f.validity[2].to_vec(), vec![true, false]);
        assert!(matches!(f.data[3].as_ref(), Column::Str(v) if v.to_vec() == vec!["[1]".to_string(), "{\"a\":1}".to_string()]));
        let two = from_json(r#"{"a": [{"x": 1}], "b": [{"y": 2}]}"#).err().unwrap_or_default();
        assert!(two.contains("more than one list of records ('a', 'b')"), "{}", two);
        assert!(from_json("[1, 2]").err().unwrap_or_default().contains("item 0 of the JSON's list is not a record"));
        assert!(from_json("{\"n\": 3}").err().unwrap_or_default().contains("holds no list of records"));
        assert!(from_json("[").err().unwrap_or_default().starts_with("load: not JSON"));
        let empty = from_json("[]").unwrap_or_else(|e| panic!("{}", e));
        assert_eq!((empty.nrows(), empty.ncols()), (0, 0));
    }

    fn sample() -> CrowFrame {
        CrowFrame::new(
            vec!["sym".into(), "price".into(), "shares".into(), "open".into(), "at".into()],
            vec![
                Column::strs(vec!["AAPL".into(), "".into(), "NVDA".into()]),
                Column::Float(vec![187.5, 402.25, f64::NAN]),
                Column::Int(vec![10, 5, 40]),
                Column::Bool(vec![true, false, true]),
                Column::DateTime(vec![1_784_035_800_000, 0, 1_784_035_860_000], vec![0, 0, 0]),
            ],
            vec![vec![true, false, true], vec![true, true, false], vec![true, true, true], vec![true, true, true], vec![true, false, true]],
        )
    }

    /// A frame written as JSON reads back as the frame it was, its columns
    /// in name order
    #[test]
    fn a_frame_round_trips_through_json() {
        let f = sample();
        let back = from_json(&to_json(&f)).unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(back.columns, vec!["at", "open", "price", "shares", "sym"]);
        assert_eq!(back.validity[4].to_vec(), vec![true, false, true]);
        assert!(matches!(back.data[3].as_ref(), Column::Int(v) if v == &vec![10, 5, 40]));
        assert!(matches!(back.data[1].as_ref(), Column::Bool(v) if v == &vec![true, false, true]));
    }

    /// A frame written as Parquet reads back as the frame it was: its
    /// columns in their order, each of its kind, a missing cell missing
    #[cfg(feature = "parquet")]
    #[test]
    fn a_frame_round_trips_through_parquet() {
        let f = sample();
        let bytes = to_parquet(&f).unwrap_or_else(|e| panic!("{}", e));
        assert!(bytes.starts_with(b"PAR1"));
        let back = from_parquet(bytes).unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(back.columns, f.columns);
        for i in 0..f.ncols() {
            assert_eq!(back.validity[i].as_ref(), f.validity[i].as_ref(), "column {}", f.columns[i]);
        }
        assert!(matches!(back.data[0].as_ref(), Column::Str(v) if &v[0] == "AAPL" && &v[2] == "NVDA"));
        assert!(matches!(back.data[1].as_ref(), Column::Float(v) if v[0] == 187.5 && v[1] == 402.25));
        assert!(matches!(back.data[2].as_ref(), Column::Int(v) if v == &vec![10, 5, 40]));
        assert!(matches!(back.data[3].as_ref(), Column::Bool(v) if v == &vec![true, false, true]));
        assert!(matches!(back.data[4].as_ref(), Column::DateTime(ms, _) if ms[0] == 1_784_035_800_000 && ms[2] == 1_784_035_860_000));
        assert!(from_parquet(b"not parquet".to_vec()).err().unwrap_or_default().starts_with("load: not a Parquet file"));
    }
}

#[cfg(feature = "arrow")]
#[cfg(test)]
mod arrow_tests {
    use super::*;

    #[test]
    fn test_to_arrow_ipc_basic() {
        let frame = CrowFrame::new(
            vec!["id".into(), "score".into(), "name".into()],
            vec![
                Column::Int(vec![1, 2, 3]),
                Column::Float(vec![1.1, 2.2, 3.3]),
                Column::strs(vec!["a".into(), "b".into(), "c".into()]),
            ],
            vec![
                vec![true, true, true],
                vec![true, true, true],
                vec![true, true, true],
            ],
        );
        let bytes = to_arrow_ipc(&frame).expect("the frame encodes");
        // Arrow IPC starts with magic bytes "ARROW1"
        assert!(!bytes.is_empty(), "Arrow IPC bytes should not be empty");
        // Verify it starts with the IPC stream continuation marker (0xFFFFFFFF)
        assert!(bytes.len() > 8, "Arrow IPC should have header");
    }

    #[test]
    fn test_to_arrow_ipc_with_nulls() {
        let frame = CrowFrame::new(
            vec!["val".into()],
            vec![Column::Int(vec![10, 0, 30])],
            vec![vec![true, false, true]],
        );
        let bytes = to_arrow_ipc(&frame).expect("the frame encodes");
        assert!(!bytes.is_empty());
    }
}
#[cfg(test)]
mod inference_tests {
    use super::*;

    fn column_of(values: &[String]) -> Column {
        let raw: Vec<&str> = values.iter().map(|s| s.as_str()).collect();
        let valid = vec![true; raw.len()];
        infer_and_convert_refs(&raw, &valid)
    }

    /// A value past the sample decides with the rest: a column the sample
    /// took for ints read "n/a" as 0, one it took for dates panicked.
    #[test]
    fn the_whole_column_decides_its_type() {
        let mut ints: Vec<String> = (0..150).map(|i| i.to_string()).collect();
        assert!(matches!(column_of(&ints), Column::Int(_)));
        ints.push("2.5".to_string());
        assert!(matches!(column_of(&ints), Column::Float(ref v) if v[150] == 2.5));
        ints.push("n/a".to_string());
        assert!(matches!(column_of(&ints), Column::Str(v) if &v[151] == "n/a"));
        let mut dates: Vec<String> = (0..150).map(|i| format!("2024-01-{:02}", 1 + i % 28)).collect();
        assert!(matches!(column_of(&dates), Column::DateTime(..)));
        dates.push("soon".to_string());
        assert!(matches!(column_of(&dates), Column::Str(_)));
    }

    /// A misspelled aggregated column is refused with its name before the
    /// work is shared out: it panicked a worker thread.
    #[test]
    fn a_group_by_a_column_the_frame_lacks_is_refused() {
        let frame = CrowFrame::new(
            vec!["sym".into(), "px".into()],
            vec![Column::strs(vec!["a".into(), "b".into()]), Column::Float(vec![1.0, 2.0])],
            vec![vec![true, true], vec![true, true]],
        );
        assert!(frame.group("sym", &[("px", AggFn::Sum)]).is_ok());
        assert_eq!(frame.group("sym", &[("pxx", AggFn::Sum)]).err(), Some("group: column 'pxx' not found".to_string()));
    }
}

#[cfg(test)]
mod numbers_tests {
    use super::*;

    /// A column where a builtin takes numbers: its numbers, or the reason
    /// it has none, found row by row as the interpreter reads the cells.
    /// (Compiled, a missing cell was read as 0 and text as zeros.)
    #[test]
    fn a_columns_numbers_or_the_reason() {
        let frame = CrowFrame::new(
            vec!["n".into(), "x".into(), "gap".into(), "name".into(), "flag".into()],
            vec![
                Column::Int(vec![1, 2]),
                Column::Float(vec![0.5, f64::NAN]),
                Column::Float(vec![1.0, 0.0]),
                Column::strs(vec!["a".into(), "b".into()]),
                Column::Bool(vec![true, false]),
            ],
            vec![vec![true, true], vec![true, true], vec![true, false], vec![true, true], vec![true, true]],
        );
        let numbers = |c: &str| frame.col_native(c).expect("the column").numbers().map(|t| t.to_vec());
        assert_eq!(numbers("n"), Ok(vec![1.0, 2.0]));
        assert!(numbers("x").expect("a present NaN is a number")[1].is_nan());
        assert_eq!(numbers("gap"), Err("row 1 of the column is missing — fill the column first: fill[frame, 'column', value]".to_string()));
        assert_eq!(numbers("name"), Err("a column of text is not numbers".to_string()));
        assert_eq!(numbers("flag"), Err("a column of bools is not numbers".to_string()));
    }
}

#[cfg(test)]
mod datetime_tests {
    use super::parse_datetime;

    /// A moment as Postgres and ClickHouse write one: a fraction of a
    /// second kept to the millisecond, an offset of hours alone, or of
    /// hours and minutes without a colon
    #[test]
    fn a_datetime_with_a_fraction_or_a_short_offset_parses() {
        assert_eq!(parse_datetime("2024-01-02 03:04:05.123"), Some((1_704_164_645_123, 0)));
        assert_eq!(parse_datetime("2024-01-02T03:04:05.1Z"), Some((1_704_164_645_100, 0)));
        assert_eq!(parse_datetime("2024-01-02 03:04:05.123456"), Some((1_704_164_645_123, 0)));
        assert_eq!(parse_datetime("2024-01-01 19:04:05-08"), Some((1_704_164_645_000, -480)));
        assert_eq!(parse_datetime("2024-01-02 08:34:05+0530"), Some((1_704_164_645_000, 330)));
        assert_eq!(parse_datetime("2024-01-02 08:34:05+05:30"), Some((1_704_164_645_000, 330)));
        assert_eq!(parse_datetime("2024-01-02 03:04:05."), None);
        assert_eq!(parse_datetime("2024-01-02 03:04:05-8"), None);
    }

    /// A table written and read back keeps a cell with a comma or a quote
    /// in it (quoted, a quote inside written twice) and a column of bools;
    /// `True` and `FALSE`, as other writers spell them, read as bools too.
    #[test]
    fn a_table_with_quoted_cells_and_bools_is_read_back() {
        use super::{csv_frame, split_quoted, Column, CrowFrame};
        let f = CrowFrame::new(
            vec!["note".into(), "ok".into(), "n".into()],
            vec![Column::strs(vec!["a, b".into(), "say \"hi\"".into(), "plain".into()]), Column::Bool(vec![true, false, true]), Column::Int(vec![1, 2, 3])],
            vec![vec![true; 3], vec![true, false, true], vec![true; 3]],
        );
        let text = f.to_csv().expect("csv");
        assert_eq!(text, "note,ok,n\n\"a, b\",true,1\n\"say \"\"hi\"\"\",,2\nplain,true,3\n");
        assert_eq!(csv_frame(&text).expect("read back"), f);
        let spelled = csv_frame("flag,word\nTrue,x\nFALSE,\"y, z\"\n").expect("read");
        assert_eq!(spelled.data[0].as_ref(), &Column::Bool(vec![true, false]));
        assert_eq!(spelled.data[1].as_ref(), &Column::strs(vec!["x".into(), "y, z".into()]));
        assert_eq!(split_quoted("\"a,b\", c ,\"\"\"q\"\"\",d"), vec!["a,b", "c", "\"q\"", "d"]);
    }
}
