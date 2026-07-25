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
