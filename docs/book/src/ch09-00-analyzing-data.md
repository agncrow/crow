# Project: Analyzing Data

Time to put chapters 3–8 to work on Crow's home turf: tabular data. We'll
take a small file of trades from raw CSV to a grouped summary — the whole
loop of load, inspect, filter, sort, aggregate, report.

The star is the **frame**: a column-oriented table with named, typed
columns. Maps hold one type; structs hold one record; frames hold *many
records* — the third point of the collections triangle from chapter 6.

## The Data

We'll generate our data file right in the program so the example is
self-contained (in real life this arrives from disk or a database):

```crow
use frame, math

csv = 'symbol,qty,price
AAPL,100,191.5
MSFT,50,415.2
AAPL,40,193.1
NVDA,20,875.0
MSFT,25,411.9'
write['trades.csv', csv]

trades = frame.load['trades.csv']
log[trades.shape]
log[trades.columns]
rm['trades.csv']
```

```text
[5, 3]
['symbol', 'qty', 'price']
```

One detail worth pausing on: we wrote `frame.load`, not bare `load` —
because with both `frame` and `math` imported, *two* modules provide a
`load` (frames from CSV; numeric tensors from CSV). Crow refuses the
ambiguity, exactly as chapter 8 promised:

```crow,error
use frame, math
df = load['trades.csv']   --- error: 'load' is ambiguous — provided by
                          --- frame and math; use frame.load[...] or
                          --- math.load[...]
```

## Filtering and Sorting

Column access (`trades.qty`) plus a comparison gives a *mask*, and
`filter` applies it. `sort` takes a column name, with `desc` as a named
argument:

```crow
use frame

csv = 'symbol,qty,price
AAPL,100,191.5
MSFT,50,415.2
AAPL,40,193.1
NVDA,20,875.0
MSFT,25,411.9'
write['trades.csv', csv]
trades = frame.load['trades.csv']

big = filter[trades, trades.qty gte 40]
log[len[big]]

by_price = sort[trades, 'price', desc = true]
log[by_price]
rm['trades.csv']
```

```text
3
symbol  qty  price
   str  int  float
------  ---  -----
  NVDA   20    875
  MSFT   50  415.2
  MSFT   25  411.9
  AAPL   40  193.1
  AAPL  100  191.5

[5 rows x 3 cols]
```

Frames print themselves as aligned tables with their column types — no
formatting code required.

## Aggregating and Reporting

Math reductions accept columns directly, `fmt` handles presentation, and
`group` aggregates by key:

```crow
use frame, math

csv = 'symbol,qty,price
AAPL,100,191.5
MSFT,50,415.2
AAPL,40,193.1
NVDA,20,875.0
MSFT,25,411.9'
write['trades.csv', csv]
trades = frame.load['trades.csv']

log['total shares: `sum[trades.qty]`']
avg_price = sum[trades.price] / 5.0
log['avg price: `fmt[avg_price, decimals = 2]`']

by_symbol = group[trades, 'symbol', ['qty', 'sum']]
log[by_symbol]
rm['trades.csv']
```

```text
total shares: 235
avg price: 417.34
symbol  qty_sum
   str    float
------  -------
  AAPL      140
  MSFT       75
  NVDA       20

[3 rows x 2 cols]
```

Twenty-odd lines: load, validate shape, filter, sort, aggregate, format.
Every piece was a chapter you've already read — frames just give the
pieces something worth doing. Chapter 12 does this against a live
database with a web UI on top; next, the database half of that story.
