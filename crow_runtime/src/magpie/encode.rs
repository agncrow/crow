//! A column chunk: the cells of one column over one row group, encoded by
//! the column's kind and passed through the codec where that shrinks
//! them. The chunk is self-describing past its kind:
//!
//! ```text
//! u8  encoding      u8 flags (bit 0: a validity bitmap follows the header;
//!                             bit 1: the payload is codec-compressed)
//! u32 rows          u32 raw payload length    u32 stored payload length
//! [validity bitmap, ceil(rows / 8) bytes, bit i set where cell i is there]
//! payload
//! ```
//!
//! Encodings: ints by frame of reference (a base and each value's offset
//! from it, bit-packed) or by delta from the cell before (a monotone
//! column — positions, timestamps — packs to a few bits a row), whichever
//! is smaller; floats as their bits; text as bit-packed lengths and bytes,
//! or as a dictionary of distinct values and bit-packed codes where the
//! distinct values are at most half the cells; bools as a bitmap; moments
//! as their milliseconds (an int encoding) and their offsets (frame of
//! reference). A missing cell is written as its neighbour's value, so it
//! widens nothing, and read back as missing by the bitmap.

use super::codec;
use super::pack::{self, Cursor};
use crate::frame::Column;
use crate::textcol::Text;
use crate::validity::Validity;

/// A column's kind as the file writes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Int = 0,
    Float = 1,
    Text = 2,
    Bool = 3,
    Moment = 4,
    /// A vector of one width per row: a float chunk of `rows × width`.
    Vector = 5,
}

impl Kind {
    pub fn of(col: &Column) -> Kind {
        match col {
            Column::Int(_) => Kind::Int,
            Column::Float(_) => Kind::Float,
            Column::Str(_) => Kind::Text,
            Column::Bool(_) => Kind::Bool,
            Column::DateTime(..) => Kind::Moment,
            Column::Vectors(..) => Kind::Vector,
        }
    }

    pub fn from_byte(b: u8) -> Result<Kind, String> {
        Ok(match b {
            0 => Kind::Int,
            1 => Kind::Float,
            2 => Kind::Text,
            3 => Kind::Bool,
            4 => Kind::Moment,
            5 => Kind::Vector,
            other => return Err(format!("no column kind is numbered {}", other)),
        })
    }

    /// The kind's word, as a program reads it (`int`, `float`, `text`,
    /// `bool`, `moment`, `vector`).
    pub fn word(self) -> &'static str {
        match self {
            Kind::Int => "int",
            Kind::Float => "float",
            Kind::Text => "text",
            Kind::Bool => "bool",
            Kind::Moment => "moment",
            Kind::Vector => "vector",
        }
    }

    /// An empty column of the kind.
    pub fn empty_column(self) -> Column {
        match self {
            Kind::Int => Column::Int(Vec::new()),
            Kind::Float => Column::Float(Vec::new()),
            Kind::Text => Column::Str(crate::textcol::Text::new()),
            Kind::Bool => Column::Bool(Vec::new()),
            Kind::Moment => Column::DateTime(Vec::new(), Vec::new()),
            Kind::Vector => Column::Vectors(0, Vec::new()),
        }
    }
}

const ENC_INT_FOR: u8 = 0;
const ENC_INT_DELTA: u8 = 1;
const ENC_FLOAT: u8 = 2;
const ENC_TEXT_PLAIN: u8 = 3;
const ENC_TEXT_DICT: u8 = 4;
const ENC_BOOL: u8 = 5;
const ENC_MOMENT: u8 = 6;
/// A column of vectors: the width as a u32, then every row's floats
/// (64 bits each, or 32 with `narrow`).
const ENC_VECTOR: u8 = 8;
const ENC_VECTOR32: u8 = 9;
/// A float column the log declared narrow: 32 bits on disk, lossy, and
/// read back as 64.
const ENC_FLOAT32: u8 = 7;

const FLAG_VALIDITY: u8 = 1;
const FLAG_CODEC: u8 = 2;

/// Past this many payload bytes the codec is tried.
const CODEC_FROM: usize = 64;

/// The ints with each missing cell replaced by the cell before it (or
/// the first present one), so a placeholder widens no range.
fn filled_ints(vals: &[i64], valid: &[bool]) -> Vec<i64> {
    if valid.iter().all(|&v| v) {
        return vals.to_vec();
    }
    let first = valid.iter().position(|&v| v).map_or(0, |i| vals[i]);
    let mut out = Vec::with_capacity(vals.len());
    let mut last = first;
    for (i, &v) in vals.iter().enumerate() {
        if valid[i] {
            last = v;
        }
        out.push(last);
    }
    out
}

/// An int sequence encoded: the encoding byte and its payload, the
/// smaller of frame-of-reference and delta.
fn encode_ints(vals: &[i64]) -> (u8, Vec<u8>) {
    let n = vals.len();
    if n == 0 {
        return (ENC_INT_FOR, vec![0u8; 9]);
    }
    let base = *vals.iter().min().unwrap_or(&0);
    let top = *vals.iter().max().unwrap_or(&0);
    let for_width = pack::width_of((top as i128 - base as i128) as u64);
    let for_size = 9 + (n * for_width as usize).div_ceil(8);
    let mut delta_choice = None;
    if n > 1 {
        let mut dmin = i128::MAX;
        let mut dmax = i128::MIN;
        for w in vals.windows(2) {
            let d = w[1] as i128 - w[0] as i128;
            dmin = dmin.min(d);
            dmax = dmax.max(d);
        }
        // a delta span past 64 bits is not packed
        if dmax - dmin <= u64::MAX as i128 && dmin >= i64::MIN as i128 && dmin <= i64::MAX as i128 {
            let width = pack::width_of((dmax - dmin) as u64);
            let size = 17 + ((n - 1) * width as usize).div_ceil(8);
            if size < for_size {
                delta_choice = Some((dmin as i64, width));
            }
        }
    }
    if let Some((dmin, width)) = delta_choice {
        let mut out = Vec::with_capacity(17 + ((n - 1) * width as usize).div_ceil(8));
        out.extend_from_slice(&vals[0].to_le_bytes());
        out.extend_from_slice(&dmin.to_le_bytes());
        out.push(width);
        let deltas: Vec<u64> = vals.windows(2).map(|w| (w[1] as i128 - w[0] as i128 - dmin as i128) as u64).collect();
        out.extend(pack::pack(&deltas, width));
        return (ENC_INT_DELTA, out);
    }
    let mut out = Vec::with_capacity(for_size);
    out.extend_from_slice(&base.to_le_bytes());
    out.push(for_width);
    let offsets: Vec<u64> = vals.iter().map(|&v| (v as i128 - base as i128) as u64).collect();
    out.extend(pack::pack(&offsets, for_width));
    (ENC_INT_FOR, out)
}

fn decode_ints(enc: u8, c: &mut Cursor, n: usize) -> Result<Vec<i64>, String> {
    match enc {
        ENC_INT_FOR => {
            let base = c.i64()?;
            let width = c.u8()?;
            let bytes = c.take((n * width as usize).div_ceil(8))?;
            let offsets = pack::unpack(bytes, n, width)?;
            Ok(offsets.iter().map(|&o| (base as i128 + o as i128) as i64).collect())
        }
        ENC_INT_DELTA => {
            let first = c.i64()?;
            let dmin = c.i64()?;
            let width = c.u8()?;
            if n == 0 {
                return Ok(Vec::new());
            }
            let bytes = c.take(((n - 1) * width as usize).div_ceil(8))?;
            let deltas = pack::unpack(bytes, n - 1, width)?;
            let mut out = Vec::with_capacity(n);
            let mut cur = first as i128;
            out.push(first);
            for d in deltas {
                cur += dmin as i128 + d as i128;
                out.push(cur as i64);
            }
            Ok(out)
        }
        other => Err(format!("no int encoding is numbered {}", other)),
    }
}

/// Text cells as bit-packed lengths and their bytes.
fn encode_plain_text(cells: &[&str]) -> Vec<u8> {
    let lens: Vec<u64> = cells.iter().map(|s| s.len() as u64).collect();
    let width = pack::width_of(lens.iter().copied().max().unwrap_or(0));
    let total: usize = cells.iter().map(|s| s.len()).sum();
    let mut out = Vec::with_capacity(9 + lens.len() + total);
    out.push(width);
    out.extend(pack::pack(&lens, width));
    for s in cells {
        out.extend_from_slice(s.as_bytes());
    }
    out
}

fn decode_plain_text(c: &mut Cursor, n: usize) -> Result<Text, String> {
    let width = c.u8()?;
    let bytes = c.take((n * width as usize).div_ceil(8))?;
    let lens = pack::unpack(bytes, n, width)?;
    let total: u64 = lens.iter().sum();
    let data = c.take(total as usize)?;
    let mut offsets = Vec::with_capacity(n + 1);
    let mut at = 0u64;
    offsets.push(0);
    for l in &lens {
        at += l;
        offsets.push(at);
    }
    // the bytes are checked as text once, not per cell
    std::str::from_utf8(data).map_err(|_| "a text chunk holds bytes that are no UTF-8".to_string())?;
    Ok(Text::Plain { offsets, bytes: data.to_vec() })
}

/// The text cells of the rows, with a missing cell as empty text.
fn text_cells<'a>(t: &'a Text, valid: &[bool], start: usize, end: usize) -> Vec<&'a str> {
    (start..end).map(|i| if valid[i - start] { t.get(i) } else { "" }).collect()
}

fn encode_text(t: &Text, valid: &[bool], start: usize, end: usize) -> (u8, Vec<u8>) {
    let n = end - start;
    // a dictionary where the column already keeps one, or where the
    // distinct values are at most half the cells
    let mut dict: Vec<&str> = Vec::new();
    let mut codes: Vec<u64> = Vec::with_capacity(n);
    let mut seen: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
    let limit = (n / 2).max(1);
    let mut dictionary = true;
    for i in start..end {
        let s = if valid[i - start] { t.get(i) } else { "" };
        let code = match seen.get(s) {
            Some(&c) => c,
            None => {
                if dict.len() >= limit {
                    dictionary = false;
                    break;
                }
                let c = dict.len() as u64;
                dict.push(s);
                seen.insert(s, c);
                c
            }
        };
        codes.push(code);
    }
    if dictionary && n > 1 {
        let mut out = Vec::new();
        out.extend_from_slice(&(dict.len() as u32).to_le_bytes());
        out.extend(encode_plain_text(&dict));
        let width = pack::width_of(dict.len().saturating_sub(1) as u64);
        out.push(width);
        out.extend(pack::pack(&codes, width));
        return (ENC_TEXT_DICT, out);
    }
    (ENC_TEXT_PLAIN, encode_plain_text(&text_cells(t, valid, start, end)))
}

fn decode_text(enc: u8, c: &mut Cursor, n: usize) -> Result<Text, String> {
    match enc {
        ENC_TEXT_PLAIN => decode_plain_text(c, n),
        ENC_TEXT_DICT => {
            let nd = c.u32()? as usize;
            let values = decode_plain_text(c, nd)?;
            let width = c.u8()?;
            let bytes = c.take((n * width as usize).div_ceil(8))?;
            let codes = pack::unpack(bytes, n, width)?;
            if codes.iter().any(|&k| k as usize >= nd) {
                return Err("a text chunk's code names no value of its dictionary".to_string());
            }
            Ok(Text::Dict { codes: codes.into_iter().map(|k| k as u32).collect(), values: Box::new(values) })
        }
        other => Err(format!("no text encoding is numbered {}", other)),
    }
}

/// The chunk of rows `start..end` of the column, as the file's bytes; a
/// float column with `narrow` is written in 32 bits.
pub fn encode(col: &Column, valid: &Validity, start: usize, end: usize, narrow: bool) -> Vec<u8> {
    let n = end - start;
    let present: Vec<bool> = valid.slice(start, end);
    let all_present = present.iter().all(|&v| v);
    let (enc, payload): (u8, Vec<u8>) = match col {
        Column::Int(v) => encode_ints(&filled_ints(&v[start..end], &present)),
        Column::Float(v) if narrow => {
            let mut out = Vec::with_capacity(n * 4);
            for i in start..end {
                let x = if present[i - start] { v[i] as f32 } else { 0.0 };
                out.extend_from_slice(&x.to_bits().to_le_bytes());
            }
            (ENC_FLOAT32, out)
        }
        Column::Float(v) => {
            let mut out = Vec::with_capacity(n * 8);
            for i in start..end {
                let x = if present[i - start] { v[i] } else { 0.0 };
                out.extend_from_slice(&x.to_bits().to_le_bytes());
            }
            (ENC_FLOAT, out)
        }
        Column::Str(t) => encode_text(t, &present, start, end),
        Column::Bool(v) => {
            let bits: Validity = (start..end).map(|i| present[i - start] && v[i]).collect();
            (ENC_BOOL, bitmap_bytes(&bits))
        }
        Column::DateTime(ms, off) => {
            let (ienc, ipay) = encode_ints(&filled_ints(&ms[start..end], &present));
            let offs: Vec<i64> = (start..end).map(|i| off[i] as i64).collect();
            let (oenc, opay) = encode_ints(&offs);
            let mut out = Vec::with_capacity(2 + ipay.len() + opay.len());
            out.push(ienc);
            out.extend(ipay);
            out.push(oenc);
            out.extend(opay);
            (ENC_MOMENT, out)
        }
        Column::Vectors(w, v) => {
            let per = if narrow { 4 } else { 8 };
            let mut out = Vec::with_capacity(4 + n * w * per);
            out.extend_from_slice(&(*w as u32).to_le_bytes());
            for i in start..end {
                for j in 0..*w {
                    let x = if present[i - start] { v[i * w + j] } else { 0.0 };
                    if narrow {
                        out.extend_from_slice(&(x as f32).to_bits().to_le_bytes());
                    } else {
                        out.extend_from_slice(&x.to_bits().to_le_bytes());
                    }
                }
            }
            (if narrow { ENC_VECTOR32 } else { ENC_VECTOR }, out)
        }
    };
    let raw_len = payload.len();
    let (stored, coded) = if raw_len >= CODEC_FROM {
        let c = codec::compress(&payload);
        if c.len() + c.len() / 10 < raw_len { (c, true) } else { (payload, false) }
    } else {
        (payload, false)
    };
    let mut flags = 0u8;
    if !all_present {
        flags |= FLAG_VALIDITY;
    }
    if coded {
        flags |= FLAG_CODEC;
    }
    let mut out = Vec::with_capacity(14 + n.div_ceil(8) + stored.len());
    out.push(enc);
    out.push(flags);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(raw_len as u32).to_le_bytes());
    out.extend_from_slice(&(stored.len() as u32).to_le_bytes());
    if !all_present {
        out.extend(bitmap_bytes(&Validity::from(present)));
    }
    out.extend(stored);
    out
}

/// A bitmap's bytes: bit i of byte i / 8 is cell i.
fn bitmap_bytes(bits: &Validity) -> Vec<u8> {
    let n = bits.len();
    let mut out = vec![0u8; n.div_ceil(8)];
    for (i, b) in bits.iter().enumerate() {
        if *b {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

fn bitmap_of(bytes: &[u8], n: usize) -> Validity {
    (0..n).map(|i| bytes[i / 8] & (1 << (i % 8)) != 0).collect()
}

/// The chunk's header alone: its rows and its stored length, so a reader
/// knows how much to read before decoding.
pub const HEADER: usize = 14;

/// The column and its validity read back from a chunk's bytes.
pub fn decode(bytes: &[u8], kind: Kind) -> Result<(Column, Validity), String> {
    let mut c = Cursor::new(bytes);
    let enc = c.u8()?;
    let flags = c.u8()?;
    let n = c.u32()? as usize;
    let raw_len = c.u32()? as usize;
    let stored_len = c.u32()? as usize;
    let valid = if flags & FLAG_VALIDITY != 0 {
        let b = c.take(n.div_ceil(8))?;
        bitmap_of(b, n)
    } else {
        Validity::all(n)
    };
    let stored = c.take(stored_len)?;
    let owned;
    let payload: &[u8] = if flags & FLAG_CODEC != 0 {
        owned = codec::decompress(stored, raw_len)?;
        &owned
    } else {
        if stored.len() != raw_len {
            return Err(format!("a chunk promises {} bytes and holds {}", raw_len, stored.len()));
        }
        stored
    };
    let mut p = Cursor::new(payload);
    let col = match kind {
        Kind::Int => Column::Int(decode_ints(enc, &mut p, n)?),
        Kind::Float => match enc {
            ENC_FLOAT => {
                let b = p.take(n * 8)?;
                Column::Float(b.chunks_exact(8).map(|x| f64::from_bits(u64::from_le_bytes(x.try_into().unwrap_or([0; 8])))).collect())
            }
            ENC_FLOAT32 => {
                let b = p.take(n * 4)?;
                Column::Float(b.chunks_exact(4).map(|x| f32::from_bits(u32::from_le_bytes(x.try_into().unwrap_or([0; 4]))) as f64).collect())
            }
            other => return Err(format!("a float chunk is not encoding {}", other)),
        },
        Kind::Text => Column::Str(decode_text(enc, &mut p, n)?),
        Kind::Bool => {
            if enc != ENC_BOOL {
                return Err(format!("a bool chunk is not encoding {}", enc));
            }
            let b = p.take(n.div_ceil(8))?;
            Column::Bool(bitmap_of(b, n).to_vec())
        }
        Kind::Moment => {
            if enc != ENC_MOMENT {
                return Err(format!("a moment chunk is not encoding {}", enc));
            }
            let ienc = p.u8()?;
            let ms = decode_ints(ienc, &mut p, n)?;
            let oenc = p.u8()?;
            let offs = decode_ints(oenc, &mut p, n)?;
            Column::DateTime(ms, offs.into_iter().map(|o| o as i16).collect())
        }
        Kind::Vector => {
            if enc != ENC_VECTOR && enc != ENC_VECTOR32 {
                return Err(format!("a vector chunk is not encoding {}", enc));
            }
            let w = u32::from_le_bytes(p.take(4)?.try_into().unwrap_or([0; 4])) as usize;
            let data: Vec<f64> = if enc == ENC_VECTOR {
                let b = p.take(n * w * 8)?;
                b.chunks_exact(8).map(|x| f64::from_bits(u64::from_le_bytes(x.try_into().unwrap_or([0; 8])))).collect()
            } else {
                let b = p.take(n * w * 4)?;
                b.chunks_exact(4).map(|x| f32::from_bits(u32::from_le_bytes(x.try_into().unwrap_or([0; 4]))) as f64).collect()
            };
            Column::Vectors(w, data)
        }
    };
    if col.len() != n {
        return Err(format!("a chunk promises {} rows and decodes {}", n, col.len()));
    }
    Ok((col, valid))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(col: Column, valid: Vec<bool>) {
        let n = col.len();
        let v = Validity::from(valid.clone());
        let bytes = encode(&col, &v, 0, n, false);
        let (back, bv) = decode(&bytes, Kind::of(&col)).expect("decodes");
        assert_eq!(bv.to_vec(), valid);
        for i in 0..n {
            if !valid[i] {
                continue;
            }
            match (&col, &back) {
                (Column::Int(a), Column::Int(b)) => assert_eq!(a[i], b[i]),
                (Column::Float(a), Column::Float(b)) => assert!(a[i].to_bits() == b[i].to_bits()),
                (Column::Str(a), Column::Str(b)) => assert_eq!(a.get(i), b.get(i)),
                (Column::Bool(a), Column::Bool(b)) => assert_eq!(a[i], b[i]),
                (Column::Vectors(wa, a), Column::Vectors(wb, b)) => {
                    assert_eq!(wa, wb);
                    // (a missing row is written as zeros; a NaN by its bits)
                    for j in 0..*wa {
                        let (x, y) = (a[i * wa + j], b[i * wb + j]);
                        assert!(if valid[i] { x.to_bits() == y.to_bits() } else { y == 0.0 }, "row {} number {}: {} vs {}", i, j, x, y);
                    }
                }
                (Column::DateTime(a, ao), Column::DateTime(b, bo)) => {
                    assert_eq!(a[i], b[i]);
                    assert_eq!(ao[i], bo[i]);
                }
                _ => panic!("the kind changed"),
            }
        }
        // a part of the column round-trips too
        if n > 3 {
            let bytes = encode(&col, &v, 1, n - 1, false);
            let (back, _) = decode(&bytes, Kind::of(&col)).expect("decodes");
            assert_eq!(back.len(), n - 2);
        }
    }

    #[test]
    fn every_kind_round_trips_with_missing_cells() {
        round_trip(Column::Int(vec![5, -3, i64::MAX, i64::MIN, 0, 7]), vec![true, false, true, true, true, true]);
        round_trip(Column::Int((0..5000).map(|i| 1_700_000_000_000 + i * 60_000).collect()), vec![true; 5000]);
        round_trip(Column::Int(vec![1, 1, 1, 1]), vec![true; 4]);
        round_trip(Column::Int(vec![]), vec![]);
        round_trip(Column::Float(vec![1.5, f64::NAN, -0.0, f64::INFINITY, 2.0]), vec![true, true, true, false, true]);
        let mut t = Text::new();
        for s in ["a", "bb", "", "héllo", "a", "bb", "a", "a"] {
            t.push(s);
        }
        round_trip(Column::Str(t.clone()), vec![true, true, false, true, true, true, true, true]);
        round_trip(Column::Str(t.coded()), vec![true; 8]);
        let mut distinct = Text::new();
        for i in 0..100 {
            distinct.push(&format!("value number {}", i));
        }
        round_trip(Column::Str(distinct), vec![true; 100]);
        round_trip(Column::Bool(vec![true, false, true, true, false, false, true, true, true]), vec![true, true, false, true, true, true, true, true, true]);
        round_trip(Column::DateTime(vec![1_700_000_000_000, 1_700_000_060_000, 1_700_000_120_000], vec![0, 330, -300]), vec![true, false, true]);
        round_trip(Column::Vectors(3, vec![1.0, 0.5, -2.0, 0.0, 0.0, 0.0, 3.25, f64::NAN, 1e300]), vec![true, false, true]);
        round_trip(Column::Vectors(2, vec![]), vec![]);
    }

    #[test]
    fn a_monotone_column_packs_to_a_few_bits_and_a_dictionary_pays_once() {
        let positions: Vec<i64> = (0..100_000).collect();
        let bytes = encode(&Column::Int(positions), &Validity::all(100_000), 0, 100_000, false);
        assert!(bytes.len() < 200, "{}", bytes.len());
        let mut t = Text::new();
        for i in 0..50_000 {
            t.push(["AAPL", "MSFT", "GOOG"][i % 3]);
        }
        let bytes = encode(&Column::Str(t), &Validity::all(50_000), 0, 50_000, false);
        assert!(bytes.len() < 13_000, "{}", bytes.len());
    }

    #[test]
    fn a_narrow_float_column_keeps_32_bits() {
        let col = Column::Float(vec![1.5, 0.1, -2.0e30, f64::NAN]);
        let bytes = encode(&col, &Validity::all(4), 0, 4, true);
        let (back, _) = decode(&bytes, Kind::Float).unwrap();
        let Column::Float(v) = back else { panic!("floats") };
        assert_eq!(v[0], 1.5);
        assert_eq!(v[1], 0.1f32 as f64);
        assert_eq!(v[2], -2.0e30f32 as f64);
        assert!(v[3].is_nan());
    }

    #[test]
    fn a_corrupt_chunk_is_refused() {
        let bytes = encode(&Column::Int(vec![1, 2, 3]), &Validity::all(3), 0, 3, false);
        assert!(decode(&bytes[..bytes.len() - 1], Kind::Int).is_err());
        assert!(decode(&bytes, Kind::Float).is_err());
        assert!(decode(&[], Kind::Int).is_err());
    }
}
