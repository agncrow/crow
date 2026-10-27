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
pub mod file;
pub mod inflate;
pub mod pack;
pub mod snappy;

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
fn file_of(frame: &CrowFrame, first: i64, last: i64, keys: &[String], texts: &[String]) -> Result<Vec<u8>, String> {
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
    let mut w = Writer::new(columns);
    let n = frame.nrows();
    let per = rows_per_group(frame);
    let mut start = 0;
    while start < n {
        let end = (start + per).min(n);
        w.group(frame, start, end)?;
        start = end;
    }
    Ok(w.finish(first, last, keys))
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
    let bytes = file_of(&sorted, first, first + n as i64 - 1, keys, texts).map_err(|e| format!("batch: {}", e))?;
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
enum Parsed {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

fn parsed(kind: Kind, value: &str) -> Result<Parsed, String> {
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
fn parsed_bytes(p: &Parsed) -> Vec<u8> {
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

/// What one file contributes to a search: how many documents its indexed
/// column holds and how many words they have between them (read from the
/// file's index), and — where a group's filter admits a word asked — the
/// rows holding one, each with its length in words and its count of each
/// word asked.
struct Searched {
    docs: usize,
    words: usize,
    rows: Option<CrowFrame>,
    lengths: Vec<f64>,
    counts: Vec<Vec<f64>>,
}

fn searched(path: &str, column: &str, asked: &[String]) -> Result<Searched, String> {
    let r = Reader::open(path)?;
    let f = r.footer();
    let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", path, column))?;
    if !f.columns[c].indexed {
        return Err(format!("batch: '{}' indexes no words of column '{}'", path, column));
    }
    let entries = r.entries(c)?;
    let docs: usize = entries.iter().filter_map(|e| e.tokens.as_ref()).map(|t| t.docs as usize).sum();
    let words: usize = entries.iter().filter_map(|e| e.tokens.as_ref()).map(|t| t.words as usize).sum();
    let none = Searched { docs, words, rows: None, lengths: Vec::new(), counts: Vec::new() };
    let mut groups = Vec::new();
    for (g, e) in entries.iter().enumerate() {
        let Some(t) = &e.tokens else { continue };
        if t.docs == 0 {
            continue;
        }
        let bloom = r.token_bloom(t)?;
        if asked.iter().any(|w| bloom.has(w.as_bytes())) {
            groups.push(g);
        }
    }
    if docs == 0 || groups.is_empty() {
        return Ok(none);
    }
    let all: Vec<usize> = (0..f.columns.len()).collect();
    let frame = r.read(&groups, &all)?;
    let Column::Str(t) = frame.data[c].as_ref() else {
        return Err(format!("batch: column '{}' holds no text to search", column));
    };
    let valid = frame.validity[c].as_ref();
    let position: std::collections::HashMap<&str, usize> = asked.iter().enumerate().map(|(i, w)| (w.as_str(), i)).collect();
    let n = frame.nrows();
    let mut mask = vec![false; n];
    let (mut lengths, mut counts) = (Vec::new(), Vec::new());
    for row in 0..n {
        if !valid[row] {
            continue;
        }
        let toks = tokens(t.get(row));
        let mut count = vec![0.0; asked.len()];
        let mut hit = false;
        for tok in &toks {
            if let Some(&i) = position.get(tok.as_str()) {
                count[i] += 1.0;
                hit = true;
            }
        }
        if hit {
            mask[row] = true;
            lengths.push(toks.len() as f64);
            counts.push(count);
        }
    }
    if lengths.is_empty() {
        return Ok(none);
    }
    Ok(Searched { docs, words, rows: Some(frame.pick_rows(&mask)?), lengths, counts })
}

/// An empty frame with a file's columns and a score.
fn empty_scored(path: Option<&String>) -> Result<CrowFrame, String> {
    let Some(path) = path else {
        return Ok(CrowFrame::new(vec!["score".to_string()], vec![Column::Float(Vec::new())], vec![Vec::new()]));
    };
    let r = Reader::open(path)?;
    let all: Vec<usize> = (0..r.footer().columns.len()).collect();
    r.empty(&all).with_column("score", Column::Float(Vec::new()))
}

/// `sys.batch_search[paths, column, words, k]`: the `k` rows of the files
/// whose indexed `column` best match the words, with the files' columns
/// and a `score`, the best first and an earlier position first between
/// equals. BM25 as Lucene's `BM25Similarity` scores it (k1 = 1.2, b =
/// 0.75), over the files as one corpus: `n` and the average length from
/// every file's index, the document frequencies from the rows read. Each
/// word asked is read as its tokens, lowercased, a word asked twice
/// counting once; a group whose filter admits no word is not read.
pub fn search(paths: &[String], column: &str, words: &[String], k: i64) -> Result<CrowFrame, String> {
    if k < 0 {
        return Err(format!("batch: a search answers at most k rows, and k is {}", k));
    }
    let mut asked: Vec<String> = words.iter().flat_map(|w| tokens(w)).collect();
    let mut seen = std::collections::HashSet::new();
    asked.retain(|w| seen.insert(w.clone()));
    if asked.is_empty() || paths.is_empty() {
        return empty_scored(paths.first());
    }
    let parts = crate::cores::map_items(paths, paths.len() * GROUP_ROWS_LEAST, |p| searched(p, column, &asked));
    let parts: Vec<Searched> = parts.into_iter().collect::<Result<_, _>>()?;
    let n: usize = parts.iter().map(|p| p.docs).sum();
    let total: usize = parts.iter().map(|p| p.words).sum();
    let mut df = vec![0.0; asked.len()];
    for p in &parts {
        for count in &p.counts {
            for (i, c) in count.iter().enumerate() {
                if *c > 0.0 {
                    df[i] += 1.0;
                }
            }
        }
    }
    if n == 0 || df.iter().all(|d| *d == 0.0) {
        return empty_scored(paths.first());
    }
    let avgdl = total as f64 / n as f64;
    let idf: Vec<f64> = df.iter().map(|d| (1.0 + (n as f64 - d + 0.5) / (d + 0.5)).ln()).collect();
    let (k1, b) = (1.2, 0.75);
    let mut whole: Option<CrowFrame> = None;
    let mut scores: Vec<f64> = Vec::new();
    for p in parts {
        let Some(rows) = p.rows else { continue };
        for (dl, count) in p.lengths.iter().zip(&p.counts) {
            let mut s = 0.0;
            for (i, tf) in count.iter().enumerate() {
                if *tf > 0.0 {
                    s += idf[i] * (tf / (tf + k1 * (1.0 - b + b * dl / avgdl)));
                }
            }
            scores.push(s);
        }
        whole = Some(match whole {
            None => rows,
            Some(w) => w.stack(&rows)?,
        });
    }
    let Some(whole) = whole else { return empty_scored(paths.first()) };
    let scored = whole.with_column("score", Column::Float(scores))?;
    let by_position = if scored.col_index(POSITION).is_some() { scored.sort(POSITION, false)? } else { scored };
    let ranked = by_position.sort("score", true)?;
    let keep: Vec<bool> = (0..ranked.nrows()).map(|i| i < k as usize).collect();
    ranked.pick_rows(&keep)
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

/// `sys.batch_since[paths, after, wanted]`: the rows of the files at a
/// position past `after`, in position order, with the columns `wanted`
/// (every column when none is named; `_pos` read for the picking and
/// dropped where not asked). A file's groups whose last position is at
/// or before `after` are not read; the files are read across the cores.
pub fn since(paths: &[String], after: i64, wanted: &[String]) -> Result<CrowFrame, String> {
    let needed = with_position(wanted);
    let parts: Vec<Result<CrowFrame, String>> = crate::cores::map_items(paths, paths.len() * GROUP_ROWS_LEAST, |p| {
        let r = Reader::open(p)?;
        let f = r.footer();
        let pc = f.column(POSITION).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, POSITION))?;
        let entries = r.entries(pc)?;
        let groups: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| !matches!(e.max, Some(Bound::Int(m)) if m <= after)).map(|(g, _)| g).collect();
        let cols = r.wanted(&needed)?;
        let frame = r.read(&groups, &cols)?;
        let c = frame.col_index(POSITION).ok_or_else(|| format!("batch: '{}' holds no column '{}'", p, POSITION))?;
        let Column::Int(pos) = frame.data[c].as_ref() else {
            return Err(format!("batch: '{}' holds no positions", p));
        };
        if pos.iter().all(|&x| x > after) {
            return Ok(frame);
        }
        let mask: Vec<bool> = pos.iter().map(|&x| x > after).collect();
        frame.pick_rows(&mask)
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
/// pruning, its keys no identity), the statistics as a batch's, the words
/// of every column any input indexed indexed again. Answers the segment's
/// path; the inputs are left for the caller to remove once the segment is
/// in place. Refused: no files, a file that is neither, files of
/// different columns.
pub fn merge(dir: &str, paths: &[String], keys: &[String], every: bool) -> Result<String, String> {
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
        let r = Reader::open(p)?;
        for c in &r.footer().columns {
            if c.indexed {
                texts.insert(c.name.clone());
            }
        }
        let f = read(p, &[])?;
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
    let bytes = file_of(&merged, first, last, keys, &texts).map_err(|e| format!("batch: {}", e))?;
    place(dir, &segment_name(lo, hi), &bytes, true)
}

/// `sys.batch_sync[dir]`: every batch and segment under `dir` flushed to
/// the disk, then the directory — what a batch written with `flush`
/// false had skipped. Answers how many files were flushed.
pub fn sync(dir: &str) -> Result<i64, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("batch: cannot read '{}': {}", dir, e))?;
    let mut n = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if covered(&name).is_none() {
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
        if !f.columns[c].indexed {
            return Err(format!("batch: '{}' indexes no words of column '{}'", path, column));
        }
        let word = value.to_lowercase();
        for e in entries.iter() {
            if let Some(t) = &e.tokens
                && r.token_bloom(t)?.has(word.as_bytes())
            {
                return Ok(true);
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
        assert!(admits(&path, "id", "1", true).unwrap_err().contains("indexes no words of column 'id'"));
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
        let seg = merge(&dir, &[pa.clone(), pb.clone()], &["id".to_string()], false).unwrap();
        assert!(seg.ends_with("000000000000-000000000001.segment"), "{}", seg);
        assert_eq!(covered("000000000000-000000000001.segment"), Some((0, 1)));
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
        let seg2 = merge(&dir, &[seg.clone(), pc.clone()], &[], false).unwrap();
        assert!(seg2.ends_with("000000000000-000000000002.segment"));
        assert_eq!(read(&seg2, &[]).unwrap().nrows(), 5);
        assert!(merge(&dir, &[seg.clone(), pc.clone()], &[], false).unwrap_err().contains("was written already"));
        assert!(merge(&dir, &[], &[], false).unwrap_err().contains("one file or more"));
        let mut t4 = Text::new();
        t4.push("four once more");
        let d = CrowFrame::new(vec!["id".into(), "name".into()], vec![Column::Int(vec![4]), Column::Str(t4)], vec![vec![true], vec![true]]);
        let pd = write(&dir, 3, 6, &d, &["id".to_string()], &[], true).unwrap();
        let kept = merge(&dir, &[seg2.clone(), pd], &["id".to_string()], true).unwrap();
        assert!(kept.ends_with("000000000000-000000000003.segment"));
        let k = read(&kept, &[]).unwrap();
        assert_eq!(k.nrows(), 6);
        assert_eq!(ints(&k, 0), vec![1, 2, 3, 4, 4, 4]);
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
        let seg = merge(&dir, &[pa.clone()], &["id".to_string()], false).unwrap();
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
        assert_eq!(segment_name(3, 17), "000000000003-000000000017.segment");
        assert_eq!(covered("000000000017.batch"), Some((17, 17)));
        assert_eq!(covered("x-y.segment"), None);
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
        let seg = merge(&dir, &[pa.clone()], &["day".to_string(), "id".to_string()], true).unwrap();
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
        let all = since(&paths, -1, &[]).unwrap();
        assert_eq!(all.nrows(), 40000);
        let pos = ints(&all, all.col_index(POSITION).unwrap());
        assert!(pos.windows(2).all(|w| w[0] < w[1]));
        let late = since(&paths, 39990, &["name".to_string()]).unwrap();
        assert_eq!(late.nrows(), 9);
        assert_eq!(late.columns, vec!["name"]);
        let none = since(&paths, 40000, &[]).unwrap();
        assert_eq!(none.nrows(), 0);
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
        let hits = search(&paths, "text", &["fox".to_string()], 10).unwrap();
        assert_eq!(ints(&hits, 0), vec![1, 3]);
        let scores = match hits.data[hits.col_index("score").unwrap()].as_ref() { Column::Float(v) => v.clone(), _ => panic!() };
        assert!(scores[0] > scores[1]);
        let two = search(&paths, "text", &["brown".to_string(), "dog".to_string()], 2).unwrap();
        assert_eq!(two.nrows(), 2);
        let none = search(&paths, "text", &["zebra".to_string()], 10).unwrap();
        assert_eq!(none.nrows(), 0);
        assert!(none.col_index("score").is_some());
        assert!(search(&paths, "id", &["fox".to_string()], 10).unwrap_err().contains("indexes no words"));
        assert!(search(&paths, "text", &["fox".to_string()], -1).is_err());
        let seg = merge(&dir, &paths, &[], false).unwrap();
        let again = search(&[seg], "text", &["fox".to_string()], 10).unwrap();
        assert_eq!(ints(&again, 0), vec![1, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
