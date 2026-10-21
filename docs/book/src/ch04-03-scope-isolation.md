# Function Scope Isolation

Here is the rule, and it is stricter than Python's:

> **Module variables are invisible inside functions.** A function sees its
> parameters, its own locals, other functions, structs, and imported
> vocabulary — nothing else. Data enters through parameters and leaves
> through the return value.

```crow,error
count = 5

fn bad[]
    --- error: 'count' is a module variable, which a function does not
    --- see — pass it as a parameter
    return count + 1
```

The fix is always the same shape:

```crow
count = 5

fn bump[n]
    return n + 1

log[bump[count]]
```

```text
6
```

A local that happens to reuse a module name shadows nothing — module
variables were never visible, so it's simply a fresh binding. No `global`
keyword, no `nonlocal`, no scope-resolution table to memorize.

## Why So Strict?

Two reasons, and they're both load-bearing:

1. **A signature that tells the whole truth.** If a function can secretly
   read ambient state, its behavior depends on things not in its
   signature — refactoring, testing, and `crow check` all get harder.
   Parameters-in, return-out means every dependency is visible at the
   call site.
2. **Parallelism needs it.** Chapter 6's parallel loops work because
   every read and write in the loop body is visible at the call site. Hidden reads would turn "parallel by default" into "data races
   by default." Crow chose the scope rule that makes the loop rule sound.

The one sanctioned exception is the state of a program that serves
(chapter 12): `view` blocks, event handlers and derivations read a page's
reactive state, and a route's handler reads the derivations its app
holds — a managed channel, synchronized by the runtime, not ambient
mutation.
