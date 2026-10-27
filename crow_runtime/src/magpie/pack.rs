//! Bits and varints: the two ways a number is written into a Magpie file.
//! Bit packing writes each of `n` values in `width` bits, little-endian
//! bit order, so a column of small ints or dictionary codes takes the
//! bits its largest value needs and no more; a varint is LEB128, for the
//! footer's counts and lengths.

/// How many bits the value needs: 0 for 0, 64 for u64::MAX.
pub fn width_of(max: u64) -> u8 {
    (64 - max.leading_zeros()) as u8
}

/// The values packed at `width` bits each, little-endian within and
/// across bytes; `width` 0 writes nothing (every value is 0).
pub fn pack(values: &[u64], width: u8) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let bits = values.len() * width as usize;
    let mut out = Vec::with_capacity(bits.div_ceil(8));
    let mut buf: u128 = 0;
    let mut have: u32 = 0;
    for &v in values {
        buf |= (v as u128) << have;
        have += width as u32;
        while have >= 8 {
            out.push(buf as u8);
            buf >>= 8;
            have -= 8;
        }
    }
    if have > 0 {
        out.push(buf as u8);
    }
    out
}

/// `n` values of `width` bits each read back from `bytes`; refused where
/// the bytes are too few.
pub fn unpack(bytes: &[u8], n: usize, width: u8) -> Result<Vec<u64>, String> {
    if width == 0 {
        return Ok(vec![0; n]);
    }
    if width > 64 {
        return Err(format!("a packed value is at most 64 bits wide, not {}", width));
    }
    let bits = n * width as usize;
    if bytes.len() * 8 < bits {
        return Err(format!("{} values of {} bits need {} bytes, and {} are there", n, width, bits.div_ceil(8), bytes.len()));
    }
    let mask: u64 = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
    let mut out = Vec::with_capacity(n);
    let mut buf: u128 = 0;
    let mut have: u32 = 0;
    let mut at = 0usize;
    for _ in 0..n {
        while have < width as u32 {
            buf |= (bytes[at] as u128) << have;
            at += 1;
            have += 8;
        }
        out.push((buf as u64) & mask);
        buf >>= width;
        have -= width as u32;
    }
    Ok(out)
}

/// A varint (LEB128) appended.
pub fn put_var(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A reader over a byte slice: varints, fixed-width numbers and strings,
/// each refused with its reason where the bytes run out.
pub struct Cursor<'a> {
    bytes: &'a [u8],
    pub at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Cursor<'a> {
        Cursor { bytes, at: 0 }
    }

    pub fn rest(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.at + n > self.bytes.len() {
            return Err(format!("the file ends {} bytes early", self.at + n - self.bytes.len()));
        }
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    pub fn var(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *self.bytes.get(self.at).ok_or_else(|| "a count is cut short".to_string())?;
            self.at += 1;
            if shift >= 64 {
                return Err("a count is longer than 64 bits".to_string());
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap_or([0; 2])))
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap_or([0; 4])))
    }

    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap_or([0; 8])))
    }

    pub fn i64(&mut self) -> Result<i64, String> {
        Ok(self.u64()? as i64)
    }

    pub fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// A string written as its varint length and its UTF-8.
    pub fn string(&mut self) -> Result<String, String> {
        let n = self.var()? as usize;
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| "a name is no UTF-8".to_string())
    }

    /// Bytes written as their varint length and themselves.
    pub fn blob(&mut self) -> Result<&'a [u8], String> {
        let n = self.var()? as usize;
        self.take(n)
    }
}

/// A string appended as its varint length and its UTF-8.
pub fn put_string(out: &mut Vec<u8>, s: &str) {
    put_var(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// Bytes appended as their varint length and themselves.
pub fn put_blob(out: &mut Vec<u8>, b: &[u8]) {
    put_var(out, b.len() as u64);
    out.extend_from_slice(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_at_every_width() {
        for width in 0..=64u8 {
            let max = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
            let values: Vec<u64> = (0..37u64).map(|i| (i.wrapping_mul(0x9e3779b97f4a7c15)) & max).collect();
            let packed = pack(&values, width);
            assert_eq!(packed.len(), (37 * width as usize).div_ceil(8));
            assert_eq!(unpack(&packed, 37, width).unwrap(), values, "width {}", width);
        }
        assert_eq!(width_of(0), 0);
        assert_eq!(width_of(1), 1);
        assert_eq!(width_of(255), 8);
        assert_eq!(width_of(256), 9);
        assert_eq!(width_of(u64::MAX), 64);
        assert!(unpack(&[1, 2], 5, 5).is_err());
    }

    #[test]
    fn varints_and_strings_round_trip() {
        let mut out = Vec::new();
        for v in [0u64, 1, 127, 128, 300, 1 << 40, u64::MAX] {
            put_var(&mut out, v);
        }
        put_string(&mut out, "héllo");
        put_blob(&mut out, &[1, 2, 3]);
        let mut c = Cursor::new(&out);
        for v in [0u64, 1, 127, 128, 300, 1 << 40, u64::MAX] {
            assert_eq!(c.var().unwrap(), v);
        }
        assert_eq!(c.string().unwrap(), "héllo");
        assert_eq!(c.blob().unwrap(), &[1, 2, 3]);
        assert_eq!(c.rest(), 0);
        assert!(c.u8().is_err());
    }
}
