# Database Module (`use db`)

Top-level database module for Crow. Provides unified connectivity to SQLite, PostgreSQL, and ClickHouse with connection pooling, bulk operations, transactions, and migrations.

## Architecture

The `db` module lives in the `crow_db` crate — separate from `crow_web`. Databases are a general-purpose concern (scripts, pipelines, CLI tools), not a web-specific feature.

### Crate: `crow_db/`

Feature-gated backends via Cargo feature flags:

| Feature | Crate | Protocol |
|---|---|---|
| `sqlite` | `rusqlite` | File-based |
| `postgres` | `tokio-postgres` + `deadpool-postgres` | TCP, COPY protocol |
| `clickhouse` | `klickhouse` | Native RowBinary |

`use db` in Crow source triggers `needs_crow_db` in the transpiler, adding the crate to the generated `Cargo.toml`.

## API

### `connect[url]` / `connect[url, pool_size]`

Opens a database connection pool. URL scheme determines the backend.

| URL Scheme | Backend | Default Pool |
|---|---|---|
| `sqlite://path` | SQLite | 1 |
| `postgres://user:pass@host/db` | PostgreSQL | 10 |
| `http://host:port/db` | ClickHouse | 10 |

Returns a connection handle. The handle is internally a pool — users never manage individual connections.

### `query[conn, sql]`

Executes a SELECT statement and returns results as a **frame**. Same data type as `load['file.csv']` — all frame operations (`filter`, `sort`, `group`, `join`, etc.) work on query results.

### `exec[conn, sql]` / `exec[conn, sql, params]`

Executes DDL/DML (CREATE, INSERT, UPDATE, DELETE). Returns affected row count as an integer. Optional third argument is an array of bind values for parameterized queries.

### `insert[conn, table, frame]`

Bulk inserts a frame into a table. Uses backend-specific fast paths:

- **PostgreSQL:** COPY protocol (~5-10s for 1M rows)
- **ClickHouse:** Native RowBinary (~2-3s for 1M rows)
- **SQLite:** Batched prepared statements in a transaction (~10-15s for 1M rows)

Frame only — no arrays of maps. One data format for DB operations (Rule 1).

### `transact[conn, fn]`

Executes a named function inside a transaction:

1. Checks out one connection from the pool
2. Begins a transaction
3. Calls `fn[conn]` with the connection handle
4. Commits on success, rolls back on error
5. Returns the connection to the pool

No `begin`/`commit`/`rollback` builtins — one way to do transactions (Rule 1).

### `migrate[conn, dir]`

Runs forward-only SQL migrations:

1. Reads `.sql` files from `dir`, sorted by filename
2. Creates `_crow_migrations` table if it doesn't exist
3. Skips already-applied migrations
4. Executes new migrations in order
5. Returns count of newly applied migrations

No rollback files — rollbacks are a new migration that undoes the change.

## Connection Pooling

The pool is transparent to users. Crow manages it automatically:

| Context | Behavior |
|---|---|
| Individual statement | Check out one connection, execute, return |
| `while` loop | Sequential — one connection per statement |
| `for` loop (parallel) | Each parallel iteration gets its own connection |
| `transact[conn, fn]` | One connection held for the entire function |
| `insert[conn, table, frame]` | One connection, one bulk operation |

## Conflict Resolution

`connect` is exported by both `db` and `web.ws`. When both modules are imported:

```
use db
use web

--- ERROR: ambiguous — use db.connect or web.connect ---
conn = connect['postgres://...']

--- Correct ---
db_conn = db.connect['postgres://user:pass@host/mydb']
ws_conn = web.connect['ws://localhost/ws']
```

This is a language-level rule, not db-specific. See `docs/language-spec.md` § Conflict Resolution.

## Trait Design

```rust
pub trait CrowDb: Send + Sync {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String>;
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String>;
    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String>;
    fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String>;
    fn migrate(&self, dir: &str) -> Result<i64, String>;
    fn backend_name(&self) -> &str;
}
```

SQL parameters are passed as `serde_json::Value` arrays — the Crow interpreter converts Crow values (int, float, str, bool) to JSON for binding. Each backend implements this trait. `connect[url]` dispatches to the correct backend based on URL scheme and returns a `DbHandle` (thread-safe `Arc<dyn CrowDb>` wrapper).

## Implementation Status

| Backend | Status | Notes |
|---|---|---|
| SQLite | ✅ Done | Full support: query, exec, insert, transact, migrate |
| PostgreSQL | ✅ Done | `tokio-postgres` + `deadpool-postgres`. Connection pooling, parameterized queries ($1/$2), transactions, bulk insert, migrations. Tested against PostgreSQL on port 5432. |
| ClickHouse | ✅ Done | `klickhouse` native TCP protocol (port 9000). Streaming block-based queries, columnar-to-frame conversion, bulk insert. Transactions correctly refused (OLAP engine). Parameterized queries not supported (use string interpolation). Tested against ClickHouse on port 9000. |

### Interpreter

All 6 builtins work in `crow run -i` mode. Transactions use manual `BEGIN`/`COMMIT`/`ROLLBACK` (avoids Rust closure lifetime issues with Crow function callbacks). The `db` module is registered via `ModuleRegistry` and takes priority over core builtins.

### Transpiler

`use db` sets `needs_crow_db = true` and adds `crow_db` with feature flags to the generated `Cargo.toml`. Codegen emits direct calls to `crow_db::connect()`, `handle.query()`, etc. Both bare (`connect[url]`) and prefixed (`db.connect[url]`) syntax are supported.

### Tests

- 9 unit tests in `crow_db` crate (SQLite backend)
- 8 interpreter tests (connect, exec, query, params, transact, bad URL)
- 8 transpiler codegen tests
- Integration example: `examples/db_demo.crow`

## Example

```
use db

conn = connect['sqlite://:memory:']

--- Create and populate ---
exec[conn, 'CREATE TABLE users (id INTEGER, name TEXT, age INTEGER)']
exec[conn, 'INSERT INTO users VALUES (1, ?, ?)', ['alice', 30]]
exec[conn, 'INSERT INTO users VALUES (2, ?, ?)', ['bob', 25]]

--- Query returns a frame ---
users = query[conn, 'SELECT * FROM users']
log[len[users]]

--- Parameterized query ---
young = query[conn, 'SELECT name FROM users WHERE age < ?', [30]]

--- Bulk insert ---
new_users = load['new_users.csv']
insert[conn, 'users', new_users]

--- Transaction ---
fn transfer[t]
  exec[t, 'UPDATE accounts SET balance = balance - 100 WHERE id = ?', [1]]
  exec[t, 'UPDATE accounts SET balance = balance + 100 WHERE id = ?', [2]]

transact[conn, transfer]

--- Migrations ---
migrate[conn, 'migrations/']

--- Module-prefixed (when web.ws is also imported) ---
db_conn = db.connect['sqlite://data.db']
db.exec[db_conn, 'SELECT 1']
```
