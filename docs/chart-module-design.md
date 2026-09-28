# The chart module — design

**Status: C0–C2 shipped (September 2026) — candles, lines, volume,
marker, tz, panes with levels, points, seek — plus `use chart`, the
indicator module (`crow_runtime::indicators`, rows of the builtin table
over the kernels in `crow_runtime::ta`: sma, ema, rsi and bucketed rsi,
macd, vwap and trailing vwap, rolling_sum, cvd). The finch app's
simulation page (docs/finch-sim-design.md) is the driving application;
the module is general. Next: C3 hover readout and shared x-windows.**

Charts are the first UI element whose *content* is dense numeric data
rather than text. That makes them a different kind of component: the
existing elements (`div`, `table`, `button`) are cheap to render and diff;
a chart over a trading day is thousands of points redrawn every refresh.
This document plans a chart module comprehensive enough to grow for years
and fast enough to live inside the LiveView loop.

---

## 1. Doctrine — what a chart is in Crow

**A chart is a view over a frame.** Not a widget with its own state, not a
JS library wrapped in an escape hatch — a pure function from data to
picture, re-derived at every render like every other view. Three
consequences:

1. **Declarative or nothing.** The caller names columns and intents
   (`candles = [...]`, `lines = [...]`); the module owns layout, scales,
   axes, colors. No imperative draw calls, ever — the moment a chart API
   grows `draw_line[x1, y1, x2, y2]`, it has stopped being Crow.
2. **Server-rendered SVG in the vnode tree.** Charts diff and patch like
   any element; the play-head moving is a one-attribute patch, not a
   redraw. No client-side chart library, no canvas, no new JS beyond the
   delegation that already exists.
3. **The chart renders what it is given.** Aggregation is data work and
   belongs in frame ops, visible in the caller's code — a chart that
   secretly resamples is a chart that lies about its input. The module
   *enforces* a point budget (§5) and errors with the fix spelled out,
   teaching the `bucket` call rather than doing it silently.

## 2. Surface — one element, named intents

```
chart[bars,
      x = 'ts',                                --- moments on the x axis
      candles = ['open', 'high', 'low', 'close'],
      lines = ['vwap', 'rvwap'],
      bands = [['rvwap_lo', 'rvwap_hi']],
      bars_y = 'volume',                       --- volume pane below
      marker = clock,                          --- vertical line (play-head)
      points = trade_marks,                    --- frame: ts, y, label, kind
      seek = jump,                             --- click → handler[ts]
      tz = 'America/New_York',
      height = 320]
```

Everything except the frame and `x` is optional; any combination of
intents composes. Axes, gridlines, hover targets, and colors are derived —
axis ticks obey the `tz` (a 09:30 label means 09:30 *in that zone*,
F-7 discipline), y-scale fits the visible series jointly, volume pane
appears iff `bars_y` is given.

The vocabulary rule holds: `chart` is one reserved word (checked against
the corpus — no collisions), and every capability is a named argument on
it, not a sibling element. `sparkline`, `candlestick`, `heatmap` as
separate elements would be the fmt-mini-language mistake in UI form.

## 3. Capability roadmap

| Stage | Capabilities | Driven by |
|---|---|---|
| **C0** | line + multi-line, x = moments, axes/ticks/tz, height, y autoscale | replay M2 |
| **C1** | candles, volume pane (`bars_y`), marker | replay M2–M3 |
| **C2** | `seek` (click → ts handler), `points` (trade markers), bands | replay M3 |
| **C3** | hover readout (crosshair + values at cursor, server-round-trip like every event), multi-pane sync (two charts sharing an x-window) | replay M4, dashboard |
| **C4+** | zoom/pan as *state* (`window = [t0, t1]` — the caller owns it, a zoom is a handler writing state; the chart never keeps private viewport state), log scale, second y-axis, themes | future apps |

Each stage lands with parity coverage, `app[]` harness tests, and a book
section before the next begins. C0–C2 are the replay app's needs; the
architecture must not paint C3/C4 into a corner, which is what §4–5 are
for.

## 4. Architecture — where the code lives

```
crow_runtime/src/chart.rs      geometry kernel (pure, no I/O):
                               scales, tick selection, path building,
                               candle layout, downsample check
crow_web/src/view/chart.rs     vnode emission: kernel geometry → SVG nodes,
                               data-ts attributes, event wiring
src/lower.rs                   chart named-arg signature
src/checker.rs · validator     vocabulary, arg validation, teaching errors
```

The split is the shared-kernel principle applied to pixels: **geometry is
math and lives in `crow_runtime`** where both modes and unit tests reach
it; only the final SVG assembly touches `crow_web`. The kernel computes
`[(x, y)]` paths and tick positions from column slices — it never sees a
vnode. This is what keeps C3/C4 cheap: hover readouts and zoom re-use the
same geometry functions with a different window.

Axis-tick selection for *time* axes is its own kernel function with real
rules (round to minute/5m/30m/hour/day boundaries **in the chart's tz**)
— tick logic is where naive charts betray timezones, and we just deleted
machine-local time from the language; the chart must not reintroduce it.

## 5. Performance — budgets, not vibes

The LiveView loop re-renders every `refresh` seconds; the chart must never
be the reason a render misses its budget.

- **Point budget: 1,000 per series, enforced.** Over budget is a teaching
  error naming the fix: `chart got 9360 points — bucket first:
  bars_1m = bucket[bars, 'ts', 60.0, ...]`. At ≤1k points, SVG path
  strings are small (tens of KB) and the browser is comfortable.
- **Geometry cost target: <1ms per chart at budget** in the kernel
  (measured by kernel benches, the same gate F-2 used). Path-string
  assembly is O(points) string pushes — preallocate.
- **Diff-friendly output.** Stable node identity: series are keyed nodes;
  data updates patch `d` attributes; the marker is one `line` node whose
  `x` moves. The morphdom-style patcher already in `crow_web` does the
  rest. A moving play-head must transmit ~one attribute, not a chart.
- **Measured, then optimized.** The replay app at `refresh = 1` with 4
  charts is the acceptance test; if render time exceeds budget the answer
  is found by measurement (likely: memoize geometry on unchanged column
  hashes), not guessed in advance.

## 6. The one language prerequisite: parameterized events (F-11)

`seek = jump` needs the clicked bucket's `ts` delivered to the handler —
the first UI event that carries a *value*. The scanner table needs the
same feature (`click = focus[sym]`). Sketch, subject to its own round:

- Handler attributes may be **call-shaped with frozen args**: `click =
  focus[sym]` captures `sym`'s render-time value; the chart's `seek`
  passes the datum's `ts` as the final argument.
- Lowering: value serialized into `data-crow-arg`; the existing click
  delegation posts it back; dispatch deserializes and calls the handler.
  No new client machinery.
- The checker enforces handler arity against the attribute, and the
  teaching error for a bare `click = focus` (needs an arg) names the
  spelling.

F-11 gates C2's `seek` and the replay app's M4 — it is the only place
this module needs the *language* to move.

## 7. Testing

- **Kernel unit tests** (geometry): scales, tz-correct ticks across DST
  boundaries, candle layout, budget enforcement — pure functions, exact
  assertions.
- **Parity**: a `29_chart.co` rendering to the html string in both
  modes; byte-identical output is the bar (same discipline as everything
  else).
- **`app[]` harness**: render a view containing a chart, assert on the
  SVG (series present, marker position), click-to-seek once F-11 lands.
- **The replay app is the perf test**: 4 charts × 1s refresh × a real
  trading day, measured.

## 8. Open questions for this round

1. **Surface** (§2) — one `chart` element with named intents vs. a family
   of elements. The doc argues one element; push back if the argument is
   wrong.
2. **Budget-as-error** (§5) — a chart that *refuses* oversized input and
   teaches `bucket` is opinionated; the alternative (silent downsample)
   is friendlier and dishonest. Which Crow is this?
3. **Hover (C3) server round-trip** — LiveView purity says crosshair
   position is an event like any other; at 60Hz mouse-move that is
   obviously wrong, so C3 will need a throttled or client-side answer.
   Park it until C3, or rule now that hover may be the one blessed piece
   of client-side behavior?
4. **F-11 spelling** (§6) — `click = focus[sym]`, frozen at render?
