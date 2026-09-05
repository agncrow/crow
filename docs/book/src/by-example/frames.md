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
d = datetime['2026-07-14T13:30:00Z']
log[tz[d, '+09:00']]
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
190
['AAPL']
['AAPL', 'MSFT']
['symbol', 'shares', 'price']
6
true
3
3
2026-07-14T22:30:00+09:00
```

Reference: spec section 13 (frame) and book chapter 10.
