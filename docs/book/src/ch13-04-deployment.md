# Polish and Deployment

## One Binary

```bash
crow build dashboard.co
```

The output is a single native executable containing the web server, the
LiveView runtime, the SQLite driver, and your compiled logic. Deployment
is copying one file (plus `trades.db`) to a machine and running it.
There is no interpreter to install, no `node_modules`, no Python
environment — the binary is the app.

## Growing It

Every direction from here is a chapter you've already read:

- **More data** — the seed script becomes a real ingest job; `poll` a
  price feed into reactive state for tick updates
- **More pages** — `page['/positions', positions_view]`; components
  shared through a module (`use widgets as w`)
- **A real database** — change one connection string to `postgres://…`
- **Auth, sessions, JSON APIs** — `use web.auth`, `route['/api/pnl', pnl_json]`

## What You've Built

A data application with live UI, in one language, one file, one binary —
parallel where it helps, sequential where it matters, with failures that
carry their reasons all the way to the pixel. That's Crow's whole bet.

The reference (`docs/language-spec.md`) covers what this book didn't;
the Design Notes there explain every "why can't I…" you'll hit next.
Welcome aboard.
