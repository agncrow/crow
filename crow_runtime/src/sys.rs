//! `use sys`: the standard library's own layer (spec, "The standard
//! library's layer") — bytes, and a connection to a host, plain or behind
//! TLS, read and written as bytes. A program may not import it: the
//! modules written in Crow build `request`, `socket` and the readers on
//! it, and a program reaches those. Both backends call these functions.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Bytes: what a connection reads and writes, and text encoded as UTF-8.
/// Its own type, not `Vec<u8>`, so it shows as bytes do and is told apart
/// from a list of ints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn len(&self) -> i64 {
        self.0.len() as i64
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl crate::display::CrowDisplay for Bytes {
    fn crow_repr(&self) -> String {
        format!("<{} bytes>", self.0.len())
    }
}

impl std::fmt::Display for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} bytes>", self.0.len())
    }
}

/// `sys.bytes[text]`: the text's UTF-8.
pub fn bytes_of_text(s: &str) -> Bytes {
    Bytes(s.as_bytes().to_vec())
}

/// `sys.text[bytes]`: the text the bytes spell in UTF-8, or the reason
/// they spell none.
pub fn text_of_bytes(b: &Bytes) -> Result<String, String> {
    String::from_utf8(b.0.clone()).map_err(|e| format!("not UTF-8 text: invalid byte at {}", e.utf8_error().valid_up_to()))
}

/// `sys.slice[bytes, from, to]`: the bytes from `from` to `to` (excluded),
/// counted from the end when negative, as a text's slice is.
pub fn slice(b: &Bytes, from: i64, to: i64) -> Result<Bytes, String> {
    let n = b.0.len() as i64;
    let at = |i: i64| if i < 0 { n + i } else { i };
    let (lo, hi) = (at(from), at(to));
    if lo < 0 || hi > n || lo > hi {
        return Err(format!("slice [{}, {}) is outside {} bytes", from, to, n));
    }
    Ok(Bytes(b.0[lo as usize..hi as usize].to_vec()))
}

/// `sys.find[bytes, needle]`: where the needle first occurs, or -1.
pub fn find(b: &Bytes, needle: &Bytes) -> i64 {
    if needle.0.is_empty() {
        return 0;
    }
    b.0.windows(needle.0.len()).position(|w| w == needle.0.as_slice()).map_or(-1, |i| i as i64)
}

/// `sys.concat[a, b]`: the two in order.
pub fn concat(a: &Bytes, b: &Bytes) -> Bytes {
    let mut out = a.0.clone();
    out.extend_from_slice(&b.0);
    Bytes(out)
}

/// `sys.byte[bytes, i]`: the byte at `i` as an int, counted from the end
/// when negative.
pub fn byte(b: &Bytes, i: i64) -> Result<i64, String> {
    let n = b.0.len();
    let at = crate::stops::item(i, n)?;
    Ok(b.0[at] as i64)
}

/// `len[bytes]`: how many.
pub fn len_bytes(b: &Bytes) -> i64 {
    b.len()
}

/// `sys.bytes[n]`: `n` zero bytes — a buffer a kernel written in Crow
/// writes into by position (`set`), as `bytes[text]` is a text's UTF-8
/// (docs/magpie-in-crow.md, 2.1).
pub fn bytes_zeroed(n: i64) -> Result<Bytes, String> {
    let n = usize::try_from(n).map_err(|_| format!("bytes: a count is 0 or more, got {}", n))?;
    // (a count the machine cannot hold is refused, not an abort: `bytes[2
    // ^ 63 - 1]` had taken the interpreter down — crow check --self)
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| format!("bytes: {} bytes are more than can be held", n))?;
    out.resize(n, 0);
    Ok(Bytes(out))
}

fn byte_value(v: i64) -> Result<u8, String> {
    u8::try_from(v).map_err(|_| format!("a byte is 0 to 255, got {}", v))
}

/// `sys.set[bytes, i, v]`: the byte at `i` replaced by `v` (0 to 255),
/// counted from the end when negative, outside a stop — a copy here, as
/// the row's general call; the taken form writes where the bytes lie.
pub fn set_byte(b: &Bytes, i: i64, v: i64) -> Result<Bytes, String> {
    set_byte_taken(b.clone(), i, v)
}

/// `set[b, i, v]` given the bytes (`b = set[b, i, v]`): written where
/// they lie.
pub fn set_byte_taken(mut b: Bytes, i: i64, v: i64) -> Result<Bytes, String> {
    let at = crate::stops::item(i, b.0.len())?;
    b.0[at] = byte_value(v)?;
    Ok(b)
}

/// `sys.push[bytes, v]`: the byte appended (0 to 255).
pub fn push_byte(b: &Bytes, v: i64) -> Result<Bytes, String> {
    push_byte_taken(b.clone(), v)
}

/// `push[b, v]` given the bytes (`b = push[b, v]`): grown where they lie.
pub fn push_byte_taken(mut b: Bytes, v: i64) -> Result<Bytes, String> {
    b.0.push(byte_value(v)?);
    Ok(b)
}

/// `concat[a, b]` given `a` (`a = concat[a, b]`): `b` appended where `a`
/// lies.
pub fn concat_taken(mut a: Bytes, b: &Bytes) -> Bytes {
    a.0.extend_from_slice(&b.0);
    a
}

/// `sys.compress_rs[bytes]`: Magpie's Rust codec, for the Crow codec's
/// tests alone (docs/magpie-in-crow.md): gone with the kernel.
pub fn compress_rs(b: &Bytes) -> Bytes {
    Bytes(crate::magpie::codec::compress(&b.0))
}

/// `sys.decompress_rs[bytes, n]`: the Rust codec's decode, for the tests.
pub fn decompress_rs(b: &Bytes, n: i64) -> Result<Bytes, String> {
    let n = usize::try_from(n).map_err(|_| format!("a length is 0 or more, got {}", n))?;
    crate::magpie::codec::decompress(&b.0, n).map(Bytes)
}

/// `sys.encode_rs[column, from, to, narrow]`: the kernel's chunk encoder,
/// for `magpie.encode`'s tests alone.
pub fn encode_rs(c: &crate::frame::CrowColumn, from: i64, to: i64, narrow: bool) -> Result<Bytes, String> {
    let (from, to) = column_rows(c, from, to, "encode_rs")?;
    Ok(Bytes(crate::magpie::encode::encode(&c.data, &c.valid, from, to, narrow)))
}

/// `sys.decode_rs[chunk, kind]`: the kernel's chunk decoder, for the tests.
pub fn decode_rs(chunk: &Bytes, kind: &str) -> Result<crate::frame::CrowColumn, String> {
    let kind = match kind {
        "int" => crate::magpie::encode::Kind::Int,
        "float" => crate::magpie::encode::Kind::Float,
        "text" => crate::magpie::encode::Kind::Text,
        "bool" => crate::magpie::encode::Kind::Bool,
        "moment" => crate::magpie::encode::Kind::Moment,
        "vector" => crate::magpie::encode::Kind::Vector,
        other => return Err(format!("no column kind is named {}", other)),
    };
    let (col, valid) = crate::magpie::encode::decode(&chunk.0, kind)?;
    Ok(column_of(col, valid))
}

/// `sys.tokens[text]`: the words of a text — each run of letters and
/// digits (Unicode's), lowercased — in order, as a search index and a
/// query split them (a table of which characters are letters is the
/// runtime's, as it is for `lower`).
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// `sys.index_rs[column, from, to, stems]`: the kernel's postings index
/// over a text column's rows, for `magpie.postings`'s tests alone.
pub fn index_rs(c: &crate::frame::CrowColumn, from: i64, to: i64, stems: bool) -> Result<Bytes, String> {
    let (from, to) = column_rows(c, from, to, "index_rs")?;
    let crate::frame::Column::Str(t) = &*c.data else { return Err(not_a("index_rs", "a text", &c.data)) };
    let docs: Vec<Option<&str>> = (from..to).map(|i| if c.valid[i] { Some(t.get(i)) } else { None }).collect();
    let (blob, _, _) = crate::magpie::postings::build(&docs, stems);
    Ok(Bytes(blob))
}

/// `sys.file_rs[frame, first, last, keys, texts, narrow, stems, per]`: the
/// kernel's file of a frame in groups of `per` rows, for `magpie.file`'s
/// tests (the Crow writer sizes its groups by its own rule).
#[allow(clippy::too_many_arguments)]
pub fn file_rs(f: &crate::frame::CrowFrame, first: i64, last: i64, keys: Vec<String>, texts: Vec<String>, narrow: Vec<String>, stems: bool, per: i64) -> Result<Bytes, String> {
    use crate::magpie::{encode::Kind, file::Writer};
    let columns: Vec<(String, Kind, bool)> = f.columns.iter().enumerate().map(|(i, name)| (name.clone(), Kind::of(f.data[i].as_ref()), texts.contains(name))).collect();
    let narrowed: Vec<bool> = columns.iter().map(|(n, k, _)| *k == Kind::Float && narrow.contains(n)).collect();
    let mut w = Writer::with_narrow(columns, narrowed);
    w.stems = stems;
    let per = usize::try_from(per).ok().filter(|p| *p > 0).ok_or_else(|| "file_rs: rows per group are 1 or more".to_string())?;
    let n = f.nrows();
    let mut start = 0;
    while start < n {
        let end = (start + per).min(n);
        w.group(f, start, end)?;
        start = end;
    }
    Ok(Bytes(w.finish(first, last, &keys, &[])))
}

/// `sys.stem_rs[word]`: the kernel's Porter stemmer, for `magpie.stem`'s
/// tests alone.
pub fn stem_rs(word: &str) -> String {
    crate::magpie::stem::stem(word)
}

/// `sys.bloom_rs[items]`: the kernel's bloom filter over the byte strings,
/// sized by their count, as a file holds it — for `magpie.bloom`'s tests
/// alone.
pub fn bloom_rs(items: &[Bytes]) -> Bytes {
    let mut b = crate::magpie::bloom::Bloom::sized(items.len());
    for it in items {
        b.add(&it.0);
    }
    Bytes(b.to_bytes())
}

/// `sys.bloom_has_rs[filter, item]`: whether the kernel's filter, read
/// from its bytes, may hold the byte string — for the tests.
pub fn bloom_has_rs(raw: &Bytes, item: &Bytes) -> Result<bool, String> {
    Ok(crate::magpie::bloom::Bloom::from_bytes(&raw.0)?.has(&item.0))
}

/// `sys.pack_rs[values, width]`: the kernel's bit packing, for
/// `magpie.packing`'s tests alone; each value a 64-bit pattern.
pub fn pack_rs(values: Vec<i64>, width: i64) -> Result<Bytes, String> {
    let width = u8::try_from(width).ok().filter(|w| *w <= 64).ok_or_else(|| format!("a width is 0 to 64 bits, got {}", width))?;
    let patterns: Vec<u64> = values.iter().map(|&v| v as u64).collect();
    Ok(Bytes(crate::magpie::pack::pack(&patterns, width)))
}

/// `sys.unpack_rs[bytes, n, width]`: the kernel's unpacking, for the tests.
pub fn unpack_rs(b: &Bytes, n: i64, width: i64) -> Result<Vec<i64>, String> {
    let n = usize::try_from(n).map_err(|_| format!("a count is 0 or more, got {}", n))?;
    let width = u8::try_from(width).map_err(|_| format!("a width is 0 to 64 bits, got {}", width))?;
    Ok(crate::magpie::pack::unpack(&b.0, n, width)?.into_iter().map(|v| v as i64).collect())
}

/// `sys.put_var_rs[v]`: the kernel's varint, for the tests.
pub fn put_var_rs(v: i64) -> Bytes {
    let mut out = Vec::new();
    crate::magpie::pack::put_var(&mut out, v as u64);
    Bytes(out)
}

/// `sys.read_var_rs[bytes, at]`: the kernel's varint read, for the tests:
/// the value and the position after it.
pub fn read_var_rs(b: &Bytes, at: i64) -> Result<Vec<i64>, String> {
    let at = usize::try_from(at).map_err(|_| format!("a position is 0 or more, got {}", at))?;
    if at > b.0.len() {
        return Err("a count is cut short".to_string());
    }
    let mut c = crate::magpie::pack::Cursor::new(&b.0[at..]);
    let v = c.var()?;
    Ok(vec![v as i64, (at + c.at) as i64])
}

/// `sys.gather[chunks]`: the chunks joined in order — what a connection
/// read, piece by piece, as one (a `concat` per piece would copy the whole
/// each time).
pub fn gather(chunks: &[Bytes]) -> Bytes {
    let mut out = Vec::with_capacity(chunks.iter().map(|c| c.0.len()).sum());
    for c in chunks {
        out.extend_from_slice(&c.0);
    }
    Bytes(out)
}

/// `sys.file[path]`: a file's bytes, as they are (the words `read`'s
/// reason has, since `read` is written on it).
pub fn file(path: &str) -> Result<Bytes, String> {
    std::fs::read(path).map(Bytes).map_err(|e| format!("cannot read '{}': {}", path, e))
}

/// A file's lines, read a block at a time: the lines `io.lines_of` makes of
/// the file's text — split at each `\n`, one `\r` before it dropped, a
/// last line's trailing `\r` too, no empty line after the last break — and
/// the reason a line is not UTF-8 where it is not (a `\n` lies inside no
/// character, so line by line is the whole text's answer). The whole text
/// had been held beside its lines: a 254 MB file's 4 million lines peaked
/// at 624 MB.
pub fn file_lines(path: &str) -> Result<Vec<String>, String> {
    use std::io::BufRead;
    let cannot = |e: std::io::Error| format!("cannot read '{}': {}", path, e);
    let mut reader = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).map_err(cannot)?);
    let mut out = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(cannot)? == 0 {
            return Ok(out);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        match std::str::from_utf8(&line) {
            Ok(text) => out.push(text.to_string()),
            Err(_) => return Err(format!("cannot read '{}': it is not text (UTF-8)", path)),
        }
    }
}

/// A connection's answer, as the HTTP client written in Crow reads one
/// (`sys.exchange`, `sys.fetch`): the status and its phrase, the headers
/// (names in lower case, repeated ones joined) and the body's bytes. The
/// record both modes hold, as `request`'s reply is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub status: i64,
    pub phrase: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub body: Bytes,
}

impl Answer {
    /// The record a failed call holds, as a compiled program's own structs
    /// have theirs (`crow_default`).
    pub fn crow_default() -> Self {
        Answer::default()
    }
}

/// As the interpreter's record shows: its fields in name order.
impl crate::display::CrowDisplay for Answer {
    fn crow_repr(&self) -> String {
        format!("[body = {}, headers = {}, phrase = {}, status = {}]", self.body.crow_repr(), self.headers.crow_repr(), self.phrase.crow_repr(), self.status.crow_repr())
    }
}

impl crate::json::ToJson for Answer {
    fn to_json(&self) -> crate::json::Json {
        crate::json::object(vec![
            ("body", String::from_utf8_lossy(&self.body.0).to_string().to_json()),
            ("headers", self.headers.to_json()),
            ("phrase", self.phrase.to_json()),
            ("status", self.status.to_json()),
        ])
    }
}

/// `sys.answer[status, phrase, headers, body]`: an answer from its parts.
pub fn answer(status: i64, phrase: &str, headers: &std::collections::BTreeMap<String, String>, body: &Bytes) -> Answer {
    Answer { status, phrase: phrase.to_string(), headers: headers.clone(), body: body.clone() }
}

/// `sys.csv_rows[text]`: CSV text as rows, each a map from its column's
/// name to its text (`io.csv` is written on it).
pub fn csv_rows(text: &str) -> Result<Vec<std::collections::BTreeMap<String, String>>, String> {
    crate::io::parse_csv_string(text)
}

/// `sys.frame_of_csv[text]`: a frame from CSV text, each column's type
/// read from its cells (`frame.load` is written on it).
pub fn frame_of_csv(text: &str) -> Result<crate::frame::CrowFrame, String> {
    crate::frame::csv_frame(text)
}

/// `sys.frame_of_csv_bytes[bytes]`: a frame from a CSV file's bytes, read
/// where they are (the text had been copied whole first). Bytes that are
/// not text are the reason.
pub fn frame_of_csv_bytes(b: &Bytes) -> Result<crate::frame::CrowFrame, String> {
    let text = std::str::from_utf8(&b.0).map_err(|_| "it is not text (UTF-8)".to_string())?;
    crate::frame::csv_frame(text)
}

/// `sys.frame_of_json[text]`: a frame from JSON records.
pub fn frame_of_json(text: &str) -> Result<crate::frame::CrowFrame, String> {
    crate::frame::from_json(text)
}

/// `sys.frame_of_rowbinary[bytes]`: a frame from ClickHouse's
/// `RowBinaryWithNamesAndTypes`, each column typed as the header names it
/// (crate::rowbinary).
pub fn frame_of_rowbinary(bytes: &Bytes) -> Result<crate::frame::CrowFrame, String> {
    crate::rowbinary::frame_of_rowbinary(&bytes.0)
}

/// `sys.frame_of_postgres[bytes]`: a frame from the messages Postgres
/// answers a query with (crate::postgres_rows).
pub fn frame_of_postgres(bytes: &Bytes) -> Result<crate::frame::CrowFrame, String> {
    crate::postgres_rows::frame_of_postgres(&bytes.0)
}

/// `sys.frame_of_parquet[bytes]`: a frame from a Parquet file's bytes,
/// where the program was built to read Parquet (the interpreter always
/// is; a compiled program whose paths may name one).
pub fn frame_of_parquet(bytes: &Bytes) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::parquet_frame(bytes.0.clone())
}

/// `sys.batch_write[dir, number, first, frame, keys, texts]`: the frame
/// as the log's next batch file (crate::magpie), its path answered.
pub fn batch_write(dir: &str, number: i64, first: i64, f: &crate::frame::CrowFrame, keys: Vec<String>, texts: Vec<String>, flush: bool) -> Result<String, String> {
    crate::frames::batch_write(dir, number, first, f, &keys, &texts, flush)
}

/// `sys.batch_read[path]`: a batch file as the frame it holds.
pub fn batch_read(path: &str, wanted: Vec<String>) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::batch_read(path, &wanted)
}

/// `sys.batch_stats[path]`: a batch file's statistics as JSON text.
pub fn batch_stats(path: &str) -> Result<String, String> {
    crate::frames::batch_stats(path)
}

/// `sys.batch_admits[path, column, value, token]`: whether the file may
/// hold the value (or, with `token`, the word) in the column.
pub fn batch_admits(path: &str, column: &str, value: &str, token: bool) -> Result<bool, String> {
    crate::frames::batch_admits(path, column, value, token)
}

/// `sys.batch_merge[dir, paths, keys]`: the files folded into one segment
/// (crate::magpie::merge), its path answered.
pub fn batch_merge(dir: &str, paths: Vec<String>, every: bool) -> Result<String, String> {
    crate::frames::batch_merge(dir, &paths, every)
}

/// `sys.stream_rs[dir, paths, keys, texts, narrow, stems, every, taken,
/// group_bytes, segment_bytes]`: the kernel's streaming merge at the sizes
/// given, its segments named from the counter past `taken`; answers their
/// names. The oracle `magpie.merge`'s `stream` is held to, byte for byte,
/// at sizes small enough to cut groups and segments (test only).
#[allow(clippy::too_many_arguments)] // the merge's settings, as the row hands them
pub fn stream_rs(dir: &str, paths: Vec<String>, keys: Vec<String>, texts: Vec<String>, narrow: Vec<String>, stems: bool, every: bool, taken: i64, group_bytes: i64, segment_bytes: i64) -> Result<Vec<String>, String> {
    let size = |n: i64| usize::try_from(n).map_err(|_| format!("stream_rs: a size is 0 or more, not {}", n));
    let mut counter = taken;
    let written = crate::magpie::merge::stream(dir, &paths, &keys, &texts, &narrow, stems, every, &mut counter, size(group_bytes)?, size(segment_bytes)?)?;
    Ok(written.into_iter().map(|w| w.name).collect())
}

/// `sys.batch_summary[paths, by, of, how, on, ops, equal, lo, hi, upto]`:
/// aggregates over the files, pushed below the reader.
pub fn batch_summary(paths: Vec<String>, by: Vec<String>, of: Vec<String>, how: Vec<String>, on: Vec<String>, ops: Vec<String>, equal: Vec<String>, lo: &str, hi: &str, upto: i64) -> Result<crate::frame::CrowFrame, String> {
    crate::magpie::summary(&paths, &by, &of, &how, &on, &ops, &equal, lo, hi, upto)
}

/// `sys.batch_asof[paths, labels, keys, time, wanted, upto]`: the as-of join over the files, as of the position `upto` (-1 for now).
pub fn batch_asof(paths: Vec<String>, labels: &crate::frame::CrowFrame, keys: Vec<String>, time: &str, wanted: Vec<String>, upto: i64) -> Result<crate::frame::CrowFrame, String> {
    crate::magpie::asof(&paths, labels, &keys, time, &wanted, upto)
}

/// `sys.batch_declared[dir]`: what the log declares of itself (its keys
/// and notes) as JSON text, or empty text where it has no manifest.
pub fn batch_declared(dir: &str) -> Result<String, String> {
    crate::magpie::declared(dir)
}

/// `sys.batch_declare[dir, keys, notes]`: the log's manifest written with
/// its keys and notes; refused where one exists with other keys.
pub fn batch_declare(dir: &str, keys: Vec<String>, notes: &str) -> Result<String, String> {
    crate::magpie::declare(dir, &keys, notes)
}

/// `sys.tail_put[dir, frame]`: the rows appended to the log's pending
/// rows, visible to this process at once; answers the first position.
pub fn tail_put(dir: &str, f: &crate::frame::CrowFrame) -> Result<i64, String> {
    let first = crate::magpie::tail::put(dir, f)?;
    crate::magpie::crash_at("pending");
    Ok(first)
}

/// `sys.tail_flush[dir]`: what is pending written as one batch file and
/// flushed to the disk; its first position, -1 for nothing.
pub fn tail_flush(dir: &str) -> Result<i64, String> {
    crate::magpie::tail::flush(dir)
}

/// `sys.tail_pending[dir]`: `[rows, first, last]` of the pending rows.
pub fn tail_pending(dir: &str) -> Result<Vec<i64>, String> {
    crate::magpie::tail::pending(dir)
}

/// `sys.tail_get[dir, values]`: the newest row of the key, from the image.
pub fn tail_get(dir: &str, values: Vec<String>) -> Result<crate::frame::CrowFrame, String> {
    crate::magpie::tail::get(dir, &values)
}

/// `sys.tail_last[dir, values]`: the last position the key was written at, -1 for none.
pub fn tail_last(dir: &str, values: Vec<String>) -> Result<i64, String> {
    crate::magpie::tail::last(dir, &values)
}

/// `sys.tail_forget[dir]`: the log's tail forgotten by this process, its
/// pending rows dropped, never written.
pub fn tail_forget(dir: &str) -> Result<i64, String> {
    crate::magpie::tail::forget(dir);
    Ok(0)
}

/// `sys.batch_catalog[dir]`: the manifest's segments as a frame.
pub fn batch_catalog(dir: &str) -> Result<crate::frame::CrowFrame, String> {
    crate::magpie::catalog(dir)
}

/// `sys.batch_search[paths, column, words, k]`: the k rows of the files
/// whose indexed column best match the words, BM25 as Lucene scores it,
/// with a `score` column.
pub fn batch_search(paths: Vec<String>, column: &str, words: Vec<String>, k: i64, stems: bool, snippet: i64) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::batch_search(&paths, column, &words, k, stems, snippet)
}

/// `sys.batch_nearest[paths, column, query, k]`: the k rows of the files
/// whose vector column lies nearest the query by the cosine.
pub fn batch_nearest(paths: Vec<String>, column: &str, query: &crate::tensor::CrowTensor, k: i64) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::batch_nearest(&paths, column, query, k)
}

/// `sys.batch_find[path, column, value]`: the rows of the file whose
/// column holds the value, the file's filter and its row groups'
/// statistics asked first.
pub fn batch_find(paths: Vec<String>, columns: Vec<String>, values: Vec<String>, wanted: Vec<String>) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::batch_find(&paths, &columns, &values, &wanted)
}

/// `sys.batch_since[paths, after, wanted]`: the files' rows past a
/// position, in position order, with the columns wanted.
pub fn batch_since(paths: Vec<String>, after: i64, upto: i64, wanted: Vec<String>) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::batch_since(&paths, after, upto, &wanted)
}

/// `sys.batch_between[paths, column, lo, hi, upto, wanted]`: the files'
/// rows whose column lies within the bounds, at or before the position.
pub fn batch_between(paths: Vec<String>, column: &str, lo: &str, hi: &str, upto: i64, wanted: Vec<String>) -> Result<crate::frame::CrowFrame, String> {
    crate::magpie::between(&paths, column, lo, hi, upto, &wanted)
}

/// `sys.batch_sync[dir]`: every file of a log flushed to the disk, then
/// the directory; how many files.
pub fn batch_sync(dir: &str) -> Result<i64, String> {
    crate::frames::batch_sync(dir)
}

/// `sys.records[bytes]`: a Kafka message set as a frame (crate::kafka).
pub fn records(b: &Bytes) -> Result<crate::frame::CrowFrame, String> {
    crate::kafka::records(&b.0)
}

/// `sys.records_of[frame]`: a frame as a Kafka message set.
pub fn records_of(f: &crate::frame::CrowFrame) -> Result<Bytes, String> {
    crate::kafka::records_of(f).map(Bytes)
}

/// `sys.unpack[bytes, at, width]`: the big-endian int at a position.
pub fn unpack(b: &Bytes, at: i64, width: i64, little: bool, signed: bool) -> Result<i64, String> {
    let at = usize::try_from(at).map_err(|_| format!("unpack: a position is 0 or more, got {}", at))?;
    let w = match width {
        1 | 2 | 4 | 8 => width as usize,
        other => return Err(format!("unpack: a width is 1, 2, 4 or 8 bytes, got {}", other)),
    };
    let s = b.0.get(at..at + w).ok_or_else(|| format!("unpack: {} bytes at {} lie outside {} bytes", w, at, b.0.len()))?;
    let mut bytes = s.to_vec();
    if little {
        bytes.reverse();
    }
    let mut v: u64 = 0;
    for &x in &bytes {
        v = (v << 8) | x as u64;
    }
    if signed {
        // the top bit of the width read carried into every bit above it
        let shift = 64 - 8 * w as u32;
        Ok(((v << shift) as i64) >> shift)
    } else if w == 8 && v > i64::MAX as u64 {
        Err(format!("unpack: the 8 bytes at {} spell {}, past what an int holds in 64 bits — read them signed", at, v))
    } else {
        Ok(v as i64)
    }
}

/// CRC-32 as gzip, zlib and Kafka's message sets carry it (IEEE 802.3,
/// reflected, 0xEDB88320), written here as the other digests are not:
/// a table made once.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// `sys.ieee[bytes, at, width = 8, little = false]`: the float the bytes
/// at `at` spell, a double in 8 or single precision in 4.
pub fn ieee_at(b: &Bytes, at: i64, width: i64, little: bool) -> Result<f64, String> {
    let at = usize::try_from(at).map_err(|_| format!("ieee: a position is 0 or more, got {}", at))?;
    let w = match width {
        8 | 4 => width as usize,
        other => return Err(format!("ieee: a float's width is 8 bytes, or 4 for single precision, not {}", other)),
    };
    let s = b.0.get(at..at + w).ok_or_else(|| format!("ieee: {} bytes at {} lie outside {} bytes", w, at, b.0.len()))?;
    let mut bytes = s.to_vec();
    if little {
        bytes.reverse();
    }
    Ok(if w == 8 { f64::from_be_bytes(bytes.try_into().unwrap_or([0; 8])) } else { f32::from_be_bytes(bytes.try_into().unwrap_or([0; 4])) as f64 })
}

/// `sys.float_bytes[floats, width, little]`: the floats' bits one after
/// another, 8 bytes each or 4 of single precision — a chunk's worth at
/// once, where `ieee` writes one.
pub fn float_bytes(floats: Vec<f64>, width: i64, little: bool) -> Result<Bytes, String> {
    let mut out = Vec::with_capacity(floats.len() * width.clamp(0, 8) as usize);
    match (width, little) {
        (8, false) => floats.iter().for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
        (8, true) => floats.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
        (4, false) => floats.iter().for_each(|x| out.extend_from_slice(&(*x as f32).to_be_bytes())),
        (4, true) => floats.iter().for_each(|x| out.extend_from_slice(&(*x as f32).to_le_bytes())),
        (other, _) => return Err(format!("float_bytes: a float's width is 8 bytes, or 4 for single precision, not {}", other)),
    }
    Ok(Bytes(out))
}

/// `sys.float_values[bytes, at, n, width, little]`: `n` floats read from
/// the bytes at `at`, the inverse of `float_bytes`.
pub fn float_values(b: &Bytes, at: i64, n: i64, width: i64, little: bool) -> Result<Vec<f64>, String> {
    let at = usize::try_from(at).map_err(|_| format!("float_values: a position is 0 or more, got {}", at))?;
    let n = usize::try_from(n).map_err(|_| format!("float_values: a count is 0 or more, got {}", n))?;
    let w = match width {
        8 | 4 => width as usize,
        other => return Err(format!("float_values: a float's width is 8 bytes, or 4 for single precision, not {}", other)),
    };
    let s = b.0.get(at..at + n * w).ok_or_else(|| format!("float_values: {} floats of {} bytes at {} lie outside {} bytes", n, w, at, b.0.len()))?;
    Ok(s.chunks_exact(w)
        .map(|c| {
            if w == 8 {
                let a: [u8; 8] = c.try_into().unwrap_or([0; 8]);
                if little { f64::from_le_bytes(a) } else { f64::from_be_bytes(a) }
            } else {
                let a: [u8; 4] = c.try_into().unwrap_or([0; 4]);
                (if little { f32::from_le_bytes(a) } else { f32::from_be_bytes(a) }) as f64
            }
        })
        .collect())
}

/// `sys.file[path, from, n]`: the bytes of a region of a file, read
/// without the rest (a footer, one chunk).
pub fn file_region(path: &str, from: i64, n: i64) -> Result<Bytes, String> {
    use std::io::{Read, Seek, SeekFrom};
    let cannot = |e: std::io::Error| format!("cannot read '{}': {}", path, e);
    let from = u64::try_from(from).map_err(|_| format!("file: a position is 0 or more, got {}", from))?;
    let n = usize::try_from(n).map_err(|_| format!("file: a count is 0 or more, got {}", n))?;
    let mut f = std::fs::File::open(path).map_err(cannot)?;
    let size = f.metadata().map_err(cannot)?.len();
    if from + n as u64 > size {
        return Err(format!("file: {} bytes at {} lie outside '{}', which holds {}", n, from, path, size));
    }
    f.seek(SeekFrom::Start(from)).map_err(cannot)?;
    let mut out = vec![0u8; n];
    f.read_exact(&mut out).map_err(cannot)?;
    Ok(Bytes(out))
}

/// `sys.modified[path]`: when the file was last written, a moment (UTC).
pub fn modified(path: &str) -> Result<crate::moments::Moment, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read '{}': {}", path, e))?;
    let when = meta.modified().map_err(|e| format!("cannot read when '{}' was written: {}", path, e))?;
    let ms = match when.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    };
    Ok(crate::moments::Moment { ms, offset: 0 })
}

/// `sys.flush[path]`: the file, or the directory, written through to the
/// disk (`F_FULLFSYNC` on macOS, `fsync` elsewhere, as `File::sync_all`
/// does); answers the path.
pub fn flush(path: &str) -> Result<String, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("cannot flush '{}': {}", path, e))?;
    f.sync_all().map_err(|e| format!("cannot flush '{}': {}", path, e))?;
    Ok(path.to_string())
}

/// `sys.link[from, to]`: a hard link, refused where `to` exists (the
/// system's reason); answers `to`.
pub fn link(from: &str, to: &str) -> Result<String, String> {
    std::fs::hard_link(from, to).map_err(|e| format!("cannot link '{}' as '{}': {}", from, to, e))?;
    Ok(to.to_string())
}

/// `sys.move[from, to]`: renamed over `to`, atomically where the system
/// renames so; answers `to`.
pub fn move_path(from: &str, to: &str) -> Result<String, String> {
    std::fs::rename(from, to).map_err(|e| format!("cannot move '{}' to '{}': {}", from, to, e))?;
    Ok(to.to_string())
}

// A column as its parts, and from them (docs/magpie-in-crow.md, 2.5):
// what a file's writer and reader hold, with no value per cell.

fn column_word(c: &crate::frame::Column) -> &'static str {
    match c {
        crate::frame::Column::Int(_) => "int",
        crate::frame::Column::Float(_) => "float",
        crate::frame::Column::Str(_) => "text",
        crate::frame::Column::Bool(_) => "bool",
        crate::frame::Column::DateTime(..) => "moment",
        crate::frame::Column::Vectors(..) => "vector",
    }
}

fn column_rows(c: &crate::frame::CrowColumn, from: i64, to: i64, what: &str) -> Result<(usize, usize), String> {
    let n = c.len();
    let from = usize::try_from(from).map_err(|_| format!("{}: a row is 0 or more, got {}", what, from))?;
    let to = usize::try_from(to).map_err(|_| format!("{}: a row is 0 or more, got {}", what, to))?;
    if from > to || to > n {
        return Err(format!("{}: rows {} to {} lie outside a column of {}", what, from, to, n));
    }
    Ok((from, to))
}

fn not_a(what: &str, wanted: &str, c: &crate::frame::Column) -> String {
    format!("{} takes {} column, not a column of {}", what, wanted, column_word(c))
}

/// `sys.cell_kind[column]`: `int`, `float`, `text`, `bool`, `moment` or
/// `vector`.
pub fn cell_kind(c: &crate::frame::CrowColumn) -> String {
    column_word(&c.data).to_string()
}

/// The values of rows `from..to` with each missing cell as `zero`: a
/// missing cell's value is one placeholder in both modes (the interpreter
/// holds a column as its cells, and a missing float had come back as NaN
/// where the compiled program kept what was written).
fn present_or<T: Copy>(v: &[T], valid: &crate::validity::Validity, from: usize, to: usize, zero: T) -> Vec<T> {
    (from..to).map(|i| if valid[i] { v[i] } else { zero }).collect()
}

/// `sys.ints_of[column, from, to]`: an int column's values of rows `from`
/// to `to` (excluded), a moment column's milliseconds; a missing cell
/// reads as 0 (`present_of` says which).
pub fn ints_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<i64>, String> {
    let (from, to) = column_rows(c, from, to, "ints_of")?;
    match &*c.data {
        crate::frame::Column::Int(v) => Ok(present_or(v, &c.valid, from, to, 0)),
        crate::frame::Column::DateTime(ms, _) => Ok(present_or(ms, &c.valid, from, to, 0)),
        other => Err(not_a("ints_of", "an int or a moment", other)),
    }
}

/// `sys.floats_of[column, from, to]`: a float column's values, or a vector
/// column's rows laid flat; a missing cell reads as 0.
pub fn floats_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<f64>, String> {
    let (from, to) = column_rows(c, from, to, "floats_of")?;
    match &*c.data {
        crate::frame::Column::Float(v) => Ok(present_or(v, &c.valid, from, to, 0.0)),
        crate::frame::Column::Vectors(..) => vectors_of(c, from as i64, to as i64),
        other => Err(not_a("floats_of", "a float or a vector", other)),
    }
}

/// `sys.bools_of[column, from, to]`: a bool column's values; a missing
/// cell reads as false.
pub fn bools_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<bool>, String> {
    let (from, to) = column_rows(c, from, to, "bools_of")?;
    match &*c.data {
        crate::frame::Column::Bool(v) => Ok(present_or(v, &c.valid, from, to, false)),
        other => Err(not_a("bools_of", "a bool", other)),
    }
}

/// `sys.millis_of[column, from, to]`: a moment column's milliseconds; a
/// missing cell reads as 0.
pub fn millis_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<i64>, String> {
    let (from, to) = column_rows(c, from, to, "millis_of")?;
    match &*c.data {
        crate::frame::Column::DateTime(ms, _) => Ok(present_or(ms, &c.valid, from, to, 0)),
        other => Err(not_a("millis_of", "a moment", other)),
    }
}

/// `sys.zones_of[column, from, to]`: a moment column's offsets, minutes;
/// a missing cell reads as 0.
pub fn zones_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<i64>, String> {
    let (from, to) = column_rows(c, from, to, "zones_of")?;
    match &*c.data {
        crate::frame::Column::DateTime(_, off) => Ok(present_or(off, &c.valid, from, to, 0).into_iter().map(|o| o as i64).collect()),
        other => Err(not_a("zones_of", "a moment", other)),
    }
}

/// `sys.vectors_of[column, from, to]`: a vector column's rows laid flat;
/// a missing row reads as zeros.
pub fn vectors_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<f64>, String> {
    let (from, to) = column_rows(c, from, to, "vectors_of")?;
    match &*c.data {
        crate::frame::Column::Vectors(w, v) => {
            let mut out = Vec::with_capacity((to - from) * w);
            for i in from..to {
                if c.valid[i] {
                    out.extend_from_slice(&v[i * w..(i + 1) * w]);
                } else {
                    out.extend(std::iter::repeat_n(0.0, *w));
                }
            }
            Ok(out)
        }
        other => Err(not_a("vectors_of", "a vector", other)),
    }
}

/// `sys.vector_width[column]`: how many numbers each vector holds.
pub fn vector_width(c: &crate::frame::CrowColumn) -> Result<i64, String> {
    match &*c.data {
        crate::frame::Column::Vectors(w, _) => Ok(*w as i64),
        other => Err(not_a("vector_width", "a vector", other)),
    }
}

/// `sys.text_bytes_of[column, from, to]`: a text column's cells' UTF-8,
/// one after another (a missing cell is empty).
pub fn text_bytes_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Bytes, String> {
    let (from, to) = column_rows(c, from, to, "text_bytes_of")?;
    match &*c.data {
        crate::frame::Column::Str(t) => {
            let mut out = Vec::new();
            for i in from..to {
                if c.valid[i] {
                    out.extend_from_slice(t.get(i).as_bytes());
                }
            }
            Ok(Bytes(out))
        }
        other => Err(not_a("text_bytes_of", "a text", other)),
    }
}

/// `sys.text_ends_of[column, from, to]`: where each cell's bytes end in
/// `text_bytes_of`, one int per cell.
pub fn text_ends_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Vec<i64>, String> {
    let (from, to) = column_rows(c, from, to, "text_ends_of")?;
    match &*c.data {
        crate::frame::Column::Str(t) => {
            let mut at = 0i64;
            let mut out = Vec::with_capacity(to - from);
            for i in from..to {
                if c.valid[i] {
                    at += t.get(i).len() as i64;
                }
                out.push(at);
            }
            Ok(out)
        }
        other => Err(not_a("text_ends_of", "a text", other)),
    }
}

/// `sys.present_of[column, from, to]`: which cells are present, as a
/// bitmap's bytes — bit i of byte i / 8 for cell `from + i` — and no
/// bytes where every cell is.
pub fn present_of(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<Bytes, String> {
    let (from, to) = column_rows(c, from, to, "present_of")?;
    if (from..to).all(|i| c.valid[i]) {
        return Ok(Bytes(Vec::new()));
    }
    let n = to - from;
    let mut out = vec![0u8; n.div_ceil(8)];
    for i in from..to {
        if c.valid[i] {
            out[(i - from) / 8] |= 1 << ((i - from) % 8);
        }
    }
    Ok(Bytes(out))
}

/// `sys.missing_in[column, from, to]`: how many cells are missing.
pub fn missing_in(c: &crate::frame::CrowColumn, from: i64, to: i64) -> Result<i64, String> {
    let (from, to) = column_rows(c, from, to, "missing_in")?;
    Ok((from..to).filter(|&i| !c.valid[i]).count() as i64)
}

/// A column's validity from a bitmap's bytes: none for every cell present.
fn validity_of(present: &Bytes, n: usize, what: &str) -> Result<crate::validity::Validity, String> {
    if present.0.is_empty() {
        return Ok(crate::validity::Validity::all(n));
    }
    if present.0.len() < n.div_ceil(8) {
        return Err(format!("{}: {} cells need {} bytes of presence, and {} are there", what, n, n.div_ceil(8), present.0.len()));
    }
    Ok(crate::validity::Validity::from((0..n).map(|i| present.0[i / 8] & (1 << (i % 8)) != 0).collect::<Vec<bool>>()))
}

fn column_of(data: crate::frame::Column, valid: crate::validity::Validity) -> crate::frame::CrowColumn {
    crate::frame::CrowColumn { data: Arc::new(data), valid: Arc::new(valid) }
}

/// `sys.ints_column[ints, present]`: an int column from its values and
/// its presence bitmap (`present_of`'s bytes, or none).
pub fn ints_column(ints: Vec<i64>, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    let valid = validity_of(present, ints.len(), "ints_column")?;
    Ok(column_of(crate::frame::Column::Int(ints), valid))
}

/// `sys.floats_column[floats, present]`: a float column from its values.
pub fn floats_column(floats: Vec<f64>, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    let valid = validity_of(present, floats.len(), "floats_column")?;
    Ok(column_of(crate::frame::Column::Float(floats), valid))
}

/// `sys.bools_column[bits, n, present]`: a bool column of `n` cells from
/// a bitmap's bytes (bit i of byte i / 8).
pub fn bools_column(bits: &Bytes, n: i64, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    let n = usize::try_from(n).map_err(|_| format!("bools_column: a count is 0 or more, got {}", n))?;
    if bits.0.len() < n.div_ceil(8) {
        return Err(format!("bools_column: {} cells need {} bytes, and {} are there", n, n.div_ceil(8), bits.0.len()));
    }
    let values: Vec<bool> = (0..n).map(|i| bits.0[i / 8] & (1 << (i % 8)) != 0).collect();
    let valid = validity_of(present, n, "bools_column")?;
    Ok(column_of(crate::frame::Column::Bool(values), valid))
}

/// `sys.moments_column[millis, zones, present]`: a moment column from its
/// milliseconds and its offsets in minutes.
pub fn moments_column(millis: Vec<i64>, zones: Vec<i64>, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    if zones.len() != millis.len() {
        return Err(format!("moments_column: {} milliseconds and {} offsets", millis.len(), zones.len()));
    }
    let mut offs = Vec::with_capacity(zones.len());
    for z in zones {
        offs.push(i16::try_from(z).map_err(|_| format!("moments_column: an offset is minutes within a day, not {}", z))?);
    }
    let valid = validity_of(present, millis.len(), "moments_column")?;
    Ok(column_of(crate::frame::Column::DateTime(millis, offs), valid))
}

/// `sys.vectors_column[width, floats, present]`: a vector column from its
/// rows laid flat.
pub fn vectors_column(width: i64, floats: Vec<f64>, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    let w = usize::try_from(width).map_err(|_| format!("vectors_column: a width is 0 or more, got {}", width))?;
    let n = if w == 0 {
        if !floats.is_empty() {
            return Err("vectors_column: a width of 0 holds no numbers".to_string());
        }
        0
    } else {
        if floats.len() % w != 0 {
            return Err(format!("vectors_column: {} numbers do not fill rows of {}", floats.len(), w));
        }
        floats.len() / w
    };
    let valid = validity_of(present, n, "vectors_column")?;
    Ok(column_of(crate::frame::Column::Vectors(w, floats), valid))
}

/// `sys.text_dict_column[bytes, ends, codes, present]`: a text column
/// kept as a dictionary — the distinct values as `text_column` takes
/// them, and each cell the code of its value — as a chunk encodes it and
/// the frame holds it; refused where a code names no value.
pub fn text_dict_column(bytes: &Bytes, ends: Vec<i64>, codes: Vec<i64>, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    let values = text_column(bytes, ends, &Bytes(Vec::new()))?;
    let n = values.len();
    let mut out = Vec::with_capacity(codes.len());
    for c in codes {
        if c < 0 || c as usize >= n {
            return Err(format!("text_dict_column: a code of {} names no value of {}", c, n));
        }
        out.push(c as u32);
    }
    let crate::frame::Column::Str(values) = std::sync::Arc::unwrap_or_clone(values.data) else { return Err("a bug in Crow: text_column made no text".to_string()) };
    let valid = validity_of(present, out.len(), "text_dict_column")?;
    Ok(column_of(crate::frame::Column::Str(crate::textcol::Text::Dict { codes: out, values: Box::new(values) }), valid))
}

/// `sys.text_column[bytes, ends, present]`: a text column from its cells'
/// UTF-8 one after another and where each ends; refused where the bytes
/// spell no text or an end lies outside them.
pub fn text_column(bytes: &Bytes, ends: Vec<i64>, present: &Bytes) -> Result<crate::frame::CrowColumn, String> {
    std::str::from_utf8(&bytes.0).map_err(|_| "text_column: the bytes spell no UTF-8 text".to_string())?;
    let mut offsets: Vec<u64> = Vec::with_capacity(ends.len() + 1);
    offsets.push(0);
    let mut last = 0i64;
    for e in &ends {
        if *e < last || *e as usize > bytes.0.len() {
            return Err(format!("text_column: a cell ends at {}, outside the {} bytes or before the cell before it", e, bytes.0.len()));
        }
        offsets.push(*e as u64);
        last = *e;
    }
    if last as usize != bytes.0.len() {
        return Err(format!("text_column: the cells end at {}, and {} bytes are there", last, bytes.0.len()));
    }
    let valid = validity_of(present, ends.len(), "text_column")?;
    Ok(column_of(crate::frame::Column::Str(crate::textcol::Text::Plain { offsets, bytes: bytes.0.clone() }), valid))
}

/// `sys.frame_csv[frame]`: a frame as CSV text.
pub fn frame_csv(f: &crate::frame::CrowFrame) -> Result<String, String> {
    f.to_csv()
}

/// `sys.frame_json[frame]`: a frame as JSON records.
pub fn frame_json(f: &crate::frame::CrowFrame) -> String {
    crate::frame::to_json(f)
}

/// `sys.frame_parquet[frame]`: a frame as a Parquet file's bytes.
pub fn frame_parquet(f: &crate::frame::CrowFrame) -> Result<Bytes, String> {
    crate::frames::parquet_bytes(f).map(Bytes)
}

/// `sys.tensor_of_frame[frame]`: a frame's cells as a matrix of numbers
/// (`math.load` is written on it).
pub fn tensor_of_frame(f: &crate::frame::CrowFrame) -> Result<crate::tensor::CrowTensor, String> {
    // a table with no rows is a matrix with none, whatever its columns
    // would have held (a header alone types them as text)
    if f.nrows() == 0 && f.ncols() > 0 {
        return Ok(crate::tensor::CrowTensor::from_vec(vec![], vec![0, f.ncols()]));
    }
    let (data, nrows, ncols) = crate::frame::frame_to_tensor_data(f, None)?;
    Ok(crate::tensor::CrowTensor::from_vec(data, vec![nrows, ncols]))
}

/// `sys.store[path, bytes]`: the bytes written to a file, as they are.
pub fn store(path: &str, b: &Bytes) -> Result<(), String> {
    std::fs::write(path, &b.0).map_err(|e| format!("cannot write '{}': {}", path, e))
}

/// `sys.filesize[path]`: how many bytes a file holds, read from its
/// metadata, the file itself unread.
pub fn filesize(path: &str) -> Result<i64, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read '{}': {}", path, e))?;
    if meta.is_dir() {
        return Err(format!("'{}' is a directory, not a file", path));
    }
    i64::try_from(meta.len()).map_err(|_| format!("'{}' is too large to size", path))
}

/// `sys.random[n]`: n bytes from the operating system's randomness.
pub fn random(n: i64) -> Result<Bytes, String> {
    let n = usize::try_from(n).map_err(|_| format!("random: a count cannot be negative (got {})", n))?;
    crate::signin::random_bytes(n).map(Bytes)
}

/// `sys.hex[bytes]`: the bytes as lowercase hex, two digits each.
pub fn hex(b: &Bytes) -> String {
    b.0.iter().map(|x| format!("{:02x}", x)).collect()
}

/// `sys.text[bytes, fill]`: the text the bytes spell, each run that spells
/// none replaced by `fill` (an answer's body shown whatever it holds: the
/// replacement character U+FFFD, as a browser shows it, or any text).
pub fn text_with(b: &Bytes, fill: &str) -> String {
    let lossy = String::from_utf8_lossy(&b.0);
    if fill == "\u{FFFD}" { lossy.to_string() } else { lossy.replace('\u{FFFD}', fill) }
}

/// `sys.hash[bytes, algo]`: the digest as lowercase hex — `'sha256'`,
/// `'sha512'`, `'md5'`.
pub fn hash(b: &Bytes, algo: &str) -> Result<String, String> {
    use sha2::Digest;
    match algo {
        "sha256" => Ok(format!("{:x}", sha2::Sha256::digest(&b.0))),
        "sha512" => Ok(format!("{:x}", sha2::Sha512::digest(&b.0))),
        "sha1" => Ok(format!("{:x}", sha1::Sha1::digest(&b.0))),
        "md5" => Ok(format!("{:x}", md5::Md5::digest(&b.0))),
        "crc32" => Ok(format!("{:08x}", crc32(&b.0))),
        other => Err(format!("hash: '{}' is not a digest of bytes — sha256, sha512, sha1, md5 or crc32", other)),
    }
}

/// `sys.digest[bytes, algo]`: the digest as bytes — `'sha1'` (a socket's
/// accept key), `'sha256'`, `'sha512'` or `'md5'`.
pub fn digest(b: &Bytes, algo: &str) -> Result<Bytes, String> {
    use sha2::Digest;
    match algo {
        "sha1" => Ok(Bytes(sha1::Sha1::digest(&b.0).to_vec())),
        "sha256" => Ok(Bytes(sha2::Sha256::digest(&b.0).to_vec())),
        "sha512" => Ok(Bytes(sha2::Sha512::digest(&b.0).to_vec())),
        "md5" => Ok(Bytes(md5::Md5::digest(&b.0).to_vec())),
        "crc32" => Ok(Bytes(crc32(&b.0).to_be_bytes().to_vec())),
        other => Err(format!("digest: '{}' is not a digest of bytes — sha1, sha256, sha512, md5 or crc32", other)),
    }
}

/// `sys.mask[bytes, key]`: the bytes XORed with the 4-byte key, cycling —
/// a client's frame masked, a masked frame read (RFC 6455). A key of
/// another length stops.
pub fn mask(b: &Bytes, key: &Bytes) -> Result<Bytes, String> {
    if key.0.len() != 4 {
        return Err(format!("mask: a key is 4 bytes, got {}", key.0.len()));
    }
    Ok(Bytes(b.0.iter().enumerate().map(|(i, x)| x ^ key.0[i % 4]).collect()))
}

/// A WebSocket client's connection (`socket[url]`, written in Crow over
/// this layer): the stream after its handshake, the URL it was opened to,
/// and the bytes read past what the socket last needed — the handshake's
/// head, a frame's end — held until it reads on. Shared by every value
/// that holds it.
#[derive(Clone)]
pub struct Socket(Arc<SocketInner>);

struct SocketInner {
    stream: CrowStream,
    url: String,
    unread: Mutex<Vec<u8>>,
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "socket {}", self.0.url)
    }
}

impl crate::display::CrowDisplay for Socket {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

/// A socket in text, as it shows (`'v=`s`'` had not built compiled).
impl std::fmt::Display for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<socket {}>", self.0.url)
    }
}

impl Socket {
    /// A socket that is closed: what a socket that failed to open holds,
    /// so every call on it answers that it is closed.
    pub fn closed() -> Self {
        Socket(Arc::new(SocketInner { stream: CrowStream::closed(), url: String::new(), unread: Mutex::new(Vec::new()) }))
    }
}

/// `sys.socket_of[stream, url, unread]`: a socket over a stream the
/// handshake upgraded, holding what came after the handshake's head.
pub fn socket_of(stream: &CrowStream, url: &str, unread: &Bytes) -> Socket {
    Socket(Arc::new(SocketInner { stream: stream.clone(), url: url.to_string(), unread: Mutex::new(unread.0.clone()) }))
}

/// `sys.stream_of[socket]`: the stream a socket talks over.
pub fn stream_of(s: &Socket) -> CrowStream {
    s.0.stream.clone()
}

/// `sys.leftover[socket]`: the bytes a socket holds unread, taken — none
/// stay with it.
pub fn leftover(s: &Socket) -> Bytes {
    Bytes(std::mem::take(&mut *crate::lock(&s.0.unread)))
}

/// `sys.unread[socket, bytes]`: bytes put back for the socket's next read,
/// before anything it holds (what a read took past a frame's end).
pub fn unread(s: &Socket, b: &Bytes) {
    let mut held = crate::lock(&s.0.unread);
    let mut again = b.0.clone();
    again.extend_from_slice(&held);
    *held = again;
}

/// `sys.base64[bytes]`: the bytes as base64, the standard alphabet padded
/// with `=`, as HTTP's Basic authorization and SMTP's AUTH PLAIN want it.
pub fn base64(b: &Bytes) -> String {
    crate::text::base64_encode(&b.0)
}

/// `sys.base64url[bytes]`: the bytes as base64 in the URL-safe alphabet
/// (`-` and `_`), unpadded, as a token's parts are written (RFC 7515).
pub fn base64url(b: &Bytes) -> String {
    crate::text::base64_encode(&b.0).replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

/// `sys.unbase64[text]`: base64's bytes, either alphabet, padded or not;
/// an error value of kind `'parse'` where the text is no base64.
pub fn unbase64(text: &str) -> Result<Bytes, String> {
    crate::text::base64_decode(text).map(Bytes)
}

/// `sys.same[a, b]`: whether two byte strings are equal, decided in time
/// that does not depend on where they differ (a signature checked against
/// the one computed gives nothing away by how long the check took).
pub fn same(a: &Bytes, b: &Bytes) -> bool {
    if a.0.len() != b.0.len() {
        return false;
    }
    a.0.iter().zip(&b.0).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `sys.pack[n, width]`: an int as `width` bytes, big-endian — 1, 2, 4 or
/// 8, the low bytes of its two's complement (a length in a frame's header,
/// a MessagePack number). Another width stops.
pub fn pack(n: i64, width: i64, little: bool) -> Result<Bytes, String> {
    let mut out = match width {
        1 | 2 | 4 | 8 => n.to_be_bytes()[(8 - width as usize)..].to_vec(),
        other => return Err(format!("pack: a width is 1, 2, 4 or 8 bytes, not {}", other)),
    };
    if little {
        out.reverse();
    }
    Ok(Bytes(out))
}

/// `sys.ieee[x, width = 8, little = false]`: a number as the 8 bytes of
/// its IEEE 754 double, big-endian (a MessagePack float), or the 4 of its
/// single precision.
pub fn ieee(x: f64, width: i64, little: bool) -> Result<Bytes, String> {
    let mut out = match width {
        8 => x.to_be_bytes().to_vec(),
        4 => (x as f32).to_be_bytes().to_vec(),
        other => return Err(format!("ieee: a float's width is 8 bytes, or 4 for single precision, not {}", other)),
    };
    if little {
        out.reverse();
    }
    Ok(Bytes(out))
}

/// A mailer `smtp[host, port, user, pass]` made: the server to talk to and
/// the credentials, which the module written in Crow (src/modules/mail.co)
/// reads through `mailer_field`; it shows as its host, never its password.
#[derive(Clone, Default)]
pub struct Mailer(Arc<MailerInner>);

#[derive(Default)]
struct MailerInner {
    host: String,
    port: i64,
    user: String,
    pass: String,
}

impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mailer:{}", self.0.host)
    }
}

impl crate::display::CrowDisplay for Mailer {
    fn crow_repr(&self) -> String {
        format!("<mailer:{}>", self.0.host)
    }
}

impl std::fmt::Display for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<mailer:{}>", self.0.host)
    }
}

/// `smtp[host, port, user, pass]`: a mailer; its messages come from `user`.
pub fn mailer(host: &str, port: i64, user: &str, pass: &str) -> Mailer {
    Mailer(Arc::new(MailerInner { host: host.to_string(), port, user: user.to_string(), pass: pass.to_string() }))
}

/// `sys.mailer_field[mailer, name]`: what a mailer holds — its `host`,
/// `port` (as text), `user` or `pass`; another name stops.
pub fn mailer_field(m: &Mailer, name: &str) -> Result<String, String> {
    match name {
        "host" => Ok(m.0.host.clone()),
        "port" => Ok(m.0.port.to_string()),
        "user" => Ok(m.0.user.clone()),
        "pass" => Ok(m.0.pass.clone()),
        other => Err(format!("mailer_field: a mailer has a host, a port, a user and a pass, not '{}'", other)),
    }
}

/// `sys.secure[stream, host]`: a plain connection put behind TLS from here
/// on (SMTP's STARTTLS), the certificate checked against the host's name;
/// the same stream, for every value that holds it. An error value of kind
/// `'sys'` where the stream is closed, already secured, or the name is
/// none a certificate can carry.
pub fn secure(s: &CrowStream, host: &str) -> Result<CrowStream, String> {
    let mut guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| format!("{} is no name a certificate can carry", host))?;
    let secured = match guard.take() {
        None => return Err("the stream is closed".to_string()),
        Some(Conn::Process(p)) => {
            *guard = Some(Conn::Process(p));
            return Err("a process is not put behind TLS: a host is".to_string());
        }
        Some(Conn::Listener(l)) => {
            *guard = Some(Conn::Listener(l));
            return Err("a listener is not put behind TLS: a connection is".to_string());
        }
        Some(Conn::Net(Net::Tls(t))) => {
            *guard = Some(Conn::Net(Net::Tls(t)));
            return Err("the stream is already behind TLS".to_string());
        }
        Some(Conn::Net(Net::Plain(reader))) => {
            // (what the server sent before the handshake would be read as
            // TLS: a protocol secures a connection it has read to the end)
            if !reader.buffer().is_empty() {
                let n = reader.buffer().len();
                *guard = Some(Conn::Net(Net::Plain(reader)));
                return Err(format!("{} bytes arrived before the connection was secured", n));
            }
            let tcp = reader.into_inner();
            let client = match rustls::ClientConnection::new(tls_config(), name) {
                Ok(c) => c,
                Err(e) => {
                    *guard = Some(Conn::plain(tcp));
                    return Err(format!("TLS to {}: {}", host, e));
                }
            };
            Conn::Net(Net::Tls(Box::new(rustls::StreamOwned::new(client, tcp))))
        }
    };
    *guard = Some(secured);
    Ok(s.clone())
}

/// `sys.hmac[key, message, algo]`: the keyed digest, as bytes — `'sha256'`
/// (an S3 signature's chain) or `'sha512'`.
pub fn hmac(key: &Bytes, message: &Bytes, algo: &str) -> Result<Bytes, String> {
    use hmac::{Hmac, Mac};
    Ok(Bytes(match algo {
        "sha256" => {
            let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&key.0).map_err(|e| format!("hmac: {}", e))?;
            mac.update(&message.0);
            mac.finalize().into_bytes().to_vec()
        }
        "sha512" => {
            let mut mac = Hmac::<sha2::Sha512>::new_from_slice(&key.0).map_err(|e| format!("hmac: {}", e))?;
            mac.update(&message.0);
            mac.finalize().into_bytes().to_vec()
        }
        other => return Err(format!("hmac: '{}' is not a keyed digest — sha256 or sha512", other)),
    }))
}

/// How much a plain connection reads ahead: a protocol's small reads — a
/// message's five-byte head, a reply line — cost one system call per
/// buffer filled, not one each (a million Postgres rows read a message at
/// a time had cost two million; TLS buffers a record on its own).
const READ_AHEAD: usize = 64 * 1024;

/// What a stream talks over: a host, plain or behind TLS, or a process the
/// program started (`sys.start`), its standard input written and its
/// standard output read.
enum Conn {
    Net(Net),
    Process(Box<ProcessConn>),
    /// A listening socket (`sys.listen`): accepted on, never read or written.
    Listener(std::net::TcpListener),
}

enum Net {
    Plain(std::io::BufReader<std::net::TcpStream>),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>),
}

impl Conn {
    fn plain(tcp: std::net::TcpStream) -> Conn {
        Conn::Net(Net::Plain(std::io::BufReader::with_capacity(READ_AHEAD, tcp)))
    }
}

impl Net {
    fn tcp(&self) -> &std::net::TcpStream {
        match self {
            Net::Plain(s) => s.get_ref(),
            Net::Tls(s) => s.get_ref(),
        }
    }
}

/// A process the program talks to (`sys.start[command]`): the child, its
/// input, what the thread reading its output has handed over and a read
/// has not yet taken, and the command, for messages. Let go — `close`, or
/// the last value holding it gone — it is ended as the MCP stdio
/// transport's shutdown says: its input closed, its exit waited for, then
/// its group signalled, TERM before KILL.
struct ProcessConn {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    out: std::sync::mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
}

/// How long a process let go is given to end on its own after its input
/// closes, and then after TERM, before the next step.
const END_ON_EOF: std::time::Duration = std::time::Duration::from_millis(1500);
const END_ON_TERM: std::time::Duration = std::time::Duration::from_millis(500);

impl ProcessConn {
    /// Whether the process has ended (reaped where it has).
    fn ended(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Whether the process ends within the duration, asked every 20 ms.
    fn ends_within(&mut self, most: std::time::Duration) -> bool {
        let began = std::time::Instant::now();
        loop {
            if self.ended() {
                return true;
            }
            if began.elapsed() >= most {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// The process ended: its input closed (the end-of-file a server exits
    /// on), a wait, TERM to its group (`sh -c` and what it runs), a wait,
    /// KILL; and reaped.
    fn end(&mut self) {
        drop(self.stdin.take());
        if self.ends_within(END_ON_EOF) {
            return;
        }
        signal_group(self.child.id(), "TERM");
        if self.ends_within(END_ON_TERM) {
            return;
        }
        signal_group(self.child.id(), "KILL");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ProcessConn {
    fn drop(&mut self) {
        self.end();
    }
}

/// A signal to the process's group — the process is its group's leader
/// (`process_group(0)`), so `sh -c` and the command it runs both get it.
#[cfg(unix)]
fn signal_group(pid: u32, name: &str) {
    if let Some(args) = group_signal_args(pid, name) {
        let _ = std::process::Command::new("kill").args(&args).output();
    }
}

/// `kill`'s arguments for a signal to a child's group: the group named
/// after `--`, as POSIX writes it — Linux's procps `kill` had read
/// `-TERM -12345` as a signal to every process the user may signal, and on
/// CI the unit tests' step was ended and a runner lost (2026-10-22) — and
/// none for a pid that names no child's group (0 is the caller's own, 1
/// every process).
#[cfg_attr(not(unix), allow(dead_code))]
fn group_signal_args(pid: u32, name: &str) -> Option<Vec<String>> {
    (pid > 1).then(|| vec![format!("-{}", name), "--".to_string(), format!("-{}", pid)])
}

#[cfg(not(unix))]
fn signal_group(_pid: u32, _name: &str) {}

thread_local! {
    /// The processes started on this thread, by command: one per command on
    /// each thread, kept while the thread lives (`start` answers the running
    /// one again, or starts the command anew once it ended). Per thread, not
    /// per program: a served program's handlers run on threads of their
    /// own, and two exchanges with one process — each a write, then reads
    /// until its line, the bytes past it put back — would interleave and
    /// take each other's lines.
    static STARTED: std::cell::RefCell<std::collections::HashMap<String, Socket>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Whether a started process still runs: its stream open, its child not
/// ended.
fn running(s: &Socket) -> bool {
    let Ok(mut guard) = s.0.stream.0.conn.lock() else { return false };
    match guard.as_mut() {
        Some(Conn::Process(p)) => !p.ended(),
        Some(Conn::Net(_)) | Some(Conn::Listener(_)) | None => false,
    }
}

/// `sys.start[command]`: the process the command names, started under this
/// program — `sh -c` runs the command, as `shell` runs one — with its
/// standard input and output piped and its standard error the program's
/// own, as a socket: its stream is written (the process's input) and read
/// (its output), and the socket keeps what a read took past a message for
/// the next. One process per command on each thread: a `start` of a
/// command whose process runs on this thread answers the same socket, one
/// whose process ended (or was closed) starts it again. An error value of kind `'sys'` where the
/// command is empty or the shell cannot start; a command the shell does
/// not find starts, says so on standard error and ends — a read then
/// answers nothing and a write `the process ended`.
pub fn start(command: &str) -> Result<Socket, String> {
    if command.trim().is_empty() {
        return Err("a command is text, and '' is none".to_string());
    }
    STARTED.with(|held| {
        let mut held = held.borrow_mut();
        if let Some(s) = held.get(command)
            && running(s)
        {
            return Ok(s.clone());
        }
        let s = spawn(command)?;
        held.insert(command.to_string(), s.clone());
        Ok(s)
    })
}

/// The command started: the child in a process group of its own, a thread
/// handing its output over a channel (a read waits on the channel, so it
/// can give up after its seconds), and the stream over both.
fn spawn(command: &str) -> Result<Socket, String> {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start '{}': {}", command, e))?;
    let stdin = child.stdin.take();
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(format!("cannot start '{}': its output is not piped", command));
    };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let reading = std::thread::Builder::new().name("crow-process-out".to_string()).spawn(move || {
        let mut buf = vec![0u8; READ_AHEAD];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    if let Err(e) = reading {
        let _ = child.kill();
        return Err(format!("cannot start '{}': {}", command, e));
    }
    let conn = Conn::Process(Box::new(ProcessConn { child, stdin, out: rx, pending: Vec::new() }));
    let stream = CrowStream(Arc::new(StreamInner { conn: Mutex::new(Some(conn)), name: command.to_string(), wait: None, notes: Mutex::new(std::collections::HashMap::new()), timeout_us: std::sync::atomic::AtomicU64::new(0) }));
    Ok(socket_of(&stream, command, &Bytes(Vec::new())))
}

/// A read from a process: what its output's thread handed over, up to
/// `most` bytes, the rest kept; the wait in slices, ending with a stop as
/// a host's read does; nothing once the output closed.
fn recv_process(p: &mut ProcessConn, name: &str, most: i64, seconds: f64, wait: Option<std::time::Duration>) -> Result<Bytes, String> {
    let deadline = wait.and_then(|w| std::time::Instant::now().checked_add(w));
    loop {
        if !p.pending.is_empty() {
            let n = (most as usize).min(p.pending.len());
            return Ok(Bytes(p.pending.drain(..n).collect()));
        }
        let slice = match deadline {
            Some(d) => d.saturating_duration_since(std::time::Instant::now()).min(std::time::Duration::from_millis(100)).max(std::time::Duration::from_millis(1)),
            None => std::time::Duration::from_millis(100),
        };
        match p.out.recv_timeout(slice) {
            Ok(chunk) => p.pending = chunk,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(Bytes(Vec::new())),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if crate::cancel::current_stopped() {
                    return Err("cancelled: nobody waits for this query any more".to_string());
                }
                if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                    return Err(format!("no data from {} in {} s", name, seconds));
                }
            }
        }
    }
}

/// A connection to a host: one, behind a lock, however many values hold
/// it; closed by `close` or when it is let go.
#[derive(Clone)]
pub struct CrowStream(Arc<StreamInner>);

struct StreamInner {
    conn: Mutex<Option<Conn>>,
    name: String,
    /// The wait `open` gave it, which a read given no seconds goes back to.
    wait: Option<std::time::Duration>,
    /// What a protocol written in Crow notes on the connection, by key: a
    /// query's id and a backend's process and key (what a cancel needs
    /// later), how long the server keeps it, the statements prepared on it.
    notes: Mutex<std::collections::HashMap<String, String>>,
    /// The read timeout last set on the socket, in microseconds (0: none):
    /// a read sets it only when it changes, a system call saved per read.
    timeout_us: std::sync::atomic::AtomicU64,
}

/// A read timeout in microseconds, as `StreamInner::timeout_us` keeps it.
fn micros(t: Option<std::time::Duration>) -> u64 {
    t.map(|d| (d.as_micros() as u64).max(1)).unwrap_or(0)
}

/// The size a thread's read buffer starts at, and the most it grows to.
const SCRATCH_START: usize = 64 * 1024;
const SCRATCH_MOST: usize = 4 * 1024 * 1024;

thread_local! {
    /// A thread's buffer for what a read takes in, copied out at its size.
    static SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl std::fmt::Debug for CrowStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stream {}", self.0.name)
    }
}

impl crate::display::CrowDisplay for CrowStream {
    fn crow_repr(&self) -> String {
        format!("<stream {}>", self.0.name)
    }
}

impl CrowStream {
    /// What a failed `open` holds: a stream that is closed.
    pub fn closed() -> Self {
        CrowStream(Arc::new(StreamInner { conn: Mutex::new(None), name: "closed".to_string(), wait: None, notes: Mutex::new(std::collections::HashMap::new()), timeout_us: std::sync::atomic::AtomicU64::new(0) }))
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
        })
        .clone()
}

/// A wait of `seconds`: none where they are 0 or fewer, or more than a clock
/// holds (`Duration::from_secs_f64` had panicked on `2 ^ 100` seconds, a
/// stop with no line compiled — crow check --self).
fn wait_of(seconds: f64) -> Option<std::time::Duration> {
    if seconds > 0.0 { std::time::Duration::try_from_secs_f64(seconds).ok() } else { None }
}

/// `sys.open[host, port, tls, seconds]`: a connection to the host's port,
/// behind TLS when asked, each read and write giving up after the seconds
/// (none when they are 0), or the reason none could be made.
pub fn open(host: &str, port: i64, tls: bool, seconds: f64) -> Result<CrowStream, String> {
    use std::net::ToSocketAddrs;
    if !(0..=65535).contains(&port) {
        return Err(format!("port {} is outside 0 to 65535", port));
    }
    let timeout = wait_of(seconds);
    let addrs: Vec<std::net::SocketAddr> = (host, port as u16).to_socket_addrs().map_err(|e| format!("cannot resolve {}: {}", host, e))?.collect();
    if addrs.is_empty() {
        return Err(format!("cannot resolve {}", host));
    }
    // a name's addresses in turn (`localhost` is ::1 and 127.0.0.1, and a
    // server may listen on one), the last refusal the reason when none answers
    let mut tcp = None;
    let mut refused = String::new();
    for addr in &addrs {
        let tried = match timeout {
            Some(t) => std::net::TcpStream::connect_timeout(addr, t),
            None => std::net::TcpStream::connect(addr),
        };
        match tried {
            Ok(t) => {
                tcp = Some(t);
                break;
            }
            Err(e) => refused = e.to_string(),
        }
    }
    let Some(tcp) = tcp else { return Err(format!("cannot connect to {}:{}: {}", host, port, refused)) };
    tcp.set_read_timeout(timeout).map_err(|e| e.to_string())?;
    tcp.set_write_timeout(timeout).map_err(|e| e.to_string())?;
    let conn = if tls {
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| format!("{} is no name a certificate can carry", host))?;
        let client = rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("TLS to {}: {}", host, e))?;
        Conn::Net(Net::Tls(Box::new(rustls::StreamOwned::new(client, tcp))))
    } else {
        Conn::plain(tcp)
    };
    Ok(CrowStream(Arc::new(StreamInner { conn: Mutex::new(Some(conn)), name: format!("{}:{}", host, port), wait: timeout, notes: Mutex::new(std::collections::HashMap::new()), timeout_us: std::sync::atomic::AtomicU64::new(micros(timeout)) })))
}

/// `sys.listen[host, port]`: a socket listening on the host's port (port 0
/// for one the system picks; `address` says which), as a stream that is
/// accepted on: `accept` answers each connection that arrives as a stream
/// of its own. The reason where the port cannot be bound.
pub fn listen(host: &str, port: i64) -> Result<CrowStream, String> {
    if !(0..=65535).contains(&port) {
        return Err(format!("port {} is outside 0 to 65535", port));
    }
    let listener = std::net::TcpListener::bind((host, port as u16)).map_err(|e| format!("cannot listen on {}:{}: {}", host, port, e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let name = listener.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| format!("{}:{}", host, port));
    Ok(CrowStream(Arc::new(StreamInner { conn: Mutex::new(Some(Conn::Listener(listener))), name, wait: None, notes: Mutex::new(std::collections::HashMap::new()), timeout_us: std::sync::atomic::AtomicU64::new(0) })))
}

/// `sys.accept[listener, seconds]`: the next connection that arrives, as a
/// stream named by the peer's address, waiting at most the seconds (as
/// long as it takes when 0), or the reason: no connection in time, a
/// stream that is no listener or is closed. The wait ends early with a
/// stop of the work this is in, as a read's does.
pub fn accept(s: &CrowStream, seconds: f64) -> Result<CrowStream, String> {
    let guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let Some(conn) = guard.as_ref() else { return Err("the stream is closed".to_string()) };
    let Conn::Listener(listener) = conn else { return Err(format!("{} is no listener: listen[host, port] makes one", s.0.name)) };
    let deadline = wait_of(seconds).and_then(|w| std::time::Instant::now().checked_add(w));
    loop {
        match listener.accept() {
            Ok((tcp, peer)) => {
                tcp.set_nonblocking(false).map_err(|e| e.to_string())?;
                return Ok(CrowStream(Arc::new(StreamInner { conn: Mutex::new(Some(Conn::plain(tcp))), name: peer.to_string(), wait: None, notes: Mutex::new(std::collections::HashMap::new()), timeout_us: std::sync::atomic::AtomicU64::new(0) })));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if crate::cancel::current_stopped() {
                    return Err("cancelled: nobody waits for this connection any more".to_string());
                }
                if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                    return Err(format!("no connection on {} in {} s", s.0.name, seconds));
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(e) => return Err(format!("cannot accept on {}: {}", s.0.name, e)),
        }
    }
}

/// `sys.address[stream]`: the address a stream is named by — a listener's
/// own (`127.0.0.1:54321`, the port the system picked for port 0), an
/// accepted connection's peer, an opened connection's host and port.
pub fn address(s: &CrowStream) -> String {
    s.0.name.clone()
}

/// `sys.send[stream, bytes]`: the bytes written, all of them, or the reason.
pub fn send(s: &CrowStream, b: &Bytes) -> Result<i64, String> {
    let mut guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let Some(conn) = guard.as_mut() else { return Err("the stream is closed".to_string()) };
    let r = match conn {
        Conn::Net(Net::Plain(t)) => t.get_mut().write_all(&b.0).and_then(|_| t.get_mut().flush()),
        Conn::Net(Net::Tls(t)) => t.write_all(&b.0).and_then(|_| t.flush()),
        // (a process that ended has closed its input: the pipe is broken)
        Conn::Process(p) => match p.stdin.as_mut() {
            Some(w) => match w.write_all(&b.0).and_then(|_| w.flush()) {
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Err("the process ended".to_string()),
                other => other,
            },
            None => return Err("the process's input is closed".to_string()),
        },
        Conn::Listener(_) => return Err(format!("{} is a listener: it is accepted on, not written", s.0.name)),
    };
    r.map_err(|e| format!("cannot send to {}: {}", s.0.name, e))?;
    Ok(b.len())
}

/// `sys.recv[stream, most, seconds]`: up to `most` bytes as they arrive —
/// none when the other side closed — waiting at most the seconds (the
/// connection's own wait when they are 0), or the reason: a wait that
/// passed its seconds, a stream that is closed.
pub fn recv(s: &CrowStream, most: i64, seconds: f64) -> Result<Bytes, String> {
    if most <= 0 {
        return Err(format!("recv[] reads at most a positive number of bytes, got {}", most));
    }
    let mut guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let Some(conn) = guard.as_mut() else { return Err("the stream is closed".to_string()) };
    // (a read given no seconds waits as the connection was opened to,
    // whatever the read before it waited)
    let wait = if seconds > 0.0 { wait_of(seconds) } else { s.0.wait };
    let conn = match conn {
        Conn::Process(p) => return recv_process(p, &s.0.name, most, seconds, wait),
        Conn::Listener(_) => return Err(format!("{} is a listener: it is accepted on, not read", s.0.name)),
        Conn::Net(n) => n,
    };
    // the wait is taken in slices, and ends with a stop: of the work this
    // is in — a derivation's query, whose flag is raised when nobody waits
    // for it any more — or of the program (the driver written in Crow
    // then tells the server; crow_db::driven lets the connection go)
    let deadline = wait.and_then(|w| std::time::Instant::now().checked_add(w));
    // what arrives is read into the thread's own buffer and copied out at
    // its size: a buffer of `most` bytes zeroed for every read (a driver
    // asks for megabytes, and an answer is often a few hundred bytes) had
    // cost more than the read, and stayed in the answer, unused. The buffer
    // grows when a read fills it, up to `most`.
    SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        let want = (most as usize).min(buf.len().max(SCRATCH_START));
        if buf.len() < want {
            buf.resize(want, 0);
        }
        loop {
            let slice = match deadline {
                Some(d) => Some(d.saturating_duration_since(std::time::Instant::now()).min(std::time::Duration::from_millis(100)).max(std::time::Duration::from_millis(1))),
                None => Some(std::time::Duration::from_millis(100)),
            };
            let us = micros(slice);
            if s.0.timeout_us.swap(us, std::sync::atomic::Ordering::Relaxed) != us {
                conn.tcp().set_read_timeout(slice).map_err(|e| e.to_string())?;
            }
            let r = match conn {
                Net::Plain(t) => t.read(&mut buf[..want]),
                Net::Tls(t) => t.read(&mut buf[..want]),
            };
            match r {
                Ok(n) => {
                    let got = Bytes(buf[..n].to_vec());
                    if n == want && want < (most as usize).min(SCRATCH_MOST) {
                        let grown = (want * 2).min(most as usize).min(SCRATCH_MOST);
                        buf.resize(grown, 0);
                    }
                    return Ok(got);
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if crate::cancel::current_stopped() {
                        return Err("cancelled: nobody waits for this query any more".to_string());
                    }
                    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                        return Err(format!("no data from {} in {} s", s.0.name, seconds));
                    }
                }
                Err(e) => return Err(format!("cannot read from {}: {}", s.0.name, e)),
            }
        }
    })
}

/// `sys.note[stream, key, text]`: a note kept on the connection under a
/// key — a query's id, a backend's process and key, a statement prepared
/// on it — read back by `noted`; empty text lets the key go.
pub fn note(s: &CrowStream, key: &str, text: &str) {
    if let Ok(mut n) = s.0.notes.lock() {
        if text.is_empty() {
            n.remove(key);
        } else {
            n.insert(key.to_string(), text.to_string());
        }
    }
}

/// `sys.noted[stream, key]`: the note the connection carries under a key,
/// empty when it carries none.
pub fn noted(s: &CrowStream, key: &str) -> String {
    s.0.notes.lock().ok().and_then(|n| n.get(key).cloned()).unwrap_or_default()
}

/// `sys.machine[]`: this machine's name, as a query's tag carries it:
/// letters, digits, dots and dashes, or `local`.
pub fn machine() -> String {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let raw = std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
        if clean.is_empty() { "local".to_string() } else { clean }
    })
    .clone()
}

/// `sys.process[]`: this process's number.
pub fn process() -> i64 {
    std::process::id() as i64
}

/// `sys.alive[pid]`: whether a process of this machine is running (a
/// number that is none is taken as running: nothing is stopped on a guess).
pub fn alive(pid: i64) -> bool {
    if pid <= 0 {
        return true;
    }
    std::process::Command::new("kill").arg("-0").arg(pid.to_string()).output().map(|o| o.status.success()).unwrap_or(true)
}

/// `sys.xor[a, b]`: the bytes XORed one with the other, of one length (a
/// SCRAM proof: the client key with its signature).
pub fn xor(a: &Bytes, b: &Bytes) -> Result<Bytes, String> {
    if a.0.len() != b.0.len() {
        return Err(format!("xor: the byte strings are of two lengths, {} and {}", a.0.len(), b.0.len()));
    }
    Ok(Bytes(a.0.iter().zip(&b.0).map(|(x, y)| x ^ y).collect()))
}

/// `sys.copy_of_frame[frame]`: the frame's rows in Postgres's COPY text
/// format — a tab between cells, a line per row, `\N` for a missing
/// cell, a backslash, a tab, a line break and a return escaped; a whole
/// float written without its decimals (an integer column takes it), a
/// truth as `t` or `f`, a moment as it is written.
pub fn copy_of_frame(f: &crate::frame::CrowFrame) -> Bytes {
    use crate::frame::Column;
    let mut out = Vec::with_capacity(f.nrows() * f.ncols() * 8);
    for row in 0..f.nrows() {
        for col in 0..f.ncols() {
            if col > 0 {
                out.push(b'\t');
            }
            if !f.validity[col][row] {
                out.extend_from_slice(b"\\N");
                continue;
            }
            let text = match f.data[col].as_ref() {
                Column::Int(v) => v[row].to_string(),
                Column::Float(v) => {
                    let x = v[row];
                    if x.is_finite() && x == x.trunc() && x.abs() < 1e15 { format!("{}", x as i64) } else { x.to_string() }
                }
                Column::Str(v) => v[row].to_string(),
                Column::Bool(v) => if v[row] { "t".to_string() } else { "f".to_string() },
                Column::DateTime(v, z) => crate::frame::format_datetime(v[row], z[row]),
                // (a vector as its JSON list, which Postgres reads as an array's text)
                Column::Vectors(..) => f.data[col].fmt_cell(row, true),
            };
            for c in text.bytes() {
                match c {
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    b'\t' => out.extend_from_slice(b"\\t"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    c => out.push(c),
                }
            }
        }
        out.push(b'\n');
    }
    Bytes(out)
}

/// `sys.rowbinary_of_frame[frame, types]`: a frame as ClickHouse's
/// RowBinary, each column encoded as the table's type for it names
/// (crate::rowbinary).
pub fn rowbinary_of_frame(frame: &crate::frame::CrowFrame, types: Vec<String>) -> Result<Bytes, String> {
    crate::rowbinary::rowbinary_of_frame(frame, &types).map(Bytes)
}

/// `sys.close[stream]`: the connection let go; every call after is refused.
pub fn close(s: &CrowStream) {
    if let Ok(mut guard) = s.0.conn.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod listener_tests {
    use super::*;

    /// A listener on a port the system picks is named by it; a client that
    /// opens it is accepted as a stream the two sides talk over; a listener
    /// is neither read nor written; a wait with nobody arriving ends with
    /// its reason
    #[test]
    fn a_listener_accepts_a_connection() {
        let l = listen("127.0.0.1", 0).expect("a listener");
        let addr = address(&l);
        let port: i64 = addr.rsplit(':').next().unwrap().parse().unwrap();
        assert!(port > 0);
        assert!(accept(&l, 0.05).unwrap_err().contains("no connection on"));
        let client = open("127.0.0.1", port, false, 2.0).expect("a client");
        let server = accept(&l, 2.0).expect("accepted");
        assert!(address(&server).starts_with("127.0.0.1:"));
        assert_eq!(send(&client, &Bytes(b"ping".to_vec())).unwrap(), 4);
        assert_eq!(recv(&server, 100, 2.0).unwrap(), Bytes(b"ping".to_vec()));
        assert_eq!(send(&server, &Bytes(b"pong".to_vec())).unwrap(), 4);
        assert_eq!(recv(&client, 100, 2.0).unwrap(), Bytes(b"pong".to_vec()));
        assert!(send(&l, &Bytes(b"x".to_vec())).unwrap_err().contains("accepted on, not written"));
        assert!(recv(&l, 10, 0.1).unwrap_err().contains("accepted on, not read"));
        assert!(accept(&client, 0.1).unwrap_err().contains("no listener"));
        close(&l);
        assert_eq!(accept(&l, 0.1).unwrap_err(), "the stream is closed");
        assert!(listen("127.0.0.1", 70000).unwrap_err().contains("outside 0 to 65535"));
    }
}

#[cfg(test)]
mod file_lines_tests {
    use super::*;

    /// A file's lines read a block at a time are the lines of its text, as
    /// `io.lines_of` makes them, and the reasons are the reader's
    #[test]
    fn a_files_lines_are_its_texts() {
        let path = std::env::temp_dir().join(format!("crow_file_lines_{}", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let cases: [(&str, &[&str]); 9] = [
            ("a\nb\n", &["a", "b"]),
            ("a\r\nb", &["a", "b"]),
            ("", &[]),
            ("one", &["one"]),
            ("\n\n", &["", ""]),
            ("a\r", &["a"]),
            ("x\r\r\ny", &["x\r", "y"]),
            ("a\rb\n", &["a\rb"]),
            ("über\ndög\r\n", &["über", "dög"]),
        ];
        for (text, lines) in cases {
            std::fs::write(&path, text).unwrap();
            assert_eq!(file_lines(&p).unwrap(), lines.iter().map(|l| l.to_string()).collect::<Vec<_>>(), "{:?}", text);
        }
        std::fs::write(&path, [b'a', b'\n', 0xff, b'\n']).unwrap();
        assert_eq!(file_lines(&p), Err(format!("cannot read '{}': it is not text (UTF-8)", p)));
        let _ = std::fs::remove_file(&path);
        assert!(file_lines(&p).unwrap_err().starts_with(&format!("cannot read '{}': ", p)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_pack_both_ways_and_the_checksum_is_crc32() {
        assert_eq!(pack(258, 2, false).unwrap().0, vec![1, 2]);
        assert_eq!(pack(258, 2, true).unwrap().0, vec![2, 1]);
        assert_eq!(unpack(&Bytes(vec![0xff, 0xfe]), 0, 2, false, true).unwrap(), -2);
        assert_eq!(unpack(&Bytes(vec![0xff, 0xfe]), 0, 2, false, false).unwrap(), 65534);
        assert_eq!(unpack(&Bytes(vec![2, 1, 0, 0]), 0, 4, true, true).unwrap(), 258);
        assert!(unpack(&Bytes(vec![0x80, 0, 0, 0, 0, 0, 0, 0]), 0, 8, false, false).unwrap_err().contains("read them signed"));
        assert_eq!(unpack(&Bytes(vec![0x80, 0, 0, 0, 0, 0, 0, 0]), 0, 8, false, true).unwrap(), i64::MIN);
        assert!(unpack(&Bytes(vec![1]), 0, 2, false, true).unwrap_err().contains("outside"));
        assert_eq!(ieee(1.5, 8, false).unwrap().0, 1.5f64.to_be_bytes().to_vec());
        assert_eq!(ieee(1.5, 4, true).unwrap().0, 1.5f32.to_le_bytes().to_vec());
        assert_eq!(ieee_at(&ieee(2.25, 8, false).unwrap(), 0, 8, false).unwrap(), 2.25);
        assert_eq!(ieee_at(&ieee(2.25, 4, true).unwrap(), 0, 4, true).unwrap(), 2.25);
        assert!(ieee(1.0, 3, false).is_err() && ieee_at(&Bytes(vec![0; 2]), 0, 8, false).is_err());
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(hash(&Bytes(b"123456789".to_vec()), "crc32").unwrap(), "cbf43926");
        assert_eq!(digest(&Bytes(Vec::new()), "crc32").unwrap().0, vec![0, 0, 0, 0]);
    }

    #[test]
    fn a_column_is_its_parts_and_is_made_of_them() {
        use crate::frame::{Column, CrowColumn};
        let col = CrowColumn { data: Arc::new(Column::Int(vec![3, 1, 2])), valid: Arc::new(crate::validity::Validity::from(vec![true, false, true])) };
        assert_eq!(cell_kind(&col), "int");
        assert_eq!(ints_of(&col, 0, 3).unwrap(), vec![3, 0, 2], "a missing cell reads as 0");
        assert_eq!(ints_of(&col, 1, 2).unwrap(), vec![0]);
        assert_eq!(missing_in(&col, 0, 3).unwrap(), 1);
        assert_eq!(present_of(&col, 0, 3).unwrap().0, vec![0b101]);
        assert!(present_of(&col, 0, 1).unwrap().0.is_empty());
        assert!(floats_of(&col, 0, 3).unwrap_err().contains("a float or a vector column, not a column of int"));
        assert!(ints_of(&col, 2, 5).unwrap_err().contains("outside"));
        let back = ints_column(vec![3, 1, 2], &Bytes(vec![0b101])).unwrap();
        assert_eq!(back.valid.to_vec(), vec![true, false, true]);
        assert!(ints_column(vec![1; 9], &Bytes(vec![1])).unwrap_err().contains("need 2 bytes"));
        let mut t = crate::textcol::Text::with_capacity(3, 16);
        for s in ["Apple", "", "cherry"] {
            t.push(s);
        }
        let text = CrowColumn { data: Arc::new(Column::Str(t)), valid: Arc::new(crate::validity::Validity::from(vec![true, false, true])) };
        assert_eq!(text_bytes_of(&text, 0, 3).unwrap().0, b"Applecherry".to_vec());
        assert_eq!(text_ends_of(&text, 0, 3).unwrap(), vec![5, 5, 11]);
        let made = text_column(&Bytes(b"Applecherry".to_vec()), vec![5, 5, 11], &Bytes(vec![0b101])).unwrap();
        if let Column::Str(t) = &*made.data {
            assert_eq!((t.get(0), t.get(1), t.get(2)), ("Apple", "", "cherry"));
        } else {
            panic!("a text column");
        }
        assert!(text_column(&Bytes(vec![0xff]), vec![1], &Bytes(Vec::new())).unwrap_err().contains("UTF-8"));
        assert!(text_column(&Bytes(b"ab".to_vec()), vec![3], &Bytes(Vec::new())).unwrap_err().contains("outside"));
        let bools = bools_column(&Bytes(vec![0b110]), 3, &Bytes(Vec::new())).unwrap();
        assert_eq!(bools_of(&bools, 0, 3).unwrap(), vec![false, true, true]);
        let moments = moments_column(vec![1000, 2000], vec![0, 60], &Bytes(Vec::new())).unwrap();
        assert_eq!(cell_kind(&moments), "moment");
        assert_eq!((millis_of(&moments, 0, 2).unwrap(), zones_of(&moments, 0, 2).unwrap()), (vec![1000, 2000], vec![0, 60]));
        assert!(moments_column(vec![1], vec![100000], &Bytes(Vec::new())).is_err());
        let vectors = vectors_column(2, vec![1.0, 2.0, 3.0, 4.0], &Bytes(Vec::new())).unwrap();
        assert_eq!(vector_width(&vectors).unwrap(), 2);
        assert_eq!(vectors_of(&vectors, 1, 2).unwrap(), vec![3.0, 4.0]);
        assert_eq!(floats_of(&vectors, 0, 2).unwrap().len(), 4);
        assert!(vectors_column(3, vec![1.0, 2.0], &Bytes(Vec::new())).unwrap_err().contains("fill rows"));
    }

    #[test]
    fn a_file_is_read_by_region_flushed_linked_and_moved() {
        let dir = std::env::temp_dir().join(format!("crow-sys-files-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("a.bin").to_string_lossy().to_string();
        store(&path, &Bytes(b"0123456789".to_vec())).unwrap();
        assert_eq!(file_region(&path, 3, 4).unwrap().0, b"3456".to_vec());
        assert!(file_region(&path, 8, 4).unwrap_err().contains("outside"));
        assert_eq!(flush(&path).unwrap(), path);
        assert_eq!(flush(&dir.to_string_lossy()).unwrap(), dir.to_string_lossy().to_string());
        let linked = dir.join("b.bin").to_string_lossy().to_string();
        assert_eq!(link(&path, &linked).unwrap(), linked);
        assert!(link(&path, &linked).is_err(), "a name taken is refused");
        let moved = dir.join("c.bin").to_string_lossy().to_string();
        assert_eq!(move_path(&linked, &moved).unwrap(), moved);
        assert!(!std::path::Path::new(&linked).exists() && std::path::Path::new(&moved).exists());
        assert!(modified(&moved).unwrap().ms > 0);
        assert!(modified(&dir.join("nowhere").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bytes_are_written_in_place_and_refuse_what_no_byte_holds() {
        let b = bytes_zeroed(3).unwrap();
        assert_eq!(b.0, vec![0, 0, 0]);
        let b = set_byte_taken(b, 1, 255).unwrap();
        let b = set_byte_taken(b, -1, 9).unwrap();
        let b = push_byte_taken(b, 7).unwrap();
        assert_eq!(b.0, vec![0, 255, 9, 7]);
        let c = concat_taken(b.clone(), &Bytes(vec![1, 2]));
        assert_eq!(c.0, vec![0, 255, 9, 7, 1, 2]);
        assert_eq!(set_byte(&b, 0, 1).unwrap().0, vec![1, 255, 9, 7]);
        assert_eq!(push_byte(&b, 0).unwrap().0.len(), 5);
        assert!(set_byte(&b, 4, 1).unwrap_err().contains("4"));
        assert_eq!(set_byte(&b, 0, 256).unwrap_err(), "a byte is 0 to 255, got 256");
        assert_eq!(push_byte(&b, -1).unwrap_err(), "a byte is 0 to 255, got -1");
        assert!(bytes_zeroed(-1).unwrap_err().contains("0 or more"));
    }

    /// A wait of more seconds than a clock holds is none, as 0 is: it had
    /// panicked converting `2 ^ 100` seconds (crow check --self).
    #[test]
    fn a_wait_past_what_a_clock_holds_is_none() {
        assert_eq!(super::wait_of(0.0), None);
        assert_eq!(super::wait_of(-1.0), None);
        assert_eq!(super::wait_of(f64::NAN), None);
        assert_eq!(super::wait_of(1.2676506002282294e30), None);
        assert_eq!(super::wait_of(2.5), Some(std::time::Duration::from_millis(2500)));
    }

    /// A plain connection reads ahead: small reads are served from the
    /// buffer, whole and in order, and a read past what arrived waits for
    /// the socket — the bytes a protocol reads a message at a time are the
    /// bytes sent
    #[test]
    fn small_reads_come_from_the_buffer_whole_and_in_order() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        let sent: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let payload = sent.clone();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            std::io::Write::write_all(&mut c, &payload).unwrap();
        });
        let s = super::open("127.0.0.1", port, false, 5.0).unwrap();
        let mut got = Vec::new();
        for _ in 0..1000 {
            let piece = super::recv(&s, 7, 0.0).unwrap();
            assert_eq!(piece.len(), 7, "a small read is served whole");
            got.extend_from_slice(&piece.0);
        }
        while got.len() < sent.len() {
            let piece = super::recv(&s, 1 << 20, 0.0).unwrap();
            assert!(piece.len() > 0);
            got.extend_from_slice(&piece.0);
        }
        assert_eq!(got, sent);
        assert_eq!(super::recv(&s, 10, 0.0).unwrap().len(), 0, "the other side closed");
        server.join().unwrap();
    }

    /// COPY's text form of a frame: tabs, lines, `\N`, the escapes, a
    /// whole float as an integer; and bytes XORed of one length
    #[test]
    fn a_frame_copies_in_as_postgres_reads_it() {
        use crate::frame::{Column, CrowFrame};
        let f = CrowFrame::new(
            vec!["n".into(), "x".into(), "s".into(), "b".into(), "t".into()],
            vec![Column::Int(vec![1, 2]), Column::Float(vec![2.0, 2.5]), Column::strs(vec!["a\tb".into(), "back\\slash".into()]), Column::Bool(vec![true, false]), Column::DateTime(vec![0, 86_400_000], vec![0, 0])],
            vec![vec![true, true], vec![true, false], vec![true, true], vec![true, true], vec![true, true]],
        );
        assert_eq!(String::from_utf8(super::copy_of_frame(&f).0).unwrap(), "1\t2\ta\\tb\tt\t1970-01-01\n2\t\\N\tback\\\\slash\tf\t1970-01-02\n");
        assert_eq!(super::xor(&Bytes(vec![0xf0, 0x0f]), &Bytes(vec![0xff, 0x01])).unwrap(), Bytes(vec![0x0f, 0x0e]));
        assert_eq!(super::xor(&Bytes(vec![1]), &Bytes(vec![1, 2])).unwrap_err(), "xor: the byte strings are of two lengths, 1 and 2");
    }

    /// A name with several addresses connects to the one that answers:
    /// `localhost` is ::1 before 127.0.0.1 on macOS, and a server listening
    /// on 127.0.0.1 alone was refused as "connection refused" by the first
    #[test]
    fn a_name_with_several_addresses_connects_to_the_one_that_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        let accepting = std::thread::spawn(move || listener.accept().map(|_| ()));
        let opened = super::open("localhost", port, false, 2.0);
        assert!(opened.is_ok(), "{:?}", opened.err());
        drop(opened);
        assert!(accepting.join().unwrap().is_ok());
    }

    use super::*;

    /// A table's cells are a matrix, a row each; a cell that is no number
    /// is the reason, naming its column; a header alone is a matrix with
    /// no rows; CSV rows are maps from the column's name to its text.
    #[test]
    fn a_frame_of_numbers_is_a_matrix_and_csv_rows_are_maps() {
        let f = frame_of_csv("a,b\n1,2\n3,4.5\n").unwrap();
        let t = tensor_of_frame(&f).unwrap();
        assert_eq!(t.shape(), &[2, 2]);
        assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0, 4.5]);
        let neg = tensor_of_frame(&frame_of_csv("x\n-1\n-2.5\n").unwrap()).unwrap();
        assert_eq!((neg.shape(), neg.to_vec()), (&[2usize, 1][..], vec![-1.0, -2.5]));
        let text = tensor_of_frame(&frame_of_csv("sym,qty\nAAPL,1\n").unwrap()).unwrap_err();
        assert!(text.contains("sym"), "{}", text);
        let header = tensor_of_frame(&frame_of_csv("a,b\n").unwrap()).unwrap();
        assert_eq!(header.shape(), &[0, 2]);
        let rows = csv_rows("name,age\nalice,30\nbob,25\n").unwrap();
        assert_eq!((rows.len(), rows[1]["age"].as_str()), (2, "25"));
        assert!(csv_rows("").is_err());
    }

    /// A read given no seconds waits as the connection was opened to,
    /// whatever the read before it waited (a timed read had left its wait
    /// on the connection, and a socket's untimed read gave up after it)
    #[test]
    fn a_read_given_no_seconds_waits_as_the_connection_does() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((s, _)) = listener.accept() {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                drop(s);
            }
        });
        let s = open("127.0.0.1", port as i64, false, 0.6).unwrap();
        let quick = std::time::Instant::now();
        assert!(recv(&s, 1, 0.1).unwrap_err().contains("in 0.1 s"));
        assert!(quick.elapsed() < std::time::Duration::from_millis(500));
        let own = std::time::Instant::now();
        assert!(recv(&s, 1, 0.0).unwrap_err().contains("no data"));
        assert!(own.elapsed() >= std::time::Duration::from_millis(500), "waited {:?}", own.elapsed());
    }

    /// A server that echoes what it is sent, once, then closes.
    fn echo_once() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 64];
                if let Ok(n) = s.read(&mut buf) {
                    let _ = s.write_all(&buf[..n]);
                }
            }
        });
        port
    }

    #[test]
    fn bytes_are_text_sliced_found_and_joined() {
        let b = bytes_of_text("héllo");
        assert_eq!(b.len(), 6);
        assert_eq!(text_of_bytes(&b).unwrap(), "héllo");
        assert_eq!(text_of_bytes(&slice(&b, 1, 2).unwrap()).unwrap_err().contains("not UTF-8"), true);
        assert_eq!(find(&b, &bytes_of_text("llo")), 3);
        assert_eq!(find(&b, &bytes_of_text("x")), -1);
        assert_eq!(text_of_bytes(&concat(&slice(&b, 0, 1).unwrap(), &slice(&b, -2, 6).unwrap())).unwrap(), "hlo");
        assert_eq!(byte(&b, -1).unwrap(), b'o' as i64);
        assert!(slice(&b, 2, 1).is_err());
        assert_eq!(format!("{}", b), "<6 bytes>");
    }

    #[test]
    fn a_connection_sends_receives_and_closes() {
        let port = echo_once();
        let s = open("127.0.0.1", port as i64, false, 2.0).unwrap();
        assert_eq!(send(&s, &bytes_of_text("ping")).unwrap(), 4);
        assert_eq!(text_of_bytes(&recv(&s, 64, 2.0).unwrap()).unwrap(), "ping");
        // the server closed: nothing more comes
        assert!(recv(&s, 64, 2.0).unwrap().is_empty());
        close(&s);
        assert!(send(&s, &bytes_of_text("x")).unwrap_err().contains("closed"));
        assert!(open("127.0.0.1", 1, false, 0.2).unwrap_err().contains("cannot connect"));
        assert!(open("127.0.0.1", 70000, false, 0.2).unwrap_err().contains("outside"));
    }

    /// A process started is a socket: what is written to its stream is its
    /// input, what is read its output; a second start of the command while
    /// it runs is the same socket; closed, its input ends it and the next
    /// start is a new process; an empty command is refused
    #[test]
    fn a_process_is_a_socket_written_read_and_closed() {
        let p = start("/bin/cat").unwrap();
        let st = stream_of(&p);
        assert_eq!(format!("{}", p), "<socket /bin/cat>");
        assert_eq!(send(&st, &bytes_of_text("hello\n")).unwrap(), 6);
        assert_eq!(text_of_bytes(&recv(&st, 64, 5.0).unwrap()).unwrap(), "hello\n");
        let again = start("/bin/cat").unwrap();
        assert!(Arc::ptr_eq(&p.0, &again.0), "the running process is answered again");
        // what a read takes past what it asked for stays for the next
        send(&st, &bytes_of_text("abcdef\n")).unwrap();
        assert_eq!(text_of_bytes(&recv(&st, 3, 5.0).unwrap()).unwrap(), "abc");
        assert_eq!(text_of_bytes(&recv(&st, 64, 5.0).unwrap()).unwrap(), "def\n");
        let closing = std::time::Instant::now();
        close(&st);
        assert!(closing.elapsed() < std::time::Duration::from_secs(1), "cat ends on its input's end, at once");
        assert_eq!(send(&st, &bytes_of_text("x")).unwrap_err(), "the stream is closed");
        let fresh = start("/bin/cat").unwrap();
        assert!(!Arc::ptr_eq(&p.0, &fresh.0), "a closed process is started anew");
        assert_eq!(send(&stream_of(&fresh), &bytes_of_text("z\n")).unwrap(), 2);
        assert_eq!(text_of_bytes(&recv(&stream_of(&fresh), 64, 5.0).unwrap()).unwrap(), "z\n");
        close(&stream_of(&fresh));
        assert_eq!(start("  ").unwrap_err(), "a command is text, and '' is none");
    }

    /// A child's group is named after `--`, and a pid naming the caller's
    /// group or every process is signalled not at all: Linux's procps `kill`
    /// had read `-TERM -12345` as a signal to every process of the user
    #[test]
    fn a_group_signal_names_its_group_after_the_options() {
        assert_eq!(group_signal_args(12345, "TERM"), Some(vec!["-TERM".to_string(), "--".to_string(), "-12345".to_string()]));
        assert_eq!(group_signal_args(0, "TERM"), None);
        assert_eq!(group_signal_args(1, "KILL"), None);
    }

    /// A command the shell does not find starts, says so on standard error
    /// and ends: a read answers nothing, a write that the process ended,
    /// and the next start runs it again (a new process)
    #[test]
    fn a_command_that_is_not_there_ends() {
        let p = start("crow-no-such-command-for-the-sys-tests").unwrap();
        let st = stream_of(&p);
        assert!(recv(&st, 64, 5.0).unwrap().is_empty(), "its output closed");
        // (the write may find the pipe already broken, or the shell still
        // reading: either way the next read answers nothing)
        let _ = send(&st, &bytes_of_text("x\n"));
        assert!(recv(&st, 64, 5.0).unwrap().is_empty());
        let again = start("crow-no-such-command-for-the-sys-tests").unwrap();
        assert!(!Arc::ptr_eq(&p.0, &again.0), "an ended process is started anew");
        assert!(recv(&stream_of(&again), 64, 5.0).unwrap().is_empty());
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(send(&stream_of(&again), &bytes_of_text("x\n")).unwrap_err(), "the process ended");
    }

    /// A read waits at most its seconds, and a read given none waits as
    /// long as it takes; a process that ignores its input's end is ended
    /// by its signal, within the shutdown's bounds
    #[test]
    fn a_process_read_waits_its_seconds_and_a_deaf_process_is_ended() {
        // (`cat`, not `/bin/cat`: a test of its own command, though the
        // processes are one per command on each thread and each test runs
        // on its own)
        let p = start("cat").unwrap();
        let st = stream_of(&p);
        let waited = std::time::Instant::now();
        assert_eq!(recv(&st, 64, 0.2).unwrap_err(), "no data from cat in 0.2 s");
        assert!(waited.elapsed() < std::time::Duration::from_millis(900));
        send(&st, &bytes_of_text("late\n")).unwrap();
        assert_eq!(text_of_bytes(&recv(&st, 64, 0.0).unwrap()).unwrap(), "late\n");
        close(&st);
        let deaf = start("sleep 30").unwrap();
        let closing = std::time::Instant::now();
        close(&stream_of(&deaf));
        let took = closing.elapsed();
        assert!(took >= END_ON_EOF && took < std::time::Duration::from_secs(4), "ended by TERM after the wait: {:?}", took);
        assert!(!running(&deaf));
    }

    /// One process per command on each thread: a thread's second start
    /// answers its own process, another thread's start another process, so
    /// two handlers of a served program never read each other's lines
    #[test]
    fn each_thread_talks_to_a_process_of_its_own() {
        // (a shell loop answers each line as it comes: `tr` and its kind hold
        // their output until their input ends, when it is a pipe)
        const TWICE: &str = "while read -r l; do echo \"$l $l\"; done";
        let here = start(TWICE).unwrap();
        assert!(Arc::ptr_eq(&here.0, &start(TWICE).unwrap().0));
        let there = std::thread::spawn(|| {
            let p = start(TWICE).unwrap();
            send(&stream_of(&p), &bytes_of_text("there\n")).unwrap();
            let said = text_of_bytes(&recv(&stream_of(&p), 64, 5.0).unwrap()).unwrap();
            close(&stream_of(&p));
            (p, said)
        });
        send(&stream_of(&here), &bytes_of_text("here\n")).unwrap();
        assert_eq!(text_of_bytes(&recv(&stream_of(&here), 64, 5.0).unwrap()).unwrap(), "here here\n");
        let Ok((p, said)) = there.join() else { panic!("the other thread stopped") };
        assert_eq!(said, "there there\n");
        assert!(!Arc::ptr_eq(&here.0, &p.0), "another thread, another process");
        close(&stream_of(&here));
    }

    /// A long output comes in pieces, every byte once and in order; a
    /// process is no host, so TLS refuses it
    #[test]
    fn a_long_output_comes_in_pieces() {
        let p = start("head -c 300000 /dev/zero | tr '\\0' 'a'").unwrap();
        let st = stream_of(&p);
        let mut total = 0;
        loop {
            let got = recv(&st, 1 << 16, 5.0).unwrap();
            if got.is_empty() {
                break;
            }
            assert!(got.0.iter().all(|b| *b == b'a'));
            total += got.len();
        }
        assert_eq!(total, 300000);
        assert_eq!(secure(&st, "example.com").unwrap_err(), "a process is not put behind TLS: a host is");
        close(&st);
    }
}
