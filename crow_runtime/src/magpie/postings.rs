//! A text column's index over one row group (docs/magpie-design.md, phase
//! 7): the lengths of its documents, a sorted dictionary of its terms,
//! and for each term its postings — the rows holding it with the count
//! and the positions — so a query costs the postings of its words, not
//! the column. The blob a group keeps:
//!
//! ```text
//! u32 docs            varint-packed lengths (one per row; 0 for a missing cell)
//! u32 terms           per term: string, u32 df, u64 postings offset, u32 postings length
//! postings blob       per term: blocks of up to 128 postings —
//!                     u8 count, u32 max tf, u32 min length, then per posting
//!                     varint row delta, varint tf, varint count of positions,
//!                     varint position deltas
//! ```
//!
//! The same counts a scan of the column yields, so a group with an index
//! and one without score alike.

use super::pack::{self, Cursor};
use std::collections::BTreeMap;

/// Postings per block, and the bound a block carries: its largest term
/// frequency and its shortest document, which give the best score a
/// posting of the block can reach.
pub const BLOCK: usize = 128;

/// The tokens of a text with their positions.
pub fn tokens_at(text: &str) -> Vec<(String, u32)> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).enumerate().map(|(i, w)| (w.to_lowercase(), i as u32)).collect()
}

/// One posting read back: the row, the term's count in it, its positions.
#[derive(Clone, Debug, PartialEq)]
pub struct Posting {
    pub row: u32,
    pub tf: u32,
    pub positions: Vec<u32>,
}

/// A term's postings as blocks, each with its bound.
#[derive(Clone, Debug)]
pub struct Block {
    pub max_tf: u32,
    pub min_len: u32,
    pub postings: Vec<Posting>,
}

/// A term of the dictionary: its text, how many documents hold it, and
/// where its postings lie in the blob.
#[derive(Clone, Debug)]
pub struct Term {
    pub text: String,
    pub df: u32,
    off: u64,
    len: u32,
}

/// The index of one group, parsed: lengths, dictionary, the postings blob.
pub struct GroupIndex {
    pub lengths: Vec<u32>,
    pub terms: Vec<Term>,
    blob: Vec<u8>,
}

/// The index of a group built from its documents (`None` for a missing
/// cell), with an optional stemmer over each token: the blob's bytes,
/// the document count and the word count.
pub fn build(docs: &[Option<&str>], stem: bool) -> (Vec<u8>, u32, u32) {
    let mut lengths: Vec<u64> = Vec::with_capacity(docs.len());
    let mut by_term: BTreeMap<String, Vec<(u32, Vec<u32>)>> = BTreeMap::new();
    let (mut ndocs, mut words) = (0u32, 0u32);
    for (row, doc) in docs.iter().enumerate() {
        let Some(text) = doc else {
            lengths.push(0);
            continue;
        };
        let toks = tokens_at(text);
        ndocs += 1;
        words += toks.len() as u32;
        lengths.push(toks.len() as u64);
        let mut here: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (t, p) in toks {
            let t = if stem { super::stem::stem(&t) } else { t };
            here.entry(t).or_default().push(p);
        }
        for (t, positions) in here {
            by_term.entry(t).or_default().push((row as u32, positions));
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(docs.len() as u32).to_le_bytes());
    let width = pack::width_of(lengths.iter().copied().max().unwrap_or(0));
    out.push(width);
    out.extend(pack::pack(&lengths, width));
    // the postings blob first, so each term's offsets are known
    let mut blob = Vec::new();
    let mut dict: Vec<(String, u32, u64, u32)> = Vec::with_capacity(by_term.len());
    for (term, postings) in &by_term {
        let off = blob.len() as u64;
        let mut last_row = 0u32;
        for chunk in postings.chunks(BLOCK) {
            blob.push(chunk.len() as u8);
            let max_tf = chunk.iter().map(|(_, p)| p.len() as u32).max().unwrap_or(0);
            let min_len = chunk.iter().map(|(r, _)| lengths[*r as usize] as u32).min().unwrap_or(0);
            blob.extend_from_slice(&max_tf.to_le_bytes());
            blob.extend_from_slice(&min_len.to_le_bytes());
            for (row, positions) in chunk {
                pack::put_var(&mut blob, (*row - last_row) as u64);
                last_row = *row;
                pack::put_var(&mut blob, positions.len() as u64);
                pack::put_var(&mut blob, positions.len() as u64);
                let mut last_p = 0u32;
                for &p in positions {
                    pack::put_var(&mut blob, (p - last_p) as u64);
                    last_p = p;
                }
            }
        }
        dict.push((term.clone(), postings.len() as u32, off, (blob.len() as u64 - off) as u32));
    }
    pack::put_var(&mut out, dict.len() as u64);
    for (term, df, off, len) in &dict {
        pack::put_string(&mut out, term);
        out.extend_from_slice(&df.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
    }
    out.extend_from_slice(&blob);
    (out, ndocs, words)
}

impl GroupIndex {
    pub fn parse(bytes: &[u8]) -> Result<GroupIndex, String> {
        let mut c = Cursor::new(bytes);
        let n = c.u32()? as usize;
        let width = c.u8()?;
        let packed = c.take((n * width as usize).div_ceil(8))?;
        let lengths: Vec<u32> = pack::unpack(packed, n, width)?.into_iter().map(|l| l as u32).collect();
        let nt = c.var()? as usize;
        let mut terms = Vec::with_capacity(nt);
        for _ in 0..nt {
            let text = c.string()?;
            let df = c.u32()?;
            let off = c.u64()?;
            let len = c.u32()?;
            terms.push(Term { text, df, off, len });
        }
        let blob = c.take(c.rest())?.to_vec();
        Ok(GroupIndex { lengths, terms, blob })
    }

    /// The term's place in the dictionary, if there.
    pub fn find(&self, term: &str) -> Option<usize> {
        self.terms.binary_search_by(|t| t.text.as_str().cmp(term)).ok()
    }

    /// The terms with the prefix, in order.
    pub fn with_prefix(&self, prefix: &str) -> Vec<usize> {
        let start = self.terms.partition_point(|t| t.text.as_str() < prefix);
        (start..self.terms.len()).take_while(|&i| self.terms[i].text.starts_with(prefix)).collect()
    }

    /// The terms within the edit distance of the word (Levenshtein, at
    /// most 2, as Lucene's fuzzy query takes it; the word itself first).
    pub fn within(&self, word: &str, edits: usize) -> Vec<usize> {
        let w: Vec<char> = word.chars().collect();
        let mut out: Vec<(usize, usize)> = self
            .terms
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let tc: Vec<char> = t.text.chars().collect();
                if tc.len().abs_diff(w.len()) > edits {
                    return None;
                }
                let d = levenshtein(&w, &tc, edits);
                (d <= edits).then_some((d, i))
            })
            .collect();
        out.sort();
        out.into_iter().map(|(_, i)| i).collect()
    }

    /// A term's postings, block by block.
    pub fn postings(&self, term: usize) -> Result<Vec<Block>, String> {
        let t = &self.terms[term];
        let bytes = self.blob.get(t.off as usize..(t.off + t.len as u64) as usize).ok_or_else(|| "the index's postings end early".to_string())?;
        let mut c = Cursor::new(bytes);
        let mut blocks = Vec::new();
        let mut last_row = 0u32;
        let mut seen = 0u32;
        while seen < t.df {
            let count = c.u8()? as usize;
            let max_tf = c.u32()?;
            let min_len = c.u32()?;
            let mut postings = Vec::with_capacity(count);
            for _ in 0..count {
                let row = last_row + c.var()? as u32;
                last_row = row;
                let tf = c.var()? as u32;
                let np = c.var()? as usize;
                let mut positions = Vec::with_capacity(np);
                let mut last_p = 0u32;
                for _ in 0..np {
                    let p = last_p + c.var()? as u32;
                    last_p = p;
                    positions.push(p);
                }
                postings.push(Posting { row, tf, positions });
            }
            seen += count as u32;
            blocks.push(Block { max_tf, min_len, postings });
        }
        Ok(blocks)
    }
}

/// The edit distance of two words, cut at `most` (an answer past it is
/// answered as `most + 1`).
pub fn levenshtein(a: &[char], b: &[char], most: usize) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(cur[j]);
        }
        if row_min > most {
            return most + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_round_trips_and_answers_terms_prefixes_and_edits() {
        let docs = [Some("the quick brown fox"), None, Some("the fox and the dog"), Some("brown bread"), Some("")];
        let (bytes, ndocs, words) = build(&docs, false);
        assert_eq!((ndocs, words), (4, 11));
        let ix = GroupIndex::parse(&bytes).unwrap();
        assert_eq!(ix.lengths, vec![4, 0, 5, 2, 0]);
        assert_eq!(ix.terms.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), vec!["and", "bread", "brown", "dog", "fox", "quick", "the"]);
        let the = ix.find("the").unwrap();
        assert_eq!(ix.terms[the].df, 2);
        let blocks = ix.postings(the).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].postings, vec![Posting { row: 0, tf: 1, positions: vec![0] }, Posting { row: 2, tf: 2, positions: vec![0, 3] }]);
        assert_eq!((blocks[0].max_tf, blocks[0].min_len), (2, 4));
        assert_eq!(ix.with_prefix("br"), vec![1, 2]);
        assert!(ix.with_prefix("zz").is_empty());
        assert_eq!(ix.within("fix", 1), vec![ix.find("fox").unwrap()]);
        assert_eq!(ix.within("brwn", 2), vec![ix.find("brown").unwrap()]);
        assert!(ix.find("cat").is_none());
    }

    #[test]
    fn many_postings_make_several_blocks() {
        let texts: Vec<String> = (0..300).map(|i| if i % 3 == 0 { format!("word {} and word", i) } else { "other".to_string() }).collect();
        let docs: Vec<Option<&str>> = texts.iter().map(|t| Some(t.as_str())).collect();
        let (bytes, _, _) = build(&docs, false);
        let ix = GroupIndex::parse(&bytes).unwrap();
        let word = ix.find("word").unwrap();
        assert_eq!(ix.terms[word].df, 100);
        let blocks = ix.postings(word).unwrap();
        assert_eq!(blocks.iter().map(|b| b.postings.len()).sum::<usize>(), 100);
        assert_eq!(blocks[0].postings[1].row, 3);
        assert_eq!(blocks[0].postings[1].positions, vec![0, 3]);
        assert_eq!(blocks[0].max_tf, 2);
    }

    #[test]
    fn edit_distance_is_cut_at_the_limit() {
        let a: Vec<char> = "kitchen".chars().collect();
        assert_eq!(levenshtein(&a, &"kitchn".chars().collect::<Vec<_>>(), 2), 1);
        assert_eq!(levenshtein(&a, &"kitten".chars().collect::<Vec<_>>(), 2), 2);
        assert_eq!(levenshtein(&a, &"kittens".chars().collect::<Vec<_>>(), 2), 3, "past the limit: the limit plus one");
        assert_eq!(levenshtein(&a, &"garden".chars().collect::<Vec<_>>(), 2), 3);
    }
}
