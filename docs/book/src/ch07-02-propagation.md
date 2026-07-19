# Propagation

What happens when a failed value meets an operator? The failure wins, and
keeps its story:

```crow
bad = int['oops']
worse = bad + 10
log[worse.ok]
log[worse.reason]
```

```text
false
cannot convert 'oops' to int
```

No crash, no exception — `bad + 10` is simply another error value with
the *original* reason. Arithmetic, interpolation, function calls: pass a
failure through any of them and it propagates unchanged, appending to
`.trace` as it crosses function boundaries.

This is the quiet radicalism of the model: **the code between where a
failure happens and where it's handled doesn't participate.** No `?` on
every call, no `try` blocks around every section, no exception
specifications. Intermediate code stays written for the happy path,
and failures flow through it inert — they only *matter* when something
tries to make a decision with one. Which is the next section.
