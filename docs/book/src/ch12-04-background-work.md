# Slow Work and the Live Loop

A live Crow app serves every reader's page from the server, and each page
has its own loop: a click runs a handler, then the page renders again. Two
kinds of slow work can get in its way — a query a view needs, and a handler
that takes a while — and Crow treats them differently.

## Slow data is the runtime's job

A derivation that queries a database is computed off the render. When its
value is stale, the page renders at once with the value it has, and renders
again when the new value lands; a view still catching up with something
the reader did is dimmed after a moment, so the reader can see that part is
catching up. One that is only behind a moving clock is not dimmed: it is
always one query behind while the clock plays. A click never
waits for a query it did not cause. A query it did cause — the rows its own
handler just inserted — is part of its answer:

```crow,norun
use web, ui, db

book = db.connect['sqlite://book.db']
sym = 'IMRN'

derive trades
    return query[book, 'SELECT id, px FROM trades WHERE symbol = ?', [sym]]

fn buy[]
    exec[book, 'INSERT INTO trades (symbol, px) VALUES (?, ?)', [sym, 1.79]]

view render[]
    return stack[table[trades], button['Buy', click = buy]]

page['/', render]
serve[3000]
```

Pressing Buy answers with the new row already in the table: the handler
wrote `trades`, so the derivation over it is recomputed before that render.
Changing `sym` answers at once with the old rows, dimmed, and the new ones
arrive a query later. You wrote nothing for either.

Before its first value, a derivation that queries reads as an error of kind
`'pending'` with the reason `loading` — a view that renders a failed read
in place shows "loading", and the page appears at once instead of waiting
for its slowest query. `running[trades]` is true while it recomputes, for a
spinner of your own.

Nothing one reader does holds another. Handlers and renders run on worker
threads, never on the threads that carry the sockets; database handles are
pools; a derivation shared by every page is computed once, however many
pages ask for it together. And a write reaches every reader: when one page
inserts a row, every other page that read that table renders again with it.

## Slow handlers

A handler runs off the page's loop too. While it works the page stays live
— the clock ticks, landed values render — and the button that started it
shows it is working, disabled, with nothing written in the program for it:

```crow,norun
use web, ui

status = 'ready'

fn load_book[]
    sleep[2]                 --- stand-in for the slow part: a remote query,
    status = 'updated'       --- a model load, a shell call…

fn refresh[]
    load_book[]

view render[]
    return div[h1['Live book'], p['`status`'],
               button['refresh', click = refresh]]

page['/', render]
serve[3000]
```

Click refresh: for two seconds the button is disabled and marked busy, and
the rest of the page carries on. What waits is the session's *other*
clicks — events are applied one at a time, in order, so a second click
lands after the first handler ends, and a second click on the running
handler is dropped as the double-click it is. `crow check` points this out
— the *blocking in view* finding — because when the reader should be free
to keep clicking while the work runs, there is a verb for it.

## later: the kitchen

`later[fn]` runs a named fn off the loop and returns immediately. The fn
does its slow work in the background; the reactive state it writes is
applied back on the loop, one schedule at a time, and the page re-renders
when the results land:

```crow,ignore
fn refresh[]
    status = 'loading'
    later[load_book]
```

The click now takes a millisecond: `refresh` sets `status`, schedules
`load_book`, and returns — your screen shows "loading" instantly, and the
reader can go on clicking. Two seconds later the work finishes, `status`
updates, and the page re-renders. Think of the loop as a waiter and
`later` as handing the slow order to the kitchen: the waiter keeps waiting
tables, and the dish comes out when it's ready. The schedule is made when
`refresh` commits — a handler that stops never leaves work running.

Three rules make this safe, and you get all of them without writing
anything:

1. **State is the only mailbox.** The background fn communicates by
   writing reactive state — no return values, no callbacks, no `await`.
   Its writes are applied between renders, serially, so a view never sees
   half-finished state.
2. **Impatience is harmless.** A second `later[load_book]` while the
   first is still running is skipped. Hammering the refresh button cannot
   stack five queries.
3. **Failure stops it cleanly.** If a query inside the fn fails at a
   decision, the fn stops as a handler does: nothing it wrote lands, the
   page keeps its last good values, and the notice says why.

For the impatient-user experience there's `running[fn]`, true while a
schedule is in flight — disable the button, show a spinner:

```crow,ignore
view render[]
    label = 'refresh'
    if running[load_book]
        label = 'loading…'
    return div[p['`status`'], button[label, click = refresh]]
```

## later in scripts

The same verb works outside the web layer, because scripts have loops too —
a poll loop that must tick on schedule shouldn't stall while one slow batch
processes. In a script, extra arguments are captured by value and passed to
the fn, and the program waits for background work to finish before exiting:

```crow
fn announce[msg]
    log[msg]

later[announce, 'report ready']
```

```text
report ready
```

One honest subtlety: in the interpreter (`crow run -i`, `crow test`),
`later` runs the work *immediately, inline*. That's not a parity break —
`later` promises the work happens *eventually* and its writes apply
*atomically*, and "immediately" is a perfectly legal schedule. Tests get
deterministic behavior; the compiled app gets true background threads.

## Periodic freshness

Some pages shouldn't wait for a click at all — a dashboard should just stay
current. That's not a loop you write; it's a property you declare on
`serve`:

```crow,norun
use web, ui

status = 'ready'

view render[]
    return div[h1['Board'], p['`status`']]

page['/', render]
serve[3000, refresh = 5, budget = 50]
```

`refresh = 5` re-renders every connected session every five seconds —
render-time queries re-run, so the page tracks the database with zero
lines of update code. If a render is slow, missed ticks are *skipped*,
never stacked. And `budget = 50` is the loop's own heartbeat: any handler
or render over 50ms gets logged with its name, so a freezing loop is
loud in development instead of mysterious in production.

There is no `every[]` and no timer in Crow, deliberately: hand-rolled
poll loops always end up needing an overlap guard, backoff, and shutdown
handling, and every codebase writes them badly once first. The runtime
owns the clock.

## What later is not

`later` is for **responsiveness**; `for` is for **throughput**. Crunching
a million rows is already parallel — that's chapter 6. `later` exists so an
interaction never waits on slow work: remote queries, `shell` calls, model
loads.

And `later` is in-process, best-effort: if the process dies mid-flight,
that work is gone. Refreshing a panel can be redone; placing an order
cannot. Work that must survive a crash belongs in a durable queue — a
design on Crow's roadmap, not a property of `later`.

## The checker knows this chapter

`crow check` detects the frozen version statically — a view or handler
that reaches `sleep`, `shell`, or `input` through any chain of calls gets
flagged with the path, and the interactive loop can apply the fix for you:

```text
[1/1] scanner.co:14 in refresh
      'refresh' holds the session's other clicks until it ends — sleep[]
      blocks it (refresh → score_board → sleep)
      fix: schedule it off the loop: later[score_board]

      enter = recheck, s = skip, f = fix (later[]), q = quit > f

  rewrote scanner.co: score_board[] → later[score_board] — rechecking...
  ✓ all good
```

Slow work scheduled through `later` is the sanctioned home for blocking
calls — the checker never flags it.
