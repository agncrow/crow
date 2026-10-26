//! A batch file: one commit of a log as one file, written once (spec,
//! lake: The log). The file is an Arrow IPC stream holding a frame with a
//! position column added in arrival order, its rows sorted by the key
//! columns, and, in the stream's own metadata, the statistics a reader
//! prunes by: per column its kind, min, max, the count of missing cells
//! and a bloom filter over the cells; per text column named, a bloom
//! filter over its tokens. One file is at once the log entry (positions),
//! the level-0 segment (sorted, with statistics) and the search index
//! (token filters). It is written to a temporary name, flushed to the
//! disk (`sync_all`: `F_FULLFSYNC` on macOS, past the drive's cache),
//! renamed into place, and the directory flushed, so a crash leaves the
//! log as it was or with the whole file. A merge folds many files into
//! one segment: the same frame and statistics as a Parquet file, named by
//! the batch numbers it covers (`000000000003-000000000017.segment`),
//! the latest version of each key kept where keys are given. One reader
//! serves both kinds, told apart by the file's first bytes. Both backends
//! call these functions; the Crow-written module over them is `lake`.

use crate::frame::{Column, CrowFrame};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

/// The column every batch file adds: the row's position in the log.
pub const POSITION: &str = "_pos";
/// The metadata key the statistics are kept under.
const META: &str = "crow.batch";
/// A filter's bits per row (about 2% false positives at six hashes).
const BITS_PER_ROW: usize = 8;
const HASHES: u32 = 6;
const EXTENSION: &str = ".batch";
/// A merged segment's rows per row group.
pub const ROW_GROUP: usize = 8192;

/// A batch file's name by its number, so the names list in order.
pub fn file_name(number: i64) -> String {
    format!("{:012}{}", number, EXTENSION)
}

/// The number a batch file's name says, or none for another file.
pub fn number_of(name: &str) -> Option<i64> {
    let stem = name.strip_suffix(EXTENSION)?;
    if stem.len() != 12 || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

const SEGMENT: &str = ".segment";

/// A segment's name by the batch numbers it covers.
pub fn segment_name(lo: i64, hi: i64) -> String {
    format!("{:012}-{:012}{}", lo, hi, SEGMENT)
}

/// The batch numbers a file covers: a batch its own, a segment the range
/// its name says; none for another file.
pub fn covered(name: &str) -> Option<(i64, i64)> {
    if let Some(n) = number_of(name) {
        return Some((n, n));
    }
    let stem = name.strip_suffix(SEGMENT)?;
    let (a, b) = stem.split_once('-')?;
    if a.len() != 12 || b.len() != 12 || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// A bloom filter over byte strings: `k` hashes into `m` bits, the two
/// hashes FNV-1a and a mix of it (Kirsch and Mitzenmacher's double
/// hashing), so a filter written today reads the same tomorrow — no
/// hasher whose keys the standard library may change.
pub struct Bloom {
    bits: Vec<u8>,
    m: u64,
    k: u32,
}

impl Bloom {
    pub fn sized(items: usize) -> Bloom {
        let m = (items.max(8) * BITS_PER_ROW).next_multiple_of(8) as u64;
        Bloom { bits: vec![0; (m / 8) as usize], m, k: HASHES }
    }

    fn hashes(b: &[u8]) -> (u64, u64) {
        let mut h: u64 = 0xcbf29ce484222325;
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        // murmur's fmix64, so the second hash is independent of the first
        let mut g = h;
        g ^= g >> 33;
        g = g.wrapping_mul(0xff51afd7ed558ccd);
        g ^= g >> 33;
        g = g.wrapping_mul(0xc4ceb9fe1a85ec53);
        g ^= g >> 33;
        (h, g | 1)
    }

    pub fn add(&mut self, b: &[u8]) {
        let (h1, h2) = Self::hashes(b);
        for i in 0..self.k as u64 {
            let at = h1.wrapping_add(i.wrapping_mul(h2)) % self.m;
            self.bits[(at / 8) as usize] |= 1 << (at % 8);
        }
    }

    pub fn has(&self, b: &[u8]) -> bool {
        let (h1, h2) = Self::hashes(b);
        (0..self.k as u64).all(|i| {
            let at = h1.wrapping_add(i.wrapping_mul(h2)) % self.m;
            self.bits[(at / 8) as usize] & (1 << (at % 8)) != 0
        })
    }

    /// The filter as text: its hash count, its bit count and its bits, in
    /// base64.
    pub fn encode(&self) -> String {
        let mut out = Vec::with_capacity(9 + self.bits.len());
        out.push(self.k as u8);
        out.extend_from_slice(&self.m.to_le_bytes());
        out.extend_from_slice(&self.bits);
        crate::text::base64_encode(&out)
    }

    pub fn decode(text: &str) -> Result<Bloom, String> {
        let raw = crate::text::base64_decode(text)?;
        if raw.len() < 9 {
            return Err("a filter needs its hash count and its size".to_string());
        }
        let k = raw[0] as u32;
        let m = u64::from_le_bytes(raw[1..9].try_into().map_err(|_| "a filter's size is eight bytes".to_string())?);
        let bits = raw[9..].to_vec();
        if m == 0 || k == 0 || bits.len() as u64 * 8 < m {
            return Err(format!("a filter of {} bits cannot lie in {} bytes", m, bits.len()));
        }
        Ok(Bloom { bits, m, k })
    }
}

/// A cell's bytes for hashing: an int or a moment's millis as eight
/// little-endian bytes, a float's bits (negative zero as zero), a bool as
/// one byte, text as its UTF-8.
fn cell_bytes(col: &Column, i: usize, out: &mut Vec<u8>) {
    out.clear();
    match col {
        Column::Int(v) => out.extend_from_slice(&v[i].to_le_bytes()),
        Column::Float(v) => {
            let x = if v[i] == 0.0 { 0.0 } else { v[i] };
            out.extend_from_slice(&x.to_bits().to_le_bytes());
        }
        Column::Bool(v) => out.push(v[i] as u8),
        Column::Str(t) => out.extend_from_slice(t.get(i).as_bytes()),
        Column::DateTime(ms, _) => out.extend_from_slice(&ms[i].to_le_bytes()),
    }
}

/// A value written as text, as the bytes the column's cells hash to, or
/// why the text is no value of the column's kind.
fn value_bytes(kind: &str, value: &str) -> Result<Vec<u8>, String> {
    Ok(match kind {
        "int" => value.trim().parse::<i64>().map_err(|_| format!("'{}' is no int", value))?.to_le_bytes().to_vec(),
        "moment" => value.trim().parse::<i64>().map_err(|_| format!("'{}' is no moment's milliseconds", value))?.to_le_bytes().to_vec(),
        "float" => {
            let x: f64 = value.trim().parse().map_err(|_| format!("'{}' is no number", value))?;
            let x = if x == 0.0 { 0.0 } else { x };
            x.to_bits().to_le_bytes().to_vec()
        }
        "bool" => match value.trim() {
            "true" => vec![1],
            "false" => vec![0],
            _ => return Err(format!("'{}' is no bool", value)),
        },
        "text" => value.as_bytes().to_vec(),
        other => return Err(format!("a column of kind '{}' is not looked up", other)),
    })
}

/// The tokens of a text: its runs of letters and digits, lowercased.
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

fn kind_word(col: &Column) -> &'static str {
    match col {
        Column::Int(_) => "int",
        Column::Float(_) => "float",
        Column::Bool(_) => "bool",
        Column::Str(_) => "text",
        Column::DateTime(..) => "moment",
    }
}

/// How many distinct values a column's filter covers at most: past it
/// the column is left to its min and max (a timestamp, a position, a
/// price — every cell its own value — had been given a filter sized by
/// the rows, 130 MB of filters in a month's segment and 180 ms to open it).
pub const DISTINCT_MOST: usize = 1 << 16;

/// The statistics of one column: its kind, min and max over the present
/// cells (none where every cell is missing; a NaN is no min or max), how
/// many cells are missing, and a filter over the distinct present values
/// where they are few enough (none otherwise).
fn column_stats(col: &Column, valid: &crate::validity::Validity) -> serde_json::Value {
    use serde_json::{json, Value};
    let n = col.len();
    let present: Vec<usize> = (0..n).filter(|&i| valid[i]).collect();
    let mut distinct: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut bytes = Vec::new();
    let mut filtered = true;
    for &i in &present {
        cell_bytes(col, i, &mut bytes);
        if !distinct.contains(&bytes) {
            if distinct.len() >= DISTINCT_MOST {
                filtered = false;
                break;
            }
            distinct.insert(bytes.clone());
        }
    }
    let bloom = if filtered {
        let mut b = Bloom::sized(distinct.len());
        for v in &distinct {
            b.add(v);
        }
        Value::from(b.encode())
    } else {
        Value::Null
    };
    let (min, max): (Value, Value) = match col {
        Column::Int(v) => {
            let it = present.iter().map(|&i| v[i]);
            (it.clone().min().map_or(Value::Null, Value::from), it.max().map_or(Value::Null, Value::from))
        }
        Column::DateTime(ms, _) => {
            let it = present.iter().map(|&i| ms[i]);
            (it.clone().min().map_or(Value::Null, Value::from), it.max().map_or(Value::Null, Value::from))
        }
        Column::Float(v) => {
            let it = present.iter().map(|&i| v[i]).filter(|x| !x.is_nan());
            (it.clone().reduce(f64::min).map_or(Value::Null, Value::from), it.reduce(f64::max).map_or(Value::Null, Value::from))
        }
        Column::Bool(v) => {
            let it = present.iter().map(|&i| v[i]);
            (it.clone().min().map_or(Value::Null, Value::from), it.max().map_or(Value::Null, Value::from))
        }
        Column::Str(t) => {
            let it = present.iter().map(|&i| t.get(i));
            (it.clone().min().map_or(Value::Null, |s| Value::from(s)), it.max().map_or(Value::Null, |s| Value::from(s)))
        }
    };
    json!({ "kind": kind_word(col), "min": min, "max": max, "missing": n - present.len(), "bloom": bloom })
}

/// The statistics of a frame about to be written: its row count and
/// positions, the keys it is sorted by, each column's statistics, and a
/// token filter per text column named.
fn statistics(frame: &CrowFrame, first: i64, last: i64, keys: &[String], texts: &[String]) -> Result<String, String> {
    use serde_json::{json, Map, Value};
    let n = frame.nrows();
    let mut columns = Map::new();
    for (i, name) in frame.columns.iter().enumerate() {
        columns.insert(name.clone(), column_stats(frame.data[i].as_ref(), frame.validity[i].as_ref()));
    }
    let mut token_filters = Map::new();
    for name in texts {
        let i = frame.col_index(name).ok_or_else(|| format!("no column '{}' to index the words of", name))?;
        let Column::Str(t) = frame.data[i].as_ref() else {
            return Err(format!("column '{}' holds no text to index the words of", name));
        };
        let valid = frame.validity[i].as_ref();
        let mut words: Vec<String> = Vec::new();
        for r in 0..n {
            if valid[r] {
                words.extend(tokens(t.get(r)));
            }
        }
        words.sort();
        words.dedup();
        let mut bloom = Bloom::sized(words.len());
        for w in &words {
            bloom.add(w.as_bytes());
        }
        token_filters.insert(name.clone(), Value::from(bloom.encode()));
    }
    let stats = json!({
        "rows": n,
        "first": first,
        "last": last,
        "keys": keys,
        "columns": Value::Object(columns),
        "tokens": Value::Object(token_filters),
    });
    Ok(stats.to_string())
}

/// `sys.batch_write[dir, number, first, frame, keys, texts]`: the frame
/// written as batch file `number` under `dir`, its rows given the
/// positions from `first` in the order they arrive and sorted by `keys`
/// (ties in arrival order), the words of each column in `texts` filtered.
/// Answers the file's path. Refused: a frame with no rows, a key or a text
/// column the frame lacks, a column already named `_pos`, a number whose
/// file exists.
pub fn write(dir: &str, number: i64, first: i64, frame: &CrowFrame, keys: &[String], texts: &[String]) -> Result<String, String> {
    use arrow::ipc::writer::StreamWriter;
    use arrow::record_batch::RecordBatch;
    if number < 0 || first < 0 {
        return Err(format!("batch: a number and a first position are 0 or more (got {} and {})", number, first));
    }
    let n = frame.nrows();
    if n == 0 {
        return Err("batch: a batch holds at least one row".to_string());
    }
    if frame.col_index(POSITION).is_some() {
        return Err(format!("batch: the frame already holds a column '{}', which the log adds", POSITION));
    }
    for k in keys {
        if frame.col_index(k).is_none() {
            return Err(format!("batch: no column '{}' to sort by", k));
        }
    }
    let path = std::path::Path::new(dir).join(file_name(number));
    if path.exists() {
        return Err(format!("batch: '{}' exists — batch {} was written already", path.display(), number));
    }
    let positions = Column::Int((first..first + n as i64).collect());
    let numbered = frame.with_column(POSITION, positions)?;
    let sorted = if keys.is_empty() {
        numbered
    } else {
        let names: Vec<&str> = keys.iter().map(String::as_str).collect();
        numbered.sort_keys(&names, &vec![false; names.len()])?
    };
    let stats = statistics(&sorted, first, first + n as i64 - 1, keys, texts).map_err(|e| format!("batch: {}", e))?;
    let batch = crate::frame::record_batch(&sorted)?;
    let mut metadata = HashMap::new();
    metadata.insert(META.to_string(), stats);
    let schema = Arc::new(batch.schema().as_ref().clone().with_metadata(metadata));
    let batch = RecordBatch::try_new(schema.clone(), batch.columns().to_vec()).map_err(|e| format!("batch: {}", e))?;
    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, &schema).map_err(|e| format!("batch: {}", e))?;
        writer.write(&batch).map_err(|e| format!("batch: {}", e))?;
        writer.finish().map_err(|e| format!("batch: {}", e))?;
    }
    place(dir, &file_name(number), &buf)
}

/// The bytes written as a file of the directory: to a temporary name,
/// flushed to the disk, linked into place — a link, never a rename, so a
/// file of the name that appeared meanwhile is not overwritten and the
/// writer learns it lost the number — then the directory flushed, or a
/// crash could forget the name of a file whose bytes it kept.
fn place(dir: &str, name: &str, bytes: &[u8]) -> Result<String, String> {
    let path = std::path::Path::new(dir).join(name);
    let part = std::path::Path::new(dir).join(format!(".{}.part", name));
    let cannot = |what: &str, e: std::io::Error| format!("batch: cannot {} '{}': {}", what, path.display(), e);
    let mut file = std::fs::File::create(&part).map_err(|e| cannot("write", e))?;
    file.write_all(bytes).map_err(|e| cannot("write", e))?;
    file.sync_all().map_err(|e| cannot("flush", e))?;
    drop(file);
    let linked = std::fs::hard_link(&part, &path);
    let _ = std::fs::remove_file(&part);
    if let Err(e) = linked {
        if path.exists() {
            return Err(format!("batch: '{}' exists — it was written already", path.display()));
        }
        return Err(cannot("place", e));
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(path.to_string_lossy().to_string())
}

fn open(path: &str) -> Result<arrow::ipc::reader::StreamReader<std::io::BufReader<std::fs::File>>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("batch: cannot read '{}': {}", path, e))?;
    arrow::ipc::reader::StreamReader::try_new(std::io::BufReader::new(file), None).map_err(|e| format!("batch: '{}' is no batch file: {}", path, e))
}

/// Whether the file is a segment (Parquet, by its first bytes) rather
/// than a batch (an Arrow stream).
fn is_segment(path: &str) -> Result<bool, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("batch: cannot read '{}': {}", path, e))?;
    let mut head = [0u8; 4];
    let n = file.read(&mut head).map_err(|e| format!("batch: cannot read '{}': {}", path, e))?;
    Ok(n == 4 && &head == b"PAR1")
}

/// A segment's parsed footer, kept per process by the file's path, size
/// and modification time: a segment is immutable, and parsing a month's
/// footer had cost each call 180 ms.
#[cfg(feature = "parquet")]
fn segment_metadata(path: &str) -> Result<(std::fs::File, parquet::arrow::arrow_reader::ArrowReaderMetadata), String> {
    use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions};
    use std::sync::{Mutex, OnceLock};
    type Key = (String, u64, Option<std::time::SystemTime>);
    static FOOTERS: OnceLock<Mutex<std::collections::HashMap<Key, ArrowReaderMetadata>>> = OnceLock::new();
    let file = std::fs::File::open(path).map_err(|e| format!("batch: cannot read '{}': {}", path, e))?;
    let meta = file.metadata().map_err(|e| format!("batch: cannot read '{}': {}", path, e))?;
    let key: Key = (path.to_string(), meta.len(), meta.modified().ok());
    let footers = FOOTERS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(found) = footers.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return Ok((file, found));
    }
    let parsed = ArrowReaderMetadata::load(&file, ArrowReaderOptions::new()).map_err(|e| format!("batch: '{}' is no segment: {}", path, e))?;
    if let Ok(mut m) = footers.lock() {
        if m.len() > 1024 {
            m.clear();
        }
        m.insert(key, parsed.clone());
    }
    Ok((file, parsed))
}

/// A segment's reader over its file: the footer read once per process,
/// the rows not until asked (reading the whole file for its statistics
/// had cost a lookup ten milliseconds).
#[cfg(feature = "parquet")]
fn segment_builder(path: &str) -> Result<parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder<std::fs::File>, String> {
    let (file, metadata) = segment_metadata(path)?;
    Ok(parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata))
}

/// The statistics a Parquet segment carries: in its Arrow schema, or in
/// the file's own key-value metadata (the writer sets both).
#[cfg(feature = "parquet")]
fn segment_stats<T: parquet::file::reader::ChunkReader + 'static>(builder: &parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder<T>) -> Option<String> {
    if let Some(s) = builder.schema().metadata().get(META) {
        return Some(s.clone());
    }
    builder.metadata().file_metadata().key_value_metadata()?.iter().find(|kv| kv.key == META).and_then(|kv| kv.value.clone())
}

/// A segment's row groups decoded a column at a time across the cores:
/// each column its own reader over the file (the footer parsed once and
/// shared), its row groups in one pass, and the columns gathered into the
/// frame with no row copied (decoding the groups across the cores and
/// joining the pieces had been slower than one reader: the join copied
/// every row; a column's decode is a third of a day's cost, and twelve
/// columns run at once).
#[cfg(feature = "parquet")]
fn read_groups(path: &str, groups: &[usize]) -> Result<CrowFrame, String> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use parquet::arrow::ProjectionMask;
    let (file, metadata) = segment_metadata(path)?;
    let schema = metadata.schema().clone();
    if groups.is_empty() {
        return crate::frame::frame_of_batches(&schema, std::iter::empty(), "batch");
    }
    let n = schema.fields().len();
    let rows: usize = groups.iter().map(|&g| metadata.metadata().row_group(g).num_rows() as usize).sum();
    // (one column, or few rows: one reader does it)
    if n <= 1 || rows < ROW_GROUP {
        let reader = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata).with_row_groups(groups.to_vec()).with_batch_size(ROW_GROUP).build().map_err(|e| format!("batch: '{}': {}", path, e))?;
        return crate::frame::frame_of_batches(&schema, reader, "batch");
    }
    drop(file);
    let columns: Vec<usize> = (0..n).collect();
    let parts: Vec<Result<CrowFrame, String>> = crate::cores::map_items(&columns, rows * n, |&i| {
        let file = std::fs::File::open(path).map_err(|e| format!("batch: cannot read '{}': {}", path, e))?;
        let mask = ProjectionMask::leaves(metadata.parquet_schema(), [i]);
        let one = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![schema.field(i).clone()]));
        let reader = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata.clone()).with_projection(mask).with_row_groups(groups.to_vec()).with_batch_size(ROW_GROUP).build().map_err(|e| format!("batch: '{}': {}", path, e))?;
        crate::frame::frame_of_batches(&one, reader, "batch")
    });
    let mut names = Vec::with_capacity(n);
    let mut data = Vec::with_capacity(n);
    let mut validity = Vec::with_capacity(n);
    for part in parts {
        let part = part?;
        names.extend(part.columns.into_iter());
        data.extend(part.data.into_iter());
        validity.extend(part.validity.into_iter());
    }
    Ok(CrowFrame { columns: names, data, validity })
}

/// The frame a file holds and the statistics in its head, by its kind.
fn read_with_stats(path: &str) -> Result<(CrowFrame, String), String> {
    if is_segment(path)? {
        #[cfg(feature = "parquet")]
        {
            let builder = segment_builder(path)?;
            let stats = segment_stats(&builder).ok_or_else(|| format!("batch: '{}' is no segment: it carries no statistics", path))?;
            let groups: Vec<usize> = (0..builder.metadata().num_row_groups()).collect();
            let frame = read_groups(path, &groups)?;
            return Ok((frame, stats));
        }
        #[cfg(not(feature = "parquet"))]
        {
            return Err(format!("batch: '{}' is a segment, and this program was built without Parquet", path));
        }
    }
    let reader = open(path)?;
    let schema = reader.schema();
    let stats = schema.metadata().get(META).cloned().ok_or_else(|| format!("batch: '{}' is no batch file: it carries no statistics", path))?;
    let rows = serde_json::from_str::<serde_json::Value>(&stats).ok().and_then(|s| s["rows"].as_u64());
    let frame = crate::frame::frame_of_batches(&schema, reader, "batch")?;
    // a stream cut short can end cleanly between messages: the count the
    // head promised is what says the rows are all there
    if rows != Some(frame.nrows() as u64) {
        return Err(format!("batch: '{}' is cut short: its head promises {} rows, {} are there", path, rows.map_or("no".to_string(), |r| r.to_string()), frame.nrows()));
    }
    Ok((frame, stats))
}

/// `sys.batch_read[path]`: the batch file, or the segment, as the frame
/// it holds, its position column included. A file cut short is refused
/// with its reason.
pub fn read(path: &str) -> Result<CrowFrame, String> {
    read_with_stats(path).map(|(f, _)| f)
}

/// `sys.batch_stats[path]`: the file's statistics as JSON text — `rows`,
/// `first`, `last`, `keys`, `columns` (each `kind`, `min`, `max`,
/// `missing`, `bloom`) and `tokens` — read from the file's head alone.
pub fn stats(path: &str) -> Result<String, String> {
    if is_segment(path)? {
        #[cfg(feature = "parquet")]
        {
            let builder = segment_builder(path)?;
            return segment_stats(&builder).ok_or_else(|| format!("batch: '{}' is no segment: it carries no statistics", path));
        }
        #[cfg(not(feature = "parquet"))]
        {
            return Err(format!("batch: '{}' is a segment, and this program was built without Parquet", path));
        }
    }
    let reader = open(path)?;
    let schema = reader.schema();
    schema.metadata().get(META).cloned().ok_or_else(|| format!("batch: '{}' is no batch file: it carries no statistics", path))
}

/// A value written as text, read as a column's kind.
enum Parsed {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

fn parsed(kind: &str, value: &str) -> Result<Parsed, String> {
    Ok(match kind {
        "int" | "moment" => Parsed::Int(value.trim().parse::<i64>().map_err(|_| format!("'{}' is no {}", value, if kind == "int" { "int" } else { "moment's milliseconds" }))?),
        "float" => Parsed::Float(value.trim().parse::<f64>().map_err(|_| format!("'{}' is no number", value))?),
        "bool" => match value.trim() {
            "true" => Parsed::Bool(true),
            "false" => Parsed::Bool(false),
            _ => return Err(format!("'{}' is no bool", value)),
        },
        "text" => Parsed::Text(value.to_string()),
        other => return Err(format!("a column of kind '{}' is not looked up", other)),
    })
}

/// The rows of a frame whose column holds the value.
fn rows_holding(frame: &CrowFrame, column: &str, want: &Parsed) -> Result<CrowFrame, String> {
    let c = frame.col_index(column).ok_or_else(|| format!("batch: no column '{}'", column))?;
    let valid = frame.validity[c].as_ref();
    let n = frame.nrows();
    let mask: Vec<bool> = match (frame.data[c].as_ref(), want) {
        (Column::Int(v), Parsed::Int(w)) => (0..n).map(|i| valid[i] && v[i] == *w).collect(),
        (Column::DateTime(ms, _), Parsed::Int(w)) => (0..n).map(|i| valid[i] && ms[i] == *w).collect(),
        (Column::Float(v), Parsed::Float(w)) => (0..n).map(|i| valid[i] && v[i] == *w).collect(),
        (Column::Bool(v), Parsed::Bool(w)) => (0..n).map(|i| valid[i] && v[i] == *w).collect(),
        (Column::Str(t), Parsed::Text(w)) => (0..n).map(|i| valid[i] && t.get(i) == w).collect(),
        _ => return Err(format!("batch: column '{}' holds another kind than the value", column)),
    };
    frame.pick_rows(&mask)
}

/// Whether a row group's statistics for the column admit the value.
#[cfg(feature = "parquet")]
fn group_admits(stats: Option<&parquet::file::statistics::Statistics>, want: &Parsed) -> bool {
    use parquet::file::statistics::Statistics;
    let Some(stats) = stats else { return true };
    match (stats, want) {
        (Statistics::Int64(s), Parsed::Int(w)) => s.min_opt().is_none_or(|m| m <= w) && s.max_opt().is_none_or(|m| m >= w),
        (Statistics::Int32(s), Parsed::Int(w)) => s.min_opt().is_none_or(|m| i64::from(*m) <= *w) && s.max_opt().is_none_or(|m| i64::from(*m) >= *w),
        (Statistics::Double(s), Parsed::Float(w)) => s.min_opt().is_none_or(|m| m <= w) && s.max_opt().is_none_or(|m| m >= w),
        (Statistics::Boolean(s), Parsed::Bool(w)) => s.min_opt().is_none_or(|m| m <= w) && s.max_opt().is_none_or(|m| m >= w),
        (Statistics::ByteArray(s), Parsed::Text(w)) => s.min_bytes_opt().is_none_or(|m| m <= w.as_bytes()) && s.max_bytes_opt().is_none_or(|m| m >= w.as_bytes()),
        _ => true,
    }
}

/// `sys.batch_find[path, column, value]`: the rows of the file whose
/// `column` holds the value (written as text, read as the column's kind),
/// with the file's own columns. The file's filter is asked first; a
/// segment is read row group by row group, only the groups whose
/// statistics admit the value — one, for a key the segment is sorted by.
pub fn find(path: &str, column: &str, value: &str) -> Result<CrowFrame, String> {
    let head: serde_json::Value = serde_json::from_str(&stats(path)?).map_err(|e| format!("batch: '{}' has unreadable statistics: {}", path, e))?;
    let Some(col) = head["columns"].get(column) else {
        return Err(format!("batch: '{}' holds no column '{}'", path, column));
    };
    let kind = col["kind"].as_str().unwrap_or("");
    let want = parsed(kind, value).map_err(|e| format!("batch: column '{}': {}", column, e))?;
    let bytes = value_bytes(kind, value).map_err(|e| format!("batch: column '{}': {}", column, e))?;
    let bloom = match col["bloom"].as_str() {
        Some(encoded) => Some(Bloom::decode(encoded).map_err(|e| format!("batch: '{}': {}", path, e))?),
        None => None,
    };
    let admitted = |b: &[u8]| bloom.as_ref().is_none_or(|f| f.has(b));
    if is_segment(path)? {
        #[cfg(feature = "parquet")]
        {
            let builder = segment_builder(path)?;
            let schema = builder.schema().clone();
            if !admitted(&bytes) {
                return crate::frame::frame_of_batches(&schema, std::iter::empty(), "batch");
            }
            let leaf = schema.fields().iter().position(|f| f.name() == column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", path, column))?;
            let groups: Vec<usize> = (0..builder.metadata().num_row_groups()).filter(|&g| group_admits(builder.metadata().row_group(g).column(leaf).statistics(), &want)).collect();
            if groups.is_empty() {
                return crate::frame::frame_of_batches(&schema, std::iter::empty(), "batch");
            }
            let frame = read_groups(path, &groups)?;
            return rows_holding(&frame, column, &want);
        }
        #[cfg(not(feature = "parquet"))]
        {
            return Err(format!("batch: '{}' is a segment, and this program was built without Parquet", path));
        }
    }
    let reader = open(path)?;
    let schema = reader.schema();
    if !admitted(&bytes) {
        return crate::frame::frame_of_batches(&schema, std::iter::empty(), "batch");
    }
    let frame = crate::frame::frame_of_batches(&schema, reader, "batch")?;
    rows_holding(&frame, column, &want)
}

/// Whether two rows hold the same cells in the columns.
fn same_key(frame: &CrowFrame, cols: &[usize], i: usize, j: usize) -> bool {
    cols.iter().all(|&c| {
        let (vi, vj) = (frame.validity[c][i], frame.validity[c][j]);
        if vi != vj {
            return false;
        }
        if !vi {
            return true;
        }
        match frame.data[c].as_ref() {
            Column::Int(v) => v[i] == v[j],
            Column::Float(v) => v[i].to_bits() == v[j].to_bits() || v[i] == v[j],
            Column::Bool(v) => v[i] == v[j],
            Column::Str(t) => t.get(i) == t.get(j),
            Column::DateTime(ms, _) => ms[i] == ms[j],
        }
    })
}

/// `sys.batch_merge[dir, paths, keys, every]`: the files (batches,
/// segments) folded into one segment under `dir`, named by the batch
/// numbers they cover, their rows sorted by `keys` then position, the
/// latest version of each key kept where keys are given and `every` is
/// false (every row kept otherwise: an append-only table sorted for
/// pruning, its keys no identity), the statistics in its head as a
/// batch's, the words of every column any input indexed indexed again.
/// Answers the segment's path; the inputs are left for the caller to
/// remove once the segment is in place. Refused: no files, a file that is
/// neither, files of different columns.
#[cfg(feature = "parquet")]
pub fn merge(dir: &str, paths: &[String], keys: &[String], every: bool) -> Result<String, String> {
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::metadata::KeyValue;
    use parquet::file::properties::WriterProperties;
    use arrow::record_batch::RecordBatch;
    if paths.is_empty() {
        return Err("batch: a merge takes one file or more".to_string());
    }
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    let mut texts: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut whole: Option<CrowFrame> = None;
    for p in paths {
        let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let (a, b) = covered(&name).ok_or_else(|| format!("batch: '{}' is neither a batch nor a segment by its name", p))?;
        lo = lo.min(a);
        hi = hi.max(b);
        let (f, st) = read_with_stats(p)?;
        if let Some(t) = serde_json::from_str::<serde_json::Value>(&st).ok().and_then(|s| s["tokens"].as_object().cloned()) {
            texts.extend(t.keys().cloned());
        }
        whole = Some(match whole {
            None => f,
            Some(w) => w.stack(&f).map_err(|_| format!("batch: '{}' has other columns than the files before it — a merge takes files of one shape", p))?,
        });
    }
    let whole = whole.ok_or_else(|| "batch: a merge takes one file or more".to_string())?;
    for k in keys {
        if whole.col_index(k).is_none() {
            return Err(format!("batch: no column '{}' to merge by", k));
        }
    }
    let pos = whole.col_index(POSITION).ok_or_else(|| "batch: the files hold no position column".to_string())?;
    let mut order: Vec<&str> = keys.iter().map(String::as_str).collect();
    order.push(POSITION);
    let sorted = whole.sort_keys(&order, &vec![false; order.len()])?;
    let n = sorted.nrows();
    let merged = if keys.is_empty() || every {
        sorted
    } else {
        // the last row of each run of one key is its latest version
        let cols: Vec<usize> = keys.iter().filter_map(|k| sorted.col_index(k)).collect();
        let keep: Vec<bool> = (0..n).map(|i| i + 1 == n || !same_key(&sorted, &cols, i, i + 1)).collect();
        sorted.pick_rows(&keep)?
    };
    let (first, last) = match merged.data[pos].as_ref() {
        Column::Int(v) => (v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(-1)),
        _ => return Err("batch: the position column holds no ints".to_string()),
    };
    let texts: Vec<String> = texts.into_iter().collect();
    let stats = statistics(&merged, first, last, keys, &texts).map_err(|e| format!("batch: {}", e))?;
    let batch = crate::frame::record_batch(&merged)?;
    let mut metadata = HashMap::new();
    metadata.insert(META.to_string(), stats.clone());
    let schema = Arc::new(batch.schema().as_ref().clone().with_metadata(metadata));
    let batch = RecordBatch::try_new(schema.clone(), batch.columns().to_vec()).map_err(|e| format!("batch: {}", e))?;
    // row groups of 8192 rows (ClickHouse's granule): a lookup by the key
    // reads the one group whose statistics admit it
    let props = WriterProperties::builder().set_compression(Compression::SNAPPY).set_max_row_group_row_count(Some(ROW_GROUP)).set_key_value_metadata(Some(vec![KeyValue::new(META.to_string(), stats)])).build();
    let mut buf = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut buf, schema, Some(props)).map_err(|e| format!("batch: {}", e))?;
        writer.write(&batch).map_err(|e| format!("batch: {}", e))?;
        writer.close().map_err(|e| format!("batch: {}", e))?;
    }
    place(dir, &segment_name(lo, hi), &buf)
}

/// `sys.batch_admits[path, column, value, token]`: whether the file may
/// hold `value` in `column` — false is certain, true means scan it. With
/// `token` true, whether the column's words may include `value`
/// (lowercased, one word). The value is written as text and read as the
/// column's kind: a moment as its milliseconds.
pub fn admits(path: &str, column: &str, value: &str, token: bool) -> Result<bool, String> {
    let stats: serde_json::Value = serde_json::from_str(&stats(path)?).map_err(|e| format!("batch: '{}' has unreadable statistics: {}", path, e))?;
    if token {
        let Some(filter) = stats["tokens"][column].as_str() else {
            return Err(format!("batch: '{}' indexes no words of column '{}'", path, column));
        };
        let bloom = Bloom::decode(filter).map_err(|e| format!("batch: '{}': {}", path, e))?;
        let word = value.to_lowercase();
        return Ok(bloom.has(word.as_bytes()));
    }
    let Some(col) = stats["columns"].get(column) else {
        return Err(format!("batch: '{}' holds no column '{}'", path, column));
    };
    let kind = col["kind"].as_str().unwrap_or("");
    let bytes = value_bytes(kind, value).map_err(|e| format!("batch: column '{}': {}", column, e))?;
    // a column of too many distinct values has no filter: it may hold anything
    let Some(encoded) = col["bloom"].as_str() else { return Ok(true) };
    let bloom = Bloom::decode(encoded).map_err(|e| format!("batch: '{}': {}", path, e))?;
    Ok(bloom.has(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textcol::Text;

    fn scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("crow-batch-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.to_string_lossy().to_string()
    }

    fn frame() -> CrowFrame {
        let mut t = Text::new();
        for s in ["Apple pie", "banana split", "Cherry tart", "apple crumble"] {
            t.push(s);
        }
        CrowFrame::new(
            vec!["id".into(), "price".into(), "name".into(), "ok".into()],
            vec![Column::Int(vec![3, 1, 2, 1]), Column::Float(vec![2.5, 1.0, f64::NAN, 0.5]), Column::Str(t), Column::Bool(vec![true, false, true, true])],
            vec![vec![true; 4], vec![true, true, true, false], vec![true; 4], vec![true; 4]],
        )
    }

    /// A frame round-trips: positions in arrival order, rows sorted by the
    /// keys (ties in arrival order), every cell and every missing cell as
    /// it was.
    #[test]
    fn a_frame_round_trips_sorted_by_its_keys() {
        let dir = scratch("round");
        let path = write(&dir, 7, 100, &frame(), &["id".to_string()], &["name".to_string()]).expect("written");
        assert!(path.ends_with("000000000007.batch"));
        let back = read(&path).expect("read back");
        assert_eq!(back.columns, vec!["id", "price", "name", "ok", POSITION]);
        let ids = match back.data[0].as_ref() { Column::Int(v) => v.clone(), _ => panic!("ints") };
        assert_eq!(ids, vec![1, 1, 2, 3]);
        let pos = match back.data[4].as_ref() { Column::Int(v) => v.clone(), _ => panic!("ints") };
        assert_eq!(pos, vec![101, 103, 102, 100]);
        let names = match back.data[2].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!("text") };
        assert_eq!(names, vec!["banana split", "apple crumble", "Cherry tart", "Apple pie"]);
        // the missing price (row 4 arrived, id 1) is still missing, and the NaN a value
        let present: Vec<bool> = back.validity[1].iter().copied().collect();
        assert_eq!(present, vec![true, false, true, true]);
        let prices = match back.data[1].as_ref() { Column::Float(v) => v.clone(), _ => panic!("floats") };
        assert!(prices[2].is_nan());
        let stats: serde_json::Value = serde_json::from_str(&stats(&path).unwrap()).unwrap();
        assert_eq!(stats["rows"], 4);
        assert_eq!(stats["first"], 100);
        assert_eq!(stats["last"], 103);
        assert_eq!(stats["columns"]["id"]["min"], 1);
        assert_eq!(stats["columns"]["id"]["max"], 3);
        assert_eq!(stats["columns"]["price"]["missing"], 1);
        assert_eq!(stats["columns"]["price"]["max"], 2.5);
        assert_eq!(stats["columns"]["name"]["min"], "Apple pie");
        assert_eq!(stats["columns"]["ok"]["kind"], "bool");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A value present is admitted; one absent is (almost always) not; a
    /// word of a text column is found lowercased; a word absent is not.
    #[test]
    fn a_filter_admits_what_is_there() {
        let dir = scratch("admits");
        let path = write(&dir, 1, 0, &frame(), &[], &["name".to_string()]).expect("written");
        assert!(admits(&path, "id", "3", false).unwrap());
        assert!(admits(&path, "id", " 1 ", false).unwrap());
        assert!(admits(&path, "price", "2.5", false).unwrap());
        assert!(admits(&path, "ok", "false", false).unwrap());
        assert!(admits(&path, "name", "Cherry tart", false).unwrap());
        assert!(admits(&path, "name", "APPLE", true).unwrap());
        assert!(admits(&path, "name", "crumble", true).unwrap());
        let absent = (1000..1200).filter(|i| admits(&path, "id", &i.to_string(), false).unwrap()).count();
        assert!(absent <= 10, "{} of 200 absent ints admitted", absent);
        assert!(!admits(&path, "name", "durian", true).unwrap());
        assert_eq!(admits(&path, "id", "x", false).unwrap_err(), "batch: column 'id': 'x' is no int");
        assert!(admits(&path, "nosuch", "1", false).unwrap_err().contains("holds no column 'nosuch'"));
        assert!(admits(&path, "id", "1", true).unwrap_err().contains("indexes no words of column 'id'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file cut short at any byte is refused with its reason, never a
    /// panic; a `.part` left by a crash is no batch; a number written
    /// twice is refused; what the writer refuses, it refuses before any
    /// file exists.
    #[test]
    fn a_torn_file_is_refused_and_a_number_is_written_once() {
        let dir = scratch("torn");
        let path = write(&dir, 2, 0, &frame(), &["id".to_string()], &[]).expect("written");
        let whole = std::fs::read(&path).unwrap();
        // the last eight bytes are the stream's end marker: a file cut
        // inside its rows is refused, one cut only there holds every row
        for cut in (0..whole.len() - 8).step_by(7) {
            let torn = std::path::Path::new(&dir).join("torn.batch");
            std::fs::write(&torn, &whole[..cut]).unwrap();
            let r = read(&torn.to_string_lossy());
            assert!(r.is_err(), "a file cut at {} of {} bytes read", cut, whole.len());
            assert!(stats(&torn.to_string_lossy()).is_err() || cut > 0);
        }
        let torn = std::path::Path::new(&dir).join("torn.batch");
        std::fs::write(&torn, &whole[..whole.len() - 8]).unwrap();
        assert_eq!(read(&torn.to_string_lossy()).unwrap().nrows(), 4);
        assert!(write(&dir, 2, 0, &frame(), &[], &[]).unwrap_err().contains("was written already"));
        assert!(write(&dir, 3, 0, &frame(), &["nosuch".to_string()], &[]).unwrap_err().contains("no column 'nosuch' to sort by"));
        assert!(write(&dir, 3, 0, &frame(), &[], &["id".to_string()]).unwrap_err().contains("holds no text"));
        assert!(!std::path::Path::new(&dir).join(file_name(3)).exists());
        let empty = CrowFrame::new(vec!["a".into()], vec![Column::Int(vec![])], vec![vec![]]);
        assert!(write(&dir, 3, 0, &empty, &[], &[]).unwrap_err().contains("at least one row"));
        assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two batches and a segment fold into one segment: the latest
    /// version of each key kept, sorted by key, positions as they were,
    /// the statistics and the word filters as a batch's; the segment reads
    /// through the same three functions, and merges again
    #[test]
    fn files_merge_into_a_segment_keeping_the_latest_version() {
        let dir = scratch("merge");
        let mut t = Text::new();
        for s in ["one", "two", "three"] {
            t.push(s);
        }
        let a = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![1, 2, 3]), Column::Str(t)], vec![vec![true; 3], vec![true; 3]]);
        let mut t2 = Text::new();
        t2.push("TWO again");
        t2.push("four");
        let b = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![2, 4]), Column::Str(t2)], vec![vec![true; 2], vec![true; 2]]);
        let pa = write(&dir, 0, 0, &a, &["id".to_string()], &["name".to_string()]).unwrap();
        let pb = write(&dir, 1, 3, &b, &["id".to_string()], &["name".to_string()]).unwrap();
        let seg = merge(&dir, &[pa.clone(), pb.clone()], &["id".to_string()], false).unwrap();
        assert!(seg.ends_with("000000000000-000000000001.segment"), "{}", seg);
        assert_eq!(covered("000000000000-000000000001.segment"), Some((0, 1)));
        let f = read(&seg).unwrap();
        let ids = match f.data[0].as_ref() { Column::Int(v) => v.clone(), _ => panic!() };
        assert_eq!(ids, vec![1, 2, 3, 4]);
        let names = match f.data[1].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!() };
        assert_eq!(names, vec!["one", "TWO again", "three", "four"]);
        let pos = match f.data[2].as_ref() { Column::Int(v) => v.clone(), _ => panic!() };
        assert_eq!(pos, vec![0, 3, 2, 4]);
        let st: serde_json::Value = serde_json::from_str(&stats(&seg).unwrap()).unwrap();
        assert_eq!(st["rows"], 4);
        assert_eq!(st["first"], 0);
        assert_eq!(st["last"], 4);
        assert_eq!(st["columns"]["id"]["max"], 4);
        assert!(admits(&seg, "name", "again", true).unwrap());
        assert!(!admits(&seg, "name", "durian", true).unwrap());
        assert!(admits(&seg, "id", "4", false).unwrap());
        // a segment and a new batch merge again; without keys every row stays
        let mut t3 = Text::new();
        t3.push("four again");
        let c = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![4]), Column::Str(t3)], vec![vec![true], vec![true]]);
        let pc = write(&dir, 2, 5, &c, &["id".to_string()], &[]).unwrap();
        let seg2 = merge(&dir, &[seg.clone(), pc.clone()], &[], false).unwrap();
        assert!(seg2.ends_with("000000000000-000000000002.segment"));
        assert_eq!(read(&seg2).unwrap().nrows(), 5);
        assert!(merge(&dir, &[seg.clone(), pc.clone()], &[], false).unwrap_err().contains("was written already"));
        assert!(merge(&dir, &[], &[], false).unwrap_err().contains("one file or more"));
        // every row kept, sorted by the key: all three versions of id 4 stay, in key order
        let mut t4 = Text::new();
        t4.push("four once more");
        let d = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![4]), Column::Str(t4)], vec![vec![true], vec![true]]);
        let pd = write(&dir, 3, 6, &d, &["id".to_string()], &[]).unwrap();
        let kept = merge(&dir, &[seg2.clone(), pd], &["id".to_string()], true).unwrap();
        assert!(kept.ends_with("000000000000-000000000003.segment"));
        let k = read(&kept).unwrap();
        assert_eq!(k.nrows(), 6);
        assert_eq!(match k.data[0].as_ref() { Column::Int(v) => v.clone(), _ => panic!() }, vec![1, 2, 3, 4, 4, 4]);
        assert!(merge(&dir, &[pa.clone()], &["nosuch".to_string()], false).unwrap_err().contains("no column 'nosuch'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lookup answers the rows holding the value through a batch and
    /// through a segment of many row groups, reading only the groups whose
    /// statistics admit it; a value no filter admits is an empty frame with
    /// the file's columns; a value of another kind is refused
    #[test]
    fn a_find_reads_the_row_groups_that_admit_the_value() {
        let dir = scratch("find");
        let n = 30_000i64;
        let ids: Vec<i64> = (0..n).rev().collect();
        let mut names = Text::new();
        for i in &ids {
            names.push(&format!("row {}", i));
        }
        let f = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(ids), Column::Str(names)], vec![vec![true; n as usize], vec![true; n as usize]]);
        let pa = write(&dir, 0, 0, &f, &["id".to_string()], &[]).unwrap();
        let hit = find(&pa, "id", "12345").unwrap();
        assert_eq!(hit.nrows(), 1);
        assert_eq!(match hit.data[2].as_ref() { Column::Int(v) => v[0], _ => panic!() }, n - 1 - 12345);
        let none = find(&pa, "id", "-5").unwrap();
        assert_eq!(none.nrows(), 0);
        assert_eq!(none.columns, vec!["id", "name", POSITION]);
        let seg = merge(&dir, &[pa.clone()], &["id".to_string()], false).unwrap();
        let hit = find(&seg, "id", "12345").unwrap();
        assert_eq!(hit.nrows(), 1);
        assert_eq!(match hit.data[1].as_ref() { Column::Str(t) => t.get(0).to_string(), _ => panic!() }, "row 12345");
        assert_eq!(find(&seg, "name", "row 7").unwrap().nrows(), 1);
        assert_eq!(find(&seg, "id", "99999999").unwrap().nrows(), 0);
        assert_eq!(find(&seg, "id", "x").unwrap_err(), "batch: column 'id': 'x' is no int");
        assert!(find(&seg, "nosuch", "1").unwrap_err().contains("holds no column 'nosuch'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_count_and_tokens_split() {
        assert_eq!(file_name(17), "000000000017.batch");
        assert_eq!(number_of("000000000017.batch"), Some(17));
        assert_eq!(number_of("17.batch"), None);
        assert_eq!(number_of(".000000000017.batch.part"), None);
        assert_eq!(segment_name(3, 17), "000000000003-000000000017.segment");
        assert_eq!(covered("000000000017.batch"), Some((17, 17)));
        assert_eq!(covered("x-y.segment"), None);
        assert_eq!(tokens("Hello, World! it's 2026-10-25"), vec!["hello", "world", "it", "s", "2026", "10", "25"]);
        let mut b = Bloom::sized(3);
        b.add(b"x");
        let again = Bloom::decode(&b.encode()).unwrap();
        assert!(again.has(b"x"));
        assert!(!again.has(b"y"));
    }
}

#[cfg(all(test, feature = "parquet"))]
mod profile {
    //! Where a day's read goes: run by hand on a real segment,
    //! `CROW_SEGMENT=path cargo test --release -p crow_runtime --features parquet profile -- --ignored --nocapture`
    use super::*;

    #[test]
    #[ignore]
    fn stages_of_a_days_read() {
        use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder};
        let Ok(path) = std::env::var("CROW_SEGMENT") else { return };
        let day = std::env::var("CROW_DAY").unwrap_or("2026-09-17".to_string());
        let t = std::time::Instant::now();
        let file = std::fs::File::open(&path).unwrap();
        let metadata = ArrowReaderMetadata::load(&file, ArrowReaderOptions::new()).unwrap();
        println!("footer: {:?} ({} groups)", t.elapsed(), metadata.metadata().num_row_groups());
        let want = Parsed::Text(day.clone());
        let leaf = metadata.schema().fields().iter().position(|f| f.name() == "day").unwrap();
        let groups: Vec<usize> = (0..metadata.metadata().num_row_groups()).filter(|&g| group_admits(metadata.metadata().row_group(g).column(leaf).statistics(), &want)).collect();
        println!("groups admitted: {}", groups.len());
        // a: decode to record batches only, one reader
        let t = std::time::Instant::now();
        let file = std::fs::File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata.clone()).with_row_groups(groups.clone()).with_batch_size(ROW_GROUP).build().unwrap();
        let batches: Vec<_> = reader.map(|b| b.unwrap()).collect();
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        println!("decode to arrow, one reader: {:?} ({} rows, {} batches)", t.elapsed(), rows, batches.len());
        // b: conversion of those batches
        let t = std::time::Instant::now();
        let schema = metadata.schema().clone();
        let f = crate::frame::frame_of_batches(&schema, batches.into_iter().map(Ok), "batch").unwrap();
        println!("arrow to frame: {:?} ({} rows)", t.elapsed(), f.nrows());
        // c: the parallel reader as built
        let t = std::time::Instant::now();
        let f = read_groups(&path, &groups).unwrap();
        println!("read_groups across the cores: {:?} ({} rows)", t.elapsed(), f.nrows());
        // d: per column decode cost: projection of one column at a time
        for (i, field) in schema.fields().iter().enumerate() {
            let t = std::time::Instant::now();
            let file = std::fs::File::open(&path).unwrap();
            let mask = parquet::arrow::ProjectionMask::leaves(metadata.parquet_schema(), [i]);
            let reader = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata.clone()).with_projection(mask).with_row_groups(groups.clone()).with_batch_size(ROW_GROUP).build().unwrap();
            let n: usize = reader.map(|b| b.unwrap().num_rows()).sum();
            println!("  column {} ({:?}): {:?} ({} rows)", field.name(), field.data_type(), t.elapsed(), n);
        }
    }
}
