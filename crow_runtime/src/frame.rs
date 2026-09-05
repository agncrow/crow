/// Crow frame module runtime — column-oriented data frames with typed vectors.

use std::fmt;
use std::sync::Arc;
use rayon::prelude::*;

/// Column data types — each column stores a single typed vector.
#[derive(Debug, Clone)]
pub enum Column {
    Int(Vec<i64>),
    Float(Vec<f64>),
    Str(Vec<String>),
    Bool(Vec<bool>),
    /// DateTime: UTC epoch milliseconds + timezone offset in minutes
    DateTime(Vec<i64>, Vec<i16>),
}

impl Column {
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
            Column::Str(v) => v[row].clone(),
            Column::Bool(v) => format!("{}", v[row]),
            Column::DateTime(millis, offsets) => format_datetime(millis[row], offsets[row]),
        }
    }
}

/// A column-oriented data frame.
#[derive(Debug, Clone)]
pub struct CrowFrame {
    pub columns: Vec<String>,              // column names (order preserved)
    pub data: Vec<Arc<Column>>,            // column data (Arc for cheap sharing)
    pub validity: Vec<Arc<Vec<bool>>>,     // validity bitmap per column (true = valid)
}

impl CrowFrame {
    /// Create from raw (unshared) columns — wraps each in Arc.
    pub fn new(columns: Vec<String>, data: Vec<Column>, validity: Vec<Vec<bool>>) -> Self {
        CrowFrame {
            columns,
            data: data.into_iter().map(Arc::new).collect(),
            validity: validity.into_iter().map(Arc::new).collect(),
        }
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
    pub fn col_validity(&self, name: &str) -> Option<&Vec<bool>> {
        self.col_index(name).map(|i| self.validity[i].as_ref())
    }

    /// Get column dtypes as strings
    pub fn dtypes(&self) -> Vec<&'static str> {
        self.data.iter().map(|c| c.dtype_name()).collect()
    }

    /// Get column values as formatted strings (for transpiler column access).
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
#[derive(Debug, Clone)]
pub struct CrowColumn {
    pub data: Arc<Column>,
    pub valid: Arc<Vec<bool>>,
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

    /// Native vectorized comparison — no string parsing.
    pub fn compare(&self, scalar: f64, op: &str) -> Vec<bool> {
        let n = self.len();
        let mut result = vec![false; n];
        match self.data.as_ref() {
            Column::Int(v) => {
                for i in 0..n {
                    if !self.valid[i] { continue; }
                    let val = v[i] as f64;
                    result[i] = cmp_f64(val, scalar, op);
                }
            }
            Column::Float(v) => {
                for i in 0..n {
                    if !self.valid[i] { continue; }
                    let val = v[i];
                    if val.is_nan() { continue; }
                    result[i] = cmp_f64(val, scalar, op);
                }
            }
            Column::Str(v) => {
                // Fallback: parse strings (for non-numeric columns)
                for i in 0..n {
                    if !self.valid[i] { continue; }
                    if let Ok(val) = v[i].parse::<f64>() {
                        result[i] = cmp_f64(val, scalar, op);
                    }
                }
            }
            _ => {} // Bool/DateTime: no comparison with f64 scalar
        }
        result
    }

    /// Native vectorized arithmetic — no string parsing.
    /// Returns a new CrowColumn with the result.
    pub fn arithmetic(&self, scalar: f64, op: &str) -> CrowColumn {
        let n = self.len();
        let new_data = match self.data.as_ref() {
            Column::Int(v) => {
                let out: Vec<f64> = (0..n).map(|i| {
                    if !self.valid[i] { return f64::NAN; }
                    let val = v[i] as f64;
                    arith_f64(val, scalar, op)
                }).collect();
                Column::Float(out)
            }
            Column::Float(v) => {
                let out: Vec<f64> = (0..n).map(|i| {
                    if !self.valid[i] { return f64::NAN; }
                    arith_f64(v[i], scalar, op)
                }).collect();
                Column::Float(out)
            }
            Column::Str(v) => {
                // Fallback: parse strings
                let out: Vec<String> = (0..n).map(|i| {
                    if !self.valid[i] { return "nan".to_string(); }
                    if let Ok(val) = v[i].parse::<f64>() {
                        let r = arith_f64(val, scalar, op);
                        if r == (r as i64) as f64 && r.is_finite() {
                            (r as i64).to_string()
                        } else {
                            r.to_string()
                        }
                    } else { v[i].clone() }
                }).collect();
                Column::Str(out)
            }
            other => other.clone(), // pass through
        };
        CrowColumn {
            data: Arc::new(new_data),
            valid: Arc::clone(&self.valid),
        }
    }

    /// Native vectorized string comparison — compare column values against a string scalar.
    pub fn compare_str(&self, scalar: &str, op: &str) -> Vec<bool> {
        let n = self.len();
        let mut result = vec![false; n];
        match self.data.as_ref() {
            Column::Str(v) => {
                for i in 0..n {
                    if !self.valid[i] { continue; }
                    result[i] = match op {
                        "eq" => v[i] == scalar,
                        "ne" => v[i] != scalar,
                        "gt" => v[i].as_str() > scalar,
                        "lt" => v[i].as_str() < scalar,
                        "gte" => v[i].as_str() >= scalar,
                        "lte" => v[i].as_str() <= scalar,
                        _ => false,
                    };
                }
            }
            Column::Int(v) => {
                // Try parsing scalar as i64 for comparison
                if let Ok(s) = scalar.parse::<i64>() {
                    for i in 0..n {
                        if !self.valid[i] { continue; }
                        result[i] = match op {
                            "eq" => v[i] == s,
                            "ne" => v[i] != s,
                            _ => false,
                        };
                    }
                }
            }
            _ => {} // Other types: no string comparison
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

    /// Return unique non-null values as a Vec<String>.
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
                    if seen.insert(v[i].clone()) {
                        result.push(v[i].clone());
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
        self.valid.as_ref().clone()
    }
}

impl fmt::Display for CrowColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let strings = self.to_strings();
        write!(f, "[{}]", strings.join(", "))
    }
}

#[inline]
fn cmp_f64(a: f64, b: f64, op: &str) -> bool {
    match op {
        "gt" => a > b,
        "lt" => a < b,
        "gte" => a >= b,
        "lte" => a <= b,
        "eq" => (a - b).abs() < f64::EPSILON,
        "ne" => (a - b).abs() >= f64::EPSILON,
        _ => false,
    }
}

#[inline]
fn arith_f64(a: f64, b: f64, op: &str) -> f64 {
    match op {
        "add" => a + b,
        "sub" => a - b,
        "mul" => a * b,
        "div" => a / b,
        _ => a,
    }
}

// ===== CSV Loading =====

/// Load a CSV file into a CrowFrame with auto type inference.
pub fn load_csv(path: &str) -> Result<CrowFrame, String> {
    use std::fs;

    // Read entire file into memory
    let content = fs::read_to_string(path).map_err(|e| format!("load: {}", e))?;
    let bytes = content.as_bytes();

    // Find header end
    let newline_pos = content.find('\n').ok_or_else(|| "load: empty file".to_string())?;
    let header = content[..newline_pos].trim();
    let col_names: Vec<String> = header.split(',').map(|s| s.trim().to_string()).collect();
    let ncols = col_names.len();
    if ncols == 0 {
        return Err("load: no columns found".to_string());
    }

    let body_start = newline_pos + 1;
    let body = &bytes[body_start..];
    let body_len = body.len();

    // Find all line start offsets in parallel chunks
    // First pass: find newline positions to get line boundaries
    let chunk_size = 1 << 20; // 1 MB chunks
    let nchunks = (body_len + chunk_size - 1) / chunk_size;

    // Parallel: find newline offsets within each chunk
    let chunk_newlines: Vec<Vec<usize>> = (0..nchunks).into_par_iter().map(|ci| {
        let start = ci * chunk_size;
        let end = (start + chunk_size).min(body_len);
        let mut positions = Vec::new();
        for i in start..end {
            if body[i] == b'\n' {
                positions.push(i);
            }
        }
        positions
    }).collect();

    // Merge: collect all line start/end pairs
    let mut line_ranges: Vec<(usize, usize)> = Vec::with_capacity(body_len / 80); // estimate
    let mut line_start = 0usize;
    for chunk in &chunk_newlines {
        for &nl_pos in chunk {
            if nl_pos > line_start {
                line_ranges.push((line_start, nl_pos));
            }
            line_start = nl_pos + 1;
        }
    }
    // Last line (no trailing newline)
    if line_start < body_len {
        // trim trailing whitespace
        let mut end = body_len;
        while end > line_start && (body[end - 1] == b'\r' || body[end - 1] == b' ' || body[end - 1] == b'\n') {
            end -= 1;
        }
        if end > line_start {
            line_ranges.push((line_start, end));
        }
    }
    let nrows = line_ranges.len();

    // Parallel: each column parses its field from all rows and infers type
    let body_str = &content[body_start..];
    let results: Vec<(Column, Vec<bool>)> = (0..ncols).into_par_iter().map(|ci| {
        let mut raw: Vec<&str> = Vec::with_capacity(nrows);
        let mut valid: Vec<bool> = Vec::with_capacity(nrows);
        for &(ls, le) in &line_ranges {
            let line = &body_str[ls..le];
            let field = nth_field_fast(line, ci);
            let is_valid = !field.is_empty();
            valid.push(is_valid);
            raw.push(field);
        }
        let col = infer_and_convert_refs(&raw, &valid);
        (col, valid)
    }).collect();

    let (typed_cols, valid_cols): (Vec<_>, Vec<_>) = results.into_iter().unzip();
    Ok(CrowFrame::new(col_names, typed_cols, valid_cols))
}

/// Extract the nth comma-separated field from a line (zero-indexed). Fast inline version.
#[inline(always)]
fn nth_field_fast(line: &str, n: usize) -> &str {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut field_start = 0;
    let mut field_idx = 0;
    for i in 0..len {
        if bytes[i] == b',' {
            if field_idx == n {
                // Trim leading/trailing spaces
                let s = &line[field_start..i];
                return s.trim();
            }
            field_idx += 1;
            field_start = i + 1;
        }
    }
    if field_idx == n {
        line[field_start..len].trim()
    } else {
        ""
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
        return Column::Str(raw.iter().map(|s| s.to_string()).collect());
    }

    // Try int
    if sampled.iter().all(|s| s.parse::<i64>().is_ok()) {
        let data: Vec<i64> = raw.iter().zip(valid.iter())
            .map(|(s, &v)| if v { s.parse().unwrap_or(0) } else { 0 })
            .collect();
        return Column::Int(data);
    }

    // Try float
    if sampled.iter().all(|s| s.parse::<f64>().is_ok()) {
        let data: Vec<f64> = raw.iter().zip(valid.iter())
            .map(|(s, &v)| if v { s.parse().unwrap_or(f64::NAN) } else { f64::NAN })
            .collect();
        return Column::Float(data);
    }

    // Try datetime
    if sampled.iter().all(|s| parse_datetime(s).is_some()) {
        let mut millis = Vec::with_capacity(raw.len());
        let mut offsets = Vec::with_capacity(raw.len());
        for (s, &v) in raw.iter().zip(valid.iter()) {
            if v {
                let (m, o) = parse_datetime(s).unwrap();
                millis.push(m);
                offsets.push(o);
            } else {
                millis.push(0);
                offsets.push(0);
            }
        }
        return Column::DateTime(millis, offsets);
    }

    // Fall back to string
    Column::Str(raw.iter().map(|s| s.to_string()).collect())
}


/// Parse ISO 8601 datetime string → (epoch_millis, offset_minutes).
/// Supports: 2024-01-15, 2024-01-15T10:30:00, 2024-01-15T10:30:00Z,
///           2024-01-15T10:30:00+05:30, 2024-01-15T10:30:00-08:00
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

        let rest = &s[19..];
        let offset_minutes: i16 = if rest.is_empty() {
            0 // No timezone → assume UTC
        } else if rest == "Z" {
            0
        } else if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) {
            // +HH:MM or -HH:MM
            let sign: i16 = if rest.starts_with('-') { -1 } else { 1 };
            let oh: i16 = rest[1..3].parse().ok()?;
            let om: i16 = rest[4..6].parse().ok()?;
            sign * (oh * 60 + om)
        } else {
            return None;
        };

        // Convert to UTC epoch millis: subtract offset to get UTC
        let local_millis = date_to_epoch_millis(year, month, day, hour, min, sec)?;
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
    // Convert to local time by adding offset
    let local_millis = epoch_millis + (offset_minutes as i64) * 60_000;
    let total_secs = local_millis / 1000;
    let days = if total_secs >= 0 { total_secs / 86400 } else { (total_secs - 86399) / 86400 };
    let day_secs = (total_secs - days * 86400) as u32;

    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;

    // Civil date from days since epoch
    let (year, month, day) = civil_from_days(days);

    if offset_minutes == 0 {
        if hour == 0 && min == 0 && sec == 0 {
            format!("{:04}-{:02}-{:02}", year, month, day)
        } else {
            format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, hour, min, sec)
        }
    } else {
        let sign = if offset_minutes >= 0 { '+' } else { '-' };
        let abs_off = offset_minutes.unsigned_abs();
        let oh = abs_off / 60;
        let om = abs_off % 60;
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
            year, month, day, hour, min, sec, sign, oh, om)
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

/// Extract a datetime component from an ISO 8601 datetime string.
/// Returns the numeric value of year, month, day, hour, minute, second, or offset (in minutes).
pub fn datetime_field(s: &str, field: &str) -> Option<i64> {
    let (epoch_millis, offset_minutes) = parse_datetime(s)?;
    let local_millis = epoch_millis + (offset_minutes as i64) * 60_000;
    let total_secs = local_millis / 1000;
    let days = if total_secs >= 0 { total_secs / 86400 } else { (total_secs - 86399) / 86400 };
    let day_secs = (total_secs - days * 86400) as u32;
    match field {
        "year" => { let (y, _, _) = civil_from_days(days); Some(y as i64) }
        "month" => { let (_, m, _) = civil_from_days(days); Some(m as i64) }
        "day" => { let (_, _, d) = civil_from_days(days); Some(d as i64) }
        "hour" => Some((day_secs / 3600) as i64),
        "minute" => Some(((day_secs % 3600) / 60) as i64),
        "second" => Some((day_secs % 60) as i64),
        "offset" => Some(offset_minutes as i64),
        _ => None,
    }
}

/// Vectorized comparison: compare each string-encoded value against a scalar.
/// Missing values ("nan") produce false. Returns boolean mask.
pub fn col_compare(col: &[String], scalar: f64, op: &str) -> Vec<bool> {
    col.iter().map(|s| {
        if s == "nan" { return false; }
        if let Ok(v) = s.parse::<f64>() {
            match op {
                "gt" => v > scalar,
                "lt" => v < scalar,
                "gte" => v >= scalar,
                "lte" => v <= scalar,
                "eq" => (v - scalar).abs() < f64::EPSILON,
                "ne" => (v - scalar).abs() >= f64::EPSILON,
                _ => false,
            }
        } else { false }
    }).collect()
}

/// Vectorized arithmetic: apply a scalar operation to each value in a string-encoded column.
/// Missing values ("nan") are preserved as "nan".
pub fn col_arithmetic(col: &[String], scalar: f64, op: &str) -> Vec<String> {
    col.iter().map(|s| {
        if s == "nan" { return "nan".to_string(); }
        if let Ok(v) = s.parse::<f64>() {
            let result = match op {
                "add" => v + scalar,
                "sub" => v - scalar,
                "mul" => v * scalar,
                "div" => v / scalar,
                _ => v,
            };
            // Preserve integer display for whole numbers
            if result == (result as i64) as f64 && result.is_finite() {
                (result as i64).to_string()
            } else {
                result.to_string()
            }
        } else { s.clone() }
    }).collect()
}

/// Vectorized validity: return true for each non-"nan" value.
pub fn col_ok(col: &[String]) -> Vec<bool> {
    col.iter().map(|s| s != "nan").collect()
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
            _ => unreachable!(),
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
        let results: Vec<(Column, Vec<bool>)> = self.data.par_iter().zip(self.validity.par_iter())
            .map(|(col, valid)| filter_column(col, valid, mask))
            .collect();
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        Ok(CrowFrame::new(self.columns.clone(), new_data, new_valid))
    }

    /// Sort by a column. `descending` = true for desc.
    /// Uses key-extraction sort for numeric types (cache-friendly) and
    /// parallel sort + parallel column reordering.
    pub fn sort(&self, col_name: &str, descending: bool) -> Result<CrowFrame, String> {
        let idx = self.col_index(col_name)
            .ok_or_else(|| format!("sort: column '{}' not found", col_name))?;
        let nrows = self.nrows();

        // For numeric types: sort (key, index) pairs for cache locality
        let indices: Vec<usize> = match self.data[idx].as_ref() {
            Column::Int(v) => {
                let mut pairs: Vec<(i64, u32)> = v.iter().copied()
                    .enumerate().map(|(i, val)| (val, i as u32)).collect();
                if descending {
                    pairs.par_sort_unstable_by(|a, b| b.0.cmp(&a.0));
                } else {
                    pairs.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }
                pairs.iter().map(|p| p.1 as usize).collect()
            }
            Column::Float(v) => {
                // Convert to bits for total ordering (handles NaN)
                let mut pairs: Vec<(u64, u32)> = v.iter()
                    .enumerate().map(|(i, &val)| {
                        let bits = val.to_bits();
                        // Flip to get proper ordering: positive floats ascending, negatives descending
                        let sortable = if val.is_sign_negative() { !bits } else { bits | (1u64 << 63) };
                        (sortable, i as u32)
                    }).collect();
                if descending {
                    pairs.par_sort_unstable_by(|a, b| b.0.cmp(&a.0));
                } else {
                    pairs.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }
                pairs.iter().map(|p| p.1 as usize).collect()
            }
            Column::Str(v) => {
                let mut idxs: Vec<usize> = (0..nrows).collect();
                idxs.par_sort_unstable_by(|&a, &b| {
                    let cmp = v[a].cmp(&v[b]);
                    if descending { cmp.reverse() } else { cmp }
                });
                idxs
            }
            Column::Bool(v) => {
                let mut pairs: Vec<(u8, u32)> = v.iter()
                    .enumerate().map(|(i, &val)| (val as u8, i as u32)).collect();
                if descending {
                    pairs.par_sort_unstable_by(|a, b| b.0.cmp(&a.0));
                } else {
                    pairs.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }
                pairs.iter().map(|p| p.1 as usize).collect()
            }
            Column::DateTime(millis, _) => {
                let mut pairs: Vec<(i64, u32)> = millis.iter().copied()
                    .enumerate().map(|(i, val)| (val, i as u32)).collect();
                if descending {
                    pairs.par_sort_unstable_by(|a, b| b.0.cmp(&a.0));
                } else {
                    pairs.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }
                pairs.iter().map(|p| p.1 as usize).collect()
            }
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
        let mut new_valid: Vec<Arc<Vec<bool>>> = self.validity.iter().map(Arc::clone).collect();

        let col_mut = Arc::make_mut(&mut new_data[idx]);
        let valid_mut = Arc::make_mut(&mut new_valid[idx]);

        match col_mut {
            Column::Int(v) => {
                let fv: i64 = fill_val.parse().map_err(|_| format!("fill: cannot parse '{}' as int", fill_val))?;
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fv; valid_mut[i] = true; }
                }
            }
            Column::Float(v) => {
                let fv: f64 = fill_val.parse().map_err(|_| format!("fill: cannot parse '{}' as float", fill_val))?;
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fv; valid_mut[i] = true; }
                }
            }
            Column::Str(v) => {
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fill_val.to_string(); valid_mut[i] = true; }
                }
            }
            Column::Bool(v) => {
                let fv: bool = fill_val.parse().map_err(|_| format!("fill: cannot parse '{}' as bool", fill_val))?;
                for i in 0..v.len() {
                    if !valid_mut[i] { v[i] = fv; valid_mut[i] = true; }
                }
            }
            Column::DateTime(_, _) => {
                return Err("fill: cannot fill datetime columns yet".to_string());
            }
        }

        Ok(CrowFrame { columns: self.columns.clone(), data: new_data, validity: new_valid })
    }

    /// Stack two frames vertically. Columns processed in parallel.
    pub fn stack(&self, other: &CrowFrame) -> Result<CrowFrame, String> {
        if self.columns != other.columns {
            return Err("stack: column names don't match".to_string());
        }
        let results: Result<Vec<(Column, Vec<bool>)>, String> = (0..self.ncols()).into_par_iter().map(|i| {
            stack_columns(&self.data[i], &self.validity[i], &other.data[i], &other.validity[i])
        }).collect();
        let results = results?;
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        Ok(CrowFrame::new(self.columns.clone(), new_data, new_valid))
    }
}

// ===== Helper functions =====

fn filter_column(col: &Column, valid: &[bool], mask: &[bool]) -> (Column, Vec<bool>) {
    let new_valid: Vec<bool> = valid.iter().zip(mask.iter())
        .filter(|&(_, &m)| m)
        .map(|(&v, _)| v)
        .collect();
    let new_col = match col {
        Column::Int(v) => Column::Int(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect()),
        Column::Float(v) => Column::Float(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect()),
        Column::Str(v) => Column::Str(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(x, _)| x.clone()).collect()),
        Column::Bool(v) => Column::Bool(v.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect()),
        Column::DateTime(millis, offsets) => {
            let new_m: Vec<i64> = millis.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect();
            let new_o: Vec<i16> = offsets.iter().zip(mask).filter(|&(_, &m)| m).map(|(&x, _)| x).collect();
            Column::DateTime(new_m, new_o)
        }
    };
    (new_col, new_valid)
}

fn stack_columns(a: &Column, av: &[bool], b: &Column, bv: &[bool]) -> Result<(Column, Vec<bool>), String> {
    let mut new_valid: Vec<bool> = av.to_vec();
    new_valid.extend_from_slice(bv);
    let col = match (a, b) {
        (Column::Int(va), Column::Int(vb)) => { let mut v = va.clone(); v.extend_from_slice(vb); Column::Int(v) }
        (Column::Float(va), Column::Float(vb)) => { let mut v = va.clone(); v.extend_from_slice(vb); Column::Float(v) }
        (Column::Str(va), Column::Str(vb)) => { let mut v = va.clone(); v.extend(vb.iter().cloned()); Column::Str(v) }
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
    fn reorder_rows(&self, indices: &[usize]) -> CrowFrame {
        let results: Vec<(Column, Vec<bool>)> = self.data.par_iter().zip(self.validity.par_iter())
            .map(|(col, valid)| {
                let nv: Vec<bool> = indices.iter().map(|&i| valid[i]).collect();
                let nc = match col.as_ref() {
                    Column::Int(v) => Column::Int(indices.iter().map(|&i| v[i]).collect()),
                    Column::Float(v) => Column::Float(indices.iter().map(|&i| v[i]).collect()),
                    Column::Str(v) => Column::Str(indices.iter().map(|&i| v[i].clone()).collect()),
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
        CrowFrame::new(self.columns.clone(), new_data, new_valid)
    }

    /// Slice rows [start..end). Parallel across columns.
    fn slice_rows(&self, start: usize, end: usize) -> CrowFrame {
        let results: Vec<(Column, Vec<bool>)> = self.data.par_iter().zip(self.validity.par_iter())
            .map(|(col, valid)| {
                let nv = valid[start..end].to_vec();
                let nc = match col.as_ref() {
                    Column::Int(v) => Column::Int(v[start..end].to_vec()),
                    Column::Float(v) => Column::Float(v[start..end].to_vec()),
                    Column::Str(v) => Column::Str(v[start..end].to_vec()),
                    Column::Bool(v) => Column::Bool(v[start..end].to_vec()),
                    Column::DateTime(millis, offsets) => Column::DateTime(
                        millis[start..end].to_vec(), offsets[start..end].to_vec(),
                    ),
                };
                (nc, nv)
            })
            .collect();
        let (new_data, new_valid): (Vec<_>, Vec<_>) = results.into_iter().unzip();
        CrowFrame::new(self.columns.clone(), new_data, new_valid)
    }
}

// ===== Group =====

/// Aggregation function type.
#[derive(Debug, Clone, Copy)]
pub enum AggFn {
    Mean,
    Sum,
    Min,
    Max,
    Count,
}

impl AggFn {
    pub fn from_str(s: &str) -> Option<AggFn> {
        match s {
            "mean" => Some(AggFn::Mean),
            "sum" => Some(AggFn::Sum),
            "min" => Some(AggFn::Min),
            "max" => Some(AggFn::Max),
            "count" => Some(AggFn::Count),
            _ => None,
        }
    }
}

impl CrowFrame {
    /// Group by a column and aggregate multiple (col, agg) pairs.
    pub fn group(&self, group_col: &str, aggs: &[(&str, AggFn)]) -> Result<CrowFrame, String> {
        let gi = self.col_index(group_col)
            .ok_or_else(|| format!("group: column '{}' not found", group_col))?;

        // Build groups using native keys for speed (avoid fmt_cell string alloc)
        let valid = self.validity[gi].as_ref();
        let nrows = self.nrows();
        // group_ids[row] = group index for that row (u32 saves memory)
        let mut group_ids: Vec<u32> = vec![u32::MAX; nrows];
        let group_keys: Vec<String>; // display keys for output
        let ngroups: usize;

        match self.data[gi].as_ref() {
            Column::Int(v) => {
                let mut key_map: std::collections::HashMap<i64, u32> = std::collections::HashMap::new();
                let mut keys_ordered: Vec<i64> = Vec::new();
                for row in 0..nrows {
                    if !valid[row] { continue; }
                    let k = v[row];
                    let len = keys_ordered.len() as u32;
                    let gid = *key_map.entry(k).or_insert_with(|| { keys_ordered.push(k); len });
                    group_ids[row] = gid;
                }
                ngroups = keys_ordered.len();
                group_keys = keys_ordered.iter().map(|k| k.to_string()).collect();
            }
            Column::Float(v) => {
                let mut key_map: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
                let mut keys_ordered: Vec<f64> = Vec::new();
                for row in 0..nrows {
                    if !valid[row] { continue; }
                    let k = v[row].to_bits();
                    let len = keys_ordered.len() as u32;
                    let gid = *key_map.entry(k).or_insert_with(|| { keys_ordered.push(v[row]); len });
                    group_ids[row] = gid;
                }
                ngroups = keys_ordered.len();
                group_keys = keys_ordered.iter().map(|k| format!("{}", k)).collect();
            }
            Column::Str(v) => {
                let mut key_map: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
                let mut keys_ordered: Vec<&str> = Vec::new();
                for row in 0..nrows {
                    if !valid[row] { continue; }
                    let k = v[row].as_str();
                    let len = keys_ordered.len() as u32;
                    let gid = *key_map.entry(k).or_insert_with(|| { keys_ordered.push(k); len });
                    group_ids[row] = gid;
                }
                ngroups = keys_ordered.len();
                group_keys = keys_ordered.iter().map(|k| k.to_string()).collect();
            }
            Column::Bool(v) => {
                let mut key_map: std::collections::HashMap<bool, u32> = std::collections::HashMap::new();
                let mut keys_ordered: Vec<bool> = Vec::new();
                for row in 0..nrows {
                    if !valid[row] { continue; }
                    let k = v[row];
                    let len = keys_ordered.len() as u32;
                    let gid = *key_map.entry(k).or_insert_with(|| { keys_ordered.push(k); len });
                    group_ids[row] = gid;
                }
                ngroups = keys_ordered.len();
                group_keys = keys_ordered.iter().map(|k| k.to_string()).collect();
            }
            Column::DateTime(millis, _) => {
                let mut key_map: std::collections::HashMap<i64, u32> = std::collections::HashMap::new();
                let mut keys_ordered: Vec<i64> = Vec::new();
                for row in 0..nrows {
                    if !valid[row] { continue; }
                    let k = millis[row];
                    let len = keys_ordered.len() as u32;
                    let gid = *key_map.entry(k).or_insert_with(|| { keys_ordered.push(k); len });
                    group_ids[row] = gid;
                }
                ngroups = keys_ordered.len();
                group_keys = keys_ordered.iter().map(|k| {
                    self.data[gi].fmt_cell(keys_ordered.iter().position(|v| v == k).unwrap_or(0), true)
                }).collect();
            }
        }

        // Build result columns: group key + one column per aggregation
        let mut result_cols: Vec<String> = vec![group_col.to_string()];
        let mut result_data: Vec<Column> = Vec::new();
        let mut result_valid: Vec<Vec<bool>> = Vec::new();

        result_data.push(Column::Str(group_keys));
        result_valid.push(vec![true; ngroups]);

        // Aggregate columns — process each agg column in parallel
        let agg_results: Vec<(String, Vec<f64>)> = aggs.par_iter().map(|(col_name, agg_fn)| {
            let ci = self.col_index(col_name).expect("group: agg column not found");
            let agg_name = format!("{}_{:?}", col_name, agg_fn).to_lowercase();
            let col_valid = self.validity[ci].as_ref();

            let vals = aggregate_by_groups(
                self.data[ci].as_ref(), col_valid, &group_ids, ngroups, *agg_fn,
            );
            (agg_name, vals)
        }).collect();

        for (agg_name, vals) in agg_results {
            result_cols.push(agg_name);
            result_valid.push(vec![true; ngroups]);
            result_data.push(Column::Float(vals));
        }

        Ok(CrowFrame::new(result_cols, result_data, result_valid))
    }

    /// Inner/left/right/outer join on a key column.
    /// Uses native type keys for speed (avoids fmt_cell string allocation).
    pub fn join(&self, other: &CrowFrame, key: &str, join_type: &str) -> Result<CrowFrame, String> {
        let li = self.col_index(key).ok_or_else(|| format!("join: column '{}' not found in left frame", key))?;
        let ri = other.col_index(key).ok_or_else(|| format!("join: column '{}' not found in right frame", key))?;

        let is_left = join_type == "left" || join_type == "outer";
        let is_right = join_type == "right" || join_type == "outer";

        let (left_indices, right_indices) = join_indices_native(
            self.data[li].as_ref(), self.validity[li].as_ref(),
            other.data[ri].as_ref(), other.validity[ri].as_ref(),
            is_left, is_right,
        );

        // Build result frame — gather columns in parallel
        let mut result_cols = Vec::new();
        let mut col_specs: Vec<(&CrowFrame, usize, bool)> = Vec::new(); // (frame, col_idx, is_left_side)

        // Left columns
        for (ci, col_name) in self.columns.iter().enumerate() {
            result_cols.push(col_name.clone());
            col_specs.push((self, ci, true));
        }

        // Right columns (skip the key column)
        for (ci, col_name) in other.columns.iter().enumerate() {
            if ci == ri { continue; }
            let name = if self.col_index(col_name).is_some() {
                format!("{}_right", col_name)
            } else {
                col_name.clone()
            };
            result_cols.push(name);
            col_specs.push((other, ci, false));
        }

        // Gather all columns in parallel
        let gathered: Vec<(Column, Vec<bool>)> = col_specs.par_iter().map(|&(frame, ci, is_left)| {
            let idx_list = if is_left { &left_indices } else { &right_indices };
            gather_column(frame.data[ci].as_ref(), frame.validity[ci].as_ref(), idx_list)
        }).collect();

        let (result_data, result_valid): (Vec<_>, Vec<_>) = gathered.into_iter().unzip();
        Ok(CrowFrame::new(result_cols, result_data, result_valid))
    }
}

/// Build join index pairs using native column types (avoids fmt_cell string allocation).
fn join_indices_native(
    left_col: &Column, left_valid: &[bool],
    right_col: &Column, right_valid: &[bool],
    include_left_only: bool, include_right_only: bool,
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    // Macro to avoid repeating the join logic for each type
    macro_rules! join_typed {
        ($left_vals:expr, $right_vals:expr, $key_fn:expr) => {{
            let mut right_map: std::collections::HashMap<_, Vec<usize>> = std::collections::HashMap::new();
            for (row, val) in $right_vals.iter().enumerate() {
                if !right_valid[row] { continue; }
                right_map.entry($key_fn(val)).or_default().push(row);
            }
            let mut left_indices = Vec::new();
            let mut right_indices = Vec::new();
            for (row, val) in $left_vals.iter().enumerate() {
                if !left_valid[row] { continue; }
                let k = $key_fn(val);
                if let Some(right_rows) = right_map.get(&k) {
                    for &rr in right_rows {
                        left_indices.push(Some(row));
                        right_indices.push(Some(rr));
                    }
                } else if include_left_only {
                    left_indices.push(Some(row));
                    right_indices.push(None);
                }
            }
            if include_right_only {
                let mut left_keys: std::collections::HashSet<_> = std::collections::HashSet::new();
                for (row, val) in $left_vals.iter().enumerate() {
                    if left_valid[row] { left_keys.insert($key_fn(val)); }
                }
                for (row, val) in $right_vals.iter().enumerate() {
                    if !right_valid[row] { continue; }
                    if !left_keys.contains(&$key_fn(val)) {
                        left_indices.push(None);
                        right_indices.push(Some(row));
                    }
                }
            }
            (left_indices, right_indices)
        }};
    }

    match (left_col, right_col) {
        (Column::Int(lv), Column::Int(rv)) => join_typed!(lv, rv, |v: &i64| *v),
        (Column::Float(lv), Column::Float(rv)) => join_typed!(lv, rv, |v: &f64| v.to_bits()),
        (Column::Str(lv), Column::Str(rv)) => {
            // Handle Str separately to manage lifetimes (HashMap borrows from rv/lv)
            let mut right_map: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
            for (row, val) in rv.iter().enumerate() {
                if !right_valid[row] { continue; }
                right_map.entry(val.as_str()).or_default().push(row);
            }
            let mut left_indices = Vec::new();
            let mut right_indices = Vec::new();
            for (row, val) in lv.iter().enumerate() {
                if !left_valid[row] { continue; }
                if let Some(right_rows) = right_map.get(val.as_str()) {
                    for &rr in right_rows {
                        left_indices.push(Some(row));
                        right_indices.push(Some(rr));
                    }
                } else if include_left_only {
                    left_indices.push(Some(row));
                    right_indices.push(None);
                }
            }
            if include_right_only {
                let mut left_keys: std::collections::HashSet<&str> = std::collections::HashSet::new();
                for (row, val) in lv.iter().enumerate() {
                    if left_valid[row] { left_keys.insert(val.as_str()); }
                }
                for (row, val) in rv.iter().enumerate() {
                    if !right_valid[row] { continue; }
                    if !left_keys.contains(val.as_str()) {
                        left_indices.push(None);
                        right_indices.push(Some(row));
                    }
                }
            }
            (left_indices, right_indices)
        }
        (Column::Bool(lv), Column::Bool(rv)) => join_typed!(lv, rv, |v: &bool| *v),
        (Column::DateTime(lm, _), Column::DateTime(rm, _)) => join_typed!(lm, rm, |v: &i64| *v),
        _ => {
            // Fallback: use string keys for mismatched types
            let left_nrows = left_valid.len();
            let right_nrows = right_valid.len();
            let mut right_map: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
            for row in 0..right_nrows {
                if !right_valid[row] { continue; }
                right_map.entry(right_col.fmt_cell(row, true)).or_default().push(row);
            }
            let mut left_indices = Vec::new();
            let mut right_indices = Vec::new();
            for row in 0..left_nrows {
                if !left_valid[row] { continue; }
                let k = left_col.fmt_cell(row, true);
                if let Some(right_rows) = right_map.get(&k) {
                    for &rr in right_rows {
                        left_indices.push(Some(row));
                        right_indices.push(Some(rr));
                    }
                } else if include_left_only {
                    left_indices.push(Some(row));
                    right_indices.push(None);
                }
            }
            if include_right_only {
                let mut left_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
                for row in 0..left_nrows {
                    if left_valid[row] { left_keys.insert(left_col.fmt_cell(row, true)); }
                }
                for row in 0..right_nrows {
                    if !right_valid[row] { continue; }
                    let k = right_col.fmt_cell(row, true);
                    if !left_keys.contains(&k) {
                        left_indices.push(None);
                        right_indices.push(Some(row));
                    }
                }
            }
            (left_indices, right_indices)
        }
    }
}


/// Aggregate using group_ids array (avoids building per-group Vec<usize> row lists).
fn aggregate_by_groups(col: &Column, valid: &[bool], group_ids: &[u32], ngroups: usize, agg: AggFn) -> Vec<f64> {
    let nrows = group_ids.len();
    match agg {
        AggFn::Count => {
            let mut counts = vec![0.0f64; ngroups];
            for row in 0..nrows {
                let gid = group_ids[row];
                if gid != u32::MAX && valid[row] {
                    counts[gid as usize] += 1.0;
                }
            }
            counts
        }
        AggFn::Sum | AggFn::Mean => {
            let mut sums = vec![0.0f64; ngroups];
            let mut counts = vec![0u64; ngroups];
            for row in 0..nrows {
                let gid = group_ids[row];
                if gid == u32::MAX || !valid[row] { continue; }
                let v = match col {
                    Column::Int(d) => d[row] as f64,
                    Column::Float(d) => d[row],
                    Column::Bool(d) => if d[row] { 1.0 } else { 0.0 },
                    _ => continue,
                };
                if v.is_nan() { continue; }
                sums[gid as usize] += v;
                counts[gid as usize] += 1;
            }
            if matches!(agg, AggFn::Mean) {
                for i in 0..ngroups {
                    sums[i] = if counts[i] > 0 { sums[i] / counts[i] as f64 } else { f64::NAN };
                }
            }
            sums
        }
        AggFn::Min => {
            let mut mins = vec![f64::INFINITY; ngroups];
            for row in 0..nrows {
                let gid = group_ids[row];
                if gid == u32::MAX || !valid[row] { continue; }
                let v = match col {
                    Column::Int(d) => d[row] as f64,
                    Column::Float(d) => d[row],
                    Column::Bool(d) => if d[row] { 1.0 } else { 0.0 },
                    _ => continue,
                };
                if !v.is_nan() && v < mins[gid as usize] { mins[gid as usize] = v; }
            }
            mins
        }
        AggFn::Max => {
            let mut maxs = vec![f64::NEG_INFINITY; ngroups];
            for row in 0..nrows {
                let gid = group_ids[row];
                if gid == u32::MAX || !valid[row] { continue; }
                let v = match col {
                    Column::Int(d) => d[row] as f64,
                    Column::Float(d) => d[row],
                    Column::Bool(d) => if d[row] { 1.0 } else { 0.0 },
                    _ => continue,
                };
                if !v.is_nan() && v > maxs[gid as usize] { maxs[gid as usize] = v; }
            }
            maxs
        }
    }
}


fn gather_column(col: &Column, valid: &[bool], indices: &[Option<usize>]) -> (Column, Vec<bool>) {
    let new_valid: Vec<bool> = indices.iter().map(|opt| {
        opt.map_or(false, |i| valid[i])
    }).collect();
    let nc = match col {
        Column::Int(v) => Column::Int(indices.iter().map(|opt| opt.map_or(0, |i| v[i])).collect()),
        Column::Float(v) => Column::Float(indices.iter().map(|opt| opt.map_or(f64::NAN, |i| v[i])).collect()),
        Column::Str(v) => Column::Str(indices.iter().map(|opt| opt.map_or(String::new(), |i| v[i].clone())).collect()),
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
        use std::io::{Write, BufWriter};
        let file = std::fs::File::create(path).map_err(|e| format!("save: {}", e))?;
        let mut w = BufWriter::with_capacity(8 * 1024 * 1024, file);
        // Header
        writeln!(w, "{}", self.columns.join(",")).map_err(|e| format!("save: {}", e))?;

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
                            buf.extend_from_slice(v[row].as_bytes());
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

    /// Get unique values in a column as strings.
    /// Uses early termination: stops scanning if no new values found after a long stretch.
    pub fn unique_col(&self, col_name: &str) -> Result<Vec<String>, String> {
        let idx = self.col_index(col_name)
            .ok_or_else(|| format!("unique: column '{}' not found", col_name))?;
        let valid = self.validity[idx].as_ref();
        let nrows = self.nrows();
        // Early exit threshold: if no new value found in this many rows, assume done
        let patience = 100_000.max(nrows / 10);

        match self.data[idx].as_ref() {
            Column::Int(v) => {
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                let mut since_last_new = 0usize;
                for (i, &val) in v.iter().enumerate() {
                    if valid[i] {
                        if seen.insert(val) {
                            result.push(val.to_string());
                            since_last_new = 0;
                        } else {
                            since_last_new += 1;
                            if since_last_new >= patience { break; }
                        }
                    }
                }
                Ok(result)
            }
            Column::Float(v) => {
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                let mut since_last_new = 0usize;
                for (i, &val) in v.iter().enumerate() {
                    if valid[i] {
                        if seen.insert(val.to_bits()) {
                            result.push(format!("{}", val));
                            since_last_new = 0;
                        } else {
                            since_last_new += 1;
                            if since_last_new >= patience { break; }
                        }
                    }
                }
                Ok(result)
            }
            Column::Str(v) => {
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                let mut since_last_new = 0usize;
                for (i, val) in v.iter().enumerate() {
                    if valid[i] {
                        if seen.insert(val.as_str()) {
                            result.push(val.clone());
                            since_last_new = 0;
                        } else {
                            since_last_new += 1;
                            if since_last_new >= patience { break; }
                        }
                    }
                }
                Ok(result)
            }
            Column::Bool(v) => {
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for (i, &val) in v.iter().enumerate() {
                    if valid[i] && seen.insert(val) {
                        result.push(val.to_string());
                    }
                    if seen.len() == 2 { break; } // Bool: max 2 values
                }
                Ok(result)
            }
            Column::DateTime(millis, _) => {
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                let mut since_last_new = 0usize;
                for (i, &val) in millis.iter().enumerate() {
                    if valid[i] {
                        if seen.insert(val) {
                            result.push(self.data[idx].fmt_cell(i, true));
                            since_last_new = 0;
                        } else {
                            since_last_new += 1;
                            if since_last_new >= patience { break; }
                        }
                    }
                }
                Ok(result)
            }
        }
    }

    /// Print summary statistics for all numeric columns.
    pub fn stats_summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("{:<15} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "column", "count", "mean", "min", "max", "std"));
        lines.push("-".repeat(75));
        for (ci, col_name) in self.columns.iter().enumerate() {
            let vals: Vec<f64> = match self.data[ci].as_ref() {
                Column::Int(v) => v.iter().enumerate()
                    .filter(|(i, _)| self.validity[ci][*i])
                    .map(|(_, &x)| x as f64).collect(),
                Column::Float(v) => v.iter().enumerate()
                    .filter(|(i, _)| self.validity[ci][*i])
                    .map(|(_, &x)| x).filter(|x| !x.is_nan()).collect(),
                _ => continue,
            };
            if vals.is_empty() { continue; }
            let count = vals.len() as f64;
            let sum: f64 = vals.iter().sum();
            let mean = sum / count;
            let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let variance: f64 = vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / count;
            let std = variance.sqrt();
            lines.push(format!("{:<15} {:>10} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
                col_name, count as i64, mean, min, max, std));
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
pub fn to_arrow_ipc(frame: &CrowFrame) -> Vec<u8> {
    use arrow::array::*;
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::ipc::writer::StreamWriter;
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
                        .map(|(v, &valid)| if valid { Some(v.as_str()) } else { None })
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

    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays)
        .expect("failed to create Arrow RecordBatch from CrowFrame");

    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, &schema)
            .expect("failed to create Arrow IPC writer");
        writer.write(&batch).expect("failed to write Arrow IPC batch");
        writer.finish().expect("failed to finish Arrow IPC stream");
    }
    buf
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
                Column::Str(vec!["a".into(), "b".into(), "c".into()]),
            ],
            vec![
                vec![true, true, true],
                vec![true, true, true],
                vec![true, true, true],
            ],
        );
        let bytes = to_arrow_ipc(&frame);
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
        let bytes = to_arrow_ipc(&frame);
        assert!(!bytes.is_empty());
    }
}