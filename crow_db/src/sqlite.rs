//! SQLite backend for crow_db.
//!
//! Uses `rusqlite` with file-based or in-memory databases.
//! Pool size is always 1 (SQLite uses file-level locking).

use std::sync::Mutex;
use crow_runtime::frame::{CrowFrame, Column};
use crate::{CrowDb, DbTx};

/// SQLite database — wraps a single `rusqlite::Connection` behind a Mutex.
pub struct CrowSqliteDb {
    conn: Mutex<rusqlite::Connection>,
}

impl CrowSqliteDb {
    /// Open (or create) a SQLite database file.
    pub fn open(path: &str) -> Result<Self, String> {
        rusqlite::Connection::open(path)
            .map(|conn| Self { conn: Mutex::new(conn) })
            .map_err(|e| e.to_string())
    }

    /// Open an in-memory SQLite database.
    pub fn memory() -> Result<Self, String> {
        rusqlite::Connection::open_in_memory()
            .map(|conn| Self { conn: Mutex::new(conn) })
            .map_err(|e| e.to_string())
    }
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

impl CrowDb for CrowSqliteDb {
    /// The columns a statement would return, typed from the declared types
    /// of the table columns they come straight from (INTEGER → int, REAL →
    /// float, TEXT → str, BOOLEAN → bool); a computed column has no declared
    /// type and reads as text. The statement is prepared, never run.
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        Ok(stmt.columns().iter().map(|c| {
            let kind = match c.decl_type().map(|t| t.to_ascii_uppercase()) {
                Some(t) if t.contains("INT") => "int",
                Some(t) if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") || t.contains("NUM") || t.contains("DEC") => "float",
                Some(t) if t.contains("BOOL") => "bool",
                _ => "str",
            };
            (c.name().to_string(), kind.to_string())
        }).collect())
    }

    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let params_ = json_to_sql_params(params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let ncols = col_names.len();

        // Collect all rows as rusqlite::types::Value
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
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let params_ = json_to_sql_params(params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_.iter().map(|b| b.as_ref()).collect();
        conn.execute(sql, param_refs.as_slice())
            .map(|n| n as i64)
            .map_err(|e| e.to_string())
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
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;

        let wrapper = SqliteTransactWrapper { conn: &conn };
        match f(&wrapper) {
            Ok(()) => {
                conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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

