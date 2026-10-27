//! Snappy's raw block format, decoded here so the Kafka door reads a
//! producer's snappy-compressed message set with no library but our own:
//! a varint of the uncompressed length, then elements — a literal run, or
//! a copy of what was written from an offset back, in three widths. Only
//! decoding, as the door needs; the literal encoder below writes a valid
//! block of literals alone, for the tests.

/// The bytes of a snappy block.
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let cut = || "the snappy block is cut short".to_string();
    // the uncompressed length, a varint
    let mut len: usize = 0;
    let mut shift = 0u32;
    let mut i = 0usize;
    loop {
        let b = *input.get(i).ok_or_else(cut)?;
        i += 1;
        if shift >= 32 {
            return Err("the snappy block's length is longer than 32 bits".to_string());
        }
        len |= ((b & 0x7f) as usize) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let mut out: Vec<u8> = Vec::with_capacity(len);
    while i < input.len() {
        let tag = input[i];
        i += 1;
        match tag & 3 {
            0 => {
                let mut n = (tag >> 2) as usize;
                if n >= 60 {
                    let extra = n - 59;
                    let bytes = input.get(i..i + extra).ok_or_else(cut)?;
                    n = 0;
                    for (k, &b) in bytes.iter().enumerate() {
                        n |= (b as usize) << (8 * k);
                    }
                    i += extra;
                }
                let n = n + 1;
                let lit = input.get(i..i + n).ok_or_else(cut)?;
                out.extend_from_slice(lit);
                i += n;
            }
            kind => {
                let (length, offset) = match kind {
                    1 => {
                        let b = *input.get(i).ok_or_else(cut)?;
                        i += 1;
                        (4 + ((tag >> 2) & 7) as usize, (((tag >> 5) as usize) << 8) | b as usize)
                    }
                    2 => {
                        let b = input.get(i..i + 2).ok_or_else(cut)?;
                        i += 2;
                        ((tag >> 2) as usize + 1, u16::from_le_bytes([b[0], b[1]]) as usize)
                    }
                    _ => {
                        let b = input.get(i..i + 4).ok_or_else(cut)?;
                        i += 4;
                        ((tag >> 2) as usize + 1, u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                    }
                };
                if offset == 0 || offset > out.len() {
                    return Err(format!("a snappy copy reaches {} bytes back where {} were written", offset, out.len()));
                }
                let start = out.len() - offset;
                if offset >= length {
                    out.extend_from_within(start..start + length);
                } else {
                    for k in 0..length {
                        let b = out[start + k];
                        out.push(b);
                    }
                }
            }
        }
        if out.len() > len {
            return Err(format!("the snappy block decodes past the {} bytes promised", len));
        }
    }
    if out.len() != len {
        return Err(format!("the snappy block decodes to {} bytes, and {} were promised", out.len(), len));
    }
    Ok(out)
}

/// The bytes as a snappy block of literals alone: a valid block, no
/// compression — what a test writes to drive the reader.
pub fn literal(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + 8);
    let mut n = input.len();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
    for chunk in input.chunks(1 << 16) {
        let n = chunk.len() - 1;
        if n < 60 {
            out.push((n as u8) << 2);
        } else {
            out.push(61 << 2);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        out.extend_from_slice(chunk);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_copies_decode() {
        assert_eq!(decompress(&literal(b"")).unwrap(), b"");
        let text = b"snappy snappy snappy".repeat(5000);
        assert_eq!(decompress(&literal(&text)).unwrap(), text);
        // "abcd" then a one-byte-offset copy of four bytes from four back
        assert_eq!(decompress(&[8, 0x0c, b'a', b'b', b'c', b'd', 0x01, 0x04]).unwrap(), b"abcdabcd");
        // an overlapping copy: "ab" then six bytes from two back; a two-byte offset; a four-byte offset
        assert_eq!(decompress(&[8, 0x04, b'a', b'b', 0x02 | (5 << 2), 2, 0]).unwrap(), b"abababab");
        assert_eq!(decompress(&[6, 0x04, b'x', b'y', 0x03 | (3 << 2), 2, 0, 0, 0]).unwrap(), b"xyxyxy");
        assert!(decompress(&[8, 0x0c, b'a', b'b', b'c', b'd', 0x01, 0x09]).is_err());
        assert!(decompress(&[9, 0x0c, b'a', b'b', b'c', b'd', 0x01, 0x04]).is_err());
        assert!(decompress(&[8, 0x0c, b'a', b'b']).is_err());
    }
}
