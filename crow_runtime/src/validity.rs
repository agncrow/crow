//! Which cells of a column are there: one bit per cell (a byte per cell,
//! `Vec<bool>`, had been kept, where the design said a bit). Read like a
//! slice of bools — `valid[i]`, `for &ok in valid.iter()`, `valid.len()`,
//! `valid.get(i)` — so a reader of it needs no new words; written with
//! `set`, or built from bools.

use std::ops::Index;

static TRUE: bool = true;
static FALSE: bool = false;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Validity {
    bits: Vec<u64>,
    len: usize,
}

impl Validity {
    /// Every cell there.
    pub fn all(n: usize) -> Validity {
        let mut v = Validity { bits: vec![u64::MAX; n.div_ceil(64)], len: n };
        v.clear_padding();
        v
    }

    /// No cell there.
    pub fn none(n: usize) -> Validity {
        Validity { bits: vec![0; n.div_ceil(64)], len: n }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether cell `i` is there (false past the end).
    #[inline]
    pub fn at(&self, i: usize) -> bool {
        i < self.len && (self.bits[i >> 6] >> (i & 63)) & 1 == 1
    }

    /// As a slice's `get`: a reference to the bit's value, none past the end.
    #[inline]
    pub fn get(&self, i: usize) -> Option<&bool> {
        if i < self.len { Some(if self.at(i) { &TRUE } else { &FALSE }) } else { None }
    }

    #[inline]
    pub fn set(&mut self, i: usize, present: bool) {
        if i < self.len {
            if present {
                self.bits[i >> 6] |= 1 << (i & 63);
            } else {
                self.bits[i >> 6] &= !(1 << (i & 63));
            }
        }
    }

    pub fn push(&mut self, present: bool) {
        if self.len % 64 == 0 {
            self.bits.push(0);
        }
        self.len += 1;
        self.set(self.len - 1, present);
    }

    pub fn iter(&self) -> impl Iterator<Item = &bool> + '_ {
        (0..self.len).map(move |i| if self.at(i) { &TRUE } else { &FALSE })
    }

    pub fn to_vec(&self) -> Vec<bool> {
        let mut out = Vec::with_capacity(self.len);
        for (w, &word) in self.bits.iter().enumerate() {
            let take = (self.len - w * 64).min(64);
            out.extend((0..take).map(|b| (word >> b) & 1 == 1));
        }
        out
    }

    /// A bitmap from bools, a word at a time.
    fn of_bools<I: Iterator<Item = bool>>(iter: I, hint: usize) -> Validity {
        let mut bits = Vec::with_capacity(hint.div_ceil(64));
        let mut word = 0u64;
        let mut len = 0usize;
        for b in iter {
            if b {
                word |= 1 << (len & 63);
            }
            len += 1;
            if len & 63 == 0 {
                bits.push(word);
                word = 0;
            }
        }
        if len & 63 != 0 {
            bits.push(word);
        }
        Validity { bits, len }
    }

    /// How many cells are there.
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether every cell is there.
    pub fn all_present(&self) -> bool {
        self.count() == self.len
    }

    /// Cells `start..end`, as bools.
    pub fn slice(&self, start: usize, end: usize) -> Vec<bool> {
        (start..end).map(|i| self.at(i)).collect()
    }

    /// Cells `start..end`, as a bitmap.
    pub fn sliced(&self, start: usize, end: usize) -> Validity {
        if self.all_present() {
            return Validity::all(end - start);
        }
        Validity::of_bools((start..end).map(|i| self.at(i)), end - start)
    }

    /// The named cells' presence, in that order.
    pub fn gather(&self, rows: &[usize]) -> Validity {
        if self.all_present() {
            return Validity::all(rows.len());
        }
        Validity::of_bools(rows.iter().map(|&r| self.at(r)), rows.len())
    }

    /// The named cells' presence, a missing cell where no row is named.
    pub fn gather_some(&self, rows: &[Option<usize>]) -> Validity {
        Validity::of_bools(rows.iter().map(|r| r.is_some_and(|r| self.at(r))), rows.len())
    }

    /// The cells the mask keeps.
    pub fn filtered(&self, mask: &[bool]) -> Validity {
        let kept = mask.iter().filter(|&&m| m).count();
        if self.all_present() {
            return Validity::all(kept);
        }
        Validity::of_bools(mask.iter().enumerate().filter(|&(_, &m)| m).map(|(i, _)| self.at(i)), kept)
    }

    /// These cells then another column's.
    pub fn concat(&self, other: &Validity) -> Validity {
        Validity::of_bools(self.iter().chain(other.iter()).copied(), self.len + other.len)
    }

    fn clear_padding(&mut self) {
        let used = self.len % 64;
        if used != 0 {
            if let Some(last) = self.bits.last_mut() {
                *last &= (1u64 << used) - 1;
            }
        }
    }
}

impl Index<usize> for Validity {
    type Output = bool;
    fn index(&self, i: usize) -> &bool {
        if self.at(i) { &TRUE } else { &FALSE }
    }
}

impl<'a> IntoIterator for &'a Validity {
    type Item = &'a bool;
    type IntoIter = Box<dyn Iterator<Item = &'a bool> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

impl PartialEq for Validity {
    fn eq(&self, other: &Validity) -> bool {
        self.len == other.len && self.bits == other.bits
    }
}

impl FromIterator<bool> for Validity {
    fn from_iter<I: IntoIterator<Item = bool>>(iter: I) -> Validity {
        let iter = iter.into_iter();
        let hint = iter.size_hint().0;
        Validity::of_bools(iter, hint)
    }
}

impl From<Vec<bool>> for Validity {
    fn from(v: Vec<bool>) -> Validity {
        Validity::of_bools(v.iter().copied(), v.len())
    }
}

impl From<&[bool]> for Validity {
    fn from(v: &[bool]) -> Validity {
        Validity::of_bools(v.iter().copied(), v.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bitmap_reads_as_bools() {
        let v: Validity = vec![true, false, true, true].into();
        assert_eq!(v.len(), 4);
        assert!(v[0] && !v[1] && v[2] && v[3] && !v[4]);
        assert_eq!(v.get(1).copied(), Some(false));
        assert_eq!(v.get(4), None);
        assert_eq!(v.to_vec(), vec![true, false, true, true]);
        assert_eq!(v.iter().filter(|&&ok| ok).count(), 3);
        assert_eq!(v.count(), 3);
        assert_eq!(v.gather(&[3, 1]).to_vec(), vec![true, false]);
        assert_eq!(v.gather_some(&[None, Some(0)]).to_vec(), vec![false, true]);
        assert_eq!(v.filtered(&[false, true, true, false]).to_vec(), vec![false, true]);
        assert_eq!(v.slice(1, 3), vec![false, true]);
        assert_eq!(v.concat(&v).len(), 8);
        let mut w = Validity::all(70);
        assert!(w.all_present() && w[69]);
        w.set(69, false);
        assert!(!w[69] && w.count() == 69);
        w.set(69, true);
        assert_eq!(w, Validity::all(70));
        assert_eq!(Validity::none(3).count(), 0);
        let mut grown = Validity::none(0);
        for i in 0..130 {
            grown.push(i % 3 == 0);
        }
        assert_eq!(grown.len(), 130);
        assert_eq!(grown.count(), 44);
    }
}
