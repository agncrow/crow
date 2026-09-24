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

Chain conditions with `else if` (there is no `elif` keyword — it's just an
`if` in the `else` branch):

```crow
score = 72

if score gte 90
    log['A']
else if score gte 70
    log['B']
else
    log['C']
```

```text
B
```

There is no ternary expression: use an `if`/`else` statement, and set
the variable before it. A variable set inside a branch belongs to that
branch and ends with it, so a value the rest of the program reads is set
first and changed inside:

```crow
n = 42
size = 'small'
if n gt 10
    size = 'big'
log[size]
```

```text
big
```

Reading a variable after the branch that set it ended is refused before
the program runs, with the line and this fix.

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
values, and when its body only computes, it runs its iterations **in
parallel** on every core. That story deserves its own chapter, and it's
chapter 6.

```crow
doubled = for n in [1, 2, 3]
    n * 2
log[doubled]
```

```text
[2, 4, 6]
```
