//! A program's databases answered from what a recording kept, with no
//! connection: the statement logic a rehearsal (`crow check`) and a replay
//! (`crow test --fixtures`) share — how a statement as it runs matches the
//! one it was written as, where its fixture and its columns' record live,
//! the rows a write answers — and the replay's database, which `connect`
//! hands back when `CROW_REPLAY` names a project's `.crow` directory, in
//! either mode (spec, crow test --fixtures; crow-check.md, rehearsals). The
//! hash that names a fixture, the replay's switch and the misses it notes
//! are the runtime's (`crow_runtime::fixtures`), shared with a request's
//! fixture.

use crow_runtime::fixtures::{fnv, note_miss, replaying};
use crow_runtime::frame::{Column, CrowFrame};
use std::path::{Path, PathBuf};

/// Whether a statement can only read: it starts as a query does and names
/// no write anywhere (a `WITH … INSERT` writes). Only such a statement goes
/// to a real database in a recording, and only it is answered from a
/// fixture in a replay.
pub fn reads_only(sql: &str) -> bool {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in sql.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' || c == '`' => quote = Some(c),
            None if c.is_ascii_alphanumeric() || c == '_' => word.push(c.to_ascii_uppercase()),
            None => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let first = words.first().map(String::as_str).unwrap_or("");
    matches!(first, "SELECT" | "WITH" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN")
        && !words.iter().any(|w| {
            matches!(w.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "UPSERT" | "REPLACE" | "TRUNCATE" | "DROP" | "ALTER" | "CREATE" | "GRANT" | "REVOKE" | "ATTACH" | "DETACH" | "OPTIMIZE" | "KILL" | "SYSTEM" | "RENAME")
        })
}


/// Whether a statement changes rows or the schema, not only reads them.
pub fn writes(sql: &str) -> bool {
    let first = sql.trim_start().split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    matches!(first.as_str(), "INSERT" | "UPDATE" | "DELETE" | "CREATE" | "ALTER" | "DROP" | "REPLACE" | "TRUNCATE" | "UPSERT" | "MERGE")
}


/// A statement with each list of parameters as the one `?` it was written
/// as: `IN ?` handed a list runs as `IN (?, ?, ?)`, or `IN (NULL)` when the
/// list is empty, and the record keeps `IN ?`.
pub fn one_placeholder_per_list(sql: &str) -> String {
    // (by character: a statement holds text like '—', which bytes would cut)
    let c: Vec<char> = sql.chars().collect();
    let skip = |k: &mut usize| {
        while *k < c.len() && c[*k].is_whitespace() {
            *k += 1
        }
    };
    // a list goes where `IN ?` was written
    let after_in = |i: usize| {
        let mut j = i;
        while j > 0 && c[j - 1].is_whitespace() {
            j -= 1;
        }
        j >= 2 && c[j - 2].eq_ignore_ascii_case(&'I') && c[j - 1].eq_ignore_ascii_case(&'N') && (j == 2 || !(c[j - 3].is_alphanumeric() || c[j - 3] == '_'))
    };
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < c.len() {
        if c[i] == '(' && after_in(i) {
            // `(` then `?`s separated by commas, or a lone NULL, then `)`
            let mut j = i + 1;
            skip(&mut j);
            let null = c.len() >= j + 4 && c[j..j + 4].iter().collect::<String>() == "NULL";
            let list_end = if null {
                let mut k = j + 4;
                skip(&mut k);
                (k < c.len() && c[k] == ')').then_some(k)
            } else {
                let mut k = j;
                let mut seen = false;
                while k < c.len() && c[k] == '?' {
                    seen = true;
                    k += 1;
                    skip(&mut k);
                    if k < c.len() && c[k] == ',' {
                        k += 1;
                        skip(&mut k);
                    } else {
                        break;
                    }
                }
                (seen && k < c.len() && c[k] == ')').then_some(k)
            };
            if let Some(end) = list_end {
                out.push('?');
                i = end + 1;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}


/// Whether a statement as run matches one as recorded: the record's NULLs
/// stood in for spans the program interpolated, so each matches any text,
/// and a list of parameters is the one `?` it was written as.
pub fn matches_record(run: &str, recorded: &str) -> bool {
    matches_as_written(&one_placeholder_per_list(run), &one_placeholder_per_list(recorded))
}

pub fn matches_as_written(run: &str, recorded: &str) -> bool {
    let parts: Vec<&str> = recorded.split("NULL").collect();
    if parts.len() == 1 {
        return run == recorded;
    }
    let mut rest = match run.strip_prefix(parts[0]) {
        Some(r) => r,
        None => return false,
    };
    let last = parts[parts.len() - 1];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}


/// Where a statement's fixture is kept under a project's `.crow`: one file
/// per statement as it was written, a list of parameters its one `?`.
pub fn fixture_path(crow_dir: &Path, sql: &str) -> PathBuf {
    crow_dir.join("fixtures").join(format!("{}.json", fnv(&[&one_placeholder_per_list(sql)])))
}

/// A fixture's rows, from its file's text: `{"sql", "columns": [[name,
/// kind]…], "rows": [[…]…]}`, a column that can hold NULL marked `float?`.
pub fn fixture_frame(text: &str) -> Option<CrowFrame> {
    let j: serde_json::Value = serde_json::from_str(text).ok()?;
    let cols: Vec<(String, String)> = j
        .get("columns")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            let p = c.as_array()?;
            Some((p.first()?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))
        })
        .collect();
    let rows = j.get("rows")?.as_array()?;
    let mut data = Vec::new();
    let mut validity = Vec::new();
    for (i, (_, kind)) in cols.iter().enumerate() {
        let cells: Vec<&serde_json::Value> = rows.iter().map(|r| r.get(i).unwrap_or(&serde_json::Value::Null)).collect();
        validity.push(cells.iter().map(|c| !c.is_null()).collect());
        data.push(match kind.trim_end_matches('?') {
            "int" => Column::Int(cells.iter().map(|c| c.as_i64().unwrap_or(0)).collect()),
            "float" | "datetime" => Column::Float(cells.iter().map(|c| c.as_f64().unwrap_or(0.0)).collect()),
            "bool" => Column::Bool(cells.iter().map(|c| c.as_bool().unwrap_or(false)).collect()),
            _ => Column::Str(cells.iter().map(|c| c.as_str().map(|s| s.to_string()).unwrap_or_else(|| c.to_string())).collect()),
        });
    }
    Some(CrowFrame::new(cols.into_iter().map(|(n, _)| n).collect(), data, validity))
}

/// The statement a schema record holds, and its columns, from its file's
/// text (`{"via", "sql", "columns": [[name, kind]…]}`).
pub fn parse_record(text: &str) -> Option<(String, Vec<(String, String)>)> {
    let j: serde_json::Value = serde_json::from_str(text).ok()?;
    let sql = j.get("sql")?.as_str()?.to_string();
    let cols = j
        .get("columns")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            let pair = c.as_array()?;
            Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()))
        })
        .collect();
    Some((sql, cols))
}

/// Every record under a project's `.crow/schema`, in file order: what the
/// databases answered each statement was, at build time.
pub fn records(crow_dir: &Path) -> Vec<(String, Vec<(String, String)>)> {
    let mut out = Vec::new();
    if let Ok(dir) = std::fs::read_dir(crow_dir.join("schema")) {
        let mut files: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            if let Some(rec) = std::fs::read_to_string(&f).ok().and_then(|t| parse_record(&t)) {
                out.push(rec);
            }
        }
    }
    out
}

/// The columns a statement's record names, when one does.
pub fn columns_for(records: &[(String, Vec<(String, String)>)], sql: &str) -> Option<Vec<(String, String)>> {
    records.iter().find(|(s, _)| matches_record(sql, s)).map(|(_, c)| c.clone())
}

/// Rows generated from a statement's recorded columns: 1, 2, 3 for an int,
/// 1.0, 2.0, 3.0 for a float, `name1`… for text, true, false, true for a
/// flag — and NULL in the second row of a column that can hold one, so a
/// walk over the rows meets the missing cell the program has to decide on.
pub fn generated_rows(cols: &[(String, String)], n: usize) -> CrowFrame {
    let mut data = Vec::new();
    let mut validity = Vec::new();
    for (name, kind) in cols {
        validity.push((0..n).map(|i| !(kind.ends_with('?') && i == 1)).collect());
        data.push(match kind.trim_end_matches('?') {
            "int" => Column::Int((1..=n as i64).collect()),
            "float" | "datetime" => Column::Float((1..=n).map(|i| i as f64).collect()),
            "bool" => Column::Bool((0..n).map(|i| i % 2 == 0).collect()),
            _ => Column::Str((1..=n).map(|i| format!("{}{}", name, i)).collect()),
        });
    }
    CrowFrame::new(cols.iter().map(|(n, _)| n.clone()).collect(), data, validity)
}

/// The recordings that answer a connection to `url` in a replay: a database
/// server's (Postgres, ClickHouse), which a machine without it cannot reach.
/// A sqlite file is the program's own and opens as always, so a test that
/// writes to it reads back what it wrote.
pub fn replaying_for(url: &str) -> Option<PathBuf> {
    let server = ["postgres://", "postgresql://", "clickhouse://"].iter().any(|s| url.starts_with(s));
    if server { replaying() } else { None }
}

/// A database answered from what recordings kept: a statement that reads
/// is its fixture, or the query's error, naming the statement; a write
/// answers as one that landed — one row generated from its record when it
/// returns rows — and nothing is changed anywhere.
pub struct Replay {
    url: String,
    crow_dir: PathBuf,
    records: Vec<(String, Vec<(String, String)>)>,
}

impl Replay {
    pub fn open(url: &str, crow_dir: &Path) -> Replay {
        Replay { url: url.to_string(), crow_dir: crow_dir.to_path_buf(), records: records(crow_dir) }
    }

    fn answer(&self, sql: &str) -> Result<CrowFrame, String> {
        if !reads_only(sql) {
            return Ok(generated_rows(&columns_for(&self.records, sql).unwrap_or_default(), if writes(sql) { 1 } else { 0 }));
        }
        let text = std::fs::read_to_string(fixture_path(&self.crow_dir, sql)).ok();
        text.as_deref().and_then(fixture_frame).ok_or_else(|| {
            let head: String = sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
            note_miss(&format!("no fixture held `{}` — `crow check --record-fixtures` records it where the database is", head));
            format!("no fixture holds `{}` — `crow check --record-fixtures` records it where the database is", head)
        })
    }
}

struct ReplayTx<'a>(&'a Replay);

impl crate::DbTx for ReplayTx<'_> {
    fn query(&self, sql: &str, _params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.0.answer(sql)
    }
    fn exec(&self, _sql: &str, _params: &[serde_json::Value]) -> Result<i64, String> {
        Ok(1)
    }
}

impl crate::CrowDb for Replay {
    fn query(&self, sql: &str, _params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.answer(sql)
    }
    fn exec(&self, _sql: &str, _params: &[serde_json::Value]) -> Result<i64, String> {
        Ok(1)
    }
    fn insert(&self, _table: &str, frame: &CrowFrame) -> Result<i64, String> {
        Ok(frame.nrows() as i64)
    }
    fn transact(&self, f: &dyn Fn(&dyn crate::DbTx) -> Result<(), String>) -> Result<(), String> {
        f(&ReplayTx(self))
    }
    fn migrate(&self, _dir: &str) -> Result<i64, String> {
        Ok(0)
    }
    /// As the program's own connection shows: `<db:postgres>`.
    fn backend_name(&self) -> &str {
        match self.url.split("://").next().unwrap_or("") {
            "postgresql" => "postgres",
            scheme => scheme,
        }
    }
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        columns_for(&self.records, sql).ok_or_else(|| "a replay: the statement was never described".to_string())
    }
    /// (the records are what the compiler's describe answered, whole: read
    /// for more, the statement would be asked of no database)
    fn describes_fully(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CrowDb as _;

    fn project(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow_replay_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fixtures")).unwrap();
        std::fs::create_dir_all(dir.join("schema")).unwrap();
        dir
    }

    /// A read is answered from its fixture — a NULL kept, a list of
    /// parameters matching the `?` it was written as — one no fixture holds
    /// is the query's error naming the statement, and a write answers as
    /// one that landed, its returned row generated from its record
    #[test]
    fn a_replay_answers_from_what_was_kept() {
        let dir = project("answers");
        let sql = "SELECT sym, px FROM marks WHERE sym IN ? ORDER BY sym";
        let fixture = serde_json::json!({ "sql": sql, "columns": [["sym", "str"], ["px", "float?"]], "rows": [["AB", 1.5], ["CD", null]] });
        std::fs::write(fixture_path(&dir, sql), fixture.to_string()).unwrap();
        std::fs::write(dir.join("schema").join("a.json"), serde_json::json!({ "via": "book", "sql": "INSERT INTO marks VALUES (?) RETURNING id", "columns": [["id", "int"]] }).to_string()).unwrap();
        let db = Replay::open("postgres://u@h/db", &dir);
        let f = db.query("SELECT sym, px FROM marks WHERE sym IN (?, ?) ORDER BY sym", &[]).unwrap();
        assert_eq!(f.nrows(), 2);
        assert_eq!(f.validity[1].to_vec(), vec![true, false]);
        let missing = db.query("SELECT 1 AS one", &[]).unwrap_err();
        assert!(missing.starts_with("no fixture holds `SELECT 1 AS one` — `crow check --record-fixtures` records it"), "{}", missing);
        assert_eq!(db.exec("DELETE FROM marks", &[]).unwrap(), 1);
        let back = db.query("INSERT INTO marks VALUES (?) RETURNING id", &[]).unwrap();
        assert_eq!((back.nrows(), back.columns.clone()), (1, vec!["id".to_string()]));
        assert_eq!(db.backend_name(), "postgres");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A replay's description is its record, the compiler's whole answer:
    /// the statement is read for nothing more, which would ask the replay
    /// for the tables' columns — a fixture no recording holds
    #[cfg(feature = "sqlite")]
    #[test]
    fn a_replays_description_is_its_record() {
        let dir = project("describe");
        std::fs::write(dir.join("schema").join("b.json"), serde_json::json!({ "via": "book", "sql": "SELECT sym, px FROM marks WHERE sym IN ?", "columns": [["sym", "str"], ["px", "float?"]] }).to_string()).unwrap();
        let db = crate::DbHandle::new(Replay::open("postgres://u@h/db", &dir));
        let cols = db.describe("SELECT sym, px FROM marks WHERE sym IN ?").unwrap();
        assert_eq!(cols, vec![("sym".to_string(), "str".to_string()), ("px".to_string(), "float?".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
