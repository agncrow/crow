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

## Where the queries went

Before `derive`, the finch simulation's view ran four queries a second,
three of which changed a few times an hour. `crow check` warned about
it — remote-query-in-view — and could only warn. With `derive`, no view
queries anything: the bars are a derivation over the symbol and the day,
the rows at the play-head over the day and the clock, the trades over
the session. A tick recomputes the play-head and the rows; a symbol
change recomputes the bars; a buy recomputes the trades. Nothing else
moves, and nobody wrote a line about caching.

Derivations are reactive state, so they live in a `use ui` program with
a page and are computed per session. In the test harness `html[a]` is a
tick and `click[a, h]` marks what the handler wrote, so a test can watch
a derivation follow its inputs the way these blocks do.
