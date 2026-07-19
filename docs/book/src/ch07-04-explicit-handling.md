# Handling Errors Explicitly

Reading metadata (`.ok`, `.reason`, `.kind`) never propagates — that's
the one bridge from a failed value back to normal control flow. The
guard is the whole pattern:

```crow
raw = '42x'
n = int[raw]
if not n.ok
    log['bad input: `n.reason`']
else
    log['parsed `n`']
```

```text
bad input: cannot convert '42x' to int
```

Branch by category when it matters:

```crow
fail = error['connection refused', 'network']
if not fail.ok
    if fail.kind eq 'network'
        log['will retry']
    else
        log['giving up: `fail.reason`']
```

```text
will retry
```

## When to Check

The idiom that falls out of this chapter: **check `.ok` at boundaries —
user input, files, network, database — and let everything between flow.**
Interior code stays happy-path; a failure entering it comes back out
carrying its story; the boundary that cares inspects it. It's the
discipline exceptions promise ("handle errors at the right level") made
mechanical: you *can't* forget an unhandled failure, because the moment
one reaches a decision, it stops the show with its reason attached.

That's the whole error system. No other constructs exist to learn.
