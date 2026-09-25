# finch in Crow — the simulation page (design)

**Status: design round in progress. Rulings received (2026-07-27): own
database (never write finch's), chart as a comprehensive module
(→ `chart-module-design.md`), project scaffolded. M0 data spike DONE —
findings in §12.**

Dogfood app #2: **the finch trading application rebuilt in Crow**, at
`~/code/crow_projects/finch` — the whole app, page by page, starting with
the Simulation page (historical replay of the VWAP scanner with paper
trading). Each page is a view module (`views/sim.co` first; scanner,
dashboard, history follow) behind one `serve[]`. This document designs the
simulation page. Not a translation: a re-architecture under Crow's
doctrine, using the app to field-test what
shipped this month (moments, freshness, `later`, `serve`, test blocks,
`crow check --db`) and to force the next round of language work with real
requirements instead of speculation.

---

## 1. What the page does (behavior, not code)

- Pick a trade date (or a random one from the last week). The app builds a
  **universe**: that day's gainers that have complete 1-second data and
  pass the scanner's filters (price, change, cumulative volume, float).
- **Replay** the morning window (06:55 → 09:30 ET by default) on a virtual
  clock at 1x / 10x / 60x / 300x, with play, pause, seek, and a scrubber.
- **Charts**: per-symbol 1s candles with cumulative VWAP and a rolling
  VWAP band, a play-head line, click-to-seek. A scanner table shows the
  whole universe with live-computed gate status as the clock advances.
- **Paper trading**: buy/sell at the replayed NBBO (buy = ask, sell = bid,
  nearest quote within 2s, fall back to bar close — and count the
  fallbacks). One open position per symbol per session.
- **Persistence**: sessions, trades, bookmarks `(date, symbol, ts, label)`.
  A sessions-history card with P&L and win rate.

Out of scope in v1, matching finch's own scope fence: playbook auto-trade,
side-by-side compare, multi-date sessions, granularity toggles.

## 2. What dies in re-architecture

The finch implementation is ~5,200 lines of Python/TypeScript plus chart
components. Most of it is the cost of being **two programs**:

| finch component | lines | in Crow |
|---|---|---|
| REST router (15 endpoints, SSE stream, session wire-up) | 2,982 | **0** — views read state; handlers mutate it; `serve[]` is the transport |
| Session service (virtual clock, play/pause/seek/speed, tick loop) | 431 | **~15** — the clock is *derived*, not ticked (§3) |
| Precompute builder (per-symbol numpy arrays, caching) | 932 | frame ops at render, cached by the runtime (§5) |
| React page + control bar + table components | ~600+ | views |
| SQLAlchemy models + Alembic migration | 192 | `migrations/*.sql` + `crow check --db` (§6) |

The engine's actual *logic* — timeline math, fill selection, universe
filters — is a few hundred lines in either language. Everything else was
plumbing between the browser and the data. Crow's claim is that the
plumbing is the language's job; this app is the test of that claim.

## 3. The derived clock — the flagship design decision

finch runs an asyncio task advancing a virtual clock and pushing SSE
frames. That is **stored, ticking state** — pause bugs, drift bugs, and
session-cleanup bugs all live there.

Crow doctrine: **store facts, derive beliefs.** The play-head is a belief.

```
--- facts (reactive state)
base = 0.0        --- virtual clock at the last control action (a moment)
anchor = 0.0      --- real moment of the last control action
speed = 1.0
playing = false

--- the belief, derived at every render — no timer, no task, no tick
fn clock[]
    if playing
        return min[base + (now[] - anchor) * speed, window_end]
    return base
```

Every control is a re-anchoring of facts:

```
fn play[]        --- anchor = now[]; playing = true
fn pause[]       --- base = clock[]; playing = false
fn set_speed[s]  --- base = clock[]; anchor = now[]; speed = s
fn seek[t]       --- base = t; anchor = now[]
```

There is nothing to tick and nothing to clean up. `serve[port,
refresh = 1]` re-renders each second and the play-head moves because
`now[]` moved. At 1x the cadence is exactly live; at 300x each render
advances five minutes — a deliberate, visible trade (§8). The entire
clock is four pure-ish functions with **test-block laws**:

```
test[b, a, s, dt]
    if playing and s gt 0.0 and dt gte 0.0
        clock never exceeds window_end
        clock is monotonic in dt at fixed anchor
    pause[] then play[] resumes within one render of where it paused
```

The 431-line session service becomes state that cannot drift, because
nothing moves it — it is recomputed from anchors on every read.

## 4. Data: two databases, honestly different

- **ClickHouse (finch's, read-only)** — 1s bars, NBBO, gainers, dataset
  readiness. This is **external ground truth**: facts Crow observes but
  does not own. `crow check` already refuses to pretend ClickHouse
  enforces constraints; here that honesty is load-bearing — we never
  write to it, and the F-4 doctrine (trust the source, not a snapshot)
  governs how we read it.
- **sqlite (ours)** — `sessions`, `trades`, `bookmarks`. Small, local,
  fully constraint-checkable, so every `crow check --db` detector works.

The market-time story is pure F-7: the window is
`moment[y, m, d, hour = 6, minute = 55, tz = 'America/New_York']`, bars
carry epoch-second moments, and staleness/ordering is arithmetic.

## 5. Timeline math — frames, at render

finch precomputes per-symbol numpy arrays (cum VWAP, rolling VWAP, gate
booleans) in a 932-line builder with hand-rolled caching. In Crow the
same math is frame expressions over the bars *up to the clock*:

```
fn timeline[bars, t]
    seen = rows[bars, 'ts lte ?', t]          --- bars the clock has revealed
    vwap = cumsum[seen.close * seen.vol] ./ cumsum[seen.vol]
    ...
```

VWAP-at-clock, gate status, and the scanner table are all *derived* the
same way the clock is. Where render-time cost bites (100 symbols × 9,000
bars), the answer is aggregation before charting (§7) and measured
caching — not a parallel precompute subsystem. **M0 audits which frame
ops exist**; `cumsum` and time-bucket aggregation are the expected gaps
(§9).

## 6. The paper book — invariants first, then code

The one-position-per-symbol rule is exactly the class this month's work
targets. It goes in the schema, not in an `if`:

```sql
CREATE TABLE trades (
  session_id INTEGER NOT NULL REFERENCES sessions(id),
  symbol TEXT NOT NULL,
  entry_ts REAL NOT NULL, entry_px REAL NOT NULL,
  exit_ts REAL, exit_px REAL,
  shares INTEGER NOT NULL, fallback_fill INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX one_open_position
  ON trades(session_id, symbol) WHERE exit_ts IS NULL;
```

The naive spelling (`query` open positions, then `exec` an INSERT) is
precisely detector 1a/1d territory — **`crow check --db` runs as the
design reviewer before the first order is placed**, and the partial
unique index is the fix it should rank first. If the detectors stay
silent on the naive version, that is a detector bug and this app just
earned its keep.

Fills are a pure function with laws:

```
fn fill[nbbo, t, side]
    --- nearest quote within 2s of t; buy = ask, sell = bid;
    --- else error kind 'no_quote' (caller falls back to bar close, counted)

test[t]
    fills use a quote within 2.0 seconds of t, or answer 'no_quote'
    buy fills never below the bid; sell fills never above the ask
```

Prices stay floats in v1 and P&L is display-rounded — **F-9 (money) gets
its requirements from this app** rather than from speculation.

## 7. Views and the chart

One page, four regions — scanner table, focused chart, control bar,
blotter/history. All are ordinary views over state; the control bar's
buttons call the §3 handlers.

The chart is the one place Crow has nothing today. Proposal — a `chart`
UI element, declarative like `table`:

```
chart[bars_1m, x = 'ts',
      candles = ['open', 'high', 'low', 'close'],
      lines = ['vwap', 'rvwap'],
      marker = clock[],
      seek = seek,          --- click passes the bucket's ts to the handler
      height = 320]
```

- **Server-rendered SVG** inside the vnode tree. No JS chart library, no
  canvas escape hatch: a chart is a pure function of a frame, diffed and
  patched like every other element. This keeps the LiveView story intact.
- **The view aggregates; the chart renders what it is given.** 9,000 1s
  bars would melt SVG; the view buckets to ~300 points with the same
  time-bucket op the timeline needs anyway. Chart stays dumb — no hidden
  resampling policy inside a UI element.
- `marker` + `seek` give play-head and click-to-seek with no new event
  machinery beyond §9's one feature.

## 8. Honest limits, stated up front

- **Single-operator app.** Crow's reactive state is shared, not
  per-client; two browsers would drive one replay. finch needed sessions
  because it is multi-user infrastructure; this tool has exactly one
  trader. Declared, not discovered. If Crow ever wants per-client state,
  this app documents the requirement — it does not smuggle the feature.
- **Render cadence bounds smoothness.** `refresh = 1` means the play-head
  hops at high speed. Acceptable for scanning; if it isn't, the finding
  is a *measured* argument about `refresh` granularity, which is better
  than an assumed one.
- **No live trading.** Replay only. The live scanner stays in finch.

## 9. What this forces Crow to grow — the dogfood payoff

Confirmed gaps, in expected order of pain:

1. **F-11: parameterized UI events** (language). Handlers today are
   zero-arg (`click = inc`). The scanner table needs `click =
   focus[sym]` per row; the chart needs `seek = seek` to deliver the
   clicked bucket's `ts`. This is the app's one genuine language
   feature — args captured at render, delivered on event. Needs its own
   design round (vocabulary rule applies; likely spelling: a call-shaped
   attribute value that lowers to data attributes + existing delegation).
2. **`chart` element** (module, the big build): SVG candles/lines/marker/
   seek in `crow_web`, both modes, `app[]`-harness testable.
3. **Frame ops** (module): `cumsum`, time-bucket aggregation — audit in
   M0, add what's missing to the kernel.
4. ~~Multi-file programs~~ — **not a gap**: Crow already has project
   modules (`use views.home`, `use lib.model`); the scaffold uses them.
5. **Persist spike vehicle**: v1 uses explicit sqlite + migrations so the
   detectors have a real schema. When `persist` gets its round, porting
   this book is the spike.

## 10. Milestones — each ends with something visible

- **M0 — data spike + audit.** Connect to finch's ClickHouse from Crow,
  load one day's 1s bars for one symbol into a frame, print row counts
  and load time. Audit frame ops against §5. *Proves the read path
  before any design survives contact.*
- **M1 — the clock.** Derived clock + controls on a text readout.
  Test-block laws green in both modes. *The flagship section, proven
  smallest-first.*
- **M2 — the chart.** `chart` element: static full-day candles+VWAP,
  then marker, then seek. `app[]` harness renders it.
- **M3 — practice mode.** One symbol end-to-end: chart, clock, NBBO
  fills, blotter, sqlite book. `crow check --db` clean over real
  invariants. *First usable session.*
- **M4 — scanner replay.** Universe query, gate table, per-row focus —
  this is where F-11 lands, designed against a live requirement.
- **M5 — history + field test.** Sessions card, bookmarks, random date.
  Then a real practice session, and a retro that feeds
  `discovered-features.md` — the same loop that produced `later`,
  `serve`, and `captured`.

## 11. Open questions for this design round

1. **Derived clock** (§3) — accept? It is the largest departure from
   finch's architecture and everything else leans on it.
2. **Server-side SVG chart** (§7) — accept, versus a JS-library escape
   hatch (which would breach the no-client-code doctrine)?
3. **Single-operator scope** (§8) — accept for v1?
4. **F-11 spelling** — `click = focus[sym]` (call-shaped, args at
   render)? Alternatives exist; needs the vocabulary treatment at M4.
5. ~~Where does the app live~~ — **ruled**: its own Crow project at
   `~/code/crow_projects/finch` (via `crow new`, then renamed: the
   project is the whole app, the simulation page is only where the build
   starts), with its own sqlite database `db/finch.db`. finch's Python
   databases are read-only ground truth; every write lands in ours.


## 12. M0 results (2026-07-27) — the read path works; the fix list is real

The spike connected Crow to finch's ClickHouse and pulled a full trading
day: **VIVK, 2026-07-24, 27,884 one-second bars (04:00–19:59 ET),
sub-second load**, timestamps flowing straight into moments and
`fmt[..., tz = 'America/New_York']`. The practice window (06:55–09:30) is
~9,300 bars — comfortably inside the chart budget after bucketing.

Real tables: `dataset_bars(dataset_id, symbol, date, timestamp
DateTime64(3,'America/New_York'), open, high, low, close, volume, vwap,
trade_count)`, `nbbo_1s(symbol, date, timestamp, bid_price, bid_size,
ask_price, ask_size, spread, mid_price)`, plus `ohlc_1s`,
`databento_trades`, `in_gate_trades`.

**Crow fixes the spike demands, in priority order:**

1. **ClickHouse `?` parameter binding** — the backend refuses params and
   tells you to interpolate, which is exactly what the injection check
   flags. The blessed `query[conn, sql, [params]]` form must work on
   every backend: implement client-side substitution with proper
   escaping. *(One-way-to-do-things is at stake, not convenience.)*
2. **Frame-column arithmetic** — `bars.close * bars.vol` fails while
   tensor `.*`/`sum` work. Columns must do math (or coerce to tensors)
   for §5's timeline to be writable at all.
3. **`cumsum`** (cum VWAP), **aggregate `min`/`max` over arrays** (`max`
   is binary today; high-of-day = `sort[xs][-1]` is a workaround, not a
   spelling), **time-bucket aggregation** (1s → 1m for charts).
4. **CH `Date`/`DateTime64` decode → moments** — currently unmapped.
   DateTime64 *is* an epoch number with a zone attached; it should
   decode to a Crow moment natively. (Enabled the `clickhouse` feature
   in the crow binary — it was sqlite-only.)
5. **Error UX**: CH server exceptions dump a full C++ stack (trim to the
   message line); binding an error value as a SQL param says
   `cannot bind done` instead of naming the error's reason; interpolating
   an error value prints `done`.

Item 5's `done` leaks are language bugs, not module gaps — an error
value's reason should survive every surface it reaches.


## 13. M1 results (2026-07-27) — the app serves; compiled mode taught us plenty

`crow_projects/finch` serves the simulation page compiled (`crow build
main.co`, port 3100): derived-clock controls, session start, NBBO
buy/sell against the invariant-guarded book, live blotter. Single-file
`main.co` for now (see fix 1). Getting there surfaced a second fix
list — all compiled-mode, all found by this app:

1. **Module-scoped reactive state is broken in both modes** (top
   priority). Validator: only click-referenced handlers are F1-exempt, so
   a helper like `set_speed[s]` can neither read nor write state — and
   the fix (`click = set_speed[10.0]`) is F-11, its third appearance.
   Transpiler: state threading loses the module prefix entirely
   (handlers referenced bare `session_id` where state held
   `sim_session_id`). Until fixed, live pages are single-file.
2. **Compiled query frames are stringly typed** (second priority). No
   type introspection for ClickHouse (or expression columns anywhere), so
   every column reads as str compiled but native-typed interpreted — a
   parity break in the type system. Cascaded into six separate app
   errors. Cure: CH DESCRIBE-based introspection + DateTime64 → moment.
3. **Fixed in crow this session:** (a) `expand_module_imports` now
   rewrites qualified refs (`home.render` → `home_render`) and the
   interpreter no-ops pre-expanded user modules instead of dynamically
   double-loading them — `crow new --web`'s own template never ran
   before this; (b) assignment into a typed variable is now a
   **consumption point**: `result[T]` unwraps into `T`, aborting the
   handler on error (the checked-fallible-into-state pattern was
   previously unwritable); (c) auto-parallel waves no longer fuse
   db-handle assignments (the rayon::join bypassed the OnceCell `.set()`
   protocol — handles stayed forever unset); (d) ClickHouse driver runs
   its blocking work off-runtime (compiled apps are #[tokio::main];
   `block_on` inside it is illegal); (e) the generated panic hook exits
   only for main-thread panics — a contained task panic killed the whole
   server; (f) `crow new --web` template fixed (`view render` +
   `home.render`); (g) generated crates now compile in the db features
   the connect URLs actually name.
4. **Smaller findings:** bare `connect` means websocket once `use web`
   is present (must write `db.connect` — a vocabulary collision worth a
   teaching error); mixed-type SQL param arrays don't exist (params were
   stringified through sqlite affinity); `exec`'s affected-count can't be
   compared without tripping handler propagation; `serve`/`page` are
   interpreter no-op mocks, so `crow run -i` never serves (harness-only).

**M1 addendum (2026-07-28):** F-11 shipped (deferred calls in event
position) — the speed handlers collapsed to one; main.co is 198
lines. New finding: `crow build` printed "build ok" while its inner
cargo invocation had not recompiled the changed crow_web dependency —
the copied binary was stale. Needs a build-freshness guarantee (fix
list).

## 14. Reimplementation, not port — the page rethought (2026-09-07)

The vocabulary is complete (ui-engine-design.md, "The primitives"), so
the question is no longer *can* Crow draw the Python page but *should*
it. It should not. The Python simulation page is 2,458 lines of React in
four components, fifteen effects coordinating fetches, two modes sharing
one session hook, five polling and navigation flows (next in gate, next
setup, next RSI>70, auto-advance with grace windows, skip lists), nine
filter inputs of which five are disabled inputs standing in for labels,
a Refresh button on every panel, a Start button whose label mutates to
"Starting…", keyboard hints inside tab labels, and a broker badge on a
page that never talks to a broker. Each of those is a habit that a
framework without derived views forces on you. Crow removes the cause,
so the page is redesigned from its facts.

### 14.1 The habits we leave behind, and the principle that replaces each

| The Python page does | Because | Crow does instead |
|---|---|---|
| Polls, and puts a Refresh button on every panel | Views own fetched copies of data | Views derive from facts at every render; `serve[..., refresh = 1]` declares the period (F-3). No Refresh anywhere. |
| Shows the live scanner config as five disabled number inputs | An input was the only component that shows a number with a label | One muted sentence: *Gate: $1–15 · change ≥ 1% · float ≤ 50M · rel vol ≥ 2 — the live scanner's, edit on the Scanner page.* One home for the config (one way to do things). |
| Mutates the button label to "Starting…" | Status had nowhere else to live | Status is a value: a status line and a toast; the button keeps its name and `disabled = starting` follows state. |
| Two modes, a toggle, and "(A)/(B)" key hints | Two components grew separately | One session. "Mode" is derived: a session that placed trades was practice. The scanner's navigation (next in gate, next setup) is available in every session. No tabs. |
| Window start and end as time pickers, a speed input, an Autoplay checkbox | Every parameter became a control | A window preset `select` (Pre-market 06:55–09:30 by default, Open 09:30–10:30, Full day), speed as a `radio` bound to state, Start starts paused. |
| A free-text symbol field | Nothing knew the universe at that point | `select` over the day's universe — the scanner already computes it; no typos, no "no such symbol". |
| A ten-column sessions table with NBBO/fallback counters and an unconfirmed ✕ | Diagnostics leaked into the product | A list of session rows: when · day · symbols · trades · W/L · P&L. The row replays; delete is behind a confirm `modal`; fallback counters stay in the database. |
| A client-side chart with its own zoom, crosshair and clock | KLineChart owns state the app can't see | The chart is a view over the frame; seek by clicking a bar; the play-head is derived from base/anchor/speed/playing (§3). |
| Fifteen effects | Every fetch needed a lifecycle | None. Handlers set facts; views derive. "Next in gate" is one handler that asks ClickHouse for the next moment and sets `base`. |
| Bookmarks in a separate card with its own Refresh | A separate fetch | A list under the sessions, present only when there are bookmarks. A bookmark is a fact (day, symbol, moment, label); clicking one starts a session at that moment. |
| A 13-page nav and a broker connection badge | The shell was shared with the live app | The shell lists the pages this app has and grows with it; the chip shows the data facts (ClickHouse `finch_trading`, datasets ready through the latest day). |

### 14.2 The page, region by region

The look is one line: `serve[3100, refresh = 1, theme = 'dark',
density = 'compact', wide = true]`. Composites are views: `stat[label,
value]`, `panel[title, ...]`, `session_row[s]`.

1. **Shell.** `nav[Simulation, Scanner, active = path]` on the left; the
   data chip on the right.
2. **Setup** (no session). One card, one row: Day (`field`, date),
   Symbol (`select` over the day's universe), Window (`select` of presets),
   Speed (`radio`), *Random day & symbol*, **Start**. Under it the gate
   sentence. That is six controls where the Python page has fourteen.
3. **Session** (running). A header row with `sym · day` and the stat
   tiles — Clock (ET), Progress, Last, High, Volume — and two buttons,
   *Bookmark* and *End session* (confirm modal). The chart at full width:
   candles, both VWAP lines, volume, RSI and CVD panes, fills as markers,
   click to seek. A transport row: one Play/Pause button whose label
   derives from `playing`, the speed radio, *Jump to 09:25*, *Restart*, the
   seek slider, and the navigation trio *Next in gate · Next setup · Next
   RSI > 70*. A trading row: Qty (`field`, number), **Buy**, **Sell**, the
   status line. The scanner at the play-head in a `scroll`, gate rows
   marked, click focuses the symbol. The blotter, a table with `delta =
   ['pnl']`.
4. **Past sessions.** A list of `session_row` views; empty state one
   line. **Bookmarks** below it when any exist.

Nothing on the page polls, refreshes, or holds a copy of anything.

### 14.3 What Crow makes easier, and what this will teach it

- The session is about ten facts (day, symbol, window, base, anchor,
  speed, playing, session id, qty, status); the page is a function of
  them and `now[]`.
- The acceptance test is a test block, not a screenshot: `app[sim, width
  = 1280, wide = true, density = 'compact']`, start, seek, buy, sell, end,
  and the sessions list shows the row. Screenshots are for the reader.
- Modules: `views/shell.co`, `views/sim.co`, `scanner.co`; module
  state now survives both modes.
- Gaps this is likely to surface, in order of likelihood: the
  stringified SQL parameters (`'`session_id`'`) that stand in for
  mixed-type parameter lists; a P&L column computed at render from two
  frame columns; the harness's look arguments; `for` as a call argument.
  Each is a language fix, not an app workaround.

### 14.4 Acceptance

Region by region, in the order above: a harness test for the region and
a side-by-side screenshot against the Python page at 1280px. Not a pixel
match — the same information in the same order, with fewer controls and
no button that exists to compensate for a missing derivation.

### 14.5 What the reimplementation taught Crow (2026-09-07)

The page is now parts: `sim/session.co` (day, symbol, window, size,
the open session, what the page last said), `sim/clock.co` (four
anchors and a window; `head` derived), `sim/tape.co` (`bars` five
minutes old at most, `so_far` at the play-head), `sim/scan.co` (the
scanner at the play-head), `sim/trading.co` (fills, `trades`, the
blotter), `sim/history.co` (sessions and bookmarks as lists of rows,
delete behind a `modal`), `views/shell.co` (nav and the data chip),
`views/sim.co` (the page), and `main.co` is six lines. Every part
carries its facts, derivations, handlers, views and tests; nothing polls,
refreshes or holds a copy. What had to change in Crow to get there, in
the order it surfaced:

- **Modules are parts.** Derivations and multi-assigns inside a module
  were dropped by the expansion; qualified reads inside interpolation
  spans, maps, named arguments and derive bodies were not rewritten; a
  module reached through two importers was expanded twice under stacked
  prefixes; module paths resolved from the importing file's directory,
  not the project's. Now a module file is expanded once under its path
  (`sim_clock_`), rooted at the crowfile, and `session.day = d` in another
  part's handler writes the session's fact. `crow test sim/clock.co`
  runs a part's tests with its imports inlined; `crow check main.co`
  checks the whole app.
- **Every view renders from the session.** Only the page's root view had
  the state; a part's view reading its own facts had nothing to read
  from. Every view of a page program now takes the state, and a view
  calling a view hands it on.
- **A handler's epilogue writes only what it assigned.** It wrote every
  fact back from the locals it read at entry, so a helper's writes
  (`clock.begin[...]` from `start_session`) were put back to their old
  values a moment later. The epilogue is now the handler's own writes,
  and after a call into a state-writing helper the caller reads those
  facts again.
- **A derivation's name is its value everywhere** — passed as a call
  argument too, where it was typed as a function reference.
- **A query is typed against its own connection**, and sqlite has a
  `describe` from declared column types; before, every connection was
  tried in turn and a helper scanned before its handle's part was known
  had its failure cached — every book frame read as text, and a text
  session id could not be assigned to the integer `confirm_id`.
- **A local that holds a view or an element holds its markup**, in a
  list or alone, as it does in the interpreter; sub-views' locals were
  annotated as strings while elements produced nodes.
- **Smaller:** `click = replay[s.id[i]]` froze its argument inside a
  `json!` array as a block, which the macro read as a map; a select inside
  a view reached `frame.select` in the interpreter; each part's `use db`
  was inlined again and again until `query` was "ambiguous — provided by
  db and db"; a view named `live` was silently shadowed by the web
  builtin of that name in one mode — now reserved in both; a test block's
  variables are globals, so a test's `s` collided with a view's local
  `s`.
- **Speed is buttons, not a radio.** A bound input cannot re-anchor the
  clock: changing speed must freeze the play-head and move the anchor, so
  it is a handler per speed with the active one disabled. A derived clock
  decides this, and the design's radio gave way.
- **Still open:** the layout engine sizes a `select` by all of its
  options joined, where the browser sizes it by the widest; the harness
  has no `wide`/`density` look, so the page's layout tests see the
  reading column; mixed-type SQL parameters are still stringified.
- **A handler lives in the part that wires it** (2026-09-07). A part is
  validated alone, with only its imports inlined, so `skip_symbol` in the
  session part — wired only by the nav part's button — was a plain
  function reading facts there. It moved beside its button. And a helper
  that reads state and assigns none is a derivation: `band` (the
  config's price band) is `derive band`, not a function. And a part's
  names are reserved as its file is: `derive progress` beside `use ui`,
  `derive sessions` beside `use web` and `view stats` beside `use frame`
  were fine in the app only because expansion prefixed them; each part is
  now validated alone as it is expanded, whichever file is the entry.
- **One validation gate.** `crow check` and `crow test` ran no validator,
  so a whole-app check said "all good" for a program `crow run` refused.
  All four pipelines now pass the same gate, with the same message.
- **A fact a handler assigns a float is a float from its declaration.**
  `deleted = 0` then `deleted = id` from a float key column built the
  handler's `deleted = id` as int = float. The int/float rule now spans
  the program, for views and handlers; a plain function's local of the
  same name is a new binding and says nothing.
- **An error hands back across types, and a handler stops.** `if not
  b.ok` / `return b` in `next_rsi` (a frame's error from a function
  whose value is a moment) would not build; the error now travels with
  the function's default value. A fallible condition in a handler
  (`t gt 0.0` on a result) tried to return an error from a handler that
  returns nothing; it now stops the handler, lands what it wrote, and
  logs the reason.
- **A helper sees what its caller wrote so far.** Skip records the
  symbol, then calls the gate hop — which, compiled, read the state the
  caller had not written back yet and hopped to the symbol just skipped;
  the interpreter's helper reads the module variable directly. A
  handler's own facts are now written through before a call into a
  helper (an unchanged value costs nothing), as they are read back after.
- **A session id is the database's.** `round[now[]]` collided when two
  sessions started in one second — the part tests did. The insert says
  `RETURNING id`, which took two compiler fixes: sqlite gives a
  `RETURNING` cell no declared type, so it is typed from the table's
  columns; and a frame assigned in a parallel wave (`r = query[...]`
  beside the independent `starting = false`) kept no column types for
  the reads after it, so `session_id = r.id[0]` read as text.

## 15. Feature parity checklist (2026-09-07)

Vijay's rule, after seeing the first parts build beside the Python page:
*a reimplementation has the same functionality.* §14.1 kept the
information and cut controls; some of those controls were capabilities.
This list is read from the React sources (`ScannerReplayMode`,
`PracticeMode`, `ReplayControlBar`, `TradePanel`, `ComplexOrderCard`,
`SessionsHistoryCard`, `BookmarksCard`, `patternChips`, `replayApi`) and
the server (`api/scanner_vwap_replay.py`, `patterns/`). What §14.1
retires stays retired only where it was a habit and not a capability:
Refresh buttons, mutating button labels, five disabled inputs standing
in for the live config. Everything else is built, in this order.

| # | Capability | Python | Crow status |
|---|---|---|---|
| 1 | History as a table: When, Mode, Date, Symbols (+N), Trades, W/L with %, PnL, PnL %, NBBO/Fb, delete; the count | SessionsHistoryCard | built (2026-09-07): `table[sessions, click = replay, key = 'id', delta = ['pnl', 'pnl %'], actions = map[Delete = ask_delete], hide = ['id']]`, every figure in the SQL |
| 2 | Bookmarks: count, a label, open in replay, delete | BookmarksCard | built (2026-09-07): a label field beside Bookmark, a table with open-on-click and Delete; pinning is not in the schema yet |
| 3 | Two modes: Scanner Replay (the day's universe, Max symbols) and Practice (one symbol) | mode tabs | built (2026-09-07): `tabs[modes, mode]` over the `mode` fact; the practice setup is one typed symbol; `mode_code` records the mode with the session |
| 4 | Window start and end as times; random symbol/date over the last N months | setup | built (2026-09-07): `field[..., type = 'time']` for From and To; `random_pick` draws a day from the last N months (the `ranges` select) and a symbol from that day's universe |
| 5 | Next in-gate episode (this symbol, any symbol); skip current stock | ReplayControlBar, next-in-gate routes | built (2026-09-07): `sim/nav.co` — a ClickHouse window query finds the next episode after the play-head, any symbol or this one; Skip records the symbol and hops to the next in-gate stock, as the bar's Skip does |
| 6 | Next RSI(14, 3m) > 70 crossing | ReplayControlBar | built (2026-09-07): kernel `rsi` over 3-minute bars, the first crossing above 70 after the play-head, any symbol or this one; hops are not bounded by the window's end, as in Python |
| 7 | Scanner table filters: in-gate only, RSI > 70 charts only; per-symbol mini charts | ScannerReplayMode | built (2026-09-24): the table leaves out skipped symbols and carries RSI(14, 3m) from the kernel's `rsi` over 3-minute closes; three boxes (In-gate only, RSI > 70 charts, Show charts) choose up to twelve cards, strongest slope first, each a `chart` of price and both VWAPs since the window opened that focuses its symbol on click. One chart per card where finch draws two (the 1-second chart and the day's context): the window so far is both. Building it found seven compiler bugs (crow dbb96ed) and one rule for apps: a remote derivation is computed off the render, so a view must not assume two of them caught up together (a card draws its tape only when the tape has rows) |
| 8 | Patterns: Flag / Pennant, VWAP Launch, Burst Ignition, VWAP Power Cross, Ascending Staircase, Leg Scalp, B2e Surge, Mid-day Grinder, EVT Fade — chips select which are searched | patterns/ (9 detectors, ~3.1k lines) | built (2026-09-24): `patterns/` holds each detector as a part of pure functions over the 1-second bars as lists of numbers (with `tape_state` for the power cross), returning a record of lists; `sim/patterns.co` derives each over the bars up to the play-head when its chip is on (on as finch's `pattern_configs` has them, EVT Fade off), marks what each found on the tape and says its live setup under its chip. Each detector's tests are finch's own truths; the last four were run against the Python on real days — Flag / Pennant on three, Mid-day Grinder on eight (LEDS and ENHA of 2026-07-17 among them), B2e Surge on seven, EVT Fade at every 20th bar of seven — and agree signal for signal, to the last bits of a float. Building it found two dozen compiler bugs, among them a part's locals read as another file's names, fallible record fields, quadratic reads of a record's list in the interpreter, and records unreadable by a compiled test (crow, row 8) |
| 9 | Next pattern setup; Auto Next as a running mode; auto-advance when the closing window ends | PracticeMode, next-setup | built but for practice's roll-over (2026-09-25). **Next setup**: `patterns/starts.co` finds when each confirmed setup of a day starts — its earliest drawn point, finch's rule, per detector — with the tape's caps on how many it draws lifted as finch meant them ("so early-session setups are reachable") and leg scalp's leg lines left out (drawings, not setups); `sim/setups.co` runs every detector once per session over each symbol's day; the button pauses the play-head a minute before the next setup of a chip that is on, where its symbol shows in the scanner, and a second press goes on from that setup. Checked against finch's own search on six real days (MNTS, BMR, YYAI, IBIO, LEDS, ENHA), setup for setup, and in Chrome. **Auto Next**: the clock follows a chain through the day's episodes (`sim/autonext.co`) — in a scanner replay the next symbol to enter the gate, from its lead before, once the focused one has been out of its gate for ten seconds ("⏳ Closing m:ss" counts it down); with RSI > 70 charts checked, the next RSI(14, 3m) crossing above 70, taken where it crosses, a symbol staying active while its RSI is above 70 (or it is in its gate, with In-gate only checked too); in practice, the symbol's own next entry. The focused symbol is derived from the chain (the reader's pick is the fact `picked`; focusing is the scanner's), and every move of the head starts the chain again where it lands — in the scanner waiting for the symbol's episode to close, as finch moves on only when it sees the symbol leave its gate. Differences by decision: the boxes are read when Auto Next is switched on, and a turn into the gate outside the price band is no entry (finch's hops skip it). Checked in Chrome on 2026-05-26: the focus moved MNTS → YMAT → MNTS → YMAT → OKLL → YMAT by the gate and MNTS → CVU → TACT by RSI, with nothing clicked. Next: practice's roll-over onto a random day, which starts a session and so needs an event the chain cannot give by itself. Building it found eleven compiler bugs, among them a part's locals taken for the entry's names, a ui program's facts computed before its top level ran, a record's numbers given as a tensor, a served session's records lost between events, and a compiled test's click whose list and record arguments never arrived |
| 10 | Brackets: arm at trigger/stop with qty, target, trail, +2R stop floor, limit sells, flatten, unrealised P&L; bracket defaults | TradePanel, ComplexOrderCard, bracket routes | built (2026-09-25): `sim/book.co` walks a symbol's orders over its 1-second bars up to the head — the managed bracket (its stop trails the peak and only rises, its target stands over the average, a hit sells at that bar's bid), the resting limit (filled at its price on the first bar through it), a flatten, long only and clamped to what is held; `sim/trading.co` keeps the orders as a fact, fills a market order at the quote nearest the head (the bar's close when none is within 2 s, refused 5% from the last price or past 5000 shares), shows the bracket card and the limit row in Practice, and at End writes the fills up to the head as FIFO lots with the brackets, whose last settings the next session starts from. The manual arm (trigger, stop, size, lock 2R) is #11's arm, placed by the reader. Checked in Chrome: the stop rose under a new peak, uPnL followed the head, End wrote the open lot and the bracket. Building it found nine compiler bugs, among them records a fact holds (no serde traits compiled), derivations called in a cycle through a function's own `fills`, and a list joined to numbers |
| 11 | Session-wide Auto-trade on setups | auto-trade route | built (2026-09-25): `sim/exits.co` is finch's exit engine (fast fail, the pending race, confluence, the rolling channel, the trailing window and its ladder, the big-winner lock, stall), each rule a function of the bars up to one bar, with finch's own tests as truths; `sim/arms.co` is the arm — made from a B2e Surge signal (read at its own second), sized by finch's tiers (the extended-RSI tier and press-after-win; the three that read the day before are 1.0 without its data key, as in finch), filled when a bar trades through the trigger while a setup is ready or has broken out, and exited by the engine in finch's order; the book walks arms between the bracket and the limit on each bar. Switched on for the session (100 shares) or for a symbol (the manual arm's size and lock), from the play-head on. Verified against finch's route run at 1-second heartbeats on real days: with signals read as finch reads them at its heartbeat, the first arms agree and the rest diverge where a signal was found a second late and filled back-dated; with finch's signals read at their own second, every arm and fill agrees on MNTS 2026-05-26 (nine arms), BMR 2024-02-12 and YYAI 2025-10-22, and on IBIO 2024-03-27 all but one: switched on, finch takes the day's earliest signal however old (its "no age limit"), here one from before the window that it fills at 06:55:00 and loses, which puts its next arm on probation; the port trades the signals from the switch on. The MNTS test pins finch's three arms of 06:55–07:20. Building it found seven more compiler bugs (loop-carried types in the call-site scan, a top-level index taken for a call, record constructors propagating failure, an empty first value typed as able to fail, a path write's borrow, the runtime's stack size, a derivation's read set through a function's own names) |
| 12 | Autoplay at start | setup | built (2026-09-07): the Autoplay checkbox; `start_session` presses Play; `test autoplay_starts_the_clock` |

**Rows 9–11 without a tick (decided 2026-09-25).** Vijay chose no new
construct: what a replay does with nobody clicking is a derivation of the
play-head. It can be, because everything finch's replay does on its own
is causal — a decision at second *t* reads the bars up to *t* and the
orders placed before it — so it is computed over the window once and
revealed up to the head, and a render asks what has happened by the
head. Nothing ticks, as the clock does not (§3). The rules are finch's
replay routes, its `exit_engine` and `patterns/b2e.py`, read end to end
before building and recorded with their constants in
`finch-replay-rules.md`.

- **The book is derived.** An order is a fact written when it is placed,
  with the play-head it was placed at: a market buy or sell, filled at
  placement at the quote nearest the head (buys at the ask, sells at the
  bid, the bar's close when no quote is within 2 s, as finch's
  `nbbo_fills` does); a managed bracket (a market buy with a trail and a
  target, in % or ¢); a resting limit sell a few cents over the bid; a
  manual arm (trigger, stop, size, lock 2R); and the session's auto-trade
  switch. What follows an order — a stop moving up under a new peak, a
  target or a stop hit, a limit filling, an arm triggering — is computed
  per bar over the bars after it. The book at the head is every fill up
  to the head: the position, the average price, unrealised P&L, the
  blotter. A rewind shows the book as it stood at the new head, and
  playing on replays what the orders do.
- **Auto-trade** reads the B2e Surge signals (patterns/surge.co, one
  causal pass) of each of the session's symbols over the window. Each
  signal after the last exit is an arm, sized by finch's five
  multipliers, filled when a bar trades through its trigger while a
  setup is ready or has broken out, and exited by finch's exit engine
  (fast fail, confluence, channel, stall, the trailing window and its
  ladder). The signals are computed once per symbol and session; the
  head reveals them.
- **Auto Next** moves the play-head itself. With it on, the head is the
  point at the played distance along a chain: the focused symbol until
  it has been out of its gate (or its RSI > 70 run) for ten seconds,
  then the next symbol to enter, a few seconds before it does. The chain
  is computed from the day's episodes, the head walks it, and the
  focused symbol is the chain's at the head. Practice's roll-over onto a
  random day is the chain continuing into that day. Next setup is a
  button: the next confirmed pattern after the head, a minute before it.
- **What is written.** The session's fills up to the head are written at
  End, as FIFO lots — closed lots with both sides, one open row per
  symbol still held — with each bracket's trail and target, so the next
  session's bracket starts from the last one used. finch writes each
  fill as it happens.

Where this differs from finch's replay, by decision: finch evaluates at
its stream's heartbeat, so a fill is found up to a second late and dated
back, and a seek backwards re-walks bars under an order already done;
here every order is evaluated per bar once, and a rewind is the book as
it was. finch draws the lead before a gate episode at random; here it is
a hash of the episode, the same on every replay.

**The look.** Three causes, none per-app CSS: records were drawn as rows
where the vocabulary has `table`; the kernel styled every paragraph in a
card as a monospace figure (a stat-tile rule that leaked onto the gate
sentence); the kernel has no bar for a shell. The fixes are in the
kernel and the vocabulary — a `font = 'mono'` text attribute for figures,
`header` as a bar, a row action on `table` — with the layout fixtures
re-measured after each stylesheet change.
