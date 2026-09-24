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

/// Resolves when the work under `flag` — or the whole program — should
/// stop: a driver that can stop a running query on its server races the
/// query against this and stops it.
#[cfg(any(feature = "clickhouse", feature = "postgres"))]
pub(crate) async fn stop_requested(flag: Option<Arc<std::sync::atomic::AtomicBool>>) {
    loop {
        if crow_runtime::cancel::stopped(&flag) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Operations available inside a transaction — exec and query only.
/// No Send + Sync requirement (transaction handles are single-threaded).
pub trait DbTx {
    /// Execute a SELECT and return results as a frame.
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String>;

    /// Execute DDL/DML. Returns affected row count.
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String>;
}

/// An array parameter stands for a list: its `?` becomes `(?, ?, …)` and
/// its elements take those places — `WHERE id IN ?` on every backend.
/// Placeholders inside single-quoted strings are left alone.
pub fn expand_array_params(sql: &str, params: &[serde_json::Value]) -> (String, Vec<serde_json::Value>) {
    if !params.iter().any(|p| p.is_array()) {
        return (sql.to_string(), params.to_vec());
    }
    let mut out = String::with_capacity(sql.len() + 16);
    let mut flat: Vec<serde_json::Value> = Vec::with_capacity(params.len());
    let mut idx = 0usize;
    let mut in_string = false;
    for c in sql.chars() {
        match c {
            '\'' => { in_string = !in_string; out.push(c); }
            '?' if !in_string => {
                match params.get(idx) {
                    Some(serde_json::Value::Array(items)) => {
                        if items.is_empty() {
                            out.push_str("(NULL)");
                        } else {
                            out.push('(');
                            for (k, item) in items.iter().enumerate() {
                                if k > 0 { out.push_str(", "); }
                                out.push('?');
                                flat.push(item.clone());
                            }
                            out.push(')');
                        }
                    }
                    Some(other) => { out.push('?'); flat.push(other.clone()); }
                    None => out.push('?'),
                }
                idx += 1;
            }
            _ => out.push(c),
        }
    }
    (out, flat)
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

    /// Open a transaction for a handler's scope on a connection of its own.
    /// `None` for a backend without transactions (ClickHouse): its writes
    /// land as they are made.
    fn begin_scope(&self) -> Option<Result<Box<dyn ScopeTx>, String>> {
        None
    }
}

/// A transaction a handler opened on one connection, held for the
/// handler's length: its reads see its writes, and it ends as one.
pub trait ScopeTx: Send {
    fn query(&mut self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String>;
    fn exec(&mut self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String>;
    fn finish(self: Box<Self>, commit: bool) -> Result<(), String>;
}

thread_local! {
    /// The transactions the handler running on this thread opened, by handle,
    /// in the order it opened them — `None` outside a handler.
    static SCOPE: std::cell::RefCell<Option<Vec<(usize, Box<dyn ScopeTx>)>>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// The tables written through any handle on this thread since the last
    /// `take_written`: what an interpreted handler wrote, for the live
    /// runtime to wake the readers of (a compiled handler knows statically).
    static WRITTEN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_written(sql: &str) {
    let (_, tables) = crow_runtime::sql::tables_of_sql(sql);
    if !tables.is_empty() {
        WRITTEN.with(|w| w.borrow_mut().extend(tables));
    }
}

/// The tables written on this thread since the last call, each once.
pub fn take_written() -> Vec<String> {
    let mut out = WRITTEN.with(|w| std::mem::take(&mut *w.borrow_mut()));
    out.sort();
    out.dedup();
    out
}

fn scope_begin() {
    SCOPE.with(|s| *s.borrow_mut() = Some(Vec::new()));
}

fn scope_finish(commit: bool) -> Result<(), String> {
    let txs = SCOPE.with(|s| s.borrow_mut().take()).unwrap_or_default();
    let mut first_error = None;
    for (_, tx) in txs {
        // a commit that fails rolls the rest back: the handler lands nothing more
        let ok = commit && first_error.is_none();
        if let Err(e) = tx.finish(ok) {
            first_error.get_or_insert(e);
        }
    }
    match first_error {
        Some(e) => Err(format!("the database refused the commit: {}", e)),
        None => Ok(()),
    }
}

/// One cell of a frame as a statement parameter: null when missing.
fn cell_json(frame: &CrowFrame, col: usize, row: usize) -> serde_json::Value {
    use crow_runtime::frame::Column;
    if !frame.validity[col][row] {
        return serde_json::Value::Null;
    }
    match frame.data[col].as_ref() {
        Column::Int(v) => serde_json::json!(v[row]),
        Column::Float(v) => serde_json::json!(v[row]),
        Column::Str(v) => serde_json::json!(v[row]),
        Column::Bool(v) => serde_json::json!(v[row]),
        Column::DateTime(v, _) => serde_json::json!(v[row]),
    }
}

/// Whether a statement writes: what opens a handler's transaction.
fn writes(sql: &str) -> bool {
    let first = sql.trim_start().split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_ascii_uppercase();
    matches!(first.as_str(), "INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "UPSERT" | "MERGE" | "CREATE" | "DROP" | "ALTER")
}

/// Thread-safe database handle — wraps a trait object.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<dyn CrowDb>,
}

impl DbHandle {
    pub fn new(db: impl CrowDb + 'static) -> Self {
        crow_runtime::txn::register(crow_runtime::txn::Hooks { begin: scope_begin, finish: scope_finish });
        DbHandle { inner: Arc::new(db) }
    }

    /// Run `f` on this handle's transaction when a handler is running on
    /// this thread: its first write opens one, and every read after that is
    /// inside it. `None` outside a handler, for a read before any write, and
    /// for a backend without transactions.
    fn in_scope<T>(&self, write: bool, f: impl FnOnce(&mut dyn ScopeTx) -> Result<T, String>) -> Option<Result<T, String>> {
        let key = Arc::as_ptr(&self.inner) as *const () as usize;
        SCOPE.with(|s| {
            let mut s = s.borrow_mut();
            let txs = s.as_mut()?;
            if let Some((_, tx)) = txs.iter_mut().find(|(k, _)| *k == key) {
                return Some(f(tx.as_mut()));
            }
            if !write {
                return None;
            }
            match self.inner.begin_scope()? {
                Ok(mut tx) => {
                    let r = f(tx.as_mut());
                    txs.push((key, tx));
                    Some(r)
                }
                Err(e) => Some(Err(e)),
            }
        })
    }

    pub fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let (sql, params) = expand_array_params(sql, params);
        if writes(&sql) {
            note_written(&sql);
        }
        if let Some(r) = self.in_scope(writes(&sql), |tx| tx.query(&sql, &params)) {
            return r;
        }
        self.inner.query(&sql, &params)
    }

    /// Column names and kinds of a statement's result, without running it.
    pub fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        self.inner.describe(sql)
    }

    pub fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let (sql, params) = expand_array_params(sql, params);
        note_written(&sql);
        if let Some(r) = self.in_scope(true, |tx| tx.exec(&sql, &params)) {
            return r;
        }
        self.inner.exec(&sql, &params)
    }

    pub fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        WRITTEN.with(|w| w.borrow_mut().push(table.to_string()));
        // inside a handler the rows go into its transaction, one by one
        let rows = |tx: &mut dyn ScopeTx| -> Result<i64, String> {
            if frame.nrows() == 0 {
                return Ok(0);
            }
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({})",
                table,
                frame.columns.join(", "),
                vec!["?"; frame.ncols()].join(", ")
            );
            for r in 0..frame.nrows() {
                let params: Vec<serde_json::Value> = (0..frame.ncols()).map(|c| cell_json(frame, c, r)).collect();
                tx.exec(&sql, &params)?;
            }
            Ok(frame.nrows() as i64)
        };
        if let Some(r) = self.in_scope(true, rows) {
            return r;
        }
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
    // One handle per server and process: parts that each connect to the
    // same Postgres or ClickHouse share its pool, so the program's queries
    // to one server are bounded by one pool, not one per declaration. A
    // sqlite file is one connection already, and `:memory:` is a new
    // database each time by design.
    static SHARED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, Option<usize>), DbHandle>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let shared = !url.starts_with("sqlite://");
    if shared {
        if let Some(h) = SHARED.lock().unwrap_or_else(|p| p.into_inner()).get(&(url.to_string(), pool_size)) {
            return Ok(h.clone());
        }
    }
    let handle = open_handle(url, pool_size)?;
    if shared {
        SHARED.lock().unwrap_or_else(|p| p.into_inner()).insert((url.to_string(), pool_size), handle.clone());
    }
    Ok(handle)
}

fn open_handle(url: &str, pool_size: Option<usize>) -> Result<DbHandle, String> {
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
            let _ = pool_size; // the driver keeps its own pool (clickhouse.rs, POOL)
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

