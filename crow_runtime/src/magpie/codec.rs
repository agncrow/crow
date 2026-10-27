//! Magpie's codec: an LZ77 byte codec of our own, applied to a chunk's
//! payload where it shrinks it. A stream is a run of tokens, each a
//! literal run followed by a match (an offset back into what was written
//! and a length), the last token literals alone:
//!
//! ```text
//! token byte: high nibble = literal length, low nibble = match length - 4
//!   (a nibble of 15 continues in bytes of 255 and a final byte less)
//! literal bytes
//! offset: u16 little-endian, 1..=65535 bytes back (absent after the last token)
//! match length extension bytes (where the nibble was 15)
//! ```
//!
//! Matches are at least four bytes, found through a hash of four bytes
//! into a table of the last position seen; the encoder is greedy. The
//! decoder is the loop every LZ decoder is, with every offset and length
//! checked against what is there, so a corrupt chunk is refused and
//! never read past its end.

const MIN_MATCH: usize = 4;
const WINDOW: usize = 65535;
const HASH_BITS: u32 = 15;

#[inline]
fn hash4(b: &[u8]) -> usize {
    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    (v.wrapping_mul(2654435761) >> (32 - HASH_BITS)) as usize
}

fn put_len(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

/// The bytes compressed. Short inputs are literals alone.
pub fn compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut out = Vec::with_capacity(n / 2 + 16);
    let mut table = vec![u32::MAX; 1 << HASH_BITS];
    let mut lit_start = 0usize;
    let mut i = 0usize;
    // the last five bytes are never the start of a match: a match needs
    // four bytes, and the final token carries literals only
    let last_match_start = n.saturating_sub(MIN_MATCH + 1);
    while i < last_match_start {
        let h = hash4(&input[i..]);
        let cand = table[h];
        table[h] = i as u32;
        let cand = cand as usize;
        if cand != u32::MAX as usize && i - cand <= WINDOW && input[cand..cand + MIN_MATCH] == input[i..i + MIN_MATCH] {
            // extend the match as far as it goes, leaving the final five bytes as literals
            let limit = n - 5;
            let mut len = MIN_MATCH;
            while i + len < limit && input[cand + len] == input[i + len] {
                len += 1;
            }
            let lits = i - lit_start;
            let lit_nib = lits.min(15) as u8;
            let mat_nib = (len - MIN_MATCH).min(15) as u8;
            out.push((lit_nib << 4) | mat_nib);
            if lits >= 15 {
                put_len(&mut out, lits - 15);
            }
            out.extend_from_slice(&input[lit_start..i]);
            out.extend_from_slice(&((i - cand) as u16).to_le_bytes());
            if len - MIN_MATCH >= 15 {
                put_len(&mut out, len - MIN_MATCH - 15);
            }
            // hash the bytes the match covered, so the next match may start inside it
            let end = i + len;
            let mut j = i + 1;
            while j < end && j < last_match_start {
                table[hash4(&input[j..])] = j as u32;
                j += 1;
            }
            i = end;
            lit_start = i;
        } else {
            i += 1;
        }
    }
    // the final token: literals, no match
    let lits = n - lit_start;
    let lit_nib = lits.min(15) as u8;
    out.push(lit_nib << 4);
    if lits >= 15 {
        put_len(&mut out, lits - 15);
    }
    out.extend_from_slice(&input[lit_start..]);
    out
}

/// The bytes decompressed to exactly `raw_len` bytes; a stream that does
/// not decode to that many, or reaches outside itself, is refused.
pub fn decompress(input: &[u8], raw_len: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(raw_len);
    let mut i = 0usize;
    let cut = || "the codec's stream is cut short".to_string();
    loop {
        let token = *input.get(i).ok_or_else(cut)?;
        i += 1;
        let mut lits = (token >> 4) as usize;
        if lits == 15 {
            loop {
                let b = *input.get(i).ok_or_else(cut)?;
                i += 1;
                lits += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        let lit_end = i + lits;
        if lit_end > input.len() {
            return Err(cut());
        }
        out.extend_from_slice(&input[i..lit_end]);
        i = lit_end;
        if i == input.len() {
            break;
        }
        if i + 2 > input.len() {
            return Err(cut());
        }
        let offset = u16::from_le_bytes([input[i], input[i + 1]]) as usize;
        i += 2;
        if offset == 0 || offset > out.len() {
            return Err(format!("the codec's stream reaches {} bytes back where {} were written", offset, out.len()));
        }
        let mut len = (token & 0x0f) as usize + MIN_MATCH;
        if token & 0x0f == 15 {
            loop {
                let b = *input.get(i).ok_or_else(cut)?;
                i += 1;
                len += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        if out.len() + len > raw_len {
            return Err(format!("the codec's stream decodes past the {} bytes promised", raw_len));
        }
        let start = out.len() - offset;
        if offset >= len {
            out.extend_from_within(start..start + len);
        } else {
            // an overlapping match: a repeat of the last `offset` bytes
            for k in 0..len {
                let b = out[start + k];
                out.push(b);
            }
        }
    }
    if out.len() != raw_len {
        return Err(format!("the codec's stream decodes to {} bytes, and {} were promised", out.len(), raw_len));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trips(input: &[u8]) -> usize {
        let c = compress(input);
        let d = decompress(&c, input.len()).expect("decodes");
        assert_eq!(d, input);
        c.len()
    }

    #[test]
    fn repeats_shrink_and_everything_round_trips() {
        assert_eq!(round_trips(b""), 1);
        assert_eq!(round_trips(b"abc"), 4);
        let zeros = vec![0u8; 100_000];
        assert!(round_trips(&zeros) < 1000);
        let text: Vec<u8> = b"the quick brown fox jumps over the lazy dog; ".iter().cycle().take(50_000).copied().collect();
        assert!(round_trips(&text) < 2000);
        // overlapping matches: a run of one byte, then a run of a two-byte pattern
        let mut mixed = vec![7u8; 300];
        mixed.extend((0..300).map(|i| (i % 2) as u8));
        round_trips(&mixed);
        // incompressible bytes grow by a few per 255 literals at most
        let mut x: u64 = 0x12345678;
        let random: Vec<u8> = (0..65_536)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        let size = round_trips(&random);
        assert!(size <= random.len() + random.len() / 200 + 16, "{}", size);
        // a window's worth and more of distance
        let mut far = random.clone();
        far.extend_from_slice(&random[..1000]);
        round_trips(&far);
    }

    #[test]
    fn a_corrupt_stream_is_refused() {
        let good = compress(b"hello hello hello hello hello");
        assert!(decompress(&good, 5).is_err());
        assert!(decompress(&good, 100).is_err());
        assert!(decompress(&good[..good.len() - 1], 29).is_err());
        let mut bad = good.clone();
        bad[1] = 0xff;
        let _ = decompress(&bad, 29);
        assert!(decompress(&[0x05, b'a'], 5).is_err());
        assert!(decompress(&[0x00, 9, 0], 9).is_err());
    }
}
