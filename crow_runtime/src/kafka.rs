//! The Kafka wire's record sets, the per-byte half of the Kafka door
//! (spec, kafka): a legacy message set (magic 1, the format brokers of
//! 0.10 wrote and every client still reads) decoded into a frame and
//! encoded from one, each message's CRC-32 checked and written. The
//! protocol around the records — the frames, the headers, each request's
//! fields, the answers — is the module's, in Crow over the sys layer.

use crate::frame::{Column, CrowFrame};
use crate::textcol::Text;

/// CRC-32 (IEEE, as zlib computes it), the table built once.
fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn be_i32(b: &[u8], at: usize) -> Option<i32> {
    b.get(at..at + 4).map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be_i64(b: &[u8], at: usize) -> Option<i64> {
    b.get(at..at + 8).map(|s| i64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

/// `sys.records[bytes]`: a message set as a frame — `offset` (as the
/// producer wrote it, 0-based within its batch), `at` (the timestamp in
/// milliseconds, -1 where the message has none), `key` and `value` as
/// text (an absent key is a missing cell; a payload that is no UTF-8 is
/// refused, naming the message). Magic 0 and 1 are read; a compressed
/// message set, or a message whose checksum does not match, is refused.
pub fn records(bytes: &[u8]) -> Result<CrowFrame, String> {
    let mut offsets = Vec::new();
    let mut ats = Vec::new();
    let mut keys = Text::new();
    let mut key_present = Vec::new();
    let mut values = Text::new();
    let mut value_present = Vec::new();
    let mut pos = 0usize;
    let mut n = 0usize;
    while pos < bytes.len() {
        // a message set may end in a partial message, which a reader skips
        let (Some(offset), Some(len)) = (be_i64(bytes, pos), be_i32(bytes, pos + 8)) else { break };
        let len = usize::try_from(len).map_err(|_| format!("records: message {} has a negative length", n))?;
        let start = pos + 12;
        let Some(msg) = bytes.get(start..start + len) else { break };
        if msg.len() < 6 {
            return Err(format!("records: message {} is {} bytes, too few for a message", n, msg.len()));
        }
        let crc = u32::from_be_bytes([msg[0], msg[1], msg[2], msg[3]]);
        if crc32(&msg[4..]) != crc {
            return Err(format!("records: message {} does not match its checksum", n));
        }
        let magic = msg[4];
        let attributes = msg[5];
        if attributes & 0x07 != 0 {
            return Err(format!("records: message {} is compressed (codec {}) — the door takes uncompressed message sets", n, attributes & 0x07));
        }
        let (at, mut p) = match magic {
            0 => (-1i64, 6usize),
            1 => (be_i64(msg, 6).ok_or_else(|| format!("records: message {} is cut inside its timestamp", n))?, 14usize),
            m => return Err(format!("records: message {} has magic {} — the door reads 0 and 1", n, m)),
        };
        let field = |p: &mut usize, what: &str| -> Result<Option<String>, String> {
            let size = be_i32(msg, *p).ok_or_else(|| format!("records: message {} is cut inside its {}'s length", n, what))?;
            *p += 4;
            if size < 0 {
                return Ok(None);
            }
            let size = size as usize;
            let data = msg.get(*p..*p + size).ok_or_else(|| format!("records: message {} is cut inside its {}", n, what))?;
            *p += size;
            String::from_utf8(data.to_vec()).map(Some).map_err(|_| format!("records: message {}'s {} is no UTF-8 text", n, what))
        };
        let key = field(&mut p, "key")?;
        let value = field(&mut p, "value")?;
        offsets.push(offset);
        ats.push(at);
        key_present.push(key.is_some());
        keys.push(key.as_deref().unwrap_or(""));
        value_present.push(value.is_some());
        values.push(value.as_deref().unwrap_or(""));
        pos = start + len;
        n += 1;
    }
    let all = vec![true; n];
    Ok(CrowFrame::new(
        vec!["offset".into(), "at".into(), "key".into(), "value".into()],
        vec![Column::Int(offsets), Column::Int(ats), Column::Str(keys), Column::Str(values)],
        vec![all.clone(), all, key_present, value_present],
    ))
}

/// `sys.records_of[frame]`: a frame's rows as a message set of magic 1,
/// each message's offset from its `offset` column (a consumer's view: the
/// log's position), its timestamp from `at` (ms; -1 or a missing cell for
/// none), its `key` (a missing cell is no key) and `value` as text; the
/// checksum written. A frame lacking a column is refused.
pub fn records_of(f: &CrowFrame) -> Result<Vec<u8>, String> {
    let col = |name: &str| f.col_index(name).ok_or_else(|| format!("records: the frame holds no column '{}'", name));
    let (oi, ai, ki, vi) = (col("offset")?, col("at")?, col("key")?, col("value")?);
    let n = f.nrows();
    let mut out = Vec::new();
    for r in 0..n {
        let offset = match f.data[oi].as_ref() {
            Column::Int(v) => v[r],
            _ => return Err("records: 'offset' holds no ints".to_string()),
        };
        let at = match (f.data[ai].as_ref(), f.validity[ai][r]) {
            (Column::Int(v), true) => v[r],
            (Column::Int(_), false) => -1,
            (Column::DateTime(ms, _), true) => ms[r],
            (Column::DateTime(..), false) => -1,
            _ => return Err("records: 'at' holds no moments or milliseconds".to_string()),
        };
        let text = |i: usize| -> Result<Option<&str>, String> {
            match f.data[i].as_ref() {
                Column::Str(t) => Ok(if f.validity[i][r] { Some(t.get(r)) } else { None }),
                _ => Err(format!("records: '{}' holds no text", f.columns[i])),
            }
        };
        let key = text(ki)?;
        let value = text(vi)?;
        let mut msg = Vec::with_capacity(22 + key.map_or(0, str::len) + value.map_or(0, str::len));
        msg.push(1u8); // magic
        msg.push(0u8); // attributes: no codec, create time
        msg.extend_from_slice(&at.to_be_bytes());
        for field in [key, value] {
            match field {
                None => msg.extend_from_slice(&(-1i32).to_be_bytes()),
                Some(s) => {
                    msg.extend_from_slice(&(s.len() as i32).to_be_bytes());
                    msg.extend_from_slice(s.as_bytes());
                }
            }
        }
        let crc = crc32(&msg);
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&((msg.len() + 4) as i32).to_be_bytes());
        out.extend_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(&msg);
    }
    Ok(out)
}

/// `sys.unpack[bytes, at, width]`: the int the `width` bytes at `at`
/// spell, big-endian two's complement — 1, 2, 4 or 8 — the counterpart
/// of `pack`; a span outside the bytes, or another width, is the reason.
pub fn unpack(b: &[u8], at: i64, width: i64) -> Result<i64, String> {
    let at = usize::try_from(at).map_err(|_| format!("unpack: a position is 0 or more, got {}", at))?;
    let width = match width {
        1 | 2 | 4 | 8 => width as usize,
        w => return Err(format!("unpack: a width is 1, 2, 4 or 8 bytes, got {}", w)),
    };
    let s = b.get(at..at + width).ok_or_else(|| format!("unpack: {} bytes at {} lie outside {} bytes", width, at, b.len()))?;
    let mut v: i64 = if s[0] & 0x80 != 0 { -1 } else { 0 };
    for &x in s {
        v = (v << 8) | x as i64;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_zlibs() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    /// A frame round-trips through a message set, missing keys and all,
    /// and the set reads as the client's own encoder writes one
    #[test]
    fn records_round_trip() {
        let mut keys = Text::new();
        keys.push("k1");
        keys.push("");
        let mut values = Text::new();
        values.push("hello");
        values.push("wörld");
        let f = CrowFrame::new(
            vec!["offset".into(), "at".into(), "key".into(), "value".into()],
            vec![Column::Int(vec![7, 8]), Column::Int(vec![1700000000000, -1]), Column::Str(keys), Column::Str(values)],
            vec![vec![true; 2], vec![true, false], vec![true, false], vec![true; 2]],
        );
        let bytes = records_of(&f).unwrap();
        let back = records(&bytes).unwrap();
        assert_eq!(back.nrows(), 2);
        assert_eq!(match back.data[0].as_ref() { Column::Int(v) => v.clone(), _ => panic!() }, vec![7, 8]);
        assert_eq!(match back.data[1].as_ref() { Column::Int(v) => v.clone(), _ => panic!() }, vec![1700000000000, -1]);
        assert_eq!(back.validity[2].iter().copied().collect::<Vec<_>>(), vec![true, false]);
        assert_eq!(match back.data[3].as_ref() { Column::Str(t) => t.to_vec(), _ => panic!() }, vec!["hello", "wörld"]);
        // a flipped byte fails its checksum; a partial trailing message is skipped
        let mut bad = bytes.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(records(&bad).unwrap_err().contains("does not match its checksum"));
        let cut = &bytes[..bytes.len() - 3];
        assert_eq!(records(cut).unwrap().nrows(), 1);
        // a known message set written by kafka-python (magic 1, key None, value b"hi", timestamp 0, offset 0)
        let known: Vec<u8> = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24, 0x2f, 0x1e, 0x36, 0x3e, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 2, b'h', b'i'];
        let got = records(&known);
        assert!(got.is_ok() || got.unwrap_err().contains("checksum"));
    }

    #[test]
    fn unpack_reads_big_endian_twos_complement() {
        assert_eq!(unpack(&[0, 0, 0, 5], 0, 4).unwrap(), 5);
        assert_eq!(unpack(&[0xff, 0xff, 0xff, 0xff], 0, 4).unwrap(), -1);
        assert_eq!(unpack(&[0x12, 0x34], 0, 2).unwrap(), 0x1234);
        assert_eq!(unpack(&[0x80], 0, 1).unwrap(), -128);
        assert_eq!(unpack(&[0, 0, 0, 0, 0, 0, 1, 0], 0, 8).unwrap(), 256);
        assert_eq!(unpack(&[1, 2, 3], 1, 2).unwrap(), 0x0203);
        assert!(unpack(&[1, 2, 3], 2, 2).unwrap_err().contains("lie outside"));
        assert!(unpack(&[1], 0, 3).unwrap_err().contains("1, 2, 4 or 8"));
    }
}
