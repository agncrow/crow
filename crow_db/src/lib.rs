//! `crow_db` — Crow database module.
//!
//! Provides unified connectivity to SQLite, PostgreSQL, and ClickHouse
//! with connection pooling, bulk operations, transactions, and migrations.
//!
//! ```text
//! use db
//!
//! conn = connect['sqlite://data.db']
//! exec[conn, 'CREATE TABLE users [id INTEGER PRIMARY KEY, name TEXT]']
//! users = query[conn, 'SELECT * FROM users']
//! ```

use std::sync::Arc;
use crow_runtime::frame::CrowFrame;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "clickhouse")]
pub mod clickhouse;

pub mod migrate;

/// Operations available inside a transaction — exec and query only.
/// No Send + Sync requirement (transaction handles are single-threaded).
pub trait DbTx {
    /// Execute a SELECT and return results as a frame.
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String>;

    /// Execute DDL/DML. Returns affected row count.
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String>;
}

/// A database connection handle — internally a pool.
/// Users never manage individual connections.
pub trait CrowDb: Send + Sync {
    /// Execute a SELECT and return results as a frame.
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String>;

    /// Execute DDL/DML (CREATE, INSERT, UPDATE, DELETE). Returns affected row count.
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String>;

    /// Bulk insert a frame into a table using backend-specific fast paths.
    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String>;

    /// Execute a function inside a transaction.
    /// Checks out one connection, begins transaction, calls f, commits or rolls back.
    fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String>;

    /// Run forward-only SQL migrations from a directory.
    fn migrate(&self, dir: &str) -> Result<i64, String>;

    /// Backend name for error messages.
    fn backend_name(&self) -> &str;

    /// The columns a statement would return — name and coarse kind
    /// (`int` `float` `str` `bool` `datetime`) — without running it. The
    /// compiler asks so a frame read from a query has typed cells.
    fn describe(&self, _sql: &str) -> Result<Vec<(String, String)>, String> {
        Err(format!("{}: describe is not supported", self.backend_name()))
    }
}

/// Thread-safe database handle — wraps a trait object.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<dyn CrowDb>,
}

impl DbHandle {
    pub fn new(db: impl CrowDb + 'static) -> Self {
        DbHandle { inner: Arc::new(db) }
    }

    pub fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.inner.query(sql, params)
    }

    /// Column names and kinds of a statement's result, without running it.
    pub fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        self.inner.describe(sql)
    }

    pub fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.inner.exec(sql, params)
    }

    pub fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        self.inner.insert(table, frame)
    }

    pub fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        self.inner.transact(f)
    }

    pub fn migrate(&self, dir: &str) -> Result<i64, String> {
        self.inner.migrate(dir)
    }

    pub fn backend_name(&self) -> &str {
        self.inner.backend_name()
    }
}

impl std::fmt::Debug for DbHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DbHandle({})", self.inner.backend_name())
    }
}

/// Connect to a database using a URL scheme to dispatch to the correct backend.
///
/// - `sqlite://path` → SQLite
/// - `postgres://user:pass@host/db` → PostgreSQL
/// - `clickhouse://user:pass@host:port/db` → ClickHouse (native TCP)
pub fn connect(url: &str, pool_size: Option<usize>) -> Result<DbHandle, String> {
    if url.starts_with("sqlite://") {
        #[cfg(feature = "sqlite")]
        {
            let path = &url["sqlite://".len()..];
            let db = sqlite::CrowSqliteDb::open(path)?;
            let _ = pool_size; // SQLite ignores pool_size (single connection)
            Ok(DbHandle::new(db))
        }
        #[cfg(not(feature = "sqlite"))]
        {
            Err("SQLite support not enabled — compile with feature 'sqlite'".into())
        }
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        #[cfg(feature = "postgres")]
        {
            let size = pool_size.unwrap_or(10);
            let db = postgres::CrowPostgresDb::connect(url, size)?;
            Ok(DbHandle::new(db))
        }
        #[cfg(not(feature = "postgres"))]
        {
            let _ = pool_size;
            Err("PostgreSQL support not enabled — compile with feature 'postgres'".into())
        }
    } else if url.starts_with("clickhouse://") {
        #[cfg(feature = "clickhouse")]
        {
            let _ = pool_size; // klickhouse manages its own connection
            let db = clickhouse::CrowClickhouseDb::connect(url)?;
            Ok(DbHandle::new(db))
        }
        #[cfg(not(feature = "clickhouse"))]
        {
            let _ = pool_size;
            Err("ClickHouse support not enabled — compile with feature 'clickhouse'".into())
        }
    } else {
        Err(format!("unknown database URL scheme: {}", url))
    }
}

