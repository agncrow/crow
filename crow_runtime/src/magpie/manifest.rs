//! The manifest: the list of a log's live segments and what the log
//! declares of itself (its keys, its time column, its narrow columns),
//! a Magpie file of one group at `dir/manifest` — one row per segment:
//! its file name, its level, its position range, its rows, its bytes and
//! its key ranges as JSON — with the keys in the file's own key list and
//! the declarations in its notes. Written beside and renamed over the
//! old one, so a reader sees the old list or the new, never half.
//! Batches are not in it: they are the tail, found by listing the
//! directory, which is short because the merge runs.

use super::encode::Kind;
use super::file::{Reader, Writer};
use crate::frame::{Column, CrowFrame};
use crate::textcol::Text;
use std::collections::BTreeMap;

pub const MANIFEST: &str = "manifest";

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// The file's name under the log's directory.
    pub name: String,
    pub level: i64,
    /// The batch numbers the segment absorbed, lowest and highest: the
    /// next batch takes the number past every segment's `high`.
    pub number: i64,
    pub high: i64,
    pub first: i64,
    pub last: i64,
    pub rows: i64,
    pub bytes: i64,
    /// Each key's `[lo, hi]` in the segment, as a JSON object.
    pub ranges: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub keys: Vec<String>,
    pub notes: BTreeMap<String, String>,
    pub segments: Vec<Segment>,
}

const COLUMNS: [&str; 9] = ["name", "level", "number", "high", "first", "last", "rows", "bytes", "ranges"];

impl Manifest {
    /// The float columns the log declared narrow (the note `narrow`, names
    /// joined by commas).
    pub fn narrow(&self) -> Vec<String> {
        self.notes.get("narrow").map(|n| n.split(',').filter(|c| !c.is_empty()).map(|c| c.to_string()).collect()).unwrap_or_default()
    }

    /// Whether the log stems its tokens (the note `stem`, `true`).
    pub fn stems(&self) -> bool {
        self.notes.get("stem").is_some_and(|v| v == "true")
    }

    /// The highest segment counter any listed segment carries, 0 for none.
    pub fn counter(&self) -> i64 {
        self.segments.iter().filter_map(|s| super::segment_counter(&s.name)).max().unwrap_or(0)
    }

    /// The manifest as the frame its file holds.
    pub fn frame(&self) -> CrowFrame {
        let n = self.segments.len();
        let mut names = Text::with_capacity(n, n * 24);
        let mut ranges = Text::with_capacity(n, n * 64);
        for s in &self.segments {
            names.push(&s.name);
            ranges.push(&s.ranges);
        }
        CrowFrame::new(
            COLUMNS.iter().map(|c| c.to_string()).collect(),
            vec![
                Column::Str(names),
                Column::Int(self.segments.iter().map(|s| s.level).collect()),
                Column::Int(self.segments.iter().map(|s| s.number).collect()),
                Column::Int(self.segments.iter().map(|s| s.high).collect()),
                Column::Int(self.segments.iter().map(|s| s.first).collect()),
                Column::Int(self.segments.iter().map(|s| s.last).collect()),
                Column::Int(self.segments.iter().map(|s| s.rows).collect()),
                Column::Int(self.segments.iter().map(|s| s.bytes).collect()),
                Column::Str(ranges),
            ],
            vec![vec![true; n]; 9],
        )
    }

    /// The manifest's declarations as JSON text: `keys` and `notes`.
    pub fn declared(&self) -> String {
        let notes: serde_json::Map<String, serde_json::Value> = self.notes.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
        serde_json::json!({ "keys": self.keys, "notes": notes }).to_string()
    }
}

/// The manifest of the log under `dir`, none where the log has none yet.
pub fn read(dir: &str) -> Result<Option<Manifest>, String> {
    let path = std::path::Path::new(dir).join(MANIFEST);
    if !path.exists() {
        return Ok(None);
    }
    let path = path.to_string_lossy().to_string();
    let r = Reader::open(&path).map_err(|e| format!("magpie: the manifest of '{}': {}", dir, e))?;
    let f = r.footer().clone();
    let cols = r.wanted(&COLUMNS.iter().map(|c| c.to_string()).collect::<Vec<_>>())?;
    let groups: Vec<usize> = (0..f.groups.len()).collect();
    let frame = r.read(&groups, &cols)?;
    let n = frame.nrows();
    let text = |c: usize, i: usize| -> String {
        match frame.data[c].as_ref() {
            Column::Str(t) => t.get(i).to_string(),
            _ => String::new(),
        }
    };
    let int = |c: usize, i: usize| -> i64 {
        match frame.data[c].as_ref() {
            Column::Int(v) => v[i],
            _ => 0,
        }
    };
    let segments = (0..n).map(|i| Segment { name: text(0, i), level: int(1, i), number: int(2, i), high: int(3, i), first: int(4, i), last: int(5, i), rows: int(6, i), bytes: int(7, i), ranges: text(8, i) }).collect();
    Ok(Some(Manifest { keys: f.keys.clone(), notes: f.notes.iter().cloned().collect(), segments }))
}

/// The manifest written: to `manifest.part`, flushed, renamed over
/// `manifest`, the directory flushed — one atomic replacement.
pub fn write(dir: &str, m: &Manifest) -> Result<(), String> {
    use std::io::Write;
    let frame = m.frame();
    let columns: Vec<(String, Kind, bool)> = frame.columns.iter().enumerate().map(|(i, n)| (n.clone(), Kind::of(frame.data[i].as_ref()), false)).collect();
    let mut w = Writer::new(columns);
    if frame.nrows() > 0 {
        w.group(&frame, 0, frame.nrows())?;
    }
    let notes: Vec<(String, String)> = m.notes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let bytes = w.finish(0, -1, &m.keys, &notes);
    let path = std::path::Path::new(dir).join(MANIFEST);
    let part = std::path::Path::new(dir).join(".manifest.part");
    let cannot = |what: &str, e: std::io::Error| format!("magpie: cannot {} the manifest of '{}': {}", what, dir, e);
    let mut file = std::fs::File::create(&part).map_err(|e| cannot("write", e))?;
    file.write_all(&bytes).map_err(|e| cannot("write", e))?;
    file.sync_all().map_err(|e| cannot("flush", e))?;
    drop(file);
    std::fs::rename(&part, &path).map_err(|e| cannot("place", e))?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_round_trips_and_is_replaced_whole() {
        let dir = std::env::temp_dir().join(format!("crow-magpie-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.to_string_lossy().to_string();
        assert_eq!(read(&dir).unwrap(), None);
        let mut m = Manifest { keys: vec!["day".into(), "symbol".into()], notes: BTreeMap::new(), segments: Vec::new() };
        m.notes.insert("time".into(), "timestamp".into());
        write(&dir, &m).unwrap();
        assert_eq!(read(&dir).unwrap(), Some(m.clone()));
        m.segments.push(Segment { name: "S000000000001.segment".into(), level: 1, number: 0, high: 3, first: 0, last: 99, rows: 100, bytes: 4096, ranges: r#"{"day":["2026-09-01","2026-09-02"]}"#.into() });
        m.segments.push(Segment { name: "S000000000002.segment".into(), level: 1, number: 4, high: 4, first: 100, last: 149, rows: 50, bytes: 2048, ranges: "{}".into() });
        write(&dir, &m).unwrap();
        let back = read(&dir).unwrap().unwrap();
        assert_eq!(back, m);
        assert_eq!(back.declared(), r#"{"keys":["day","symbol"],"notes":{"time":"timestamp"}}"#);
        assert!(!std::path::Path::new(&dir).join(".manifest.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
