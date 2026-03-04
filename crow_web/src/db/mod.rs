//! `web.db` module — SQLite and PostgreSQL database access.
//!
//! ```text
//! use web.db
//!
//! db = sqlite['data.db']
//! exec[db, 'CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)']
//! exec[db, 'INSERT INTO users (name) VALUES (?)', ['alice']]
//! rows = query[db, 'SELECT * FROM users']
//! log[rows]
//! ```

use std::collections::HashMap;

/// A database row — maps column names to JSON values.
pub type CrowRow = HashMap<String, serde_json::Value>;

// ── SQLite ──────────────────────────────────────────────────────────

/// SQLite database wrapper.
pub struct CrowSqlite {
    conn: rusqlite::Connection,
}

impl CrowSqlite {
    /// Open (or create) a SQLite database file.
    pub fn open(path: &str) -> Result<Self, String> {
        rusqlite::Connection::open(path)
            .map(|conn| Self { conn })
            .map_err(|e| e.to_string())
    }

    /// Open an in-memory SQLite database.
    pub fn memory() -> Result<Self, String> {
        rusqlite::Connection::open_in_memory()
            .map(|conn| Self { conn })
            .map_err(|e| e.to_string())
    }

    /// Execute a statement (INSERT, UPDATE, DELETE, CREATE, etc.).
    pub fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<usize, String> {
        let params_: Vec<Box<dyn rusqlite::types::ToSql>> = params
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
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_.iter().map(|b| b.as_ref()).collect();
        self.conn
            .execute(sql, param_refs.as_slice())
            .map_err(|e| e.to_string())
    }

    /// Query rows (SELECT).
    pub fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<Vec<CrowRow>, String> {
        let params_: Vec<Box<dyn rusqlite::types::ToSql>> = params
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
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_.iter().map(|b| b.as_ref()).collect();

        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let mut map = HashMap::new();
                for (i, name) in col_names.iter().enumerate() {
                    let val: rusqlite::types::Value = row.get(i)?;
                    let json_val = match val {
                        rusqlite::types::Value::Null => serde_json::Value::Null,
                        rusqlite::types::Value::Integer(n) => serde_json::json!(n),
                        rusqlite::types::Value::Real(f) => serde_json::json!(f),
                        rusqlite::types::Value::Text(s) => serde_json::json!(s),
                        rusqlite::types::Value::Blob(b) => {
                            serde_json::json!(base64_encode(&b))
                        }
                    };
                    map.insert(name.clone(), json_val);
                }
                Ok(map)
            })
            .map_err(|e| e.to_string())?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    }
}

/// Simple base64 encode for blob data.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        result.push(CHARS[(b0 >> 2)] as char);
        result.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((b1 & 0xF) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(b2 & 0x3F)] as char);
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
    fn test_sqlite_memory() {
        let db = CrowSqlite::memory().unwrap();
        db.exec("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", &[])
            .unwrap();
        db.exec(
            "INSERT INTO t (name) VALUES (?)",
            &[serde_json::json!("alice")],
        )
        .unwrap();
        let rows = db.query("SELECT * FROM t", &[]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "alice");
    }

    #[test]
    fn test_sqlite_multiple_rows() {
        let db = CrowSqlite::memory().unwrap();
        db.exec("CREATE TABLE t (v INTEGER)", &[]).unwrap();
        for i in 0..5 {
            db.exec("INSERT INTO t (v) VALUES (?)", &[serde_json::json!(i)])
                .unwrap();
        }
        let rows = db.query("SELECT * FROM t ORDER BY v", &[]).unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0]["v"], 0);
        assert_eq!(rows[4]["v"], 4);
    }

    #[test]
    fn test_sqlite_exec_returns_affected() {
        let db = CrowSqlite::memory().unwrap();
        db.exec("CREATE TABLE t (v TEXT)", &[]).unwrap();
        db.exec("INSERT INTO t VALUES ('a')", &[]).unwrap();
        db.exec("INSERT INTO t VALUES ('b')", &[]).unwrap();
        let affected = db.exec("DELETE FROM t", &[]).unwrap();
        assert_eq!(affected, 2);
    }

    #[test]
    fn test_sqlite_query_empty() {
        let db = CrowSqlite::memory().unwrap();
        db.exec("CREATE TABLE t (v TEXT)", &[]).unwrap();
        let rows = db.query("SELECT * FROM t", &[]).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
    }
}

