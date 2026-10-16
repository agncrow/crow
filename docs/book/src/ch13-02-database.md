# The Database Layer

The dashboard's data needs are three queries' worth of chapter 11 and
chapter 10 — run against the seeded file, straight into frames:

```crow
use db, frame, math

conn = connect['sqlite://ch12demo.db']
exec[conn, 'CREATE TABLE trades (id INTEGER PRIMARY KEY, symbol TEXT, qty INTEGER, price REAL, pnl REAL)']
exec[conn, 'INSERT INTO trades VALUES (1, ?, ?, ?, ?)', ['AAPL', 100, 191.5, 250.0]]
exec[conn, 'INSERT INTO trades VALUES (2, ?, ?, ?, ?)', ['MSFT', 50, 415.2, -120.5]]
exec[conn, 'INSERT INTO trades VALUES (3, ?, ?, ?, ?)', ['NVDA', 20, 875.0, 890.2]]

trades = query[conn, 'SELECT symbol, qty, price, pnl FROM trades ORDER BY id']
total_pnl = sum[trades.pnl]
log['P&L: `fmt[total_pnl, decimals = 2, commas = true]`']

by_symbol = group[trades, 'symbol', ['pnl', 'sum']]
log[by_symbol]
rm['ch12demo.db']
```

```text
P&L: 1,019.70
symbol  pnl_sum
   str    float
------  -------
  AAPL      250
  MSFT   -120.5
  NVDA    890.2

[3 rows x 2 cols]
```

Query results are frames, frames feed `sum`/`group`/`sort` and masks, and
`fmt` dresses the numbers. This is the entire "backend" of the dashboard —
there is no separate API layer to write, because the views run in the
same process as the data.
