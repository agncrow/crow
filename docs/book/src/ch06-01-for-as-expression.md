# for as an Expression

Assign a `for` loop and it *collects* — the last expression of each
iteration becomes an element of the result:

```crow
squares = for x in [1, 2, 3, 4, 5]
    x * x
log[squares]
```

```text
[1, 4, 9, 16, 25]
```

A trailing `if` filters — iterations that produce no value contribute
nothing:

```crow
evens = for x in range[10]
    if x % 2 eq 0
        x
log[evens]
```

```text
[0, 2, 4, 6, 8]
```

This one construct is Crow's `map`, `filter`, and list comprehension —
none of which exist separately, because they'd all be second ways to
write this.

## The Parallel Part

Those iterations did not run in order. Crow analyzes what each statement
reads and writes and runs independent work concurrently — a `for` over ten
thousand rows uses every core, for free:

```crow
celsius = [0.0, 21.5, 100.0]
fahrenheit = for c in celsius
    c * 9.0 / 5.0 + 32.0
log[fahrenheit]
```

```text
[32, 70.7, 212]
```

The *results* arrive in input order — collection is deterministic. The
iterations run in parallel only when nothing in the body can show the
order they ran in. A body that prints, writes a file or a fact, draws a
random number, renders a view, or accumulates into an outer variable
(`total = total + n`) runs in order, so it stays correct and just
forfeits the parallelism. A function you call counts by what it does,
however deep: a loop over a function that only computes runs on every
core, one over a function that logs runs in order. `crow build --ir`
shows which loops run in parallel. The idiom that keeps a loop parallel
is the one you've been seeing all book: **compute in the loop, collect
the values, print after.**

`cpu[8]` before a loop caps its thread count; `cpu[1]` runs every loop on
one core.
