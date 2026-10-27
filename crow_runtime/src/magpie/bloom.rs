//! A bloom filter over byte strings: `k` hashes into `m` bits, the two
//! hashes FNV-1a and a mix of it (Kirsch and Mitzenmacher's double
//! hashing), so a filter written today reads the same tomorrow — no
//! hasher whose keys the standard library may change. Written into a
//! file as its hash count, its bit count and its bits.

/// A filter's bits per item (about 2% false positives at six hashes).
const BITS_PER_ITEM: usize = 8;
const HASHES: u32 = 6;

pub struct Bloom {
    bits: Vec<u8>,
    m: u64,
    k: u32,
}

impl Bloom {
    pub fn sized(items: usize) -> Bloom {
        let m = (items.max(8) * BITS_PER_ITEM).next_multiple_of(8) as u64;
        Bloom { bits: vec![0; (m / 8) as usize], m, k: HASHES }
    }

    fn hashes(b: &[u8]) -> (u64, u64) {
        let mut h: u64 = 0xcbf29ce484222325;
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        // murmur's fmix64, so the second hash is independent of the first
        let mut g = h;
        g ^= g >> 33;
        g = g.wrapping_mul(0xff51afd7ed558ccd);
        g ^= g >> 33;
        g = g.wrapping_mul(0xc4ceb9fe1a85ec53);
        g ^= g >> 33;
        (h, g | 1)
    }

    pub fn add(&mut self, b: &[u8]) {
        let (h1, h2) = Self::hashes(b);
        for i in 0..self.k as u64 {
            let at = h1.wrapping_add(i.wrapping_mul(h2)) % self.m;
            self.bits[(at / 8) as usize] |= 1 << (at % 8);
        }
    }

    pub fn has(&self, b: &[u8]) -> bool {
        let (h1, h2) = Self::hashes(b);
        (0..self.k as u64).all(|i| {
            let at = h1.wrapping_add(i.wrapping_mul(h2)) % self.m;
            self.bits[(at / 8) as usize] & (1 << (at % 8)) != 0
        })
    }

    /// The filter as bytes: its hash count, its bit count and its bits.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.bits.len());
        out.push(self.k as u8);
        out.extend_from_slice(&self.m.to_le_bytes());
        out.extend_from_slice(&self.bits);
        out
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Bloom, String> {
        if raw.len() < 9 {
            return Err("a filter needs its hash count and its size".to_string());
        }
        let k = raw[0] as u32;
        let m = u64::from_le_bytes(raw[1..9].try_into().map_err(|_| "a filter's size is eight bytes".to_string())?);
        let bits = raw[9..].to_vec();
        if m == 0 || k == 0 || bits.len() as u64 * 8 < m {
            return Err(format!("a filter of {} bits cannot lie in {} bytes", m, bits.len()));
        }
        Ok(Bloom { bits, m, k })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_admits_what_it_holds_and_little_else() {
        let mut b = Bloom::sized(1000);
        for i in 0..1000u32 {
            b.add(&i.to_le_bytes());
        }
        let back = Bloom::from_bytes(&b.to_bytes()).unwrap();
        assert!((0..1000u32).all(|i| back.has(&i.to_le_bytes())));
        let false_positives = (1000..11000u32).filter(|i| back.has(&i.to_le_bytes())).count();
        assert!(false_positives < 500, "{} false positives in 10,000", false_positives);
        assert!(Bloom::from_bytes(&[1, 2]).is_err());
        assert!(Bloom::from_bytes(&[6, 64, 0, 0, 0, 0, 0, 0, 0, 1]).is_err());
    }
}
