# Control Flow

## if / else

Conditions use word operators — `eq`, `ne`, `lt`, `gt`, `lte`, `gte`,
combined with `and`, `or`, `not`. Blocks are indentation, no colons:

```crow
age = 20

if age gte 18 and age lt 65
    log['working age']
else
    log['not working age']
```

```text
working age
```

There is no `elif` — nest an `if` under `else` (you saw this in the game).
There is also no ternary expression: use an `if`/`else` statement.

## while

```crow
i = 0
while i lt 3
    i = i + 1
log[i]
```

```text
3
```

(No `+=` in Crow — write `i = i + 1`. The compiler optimizes it the same.)

## for

`for` iterates arrays and ranges:

```crow
total = 0
for n in [1, 2, 3, 4]
    total = total + n
log[total]
```

```text
10
```

But `for` is bigger than this in Crow: it's an *expression* that collects
values, and it runs its iterations **in parallel** — which is why we won't
print from inside a loop body (output order is unspecified). That story
deserves its own chapter, and it's next-but-one: chapter 5.

```crow
doubled = for n in [1, 2, 3]
    n * 2
log[doubled]
```

```text
[2, 4, 6]
```
