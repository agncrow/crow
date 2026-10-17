# crow check — schema-aware database checks

`crow check` verifies a program without running it: purity analysis, edge-case
generation, and regression snapshots. With a database URL it also
cross-references the program's SQL against the schema the database actually
enforces, and reports **concurrency hazards** — the class of bug where code is
correct alone but wrong the moment two copies of it run.

```
crow check app.co --db 'sqlite://dev.db'      --- live introspection
crow check app.co --db migrations://migrations --- offline: schema from .sql files
crow check app.co --strict                     --- exit 1 on any finding (CI)
```

The url can also live in the crowfile: `db = 'sqlite://dev.db'`.
Findings are advisory by default — they print, teach, and never block.

The checks read the entry and every part it imports, each against its own
lines, so an app that keeps its queries in its parts has them all checked;
a part's finding is named by its file. What a file cannot say alone, the
program says: a connection another part declares, reached as `store.book`,
is checked against its own database. In a program with several databases,
a query whose connection a file cannot tell (a function's parameter) is
left alone rather than checked against the wrong one, and counted in a
note. A handler is one transaction (every write in it lands together or not
at all), so two writes in a handler are not reported as separate.

Before any of it runs, the program passes the checks `crow run` passes it
through — scope, reservation and `ui` rules, made as the program is
lowered — so `crow check` and `crow test` refuse what `crow run` refuses,
with the same message.

## The baseline

`crow check` derives what the program does — every fact, derivation,
handler, view and page in a canonical form — and keeps it under
`.crow/baseline/<entry>/`, one file per declaration, tracked like the
snapshot store. A handler's file is its **signature**: every path
through it as a condition over facts and inputs, the facts it writes and
from what, the effects it would perform. A view's file is its
**conditional tree**: elements, attributes, controls and sub-views under
their guards. Locals are substituted, values from calls are named
`v1, v2, …`, paths are sorted: a rename, a temporary, a reordering or a
helper moved between parts changes nothing; a guard flipped from `lte`
to `lt`, a fact written or not, a button wired elsewhere shows as a diff.

A page's `states` file is its **reachable states**: from the declared
facts, every state the wired handlers can bring the page to, searched
over small, abstract values — each fact a guard reads takes a literal it
is declared with, written with or offered by an input, or `*`, some other
value (a number whose only literal is 0 holds a positive one: zero or a
session), and the signed-in reader is `reader`, which `sign_in` and
`sign_out` write. The file says each control once, with when it is
offered — `button 'Buy' → sim_trading_buy: when sim_session_session_id =
*`, `button '10x' → sim_clock_set_speed[10.0]: when … session_id = *,
except when sim_clock_speed = 10.0` — so a control that becomes
reachable where it was not ("Buy with no session") is its line changing.
A closed modal or drawer and a hidden element offer nothing; a guard the
search cannot judge (a query's cell, a derivation) keeps both ways. The
search visits at most 4096 states. A fact only a handler's conditions
read is tracked too, so the handler takes the way its facts decide; when
that makes more states than the search visits (a page of many fields),
the search runs again with only the facts the page's guards read — a
condition on one left out keeps both ways, so what each state offers
stays right — and the file says so.

Each file's `reach` is what it can touch outside its values: every
effect its functions, derivations and top level reach, through
whatever they call, by capability — `database` (a query, an exec, a
connection), `files`, `network`, `serving` (a page, a route), `shell`,
`mail`, `clock`, `process` (an argument, the environment, an exit) and
`session` (a notice, a sign-in, a download, work held for later):

```
reach board/cards.co
  database: connect, exec, query
  files: cp, mkdir
  session: download, notice
```

A file that touches nothing says `nothing`, so a part that starts
writing files or calling a shell is its reach changing, a change to
accept like any other. `crow check` prints each file's capabilities on
one line (`board/cards.co: database, files, session`). Nothing declares
a reach: it is derived from the program, which already names each
effect it makes (second-half-design.md §1).

A page's `layout` file is its **layout bounds**: each state of the page
(each path of its view) laid out by the engine the harness uses, at 1280
and 390 wide, with its height and every element that runs past the width
— `390: 4733 high, past the width: button 'End session' by 256` — so a
row that stops fitting a phone, or a page that changes shape, is a diff.

A named test that opens the app is a **scenario**: `crow check` runs it
in a *rehearsal* and keeps what it did. In a rehearsal nothing connects —
`connect` hands back a database of the rehearsal's own. A query is
answered from a fixture recorded on purpose, `.crow/fixtures/<hash>.json`
(`{"sql", "columns": [[name, kind]…], "rows": [[…]…]}`, the hash an
FNV-1a of the statement), else from three rows generated from the columns
the database answered at build time (`.crow/schema`, the record every
build keeps): 1, 2, 3 for an int, 1.0, 2.0, 3.0 for a float, `name1`… for
text, true, false, true for a flag — and NULL in the second row of a
column that can hold one (`float?` in the record), so a program that walks
the rows meets the missing cell it has to decide on. A statement that
hands `IN ?` a list runs as `IN (?, ?, …)`, and matches its record and
fixture as the `?` it was written as. `crow check --record-fixtures` runs
the scenarios as *recordings*: a statement that can only read goes to the
real database, what it answers first is kept as its fixture (its first
10,000 rows; the run says when it kept fewer than there were), and a later
read of it is answered from that, as the rehearsals after will be. The
same fixtures run the tests where the servers are not: `crow test
--fixtures` answers each Postgres and ClickHouse read from them, in both
modes (spec, `crow test --fixtures`). A write
is recorded as in any rehearsal, so a recording never changes a database. A write — `exec`, `insert`, a
migration, a statement through `query` that changes rows — is recorded
with its payload and the program sees success; a shell command and a file
written, moved or removed are recorded instead of run; an HTTP `request`
is answered from its fixture where a recording kept one, else as a
request that landed — `200` with nothing to say — and recorded; mail is
recorded as the harness records it, and `fetch` answers nothing, as in
any test. A dependency's functions (`crow_modules/`, a package's) are not probed
with edge cases: a package is held by its own tests, and the app's check
reports the app's. In a recording a request is sent, and what it answered is kept
as its fixture — named by its method, its URL with the query and what it
sends, its headers no part of it — so `crow test --fixtures` answers it
with no server and no model, in both modes (spec, Requests), and the
rehearsals after answer it the same, so a scenario that asks a model
comes out as the recording did; a recording runs the tests that open no
app too, for their fixtures alone, since the replay runs every test. The clock stands at 2026-01-01 00:00 UTC until the
test moves it, and random draws are seeded, so a scenario comes out the
same on every run. An assertion that does not hold is reported, not
failed — the data was the rehearsal's. The scenario's file says how it
ended, what it wrote and mailed, which statements were answered with
generated rows, which assertions did not hold, and every fact it changed,
so a handler that starts writing somewhere new, or leaves a fact
different, is a diff. A test of pure functions leaves no state and is not
a scenario; `crow test` runs it, and every test, against the real data.

The first check records the baseline. Every check after diffs against
it, per declaration, and reports each difference with its cause (the
part's file), its reach (the views and pages it can affect, through the
wiring) and whether the cause is a file git says you changed — a
difference reached from a file you did not touch is the headline. Accept
with `a` in the loop and the baseline moves; `crow check --accept`
accepts every change at once. Without a terminal an unaccepted change
exits non-zero, so CI blocks on it and the accepted baseline travels
with the change in review. New declarations are recorded; removed ones
are reported. Nothing is written and nothing is called: the artifacts
come from the source alone, and the scenarios from rehearsals that
connect to nothing.

`crow book`, run inside the app, is the same material as a book: each
page with its composition tree and wireframes, a chapter per part, the
data wiring and the changes, served on port 3008 and derived afresh on
every start (`app-baseline-design.md` §8).

`crow check --show` prints the app as a tree — each page, the views it
composes under their guards, the inputs they bind, the controls and the
handlers they fire, then the handlers with their writes and effects,
the facts with their binders, writers and readers, and the derivations
with their reads. `crow check --show buy` prints one declaration's
artifact in full, matched by the end of its name.

Four kernel rules read the same wiring, generic and nobody's to
maintain: a `modal`, `drawer` or `toast` whose `open` fact no handler
sets true, or none sets false; a `disabled` fact no handler writes; a
control whose handler writes nothing and performs nothing; a function
that assigns a fact but is wired by nothing and called by nothing. The
design is `app-baseline-design.md`.

## The hazard catalog

| # | Finding | What it means |
|---|---------|---------------|
| 1a | write skew | a query-guarded write whose invariant no constraint enforces — two programs can both pass the check |
| 1b | lost update | read a value, compute, write it back — concurrent updates silently vanish |
| 1c | one-way latch | a column only ever written in one direction, gating work elsewhere — records "it happened once", not "it is true now" |
| 1d | assumed unique | single-row access (`u.name[0]`) filtered by a column the schema lets repeat |
| 1e | unpaired writes | two UPDATE/DELETEs with no `transact` — a crash between them half-applies |
| 1f | verification | unknown table/column (with did-you-mean), placeholder/param count, type mismatch, interpolated-SQL injection risk |

Every concurrency hazard is reported as a two-actor timeline in plain
language, followed by a ranked fix menu — safest first. Constraint fixes rank
above code rewrites because a database rule binds **every** writer, including
programs that are not this one.

## Live-loop checks (no database needed)

Views and event handlers run on the live loop — mined from a production
trading system's bug history:

| Finding | What it means |
|---|---|
| blocking in view/handler | `sleep`/`input`/`shell` reachable (directly or through helpers, path shown) — every connected client freezes while it runs. Do the work before `serve[]`; render-time `query` is fine and deliberately not flagged. |
| unguarded staleness | a value read from outside flows into a decision (directly or through computation) with no freshness check — guard it with `now[] - captured[x]`. This is the propagation the runtime deliberately does not carry. |
| stale snapshot (error) | a top-level variable queried once at startup is read by a view — every render serves launch-time data. The cure is a derivation: `d` rewrites `x = query[…]` into `derive x`, which recomputes when a fact it reads or a table this program writes changes. Re-querying in a handler also clears the finding. |
| remote query in view (error) | a view queries a remote database — a network hop per render, every connected client waiting on it. `d` hoists a single-line `rows = query[pg, …]` above the view as `derive rows[fresh = 60]`; `a` records `--- crow: remote-render` above the view when querying every render is the design. |

The language gaps these point at (background work, freshness, periodicity)
are documented as reviewable proposals in `discovered-features.md`.

## crow check --self — Crow held to itself

`crow check --self` checks the language rather than a program. It writes
its cases from the compiler's own description of Crow, so a builtin, a
kind or an operator added to the compiler is covered the day it is added,
with no test someone has to remember to write:

- every builtin row (`src/table.rs`) called with a value of each kind its
  parameters take — the kinds' values tried until a call lowers — then each
  parameter varied over the catalog's values: the usual one and its edges
  (zero, negative, empty, past 64 bits, text that reads as a number, a
  value that can fail, one that failed) — and every two of its parameters
  varied at once over the kinds' edges (`PAIRED`), where a row's
  assumptions meet (`slice[m, -1, 3]` of a matrix, a matrix and a negative
  start each tried alone, had counted its numbers);
- every collection read and written at every kind of index: a position,
  one from the end, past the end, a key, a mask, a column of bools;
- every operator over every pair of operands — the operators read from
  the AST itself, so one added there is covered with no word here — and
  `in` and `not in`;
- the shapes a value meets: shown, measured, spliced into text, decided
  on, walked, collected, handed to a function and back, held in a record,
  a list and a map, copied, lent, reassigned, assigned two at once, walked
  by a `while` that skips and stops, written as JSON and read back;
- a value carried two steps — through a function, a function value, a
  record, a list, a map, a loop, two variables, a branch;
- a helper called with two kinds in one program (a copy per signature);
- `fmt` by its named options, ints past 64 bits, records that link records
  of their kind, and the databases: SQLite in memory, a scratch Postgres
  cluster (where `initdb` is on the PATH) and a ClickHouse stand-in;
- programs that serve (`src/selfcheck/web.rs`): every response
  (`text`, `html`, `json`, `msgpack`, `arrow`, `bytes`, `respond`,
  `redirect`, `stream` — a unit test holds every name web reserves to
  this list or the list of names that answer no route) with every value
  of the catalog, each case a route's handler asked by GET; and every way
  a handler reads its request — the path's params, the query, the method
  and path, the body as text, as JSON and a field of it, a header, a
  form, a multipart body's files, a status read from the query — and what
  answers no handler's value: a method the route does not name, a path
  nothing answers, a limit passed, a stop;
- views drawn as pages: every element with every value of the catalog as
  its child, and text holding what HTML escapes; the attributes that pass
  through to HTML with text of every kind; a class joined to a
  container's own, written and computed; HTML's boolean attributes true
  and false; and the layout vocabulary's attributes over its words;
- a page's state (the state family): its facts — of every kind — moved by
  its handlers, triggered by a click, a click with the arguments it froze,
  a bound input (a field, a number, a checkbox, a select, a slider); a
  record's field, a list's item and a map's key written in a handler; a
  derivation and a derivation of one; a helper that writes for its
  handler; a handler that stops (nothing lands, and the page says so);
  work handed to `later`; an event the page did not offer; and the page
  opened again — each page driven by its events over its socket, as
  `crow test --served` drives one, the page after each event the answer;
- the modules that speak to a server, against the check's own: a
  WebSocket server that sends back what it is sent (`web.ws`: a text and
  its echo, two in order, an empty one, a wait that ends with nothing, a
  socket closed) and a mail server that takes every message to an address
  (`web.mail`: a message, one beyond ASCII, a body with a lone dot, a
  recipient that is no address);
- random draws (the draws family): each distribution drawn 20,000 times
  from a fixed seed — `randn`, `normal`, `uniform`, `exponential`,
  `poisson`, `bernoulli`, `beta_dist`, the core's `rand` in its three
  forms — its mean, spread, least and greatest held to the distribution's
  (the mean within four standard errors: the seed fixes the draws, so the
  check is never one in a thousand wrong), and `shuffle`, `choice` and
  `permutation` held to what they must be;
- the standard modules' own tests (`src/modules/*.co`), run by `crow test`
  under each interpreted configuration — the walker, no fusion, no moves —
  each run in a directory of its own: every run passes, and all say what
  the interpreted run says (the test-block family; `--only tests`).

Each case is lowered first, in the process, in seconds. A refusal is held
to the rules every message keeps (it names a line, and only what the
program wrote); a panic, an internal error or a verifier finding is a bug
in Crow. The cases that lower run in batches — a program of one function
per case, a marker before each — under five configurations that must
agree:

| Configuration | What differs |
|---|---|
| interpreted | `crow run -i`: the prepared form, typed registers, parallel loops |
| walked | `CROW_WALK=1`: every body walked, every loop in order |
| unfused | `CROW_NO_FUSE=1`: no fused tensor chains or frame masks |
| unmoved | `CROW_NO_MOVES=1`: no moves and no early frees |
| compiled | `crow build`, then the binary |

A batch that serves is served by each configuration on a port of its own
(from a counter below the range the system hands out, each run starting
at a place of its own), and is the server's only when it answers the
run's token at `/self-ready`; each case's requests are asked in turn and
each answer read as `crow test --served` reads one — the status line, the
content type and location, the body (bytes that are not UTF-8 as their
hex), a failure's id read past. A server that stops answering ends the
case it was asking, with what it said, and is started again for the next.

Effects run too, each batch in a directory of its own made anew for every
configuration — a text, a CSV, a JSON document and a directory to read,
the files a case writes its own — with the environment cleared (no key,
no token, no database's address), the draws seeded, the clock standing
still and an empty replay answering every request and every database
server's statement, so nothing reaches the network or a model. The rows
that speak to a server speak to the check's own, on the loopback: an echo
server that answers HTTP (the standard library's own layer — `use sys`,
which the check and its cases may open — its streams, `exchange`,
`fetch`), a ClickHouse stand-in and the scratch Postgres cluster; a case
against one of them runs without the replay, and fails as a finding of
its own (`live`) if it fails against a server the check started — two
modes that cannot reach a server agree. Every finding shows its case's
code beside its id — `op/^/int/bool` is `log[3 ^ true]`: the family, then
what it holds.

**Agreeing is not being right.** A value handed back — through a
function, a record, a list, a loop — must show as it shows alone, and
`tests/self/reference.py` answers what a builtin, an operator, an index,
a write or `fmt` computes from the spec's words, without Crow's runtime:
Python's own arithmetic and text, the system's libm for a number's
functions (Crow's), numpy for math's reductions (the spec holds `sum` to
it), Rust's float display — and for the modules: `bits` by Python's own
operators, `encode`/`decode`'s schemes by `base64` and `urllib`, regular
expressions by `re`, `hash` by `hashlib`, statistics and `math.nn` by
their formulas over numpy (a test's p by scipy), math's tensors —
element-wise forms, scans, sorts, constructors, shapes, linear algebra,
convolution, windows and transforms — by numpy within a tolerance scaled
to the answer, fractions by Python's `fractions`. A route's answer is computed the same way:
the status and its words, the content type a form gives (or `text/plain`,
which a response that names none is served as), the body — JSON as
`encode` writes it, MessagePack packed as the format says, an Arrow
stream read back with pyarrow and held to the frame. A page's markup is
read back with Python's own HTML parser: the tag an element draws as, its
text — entities decoded — what the value shows, an attribute as written,
a boolean one present or absent; a layout attribute by the spec's table —
the words its row names (any other refused), the spacing and type scales'
pixels, a colour's token; and a page's state after each event, by what
its fact holds then. A case answered
otherwise is a finding, `wrong`; one the reference has no answer for is
skipped, never guessed. `CROW_SELF_PYTHON` names the Python to ask (by
default `python3` on the path): one with numpy, scipy and pyarrow
answers every case that needs them.

A disagreement, a crash, a hang, a build the Rust compiler refuses, or an
internal error in any of them is a finding. Every finding is written to
`target/crow_self/findings.txt` with each configuration's answer; the
refusals go to `refusals.txt`, and the rows the check cannot run here —
effects, servers, files, the network — to `unexercised.txt`.

The findings known are listed in `tests/self/known.txt`, and the list only
shrinks: a new finding fails the check, and so does a listed one no run
finds any more (strike it, or `--bless` to write the list again).

Three more lists hold what is added later, so it is tested or the gate
names it — each changed only by `--bless`, which keeps the reasons people
wrote:

| List | Holds | A new entry means |
|---|---|---|
| `tests/self/not-called.txt` | the builtins no case calls, with why | a builtin the cases' kinds and the registry's modules do not reach |
| `tests/self/agreement-only.txt` | the builtins whose cases run and agree, none of which the reference answers, with why — and the responses and request reads likewise (`web.json`, `web.read.query`) | a builtin, a response or a request read with no right answer: give it one in the reference, or say why not |
| `tests/self/forms.txt` | the forms of the language (`src/ast.rs`'s statements, expressions and operators, read from the source) no case reaches | a form with no case |

A builtin the machine cannot reach (no `initdb`, a scratch cluster that
did not start) and a case the reference answers only with a library the
machine lacks (numpy, pyarrow) are noted, never failing.

Every process the check starts is held to a budget: together they may
hold the machine's memory less 9 GB kept for the system (half of a
machine of 18 GB or less; `--memory GB` says another amount), any
one of them a gigabyte, thirty seconds and a megabyte of output. A case
past its share is stopped and is a finding (`memory`, `hang`, `output`):
a program that asks for `range[9223372036854775807]` is one.

Flags: `--only word` (the cases whose ids hold it), `--lower-only` (the
lowering alone), `--bless`, `--clean` (removes the builders' caches,
which otherwise stay for the next run), `--memory GB`.

## crow fix

Without `--hazard`, `crow fix app.co` first mends what the program is refused
for, where the refusal names the fix — a slip, a missing import, another
language's habit (spec, Error Messages) — and lists each change; a program
that is not refused goes on to its hazards.

Each hazard's menu is executable:

```
crow fix app.co --hazard 1 --option 1 [--db url]
```

- **constraint options** write `migrations/NNNN_name.sql`, apply it to the
  `--db` database, and re-run the checks to confirm the hazard is gone
- **fusion options** rewrite the slice into one atomic statement (e.g.
  `SET balance = ?` fed by read-plus-delta becomes `SET balance = balance + ?`)
- **transact options** wrap consecutive writes in a generated fn + `transact`

Every applied fix is self-verifying: the file must still parse and a re-check
must no longer report the hazard. Options that cannot be safely automated
print the exact manual edit instead.

## What crow fix can and cannot do — the three tiers

Every finding falls into one of three tiers, and the tier decides whether a
fix can be applied mechanically:

**Tier 1 — mechanical and unambiguous.** The correct code is derivable from
what's written, and there is no legitimate version of the flagged pattern.
`crow fix` applies these and self-verifies: constraint migrations (1a, 1d),
lost-update fusion (1b), transact wrapping (1e).

**Tier 2 — mechanical but intent-gated.** The edit is derivable, but only
the developer knows whether the pattern is a bug or a design. A stale
snapshot is a bug for live coverage and correct for a static lookup table.
These fixes are offered, never assumed — and the deliberate case gets an
acknowledgment comment. Current members: stale snapshot and remote query in
view, both cured by a derivation (`d = derive` in the check loop rewrites
the exact single-line shapes and spells out the manual edit otherwise), and
blocking in view (`f`, below).

**Tier 3 — design work.** The cure is a restructuring only the developer can
choose — or the language cannot express it yet. The one-way latch's fix is
modeling ("derive from ground truth"). The check teaches; the code is yours.

Blocking-in-view graduated from Tier 3 to Tier 2 when `later[]` shipped:
the interactive check loop offers `f = fix`, which rewrites the bare
`helper[]` call in the handler to `later[helper]` and re-checks. Slow work
scheduled via `later[]` is the sanctioned home for blocking calls and is
never flagged.

### Where data belongs in a live app — the decision matrix these checks enforce

| Data | Blessed home | Checks fencing it |
|---|---|---|
| local (sqlite) state | a derivation over the table — recomputed when this program writes it — or a query inside the view | stale-snapshot pushes in, with `d = derive` |
| remote (network db) state | `derive rows[fresh = N]` — once per window, shared by every session when it reads no session fact | remote-query-in-view pushes out of render, with `d = derive`; stale-snapshot still bars startup-forever |
| on-demand background work | `later[fn]` from a handler; results land in state | blocking-in-view teaches and applies it |
| periodic freshness | `serve[port, refresh = N]` — runtime-owned, skipped-not-stacked | stale-loop-read steers script polls; UI polls need no code at all |

## Accepting a hazard

Some flagged sites are safe by design (a genuinely single-writer batch job).
Acknowledge one — visibly, in the code — with a comment on the line above:

```
--- crow: single-writer
open_trades = query[conn, '...']
```

## Backends and limits

- **sqlite** and **postgres**: full introspection, including partial unique
  indexes (`... WHERE status = 'active'`).
- **clickhouse**: refused honestly — it does not enforce unique constraints,
  so no schema-aware guarantee is possible against it.
- **offline** (`migrations://dir`): the schema is parsed from `CREATE TABLE` /
  `CREATE UNIQUE INDEX` statements — no connection or privileges needed.

The analysis is deliberately conservative: literal SQL strings with `?`
params (the blessed form — interpolated SQL is itself flagged), dataflow
within one scope, and no hazard without demonstrable evidence. A clean report
is not a proof of absence; a finding is always worth reading.
