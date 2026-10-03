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

## Running It for Others

The binary reads a few settings from its environment:

- `CROW_PORT` — the port, over the one `serve` names.
- `CROW_HOST` — `0.0.0.0` to listen beyond this machine; a served app
  listens on this machine only until told otherwise.
- `CROW_SECRET` — at least 32 characters, the key sign-in cookies are
  sealed with. Without it the app writes a key of its own to
  `.crow/keys/`; changing the key signs everyone out.

Beside the binary it keeps `.crow/`: `keys/` (that key, and the sign-ins
signed out), `sessions/` (with `serve[…, keep = true]`) and `uploads/`
(files in flight). The directory must be writable, and `keys/` must outlive
a restart — on a machine whose disk starts fresh each time, set
`CROW_SECRET`.

An app others reach sits behind a proxy, which holds the certificate and
limits connections and slow clients. The proxy hands on the page's socket
and says who asked:

```nginx
location / {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

Each line has a reason. `Upgrade` and `Connection` carry the page's
socket. `Host` keeps the address the reader typed: a page's socket opens
only from the app's own page, so a proxy that rewrites it — nginx does
unless told — leaves every page dim, reconnecting. `X-Forwarded-Proto`
makes the sign-in cookie `Secure` over https, and `X-Forwarded-For` lets a
route's `limit` count each client rather than the proxy (the app believes
it only from a proxy on its own machine).

## How Far One Process Goes

A Crow app is one process: its sessions, its derivations' values and its
tables' versions live in it, and that is a decision, not a limit you
discover later (session-layer-design.md §6). How far one goes is
measured, not guessed. `tests/capacity.rs` serves a compiled app — a
route that queries, a route that answers JSON, a live page — and
measures it:

```bash
cargo test --release --test capacity -- --ignored --nocapture
```

On a laptop (an M-series Mac, over loopback, eight clients), one process:

```text
sessions: 2007 opened in 0.1s (24903 a second), 0 failed, p50 0.29 ms, p99 0.57 ms
memory: 30784 KB before, 39184 KB after: 4.2 KB a session
tabs: 500 sockets opened and held in 2.1s, each sent its render; 142.0 KB a connected session
a route answering JSON (/status): 34418 a second, 0 failed, p50 0.21 ms, p99 0.30 ms
a route that queries (/count): 29972 a second, 0 failed, p50 0.25 ms, p99 0.34 ms
a page load (/): 27260 a second, 0 failed, p50 0.27 ms, p99 0.37 ms
```

Read it as: a tab that is open costs about 140 KB while it is open, so a
gigabyte holds some seven thousand open tabs of a page like this one (a
bigger page holds a bigger render); a page loaded and left costs a few
kilobytes until its grace period ends; and the routes themselves are not
where the time goes — a real route is as fast as its query. The soak test
(`tests/soak.rs`) holds an app at a steady load for as long as you ask
and watches that memory and latency stay flat.

What bounds a process in practice: the file descriptors it may hold (one
per open tab — raise `ulimit -n` on a server), the proxy's own limits on
connections, the memory the open tabs' renders take, and the database
behind the routes. When an app outgrows that, a second process is a
second app: run one per port behind a proxy that sends each tab back to
the process it started on (a sticky route by cookie or address), and let
the database be what they share. A write one process makes shows in
another's derivations when their `fresh = N` window elapses, as an
external source's does — the table's version that wakes a derivation at
once lives in the process that wrote it. Sessions that any process could
resume, and a change that wakes every process, are not built: not before
an app needs them.

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
