# db

Databases as connections: every statement through query, bulk insert, transactions, the tables a program declares, and pooling (migrations are the tool's: crow db apply).


The `db` module provides database connectivity. Import with `use db`. Supports SQLite, PostgreSQL, and ClickHouse via URL-scheme dispatch. See `docs/db-module.md` for full architecture details.

### Connection

```
use db

--- SQLite
conn = connect['sqlite://data.db']

--- PostgreSQL (default pool: 10 connections)
conn = connect['postgres://user:pass@host/mydb']

--- ClickHouse (its HTTP interface: port 8123 when none is given)
conn = connect['clickhouse://default@localhost:8123/default']

--- Custom pool size
conn = connect['postgres://user:pass@host/mydb', 20]
```

| Function | Description | Example |
|---|---|---|
| `connect[url]` | Open a database connection (pool) with default pool size, checking its database holds the tables the program declares on it (The schema a program declares, below) | [Databases: `use db`](../by-example/db.md) · [Web: responses and submodules](../by-example/web-more.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `connect[url, pool_size]` | Open a database connection (pool) with custom pool size; a pool size of 0 is the default | [Databases: `use db`](../by-example/db.md) · [Web: responses and submodules](../by-example/web-more.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `describe[conn, sql]` | The columns a statement answers, without running it: a frame of `name` and `type` (`int`, `float`, `str`, `bool`, `datetime`, with `?` after a kind where the column can hold NULL); a `?` placeholder stands in as NULL. What the compiler asks at build time (Typed cells, below), asked by the program | [Databases: `use db`](../by-example/db.md) · [The standard library's layer: `use sys`](../by-example/sys.md) |

**The ClickHouse driver is written in Crow** (`src/modules/clickhouse.co`, design rule 20, docs/db-in-crow-design.md), over its HTTP interface: `clickhouse://user:pass@host:port/database` names that interface, port 8123 when none is given; the native protocol's port, 9000, is no longer spoken, and a URL that names it is read as the HTTP port, which the log says once. Each statement is sent with an id (`crow-<machine>-<process>-<draw>`), its rows come back as RowBinary and are decoded by the `frame_of_rowbinary` kernel, typed as the server names each column; an insert goes as RowBinary encoded by the table's types; a statement a stop abandoned is killed by its id (tests/query_cancel.rs); at connect, the statements a dead run of this machine left running are swept. The server says how long it keeps a connection open (its `Keep-Alive` header), and the driver notes it: an idle connection past that is let go before the server closes it, not found closed under the next statement. The handle, its pool of kept connections (four, since ClickHouse runs one query on every core already), the reconnect and the cancel are `crow_db`'s, which calls the driver through a table of calls the program registers (the compiled program's generated functions; the interpreter's child over the program's functions). A program never imports the driver: `use db` carries the drivers its connection URLs name.

**The Postgres driver is written in Crow too** (`src/modules/postgres.co`), speaking the wire protocol over `sys`: a connection started and authenticated — trust, a clear password, md5, or SCRAM-SHA-256 as RFC 7677 says, its proof held to the RFC's own example in the module's tests — and its backend's process and key noted for a cancel; a statement without parameters as a simple query, one with parameters through the extended protocol, each parameter typed by its kind (text, int8, float8, bool; a cast like `?::date` is a cast from text) and bound as text; the answer decoded by the `frame_of_postgres` kernel, typed as the server names each column; a command's count from its completion; `insert` as `COPY … FROM STDIN` in the text format (`copy_of_frame`); `describe` from Parse and Describe without an Execute; a statement a stop abandoned cancelled by a CancelRequest on a connection of its own. tokio-postgres and deadpool are gone; the pool (ten), a handler's transaction on one connection, `transact`, the advisory lock around reconciling a schema and the reconnect are `crow_db`'s.

A connection is not the program's lifetime. When the server closes it — a restart drops every one — the next query reopens it and runs, and the server log says so; **a program told to stop stops the statements it had in flight on their servers** (ClickHouse by the statement's id, Postgres by a CancelRequest) and waits for that, three seconds at most, before it ends: a restarted app does not leave its predecessor's queries running; Postgres and ClickHouse connections are pooled, so queries from different sessions run side by side, and sqlite is a file. Every `connect` to one Postgres or ClickHouse URL in a program is one handle: parts that each connect to the same server share its pool.

**A query nobody waits for is stopped.** A background recompute whose session has gone, or whose inputs moved while it ran, stops its query on the server (ClickHouse and Postgres); so does every query in flight when a served program is told to stop — `crow stop`, a rebuild, Ctrl-C. A program killed outright cannot stop anything; the next run that connects to a ClickHouse server on the same machine stops the queries the dead run left behind, and the log says so. Each ClickHouse statement carries a leading comment naming the machine, the process and a number, which is how it is found again.

### Statements

| Function | Description | Example |
|---|---|---|
| `query[conn, sql]` | Run a statement; the rows it answers, as a frame | [Databases: `use db`](../by-example/db.md) · [Web: responses and submodules](../by-example/web-more.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `query[conn, sql, params]` | The same, with a list of bind values for its `?`s | [Databases: `use db`](../by-example/db.md) · [Web: responses and submodules](../by-example/web-more.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |

**One verb runs every statement.** `query` answers a frame: the rows a `SELECT` answers, the rows a `RETURNING` clause reads back, and an empty frame for a statement that answers none — a `CREATE`, an `UPDATE`, a `DELETE`. No count of rows affected is answered: a write that must say what it touched reads it back with `RETURNING`.

**The statement says whether it reads or writes, not the name.** A literal statement is read by the compiler: a `SELECT` (without `INTO`), `VALUES`, `SHOW`, `EXPLAIN`, `DESCRIBE` and a `PRAGMA` asked are reads (`db.read`); anything else is a write (`db.write`), `RETURNING` included, so a derivation may hold a literal read and a handler's writes are held to its commit (Effects Have Kinds). A statement built as the program runs — a string joined from pieces, a parameter — writes for all the compiler can tell: a derivation that hands one is refused, naming the fix, which is the literal with `?` for its parameters.

```
use db
conn = connect['postgres://user:pass@host/mydb']

users = query[conn, 'SELECT * FROM users']
log[len[users]]
filtered = users[users.age gt 25]

query[conn, 'CREATE TABLE IF NOT EXISTS users (id INT, name TEXT)']
gone = query[conn, 'DELETE FROM users WHERE age < ? RETURNING id', [18]]
log[len[gone]]
```

**Design Note: one statement verb.** Considered: `exec` beside `query`, two names carrying what the compiler needs before the program runs — the answer's kind (a count or a frame) and the effect's (a write or a read) — which every typed client has (Go's `Exec` and `Query`, sqlx, JDBC); a flag on the call. Rejected: the statement already says both, and the compiler reads literal statements anyway; the count was read in one program of the corpus; and a `SELECT` through `exec` failed on SQLite with the library's words while Postgres and ClickHouse ran it and threw the rows away. Decided (Vijay, 2026-10-19): one verb, `query`, a frame for every statement, the literal's first word deciding its kind, a computed statement a write.

**List parameters.** An array bound as a parameter stands for a list: its `?` becomes `(?, ?, …)` with one placeholder per element, on every backend. Write `WHERE id IN ?` and pass `[[1, 3]]`; an empty list matches nothing. A list goes after `IN` and nowhere else: handed to another `?` — `x = ANY(?)` — it would be sent as a row where the database wants an array, so a literal statement that does so is refused, in both modes, before it runs. The compiler asks a database about `IN ?` as the list of one it is sent as, `IN (?)`.

**Typed cells in compiled programs.** The compiler asks the program's databases what each literal query returns — a `prepare` on Postgres, `DESCRIBE` on ClickHouse — and types every column: `rows.price[0]` is a float, `rows.n[0]` an int, in the compiled program as it always was in the interpreter. A column that can hold NULL has fallible cells, in both modes: a NULL one is an error value of reason `missing` (see frame). The compiler reads which can from the statement and the tables' declarations — a column read from a table can hold NULL unless it is declared `NOT NULL` (or is SQLite's `INTEGER PRIMARY KEY`); every column of the side of an outer join a row can have no match on can; an expression can where what it reads can, except `count`, `coalesce` with an argument that cannot, `IS NULL` and a literal, and an aggregate over no rows (no `GROUP BY`) can; a function it does not know can; and a column the `WHERE` clause compares or asks `IS NOT NULL` of cannot, in the rows it keeps. ClickHouse says it itself, by a `Nullable` type. A statement it cannot read that way can hold NULL in every column. Connections bound to a literal URL (`pg = db.connect['postgres://…']`) are asked in declaration order; `?` placeholders and interpolated spans stand in as NULL. A database that cannot be reached at compile time is skipped. What each database answers is kept in the project's `.crow/schema` — a file per statement and the connection it was asked on, holding the statement and its columns' types, a column that can hold NULL marked `?` (`float?`) — and when no database answers a statement its cells are typed from that record, and the build says so once; with no record they are dynamic, as below. A project may keep `.crow/schema` in version control, so a machine without the databases — CI, a laptop without the data — builds the program typed as it was. The frames the standard modules build are typed as they are built: `doc.chapters`' `title`, `path` and `part` are text and its `depth` an int, and `chart.macd`'s columns are floats. Frames derived through `select`, a mask, `sort`, `head`, `tail`, `fill`, `stack`, and `join` keep their column types; `group` gives its key columns as they were and a float per aggregate (`group[df, 'dept', ['salary', 'mean']]` has `dept` and `salary_mean`); `pivot` gives its rows column as it was, the rest as they show; a `window` is typed by what it computes (frame). A frame handed to a function keeps its column types there — each call's, the function a copy per set of columns (see frame, A frame's type is its columns) — so a function reads its cells as the numbers or text they are. A cell of a column whose type nothing tells the compiler — a file loaded as it runs, a query no database answered and no record holds — is dynamic, as a value `json[]` parsed is, in both modes: `int[c]`, `float[c]` and `str[c]` read it as one type (and fail as they do on text), arithmetic or a comparison with a number reads it as a number, and giving one to a variable that holds another type reads it as that type, as a parsed value's field is read (Parsed values: read as its use implies) — a cell that is not what the use reads stops the program and says where (`a parsed value holds text 'abc' where the program reads a number (met in mul at line 6)`) — and a missing one is JSON's `null`, an error value of reason `null`. (Compiled, such a cell was read as text while the interpreter read the number: the same line meant two things.) On sqlite, a computed column (`count(*)`, `round(...)`) is typed by stepping the SELECT once with null parameters; when no row comes back — `WHERE session_id = ?`, an empty table — it is typed from the statement itself, by the declared types of the columns it reads and SQLite's rules (`round`, `avg` and arithmetic on a `REAL` are floats, `count` and comparisons ints, `||` and `datetime` text; a parameter or `NULL` alone stays text). The cells of a `RETURNING` clause — `query[book, 'INSERT ... RETURNING id', [...]]`, the way to read back a row the database numbered — are typed from the table's declared columns; asking never runs a write, and never creates a sqlite file that is not there yet. Every command asks the same way — `crow run`, `crow run -i`, `crow test`, `crow check` and the app's book — so a program refused for its columns' types (a local given an int cell, then text) is refused by all of them. Set `CROW_NO_DESCRIBE=1` to build without asking: no database answers then, so the records type the cells, and nothing is written.


### Bulk Insert

| Function | Description | Example |
|---|---|---|
| `insert[conn, table, frame]` | Bulk insert a frame into a table using native fast paths | [Databases: `use db`](../by-example/db.md) |

Uses backend-specific protocols for maximum performance:
- **PostgreSQL:** `COPY … FROM STDIN`, the text format
- **ClickHouse:** RowBinary over the HTTP interface, encoded by the table's types (`DESCRIBE` first)
- **SQLite:** Batched prepared statements in a transaction

```
use db
conn = connect['clickhouse://localhost/default']
insert[conn, 'events', events_frame]
```

### Transactions

| Function | Description | Example |
|---|---|---|
| `transact[conn, fn]` | The function run inside a transaction: its value, committed; or its error, rolled back | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) |

One connection is checked out, `BEGIN` run on it, and the function handed it as its one parameter; its statements through that parameter are the transaction's. **`transact` answers what the function answers.** A function that fails — its value an error (`return error['no', 'db']`, or a failed value it returns), or a stop inside it — rolls the transaction back, and `transact` answers that error (a stop stops, as a stop does); a function that succeeds commits, and a commit the server refuses answers an error of kind `'db'` with the server's words. Inside a handler, whose writes through a connection are one transaction already (A handler is a transaction), `transact` is a savepoint on it: released when the function succeeds, rolled back to when it fails, the handler's other writes untouched. The same in both modes (parity 167). No `begin`/`commit`/`rollback` builtins — one way to do transactions.

```
use db
conn = connect['postgres://user:pass@host/mydb']

fn transfer[t]
  query[t, 'UPDATE accounts SET balance = balance - 100 WHERE id = 1']
  query[t, 'UPDATE accounts SET balance = balance + 100 WHERE id = 2']

transact[conn, transfer]
```

### Migrations

Migrations are the tool's, not a program's: `crow db apply <dir> <url>` runs the `.sql` files of a directory not yet applied on the database, in name order, recording each in a `_crow_migrations` table, and says how many were new. It is the apply half of `crow db diff`, which drafts the statements a schema change needs (below).

### The schema a program declares

**A program's tables are the ones its own statements make.** Every literal `CREATE TABLE` a program hands a connection — `query[book, 'CREATE TABLE IF NOT EXISTS cards (…)']`, directly or through a function the connection is handed to (`setup[book]`) — declares that table, with its columns, on that connection. A table the program also drops there (`DROP TABLE IF EXISTS scratch`) is one it remakes each run, and declares nothing. A connection the program declares no table on is one it does not own: nothing below reads or changes it, and the tables and columns a database has beyond the declared ones are its own.

**Opening a connection checks that the tables the database has hold what the program declares**, in one of three modes, the same in both modes of running. A declared table the database does not have yet is not a difference: the program's own statement makes it as it runs, so a fresh database starts as it always has.

| Mode | At connect | Example |
|---|---|---|
| `reconcile` | What a table lacks is made: a declared column it does not have is added (`ALTER TABLE … ADD COLUMN`, as the statement declares it), under the database's lock (Postgres: an advisory lock, in one transaction; SQLite: an immediate transaction), so two processes starting at once make it once. A column SQLite cannot add in place — a key, or `NOT NULL` without a default — is made by making a table with no rows anew. A difference reconciling cannot make without losing data or guessing — a column of another kind than declared, a `NOT NULL` column without a default for a table that has rows — stops the program with each difference and the statements that would make it, and nothing is made. |  |
| `verify` | Nothing is changed. The database's catalog is read (reading it needs no right beyond reading), and every declared column of every table the database has must be there, of the declared kind; a difference stops the program at connect with each difference and the statements that would mend it. | [Web: responses and submodules](../by-example/web-more.md) |
| `trust` | Nothing is read, and every start says so: `schema not verified — schema = 'trust'`. |  |

A column's kind is the kind Crow reads its cells as (int, float, str, bool, datetime): `TEXT` and `VARCHAR(20)` are one kind, `INTEGER` and `TEXT` two. A column declared without a type is of any kind.

A program run from its source — `crow run`, `crow run -i`, `crow test` — reconciles: it is being developed, against a database of its own. A program `crow build` makes verifies: it runs where the database is someone else's to change. The crowfile's `schema = 'verify'` (or `'reconcile'`, `'trust'`) says otherwise for both, and is what a built program starts with; the environment's `CROW_SCHEMA` says otherwise for one run (`CROW_SCHEMA=trust ./app`).

`crow db diff [file.co]` prints, for each connection a program declares tables on and opens with a url it writes, the SQL that would make its database hold what the program declares: the `CREATE TABLE` of each declared table it lacks, the `ADD COLUMN` statements reconciling would run and, commented out, the changes reconciling will not make (a column of another kind, a `NOT NULL` column a table with rows cannot take without a default) — a draft for a migration to review. It only reads the catalog; a SQLite file that is not there is not made.

```
$ crow db diff app.co
-- sqlite://board.db (book)
-- table 'cards' lacks column 'due'
ALTER TABLE "cards" ADD COLUMN due TEXT;
-- table 'labels' is missing
CREATE TABLE IF NOT EXISTS labels (card INTEGER, name TEXT);
```

**Design Note: schema modes.** Considered: migrations alone, applied by hand, which leave a table the program changed — a new column — to fail at the first query that reads it, in development and in production alike; making the schema everywhere (Prisma's `db push`, Django's old `syncdb`), which alters production databases the program's user has no right to alter and must not; and a check only in production. Decided (Vijay, July 2026; approved September 2026): a program's `CREATE` statements declare its tables; development makes what is missing under a lock, a built program checks it at connect, and `crow db diff` drafts the migration for what making would lose (as Terraform's `plan` drafts before an `apply`, and Prisma's `db push` in development meets `migrate deploy` in production). At connect a missing table is left to the program's own `CREATE`, which runs moments later: checking it there stopped every built program on its first run against a fresh database, and making it there broke a program whose own `CREATE TABLE` (without `IF NOT EXISTS`) then found it made — the drift worth stopping for is the table that exists with other columns, which `CREATE TABLE IF NOT EXISTS` silently keeps.

### Pool Defaults

| Backend | Default Pool Size | Notes | Example |
|---|---|---|---|
| SQLite | 1 | File-based, concurrent writes lock | |
| PostgreSQL | 10 | Standard for web/data apps; the driver written in Crow keeps the connections it opened | |
| ClickHouse | 4 | The HTTP interface, kept alive; opened as queries need them. One query already runs on every core of the server, so more at once contend rather than help | |



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
