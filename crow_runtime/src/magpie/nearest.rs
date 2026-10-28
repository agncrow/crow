//! `nearest` over the log's files: the k rows whose vector column lies
//! nearest a query vector by the cosine — the inner product of the row's
//! unit vector and the query's, the score packages/vectors' flat index
//! computes (`index` holds the rows as unit vectors, `nearest` the inner
//! product with the query's unit vector), so the two agree row for row.
//! Exact: every present row of every file is scored (a group at a time,
//! the files across the cores), the k best kept in a heap — a greater
//! score first, an earlier position between equals — and the winners'
//! rows read whole, with a `score` column. A tail's pending rows are a
//! file like any other, so a row put a moment ago is found. An index per
//! segment (an HNSW) is the step when a corpus asks for one.

use super::file::Reader;
use super::POSITION;
use crate::frame::{Column, CrowFrame};
use std::collections::HashMap;

/// A scored row: the file, the group, the row, its position, the score.
#[derive(Clone, Copy, Debug)]
struct Scored {
    file: usize,
    group: usize,
    row: u32,
    pos: i64,
    score: f64,
}

/// A float ordered as a total order, for the heap.
#[derive(Clone, Copy, PartialEq, Debug)]
struct F(f64);
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

/// A ranked hit, reversed so the heap's top is the least of the best k:
/// the score, then the earlier position as the greater.
type Ranked = std::cmp::Reverse<(F, i64, usize, usize, u32)>;

struct Best {
    k: usize,
    heap: std::collections::BinaryHeap<Ranked>,
}

impl Best {
    fn new(k: usize) -> Best {
        Best { k, heap: std::collections::BinaryHeap::new() }
    }

    fn offer(&mut self, s: Scored) {
        if self.k == 0 {
            return;
        }
        let entry = std::cmp::Reverse((F(s.score), -s.pos, s.file, s.group, s.row));
        if self.heap.len() < self.k {
            self.heap.push(entry);
        } else if let Some(least) = self.heap.peek()
            && entry < *least
        {
            self.heap.pop();
            self.heap.push(entry);
        }
    }

    /// The best first, an earlier position first between equals, then
    /// the earlier file, group and row.
    fn ranked(self) -> Vec<Scored> {
        let mut out: Vec<Scored> = self.heap.into_iter().map(|r| Scored { score: r.0.0.0, pos: -r.0.1, file: r.0.2, group: r.0.3, row: r.0.4 }).collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.pos.cmp(&b.pos)).then(a.file.cmp(&b.file)).then(a.group.cmp(&b.group)).then(a.row.cmp(&b.row)));
        out
    }
}

/// The vector as a unit vector: each number over the norm (the square
/// root of the squares summed in order), as the vectors package's `unit`
/// computes it; a vector of zeros stays as it is.
fn unit(v: &[f64]) -> Vec<f64> {
    let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / n).collect()
}

/// The inner product, summed in order.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b) {
        s += x * y;
    }
    s
}

/// One file scored: its reader and the best of its rows.
struct Part {
    reader: Reader,
    best: Vec<Scored>,
}

fn score_part(fi: usize, path: &str, column: &str, uq: &[f64], k: usize) -> Result<Part, String> {
    let reader = Reader::open(path)?;
    let f = reader.footer();
    let c = f.column(column).ok_or_else(|| format!("batch: '{}' holds no column '{}'", path, column))?;
    if f.columns[c].kind != super::encode::Kind::Vector {
        return Err(format!("batch: column '{}' holds {}, not vectors", column, f.columns[c].kind.word()));
    }
    let pos_col = f.column(POSITION);
    let entries = reader.entries(c)?;
    let mut best = Best::new(k);
    for (g, e) in entries.iter().enumerate() {
        if e.rows == 0 || e.missing == e.rows {
            continue;
        }
        let (col, valid) = reader.chunk(c, e)?;
        let Column::Vectors(w, data) = &col else {
            return Err(format!("batch: column '{}' holds {}, not vectors", column, col.holds()));
        };
        if *w != uq.len() {
            return Err(format!("batch: column '{}' holds vectors of width {}, and the query has {} numbers", column, w, uq.len()));
        }
        let positions: Option<Vec<i64>> = match pos_col {
            Some(pc) => match reader.group(g, &[pc])?.data[0].as_ref() {
                Column::Int(v) => Some(v.clone()),
                _ => None,
            },
            None => None,
        };
        for row in 0..col.len() {
            if !valid[row] {
                continue;
            }
            let v = &data[row * w..(row + 1) * w];
            let s = dot(&unit(v), uq);
            let pos = positions.as_ref().and_then(|p| p.get(row).copied()).unwrap_or(row as i64);
            best.offer(Scored { file: fi, group: g, row: row as u32, pos, score: s });
        }
    }
    Ok(Part { reader, best: best.ranked() })
}

/// The files' columns and a `score`, with no rows.
fn empty_scored(path: Option<&String>) -> Result<CrowFrame, String> {
    let base = match path {
        None => CrowFrame::new(Vec::new(), Vec::new(), Vec::new()),
        Some(path) => {
            let r = Reader::open(path)?;
            let all: Vec<usize> = (0..r.footer().columns.len()).collect();
            r.empty(&all)
        }
    };
    base.with_column("score", Column::Float(Vec::new()))
}

/// `sys.batch_nearest[paths, column, query, k]`: the `k` rows of the
/// files whose vector `column` lies nearest the query, with the files'
/// columns and a `score` — the cosine — the best first and an earlier
/// position first between equals.
pub fn nearest(paths: &[String], column: &str, query: &[f64], k: i64) -> Result<CrowFrame, String> {
    if k < 0 {
        return Err(format!("batch: nearest answers at most k rows, and k is {}", k));
    }
    if query.is_empty() {
        return Err("batch: a query vector holds one number or more".to_string());
    }
    if paths.is_empty() || k == 0 {
        return empty_scored(paths.first());
    }
    let uq = unit(query);
    let parts: Vec<Result<Part, String>> = crate::cores::map_items(paths, paths.len() * 1024, |p| score_part(0, p, column, &uq, k as usize));
    let mut parts: Vec<Part> = parts.into_iter().collect::<Result<_, _>>()?;
    let mut best = Best::new(k as usize);
    for (fi, p) in parts.iter_mut().enumerate() {
        for s in &p.best {
            best.offer(Scored { file: fi, ..*s });
        }
    }
    let chosen = best.ranked();
    if chosen.is_empty() {
        return empty_scored(paths.first());
    }
    // the winners' rows read whole, a group read once
    let mut frames: Vec<CrowFrame> = Vec::with_capacity(chosen.len());
    let mut scores: Vec<f64> = Vec::with_capacity(chosen.len());
    let mut by_group: HashMap<(usize, usize), CrowFrame> = HashMap::new();
    for s in &chosen {
        let key = (s.file, s.group);
        if let std::collections::hash_map::Entry::Vacant(slot) = by_group.entry(key) {
            let all: Vec<usize> = (0..parts[s.file].reader.footer().columns.len()).collect();
            slot.insert(parts[s.file].reader.group(s.group, &all)?);
        }
        let frame = &by_group[&key];
        let mask: Vec<bool> = (0..frame.nrows()).map(|i| i == s.row as usize).collect();
        frames.push(frame.pick_rows(&mask)?);
        scores.push(s.score);
    }
    let whole = CrowFrame::stack_all(&frames)?;
    whole.with_column("score", Column::Float(scores))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::CrowFrame;

    fn log_dir(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("crow_nearest_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.to_string_lossy().to_string()
    }

    /// The nearest rows come best first, an earlier position first
    /// between equals, across two files, the scores the cosines; a row
    /// of zeros scores 0 and a missing cell is skipped; a width that is
    /// not the query's, a column of another kind and a negative k are
    /// refused.
    #[test]
    fn the_nearest_rows_come_best_first_across_files() {
        let dir = log_dir("two");
        let f1 = CrowFrame::new(
            vec!["id".into(), "e".into()],
            vec![Column::Int(vec![1, 2, 3]), Column::Vectors(2, vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0])],
            vec![vec![true; 3], vec![true, true, true]],
        );
        let f2 = CrowFrame::new(
            vec!["id".into(), "e".into()],
            vec![Column::Int(vec![4, 5, 6]), Column::Vectors(2, vec![2.0, 0.0, 0.0, 0.0, 3.0, 4.0])],
            vec![vec![true; 3], vec![true, true, false]],
        );
        let p1 = super::super::write(&dir, 1, 0, &f1, &[], &[], true).expect("written");
        let p2 = super::super::write(&dir, 2, 3, &f2, &[], &[], true).expect("written");
        let paths = vec![p1, p2];
        let hits = nearest(&paths, "e", &[1.0, 0.0], 3).expect("nearest");
        let ids = match hits.col_native("id").expect("id").data.as_ref() {
            Column::Int(v) => v.clone(),
            _ => panic!("ints"),
        };
        // 1 and 4 score 1 (1 first: the earlier position), then 3 at the cosine of 45 degrees
        assert_eq!(ids, vec![1, 4, 3]);
        let scores = match hits.col_native("score").expect("score").data.as_ref() {
            Column::Float(v) => v.clone(),
            _ => panic!("floats"),
        };
        assert_eq!(scores[0], 1.0);
        assert_eq!(scores[1], 1.0);
        assert!((scores[2] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15, "{}", scores[2]);
        // every row asked: the zero row scores 0, the missing row is not there
        let all = nearest(&paths, "e", &[0.0, 1.0], 10).expect("nearest");
        assert_eq!(all.nrows(), 5);
        let ids = match all.col_native("id").expect("id").data.as_ref() {
            Column::Int(v) => v.clone(),
            _ => panic!("ints"),
        };
        assert_eq!(ids, vec![2, 3, 1, 4, 5]);
        assert!(nearest(&paths, "e", &[1.0, 0.0, 0.0], 3).expect_err("width").contains("vectors of width 2, and the query has 3 numbers"));
        assert!(nearest(&paths, "id", &[1.0, 0.0], 3).expect_err("kind").contains("holds int, not vectors"));
        assert!(nearest(&paths, "e", &[1.0, 0.0], -1).expect_err("k").contains("k is -1"));
        assert_eq!(nearest(&paths, "e", &[1.0, 0.0], 0).expect("none").nrows(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
