# Every Value Has Status

All values — not a special error type, *all* of them — carry four pieces
of metadata: `.ok`, `.reason`, `.kind`, and `.trace`. A successful value
answers boringly, and auto-unwraps so you never think about the wrapper:

```crow
n = int['42']
log[n]
log[n.ok]
log[n + 8]
```

```text
42
true
50
```

A failure is the same shape with a different story:

```crow
bad = int['oops']
log[bad.ok]
log[bad.reason]
```

```text
false
cannot convert 'oops' to int
```

You can also make one deliberately — this is Crow's `raise`, except
nothing is thrown; you just return it:

```crow
down = error['price feed is down', 'network']
log[down.ok]
log[down.reason]
log[down.kind]
```

```text
false
price feed is down
network
```

`.kind` is a category for programmatic handling (`'parse'`, `'io'`,
`'network'`); `.trace` accumulates the path a failure traveled, for
debugging.
