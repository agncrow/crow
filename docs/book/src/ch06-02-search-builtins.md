# Searching with any, all, and find

There is no `break` in a `for` loop — deliberately. A break-driven search
forces sequential execution, exactly what Crow's loops exist to avoid. In
its place, three builtins ask the three questions searches actually ask —
and because *Crow* implements the early exit, they stop across all cores
at once:

```crow
fn is_big[x]
    return x gt 1000

nums = [3, 12, 4021, 55]
log[any[nums, is_big]]
log[all[nums, is_big]]
log[find[nums, is_big]]
```

```text
true
false
4021
```

Predicates are **named functions** — the same functions-as-values idea
from chapter 4, and every predicate is independently testable.

`find` returns the first matching *element*. And when nothing matches? It
returns the value you already know how to read — an error value:

```crow
fn is_big[x]
    return x gt 1000

fn is_negative[x]
    return x lt 0

nums = [3, 12, 4021, 55]
missing = find[nums, is_negative]
log[missing.ok]
log[missing.reason]
```

```text
false
no match
```

No sentinel `-1`, no null, no exception — a miss is a value carrying its
own explanation, checked with `.ok` like every other fallible result.
