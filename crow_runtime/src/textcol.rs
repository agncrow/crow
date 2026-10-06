//! A column of text, held as Arrow holds one: every cell's bytes in one
//! buffer with an offset per cell, so a column is two allocations however
//! many rows it has — or, for a column of few distinct values (a symbol,
//! a side), a code per cell into a dictionary of the values, four bytes a
//! cell, which a group, a join, a sort or a filter can work on without
//! reading the text. A cell is read by index as `&str`. (A `String` per
//! cell had cost 24 bytes and an allocation each: 160 of a benchmark row's
//! 220 bytes, and 40 million allocations to sort ten million rows.)

use std::collections::HashMap;
use std::ops::Index;

/// How many distinct values a dictionary holds at most; a column with
/// more is held plain.
pub const DICT_MAX: usize = 1 << 16;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Text {
    /// `offsets[i]..offsets[i + 1]` of `bytes` is cell `i`.
    Plain { offsets: Vec<u64>, bytes: Vec<u8> },
    /// Cell `i` is `values[codes[i]]`; the values are distinct, in the
    /// order first seen.
    Dict { codes: Vec<u32>, values: Box<Text> },
}

impl Default for Text {
    fn default() -> Self {
        Text::new()
    }
}

impl Text {
    pub fn new() -> Text {
        Text::Plain { offsets: vec![0], bytes: Vec::new() }
    }

    pub fn with_capacity(cells: usize, bytes: usize) -> Text {
        let mut offsets = Vec::with_capacity(cells + 1);
        offsets.push(0);
        Text::Plain { offsets, bytes: Vec::with_capacity(bytes) }
    }

    /// The same value in every cell: one dictionary entry.
    pub fn repeated(value: &str, n: usize) -> Text {
        let mut values = Text::with_capacity(1, value.len());
        values.push(value);
        Text::Dict { codes: vec![0; n], values: Box::new(values) }
    }

    pub fn len(&self) -> usize {
        match self {
            Text::Plain { offsets, .. } => offsets.len() - 1,
            Text::Dict { codes, .. } => codes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Cell `i`.
    pub fn get(&self, i: usize) -> &str {
        match self {
            Text::Plain { offsets, bytes } => {
                let (a, b) = (offsets[i] as usize, offsets[i + 1] as usize);
                std::str::from_utf8(&bytes[a..b]).unwrap_or("")
            }
            Text::Dict { codes, values } => values.get(codes[i] as usize),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).map(move |i| self.get(i))
    }

    /// A cell appended to a plain column (a dictionary column is built by
    /// `DictBuilder`; appended to, it is held plain first).
    pub fn push(&mut self, s: &str) {
        if let Text::Dict { .. } = self {
            *self = self.plain();
        }
        if let Text::Plain { offsets, bytes } = self {
            bytes.extend_from_slice(s.as_bytes());
            offsets.push(bytes.len() as u64);
        }
    }

    /// The column held plain, its cells as they are.
    pub fn plain(&self) -> Text {
        match self {
            Text::Plain { .. } => self.clone(),
            Text::Dict { codes, values } => {
                let total: usize = codes.iter().map(|&c| values.get(c as usize).len()).sum();
                let mut out = Text::with_capacity(codes.len(), total);
                for &c in codes {
                    out.push(values.get(c as usize));
                }
                out
            }
        }
    }

    /// The codes and the dictionary of a column held by one.
    pub fn dictionary(&self) -> Option<(&[u32], &Text)> {
        match self {
            Text::Dict { codes, values } => Some((codes, values)),
            Text::Plain { .. } => None,
        }
    }

    /// The column dictionary-coded, where it has at most `DICT_MAX`
    /// distinct values; itself otherwise.
    pub fn coded(&self) -> Text {
        if let Text::Dict { .. } = self {
            return self.clone();
        }
        let mut b = DictBuilder::with_capacity(self.len());
        for s in self.iter() {
            b.push(s);
            if b.over() {
                return self.clone();
            }
        }
        b.finish()
    }

    /// The bytes the column holds, dictionary included.
    pub fn bytes_held(&self) -> usize {
        match self {
            Text::Plain { offsets, bytes } => offsets.len() * 8 + bytes.len(),
            Text::Dict { codes, values } => codes.len() * 4 + values.bytes_held(),
        }
    }

    /// The named cells, in that order (a dictionary column keeps its
    /// dictionary: only the codes move).
    pub fn gather(&self, rows: &[usize]) -> Text {
        match self {
            Text::Dict { codes, values } => Text::Dict { codes: rows.iter().map(|&r| codes[r]).collect(), values: values.clone() },
            Text::Plain { offsets, bytes } => {
                let total: usize = rows.iter().map(|&r| (offsets[r + 1] - offsets[r]) as usize).sum();
                let mut out = Text::with_capacity(rows.len(), total);
                if let Text::Plain { offsets: o, bytes: b } = &mut out {
                    for &r in rows {
                        b.extend_from_slice(&bytes[offsets[r] as usize..offsets[r + 1] as usize]);
                        o.push(b.len() as u64);
                    }
                }
                out
            }
        }
    }

    /// The named cells, an empty cell where no row is named (its presence
    /// is the caller's to record).
    pub fn gather_some(&self, rows: &[Option<usize>]) -> Text {
        match self {
            Text::Dict { codes, values } => {
                // a hole is an empty cell, as in a plain column: the empty
                // value's code, added to the dictionary where it has none
                let mut values = values.clone();
                let found = values.iter().position(str::is_empty);
                let empty = match found {
                    Some(c) => c as u32,
                    None => {
                        let c = values.len() as u32;
                        values.push("");
                        c
                    }
                };
                Text::Dict { codes: rows.iter().map(|r| r.map_or(empty, |r| codes[r])).collect(), values }
            }
            Text::Plain { offsets, bytes } => {
                let total: usize = rows.iter().flatten().map(|&r| (offsets[r + 1] - offsets[r]) as usize).sum();
                let mut out = Text::with_capacity(rows.len(), total);
                if let Text::Plain { offsets: o, bytes: b } = &mut out {
                    for r in rows {
                        if let Some(r) = r {
                            b.extend_from_slice(&bytes[offsets[*r] as usize..offsets[*r + 1] as usize]);
                        }
                        o.push(b.len() as u64);
                    }
                }
                out
            }
        }
    }

    /// The cells the mask keeps.
    pub fn filtered(&self, mask: &[bool]) -> Text {
        let rows: Vec<usize> = mask.iter().enumerate().filter(|&(_, &m)| m).map(|(i, _)| i).collect();
        self.gather(&rows)
    }

    /// Cells `start..end`.
    pub fn slice(&self, start: usize, end: usize) -> Text {
        match self {
            Text::Dict { codes, values } => Text::Dict { codes: codes[start..end].to_vec(), values: values.clone() },
            Text::Plain { offsets, bytes } => {
                let base = offsets[start];
                Text::Plain {
                    offsets: offsets[start..=end].iter().map(|o| o - base).collect(),
                    bytes: bytes[base as usize..offsets[end] as usize].to_vec(),
                }
            }
        }
    }

    /// This column's cells then another's (two columns coded on the same
    /// dictionary stay coded).
    pub fn concat(&self, other: &Text) -> Text {
        if let (Text::Dict { codes: a, values: va }, Text::Dict { codes: b, values: vb }) = (self, other) {
            if va == vb {
                let mut codes = a.clone();
                codes.extend_from_slice(b);
                return Text::Dict { codes, values: va.clone() };
            }
        }
        let mut out = Text::with_capacity(self.len() + other.len(), 0);
        for s in self.iter().chain(other.iter()) {
            out.push(s);
        }
        out
    }

    /// The column with `value` where `present` is false (a `fill`).
    pub fn filled(&self, present: &[bool], value: &str) -> Text {
        let mut out = Text::with_capacity(self.len(), 0);
        for (i, s) in self.iter().enumerate() {
            out.push(if present[i] { s } else { value });
        }
        out
    }

    pub fn to_vec(&self) -> Vec<String> {
        self.iter().map(str::to_string).collect()
    }
}

impl Text {
    /// Columns joined into one, in order: all coded, one dictionary over
    /// them all (each one's codes renumbered); any plain, or a joined
    /// dictionary past the limit, a plain column.
    pub fn joined(parts: &[Text]) -> Text {
        let total: usize = parts.iter().map(Text::len).sum();
        if parts.iter().all(|p| matches!(p, Text::Dict { .. })) {
            let mut values = Text::new();
            let mut index: HashMap<String, u32> = HashMap::new();
            let mut codes: Vec<u32> = Vec::with_capacity(total);
            for part in parts {
                if let Text::Dict { codes: pc, values: pv } = part {
                    let map: Vec<u32> = pv
                        .iter()
                        .map(|v| match index.get(v) {
                            Some(&c) => c,
                            None => {
                                let c = values.len() as u32;
                                values.push(v);
                                index.insert(v.to_string(), c);
                                c
                            }
                        })
                        .collect();
                    codes.extend(pc.iter().map(|&c| map[c as usize]));
                }
            }
            if values.len() <= DICT_MAX {
                return Text::Dict { codes, values: Box::new(values) };
            }
        }
        let bytes: usize = parts.iter().map(|p| match p { Text::Plain { bytes, .. } => bytes.len(), Text::Dict { codes, values } => codes.len() * (values.bytes_held() / values.len().max(1)) }).sum();
        let mut out = Text::with_capacity(total, bytes);
        for part in parts {
            for s in part.iter() {
                out.push(s);
            }
        }
        out
    }
}

impl Index<usize> for Text {
    type Output = str;
    fn index(&self, i: usize) -> &str {
        self.get(i)
    }
}

/// Two columns are equal when their cells are, however each is held.
impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl<'a> FromIterator<&'a str> for Text {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Text {
        let mut out = Text::new();
        for s in iter {
            out.push(s);
        }
        out
    }
}

impl FromIterator<String> for Text {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Text {
        let mut out = Text::new();
        for s in iter {
            out.push(&s);
        }
        out
    }
}

impl From<Vec<String>> for Text {
    fn from(v: Vec<String>) -> Text {
        v.into_iter().collect()
    }
}

impl From<&[&str]> for Text {
    fn from(v: &[&str]) -> Text {
        v.iter().copied().collect()
    }
}

/// A dictionary column built cell by cell: each new value a code, a
/// value seen again its code. Over `DICT_MAX` distinct values the column
/// is better held plain, which `over` says.
pub struct DictBuilder {
    codes: Vec<u32>,
    values: Text,
    index: HashMap<String, u32>,
}

impl DictBuilder {
    pub fn with_capacity(cells: usize) -> DictBuilder {
        DictBuilder { codes: Vec::with_capacity(cells), values: Text::new(), index: HashMap::new() }
    }

    pub fn push(&mut self, s: &str) {
        let code = match self.index.get(s) {
            Some(&c) => c,
            None => {
                let c = self.values.len() as u32;
                self.values.push(s);
                self.index.insert(s.to_string(), c);
                c
            }
        };
        self.codes.push(code);
    }

    pub fn over(&self) -> bool {
        self.values.len() > DICT_MAX
    }

    pub fn len(&self) -> usize {
        self.codes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }

    pub fn distinct(&self) -> usize {
        self.values.len()
    }

    /// The column so far, coded.
    pub fn snapshot(&self) -> Text {
        Text::Dict { codes: self.codes.clone(), values: Box::new(self.values.clone()) }
    }

    /// The column: coded, or plain where the dictionary grew too large.
    pub fn finish(self) -> Text {
        let coded = Text::Dict { codes: self.codes, values: Box::new(self.values) };
        if coded.dictionary().is_some_and(|(_, v)| v.len() > DICT_MAX) {
            coded.plain()
        } else {
            coded
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_reads_the_same_however_it_is_held() {
        let plain: Text = ["b", "a", "", "b", "é"].as_slice().into();
        let coded = plain.coded();
        assert!(coded.dictionary().is_some_and(|(codes, values)| codes == [0, 1, 2, 0, 3] && values.len() == 4));
        assert_eq!(plain, coded);
        assert_eq!(coded.get(4), "é");
        assert_eq!(&coded[2], "");
        assert_eq!(plain.gather(&[4, 0]).to_vec(), vec!["é", "b"]);
        assert_eq!(coded.gather(&[4, 0]), plain.gather(&[4, 0]));
        assert_eq!(coded.gather_some(&[None, Some(1)]).to_vec(), vec!["", "a"]);
        assert_eq!(plain.slice(1, 3).to_vec(), vec!["a", ""]);
        assert_eq!(coded.slice(1, 3), plain.slice(1, 3));
        assert_eq!(plain.filtered(&[true, false, false, true, false]).to_vec(), vec!["b", "b"]);
        assert_eq!(coded.concat(&coded).len(), 10);
        assert!(coded.concat(&coded).dictionary().is_some());
        assert_eq!(plain.concat(&coded).to_vec().len(), 10);
        assert_eq!(coded.filled(&[true, false, true, true, true], "-").to_vec(), vec!["b", "-", "", "b", "é"]);
        assert_eq!(Text::repeated("x", 3).to_vec(), vec!["x", "x", "x"]);
        let mut grown = coded.clone();
        grown.push("z");
        assert_eq!(grown.len(), 6);
        assert_eq!(grown.get(5), "z");
        assert!(plain.bytes_held() < 5 * 24);
    }

    /// A dictionary that grows past its limit yields a plain column.
    #[test]
    fn a_wide_dictionary_is_held_plain() {
        let mut b = DictBuilder::with_capacity(DICT_MAX + 2);
        for i in 0..DICT_MAX + 2 {
            b.push(&i.to_string());
        }
        assert!(b.over());
        let t = b.finish();
        assert!(t.dictionary().is_none());
        assert_eq!(t.get(DICT_MAX + 1), (DICT_MAX + 1).to_string());
    }
}
