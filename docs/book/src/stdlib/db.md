# db

Databases as connections: query, exec, bulk insert, transactions, migrations, and pooling.


The `db` module provides database connectivity. Import with `use db`. Supports SQLite, PostgreSQL, and ClickHouse via URL-scheme dispatch. See `docs/db-module.md` for full architecture details.

### Connection

```
use db

--- SQLite
conn = connect['sqlite://data.db']

--- PostgreSQL (default pool: 10 connections)
conn = connect['postgres://user:pass@host/mydb']

--- ClickHouse
conn = connect['http://localhost:8123/default']

--- Custom pool size
conn = connect['postgres://user:pass@host/mydb', 20]
```

| Function | Description | Example |
|---|---|---|
| `connect[url]` | Open a database connection (pool) with default pool size | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `connect[url, pool_size]` | Open a database connection (pool) with custom pool size | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |

Note: `connect` is also exported by `web.ws`. If both `use db` and `use web` are active, bare `connect[...]` is ambiguous — use `db.connect[...]` or `web.connect[...]`.

### Query & Execute

| Function | Description | Example |
|---|---|---|
| `query[conn, sql]` | Execute a SELECT and return results as a frame | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `exec[conn, sql]` | Execute DDL/DML (CREATE, INSERT, UPDATE, DELETE), return affected row count | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |
| `exec[conn, sql, params]` | Execute parameterized DDL/DML with an array of bind values | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |

```
use db
conn = connect['postgres://user:pass@host/mydb']

--- Query returns a frame
users = query[conn, 'SELECT * FROM users']
log[len[users]]
filtered = filter[users, users.age gt 25]

--- Execute returns affected row count
exec[conn, 'CREATE TABLE users (id INT, name TEXT)']
n = exec[conn, 'DELETE FROM users WHERE age < ?', [18]]
log[n]
```

**List parameters.** An array bound as a parameter stands for a list: its `?` becomes `(?, ?, …)` with one placeholder per element, on every backend. Write `WHERE id IN ?` and pass `[[1, 3]]`; an empty list matches nothing.

**Typed cells in compiled programs.** The compiler asks the program's databases what each literal query returns — a `prepare` on Postgres, `DESCRIBE` on ClickHouse — and types every column: `rows.price[0]` is a float, `rows.n[0]` an int, in the compiled program as it always was in the interpreter. Connections bound to a literal URL (`pg = db.connect['postgres://…']`) are asked in declaration order; `?` placeholders and interpolated spans stand in as NULL. A database that cannot be reached at compile time is skipped and the build says so once; the cells of that query then read as text, the pre-typing behaviour. Frames derived through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack`, and `join` keep their column types. On sqlite, a computed column (`count(*)`, `round(...)`) is typed by stepping the SELECT once with null parameters, and the cells of a `RETURNING` clause — `query[book, 'INSERT ... RETURNING id', [...]]`, the way to read back a row the database numbered — are typed from the table's declared columns; asking never runs a write. Set `CROW_NO_DESCRIBE=1` to build without asking.


### Bulk Insert

| Function | Description | Example |
|---|---|---|
| `insert[conn, table, frame]` | Bulk insert a frame into a table using native fast paths | [Databases: `use db`](../by-example/db.md) |

Uses backend-specific protocols for maximum performance:
- **PostgreSQL:** COPY protocol
- **ClickHouse:** Native RowBinary
- **SQLite:** Batched prepared statements in a transaction

```
use db
conn = connect['http://localhost:8123/default']
insert[conn, 'events', events_frame]
```

### Transactions

| Function | Description | Example |
|---|---|---|
| `transact[conn, fn]` | Execute a named function inside a transaction (auto commit/rollback) | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) |

Checks out one connection from the pool, passes it to the function, commits on success, rolls back on error. No `begin`/`commit`/`rollback` builtins — one way to do transactions.

```
use db
conn = connect['postgres://user:pass@host/mydb']

fn transfer[t]
  exec[t, 'UPDATE accounts SET balance = balance - 100 WHERE id = 1']
  exec[t, 'UPDATE accounts SET balance = balance + 100 WHERE id = 2']

transact[conn, transfer]
```

### Migrations

| Function | Description | Example |
|---|---|---|
| `migrate[conn, dir]` | Run forward-only SQL migrations from a directory | [Databases: `use db`](../by-example/db.md) |

Reads `.sql` files sorted by filename, executes new ones in order, tracks applied migrations in `_crow_migrations` table. Returns count of newly applied migrations.

```
use db
conn = connect['postgres://user:pass@host/mydb']
n = migrate[conn, 'migrations/']
log['`n` migrations applied']
```

### Pool Defaults

| Backend | Default Pool Size | Notes | Example |
|---|---|---|---|
| SQLite | 1 | File-based, concurrent writes lock | |
| PostgreSQL | 10 | Standard for web/data apps | |
| ClickHouse | 10 | Native protocol | |



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
