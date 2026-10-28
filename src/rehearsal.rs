//! Rehearsal (app-baseline-design.md §9, B3): a program's named tests run
//! with no connection. While one is active, `connect` hands back a
//! database of its own: a query is answered from a fixture recorded once
//! on purpose (`.crow/fixtures/<hash>.json`), else from rows generated
//! from what the database answered at build time (`.crow/schema`); a write
//! — a statement through `query` that changes rows, a bulk `insert` — is
//! recorded as an effect with its payload, and the program sees success.
//! Nothing leaves the machine.
//!
//! A recording (`crow check --record-fixtures`) is a rehearsal whose reads
//! go to the real databases: what each statement answers first is kept as
//! its fixture, and a later read of it is answered from that, as the
//! rehearsals after will be. Writes are recorded as in any rehearsal — a
//! recording never changes a database.

use crow_db::fixtures::{one_placeholder_per_list, reads_only, writes};
use crow_runtime::frame::{Column, CrowFrame};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Rows a query is answered with when no fixture holds it: three, their
/// cells counted by kind (1, 2, 3; 1.0, 2.0, 3.0; `name1`…; true, false).
const GENERATED_ROWS: usize = 3;

/// Rows a recorded fixture keeps of what its database answered: a day of
/// one-second bars is too many to keep in a project.
pub const FIXTURE_ROWS: usize = 10_000;

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
    /// Recording: reads go to the real databases, and what each answers is
    /// kept.
    recording: bool,
    /// The real connections a recording reads through, by URL.
    real: Mutex<std::collections::BTreeMap<String, Result<crow_db::DbHandle, String>>>,
    /// What each statement read answered first, by the statement as it was
    /// written: the statement as run, the rows kept, the rows answered.
    pub recorded: Mutex<std::collections::BTreeMap<String, (String, CrowFrame, usize)>>,
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
    begin_as(root, false)
}

/// Start a recording of the project at `root`: a rehearsal whose reads the
/// real databases answer.
pub fn begin_recording(root: &Path) -> Arc<Rehearsal> {
    begin_as(root, true)
}

fn begin_as(root: &Path, recording: bool) -> Arc<Rehearsal> {
    let schema = crow_db::fixtures::records(&root.join(".crow"));
    let r = Arc::new(Rehearsal {
        schema,
        fixtures: root.join(".crow").join("fixtures"),
        effects: Mutex::new(Vec::new()),
        generated: Mutex::new(Vec::new()),
        unheld: Mutex::new(Vec::new()),
        recording,
        real: Mutex::new(std::collections::BTreeMap::new()),
        recorded: Mutex::new(std::collections::BTreeMap::new()),
    });
    ACTIVE.with(|a| *a.borrow_mut() = Some(r.clone()));
    // the requests go through `request`'s own body (crow_runtime::fixtures):
    // a recording keeps what they answer, a rehearsal answers from what one
    // kept and stands in for the rest
    let crow_dir = root.join(".crow");
    crow_runtime::fixtures::record_to(if recording { Some(crow_dir.clone()) } else { None });
    crow_runtime::fixtures::rehearse_in(if recording { None } else { Some(crow_dir) });
    r
}

/// End the rehearsal on this thread.
pub fn end() {
    ACTIVE.with(|a| *a.borrow_mut() = None);
    crow_runtime::fixtures::record_to(None);
    crow_runtime::fixtures::rehearse_in(None);
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
/// landed. `None` outside a rehearsal, or for a builtin that stays real —
/// a request, which its own body answers: from the fixture a recording
/// kept, as `200` with nothing to say in a rehearsal that holds none, and
/// sent in a recording, its answer kept (`crow_runtime::fixtures`).
pub fn outside(name: &str, vals: &[crate::value::CrowValue]) -> Option<crate::value::CrowValue> {
    use crate::value::{CrowValue, Value};
    // (outside a rehearsal nothing is recorded: asked first, since every
    // call by name comes through here, and reading the kinds first had put
    // a scan of the table behind each one)
    let r = active()?;
    // (recorded: what its kind says — a file written, a command run; a
    // request is noted here and runs through its own fixtures)
    let kinds = crate::table::effects_of(name);
    let recorded = kinds.iter().any(|e| e.recorded());
    let noted = kinds.contains(&crate::effects::Effect::Network);
    if !recorded && !noted {
        return None;
    }
    let shown = |v: &CrowValue| match &v.value {
        Value::Str(s) => {
            let s: String = s.chars().take(80).collect();
            format!("{:?}", s)
        }
        other => format!("{}", other),
    };
    let mut args: Vec<String> = vals.iter().map(shown).collect();
    if name == "request" && vals.len() == 9 {
        // a request's options by name, the ones it was given
        const OPTIONS: [&str; 6] = ["headers", "query", "form", "files", "save", "timeout"];
        args = vals[..3].iter().map(shown).collect();
        for (option, v) in OPTIONS.iter().zip(&vals[3..]) {
            let given = match &v.value {
                Value::Str(s) => !s.is_empty(),
                Value::Map(m) => !m.is_empty(),
                Value::Float(f) => *f != 0.0,
                Value::Int(n) => *n != 0,
                _ => true,
            };
            if given {
                args.push(format!("{} = {}", option, shown(v)));
            }
        }
    }
    if let Ok(mut e) = r.effects.lock() {
        e.push(format!("{}[{}]", name, args.join(", ")));
    }
    if !recorded {
        return None;
    }
    Some(if kinds.contains(&crate::effects::Effect::Shell) {
        // a command that ran and said nothing
        let mut m = std::collections::BTreeMap::new();
        m.insert("status".to_string(), CrowValue::ok(Value::Int(0)));
        m.insert("stdout".to_string(), CrowValue::ok(Value::Str(String::new())));
        m.insert("stderr".to_string(), CrowValue::ok(Value::Str(String::new())));
        CrowValue::ok(Value::map(m))
    } else {
        // a file operation answers empty text, as a real one does
        CrowValue::ok(Value::Str(String::new()))
    })
}

/// A connection in rehearsal: `url` names it in the effects.
pub fn connect(url: &str) -> crow_db::DbHandle {
    let r = active().expect("a rehearsal is active");
    crow_db::DbHandle::new(RehearsalDb { url: url.to_string(), r })
}

impl Rehearsal {
    fn columns_for(&self, sql: &str) -> Option<Vec<(String, String)>> {
        crow_db::fixtures::columns_for(&self.schema, sql)
    }

    /// A fixture recorded for the statement: `{"sql", "columns", "rows"}`.
    fn fixture(&self, sql: &str) -> Option<CrowFrame> {
        let path = self.fixtures.join(format!("{}.json", crow_runtime::fixtures::fnv(&[&one_placeholder_per_list(sql)])));
        crow_db::fixtures::fixture_frame(&std::fs::read_to_string(path).ok()?)
    }

    /// Rows generated from the statement's recorded columns (none when it
    /// was never recorded: an empty frame).
    fn generated_rows(&self, sql: &str, n: usize) -> CrowFrame {
        crow_db::fixtures::generated_rows(&self.columns_for(sql).unwrap_or_default(), n)
    }

    fn answer(&self, url: &str, sql: &str, params: &[serde_json::Value]) -> CrowFrame {
        if writes(sql) {
            self.record(url, sql, params);
        } else if self.recording && reads_only(sql) {
            if let Some(f) = self.read_real(url, sql, params) {
                return f;
            }
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

    /// A read in a recording: the real database answers it once, and that
    /// answer — its first FIXTURE_ROWS rows — is the statement's from then
    /// on, as the fixture it becomes will be. `None` when the database
    /// cannot answer (it is not there): the rehearsal answers.
    fn read_real(&self, url: &str, sql: &str, params: &[serde_json::Value]) -> Option<CrowFrame> {
        let key = one_placeholder_per_list(sql);
        if let Some((_, f, _)) = self.recorded.lock().ok()?.get(&key) {
            return Some(f.clone());
        }
        let db = {
            let mut real = self.real.lock().ok()?;
            real.entry(url.to_string()).or_insert_with(|| crow_db::connect(url, Some(1))).clone().ok()?
        };
        let frame = db.query(sql, params).ok()?;
        let answered = frame.nrows();
        let kept = if answered > FIXTURE_ROWS { frame.head(FIXTURE_ROWS) } else { frame };
        self.recorded.lock().ok()?.insert(key, (sql.to_string(), kept.clone(), answered));
        Some(kept)
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

/// The fixtures a recording kept, written to `.crow/fixtures`: one file per
/// statement (`{"sql", "columns", "rows", "answered"}`), a statement a
/// scenario of this run recorded first keeping its first answer. Each
/// statement written, with the rows kept and the rows its database answered.
pub fn keep_fixtures(r: &Rehearsal) -> Result<Vec<(String, usize, usize)>, String> {
    static KEPT: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());
    let recorded = r.recorded.lock().map_err(|e| e.to_string())?;
    let mut kept = KEPT.lock().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (key, (_, frame, answered)) in recorded.iter() {
        if !kept.insert(key.clone()) {
            continue;
        }
        std::fs::create_dir_all(&r.fixtures).map_err(|e| e.to_string())?;
        let columns: Vec<serde_json::Value> = frame
            .columns
            .iter()
            .zip(frame.data.iter())
            .map(|(name, col)| {
                let kind = match col.as_ref() {
                    Column::Int(_) => "int",
                    Column::Float(_) => "float",
                    Column::Bool(_) => "bool",
                    _ => "str",
                };
                serde_json::json!([name, kind])
            })
            .collect();
        let rows: Vec<serde_json::Value> = (0..frame.nrows())
            .map(|i| {
                serde_json::Value::Array(
                    frame
                        .data
                        .iter()
                        .zip(frame.validity.iter())
                        .map(|(col, valid)| {
                            if !valid[i] {
                                return serde_json::Value::Null;
                            }
                            match col.as_ref() {
                                Column::Int(v) => serde_json::json!(v[i]),
                                Column::Float(v) => serde_json::json!(v[i]),
                                Column::Bool(v) => serde_json::json!(v[i]),
                                Column::Str(v) => serde_json::json!(&v[i]),
                                Column::DateTime(millis, offsets) => serde_json::json!(crow_runtime::frame::format_datetime(millis[i], offsets[i])),
                                Column::Vectors(w, v) => serde_json::json!(&v[i * w..(i + 1) * w]),
                            }
                        })
                        .collect(),
                )
            })
            .collect();
        let doc = serde_json::json!({ "sql": key, "columns": columns, "rows": rows, "answered": answered });
        let path = r.fixtures.join(format!("{}.json", crow_runtime::fixtures::fnv(&[key])));
        std::fs::write(&path, serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n").map_err(|e| e.to_string())?;
        out.push((key.clone(), frame.nrows(), *answered));
    }
    Ok(out)
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
        rehearsed_query(&self.db.r, &self.db.url, sql, params)
    }
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.db.r.record(&self.db.url, sql, params);
        Ok(1)
    }
}

/// Every statement comes through `query`: one that writes is recorded as
/// the write it is and answers no rows (the rows a `RETURNING` clause
/// reads back are generated, as a read's are); a read is answered with
/// generated rows.
fn rehearsed_query(r: &Rehearsal, url: &str, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
    match crow_db::describe::classify(sql) {
        crow_db::describe::Does::Writes { reads_back } => {
            r.record(url, sql, params);
            if reads_back {
                return Ok(r.answer(url, sql, params));
            }
            Ok(CrowFrame::new(vec![], vec![], vec![]))
        }
        _ => Ok(r.answer(url, sql, params)),
    }
}

impl crow_db::CrowDb for RehearsalDb {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        rehearsed_query(&self.r, &self.url, sql, params)
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
    use crow_db::fixtures::matches_record;

    #[test]
    fn a_statement_matches_its_record_with_spans_as_nulls() {
        assert!(matches_record("SELECT a FROM t WHERE x = ?", "SELECT a FROM t WHERE x = ?"));
        assert!(matches_record("SELECT a FROM t_2026 WHERE x = 5", "SELECT a FROM t_NULL WHERE x = NULL"));
        assert!(!matches_record("SELECT b FROM t", "SELECT a FROM t"));
        assert!(writes("  insert into t values (1)"));
        assert!(!writes("SELECT 1"));
    }

    /// A recording reads the real database and keeps what it answered; its
    /// writes are recorded, not made; a rehearsal after reads the fixture
    #[test]
    fn a_recording_keeps_what_the_database_answered() {
        let root = std::env::temp_dir().join(format!("crow_recording_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("real.db");
        let url = format!("sqlite://{}", path.display());
        let real = crow_db::connect(&url, Some(1)).unwrap();
        real.exec("CREATE TABLE marks (sym TEXT NOT NULL, px REAL)", &[]).unwrap();
        real.exec("INSERT INTO marks VALUES ('AB', 1.5), ('CD', NULL)", &[]).unwrap();
        let sql = "SELECT sym, px FROM marks WHERE sym IN ? ORDER BY sym -- a recording";
        let r = begin_recording(&root);
        let db = connect(&url);
        let f = db.query(sql, &[serde_json::json!(["AB", "CD"])]).unwrap();
        assert_eq!(f.nrows(), 2);
        db.exec("INSERT INTO marks VALUES ('EF', 2.0)", &[]).unwrap();
        let kept = keep_fixtures(&r).unwrap();
        end();
        assert_eq!(kept.len(), 1, "{:?}", kept);
        assert_eq!(real.query("SELECT count(*) AS n FROM marks", &[]).unwrap().nrows(), 1);
        let n = real.query("SELECT count(*) AS n FROM marks", &[]).unwrap();
        assert!(matches!(n.data[0].as_ref(), Column::Int(v) if v[0] == 2), "the recording wrote to the database");
        assert!(r.effects.lock().unwrap().iter().any(|e| e.contains("INSERT INTO marks VALUES ('EF', 2.0)")));
        // a rehearsal after answers from the fixture: the rows, a NULL kept
        let _r2 = begin(&root);
        let g = connect(&url).query(sql, &[serde_json::json!(["AB"])]).unwrap();
        end();
        assert_eq!(g.nrows(), 2);
        assert_eq!(g.validity[1].to_vec(), vec![true, false]);
        assert!(!reads_only("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x"));
        assert!(reads_only("SELECT 'insert' AS word FROM t"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A request in a rehearsal is recorded, not sent: its URL, method and
    /// body, then the options it was given, by name; it answers as one that
    /// landed, with nothing to say, and nothing is saved
    #[test]
    fn a_rehearsed_request_is_recorded_with_its_options_by_name() {
        use crate::value::{CrowValue, Value};
        let text = |s: &str| CrowValue::ok(Value::Str(s.to_string()));
        let map = |pairs: &[(&str, &str)]| CrowValue::ok(Value::map(pairs.iter().map(|(k, v)| (k.to_string(), text(v))).collect()));
        let r = begin(Path::new("/no/such/project"));
        let vals = vec![
            text("https://example.com/upload"),
            text("POST"),
            text(""),
            map(&[("authorization", "Bearer k")]),
            map(&[]),
            map(&[("title", "July")]),
            map(&[("report", "july.csv")]),
            text("kept.bin"),
            CrowValue::ok(Value::Float(5.0)),
        ];
        // (the request's own body answers it: from a fixture a recording
        // kept, else as a request that landed with nothing to say)
        let answer = outside("request", &vals);
        end();
        let recorded = r.effects.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![r#"request["https://example.com/upload", "POST", "", headers = [authorization = 'Bearer k'], form = [title = 'July'], files = [report = 'july.csv'], save = "kept.bin", timeout = 5]"#.to_string()]
        );
        assert!(answer.is_none(), "a request is answered by its own body: {:?}", answer);
        assert!(!Path::new("kept.bin").exists());
    }

    /// Generated rows hold NULL in the second row of a column that can hold
    /// one, and only there
    #[test]
    fn a_column_that_can_hold_null_holds_one_in_its_second_row() {
        let mut r = Rehearsal::default();
        r.schema.push(("SELECT a, b FROM t".to_string(), vec![("a".to_string(), "int".to_string()), ("b".to_string(), "float?".to_string())]));
        let f = r.generated_rows("SELECT a, b FROM t", 3);
        assert_eq!(f.validity[0].to_vec(), vec![true, true, true]);
        assert_eq!(f.validity[1].to_vec(), vec![true, false, true]);
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
        // a call with one parameter is not a list
        assert_eq!(one_placeholder_per_list("toDate(?) AND x NOT IN (?, ?)"), "toDate(?) AND x NOT IN ?");
        // text beyond ASCII is read by character (finch's '—' panicked a byte slice)
        assert_eq!(one_placeholder_per_list("SELECT coalesce(s, '—') AS t FROM x WHERE y IN (?, ?) AND z = 'é'"), "SELECT coalesce(s, '—') AS t FROM x WHERE y IN ? AND z = 'é'");
    }
}
