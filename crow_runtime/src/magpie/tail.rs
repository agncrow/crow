//! The sequencer (docs/magpie-design.md, phase 6): a memtable and its
//! write-ahead log. One structure per log directory in the process holds
//! the rows written since the last flush, with the positions they were
//! given as they arrived, the key index (each key's last position, over
//! the files once read and the pending rows), and the image (the newest
//! row per key, built from the files when first asked and kept current
//! by every write after). Every write of the process goes through it:
//! `put` appends under its lock and answers the first position; a thread
//! flushes what is pending as one batch file every `FLUSH_MS`, or `flush`
//! does at once; a commit validates its keys against the index in
//! memory. A reader of the process sees the pending rows at once: a
//! listing names them `<dir>/.pending@first-last`, by the positions it
//! saw, and a reader opens that as an in-memory Magpie file like any
//! other — or, where the thread flushed them between the listing and
//! the read, as the file they went to (`pending_source`). A listing
//! reads the pending rows before it lists the files, and leaves them
//! out where a file covers them, so a flush between its two steps hides
//! nothing and doubles nothing. Another process sees the rows once they
//! are flushed.

use super::encode::Kind;
use super::file::{self, Writer};
use super::{manifest, POSITION};
use crate::frame::{Column, CrowFrame};
use crate::textcol::Text;
use crate::validity::Validity;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// How often the sequencer's thread flushes what is pending.
pub const FLUSH_MS: u64 = 10;
/// The name under the log's directory a reader opens the pending rows by:
/// `<dir>/.pending@first-last`, the positions the listing saw.
pub const PENDING: &str = ".pending";

/// A key's cells as bytes, as the filters hash them, each followed by a
/// zero (a missing cell is the zero alone).
pub fn key_bytes(frame: &CrowFrame, cols: &[usize], i: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cell = Vec::new();
    for &c in cols {
        if frame.validity[c][i] {
            file::cell_bytes(frame.data[c].as_ref(), i, &mut cell);
            out.extend_from_slice(&cell);
        }
        out.push(0);
    }
    out
}

/// One cell of a row the image keeps.
#[derive(Clone, Debug)]
enum Cell {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Moment(i64, i16),
    Missing,
}

fn cell_of(frame: &CrowFrame, c: usize, i: usize) -> Cell {
    if !frame.validity[c][i] {
        return Cell::Missing;
    }
    match frame.data[c].as_ref() {
        Column::Int(v) => Cell::Int(v[i]),
        Column::Float(v) => Cell::Float(v[i]),
        Column::Str(t) => Cell::Text(t.get(i).to_string()),
        Column::Bool(v) => Cell::Bool(v[i]),
        Column::DateTime(ms, off) => Cell::Moment(ms[i], off[i]),
    }
}

/// A frame of the rows given, in the columns' order and kinds.
fn frame_of_rows(columns: &[(String, Kind)], rows: &[&Vec<Cell>]) -> CrowFrame {
    let n = rows.len();
    let mut data = Vec::with_capacity(columns.len());
    let mut validity = Vec::with_capacity(columns.len());
    for (c, (_, kind)) in columns.iter().enumerate() {
        let mut valid = Vec::with_capacity(n);
        let col = match kind {
            Kind::Int => Column::Int(rows.iter().map(|r| match &r[c] { Cell::Int(v) => { valid.push(true); *v } _ => { valid.push(false); 0 } }).collect()),
            Kind::Float => Column::Float(rows.iter().map(|r| match &r[c] { Cell::Float(v) => { valid.push(true); *v } _ => { valid.push(false); 0.0 } }).collect()),
            Kind::Bool => Column::Bool(rows.iter().map(|r| match &r[c] { Cell::Bool(v) => { valid.push(true); *v } _ => { valid.push(false); false } }).collect()),
            Kind::Moment => {
                let (mut ms, mut off) = (Vec::with_capacity(n), Vec::with_capacity(n));
                for r in rows {
                    match &r[c] {
                        Cell::Moment(m, o) => { valid.push(true); ms.push(*m); off.push(*o); }
                        _ => { valid.push(false); ms.push(0); off.push(0); }
                    }
                }
                Column::DateTime(ms, off)
            }
            Kind::Text => {
                let mut t = Text::with_capacity(n, n * 8);
                for r in rows {
                    match &r[c] {
                        Cell::Text(s) => { valid.push(true); t.push(s); }
                        _ => { valid.push(false); t.push(""); }
                    }
                }
                Column::Str(t)
            }
        };
        data.push(col);
        validity.push(valid);
    }
    CrowFrame::new(columns.iter().map(|(n, _)| n.clone()).collect(), data, validity)
}

/// The newest row per key, each row's cells in the image's columns.
struct Image {
    columns: Vec<(String, Kind)>,
    rows: HashMap<Vec<u8>, Vec<Cell>>,
}

/// One log's pending rows, index and image.
pub struct Tail {
    dir: String,
    keys: Vec<String>,
    /// The parts written since the last flush, each with its `_pos` column.
    parts: Vec<CrowFrame>,
    /// The position the next row takes.
    next: i64,
    /// Each key's last position, over the pending rows and, once the
    /// image is built, the files.
    index: HashMap<Vec<u8>, i64>,
    image: Option<Image>,
    /// The pending rows as an in-memory file, rebuilt when read after a put.
    encoded: Option<Arc<Vec<u8>>>,
    /// Why the thread's last flush failed, answered by the next put or flush.
    failed: Option<String>,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Mutex<Tail>>>> {
    static TAILS: OnceLock<Mutex<HashMap<String, Arc<Mutex<Tail>>>>> = OnceLock::new();
    TAILS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn canonical(dir: &str) -> String {
    std::fs::canonicalize(dir).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| dir.to_string())
}

/// The log's tail in this process, none where no write went through it yet.
fn existing(dir: &str) -> Option<Arc<Mutex<Tail>>> {
    registry().lock().ok().and_then(|r| r.get(&canonical(dir)).cloned())
}

/// The log's tail in this process, made on the first use from what the
/// log holds (its keys from the manifest, the next position past its
/// head). The log's directory exists by then.
pub fn tail_of(dir: &str) -> Result<Arc<Mutex<Tail>>, String> {
    if let Some(t) = existing(dir) {
        return Ok(t);
    }
    let keys = manifest::read(dir)?.map(|m| m.keys).unwrap_or_default();
    let next = super::head_of(dir)? + 1;
    let tail = Arc::new(Mutex::new(Tail { dir: dir.to_string(), keys, parts: Vec::new(), next, index: HashMap::new(), image: None, encoded: None, failed: None }));
    if let Ok(mut r) = registry().lock() {
        r.insert(canonical(dir), tail.clone());
    }
    start_flusher();
    Ok(tail)
}

/// The thread that flushes every log's pending rows every `FLUSH_MS`.
fn start_flusher() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("magpie-flush".to_string())
            .spawn(|| loop {
                std::thread::sleep(std::time::Duration::from_millis(FLUSH_MS));
                let tails: Vec<Arc<Mutex<Tail>>> = registry().lock().map(|r| r.values().cloned().collect()).unwrap_or_default();
                for t in tails {
                    if let Ok(mut tail) = t.lock()
                        && !tail.parts.is_empty()
                        && let Err(e) = tail.flush_locked()
                    {
                        tail.failed = Some(e);
                    }
                }
            })
            .ok();
    });
}

fn poisoned() -> String {
    "batch: the log's tail is poisoned".to_string()
}

impl Tail {
    /// What the tail holds let go: nothing pending (a flush in flight
    /// finds nothing to write), the index and the image rebuilt from the
    /// files at the next use.
    fn clear(&mut self) {
        self.parts.clear();
        self.index.clear();
        self.image = None;
        self.encoded = None;
        self.failed = None;
    }

    fn pending_rows(&self) -> usize {
        self.parts.iter().map(|p| p.nrows()).sum()
    }

    /// The pending rows as one frame (with `_pos`), none where nothing is pending.
    fn pending_frame(&self) -> Result<Option<CrowFrame>, String> {
        if self.parts.is_empty() {
            return Ok(None);
        }
        Ok(Some(CrowFrame::stack_all(&self.parts)?))
    }

    /// The pending rows as an in-memory Magpie file, for the readers.
    fn encoded(&mut self) -> Result<Option<Arc<Vec<u8>>>, String> {
        if self.parts.is_empty() {
            return Ok(None);
        }
        if let Some(e) = &self.encoded {
            return Ok(Some(e.clone()));
        }
        let frame = self.pending_frame()?.ok_or_else(|| "nothing pending".to_string())?;
        let first = self.next - frame.nrows() as i64;
        let bytes = Arc::new(self.encode_frame(&frame, first, self.next - 1)?);
        self.encoded = Some(bytes.clone());
        Ok(Some(bytes))
    }

    /// The pending rows at or before `last` as an in-memory Magpie file:
    /// what a listing that saw the rows up to `last` reads, the rows put
    /// since left out.
    fn encoded_upto(&self, last: i64) -> Result<Arc<Vec<u8>>, String> {
        let frame = self.pending_frame()?.ok_or_else(|| "nothing pending".to_string())?;
        let pos = frame.col_index(POSITION).ok_or_else(|| "batch: the pending rows hold no positions".to_string())?;
        let keep: Vec<bool> = match frame.data[pos].as_ref() {
            Column::Int(v) => v.iter().map(|&p| p <= last).collect(),
            _ => vec![true; frame.nrows()],
        };
        let cut = frame.pick_rows(&keep)?;
        let first = self.next - frame.nrows() as i64;
        Ok(Arc::new(self.encode_frame(&cut, first, last)?))
    }

    /// A frame with its positions written as a Magpie file in memory.
    fn encode_frame(&self, frame: &CrowFrame, first: i64, last: i64) -> Result<Vec<u8>, String> {
        let columns: Vec<(String, Kind, bool)> = frame.columns.iter().enumerate().map(|(i, n)| (n.clone(), Kind::of(frame.data[i].as_ref()), false)).collect();
        let mut w = Writer::new(columns);
        w.group(frame, 0, frame.nrows())?;
        Ok(w.finish(first, last, &self.keys, &[]))
    }

    fn key_cols(&self, frame: &CrowFrame) -> Result<Vec<usize>, String> {
        let cols: Vec<usize> = self.keys.iter().filter_map(|k| frame.col_index(k)).collect();
        if cols.len() != self.keys.len() {
            return Err(format!("batch: the rows lack a key column of the log ({})", self.keys.join(", ")));
        }
        Ok(cols)
    }

    /// Rows appended: positions given as they arrive, the index and the
    /// image kept current. Answers the first position.
    fn put(&mut self, frame: &CrowFrame) -> Result<i64, String> {
        if let Some(e) = self.failed.take() {
            return Err(format!("batch: the last flush of '{}' failed: {}", self.dir, e));
        }
        let n = frame.nrows();
        if n == 0 {
            return Err("batch: a put holds at least one row".to_string());
        }
        if frame.col_index(POSITION).is_some() {
            return Err(format!("batch: the frame already holds a column '{}', which the log adds", POSITION));
        }
        if let Some(first) = self.parts.first() {
            let had: Vec<&String> = first.columns.iter().filter(|c| *c != POSITION).collect();
            let has: Vec<&String> = frame.columns.iter().collect();
            if had != has {
                return Err("batch: the rows put hold other columns than the rows pending — flush first".to_string());
            }
        }
        // nothing pending: the next position is read off the disk again,
        // since another process, a restore or a log made afresh at the path
        // may have moved the head
        if self.parts.is_empty() {
            self.next = super::head_of(&self.dir)? + 1;
            self.keys = manifest::read(&self.dir)?.map(|m| m.keys).unwrap_or_default();
            if self.image.is_some() {
                self.image = None;
                self.index.clear();
            }
        }
        let first = self.next;
        let positions = Column::Int((first..first + n as i64).collect());
        let numbered = frame.with_column(POSITION, positions)?;
        let cols = self.key_cols(&numbered)?;
        if !self.keys.is_empty() {
            for i in 0..n {
                let kb = key_bytes(&numbered, &cols, i);
                self.index.insert(kb.clone(), first + i as i64);
                if let Some(img) = &mut self.image {
                    let row: Vec<Cell> = img.columns.iter().map(|(name, _)| numbered.col_index(name).map(|c| cell_of(&numbered, c, i)).unwrap_or(Cell::Missing)).collect();
                    img.rows.insert(kb, row);
                }
            }
        }
        self.parts.push(numbered);
        self.next += n as i64;
        self.encoded = None;
        Ok(first)
    }

    /// What is pending written as one batch file and flushed to the
    /// disk, the positions as given; nothing pending answers -1. The
    /// batch number is the next the directory has; a number another
    /// writer took first is tried again.
    fn flush_locked(&mut self) -> Result<i64, String> {
        let Some(frame) = self.pending_frame()? else { return Ok(-1) };
        let n = frame.nrows();
        let first = self.next - n as i64;
        // the rows arrive in position order; the kernel's write sorts by
        // the keys and keeps that order among equals
        let names: Vec<&str> = frame.columns.iter().filter(|c| *c != POSITION).map(String::as_str).collect();
        let plain = frame.select(&names)?;
        let mut tries = 0;
        loop {
            let number = super::next_number(&self.dir)?;
            match super::write(&self.dir, number, first, &plain, &self.keys, &[], true) {
                Ok(_) => break,
                Err(e) if e.contains("was written already") && tries < 100 => tries += 1,
                Err(e) => return Err(e),
            }
        }
        self.parts.clear();
        self.encoded = None;
        Ok(first)
    }

    /// The image installed from the log's table, read outside the lock
    /// (reading the pending rows takes the lock); the parts pending now
    /// applied over it, so a put between the read and this is in it.
    fn install_image(&mut self, table: &CrowFrame) -> Result<(), String> {
        if self.image.is_some() {
            return Ok(());
        }
        let columns: Vec<(String, Kind)> = table.columns.iter().enumerate().map(|(i, n)| (n.clone(), Kind::of(table.data[i].as_ref()))).collect();
        let cols = self.key_cols(table)?;
        let pos = table.col_index(POSITION);
        let mut rows = HashMap::with_capacity(table.nrows());
        for i in 0..table.nrows() {
            let kb = key_bytes(table, &cols, i);
            if let Some(p) = pos
                && let Column::Int(v) = table.data[p].as_ref()
            {
                self.index.entry(kb.clone()).or_insert(v[i]);
            }
            rows.insert(kb, (0..columns.len()).map(|c| cell_of(table, c, i)).collect());
        }
        let mut img = Image { columns, rows };
        for part in &self.parts {
            let pcols = self.key_cols(part)?;
            for i in 0..part.nrows() {
                let kb = key_bytes(part, &pcols, i);
                let row: Vec<Cell> = img.columns.iter().map(|(name, _)| part.col_index(name).map(|c| cell_of(part, c, i)).unwrap_or(Cell::Missing)).collect();
                img.rows.insert(kb, row);
            }
        }
        self.image = Some(img);
        Ok(())
    }

    /// The newest row of a key, as a frame of one row or none.
    fn get(&self, values: &[String]) -> Result<CrowFrame, String> {
        let img = self.image.as_ref().ok_or_else(|| "no image".to_string())?;
        if values.len() != self.keys.len() {
            return Err(format!("batch: a get takes one value per key column: {} columns, {} values", self.keys.len(), values.len()));
        }
        let mut kb = Vec::new();
        for (k, v) in self.keys.iter().zip(values) {
            let (_, kind) = img.columns.iter().find(|(n, _)| n == k).ok_or_else(|| format!("batch: no key column '{}'", k))?;
            let parsed = super::parsed(*kind, v).map_err(|e| format!("batch: column '{}': {}", k, e))?;
            kb.extend_from_slice(&super::parsed_bytes(&parsed));
            kb.push(0);
        }
        match img.rows.get(&kb) {
            Some(row) => Ok(frame_of_rows(&img.columns, &[row])),
            None => Ok(frame_of_rows(&img.columns, &[])),
        }
    }
}

/// `sys.tail_put[dir, frame]`: the rows appended to the log's pending
/// rows, visible to this process at once, flushed within `FLUSH_MS` or at
/// `tail_flush`; answers the first position.
pub fn put(dir: &str, frame: &CrowFrame) -> Result<i64, String> {
    let t = tail_of(dir)?;
    let mut tail = t.lock().map_err(|_| poisoned())?;
    tail.put(frame)
}

/// `sys.tail_flush[dir]`: what is pending written as one batch file and
/// flushed to the disk; answers its first position, -1 for nothing.
pub fn flush(dir: &str) -> Result<i64, String> {
    let Some(t) = existing(dir) else { return Ok(-1) };
    let mut tail = t.lock().map_err(|_| poisoned())?;
    if let Some(e) = tail.failed.take() {
        return Err(format!("batch: the last flush of '{}' failed: {}", dir, e));
    }
    tail.flush_locked()
}

/// The pending rows of the log as an in-memory file, none where nothing
/// is pending (what `Reader::open` answers for `<dir>/.pending`).
pub fn pending_bytes(dir: &str) -> Result<Option<Arc<Vec<u8>>>, String> {
    let Some(t) = existing(dir) else { return Ok(None) };
    let mut tail = t.lock().map_err(|_| poisoned())?;
    tail.encoded()
}

/// What a reader of `<dir>/.pending@first-last` reads: the pending rows
/// as an in-memory file where the positions are still pending (the rows
/// put since the listing left out), else the file the thread flushed
/// them to between the listing and the read — so a listing's rows are
/// always read, and never twice.
pub enum PendingSource {
    Bytes(Arc<Vec<u8>>),
    File(String),
}

pub fn pending_source(dir: &str, first: i64, last: i64) -> Result<PendingSource, String> {
    if let Some(t) = existing(dir) {
        let mut tail = t.lock().map_err(|_| poisoned())?;
        let n = tail.pending_rows() as i64;
        if n > 0 && tail.next - n <= first && first < tail.next {
            if last >= tail.next - 1 {
                let bytes = tail.encoded()?.ok_or_else(|| "nothing pending".to_string())?;
                return Ok(PendingSource::Bytes(bytes));
            }
            return Ok(PendingSource::Bytes(tail.encoded_upto(last)?));
        }
    }
    for p in super::live_paths(dir)? {
        let r = file::Reader::open(&p)?;
        if r.footer().first <= first && first <= r.footer().last {
            return Ok(PendingSource::File(p));
        }
    }
    Err(format!("batch: the rows {}..{} of '{}' are neither pending nor in a file", first, last, dir))
}

/// `sys.tail_pending[dir]`: how many rows are pending, and their first
/// and last positions, as `[rows, first, last]` (`[0, -1, -1]` for none).
pub fn pending(dir: &str) -> Result<Vec<i64>, String> {
    let Some(t) = existing(dir) else { return Ok(vec![0, -1, -1]) };
    let tail = t.lock().map_err(|_| poisoned())?;
    let n = tail.pending_rows() as i64;
    if n == 0 {
        return Ok(vec![0, -1, -1]);
    }
    Ok(vec![n, tail.next - n, tail.next - 1])
}

/// `sys.tail_get[dir, values]`: the newest row of the key, from the
/// image (built from the table when first asked, kept current by every
/// put), as a frame of one row or none.
pub fn get(dir: &str, values: &[String]) -> Result<CrowFrame, String> {
    let t = with_image(dir)?;
    let tail = t.lock().map_err(|_| poisoned())?;
    tail.get(values)
}

/// The log's tail with its image built: the table read with the lock
/// released (the pending rows are read through it), then installed.
fn with_image(dir: &str) -> Result<Arc<Mutex<Tail>>, String> {
    let t = tail_of(dir)?;
    let keys = {
        let tail = t.lock().map_err(|_| poisoned())?;
        if tail.image.is_some() {
            return Ok(t.clone());
        }
        if tail.keys.is_empty() {
            return Err(format!("batch: the log at '{}' has no keys, so no row is the newest of one", dir));
        }
        tail.keys.clone()
    };
    let table = super::latest_of(dir, &keys)?;
    let mut tail = t.lock().map_err(|_| poisoned())?;
    tail.install_image(&table)?;
    drop(tail);
    Ok(t)
}

/// `sys.tail_last[dir, values]`: the last position at which the key was
/// written, as the index knows it (the pending rows, and the files once
/// the image is built); -1 where it does not.
pub fn last(dir: &str, values: &[String]) -> Result<i64, String> {
    let t = with_image(dir)?;
    let tail = t.lock().map_err(|_| poisoned())?;
    let img = tail.image.as_ref().ok_or_else(|| "no image".to_string())?;
    let mut kb = Vec::new();
    for (k, v) in tail.keys.iter().zip(values) {
        let (_, kind) = img.columns.iter().find(|(n, _)| n == k).ok_or_else(|| format!("batch: no key column '{}'", k))?;
        let parsed = super::parsed(*kind, v).map_err(|e| format!("batch: column '{}': {}", k, e))?;
        kb.extend_from_slice(&super::parsed_bytes(&parsed));
        kb.push(0);
    }
    Ok(tail.index.get(&kb).copied().unwrap_or(-1))
}

/// Dropped when a log is removed from under the process: the tail
/// forgotten, so a log made again at the path starts afresh.
pub fn forget(dir: &str) {
    let taken = registry().lock().ok().and_then(|mut r| {
        let by_canonical = r.remove(&canonical(dir));
        let by_name = r.remove(dir);
        by_canonical.or(by_name)
    });
    if let Some(t) = taken
        && let Ok(mut tail) = t.lock()
    {
        tail.clear();
    }
}

/// Every tail of a log at `path` or under it forgotten, its pending
/// rows dropped: `rm[path]` of a log's directory, or of a root holding
/// logs. The flusher, holding the tails from its last listing, had
/// written a log's pending rows into its directory ten milliseconds
/// after a program removed it, and the directory came back as one
/// batch file.
pub fn forget_under(path: &str) {
    // (no path names no log; and every canonical key begins with '/')
    if path.is_empty() {
        return;
    }
    let root = canonical(path);
    let under = |k: &str| k == root || k.starts_with(&format!("{}/", root)) || k == path || k.starts_with(&format!("{}/", path));
    let taken: Vec<Arc<Mutex<Tail>>> = match registry().lock() {
        Ok(mut r) => {
            let keys: Vec<String> = r.keys().filter(|k| under(k)).cloned().collect();
            keys.iter().filter_map(|k| r.remove(k)).collect()
        }
        Err(_) => Vec::new(),
    };
    for t in taken {
        if let Ok(mut tail) = t.lock() {
            tail.clear();
        }
    }
}

#[allow(dead_code)]
fn unused(_: Validity) {}
