# Live apps: the session layer and what comes after

*Design and plan. Drafted 2026-09-22 after measuring why finch's
controls lagged and auditing the web runtime; revised 2026-09-23 after
review (a second model's critique, the own-writes hole it led to, and a
comparison with LiveView, Blazor, Shiny, Livewire, Next.js and
Streamlit). On 2026-09-23 Vijay asked for the full plan and for each
phase to be built in order; the decisions in §10 are taken as
recommended. Written in the spec's voice so each section moves into
`language-spec.md` as its phase lands. It builds `core-design.md` (a
handler is a transaction; a handler that meets an error stops and
reports; one moment per event) into the runtime both backends share
(`ir-design.md` §7).*

The promise, and the test of it:

> **A click's own effect is on the page after one render; whatever is
> slow catches up; nothing one reader does stalls another; other
> readers' changes arrive on their own; a reader's session outlives
> their connection.** finch's program, unchanged, keeps that promise.

## 1. What the measurements said

Measured on finch's simulation page (debug build, as `crow run` makes
it), with `serve[…, budget = 1]` logging every render and handler, a
socket probe timing each click to the first render showing its effect,
Chrome for the browser side, and cut-down variants of the page.

| | render | click → effect | 20-step slider drag |
|---|---|---|---|
| finch as it was | 640 ms | ~1 s (0.7–1.6 s) | 13 s behind |
| without the two per-tick ClickHouse derivations | 8 ms | 16–37 ms | instant |
| without the chart too | 3 ms | 2–9 ms | instant |

Handlers took under 1 ms. The browser applied a 470 KB render in about
11 ms. **The whole delay was renders waiting on derivations that query**
(`tape.so_far` ≈ 250 ms and `scan.rows` ≈ 300 ms, each paying ClickHouse's
part-pruning floor on a table of 143,575 parts), and every click being a
render. The audit that followed found the rest of the family:

- **A slow handler freezes its page.** During a 2-second handler the page
  received nothing, not even the refresh tick; a click made meanwhile
  waited 1.7 s. A `busy = true … busy = false` flag is never rendered in
  between, so a button can never show that it is working (finch's
  `starting` is dead in effect).
- **One reader's slow work stalls everyone.** Session A's 2 ms click took
  6.9 s while session B's 7 s handler ran; new visitors could not open
  the page during a 15-second render. The database drivers block the
  async runtime's threads, ClickHouse queries share one connection for
  the whole app, and SQLite has one locked connection.
- **Every render and handler reads every fact back from JSON.** The
  compiled program keeps facts as JSON between events, and the preamble
  of every render and handler parses all of them (about 30 in finch).
- **A session dies with its socket.** A reconnect (a rebuild, a laptop
  sleep, a network drop) starts a fresh session; the running replay is
  gone.
- **The harness and the server disagree about who owns a fact.** In
  `crow test`, two `app[]` sessions share their facts (the book's
  `two_people` example asserts "Bob's reset is Alice's reset"). In the
  served program each socket has its own facts. The spec sides with the
  server: a derivation "reads a fact a session writes"; derive-design
  §3.5: "live state is per connected session".
- **Other readers' writes arrive only on a tick.** A table write bumps
  the table's version for every session, but no session is told; Bob
  sees Alice's order at his next refresh tick, or never without one.
- **Every failure, confirmation and pending state is hand-rolled.** finch
  keeps `status`, `noticed_at`, a `notice_open` derivation and a toast to
  report outcomes, and a fact, three handlers and a modal per
  confirmation.

Fixed on 2026-09-22, before this plan (spec: *A page and its session*,
*A page sends only what it was offered*): the socket opens from the
app's own page only; a session accepts only the events its renders
offered; a socket is sent its session's render at once; the page load
renders once and the socket adopts it; the page says *reconnecting…*
and takes no input while down; a click's render is not a tick; queued
input is answered by one render; dev builds optimize the runtime
crates; a served app listens on this machine unless `CROW_HOST` says
otherwise; the book's runner answers only its own page. Measured again:
while paused, the speed buttons answer in 15 ms (median) and a 20-step
slider drag lands 13 ms after the last movement, in one render. Every
click whose render needs a recomputed query still waits for it — Play
(534 ms median), Seek (546 ms), Restart (885 ms), Jump (1.0 s), and any
click while playing (about 540 ms), since the moving clock moves the
play-head. That remainder, and everything in the list above, is this
plan.

## 2. Principles

1. **Nothing on the page waits for the network** — except the click's
   own writes. A render reads the last value of anything slow; the slow
   thing catches up and the page updates when it lands.
2. **The loop never blocks** (spec, *The Loop Never Blocks*) — now true
   of derivations and handlers, not only of `later`. One session's work
   never stalls another's.
3. **A session is the reader's.** Its facts are its own; it survives a
   dropped connection; tables are what sessions share, and a write to a
   table reaches every reader of it.
4. **The runtime owns what every app would hand-roll:** pending state,
   failure reports, confirmations, the URL, staleness.
5. **One kernel, both modes** (design rule 12). The session layer lives
   in `crow_runtime`; the compiled program and the IR interpreter drive
   the same code, and the harness is a client of it.

## 3. The model

### 3.1 A session is the reader's

Facts are per session: each reader's page has its own copy, as the
served program does today. Tables — the program's databases — are what
sessions share, and a derivation that reads only tables, constants and
the clock is shared (computed once for the app), as today.

The harness follows the server. Each `app[view]` opens a session with
its own facts, starting from their declared values. `click`, `change`,
`html` and the other verbs act on the session they are given. A bare
fact name in a test block reads the session the test acted on last, so
a one-session test reads as it does today; a test with two sessions
reads each through its session (`alice.min_qty`, the way the book
already reads a session's fields). The book's `two_people` example is
rewritten: Bob's reset is his own, and what the example shows as shared
becomes a table write — a row Bob inserts is Alice's too.

### 3.2 Events are applied one at a time, as transactions

`core-design.md` §1–§2, adopted as written. Every event that can write
facts — a click, a bound input's change, a `later` schedule landing — is
applied one at a time, in arrival order. A handler runs on a working
copy of the session's facts, sees its own writes, and commits them as
one at its end; effects whose answer it uses run when asked, the rest
at commit; the first write through a connection begins a database
transaction on it, committed at the end; a stop commits nothing and
reports (§3.9).

A bound input that arrives while a handler runs waits its turn; the
browser keeps showing what the reader typed. There is no rule for two
writers of one fact, because there are never two.

Facts live in the session as values. The working copy replaces the JSON
round trip: a render reads the committed facts in place, and a handler
copies what it writes, not the whole state.

### 3.3 Handlers run off the loop

A handler runs on a worker thread, not on the loop that owns the
session's socket. While it runs the loop keeps going: refresh ticks
render, landed derivations render, the page stays live — from committed
facts, so a render never sees a handler halfway.

- **Pending is automatic.** `running[f]` — today true while a `later`
  schedule of `f` is in flight — is also true while `f` runs as a
  handler, and every element that offers `f` renders `disabled` with
  `aria-busy="true"`, so a button shows it is working with no flag in
  the program.
- **One handler at a time per session.** Other events wait behind it,
  in order. A second click on the handler that is running is dropped
  (the double-click), as `later` already skips a schedule in flight.
- **`later[f]` is the one way to say "other clicks need not wait".** A
  handler holds the session's queue until it ends; a scheduled function
  does not. (Design Note: *Handlers and later*.)

### 3.4 Derivations that reach out run off the loop

A derivation whose read set includes a table or an external source — one
that queries — is computed off the loop. A render reads its latest
value; when it is stale the runtime starts the recompute and the render
goes ahead with what it has. Staleness has a cause, and the cause
decides what the render waits for:

- **The click's own writes are on the page.** A derivation that reads a
  table the current event wrote is recomputed before that event's
  render: after Buy, the blotter shows the trade in the same render as
  the notice that says so. (Today this holds because every render
  computes everything; the rule keeps it once renders stop waiting.)
- **Everything else catches up.** A derivation stale because time moved,
  because a fact it reads changed, or because another session wrote its
  table shows its last value; the recompute runs on a worker and the
  page renders again when it lands.

The rest of the rules:

- **Before its first value** a reaching-out derivation reads as an error
  value of kind `'pending'` (reason `loading`). Views already render a
  failed derivation in place (`if not bars.ok` / `p['no tape: …']`), so
  a page with one slow part is still a page from its first render.
- **Latest inputs win.** At most one recompute of a derivation is in
  flight per holder; if its inputs move before it lands, its value is
  dropped and one more recompute runs with the latest inputs. A drag of
  `d` seconds over a query of `q` seconds costs about `d / q + 1`
  queries, the last landing one query after the drag ends.
- **Shared derivations compute once.** One recompute app-wide
  (single-flight); every session waiting on it keeps the last value
  meanwhile. (Today the cache is checked under its lock and computed
  outside it, so sessions that find a shared value stale at the same
  moment each run the query.)
- **A recompute nobody will read is stopped** on the server (ClickHouse,
  Postgres): one whose session has gone, one whose facts or tables moved
  while it ran — it is replaced by one from the latest inputs; a clock
  tick alone replaces nothing, so a query slower than the tick still
  lands — and everything in flight when a served program is told to stop.
  A program killed outright stops nothing; the next run that connects to
  a ClickHouse server on the same machine stops what the dead run left.
- **Readers can see it.** `running[x]` is true while a recompute of `x`
  is in flight; `captured[x]` is the moment it last ran, as today.
- **Staleness is visible by default.** The root element of a view that
  read a derivation still recomputing carries `data-crow-stale`; the
  default stylesheet dims it slightly after 300 ms, so a fast recompute
  never flashes. A view that wants its own treatment reads `running[x]`.
- **Handlers read fresh.** A handler is already off the loop, so a
  stale derivation it reads is computed before the read: `r = trades`
  after `exec[book, 'INSERT …']` sees the insert. The handler waits; the
  page does not.
- **Pure derivations stay in the render.** A derivation over facts and
  the clock alone (`head`, `clock_str`) costs microseconds and is
  computed where it is read, as today.

On finch this should bring every control to what the page measured with
its two queries removed (16–37 ms in a debug build), with the scanner
and the readouts landing one query later (0.3–0.6 s).

### 3.5 A write reaches every reader

A table write marks every derivation that reads the table, in every
session, and wakes the sessions that hold one: each recomputes in the
background (a shared derivation once) and renders when the value lands.
Bob sees Alice's order without a refresh tick and without a line of
publish/subscribe code: the read sets are already known.

Invalidation is by table. A write to `trades` makes every session's
blotter over `trades` recompute, even one filtered to another session's
id; that is right and cheap at tens of sessions and wasteful at
thousands. Invalidation by key — the write's key columns against the
derivation's `WHERE` — is later work (§9, after phase I).

Writes made outside the program (another service, a cron job, a person
at a SQL prompt) are seen only through time: a derivation over a table
the program never writes recomputes per refresh tick, or per its
`fresh = n` window, as today.

### 3.6 One moment per event

`core-design.md` §3, adopted. `now[]` is the moment the event began; a
refresh tick is an event; the harness sets the moment with
`change[a, 'now', t]`, so finch's clock tests stop comparing against the
wall clock.

### 3.7 Nothing blocks the runtime's threads

Session work — handlers, renders, derivation recomputes — runs on
worker threads, never on the async runtime's I/O threads, so a slow
query ties up one worker and the sockets keep moving. Database handles
become pools, one per server however many parts connect to it:
ClickHouse keeps up to four connections instead of one, Postgres is
pooled already, and SQLite stays one connection (its queries are local
and short). Four, not ten, because ClickHouse already runs one query on
every core of the server: on finch's 143,575-part table two warm queries
at once each took under a second, while four identical cold ones took
35 s. The pool is the app's cap on queries in flight; per session there
is one handler and one recompute per derivation at a time.

### 3.8 A session outlives its connection

The page's token becomes the session's id. The page keeps it for the tab
(`sessionStorage`) and presents it when it reconnects; the server keeps
a session for ten minutes after its socket closes and resumes it on
reconnect: the same facts, the replay still where it was.

- **A waiting session keeps its facts and its derivations' last
  values.** Dropping the values would save memory but make every resumed
  page flash "loading" while its queries run again; the wait is bounded
  instead — at most a thousand sessions wait at once, the soonest to
  expire giving way.
- **The token is a credential.** It is sent only over the socket, used
  once, and replaced on every resume, so a copied token resumes nothing
  after its tab has.
- **Across a restart only when asked.** `serve[…, keep = true]` writes
  each session's committed facts to `.crow/sessions/`, under the tab's
  current token, as they change; a tab that comes back to a restarted
  program finds its file (only its own token opens it) within the same
  ten minutes; a fact whose name or type changed since starts from its
  declared value, and the log names it. Without `keep`, a restart starts
  every session fresh.

### 3.9 The runtime reports, confirms, and remembers the view

- **The notice** (`core-design.md` §1.4): `notice[text]` is an effect
  held to commit; a stop's reason is delivered the same way; `notice` is
  a runtime-provided derivation with `text` and `at`. A page that reads
  no `notice` gets the runtime's toast. finch's `say`, `status`,
  `noticed_at`, `notice_open` and toast go.
- **Confirmation**: `button['End session', click = end_session, confirm
  = 'End this session?']`; on a table, a map from action to text,
  `actions = map[Delete = delete_session], confirm = map[Delete =
  'Delete this session? Its trades go with it.']`. The runtime renders
  the dialog as part of the session's page and offers the click only
  from the dialog's own button, so the server enforces it through the
  offers. The text is the same for every row.
- **The URL**: `page['/', sim.render, url = ['session.day',
  'session.sym']]` reads those facts from the query string when a
  session starts and writes them back when they change
  (`history.replaceState`), so a reload or a shared link opens the same
  view.

### 3.10 What the page receives

- **Patches, not pages.** A render sends the difference: the session
  layer diffs the tree (elements matched by `id`, then by position, as
  the client's morph does) and sends patches the client applies.
- **Charts in layers.** A chart draws in layers with ids — the candles,
  which stay put while the day does; the panes; the marker, which moves
  — so a moving marker is a one-attribute patch instead of a 400 KB
  drawing, and the client stops swapping the SVG whole. finch while
  playing sends 428 KB a render today; with patches, a few KB.
- **An instant busy mark.** The client marks the element it sent a click
  from `data-crow-busy` at once, before the server answers; every
  event carries a sequence number and every render the last number it
  applied, so the mark clears when the answer arrives, not at the next
  tick. Over the internet the reader sees their click taken in the same
  frame, with no program code in the browser.

### 3.11 Both modes serve; a test sees a settled page

The session layer is one kernel in `crow_runtime`. The compiled program
drives it with generated handlers, views and derivations; the IR
interpreter drives it by walking them. So `crow run -i` serves pages —
the refusal added on 2026-09-22 goes — and a page's edit-to-screen loop
no longer waits for a Rust compile. Because the compiled program drives
the same layer, `crow test` can also run a page's test blocks against
the compiled program, so the page a test checks is the page that ships.

In the harness, time and scheduling are the test's. `click[s, h]` runs
the handler, commits, runs every recompute the commit made stale, and
returns when they have landed, so the next line reads the page a reader
would see once the data is in — as `later[]` already runs inline in
tests. Every test written today keeps its meaning. The pending render
and a disabled button while its handler runs are checked by the layer's
own tests. `now[]` inside the harness is the moment the test sets with
`change[a, 'now', t]`, or the wall clock at the event when it sets none.

## 4. What apps need next

The session layer changes how the runtime behaves, not what a program
can express. Compared with LiveView, Blazor, Shiny and Livewire, four
capabilities decide whether Crow sustains more than data tools; each is
sketched here and settled in its phase's first step.

### 4.1 Who the reader is

A live page has no notion of a signed-in user today (`web.auth` signs
JWTs, `web.session` is an in-memory store, neither reaches a page), so
handlers cannot authorize. Sketch: one read-only fact per session,
`reader`, a record (empty when signed out) that the runtime fills from
the signed cookie of the request that opened the page. A handler signs
in with `sign_in[record]` and out with `sign_out[]`, effects held to
commit that set or clear the cookie and the fact. No input can bind
`reader` and no event can write it, so the page cannot forge it;
authorization is ordinary code in handlers (`if reader.role ne 'admin':
return`), and a derivation that reads `reader` is per session.

### 4.2 Files in and out

Live pages take no files and give none. Sketch: `field['Attachment',
attachment, type = 'file']` binds a file record (`name`, `type`,
`size`, `path` to a server-side copy) — the one input kind, not a new
element; `download[value, 'trades.csv']` is an effect held to commit
that sends a frame (as CSV), text or a file's bytes to the reader's
browser. A file larger than the app's limit is refused with the limit
named.

### 4.3 Big collections

A table renders every row today. Sketch: `table[frame, page = 50]`
shows fifty rows with page controls, the frame staying on the server;
a table over 1,000 rows without `page` is refused with an error that
names `page` — the chart's point-budget rule, never a silent cut.

### 4.4 One way to embed a client widget

The element vocabulary is closed, which keeps Crow simple, but it means
every map, rich editor or calendar needs a language change; every
framework compared has a sanctioned escape hatch. Sketch: `widget['map',
props = map[…], on = map[pick = choose]]` names a JavaScript module in
the project's `widgets/` folder that exports `mount(el, props, send)`
and `update(props)`. The runtime loads it once, passes `props` as JSON
on every render that changed them, and routes what it sends through the
offers like any click: `choose` receives the value. One element, one
module shape; no other client code.

### 4.5 A second reference app

Every decision so far was shaped by finch, which is single-user,
read-heavy and runs on one laptop. A second app of the opposite shape —
a team task board: sign-in, boards, cards with comments and attachments,
live updates across readers, an audit log — is built from the
vocabulary alone, with its own harness tests, and measured the way
finch was. It is the acceptance test for §3.5 and §4.1–§4.4.

## 5. finch after

Unchanged, finch gets: controls that answer in tens of milliseconds;
hops that no longer freeze the page (their buttons show that they are
working); a replay that survives a laptop sleep; the scanner catching up
one query behind the clock; stale panes dimmed while they catch up.

What finch can then delete, each a separate edit, none required:

| today | after |
|---|---|
| `say[msg]`, `status`, `noticed_at`, `notice_open`, the toast view | `notice['…']`; stop reports arrive on their own |
| `if not r.ok: say['… failed: …']; return` after each query | nothing — the next decision on `r` stops the handler and reports |
| `ending`, `ask_end`, `cancel_end`, the End modal | `button['End session', click = end_session, confirm = 'End this session?']` |
| `confirm_id`, `ask_delete`, `cancel_delete`, the Delete modal | `actions = map[Delete = delete_session], confirm = map[Delete = '…']` |
| `starting = true … starting = false` | nothing — the Start button is disabled while `start_session` runs |

## 6. What stays out

- **async/await, promises, futures in user code.** Function colours
  infect every caller (spec, *The Loop Never Blocks*); the runtime does
  the scheduling, the program stays straight-line.
- **Program code in the browser.** A second execution model is a second
  language. The widget module (§4.4) is the one door, and it carries no
  program logic; the tick is the clock.
- **Offline.** A page needs its server. Resume (§3.8) covers a dropped
  connection, not an absent one.
- **More than one server process.** Sessions, shared derivations and
  table versions live in one process. An app that outgrows one process
  would need sessions any process can resume from a shared store — the
  LiveView shape — and cross-process invalidation; not before an app
  needs it.
- **Concurrent handlers within one session.** One at a time; merging
  concurrent working copies is a database's job, not a page's.
- **Presence** (who else is looking) waits for the reference app to ask
  for it.

## 7. Design Notes

### Pages Do Not Wait for Their Data

**What was considered:** keeping derivations synchronous (today —
measured: a click costs the slowest query on the page); making every
derivation asynchronous (rejected: a derivation over facts costs
microseconds, and deferring it only adds a render); a render budget that
refuses a slow render (the deferred item in *The Loop Never Blocks* —
superseded: renders cannot be slow once queries are off the loop);
opt-in background work as LiveView (`assign_async`), Shiny
(`ExtendedTask`) and Dash (`background = True`) have it (rejected:
discipline, which the spec's own note says fails everywhere).

**Why this design:** only derivations that reach out are slow, and they
are known from the read set the front end already computes. Showing the
last value is what every dashboard does by hand; here it is the default.

### A Click's Own Writes Are on the Page

**What was considered:** stale-while-revalidate for every derivation
(rejected: after Buy the notice says "bought" while the blotter shows
the old rows — the page contradicts itself about the reader's own act);
waiting for every stale derivation (today: every click costs the
slowest query).

**Why this design:** staleness has a cause. What the reader just did is
part of the answer to their click; what time or other readers did is
news that may arrive a moment later. Next.js server actions make the
same cut: the action's revalidated data returns in the same round trip.

### Handlers and later

**What was considered:** dropping `later` once handlers run off the loop
(rejected: a thirty-second import would hold every other click of the
session); a second kind of handler (rejected: two ways to handle an
event).

**Why this design:** a programmer never chooses where a handler runs;
the runtime does, invisibly. The only choice left is whether the
session's other clicks should wait for this work, and `later[f]` is the
one way to say they should not.

### The First Render Shows What It Has

**What was considered:** blocking a session's first render until every
derivation has a value (today's behaviour: 8–34 s from Start to finch's
first session render, measured 2026-09-22 — the Start handler itself is
one SQLite insert).

**Why this design:** a failed derivation already renders in place; a
pending one is the same shape with kind `'pending'`. The page appears at
once and fills in.

### A Session Belongs to Its Reader

**What was considered:** shared facts, as the harness has them (every
tab sees every other tab's clicks); per-browser sessions through a
cookie.

**Why this design:** the served program, finch, and the spec's own words
are per session; a trading replay in two tabs is two replays. Shared
state is what tables are for, with transactions and invariants the
database keeps, and §3.5 makes a table write reach every reader. Per
tab rather than per browser because a second tab is how a reader
compares two views.

### Sessions Survive a Restart Only When Asked

**What was considered:** keeping sessions across restarts by default in
dev builds, so an edit never loses the replay.

**Why this design:** restoring saved facts into changed code needs
migrations — Lamdera, which keeps Elm backend state across deploys,
makes the programmer write typed ones — and a long-lived resumable
session makes its token a long-lived credential. Resume within one run
covers the dropped connection; `keep = true` is the explicit choice for
the rest, restoring only facts whose name and type are unchanged.

## 8. Spec, book and code changes

- Spec: *Derivations* (off-the-loop recompute, own writes, pending,
  latest wins, single-flight, cancellation, staleness, `running[x]`,
  wake on write); *Handlers and their helpers* (transactions, serial
  events, off the loop, pending); §10 *Errors in Conditions* (stops,
  facts never hold an error); §15 `later` (the Design Note); *The Loop
  Never Blocks* (deferred items delivered); *A page and its session*
  (resume, `keep`); `page[…, url = …]`; `confirm =`; the notice;
  `reader`, `sign_in`, `sign_out`; `field[…, type = 'file']`,
  `download`; `table[…, page = n]`; `widget`; §16's harness (sessions,
  settled events, `change[a, 'now', t]`, compiled runs); `crow run -i`
  serves.
- Book: ch12-02 (sessions are per reader; resume; pending; staleness),
  ch12-04 (slow work is the runtime's), ch12-05 (the two-users example;
  time in tests), ch12-08 (derivations off the loop), a new chapter for
  sign-in, files and widgets.
- Code: `crow_runtime::session` (facts store, event queue, working copy,
  commit, held effects, notices, offers, derivation scheduler, pools);
  `crow_web::live` becomes its transport; the transpiler's
  `flush_before` / `reload_after` / `refresh_derivations` machinery is
  replaced by the working copy; the IR interpreter's harness moves onto
  the layer.

## 9. The plan, in order

Each phase lands only when every gate is green (`cargo test
--workspace`, `cargo test -p crow_web --features live,http`, parity,
docs, book, layout), finch runs unchanged, and its target is measured.
Each follows the workflow in `design-rules.md`: spec text first, then
code in both modes, a parity program, the book, and the spec reconciled
with what shipped.

**Where the core rewrite's step 4 goes.** `ir-design.md` put the session
layer after step 4 (the Rust backend from the IR). This plan builds it
first: finch needs it now, and what each phase changes in the
transpiler is kept to calls into `crow_runtime::session`, so step 4's
emitter ports calls, not semantics. Step 4 follows phase I.

| phase | what | target |
|---|---|---|
| **0. One session, one set of facts** | the harness gives each `app[view]` its own facts (§3.1); bare fact names read the session last acted on; the book's `two_people` rewritten | harness tests where two sessions' facts differ and a table write is shared; every existing harness test unchanged |
| **A. Pages never wait** | worker threads off the I/O threads; database pools; reaching-out derivations off the render with the own-writes rule, pending, latest-wins, single-flight and cancellation; wake on write (§3.5); `running[x]`; `data-crow-stale`; the instant busy mark | every finch control ≤ 50 ms median (debug build); a 7 s handler in one session leaves another's clicks ≤ 50 ms; four sessions arriving together run a shared query once; Bob's page shows Alice's insert within one query, no refresh tick |
| **B. Handlers are transactions** | `core-design.md` §1–§3: working copy and commit, held effects and one database transaction per connection, stops and the notice, facts never hold an error, one moment per event; handlers off the loop with automatic pending | the busy-flag probe shows the button disabled while its handler runs; core-design's parity programs; finch's skip case lands nothing on a failed query |
| **C. Sessions outlive connections** | resume by token with a grace period, caches dropped while waiting, token rotation; `serve[…, keep = true]`; `page[…, url = …]`; `confirm =` | the restart experiment resumes the replay with `keep`; a URL reopens a view; the offer test refuses an unconfirmed confirm |
| **D. Both modes serve** | the harness on the kernel (settled events, `change[a, 'now', t]`); test blocks against the compiled program; `crow run -i` serves; a served-session parity harness | parity and book green with a served-page corpus; finch's test blocks pass in both modes |
| **E. Patches** | diffs instead of pages; charts in layers | ≤ 10 KB a second to a playing finch page |
| **F. Who the reader is** | `reader`, `sign_in`, `sign_out` (§4.1) | a page that authorizes a handler by role; a forged event cannot change `reader` |
| **G. Files and big collections** | `field[…, type = 'file']`, `download`; `table[…, page = n]` and the row budget (§4.2–§4.3) | upload and download round-trip in Chrome; a 100,000-row frame pages without sending more than a page |
| **H. Client widgets** | `widget` (§4.4) | a map or editor widget embedded, its events through the offers |
| **I. The second reference app** | the team task board (§4.5) | built from the vocabulary alone; its harness tests green in both modes; measured like finch |

Later, in this order: invalidation by key (§3.5), step 4's emitter,
presence if the reference app asks for it, durable jobs.

Estimates, honest, in working weeks: 0 under one, A two, B two to three,
C one to two, D two, E one, F one, G one to two, H one, I one.

## 10. Decisions

Taken as recommended on 2026-09-23 (Vijay: build the plan in order).
Each can be reopened; the phase that depends on it says so.

1. **Facts are per session** in both modes; the harness changes and the
   book's two-users example is rewritten (phase 0).
2. **Events are applied one at a time** per session, bound inputs
   included; a repeat of the running handler is dropped (phase B).
3. **A derivation's first value renders as pending**, not a blocked
   first render (phase A).
4. **Resume within a run** for ten minutes; **across restarts only with
   `serve[…, keep = true]`**; the token is single-use and rotated
   (phase C).
5. **A session per tab** (phase C).
6. **`page[…, url = ['fact', …]]`** (phase C).
7. **`confirm = 'text'`** on any clickable element, a map on a table
   (phase C).
8. **The runtime's toast** for notices on pages that read none
   (phase B).
9. **Harness events settle** before the next line runs (phases 0–D).
10. **The click's own writes are on the page; everything else catches
    up** (phase A).
11. **A table write wakes every reader**, invalidating by table first
    (phase A).
12. **Staleness is visible by default** through `data-crow-stale`
    (phase A).
13. **The session layer comes before step 4** of the core rewrite.

## 11. Build log

**Phase 0 — built 2026-09-23.** `app[view]` sessions hold their own
facts in the harness (`Value::App` carries its session; the interpreter
swaps a session's facts in when a verb acts on it). The book's
`two_people` example and ch12-02's "every open tab shows the new value"
were wrong the same way and were rewritten.

**Phase A — built 2026-09-23.** On finch, unchanged, debug build, warm
ClickHouse: Play, Pause, the speed buttons, Jump, Restart and Seek answer
in 7 ms median (534 ms – 1.0 s before); while one tab's hop ran for
3.6 s another tab's clicks answered in 1 ms; four tabs opened together
ran one shared query; a session started in one tab reached another tab's
history 3 ms after the click. What building it found:

- *Queries nobody waits for pile up.* A killed server's queries kept
  running on ClickHouse for a minute, and each restart added its own on
  top; on a table of many parts concurrent cold queries stall each other
  (four identical ones: 35 s each). Hence the stopping rules in §3.4 and
  the pool of four in §3.7. ClickHouse marks a stopped query at once but
  ends it only at its next check, which on that table can be seconds.
- *A write made through `query` was not a write.* `query[book, 'INSERT …
  RETURNING id']` wrote its table without the program knowing, so
  nothing over `sessions` ever recomputed after a Start. Fixed for both
  the cache and the wake (spec, *Derivations* rule 3).
- *The spec promised a ClickHouse pool of ten; the driver kept one
  connection.* Now four, with the reason in the spec.
- Cold ClickHouse still dominates a first session: a column's marks over
  143,575 parts take 10–15 s to load. The page no longer waits for it —
  the tape reads "loading" meanwhile.

**Phase B — built 2026-09-23.** Both modes: one moment per event
(`crow_runtime::moments::at`; the harness clock is the test's, moved by
`change[a, 'now', t]`); a handler runs on a working copy that commits
the facts it changed, as one, or nothing when it stops; a stop — an
error at a decision, at a fact, or in an effect's arguments, parameter
lists included — becomes the session's notice (`h stopped: reason`);
`notice[text]` and `later` are held to the commit; one database
transaction per connection per handler (`crow_runtime::txn` hooks,
`crow_db` scope: SQLite a connection of its own with `BEGIN IMMEDIATE`,
`:memory:` its one connection, Postgres a pooled one; ClickHouse keeps
its writes); a page that reads no notice gets the runtime's toast, in
the harness too. Served only: a handler runs off the page's loop — the
page keeps rendering from committed facts, every element offering the
running handler renders `disabled` and `aria-busy`, events queue one at
a time, a second click on the running handler is dropped, and a handler
that ends within 100 ms is answered by one render. On finch, unchanged:
controls 1 ms paused and 6 ms playing (release build), the three phase-A
checks still hold, every harness test passes. What building it found:

- *A helper's stop did not stop its handler* in either mode (the
  compiled helper wrote its facts and returned; the interpreter handed
  the error back). Both now stop the whole click.
- *An error in a statement's parameter list* reached the database as a
  serialized value in compiled code and was refused there; now it stops
  the handler in both modes.
- *Not built:* facts still round-trip through JSON (the working copy is
  a clone of the JSON store), `mail` has no builtin to hold, and a
  handler's own derivation over a table it wrote is recomputed from its
  uncommitted transaction only when it reads through the same handle.
  Comparing a bare `error[…]` value with a number does not compile (its
  value has no type) — a pre-existing gap, noted, not fixed.

**Phase C — built 2026-09-23.** A session outlives its socket: parked
for ten minutes under a token the tab keeps in `sessionStorage`,
replaced at every connect and good once; the page sends the tab's token
and its own, and a resumed session is sent its render first; a handler
running when the socket closes commits before the session is parked.
`serve[…, keep = true]` writes each session's facts to
`.crow/sessions/<token>.json` as they change (the directory ignores
itself in git); a tab coming back to a restarted program finds its file
within the same ten minutes, and facts whose shape changed start from
their declared values. `page[…, url = ['day']]` starts a session from
the query string and the address follows (`history.replaceState`).
`confirm = 'text'` on any clickable element, and `confirm = map[…]` on a
table's actions, make the element offer `__ask`; the runtime draws the
dialog and only its Confirm button reaches the handler — a forged click
is refused. Found on the way: *the interpreter's harness rendered
buttons without their `data-crow-click` wiring* (the compiled page had
it), so a test could not see what a button calls; both modes now render
the same wiring through `crow_runtime::layout::confirm_wiring`. A
waiting session keeps its derivations' values (not dropped, as first
planned): dropping them made a resumed page flash "loading".

**Phase D — built 2026-09-23, one part deferred.** `crow run -i` serves:
the program's main runs in the interpreter, recording what `page` and
`serve` ask for, and the live runtime serves it with the interpreter
rendering and handling on a copy of that interpreter holding the
session's facts (`src/served.rs`); resources — database handles,
functions — stay the template's, never copied into a session. `crow test
--served` drives every page in `tests/served/` in both modes with the
same scripted events (`name.events`) and compares the canonical pages
each settles on after every step; four pages, 18 steps, identical, in CI.
Found on the way: a program whose file name starts with a digit could
not be compiled outside a crowfile's folder (the crate took its name);
the interpreter's pages were wrapped in an extra element when the
runtime added a toast or a dialog, and showed the notice twice. The
harness keeps its settled, synchronous meaning (phase B gave it
sessions, the clock and transactions). *Deferred:* running test blocks
against the compiled program. It needs the harness verbs in Rust and the
layout engine (written in Crow) compiled with the test; the served
parity suite covers the risk it was for — a compiled handler that
disagrees with the interpreter — for every page in the corpus, and the
full compiled harness stays on the list after phase I.

**Phase E — built 2026-09-23.** A render goes to the page as the changes
from the render the page holds (`CrowVNode::changes`; the page's
`applyChange`), or whole when that is smaller. To address a change, the
server must hold the page *as the browser built it*, so every render is
written out and read back by a reader that follows the browser's parser
(`parse_markup`, `as_built`): text runs merged, void elements childless,
a row given its `tbody`, `<x/>` closed only in SVG, a script's text raw,
entities in text left for the browser. Checked against headless Chrome
(`the_browser_agrees`, ignored in CI — it needs Chrome) on 68,000
generated pages. The browser *repairs* malformed markup with an
algorithm this reader does not copy (misnested formatting tags,
content outside a table's cells; Chrome's new `select` and
processing-instruction parsing); the reader notes the first repair
instead, and a page that needs one goes whole, with one log line. Children match by `id`
first and then by position, as the morph does. Each change names the
element it expects; a page that finds another asks for the whole page
(`__resync`). On finch, playing: 1.2 KB/s at 1x, 4–7 KB/s at 10x and 60x
(428 KB a render before); the page built from 124 changes matched a
whole render exactly; all 29 controls work in Chrome. The one whole page
while playing is the chart replacing its "loading" line: 416 KB of a
485 KB page. Charts did not need layers with ids: the play-head is
already one attribute change, because the candles keep their places.
Found on the way: the morph that places a whole page lost its place
after swapping an SVG (it read the next sibling from the node it had
just removed), so everything after a changed chart was added again
below the old copy. finch never showed it because its charts are the
last thing in their card. `the_browser_applies_changes` drives a page
through Chrome's DevTools protocol and reproduces it. The gate also
turned up a flaky book suite: its workers' cargo caches lived in the
system temp directory, and macOS deletes temp files nobody has read for
three days — which took libsqlite3-sys's generated `bindgen.rs` from
under a cache cargo still believed fresh, failing 16 blocks. The caches
now live under `target/crow_book_test`.

**Phase F — built 2026-09-23.** First, a fix F depends on. The spec
said any string that began and ended with a tag was markup in a view,
and both renderers applied it. So a reader who typed `<img src=x
onerror=…>` into a field that other readers' pages showed ran script in
their pages. A sign-in makes that exploitable. **Text is text** now:
- markup is only what a view, an element, a chart, an icon, a bound
  input or `doc`'s `markdown`, `menu` and `pager` make;
- the interpreter carries it as a flag on the value, the compiler as a
  `markup` type (a string wherever a string is read);
- a view given markup at one call and text at another is compiled once
  for each, so the modes place every child alike (`tests/served/05`).

Then **who the reader is**:
- **`reader`** is a map of strings the runtime keeps with the session's
  facts. A web program's `reader` becomes `__reader[]`, in string
  interpolations too (`lower_fragment`). No handler assigns it, and no
  input writes it, whatever a page offers.
- **`sign_in[record]` and `sign_out[]`** are held effects. A sign-in
  seals the record, a sign-in id and an expiry with HMAC-SHA256 under the
  app's secret (`CROW_SECRET`, or `.crow/keys/secret`). The socket hands
  the page a one-time ticket, which the page trades by POST for the
  `HttpOnly` cookie a socket cannot set.
- **A sign-out ends the sign-in everywhere.** Every session holding it is
  woken and signed out, and the sign-in id is kept in
  `.crow/keys/signed_out` until the cookie would have expired.
- **Passwords:** `hash[pw, 'password']` is Argon2id, and `hash[attempt,
  stored] eq stored` checks one. There is now one `hash` for both modes
  in `crow_runtime`. The compiled `hash` used to panic on an unknown
  algorithm where the interpreter returned an error value.

Tests:
- `tests/served/06_sign_in` covers a wrong password, a right one, a
  reopened page that stays signed in, an admin-only action that stops for
  nobody, and a sign-out. Both modes serve it alike.
- `a_reader_signs_in_and_out_through_a_cookie` covers the ticket (good
  once), the cookie's flags, another tab signed out at once, and the old
  cookie refused.

Found on the way:
- The interpreter wrote `progress` and a chart's error as unmarked
  strings, and the chart error's reason went into the markup unescaped.
- A compiled handler that failed outright (a key its map lacks) left its
  page without a notice. Both modes now say `wipe stopped: map has no key
  'role'`.
- An element passed to a view as an argument did not compile.
- finch is unchanged and clean: 29 of 29 controls, no markup shown as
  text in any state.

**Phase G — built 2026-09-23.** Three pieces, in both modes.

*Tables that page.* `table[frame, page = n]` draws that page's rows and a
footer: `1–50 of 12000`, with Previous and Next. The frame never leaves
the server. Each paged table keeps its page in the session, under
`__pages` with the facts, so a kept session keeps its place. A table
over 1,000 rows without `page` is refused in place, as an over-budget
chart is. One writer does all of it for both modes
(`crow_runtime::layout::frame_table_html`, with `Paging`).

*Downloads.* `download[value, name]` is an effect held to the commit: a
frame goes as its CSV (`CrowFrame::to_csv`, the same writer `save`
uses), a string as text, a picked file as its bytes. The page is sent a
one-time link and saves the file under its name. In a test,
`downloads[a]` lists what was handed over.

*Uploads.* A file field binds a map of strings — `name`, `type`, `size`,
`path` — the same shape as `reader`. The page asks over the socket for a
ticket, giving the file's name, type and size. The server refuses a file
over `serve[…, uploads = N]` MB (10 by default) with a notice, and only
answers for a file field the page drew. The page then POSTs the bytes,
which the server reads no further than the limit. The session's copy
lives under `.crow/uploads/` and is removed when the session ends.

Tests:
- The served suite has `07_paging`, `08_download` and `09_upload`. Its
  runner now fetches downloads and sends uploads the way a page does.
- `a_file_round_trips_through_the_browser` picks a file in Chrome over
  DevTools, sees it land in the field, downloads it back and compares the
  bytes.

Found on the way:
- A fact declared `map[]` that no handler assigns did not compile: its
  empty default had no type.
- Chrome applies a download folder set over DevTools only while that
  DevTools session stays open.
- A server that refuses a POST without reading its body may reset the
  connection before the client reads the refusal.

**Phase H — built 2026-09-23.** `widget[name, props = value, on =
map[event = handler]]` is the one way out of the vocabulary. It renders
the same element in both modes (`CrowVNode::widget`): the module name,
the props as JSON and the event map as attributes. The page imports
`widgets/name.js` once, from a route that serves only that folder's
`*.js`, and calls `mount(el, props, send)`. It calls `update(el, props)`
when a render changes the props.

One change from the sketch: `update` takes the element too, since one
module can draw several widgets on a page.

The morph now leaves a widget's insides alone and follows only its
attributes. Changes never address them, because the server's tree has
none. A widget's events are offers like clicks, open to the handlers its
`on` names, which take any value, as a field's handler takes its text.
The validator refuses an `on` value that is not a handler.

Tests: `tests/served/10_widget` covers both modes, and
`a_widget_draws_and_talks_to_its_handler` checks, in Chrome over
DevTools, that a module mounts once, its event reaches the handler, and
the next render calls `update` without wiping what the module drew.

**Phase I — built 2026-09-23.** The team task board
(`examples/taskboard/`) is five parts and one widget, from the
vocabulary alone:
- `store` makes the tables and seeds two people with stored password
  hashes;
- `door` signs people in and out (`reader`, `sign_in`, `hash`);
- `cards` holds boards, cards, comments and attachments. Only an admin
  makes a board. A file field and `download` move files in and out, and
  the stars are a widget;
- `audit` is a log of who did what, paged at ten rows;
- `page` shows the door until someone signs in.

It needed no presence, and nothing outside the vocabulary.

Tests:
- Five harness tests in `door.co`, `cards.co` and `page.co`: the door,
  an admin-only action, two readers sharing a board, a card's stars,
  talk and files, and the page before and after signing in. Each makes
  a fresh board, so they stay exact on a database that persists.
- `crow test --served` now also drives every `examples/<app>/main.co`
  that has a `main.events`, from a fresh copy in each mode. The board's
  script signs in, reopens with the cookie, makes a board, adds and
  moves a card, rates it through the widget, comments, attaches a file,
  downloads it and signs out. Both modes settle on the same pages at
  all 21 steps.
- CI runs the harness tests and `crow check` against the accepted
  baseline.

Measured on the release build, with two readers in Chrome (Ada and Bob
in separate browser contexts), 18 of 18 checks passing:
- first page: 28.8 KB;
- sign-in: 37–50 ms from click to page, with the Argon2 check;
- a card Ada adds: on her page in 38 ms (median; 53 max), on Bob's in
  26 ms (45 max), at 1.5–1.7 KB of changes each;
- Bob's two stars redraw Ada's widget in place in 53 ms;
- over the whole run, Bob's page took one whole page and 44 change
  messages, 33 KB in all.

Found on the way:
- *A widget's handler did not compile* when it read its value as a
  number. The compiled handler declared the parameter as text, but its
  body was typed without it, so `int[n]` became a cast. Inference now
  types a widget handler's untyped parameter as text, as a chart's
  seek handler's is typed a float (`tests/served/10_widget`,
  `a_widget_handler_takes_text`).
- *`crow check` did not know the session layer's builtins*: `reader`,
  `notice`, `hash`, `sign_in`, `sign_out` and `download` read as
  undefined variables. It now takes the validator's list of builtins,
  so the two cannot drift.
- *A download button read as a dead control*: the baseline's effects
  lacked `notice`, `sign_in`, `sign_out` and `download`.
- *A password-hashing function read as nondeterministic*: the purity
  scan now counts `hash[x, 'password']` as random, since the salt is
  fresh each call.
- *`crow check` named `main.co`* for every finding in a part. It now
  names the part's file.
- *Checking created an empty database*: the database check opened the
  crowfile's SQLite file, which creates it. A file that does not exist
  yet is now left alone.
- *A test could read a session's facts but not its derivations*;
  `a.cards` now reads one as that session sees it. A failed truth that
  read a derivation named the derivation's last line instead of its
  own.
- *An app's accepted baseline in a subfolder was hidden from git* by
  the `*.txt` rule. The exception now covers any `.crow/baseline/`.
- *The book suite hung when the disk filled.* Its six workers' build
  caches, with the rest of the machine's, used the last of the disk.
  The runner ignored a failed write of a block's file, then ran the
  previous block's file instead: a page that serves, which never
  exits. A block the runner cannot write now fails, with the reason.

Not built:
- The database check reads only the entry file, so an app whose queries
  live in parts has them unchecked: the board's, and finch's.
- Stopping `crow run` with a signal leaves its compiled program serving.
