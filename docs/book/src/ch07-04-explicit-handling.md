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

A guard can be its own `if`, telling each case apart:

```crow
text = read['no_such_file.txt']
if not text.ok
    log['unreadable']
else if len[text] eq 0
    log['empty']
else
    log['`len[text]` characters']
```

```text
unreadable
```

Or one condition, where the cases are handled alike: `and` and `or`
decide from the left, so the right side runs only where the left has not
already decided — `len[text]` is never asked of a read that failed:

```crow
text = read['no_such_file.txt']
if not text.ok or len[text] eq 0
    log['nothing to read']
else
    log['`len[text]` characters']
```

```text
nothing to read
```

Inside a function, a guard whose branch leaves also *narrows*: after
`if not text.ok` ending in `return`, `text` is known good for the rest of
the block, so `len[text]` there is a plain number, and the function is not
typed as able to fail:

```crow
fn size_of[path]
    text = read[path]
    if not text.ok
        return 0
    return len[text]

log[size_of['no_such_file.txt']]
```

```text
0
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
