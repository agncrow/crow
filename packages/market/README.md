# market

US equity hours, the NYSE calendar, and bars, written in Crow. The chart
module computes the indicators (`sma`, `ema`, `rsi`, `macd`, `vwap`, `cvd`)
and draws the candles; this package supplies what they run over.

```
deps = map[market = 'github.com/agncrow/crow/packages/market@main']
```

```
use market.sessions, market.bars
```

## sessions

A day is `Day[year, month, day]` in New York, where the exchange keeps its
hours, and a moment is epoch seconds, as everywhere in Crow.

| Function | Answers |
|----------|---------|
| `trading[d]` | whether the exchange trades that day |
| `holiday[d]` | whether a weekday is a holiday: the standing rules (a Saturday holiday kept the Friday before, a Sunday one the Monday after, New Year's Day on a Saturday closing nothing) and the one-off closings since 2001 |
| `early[d]` | whether the regular session ends at 13:00: the day before Independence Day and Christmas when a trading day, and the day after Thanksgiving |
| `opens[d]`, `closes[d]` | the regular session's first and last moment |
| `at[d, hour, minute]` | the moment New York's clock reads that time on `d` |
| `phase[t]` | `'early'` (04:00–09:30), `'regular'`, `'late'` (to 20:00, or 17:00 after an early close), or `'closed'` |
| `day_of[t]` | the day a moment falls on in New York |
| `next[d]`, `previous[d]` | the nearest trading day after or before |
| `between[first, last]` | the trading days in a span, both ends included |
| `easter[year]` | Easter Sunday (Good Friday is two days before) |

The calendar is exact from 2001 on. `phase` can fail, as the calendar
underneath it can (a moment that is not finite), so a program guards it
with `.ok` where every value must decide something.

```
use market.sessions

d = sessions.Day[2026, 11, 27]
log[sessions.early[d]]                      --- true
log[sessions.phase[sessions.at[d, 14, 0]]]  --- late
```

## bars

A bar covers `secs` seconds of the clock, aligned to it, and holds its start
`ts`, `o`, `hi`, `lo`, `c`, its volume `vol`, its trade count `n` and its
volume-weighted price `vwap`. A stretch with no trade has no bar.

| Function | Answers |
|----------|---------|
| `build[ts, price, size, secs]` | the bars of trades in time order |
| `resample[bars, secs]` | longer bars from shorter ones |
| `during[bars, part]` | the bars that start in a part of the day (`sessions.phase`) |

```
use chart, market.bars

b = bars.build[ts, price, size, 60]
b.rsi = rsi[b.c, 14]
```

## Tests

`sessions.co` checks the exchange's published calendars for 2024–2026
(holidays, early closes, 252, 250 and 251 trading days); `bars.co` builds,
resamples and splits bars by session. In the Crow repository `crow test
--packages market` runs both, and runs `example.co` in both modes.
