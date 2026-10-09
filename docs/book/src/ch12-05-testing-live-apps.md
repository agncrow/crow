# Testing a Live App

A view is a function from reactive state to a tree. That one fact is why
Crow's UI tests need no browser, no driver, and no fixtures: render the
tree, run a handler, render again, and state truths about what you see.
UI tests are ordinary named `test` blocks (chapter 9), run by `crow test`,
stripped from every build.

Both `crow run app.co` (compiled) and `crow run -i app.co` (the
interpreter, with no compile between an edit and the page) serve it, and
they serve the same page: `crow test --served` drives a set of pages in
both modes with the same clicks and compares what they settle on.

## The harness

Five verbs, valid only inside test blocks:

| Verb | What it does |
|---|---|
| `s = app[view]` | opens a headless session that renders `view` against the program's reactive state |
| `click[s, handler]` | runs an event handler on the session — the same thing a button click does (deferred calls too: `click[s, set_speed[10.0]]`) |
| `change[s, 'var', value]` | what typing into a bound `field` does: sets the reactive variable, so the next render and the next handler see it |
| `html[s]` | the rendered view as one HTML string — for "does the page say *ready*" |
| `elements[s, selector]` | the rendered elements matching a selector, in document order, as records — for *where* and *what* things are |
| `box[s, selector]` | the layout engine's boxes for matching elements — `x y width height overflow` — at the session's viewport (`app[view, width = 390]`) |
| `style[s, selector]` | resolved colour and type tokens — `background color size weight` |
| `fits[s, width]` | true when nothing overflows at that width |

Reactive state reads as fields of the session: `s.status`, `s.min_qty`,
and a derivation as that session sees it: `len[s.orders]`.

A selector is a tag (`'button'`), a class (`'.card'`), both
(`'div.stats'`), or a space-separated chain that must nest
(`'div.controls button'`). Each record has `tag`, `text`, and `class`,
plus every attribute the element carries — `href`, `name`, `src` — as a
key.

## Behaviour: a click changes state

```crow,norun
use web, ui

status = 'ready'
count = 0

fn slow_refresh[]
    count = count + 1
    status = 'updated'

fn go[]
    status = 'loading'
    later[slow_refresh]

view render[]
    return div[h1['Live book'], p['status: `status` count: `count`'],
               button['refresh', click = go]]

test refresh_flow[]
    s = app[render]
    assert['ready' in html[s]]
    click[s, go]
    assert[s.status eq 'updated']
    assert['count: 1' in html[s]]

page['/', render]
serve[3000]
```

The `later[]` call is the interesting part. In production it schedules
`slow_refresh` off the live loop; in a test it runs inline, so the line
after `click` already sees the final state. Background work is
deterministic in tests and concurrent in production — the same code,
verified without waiting or flakiness.

## Structure, attributes, and two-way binding

`html[s]` is one string. For *where* things are — the button inside the
controls panel, two cards in the stats row, the heading first — ask for
elements. For what a link points at, read its attribute. For typing into a
bound field, `change` the variable it is bound to:

```crow
use web, ui

min_qty = 0

fn reset[]
    min_qty = 0

view panel[]
    return div[
        h1['Trades'],
        a['docs', href = '/docs'],
        div[field['Min qty', min_qty], button['Reset', click = reset], class = 'controls'],
        div[div['P&L', class = 'card'], div['Count', class = 'card'], class = 'stats'],
        p['showing trades with qty above `min_qty`']
    ]

test layout[]
    s = app[panel]
    assert[len[elements[s, 'div.controls button']] eq 1]
    assert[elements[s, 'button'][0].text eq 'Reset']
    assert[len[elements[s, '.stats .card']] eq 2]
    assert[len[elements[s, '.stats button']] eq 0]
    assert[elements[s, 'h1'][0].text eq 'Trades']
    assert[elements[s, 'a'][0].href eq '/docs']

test typing_into_the_field[]
    s = app[panel]
    change[s, 'min_qty', 50]
    assert[elements[s, 'p'][0].text eq 'showing trades with qty above 50']
    click[s, reset]
    assert[s.min_qty eq 0]

page['/', panel]
log['panel defined']
```

```text
panel defined
```

Read the `layout` block as a set of claims about the tree: one button in
the controls panel, none in the stats panel, two cards, the heading text,
the link target. Move the Reset button into the stats panel and the fourth
line fails in `crow test` with both sides shown — the kind of mistake that
otherwise survives until someone looks at the page. The second block is
two-way binding end to end: the field's variable changes, the paragraph
re-renders with it, the handler puts it back.

## What a test block sees

A test block sees declarations: functions, views, structs, imports. Of the
top-level assignments only two kinds ride along — literal values, which
are reactive-state defaults the harness reads and handlers write, and
connections (`db.connect[...]`), which are resources a render-time view
needs to exist at all. Everything computed at the top level is invisible,
exactly as it is inside a function, so a view under test cannot depend on
work the program did before `page[]`.

## Failure renders, it doesn't crash

A view renders what it is given, and what it is given may be an error
value (chapter 7). The page shows the failure as `error: the server's log
has why, as <id>` — a reader is told an id, never the reason, which names
what the program reaches — and the server's log has the reason beside the
id. Feed a view one on purpose and check that the failure reaches the
page; the reason itself is yours to check on the value:

```crow
use web, ui

fn latest_price[]
    return error['price feed is down', 'network']

view quote[]
    px = latest_price[]
    return div[h1['AAPL'], p['last: `px`'], class = 'card']

test feed_down_is_visible[]
    s = app[quote]
    assert['the server\'s log has why' in elements[s, 'p'][0].text]
    assert[latest_price[].reason eq 'price feed is down']

page['/', quote]
log['quote defined']
```

```text
quote defined
```

## Time is an input

Inside a test the clock is yours: it starts when the first `app[…]` opens
and moves only when you move it, the way a field changes when you type in
it. A handler that stamps `now[]` stamps the moment you set, and the fact
holds a moment — `moment[0.0]`, the epoch, until a handler moves it:

```crow
use web, ui

stamped = moment[0.0]

fn stamp[]
    stamped = now[]

view clock[]
    return p['stamped `stamped`']

test the_test_sets_the_clock[]
    a = app[clock]
    change[a, 'now', 1000.0]
    click[a, stamp]
    assert[a.stamped eq moment[1000.0]]
    change[a, 'now', moment['2026-07-14T09:30:00Z']]
    click[a, stamp]
    assert[str[a.stamped] eq '2026-07-14T09:30:00Z']

page['/', clock]
log['clock defined']
```

```text
clock defined
```

A click that stops — an error at a decision, a fact, a database call —
evaluates to that error, and the session is exactly as it was before the
click: `r = click[a, apply]`, then `assert[not r.ok]`.

## Two users at once

Two sessions are two independent pages, each with its own reactive
state — exactly what two browser tabs get from the server. That is how
you test the awkward orderings — one person resets while another is
still typing — with no threads to wrangle:

```crow
use web, ui, db

book = db.connect['sqlite://:memory:']
min_qty = 0

fn setup[]
    exec[book, 'CREATE TABLE IF NOT EXISTS orders (id INTEGER PRIMARY KEY, qty INTEGER)']

derive orders
    return query[book, 'SELECT qty FROM orders ORDER BY id']

fn reset[]
    min_qty = 0

fn order[]
    exec[book, 'INSERT INTO orders (qty) VALUES (?)', [min_qty]]

view panel[]
    return stack[
        p['min `min_qty`'],
        p['orders: `len[orders]`'],
        button['Reset', click = reset],
        button['Order', click = order]
    ]

test two_people[]
    alice = app[panel]
    bob = app[panel]
    click[alice, setup]
    change[alice, 'min_qty', 25]
    click[bob, reset]
    assert[alice.min_qty eq 25]
    change[bob, 'min_qty', 10]
    click[bob, order]
    assert[elements[alice, 'p'][1].text eq 'orders: 1']

page['/', panel]
log['panel defined']
```

```text
panel defined
```

Bob's reset is his own: Alice still has 25. What the two share is the
database — the order Bob places is in Alice's list the next time her
page renders. Per-user state is the default; shared state is a table.

## What kind of mistake, which truth

| The mistake | How you catch it |
|---|---|
| Wrong structure — a button in the wrong panel, a missing heading | `elements[s, sel]` with a nested selector |
| Wrong content or formatting | `elements[...].text`, or `… in html[s]` |
| A click doesn't change state, or changes the wrong thing | `click[s, handler]` then `s.field` |
| Typing into a bound field doesn't propagate | `change[s, 'var', value]`, then render again |
| Wrong `href`, missing `name`, wrong class | attribute keys on element records |
| A failed query crashes instead of rendering its reason | render with an error value, assert on the tree |
| Two users' events interleave badly | two `app[]` sessions in one test |
| Background work never lands | `later[]` runs inline in tests — observable on the next line |
| A view blocks the live loop or serves a stale snapshot | `crow check`, statically — no test needed |
| A chart draws the wrong thing | `elements[s, 'rect']` walks the SVG too |

## What this cannot see

- **Pixels beyond the layout model.** Positions, sizes, and colours *are*
  testable now — `box[]`, `style[]`, `fits[]` in the next section — for
  everything built from the layout vocabulary. What they can't see is
  whatever lives in `css/app.css` outside that vocabulary, and glyph
  rendering, which stays the platform's. The design and its limits:
  `docs/ui-engine-design.md`.
- **Routing.** That a page is registered on the right path is not
  testable through a session today.
- **Remote data.** `fetch` and `poll` sources cannot be stubbed inside a
  test yet; put the logic that depends on them in a function that takes
  the data as a parameter, and test that.

## Against the compiled program

`crow test` runs a block in the interpreter, and the apps it opens with
it. `crow test --compiled app.co` runs the same blocks with every app they
open compiled: the program is built once, each run that opens an app
starts it fresh, and `click`, `change`, `html`, `elements`, `box`, a
session's facts and derivations all reach the compiled program. The block
itself, its truths and the functions it calls directly, still runs in the
interpreter. So a handler the compiled program runs differently fails the
test that exercises it, with the same message. A handler a test clicks
with arguments (`click[a, begin[1000.0, 100.0, 1050.0]]`) is typed by
them when nothing in the program types it. Blocks that open no app run as
they always do, and nothing is built for them.

```text
$ crow test --compiled sim/clock.co
  ✓ 2 blocks pass, the apps they opened compiled (1 run) — 0 truths, 1 law, 51 inputs run
```

`crow test` runs these blocks; `crow check` runs them plus the live-loop
hazards; `crow run` and `crow build` ignore them. Next: the two things
every dashboard needs that you haven't written yet — a chart, and a page
that looks right.

## The baseline: what changed, without writing a test

Every `crow check` derives what the program does and keeps it beside the
source, under `.crow/baseline/`: for each handler, every path through it
as a condition over facts and inputs with the facts it writes; for each
view, its tree of elements, controls and sub-views under their guards;
for each fact, who writes it and who reads it. The first check records
it. Every check after diffs against it.

```text
$ crow check
  baseline — 95 unchanged, 1 changed (.crow/baseline/main)
  [1/1] sim/session.co in start_session
        handler start_session changed (sim/session.co, in what you edited):
          ~ …when (not (v2 «lte» → «lt» v1)) and v1.ok and v2.ok:
        fix: if intended, accept with a and the baseline moves; otherwise fix the cause — reaches page /
```

Locals and temporaries are substituted and paths are sorted, so a
rename, a reordering or a helper moved to another part changes nothing.
A guard, a write, an effect or a wiring that changed shows as a diff,
with the file it came from and the views and pages it reaches; a diff
reached from a file you did not edit is the headline. Accept with `a`
and the baseline travels with your change; without a terminal, an
unaccepted change fails the check, so CI blocks on it. Nothing runs and
nothing is written: the artifacts come from the source alone.

Each page also has its **states**: every state the handlers can bring it
to, from the declared facts, with each control said once with when it is
offered.

```text
states / (board_page_render)
  button 'Sign in' → board_door_log_in: when reader = 'nobody'
  button 'Attach' → board_cards_attach: when board_cards_card_id = * · reader = 'someone'
  ...
```

A fact takes the few values the program gives it — a literal, or `*`
for some other — and `reader` is the signed-in reader. A button that
becomes reachable where it was not, a field left on screen after its
session ended, is that control's line changing.

And each page has its **layout**: every state laid out by the engine at
desktop and phone widths, with its height and anything that runs past
the width (`390: 185 high, past the width: button 'Sign in' by 181`), so
a row that no longer fits a phone shows in the check.

## The app's book

`crow book`, run inside the app, serves a book derived from the source on
every start: the app and its parts, each page as a composition tree with
a wireframe per state at desktop and phone widths, a chapter per part
with its facts, derivations, handlers as paths, views with their trees
and wireframes, and tests as steps, then the data wiring and the changes
since the accepted baseline. The wireframes are the layout engine's
boxes over the view's tree with placeholders, so nothing runs and no
database is touched.

```text
$ crow book
crow book: the app's book is written to .crow/book/site
crow book: http://localhost:3101/  — the app's book (Ctrl-C or `crow stop book` to stop)
```

The book serves next to the app, on its `serve` port plus one; `--port`
chooses another. `crow stop book` stops it from anywhere in the project.
