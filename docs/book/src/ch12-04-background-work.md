# Slow Work and the Live Loop

A live Crow app is one loop serving every connected browser. Every click
runs a handler on that loop; every update renders views on it. That design
is why reactive state needs no locks — one loop, one writer — but it comes
with a law: **anything slow on the loop freezes everyone**. A handler that
spends two seconds querying a remote database doesn't just delay the person
who clicked; every other client's clicks and renders wait in line behind it.

This is not a Crow quirk. It's the law of every interactive system — the
browser's main thread, Python's asyncio, a phone app's UI thread — and in
most of them, staying responsive is a discipline you must remember. In Crow
it's a verb.

## The frozen version

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

Click refresh, and for two seconds the whole app holds its breath. `crow
check` will tell you so — this is the *blocking in view* finding — and its
fix line teaches the verb this chapter is about.

## later: the kitchen

`later[fn]` runs a named fn off the loop and returns immediately. The fn
does its slow work in the background; the reactive state it writes is
applied back on the loop, one schedule at a time, and every client
re-renders when the results land:

```crow,ignore
fn refresh[]
    status = 'loading'
    later[load_book]
```

The click now takes a millisecond: `refresh` sets `status`, schedules
`load_book`, and returns — your screen shows "loading" instantly and
everyone else's app stays live. Two seconds later the work finishes,
`status` updates, and every connected screen re-renders. Think of the
loop as a waiter and `later` as handing the slow order to the kitchen: the
waiter keeps waiting tables, and the dish comes out when it's ready.

Three rules make this safe, and you get all of them without writing
anything:

1. **State is the only mailbox.** The background fn communicates by
   writing reactive state — no return values, no callbacks, no `await`.
   Its writes are applied between renders, serially, so a view never sees
   half-finished state.
2. **Impatience is harmless.** A second `later[load_book]` while the
   first is still running is skipped. Hammering the refresh button cannot
   stack five queries.
3. **Failure is a value.** If a query inside the fn fails, the state var
   it writes holds an ordinary error value, and a view can render its
   `.reason` — the same error story as everywhere else in Crow.

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
      'refresh' can freeze every connected client — sleep[] blocks the
      live loop (refresh → score_board → sleep)
      fix: schedule it off the loop: later[score_board]

      enter = recheck, s = skip, f = fix (later[]), q = quit > f

  rewrote scanner.co: score_board[] → later[score_board] — rechecking...
  ✓ all good
```

Slow work scheduled through `later` is the sanctioned home for blocking
calls — the checker never flags it.
