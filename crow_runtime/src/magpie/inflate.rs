//! Inflate (RFC 1951) and the gzip wrapper (RFC 1952), written here so the
//! Kafka door reads a producer's gzip-compressed message set with no
//! library but our own: stored blocks, fixed Huffman blocks and dynamic
//! Huffman blocks, the back-references of a 32 KB window, the gzip
//! header's optional fields skipped and its CRC-32 and length checked.
//! Only decoding: the door answers fetches uncompressed, which the
//! protocol allows.

/// A bit reader over a byte slice, least-significant bit first, as
/// deflate reads its stream.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    buf: u64,
    have: u32,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Bits<'a> {
        Bits { bytes, at: 0, buf: 0, have: 0 }
    }

    fn need(&mut self, n: u32) -> Result<(), String> {
        while self.have < n {
            let b = *self.bytes.get(self.at).ok_or_else(|| "the deflate stream is cut short".to_string())?;
            self.at += 1;
            self.buf |= (b as u64) << self.have;
            self.have += 8;
        }
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.have -= n;
        Ok(v)
    }

    /// To the next byte boundary (a stored block begins there).
    fn align(&mut self) {
        let drop = self.have % 8;
        self.buf >>= drop;
        self.have -= drop;
    }

    /// Whole bytes, after an align: what the bit buffer holds first.
    fn bytes(&mut self, n: usize, out: &mut Vec<u8>) -> Result<(), String> {
        for _ in 0..n {
            out.push(self.bits(8)? as u8);
        }
        Ok(())
    }
}

/// A canonical Huffman code: for each length, how many codes, and the
/// symbols in code order (the decoding of RFC 1951 section 3.2.2).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        // an over-subscribed set of lengths is no code
        let mut left: i32 = 1;
        for len in 1..16 {
            left <<= 1;
            left -= counts[len] as i32;
            if left < 0 {
                return Err("a Huffman code is over-subscribed".to_string());
            }
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= bits.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return self.symbols.get((index + (code - first)) as usize).copied().ok_or_else(|| "a Huffman code names no symbol".to_string());
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("a Huffman code is longer than 15 bits".to_string())
    }
}

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CODE_LENGTH_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// The literal/length and distance codes of a block decoded into `out`.
fn inflate_codes(bits: &mut Bits, lit: &Huffman, dist: &Huffman, out: &mut Vec<u8>) -> Result<(), String> {
    loop {
        let sym = lit.decode(bits)?;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let i = (sym - 257) as usize;
            if i >= 29 {
                return Err("a deflate length code is out of range".to_string());
            }
            let len = LENGTH_BASE[i] as usize + bits.bits(LENGTH_EXTRA[i] as u32)? as usize;
            let d = dist.decode(bits)? as usize;
            if d >= 30 {
                return Err("a deflate distance code is out of range".to_string());
            }
            let distance = DIST_BASE[d] as usize + bits.bits(DIST_EXTRA[d] as u32)? as usize;
            if distance > out.len() {
                return Err("a deflate back-reference reaches before the start".to_string());
            }
            let start = out.len() - distance;
            if distance >= len {
                out.extend_from_within(start..start + len);
            } else {
                for k in 0..len {
                    let b = out[start + k];
                    out.push(b);
                }
            }
        }
    }
}

/// The bytes of a raw deflate stream.
pub fn inflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut bits = Bits::new(input);
    let mut out = Vec::with_capacity(input.len() * 3);
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => {
                bits.align();
                let len = bits.bits(16)? as usize;
                let nlen = bits.bits(16)? as usize;
                if len != (!nlen & 0xffff) {
                    return Err("a stored block's length and its complement disagree".to_string());
                }
                bits.bytes(len, &mut out)?;
            }
            1 => {
                let mut lengths = [0u8; 288];
                for (i, l) in lengths.iter_mut().enumerate() {
                    *l = match i {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let lit = Huffman::new(&lengths)?;
                let dist = Huffman::new(&[5u8; 30])?;
                inflate_codes(&mut bits, &lit, &dist, &mut out)?;
            }
            2 => {
                let nlen = bits.bits(5)? as usize + 257;
                let ndist = bits.bits(5)? as usize + 1;
                let ncode = bits.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("a dynamic block declares too many codes".to_string());
                }
                let mut code_lengths = [0u8; 19];
                for &i in CODE_LENGTH_ORDER.iter().take(ncode) {
                    code_lengths[i] = bits.bits(3)? as u8;
                }
                let code = Huffman::new(&code_lengths)?;
                let mut lengths = vec![0u8; nlen + ndist];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = code.decode(&mut bits)?;
                    match sym {
                        0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            if i == 0 {
                                return Err("a dynamic block repeats a length before any".to_string());
                            }
                            let prev = lengths[i - 1];
                            let n = 3 + bits.bits(2)? as usize;
                            if i + n > lengths.len() {
                                return Err("a dynamic block's lengths overrun".to_string());
                            }
                            lengths[i..i + n].fill(prev);
                            i += n;
                        }
                        17 | 18 => {
                            let n = if sym == 17 { 3 + bits.bits(3)? as usize } else { 11 + bits.bits(7)? as usize };
                            if i + n > lengths.len() {
                                return Err("a dynamic block's lengths overrun".to_string());
                            }
                            i += n;
                        }
                        _ => return Err("a dynamic block's code is out of range".to_string()),
                    }
                }
                if lengths[256] == 0 {
                    return Err("a dynamic block has no end-of-block code".to_string());
                }
                let lit = Huffman::new(&lengths[..nlen])?;
                let dist = Huffman::new(&lengths[nlen..])?;
                inflate_codes(&mut bits, &lit, &dist, &mut out)?;
            }
            _ => return Err("a deflate block is of type 3, which is no type".to_string()),
        }
        if last {
            return Ok(out);
        }
    }
}

/// CRC-32 (IEEE), as gzip checks its data.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The bytes of a gzip member: the header read past its optional fields,
/// the deflate stream inflated, the trailer's CRC-32 and length checked.
pub fn gunzip(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() < 18 || input[0] != 0x1f || input[1] != 0x8b {
        return Err("no gzip header".to_string());
    }
    if input[2] != 8 {
        return Err(format!("gzip method {} is not deflate", input[2]));
    }
    let flags = input[3];
    let mut at = 10usize;
    let cut = || "the gzip header is cut short".to_string();
    if flags & 4 != 0 {
        let xlen = u16::from_le_bytes([*input.get(at).ok_or_else(cut)?, *input.get(at + 1).ok_or_else(cut)?]) as usize;
        at += 2 + xlen;
    }
    if flags & 8 != 0 {
        at += input.get(at..).ok_or_else(cut)?.iter().position(|&b| b == 0).ok_or_else(cut)? + 1;
    }
    if flags & 16 != 0 {
        at += input.get(at..).ok_or_else(cut)?.iter().position(|&b| b == 0).ok_or_else(cut)? + 1;
    }
    if flags & 2 != 0 {
        at += 2;
    }
    if at + 8 > input.len() {
        return Err(cut());
    }
    let body = &input[at..input.len() - 8];
    let out = inflate(body)?;
    let trailer = &input[input.len() - 8..];
    let crc = u32::from_le_bytes(trailer[..4].try_into().unwrap_or([0; 4]));
    let len = u32::from_le_bytes(trailer[4..].try_into().unwrap_or([0; 4]));
    if len != out.len() as u32 {
        return Err(format!("the gzip trailer promises {} bytes and {} were inflated", len, out.len()));
    }
    if crc != crc32(&out) {
        return Err("the gzip checksum does not match".to_string());
    }
    Ok(out)
}

/// The bytes as a gzip member of stored blocks: no compression, a valid
/// stream — what a test writes to drive the reader without a compressor.
pub fn gzip_stored(input: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    let mut chunks = input.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(c) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(last as u8);
        out.extend_from_slice(&(c.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&crc32(input).to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_fixed_and_dynamic_blocks_inflate() {
        assert_eq!(gunzip(&gzip_stored(b"")).unwrap(), b"");
        let text = b"the quick brown fox jumps over the lazy dog".repeat(2000);
        assert_eq!(gunzip(&gzip_stored(&text)).unwrap(), text);
        // zlib.compress(b'hello hello hello hello', 9): a fixed-Huffman block with a back-reference
        let fixed: [u8; 16] = [120, 218, 203, 72, 205, 201, 201, 87, 200, 64, 39, 1, 104, 3, 8, 177];
        assert_eq!(inflate(&fixed[2..fixed.len() - 4]).unwrap(), b"hello hello hello hello");
        // zlib.compress of 400 words drawn from ten, level 9: a dynamic block of 2180 bytes
        let dynamic: [u8; 433] = [
            120, 218, 125, 86, 141, 206, 195, 32, 8, 124, 21, 95, 205, 47, 235, 186, 38, 237, 108, 242, 245, 253, 51, 45, 200, 29, 232, 150, 172, 171, 202, 143, 112, 28, 164, 229, 153, 214, 124, 28, 57, 229, 247, 35, 229, 253, 124, 229, 244, 183, 92, 57, 237, 101, 149, 69, 121, 166, 179, 252, 111, 215, 86, 222, 42, 111, 162, 199, 178, 87, 25, 233, 223, 230, 245, 185, 55, 34, 53, 63, 112, 109,
            158, 72, 45, 120, 183, 45, 206, 205, 156, 111, 109, 142, 37, 242, 235, 181, 220, 58, 178, 179, 192, 205, 190, 201, 161, 237, 101, 182, 16, 215, 53, 89, 159, 253, 183, 224, 68, 253, 200, 235, 185, 45, 150, 99, 181, 214, 19, 211, 211, 125, 149, 180, 40, 196, 74, 226, 32, 140, 134, 72, 97, 165, 171, 38, 240, 216, 57, 112, 197, 99, 233, 165, 84, 163, 73, 69, 205, 71, 213, 149, 95, 140,
            52, 238, 113, 89, 139, 129, 15, 197, 181, 139, 140, 162, 85, 60, 68, 9, 249, 72, 128, 62, 149, 30, 46, 149, 184, 121, 96, 152, 58, 187, 248, 254, 33, 215, 150, 32, 12, 249, 164, 7, 83, 95, 92, 2, 114, 195, 151, 49, 203, 34, 42, 96, 220, 237, 247, 214, 241, 247, 49, 166, 12, 55, 216, 25, 56, 21, 72, 196, 143, 131, 39, 116, 1, 117, 144, 106, 80, 82, 99, 211, 82, 201, 66, 50, 63, 58,
            125, 218, 165, 198, 183, 134, 0, 219, 128, 174, 32, 141, 30, 76, 95, 205, 158, 74, 208, 99, 32, 158, 33, 64, 206, 92, 81, 142, 194, 94, 108, 35, 171, 211, 171, 11, 88, 247, 199, 14, 67, 95, 131, 116, 188, 247, 157, 41, 151, 162, 9, 40, 169, 224, 19, 184, 72, 203, 197, 17, 233, 138, 104, 115, 200, 157, 78, 134, 98, 104, 44, 64, 142, 176, 29, 138, 218, 3, 166, 66, 180, 26, 250, 64, 45,
            136, 221, 177, 197, 195, 233, 100, 222, 161, 60, 152, 189, 140, 153, 205, 36, 132, 55, 182, 31, 208, 236, 5, 29, 177, 237, 18, 16, 83, 108, 39, 211, 184, 171, 225, 90, 191, 2, 153, 230, 105, 50, 34, 252, 31, 239, 193, 112, 238, 145, 48, 235, 184, 73, 34, 183, 145, 108, 64, 137, 167, 4, 36, 128, 219, 229, 232, 54, 112, 73, 62, 130, 75, 16, 220, 227, 60, 48, 40, 210, 199, 211, 57, 124,
            0, 132, 6, 55, 82, 96, 72, 139, 151, 15, 6, 255, 15, 223,
        ];
        assert_eq!((dynamic[2] >> 1) & 3, 2, "a dynamic block");
        let got = inflate(&dynamic[2..dynamic.len() - 4]).unwrap();
        assert_eq!(got.len(), 2180);
        assert_eq!(&got[..13], b"of gamma and ");
        assert_eq!(crc32(&got), 0x29529238);
        // a gzip member carrying a file name in its header
        let named: [u8; 34] = [31, 139, 8, 8, 0, 0, 0, 0, 2, 255, 120, 46, 116, 120, 116, 0, 203, 72, 205, 201, 201, 87, 200, 64, 39, 1, 227, 81, 61, 141, 23, 0, 0, 0];
        assert_eq!(gunzip(&named).unwrap(), b"hello hello hello hello");
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }

    #[test]
    fn a_torn_or_wrong_stream_is_refused() {
        let good = gzip_stored(b"some bytes to keep");
        assert!(gunzip(&good[..good.len() - 3]).is_err());
        let mut bad = good.clone();
        bad[12] ^= 1;
        assert!(gunzip(&bad).is_err());
        assert!(gunzip(b"not gzip at all, not even close to it").is_err());
        assert!(inflate(&[0x07]).is_err());
        assert!(inflate(&[0x03]).is_err());
    }
}
