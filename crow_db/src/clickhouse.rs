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
    client: Client,
    rt: Arc<Runtime>,
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

        let client = Self::off_runtime(|| rt.block_on(async {
            Client::connect(&addr, options).await
                .map_err(|e| format!("clickhouse connection failed: {}", e))
        }))?;

        // Ping to verify connection
        Self::off_runtime(|| rt.block_on(async {
            client.execute("SELECT 1").await
                .map_err(|e| format!("clickhouse ping failed: {}", e))
        }))?;

        Ok(Self { client, rt: Arc::new(rt) })
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


impl CrowDb for CrowClickhouseDb {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        if !params.is_empty() {
            return Err("ClickHouse does not support parameterized queries via native protocol; \
                        use string interpolation in the SQL".into());
        }

        Self::off_runtime(|| self.rt.block_on(async {
            let mut stream = self.client.query_raw(sql).await
                .map_err(|e| format!("clickhouse query error: {}", e))?;

            let mut all_col_names: Vec<String> = Vec::new();
            let mut all_col_data: Vec<Vec<Value>> = Vec::new();

            while let Some(block_result) = stream.next().await {
                let block = block_result.map_err(|e| format!("clickhouse block error: {}", e))?;
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
        }))
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        if !params.is_empty() {
            return Err("ClickHouse does not support parameterized queries via native protocol; \
                        use string interpolation in the SQL".into());
        }

        Self::off_runtime(|| self.rt.block_on(async {
            self.client.execute(sql).await
                .map_err(|e| format!("clickhouse exec error: {}", e))?;
            // ClickHouse native protocol doesn't return affected row counts
            Ok(0)
        }))
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

        Self::off_runtime(|| self.rt.block_on(async {
            self.client.insert_native_block(&sql, rows).await
                .map_err(|e| format!("clickhouse insert error: {}", e))?;
            Ok(nrows as i64)
        }))
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