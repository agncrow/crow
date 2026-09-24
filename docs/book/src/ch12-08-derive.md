# Derivations

Facts are stored; beliefs are derived. A live page holds a few facts —
the symbol, the day, where the play-head was anchored — and shows many
things computed from them: the bars for that symbol, the rows at the
play-head, the trades of this session. In most frameworks the view
fetches and computes those on every render, or an effect fetches them
and stores a copy that goes stale. Crow has a form for them.

## A value that follows what it reads

```crow
use web, ui

qty = 100
price = 1.79

derive cost
    return qty * price

fn more[]
    qty = qty + 100

view render[]
    return stack[p['`qty` shares cost `cost`'], button['More', click = more]]

test cost_follows_qty[]
    a = app[render]
    assert[contains[html[a], 'cost 179']]
    click[a, more]
    assert[contains[html[a], 'cost 358']]

page['/', render]
```

`derive cost` is read like a fact and never assigned — `cost = 5` in a
handler is an error that tells you to change `qty` or `price` instead. It
is recomputed only when something it read changed: the handler wrote
`qty`, so the next render recomputes `cost`; a render where nothing
changed serves the cached value.

A helper that only reads state is a derivation too. A `fn band[]` that
returns the config's price band is refused — a function sees its
parameters and nothing else — and `derive band` is what it was all along:
a value that follows the config, read by the handlers as `band`.

## What a derivation reads is decided from the source

Both execution modes obey the same read sets: the facts and other
derivations a body names, transitively through the functions it calls
and inside interpolated strings; `now[]`; and the tables its queries
read. There is nothing to declare.

```crow
use web, ui, db

conn = db.connect['sqlite://:memory:']
sym = 'IMRN'

fn setup[]
    exec[conn, 'CREATE TABLE IF NOT EXISTS trades (id INTEGER PRIMARY KEY, symbol TEXT, px REAL)']

derive trades
    return query[conn, 'SELECT id, symbol, px FROM trades WHERE symbol = ?', [sym]]

derive summary
    return '`sym`: `len[trades]` trades'

fn buy[]
    exec[conn, 'INSERT INTO trades (symbol, px) VALUES (?, ?)', [sym, 1.79]]

view render[]
    return stack[p[summary], button['Buy', click = buy]]

test a_write_recomputes_the_query[]
    a = app[render]
    click[a, setup]
    assert[contains[html[a], 'IMRN: 0 trades']]
    click[a, buy]
    assert[contains[html[a], 'IMRN: 1 trades']]

page['/', render]
```

`trades` reads the table `trades`; the handler `buy` writes it, so the
derivation is recomputed on the next read and `summary`, which reads
`trades`, follows. A derivation whose value did not change — an equal
number, string or bool — cuts the chain: nothing downstream recomputes.

## Freshness for sources you don't write

A table another system writes — a day's bars in ClickHouse — cannot be
invalidated by anything in your program. A derivation over it says how
old it may be:

```crow,norun
use web, ui, db

ch = db.connect['clickhouse://localhost:9000/market']
sym = 'IMRN'
day = '2026-09-04'

derive bars[fresh = 300]
    return query[ch, 'SELECT ts, o, hi, lo, c, vol FROM bars WHERE symbol = ? AND date = ?', [sym, day]]

view render[]
    return table[bars]

page['/', render]
serve[3000, refresh = 1]
```

The default is `fresh = 0`: *as fresh as the loop*, recomputed every
`refresh` tick, which is exactly what a query inside a view did before.
Declaring `fresh` is the relaxation, never the other way round. A
derivation that reads `now[]` recomputes every tick too, which is how a
play-head derived from the clock stays honest.

A recompute never holds the page. A derivation that queries is computed
off the render: the tick renders at once with the value it has, and again
when the new value lands, and until its first value it reads as "loading"
(chapter 12.4 has the rules). In `crow test` every event settles before
the next line, so a test always reads the landed value.

## One value for the app, or one per session

Who holds a derivation is inferred from what it reads. A fact a session
writes belongs to it — assigned by a handler, or bound to an input the
reader types into: their own symbol, their play-head — so a derivation
that reads one is computed per session. A
derivation over constants, tables and the clock alone is the same for
everyone, so it is computed once for the app and every session reads
it: the day's universe, the latest bar in a table, a scanner's config.
Database handles are not facts, and neither are `fetch`/`poll`
bindings, so `query[conn, …]` over a table nobody's handler touches is
shared. A table write in any session reaches every session's
derivations over that table: when one reader buys, the blotter of
another reader watching the same book follows at once — the write wakes
every page that read the table, with no refresh tick and no code.

`crow check` prints the read set and the holder of each derivation, so
the split is never a guess:

```text
  derivations — what each reads, and who holds it:
    bars: reads day, sym · tables bars (external, fresh 300s) — per session
    universe: reads day · tables datasets — shared by every session
```

## Rows that keep their place

A list rendered from data is patched by position unless its rows say
who they are. Give a row's root an `id` built from its item's key and the
live client matches it to the same row in the previous render: a
deletion in the middle removes one node, a reorder moves nodes, and
nothing else on the page is rebuilt. Elements without an `id` still
match by position.

```crow
use web, ui

items = [1, 2, 3]

view item_row[i]
    return p['item `i`', id = 'r-`i`']

derive rows
    out = for i in items
        item_row[i]
    return out

fn drop_middle[]
    items = [1, 3]

view render[]
    return stack[stack[rows], button['Drop', click = drop_middle]]

test rows_carry_their_identity[]
    a = app[render]
    assert[contains[html[a], 'id="r-2"']]
    click[a, drop_middle]
    assert[not contains[html[a], 'id="r-2"']]
    assert[contains[html[a], 'id="r-3"']]

page['/', render]
```

Because nodes survive a render, so does everything the browser keeps on
them: the field a reader is typing in keeps its focus and caret while
the rest of the page updates around it.

## Where the queries went

Before `derive`, the finch simulation's view ran four queries a second,
three of which changed a few times an hour. `crow check` warned about
it — remote-query-in-view — and could only warn. With `derive`, no view
queries anything: the bars are a derivation over the symbol and the day,
the rows at the play-head over the day and the clock, the trades over
the session. A tick recomputes the play-head and the rows; a symbol
change recomputes the bars; a buy recomputes the trades. Nothing else
moves, and nobody wrote a line about caching.

The checker now teaches the form instead of warning about its absence.
A top-level variable assigned from a query and read by a view — a
launch-time snapshot served forever — and a query against a remote
database inside a view are errors in `crow check`, each with a
one-key rewrite: `d` turns `trades = query[…]` into `derive trades`, and
hoists a `rows = query[pg, …]` line out of a view as
`derive rows[fresh = 60]` above it, the view reading `rows` as before.

Derivations are reactive state, so they live in a `use ui` program with
a page. In the test harness `html[a]` is a tick and `click[a, h]` marks
what the handler wrote, so a test can watch a derivation follow its
inputs the way these blocks do.
