# derive — design round

**Status: design round (2026-09-07). Nothing implemented.** This closes
the half of the July concurrency round that stayed open: "store facts,
declare invariants, derive beliefs" shipped its invariants work (`crow
check --db`, `claim`-shaped fusions) and its periodicity ruling (`serve
refresh`), and left `derive` as a direction: *beliefs recomputed on read,
unassignable; a future `cache[TTL]` as reconciliation by time*. The finch
simulation page (finch-sim-design.md §14) is the driving application; the
feature is general.

---

## 1. The problem, as finch has it today

`render[]` in finch runs four queries every second: the day's bars for a
symbol, the tape statistics at the play-head, the scanner rows at the
play-head, and the session's trades. Three of them depend only on the
symbol and the day, which change a few times an hour. The language has
no place to put a value that is *computed from* facts, so the view does
the computing, and `refresh = 1` recomputes everything.

The checker knows this is wrong — remote-query-in-view, stale-snapshot,
stale-loop-read are all warnings about the same missing form — and can
only warn, because there is nothing to point at. The dashboard's
render-time queries "already embody" derivation, the July notes said. They
embody it by hand.

The doctrine names the fix. Facts are stored (top-level variables written
by handlers). Beliefs are derived. Crow has the first and not the second.

## 2. What others do

| System | The derived value | Recomputed when | What we take |
|---|---|---|---|
| Spreadsheets | a cell with a formula | a referenced cell changes | read like a fact; dependencies are what the formula read |
| Observable | a notebook cell | any cell it references changes; the runtime knows the graph | implicit dependencies, minimal recomputation |
| Convex | a query function subscribed by the client | a mutation touches something its read set covered | database writes invalidate; per-read-set precision |
| Salsa, Incremental, Adapton | a memoized function | an input's version changed; early cutoff when the value is equal | versions, cutoff, composition of derivations |
| Solid, Svelte 5 | a signal-level computation | a signal it read changed | too fine for a server-rendered tree; the diff gives us granularity already |
| Elm | `Html.Lazy` over a value | the value's identity changed | memo by hand; derivations are the automatic form |
| Phoenix LiveView | an assign set in a handler | the handler runs | derivation done in handlers; exactly the manual work we remove |

The old `use web.store` sketch in web-framework.md (`store`, `computed`,
`watch`, signals with subscriber lists) was the same idea in a
JavaScript-shaped costume. It was never implemented; this round deletes it.

## 3. The design

### 3.1 The form

A derivation is a top-level definition with a body, like a view, and no
parameters ever:

```
derive bars
    b = query[ch, 'SELECT ... WHERE symbol = ? AND date = ?', [sym, day]]
    b = with[b, 'rsi', rsi[b.c, 14]]
    return with[b, 'cvd', cvd[b.c, b.vol]]

derive head
    return clock_at[base, anchor, speed, playing, win_end]

derive rows
    return rows_at[ch, day, head, universe.dataset_id]
```

It is read like a fact, by name, anywhere a fact can be read — views,
handlers, tests, other derivations:

```
view session[]
    return chart[bars, marker = head, points = fills, seek = jump, ...]
```

It is never assigned. `bars = x` in a handler is a compile error that
names the rule: *bars is derived from sym and day; change those.*

One form only. A one-line derivation is a two-line block; there is no
`derive x = expr` shorthand, for the same reason `fn` has no expression
form.

### 3.2 What a derivation depends on

Its **read set**, computed statically by the front end and shared by both
modes:

- facts and derivations it reads, transitively through the functions it
  calls (the wave scheduler's read-set analysis, which exists);
- `now[]`, if it or anything it calls reads the clock;
- the tables its queries read, from the SQL literals (the parser `crow
  check --db` already uses); `read[]` files likewise by path when literal.

Static analysis over-approximates: a function that *might* read a fact
counts as reading it. Both modes use the same sets, so recomputation
behavior is identical by construction. Where over-approximation hurts in
a real program is the one open unknown of this design, and finch is the
test.

### 3.3 When it recomputes

A derivation is **dirty** when, since it was last computed:

1. a handler wrote a fact in its read set;
2. a derivation in its read set was recomputed and its value changed
   (early cutoff: an equal value marks nothing downstream);
3. an `exec` in this app wrote a table in its read set;
4. it reads `now[]` and a tick of `serve refresh` has passed;
5. its freshness window elapsed (3.4).

A dirty derivation recomputes **on read**, once per render, not eagerly:
a handler that writes five facts triggers one recomputation of each
downstream derivation at the next render, not five.

`refresh` keeps its meaning and gains its explanation: it is the clock,
and `derive` is what the clock moves. Nothing else recomputes on a tick.

### 3.4 Freshness for sources this app does not write

ClickHouse bars are written by another system. Nothing in this app can
invalidate them, so a derivation over them declares how old it may be:

```
derive bars[fresh = 300]
    ...
```

The header takes named arguments the way `serve` and `test` do. `fresh`
is seconds. **The default is `fresh = 0`: as fresh as the loop**, which
is today's behavior — a derivation over an external source recomputes
every refresh tick unless told it may be older. Nothing silently goes
stale by default; declaring `fresh` is the relaxation. This is the July
`cache[30s]`, renamed to say what it promises rather than what it does.

`captured[bars]` is the moment the derivation last ran, so F-2's
arithmetic (`now[] - captured[bars] gt 60.0`) works unchanged.

### 3.5 Shared and per-session

Live state is per connected session. A derivation whose read set contains
a session fact is computed per session. A derivation whose read set has
only sources and the clock — the scanner's live config, the latest day
with data — is computed once for the app and shared. The split is
inferred from the read set; no annotation.

*As shipped:* a "session fact" is a fact a session writes — assigned by a
handler or a helper it calls, or bound to an input (`field`, `slider`,
`checkbox`, `toggle`, `radio`, `tabs`, `select`) — or one the runtime
writes for each session: a fact a page's address carries (a `:name` of
its path, a name of its `url = […]`) and a `fetch`/`poll` source, whose
answers land in each session (both had made a derivation the app's until
October 2026: one session's address, or the source's first answer, for
all). A top-level constant or a database handle does not make a
derivation per-session. The split is a fixpoint over the read sets (a
derivation that reads a per-session one is per-session), computed by
`derive::read_sets` for both modes — each per-session one keeps why — and
printed by `crow check`. Table versions are app-wide, so a write in one
session invalidates every session's derivations over that table.

### 3.5a Routes read what the app holds (October 2026)

A route has no session, so its handler reads the derivations the app
holds and is refused one a session holds (the refusal says what makes it
the session's). A `use web` program that declares a derivation holds
state as a ui program does: its top-level names are facts, one value
each, which its derivations read. The pieces, one per mode where a mode
needs its own and shared everywhere else:

- *Who reads once.* The lowering binds each derivation a route's handler
  names to a local of its name as the handler starts (`lower_fn`,
  `route_derived`); both backends run that one read, and a read inside a
  loop is a local's.
- *Where the value lives.* `crow_web::live`: the compiled program holds
  its state as declared (`hold_app`, after main sets the facts) and a
  route reads through `app()`; the interpreter keeps one per served
  program (`app_state_from`), its derivations named with the program's
  tag. Both read `SHARED_DERIVED` and `TABLE_VERSIONS` by the rules of
  §3.3, with the app's read a moment of its own: what moves with the
  clock, or is as fresh as the loop, is computed for each request.
- *One computation for a burst.* A shared derivation computed for a
  reader who waits (a handler's read, a route's) is a flight: readers
  arriving with the same inputs wait for its value (`FLIGHTS`), and a
  stop is kept by nobody.
- *A route's writes.* Taken as its handler ends (`__CrowWrites` compiled,
  `route_answer` interpreted) and published for the app
  (`publish_writes`), which wakes the pages over the table; a program
  without derivations lets them go. The thread's record of writes had
  grown with every write a route made.

### 3.6 Errors

A derivation whose query fails holds the error value. Readers see `.ok`
and `.reason` like any fallible value; a view renders the reason in
place. A derivation does not fail its readers — the same rule views
follow.

### 3.7 Keyed children

Same round, same runtime: a list child rendered from data (`rows = for s
in past` then `stack[rows]`) is diffed by position today, so a deletion
in the middle patches every row below it. A row view whose root carries
an identity is matched by it; the patcher moves, inserts and removes
instead of rewriting. This is what makes a derived list cheap to show.

*As shipped:* the identity is the HTML `id` attribute, not a new `key`
word — `key` already names the column a `table[…, click = h]` sends, and
the browser's own notion of element identity is `id`, which both
renderers already pass through. The live client morphs the DOM in place
(`morphKids` in crow_web's client script): children with an `id` match
by `id`, the rest by position, attributes and form-control state are
synced on the surviving node, and a chart's `svg` is swapped whole when
it changed. A node that survives keeps focus, caret and scroll, which
retired the save-and-restore of the focused input.

### 3.8 The harness

`app[sim]` holds derivations like facts; `click[a, start_session]` marks
what the handler dirtied; `html[a]` recomputes on read. A test can assert
on a derivation directly: `assert[len[rows] gt 0]`. Scenarios for the
blueprint chapter fall out of the same mechanism.

## 4. What this deletes and changes

- `web-framework.md`: the `store` / `computed` / `watch` section, gone.
- remote-query-in-view and stale-snapshot: from warnings to teaching
  errors with a fix — *move the query to a `derive`*. `crow fix` gets the
  applier (the tier-2 plumbing the checker doc lists as pending).
- The blessed "sqlite in render" idiom stays a warning, not an error: a
  cheap local query in a view is not wrong, only unnamed.
- finch: no query inside any view.

## 5. Worked example: the simulation page

```
--- sim/clock.co
base = 0.0
anchor = 0.0
speed = 1.0
playing = false

derive head
    return clock_at[base, anchor, speed, playing, win_end]

fn set_speed[s]
    base, anchor, speed = head, now[], s

--- sim/tape.co
derive bars[fresh = 300]
    b = query[ch, '... WHERE symbol = ? AND date = ?', [sym, day]]
    return with[with[b, 'rsi', rsi[b.c, 14]], 'cvd', cvd[b.c, b.vol]]

--- scanner.co
derive universe
    return universe_for[pg, day]

derive rows
    return rows_at[ch, day, head, universe.dataset_id]

--- sim/trading.co
derive trades
    return query[book, 'SELECT ... WHERE session_id = ?', [session_id]]
```

A press of **10x**: `set_speed` writes `base`, `anchor`, `speed`. `head`
read them: dirty. `rows` read `head`: dirty. `bars` read `sym` and `day`:
untouched. `trades` read `session_id`: untouched. The next render
recomputes `head` and `rows`, re-renders the views that read them, and
the client receives one attribute on the chart marker and a few table
rows. A **Buy**: `buy` executes an insert into `trades`; the `trades`
derivation read that table: dirty; the blotter re-renders. A tick with
nothing pressed: `head` (it reads `now[]`) and therefore `rows`; nothing
else.

## 6. Parity

The read sets are computed once, in the front end, and both modes obey
them; the interpreter carries a dirty flag and a version per fact, the
compiled `CrowLiveState` the same in generated code. Parity programs
cover derivations over facts and over sqlite tables; `now[]`-driven
recomputation is covered by the harness, where the clock is stepped, not
by wall time.

## 7. Phases

| Phase | Lands | Proof |
|---|---|---|
| D0 | `derive` in the parser, lowering, validator (unassignable, no params), static read sets | unit tests; `crow check` shows a derivation's read set |
| D1 | interpreter runtime: versions, dirty-on-read, harness support | harness tests: a click recomputes exactly the dirtied derivations |
| D2 | compiled runtime in `CrowLiveState`; parity corpus | parity program with derivations over facts and sqlite |
| D3 | `fresh`, table invalidation from SQL literals, shared-vs-session | finch's bars recompute once per symbol; the blotter after a buy |
| D4 | keyed children in the diff | a deleted middle row patches one node |
| D5 | the checker's warnings become errors with a `crow fix` applier; the web-framework section deleted; spec and book | `crow test --docs`, `--book` |

finch moves to derivations in D3, as parts (finch-sim-design.md §14).

*Status, Sept 7 2026:* D0–D2 shipped in 45f27fd (`derive`, read sets,
both runtimes, harness, spec, book ch12-08). D3's `fresh` and table
invalidation shipped with them; the shared-vs-session split, D4 (`id`
identity in the client morph) and D5 (stale-snapshot and
remote-query-in-view are errors in `crow check` with `d = derive`
rewrites; the web-framework section is gone) shipped together next.
finch's move to parts is the remaining item.

## 8. Open questions for this round

1. The header: `derive bars[fresh = 300]` reads as a call with named
   arguments, which is what `serve` and `test` do. The alternative is a
   line inside the body. Which?
2. The default freshness. `fresh = 0`, every tick, is safe and matches
   today. `never` for sources this app writes is implied by the write
   rule. Is there a case for a different default on remote sources?
3. May a handler read a derivation? Proposed yes (`set_speed` reads
   `head`). It reads the value as of the handler's start.
   *Decided otherwise, 2026-09-22:* the spec's rule 1 holds inside a
   handler too — a read after a write to what the derivation reads, the
   handler's own or a helper's, sees the write. The interpreter always
   did; the compiled handler read each derivation once at its start, and
   finch's hops announced the time they jumped from. It now flushes the
   handler's writes and reads the derivation again before each statement
   that reads it (the memo makes an unchanged one free).
4. Early cutoff on frames: content equality is a full scan. Proposed:
   cutoff for scalars and strings in v1, frames always propagate.
5. Should a derivation over a source this app writes *also* accept
   `fresh`, for tables other systems write too? Proposed yes; the two
   rules combine with "whichever first".
6. The word. `derive` is the July word and the doctrine's verb. `belief`
   was the noun there; `derive` is what you write.
