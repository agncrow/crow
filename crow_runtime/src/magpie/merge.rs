//! The streaming merge (phase 3): the inputs — batches and segments, each
//! sorted by the log's keys then position — read one row group at a time
//! and merged k-way into segments of bounded size, the newest version of
//! each key kept (a key's versions are adjacent in the merged order, the
//! last of a run the newest) unless every row is wanted. Memory is one
//! group per input and one for the output. A segment is cut at
//! `SEGMENT_BYTES` and named by a counter; the manifest names each with
//! its level, its position and batch-number ranges, its rows, its bytes
//! and its key ranges.

use super::encode::Kind;
use super::file::{Reader, Writer};
use super::{place, POSITION};
use crate::frame::{Column, CrowFrame};
use crate::textcol::TextBuilder;
use std::sync::Arc;

/// A segment's size on disk at which the merge cuts it and starts the next.
pub const SEGMENT_BYTES: usize = 1 << 30;
/// How many segments a level holds before its oldest fold into the next.
pub const FANOUT: usize = 8;

/// One input of the merge: its reader, its current group as a frame, and
/// the row the merge is at.
struct Source {
    reader: Reader,
    cols: Vec<usize>,
    group: usize,
    frame: Option<Arc<CrowFrame>>,
    row: usize,
}

impl Source {
    fn open(path: &str, names: &[String]) -> Result<Source, String> {
        let reader = Reader::open(path)?;
        let cols = reader.wanted(names)?;
        let mut s = Source { reader, cols, group: 0, frame: None, row: 0 };
        s.load()?;
        Ok(s)
    }

    /// The next group with rows loaded, none where the input is spent.
    fn load(&mut self) -> Result<(), String> {
        loop {
            if self.group >= self.reader.footer().groups.len() {
                self.frame = None;
                return Ok(());
            }
            let f = self.reader.group(self.group, &self.cols)?;
            self.group += 1;
            self.row = 0;
            if f.nrows() > 0 {
                self.frame = Some(Arc::new(f));
                return Ok(());
            }
        }
    }

    fn done(&self) -> bool {
        self.frame.is_none()
    }

    /// Past the current row; the next group loaded where this one ends.
    fn advance(&mut self) -> Result<(), String> {
        self.row += 1;
        if let Some(f) = &self.frame
            && self.row >= f.nrows()
        {
            self.load()?;
        }
        Ok(())
    }
}

/// Two cells of one column compared as the frame sort orders them, which
/// is the order every file holds its rows in: a missing cell (or a float
/// that is not a number) after every present one, -0 before 0. (A missing
/// cell had come first here while the files hold it last, so a key's
/// versions could interleave across inputs and two of them survive the
/// merge — found porting the merge to Crow.)
fn cmp_cell(a: &CrowFrame, ai: usize, b: &CrowFrame, bi: usize, c: usize) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let nan = |f: &CrowFrame, i: usize| matches!(f.data[c].as_ref(), Column::Float(x) if x[i].is_nan());
    let (va, vb) = (a.validity[c][ai] && !nan(a, ai), b.validity[c][bi] && !nan(b, bi));
    match (va, vb) {
        (false, false) => return Equal,
        (false, true) => return Greater,
        (true, false) => return Less,
        _ => {}
    }
    match (a.data[c].as_ref(), b.data[c].as_ref()) {
        (Column::Int(x), Column::Int(y)) => x[ai].cmp(&y[bi]),
        (Column::Float(x), Column::Float(y)) => x[ai].total_cmp(&y[bi]),
        (Column::Bool(x), Column::Bool(y)) => x[ai].cmp(&y[bi]),
        (Column::Str(x), Column::Str(y)) => x.get(ai).cmp(y.get(bi)),
        (Column::DateTime(x, _), Column::DateTime(y, _)) => x[ai].cmp(&y[bi]),
        // (a key is never a column of vectors: the writer refuses one)
        _ => Equal,
    }
}

/// The rows compared by the key columns, then by position.
fn cmp_rows(a: &CrowFrame, ai: usize, b: &CrowFrame, bi: usize, keys: &[usize], pos: usize) -> std::cmp::Ordering {
    for &c in keys {
        let o = cmp_cell(a, ai, b, bi, c);
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    cmp_cell(a, ai, b, bi, pos)
}

fn same_keys(a: &CrowFrame, ai: usize, b: &CrowFrame, bi: usize, keys: &[usize]) -> bool {
    keys.iter().all(|&c| cmp_cell(a, ai, b, bi, c) == std::cmp::Ordering::Equal)
}

/// A heap of source indices ordered by the sources' current rows: the
/// least row on top. Sift by hand, since the order reads the sources.
struct Heap {
    items: Vec<usize>,
}

impl Heap {
    fn less(sources: &[Source], a: usize, b: usize, keys: &[usize], pos: usize) -> bool {
        let (fa, fb) = (sources[a].frame.as_ref(), sources[b].frame.as_ref());
        match (fa, fb) {
            (Some(fa), Some(fb)) => cmp_rows(fa, sources[a].row, fb, sources[b].row, keys, pos).then(a.cmp(&b)) == std::cmp::Ordering::Less,
            _ => false,
        }
    }

    fn push(&mut self, sources: &[Source], i: usize, keys: &[usize], pos: usize) {
        self.items.push(i);
        let mut at = self.items.len() - 1;
        while at > 0 {
            let parent = (at - 1) / 2;
            if Self::less(sources, self.items[at], self.items[parent], keys, pos) {
                self.items.swap(at, parent);
                at = parent;
            } else {
                break;
            }
        }
    }

    fn pop(&mut self, sources: &[Source], keys: &[usize], pos: usize) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let top = self.items[0];
        let last = self.items.pop().unwrap_or(top);
        if !self.items.is_empty() {
            self.items[0] = last;
            let mut at = 0;
            loop {
                let (l, r) = (2 * at + 1, 2 * at + 2);
                let mut least = at;
                if l < self.items.len() && Self::less(sources, self.items[l], self.items[least], keys, pos) {
                    least = l;
                }
                if r < self.items.len() && Self::less(sources, self.items[r], self.items[least], keys, pos) {
                    least = r;
                }
                if least == at {
                    break;
                }
                self.items.swap(at, least);
                at = least;
            }
        }
        Some(top)
    }
}

/// The output's columns being built for one group.
enum Builder {
    Int(Vec<i64>),
    Float(Vec<f64>),
    Text(TextBuilder),
    Bool(Vec<bool>),
    Moment(Vec<i64>, Vec<i16>),
    /// The width, and the rows' floats laid flat.
    Vectors(usize, Vec<f64>),
}

struct Group {
    builders: Vec<Builder>,
    valid: Vec<Vec<bool>>,
    rows: usize,
    bytes: usize,
}

impl Group {
    fn new(kinds: &[Kind]) -> Group {
        let builders = kinds
            .iter()
            .map(|k| match k {
                Kind::Int => Builder::Int(Vec::new()),
                Kind::Float => Builder::Float(Vec::new()),
                Kind::Text => Builder::Text(TextBuilder::with_capacity(1024)),
                Kind::Bool => Builder::Bool(Vec::new()),
                Kind::Moment => Builder::Moment(Vec::new(), Vec::new()),
                Kind::Vector => Builder::Vectors(0, Vec::new()),
            })
            .collect();
        Group { builders, valid: vec![Vec::new(); kinds.len()], rows: 0, bytes: 0 }
    }

    /// A row of a frame appended.
    fn push(&mut self, f: &CrowFrame, i: usize) {
        for (c, b) in self.builders.iter_mut().enumerate() {
            let present = f.validity[c][i];
            self.valid[c].push(present);
            match (b, f.data[c].as_ref()) {
                (Builder::Int(v), Column::Int(x)) => {
                    v.push(x[i]);
                    self.bytes += 8;
                }
                (Builder::Float(v), Column::Float(x)) => {
                    v.push(x[i]);
                    self.bytes += 8;
                }
                (Builder::Text(t), Column::Str(x)) => {
                    let s = if present { x.get(i) } else { "" };
                    t.push(s);
                    self.bytes += s.len() + 4;
                }
                (Builder::Bool(v), Column::Bool(x)) => {
                    v.push(x[i]);
                    self.bytes += 1;
                }
                (Builder::Moment(ms, off), Column::DateTime(x, o)) => {
                    ms.push(x[i]);
                    off.push(o[i]);
                    self.bytes += 10;
                }
                // (a column of vectors of no rows has no width yet: the
                // first row written fixes it; a file of another width than
                // the group's is refused by the writer, as a kind is)
                (Builder::Vectors(w, v), Column::Vectors(x_w, x)) => {
                    if *w == 0 {
                        *w = *x_w;
                    }
                    if *w == *x_w {
                        v.extend_from_slice(&x[i * x_w..(i + 1) * x_w]);
                    } else {
                        v.extend(std::iter::repeat_n(0.0, *w));
                    }
                    self.bytes += 8 * *w;
                }
                _ => {}
            }
        }
        self.rows += 1;
    }

    fn frame(self, names: &[String]) -> CrowFrame {
        let data: Vec<Column> = self
            .builders
            .into_iter()
            .map(|b| match b {
                Builder::Int(v) => Column::Int(v),
                Builder::Float(v) => Column::Float(v),
                Builder::Text(t) => Column::Str(t.finish()),
                Builder::Bool(v) => Column::Bool(v),
                Builder::Moment(ms, off) => Column::DateTime(ms, off),
                Builder::Vectors(w, v) => Column::Vectors(w, v),
            })
            .collect();
        CrowFrame::new(names.to_vec(), data, self.valid)
    }
}

/// What one merge wrote: each segment's name, its level, its position
/// and batch-number ranges, its rows and bytes.
pub struct Written {
    pub name: String,
    pub first: i64,
    pub last: i64,
    pub rows: i64,
    pub bytes: i64,
}

/// The files merged into segments under `dir`, each at most
/// `SEGMENT_BYTES`, named from `counter` on: the rows in key-then-position
/// order, one version per key where `keys` are given and `every` is
/// false. Answers what was written, in order.
#[allow(clippy::too_many_arguments)] // the merge's settings, as the row hands them
pub fn stream(dir: &str, paths: &[String], keys: &[String], texts: &[String], narrow: &[String], stems: bool, every: bool, counter: &mut i64, group_bytes: usize, segment_bytes: usize) -> Result<Vec<Written>, String> {
    if paths.is_empty() {
        return Err("batch: a merge takes one file or more".to_string());
    }
    // the columns are the first file's, in its order; every file holds them
    let first = Reader::open(&paths[0])?;
    let names: Vec<String> = first.footer().columns.iter().map(|c| c.name.clone()).collect();
    let kinds: Vec<Kind> = first.footer().columns.iter().map(|c| c.kind).collect();
    drop(first);
    let mut sources = Vec::with_capacity(paths.len());
    for p in paths {
        let s = Source::open(p, &names)?;
        for (i, &c) in s.cols.iter().enumerate() {
            if s.reader.footer().columns[c].kind != kinds[i] {
                return Err(format!("batch: '{}' has other columns than the files before it — a merge takes files of one shape", p));
            }
        }
        sources.push(s);
    }
    let key_cols: Vec<usize> = keys.iter().map(|k| names.iter().position(|n| n == k).ok_or_else(|| format!("batch: no column '{}' to merge by", k))).collect::<Result<_, _>>()?;
    let pos = names.iter().position(|n| n == POSITION).ok_or_else(|| "batch: the files hold no position column".to_string())?;
    let columns: Vec<(String, Kind, bool)> = names.iter().zip(&kinds).map(|(n, k)| (n.clone(), *k, texts.contains(n) && *k == Kind::Text)).collect();
    let narrowed: Vec<bool> = names.iter().zip(&kinds).map(|(n, k)| *k == Kind::Float && narrow.contains(n)).collect();
    let mut heap = Heap { items: Vec::new() };
    for i in 0..sources.len() {
        if !sources[i].done() {
            heap.push(&sources, i, &key_cols, pos);
        }
    }
    let dedup = !every && !key_cols.is_empty();
    let mut written = Vec::new();
    let mut writer: Option<(Writer, i64, i64, i64)> = None; // writer, first, last, rows
    let mut group = Group::new(&kinds);
    let mut pending: Option<(Arc<CrowFrame>, usize)> = None;
    let flush_group = |group: &mut Group, writer: &mut Option<(Writer, i64, i64, i64)>, written: &mut Vec<Written>, counter: &mut i64, last: bool| -> Result<(), String> {
        if group.rows == 0 && !last {
            return Ok(());
        }
        if group.rows > 0 {
            let f = std::mem::replace(group, Group::new(&kinds)).frame(&names);
            let (lo, hi) = match f.data[pos].as_ref() {
                Column::Int(v) => (v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(-1)),
                _ => (0, -1),
            };
            if writer.is_none() {
                let mut w = Writer::with_narrow(columns.clone(), narrowed.clone());
                w.stems = stems;
                *writer = Some((w, lo, hi, 0));
            }
            if let Some((w, first, last_pos, rows)) = writer.as_mut() {
                w.group(&f, 0, f.nrows())?;
                *first = (*first).min(lo);
                *last_pos = (*last_pos).max(hi);
                *rows += f.nrows() as i64;
            }
        }
        let cut = writer.as_ref().is_some_and(|(w, ..)| w.written() >= segment_bytes);
        if (cut || last) && let Some((w, first, last_pos, rows)) = writer.take() {
            *counter += 1;
            let name = super::segment_name_of(*counter);
            let bytes = w.finish(first, last_pos, keys, &[]);
            place(dir, &name, &bytes, true)?;
            written.push(Written { name, first, last: last_pos, rows, bytes: bytes.len() as i64 });
        }
        Ok(())
    };
    while let Some(i) = heap.pop(&sources, &key_cols, pos) {
        let f = sources[i].frame.clone().ok_or_else(|| "batch: a source ran dry on the heap".to_string())?;
        let r = sources[i].row;
        if dedup {
            let replace = match &pending {
                Some((pf, pr)) => same_keys(pf, *pr, &f, r, &key_cols),
                None => false,
            };
            if !replace && let Some((pf, pr)) = pending.take() {
                group.push(&pf, pr);
                if group.bytes >= group_bytes {
                    flush_group(&mut group, &mut writer, &mut written, counter, false)?;
                }
            }
            pending = Some((f, r));
        } else {
            group.push(&f, r);
            if group.bytes >= group_bytes {
                flush_group(&mut group, &mut writer, &mut written, counter, false)?;
            }
        }
        sources[i].advance()?;
        if !sources[i].done() {
            heap.push(&sources, i, &key_cols, pos);
        }
    }
    if let Some((pf, pr)) = pending.take() {
        group.push(&pf, pr);
    }
    flush_group(&mut group, &mut writer, &mut written, counter, true)?;
    Ok(written)
}
