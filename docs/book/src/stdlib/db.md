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
| `connect[url]` | Open a database connection (pool) with default pool size | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `connect[url, pool_size]` | Open a database connection (pool) with custom pool size | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |

Note: `connect` is also exported by `web.ws`. If both `use db` and `use web` are active, bare `connect[...]` is ambiguous — use `db.connect[...]` or `web.connect[...]`.

A connection is not the program's lifetime. When the server closes it — ClickHouse drops an idle native connection after an hour, and a restart drops every one — the next query reopens it and runs, and the server log says so; Postgres and ClickHouse connections are pooled, so queries from different sessions run side by side, and sqlite is a file. Every `connect` to one Postgres or ClickHouse URL in a program is one handle: parts that each connect to the same server share its pool.

**A query nobody waits for is stopped.** A background recompute whose session has gone, or whose inputs moved while it ran, stops its query on the server (ClickHouse and Postgres); so does every query in flight when a served program is told to stop — `crow stop`, a rebuild, Ctrl-C. A program killed outright cannot stop anything; the next run that connects to a ClickHouse server on the same machine stops the queries the dead run left behind, and the log says so. Each ClickHouse statement carries a leading comment naming the machine, the process and a number, which is how it is found again.

### Query & Execute

| Function | Description | Example |
|---|---|---|
| `query[conn, sql]` | Execute a SELECT and return results as a frame | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `exec[conn, sql]` | Execute DDL/DML (CREATE, INSERT, UPDATE, DELETE), return affected row count | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |
| `exec[conn, sql, params]` | Execute parameterized DDL/DML with an array of bind values | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |

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

**List parameters.** An array bound as a parameter stands for a list: its `?` becomes `(?, ?, …)` with one placeholder per element, on every backend. Write `WHERE id IN ?` and pass `[[1, 3]]`; an empty list matches nothing. A list goes after `IN` and nowhere else: handed to another `?` — `x = ANY(?)` — it would be sent as a row where the database wants an array, so a literal statement that does so is refused, in both modes, before it runs. The compiler asks a database about `IN ?` as the list of one it is sent as, `IN (?)`.

**Typed cells in compiled programs.** The compiler asks the program's databases what each literal query returns — a `prepare` on Postgres, `DESCRIBE` on ClickHouse — and types every column: `rows.price[0]` is a float, `rows.n[0]` an int, in the compiled program as it always was in the interpreter. A column that can hold NULL has fallible cells, in both modes: a NULL one is an error value of reason `missing` (see frame). The compiler reads which can from the statement and the tables' declarations — a column read from a table can hold NULL unless it is declared `NOT NULL` (or is SQLite's `INTEGER PRIMARY KEY`); every column of the side of an outer join a row can have no match on can; an expression can where what it reads can, except `count`, `coalesce` with an argument that cannot, `IS NULL` and a literal, and an aggregate over no rows (no `GROUP BY`) can; a function it does not know can; and a column the `WHERE` clause compares or asks `IS NOT NULL` of cannot, in the rows it keeps. ClickHouse says it itself, by a `Nullable` type. A statement it cannot read that way can hold NULL in every column. Connections bound to a literal URL (`pg = db.connect['postgres://…']`) are asked in declaration order; `?` placeholders and interpolated spans stand in as NULL. A database that cannot be reached at compile time is skipped. What each database answers is kept in the project's `.crow/schema` — a file per statement and the connection it was asked on, holding the statement and its columns' types, a column that can hold NULL marked `?` (`float?`) — and when no database answers a statement its cells are typed from that record, and the build says so once; with no record they are dynamic, as below. A project may keep `.crow/schema` in version control, so a machine without the databases — CI, a laptop without the data — builds the program typed as it was. The frames the standard modules build are typed as they are built: `doc.chapters`' `title`, `path` and `part` are text and its `depth` an int, and `chart.macd`'s columns are floats. Frames derived through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack`, and `join` keep their column types; `group` gives its key column as it was and a float per aggregate (`group[df, 'dept', ['salary', 'mean']]` has `dept` and `salary_mean`). A frame handed to a function keeps its column types there, when every call hands that parameter the same columns — a frame built in the program, read from a database or derived from either — so a function reads its cells as the numbers or text they are. A cell of a column whose type nothing tells the compiler — a file loaded as it runs, a query no database answered and no record holds — is dynamic, as a value `json[]` parsed is, in both modes: `int[c]`, `float[c]` and `str[c]` read it as one type (and fail as they do on text), arithmetic or a comparison with a number on it is refused until the program says what it is (`cannot use '*' with json and float — … read as one first: int[…], float[…] or str[…]`), as is giving one to a variable that holds another type (`cannot reassign 'next' from str to json — … read as one first`), and a missing one is JSON's `null`, an error value of reason `null`. (Compiled, such a cell was read as text while the interpreter read the number: the same line meant two things.) On sqlite, a computed column (`count(*)`, `round(...)`) is typed by stepping the SELECT once with null parameters; when no row comes back — `WHERE session_id = ?`, an empty table — it is typed from the statement itself, by the declared types of the columns it reads and SQLite's rules (`round`, `avg` and arithmetic on a `REAL` are floats, `count` and comparisons ints, `||` and `datetime` text; a parameter or `NULL` alone stays text). The cells of a `RETURNING` clause — `query[book, 'INSERT ... RETURNING id', [...]]`, the way to read back a row the database numbered — are typed from the table's declared columns; asking never runs a write, and never creates a sqlite file that is not there yet. Every command asks the same way — `crow run`, `crow run -i`, `crow test`, `crow check` and the app's book — so a program refused for its columns' types (a local given an int cell, then text) is refused by all of them. Set `CROW_NO_DESCRIBE=1` to build without asking: no database answers then, so the records type the cells, and nothing is written.


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
| ClickHouse | 4 | Native protocol; opened as queries need them. One query already runs on every core of the server, so more at once contend rather than help | |



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
