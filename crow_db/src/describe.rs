//! What a database says a statement answers — `query`'s provider
//! (docs/extending-crow.md, section 9): the compiler hands it the literal
//! URL of the connection, the literal statement and the `CREATE TABLE`s
//! the program declares on that connection, and reads back each column's
//! name and kind (`int`, `float`, `str`, `bool`, `datetime`, with `?` where
//! the column can hold NULL). Asking keeps one connection per URL for the
//! build and one answer per statement for a minute; a SQLite database that
//! does not exist yet is described from a scratch copy the program's own
//! statements build, so asking creates nothing; a driver written in Crow
//! (ClickHouse, Postgres) is asked by a process of its own (`crow db
//! describe`); what SQLite does not type is typed from the statement
//! (`sql_types`), and which columns can hold NULL is read from the
//! statement and the tables' declarations.

thread_local! {
    static DESCRIBE_HANDLES: std::cell::RefCell<std::collections::HashMap<String, Result<crate::DbHandle, String>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// What each database answered for each statement, and when: a test
/// file's programs, each lowered alone, ask again about the same
/// statements within seconds (a run of finch's nav.co sent 147 describes
/// for 19 queries), and a server that is asked less is a server under less
/// load. One cache for the process, whatever thread lowers: `crow check`
/// lowers a part per worker, and a cache per thread sent finch's 26
/// statements 578 times. An answer is kept a minute, so a long-lived
/// process (the language server) sees a changed table soon. A refusal is
/// kept as an answer is: a statement of one database's dialect is asked
/// of every connection a program has when none is named, and the one that
/// refuses it would be asked again by every worker.
static DESCRIBE_ANSWERS: std::sync::Mutex<Option<std::collections::HashMap<(String, String), (std::time::Instant, Result<Vec<(String, String)>, String>)>>> = std::sync::Mutex::new(None);

/// How long a database's answer about a statement is kept
const DESCRIBE_KEPT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(test)]
thread_local! {
    /// How many times this thread asked a database to describe a statement
    pub static DESCRIBES_ASKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Describe a statement against a database URL, keeping one connection per
/// URL for the build. A SQLite file that does not exist yet is not created
/// by asking: only the program makes its database. For a program that makes
/// its own tables, a SQLite database that
/// does not exist yet — or one missing a table the statement reads — is
/// described from a scratch copy the program's `CREATE` statements build,
/// so a new app's columns are typed on its first build as on every later one.
/// `IN ?` as the list of one it is sent as, `IN (?)`: a list parameter's
/// `?` is only ever sent expanded, and no database prepares `IN $2` (finch's
/// headlines, asked about as written, went untyped)
pub fn in_lists_as_one(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '?' => {
                let mut j = i;
                while j > 0 && chars[j - 1].is_whitespace() {
                    j -= 1;
                }
                let after_in = j >= 2
                    && chars[j - 2..j].iter().collect::<String>().eq_ignore_ascii_case("IN")
                    && (j == 2 || !(chars[j - 3].is_ascii_alphanumeric() || chars[j - 3] == '_'));
                if after_in {
                    out.push_str("(?)");
                    continue;
                }
            }
            None => {}
        }
        out.push(c);
    }
    out
}

/// What a statement does, read from its text in any dialect sqlparser
/// reads (its `IN ?` as the `IN (?)` it is sent as): a read — a `SELECT`
/// without `INTO`, `VALUES`, `SHOW`, `EXPLAIN`, `DESCRIBE`, a `PRAGMA`
/// asked — a write, reading rows back where a `RETURNING` clause says, or
/// not known, where the text parses in no dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Does {
    Reads,
    Writes { reads_back: bool },
    Unknown,
}

pub fn classify(statement: &str) -> Does {
    use sqlparser::ast::{SetExpr, Statement};
    use sqlparser::dialect::{ClickHouseDialect, Dialect, GenericDialect, PostgreSqlDialect, SQLiteDialect};
    use sqlparser::parser::Parser;
    let listed = in_lists_as_one(statement);
    let dialects: [&dyn Dialect; 4] = [&GenericDialect {}, &PostgreSqlDialect {}, &SQLiteDialect {}, &ClickHouseDialect {}];
    let Some(statements) = dialects.iter().find_map(|d| Parser::parse_sql(*d, &listed).ok()) else { return Does::Unknown };
    let mut writes = false;
    let mut reads_back = false;
    for s in &statements {
        match s {
            Statement::Query(q) => {
                if let SetExpr::Select(select) = q.body.as_ref() {
                    writes |= select.into.is_some();
                }
            }
            Statement::Explain { .. } | Statement::ExplainTable { .. } | Statement::ShowTables { .. } | Statement::ShowColumns { .. } | Statement::ShowVariable { .. } | Statement::ShowDatabases { .. } | Statement::ShowSchemas { .. } | Statement::ShowFunctions { .. } | Statement::ShowViews { .. } => {}
            Statement::Pragma { value, .. } => writes |= value.is_some(),
            Statement::Insert(insert) => {
                writes = true;
                reads_back |= insert.returning.is_some();
            }
            Statement::Update(update) => {
                writes = true;
                reads_back |= update.returning.is_some();
            }
            Statement::Delete(delete) => {
                writes = true;
                reads_back |= delete.returning.is_some();
            }
            _ => writes = true,
        }
    }
    if writes { Does::Writes { reads_back } } else { Does::Reads }
}

/// Whether a statement writes, for the compiler's effect kinds: handed
/// the literal, or nothing where the statement is built as the program
/// runs, which writes for all it can tell. A statement no dialect reads
/// is judged by its first word (a `SELECT` with no `INTO` reads).
pub fn writes(statement: Option<&str>) -> bool {
    let Some(text) = statement else { return true };
    match classify(text) {
        Does::Reads => false,
        Does::Writes { .. } => true,
        Does::Unknown => {
            let upper = text.to_ascii_uppercase();
            let first = upper.split(|c: char| !c.is_ascii_alphabetic()).find(|w| !w.is_empty()).unwrap_or("");
            !(matches!(first, "SELECT" | "WITH" | "VALUES" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC" | "PRAGMA") && !upper.contains(" INTO "))
        }
    }
}

/// The columns a statement answers on a live connection to `url`, each
/// marked where it can hold NULL: what `crow db describe <url> <sql>`
/// prints, which `describe_by_process` reads for a driver written in Crow.
pub fn of_url(url: &str, sql: &str) -> Result<Vec<(String, String)>, String> {
    let db = crate::connect(url, Some(1))?;
    described(&db, sql)
}

/// The columns a statement answers on a connection, without running it —
/// the one answer the compiler asks for at build time and `describe[conn,
/// sql]` gives as the program runs (`DbHandle::describe`): what the
/// backend says, then, where SQLite types no computed column (stepped once
/// with null parameters, no row came back), the statement's own reading
/// (`sql_types::select_kinds`), each column that can hold NULL marked `?`
/// (`mark_nullable`), and a column nothing types — a parameter alone —
/// text, as the compiler reads it. (The program's `describe` had answered
/// the backend's word alone: no `?` on SQLite or Postgres, and on SQLite
/// `unknown` for a computed column the probe found no row for.) A replay's
/// records are the compiler's answers already, and are answered as they
/// are.
pub fn described(db: &crate::DbHandle, sql: &str) -> Result<Vec<(String, String)>, String> {
    if db.describes_fully() {
        return db.described_by_backend(sql);
    }
    let listed = in_lists_as_one(sql);
    let sql = listed.as_str();
    let mut cols = db.described_by_backend(sql)?;
    if db.backend_name() == "sqlite" && cols.iter().any(|(_, k)| k == "unknown") {
        let columns_of = |table: &str| -> Vec<(String, String)> {
            let frame = match db.query(&format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")), &[]) {
                Ok(f) => f,
                Err(_) => return Vec::new(),
            };
            let text = |name: &str| -> Vec<String> {
                frame.columns.iter().position(|c| c == name)
                    .and_then(|i| match frame.data[i].as_ref() { crow_runtime::frame::Column::Str(v) => Some(v.to_vec()), _ => None })
                    .unwrap_or_default()
            };
            text("name").into_iter().zip(text("type")).collect()
        };
        if let Some(kinds) = crate::sql_types::select_kinds(sql, &columns_of)
            && kinds.len() == cols.len()
        {
            for ((_, k), typed) in cols.iter_mut().zip(kinds) {
                if k == "unknown"
                    && let Some(t) = typed
                {
                    *k = t;
                }
            }
        }
    }
    mark_nullable(db, sql, &mut cols);
    for (_, k) in cols.iter_mut() {
        if let Some(rest) = k.strip_prefix("unknown") {
            *k = format!("str{}", rest);
        }
    }
    Ok(cols)
}

pub fn columns(url: &str, sql: &str, ddl: &[String]) -> Result<Vec<(String, String)>, String> {
    // (a statement that writes and reads nothing back answers an empty
    // frame: no database is asked about a CREATE)
    if classify(sql) == (Does::Writes { reads_back: false }) {
        return Ok(Vec::new());
    }
    let listed = in_lists_as_one(sql);
    let sql = listed.as_str();
    let sqlite_path = url.strip_prefix("sqlite://");
    let missing = sqlite_path.map_or(false, |p| p == ":memory:" || !std::path::Path::new(p).exists());
    if missing {
        if ddl.is_empty() {
            return Err(format!("{} does not exist yet", url));
        }
        return describe_on(&program_key(url, ddl), "sqlite://:memory:", ddl, sql);
    }
    match describe_on(url, url, &[], sql) {
        Err(e) if sqlite_path.is_some() && !ddl.is_empty() && e.contains("no such table") => describe_on(&program_key(url, ddl), "sqlite://:memory:", ddl, sql),
        other => other,
    }
}

/// The scratch database a program's own CREATE statements build: one per
/// set of statements, so programs lowered in one process — the test suite,
/// the language server — never meet each other's tables. (Keyed by the URL
/// alone, every `sqlite://:memory:` program shared the first one's tables,
/// and a later program's cells went untyped.)
fn program_key(url: &str, ddl: &[String]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ddl.hash(&mut h);
    format!("{}#program#{:016x}", url, h.finish())
}

/// Describe on the build's connection for `key` (opened on `url`, built from
/// `ddl` when it is new).
pub fn describe_on(key: &str, url: &str, ddl: &[String], sql: &str) -> Result<Vec<(String, String)>, String> {
    let asked = (key.to_string(), sql.to_string());
    let kept = DESCRIBE_ANSWERS.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|a| a.get(&asked).filter(|(at, _)| at.elapsed() < DESCRIBE_KEPT).map(|(_, c)| c.clone()));
    if let Some(answer) = kept {
        return answer;
    }
    let answer = describe_asking(key, url, ddl, sql);
    DESCRIBE_ANSWERS.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(std::collections::HashMap::new).insert(asked, (std::time::Instant::now(), answer.clone()));
    answer
}

/// The answer of `crow db describe <url> <sql>`, run by this very binary.
fn describe_by_process(url: &str, sql: &str) -> Result<Vec<(String, String)>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find crow itself: {}", e))?;
    // (a test binary is not crow: the database is not reached, as one that
    // does not answer is not)
    if exe.file_stem().and_then(|s| s.to_str()) != Some("crow") {
        return Err("crow db describe runs from the crow binary alone".to_string());
    }
    if std::env::var("CROW_DEBUG_SCHEMAS").is_ok() {
        eprintln!("DESCRIBE asks {} for `{}…`", crate::schema::shown_url(url), sql.chars().take(50).collect::<String>());
    }
    let out = std::process::Command::new(exe).args(["db", "describe", url, sql]).env("CROW_NO_DESCRIBE", "1").output().map_err(|e| format!("cannot run crow db describe: {}", e))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if said.is_empty() { "crow db describe answered nothing".to_string() } else { said });
    }
    let cols: Vec<(String, String)> = serde_json::from_slice(&out.stdout).map_err(|e| format!("crow db describe answered no columns: {}", e))?;
    Ok(cols)
}

/// Ask the database (see `describe_on`, which keeps the answers).
pub fn describe_asking(key: &str, url: &str, ddl: &[String], sql: &str) -> Result<Vec<(String, String)>, String> {
    #[cfg(test)]
    DESCRIBES_ASKED.with(|n| n.set(n.get() + 1));
    // a database whose driver is written in Crow is asked by a process of
    // its own (`crow db describe`): the driver runs in an interpreter, and
    // this lowering, which asks from inside, is not a place to start one
    if url.starts_with("clickhouse://") || url.starts_with("postgres://") || url.starts_with("postgresql://") {
        return describe_by_process(url, sql);
    }
    DESCRIBE_HANDLES.with(|h| {
        let mut map = h.borrow_mut();
        let handle = map.entry(key.to_string()).or_insert_with(|| {
            let db = crate::connect(url, Some(1))?;
            for statement in ddl {
                let _ = db.exec(statement, &[]);
            }
            Ok(db)
        });
        match handle {
            // (SQLite types no computed column, and a statement whose
            // parameters match no row shows none: `described` types those
            // from the statement and the tables' declared columns)
            Ok(db) => described(db, sql),
            Err(e) => Err(e.clone()),
        }
    })
}

/// Mark the columns of a statement's rows that can hold NULL, as its tables'
/// declarations and the statement say (`sql_types::select_nulls`): their
/// kinds end in `?`, and their cells are fallible. SQLite and Postgres
/// describe no column's nullability, so each table's declared NOT NULLs
/// are asked for; a statement that is not read that way can hold NULL in
/// every column. ClickHouse's DESCRIBE says it itself (`Nullable(…)`).
pub fn mark_nullable(db: &crate::DbHandle, sql: &str, cols: &mut [(String, String)]) {
    use crow_runtime::frame::Column;
    let postgres = db.backend_name() == "postgres";
    if !postgres && db.backend_name() != "sqlite" {
        return;
    }
    let read = |frame: &crow_runtime::frame::CrowFrame, name: &str| -> Vec<String> {
        frame.columns.iter().position(|c| c == name).map(|i| match frame.data[i].as_ref() {
            Column::Str(v) => v.to_vec(),
            Column::Int(v) => v.iter().map(|x| x.to_string()).collect(),
            Column::Bool(v) => v.iter().map(|x| x.to_string()).collect(),
            _ => Vec::new(),
        }).unwrap_or_default()
    };
    let columns_of = |table: &str| -> Vec<(String, bool)> {
        if postgres {
            let q = "SELECT attname::text AS name, attnotnull AS notnull FROM pg_attribute WHERE attrelid = to_regclass(?) AND attnum > 0 AND NOT attisdropped ORDER BY attnum";
            let Ok(frame) = db.query(q, &[serde_json::Value::String(table.to_string())]) else { return Vec::new() };
            read(&frame, "name").into_iter().zip(read(&frame, "notnull")).map(|(n, nn)| (n, nn == "true")).collect()
        } else {
            let t = table.rsplit('.').next().unwrap_or(table);
            let Ok(frame) = db.query(&format!("PRAGMA table_info(\"{}\")", t.replace('"', "\"\"")), &[]) else { return Vec::new() };
            let (names, types, notnull, pk) = (read(&frame, "name"), read(&frame, "type"), read(&frame, "notnull"), read(&frame, "pk"));
            // an INTEGER PRIMARY KEY is the row's id, never NULL
            let keys = pk.iter().filter(|p| *p != "0").count();
            names.into_iter().enumerate().map(|(i, n)| {
                let rowid = keys == 1 && pk.get(i).map_or(false, |p| p != "0") && types.get(i).map_or(false, |t| t.eq_ignore_ascii_case("integer"));
                (n, rowid || notnull.get(i).map_or(false, |x| x == "1"))
            }).collect()
        }
    };
    let nulls = crate::sql_types::select_nulls(sql, postgres, &columns_of).filter(|v| v.len() == cols.len());
    for (i, (_, kind)) in cols.iter_mut().enumerate() {
        if nulls.as_ref().map_or(true, |v| v[i]) && !kind.ends_with('?') {
            kind.push('?');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SQLite computed column is typed from the statement when no row
    /// answers the describe probe — finch's blotter P&L read as text in the
    /// compiled program while the interpreter read a number
    #[test]
    fn a_sqlite_computed_column_is_typed_without_a_row() {
        let dir = std::env::temp_dir().join(format!("crow_describe_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("book.db");
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crate::connect(&url, Some(1)).unwrap().exec("CREATE TABLE trades (session_id INTEGER, shares INTEGER, entry_px REAL, exit_px REAL)", &[]).unwrap();
        let cols = columns(&url, "SELECT shares, round((exit_px - entry_px) * shares, 2) AS pnl FROM trades WHERE session_id = ?", &[]).unwrap();
        // (no column is declared NOT NULL: each can hold NULL)
        assert_eq!(cols, vec![("shares".to_string(), "int?".to_string()), ("pnl".to_string(), "float?".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a program's `describe[conn, sql]` answers on SQLite is what the
    /// compiler reads (`DbHandle::describe`): a column that can hold NULL
    /// marked `?`, a computed column of a statement no row answers typed
    /// from the statement, `IN ?` described as a list of one, and a
    /// parameter alone text — where it had answered the database's word
    /// alone: no `?`, `unknown`, and an error for `IN ?`
    #[test]
    fn a_programs_describe_is_the_compilers() {
        let db = crate::connect("sqlite://:memory:", None).unwrap();
        db.exec("CREATE TABLE trades (id INTEGER PRIMARY KEY, sym TEXT NOT NULL, shares INTEGER, entry_px REAL, exit_px REAL)", &[]).unwrap();
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> { v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect() };
        let cols = db.describe("SELECT id, sym, shares, round((exit_px - entry_px) * shares, 2) AS pnl FROM trades WHERE id = ?").unwrap();
        assert_eq!(cols, pairs(&[("id", "int"), ("sym", "str"), ("shares", "int?"), ("pnl", "float?")]));
        let cols = db.describe("SELECT count(*) AS n, sym FROM trades WHERE sym IN ? GROUP BY sym").unwrap();
        assert_eq!(cols, pairs(&[("n", "int"), ("sym", "str")]));
        let cols = db.describe("SELECT ? AS asked").unwrap();
        assert_eq!(cols, pairs(&[("asked", "str?")]));
        // (the compiler's own answer, asked through the provider, is the same)
        let path = std::env::temp_dir().join(format!("crow_describe_program_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crate::connect(&url, Some(1)).unwrap().exec("CREATE TABLE trades (id INTEGER PRIMARY KEY, sym TEXT NOT NULL, shares INTEGER, entry_px REAL, exit_px REAL)", &[]).unwrap();
        let sql = "SELECT id, sym, shares, round((exit_px - entry_px) * shares, 2) AS pnl FROM trades WHERE id = ?";
        assert_eq!(columns(&url, sql, &[]).unwrap(), crate::connect(&url, Some(1)).unwrap().describe(sql).unwrap());
        let _ = std::fs::remove_file(&path);
    }

    /// A statement with a list parameter (`IN ?`) is asked about as the list
    /// of one it is sent as: no database prepares `IN ?`
    #[test]
    fn a_list_parameter_is_described_as_a_list_of_one() {
        assert_eq!(in_lists_as_one("SELECT a FROM t WHERE b IN ? AND c NOT IN  ? AND d = '? IN ?' AND e = ? AND MIN(f) > ?"), "SELECT a FROM t WHERE b IN (?) AND c NOT IN  (?) AND d = '? IN ?' AND e = ? AND MIN(f) > ?");
        let dir = std::env::temp_dir().join(format!("crow_describe_in_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("in.db");
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crate::connect(&url, Some(1)).unwrap().exec("CREATE TABLE news (sym TEXT NOT NULL, at REAL NOT NULL)", &[]).unwrap();
        let cols = columns(&url, "SELECT sym, at FROM news WHERE sym IN ?", &[]).unwrap();
        assert_eq!(cols, vec![("sym".to_string(), "str".to_string()), ("at".to_string(), "float".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A statement described twice within the minute is asked once: the
    /// second program lowered in a process (each test block's) takes the
    /// first's answer
    #[test]
    fn a_statement_is_described_once_a_minute() {
        let dir = std::env::temp_dir().join(format!("crow_describe_kept_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("kept.db");
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crate::connect(&url, Some(1)).unwrap().exec("CREATE TABLE bars (sym TEXT NOT NULL, px REAL)", &[]).unwrap();
        let before = DESCRIBES_ASKED.with(|n| n.get());
        let first = columns(&url, "SELECT sym, px FROM bars WHERE sym = ?", &[]).unwrap();
        let second = columns(&url, "SELECT sym, px FROM bars WHERE sym = ?", &[]).unwrap();
        assert_eq!(first, second);
        assert_eq!(first, vec![("sym".to_string(), "str".to_string()), ("px".to_string(), "float?".to_string())]);
        assert_eq!(DESCRIBES_ASKED.with(|n| n.get()) - before, 1, "the second describe asked again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two programs in one process, each making its own in-memory tables of
    /// one name, are described from their own tables (the second had been
    /// described from the first's, and its cells went untyped: a clean
    /// clone's test suite refused parity 92)
    #[test]
    fn each_program_is_described_from_its_own_tables() {
        let url = "sqlite://:memory:";
        let a = columns(url, "SELECT v FROM t_own_tables", &["CREATE TABLE t_own_tables (v INTEGER NOT NULL)".to_string()]).unwrap();
        let b = columns(url, "SELECT v, w FROM t_own_tables", &["CREATE TABLE t_own_tables (v TEXT NOT NULL, w REAL NOT NULL)".to_string()]).unwrap();
        assert_eq!(a, vec![("v".to_string(), "int".to_string())]);
        assert_eq!(b, vec![("v".to_string(), "str".to_string()), ("w".to_string(), "float".to_string())]);
    }
}
