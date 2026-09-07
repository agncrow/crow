# crow check — schema-aware database checks

`crow check` verifies a program without running it: purity analysis, edge-case
generation, and regression snapshots. With a database URL it also
cross-references the program's SQL against the schema the database actually
enforces, and reports **concurrency hazards** — the class of bug where code is
correct alone but wrong the moment two copies of it run.

```
crow check app.crow --db 'sqlite://dev.db'      --- live introspection
crow check app.crow --db migrations://migrations --- offline: schema from .sql files
crow check app.crow --strict                     --- exit 1 on any finding (CI)
```

The url can also live in the crowfile: `db = 'sqlite://dev.db'`.
Findings are advisory by default — they print, teach, and never block.

Before any of it runs, the program passes the validator `crow run` passes
it through — scope, reservation and `ui` rules — so `crow check` and
`crow test` refuse what `crow run` refuses, with the same message.

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
crow fix app.crow --hazard 1 --option 1 [--db url]
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
