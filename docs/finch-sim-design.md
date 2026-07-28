# finch in Crow — the simulation page (design)

**Status: design round in progress. Rulings received (2026-07-27): own
database (never write finch's), chart as a comprehensive module
(→ `chart-module-design.md`), project scaffolded. M0 data spike DONE —
findings in §12.**

Dogfood app #2: **the finch trading application rebuilt in Crow**, at
`~/code/crow_projects/finch` — the whole app, page by page, starting with
the Simulation page (historical replay of the VWAP scanner with paper
trading). Each page is a view module (`views/sim.crow` first; scanner,
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
