# Frames: `use frame`

```crow
use frame, math

write['ex_trades.csv', 'symbol,qty,price\nAAPL,100,191.5\nMSFT,50,415.2\nAAPL,40,193.1\n']
t = frame.load['ex_trades.csv']
log[t.columns]
log[len[t[t.symbol eq 'AAPL']]]
log[sort[t, 'qty'].qty]
log[select[t, ['symbol', 'qty']].columns]
log[head[t, 1].symbol]
log[group[t, 'symbol', ['qty', 'sum']].qty_sum]
log[group[t, ['symbol', 'qty'], ['price', 'max']].symbol]
log[pivot[t, 'symbol', 'qty', 'price'].columns]
log[window[t, 'qty', 'sum', by = 'symbol']]
log[sum[t.qty]]
log[tail[t, 1].symbol]
log[frame.unique[t, 'symbol']]
log[rename[t, 'qty', 'shares'].columns]
log[len[frame.stack[t, t]]]
parts = []
parts = push[parts, t]
parts = push[parts, head[t, 1]]
log[len[stack[parts]]]
log[len[stats[t]] gt 0]
filled = fill[t, 'qty', 0]
log[len[filled]]
save[t, 'ex_copy.csv']
log[len[frame.load['ex_copy.csv']]]
d = moment['2026-07-14T13:30:00Z']
log[moment[d, tz = '+09:00']]
g = frame.new[symbol = ['A', 'B'], px = [1.5, 2.5]]
log[g.columns]
g.day = 'today'
log[g.day[1]]
flags = frame.new[name = ['a', 'b', 'c'], on = [true, false, true]]
log[flags[flags.on].name]
log[join[['x', 'y'], '+']]
log[group[t, 'symbol', ['price', 'median'], ['qty', 'p75'], ['symbol', 'distinct']].price_median]
log[sort[t, ['symbol', 'qty'], desc = [false, true]].qty]
log[where[t.qty gt 50, 'big', 'small']]
log[unique[t, ['symbol']].price]
log[len[complete[t]]]
log[len[sample[t, 2]]]
log[unpivot[t, 'symbol', ['qty', 'price']].variable]
log['AA' in t.symbol]
log[window[t, 'price', 'median', n = 2]]
log[bucket[d, 3600]]
log[fill[frame.new[x = where[[true, false, true], 1, 2]], 'x', how = 'forward'].x]
gap = frame.new[node = ['a', 'b'], value = [0.5, 0.25]]
    then join[gap, frame.new[node = ['b', 'a'], ref = [0.25, 0.75]], 'node']
    then max[abs[gap.value - gap.ref]]
log[gap]
rm['ex_trades.csv']
rm['ex_copy.csv']
```

```text
['symbol', 'qty', 'price']
2
[40, 50, 100]
['symbol', 'qty']
['AAPL']
[140, 50]
['AAPL', 'MSFT', 'AAPL']
['symbol', '40', '50', '100']
[100, 50, 140]
190
['AAPL']
['AAPL', 'MSFT']
['symbol', 'shares', 'price']
6
4
true
3
3
2026-07-14T22:30:00+09:00
['symbol', 'px']
today
['a', 'c']
x+y
[192.3, 415.2]
[100, 40, 50]
['big', 'small', 'small']
[191.5, 415.2]
3
2
['qty', 'qty', 'qty', 'price', 'price', 'price']
[true, false, true]
[191.5, 303.35, 304.15]
2026-07-14T13:00:00Z
[1, 2, 1]
0.25
```

A mask picks a frame's rows as it picks a list's items,
`t[t.symbol eq 'AAPL']`, and a column of bools is its own mask
(`flags[flags.on]`). `frame.new[…]` builds a frame from its columns, and
`df.name = values` writes one more where the frame lies, from an array or
a repeated scalar. With `frame` imported the core `join`, `sort`, and
friends still work on arrays: the first argument decides which one you
meant. The last lines are a `then` chain written as lines: a step that
names the variable being assigned (`gap`) reads the value so far there,
so the value may change its kind — a frame through the join, a number at
the end.

## A column of vectors

```crow
use frame
use math

df = frame.new[id = [1, 2, 3], e = [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]]
log[df.dtypes]
log[df.e[2]]
along = for v in df.e
    fmt[dot[v, [1.0, 0.0]], decimals = 1]
log[along]
log[tensor[df.e].shape]
log[str[df.e]]
```

```text
['int', 'vector']
[1, 1]
['1.0', '0.0', '1.0']
[3, 2]
['[1, 0]', '[0, 1]', '[1, 1]']
```

A `vector` column holds vectors of one width, an embedding per row:
written from a matrix (its rows) or a list of vectors, read one cell at
a time as a tensor, whole as its matrix (`tensor[df.e]`). It has no
order and no cell-by-cell arithmetic — `sort` by it or a `sum` over it
stops the program — and `save` writes it as JSON records or Parquet,
not CSV. Magpie holds the column and finds rows by it (`nearest`).

Reference: spec section 13 (frame) and book chapter 10.
