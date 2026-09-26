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
read of it is answered from that, as the rehearsals after will be. A write
is recorded as in any rehearsal, so a recording never changes a database. A write — `exec`, `insert`, a
migration, a statement through `query` that changes rows — is recorded
with its payload and the program sees success; a shell command and a file
written, moved or removed are recorded instead of run; mail is recorded
as the harness records it, and `fetch` answers nothing, as in any test. The clock stands at 2026-01-01 00:00 UTC until the
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

## crow fix

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
