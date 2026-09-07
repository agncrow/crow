//! ClickHouse backend for crow_db.
//!
//! Uses `klickhouse` for native TCP protocol (port 9000).
//! ClickHouse is an OLAP engine — transactions are NOT supported.
//! URL scheme: `clickhouse://user:pass@host:port/database`

use std::sync::Arc;
use crow_runtime::frame::{CrowFrame, Column};
use crate::{CrowDb, DbTx};
use tokio::runtime::Runtime;
use klickhouse::{Client, ClientOptions, Value, RawRow};
use tokio_stream::StreamExt;

/// ClickHouse database — wraps a klickhouse Client + tokio runtime.
pub struct CrowClickhouseDb {
    /// The live connection. A connection is not the program's lifetime:
    /// the server closes an idle one (an hour by default) and a restart
    /// drops every one, so it is reopened when found closed
    client: std::sync::Mutex<Client>,
    rt: Arc<Runtime>,
    addr: String,
    options: ClientOptions,
}

/// An error from the transport rather than the query: the server hung up
/// (idle timeout, restart) and the client's channel is dead.
fn is_transport_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ["channel closed", "failed to send", "connection reset", "broken pipe", "connection closed", "unexpected end of file", "early eof", "os error"]
        .iter()
        .any(|needle| m.contains(needle))
}

impl CrowClickhouseDb {
    /// Run blocking runtime work safely: if we are already inside a tokio
    /// runtime (compiled apps run in #[tokio::main]), block_on would panic —
    /// hop to a scoped worker thread where blocking is legal.
    fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::scope(|sc| {
                sc.spawn(f).join().expect("clickhouse worker thread panicked")
            })
        } else {
            f()
        }
    }

    /// Connect to ClickHouse using a URL.
    /// `url` should be like `clickhouse://user:pass@host:port/database`
    /// or `clickhouse://host:port/database` (defaults to user=default, no password).
    pub fn connect(url: &str) -> Result<Self, String> {
        let rt = Runtime::new().map_err(|e| format!("failed to create tokio runtime: {}", e))?;

        // Parse URL: clickhouse://[user[:pass]@]host[:port][/database]
        let stripped = url.strip_prefix("clickhouse://")
            .ok_or_else(|| format!("invalid clickhouse URL: {}", url))?;

        let (userinfo, hostpath) = if let Some(at_pos) = stripped.rfind('@') {
            (&stripped[..at_pos], &stripped[at_pos + 1..])
        } else {
            ("", stripped)
        };

        let (host_port, database) = if let Some(slash_pos) = hostpath.find('/') {
            (&hostpath[..slash_pos], &hostpath[slash_pos + 1..])
        } else {
            (hostpath, "default")
        };

        // Default port 9000
        let addr = if host_port.contains(':') {
            host_port.to_string()
        } else {
            format!("{}:9000", host_port)
        };

        let (username, password) = if userinfo.is_empty() {
            ("default".to_string(), String::new())
        } else if let Some(colon_pos) = userinfo.find(':') {
            (userinfo[..colon_pos].to_string(), userinfo[colon_pos + 1..].to_string())
        } else {
            (userinfo.to_string(), String::new())
        };

        let options = ClientOptions {
            username,
            password,
            default_database: database.to_string(),
            tcp_nodelay: true,
        };

        let client = Self::open(&rt, &addr, options.clone())?;
        Ok(Self { client: std::sync::Mutex::new(client), rt: Arc::new(rt), addr, options })
    }

    /// Connect and ping.
    fn open(rt: &Runtime, addr: &str, options: ClientOptions) -> Result<Client, String> {
        let client = Self::off_runtime(|| rt.block_on(async {
            Client::connect(addr, options).await
                .map_err(|e| format!("clickhouse connection failed: {}", e))
        }))?;
        Self::off_runtime(|| rt.block_on(async {
            client.execute("SELECT 1").await
                .map_err(|e| format!("clickhouse ping failed: {}", e))
        }))?;
        Ok(client)
    }

    /// The connection to use now — reopened first when the server has
    /// closed it.
    fn client(&self) -> Result<Client, String> {
        let mut guard = self.client.lock().map_err(|e| e.to_string())?;
        if guard.is_closed() {
            eprintln!("crow db: the clickhouse connection to {} was closed; reconnecting", self.addr);
            *guard = Self::open(&self.rt, &self.addr, self.options.clone())?;
        }
        Ok(guard.clone())
    }

    /// Run an operation on the connection; if the server dropped it under
    /// the operation, reconnect and run it once more.
    fn with_client<T>(&self, op: impl Fn(Client) -> Result<T, String>) -> Result<T, String> {
        match op(self.client()?) {
            Err(e) if is_transport_error(&e) => {
                eprintln!("crow db: the clickhouse connection to {} was lost ({}); reconnecting", self.addr, e);
                let fresh = Self::open(&self.rt, &self.addr, self.options.clone())?;
                *self.client.lock().map_err(|e| e.to_string())? = fresh.clone();
                op(fresh)
            }
            other => other,
        }
    }
}

/// Convert a klickhouse Value to i64 (best effort).
fn value_to_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int8(n) => Some(*n as i64),
        Value::Int16(n) => Some(*n as i64),
        Value::Int32(n) => Some(*n as i64),
        Value::Int64(n) => Some(*n),
        Value::UInt8(n) => Some(*n as i64),
        Value::UInt16(n) => Some(*n as i64),
        Value::UInt32(n) => Some(*n as i64),
        Value::UInt64(n) => Some(*n as i64),
        _ => None,
    }
}

/// Convert a klickhouse Value to f64 (best effort).
fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Float32(n) => Some(*n as f64),
        Value::Float64(n) => Some(*n),
        Value::Int8(n) => Some(*n as f64),
        Value::Int16(n) => Some(*n as f64),
        Value::Int32(n) => Some(*n as f64),
        Value::Int64(n) => Some(*n as f64),
        Value::UInt8(n) => Some(*n as f64),
        Value::UInt16(n) => Some(*n as f64),
        Value::UInt32(n) => Some(*n as f64),
        Value::UInt64(n) => Some(*n as f64),
        _ => None,
    }
}

/// Convert a klickhouse Value to String.
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(bytes) => String::from_utf8_lossy(bytes).to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Determine the column type from the first non-null value.
fn detect_column_type(values: &[Value]) -> &'static str {
    for v in values {
        match v {
            Value::Null => continue,
            Value::Int8(_) | Value::Int16(_) | Value::Int32(_) | Value::Int64(_)
            | Value::UInt8(_) | Value::UInt16(_) | Value::UInt32(_) | Value::UInt64(_) => return "int",
            Value::Float32(_) | Value::Float64(_) => return "float",
            _ => return "str",
        }
    }
    "str"
}

/// Convert Block column data (Vec<Value>) into a CrowFrame.
fn blocks_to_frame(
    col_names: Vec<String>,
    col_data: Vec<Vec<Value>>,
) -> Result<CrowFrame, String> {
    let ncols = col_names.len();
    if ncols == 0 {
        return Ok(CrowFrame::new(vec![], vec![], vec![]));
    }

    let nrows = col_data.first().map(|c| c.len()).unwrap_or(0);
    if nrows == 0 {
        let data: Vec<Column> = (0..ncols).map(|_| Column::Str(Vec::new())).collect();
        let validity: Vec<Vec<bool>> = (0..ncols).map(|_| Vec::new()).collect();
        return Ok(CrowFrame::new(col_names, data, validity));
    }

    let mut columns: Vec<Column> = Vec::with_capacity(ncols);
    let mut validities: Vec<Vec<bool>> = Vec::with_capacity(ncols);

    for col_idx in 0..ncols {
        let values = &col_data[col_idx];
        let mut valid = vec![true; nrows];
        let col_type = detect_column_type(values);

        match col_type {
            "int" => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, v) in values.iter().enumerate() {
                    match v {
                        Value::Null => { vals.push(0); valid[row_idx] = false; }
                        _ => vals.push(value_to_i64(v).unwrap_or(0)),
                    }
                }
                columns.push(Column::Int(vals));
            }
            "float" => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, v) in values.iter().enumerate() {
                    match v {
                        Value::Null => { vals.push(0.0); valid[row_idx] = false; }
                        _ => vals.push(value_to_f64(v).unwrap_or(0.0)),
                    }
                }
                columns.push(Column::Float(vals));
            }
            _ => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, v) in values.iter().enumerate() {
                    match v {
                        Value::Null => { vals.push(String::new()); valid[row_idx] = false; }
                        _ => vals.push(value_to_string(v)),
                    }
                }
                columns.push(Column::Str(vals));
            }
        }
        validities.push(valid);
    }

    Ok(CrowFrame::new(col_names, columns, validities))
}


/// Client-side `?` parameter substitution with proper escaping — the
/// native protocol has no bind API, and telling users to interpolate is
/// exactly the injection hazard crow check exists to flag. Placeholders
/// inside string literals are left alone.
pub fn substitute_params(sql: &str, params: &[serde_json::Value]) -> Result<String, String> {
    let mut out = String::with_capacity(sql.len() + params.len() * 8);
    let mut idx = 0usize;
    let mut in_string = false;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_string = !in_string;
                out.push(c);
            }
            '\\' if in_string => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '?' if !in_string => {
                let p = params.get(idx).ok_or_else(|| {
                    format!("SQL has more ? placeholders than params ({} given)", params.len())
                })?;
                idx += 1;
                match p {
                    serde_json::Value::Null => out.push_str("NULL"),
                    serde_json::Value::Bool(b) => out.push_str(if *b { "1" } else { "0" }),
                    serde_json::Value::Number(n) => out.push_str(&n.to_string()),
                    serde_json::Value::String(v) => {
                        out.push('\'');
                        for ch in v.chars() {
                            match ch {
                                '\'' => out.push_str("\\'"),
                                '\\' => out.push_str("\\\\"),
                                '\0' => {}
                                _ => out.push(ch),
                            }
                        }
                        out.push('\'');
                    }
                    other => {
                        return Err(format!(
                            "cannot bind {} as a ClickHouse param — use a string or number",
                            other
                        ))
                    }
                }
            }
            _ => out.push(c),
        }
    }
    if idx != params.len() {
        return Err(format!(
            "SQL has {} ? placeholders but {} params were given",
            idx,
            params.len()
        ));
    }
    Ok(out)
}

/// ClickHouse server exceptions arrive with a full C++ stack trace.
/// Keep the message (including the useful "In scope SELECT ..." tail),
/// drop the frames.
fn trim_server_error(e: &str) -> String {
    let cut = e
        .find("\n0. Poco::Exception")
        .or_else(|| e.find(" 0. Poco::Exception"))
        .or_else(|| e.find("\n0. "))
        .unwrap_or(e.len());
    let msg = e[..cut].trim();
    // The server doubles the prefix: "DB::Exception: DB::Exception: ..."
    msg.replace("DB::Exception: DB::Exception: ", "DB::Exception: ")
}

/// `?` placeholders outside string literals.
fn placeholder_count(sql: &str) -> usize {
    let mut n = 0;
    let mut in_string = false;
    let mut prev = ' ';
    for c in sql.chars() {
        match c {
            '\'' if prev != '\\' => in_string = !in_string,
            '?' if !in_string => n += 1,
            _ => {}
        }
        prev = c;
    }
    n
}

/// A ClickHouse type name to the kinds the compiler knows.
fn ch_kind(t: &str) -> &'static str {
    let mut t = t.trim();
    loop {
        let inner = t.strip_prefix("Nullable(").or_else(|| t.strip_prefix("LowCardinality("));
        match inner.and_then(|x| x.strip_suffix(')')) {
            Some(x) => t = x,
            None => break,
        }
    }
    if t.starts_with("Int") || t.starts_with("UInt") { "int" }
    else if t.starts_with("Float") || t.starts_with("Decimal") { "float" }
    else if t == "Bool" { "bool" }
    else if t.starts_with("Date") { "datetime" }
    else { "str" }
}

impl CrowDb for CrowClickhouseDb {
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        // parameters are unknown at compile time: NULL stands in for each
        let nulls = vec![serde_json::Value::Null; placeholder_count(sql)];
        let filled = substitute_params(sql, &nulls)?;
        let frame = self.query(&format!("DESCRIBE TABLE ({})", filled), &[])?;
        let col = |name: &str| -> Vec<String> {
            frame.columns.iter().position(|c| c == name)
                .and_then(|i| match frame.data[i].as_ref() { Column::Str(v) => Some(v.clone()), _ => None })
                .unwrap_or_default()
        };
        let names = col("name");
        let types = col("type");
        if names.is_empty() || names.len() != types.len() {
            return Err("clickhouse describe: unexpected DESCRIBE output".into());
        }
        Ok(names.into_iter().zip(types).map(|(n, t)| (n, ch_kind(&t).to_string())).collect())
    }

    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let sql_owned;
        let sql: &str = if params.is_empty() {
            sql
        } else {
            sql_owned = substitute_params(sql, params)?;
            &sql_owned
        };

        self.with_client(|client| Self::off_runtime(|| self.rt.block_on(async {
            let mut stream = client.query_raw(sql).await
                .map_err(|e| format!("clickhouse query error: {}", trim_server_error(&e.to_string())))?;

            let mut all_col_names: Vec<String> = Vec::new();
            let mut all_col_data: Vec<Vec<Value>> = Vec::new();

            while let Some(block_result) = stream.next().await {
                let block = block_result.map_err(|e| format!("clickhouse block error: {}", trim_server_error(&e.to_string())))?;
                if block.rows == 0 {
                    // Capture column names from empty blocks too
                    if all_col_names.is_empty() {
                        all_col_names = block.column_types.keys().cloned().collect();
                        all_col_data = vec![Vec::new(); all_col_names.len()];
                    }
                    continue;
                }

                if all_col_names.is_empty() {
                    all_col_names = block.column_types.keys().cloned().collect();
                    all_col_data = vec![Vec::new(); all_col_names.len()];
                }

                // Append block data
                for (col_idx, (_name, values)) in block.column_data.into_iter().enumerate() {
                    if col_idx < all_col_data.len() {
                        all_col_data[col_idx].extend(values);
                    }
                }
            }

            blocks_to_frame(all_col_names, all_col_data)
        })))
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let sql_owned;
        let sql: &str = if params.is_empty() {
            sql
        } else {
            sql_owned = substitute_params(sql, params)?;
            &sql_owned
        };

        self.with_client(|client| Self::off_runtime(|| self.rt.block_on(async {
            client.execute(sql).await
                .map_err(|e| format!("clickhouse exec error: {}", trim_server_error(&e.to_string())))?;
            // ClickHouse native protocol doesn't return affected row counts
            Ok(0)
        })))
    }

    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        let nrows = frame.nrows();
        if nrows == 0 {
            return Ok(0);
        }
        let ncols = frame.ncols();
        let col_names = frame.columns.join(", ");
        let sql = format!("INSERT INTO {} ({}) FORMAT Native", table, col_names);

        // Build RawRows from CrowFrame
        let mut rows: Vec<RawRow> = Vec::with_capacity(nrows);
        for row_idx in 0..nrows {
            let mut raw_row = RawRow::default();
            for col_idx in 0..ncols {
                let name = &frame.columns[col_idx];
                let valid = frame.validity[col_idx][row_idx];
                if !valid {
                    raw_row.set(name.clone(), Value::Null);
                    continue;
                }
                match frame.data[col_idx].as_ref() {
                    Column::Int(v) => raw_row.set(name.clone(), v[row_idx]),
                    Column::Float(v) => raw_row.set(name.clone(), v[row_idx]),
                    Column::Str(v) => raw_row.set(name.clone(), Value::String(v[row_idx].as_bytes().to_vec())),
                    Column::Bool(v) => raw_row.set(name.clone(), if v[row_idx] { 1u8 } else { 0u8 }),
                    Column::DateTime(v, _) => raw_row.set(name.clone(), v[row_idx]),
                }
            }
            rows.push(raw_row);
        }

        self.with_client(|client| Self::off_runtime(|| self.rt.block_on(async {
            client.insert_native_block(&sql, rows.clone()).await
                .map_err(|e| format!("clickhouse insert error: {}", e))?;
            Ok(nrows as i64)
        })))
    }

    fn transact(&self, _f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        Err("ClickHouse does not support transactions (it is an OLAP engine)".into())
    }

    fn migrate(&self, dir: &str) -> Result<i64, String> {
        crate::migrate::run_migrations(self, dir)
    }

    fn backend_name(&self) -> &str {
        "clickhouse"
    }
}

#[cfg(test)]
mod param_tests {
    use super::{substitute_params, is_transport_error};

    /// The message a dead channel produces is a transport error; a
    /// server's own exception is not
    #[test]
    fn transport_errors_are_told_apart_from_query_errors() {
        assert!(is_transport_error("clickhouse query error: protocol error: failed to send query: channel closed"));
        assert!(is_transport_error("clickhouse exec error: Connection reset by peer (os error 54)"));
        assert!(!is_transport_error("clickhouse query error: Code: 60. DB::Exception: Table finch_trading.nothing does not exist"));
        assert!(!is_transport_error("clickhouse query error: Code: 62. DB::Exception: Syntax error"));
    }
    use serde_json::json;

    #[test]
    fn substitutes_types_and_counts() {
        let sql = substitute_params(
            "SELECT * FROM t WHERE s = ? AND n = ? AND f = ? AND b = ?",
            &[json!("VIVK"), json!(42), json!(1.5), json!(true)],
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE s = 'VIVK' AND n = 42 AND f = 1.5 AND b = 1");
        assert!(substitute_params("? ?", &[json!(1)]).is_err());
        assert!(substitute_params("?", &[json!(1), json!(2)]).is_err());
    }

    #[test]
    fn escapes_injection_attempts() {
        let sql = substitute_params(
            "SELECT * FROM t WHERE s = ?",
            &[json!("x'; DROP TABLE t; --")],
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE s = 'x\\'; DROP TABLE t; --'");
        let sql2 = substitute_params("SELECT ?", &[json!("a\\b")]).unwrap();
        assert_eq!(sql2, "SELECT 'a\\\\b'");
    }

    #[test]
    fn placeholders_inside_strings_are_literal() {
        let sql = substitute_params("SELECT 'a?b', ?", &[json!(7)]).unwrap();
        assert_eq!(sql, "SELECT 'a?b', 7");
    }
}
