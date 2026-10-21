# Database Module (`use db`)

Top-level database module for Crow. Provides unified connectivity to SQLite, PostgreSQL, and ClickHouse with connection pooling, bulk operations, transactions, and the migrations `crow db apply` runs.

## Architecture

The `db` module lives in the `crow_db` crate — separate from `crow_web`. Databases are a general-purpose concern (scripts, pipelines, CLI tools), not a web-specific feature.

### Crate: `crow_db/`

SQLite is the crate's one Cargo feature. The Postgres and ClickHouse drivers are written in Crow: internal modules (`db.postgres`, `db.clickhouse`) that `use db` carries for the connection URLs a program names, every driver where a URL is not a literal. `crow_db::driven` holds their handle — the pool, a handler's transaction, the reconnect and the cancel — and calls the driver through a table of calls the program registers.

| Backend | Where | Protocol |
|---|---|---|
| SQLite | Cargo feature `sqlite` (`rusqlite`, bundled) | File-based |
| PostgreSQL | written in Crow (`src/modules/postgres.co`, over `sys`) | the wire protocol, COPY |
| ClickHouse | written in Crow (`src/modules/clickhouse.co`, over `sys`) | HTTP interface, RowBinary |

`use db` in Crow source triggers `needs_crow_db` in the transpiler (from the module's entry in `src/modules/mod.rs`), adding the crate with its `sqlite` feature to the generated `Cargo.toml`.

## API

### `connect[url]` / `connect[url, pool_size]`

Opens a database connection pool. URL scheme determines the backend.

| URL Scheme | Backend | Default Pool |
|---|---|---|
| `sqlite://path` | SQLite | 1 |
| `postgres://user:pass@host/db` (or `postgresql://`) | PostgreSQL | 10 |
| `clickhouse://user:pass@host:port/db` | ClickHouse, over its HTTP interface (port 8123 when none is given) | 4 |

Returns a connection handle. The handle is internally a pool — users never manage individual connections.

Both modes open every connection through one function, `crow_db::schema::connect(url, pool, declared)`: the lowering (`src/lower_ir/schema.rs`) finds the literal `CREATE TABLE` statements the program hands the connection — directly, or through the functions it is handed to, less the tables the program also drops there — and passes them as `declared`. Opening the connection checks the tables the database has against them (`crow_db/src/schema.rs`): the statements are parsed there (no SQL library), each table's columns read from the catalog (`pragma_table_xinfo`, `information_schema.columns`, `system.columns`), and a column compared by the kind Crow reads its cells as. `reconcile` adds what a table lacks inside `CrowDb::schema_lock` (SQLite: `BEGIN IMMEDIATE`; Postgres: a transaction holding `pg_advisory_xact_lock`; ClickHouse: `ADD COLUMN IF NOT EXISTS`, no lock); `verify` only reads; `trust` reads nothing. The mode comes from `CROW_SCHEMA` (crow sets it for what it runs from source: `reconcile`, or the crowfile's `schema`), else what a built program was built with (`schema::set_mode`, from the crowfile), else `verify`. `crow db diff` prints `schema::diff`. See the spec, db, The schema a program declares.

### `query[conn, sql]` / `query[conn, sql, params]`

Runs any statement and answers a **frame**: the rows a `SELECT` answers, the rows a `RETURNING` clause reads back, and an empty frame for a statement that answers none (`CREATE`, `UPDATE`, `DELETE`). No count of rows affected is answered: a write that must say what it touched reads it back with `RETURNING`. The optional third argument is a list of bind values for the statement's `?`s; a list among them fills an `IN ?`, one placeholder per item (`WHERE id IN ?` with `[[1, 3]]`). The frame is the type `load['file.csv']` answers, so every frame operation (a mask's rows, `sort`, `group`, `join`, etc.) works on query results.

Whether a call reads or writes is the statement's to say, not the name's: a literal `SELECT`, `VALUES`, `SHOW`, `EXPLAIN`, `DESCRIBE` or a `PRAGMA` asked is a read (`db.read`), anything else a write (`db.write`), and a statement built as the program runs is a write for all the compiler can tell (spec, db, Statements). A write is lowered to the hidden row `db.__query_write`, written `query`, which both backends run as `query`.

### `describe[conn, sql]`

The columns a statement answers, without running it: a frame of `name` and `type`, a column's kind as Crow reads its cells. It is what the compiler asks a database at build time to type a query's cells (spec, db, Typed cells), asked by the program as it runs: Postgres answers from Parse and Describe without an Execute, ClickHouse from `DESCRIBE`, SQLite by preparing the statement.

### `insert[conn, table, frame]`

Bulk inserts a frame into a table and answers the count of rows inserted. Uses backend-specific fast paths:

- **PostgreSQL:** `COPY … FROM STDIN` in the text format (`copy_of_frame`)
- **ClickHouse:** RowBinary over the HTTP interface, encoded by the table's types (`DESCRIBE` first)
- **SQLite:** One prepared statement run for each row, in a transaction (a million rows of four columns in 0.24 s, measured 2026-10-21 on an M4 Max, interpreted)

Inside a handler, the rows go into the handler's transaction, a statement each.

Frame only — no arrays of maps. One data format for DB operations (Rule 1).

### `transact[conn, fn]`

Executes a named function inside a transaction:

1. Checks out one connection from the pool
2. Begins a transaction
3. Calls `fn[tx]`, its one parameter the transaction's connection: the statements it runs through it are the transaction's
4. Commits when the function succeeds; rolls back when it fails (its value an error, or a stop inside it)
5. Returns the connection to the pool

`transact` answers what the function answers: its value, committed, or its error, rolled back; a commit the server refuses answers an error of kind `'db'`. Inside a handler, whose writes through a connection are one transaction already, `transact` is a savepoint on it. ClickHouse has no transactions: `transact` on it answers an error saying so.

No `begin`/`commit`/`rollback` builtins — one way to do transactions (Rule 1).

### Migrations: `crow db apply <dir> <url>`

Migrations are the tool's, not a program's (2026-10-19; `migrate[conn, dir]` was the builtin). The command runs forward-only SQL migrations:

1. Reads `.sql` files from `dir`, sorted by filename
2. Creates `_crow_migrations` table if it doesn't exist
3. Skips already-applied migrations
4. Executes new migrations in order
5. Says how many were newly applied

No rollback files — rollbacks are a new migration that undoes the change.

## Connection Pooling

The pool is transparent to users. Crow manages it automatically:

| Context | Behavior |
|---|---|
| Individual statement | Check out one connection, execute, return |
| `while` loop | Sequential — one connection per statement |
| `for` loop | Sequential too: a loop whose body reaches the database runs its rounds in order (spec, §14 For Loops) |
| Handlers and sessions of a served program | Side by side, each statement on a connection of the pool; a handler's first write begins a transaction on one connection, held until the handler commits or stops |
| `transact[conn, fn]` | One connection held for the entire function |
| `insert[conn, table, frame]` | One connection, one bulk operation |

Every `connect` to one Postgres or ClickHouse URL in a program is one handle, so parts that each connect to the same server share its pool; parts that connect to one SQLite file share its connection.

## Conflict Resolution

`connect` is db's alone — the WebSocket client opens with `socket[url]` — so a program that uses both `db` and `web` calls it bare. When two imported modules do export one name, a bare call is refused and names the qualified spellings: a language-level rule, not db's. See `docs/language-spec.md` § Conflict Resolution.

## Trait Design

```rust
pub trait CrowDb: Send + Sync {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String>;
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String>;
    fn insert(&self, table: &str, frame: &CrowFrame) -> Result<i64, String>;
    fn transact(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String>;
    fn migrate(&self, dir: &str) -> Result<i64, String>;
    fn backend_name(&self) -> &str;
    // with defaults: none, no lock, no scope of a handler's own
    fn describe(&self, sql: &str) -> Result<Vec<(String, String)>, String>;
    fn schema_lock(&self, f: &dyn Fn(&dyn DbTx) -> Result<(), String>) -> Result<(), String>;
    fn begin_scope(&self) -> Option<Result<Box<dyn ScopeTx>, String>>;
}
```

SQL parameters are passed as `serde_json::Value` arrays — the Crow interpreter converts Crow values (int, float, str, bool, and a list for `IN ?`) to JSON for binding. A program's every statement goes through `query`; `exec` and `migrate` are the crate's own (a transaction's `BEGIN` and savepoints, the migrations `crow db apply` runs). Each backend implements this trait — SQLite's `CrowSqliteDb`, the drivers written in Crow through `Driven`, a fixture replay's `Replay`. `connect[url]` dispatches to the correct backend based on URL scheme and returns a `DbHandle` (thread-safe `Arc<dyn CrowDb>` wrapper).

## Implementation Status

| Backend | Status | Notes |
|---|---|---|
| SQLite | ✅ Done | Full support: query, insert, transact, describe |
| PostgreSQL | ✅ Done | Written in Crow over the wire protocol (docs/db-in-crow-design.md): trust, password, md5 and SCRAM-SHA-256; the extended protocol with typed parameters; `frame_of_postgres` for the rows; COPY for inserts; Parse and Describe for describe; CancelRequest; a kept connection per statement at a time; the advisory lock and the handler's transaction in `crow_db::driven`. |
| ClickHouse | ✅ Done | Written in Crow over the HTTP interface (port 8123; docs/db-in-crow-design.md): statements by id, rows as RowBinary decoded by a kernel, inserts as RowBinary encoded by the table's types, `DESCRIBE` for describe, a kept connection per statement at a time, cancel by id, a dead run's statements swept at connect. Transactions correctly refused (OLAP engine). |

### Interpreter

All five verbs (`connect`, `query`, `insert`, `transact`, `describe`) work in `crow run -i` mode. They are forms of their own in the builtin table (`table::FORMS`), answered by `src/modules/db.rs`, which is registered via `ModuleRegistry`. `transact` calls the Crow function back from the interpreter (`DbHandle::transact_calling`): outside a handler, in a transaction scope of its own (SQLite and Postgres), so the function's statements reach one connection as a handler's do; inside a handler, a savepoint on the handler's transaction.

### Transpiler

`use db` sets `needs_crow_db = true` and adds `crow_db` with its `sqlite` feature to the generated `Cargo.toml`. The emitter prints each verb itself (`src/emit_rs/builtins.rs`): `crow_db::schema::connect(url, pool, declared)`, then `query`, `insert`, `describe` and `transact` on the handle. A program whose URLs name Postgres or ClickHouse carries that driver's functions and registers them with `crow_db::driven::register` as it starts. Both bare (`connect[url]`) and prefixed (`db.connect[url]`) syntax are supported.

### Tests

- 33 unit tests in the `crow_db` crate (SQLite, the schema check, `describe` and the types read from a statement, the drivers' handle, the replay, migrations)
- 8 interpreter tests (connect, query, params, transact, bad URL)
- 8 transpiler codegen tests
- The drivers in both modes: `tests/postgres_wire.rs`, `tests/lost_connection.rs` and the Postgres half of `tests/query_cancel.rs` against a scratch cluster of their own (skipped where `initdb` is not on the PATH), `tests/clickhouse_http.rs` and the ClickHouse half against a server of the test's own; a recording replayed, `tests/fixture_replay.rs`
- Both modes compared: the parity programs that `use db` (`tests/parity/`), and `crow check --self`'s cases against its own ClickHouse stand-in and scratch Postgres
- Integration example: `examples/db_demo.co`

## Example

```
use db
use frame

conn = connect['sqlite://:memory:']

--- Create and populate
query[conn, 'CREATE TABLE users (id INTEGER, name TEXT, age INTEGER)']
query[conn, 'INSERT INTO users VALUES (1, ?, ?)', ['alice', 30]]
query[conn, 'INSERT INTO users VALUES (2, ?, ?)', ['bob', 25]]

--- Every statement answers a frame: a SELECT, its rows
users = query[conn, 'SELECT * FROM users']
log[len[users]]

--- Parameterized query
young = query[conn, 'SELECT name FROM users WHERE age < ?', [30]]

--- A write that must say what it touched reads it back
older = query[conn, 'UPDATE users SET age = age + 1 WHERE name = ? RETURNING id', ['bob']]
log[len[older]]

--- Bulk insert
new_users = load['new_users.csv']
insert[conn, 'users', new_users]

--- Transaction
query[conn, 'CREATE TABLE accounts (id INTEGER, balance INTEGER)']
query[conn, 'INSERT INTO accounts VALUES (1, 500), (2, 0)']

fn transfer[t]
  query[t, 'UPDATE accounts SET balance = balance - 100 WHERE id = ?', [1]]
  query[t, 'UPDATE accounts SET balance = balance + 100 WHERE id = ?', [2]]

transact[conn, transfer]

--- Module-prefixed: the same call
db_conn = db.connect['sqlite://data.db']
db.query[db_conn, 'SELECT 1']
```
