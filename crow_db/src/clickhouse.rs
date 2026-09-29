//! ClickHouse backend for crow_db.
//!
//! Uses `klickhouse` for native TCP protocol (port 9000).
//! ClickHouse is an OLAP engine — transactions are NOT supported.
//! URL scheme: `clickhouse://user:pass@host:port/database`

use std::sync::Arc;
use crow_runtime::frame::{CrowFrame, Column};
use crate::{CrowDb, DbTx, stop_requested};
use tokio::runtime::Runtime;
use klickhouse::{Client, ClientOptions, Value, RawRow};
use tokio_stream::StreamExt;

/// How many connections a handle keeps at most. ClickHouse already runs one
/// query on every core of the server, so more at once add little and, on a
/// table of many parts, contend: measured on finch's 143,575-part table, two
/// warm queries at once each took under a second, four cold ones 35 s.
const POOL: usize = 4;

/// ClickHouse database — a pool of klickhouse connections + a tokio runtime.
pub struct CrowClickhouseDb {
    /// The connections, opened as queries need them. One connection runs
    /// one query at a time, so queries from different sessions each take
    /// their own. A connection is not the program's lifetime: the server
    /// closes an idle one (an hour by default) and a restart drops every
    /// one, so each is reopened when found closed
    pool: Vec<std::sync::Mutex<Option<Client>>>,
    /// operations running on each connection now
    busy: Vec<std::sync::atomic::AtomicUsize>,
    rt: Arc<Runtime>,
    addr: String,
    options: ClientOptions,
}

/// This machine's name, as a query tag carries it.
fn host_name() -> &'static str {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let raw = std::process::Command::new("hostname")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
        if clean.is_empty() { "local".to_string() } else { clean }
    })
}

/// Every statement this process sends carries a tag — the machine, the
/// process, a number — so it can be stopped on the server: when the work
/// that asked for it is cancelled, when the program stops, and, for a server
/// on this machine, when a later run finds it left behind.
fn tag() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    format!("crow {} {} {}", host_name(), std::process::id(), n)
}

/// Statements of this process running on a ClickHouse server now: a
/// program that stops with none running asks the server to stop nothing
/// (each ask is a connection and a statement, and a busy server does better
/// without them).
static IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A statement in flight, for as long as it is held.
struct InFlight;

impl InFlight {
    fn start() -> Self {
        IN_FLIGHT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        InFlight
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The LIKE pattern for every statement of process `pid` on this machine.
fn process_pattern(pid: u32) -> String {
    format!("/* crow {} {} %", host_name(), pid)
}

/// Stop the statements whose text matches `pattern`, on a connection of its
/// own — the ones running them are busy.
async fn kill_matching(addr: &str, options: &ClientOptions, pattern: &str) {
    let sql = format!("KILL QUERY WHERE query LIKE '{}' ASYNC", pattern.replace('\'', ""));
    if let Ok(client) = Client::connect(addr, options.clone()).await {
        let _ = client.execute(sql.as_str()).await;
    }
}

/// Whether a server address is this machine.
fn is_local(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr).trim_matches(|c| c == '[' || c == ']');
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Whether a process on this machine is alive.
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true)
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
            // a panic on the worker goes on as itself, on this thread
            std::thread::scope(|sc| sc.spawn(f).join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
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

        // the first connection is opened now, so a wrong address fails here
        let client = Self::open(&rt, &addr, options.clone())?;
        let mut pool: Vec<std::sync::Mutex<Option<Client>>> = (0..POOL).map(|_| std::sync::Mutex::new(None)).collect();
        pool[0] = std::sync::Mutex::new(Some(client.clone()));
        let busy = (0..POOL).map(|_| std::sync::atomic::AtomicUsize::new(0)).collect();
        let db = Self { pool, busy, rt: Arc::new(rt), addr, options };
        db.sweep_left_behind(&client);
        // when the program stops, what it started stops with it
        let (addr, options) = (db.addr.clone(), db.options.clone());
        crow_runtime::cancel::on_shutdown(Box::new(move || {
            if IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                return;
            }
            let (addr, options) = (addr.clone(), options.clone());
            let _ = std::thread::spawn(move || {
                if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    rt.block_on(kill_matching(&addr, &options, &process_pattern(std::process::id())));
                }
            })
            .join();
        }));
        Ok(db)
    }

    /// Statements an earlier run of a program on this machine left running —
    /// its process is gone, the server is still at them — are stopped when a
    /// new run connects: a rebuilt app does not inherit its predecessor's
    /// queries. Only for a server on this machine, where a process number
    /// means something.
    fn sweep_left_behind(&self, client: &Client) {
        if !is_local(&self.addr) {
            return;
        }
        let me = std::process::id();
        let sql = format!(
            "SELECT DISTINCT toUInt32OrZero(extract(query, '^/[*] crow {} ([0-9]+) ')) AS pid FROM system.processes WHERE query LIKE '/* crow {} %'",
            host_name(), host_name()
        );
        let pids: Vec<u32> = Self::off_runtime(|| self.rt.block_on(async {
            let mut out = Vec::new();
            if let Ok(mut stream) = client.query_raw(sql.as_str()).await {
                while let Some(Ok(block)) = stream.next().await {
                    for (_, values) in block.column_data.into_iter() {
                        for v in values {
                            if let Some(n) = value_to_i64(&v) {
                                out.push(n as u32);
                            }
                        }
                    }
                }
            }
            out
        }));
        for pid in pids {
            if pid != 0 && pid != me && !process_alive(pid) {
                eprintln!("crow db: stopping queries a finished run (process {}) left running on {}", pid, self.addr);
                Self::off_runtime(|| self.rt.block_on(kill_matching(&self.addr, &self.options, &process_pattern(pid))));
            }
        }
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

    /// The connection an operation takes: an open one that is idle, else a
    /// new one while the pool has room, else the least busy.
    fn checkout(&self) -> usize {
        use std::sync::atomic::Ordering::SeqCst;
        let opened: Vec<bool> = self.pool.iter().map(|m| m.lock().map(|g| g.is_some()).unwrap_or(false)).collect();
        let busy: Vec<usize> = self.busy.iter().map(|b| b.load(SeqCst)).collect();
        let pick = (0..POOL)
            .find(|&i| opened[i] && busy[i] == 0)
            .or_else(|| (0..POOL).find(|&i| !opened[i]))
            .unwrap_or_else(|| (0..POOL).min_by_key(|&i| busy[i]).unwrap_or(0));
        self.busy[pick].fetch_add(1, SeqCst);
        pick
    }

    /// Connection `i` — opened now if it never was, reopened first when the
    /// server has closed it.
    fn client(&self, i: usize) -> Result<Client, String> {
        let mut guard = self.pool[i].lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(c) if !c.is_closed() => return Ok(c.clone()),
            Some(_) => eprintln!("crow db: the clickhouse connection to {} was closed; reconnecting", self.addr),
            None => {}
        }
        let fresh = Self::open(&self.rt, &self.addr, self.options.clone())?;
        *guard = Some(fresh.clone());
        Ok(fresh)
    }

    /// Run an operation on a connection of the pool; if the server dropped
    /// it under the operation, reconnect and run it once more.
    fn with_client<T>(&self, op: impl Fn(Client) -> Result<T, String>) -> Result<T, String> {
        struct Release<'a>(&'a std::sync::atomic::AtomicUsize);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let i = self.checkout();
        let _release = Release(&self.busy[i]);
        let _running = InFlight::start();
        match op(self.client(i)?) {
            Err(e) if is_transport_error(&e) => {
                eprintln!("crow db: the clickhouse connection to {} was lost ({}); reconnecting", self.addr, e);
                let fresh = Self::open(&self.rt, &self.addr, self.options.clone())?;
                *self.pool[i].lock().map_err(|e| e.to_string())? = Some(fresh.clone());
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

/// Whether a ClickHouse type can hold NULL: `Nullable(…)`, alone or
/// inside `LowCardinality(…)`. `Nullable(Nothing)` is a value made only
/// of a parameter, which describing stands NULL in for: its type is not
/// known, not able to be missing.
fn ch_nullable(t: &str) -> bool {
    let t = t.trim();
    let t = t.strip_prefix("LowCardinality(").and_then(|x| x.strip_suffix(')')).unwrap_or(t);
    t.starts_with("Nullable(") && t != "Nullable(Nothing)"
}

/// A ClickHouse type name to the kinds the compiler knows.
pub(crate) fn ch_kind(t: &str) -> &'static str {
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
        // a column that can hold NULL is marked `?`: its cells are fallible
        Ok(names
            .into_iter()
            .zip(types)
            .map(|(n, t)| (n, format!("{}{}", ch_kind(&t), if ch_nullable(&t) { "?" } else { "" })))
            .collect())
    }

    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        let sql_owned;
        let sql: &str = if params.is_empty() {
            sql
        } else {
            sql_owned = substitute_params(sql, params)?;
            &sql_owned
        };

        let flag = crow_runtime::cancel::current();
        if crow_runtime::cancel::stopped(&flag) {
            return Err("cancelled: nobody waits for this query any more".to_string());
        }
        let tag = tag();
        let tagged = format!("/* {} */ {}", tag, sql);
        self.with_client(|client| Self::off_runtime(|| self.rt.block_on(async {
            let run = async {
            let mut stream = client.query_raw(tagged.as_str()).await
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
            };
            tokio::select! {
                result = run => result,
                _ = stop_requested(flag.clone()) => {
                    kill_matching(&self.addr, &self.options, &format!("%/* {} */%", tag)).await;
                    Err("cancelled: nobody waits for this query any more".to_string())
                }
            }
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

        let flag = crow_runtime::cancel::current();
        let tag = tag();
        let tagged = format!("/* {} */ {}", tag, sql);
        self.with_client(|client| Self::off_runtime(|| self.rt.block_on(async {
            let run = async {
                client.execute(tagged.as_str()).await
                    .map_err(|e| format!("clickhouse exec error: {}", trim_server_error(&e.to_string())))?;
                // ClickHouse native protocol doesn't return affected row counts
                Ok(0)
            };
            tokio::select! {
                result = run => result,
                _ = stop_requested(flag.clone()) => {
                    kill_matching(&self.addr, &self.options, &format!("%/* {} */%", tag)).await;
                    Err("cancelled: the program is stopping".to_string())
                }
            }
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
    use super::{ch_kind, ch_nullable, substitute_params, is_transport_error, InFlight, IN_FLIGHT};

    /// A statement counts as in flight while it runs, and a program that
    /// stops with none in flight sends the server nothing
    #[test]
    fn a_statement_is_in_flight_while_it_runs() {
        let before = IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst);
        {
            let _a = InFlight::start();
            let _b = InFlight::start();
            assert!(IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst) >= before + 2);
        }
        assert_eq!(IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst), before);
    }

    /// DESCRIBE's types: a `Nullable` one, alone or inside
    /// `LowCardinality`, can hold NULL; an array of nullable values is a
    /// value
    #[test]
    fn a_nullable_type_can_hold_null() {
        assert!(ch_nullable("Nullable(Float64)"));
        assert!(ch_nullable("LowCardinality(Nullable(String))"));
        assert!(!ch_nullable("Float64"));
        assert!(!ch_nullable("LowCardinality(String)"));
        assert!(!ch_nullable("Array(Nullable(Float64))"));
        // a value made only of a parameter, NULL while describing
        assert!(!ch_nullable("Nullable(Nothing)"));
        assert_eq!(ch_kind("LowCardinality(Nullable(String))"), "str");
        assert_eq!(ch_kind("Nullable(UInt64)"), "int");
    }

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
