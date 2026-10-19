# The database drivers written in Crow

Vijay, 2026-10-05: "ok implement it now" — the `db` module's Postgres and
ClickHouse drivers written in Crow on the `sys` layer (design rule 20),
their crates dropped. Measured first (the spec's Design Note "what stays
native, and why"): the protocol in Crow with the rows decoded by a kernel
matches the native drivers or beats them, in both modes.

## 1. What stays where

A program's view of a database does not change: `use db`, `connect[url]`,
`query`, `exec`, `insert`, `transact`, `migrate`, and now `describe`. What
changes is what answers them.

| Piece | Where | Why |
|---|---|---|
| The handle (`DbHandle`), the pool, a handler's transaction scope, the race of a query against a stop, the writes a derivation is invalidated by, the schema modes, migrations, fixture replay, the rehearsal, `crow db diff` | Rust, `crow_db` | Mechanism of the runtime (the design note's first reason): they are called from Rust, by the live runtime and the compiler's tooling |
| The wire protocols — ClickHouse's HTTP interface, Postgres's startup and SCRAM, its simple and extended queries, COPY, cancellation | Crow, `src/modules/clickhouse.co` and `src/modules/postgres.co` | Formulas over the `sys` primitives, as the HTTP client, S3, mail and the WebSocket client are |
| The rows a server answers, decoded into a frame; a frame encoded for a server | Rust kernels, `frame_of_rowbinary`, `rowbinary_of_frame`, `frame_of_postgres` | The formats a table travels in (the design note's second reason): a parser written in Crow was 20–45× slower |
| SQLite | Rust, `rusqlite` | A C library (the third reason) |

## 2. How Rust calls the driver

The driver's functions are rows of the builtin table (`crow!`), so each
has a typed signature both backends know, as `web.mail`'s `mail` has. A
program that imports `db` carries the drivers its connection URLs name
(every driver when a URL is not literal): the inliner seeds them as
reached, and they are inlined into the program like any module written in
Crow.

`crow_db::driven` holds the one `CrowDb` implementation for a driver
written in Crow. It owns what is mechanism — the URL's parts, the pool of
connections (`CrowStream`s the driver opened), the semaphore that bounds
how many statements run at once, the handler's transaction scope, the
reconnect after a connection the server dropped, the cancel after a stop —
and calls the driver for everything that is protocol, through a table of
calls (`crow_db::driven::Calls`):

| Call | ClickHouse | Postgres |
|---|---|---|
| `open[parts]` | a connection to the HTTP port, kept alive | a connection started, authenticated (trust, md5, SCRAM-SHA-256) |
| `query[parts, stream, sql, params]` | POST, `RowBinaryWithNamesAndTypes`, the frame | the extended protocol with typed parameters, the frame |
| `query[parts, stream, sql, params]` | POST; 0 (ClickHouse reports no counts) | the command's count |
| `insert[parts, stream, table, frame]` | `INSERT … FORMAT RowBinary`, the frame encoded by the table's types | `COPY … FROM STDIN` |
| `describe[parts, stream, sql]` | `DESCRIBE TABLE (sql)` | Parse and Describe, no Execute |
| `cancel[parts, stream]` | `KILL QUERY WHERE query_id = …` | a CancelRequest on a new connection |

The table is registered by the program:

- **Compiled:** the generated program registers, at its start, a `Calls`
  whose methods call the generated functions (`_db_clickhouse_query`, …),
  as it registers the mail sender. Any thread may call them.
- **Interpreted:** the interpreter keeps, per thread, a child interpreter
  over the same program's functions, and registers a `Calls` that calls
  the driver's functions in it. A child, not the interpreter itself: the
  call arrives inside a builtin the interpreter is evaluating, and a
  second mutable path into it would be no path at all. A parallel loop's
  worker thread makes its own child from the program the interpreter
  loaded.
- **The compiler's tooling:** `crow db diff` and `crow db describe` build
  a child over `use db` alone and register it; the compile-time `describe`
  that types a query's cells, which runs inside the lowering, asks a
  `crow db describe` of its own process for the answer (a nested lowering
  would unsettle the lowering's thread-local state), and keeps it in
  `.crow/schema` as before.

A `Calls` method takes the URL's parts as JSON text (`Encoded`), the
stream (`Stream`), the statement and its parameters as JSON text
(`Encoded`): what every parameter kind can travel as, in both modes.

## 3. The stream's note

A connection carries a note the driver writes (`sys.note`, `sys.noted`): a
query's id for ClickHouse, the backend's process id and secret key for
Postgres's CancelRequest. A stop reaches the driver through `recv`: a wait
inside a derivation whose flag was raised ends with the reason
`cancelled`, the driver answers it, and `crow_db::driven` then calls
`cancel` with the stream and lets the stream go.

## 4. URLs

`clickhouse://user:pass@host:port/database` names the HTTP interface, port
8123 when none is given. The native protocol's port, 9000, is no longer
spoken; a URL that names it is read as the HTTP port, and the log says so
once. `postgres://user:pass@host:port/database` as before.

## 5. What the Rust drivers did that the Crow drivers keep

Reconnect after the server dropped a connection (an idle ClickHouse
connection is let go before its keep-alive runs out, as the server's
`Keep-Alive` header says, so a reconnect is a restart's, not every idle
gap's); a statement that a stopping program had in flight killed on the
server — every wait on a connection ends with the program's stop, the
driver tells the server, and the stop waits for that, three seconds at
most (`crow_runtime::cancel`, tests/query_cancel.rs); ClickHouse's
statements a dead run of this machine left running swept at connect; a
server that does not answer stopping the program at connect with the
reason; Postgres's advisory lock around reconciling a schema; a handler's
transaction on one connection; `transact` on one connection — what the
function answers, committed, or its error, rolled back; a savepoint inside
a handler's transaction (`crow_db::DbHandle::transact`, and
`transact_calling` for the interpreter, whose function speaks through the
handle itself); typed
parameters (`?::date` a cast from text); `IN ?` lists; the server's own
error words.

## 6. Milestones

1. ClickHouse (ff0975a, 2026-10-04): the driven handle, the calls table,
   the ClickHouse module, both backends' registration, the tooling, the
   native driver and klickhouse gone. The gate found a module local named
   like a derivation of finch's (`found`) refused as an assignment of it:
   the lowering's derived-assignment check now stands in the module's own
   scope like its other guards (parity 166).
2. Postgres (83e6dad, 2026-10-05): the module, scopes and transactions,
   COPY, cancellation, tokio-postgres and deadpool gone; tokio stays for
   the server. Found by its test and fixed with it: `transact` committed
   a function that failed, in both modes — it answers the function's
   value now, rolls back on its error, and is a savepoint inside a
   handler's transaction (parity 167). And a row called from Crow
   converts its arguments by its signature (JSON text to an `Encoded`
   parameter arrives as a JSON string), so a module's rows call helpers,
   never each other.
3. The spec's db section, the design note's accounting (every crate of
   the drivers gone from the manifests; the drivers listed among what is
   written in Crow, with the measurements kept), docs/db-module.md,
   docs/web-framework.md, the roadmap and the book — folded into the two
   commits above. Done.
