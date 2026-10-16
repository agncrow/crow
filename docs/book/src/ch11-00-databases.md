# Working with Databases

`use db` gives you SQL — real SQL, no ORM, no query builder. Connections
are pooled handles, results come back as the frames you learned in
chapter 10, and failures are the error values you learned in chapter 7.
Nothing in this chapter is a new concept; it's the existing concepts
meeting a database.

## Connect, Create, Insert

```crow
use db

conn = connect['sqlite://:memory:']
exec[conn, 'CREATE TABLE trades (id INTEGER PRIMARY KEY, symbol TEXT, qty INTEGER, price REAL)']
exec[conn, 'INSERT INTO trades VALUES (1, ?, ?, ?)', ['AAPL', 100, 191.5]]
exec[conn, 'INSERT INTO trades VALUES (2, ?, ?, ?)', ['MSFT', 50, 415.2]]
exec[conn, 'INSERT INTO trades VALUES (3, ?, ?, ?)', ['AAPL', 40, 193.1]]

rows = query[conn, 'SELECT symbol, qty FROM trades WHERE qty >= 50 ORDER BY id']
log[rows]
```

```text
symbol  qty
   str  int
------  ---
  AAPL  100
  MSFT   50

[2 rows x 2 cols]
```

Three things to notice:

- **The connection string picks the backend** — `sqlite://:memory:` here;
  a file path or `postgres://user:pass@host/dbname` in production. Your
  code doesn't change.
- **`?` placeholders bind values** from an array — never interpolate user
  input into SQL text. The bind array is *positional open data*, one of
  the few places a mixed-type array literal is exactly right.
- **SQL strings are SQL.** Crow strings are inert, so `>=`, quotes, and
  `%` wildcards need no escaping — and it's `>=` in there, not `gte`;
  the database speaks its own language.

`query` returns a frame — everything from chapter 10 (masks, sort,
columns, `sum`) applies to results directly. `exec` returns the affected
row count:

```crow
use db

conn = connect['sqlite://:memory:']
exec[conn, 'CREATE TABLE trades (id INTEGER PRIMARY KEY, symbol TEXT, qty INTEGER)']
exec[conn, 'INSERT INTO trades VALUES (1, ?, ?)', ['AAPL', 100]]
exec[conn, 'INSERT INTO trades VALUES (2, ?, ?)', ['MSFT', 50]]

n = exec[conn, 'UPDATE trades SET qty = qty + 10 WHERE symbol = ?', ['AAPL']]
log[n]
```

```text
1
```

## Failure Is a Value Here Too

A bad query doesn't crash and doesn't throw — it hands you chapter 7:

```crow
use db

conn = connect['sqlite://:memory:']
bad = query[conn, 'SELECT nope FROM missing_table']
log[bad.ok]
log[bad.reason]
log[bad.kind]
```

```text
false
no such table: missing_table
db
```

Check `.ok` at this boundary (databases are exactly the kind of boundary
chapter 7 meant), branch on `.kind eq 'db'` if you handle database
failures specially, and let success flow.

## Transactions

`transact[conn, fn]` runs a named function inside a transaction — commit
if it completes, roll back everything if it fails. All-or-nothing
transfers, the classic:

```crow
use db

conn = connect['sqlite://:memory:']
exec[conn, 'CREATE TABLE accounts (name TEXT PRIMARY KEY, balance INTEGER)']
exec[conn, 'INSERT INTO accounts VALUES (?, ?)', ['alice', 100]]
exec[conn, 'INSERT INTO accounts VALUES (?, ?)', ['bob', 50]]

fn transfer[tx]
    exec[tx, 'UPDATE accounts SET balance = balance - 30 WHERE name = ?', ['alice']]
    exec[tx, 'UPDATE accounts SET balance = balance + 30 WHERE name = ?', ['bob']]

transact[conn, transfer]
log[query[conn, 'SELECT * FROM accounts ORDER BY name']]
```

```text
 name  balance
  str      int
-----  -------
alice       70
  bob       80

[2 rows x 2 cols]
```

The function receives a transaction handle `tx` and uses the same `exec`
on it — passed by name, like every callback in Crow.

That's the whole database API: `connect`, `query`, `exec`, `insert`,
`transact`, `migrate`.

## Letting the Checker Read Your Schema

There's a class of database bug that no amount of testing on your laptop
will find, because it only exists when *two copies* of your program run at
once. Here's the classic — enforcing "at most one active trade per symbol"
with an `if`:

```crow
use db

conn = connect['sqlite://:memory:']
exec[conn, 'CREATE TABLE trades (symbol TEXT, qty INT, status TEXT)']

sym = 'AAPL'
open_trades = query[conn, 'SELECT symbol FROM trades WHERE symbol = ? AND status = ?', [sym, 'active']]
if len[open_trades] eq 0
    exec[conn, 'INSERT INTO trades (symbol, qty, status) VALUES (?, ?, ?)', [sym, 100, 'active']]

after = query[conn, 'SELECT symbol FROM trades']
log[len[after]]
```

```text
1
```

Runs perfectly. Ships. Then one day two copies run at the same moment:
both query, both see zero rows, both insert — two active trades, and the
`if` never lied, it just couldn't hold between two programs.

Point `crow check` at your development database and it reads *both sides*
of the story — your code's assumptions and the schema's actual guarantees:

```bash
crow check trading.co --db 'sqlite://dev.db'
```

```text
trading.co:7-9 — two programs can pass this check at the same time

  what can happen:
    1. program A runs the query (line 7) — the check passes
    2. program B runs the same query — the check passes too
    3. A writes (line 9).  4. B writes.
    5. the rule the check was protecting is now broken

  the database has no rule saying 'at most one active trades row per symbol',
  so this if is the only guard — and an if cannot hold between two programs.

  fixes, safest first:
  [1] let the database enforce it — generates migrations/one_active_per_symbol.sql:
        CREATE UNIQUE INDEX one_active_per_symbol ON trades(symbol) WHERE status = 'active';
```

`crow fix trading.co --hazard 1 --option 1` applies the first option:
writes the migration, applies it to the dev database, and re-checks to
confirm the hazard is gone. After that, the losing program's insert comes
back as an ordinary error value with `kind = 'conflict'` — loud, at the
moment it matters, instead of a silent duplicate.

The catalog goes well beyond this one shape — lost updates, one-way status
flags, single-row reads on non-unique columns, un-transacted write pairs,
plus schema verification (column typos get did-you-mean suggestions). Put
`db = 'sqlite://dev.db'` in your crowfile and every `crow check` — and the
editor, through `crow lsp` — runs them. The full reference is
`docs/crow-check.md`.

Next: putting a live web page in front of it.

## A list is a parameter too

An array bound as a parameter expands to a list, so `WHERE id IN ?`
takes `[[1, 3]]` and never needs the ids pasted into the query text:

```crow
use db

conn = db.connect['sqlite://:memory:']
exec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']
exec[conn, 'INSERT INTO t VALUES (1, ?)', ['ann']]
exec[conn, 'INSERT INTO t VALUES (2, ?)', ['bob']]
exec[conn, 'INSERT INTO t VALUES (3, ?)', ['cat']]
picked = query[conn, 'SELECT name FROM t WHERE id IN ? ORDER BY id', [[1, 3]]]
log[picked.name]
```

```text
['ann', 'cat']
```

## Your tables are the ones your program makes

The `CREATE TABLE` statements your program runs are its schema. Each
literal one it hands a connection declares that table, and opening the
connection checks the tables the database already has against them:

```crow,norun
use db

book = connect['sqlite://board.db']
exec[book, 'CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, title TEXT, due TEXT)']
```

Add `due` to that statement after `board.db` was made without it, and
`CREATE TABLE IF NOT EXISTS` alone would do nothing: the first query that
reads `due` would fail. Run from its source (`crow run`, `crow run -i`,
`crow test`), the program *reconciles* instead: the connection opens,
finds `cards` without `due`, and adds it, under a lock so two processes
starting at once add it once. What it cannot add without guessing — a
`NOT NULL` column with no default on a table that has rows, a column of
another kind than the database holds — stops the program with the SQL
that would make it.

A program `crow build` makes *verifies*: it changes nothing, reads the
catalog, and stops at connect if a table lacks a column the program
declares, printing the statement that would mend it — the database in
production is someone else's to change. `crow db diff` prints every such
statement as a migration to review, with what reconciling would not make
commented out:

```text
$ crow db diff app.co
-- sqlite://board.db (book)
-- table 'cards' lacks column 'due'
ALTER TABLE "cards" ADD COLUMN due TEXT;
```

`schema = 'verify'` in the crowfile (or `'reconcile'`, or `'trust'`,
which checks nothing and says so at every start) changes the mode for
both; `CROW_SCHEMA=trust` changes it for one run. A table your program
does not make is not its to check.

## The compiler asks your database

A query's result is a frame, and a frame's cells are typed by the
database: an integer column reads as an int, a numeric one as a float.
The interpreter sees that at run time. The compiled program cannot, so
`crow build` and `crow run` ask the databases your program connects to
what each literal query returns — a prepared statement on Postgres, a
`DESCRIBE` on ClickHouse — and type the cells the same way. A `?`
placeholder and an interpolated span stand in as NULL for the question.
If a database cannot be reached while building, the build says so once
and that query's cells read as text, which is what compiled frames always
did before. `CROW_NO_DESCRIBE=1` skips the question entirely.

Next: the web layer, where these frames become pages.
