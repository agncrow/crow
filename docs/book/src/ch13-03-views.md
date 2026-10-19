# Views and Live Updates

Now the full program — the seed script's sibling, and the payoff of every
chapter in this book:

```crow,norun
--- Live trading dashboard: SQLite -> frames -> views
--- Seed first: crow run dashboard_seed.co   Then: crow run dashboard.co
use web, ui, db, frame, math

conn = db.connect['sqlite://trades.db']

--- Reactive UI state
min_qty = 0

--- Event handlers
fn reset_filters[]
    min_qty = 0

view stat_card[label, value]
    return div[
        h3[label],
        p['`value`'],
        class = 'card'
    ]

derive trades
    return query[conn, 'SELECT symbol, qty, price, pnl FROM trades ORDER BY id']

view render[]
    total_pnl = sum[trades.pnl]

    shown = trades
    if min_qty gt 0
        shown = trades[trades.qty gte min_qty]
    shown = sort[shown, 'pnl', desc = true]

    by_symbol = group[trades, 'symbol', ['pnl', 'sum'], ['pnl', 'count']]

    return div[
        h1['Trading Dashboard'],
        div[
            stat_card['P&L', fmt[total_pnl, decimals = 2, commas = true]],
            stat_card['Trades', len[trades]],
            class = 'stats'
        ],
        div[
            field['Min qty', min_qty],
            button['Reset', click = reset_filters],
            class = 'controls'
        ],
        h2['Trades'],
        table[shown],
        h2['P&L by Symbol'],
        table[by_symbol]
    ]

serve[3000]
page['/', render]
```

Walk it top to bottom and count the chapters:

- **`db.connect`** — qualified, because `use web` also exports a
  `connect` (the websocket client); the prefix resolves it, exactly as
  chapter 8 said it must. The connection is infrastructure, not UI state
  — Crow keeps it as a process-wide handle, never serialized.
- **`min_qty` / `reset_filters`** — chapter 12's reactive loop: edit the
  field, the state changes, the page re-renders; click Reset, same.
- **`stat_card`** — a parameterized component, used at two types
  (string and int values). Chapter 4's monomorphization quietly makes
  both work.
- **`trades`** — a derivation, chapter 12's other half: the query runs
  off the render, and runs again when a handler writes the table or a
  refresh tick passes, so every page shows the database as it is *now*
  without a query per render. A view reads it like a fact; a view that
  queried itself would be refused, since a view is a function of the
  state it is given. Frames flow from the derivation through masks,
  `sort` and `group` into `table[...]` untouched.
- **A failed query renders its reason** — a view renders what it's
  given, and what it's given is chapter 7's error value. Stop the
  database and the page tells you why, instead of crashing.

Seed, run, open `localhost:3000` — the page below is real output, served
by this program during this book's own verification:

```text
Trading Dashboard
P&L: 1,116.10   Trades: 4
[Min qty ____] [Reset]
Trades:            symbol qty price pnl ...
P&L by Symbol:     AAPL 346.4 · MSFT -120.5 · NVDA 890.2
```
