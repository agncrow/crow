# Loading and Shaping the Data

Real apps get data from outside; ours needs a database to exist first. A
seed script is a plain Crow program — chapter 11, nothing more:

```crow
use db

conn = connect['sqlite://trades.db']
exec[conn, 'DROP TABLE IF EXISTS trades']
exec[conn, 'CREATE TABLE trades (id INTEGER PRIMARY KEY, symbol TEXT, qty INTEGER, price REAL, pnl REAL)']
exec[conn, 'INSERT INTO trades VALUES (1, ?, ?, ?, ?)', ['AAPL', 100, 191.5, 250.0]]
exec[conn, 'INSERT INTO trades VALUES (2, ?, ?, ?, ?)', ['MSFT', 50, 415.2, -120.5]]
exec[conn, 'INSERT INTO trades VALUES (3, ?, ?, ?, ?)', ['NVDA', 20, 875.0, 890.2]]
exec[conn, 'INSERT INTO trades VALUES (4, ?, ?, ?, ?)', ['AAPL', 40, 193.1, 96.4]]

counts = query[conn, 'SELECT count(*) AS n FROM trades']
log['seeded `counts.n[0]` trades']
rm['trades.db']
```

```text
seeded 4 trades
```

(The final line removes the file only so this book's test runner stays
tidy — the real seed script, `examples/dashboard_seed.co`, keeps it.)

`sqlite://trades.db` is a *file* this time, not `:memory:` — the data
survives between runs, and the dashboard process will open the same file.
Swap in a `postgres://...` URL later and nothing else changes.
