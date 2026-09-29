//! SQLite backend for crow_db.
//!
//! Uses `rusqlite` with file-based or in-memory databases.
//! Pool size is always 1 (SQLite uses file-level locking).

use std::sync::{Arc, Mutex};
use crow_runtime::frame::{CrowFrame, Column};
use crate::{CrowDb, DbTx, ScopeTx};

/// How long a statement waits for another connection's write to finish
/// before the database is reported locked: a handler's transaction holds
/// the file's write lock until the handler ends.
const BUSY: std::time::Duration = std::time::Duration::from_secs(5);

/// SQLite database — wraps a single `rusqlite::Connection` behind a Mutex.
pub struct CrowSqliteDb {
    conn: Arc<Mutex<rusqlite::Connection>>,
    /// the file, which a handler's transaction opens a connection of its
    /// own to; none for `:memory:`, whose one connection is the database
    path: Option<String>,
}

impl CrowSqliteDb {
    /// Open (or create) a SQLite database file.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(BUSY).map_err(|e| e.to_string())?;
        let path = if path == ":memory:" || path.is_empty() { None } else { Some(path.to_string()) };
        Ok(Self { conn: Arc::new(Mutex::new(conn)), path })
    }

    /// Open an in-memory SQLite database.
    pub fn memory() -> Result<Self, String> {
        rusqlite::Connection::open_in_memory()
            .map(|conn| Self { conn: Arc::new(Mutex::new(conn)), path: None })
            .map_err(|e| e.to_string())
    }
}

/// A handler's transaction on SQLite: a connection of its own to the file,
/// begun with the write lock taken; or, for `:memory:`, the one connection.
enum SqliteScope {
    Own(rusqlite::Connection),
    Shared(Arc<Mutex<rusqlite::Connection>>),
}

impl SqliteScope {
    fn with<T>(&mut self, f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>) -> Result<T, String> {
        match self {
            SqliteScope::Own(c) => f(c),
            SqliteScope::Shared(c) => {
                let guard = c.lock().map_err(|e| e.to_string())?;
                f(&guard)
            }
        }
    }
}

impl ScopeTx for SqliteScope {
    fn query(&mut self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.with(|c| query_on(c, sql, params))
    }

    fn exec(&mut self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.with(|c| exec_on(c, sql, params))
    }

    fn finish(mut self: Box<Self>, commit: bool) -> Result<(), String> {
        let end = if commit { "COMMIT" } else { "ROLLBACK" };
        self.with(|c| c.execute_batch(end).map_err(|e| e.to_string()))
    }
}

/// A statement's rows on one connection.
fn query_on(conn: &rusqlite::Connection, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
    let params_ = json_to_sql_params(params);
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let ncols = col_names.len();
    let mut raw_rows: Vec<Vec<rusqlite::types::Value>> = Vec::new();
    let mut rows = stmt.query(param_refs.as_slice()).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut raw_row = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let val: rusqlite::types::Value = row.get(i).map_err(|e| e.to_string())?;
            raw_row.push(val);
        }
        raw_rows.push(raw_row);
    }
    if raw_rows.is_empty() {
        let data: Vec<Column> = (0..ncols).map(|_| Column::Str(Vec::new())).collect();
        let validity: Vec<Vec<bool>> = (0..ncols).map(|_| Vec::new()).collect();
        return Ok(CrowFrame::new(col_names, data, validity));
    }
    build_frame_from_rows(&col_names, &raw_rows)
}

/// A statement's affected rows on one connection.
fn exec_on(conn: &rusqlite::Connection, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
    let params_ = json_to_sql_params(params);
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_.iter().map(|b| b.as_ref()).collect();
    conn.execute(sql, param_refs.as_slice()).map(|n| n as i64).map_err(|e| e.to_string())
}

/// Convert serde_json::Value params to rusqlite ToSql boxes.
fn json_to_sql_params(params: &[serde_json::Value]) -> Vec<Box<dyn rusqlite::types::ToSql>> {
    params
        .iter()
        .map(|v| -> Box<dyn rusqlite::types::ToSql> {
            match v {
                serde_json::Value::String(s) => Box::new(s.clone()),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Box::new(i)
                    } else {
                        Box::new(n.as_f64().unwrap_or(0.0))
                    }
                }
                serde_json::Value::Bool(b) => Box::new(*b),
                serde_json::Value::Null => Box::new(rusqlite::types::Null),
                _ => Box::new(v.to_string()),
            }
        })
        .collect()
}

impl CrowSqliteDb {
    /// Run `f` in one transaction on the connection, begun as `begin` says:
    /// committed when it finishes, rolled back when it fails.
    fn in_transaction(&self, begin: &str, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(begin).map_err(|e| e.to_string())?;
        let wrapper = SqliteTransactWrapper { conn: &conn };
        match f(&wrapper) {
            Ok(()) => conn.execute_batch("COMMIT").map_err(|e| e.to_string()),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

impl CrowDb for CrowSqliteDb {
    /// The columns a statement would return, typed from the declared types
    /// of the table columns they come straight from (INTEGER → int, REAL →
    /// float, TEXT → str, BOOLEAN → bool); a computed column has no declared
    /// type and reads as text. The statement is prepared, never run.
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let mut kinds: Vec<(String, Option<String>)> = stmt.columns().iter().map(|c| {
            (c.name().to_string(), c.decl_type().and_then(kind_of_decl))
        }).collect();
        // A RETURNING clause's cells have no declared type of their own:
        // they are the target table's columns, typed from its declaration
        if kinds.iter().any(|(_, k)| k.is_none()) {
            if let Some(table) = returning_target(sql) {
                let declared: Vec<(String, String)> = conn
                    .prepare(&format!("PRAGMA table_info({})", table))
                    .and_then(|mut s| {
                        s.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                            .map(|it| it.filter_map(|x| x.ok()).collect())
                    })
                    .unwrap_or_default();
                for (name, k) in kinds.iter_mut() {
                    if k.is_none() {
                        if let Some((_, t)) = declared.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                            *k = kind_of_decl(t);
                        }
                    }
                }
            }
        }
        // a computed column (count, round, a concatenation) has no declared
        // type: the statement is stepped once with null parameters and the
        // first row's values say what they are — a count always has a row
        if kinds.iter().any(|(_, k)| k.is_none()) && sql.trim_start().to_ascii_uppercase().starts_with("SELECT") {
            let n = stmt.parameter_count();
            let nulls: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Null; n];
            let refs: Vec<&dyn rusqlite::types::ToSql> = nulls.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
            if let Ok(mut rows) = stmt.query(refs.as_slice()) {
                if let Ok(Some(row)) = rows.next() {
                    for (i, (_, k)) in kinds.iter_mut().enumerate() {
                        if k.is_none() {
                            if let Ok(v) = row.get_ref(i) {
                                *k = match v {
                                    rusqlite::types::ValueRef::Integer(_) => Some("int".to_string()),
                                    rusqlite::types::ValueRef::Real(_) => Some("float".to_string()),
                                    rusqlite::types::ValueRef::Text(_) | rusqlite::types::ValueRef::Blob(_) => Some("str".to_string()),
                                    rusqlite::types::ValueRef::Null => None,
                                };
                            }
                        }
                    }
                }
            }
        }
        // a column neither declared nor seen in a row — a computed column of
        // a statement whose parameters match nothing, or of an empty table —
        // is `unknown`: the compiler types it from the statement itself
        Ok(kinds.into_iter().map(|(n, k)| (n, k.unwrap_or_else(|| "unknown".to_string()))).collect())
    }

    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        query_on(&conn, sql, params)
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        exec_on(&conn, sql, params)
    }

    fn begin_scope(&self) -> Option<Result<Box<dyn ScopeTx>, String>> {
        Some(match &self.path {
            Some(path) => (|| {
                let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
                conn.busy_timeout(BUSY).map_err(|e| e.to_string())?;
                // the write lock now, so two handlers never deadlock mid-way
                conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
                Ok(Box::new(SqliteScope::Own(conn)) as Box<dyn ScopeTx>)
            })(),
            None => (|| {
                self.conn.lock().map_err(|e| e.to_string())?.execute_batch("BEGIN").map_err(|e| e.to_string())?;
                Ok(Box::new(SqliteScope::Shared(self.conn.clone())) as Box<dyn ScopeTx>)
            })(),
        })
    }

    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let nrows = frame.nrows();
        if nrows == 0 {
            return Ok(0);
        }
        let ncols = frame.ncols();
        let col_names = frame.columns.join(", ");
        let placeholders: Vec<&str> = (0..ncols).map(|_| "?").collect();
        let placeholders_str = placeholders.join(", ");
        let sql = format!("INSERT INTO {} ({}) VALUES ({})", table, col_names, placeholders_str);

        // Use a transaction for bulk insert
        conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(&sql).map_err(|e| {
            let _ = conn.execute_batch("ROLLBACK");
            e.to_string()
        })?;

        for row_idx in 0..nrows {
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(ncols);
            for col_idx in 0..ncols {
                let valid = frame.validity[col_idx][row_idx];
                if !valid {
                    params.push(Box::new(rusqlite::types::Null));
                    continue;
                }
                match frame.data[col_idx].as_ref() {
                    Column::Int(v) => params.push(Box::new(v[row_idx])),
                    Column::Float(v) => params.push(Box::new(v[row_idx])),
                    Column::Str(v) => params.push(Box::new(v[row_idx].clone())),
                    Column::Bool(v) => params.push(Box::new(v[row_idx])),
                    Column::DateTime(v, _) => params.push(Box::new(v[row_idx])),
                }
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|b| b.as_ref()).collect();
            stmt.execute(param_refs.as_slice()).map_err(|e| {
                let _ = conn.execute_batch("ROLLBACK");
                e.to_string()
            })?;
        }

        conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        Ok(nrows as i64)
    }

    fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        self.in_transaction("BEGIN", f)
    }

    /// The write lock at once, in an immediate transaction: a second
    /// process starting waits here, then finds the schema made.
    fn schema_lock(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        self.in_transaction("BEGIN IMMEDIATE", f)
    }

    fn migrate(&self, dir: &str) -> Result<i64, String> {
        crate::migrate::run_migrations(self, dir)
    }

    fn backend_name(&self) -> &str {
        "sqlite"
    }
}

/// Transaction wrapper — holds a reference to the locked connection.
/// Used inside `transact` so the closure can call exec/query on the same connection.
struct SqliteTransactWrapper<'a> {
    conn: &'a rusqlite::Connection,
}

impl DbTx for SqliteTransactWrapper<'_> {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let params_ = json_to_sql_params(params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_.iter().map(|b| b.as_ref()).collect();

        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let ncols = col_names.len();

        let mut raw_rows: Vec<Vec<rusqlite::types::Value>> = Vec::new();
        let mut rows = stmt.query(param_refs.as_slice()).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let mut raw_row = Vec::with_capacity(ncols);
            for i in 0..ncols {
                let val: rusqlite::types::Value = row.get(i).map_err(|e| e.to_string())?;
                raw_row.push(val);
            }
            raw_rows.push(raw_row);
        }

        let nrows = raw_rows.len();
        if nrows == 0 {
            let data: Vec<Column> = (0..ncols).map(|_| Column::Str(Vec::new())).collect();
            let validity: Vec<Vec<bool>> = (0..ncols).map(|_| Vec::new()).collect();
            return Ok(CrowFrame::new(col_names, data, validity));
        }

        build_frame_from_rows(&col_names, &raw_rows)
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let params_ = json_to_sql_params(params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_.iter().map(|b| b.as_ref()).collect();
        self.conn
            .execute(sql, param_refs.as_slice())
            .map(|n| n as i64)
            .map_err(|e| e.to_string())
    }
}

/// Build a CrowFrame from raw rusqlite rows (shared between CrowSqliteDb and TransactWrapper).
fn build_frame_from_rows(
    col_names: &[String],
    raw_rows: &[Vec<rusqlite::types::Value>],
) -> Result<CrowFrame, String> {
    let ncols = col_names.len();
    let nrows = raw_rows.len();
    let mut columns: Vec<Column> = Vec::with_capacity(ncols);
    let mut validities: Vec<Vec<bool>> = Vec::with_capacity(ncols);

    for col_idx in 0..ncols {
        let mut col_type = None;
        for row in raw_rows {
            match &row[col_idx] {
                rusqlite::types::Value::Null => continue,
                rusqlite::types::Value::Integer(_) => { col_type = Some("int"); break; }
                rusqlite::types::Value::Real(_) => { col_type = Some("float"); break; }
                rusqlite::types::Value::Text(_) => { col_type = Some("str"); break; }
                rusqlite::types::Value::Blob(_) => { col_type = Some("str"); break; }
            }
        }

        let mut valid = vec![true; nrows];
        match col_type.unwrap_or("str") {
            "int" => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in raw_rows.iter().enumerate() {
                    match &row[col_idx] {
                        rusqlite::types::Value::Integer(n) => vals.push(*n),
                        rusqlite::types::Value::Null => { vals.push(0); valid[row_idx] = false; }
                        _ => vals.push(0),
                    }
                }
                columns.push(Column::Int(vals));
            }
            "float" => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in raw_rows.iter().enumerate() {
                    match &row[col_idx] {
                        rusqlite::types::Value::Real(f) => vals.push(*f),
                        rusqlite::types::Value::Integer(n) => vals.push(*n as f64),
                        rusqlite::types::Value::Null => { vals.push(0.0); valid[row_idx] = false; }
                        _ => vals.push(0.0),
                    }
                }
                columns.push(Column::Float(vals));
            }
            _ => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in raw_rows.iter().enumerate() {
                    match &row[col_idx] {
                        rusqlite::types::Value::Text(s) => vals.push(s.clone()),
                        rusqlite::types::Value::Blob(b) => vals.push(base64_encode(b)),
                        rusqlite::types::Value::Integer(n) => vals.push(n.to_string()),
                        rusqlite::types::Value::Real(f) => vals.push(f.to_string()),
                        rusqlite::types::Value::Null => { vals.push(String::new()); valid[row_idx] = false; }
                    }
                }
                columns.push(Column::Str(vals));
            }
        }
        validities.push(valid);
    }

    Ok(CrowFrame::new(col_names.to_vec(), columns, validities))
}

/// Simple base64 encode for blob data.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        result.push(CHARS[b0 >> 2] as char);
        result.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((b1 & 0xF) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[b2 & 0x3F] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// The cell kind a declared sqlite column type stands for.
/// The kind a declared SQLite column type gives its cells, by SQLite's
/// affinity rules: the one mapping describe and the compiler's statement
/// typer share.
pub fn kind_of_decl(t: &str) -> Option<String> {
    let t = t.to_ascii_uppercase();
    let k = if t.contains("INT") {
        "int"
    } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") || t.contains("NUM") || t.contains("DEC") {
        "float"
    } else if t.contains("BOOL") {
        "bool"
    } else {
        "str"
    };
    Some(k.to_string())
}

/// The table an INSERT, UPDATE or DELETE with a RETURNING clause writes.
fn returning_target(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    if !upper.contains(" RETURNING ") {
        return None;
    }
    let words: Vec<&str> = sql.split_whitespace().collect();
    let table = match words.first().map(|w| w.to_ascii_uppercase()).as_deref() {
        Some("INSERT") | Some("REPLACE") => words.iter().position(|w| w.eq_ignore_ascii_case("INTO")).and_then(|i| words.get(i + 1)),
        Some("UPDATE") => words.get(1),
        Some("DELETE") => words.iter().position(|w| w.eq_ignore_ascii_case("FROM")).and_then(|i| words.get(i + 1)),
        _ => None,
    }?;
    let table = table.split('(').next().unwrap_or(table).trim_matches(|c| c == '"' || c == '`' || c == '\'');
    if table.is_empty() { None } else { Some(table.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(h: &crate::DbHandle) -> i64 {
        match h.query("SELECT count(*) AS n FROM t", &[]).unwrap().data[0].as_ref() {
            Column::Int(v) => v[0],
            other => panic!("expected Int column, got {:?}", other),
        }
    }

    /// A handler's writes are one transaction on a connection of its own:
    /// it reads what it wrote, nobody else does until it commits, and a
    /// stop rolls it back (core-design.md §1.2)
    #[test]
    fn a_handler_writes_in_a_transaction_of_its_own() {
        let path = std::env::temp_dir().join(format!("crow_scope_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let h = crate::DbHandle::new(CrowSqliteDb::open(path.to_str().unwrap()).unwrap());
        h.exec("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", &[]).unwrap();

        crow_runtime::txn::begin();
        h.exec("INSERT INTO t (v) VALUES (?)", &[serde_json::json!("doomed")]).unwrap();
        assert_eq!(count(&h), 1, "the handler reads its own write");
        let other = std::thread::spawn({
            let h = h.clone();
            move || count(&h)
        })
        .join()
        .unwrap();
        assert_eq!(other, 0, "another session does not see an uncommitted write");
        crow_runtime::txn::finish(false).unwrap();
        assert_eq!(count(&h), 0, "a stop rolls the write back");

        crow_runtime::txn::begin();
        h.exec("INSERT INTO t (v) VALUES (?)", &[serde_json::json!("kept")]).unwrap();
        // another session's write waits for the handler's, then lands
        let writer = std::thread::spawn({
            let h = h.clone();
            move || h.exec("INSERT INTO t (v) VALUES (?)", &[serde_json::json!("after")])
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        crow_runtime::txn::finish(true).unwrap();
        writer.join().unwrap().unwrap();
        assert_eq!(count(&h), 2, "the commit and the waiting write both landed");
        let _ = std::fs::remove_file(&path);
    }

    /// A RETURNING clause's cells are typed from the table's declaration —
    /// and asking never runs the statement
    #[test]
    fn a_returning_clause_is_typed_from_the_table() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE sessions (id INTEGER PRIMARY KEY, day TEXT NOT NULL, speed REAL)", &[]).unwrap();
        let d = db.describe("INSERT INTO sessions (day, speed) VALUES (?, ?) RETURNING id, speed").unwrap();
        assert_eq!(d, vec![("id".to_string(), "int".to_string()), ("speed".to_string(), "float".to_string())]);
        let n = db.query("SELECT count(*) AS n FROM sessions", &[]).unwrap();
        match n.data[0].as_ref() {
            Column::Int(v) => assert_eq!(v, &[0]),
            other => panic!("expected Int column, got {:?}", other),
        }
        assert_eq!(returning_target("UPDATE t SET a = 1 WHERE id = ? RETURNING a"), Some("t".to_string()));
        assert_eq!(returning_target("DELETE FROM \"t\" WHERE id = ? RETURNING id"), Some("t".to_string()));
        assert_eq!(returning_target("SELECT 1"), None);
    }

    /// A computed column of a statement whose parameters match no row has
    /// no declared type and no value to look at: describe says `unknown`,
    /// not text, and the compiler types it from the statement
    #[test]
    fn a_computed_column_with_no_row_is_unknown() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE trades (session_id INTEGER, shares INTEGER, entry_px REAL, exit_px REAL)", &[]).unwrap();
        let d = db.describe("SELECT shares, round((exit_px - entry_px) * shares, 2) AS pnl FROM trades WHERE session_id = ?").unwrap();
        assert_eq!(d, vec![("shares".to_string(), "int".to_string()), ("pnl".to_string(), "unknown".to_string())]);
    }

    #[test]
    fn test_sqlite_memory_query_returns_frame() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        db.exec("INSERT INTO t (name) VALUES (?)", &[serde_json::json!("alice")]).unwrap();
        db.exec("INSERT INTO t (name) VALUES (?)", &[serde_json::json!("bob")]).unwrap();

        let frame = db.query("SELECT * FROM t ORDER BY id", &[]).unwrap();
        assert_eq!(frame.nrows(), 2);
        assert_eq!(frame.ncols(), 2);
        assert_eq!(frame.columns, vec!["id", "name"]);

        // id column should be Int
        match frame.data[0].as_ref() {
            Column::Int(v) => assert_eq!(v, &[1, 2]),
            other => panic!("expected Int column, got {:?}", other),
        }
        // name column should be Str
        match frame.data[1].as_ref() {
            Column::Str(v) => assert_eq!(v, &["alice", "bob"]),
            other => panic!("expected Str column, got {:?}", other),
        }
    }

    #[test]
    fn test_sqlite_exec_returns_affected() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (v TEXT)", &[]).unwrap();
        db.exec("INSERT INTO t VALUES ('a')", &[]).unwrap();
        db.exec("INSERT INTO t VALUES ('b')", &[]).unwrap();
        let affected = db.exec("DELETE FROM t", &[]).unwrap();
        assert_eq!(affected, 2);
    }

    #[test]
    fn test_sqlite_query_empty() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (v TEXT)", &[]).unwrap();
        let frame = db.query("SELECT * FROM t", &[]).unwrap();
        assert_eq!(frame.nrows(), 0);
        assert_eq!(frame.ncols(), 1);
    }

    #[test]
    fn test_sqlite_insert_frame() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (name TEXT, age INTEGER)", &[]).unwrap();

        let frame = CrowFrame::new(
            vec!["name".into(), "age".into()],
            vec![
                Column::Str(vec!["alice".into(), "bob".into()]),
                Column::Int(vec![30, 25]),
            ],
            vec![vec![true, true], vec![true, true]],
        );

        let inserted = db.insert("t", &frame).unwrap();
        assert_eq!(inserted, 2);

        let result = db.query("SELECT * FROM t ORDER BY age", &[]).unwrap();
        assert_eq!(result.nrows(), 2);
        match result.data[0].as_ref() {
            Column::Str(v) => assert_eq!(v, &["bob", "alice"]),
            other => panic!("expected Str column, got {:?}", other),
        }
    }

    #[test]
    fn test_sqlite_transact_commit() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (v INTEGER)", &[]).unwrap();

        db.transact(&|tx| {
            tx.exec("INSERT INTO t VALUES (1)", &[])?;
            tx.exec("INSERT INTO t VALUES (2)", &[])?;
            Ok(())
        }).unwrap();

        let frame = db.query("SELECT * FROM t ORDER BY v", &[]).unwrap();
        assert_eq!(frame.nrows(), 2);
    }

    #[test]
    fn test_sqlite_transact_rollback() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (v INTEGER)", &[]).unwrap();

        let result = db.transact(&|tx| {
            tx.exec("INSERT INTO t VALUES (1)", &[])?;
            Err("intentional error".into())
        });
        assert!(result.is_err());

        let frame = db.query("SELECT * FROM t", &[]).unwrap();
        assert_eq!(frame.nrows(), 0);
    }

    #[test]
    fn test_sqlite_null_handling() {
        let db = CrowSqliteDb::memory().unwrap();
        db.exec("CREATE TABLE t (id INTEGER, name TEXT)", &[]).unwrap();
        db.exec("INSERT INTO t VALUES (1, NULL)", &[]).unwrap();
        db.exec("INSERT INTO t VALUES (NULL, 'bob')", &[]).unwrap();

        let frame = db.query("SELECT * FROM t ORDER BY rowid", &[]).unwrap();
        assert_eq!(frame.nrows(), 2);
        // id: first row valid, second row invalid (null)
        assert!(frame.validity[0][0]);
        assert!(!frame.validity[0][1]);
        // name: first row invalid (null), second row valid
        assert!(!frame.validity[1][0]);
        assert!(frame.validity[1][1]);
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
    }
}

