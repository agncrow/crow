# Frames: `use frame`

```crow
use frame, math

write['ex_trades.csv', 'symbol,qty,price\nAAPL,100,191.5\nMSFT,50,415.2\nAAPL,40,193.1\n']
t = frame.load['ex_trades.csv']
log[t.columns]
log[len[filter[t, t.symbol eq 'AAPL']]]
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
log[len[stats[t]] gt 0]
filled = fill[t, 'qty', 0]
log[len[filled]]
save[t, 'ex_copy.csv']
log[len[frame.load['ex_copy.csv']]]
d = moment['2026-07-14T13:30:00Z']
log[moment[d, tz = '+09:00']]
g = frame.new[symbol = ['A', 'B'], px = [1.5, 2.5]]
log[g.columns]
log[with[g, 'day', 'today'].day[1]]
log[join[['x', 'y'], '+']]
log[group[t, 'symbol', ['price', 'median'], ['qty', 'p75'], ['symbol', 'distinct']].price_median]
log[sort[t, ['symbol', 'qty'], desc = [false, true]].qty]
log[where[t.qty gt 50, 'big', 'small']]
log[unique[t, ['symbol']].price]
log[len[complete[t]]]
log[len[sample[t, 2]]]
log[unpivot[t, 'symbol', ['qty', 'price']].variable]
log[contains[t.symbol, 'AA']]
log[window[t, 'price', 'median', n = 2]]
log[bucket[d, 3600]]
log[fill[with[frame.new[], 'x', where[[true, false, true], 1, 2]], 'x', how = 'forward'].x]
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
true
3
3
2026-07-14T22:30:00+09:00
['symbol', 'px']
today
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
```

`frame.new[]` and `with[]` build a frame column by column from arrays or
a repeated scalar. With `frame` imported the core `join`, `sort`, and
friends still work on arrays: the first argument decides which one you
meant.

Reference: spec section 13 (frame) and book chapter 10.
