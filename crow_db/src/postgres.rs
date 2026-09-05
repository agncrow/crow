//! PostgreSQL backend for crow_db.
//!
//! Uses `tokio-postgres` + `deadpool-postgres` for async connection pooling,
//! bridged to sync via a shared `tokio::Runtime`.

use std::sync::Arc;
use crow_runtime::frame::{CrowFrame, Column};
use crate::{CrowDb, DbTx};
use tokio::runtime::Runtime;
use deadpool_postgres::{Config, Pool, ManagerConfig, RecyclingMethod};
use tokio_postgres::NoTls;

/// PostgreSQL database — wraps a deadpool connection pool + tokio runtime.
pub struct CrowPostgresDb {
    pool: Pool,
    rt: Arc<Runtime>,
}

impl CrowPostgresDb {
    /// Connect to a PostgreSQL database using a connection URL.
    /// `url` should be like `postgres://user:pass@host:port/dbname`
    pub fn connect(url: &str, pool_size: usize) -> Result<Self, String> {
        let rt = Runtime::new().map_err(|e| format!("failed to create tokio runtime: {}", e))?;

        let mut cfg = Config::new();
        cfg.url = Some(url.to_string());
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        cfg.pool = Some(deadpool_postgres::PoolConfig::new(pool_size));

        let pool = cfg.create_pool(Some(deadpool_postgres::Runtime::Tokio1), NoTls)
            .map_err(|e| format!("failed to create postgres pool: {}", e))?;

        // Test the connection
        CrowPostgresDb::off_runtime(|| rt.block_on(async {
            let client = pool.get().await
                .map_err(|e| format!("postgres connection failed: {}", e))?;
            client.simple_query("SELECT 1").await
                .map_err(|e| format!("postgres ping failed: {}", e))?;
            Ok::<(), String>(())
        }))?;

        Ok(Self { pool, rt: Arc::new(rt) })
    }
}

/// Crow's one placeholder is `?` on every backend; Postgres wants `$1`,
/// `$2`, … — rewrite outside string literals so a `?` in quoted text
/// survives.
fn numbered_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0;
    let mut in_str = false;
    for c in sql.chars() {
        match c {
            '\'' => { in_str = !in_str; out.push(c); }
            '?' if !in_str => { n += 1; out.push('$'); out.push_str(&n.to_string()); }
            _ => out.push(c),
        }
    }
    out
}

/// The server's own message, not tokio-postgres's one-word summary.
fn describe(e: tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => match db.detail() {
            Some(d) => format!("{} ({})", db.message(), d),
            None => db.message().to_string(),
        },
        None => e.to_string(),
    }
}

/// Convert Crow's params to typed Postgres parameters. Every parameter is
/// declared (text, int8, float8, bool) so a cast in the SQL — `?::date`,
/// `?::my_enum` — is a cast from text, and a comparison against a numeric
/// column is numeric.
fn json_to_pg_params(params: &[serde_json::Value]) -> (Vec<Box<dyn tokio_postgres::types::ToSql + Sync>>, Vec<tokio_postgres::types::Type>) {
    use tokio_postgres::types::Type;
    let mut values: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::with_capacity(params.len());
    let mut types: Vec<Type> = Vec::with_capacity(params.len());
    for v in params {
        match v {
            serde_json::Value::String(s) => { values.push(Box::new(s.clone())); types.push(Type::TEXT); }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { values.push(Box::new(i)); types.push(Type::INT8); }
                else { values.push(Box::new(n.as_f64().unwrap_or(0.0))); types.push(Type::FLOAT8); }
            }
            serde_json::Value::Bool(b) => { values.push(Box::new(*b)); types.push(Type::BOOL); }
            serde_json::Value::Null => { values.push(Box::new(Option::<String>::None)); types.push(Type::TEXT); }
            _ => { values.push(Box::new(v.to_string())); types.push(Type::TEXT); }
        }
    }
    (values, types)
}

/// Build a CrowFrame from tokio-postgres rows.
fn rows_to_frame(rows: &[tokio_postgres::Row]) -> Result<CrowFrame, String> {
    if rows.is_empty() {
        return Ok(CrowFrame::new(vec![], vec![], vec![]));
    }

    let first = &rows[0];
    let ncols = first.columns().len();
    let col_names: Vec<String> = first.columns().iter().map(|c| c.name().to_string()).collect();
    let nrows = rows.len();

    let mut columns: Vec<Column> = Vec::with_capacity(ncols);
    let mut validities: Vec<Vec<bool>> = Vec::with_capacity(ncols);

    for col_idx in 0..ncols {
        let col_type = first.columns()[col_idx].type_();
        let mut valid = vec![true; nrows];

        use tokio_postgres::types::Type;
        match *col_type {
            Type::INT2 | Type::INT4 | Type::INT8 => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in rows.iter().enumerate() {
                    let v: Option<i64> = match *col_type {
                        Type::INT2 => row.get::<_, Option<i16>>(col_idx).map(|v| v as i64),
                        Type::INT4 => row.get::<_, Option<i32>>(col_idx).map(|v| v as i64),
                        Type::INT8 => row.get::<_, Option<i64>>(col_idx),
                        _ => unreachable!(),
                    };
                    match v {
                        Some(n) => vals.push(n),
                        None => { vals.push(0); valid[row_idx] = false; }
                    }
                }
                columns.push(Column::Int(vals));
            }
            Type::FLOAT4 | Type::FLOAT8 => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in rows.iter().enumerate() {
                    let v: Option<f64> = match *col_type {
                        Type::FLOAT4 => row.get::<_, Option<f32>>(col_idx).map(|v| v as f64),
                        Type::FLOAT8 => row.get::<_, Option<f64>>(col_idx),
                        _ => unreachable!(),
                    };
                    match v {
                        Some(n) => vals.push(n),
                        None => { vals.push(0.0); valid[row_idx] = false; }
                    }
                }
                columns.push(Column::Float(vals));
            }
            Type::BOOL => {
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in rows.iter().enumerate() {
                    match row.get::<_, Option<bool>>(col_idx) {
                        Some(b) => vals.push(b),
                        None => { vals.push(false); valid[row_idx] = false; }
                    }
                }
                columns.push(Column::Bool(vals));
            }
            _ => {
                // Default: treat as string (TEXT, VARCHAR, etc.)
                let mut vals = Vec::with_capacity(nrows);
                for (row_idx, row) in rows.iter().enumerate() {
                    match row.get::<_, Option<String>>(col_idx) {
                        Some(s) => vals.push(s),
                        None => { vals.push(String::new()); valid[row_idx] = false; }
                    }
                }
                columns.push(Column::Str(vals));
            }
        }
        validities.push(valid);
    }

    Ok(CrowFrame::new(col_names, columns, validities))
}

fn pg_kind(t: &tokio_postgres::types::Type) -> &'static str {
    use tokio_postgres::types::Type;
    match *t {
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => "int",
        Type::FLOAT4 | Type::FLOAT8 | Type::NUMERIC => "float",
        Type::BOOL => "bool",
        Type::TIMESTAMP | Type::TIMESTAMPTZ | Type::DATE => "datetime",
        _ => "str",
    }
}

impl CrowPostgresDb {
    /// A compiled Crow program is a tokio runtime; blocking on this driver's
    /// own runtime from inside it is not allowed, so the blocking work
    /// runs on a scoped thread when a runtime is already current.
    fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::scope(|sc| sc.spawn(f).join().expect("postgres worker thread panicked"))
        } else {
            f()
        }
    }
}

impl CrowDb for CrowPostgresDb {
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        Self::off_runtime(|| self.rt.block_on(async {
            let client = self.pool.get().await
                .map_err(|e| format!("postgres pool error: {}", e))?;
            let stmt = client.prepare(&numbered_placeholders(sql)).await
                .map_err(|e| format!("postgres describe error: {}", describe(e)))?;
            Ok(stmt.columns().iter().map(|c| (c.name().to_string(), pg_kind(c.type_()).to_string())).collect())
        }))
    }

    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let (pg_params, pg_types) = json_to_pg_params(params);
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|b| b.as_ref()).collect();

        Self::off_runtime(|| self.rt.block_on(async {
            let client = self.pool.get().await
                .map_err(|e| format!("postgres pool error: {}", e))?;
            let stmt = client.prepare_typed(&numbered_placeholders(sql), &pg_types).await
                .map_err(|e| format!("postgres query error: {}", describe(e)))?;
            let rows = client.query(&stmt, &param_refs).await
                .map_err(|e| format!("postgres query error: {}", describe(e)))?;
            rows_to_frame(&rows)
        }))
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let (pg_params, pg_types) = json_to_pg_params(params);
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|b| b.as_ref()).collect();

        Self::off_runtime(|| self.rt.block_on(async {
            let client = self.pool.get().await
                .map_err(|e| format!("postgres pool error: {}", e))?;
            let stmt = client.prepare_typed(&numbered_placeholders(sql), &pg_types).await
                .map_err(|e| format!("postgres exec error: {}", describe(e)))?;
            let n = client.execute(&stmt, &param_refs).await
                .map_err(|e| format!("postgres exec error: {}", describe(e)))?;
            Ok(n as i64)
        }))
    }

    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        let nrows = frame.nrows();
        if nrows == 0 {
            return Ok(0);
        }
        let ncols = frame.ncols();
        let col_names = frame.columns.join(", ");
        let placeholders: Vec<String> = (1..=ncols).map(|i| format!("${}", i)).collect();
        let placeholders_str = placeholders.join(", ");
        let sql = format!("INSERT INTO {} ({}) VALUES ({})", table, col_names, placeholders_str);

        Self::off_runtime(|| self.rt.block_on(async {
            let mut client = self.pool.get().await
                .map_err(|e| format!("postgres pool error: {}", e))?;

            let tx = client.transaction().await
                .map_err(|e| format!("postgres begin error: {}", e))?;

            for row_idx in 0..nrows {
                let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::with_capacity(ncols);
                for col_idx in 0..ncols {
                    let valid = frame.validity[col_idx][row_idx];
                    if !valid {
                        params.push(Box::new(Option::<String>::None));
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
                let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                    params.iter().map(|b| b.as_ref()).collect();
                tx.execute(&sql, &param_refs).await
                    .map_err(|e| format!("postgres insert error: {}", e))?;
            }

            tx.commit().await
                .map_err(|e| format!("postgres commit error: {}", e))?;
            Ok(nrows as i64)
        }))
    }

    fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        self.rt.block_on(async {
            let client = self.pool.get().await
                .map_err(|e| format!("postgres pool error: {}", e))?;
            client.simple_query("BEGIN").await
                .map_err(|e| format!("postgres begin error: {}", e))?;

            let wrapper = PgTransactWrapper {
                client: &client,
                rt: Arc::clone(&self.rt),
            };

            match f(&wrapper) {
                Ok(()) => {
                    client.simple_query("COMMIT").await
                        .map_err(|e| format!("postgres commit error: {}", e))?;
                    Ok(())
                }
                Err(e) => {
                    let _ = client.simple_query("ROLLBACK").await;
                    Err(e)
                }
            }
        })
    }

    fn migrate(&self, dir: &str) -> Result<i64, String> {
        crate::migrate::run_migrations(self, dir)
    }

    fn backend_name(&self) -> &str {
        "postgres"
    }
}

/// Transaction wrapper for PostgreSQL.
struct PgTransactWrapper<'a> {
    client: &'a deadpool_postgres::Client,
    rt: Arc<Runtime>,
}

impl DbTx for PgTransactWrapper<'_> {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let (pg_params, pg_types) = json_to_pg_params(params);
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|b| b.as_ref()).collect();

        self.rt.block_on(async {
            let stmt = self.client.prepare_typed(&numbered_placeholders(sql), &pg_types).await
                .map_err(|e| format!("postgres query error: {}", describe(e)))?;
            let rows = self.client.query(&stmt, &param_refs).await
                .map_err(|e| format!("postgres query error: {}", describe(e)))?;
            rows_to_frame(&rows)
        })
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let (pg_params, pg_types) = json_to_pg_params(params);
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|b| b.as_ref()).collect();

        self.rt.block_on(async {
            let stmt = self.client.prepare_typed(&numbered_placeholders(sql), &pg_types).await
                .map_err(|e| format!("postgres exec error: {}", describe(e)))?;
            let n = self.client.execute(&stmt, &param_refs).await
                .map_err(|e| format!("postgres exec error: {}", describe(e)))?;
            Ok(n as i64)
        })
    }
}

