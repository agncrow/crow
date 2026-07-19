# Structs Are Records

A struct declares named fields of any mix of types:

```crow
struct Trade [symbol, qty, price]

t = Trade['AAPL', 100, 191.5]
log[t.symbol]
log[t.qty * t.price]
log[t]
```

```text
AAPL
19150
Trade[price = 191.5, qty = 100, symbol = 'AAPL']
```

Construct positionally (or with names — `Trade[symbol = 'AAPL', ...]`),
access with dots. Structs pass to functions efficiently and print
themselves readably.

Field names are free — schema names like `type`, `sum`, or `count` are
fine as *fields* (`t.type`) even where they'd be reserved as variables.
The only names a field can't take are the four error-metadata names every
value already carries: `ok`, `reason`, `kind`, `trace`.

**The decision rule, one line:** same type under many keys → map; mixed
types under fixed names → struct; a *column* of records → chapter 9's
data frames.
