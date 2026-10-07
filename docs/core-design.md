# The core, design round: three meanings

*Decided 2026-09-07 as stage 1 of the programme in
`core-rewrite-notes.md` §9. This record is written in the spec's voice
so that its sections move into `language-spec.md` when stage 4 builds
them; until then the spec describes what runs today, and this file
describes what will. Where the two disagree, the spec is right about
today and this file is right about the direction.*

Three changes to what a Crow program means. None changes the text of a
handler; each changes when the world moves, what happens when a
handler meets an error, and what time a handler sees.

1. **A handler is a transaction.** Its writes and effects land when it
   ends, as one, or not at all.
2. **A handler that meets an error stops and reports.** No fact ever
   holds an error a handler wrote; the reason reaches the reader.
3. **One moment per event.** `now[]` is the moment the event began,
   everywhere inside it.

The worked cases are finch's clock and its skip handler, in §4.

## 1. A handler is a transaction

### 1.1 The rule

A handler runs against a working copy of the session's facts. It reads
the facts, the derivations and the stores as it goes and sees its own
writes at once, as a helper it calls does today. When the handler ends,
its writes land as one, the session's version advances by one, and
every derivation that read a changed fact recomputes on its next read.
When the handler stops (§2), nothing lands.

This is the rule `later` already has for the function it schedules
("its writes are applied on the loop between renders, one schedule at a
time"). The change is that every handler gets it, not only the
scheduled ones, and that a stop lands nothing instead of "the facts it
wrote so far".

### 1.2 Effects: answered or deferred

An effect is a call that reaches outside the session's facts: `query`,
`exec`, `insert`, `transact`, `read`, `write`, `fetch`, `shell`,
`mail`, `later`, `notice`. One rule decides when each runs:

> **An effect whose answer the handler uses runs when asked. An effect
> whose answer the handler does not use runs when the handler commits,
> in program order.**

"Uses" is decided from the text: the call's value is assigned, compared,
passed, interpolated or returned. The checker already computes this for
the baseline, and the same computation drives the runtime.

So `r = query[...]`, `n = shell['wc -l x']` and `row = exec[book,
'INSERT ... RETURNING id']` answer at once, because the handler needs
them to continue. `exec[book, 'UPDATE ...']` with no use, `insert[book,
'trades', f]`, `mail[...]`, `notice['saved']` and `later[reload]` are
held and performed at commit, and a stop discards them.

Two consequences follow and are stated rather than hidden:

- **A store transaction, one per connection.** The first write a
  handler makes through a connection begins a database transaction on
  it; every later read and write on that connection is inside it;
  commit at the handler's end, roll back on a stop. `RETURNING`
  answers inside it. A handler that writes two connections commits
  them in program order at the end; the language does not promise
  atomicity across stores, and `crow check` reports a handler that
  writes two.
- **An answered effect is not undone by a stop.** A shell command
  whose status the handler reads has run; a stop afterwards cannot
  recall it. The rule makes this visible: what the handler needed an
  answer to has happened, what it did not is discarded.

### 1.3 What the reader sees

A render never sees a handler halfway. A session renders between
events, from committed facts. Two sessions never see each other's
working copies. A shared derivation recomputes from the committed
store.

### 1.4 The notice

`notice[text]` is an effect held to commit, so a handler that stops
does not leave a message that says it succeeded. The runtime keeps one
notice per session, with the moment it landed: `notice` reads as a
record with `text` and `at`, a derivation the runtime provides, so a
view writes `toast[notice.text]` and the runtime fades it. The program
keeps no flag and clears nothing. A handler's stop report (§2) is
delivered through the same notice. `toast[..., open = var]` stays for
layers the program controls.

### 1.5 The harness and rehearsal

`click[a, h]` runs `h` as one transaction on the session: on commit the
session's facts are the handler's; on a stop `click` evaluates to the
error and the facts are as before. Effects run against whatever the
part is bound to: today the real store, under capabilities a double.
Rehearsal (app-baseline-design.md, B3) is this rule with every effect
recorded instead of performed, which is why it falls out of the
transaction and needs no stubs.

### 1.6 Both modes

The interpreter and the compiled program keep the same working copy
and the same commit point. The `flush_before` / `reload_after`
write-through that the compiled handler does today around helper calls
is replaced by the working copy, which both modes share in
`crow_runtime`.

### 1.7 Spec changes this makes

- §10 Errors in Conditions: "the handler stops, the facts it wrote so
  far land" becomes "the handler stops and nothing lands".
- §15 `later`, point 1: unchanged in meaning, now stated as the rule
  for every handler; point 3 changes under §2.
- web, "Handlers and their helpers": the working copy and the commit
  point; the effect rule; the notice.
- Design Note: *Effects Commit When the Handler Ends* (below, §5).

## 2. A handler that meets an error stops and reports

### 2.1 The rule

Errors stay values; nothing about computing with them changes. What
changes is where a handler is refused:

> **A handler stops when an error value reaches a decision, a fact, or
> an effect.** It commits nothing, and the reason is reported as the
> session's notice and in the log: `next_gate_any stopped: clickhouse
> query error: …`.

A decision is an `if` or `while` condition, as today. A fact is any
assignment to reactive state. An effect is any argument of an effect
call. Anything else, a local that holds an error and is later guarded,
computes as it always has: `.ok`, `.reason`, `.kind` and `.trace` never
propagate, so `if not r.ok` remains the way to do something other than
stop.

### 2.2 What it deletes

The idiom every handler carried:

```
r = next_gate[...]
if not r.ok
    session.say['next in gate failed: `r.reason`']
    return
```

becomes `r = next_gate[...]`, because the next line to decide on `r`
stops the handler with the same reason, and the reader sees it through
the notice. A handler keeps the guard when it wants a different outcome
than stopping, and only then.

### 2.3 Facts never hold an error

A fact is a stored value the reader acts on; a derivation is a belief
and may be wrong. Under this rule a handler cannot write an error into
a fact: the write stops the handler and the fact keeps its last value.
Derivations keep today's behaviour and hold the error they computed,
which a view renders in place.

This changes `later` §15 point 3, "a failing query inside the fn lands
in state as an ordinary error value". The fn stops, the state keeps its
last value, the notice says why. The page keeps its table instead of
losing it to an error string, and the reader learns what happened.

### 2.4 Functions, scripts, views, derivations

Unchanged. A function returns the error to its caller at a decision; a
script halts at a decision; a view renders `error: reason` in place; a
derivation holds the error.

### 2.5 Spec changes this makes

- §10: the handler paragraph in *Errors in Conditions* becomes this
  rule; a new paragraph under *Auto-Propagation* for facts and effects.
- §15 point 3 as above.
- Design Note: *A Fact Never Holds an Error* (below, §5).

## 3. One moment per event

### 3.1 The rule

> **`now[]` is the moment the current event began.** A handler and
> every helper it calls, a render and every view and derivation in it,
> a scheduled function and its work, each see one moment. In a script
> `now[]` is the clock at the call, as today.

Events are what the runtime starts: a click or change from a client, a
render, a `refresh` tick, a `later` schedule. Time advances between
events, never inside one.

### 3.2 What it fixes

- `base, anchor = head, now[]` writes two readings of the same clock
  today, microseconds apart, on every re-anchoring; the play-head
  drifts by that much each time. Under the rule they are one moment.
- Two inserts in one handler stamped with `now[]` get one stamp, which
  is what a transaction means; an id made from `now[]` was always
  wrong and the store's `RETURNING` is the id.
- A test can set the moment. The harness clock starts at the moment
  `app[view]` opened and moves only when the test moves it:
  `change[a, 'now', t]` sets it, the same verb that sets any bound
  variable, because time is an input.
- Derivations already treat `now[]` as an input (rule 4 in
  *Derivations*); handlers now do too, so a derivation and the handler
  that reads it agree on the moment.

### 3.3 What it does not do

It does not make the play-head a value over time; the clock part still
re-anchors, exactly now. That is the later round in
`core-rewrite-notes.md` §2 (5). `captured[x]` is unchanged: a read is
stamped with the event's moment.

### 3.4 Spec changes this makes

- Moments: `now[]` is "the current moment" in a script and "the moment
  this event began" in a live program.
- §16 harness: `change[a, 'now', t]`.
- Design Note: *One Moment Per Event* (below, §5).

## 4. Worked cases

### 4.1 The clock

`sim/clock.co` today:

```
fn toggle_play[]
    if playing
        base, playing = head, false
    else
        anchor, playing = now[], true

fn set_speed[s]
    base, anchor, speed = head, now[], s
```

Unchanged in text. What changes: `head` is derived from `now[]`, so
`base = head` and `anchor = now[]` in `set_speed` now use the same
moment and the head does not drift on a speed change. The handler's
three writes land as one; a render never sees `base` moved with
`anchor` still old.

`begin` is called by the session part's `start_session`, which also
inserts the session row. Today, if the insert fails after `begin`, the
clock has been re-anchored and the window set, the session id is zero,
and the page shows a transport for a session that does not exist. Under
§1 the insert's failure stops the handler; nothing lands; the notice
reads `start_session stopped: …`.

### 4.2 The skip

`sim/nav.co` today, four paths:

```
fn next_gate_any[]
    if session.session_id eq 0
        session.say['start a session first']
        return
    b = band
    s_after = '`clock.head`'
    r = next_gate[ch, session.day, s_after, session.dataset_ids, b[0], b[1], session.skipped]
    if not r.ok
        session.say['next in gate failed: `r.reason`']
        return
    if len[r] eq 0
        session.say['no symbol enters the gate after the play-head']
        return
    session.focus[r.symbol[0]]
    clock.jump[float[r.at[0]]]
    when = fmt[clock.head, style = 'time', tz = 'America/New_York']
    session.say['`r.symbol[0]` enters the gate at `when`']

fn skip_symbol[]
    session.skipped = concat[session.skipped, [session.sym]]
    next_gate_any[]
```

Under §1, §2 and the notice, three paths, one of them the session
guard that the checker's state machine can later prove unnecessary:

```
fn next_gate_any[]
    if session.session_id eq 0
        notice['start a session first']
        return
    r = next_gate[ch, session.day, '`clock.head`', session.dataset_ids, band[0], band[1], session.skipped]
    if len[r] eq 0
        notice['no symbol enters the gate after the play-head']
        return
    session.focus[r.symbol[0]]
    clock.jump[float[r.at[0]]]
    notice['`r.symbol[0]` enters the gate at `clock.clock_str`']

fn skip_symbol[]
    session.skipped = concat[session.skipped, [session.sym]]
    next_gate_any[]
```

If the query fails, `len[r]` is a decision on an error: the handler
stops, `session.skipped` is not changed (the skip did not happen, which
is right: the reader can press it again), and the notice reads
`skip_symbol stopped: clickhouse query error: …`. Today the skip lands,
the hop does not, and the status says "next in gate failed".

`session.say` and its two facts, `status` and `noticed_at`, go; the
seventeen handlers that wrote them write one effect each.

The four hop handlers together: 16 paths today, 10 after, 6 once the
guard is proven. Across finch's 28 handlers, the 37 failed-read paths
go and the 17 notice writers lose two writes each.

## 5. Design Notes

### Effects Commit When the Handler Ends

**What was considered:** leaving effects immediate (today); making
every effect deferred, including reads (rejected: `RETURNING`, a shell
status, a query the handler branches on all need an answer, and a
handler that cannot read is not a handler); two-phase commit across
stores (rejected: three databases with three transaction models cannot
promise it, and pretending would be the worst kind of promise).

**Why this design:** one rule, decidable from the text, that a reader
can apply without the runtime: needed the answer, it ran; did not, it
waits. Rehearsal, the baseline's runtime counterpart, undo and replay
all read the same held list. Elm's commands and Redux's thunks return
effects as values but make the handler write them as data; here the
handler's text is unchanged and the runtime does the holding.

### A Fact Never Holds an Error

**What was considered:** errors landing in facts (today, and `later`'s
stated rule); a per-fact choice.

**Why this design:** a fact is what the reader acts on. An error in a
fact is a page that lost its data to a string, and a handler that
half-succeeded. Stopping keeps the last good value and tells the reader
why; derivations, which are beliefs, keep holding errors so a page with
one failed part is still a page.

### One Moment Per Event

**What was considered:** a wall-clock read per call (today); a
frozen clock for the whole process (rejected: `refresh` and derivations
over `now[]` need it to move); a `clock` capability only (deferred:
capabilities bind what a part needs; the per-event moment is what the
binding delivers).

**Why this design:** an event is one thing that happened at one time.
Two readings inside it are two answers to one question. Tests set it
with the verb that sets any input, because it is one.

## 6. Order of building

These three land together in stage 4 (the IR and the runtime's working
copy), with a parity program each, the spec sections above moved in,
the book's live chapter updated, and finch rewritten to the new rules
with its baseline re-accepted: the diff of that acceptance is the
measure of what the three rules delete.
