# Errors Have No Truth Value

Here is the rule that completes the model:

> An error value cannot be compared and cannot decide a branch. It is not
> falsy. It is not truthy. It is **unanswerable**, and Crow treats it that
> way.

Consider `if bad gt 5` where `bad` is a failed `int[...]`. Exceptions
would have crashed earlier; truthy/falsy languages would silently pick a
branch — *misclassifying* corrupt input as a small number. Crow does
neither:

```crow,norun
bad = int['x']
if bad gt 5
    log['big']
```

```text
error: cannot branch on an error value — cannot convert 'x' to int
```

At top level, the program halts with the original reason. Inside a
function, something better happens: **the function returns the error to
its caller** — the decision that couldn't be made becomes the caller's
failure to inspect:

```crow
fn size_of[s]
    n = int[s]
    if n gt 100
        return 'big'
    return 'small'

r1 = size_of['150']
log[r1]

r2 = size_of['xyz']
log[r2.ok]
log[r2.reason]
```

```text
big
false
cannot convert 'xyz' to int
```

`size_of` contains no error handling at all — it's written purely for the
happy path. Feed it garbage and it *returns* the garbage's story, at the
exact moment the garbage mattered. The failure detonates at the decision,
not at the origin, and never silently.
