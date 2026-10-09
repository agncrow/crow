# Project: Analyzing Data

Time to put chapters 3–9 to work on Crow's home turf: tabular data. We'll
take a small file of trades from raw CSV to a grouped summary — the whole
loop of load, inspect, filter, sort, aggregate, report.

The star is the **frame**: a column-oriented table with named, typed
columns. Maps hold one type; structs hold one record; frames hold *many
records* — the third point of the collections triangle from chapter 5.

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

## Other Formats, Other Places

A file's extension names its format. `save` writes a frame as CSV, as
JSON records or as Parquet — the columnar format data tools share — and
`load` reads each one back:

```crow
use frame

prices = frame.new[]
prices = with[prices, 'symbol', ['AAPL', 'MSFT']]
prices = with[prices, 'price', [191.5, 415.2]]
save[prices, 'ch10_prices.parquet']
back = frame.load['ch10_prices.parquet']
log[back.columns]
log[back.symbol]
save[prices, 'ch10_prices.json']
log[read['ch10_prices.json']]
rm['ch10_prices.parquet']
rm['ch10_prices.json']
```

```text
['symbol', 'price']
['AAPL', 'MSFT']
[{"price":191.5,"symbol":"AAPL"},{"price":415.2,"symbol":"MSFT"}]
```

A path can also be a URL or an S3 object — wherever a program reads a
file, it reads those the same way. A JSON API's answer that holds its rows
in one list (`{"status": "OK", "results": [...]}`) loads as those rows, and
an S3 bucket is reached with the credentials in the environment
(`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`):

```crow,norun
use frame

day = frame.load['https://data.example.com/aapl/2026-07-14.parquet']
archive = frame.load['s3://market-archive/2026/07/14/trades.csv']
save[day, 's3://reports/aapl-2026-07-14.parquet']
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
   str      int
------  -------
  AAPL      140
  MSFT       75
  NVDA       20

[3 rows x 2 cols]
```

Twenty-odd lines: load, validate shape, filter, sort, aggregate, format.
Every piece was a chapter you've already read — frames just give the
pieces something worth doing. Chapter 13 does this against a live
database with a web UI on top; next, the database half of that story.

## Several Keys, Pivots and Windows

`group` takes a list of keys for a row per combination; `pivot` lays one
column's cells out as columns; and `window` computes a column over each
row's partition — a running sum per symbol, the previous row's price —
which `with` adds to the frame:

```crow
use frame

csv = 'symbol,day,qty,price
AAPL,1,100,191.5
MSFT,1,50,415.2
AAPL,2,40,193.1
NVDA,1,20,875.0
MSFT,2,25,411.9'
write['trades.csv', csv]
trades = frame.load['trades.csv']

by_day = group[trades, ['symbol', 'day'], ['qty', 'sum']]
log[by_day]

wide = pivot[trades, 'symbol', 'day', 'qty']
log[wide]

trades = with[trades, 'so_far', window[trades, 'qty', 'sum', by = 'symbol']]
trades = with[trades, 'prev', window[trades, 'price', 'lag', n = 1, by = 'symbol']]
log[trades]
rm['trades.csv']
```

```text
symbol  day  qty_sum
   str  int      int
------  ---  -------
  AAPL    1      100
  MSFT    1       50
  AAPL    2       40
  NVDA    1       20
  MSFT    2       25

[5 rows x 3 cols]
symbol    1        2
   str  int      int
------  ---  -------
  AAPL  100       40
  MSFT   50       25
  NVDA   20  missing

[3 rows x 3 cols]
symbol  day  qty  price  so_far     prev
   str  int  int  float     int    float
------  ---  ---  -----  ------  -------
  AAPL    1  100  191.5     100  missing
  MSFT    1   50  415.2      50  missing
  AAPL    2   40  193.1     140    191.5
  NVDA    1   20    875      20  missing
  MSFT    2   25  411.9      75    415.2

[5 rows x 6 cols]
```

A pivot's cell is missing where no row has the pair (NVDA on day 2), and
a pair two rows share stops the program, naming the `group` that would
aggregate it first. A window's `n` is the trailing rows (`n = 3` for a
three-row mean; 0, the default, for every row so far), and `how` is one
of `sum`, `mean`, `min`, `max`, `count`, `lag`, `lead` and `rank`.

## Keys, Time and Text

The same verbs reach further: `bucket` floors each moment to a span, so
a `group` over the bucket makes bars; `group` and `window` take `median`,
a percentile, `std`, `first`, `last` and `distinct` beside `sum` and
`mean`; `sort` and `join` take several keys; `where` picks a cell by a
mask; `complete` keeps the rows with no missing cell; `unpivot` makes a
wide frame long; `contains` tests a text column, and `.hour` reads a
moment column's field — each over the whole column at once:

```crow
use frame

csv = 'symbol,at,qty,price
AAPL,2026-07-14T09:30:00-04:00,100,191.5
MSFT,2026-07-14T09:31:00-04:00,50,415.2
AAPL,2026-07-14T09:35:00-04:00,,193.1
AAPL,2026-07-14T10:36:30-04:00,40,190.0
MSFT,2026-07-14T10:40:00-04:00,70,410.0'
write['ticks.csv', csv]
ticks = frame.load['ticks.csv']

hourly = with[ticks, 'hour', bucket[ticks.at, 3600]]
log[group[hourly, ['symbol', 'hour'], ['price', 'first'], ['price', 'max'], ['price', 'min'], ['price', 'last'], ['qty', 'sum']]]
log[group[ticks, 'symbol', ['price', 'median'], ['price', 'p90'], ['qty', 'distinct']]]
log[sort[ticks, ['symbol', 'qty'], desc = [false, true]].price]
log[where[ticks.qty.ok, ticks.qty, 0]]
log[complete[ticks].price]
log[unpivot[select[ticks, ['symbol', 'qty', 'price']], 'symbol', ['qty', 'price']]]
log[filter[ticks, 'AA' in ticks.symbol and ticks.at.hour eq 9].price]
log[window[ticks, 'price', 'mean', span = 3600, on = 'at', by = 'symbol']]
rm['ticks.csv']
```

```text
symbol                  hour  price_first  price_max  price_min  price_last  qty_sum
   str              datetime        float      float      float       float      int
------  --------------------  -----------  ---------  ---------  ----------  -------
  AAPL  2026-07-14T09:00:...        191.5      193.1      191.5       193.1      100
  MSFT  2026-07-14T09:00:...        415.2      415.2      415.2       415.2       50
  AAPL  2026-07-14T10:00:...          190        190        190         190       40
  MSFT  2026-07-14T10:00:...          410        410        410         410       70

[4 rows x 7 cols]
symbol  price_median  price_p90  qty_distinct
   str         float      float           int
------  ------------  ---------  ------------
  AAPL         191.5     192.78             2
  MSFT         412.6     414.68             2

[2 rows x 4 cols]
[191.5, 190, 193.1, 410, 415.2]
[100, 50, 0, 40, 70]
[191.5, 415.2, 190, 410]
symbol  variable    value
   str       str    float
------  --------  -------
  AAPL       qty      100
  MSFT       qty       50
  AAPL       qty  missing
  AAPL       qty       40
  MSFT       qty       70
  AAPL     price    191.5
  MSFT     price    415.2
  AAPL     price    193.1
  AAPL     price      190
  MSFT     price      410

[10 rows x 3 cols]
[191.5, 193.1]
[191.5, 415.2, 192.3, 190, 410]
```

A time window takes the rows within `span` seconds before each row's own
moment, on the column `on` names, so a five-minute average is `span =
300` and never a guess at how many rows five minutes hold; the frame is
sorted by that column within each partition first. A bucket's edge is
midnight in the moment's own offset, so a day in New York starts at the
exchange's midnight. Every one of these is held to pandas, to the last
digit, by `tests/parity/185_frames_held_to_pandas.co`.
