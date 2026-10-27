//! Magpie's kernel: the files of a log (spec, magpie), written and read by
//! Magpie's own format (file.rs) — no Arrow, no Parquet, no codec but our
//! own. A batch file is one commit: a frame given a position per row in
//! arrival order, sorted by the log's keys, in row groups of a few
//! megabytes, each column chunk encoded by its kind and compressed where
//! that pays, with the statistics a reader prunes by per column per group
//! (min, max, missing, distinct, a bloom filter) and a token filter per
//! indexed text column. One file is at once the log entry (positions), the
//! level-0 segment (sorted, with statistics) and the search index. It is
//! written to a temporary name, flushed to the disk (`sync_all`:
//! `F_FULLFSYNC` on macOS, past the drive's cache), linked into place, and
//! the directory flushed, so a crash leaves the log as it was or with the
//! whole file. A merge folds many files into one segment of the same
//! format, named by the batch numbers it covers, the latest version of
//! each key kept where keys are given. Both backends call these
//! functions; the Crow-written module over them is `magpie`.

pub mod bloom;
pub mod codec;
pub mod encode;
pub mod exec;
pub mod file;
pub mod inflate;
pub mod manifest;
pub mod merge;
pub mod pack;
pub mod postings;
pub mod search;
pub mod snappy;
pub mod stem;
pub mod tail;

use crate::frame::{Column, CrowFrame};
use encode::Kind;
use file::{Bound, Entry, Reader, Writer};
use std::io::Write;

pub use file::tokens;

/// The column every batch file adds: the row's position in the log.
pub const POSITION: &str = "_pos";
/// A row group's size in bytes, as the frame holds it: a lookup decodes
/// one group, so a group is small; the index is read per column, so many
/// groups cost a reader of two columns nothing.
pub const GROUP_BYTES: usize = 4 << 20;
/// The fewest rows a group holds, whatever its bytes.
const GROUP_ROWS_LEAST: usize = 1024;
const EXTENSION: &str = ".batch";
const SEGMENT: &str = ".segment";

/// The format this Crow writes and reads (file.rs: `VERSION`).
pub const FORMAT: u64 = file::VERSION as u64;

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

/// A segment's name by its counter (`S000000000017.segment`): the
/// manifest says what it holds.
pub fn segment_name_of(counter: i64) -> String {
    format!("S{:012}{}", counter, SEGMENT)
}

/// The counter a segment's name says, or none for another file.
pub fn segment_counter(name: &str) -> Option<i64> {
    let stem = name.strip_suffix(SEGMENT)?.strip_prefix('S')?;
    if stem.len() != 12 || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// Whether a file's name is a batch's or a segment's.
pub fn is_log_file(name: &str) -> bool {
    number_of(name).is_some() || segment_counter(name).is_some()
}

/// What a row of the frame takes, as the frame holds it.
fn bytes_per_row(frame: &CrowFrame) -> usize {
    let n = frame.nrows().max(1);
    frame
        .data
        .iter()
        .map(|c| match c.as_ref() {
            Column::Int(_) | Column::Float(_) => 8,
            Column::Bool(_) => 1,
            Column::DateTime(..) => 10,
            Column::Str(t) => t.bytes_held() / n + 4,
        })
        .sum::<usize>()
        .max(1)
}

/// How many rows a group of the frame holds.
fn rows_per_group(frame: &CrowFrame) -> usize {
    (GROUP_BYTES / bytes_per_row(frame)).max(GROUP_ROWS_LEAST)
}

/// The frame as a file's bytes: its columns in order, the text columns
/// named in `texts` indexed, in groups of `rows_per_group` rows.
fn file_of(frame: &CrowFrame, first: i64, last: i64, keys: &[String], texts: &[String], narrow: &[String], stems: bool) -> Result<Vec<u8>, String> {
    let mut columns = Vec::with_capacity(frame.columns.len());
    for (i, name) in frame.columns.iter().enumerate() {
        let kind = Kind::of(frame.data[i].as_ref());
        let indexed = texts.contains(name);
        if indexed && kind != Kind::Text {
            return Err(format!("column '{}' holds no text to index the words of", name));
        }
        columns.push((name.clone(), kind, indexed));
    }
    for t in texts {
        if frame.col_index(t).is_none() {
            return Err(format!("no column '{}' to index the words of", t));
        }
    }
    let narrowed: Vec<bool> = columns.iter().map(|(n, k, _)| *k == Kind::Float && narrow.contains(n)).collect();
    let mut w = Writer::with_narrow(columns, narrowed);
    w.stems = stems;
    let n = frame.nrows();
    let per = rows_per_group(frame);
    let mut start = 0;
    while start < n {
        let end = (start + per).min(n);
        w.group(frame, start, end)?;
        start = end;
    }
    Ok(w.finish(first, last, keys, &[]))
}

/// `sys.batch_write[dir, number, first, frame, keys, texts, flush]`: the
/// frame written as batch file `number` under `dir`, its rows given the
/// positions from `first` in the order they arrive and sorted by `keys`
/// (ties in arrival order), the words of each column in `texts` indexed.
/// Answers the file's path. Refused: a frame with no rows, a key or a text
/// column the frame lacks, a column already named `_pos`, a number whose
/// file exists.
pub fn write(dir: &str, number: i64, first: i64, frame: &CrowFrame, keys: &[String], texts: &[String], flush: bool) -> Result<String, String> {
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
    let listed = manifest::read(dir)?;
    let narrow = listed.as_ref().map(|m| m.narrow()).unwrap_or_default();
    let stems = listed.as_ref().is_some_and(|m| m.stems());
    let bytes = file_of(&sorted, first, first + n as i64 - 1, keys, texts, &narrow, stems).map_err(|e| format!("batch: {}", e))?;
    place(dir, &file_name(number), &bytes, flush)
}

/// The bytes written as a file of the directory: to a temporary name,
/// flushed to the disk, linked into place — a link, never a rename, so a
/// file of the name that appeared meanwhile is not overwritten and the
/// writer learns it lost the number — then the directory flushed, or a
/// crash could forget the name of a file whose bytes it kept.
pub fn place(dir: &str, name: &str, bytes: &[u8], flush: bool) -> Result<String, String> {
    let path = std::path::Path::new(dir).join(name);
    let part = std::path::Path::new(dir).join(format!(".{}.part", name));
    let cannot = |what: &str, e: std::io::Error| format!("batch: cannot {} '{}': {}", what, path.display(), e);
    let mut file = std::fs::File::create(&part).map_err(|e| cannot("write", e))?;
    file.write_all(bytes).map_err(|e| cannot("write", e))?;
    if flush {
        file.sync_all().map_err(|e| cannot("flush", e))?;
    }
    drop(file);
    let linked = std::fs::hard_link(&part, &path);
    let _ = std::fs::remove_file(&part);
    if let Err(e) = linked {
        if path.exists() {
            return Err(format!("batch: '{}' exists — it was written already", path.display()));
        }
        return Err(cannot("place", e));
    }
    if flush && let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(path.to_string_lossy().to_string())
}

/// `sys.batch_read[path, wanted]`: the batch file, or the segment, as the
/// frame it holds with the columns `wanted` (every column when none is
/// named), its position column included. A file cut short is refused
/// with its reason.
pub fn read(path: &str, wanted: &[String]) -> Result<CrowFrame, String> {
    let r = Reader::open(path)?;
    let cols = r.wanted(wanted)?;
    let groups: Vec<usize> = (0..r.footer().groups.len()).collect();
    r.read(&groups, &cols)
}

/// `sys.batch_stats[path]`: the file's facts as JSON text, read from its
/// footer alone — `format`, `rows`, `first`, `last`, `keys`, `groups`,
/// `columns` (each what its `cells` are and whether it is `indexed`) and `tokens`
/// (each indexed column's `docs` and `words` over the file).
pub fn stats(path: &str) -> Result<String, String> {
    use serde_json::{json, Map, Value};
    let r = Reader::open(path)?;
    let f = r.footer();
    let mut columns = Map::new();
    let mut tokens = Map::new();
    for (i, c) in f.columns.iter().enumerate() {
        columns.insert(c.name.clone(), json!({ "cells": c.kind.word(), "indexed": c.indexed }));
        if c.indexed {
            let entries = r.entries(i)?;
            let docs: u64 = entries.iter().filter_map(|e| e.tokens.as_ref()).map(|t| t.docs as u64).sum();
            let words: u64 = entries.iter().filter_map(|e| e.tokens.as_ref()).map(|t| t.words as u64).sum();
            tokens.insert(c.name.clone(), json!({ "docs": docs, "words": words }));
        }
    }
    let stats = json!({
        "format": FORMAT,
        "rows": f.rows,
        "first": f.first,
        "last": f.last,
        "keys": f.keys,
        "groups": f.groups.len(),
        "columns": Value::Object(columns),
        "tokens": Value::Object(tokens),
    });
    Ok(stats.to_string())
}

/// A value written as text, read as a column's kind.
pub(crate) enum Parsed {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

pub(crate) fn parsed(kind: Kind, value: &str) -> Result<Parsed, String> {
    Ok(match kind {
        Kind::Int => Parsed::Int(value.trim().parse::<i64>().map_err(|_| format!("'{}' is no int", value))?),
        Kind::Moment => Parsed::Int(value.trim().parse::<i64>().map_err(|_| format!("'{}' is no moment's milliseconds", value))?),
        Kind::Float => Parsed::Float(value.trim().parse::<f64>().map_err(|_| format!("'{}' is no number", value))?),
        Kind::Bool => match value.trim() {
            "true" => Parsed::Bool(true),
            "false" => Parsed::Bool(false),
            _ => return Err(format!("'{}' is no bool", value)),
        },
        Kind::Text => Parsed::Text(value.to_string()),
    })
}

/// The bytes a value hashes to in a column's filter.
pub(crate) fn parsed_bytes(p: &Parsed) -> Vec<u8> {
    match p {
        Parsed::Int(v) => v.to_le_bytes().to_vec(),
        Parsed::Float(x) => {
            let x = if *x == 0.0 { 0.0 } else { *x };
            x.to_bits().to_le_bytes().to_vec()
        }
        Parsed::Bool(b) => vec![*b as u8],
        Parsed::Text(s) => s.as_bytes().to_vec(),
    }
}

/// Whether a group's bounds admit the value.
fn bounds_admit(e: &Entry, want: &Parsed) -> bool {
    let below = |b: &Bound| match (b, want) {
        (Bound::Int(m), Parsed::Int(w)) => m <= w,
        (Bound::Float(m), Parsed::Float(w)) => m <= w,
        (Bound::Bool(m), Parsed::Bool(w)) => m <= w,
        (Bound::Text(m), Parsed::Text(w)) => m <= w,
        _ => true,
    };
    let above = |b: &Bound| match (b, want) {
        (Bound::Int(m), Parsed::Int(w)) => m >= w,
        (Bound::Float(m), Parsed::Float(w)) => m >= w,
        (Bound::Bool(m), Parsed::Bool(w)) => m >= w,
        (Bound::Text(m), Parsed::Text(w)) => m >= w,
        _ => true,
    };
    if e.rows > 0 && e.missing == e.rows {
        return false;
    }
    e.min.as_ref().is_none_or(below) && e.max.as_ref().is_none_or(above)
}

/// The rows of a frame whose columns hold the values, every pair at once.
fn rows_holding(frame: &CrowFrame, columns: &[String], wants: &[Parsed]) -> Result<CrowFrame, String> {
    let n = frame.nrows();
    let mut mask = vec![true; n];
    for (column, want) in columns.iter().zip(wants) {
        let c = frame.col_index(column).ok_or_else(|| format!("batch: no column '{}'", column))?;
        let valid = frame.validity[c].as_ref();
        match (frame.data[c].as_ref(), want) {
            (Column::Int(v), Parsed::Int(w)) => (0..n).for_each(|i| mask[i] &= valid[i] && v[i] == *w),
            (Column::DateTime(ms, _), Parsed::Int(w)) => (0..n).for_each(|i| mask[i] &= valid[i] && ms[i] == *w),
            (Column::Float(v), Parsed::Float(w)) => (0..n).for_each(|i| mask[i] &= valid[i] && v[i] == *w),
            (Column::Bool(v), Parsed::Bool(w)) => (0..n).for_each(|i| mask[i] &= valid[i] && v[i] == *w),
            (Column::Str(t), Parsed::Text(w)) => (0..n).for_each(|i| mask[i] &= valid[i] && t.get(i) == w),
            _ => return Err(format!("batch: column '{}' holds another kind than the value", column)),
        }
    }
    frame.pick_rows(&mask)
}

/// A frame's columns named, in the order named; a name the frame lacks is
/// refused.
fn only(frame: CrowFrame, wanted: &[String], path: &str) -> Result<CrowFrame, String> {
    if wanted.is_empty() {
        return Ok(frame);
    }
    for w in wanted {
        if frame.col_index(w).is_none() {
            return Err(format!("batch: '{}' holds no column '{}'", path, w));
        }
    }
    let names: Vec<&str> = wanted.iter().map(String::as_str).collect();
    frame.select(&names)
}

/// `sys.batch_find[path, columns, values, wanted]`: the rows of the file
/// whose `columns` hold the `values`, every pair at once (each value
/// written as text, read as its column's kind), with the columns
/// `wanted` (every column when none is named). Each key column's groups
/// are asked through their bounds and their filters first, and only the
/// groups every key admits are decoded — one, for keys the file is sorted
/// by, which is how a question about one key on one day reads one group
/// and not the key's whole month.
pub fn find(path: &str, columns: &[String], values: &[String], wanted: &[String]) -> Result<CrowFrame, String> {
    if columns.is_empty() {
        return Err("batch: a find names at least one column".to_string());
    }
    if columns.len() != values.len() {
        return Err(format!("batch: {} columns and {} values — a find takes one value per column", columns.len(), values.len()));
    }
    let r = Reader::open(path)?;
    let f = r.footer();
    let mut wants: Vec<Parsed> = Vec::with_capacity(columns.len());
    let mut admitted: Vec<bool> = vec![true; f.groups.len()];
    for (column, value) in columns.iter().zip(values) {
        let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", path, column))?;
        let want = parsed(f.columns[c].kind, value).map_err(|e| format!("batch: column '{}': {}", column, e))?;
        let bytes = parsed_bytes(&want);
        let entries = r.entries(c)?;
        for (g, e) in entries.iter().enumerate() {
            if !admitted[g] {
                continue;
            }
            if !bounds_admit(e, &want) {
                admitted[g] = false;
                continue;
            }
            if let Some(bloom) = r.bloom(e)?
                && !bloom.has(&bytes)
            {
                admitted[g] = false;
            }
        }
        wants.push(want);
    }
    // the columns a read must hold: the ones wanted, plus the keys, which
    // the rows are picked by; the keys are dropped again where not wanted
    let mut needed: Vec<String> = wanted.to_vec();
    if !wanted.is_empty() {
        for c in columns {
            if !needed.contains(c) {
                needed.push(c.clone());
            }
        }
    }
    let cols = r.wanted(&needed)?;
    let groups: Vec<usize> = (0..f.groups.len()).filter(|&g| admitted[g]).collect();
    let frame = r.read(&groups, &cols)?;
    let held = rows_holding(&frame, columns, &wants)?;
    only(held, wanted, path)
}

/// An empty frame with a file's columns, a score and, where asked, a snippet.
fn empty_scored(path: Option<&String>, snippet: bool) -> Result<CrowFrame, String> {
    let base = match path {
        None => CrowFrame::new(Vec::new(), Vec::new(), Vec::new()),
        Some(path) => {
            let r = Reader::open(path)?;
            let all: Vec<usize> = (0..r.footer().columns.len()).collect();
            r.empty(&all)
        }
    };
    let scored = base.with_column("score", Column::Float(Vec::new()))?;
    if snippet { scored.with_column("snippet", Column::Str(crate::textcol::Text::new())) } else { Ok(scored) }
}

/// `sys.batch_search[paths, column, words, k, stems, snippet]`: the `k`
/// rows of the files whose text `column` best match the words — a word,
/// a phrase (a word with a space), a prefix (`kit*`), a fuzzy word
/// (`kitchn~`) — with the files' columns and a `score` (and a `snippet`
/// of that many characters around the first hit where `snippet` is more
/// than 0), the best first and an earlier position first between
/// equals. BM25 as Lucene's `BM25Similarity` scores it, the words
/// stemmed where `stems`, over the files as one corpus (search.rs).
pub fn search(paths: &[String], column: &str, words: &[String], k: i64, stems: bool, snippet: i64) -> Result<CrowFrame, String> {
    search::search(paths, column, words, k, stems, snippet)
}

/// The frames of several files stacked, sorted by position, `_pos` dropped
/// where it was not asked for.
fn gathered(parts: Vec<CrowFrame>, wanted: &[String]) -> Result<CrowFrame, String> {
    if parts.is_empty() {
        return Ok(CrowFrame::new(Vec::new(), Vec::new(), Vec::new()));
    }
    let full: Vec<CrowFrame> = parts.iter().filter(|p| p.nrows() > 0).cloned().collect();
    let whole = if full.is_empty() { parts[0].clone() } else { CrowFrame::stack_all(&full).map_err(|e| format!("batch: the files hold different columns: {}", e))? };
    let ordered = if whole.nrows() > 1 && whole.col_index(POSITION).is_some() { whole.sort(POSITION, false)? } else { whole };
    if wanted.is_empty() || wanted.iter().any(|w| w == POSITION) {
        return Ok(ordered);
    }
    let names: Vec<&str> = wanted.iter().map(String::as_str).collect();
    ordered.select(&names)
}

/// The columns a read must hold for a pick by position: the ones wanted
/// and `_pos` (every column when none is named).
fn with_position(wanted: &[String]) -> Vec<String> {
    if wanted.is_empty() || wanted.iter().any(|w| w == POSITION) {
        return wanted.to_vec();
    }
    let mut needed = wanted.to_vec();
    needed.push(POSITION.to_string());
    needed
}

/// `sys.batch_since[paths, after, upto, wanted]`: the rows of the files
/// at a position past `after` and at or before `upto` (every position
/// past `after` with -1), in position order, with the columns `wanted`
/// (every column when none is named; `_pos` read for the picking and
/// dropped where not asked). A file's groups whose positions all lie
/// before `after` or all past `upto` are not read; the files are read
/// across the cores.
pub fn since(paths: &[String], after: i64, upto: i64, wanted: &[String]) -> Result<CrowFrame, String> {
    let needed = with_position(wanted);
    let inside = |x: i64| x > after && (upto < 0 || x <= upto);
    let parts: Vec<Result<CrowFrame, String>> = crate::cores::map_items(paths, paths.len() * GROUP_ROWS_LEAST, |p| {
        let r = Reader::open(p)?;
        let f = r.footer();
        let pc = f.column(POSITION).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, POSITION))?;
        let entries = r.entries(pc)?;
        // a group whose positions all lie before `after` or all past `upto` is not read
        let groups: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !(matches!(e.max, Some(Bound::Int(m)) if m <= after) || (upto >= 0 && matches!(e.min, Some(Bound::Int(m)) if m > upto))))
            .map(|(g, _)| g)
            .collect();
        let cols = r.wanted(&needed)?;
        let frame = r.read(&groups, &cols)?;
        let c = frame.col_index(POSITION).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, POSITION))?;
        let Column::Int(pos) = frame.data[c].as_ref() else {
            return Err(format!("batch: '{}' holds no positions", p));
        };
        if pos.iter().all(|&x| inside(x)) {
            return Ok(frame);
        }
        let mask: Vec<bool> = pos.iter().map(|&x| inside(x)).collect();
        frame.pick_rows(&mask)
    });
    gathered(parts.into_iter().collect::<Result<_, _>>()?, wanted)
}

/// Whether a parsed value is at or past another of its kind.
fn parsed_le(a: &Parsed, b: &Parsed) -> bool {
    match (a, b) {
        (Parsed::Int(x), Parsed::Int(y)) => x <= y,
        (Parsed::Float(x), Parsed::Float(y)) => x <= y,
        (Parsed::Bool(x), Parsed::Bool(y)) => x <= y,
        (Parsed::Text(x), Parsed::Text(y)) => x <= y,
        _ => true,
    }
}

/// Whether a bound is at or past a parsed value (true where the kinds differ).
fn bound_at_least(b: &Bound, p: &Parsed) -> bool {
    match (b, p) {
        (Bound::Int(x), Parsed::Int(y)) => x >= y,
        (Bound::Float(x), Parsed::Float(y)) => x >= y,
        (Bound::Bool(x), Parsed::Bool(y)) => x >= y,
        (Bound::Text(x), Parsed::Text(y)) => x >= y,
        _ => true,
    }
}

fn bound_at_most(b: &Bound, p: &Parsed) -> bool {
    match (b, p) {
        (Bound::Int(x), Parsed::Int(y)) => x <= y,
        (Bound::Float(x), Parsed::Float(y)) => x <= y,
        (Bound::Bool(x), Parsed::Bool(y)) => x <= y,
        (Bound::Text(x), Parsed::Text(y)) => x <= y,
        _ => true,
    }
}

/// Whether a cell lies within the parsed bounds (a missing cell never does).
fn cell_within(col: &Column, i: usize, lo: Option<&Parsed>, hi: Option<&Parsed>) -> bool {
    let cell = match col {
        Column::Int(v) => Parsed::Int(v[i]),
        Column::DateTime(ms, _) => Parsed::Int(ms[i]),
        Column::Float(v) => Parsed::Float(v[i]),
        Column::Bool(v) => Parsed::Bool(v[i]),
        Column::Str(t) => Parsed::Text(t.get(i).to_string()),
    };
    lo.is_none_or(|l| parsed_le(l, &cell)) && hi.is_none_or(|h| parsed_le(&cell, h))
}

/// `sys.batch_between[paths, column, lo, hi, upto, wanted]`: the rows of
/// the files whose `column` lies within `lo` and `hi` (each a value as
/// text, read as the column's kind; empty text is no bound) at a
/// position at or before `upto` (every position with -1), in position
/// order, with the columns `wanted`; a group whose bounds lie outside
/// the range, or whose positions lie past `upto`, is not read. Keys a
/// file is sorted by read the groups the range covers.
pub fn between(paths: &[String], column: &str, lo: &str, hi: &str, upto: i64, wanted: &[String]) -> Result<CrowFrame, String> {
    let needed = with_position(wanted);
    let parts: Vec<Result<CrowFrame, String>> = crate::cores::map_items(paths, paths.len() * GROUP_ROWS_LEAST, |p| {
        let r = Reader::open(p)?;
        let f = r.footer();
        let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, column))?;
        let kind = f.columns[c].kind;
        let low = if lo.is_empty() { None } else { Some(parsed(kind, lo).map_err(|e| format!("batch: column '{}': {}", column, e))?) };
        let high = if hi.is_empty() { None } else { Some(parsed(kind, hi).map_err(|e| format!("batch: column '{}': {}", column, e))?) };
        let entries = r.entries(c)?;
        let pc = f.column(POSITION).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, POSITION))?;
        let positions = r.entries(pc)?;
        let groups: Vec<usize> = entries
            .iter()
            .zip(positions.iter())
            .enumerate()
            .filter(|(_, (e, pe))| {
                let in_range = low.as_ref().is_none_or(|l| e.max.as_ref().is_none_or(|m| bound_at_least(m, l))) && high.as_ref().is_none_or(|h| e.min.as_ref().is_none_or(|m| bound_at_most(m, h)));
                in_range && !(upto >= 0 && matches!(pe.min, Some(Bound::Int(m)) if m > upto))
            })
            .map(|(g, _)| g)
            .collect();
        let mut cols_needed = needed.clone();
        if !cols_needed.is_empty() && !cols_needed.iter().any(|w| w == column) {
            cols_needed.push(column.to_string());
        }
        let cols = r.wanted(&cols_needed)?;
        let frame = r.read(&groups, &cols)?;
        let ci = frame.col_index(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, column))?;
        let pi = frame.col_index(POSITION).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, POSITION))?;
        let Column::Int(pos) = frame.data[pi].as_ref() else {
            return Err(format!("batch: '{}' holds no positions", p));
        };
        let valid = frame.validity[ci].as_ref();
        let col = frame.data[ci].as_ref();
        let mask: Vec<bool> = (0..frame.nrows()).map(|i| valid[i] && cell_within(col, i, low.as_ref(), high.as_ref()) && (upto < 0 || pos[i] <= upto)).collect();
        let picked = frame.pick_rows(&mask)?;
        if cols_needed.len() != needed.len() { only(picked, &needed, p) } else { Ok(picked) }
    });
    gathered(parts.into_iter().collect::<Result<_, _>>()?, wanted)
}

/// `sys.batch_find[paths, columns, values, wanted]`: `find` over several
/// files, the rows in position order, `_pos` dropped where not asked.
pub fn find_in(paths: &[String], columns: &[String], values: &[String], wanted: &[String]) -> Result<CrowFrame, String> {
    let needed = with_position(wanted);
    let parts: Vec<Result<CrowFrame, String>> = crate::cores::map_items(paths, paths.len() * GROUP_ROWS_LEAST, |p| find(p, columns, values, &needed));
    gathered(parts.into_iter().collect::<Result<_, _>>()?, wanted)
}

/// `sys.batch_merge[dir, paths, every]`: the files (batches, segments)
/// folded into segments under `dir` by the streaming merge (merge.rs):
/// one row group per input in memory, the rows in the log's keys' order
/// then position, the latest version of each key kept where the log has
/// keys and `every` is false (every row kept otherwise: an append-only
/// table sorted for pruning, its keys no identity), the statistics as a
/// batch's, the words of every column any input indexed indexed again, a
/// segment cut at `SEGMENT_BYTES` and named by the next counter. Then
/// the manifest is rewritten: the new segments at the level past their
/// inputs' (a batch is level 0), with the batch numbers and positions
/// they cover and their key ranges, and no longer the inputs that were
/// segments. Answers the first segment's path; the inputs are left for
/// the caller to remove once the manifest names the segments. Refused:
/// no files, a file that is neither, files of different columns.
/// `CROW_MAGPIE_CRASH` set to `placed` ends the process after the
/// segments are placed and before the manifest is switched, `listed`
/// after the switch: the crash harness's two cuts.
pub fn merge(dir: &str, paths: &[String], every: bool) -> Result<String, String> {
    if paths.is_empty() {
        return Err("batch: a merge takes one file or more".to_string());
    }
    let mut listed = manifest::read(dir)?.unwrap_or_default();
    let keys: Vec<String> = listed.keys.clone();
    let narrow = listed.narrow();
    let stems = listed.stems();
    let mut texts: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut inputs: Vec<String> = Vec::with_capacity(paths.len());
    let (mut number, mut high) = (i64::MAX, i64::MIN);
    let mut level = 0;
    for p in paths {
        let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Some(n) = number_of(&name) {
            number = number.min(n);
            high = high.max(n);
        } else if segment_counter(&name).is_some() {
            let s = listed.segments.iter().find(|s| s.name == name).ok_or_else(|| format!("batch: '{}' is a segment the manifest does not name", p))?;
            number = number.min(s.number);
            high = high.max(s.high);
            level = level.max(s.level);
        } else {
            return Err(format!("batch: '{}' is neither a batch nor a segment by its name", p));
        }
        // every text column is indexed by the merge: the producer declares
        // nothing, and an identifier-like column gives its index up per group
        let r = Reader::open(p)?;
        for c in &r.footer().columns {
            if c.kind == Kind::Text {
                texts.insert(c.name.clone());
            }
        }
        inputs.push(name);
    }
    let texts: Vec<String> = texts.into_iter().collect();
    // the next counter is past every segment the manifest names and every
    // segment file on disk: a crash's leftover keeps its name
    let mut counter = listed.counter();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(n) = segment_counter(&e.file_name().to_string_lossy()) {
                counter = counter.max(n);
            }
        }
    }
    let written = merge::stream(dir, paths, &keys, &texts, &narrow, stems, every, &mut counter, GROUP_BYTES, merge::SEGMENT_BYTES)?;
    if written.is_empty() {
        return Err("batch: the merge wrote no segment".to_string());
    }
    crash_at("placed");
    listed.segments.retain(|s| !inputs.contains(&s.name));
    for w in &written {
        let path = std::path::Path::new(dir).join(&w.name).to_string_lossy().to_string();
        let ranges = key_ranges(&path, &keys)?;
        listed.segments.push(manifest::Segment { name: w.name.clone(), level: level + 1, number, high, first: w.first, last: w.last, rows: w.rows, bytes: w.bytes, ranges });
    }
    listed.segments.sort_by(|a, b| a.first.cmp(&b.first).then(a.name.cmp(&b.name)));
    manifest::write(dir, &listed)?;
    crash_at("listed");
    Ok(std::path::Path::new(dir).join(&written[0].name).to_string_lossy().to_string())
}

/// The crash harness's cut: the process ends here where
/// `CROW_MAGPIE_CRASH` names this step (`placed` and `listed` in the
/// merge, `pending` after a put, before its flush).
pub(crate) fn crash_at(step: &str) {
    let Ok(asked) = std::env::var("CROW_MAGPIE_CRASH") else { return };
    let (name, nth) = asked.split_once(':').map(|(n, k)| (n.to_string(), k.parse::<usize>().unwrap_or(1))).unwrap_or((asked.clone(), 1));
    if name != step {
        return;
    }
    static REACHED: std::sync::Mutex<Vec<(String, usize)>> = std::sync::Mutex::new(Vec::new());
    let count = {
        let mut r = REACHED.lock().unwrap_or_else(|p| p.into_inner());
        match r.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => {
                *c += 1;
                *c
            }
            None => {
                r.push((name.clone(), 1));
                1
            }
        }
    };
    if count == nth {
        crate::display::flush_output();
        std::process::exit(3);
    }
}

/// The log's live files as the kernel sees them: the manifest's segments
/// and the batch files no segment absorbed (by number), their paths.
pub fn live_paths(dir: &str) -> Result<Vec<String>, String> {
    let listed = manifest::read(dir)?.unwrap_or_default();
    let mut paths: Vec<String> = listed.segments.iter().map(|s| std::path::Path::new(dir).join(&s.name).to_string_lossy().to_string()).collect();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("batch: cannot read '{}': {}", dir, e))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(n) = number_of(&name)
            && !listed.segments.iter().any(|s| s.number <= n && n <= s.high)
        {
            paths.push(e.path().to_string_lossy().to_string());
        }
    }
    Ok(paths)
}

/// The log's last position over its files, -1 for none.
pub fn head_of(dir: &str) -> Result<i64, String> {
    let mut head = -1;
    for p in live_paths(dir)? {
        head = head.max(Reader::open(&p)?.footer().last);
    }
    Ok(head)
}

/// The next batch number: past every batch file's and every segment's `high`.
pub fn next_number(dir: &str) -> Result<i64, String> {
    let listed = manifest::read(dir)?.unwrap_or_default();
    let mut top = listed.segments.iter().map(|s| s.high).max().unwrap_or(-1);
    let entries = std::fs::read_dir(dir).map_err(|e| format!("batch: cannot read '{}': {}", dir, e))?;
    for e in entries.flatten() {
        if let Some(n) = number_of(&e.file_name().to_string_lossy()) {
            top = top.max(n);
        }
    }
    Ok(top + 1)
}

/// The table the log holds — the newest version of each key over the
/// files and the pending rows, in position order — for the image.
pub fn latest_of(dir: &str, keys: &[String]) -> Result<CrowFrame, String> {
    // the pending rows before the files: the flush moves rows from one to
    // the other under the tail's lock, so read in this order they are seen
    // at least once, and a file covering their first position says they
    // were flushed in between (then that file holds them)
    let waiting = tail::pending(dir)?;
    let mut paths = live_paths(dir)?;
    if waiting[0] > 0 {
        let mut covered = false;
        for p in &paths {
            let r = Reader::open(p)?;
            if r.footer().first <= waiting[1] && waiting[1] <= r.footer().last {
                covered = true;
            }
        }
        if !covered {
            paths.push(std::path::Path::new(dir).join(format!("{}@{}-{}", tail::PENDING, waiting[1], waiting[2])).to_string_lossy().to_string());
        }
    }
    let all = since(&paths, -1, -1, &[])?;
    let n = all.nrows();
    if n == 0 || keys.is_empty() {
        return Ok(all);
    }
    let cols: Vec<usize> = keys.iter().map(|k| all.col_index(k).ok_or_else(|| format!("batch: no key column '{}'", k))).collect::<Result<_, _>>()?;
    // the rows are in position order: the last of each key is the newest
    let mut last: std::collections::HashMap<Vec<u8>, usize> = std::collections::HashMap::with_capacity(n);
    for i in 0..n {
        last.insert(tail::key_bytes(&all, &cols, i), i);
    }
    let mut keep = vec![false; n];
    for &i in last.values() {
        keep[i] = true;
    }
    all.pick_rows(&keep)
}

/// Each key's `[lo, hi]` over a file's groups, as a JSON object (a key
/// whose bounds no group keeps is left out).
fn key_ranges(path: &str, keys: &[String]) -> Result<String, String> {
    use serde_json::{json, Map, Value};
    let r = Reader::open(path)?;
    let mut out = Map::new();
    for k in keys {
        let Some(c) = r.footer().column(k) else { continue };
        let entries = r.entries(c)?;
        let bound = |b: &Bound| -> Value {
            match b {
                Bound::Int(v) => json!(v),
                Bound::Float(v) => json!(v),
                Bound::Bool(v) => json!(v),
                Bound::Text(v) => json!(v),
            }
        };
        let mut lo: Option<&Bound> = None;
        let mut hi: Option<&Bound> = None;
        for e in entries.iter() {
            if let Some(m) = &e.min {
                lo = Some(match lo {
                    Some(l) if bound_le(l, m) => l,
                    _ => m,
                });
            }
            if let Some(m) = &e.max {
                hi = Some(match hi {
                    Some(h) if bound_le(m, h) => h,
                    _ => m,
                });
            }
        }
        if let (Some(l), Some(h)) = (lo, hi) {
            out.insert(k.clone(), json!([bound(l), bound(h)]));
        }
    }
    Ok(Value::Object(out).to_string())
}

fn bound_le(a: &Bound, b: &Bound) -> bool {
    match (a, b) {
        (Bound::Int(x), Bound::Int(y)) => x <= y,
        (Bound::Float(x), Bound::Float(y)) => x <= y,
        (Bound::Bool(x), Bound::Bool(y)) => x <= y,
        (Bound::Text(x), Bound::Text(y)) => x <= y,
        _ => true,
    }
}

/// `sys.batch_summary[…]`: aggregates over the files, pushed below the reader (exec.rs).
#[allow(clippy::too_many_arguments)] // the row's arguments
pub fn summary(paths: &[String], by: &[String], of: &[String], how: &[String], on: &[String], ops: &[String], equal: &[String], lo: &str, hi: &str, upto: i64) -> Result<CrowFrame, String> {
    exec::summary(paths, by, of, how, on, ops, equal, lo, hi, upto)
}

/// `sys.batch_asof[…]`: the as-of join over the files (exec.rs).
pub fn asof(paths: &[String], labels: &CrowFrame, keys: &[String], time: &str, wanted: &[String], upto: i64) -> Result<CrowFrame, String> {
    exec::asof(paths, labels, keys, time, wanted, upto)
}

/// `sys.batch_declared[dir]`: what the log declares of itself, as JSON
/// text — its `keys` and its `notes` (the time column, …) — or empty
/// text where the log has no manifest yet.
pub fn declared(dir: &str) -> Result<String, String> {
    Ok(manifest::read(dir)?.map(|m| m.declared()).unwrap_or_default())
}

/// `sys.batch_declare[dir, keys, notes]`: the log's manifest written with
/// its keys and its notes (a JSON object of texts) and no segments — the
/// first write of a log. Refused where a manifest exists with other keys.
pub fn declare(dir: &str, keys: &[String], notes: &str) -> Result<String, String> {
    let parsed: serde_json::Value = if notes.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(notes).map_err(|e| format!("batch: the notes are no JSON object: {}", e))? };
    let Some(object) = parsed.as_object() else {
        return Err("batch: the notes are no JSON object".to_string());
    };
    let mut m = manifest::Manifest { keys: keys.to_vec(), notes: Default::default(), segments: Vec::new() };
    for (k, v) in object {
        let text = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        m.notes.insert(k.clone(), text);
    }
    if let Some(have) = manifest::read(dir)? {
        if have.keys != m.keys {
            return Err(format!("batch: the log at '{}' is keyed by {}, not by {}", dir, key_words(&have.keys), key_words(&m.keys)));
        }
        m.segments = have.segments;
        for (k, v) in have.notes {
            m.notes.entry(k).or_insert(v);
        }
    }
    manifest::write(dir, &m)?;
    Ok(m.declared())
}

/// Keys named for a refusal: `id`, `day and symbol`, `nothing`.
pub fn key_words(keys: &[String]) -> String {
    match keys.len() {
        0 => "nothing".to_string(),
        1 => keys[0].clone(),
        n => format!("{} and {}", keys[..n - 1].join(", "), keys[n - 1]),
    }
}

/// `sys.batch_catalog[dir]`: the manifest's segments as a frame — `name`,
/// `level`, `first`, `last`, `rows`, `bytes`, `ranges` — with no rows
/// where the log has no manifest.
pub fn catalog(dir: &str) -> Result<CrowFrame, String> {
    Ok(manifest::read(dir)?.unwrap_or_default().frame())
}

/// `sys.batch_sync[dir]`: every batch and segment under `dir` flushed to
/// the disk, then the directory — what a batch written with `flush`
/// false had skipped. Answers how many files were flushed.
pub fn sync(dir: &str) -> Result<i64, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("batch: cannot read '{}': {}", dir, e))?;
    let mut n = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_log_file(&name) && name != manifest::MANIFEST {
            continue;
        }
        let f = std::fs::File::open(entry.path()).map_err(|e| format!("batch: cannot open '{}': {}", entry.path().display(), e))?;
        f.sync_all().map_err(|e| format!("batch: cannot flush '{}': {}", entry.path().display(), e))?;
        n += 1;
    }
    let d = std::fs::File::open(dir).map_err(|e| format!("batch: cannot open '{}': {}", dir, e))?;
    d.sync_all().map_err(|e| format!("batch: cannot flush '{}': {}", dir, e))?;
    Ok(n)
}

/// `sys.batch_admits[path, column, value, token]`: whether the file may
/// hold `value` in `column` — false is certain, true means scan it. With
/// `token` true, whether the column's words may include `value`
/// (lowercased, one word). The value is written as text and read as the
/// column's kind: a moment as its milliseconds.
pub fn admits(path: &str, column: &str, value: &str, token: bool) -> Result<bool, String> {
    let r = Reader::open(path)?;
    let f = r.footer();
    let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", path, column))?;
    let entries = r.entries(c)?;
    if token {
        if f.columns[c].kind != Kind::Text {
            return Err(format!("batch: column '{}' holds no text to search", column));
        }
        let word = value.to_lowercase();
        for e in entries.iter() {
            match &e.tokens {
                Some(t) => {
                    if r.token_bloom(t)?.has(word.as_bytes()) {
                        return Ok(true);
                    }
                }
                // a group with no index may hold anything
                None => return Ok(true),
            }
        }
        return Ok(false);
    }
    let want = parsed(f.columns[c].kind, value).map_err(|e| format!("batch: column '{}': {}", column, e))?;
    let bytes = parsed_bytes(&want);
    for e in entries.iter() {
        if !bounds_admit(e, &want) {
            continue;
        }
        // a group of too many distinct values has no filter: it may hold anything
        match r.bloom(e)? {
            None => return Ok(true),
            Some(b) if b.has(&bytes) => return Ok(true),
            Some(_) => {}
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textcol::Text;

    /// `rm` of a log's directory drops the rows pending in its tail: the
    /// flusher had written them into the directory ten milliseconds after
    /// the rm, and a test's scratch log came back as one batch file.
    #[test]
    fn rm_of_a_log_drops_its_pending_rows() {
        let root = scratch("rm-root");
        let dir = format!("{}/_kv", root);
        std::fs::create_dir_all(&dir).unwrap();
        let mut keys = Text::new();
        keys.push("a");
        let f = CrowFrame::new(vec!["key".into(), "n".into()], vec![Column::Str(keys), Column::Int(vec![1])], vec![vec![true]; 2]);
        assert_eq!(tail::put(&dir, &f).unwrap(), 0);
        crate::io::rm(&root).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(tail::FLUSH_MS * 5));
        assert!(!std::path::Path::new(&root).exists(), "the flusher wrote the pending rows into the removed directory");
        // the log begins again from nothing
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(tail::put(&dir, &f).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("crow-magpie-{}-{}", name, std::process::id()));
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

    fn ints(f: &CrowFrame, c: usize) -> Vec<i64> {
        match f.data[c].as_ref() {
            Column::Int(v) => v.clone(),
            _ => panic!("ints"),
        }
    }

    /// A listing names the pending rows by their positions
    /// (`.pending@first-last`); opened after the thread flushed them, the
    /// reader is handed the file they went to, so a read planned before a
    /// flush answers the same rows after it; and rows put after the
    /// listing are not in a read by the listing's positions.
    #[test]
    fn a_pending_path_reads_its_rows_after_a_flush() {
        let dir = scratch("pending-race");
        let two = |ids: Vec<i64>, qtys: Vec<i64>| {
            let n = ids.len();
            CrowFrame::new(vec!["id".into(), "qty".into()], vec![Column::Int(ids), Column::Int(qtys)], vec![vec![true; n], vec![true; n]])
        };
        assert_eq!(tail::put(&dir, &two(vec![1, 2], vec![10, 20])).expect("put"), 0);
        let path = format!("{}/{}@0-1", dir, tail::PENDING);
        let before = read(&path, &[]).expect("read while pending, or from the file the thread wrote");
        assert_eq!(ints(&before, before.col_index("qty").unwrap()), vec![10, 20]);
        tail::flush(&dir).expect("flushed");
        let after = read(&path, &[]).expect("read after the flush");
        assert_eq!(ints(&after, after.col_index("qty").unwrap()), vec![10, 20]);
        assert_eq!(ints(&after, after.col_index(POSITION).unwrap()), vec![0, 1]);
        assert_eq!(tail::put(&dir, &two(vec![3], vec![30])).expect("put"), 2);
        let listed = tail::pending(&dir).expect("pending");
        tail::put(&dir, &two(vec![4], vec![40])).expect("put");
        if listed[0] > 0 {
            let cut = read(&format!("{}/{}@{}-{}", dir, tail::PENDING, listed[1], listed[2]), &[]).expect("read by the listing");
            let positions = ints(&cut, cut.col_index(POSITION).unwrap());
            assert!(positions.contains(&2), "the listing's rows: {positions:?}");
            if positions.len() > 1 {
                // the thread flushed both puts into one file between the listing and the read
                assert_eq!(positions, vec![2, 3]);
            }
        }
        assert!(read(&format!("{}/{}@7-9", dir, tail::PENDING), &[]).is_err(), "positions never written");
        tail::flush(&dir).expect("flushed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A frame round-trips: positions in arrival order, rows sorted by the
    /// keys (ties in arrival order), every cell and every missing cell as
    /// it was.
    #[test]
    fn a_frame_round_trips_sorted_by_its_keys() {
        let dir = scratch("round");
        let path = write(&dir, 7, 100, &frame(), &["id".to_string()], &["name".to_string()], true).expect("written");
        assert!(path.ends_with("000000000007.batch"));
        let back = read(&path, &[]).expect("read back");
        assert_eq!(back.columns, vec!["id", "price", "name", "ok", POSITION]);
        assert_eq!(ints(&back, 0), vec![1, 1, 2, 3]);
        assert_eq!(ints(&back, 4), vec![101, 103, 102, 100]);
        let names = match back.data[2].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!("text") };
        assert_eq!(names, vec!["banana split", "apple crumble", "Cherry tart", "Apple pie"]);
        let present: Vec<bool> = back.validity[1].iter().copied().collect();
        assert_eq!(present, vec![true, false, true, true]);
        let prices = match back.data[1].as_ref() { Column::Float(v) => v.clone(), _ => panic!("floats") };
        assert!(prices[2].is_nan());
        let stats: serde_json::Value = serde_json::from_str(&stats(&path).unwrap()).unwrap();
        assert_eq!(stats["format"], 2);
        assert_eq!(stats["rows"], 4);
        assert_eq!(stats["first"], 100);
        assert_eq!(stats["last"], 103);
        assert_eq!(stats["keys"][0], "id");
        assert_eq!(stats["columns"]["ok"]["cells"], "bool");
        assert_eq!(stats["columns"]["name"]["indexed"], true);
        assert_eq!(stats["tokens"]["name"]["docs"], 4);
        assert_eq!(stats["tokens"]["name"]["words"], 8);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A value present is admitted; one absent is (almost always) not; a
    /// word of a text column is found lowercased; a word absent is not.
    #[test]
    fn a_filter_admits_what_is_there() {
        let dir = scratch("admits");
        let path = write(&dir, 1, 0, &frame(), &[], &["name".to_string()], true).expect("written");
        assert!(admits(&path, "id", "3", false).unwrap());
        assert!(admits(&path, "id", " 1 ", false).unwrap());
        assert!(admits(&path, "price", "2.5", false).unwrap());
        assert!(admits(&path, "ok", "false", false).unwrap());
        assert!(admits(&path, "name", "Cherry tart", false).unwrap());
        assert!(admits(&path, "name", "APPLE", true).unwrap());
        assert!(admits(&path, "name", "crumble", true).unwrap());
        assert!(!admits(&path, "id", "4", false).unwrap(), "outside the bounds");
        let absent = (1000..1200).filter(|i| admits(&path, "id", &i.to_string(), false).unwrap()).count();
        assert_eq!(absent, 0);
        assert!(!admits(&path, "name", "durian", true).unwrap());
        assert_eq!(admits(&path, "id", "x", false).unwrap_err(), "batch: column 'id': 'x' is no int");
        assert!(admits(&path, "nosuch", "1", false).unwrap_err().contains("holds no column 'nosuch'"));
        assert!(admits(&path, "id", "1", true).unwrap_err().contains("holds no text to search"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file cut short at any byte is refused with its reason, never a
    /// panic; a number written twice is refused; what the writer refuses,
    /// it refuses before any file exists.
    #[test]
    fn a_torn_file_is_refused_and_a_number_is_written_once() {
        let dir = scratch("torn");
        let path = write(&dir, 2, 0, &frame(), &["id".to_string()], &[], true).expect("written");
        let whole = std::fs::read(&path).unwrap();
        for cut in (0..whole.len()).step_by(7) {
            let torn = std::path::Path::new(&dir).join(format!("torn{}.batch", cut));
            std::fs::write(&torn, &whole[..cut]).unwrap();
            let r = read(&torn.to_string_lossy(), &[]);
            assert!(r.is_err(), "a file cut at {} of {} bytes read", cut, whole.len());
            assert!(stats(&torn.to_string_lossy()).is_err());
        }
        assert!(write(&dir, 2, 0, &frame(), &[], &[], true).unwrap_err().contains("was written already"));
        assert!(write(&dir, 3, 0, &frame(), &["nosuch".to_string()], &[], true).unwrap_err().contains("no column 'nosuch' to sort by"));
        assert!(write(&dir, 3, 0, &frame(), &[], &["id".to_string()], true).unwrap_err().contains("holds no text"));
        assert!(write(&dir, 3, 0, &frame(), &[], &["nosuch".to_string()], true).unwrap_err().contains("no column 'nosuch' to index"));
        assert!(!std::path::Path::new(&dir).join(file_name(3)).exists());
        let empty = CrowFrame::new(vec!["a".into()], vec![Column::Int(vec![])], vec![vec![]]);
        assert!(write(&dir, 3, 0, &empty, &[], &[], true).unwrap_err().contains("at least one row"));
        assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two batches and a segment fold into one segment: the latest
    /// version of each key kept, sorted by key, positions as they were,
    /// the statistics and the word filters as a batch's; the segment reads
    /// through the same functions, and merges again
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
        let pa = write(&dir, 0, 0, &a, &["id".to_string()], &["name".to_string()], true).unwrap();
        let pb = write(&dir, 1, 3, &b, &["id".to_string()], &["name".to_string()], true).unwrap();
        declare(&dir, &["id".to_string()], "").unwrap();
        let seg = merge(&dir, &[pa.clone(), pb.clone()], false).unwrap();
        assert!(seg.ends_with("S000000000001.segment"), "{}", seg);
        let f = read(&seg, &[]).unwrap();
        assert_eq!(ints(&f, 0), vec![1, 2, 3, 4]);
        let names = match f.data[1].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!() };
        assert_eq!(names, vec!["one", "TWO again", "three", "four"]);
        assert_eq!(ints(&f, 2), vec![0, 3, 2, 4]);
        let st: serde_json::Value = serde_json::from_str(&stats(&seg).unwrap()).unwrap();
        assert_eq!(st["rows"], 4);
        assert_eq!(st["first"], 0);
        assert_eq!(st["last"], 4);
        assert!(admits(&seg, "name", "again", true).unwrap());
        assert!(!admits(&seg, "name", "durian", true).unwrap());
        assert!(admits(&seg, "id", "4", false).unwrap());
        let mut t3 = Text::new();
        t3.push("four again");
        let c = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![4]), Column::Str(t3)], vec![vec![true], vec![true]]);
        let pc = write(&dir, 2, 5, &c, &["id".to_string()], &[], true).unwrap();
        declare(&dir, &[], "").unwrap_err();
        let seg2 = merge(&dir, &[seg.clone(), pc.clone()], false).unwrap();
        assert!(seg2.ends_with("S000000000002.segment"));
        assert_eq!(read(&seg2, &[]).unwrap().nrows(), 4, "the log is keyed by id: the latest id 4 kept");
        assert!(merge(&dir, &[seg.clone(), pc.clone()], false).unwrap_err().contains("does not name"), "a merged segment is no input again");
        assert!(merge(&dir, &[], false).unwrap_err().contains("one file or more"));
        let mut t4 = Text::new();
        t4.push("four once more");
        let d = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![4]), Column::Str(t4)], vec![vec![true], vec![true]]);
        let pd = write(&dir, 3, 6, &d, &["id".to_string()], &[], true).unwrap();
        let kept = merge(&dir, &[seg2.clone(), pd], true).unwrap();
        assert!(kept.ends_with("S000000000003.segment"));
        let k = read(&kept, &[]).unwrap();
        assert_eq!(k.nrows(), 5);
        assert_eq!(ints(&k, 0), vec![1, 2, 3, 4, 4]);
        let listed = manifest::read(&dir).unwrap().unwrap();
        assert_eq!(listed.segments.len(), 1);
        assert!(listed.segments[0].name.ends_with("S000000000003.segment"));
        assert_eq!((listed.segments[0].number, listed.segments[0].high), (0, 3));
        assert_eq!(listed.segments[0].level, 3);
        assert_eq!((listed.segments[0].first, listed.segments[0].last, listed.segments[0].rows), (0, 6, 5));
        assert!(listed.segments[0].ranges.contains("\"id\":[1,4]"), "{}", listed.segments[0].ranges);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lookup answers the rows holding the value through a batch and
    /// through a segment of many row groups, reading only the groups whose
    /// statistics admit it; a value no filter admits is an empty frame with
    /// the file's columns; a value of another kind is refused
    #[test]
    fn a_find_reads_the_row_groups_that_admit_the_value() {
        let dir = scratch("find");
        let n = 200_000i64;
        let ids: Vec<i64> = (0..n).rev().collect();
        let mut names = Text::new();
        for i in &ids {
            names.push(&format!("row {}", i));
        }
        let f = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(ids), Column::Str(names)], vec![vec![true; n as usize], vec![true; n as usize]]);
        let pa = write(&dir, 0, 0, &f, &["id".to_string()], &[], true).unwrap();
        let hit = find(&pa, &["id".to_string()], &["12345".to_string()], &[]).unwrap();
        assert_eq!(hit.nrows(), 1);
        assert_eq!(ints(&hit, 2)[0], n - 1 - 12345);
        let none = find(&pa, &["id".to_string()], &["-5".to_string()], &[]).unwrap();
        assert_eq!(none.nrows(), 0);
        assert_eq!(none.columns, vec!["id", "name", POSITION]);
        declare(&dir, &["id".to_string()], "").unwrap();
        let seg = merge(&dir, &[pa.clone()], false).unwrap();
        assert!(Reader::open(&seg).unwrap().footer().groups.len() > 1, "a segment of many groups");
        let hit = find(&seg, &["id".to_string()], &["12345".to_string()], &[]).unwrap();
        assert_eq!(hit.nrows(), 1);
        assert_eq!(match hit.data[1].as_ref() { Column::Str(t) => t.get(0).to_string(), _ => panic!() }, "row 12345");
        assert_eq!(find(&seg, &["name".to_string()], &["row 7".to_string()], &[]).unwrap().nrows(), 1);
        assert_eq!(find(&seg, &["id".to_string()], &["99999999".to_string()], &[]).unwrap().nrows(), 0);
        assert_eq!(find(&seg, &["id".to_string()], &["x".to_string()], &[]).unwrap_err(), "batch: column 'id': 'x' is no int");
        assert!(find(&seg, &["nosuch".to_string()], &["1".to_string()], &[]).unwrap_err().contains("holds no column 'nosuch'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_count_and_tokens_split() {
        assert_eq!(file_name(17), "000000000017.batch");
        assert_eq!(number_of("000000000017.batch"), Some(17));
        assert_eq!(number_of("17.batch"), None);
        assert_eq!(number_of(".000000000017.batch.part"), None);
        assert_eq!(segment_name_of(17), "S000000000017.segment");
        assert_eq!(segment_counter("S000000000017.segment"), Some(17));
        assert_eq!(segment_counter("000000000003-000000000017.segment"), None);
        assert!(is_log_file("000000000017.batch") && is_log_file("S000000000001.segment") && !is_log_file("manifest"));
        assert_eq!(tokens("Hello, World! it's 2026-10-25"), vec!["hello", "world", "it", "s", "2026", "10", "25"]);
    }

    fn keyed() -> CrowFrame {
        let ids: Vec<i64> = (0..20000).map(|i| i % 1000).collect();
        let days: Vec<i64> = (0..20000).map(|i| i / 1000).collect();
        let mut names = Text::new();
        for i in 0..20000 {
            names.push(&format!("row {}", i));
        }
        CrowFrame::new(vec!["id".into(), "day".into(), "name".into()], vec![Column::Int(ids), Column::Int(days), Column::Str(names)], vec![vec![true; 20000]; 3])
    }

    /// Two keys prune to the rows holding both; the columns wanted are the
    /// ones read, the keys dropped again where not wanted.
    #[test]
    fn a_find_by_two_keys_reads_the_rows_holding_both() {
        let dir = scratch("keys");
        let pa = write(&dir, 0, 0, &keyed(), &["day".to_string(), "id".to_string()], &[], true).unwrap();
        declare(&dir, &["day".to_string(), "id".to_string()], "").unwrap();
        let seg = merge(&dir, &[pa.clone()], true).unwrap();
        for path in [&pa, &seg] {
            let hit = find(path, &["day".to_string(), "id".to_string()], &["7".to_string(), "42".to_string()], &[]).unwrap();
            assert_eq!(hit.nrows(), 1);
            assert_eq!(hit.columns, vec!["id", "day", "name", POSITION]);
            let named = find(path, &["day".to_string(), "id".to_string()], &["7".to_string(), "42".to_string()], &["name".to_string()]).unwrap();
            assert_eq!(named.columns, vec!["name"]);
            assert_eq!(named.nrows(), 1);
            let none = find(path, &["day".to_string(), "id".to_string()], &["7".to_string(), "4200".to_string()], &[]).unwrap();
            assert_eq!(none.nrows(), 0);
            let two = read(path, &["day".to_string(), "name".to_string()]).unwrap();
            assert_eq!(two.columns, vec!["day", "name"]);
            let back = find(path, &["day".to_string()], &["3".to_string()], &["name".to_string(), "id".to_string()]).unwrap();
            assert_eq!(back.columns, vec!["name", "id"]);
            assert_eq!(back.nrows(), 1000);
            assert_eq!(ints(&back, 1)[0], 0);
            assert_eq!(two.nrows(), 20000);
            assert!(read(path, &["nosuch".to_string()]).unwrap_err().contains("holds no column 'nosuch'"));
        }
        assert!(find(&seg, &["day".to_string()], &["7".to_string(), "1".to_string()], &[]).unwrap_err().contains("one value per column"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Several files read past a position come in position order, and a
    /// find over several files too, each with the columns asked
    #[test]
    fn several_files_are_read_in_position_order() {
        let dir = scratch("since");
        let pa = write(&dir, 0, 0, &keyed(), &["id".to_string()], &[], true).unwrap();
        let pb = write(&dir, 1, 20000, &keyed(), &["id".to_string()], &[], true).unwrap();
        let paths = vec![pb.clone(), pa.clone()];
        let all = since(&paths, -1, -1, &[]).unwrap();
        assert_eq!(all.nrows(), 40000);
        let pos = ints(&all, all.col_index(POSITION).unwrap());
        assert!(pos.windows(2).all(|w| w[0] < w[1]));
        let late = since(&paths, 39990, -1, &["name".to_string()]).unwrap();
        assert_eq!(late.nrows(), 9);
        assert_eq!(late.columns, vec!["name"]);
        let none = since(&paths, 40000, -1, &[]).unwrap();
        assert_eq!(none.nrows(), 0);
        let window = since(&paths, 19995, 20004, &[]).unwrap();
        assert_eq!(window.nrows(), 9, "positions 19996 to 20004, across the two files");
        let pos = ints(&window, window.col_index(POSITION).unwrap());
        assert_eq!((pos[0], pos[8]), (19996, 20004));
        let ranged = between(&paths, "id", "10", "12", -1, &["id".to_string()]).unwrap();
        assert_eq!(ranged.nrows(), 120, "ids 10, 11, 12: twenty rows each in two files");
        assert!(ints(&ranged, 0).iter().all(|&i| (10..=12).contains(&i)));
        let upto = between(&paths, "id", "10", "", 19999, &[]).unwrap();
        assert_eq!(upto.nrows(), 19800, "ids 10 and up of the first file");
        assert!(between(&paths, "id", "x", "", -1, &[]).unwrap_err().contains("no int"));
        assert!(between(&paths, "nosuch", "", "", -1, &[]).unwrap_err().contains("holds no column"));
        let both = find_in(&paths, &["id".to_string()], &["7".to_string()], &["day".to_string()]).unwrap();
        assert_eq!(both.nrows(), 40);
        assert_eq!(both.columns, vec!["day"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A batch written without a flush is read as any batch, and a sync
    /// flushes every file of the log.
    #[test]
    fn a_batch_written_without_a_flush_is_synced_later() {
        let dir = scratch("flush");
        let pa = write(&dir, 0, 0, &keyed(), &["id".to_string()], &[], false).unwrap();
        assert_eq!(read(&pa, &[]).unwrap().nrows(), 20000);
        assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")));
        assert_eq!(sync(&dir).unwrap(), 1);
        assert!(sync("/nonexistent/crow-log").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A summary over the files answers what the frame's group answers
    /// over every row, for every aggregate and every predicate; an as-of
    /// join answers the newest row at or before each label's moment.
    #[test]
    fn a_summary_answers_as_group_over_the_rows_does() {
        use crate::frame::AggFn;
        let dir = scratch("summary");
        let n = 3000usize;
        let syms: Vec<&str> = (0..n).map(|i| ["AAPL", "MSFT", "GOOG"][i % 3]).collect();
        let mut names = Text::new();
        for s in &syms {
            names.push(s);
        }
        let days: Vec<i64> = (0..n).map(|i| (i / 1000) as i64).collect();
        let px: Vec<f64> = (0..n).map(|i| 100.0 + (i as f64 * 0.37).sin() * 10.0).collect();
        let vol: Vec<i64> = (0..n).map(|i| (i * 7 % 500) as i64).collect();
        let valid_px: Vec<bool> = (0..n).map(|i| i % 97 != 0).collect();
        let f = CrowFrame::new(vec!["day".into(), "sym".into(), "px".into(), "vol".into()], vec![Column::Int(days), Column::Str(names), Column::Float(px), Column::Int(vol)], vec![vec![true; n], vec![true; n], valid_px, vec![true; n]]);
        let mut paths = Vec::new();
        paths.push(write(&dir, 0, 0, &f, &["day".to_string(), "sym".to_string()], &[], true).unwrap());
        paths.push(write(&dir, 1, n as i64, &f, &["day".to_string(), "sym".to_string()], &[], true).unwrap());
        declare(&dir, &["day".to_string(), "sym".to_string()], "").unwrap();
        let seg = merge(&dir, &paths, true).unwrap();
        let files = vec![seg.clone()];
        let all = read(&seg, &[]).unwrap();
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0);
        let floats = |f: &CrowFrame, c: &str| match f.data[f.col_index(c).unwrap()].as_ref() { Column::Float(v) => v.clone(), _ => panic!("floats") };
        let hows = ["count", "sum", "min", "max", "mean", "var", "std", "first", "last"];
        let by = ["day".to_string(), "sym".to_string()];
        let of: Vec<String> = hows.iter().map(|_| "px".to_string()).chain(hows.iter().map(|_| "vol".to_string())).collect();
        let how: Vec<String> = hows.iter().chain(hows.iter()).map(|h| h.to_string()).collect();
        let got = summary(&files, &by, &of, &how, &[], &[], &[], "", "", -1).unwrap();
        let aggs: Vec<(&str, AggFn)> = of.iter().zip(&how).map(|(c, h)| (c.as_str(), AggFn::from_str(h).unwrap())).collect();
        let want = all.group_by(&["day", "sym"], &aggs).unwrap().sort_keys(&["day", "sym"], &[false, false]).unwrap();
        assert_eq!(got.nrows(), want.nrows());
        assert_eq!(got.columns, want.columns);
        for c in &got.columns {
            let (gi, wi) = (got.col_index(c).unwrap(), want.col_index(c).unwrap());
            assert_eq!(got.validity[gi].to_vec(), want.validity[wi].to_vec(), "{}", c);
            match (got.data[gi].as_ref(), want.data[wi].as_ref()) {
                (Column::Float(a), Column::Float(b)) => assert!(a.iter().zip(b).all(|(x, y)| (x.is_nan() && y.is_nan()) || close(*x, *y)), "{}: {:?} vs {:?}", c, &a[..3], &b[..3]),
                (Column::Int(a), Column::Int(b)) => assert_eq!(a, b, "{}", c),
                (Column::Str(a), Column::Str(b)) => assert_eq!(a.to_vec(), b.to_vec(), "{}", c),
                _ => panic!("{}: kinds differ", c),
            }
        }
        // predicates: one day, two symbols; and a position bound
        let picked = summary(&files, &["sym".to_string()], &["vol".to_string()], &["sum".to_string()], &["day".to_string(), "sym".to_string(), "sym".to_string()], &["eq".to_string(), "eq".to_string(), "eq".to_string()], &["1".to_string(), "AAPL".to_string(), "GOOG".to_string()], "", "", -1).unwrap();
        let syms_all = match all.data[1].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!("text") };
        let mask: Vec<bool> = (0..all.nrows()).map(|i| ints(&all, 0)[i] == 1 && matches!(syms_all[i].as_str(), "AAPL" | "GOOG")).collect();
        let want = all.pick_rows(&mask).unwrap().group_by(&["sym"], &[("vol", AggFn::Sum)]).unwrap().sort_keys(&["sym"], &[false]).unwrap();
        assert_eq!(ints(&picked, 1), ints(&want, 1));
        let bounded = summary(&files, &["day".to_string()], &["px".to_string()], &["count".to_string()], &["px".to_string()], &["gt".to_string()], &["105".to_string()], "1", "2", 4000).unwrap();
        let mask: Vec<bool> = (0..all.nrows()).map(|i| (1..=2).contains(&ints(&all, 0)[i]) && all.validity[2][i] && floats(&all, "px")[i] > 105.0 && ints(&all, all.col_index(POSITION).unwrap())[i] <= 4000).collect();
        let want = all.pick_rows(&mask).unwrap().group_by(&["day"], &[("px", AggFn::Count)]).unwrap().sort_keys(&["day"], &[false]).unwrap();
        assert_eq!(ints(&bounded, 1), ints(&want, 1));
        assert!(summary(&files, &["day".to_string()], &["sym".to_string()], &["sum".to_string()], &[], &[], &[], "", "", -1).unwrap_err().contains("holds text"));
        assert!(summary(&files, &[], &["px".to_string()], &["median".to_string()], &[], &[], &[], "", "", -1).unwrap_err().contains("no aggregate"));
        assert!(summary(&files, &[], &["px".to_string()], &["sum".to_string()], &["day".to_string()], &["between".to_string()], &["1".to_string()], "", "", -1).unwrap_err().contains("no operator"));
        // as-of: for each label's symbol and moment, the newest row at or before it
        let mut ls = Text::new();
        for s in ["AAPL", "GOOG", "MSFT", "ZZZ"] {
            ls.push(s);
        }
        let labels = CrowFrame::new(vec!["sym".into(), "t".into(), "tag".into()], vec![Column::Str(ls), Column::Int(vec![1500, 2999, 10, 100]), Column::Int(vec![1, 2, 3, 4])], vec![vec![true; 4]; 3]);
        let tlog = scratch("asof");
        let times: Vec<i64> = (0..n as i64).collect();
        let mut names2 = Text::new();
        for s in &syms {
            names2.push(s);
        }
        let f2 = CrowFrame::new(vec!["sym".into(), "t".into(), "px".into()], vec![Column::Str(names2), Column::Int(times), Column::Float(px_of(n))], vec![vec![true; n]; 3]);
        let p2 = write(&tlog, 0, 0, &f2, &["sym".to_string()], &[], true).unwrap();
        let joined = asof(&[p2.clone()], &labels, &["sym".to_string()], "t", &[], -1).unwrap();
        assert_eq!(joined.columns, vec!["sym", "t", "tag", "px"]);
        assert_eq!(joined.validity[3].to_vec(), vec![true, true, true, false], "MSFT has rows at t 1, 4, 7 before 10; ZZZ has none");
        let joined_px = match joined.data[3].as_ref() { Column::Float(v) => v.clone(), _ => panic!("floats") };
        assert_eq!(joined_px[0], px_of(n)[1500], "AAPL at t 1500 is row 1500");
        assert_eq!(joined_px[2], px_of(n)[10], "MSFT's newest row at or before 10 is the one at 10");
        // as of a position: a row past it is no candidate
        let cut = asof(&[p2], &labels, &["sym".to_string()], "t", &[], 9).unwrap();
        assert_eq!(cut.validity[3].to_vec(), vec![true, true, true, false], "GOOG's row 8 is within position 9; ZZZ still has none");
        let cut_px = match cut.data[3].as_ref() { Column::Float(v) => v.clone(), _ => panic!("floats") };
        assert_eq!(cut_px[0], px_of(n)[9], "as of position 9, AAPL's newest row at or before t 1500 is row 9");
        assert_eq!(cut_px[2], px_of(n)[7], "as of position 9, MSFT's newest row at or before t 10 is row 7");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&tlog);
    }

    fn px_of(n: usize) -> Vec<f64> {
        (0..n).map(|i| 100.0 + (i as f64 * 0.37).sin() * 10.0).collect()
    }

    /// A phrase, a prefix, a fuzzy word, the stemmer and a snippet answer
    /// alike from a batch (scanned) and from its merged segment (indexed).
    #[test]
    fn the_query_forms_answer_alike_indexed_and_scanned() {
        let dir = scratch("forms");
        let mut t = Text::new();
        for s in ["the kitchen table is set", "a table in the garden", "kitchens of the world", "running the kitchen", "tables and chairs"] {
            t.push(s);
        }
        let f = CrowFrame::new(vec!["id".into(), "text".into()], vec![Column::Int(vec![1, 2, 3, 4, 5]), Column::Str(t)], vec![vec![true; 5]; 2]);
        let batch = write(&dir, 0, 0, &f, &[], &[], true).unwrap();
        declare(&dir, &[], "").unwrap();
        let seg = merge(&dir, &[batch.clone()], false).unwrap();
        assert!(Reader::open(&seg).unwrap().entries(1).unwrap()[0].tokens.as_ref().is_some_and(|t| t.index_len > 0), "the merge wrote the postings");
        let floats = |f: &CrowFrame| match f.data[f.col_index("score").unwrap()].as_ref() { Column::Float(v) => v.clone(), _ => panic!() };
        for words in [vec!["kitchen table".to_string()], vec!["kit*".to_string()], vec!["kitchn~".to_string()], vec!["table".to_string(), "garden".to_string()], vec!["tabel~1".to_string(), "kitchen".to_string()]] {
            let scanned = search(&[batch.clone()], "text", &words, 10, false, 0).unwrap();
            let indexed = search(&[seg.clone()], "text", &words, 10, false, 0).unwrap();
            assert_eq!(ints(&scanned, 0), ints(&indexed, 0), "{:?}", words);
            assert_eq!(floats(&scanned), floats(&indexed), "{:?}", words);
        }
        let phrase = search(&[seg.clone()], "text", &["kitchen table".to_string()], 10, false, 0).unwrap();
        assert_eq!(ints(&phrase, 0), vec![1]);
        let prefix = search(&[seg.clone()], "text", &["kit*".to_string()], 10, false, 0).unwrap();
        assert_eq!(ints(&prefix, 0), vec![1, 3, 4]);
        let fuzzy = search(&[seg.clone()], "text", &["kitchn~".to_string()], 10, false, 0).unwrap();
        assert_eq!(ints(&fuzzy, 0), vec![1, 3, 4]);
        let two = search(&[seg.clone()], "text", &["table".to_string()], 2, false, 20).unwrap();
        assert_eq!(two.nrows(), 2);
        let snippets = match two.data[two.col_index("snippet").unwrap()].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!() };
        assert!(snippets.iter().all(|s| s.contains("table")), "{:?}", snippets);
        // stemmed: `tables` and `table`, `kitchens` and `kitchen` are one term
        let stemmed = scratch("forms-stemmed");
        declare(&stemmed, &[], r#"{"stem":"true"}"#).unwrap();
        let b2 = write(&stemmed, 0, 0, &f, &[], &[], true).unwrap();
        let s2 = merge(&stemmed, &[b2.clone()], false).unwrap();
        let plain = search(&[seg.clone()], "text", &["table".to_string()], 10, false, 0).unwrap();
        let both = search(&[s2.clone()], "text", &["tables".to_string()], 10, true, 0).unwrap();
        assert_eq!(ints(&plain, 0), vec![1, 2]);
        assert_eq!(ints(&both, 0).len(), 3, "table, table, tables");
        let scanned = search(&[b2], "text", &["tables".to_string()], 10, true, 0).unwrap();
        assert_eq!(floats(&scanned), floats(&both));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&stemmed);
    }

    /// The streaming merge cuts segments at the size asked, keeps one
    /// version per key in key order, and every row when asked.
    #[test]
    fn a_stream_cuts_segments_by_size_and_keeps_one_version_per_key() {
        let dir = scratch("stream");
        let mut paths = Vec::new();
        for b in 0..3i64 {
            let ids: Vec<i64> = (0..4000).map(|i| (i * 7 + b * 3) % 5000).collect();
            let mut names = Text::new();
            for i in &ids {
                names.push(&format!("row {} of batch {}", i, b));
            }
            let f = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(ids), Column::Str(names)], vec![vec![true; 4000]; 2]);
            paths.push(write(&dir, b, b * 4000, &f, &["id".to_string()], &["name".to_string()], true).unwrap());
        }
        let mut counter = 0;
        let written = merge::stream(&dir, &paths, &["id".to_string()], &["name".to_string()], &[], false, false, &mut counter, 1 << 14, 1).unwrap();
        assert!(written.len() > 2, "{} segments", written.len());
        assert_eq!(counter, written.len() as i64);
        let mut all = Vec::new();
        let mut last_pos = -1;
        for w in &written {
            let f = read(&format!("{}/{}", dir, w.name), &[]).unwrap();
            assert_eq!(f.nrows() as i64, w.rows);
            all.extend(ints(&f, 0));
            assert!(w.first >= 0 && w.last <= 11999 && w.first <= w.last);
            last_pos = last_pos.max(w.last);
        }
        assert_eq!(last_pos, 11999);
        assert!(all.windows(2).all(|p| p[0] < p[1]), "keys in order, one version each");
        let distinct: std::collections::HashSet<i64> = (0..3i64).flat_map(|b| (0..4000).map(move |i| (i * 7 + b * 3) % 5000)).collect();
        assert_eq!(all.len(), distinct.len());
        let mut counter = 1000;
        let every = merge::stream(&dir, &paths, &["id".to_string()], &[], &[], false, true, &mut counter, 1 << 20, 1 << 30).unwrap();
        assert_eq!(every.len(), 1);
        assert_eq!(every[0].rows, 12000);
        assert_eq!(every[0].name, "S000000001001.segment");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A search ranks by BM25 over the files as one corpus, reading only
    /// the groups whose filters admit a word.
    #[test]
    fn a_search_ranks_the_files_as_one_corpus() {
        let dir = scratch("search");
        let mut t = Text::new();
        for s in ["the quick brown fox", "a lazy dog", "the fox and the dog"] {
            t.push(s);
        }
        let a = CrowFrame::new(vec!["id".into(), "text".into()], vec![Column::Int(vec![1, 2, 3]), Column::Str(t)], vec![vec![true; 3], vec![true; 3]]);
        let mut t2 = Text::new();
        t2.push("brown bread");
        let b = CrowFrame::new(vec!["id".into(), "text".into()], vec![Column::Int(vec![4]), Column::Str(t2)], vec![vec![true], vec![true]]);
        let pa = write(&dir, 0, 0, &a, &[], &["text".to_string()], true).unwrap();
        let pb = write(&dir, 1, 3, &b, &[], &["text".to_string()], true).unwrap();
        let paths = vec![pa.clone(), pb.clone()];
        let hits = search(&paths, "text", &["fox".to_string()], 10, false, 0).unwrap();
        assert_eq!(ints(&hits, 0), vec![1, 3]);
        let scores = match hits.data[hits.col_index("score").unwrap()].as_ref() { Column::Float(v) => v.clone(), _ => panic!() };
        assert!(scores[0] > scores[1]);
        let two = search(&paths, "text", &["brown".to_string(), "dog".to_string()], 2, false, 0).unwrap();
        assert_eq!(two.nrows(), 2);
        assert_eq!(ints(&two, 0), vec![4, 2], "the two best of four, by score");
        let three = search(&paths, "text", &["brown".to_string(), "dog".to_string()], 3, false, 0).unwrap();
        assert_eq!(ints(&three, 0), vec![4, 2, 1]);
        let none = search(&paths, "text", &["zebra".to_string()], 10, false, 0).unwrap();
        assert_eq!(none.nrows(), 0);
        assert!(none.col_index("score").is_some());
        assert!(search(&paths, "id", &["fox".to_string()], 10, false, 0).unwrap_err().contains("holds no text"));
        assert!(search(&paths, "text", &["fox".to_string()], -1, false, 0).is_err());
        // the same log written with no index at all answers the same ranking, and
        // so does its merged segment, which the merge indexes
        let plain = scratch("search-plain");
        let pa2 = write(&plain, 0, 0, &a, &[], &[], true).unwrap();
        let pb2 = write(&plain, 1, 3, &b, &[], &[], true).unwrap();
        let unindexed = search(&[pa2.clone(), pb2.clone()], "text", &["brown".to_string(), "dog".to_string()], 10, false, 0).unwrap();
        let indexed = search(&paths, "text", &["brown".to_string(), "dog".to_string()], 10, false, 0).unwrap();
        assert_eq!(ints(&unindexed, 0), ints(&indexed, 0));
        let floats = |f: &CrowFrame| match f.data[f.col_index("score").unwrap()].as_ref() { Column::Float(v) => v.clone(), _ => panic!() };
        assert_eq!(floats(&unindexed), floats(&indexed));
        assert!(admits(&pa2, "text", "fox", true).unwrap(), "a group with no index may hold anything");
        let seg = merge(&dir, &paths, false).unwrap();
        let again = search(&[seg.clone()], "text", &["fox".to_string()], 10, false, 0).unwrap();
        assert_eq!(ints(&again, 0), vec![1, 3]);
        assert!(Reader::open(&seg).unwrap().footer().columns.iter().any(|c| c.name == "text" && c.indexed), "the merge indexed the text column");
        let seg2 = merge(&plain, &[pa2, pb2], false).unwrap();
        let merged = search(&[seg2], "text", &["brown".to_string(), "dog".to_string()], 10, false, 0).unwrap();
        assert_eq!(floats(&merged), floats(&indexed));
        let _ = std::fs::remove_dir_all(&plain);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
