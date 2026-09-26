//! Rehearsal (app-baseline-design.md §9, B3): a program's named tests run
//! with no connection. While one is active, `connect` hands back a
//! database of its own: a query is answered from a fixture recorded once
//! on purpose (`.crow/fixtures/<hash>.json`), else from rows generated
//! from what the database answered at build time (`.crow/schema`); a write
//! — `exec`, `insert`, a migration, a statement through `query` that
//! changes rows — is recorded as an effect with its payload, and the
//! program sees success. Nothing leaves the machine.

use crow_runtime::frame::{Column, CrowFrame};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Rows a query is answered with when no fixture holds it: three, their
/// cells counted by kind (1, 2, 3; 1.0, 2.0, 3.0; `name1`…; true, false).
const GENERATED_ROWS: usize = 3;

/// One rehearsal: what it knows of the program's databases, and what the
/// program did to them.
#[derive(Default)]
pub struct Rehearsal {
    /// Each statement the databases were asked about, and its columns.
    schema: Vec<(String, Vec<(String, String)>)>,
    fixtures: PathBuf,
    /// The writes, in order, as `<url> <statement> <params>`.
    pub effects: Mutex<Vec<String>>,
    /// The statements answered with generated rows (no fixture held them).
    pub generated: Mutex<Vec<String>>,
    /// The assertions that did not hold on rehearsal data.
    pub unheld: Mutex<Vec<String>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Arc<Rehearsal>>> = const { RefCell::new(None) };
}

/// The rehearsal running on this thread, if one is.
pub fn active() -> Option<Arc<Rehearsal>> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// Start a rehearsal of the project at `root`: its `.crow/schema` read.
pub fn begin(root: &Path) -> Arc<Rehearsal> {
    let mut schema = Vec::new();
    if let Ok(dir) = std::fs::read_dir(root.join(".crow").join("schema")) {
        let mut files: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            if let Some(rec) = std::fs::read_to_string(&f).ok().and_then(|t| crate::type_infer::schema_record::parse(&t)) {
                schema.push(rec);
            }
        }
    }
    let r = Arc::new(Rehearsal {
        schema,
        fixtures: root.join(".crow").join("fixtures"),
        effects: Mutex::new(Vec::new()),
        generated: Mutex::new(Vec::new()),
        unheld: Mutex::new(Vec::new()),
    });
    ACTIVE.with(|a| *a.borrow_mut() = Some(r.clone()));
    r
}

/// End the rehearsal on this thread.
pub fn end() {
    ACTIVE.with(|a| *a.borrow_mut() = None);
}

/// An assertion that did not hold: in a rehearsal it is kept to report
/// — the data it held on was the rehearsal's — and the test goes on.
/// Whether a rehearsal took it.
pub fn unheld(what: String) -> bool {
    match active() {
        Some(r) => {
            if let Ok(mut u) = r.unheld.lock() {
                u.push(what);
            }
            true
        }
        None => false,
    }
}

/// A builtin that reaches past the program — a shell command, a file
/// written, moved or removed — recorded instead of run, with what it
/// answers the program: a shell that ran and said nothing, a write that
/// landed. `None` outside a rehearsal, or for a builtin that stays real.
pub fn outside(name: &str, vals: &[crate::value::CrowValue]) -> Option<crate::value::CrowValue> {
    use crate::value::{CrowValue, Value};
    if !matches!(name, "shell" | "write" | "mkdir" | "rm" | "mv" | "cp") {
        return None;
    }
    let r = active()?;
    let args: Vec<String> = vals
        .iter()
        .map(|v| match &v.value {
            Value::Str(s) => {
                let s: String = s.chars().take(80).collect();
                format!("{:?}", s)
            }
            other => format!("{}", other),
        })
        .collect();
    if let Ok(mut e) = r.effects.lock() {
        e.push(format!("{}[{}]", name, args.join(", ")));
    }
    Some(match name {
        "shell" => {
            let mut m = std::collections::BTreeMap::new();
            m.insert("status".to_string(), CrowValue::ok(Value::Int(0)));
            m.insert("stdout".to_string(), CrowValue::ok(Value::Str(String::new())));
            m.insert("stderr".to_string(), CrowValue::ok(Value::Str(String::new())));
            CrowValue::ok(Value::Map(m))
        }
        _ => CrowValue::done(),
    })
}

/// A connection in rehearsal: `url` names it in the effects.
pub fn connect(url: &str) -> crow_db::DbHandle {
    let r = active().expect("a rehearsal is active");
    crow_db::DbHandle::new(RehearsalDb { url: url.to_string(), r })
}

/// Whether a statement changes rows or the schema, not only reads them.
fn writes(sql: &str) -> bool {
    let first = sql.trim_start().split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    matches!(first.as_str(), "INSERT" | "UPDATE" | "DELETE" | "CREATE" | "ALTER" | "DROP" | "REPLACE" | "TRUNCATE" | "UPSERT" | "MERGE")
}

/// A statement with each list of parameters as the one `?` it was written
/// as: `IN ?` handed a list runs as `IN (?, ?, ?)`, or `IN (NULL)` when the
/// list is empty, and the record keeps `IN ?`.
fn one_placeholder_per_list(sql: &str) -> String {
    let b = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'(' {
            // `(` then `?`s separated by commas, or a lone NULL, then `)`
            let mut j = i + 1;
            let skip = |j: &mut usize| while *j < b.len() && b[*j].is_ascii_whitespace() { *j += 1 };
            skip(&mut j);
            let list_end = if sql[j..].starts_with("NULL") {
                let mut k = j + 4;
                skip(&mut k);
                (k < b.len() && b[k] == b')').then_some(k)
            } else {
                let mut k = j;
                let mut seen = false;
                loop {
                    if k < b.len() && b[k] == b'?' {
                        seen = true;
                        k += 1;
                        skip(&mut k);
                        if k < b.len() && b[k] == b',' {
                            k += 1;
                            skip(&mut k);
                            continue;
                        }
                    }
                    break;
                }
                (seen && k < b.len() && b[k] == b')').then_some(k)
            };
            if let Some(end) = list_end {
                out.push('?');
                i = end + 1;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// Whether a statement as run matches one as recorded: the record's NULLs
/// stood in for spans the program interpolated, so each matches any text,
/// and a list of parameters is the one `?` it was written as.
fn matches_record(run: &str, recorded: &str) -> bool {
    matches_as_written(&one_placeholder_per_list(run), &one_placeholder_per_list(recorded))
}

fn matches_as_written(run: &str, recorded: &str) -> bool {
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

impl Rehearsal {
    fn columns_for(&self, sql: &str) -> Option<Vec<(String, String)>> {
        self.schema.iter().find(|(s, _)| matches_record(sql, s)).map(|(_, c)| c.clone())
    }

    /// A fixture recorded for the statement: `{"sql", "columns", "rows"}`.
    fn fixture(&self, sql: &str) -> Option<CrowFrame> {
        let path = self.fixtures.join(format!("{}.json", crate::type_infer::schema_record::fnv(&[&one_placeholder_per_list(sql)])));
        let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
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
            // a column that can hold NULL is marked `float?`
            data.push(match kind.trim_end_matches('?') {
                "int" => Column::Int(cells.iter().map(|c| c.as_i64().unwrap_or(0)).collect()),
                "float" | "datetime" => Column::Float(cells.iter().map(|c| c.as_f64().unwrap_or(0.0)).collect()),
                "bool" => Column::Bool(cells.iter().map(|c| c.as_bool().unwrap_or(false)).collect()),
                _ => Column::Str(cells.iter().map(|c| c.as_str().map(|s| s.to_string()).unwrap_or_else(|| c.to_string())).collect()),
            });
        }
        Some(CrowFrame::new(cols.into_iter().map(|(n, _)| n).collect(), data, validity))
    }

    /// Rows generated from the statement's recorded columns (none when it
    /// was never recorded: an empty frame).
    fn generated_rows(&self, sql: &str, n: usize) -> CrowFrame {
        let cols = self.columns_for(sql).unwrap_or_default();
        let mut data = Vec::new();
        let mut validity = Vec::new();
        for (name, kind) in &cols {
            // a column that can hold NULL holds one in the second row: the
            // first reads as a value, and a walk over the rows meets the
            // missing cell the program has to decide on
            validity.push((0..n).map(|i| !(kind.ends_with('?') && i == 1)).collect());
            data.push(match kind.trim_end_matches('?') {
                "int" => Column::Int((1..=n as i64).collect()),
                "float" | "datetime" => Column::Float((1..=n).map(|i| i as f64).collect()),
                "bool" => Column::Bool((0..n).map(|i| i % 2 == 0).collect()),
                _ => Column::Str((1..=n).map(|i| format!("{}{}", name, i)).collect()),
            });
        }
        CrowFrame::new(cols.into_iter().map(|(n, _)| n).collect(), data, validity)
    }

    fn answer(&self, url: &str, sql: &str, params: &[serde_json::Value]) -> CrowFrame {
        if writes(sql) {
            self.record(url, sql, params);
        }
        if let Some(f) = self.fixture(sql) {
            return f;
        }
        let first_line: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Ok(mut g) = self.generated.lock() {
            if !g.contains(&first_line) {
                g.push(first_line);
            }
        }
        // a write that answers (RETURNING) answers one row
        self.generated_rows(sql, if writes(sql) { 1 } else { GENERATED_ROWS })
    }

    fn record(&self, url: &str, sql: &str, params: &[serde_json::Value]) {
        let sql: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        // a password hash carries a fresh salt each run: it is said, not shown
        let params: Vec<serde_json::Value> = params
            .iter()
            .map(|p| match p.as_str() {
                Some(s) if s.starts_with("$argon2") => serde_json::Value::String("<password hash>".into()),
                _ => p.clone(),
            })
            .collect();
        let line = if params.is_empty() { format!("{} {}", url, sql) } else { format!("{} {} {}", url, sql, serde_json::Value::Array(params)) };
        if let Ok(mut e) = self.effects.lock() {
            e.push(line);
        }
    }
}

struct RehearsalDb {
    url: String,
    r: Arc<Rehearsal>,
}

struct RehearsalTx<'a> {
    db: &'a RehearsalDb,
}

impl crow_db::DbTx for RehearsalTx<'_> {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        Ok(self.db.r.answer(&self.db.url, sql, params))
    }
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.db.r.record(&self.db.url, sql, params);
        Ok(1)
    }
}

impl crow_db::CrowDb for RehearsalDb {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        Ok(self.r.answer(&self.url, sql, params))
    }
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.r.record(&self.url, sql, params);
        Ok(1)
    }
    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        let rows = frame.data.first().map_or(0, |c| c.len()) as i64;
        self.r.record(&self.url, &format!("insert into {} ({} rows: {})", table, rows, frame.columns.join(", ")), &[]);
        Ok(rows)
    }
    fn transact(&self, f: &dyn Fn(&dyn crow_db::DbTx) -> Result<(), String>) -> Result<(), String> {
        f(&RehearsalTx { db: self })
    }
    fn migrate(&self, dir: &str) -> Result<i64, String> {
        self.r.record(&self.url, &format!("migrate {}", dir), &[]);
        Ok(0)
    }
    fn backend_name(&self) -> &str {
        "rehearsal"
    }
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        self.r.columns_for(sql).ok_or_else(|| "rehearsal: the statement was never described".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_matches_its_record_with_spans_as_nulls() {
        assert!(matches_record("SELECT a FROM t WHERE x = ?", "SELECT a FROM t WHERE x = ?"));
        assert!(matches_record("SELECT a FROM t_2026 WHERE x = 5", "SELECT a FROM t_NULL WHERE x = NULL"));
        assert!(!matches_record("SELECT b FROM t", "SELECT a FROM t"));
        assert!(writes("  insert into t values (1)"));
        assert!(!writes("SELECT 1"));
    }

    /// Generated rows hold NULL in the second row of a column that can hold
    /// one, and only there
    #[test]
    fn a_column_that_can_hold_null_holds_one_in_its_second_row() {
        let mut r = Rehearsal::default();
        r.schema.push(("SELECT a, b FROM t".to_string(), vec![("a".to_string(), "int".to_string()), ("b".to_string(), "float?".to_string())]));
        let f = r.generated_rows("SELECT a, b FROM t", 3);
        assert_eq!(*f.validity[0], vec![true, true, true]);
        assert_eq!(*f.validity[1], vec![true, false, true]);
    }

    /// A list handed to `IN ?` runs as `IN (?, ?, ?)` — or `IN (NULL)`,
    /// empty — and still matches its record, as finch's scanner query did
    /// not: the rehearsal answered it with no columns, and the scanner's
    /// join failed
    #[test]
    fn a_list_parameter_matches_the_one_it_was_written_as() {
        let recorded = "SELECT symbol FROM bars WHERE date = toDate(?) AND dataset_id IN ? AND symbol NOT IN ?";
        assert!(matches_record("SELECT symbol FROM bars WHERE date = toDate(?) AND dataset_id IN (?, ?, ?) AND symbol NOT IN (NULL)", recorded));
        assert!(matches_record("SELECT symbol FROM bars WHERE date = toDate(?) AND dataset_id IN (?) AND symbol NOT IN (?,?)", recorded));
        assert!(matches_record("INSERT INTO t (a, b) VALUES (?, ?)", "INSERT INTO t (a, b) VALUES (?, ?)"));
        assert!(!matches_record("SELECT symbol FROM bars WHERE date = toDate(?) AND dataset_id = ?", recorded));
        assert_eq!(one_placeholder_per_list("f(a, ?) IN ( ?, ? ) (x)"), "f(a, ?) IN ? (x)");
    }
}
