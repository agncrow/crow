//! A driver written in Crow, as the handle calls it (docs/db-in-crow-design.md).
//!
//! `Driven` is the one `CrowDb` for the backends whose protocol is a
//! module written in Crow (`src/modules/clickhouse.co`, `postgres.co`).
//! It owns what is mechanism — the URL's parts, the pool of connections
//! the driver opened, the semaphore that bounds how many statements run
//! at once, a handler's transaction scope, the reconnect after a
//! connection the server dropped, the cancel after a stop — and calls the
//! driver for everything that is protocol, through a table of calls the
//! program registers: the compiled program's generated functions, or the
//! interpreter's child over the program's functions.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};

use crow_runtime::frame::{Column, CrowFrame};
use crow_runtime::sys::CrowStream;

use crate::{CrowDb, DbTx, ScopeTx};

/// Why a call failed: the server's or the driver's words, and whether the
/// connection it ran on is gone (the driver says so with an error of kind
/// `sys`): a lost connection is let go, another opened, the call tried
/// once more.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub reason: String,
    pub lost: bool,
}

impl Failure {
    pub fn new(reason: impl Into<String>, lost: bool) -> Self {
        Failure { reason: reason.into(), lost }
    }
}

/// The driver's calls, each the row of the module written in Crow:
/// `parts` the URL's parts as JSON text, `params` the statement's
/// parameters as JSON text (every parameter kind travels so, in both
/// modes), `s` the connection the statement runs on.
pub trait Calls {
    fn open(&self, parts: &str) -> Result<CrowStream, Failure>;
    fn check(&self, parts: &str, s: &CrowStream) -> Result<(), Failure>;
    fn query(&self, parts: &str, s: &CrowStream, sql: &str, params: &str) -> Result<CrowFrame, Failure>;
    fn exec(&self, parts: &str, s: &CrowStream, sql: &str, params: &str) -> Result<i64, Failure>;
    fn insert(&self, parts: &str, s: &CrowStream, table: &str, frame: &CrowFrame) -> Result<i64, Failure>;
    fn describe(&self, parts: &str, s: &CrowStream, sql: &str) -> Result<CrowFrame, Failure>;
    fn cancel(&self, parts: &str, s: &CrowStream) -> Result<(), Failure>;
}

/// The calls a backend's driver answers, as a program registered them:
/// a compiled program's for every thread, an interpreter's for its own
/// thread (a child interpreter is no value another thread can hold), or
/// one made for this thread by the maker the interpreter registered.
enum Holder {
    Global(Arc<dyn Calls + Send + Sync>),
    Local(Rc<dyn Calls>),
}

impl Holder {
    fn with<T>(&self, f: impl FnOnce(&dyn Calls) -> T) -> T {
        match self {
            Holder::Global(c) => f(c.as_ref()),
            Holder::Local(c) => f(c.as_ref()),
        }
    }
}

type Maker = dyn Fn(&str) -> Option<Rc<dyn Calls>> + Send + Sync;

static GLOBAL: Mutex<Vec<(String, Arc<dyn Calls + Send + Sync>)>> = Mutex::new(Vec::new());
static MAKER: Mutex<Option<Arc<Maker>>> = Mutex::new(None);

thread_local! {
    static LOCAL: RefCell<Vec<(String, Rc<dyn Calls>)>> = const { RefCell::new(Vec::new()) };
}

/// A compiled program registers its driver for a backend at its start.
pub fn register(backend: &str, calls: Arc<dyn Calls + Send + Sync>) {
    let mut g = GLOBAL.lock().unwrap_or_else(|p| p.into_inner());
    g.retain(|(b, _)| b != backend);
    g.push((backend.to_string(), calls));
}

/// The interpreter registers its driver for this thread.
pub fn register_local(backend: &str, calls: Rc<dyn Calls>) {
    LOCAL.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(b, _)| b != backend);
        l.push((backend.to_string(), calls));
    });
}

/// The interpreter registers how a thread without a driver makes one: a
/// child interpreter over the program it loaded.
pub fn register_maker(maker: Arc<Maker>) {
    *MAKER.lock().unwrap_or_else(|p| p.into_inner()) = Some(maker);
}

/// Whether a driver for the backend is registered or can be made here.
pub fn available(backend: &str) -> bool {
    calls_for(backend).is_ok()
}

fn calls_for(backend: &str) -> Result<Holder, String> {
    if let Some(c) = LOCAL.with(|l| l.borrow().iter().find(|(b, _)| b == backend).map(|(_, c)| c.clone())) {
        return Ok(Holder::Local(c));
    }
    if let Some(c) = GLOBAL.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|(b, _)| b == backend).map(|(_, c)| c.clone()) {
        return Ok(Holder::Global(c));
    }
    let maker = MAKER.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if let Some(made) = maker.and_then(|m| m(backend)) {
        register_local(backend, made.clone());
        return Ok(Holder::Local(made));
    }
    Err(format!("no {} driver is registered in this process (a program that imports db carries it)", backend))
}

/// A URL's parts: `scheme://[user[:pass]@]host[:port][/database]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub backend: &'static str,
    pub user: String,
    pub pass: String,
    pub host: String,
    pub port: u16,
    pub database: String,
}

impl Url {
    pub fn parse(url: &str) -> Result<Url, String> {
        let (backend, rest, default_port, default_db) = if let Some(r) = url.strip_prefix("clickhouse://") {
            ("clickhouse", r, 8123u16, "default")
        } else if let Some(r) = url.strip_prefix("postgres://").or_else(|| url.strip_prefix("postgresql://")) {
            ("postgres", r, 5432u16, "postgres")
        } else {
            return Err(format!("unknown database URL scheme: {}", url));
        };
        let (userinfo, hostpath) = match rest.rfind('@') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => ("", rest),
        };
        let (user, pass) = match userinfo.split_once(':') {
            Some((u, p)) => (u.to_string(), p.to_string()),
            None => (userinfo.to_string(), String::new()),
        };
        let (hostport, database) = match hostpath.find('/') {
            Some(slash) => (&hostpath[..slash], hostpath[slash + 1..].split('?').next().unwrap_or("")),
            None => (hostpath, ""),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.contains(']') || h.ends_with(']') => (h.trim_matches(|c| c == '[' || c == ']').to_string(), p.parse::<u16>().map_err(|_| format!("the port in {} is no number: {}", url, p))?),
            _ => (hostport.trim_matches(|c| c == '[' || c == ']').to_string(), default_port),
        };
        // the native protocol's port, which the driver no longer speaks:
        // read as the HTTP interface's
        let port = if backend == "clickhouse" && port == 9000 {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| eprintln!("crow db: clickhouse://{}:9000 names the native protocol's port; the driver speaks the HTTP interface, on 8123", host));
            8123
        } else {
            port
        };
        let user = if user.is_empty() && backend == "clickhouse" { "default".to_string() } else { user };
        let database = if database.is_empty() { default_db.to_string() } else { database.to_string() };
        if host.is_empty() {
            return Err(format!("invalid {} URL: {}", backend, url));
        }
        Ok(Url { backend, user, pass, host, port, database })
    }

    /// The parts as the driver takes them.
    pub fn parts(&self) -> String {
        serde_json::json!({ "host": self.host, "port": self.port, "user": self.user, "pass": self.pass, "database": self.database, "tls": false }).to_string()
    }

    pub fn shown(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// How many statements run at once: a slot taken per statement, given
/// back when it ends (ClickHouse runs one query on every core already, so
/// four at once contend rather than help; Postgres's pool is ten).
struct Slots {
    free: Mutex<usize>,
    freed: Condvar,
}

impl Slots {
    fn take(self: &Arc<Self>) -> Slot {
        let mut free = self.free.lock().unwrap_or_else(|p| p.into_inner());
        while *free == 0 {
            free = self.freed.wait(free).unwrap_or_else(|p| p.into_inner());
        }
        *free -= 1;
        Slot(self.clone())
    }
}

struct Slot(Arc<Slots>);

impl Drop for Slot {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.0.freed.notify_one();
    }
}

/// The idle connections of a handle, each opened by the driver.
type Idle = Arc<Mutex<Vec<CrowStream>>>;

/// The advisory lock reconciling a schema holds on Postgres: "crowSCHA".
const SCHEMA_LOCK: i64 = 0x6372_6f77_5343_4841;

/// A database whose driver is written in Crow.
pub struct Driven {
    url: Url,
    parts: String,
    idle: Idle,
    slots: Arc<Slots>,
}

impl Driven {
    /// The handle opened: the first connection now, so a wrong address
    /// fails here, and the driver's check of the server (ClickHouse: a
    /// ping, and the statements a dead run left running swept).
    pub fn open(url: &str, pool: Option<usize>) -> Result<Driven, String> {
        let url = Url::parse(url)?;
        let max = pool.filter(|n| *n > 0).unwrap_or(if url.backend == "clickhouse" { 4 } else { 10 });
        let parts = url.parts();
        let calls = calls_for(url.backend)?;
        let first = calls.with(|c| c.open(&parts).and_then(|s| c.check(&parts, &s).map(|_| s))).map_err(|f| f.reason)?;
        Ok(Driven {
            url,
            parts,
            idle: Arc::new(Mutex::new(vec![first])),
            slots: Arc::new(Slots { free: Mutex::new(max), freed: Condvar::new() }),
        })
    }

    fn take_stream(&self, calls: &Holder) -> Result<CrowStream, String> {
        loop {
            let idle = self.idle.lock().unwrap_or_else(|p| p.into_inner()).pop();
            match idle {
                // (the driver noted until when the server keeps the
                // connection: one the server is about to close is let go
                // here, not found closed under the next statement)
                Some(s) if expired(&s) => crow_runtime::sys::close(&s),
                Some(s) => return Ok(s),
                None => return calls.with(|c| c.open(&self.parts)).map_err(|f| unanswered(self.url.backend, f.reason)),
            }
        }
    }

    fn give_back(&self, s: CrowStream) {
        self.idle.lock().unwrap_or_else(|p| p.into_inner()).push(s);
    }

    /// Run a statement on a connection of the pool: an idle one, or one
    /// the driver opens. A connection the server dropped under the call
    /// is let go, another opened, the call made once more; a call a stop
    /// abandoned is cancelled on the server and its connection let go.
    fn with_stream<T>(&self, op: impl Fn(&dyn Calls, &CrowStream) -> Result<T, Failure>) -> Result<T, String> {
        // (a stop waits for this: the statement the server runs is stopped
        // there before the program is gone)
        let _flight = crow_runtime::cancel::in_flight_guard();
        let _slot = self.slots.take();
        let calls = calls_for(self.url.backend)?;
        let s = self.take_stream(&calls)?;
        match calls.with(|c| op(c, &s)) {
            Ok(v) => {
                self.give_back(s);
                Ok(v)
            }
            Err(f) if f.reason.starts_with("cancelled") || f.reason.contains(": cancelled") => {
                let _ = calls.with(|c| c.cancel(&self.parts, &s));
                crow_runtime::sys::close(&s);
                Err("cancelled: nobody waits for this query any more".to_string())
            }
            Err(f) if f.lost => {
                eprintln!("crow db: the {} connection to {} was lost ({}); reconnecting", self.url.backend, self.url.shown(), f.reason);
                crow_runtime::sys::close(&s);
                let fresh = calls.with(|c| c.open(&self.parts)).map_err(|f| unanswered(self.url.backend, f.reason))?;
                match calls.with(|c| op(c, &fresh)) {
                    Ok(v) => {
                        self.give_back(fresh);
                        Ok(v)
                    }
                    Err(again) => {
                        if !again.lost {
                            self.give_back(fresh);
                        }
                        Err(again.reason)
                    }
                }
            }
            Err(f) => {
                self.give_back(s);
                Err(f.reason)
            }
        }
    }

    fn params_json(params: &[serde_json::Value]) -> String {
        serde_json::Value::Array(params.to_vec()).to_string()
    }
}

/// Whether an idle connection's note says the server's keep-alive has
/// run out (`until <epoch seconds>`, as the ClickHouse driver notes it).
fn expired(s: &CrowStream) -> bool {
    let note = crow_runtime::sys::noted(s);
    let Some(until) = note.strip_prefix("until ").and_then(|t| t.trim().parse::<f64>().ok()) else { return false };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    now >= until
}

/// A connection the driver could not open after the first: the database
/// does not answer (it is down, restarting, or out of reach), and why —
/// what a program's error value and a stopped handler's log say of an
/// outage, in the words tests/lost_connection.rs reads.
fn unanswered(backend: &str, reason: String) -> String {
    let why = reason.strip_prefix(&format!("{} connection failed: ", backend)).map(str::to_string).unwrap_or(reason);
    format!("the database does not answer: {}", why)
}

/// A ClickHouse type name as the kinds the compiler knows (the catalog's
/// own names: `Nullable(…)` and `LowCardinality(…)` read through).
pub fn ch_kind(t: &str) -> &'static str {
    let mut t = t.trim();
    loop {
        let inner = t.strip_prefix("Nullable(").or_else(|| t.strip_prefix("LowCardinality("));
        match inner.and_then(|x| x.strip_suffix(')')) {
            Some(x) => t = x,
            None => break,
        }
    }
    if t.starts_with("Int") || t.starts_with("UInt") {
        "int"
    } else if t.starts_with("Float") || t.starts_with("Decimal") {
        "float"
    } else if t == "Bool" {
        "bool"
    } else if t.starts_with("Date") {
        "datetime"
    } else {
        "str"
    }
}

/// A frame of `name` and `kind` columns as the pairs the compiler reads.
pub fn columns_of(frame: &CrowFrame) -> Result<Vec<(String, String)>, String> {
    let text = |name: &str| -> Option<Vec<String>> {
        frame.columns.iter().position(|c| c == name).and_then(|i| match frame.data[i].as_ref() {
            Column::Str(v) => Some(v.clone()),
            _ => None,
        })
    };
    match (text("name"), text("kind")) {
        (Some(n), Some(k)) if n.len() == k.len() => Ok(n.into_iter().zip(k).collect()),
        _ => Err("describe: the driver answered no name and kind columns".to_string()),
    }
}

/// The pairs the compiler reads as a frame of `name` and `kind`, for
/// `describe[conn, sql]`.
pub fn describe_frame(cols: &[(String, String)]) -> CrowFrame {
    let n = cols.len();
    CrowFrame::new(
        vec!["name".to_string(), "kind".to_string()],
        vec![Column::Str(cols.iter().map(|(a, _)| a.clone()).collect()), Column::Str(cols.iter().map(|(_, b)| b.clone()).collect())],
        vec![vec![true; n], vec![true; n]],
    )
}

impl CrowDb for Driven {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        if crow_runtime::cancel::current_stopped() {
            return Err("cancelled: nobody waits for this query any more".to_string());
        }
        let p = Self::params_json(params);
        self.with_stream(|c, s| c.query(&self.parts, s, sql, &p))
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let p = Self::params_json(params);
        self.with_stream(|c, s| c.exec(&self.parts, s, sql, &p))
    }

    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String> {
        if frame.nrows() == 0 {
            return Ok(0);
        }
        self.with_stream(|c, s| c.insert(&self.parts, s, table, frame))
    }

    fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        if self.url.backend == "clickhouse" {
            return Err("ClickHouse does not support transactions (it is an OLAP engine)".into());
        }
        let _slot = self.slots.take();
        let calls = calls_for(self.url.backend)?;
        let s = self.take_stream(&calls)?;
        calls.with(|c| c.exec(&self.parts, &s, "BEGIN", "[]")).map_err(|f| format!("postgres begin error: {}", f.reason))?;
        let tx = DrivenTx { parts: &self.parts, calls: &calls, stream: &s };
        let outcome = f(&tx);
        let end = if outcome.is_ok() { "COMMIT" } else { "ROLLBACK" };
        let ended = calls.with(|c| c.exec(&self.parts, &s, end, "[]")).map(|_| ()).map_err(|f| format!("postgres {} error: {}", end.to_lowercase(), f.reason));
        self.give_back(s);
        match (outcome, ended) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(e)) => Err(e),
            (Err(e), _) => Err(e),
        }
    }

    fn migrate(&self, dir: &str) -> Result<i64, String> {
        crate::migrate::run_migrations(self, dir)
    }

    fn backend_name(&self) -> &str {
        self.url.backend
    }

    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String> {
        let frame = self.with_stream(|c, s| c.describe(&self.parts, s, sql))?;
        columns_of(&frame)
    }

    /// Postgres: one transaction holding an advisory lock, released as it
    /// ends. ClickHouse has neither lock nor transaction: as it is.
    fn schema_lock(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String> {
        if self.url.backend != "postgres" {
            return f(&crate::Unlocked(self));
        }
        self.transact(&|tx| {
            tx.exec(&format!("SELECT pg_advisory_xact_lock({})", SCHEMA_LOCK), &[])?;
            f(tx)
        })
    }

    /// Postgres: a handler's transaction on a connection of its own, held
    /// for the handler's length. ClickHouse: none.
    fn begin_scope(&self) -> Option<Result<Box<dyn ScopeTx>, String>> {
        if self.url.backend != "postgres" {
            return None;
        }
        let begun = (|| {
            let calls = calls_for(self.url.backend)?;
            let s = self.take_stream(&calls)?;
            if let Err(f) = calls.with(|c| c.exec(&self.parts, &s, "BEGIN", "[]")) {
                return Err(format!("postgres begin: {}", f.reason));
            }
            Ok(Box::new(DrivenScope { backend: self.url.backend, parts: self.parts.clone(), idle: self.idle.clone(), stream: Some(s), slot: Some(self.slots.take()) }) as Box<dyn ScopeTx>)
        })();
        Some(begun)
    }
}

/// `transact`'s statements, on the one connection the transaction holds.
struct DrivenTx<'a> {
    parts: &'a str,
    calls: &'a Holder,
    stream: &'a CrowStream,
}

impl DbTx for DrivenTx<'_> {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let p = Driven::params_json(params);
        self.calls.with(|c| c.query(self.parts, self.stream, sql, &p)).map_err(|f| f.reason)
    }

    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let p = Driven::params_json(params);
        self.calls.with(|c| c.exec(self.parts, self.stream, sql, &p)).map_err(|f| f.reason)
    }
}

/// A handler's transaction: one connection, begun, held until the
/// handler ends, given back to the pool as it commits or rolls back.
struct DrivenScope {
    backend: &'static str,
    parts: String,
    idle: Idle,
    stream: Option<CrowStream>,
    slot: Option<Slot>,
}

impl ScopeTx for DrivenScope {
    fn query(&mut self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let stream = self.stream.as_ref().ok_or("the transaction has ended")?;
        let p = Driven::params_json(params);
        calls_for(self.backend)?.with(|c| c.query(&self.parts, stream, sql, &p)).map_err(|f| f.reason)
    }

    fn exec(&mut self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        let stream = self.stream.as_ref().ok_or("the transaction has ended")?;
        let p = Driven::params_json(params);
        calls_for(self.backend)?.with(|c| c.exec(&self.parts, stream, sql, &p)).map_err(|f| f.reason)
    }

    fn finish(mut self: Box<Self>, commit: bool) -> Result<(), String> {
        let stream = self.stream.take().ok_or("the transaction has ended")?;
        let end = if commit { "COMMIT" } else { "ROLLBACK" };
        let r = calls_for(self.backend)?.with(|c| c.exec(&self.parts, &stream, end, "[]"));
        let _ = self.slot.take();
        match r {
            Ok(_) => {
                self.idle.lock().unwrap_or_else(|p| p.into_inner()).push(stream);
                Ok(())
            }
            Err(f) => {
                if !f.lost {
                    self.idle.lock().unwrap_or_else(|p| p.into_inner()).push(stream);
                }
                Err(format!("postgres {}: {}", end.to_lowercase(), f.reason))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_read_into_its_parts() {
        let u = Url::parse("clickhouse://user:pw@db.example.com:8443/finch").unwrap();
        assert_eq!((u.backend, u.user.as_str(), u.pass.as_str(), u.host.as_str(), u.port, u.database.as_str()), ("clickhouse", "user", "pw", "db.example.com", 8443, "finch"));
        let u = Url::parse("clickhouse://localhost/finch_trading").unwrap();
        assert_eq!((u.user.as_str(), u.port, u.database.as_str()), ("default", 8123, "finch_trading"));
        // the native port is read as the HTTP interface's
        assert_eq!(Url::parse("clickhouse://localhost:9000/finch_trading").unwrap().port, 8123);
        let u = Url::parse("postgresql://vijay@127.0.0.1:5433/postgres").unwrap();
        assert_eq!((u.backend, u.user.as_str(), u.port, u.database.as_str()), ("postgres", "vijay", 5433, "postgres"));
        assert_eq!(Url::parse("postgres://h").unwrap().database, "postgres");
        assert!(Url::parse("mysql://h/d").unwrap_err().starts_with("unknown database URL scheme"));
        assert!(Url::parse("clickhouse://:x/d").is_err());
        let parts: serde_json::Value = serde_json::from_str(&u.parts()).unwrap();
        assert_eq!(parts["host"], "127.0.0.1");
        assert_eq!(parts["port"], 5433);
        assert_eq!(parts["tls"], false);
    }

    /// A driver that answers from Rust, for the handle's own logic: a
    /// connection the first call loses is let go and the call made again
    /// on a fresh one; a call a stop abandoned is cancelled; the server's
    /// own refusal keeps its connection
    struct Fake {
        opened: Mutex<usize>,
        lose_first: bool,
        cancelled: Mutex<usize>,
    }

    impl Calls for Fake {
        fn open(&self, _parts: &str) -> Result<CrowStream, Failure> {
            *self.opened.lock().unwrap() += 1;
            Ok(CrowStream::closed())
        }
        fn check(&self, _parts: &str, _s: &CrowStream) -> Result<(), Failure> {
            Ok(())
        }
        fn query(&self, _parts: &str, _s: &CrowStream, sql: &str, params: &str) -> Result<CrowFrame, Failure> {
            if sql == "lost" && self.lose_first && *self.opened.lock().unwrap() == 1 {
                return Err(Failure::new("broken pipe", true));
            }
            if sql == "refused" {
                return Err(Failure::new("Code: 60. no such table", false));
            }
            if sql == "slow" {
                return Err(Failure::new("cancelled: nobody waits for this query any more", false));
            }
            Ok(describe_frame(&[("sql".into(), sql.into()), ("params".into(), params.into())]))
        }
        fn exec(&self, _parts: &str, _s: &CrowStream, _sql: &str, _params: &str) -> Result<i64, Failure> {
            Ok(0)
        }
        fn insert(&self, _parts: &str, _s: &CrowStream, _table: &str, frame: &CrowFrame) -> Result<i64, Failure> {
            Ok(frame.nrows() as i64)
        }
        fn describe(&self, _parts: &str, _s: &CrowStream, _sql: &str) -> Result<CrowFrame, Failure> {
            Ok(describe_frame(&[("n".into(), "int".into()), ("x".into(), "float?".into())]))
        }
        fn cancel(&self, _parts: &str, _s: &CrowStream) -> Result<(), Failure> {
            *self.cancelled.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[test]
    fn the_handle_reconnects_cancels_and_keeps_the_servers_words() {
        let fake = Arc::new(Fake { opened: Mutex::new(0), lose_first: true, cancelled: Mutex::new(0) });
        register("clickhouse", fake.clone());
        let db = Driven::open("clickhouse://localhost/d", None).unwrap();
        assert_eq!(*fake.opened.lock().unwrap(), 1);
        // the first connection is lost under the call: another opened, the call made again
        let f = db.query("lost", &[serde_json::json!(1)]).unwrap();
        assert_eq!(columns_of(&f).unwrap()[1].1, "[1]");
        assert_eq!(*fake.opened.lock().unwrap(), 2);
        // the server's refusal is its words, and the connection stays
        assert_eq!(db.query("refused", &[]).unwrap_err(), "Code: 60. no such table");
        assert_eq!(*fake.opened.lock().unwrap(), 2);
        // a stop: cancelled on the server, the connection let go
        assert!(db.query("slow", &[]).unwrap_err().starts_with("cancelled"));
        assert_eq!(*fake.cancelled.lock().unwrap(), 1);
        assert_eq!(db.describe("x").unwrap(), vec![("n".to_string(), "int".to_string()), ("x".to_string(), "float?".to_string())]);
        assert!(db.transact(&|_| Ok(())).unwrap_err().contains("OLAP"));
        assert_eq!(db.backend_name(), "clickhouse");
    }
}
