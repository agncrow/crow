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

A loop is a decision too. An error has no items to walk, so a `for` over a
list that failed is refused the same way: `for i in range[len[r]]` inside a
function hands `r`'s error back when `r` failed, and at the top level it
halts with `cannot loop over an error value`.

An error could also slip past in a variable. A variable whose first value
could not fail never holds an error: a later value that fails is refused
right where it is assigned, the same way a decision refuses one.

```crow
fn add_up[items]
    acc = 0.0
    for s in items
        acc = acc + float[s]
    return acc

log[add_up[['1.5', '2']]]
bad = add_up[['1.5', 'two']]
log[bad.ok]
log[bad.reason]
```

```text
3.5
false
cannot convert 'two' to float
```

`acc` began as a plain number, so the failed item never lands in it:
`add_up` hands the error back at that line. At the top level the program
halts there with `cannot assign an error value to 'acc'`. When you want to
keep a failure and look at it, give it a name of its own whose first value
can fail: `r = float[s]`, then `if not r.ok`.

## A failure nothing reads

A line made of one call is usually there for what the call does:
`write[path, text]`, `exec[conn, sql]`, `rm[path]`. Each answers a value
that can fail, and on a line of its own nothing reads it. If that failure
simply went by, a program could lose its work without a word. So Crow
treats the line as a decision, like a branch: a function hands the failure
to its caller, and the top level halts.

```crow
fn save_note[path, note]
    write[path, note]
    return 'saved'

r = save_note['/no/such/folder/note.txt', 'hello']
log[r.ok]
log[r.reason]

kept = rm['/no/such/folder/old.txt']
log[kept.ok]
```

```text
false
cannot write '/no/such/folder/note.txt': No such file or directory (os error 2)
false
```

`save_note` never reached its `return`: the write failed, and the
function handed that failure back. The `rm` failed too, but its value was
given a name, so it is an ordinary value the program may look at or
ignore; naming it is how you say a failure is fine. At the top level, a
dropped failure stops the program on its line:

```crow,norun
write['/no/such/folder/note.txt', 'hello']
log['not reached']
```

```text
runtime error: line 1: cannot drop an error value — cannot write '/no/such/folder/note.txt': No such file or directory (os error 2)
```

A collecting loop's last line hands its value on, so it is not dropped,
and neither is a test's truth: `len[xs] eq 3` on a line of a test block is
asserted.

## What no value can carry

A few failures have no error value to become. `xs[i]` reads an element and
`a / b` of two ints makes an int, and neither an element nor an int can say
it failed. So an index outside its array, a key the map lacks, an int
divided by zero, or an int that no longer fits in 64 bits stops the program
on the line it happened on:

```crow,norun
fn pick[xs, i]
    return xs[i]

log[pick[[10, 20, 30], -1]]
log[pick[[10, 20, 30], 5]]
```

```text
30
runtime error: line 2: index 5 is out of range for an array of 3 items
```

A negative index counts from the end, so `-1` is the last item. The stop
names line 2, inside `pick`, where the index was read. The words are the
same in `crow run -i` and `crow run`, and in a handler the click stops with
nothing it did landing. When an index might be outside, check `len` first,
as you would guard any decision.
