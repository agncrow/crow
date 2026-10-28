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

// A runtime function answers its reason, or stops with it: it never
// panics (docs/bug-classes.md §3). `cargo clippy`, which the gate and CI
// run, refuses a panic, an unwrap or an expect that is not allowed with the
// invariant that makes it safe.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable, clippy::todo, clippy::unimplemented))]

use std::sync::Arc;
use crow_runtime::frame::CrowFrame;

#[cfg(feature = "sqlite")]
pub mod sqlite;

pub mod migrate;

/// The handle for a driver written in Crow: ClickHouse, Postgres.
pub mod driven;

pub mod schema;
/// What a recording kept, and the replay that answers from it.
pub mod fixtures;
/// What a database says a statement answers: `query`'s provider (the
/// compiler's; a generated program builds without SQLite and needs none).
#[cfg(feature = "sqlite")]
pub mod describe;
/// The kinds SQLite gives a SELECT's computed columns, and which columns
/// a statement can hold NULL in, read from the statement itself.
#[cfg(feature = "sqlite")]
pub mod sql_types;

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

    /// Whether `describe` answers each column's kind as the compiler reads
    /// it already — computed columns typed, those that can hold NULL marked
    /// `?` — as a replay's records do, which are the compiler's answers;
    /// otherwise the statement is read for what the backend leaves out
    /// (`describe::described`).
    fn describes_fully(&self) -> bool {
        false
    }

    /// Run `f` holding the database's lock for changing its schema, in one
    /// transaction where the database has them: two processes starting at
    /// once reconcile one after the other (crate::schema). A backend with
    /// neither (ClickHouse) runs `f` as it is, and the statements it is
    /// handed say `IF NOT EXISTS`.
    fn schema_lock(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        f(&Unlocked(self))
    }

    /// Open a transaction for a handler's scope on a connection of its own.
    /// `None` for a backend without transactions (ClickHouse): its writes
    /// land as they are made.
    fn begin_scope(&self) -> Option<Result<Box<dyn ScopeTx>, String>> {
        None
    }
}

/// A backend's statements outside any transaction: what a backend without
/// a schema lock reconciles through.
pub(crate) struct Unlocked<'a, D: ?Sized>(&'a D);

impl<D: CrowDb + ?Sized> DbTx for Unlocked<'_, D> {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.0.query(sql, params)
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.0.exec(sql, params)
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
    /// Each write on this thread since the last `take_write_keys`, with what
    /// it says about the rows it touched: which derivations it can have
    /// changed (invalidation by key, session-layer-design.md §3.5).
    static WRITE_KEYS: std::cell::RefCell<Vec<crow_runtime::sql::WriteKeys>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_written(sql: &str, params: &[serde_json::Value]) {
    let (_, tables) = crow_runtime::sql::tables_of_sql(sql);
    if !tables.is_empty() {
        // what the write says about its rows, or, when it says nothing a
        // reader can use, that it touched every row of each table
        let keys = crow_runtime::sql::write_keys(sql, params);
        WRITE_KEYS.with(|w| {
            let mut w = w.borrow_mut();
            match keys {
                Some(k) if tables.len() == 1 && tables[0] == k.table => w.push(k),
                _ => w.extend(tables.iter().map(|t| whole_table(t))),
            }
        });
        WRITTEN.with(|w| w.borrow_mut().extend(tables));
    }
}

/// A write that touched any row of a table.
fn whole_table(table: &str) -> crow_runtime::sql::WriteKeys {
    crow_runtime::sql::WriteKeys { table: table.to_string(), known: vec![], changed: vec![], exact: false }
}

/// The writes made on this thread since the last call, in order, each with
/// what it says about the rows it touched; the tables' record goes with
/// them (`take_written`).
pub fn take_write_keys() -> Vec<crow_runtime::sql::WriteKeys> {
    WRITTEN.with(|w| w.borrow_mut().clear());
    WRITE_KEYS.with(|w| std::mem::take(&mut *w.borrow_mut()))
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

/// Whether a handler (or a transaction of `transact_calling`'s own) is
/// running on this thread: its statements are routed to its transactions.
fn in_handler() -> bool {
    SCOPE.with(|s| s.borrow().is_some())
}

/// A handler's transaction as the `DbTx` a `transact` function is handed.
struct ScopeView(std::cell::RefCell<Box<dyn ScopeTx>>);

impl DbTx for ScopeView {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.0.try_borrow_mut().map_err(|_| "the transaction is busy".to_string())?.query(sql, params)
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.0.try_borrow_mut().map_err(|_| "the transaction is busy".to_string())?.exec(sql, params)
    }
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
        Column::Str(v) => serde_json::json!(&v[row]),
        Column::Bool(v) => serde_json::json!(v[row]),
        Column::DateTime(v, _) => serde_json::json!(v[row]),
        Column::Vectors(w, v) => serde_json::json!(&v[row * w..(row + 1) * w]),
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
        let key = self.key();
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
            note_written(&sql, &params);
        }
        if let Some(r) = self.in_scope(writes(&sql), |tx| tx.query(&sql, &params)) {
            return r;
        }
        self.inner.query(&sql, &params)
    }

    /// Column names and kinds of a statement's result, without running it:
    /// each column typed, a computed one too, and marked `?` where it can
    /// hold NULL — one answer for the compiler and for `describe[conn,
    /// sql]` (`describe::described`).
    pub fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        #[cfg(feature = "sqlite")]
        {
            describe::described(self, sql)
        }
        #[cfg(not(feature = "sqlite"))]
        {
            self.inner.describe(sql)
        }
    }

    /// What the backend itself says a statement answers, before the
    /// statement is read for what it leaves out.
    #[cfg(feature = "sqlite")]
    pub(crate) fn described_by_backend(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        self.inner.describe(sql)
    }

    /// Whether the backend's own answer is the whole one (a replay's).
    #[cfg(feature = "sqlite")]
    pub(crate) fn describes_fully(&self) -> bool {
        self.inner.describes_fully()
    }

    pub fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let (sql, params) = expand_array_params(sql, params);
        note_written(&sql, &params);
        if let Some(r) = self.in_scope(true, |tx| tx.exec(&sql, &params)) {
            return r;
        }
        self.inner.exec(&sql, &params)
    }

    pub fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        WRITTEN.with(|w| w.borrow_mut().push(table.to_string()));
        // a frame's rows at once: read as touching the table
        WRITE_KEYS.with(|w| w.borrow_mut().push(whole_table(table)));
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

    /// `transact[conn, f]`: `f` inside a transaction — the backend's own,
    /// on one connection (BEGIN; COMMIT when `f` answers Ok, ROLLBACK when
    /// it answers Err). Inside a handler, whose statements on this handle
    /// are one transaction already (`in_scope`), a savepoint on it:
    /// released when `f` answers Ok, rolled back to when it answers Err,
    /// the handler's other writes untouched.
    pub fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        if !in_handler() {
            return self.inner.transact(f);
        }
        match self.in_scope(true, |tx| tx.exec("SAVEPOINT crow_transact", &[])) {
            // a backend without a scope of a handler's own has no savepoint
            // to make: its own transaction
            None => return self.inner.transact(f),
            Some(Err(e)) => return Err(e),
            Some(Ok(_)) => {}
        }
        // the scope's transaction, taken out for the call: `f` speaks to
        // it through the view, and a statement through another handle
        // meanwhile finds the scope free
        let Some(tx) = self.take_scope_tx() else { return Err("the handler's transaction is gone".to_string()) };
        let view = ScopeView(std::cell::RefCell::new(tx));
        let outcome = f(&view);
        self.put_scope_tx(view.0.into_inner());
        self.end_savepoint(outcome)
    }

    /// `transact` around a function the interpreter calls back, whose
    /// statements come through this handle itself (its `tx` is the handle):
    /// outside a handler, a scope of the transaction's own, so they are
    /// routed to one connection as a handler's are — BEGIN and COMMIT on
    /// the handle where the backend has no such scope (SQLite in memory
    /// is one connection; ClickHouse has no transactions); inside a
    /// handler, a savepoint on its transaction.
    pub fn transact_calling(&self, f: &mut dyn FnMut() -> Result<(), String>) -> Result<(), String> {
        if in_handler() {
            match self.in_scope(true, |tx| tx.exec("SAVEPOINT crow_transact", &[])) {
                None => return self.transact_plainly(f),
                Some(Err(e)) => return Err(e),
                Some(Ok(_)) => {}
            }
            let outcome = f();
            return self.end_savepoint(outcome);
        }
        match self.inner.begin_scope() {
            Some(Ok(tx)) => {
                SCOPE.with(|s| *s.borrow_mut() = Some(vec![(self.key(), tx)]));
                let outcome = f();
                let finished = scope_finish(outcome.is_ok());
                outcome.and(finished)
            }
            Some(Err(e)) => Err(e),
            None => self.transact_plainly(f),
        }
    }

    /// BEGIN, `f`, then COMMIT or ROLLBACK on the handle itself.
    fn transact_plainly(&self, f: &mut dyn FnMut() -> Result<(), String>) -> Result<(), String> {
        if self.inner.backend_name() == "clickhouse" {
            return Err("ClickHouse does not support transactions (it is an OLAP engine)".to_string());
        }
        self.inner.exec("BEGIN", &[])?;
        match f() {
            Ok(()) => self.inner.exec("COMMIT", &[]).map(|_| ()),
            Err(e) => {
                let _ = self.inner.exec("ROLLBACK", &[]);
                Err(e)
            }
        }
    }

    /// The savepoint released, or rolled back to, as the outcome says.
    fn end_savepoint(&self, outcome: Result<(), String>) -> Result<(), String> {
        let end = if outcome.is_ok() { "RELEASE SAVEPOINT crow_transact" } else { "ROLLBACK TO SAVEPOINT crow_transact" };
        match self.in_scope(true, |tx| tx.exec(end, &[])) {
            Some(Err(e)) => Err(e),
            _ => outcome,
        }
    }

    fn key(&self) -> usize {
        Arc::as_ptr(&self.inner) as *const () as usize
    }

    fn take_scope_tx(&self) -> Option<Box<dyn ScopeTx>> {
        let key = self.key();
        SCOPE.with(|s| s.borrow_mut().as_mut().and_then(|txs| txs.iter().position(|(k, _)| *k == key).map(|i| txs.remove(i).1)))
    }

    fn put_scope_tx(&self, tx: Box<dyn ScopeTx>) {
        let key = self.key();
        SCOPE.with(|s| {
            if let Some(txs) = s.borrow_mut().as_mut() {
                txs.push((key, tx));
            }
        });
    }

    pub fn migrate(&self, dir: &str) -> Result<i64, String> {
        self.inner.migrate(dir)
    }

    /// Run `f` under the database's schema lock (CrowDb::schema_lock).
    pub(crate) fn schema_lock(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        self.inner.schema_lock(f)
    }

    pub fn backend_name(&self) -> &str {
        self.inner.backend_name()
    }
}

/// A connection shows its backend (`<db:sqlite>`), as the interpreter
/// shows one, through the one display `log` and `str` use (compiled, `log`
/// of a connection did not build).
impl crow_runtime::display::CrowDisplay for DbHandle {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for DbHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "<db:{}>", self.backend_name())
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
    // a replay of recordings (`crow test --fixtures`) connects to nothing:
    // its reads are what was kept
    if let Some(crow_dir) = fixtures::replaying_for(url) {
        return Ok(DbHandle::new(fixtures::Replay::open(url, &crow_dir)));
    }
    // One handle per server and process, and per sqlite file: parts that
    // each connect to the same Postgres or ClickHouse share its pool, so
    // the program's queries to one server are bounded by one pool, not one
    // per declaration; parts that each connect to one sqlite file share
    // its connection, so a handler writing through both writes in one
    // transaction (two connections to one file locked each other out:
    // finch's roll-over ends a session and starts one in a click).
    // `:memory:` is a new database each time by design.
    static SHARED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, Option<usize>), DbHandle>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // (and each thread keeps the handles it has asked for: a route that
    // connects on every request had taken the one lock every handler's
    // thread waits on, twice a request)
    if let Some(h) = asked_before(url, pool_size) {
        return Ok(h);
    }
    let key = match url.strip_prefix("sqlite://") {
        Some(path) if path.is_empty() || path.contains(":memory:") => None,
        Some(path) => Some(format!("sqlite://{}", std::path::absolute(path).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string()))),
        None => Some(url.to_string()),
    };
    if let Some(k) = &key {
        let found = SHARED.lock().unwrap_or_else(|p| p.into_inner()).get(&(k.clone(), pool_size)).cloned();
        if let Some(h) = found {
            remember_asked(url, pool_size, &h);
            return Ok(h);
        }
    }
    let handle = open_handle(url, pool_size)?;
    if let Some(k) = key {
        SHARED.lock().unwrap_or_else(|p| p.into_inner()).insert((k, pool_size), handle.clone());
    }
    remember_asked(url, pool_size, &handle);
    Ok(handle)
}

thread_local! {
    /// The handles this thread has asked `connect` for (a few at most):
    /// every handle to a server lives as long as the program does.
    static ASKED: std::cell::RefCell<Vec<(String, Option<usize>, DbHandle)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The handle this thread asked for before by this URL and pool size; a
/// sqlite file's is never kept here (its path is resolved each time).
fn asked_before(url: &str, pool_size: Option<usize>) -> Option<DbHandle> {
    if url.starts_with("sqlite://") {
        return None;
    }
    ASKED.with(|a| a.borrow().iter().find(|(u, p, _)| u == url && *p == pool_size).map(|(_, _, h)| h.clone()))
}

/// A handle this thread asked for, kept for its next ask (a few at most).
fn remember_asked(url: &str, pool_size: Option<usize>, handle: &DbHandle) {
    if url.starts_with("sqlite://") {
        return;
    }
    ASKED.with(|a| {
        let mut a = a.borrow_mut();
        if a.len() < 8 && !a.iter().any(|(u, p, _)| u == url && *p == pool_size) {
            a.push((url.to_string(), pool_size, handle.clone()));
        }
    });
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
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") || url.starts_with("clickhouse://") {
        // the drivers written in Crow (docs/db-in-crow-design.md)
        Ok(DbHandle::new(driven::Driven::open(url, pool_size)?))
    } else {
        Err(format!("unknown database URL scheme: {}", url))
    }
}


#[cfg(all(test, feature = "sqlite"))]
mod shared_handle_tests {
    /// Two parts that connect to one sqlite file share its connection, so a
    /// handler writing through both writes in one transaction: two
    /// connections to the file locked each other out ("database is locked",
    /// finch's roll-over ending a session and starting one in a click)
    #[test]
    fn two_connections_to_one_sqlite_file_are_one() {
        let dir = std::env::temp_dir().join(format!("crow_db_shared_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}", dir.join("one.db").display());
        let a = super::connect(&url, None).unwrap();
        let b = super::connect(&url, None).unwrap();
        a.exec("CREATE TABLE t (x INTEGER)", &[]).unwrap();
        crow_runtime::txn::begin();
        a.exec("INSERT INTO t VALUES (1)", &[]).unwrap();
        b.exec("INSERT INTO t VALUES (2)", &[]).unwrap();
        crow_runtime::txn::finish(true).unwrap();
        let rows = a.query("SELECT count(*) AS n FROM t", &[]).unwrap();
        assert_eq!(rows.data[0].len(), 1);
        assert!(format!("{:?}", rows.data[0]).contains('2'), "{:?}", rows.data[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
