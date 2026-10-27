//! A search over the files of a log (docs/magpie-design.md, phase 7):
//! BM25 as Lucene scores it, over the whole log as one corpus, with the
//! query forms Lucene has — a word, a phrase (a word with a space: the
//! tokens in order), a prefix (`kit*`), a fuzzy word (`kitchn~`, within
//! two edits; `~1` within one) — the stemmer where the log declares it,
//! and a snippet around the first hit where one is asked. An indexed
//! group (a segment's, the merge's) answers from its dictionary and
//! postings, skipping the blocks whose bound cannot reach the k-th
//! score; a group with no index (a batch's, an identifier-like column's)
//! is read and tokenised. The counts are the same either way, so the
//! ranking is.

use super::file::Reader;
use super::postings::{self, GroupIndex};
use super::{stem, tokens, POSITION};
use crate::frame::{Column, CrowFrame};
use std::collections::HashMap;

const K1: f64 = 1.2;
const B: f64 = 0.75;
/// A prefix or a fuzzy word scores each document it matches 1.0, as
/// Lucene's multi-term queries do by default.
const CONSTANT: f64 = 1.0;

/// One term of a query.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Word(String),
    Phrase(Vec<String>),
    Prefix(String),
    Fuzzy(String, usize),
}

/// The words asked, read as terms: each word's tokens, a word with a
/// space as a phrase, `x*` a prefix, `x~` or `x~1` fuzzy; a term asked
/// twice counts once; the stemmer applied to words and phrases where
/// the log stems.
pub fn terms_of(words: &[String], stems: bool) -> Vec<Term> {
    let mut out: Vec<Term> = Vec::new();
    let norm = |t: &str| if stems { stem::stem(t) } else { t.to_string() };
    for w in words {
        let w = w.trim();
        if w.is_empty() {
            continue;
        }
        let term = if let Some(p) = w.strip_suffix('*') {
            let toks = tokens(p);
            match toks.len() {
                0 => continue,
                _ => Term::Prefix(toks.concat()),
            }
        } else if let Some((base, edits)) = w.rsplit_once('~') {
            let edits = if edits.is_empty() { 2 } else { edits.parse::<usize>().unwrap_or(2).min(2) };
            let toks = tokens(base);
            if toks.is_empty() {
                continue;
            }
            Term::Fuzzy(toks.concat(), edits)
        } else {
            let toks: Vec<String> = tokens(w).into_iter().map(|t| norm(&t)).collect();
            match toks.len() {
                0 => continue,
                1 => Term::Word(toks.into_iter().next().unwrap_or_default()),
                _ => Term::Phrase(toks),
            }
        };
        if !out.contains(&term) {
            out.push(term);
        }
    }
    out
}

/// The tokens of the query's words and phrases, each once: what a
/// document frequency is counted for, and an idf read from.
fn atoms_of(terms: &[Term]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in terms {
        match t {
            Term::Word(w) => {
                if !out.contains(w) {
                    out.push(w.clone());
                }
            }
            Term::Phrase(ws) => {
                for w in ws {
                    if !out.contains(w) {
                        out.push(w.clone());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// What one group tells before any row is read: its documents and
/// words, and each atom's document frequency in it.
struct Counts {
    docs: usize,
    words: usize,
    df: Vec<usize>,
}

/// A document's term frequencies for the query's terms, its length, and
/// which atoms it holds.
struct Hit {
    len: u32,
    tf: Vec<u32>,
    /// A constant-scored term's match (prefix, fuzzy).
    matched: Vec<bool>,
    holds: Vec<bool>,
}

fn tf_norm(tf: f64, dl: f64, avgdl: f64) -> f64 {
    tf / (tf + K1 * (1.0 - B + B * dl / avgdl))
}

/// The phrase's occurrences in a document, from each token's positions
/// in order.
fn phrase_count(positions: &[&[u32]]) -> u32 {
    let Some(first) = positions.first() else { return 0 };
    first.iter().filter(|&&p| positions.iter().enumerate().skip(1).all(|(i, ps)| ps.binary_search(&(p + i as u32)).is_ok())).count() as u32
}

/// The hits of a document read as its tokens with positions.
fn hits_of_text(text: &str, terms: &[Term], atoms: &[String], stems: bool) -> (u32, Option<Hit>) {
    let toks = postings::tokens_at(text);
    let len = toks.len() as u32;
    let mut at: HashMap<String, Vec<u32>> = HashMap::new();
    for (t, p) in &toks {
        let t = if stems { stem::stem(t) } else { t.clone() };
        at.entry(t).or_default().push(*p);
    }
    let holds: Vec<bool> = atoms.iter().map(|a| at.contains_key(a)).collect();
    let mut tf = vec![0u32; terms.len()];
    let mut matched = vec![false; terms.len()];
    let mut any = holds.iter().any(|&h| h);
    for (i, term) in terms.iter().enumerate() {
        match term {
            Term::Word(w) => {
                if let Some(ps) = at.get(w) {
                    tf[i] = ps.len() as u32;
                }
            }
            Term::Phrase(ws) => {
                let ps: Option<Vec<&[u32]>> = ws.iter().map(|w| at.get(w).map(|v| v.as_slice())).collect();
                if let Some(ps) = ps {
                    tf[i] = phrase_count(&ps);
                }
            }
            Term::Prefix(p) => matched[i] = at.keys().any(|k| k.starts_with(p.as_str())),
            Term::Fuzzy(w, edits) => {
                let wc: Vec<char> = w.chars().collect();
                matched[i] = at.keys().any(|k| {
                    let kc: Vec<char> = k.chars().collect();
                    kc.len().abs_diff(wc.len()) <= *edits && postings::levenshtein(&wc, &kc, *edits) <= *edits
                });
            }
        }
        any |= tf[i] > 0 || matched[i];
    }
    (len, any.then_some(Hit { len, tf, matched, holds }))
}

/// A file's part in a search: pass one counts, pass two scores.
struct Part {
    reader: Reader,
    column: usize,
    /// Per group: its parsed index where it has one.
    indexes: Vec<Option<GroupIndex>>,
    /// Per group: the counts, and per group the term frequencies of each
    /// document hit (row within the group) where the group was scanned.
    counts: Vec<Counts>,
    scanned: Vec<Option<Vec<(u32, Hit)>>>,
}

fn open_part(path: &str, column: &str, terms: &[Term], atoms: &[String], stems: bool) -> Result<Part, String> {
    let reader = Reader::open(path)?;
    let f = reader.footer();
    let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", path, column))?;
    if f.columns[c].kind != super::encode::Kind::Text {
        return Err(format!("batch: column '{}' holds no text to search", column));
    }
    let entries = reader.entries(c)?;
    let mut indexes = Vec::with_capacity(entries.len());
    let mut counts = Vec::with_capacity(entries.len());
    let mut scanned = Vec::with_capacity(entries.len());
    let simple = terms.iter().all(|t| matches!(t, Term::Word(_) | Term::Phrase(_)));
    for (g, e) in entries.iter().enumerate() {
        if let Some(t) = &e.tokens {
            // the filter first: a group none of the words can be in is
            // counted and not parsed (a prefix or a fuzzy word reads the dictionary)
            let words: Vec<&str> = terms.iter().flat_map(|t| match t { Term::Word(w) => vec![w.as_str()], Term::Phrase(ws) => ws.iter().map(String::as_str).collect(), _ => vec![] }).collect();
            let admits = if simple {
                let bloom = reader.token_bloom(t)?;
                words.iter().any(|w| bloom.has(w.as_bytes()))
            } else {
                true
            };
            if !admits || t.docs == 0 {
                indexes.push(None);
                counts.push(Counts { docs: t.docs as usize, words: t.words as usize, df: vec![0; atoms.len()] });
                scanned.push(None);
                continue;
            }
            let bytes = reader.index_bytes(t)?;
            let ix = GroupIndex::parse(&bytes).map_err(|r| format!("batch: '{}': {}", path, r))?;
            let df = atoms.iter().map(|a| ix.find(a).map(|i| ix.terms[i].df as usize).unwrap_or(0)).collect();
            counts.push(Counts { docs: t.docs as usize, words: t.words as usize, df });
            indexes.push(Some(ix));
            scanned.push(None);
        } else {
            // no index: the column read and every document tokenised
            let frame = reader.group(g, &[c])?;
            let Column::Str(text) = frame.data[0].as_ref() else {
                return Err(format!("batch: column '{}' holds no text to search", column));
            };
            let valid = frame.validity[0].as_ref();
            let (mut docs, mut words) = (0usize, 0usize);
            let mut df = vec![0usize; atoms.len()];
            let mut hits = Vec::new();
            for row in 0..frame.nrows() {
                if !valid[row] {
                    continue;
                }
                let (len, hit) = hits_of_text(text.get(row), terms, atoms, stems);
                docs += 1;
                words += len as usize;
                if let Some(h) = hit {
                    for (i, d) in df.iter_mut().enumerate() {
                        if h.holds[i] {
                            *d += 1;
                        }
                    }
                    if h.tf.iter().any(|&t| t > 0) || h.matched.iter().any(|&m| m) {
                        hits.push((row as u32, h));
                    }
                }
            }
            counts.push(Counts { docs, words, df });
            indexes.push(None);
            scanned.push(Some(hits));
        }
    }
    Ok(Part { reader, column: c, indexes, counts, scanned })
}

/// A scored row: the file, the group, the row, its position, the score.
#[derive(Clone, Debug)]
struct Scored {
    file: usize,
    group: usize,
    row: u32,
    pos: i64,
    score: f64,
}

/// The k best kept, the least on top: a greater score wins, and an
/// earlier position between equals.
/// A ranked hit: the score, then the position, the part, the group and
/// the row — reversed, so the heap's top is the lowest of the best k.
type Ranked = std::cmp::Reverse<(ordered::F, i64, usize, usize, u32)>;

struct Best {
    k: usize,
    heap: std::collections::BinaryHeap<Ranked>,
}

mod ordered {
    /// A float ordered as a total order, for the heap.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct F(pub f64);
    impl Eq for F {}
    impl PartialOrd for F {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for F {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
}

impl Best {
    fn threshold(&self) -> f64 {
        if self.heap.len() < self.k { 0.0 } else { self.heap.peek().map(|r| r.0.0.0).unwrap_or(0.0) }
    }

    fn offer(&mut self, s: Scored) {
        if self.k == 0 {
            return;
        }
        // the entry's order: the score, then the earlier position as the greater
        let entry = std::cmp::Reverse((ordered::F(s.score), -s.pos, s.file, s.group, s.row));
        if self.heap.len() < self.k {
            self.heap.push(entry);
        } else if let Some(least) = self.heap.peek()
            && entry < *least
        {
            // a greater inner tuple is a lesser Reverse: the new row beats the least kept
            self.heap.pop();
            self.heap.push(entry);
        }
    }
}

/// `search` over the files: the scored rows, best first, with the
/// files' columns, a `score` and, where `snippet` is more than 0, a
/// `snippet` of that many characters around the first word hit.
pub fn search(paths: &[String], column: &str, words: &[String], k: i64, stems: bool, snippet: i64) -> Result<CrowFrame, String> {
    if k < 0 {
        return Err(format!("batch: a search answers at most k rows, and k is {}", k));
    }
    let terms = terms_of(words, stems);
    let atoms = atoms_of(&terms);
    if terms.is_empty() || paths.is_empty() {
        return super::empty_scored(paths.first(), snippet > 0);
    }
    let parts: Vec<Result<Part, String>> = crate::cores::map_items(paths, paths.len() * 1024, |p| open_part(p, column, &terms, &atoms, stems));
    let parts: Vec<Part> = parts.into_iter().collect::<Result<_, _>>()?;
    // pass one: the corpus
    let n: usize = parts.iter().flat_map(|p| p.counts.iter().map(|c| c.docs)).sum();
    let total: usize = parts.iter().flat_map(|p| p.counts.iter().map(|c| c.words)).sum();
    let mut df = vec![0usize; atoms.len()];
    for p in &parts {
        for c in &p.counts {
            for (i, d) in c.df.iter().enumerate() {
                df[i] += d;
            }
        }
    }
    let constant_only = terms.iter().all(|t| matches!(t, Term::Prefix(_) | Term::Fuzzy(..)));
    if n == 0 || (df.iter().all(|&d| d == 0) && !constant_only) {
        return super::empty_scored(paths.first(), snippet > 0);
    }
    let avgdl = total as f64 / n as f64;
    let idf_of: HashMap<&str, f64> = atoms.iter().zip(&df).map(|(a, &d)| (a.as_str(), (1.0 + (n as f64 - d as f64 + 0.5) / (d as f64 + 0.5)).ln())).collect();
    // a word's weight is its idf; a phrase's is its tokens' idfs summed,
    // as Lucene's phrase scorer weighs one
    let weight = |i: usize| match &terms[i] {
        Term::Word(w) => idf_of.get(w.as_str()).copied().unwrap_or(0.0),
        Term::Phrase(ws) => ws.iter().map(|w| idf_of.get(w.as_str()).copied().unwrap_or(0.0)).sum(),
        _ => CONSTANT,
    };
    // pass two: the scores, the k best kept, groups and blocks whose
    // bound cannot reach the k-th score skipped
    let mut best = Best { k: k as usize, heap: std::collections::BinaryHeap::new() };
    for (fi, p) in parts.iter().enumerate() {
        let pos_col = p.reader.footer().column(POSITION);
        // the positions of a group's rows, for the order between equal scores
        let positions_of = |g: usize| -> Result<Vec<i64>, String> {
            let Some(pc) = pos_col else { return Ok(Vec::new()) };
            let f = p.reader.group(g, &[pc])?;
            Ok(match f.data[0].as_ref() {
                Column::Int(v) => v.clone(),
                _ => Vec::new(),
            })
        };
        let pos_at = |positions: &[i64], row: u32| positions.get(row as usize).copied().unwrap_or(row as i64);
        for g in 0..p.counts.len() {
            if let Some(hits) = &p.scanned[g] {
                if hits.is_empty() {
                    continue;
                }
                let positions = positions_of(g)?;
                for (row, h) in hits {
                    let mut s = 0.0;
                    for i in 0..terms.len() {
                        if h.tf[i] > 0 {
                            s += weight(i) * tf_norm(h.tf[i] as f64, h.len as f64, avgdl);
                        } else if h.matched[i] {
                            s += CONSTANT;
                        }
                    }
                    best.offer(Scored { file: fi, group: g, row: *row, pos: pos_at(&positions, *row), score: s });
                }
                continue;
            }
            let Some(ix) = &p.indexes[g] else { continue };
            // each term's postings in the group, with the bound of each block
            let mut per_term: Vec<Vec<postings::Block>> = Vec::with_capacity(terms.len());
            let mut bounds: Vec<f64> = Vec::with_capacity(terms.len());
            for (i, term) in terms.iter().enumerate() {
                let blocks: Vec<postings::Block> = match term {
                    Term::Word(w) => ix.find(w).map(|t| ix.postings(t)).transpose()?.unwrap_or_default(),
                    Term::Phrase(ws) => {
                        // the phrase's postings: the rows holding every token, with the phrase's count
                        let lists: Option<Vec<Vec<postings::Block>>> = ws.iter().map(|w| ix.find(w).map(|t| ix.postings(t))).collect::<Option<Result<Vec<_>, _>>>().transpose()?;
                        match lists {
                            None => Vec::new(),
                            Some(lists) => {
                                let maps: Vec<HashMap<u32, Vec<u32>>> = lists.iter().map(|bs| bs.iter().flat_map(|b| b.postings.iter().map(|p| (p.row, p.positions.clone()))).collect()).collect();
                                let mut ps: Vec<postings::Posting> = Vec::new();
                                for (row, first) in &maps[0] {
                                    let all: Option<Vec<&[u32]>> = maps.iter().map(|m| m.get(row).map(|v| v.as_slice())).collect();
                                    if let Some(all) = all {
                                        let _ = first;
                                        let count = phrase_count(&all);
                                        if count > 0 {
                                            ps.push(postings::Posting { row: *row, tf: count, positions: Vec::new() });
                                        }
                                    }
                                }
                                ps.sort_by_key(|p| p.row);
                                let max_tf = ps.iter().map(|p| p.tf).max().unwrap_or(0);
                                let min_len = ps.iter().map(|p| ix.lengths[p.row as usize]).min().unwrap_or(0);
                                if ps.is_empty() { Vec::new() } else { vec![postings::Block { max_tf, min_len, postings: ps }] }
                            }
                        }
                    }
                    Term::Prefix(pre) => {
                        let mut ps: Vec<postings::Posting> = Vec::new();
                        for t in ix.with_prefix(pre) {
                            for b in ix.postings(t)? {
                                ps.extend(b.postings.into_iter().map(|p| postings::Posting { row: p.row, tf: 1, positions: Vec::new() }));
                            }
                        }
                        ps.sort_by_key(|p| p.row);
                        ps.dedup_by_key(|p| p.row);
                        if ps.is_empty() { Vec::new() } else { vec![postings::Block { max_tf: 1, min_len: 1, postings: ps }] }
                    }
                    Term::Fuzzy(w, edits) => {
                        let mut ps: Vec<postings::Posting> = Vec::new();
                        for t in ix.within(w, *edits) {
                            for b in ix.postings(t)? {
                                ps.extend(b.postings.into_iter().map(|p| postings::Posting { row: p.row, tf: 1, positions: Vec::new() }));
                            }
                        }
                        ps.sort_by_key(|p| p.row);
                        ps.dedup_by_key(|p| p.row);
                        if ps.is_empty() { Vec::new() } else { vec![postings::Block { max_tf: 1, min_len: 1, postings: ps }] }
                    }
                };
                let bound = match term {
                    Term::Word(_) | Term::Phrase(_) => blocks.iter().map(|b| weight(i) * tf_norm(b.max_tf as f64, b.min_len as f64, avgdl)).fold(0.0, f64::max),
                    _ => if blocks.is_empty() { 0.0 } else { CONSTANT },
                };
                per_term.push(blocks);
                bounds.push(bound);
            }
            let group_bound: f64 = bounds.iter().sum();
            if group_bound <= best.threshold() && best.heap.len() >= best.k && best.k > 0 {
                continue;
            }
            // the documents of the group with a term, each its score; a
            // block of a term is skipped where its bound with the other
            // terms' cannot reach the threshold
            let mut acc: HashMap<u32, f64> = HashMap::new();
            for (i, blocks) in per_term.iter().enumerate() {
                let others: f64 = group_bound - bounds[i];
                for b in blocks {
                    let block_bound = match &terms[i] {
                        Term::Word(_) | Term::Phrase(_) => weight(i) * tf_norm(b.max_tf as f64, b.min_len as f64, avgdl),
                        _ => CONSTANT,
                    };
                    if block_bound + others <= best.threshold() && best.heap.len() >= best.k && best.k > 0 {
                        continue;
                    }
                    for post in &b.postings {
                        let s = match &terms[i] {
                            Term::Word(_) | Term::Phrase(_) => weight(i) * tf_norm(post.tf as f64, ix.lengths[post.row as usize] as f64, avgdl),
                            _ => CONSTANT,
                        };
                        *acc.entry(post.row).or_insert(0.0) += s;
                    }
                }
            }
            if acc.is_empty() {
                continue;
            }
            let positions = positions_of(g)?;
            for (row, s) in acc {
                best.offer(Scored { file: fi, group: g, row, pos: pos_at(&positions, row), score: s });
            }
        }
    }
    // the rows of the k best read, best first, an earlier position first between equals
    // best first, an earlier position first between equals
    let mut chosen: Vec<(f64, i64, usize, usize, u32)> = best.heap.into_iter().map(|r| (r.0.0.0, -r.0.1, r.0.2, r.0.3, r.0.4)).collect();
    chosen.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)).then(a.4.cmp(&b.4)));
    let mut frames: Vec<CrowFrame> = Vec::new();
    let mut scores: Vec<f64> = Vec::new();
    let mut snippets: Vec<String> = Vec::new();
    let mut by_group: HashMap<(usize, usize), CrowFrame> = HashMap::new();
    for (score, _, fi, g, row) in &chosen {
        let key = (*fi, *g);
        if let std::collections::hash_map::Entry::Vacant(slot) = by_group.entry(key) {
            let all: Vec<usize> = (0..parts[*fi].reader.footer().columns.len()).collect();
            slot.insert(parts[*fi].reader.group(*g, &all)?);
        }
        let frame = &by_group[&key];
        let mask: Vec<bool> = (0..frame.nrows()).map(|i| i == *row as usize).collect();
        let one = frame.pick_rows(&mask)?;
        if snippet > 0 {
            let text = match frame.data[parts[*fi].column].as_ref() {
                Column::Str(t) => t.get(*row as usize).to_string(),
                _ => String::new(),
            };
            snippets.push(snippet_of(&text, &terms, snippet as usize, stems));
        }
        frames.push(one);
        scores.push(*score);
    }
    if frames.is_empty() {
        return super::empty_scored(paths.first(), snippet > 0);
    }
    let whole = CrowFrame::stack_all(&frames)?;
    let mut out = whole.with_column("score", Column::Float(scores))?;
    if snippet > 0 {
        out = out.with_column("snippet", Column::strs(snippets))?;
    }
    let _ = POSITION;
    Ok(out)
}

/// The text around the first word of the query found in it, `chars`
/// characters each side, at character boundaries; the start of the
/// text where no word is found.
fn snippet_of(text: &str, terms: &[Term], chars: usize, stems: bool) -> String {
    let lower = text.to_lowercase();
    let mut at: Option<(usize, usize)> = None;
    for term in terms {
        let needles: Vec<String> = match term {
            Term::Word(w) => vec![w.clone()],
            Term::Phrase(ws) => vec![ws.join(" ")],
            Term::Prefix(p) => vec![p.clone()],
            Term::Fuzzy(w, _) => vec![w.clone()],
        };
        for needle in needles {
            let found = lower.find(&needle).map(|i| (i, needle.len())).or_else(|| if stems { let s = stem::stem(&needle); lower.find(&s).map(|i| (i, s.len())) } else { None });
            if let Some((i, l)) = found
                && at.is_none_or(|(a, _)| i < a)
            {
                at = Some((i, l));
            }
        }
    }
    let (center, found_len) = at.unwrap_or((0, 0));
    let start = text[..center].char_indices().rev().nth(chars.saturating_sub(1)).map(|(i, _)| i).unwrap_or(0);
    let after = (center + found_len).min(text.len());
    let end = text[after..].char_indices().nth(chars).map(|(i, _)| after + i).unwrap_or(text.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&text[start..end]);
    if end < text.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_asked_read_as_terms() {
        let t = terms_of(&["Fox".into(), "kitchen table".into(), "kit*".into(), "kitchn~".into(), "kitchn~1".into(), "".into(), "fox".into()], false);
        assert_eq!(t, vec![Term::Word("fox".into()), Term::Phrase(vec!["kitchen".into(), "table".into()]), Term::Prefix("kit".into()), Term::Fuzzy("kitchn".into(), 2), Term::Fuzzy("kitchn".into(), 1)]);
        assert_eq!(terms_of(&["running".into()], true), vec![Term::Word("run".into())]);
    }

    #[test]
    fn a_phrase_counts_its_occurrences_in_order() {
        assert_eq!(phrase_count(&[&[0, 5][..], &[1, 9][..]]), 1);
        assert_eq!(phrase_count(&[&[0, 5][..], &[1, 6][..]]), 2);
        assert_eq!(phrase_count(&[&[2][..], &[1][..]]), 0);
    }

    #[test]
    fn a_snippet_sits_around_the_first_hit() {
        assert_eq!(snippet_of("a crate of apples for the shop", &[Term::Word("apples".into())], 4, false), "… of apples for…");
        assert_eq!(snippet_of("short", &[Term::Word("zzz".into())], 10, false), "short");
    }
}
