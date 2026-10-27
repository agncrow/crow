//! A Magpie file: one format for a batch, a segment and the manifest.
//!
//! ```text
//! magic "MAGPIE\0\x02"
//! group 0: chunk of column 0, chunk of column 1, …, then the group's
//!          filters (a bloom per column that has one, a token filter per
//!          indexed text column)
//! group 1: …
//! column blocks: per column, one entry per group — where its chunk lies,
//!          how many cells are missing, how many distinct values, min and
//!          max, where its filters lie and (a text column indexed) the
//!          group's document and word counts
//! footer: version, rows, first and last position, the keys, the groups'
//!          row counts, the columns (name, kind, indexed, where its block
//!          lies)
//! u32 footer length, magic again
//! ```
//!
//! A reader reads the last bytes, then the footer, then only what it is
//! asked for: one column's block to prune by, one group's chunk to
//! decode. The footer and the blocks are kept per process by the file's
//! path, size and modification time: a file is written once and never
//! changed.

use super::bloom::Bloom;
use super::encode::{self, Kind};
use crate::frame::{Column, CrowFrame};
use crate::validity::Validity;
use std::sync::{Arc, Mutex, OnceLock};

pub const MAGIC: &[u8; 8] = b"MAGPIE\0\x02";
pub const VERSION: u8 = 2;

/// The longest text kept as a group's min or max; past it the group
/// keeps no bounds for that column.
const BOUND_TEXT_MOST: usize = 256;

/// How many distinct values a column's filter covers at most: past it
/// the column is left to its min and max (a timestamp, a position, a
/// price — every cell its own value — had been given a filter sized by
/// the rows, 130 MB of filters in a month's segment and 180 ms to open
/// it).
pub const DISTINCT_MOST: usize = 1 << 16;

/// A group's min or max of a column.
#[derive(Clone, Debug, PartialEq)]
pub enum Bound {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

/// A text column's search index over one group: where its token filter
/// lies, where its dictionary and postings lie (postings.rs), and the
/// group's document and word counts.
#[derive(Clone, Debug, Default)]
pub struct Tokens {
    pub off: u64,
    pub len: u32,
    pub docs: u32,
    pub words: u32,
    pub index_off: u64,
    pub index_len: u32,
}

/// One column over one group.
#[derive(Clone, Debug)]
pub struct Entry {
    pub chunk_off: u64,
    pub chunk_len: u32,
    pub rows: u32,
    pub missing: u32,
    /// Distinct present values, or none where more than `DISTINCT_MOST`.
    pub distinct: Option<u32>,
    pub min: Option<Bound>,
    pub max: Option<Bound>,
    /// Where the value filter lies, none where the column has none.
    pub bloom: Option<(u64, u32)>,
    pub tokens: Option<Tokens>,
}

#[derive(Clone, Debug)]
pub struct ColumnMeta {
    pub name: String,
    pub kind: Kind,
    pub indexed: bool,
    pub block_off: u64,
    pub block_len: u32,
}

#[derive(Clone, Debug)]
pub struct Footer {
    pub rows: u64,
    pub first: i64,
    pub last: i64,
    pub keys: Vec<String>,
    /// What the file says of itself beyond its rows: the manifest's
    /// declarations (the log's time column, its narrow columns), each a
    /// name and a text.
    pub notes: Vec<(String, String)>,
    /// Rows per group.
    pub groups: Vec<u32>,
    pub columns: Vec<ColumnMeta>,
}

impl Footer {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
}

// ── writing ─────────────────────────────────────────────────────────────

/// The tokens of a text: its runs of letters and digits, lowercased.
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// A cell's bytes for a filter: an int or a moment's millis as eight
/// little-endian bytes, a float's bits (negative zero as zero), a bool as
/// one byte, text as its UTF-8.
pub fn cell_bytes(col: &Column, i: usize, out: &mut Vec<u8>) {
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

/// A group's statistics of one column: the bounds over the present cells
/// (none where every cell is missing or every number is NaN; a text past
/// 256 bytes gives no bound), the missing count, the distinct count and
/// its filter where the distinct values are few enough.
fn statistics(col: &Column, valid: &Validity, start: usize, end: usize) -> (u32, Option<u32>, Option<Bound>, Option<Bound>, Option<Bloom>) {
    let present: Vec<usize> = (start..end).filter(|&i| valid[i]).collect();
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
        Some(b)
    } else {
        None
    };
    let count = if filtered { Some(distinct.len() as u32) } else { None };
    let (min, max) = match col {
        Column::Int(v) => {
            let it = present.iter().map(|&i| v[i]);
            (it.clone().min().map(Bound::Int), it.max().map(Bound::Int))
        }
        Column::DateTime(ms, _) => {
            let it = present.iter().map(|&i| ms[i]);
            (it.clone().min().map(Bound::Int), it.max().map(Bound::Int))
        }
        Column::Float(v) => {
            let it = present.iter().map(|&i| v[i]).filter(|x| !x.is_nan());
            (it.clone().reduce(f64::min).map(Bound::Float), it.reduce(f64::max).map(Bound::Float))
        }
        Column::Bool(v) => {
            let it = present.iter().map(|&i| v[i]);
            (it.clone().min().map(Bound::Bool), it.max().map(Bound::Bool))
        }
        Column::Str(t) => {
            let it = present.iter().map(|&i| t.get(i));
            let lo = it.clone().min().filter(|s| s.len() <= BOUND_TEXT_MOST).map(|s| Bound::Text(s.to_string()));
            let hi = it.max().filter(|s| s.len() <= BOUND_TEXT_MOST).map(|s| Bound::Text(s.to_string()));
            (lo, hi)
        }
    };
    ((end - start - present.len()) as u32, count, min, max, bloom)
}

/// A text column's index over a group: a filter over its distinct
/// tokens, its dictionary and postings (postings.rs), its document count
/// and its word count — none where the distinct tokens exceed half the
/// documents (an identifier column: an index of every value prunes
/// nothing and costs more than the column). The stemmer applied where
/// the log stems.
fn token_index(t: &crate::textcol::Text, valid: &Validity, start: usize, end: usize, stems: bool) -> Option<(Bloom, Vec<u8>, u32, u32)> {
    let docs_in: Vec<Option<&str>> = (start..end).map(|i| if valid[i] { Some(t.get(i)) } else { None }).collect();
    let mut words: Vec<String> = Vec::new();
    let mut docs = 0u32;
    for d in docs_in.iter().flatten() {
        docs += 1;
        words.extend(tokens(d).into_iter().map(|w| if stems { super::stem::stem(&w) } else { w }));
    }
    words.sort();
    words.dedup();
    if docs >= 16 && words.len() as u32 > docs / 2 {
        return None;
    }
    let mut bloom = Bloom::sized(words.len());
    for w in &words {
        bloom.add(w.as_bytes());
    }
    let (blob, ndocs, length) = super::postings::build(&docs_in, stems);
    debug_assert_eq!(ndocs, docs);
    Some((bloom, blob, docs, length))
}

/// A file being written: its groups appended one at a time, the column
/// blocks and the footer at `finish`.
pub struct Writer {
    out: Vec<u8>,
    columns: Vec<(String, Kind, bool)>,
    narrow: Vec<bool>,
    /// Whether the indexed columns' tokens are stemmed.
    pub stems: bool,
    entries: Vec<Vec<Entry>>,
    groups: Vec<u32>,
}

impl Writer {
    /// The columns the file will hold, in order, each with whether its
    /// words are indexed (a text column only).
    pub fn new(columns: Vec<(String, Kind, bool)>) -> Writer {
        let n = columns.len();
        Self::with_narrow(columns, vec![false; n])
    }

    /// `new`, with the float columns written in 32 bits marked.
    pub fn with_narrow(columns: Vec<(String, Kind, bool)>, narrow: Vec<bool>) -> Writer {
        let mut out = Vec::with_capacity(1 << 16);
        out.extend_from_slice(MAGIC);
        let n = columns.len();
        Writer { out, columns, narrow, stems: false, entries: vec![Vec::new(); n], groups: Vec::new() }
    }

    /// The bytes written so far.
    pub fn written(&self) -> usize {
        self.out.len()
    }

    /// Rows `start..end` of the frame appended as one group; the frame's
    /// columns are the writer's, in its order.
    pub fn group(&mut self, frame: &CrowFrame, start: usize, end: usize) -> Result<(), String> {
        if frame.columns.len() != self.columns.len() {
            return Err(format!("the group holds {} columns and the file {}", frame.columns.len(), self.columns.len()));
        }
        let n = (end - start) as u32;
        let mut pending: Vec<Entry> = Vec::with_capacity(self.columns.len());
        for (c, (name, kind, indexed)) in self.columns.iter().enumerate() {
            if frame.columns[c] != *name || Kind::of(frame.data[c].as_ref()) != *kind {
                return Err(format!("column {} of the group is '{}' of kind {}, and the file's is '{}' of kind {}", c, frame.columns[c], Kind::of(frame.data[c].as_ref()).word(), name, kind.word()));
            }
            let col = frame.data[c].as_ref();
            let valid = frame.validity[c].as_ref();
            let chunk = encode::encode(col, valid, start, end, self.narrow.get(c).copied().unwrap_or(false));
            let chunk_off = self.out.len() as u64;
            self.out.extend_from_slice(&chunk);
            let (missing, distinct, min, max, bloom) = statistics(col, valid, start, end);
            let bloom = bloom.map(|b| {
                let bytes = b.to_bytes();
                let off = self.out.len() as u64;
                self.out.extend_from_slice(&bytes);
                (off, bytes.len() as u32)
            });
            let tokens = if *indexed && let Column::Str(t) = col && let Some((filter, blob, docs, words)) = token_index(t, valid, start, end, self.stems) {
                let bytes = filter.to_bytes();
                let off = self.out.len() as u64;
                self.out.extend_from_slice(&bytes);
                let index_off = self.out.len() as u64;
                self.out.extend_from_slice(&blob);
                Some(Tokens { off, len: bytes.len() as u32, docs, words, index_off, index_len: blob.len() as u32 })
            } else {
                None
            };
            pending.push(Entry { chunk_off, chunk_len: chunk.len() as u32, rows: n, missing, distinct, min, max, bloom, tokens });
        }
        for (c, e) in pending.into_iter().enumerate() {
            self.entries[c].push(e);
        }
        self.groups.push(n);
        Ok(())
    }

    /// The column blocks and the footer written; the file's bytes.
    pub fn finish(mut self, first: i64, last: i64, keys: &[String], notes: &[(String, String)]) -> Vec<u8> {
        use super::pack::{put_blob, put_string, put_var};
        let rows: u64 = self.groups.iter().map(|&g| g as u64).sum();
        let mut metas: Vec<(u64, u32)> = Vec::with_capacity(self.columns.len());
        for entries in &self.entries {
            let mut block = Vec::new();
            for e in entries {
                block.extend_from_slice(&e.chunk_off.to_le_bytes());
                block.extend_from_slice(&e.chunk_len.to_le_bytes());
                block.extend_from_slice(&e.rows.to_le_bytes());
                block.extend_from_slice(&e.missing.to_le_bytes());
                block.extend_from_slice(&e.distinct.unwrap_or(u32::MAX).to_le_bytes());
                put_bound(&mut block, e.min.as_ref());
                put_bound(&mut block, e.max.as_ref());
                let (boff, blen) = e.bloom.unwrap_or((0, 0));
                block.extend_from_slice(&boff.to_le_bytes());
                block.extend_from_slice(&blen.to_le_bytes());
                match &e.tokens {
                    Some(t) => {
                        block.push(1);
                        block.extend_from_slice(&t.off.to_le_bytes());
                        block.extend_from_slice(&t.len.to_le_bytes());
                        block.extend_from_slice(&t.docs.to_le_bytes());
                        block.extend_from_slice(&t.words.to_le_bytes());
                        block.extend_from_slice(&t.index_off.to_le_bytes());
                        block.extend_from_slice(&t.index_len.to_le_bytes());
                    }
                    None => block.push(0),
                }
            }
            let off = self.out.len() as u64;
            self.out.extend_from_slice(&block);
            metas.push((off, block.len() as u32));
        }
        let mut footer = Vec::new();
        footer.push(VERSION);
        footer.extend_from_slice(&rows.to_le_bytes());
        footer.extend_from_slice(&first.to_le_bytes());
        footer.extend_from_slice(&last.to_le_bytes());
        put_var(&mut footer, keys.len() as u64);
        for k in keys {
            put_string(&mut footer, k);
        }
        put_var(&mut footer, notes.len() as u64);
        for (name, text) in notes {
            put_string(&mut footer, name);
            put_string(&mut footer, text);
        }
        put_var(&mut footer, self.groups.len() as u64);
        for g in &self.groups {
            footer.extend_from_slice(&g.to_le_bytes());
        }
        put_var(&mut footer, self.columns.len() as u64);
        for ((name, kind, indexed), (off, len)) in self.columns.iter().zip(&metas) {
            put_string(&mut footer, name);
            footer.push(*kind as u8);
            footer.push(*indexed as u8);
            footer.extend_from_slice(&off.to_le_bytes());
            footer.extend_from_slice(&len.to_le_bytes());
        }
        let _ = put_blob;
        self.out.extend_from_slice(&footer);
        self.out.extend_from_slice(&(footer.len() as u32).to_le_bytes());
        self.out.extend_from_slice(MAGIC);
        self.out
    }
}

fn put_bound(out: &mut Vec<u8>, b: Option<&Bound>) {
    use super::pack::put_string;
    match b {
        None => out.push(0),
        Some(Bound::Int(v)) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Some(Bound::Float(v)) => {
            out.push(2);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        Some(Bound::Bool(v)) => {
            out.push(3);
            out.push(*v as u8);
        }
        Some(Bound::Text(s)) => {
            out.push(4);
            put_string(out, s);
        }
    }
}

fn get_bound(c: &mut super::pack::Cursor) -> Result<Option<Bound>, String> {
    Ok(match c.u8()? {
        0 => None,
        1 => Some(Bound::Int(c.i64()?)),
        2 => Some(Bound::Float(c.f64()?)),
        3 => Some(Bound::Bool(c.u8()? != 0)),
        4 => Some(Bound::Text(c.string()?)),
        other => return Err(format!("no bound is of kind {}", other)),
    })
}

// ── reading ─────────────────────────────────────────────────────────────

/// What a process keeps of a file once opened: its footer and the column
/// blocks read so far.
pub struct Meta {
    pub footer: Footer,
    blocks: Mutex<Vec<Option<Arc<Vec<Entry>>>>>,
}

type Key = (String, u64, Option<std::time::SystemTime>);

fn metas() -> &'static Mutex<std::collections::HashMap<Key, Arc<Meta>>> {
    static METAS: OnceLock<Mutex<std::collections::HashMap<Key, Arc<Meta>>>> = OnceLock::new();
    METAS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Bytes read at an offset, the file's position left alone (several
/// threads read one file).
pub fn read_at(file: &std::fs::File, off: u64, buf: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, off)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0usize;
        while done < buf.len() {
            let n = file.seek_read(&mut buf[done..], off + done as u64)?;
            if n == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the file ends early"));
            }
            done += n;
        }
        Ok(())
    }
}

/// What a reader reads from: a file on disk, or the pending rows of a
/// log as the bytes of a Magpie file in memory.
enum Backing {
    File(std::fs::File),
    Bytes(Arc<Vec<u8>>),
}

impl Backing {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> std::io::Result<()> {
        match self {
            Backing::File(f) => read_at(f, off, buf),
            Backing::Bytes(b) => {
                let (start, end) = (off as usize, off as usize + buf.len());
                if end > b.len() {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the bytes end early"));
                }
                buf.copy_from_slice(&b[start..end]);
                Ok(())
            }
        }
    }

    fn len(&self) -> std::io::Result<u64> {
        match self {
            Backing::File(f) => f.metadata().map(|m| m.len()),
            Backing::Bytes(b) => Ok(b.len() as u64),
        }
    }
}

/// The directory and the positions a pending path names:
/// `<dir>/.pending@first-last` (the positions a listing saw), or
/// `<dir>/.pending` for whatever is pending now.
fn pending_path(path: &str) -> Option<(&str, Option<(i64, i64)>)> {
    let (dir, name) = path.rsplit_once('/')?;
    let rest = name.strip_prefix(super::tail::PENDING)?;
    if rest.is_empty() {
        return Some((dir, None));
    }
    let (first, last) = rest.strip_prefix('@')?.split_once('-')?;
    Some((dir, Some((first.parse().ok()?, last.parse().ok()?))))
}

/// A file open for reading, its footer parsed (or found in the process's
/// cache); `<dir>/.pending@first-last` opens the log's pending rows in
/// memory, or the file a flush moved them to (`tail::pending_source`).
pub struct Reader {
    pub path: String,
    backing: Backing,
    pub meta: Arc<Meta>,
}

impl Reader {
    pub fn open(path: &str) -> Result<Reader, String> {
        if let Some((dir, range)) = pending_path(path) {
            let source = match range {
                Some((first, last)) => super::tail::pending_source(dir, first, last)?,
                None => super::tail::PendingSource::Bytes(super::tail::pending_bytes(dir)?.ok_or_else(|| format!("magpie: cannot read '{}': nothing is pending", path))?),
            };
            let bytes = match source {
                super::tail::PendingSource::File(p) => return Reader::open(&p),
                super::tail::PendingSource::Bytes(b) => b,
            };
            let backing = Backing::Bytes(bytes);
            let footer = read_footer(&backing, backing.len().unwrap_or(0), path)?;
            let n = footer.columns.len();
            return Ok(Reader { path: path.to_string(), backing, meta: Arc::new(Meta { footer, blocks: Mutex::new(vec![None; n]) }) });
        }
        let cannot = |e: std::io::Error| format!("magpie: cannot read '{}': {}", path, e);
        let file = std::fs::File::open(path).map_err(cannot)?;
        let stat = file.metadata().map_err(cannot)?;
        let key: Key = (path.to_string(), stat.len(), stat.modified().ok());
        if let Some(found) = metas().lock().ok().and_then(|m| m.get(&key).cloned()) {
            return Ok(Reader { path: path.to_string(), backing: Backing::File(file), meta: found });
        }
        let backing = Backing::File(file);
        let footer = read_footer(&backing, stat.len(), path)?;
        let n = footer.columns.len();
        let meta = Arc::new(Meta { footer, blocks: Mutex::new(vec![None; n]) });
        if let Ok(mut m) = metas().lock() {
            if m.len() > 4096 {
                m.clear();
            }
            m.insert(key, meta.clone());
        }
        Ok(Reader { path: path.to_string(), backing, meta })
    }

    pub fn footer(&self) -> &Footer {
        &self.meta.footer
    }

    fn bytes(&self, off: u64, len: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; len];
        self.backing.read_at(off, &mut buf).map_err(|e| format!("magpie: cannot read '{}' at {}: {}", self.path, off, e))?;
        Ok(buf)
    }

    /// The column's entries, one per group, read once per process.
    pub fn entries(&self, col: usize) -> Result<Arc<Vec<Entry>>, String> {
        if let Some(found) = self.meta.blocks.lock().ok().and_then(|b| b.get(col).cloned().flatten()) {
            return Ok(found);
        }
        let meta = self.meta.footer.columns.get(col).ok_or_else(|| format!("magpie: '{}' has no column {}", self.path, col))?;
        let bytes = self.bytes(meta.block_off, meta.block_len as usize)?;
        let mut c = super::pack::Cursor::new(&bytes);
        let mut entries = Vec::with_capacity(self.meta.footer.groups.len());
        for _ in 0..self.meta.footer.groups.len() {
            let chunk_off = c.u64()?;
            let chunk_len = c.u32()?;
            let rows = c.u32()?;
            let missing = c.u32()?;
            let distinct = match c.u32()? {
                u32::MAX => None,
                d => Some(d),
            };
            let min = get_bound(&mut c)?;
            let max = get_bound(&mut c)?;
            let boff = c.u64()?;
            let blen = c.u32()?;
            let bloom = if blen == 0 { None } else { Some((boff, blen)) };
            let tokens = if c.u8()? == 1 {
                Some(Tokens { off: c.u64()?, len: c.u32()?, docs: c.u32()?, words: c.u32()?, index_off: c.u64()?, index_len: c.u32()? })
            } else {
                None
            };
            entries.push(Entry { chunk_off, chunk_len, rows, missing, distinct, min, max, bloom, tokens });
        }
        let entries = Arc::new(entries);
        if let Ok(mut b) = self.meta.blocks.lock()
            && let Some(slot) = b.get_mut(col)
        {
            *slot = Some(entries.clone());
        }
        Ok(entries)
    }

    /// The value filter of a column over a group, none where the column
    /// keeps none there.
    pub fn bloom(&self, e: &Entry) -> Result<Option<Bloom>, String> {
        let Some((off, len)) = e.bloom else { return Ok(None) };
        let bytes = self.bytes(off, len as usize)?;
        Bloom::from_bytes(&bytes).map(Some).map_err(|r| format!("magpie: '{}': {}", self.path, r))
    }

    /// The token filter of a text column over a group.
    pub fn token_bloom(&self, t: &Tokens) -> Result<Bloom, String> {
        let bytes = self.bytes(t.off, t.len as usize)?;
        Bloom::from_bytes(&bytes).map_err(|r| format!("magpie: '{}': {}", self.path, r))
    }

    /// The dictionary and postings of a text column over a group, as bytes.
    pub fn index_bytes(&self, t: &Tokens) -> Result<Vec<u8>, String> {
        self.bytes(t.index_off, t.index_len as usize)
    }

    /// One column's chunk over one group, decoded.
    pub fn chunk(&self, col: usize, e: &Entry) -> Result<(Column, Validity), String> {
        let kind = self.meta.footer.columns[col].kind;
        let bytes = self.bytes(e.chunk_off, e.chunk_len as usize)?;
        encode::decode(&bytes, kind).map_err(|r| format!("magpie: '{}', column '{}': {}", self.path, self.meta.footer.columns[col].name, r))
    }

    /// The columns named (every column when none is), as indices in the
    /// order asked; a name the file lacks is refused.
    pub fn wanted(&self, names: &[String]) -> Result<Vec<usize>, String> {
        if names.is_empty() {
            return Ok((0..self.meta.footer.columns.len()).collect());
        }
        names.iter().map(|w| self.meta.footer.column(w).ok_or_else(|| format!("magpie: '{}' holds no column '{}'", self.path, w))).collect()
    }

    /// One group's rows with the columns asked.
    pub fn group(&self, g: usize, cols: &[usize]) -> Result<CrowFrame, String> {
        let mut names = Vec::with_capacity(cols.len());
        let mut data = Vec::with_capacity(cols.len());
        let mut validity = Vec::with_capacity(cols.len());
        for &c in cols {
            let entries = self.entries(c)?;
            let e = entries.get(g).ok_or_else(|| format!("magpie: '{}' has no group {}", self.path, g))?;
            let (col, valid) = self.chunk(c, e)?;
            names.push(self.meta.footer.columns[c].name.clone());
            data.push(Arc::new(col));
            validity.push(Arc::new(valid));
        }
        Ok(CrowFrame { columns: names, data, validity })
    }

    /// The groups' rows stacked, with the columns asked, the groups
    /// decoded across the cores.
    pub fn read(&self, groups: &[usize], cols: &[usize]) -> Result<CrowFrame, String> {
        if groups.is_empty() {
            return Ok(self.empty(cols));
        }
        if groups.len() == 1 {
            return self.group(groups[0], cols);
        }
        let rows: usize = groups.iter().map(|&g| self.meta.footer.groups.get(g).copied().unwrap_or(0) as usize).sum();
        let parts: Vec<Result<CrowFrame, String>> = crate::cores::map_items(groups, rows * cols.len().max(1), |&g| self.group(g, cols));
        let parts: Vec<CrowFrame> = parts.into_iter().collect::<Result<_, _>>()?;
        CrowFrame::stack_all(&parts)
    }

    /// A frame of the columns asked and no rows.
    pub fn empty(&self, cols: &[usize]) -> CrowFrame {
        let mut names = Vec::with_capacity(cols.len());
        let mut data = Vec::with_capacity(cols.len());
        let mut validity = Vec::with_capacity(cols.len());
        for &c in cols {
            let m = &self.meta.footer.columns[c];
            names.push(m.name.clone());
            data.push(Arc::new(match m.kind {
                Kind::Int => Column::Int(Vec::new()),
                Kind::Float => Column::Float(Vec::new()),
                Kind::Text => Column::Str(crate::textcol::Text::new()),
                Kind::Bool => Column::Bool(Vec::new()),
                Kind::Moment => Column::DateTime(Vec::new(), Vec::new()),
            }));
            validity.push(Arc::new(Validity::all(0)));
        }
        CrowFrame { columns: names, data, validity }
    }
}

/// The footer read from the end of the file.
fn read_footer(file: &Backing, size: u64, path: &str) -> Result<Footer, String> {
    let no = |why: &str| format!("magpie: '{}' is no Magpie file: {}", path, why);
    if size < 8 + 12 {
        return Err(no("it is too short"));
    }
    let mut tail = [0u8; 12];
    file.read_at(size - 12, &mut tail).map_err(|e| format!("magpie: cannot read '{}': {}", path, e))?;
    if &tail[4..] != MAGIC {
        let mut head = [0u8; 8];
        let _ = file.read_at(0, &mut head);
        if &head == MAGIC {
            return Err(format!("magpie: '{}' is cut short: its end carries no footer", path));
        }
        if head.starts_with(b"MAGPIE") {
            return Err(format!("magpie: '{}' was written in format {}, and this Crow reads format {} — read it with the Crow that wrote it, or a newer one", path, head[7], VERSION));
        }
        return Err(no("its end carries no magic"));
    }
    let flen = u32::from_le_bytes(tail[..4].try_into().unwrap_or([0; 4])) as u64;
    if flen + 12 + 8 > size {
        return Err(no("its footer is longer than the file"));
    }
    let mut footer = vec![0u8; flen as usize];
    file.read_at(size - 12 - flen, &mut footer).map_err(|e| format!("magpie: cannot read '{}': {}", path, e))?;
    let mut c = super::pack::Cursor::new(&footer);
    let bad = |e: String| format!("magpie: '{}' has an unreadable footer: {}", path, e);
    let version = c.u8().map_err(bad)?;
    if version > VERSION {
        return Err(format!("magpie: '{}' was written in format {}, and this Crow reads format {} — read it with the Crow that wrote it, or a newer one", path, version, VERSION));
    }
    let rows = c.u64().map_err(bad)?;
    let first = c.i64().map_err(bad)?;
    let last = c.i64().map_err(bad)?;
    let nk = c.var().map_err(bad)? as usize;
    let mut keys = Vec::with_capacity(nk);
    for _ in 0..nk {
        keys.push(c.string().map_err(bad)?);
    }
    let nn = c.var().map_err(bad)? as usize;
    let mut notes = Vec::with_capacity(nn);
    for _ in 0..nn {
        let name = c.string().map_err(bad)?;
        let text = c.string().map_err(bad)?;
        notes.push((name, text));
    }
    let ng = c.var().map_err(bad)? as usize;
    let mut groups = Vec::with_capacity(ng);
    for _ in 0..ng {
        groups.push(c.u32().map_err(bad)?);
    }
    let nc = c.var().map_err(bad)? as usize;
    let mut columns = Vec::with_capacity(nc);
    for _ in 0..nc {
        let name = c.string().map_err(bad)?;
        let kind = Kind::from_byte(c.u8().map_err(bad)?).map_err(bad)?;
        let indexed = c.u8().map_err(bad)? != 0;
        let block_off = c.u64().map_err(bad)?;
        let block_len = c.u32().map_err(bad)?;
        columns.push(ColumnMeta { name, kind, indexed, block_off, block_len });
    }
    if groups.iter().map(|&g| g as u64).sum::<u64>() != rows {
        return Err(no("its groups do not add up to its rows"));
    }
    Ok(Footer { rows, first, last, keys, notes, groups, columns })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textcol::Text;

    fn frame() -> CrowFrame {
        let mut t = Text::new();
        for s in ["Apple pie", "banana split", "Cherry tart", "apple crumble", "durian"] {
            t.push(s);
        }
        CrowFrame::new(
            vec!["id".into(), "price".into(), "name".into(), "ok".into(), "_pos".into()],
            vec![Column::Int(vec![3, 1, 2, 1, 9]), Column::Float(vec![2.5, 1.0, f64::NAN, 0.5, 4.0]), Column::Str(t), Column::Bool(vec![true, false, true, true, false]), Column::Int(vec![0, 1, 2, 3, 4])],
            vec![vec![true; 5], vec![true, true, true, false, true], vec![true; 5], vec![true; 5], vec![true; 5]],
        )
    }

    fn scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("crow-magpie-file-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn a_file_of_two_groups_reads_back_by_column_and_by_group() {
        let f = frame();
        let columns: Vec<(String, Kind, bool)> = f.columns.iter().enumerate().map(|(i, n)| (n.clone(), Kind::of(f.data[i].as_ref()), n == "name")).collect();
        let mut w = Writer::new(columns);
        w.group(&f, 0, 3).unwrap();
        w.group(&f, 3, 5).unwrap();
        let bytes = w.finish(0, 4, &["id".to_string()], &[("time".to_string(), "at".to_string())]);
        let dir = scratch("two-groups");
        let path = format!("{}/a.batch", dir);
        std::fs::write(&path, &bytes).unwrap();
        let r = Reader::open(&path).unwrap();
        let footer = r.footer();
        assert_eq!((footer.rows, footer.first, footer.last), (5, 0, 4));
        assert_eq!(footer.keys, vec!["id".to_string()]);
        assert_eq!(footer.notes, vec![("time".to_string(), "at".to_string())]);
        assert_eq!(footer.groups, vec![3, 2]);
        assert_eq!(footer.columns.len(), 5);
        assert!(footer.columns[2].indexed && !footer.columns[0].indexed);
        let ids = r.entries(0).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!((ids[0].min.clone(), ids[0].max.clone()), (Some(Bound::Int(1)), Some(Bound::Int(3))));
        assert_eq!((ids[1].min.clone(), ids[1].max.clone()), (Some(Bound::Int(1)), Some(Bound::Int(9))));
        assert_eq!(ids[0].distinct, Some(3));
        let prices = r.entries(1).unwrap();
        assert_eq!(prices[1].missing, 1);
        assert_eq!(prices[1].min, Some(Bound::Float(4.0)));
        let names = r.entries(2).unwrap();
        let t = names[0].tokens.as_ref().unwrap();
        assert_eq!((t.docs, t.words), (3, 6));
        let filter = r.token_bloom(t).unwrap();
        assert!(filter.has(b"cherry") && !filter.has(b"durian"));
        let bloom = r.bloom(&ids[0]).unwrap().unwrap();
        assert!(bloom.has(&3i64.to_le_bytes()) && !bloom.has(&9i64.to_le_bytes()));
        let whole = r.read(&[0, 1], &r.wanted(&[]).unwrap()).unwrap();
        assert_eq!(whole.nrows(), 5);
        assert_eq!(match whole.data[2].as_ref() { Column::Str(t) => t.get(4).to_string(), _ => panic!("text") }, "durian");
        assert!(!whole.validity[1][3]);
        let two = r.read(&[1], &r.wanted(&["name".to_string(), "_pos".to_string()]).unwrap()).unwrap();
        assert_eq!(two.columns, vec!["name".to_string(), "_pos".to_string()]);
        assert_eq!(match two.data[1].as_ref() { Column::Int(v) => v[1], _ => panic!("ints") }, 4);
        assert!(r.wanted(&["nosuch".to_string()]).is_err());
        assert_eq!(r.read(&[], &[0]).unwrap().nrows(), 0);
        // the same file again comes from the cache
        let again = Reader::open(&path).unwrap();
        assert!(Arc::ptr_eq(&again.meta, &r.meta));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_torn_or_foreign_file_is_refused_with_its_reason() {
        let f = frame();
        let columns: Vec<(String, Kind, bool)> = f.columns.iter().enumerate().map(|(i, n)| (n.clone(), Kind::of(f.data[i].as_ref()), false)).collect();
        let mut w = Writer::new(columns);
        w.group(&f, 0, 5).unwrap();
        let bytes = w.finish(0, 4, &[], &[]);
        let dir = scratch("torn");
        let cut = format!("{}/cut.batch", dir);
        std::fs::write(&cut, &bytes[..bytes.len() - 20]).unwrap();
        let e = Reader::open(&cut).err().expect("refused");
        assert!(e.contains("cut short"), "{}", e);
        let foreign = format!("{}/foreign.batch", dir);
        std::fs::write(&foreign, b"PAR1 this is not a magpie file at all, not even close").unwrap();
        assert!(Reader::open(&foreign).err().expect("refused").contains("no Magpie file"));
        let newer = format!("{}/newer.batch", dir);
        let mut b = bytes.clone();
        let n = b.len();
        b[n - 1] = 9;
        b[7] = 9;
        std::fs::write(&newer, &b).unwrap();
        assert!(Reader::open(&newer).err().expect("refused").contains("format 9"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
