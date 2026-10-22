# Variables

A variable is created by assigning to it. No keyword, no type annotation:

```crow
count = 10
name = 'crow'
ready = true
log[count]
```

```text
10
```

Reassignment is fine — but a variable's *type* cannot change:

```crow,error
items = [1, 2, 3]
items = 'three things'    --- error: cannot reassign from array to string
```

The one deliberate loosening: a variable ever assigned a float is a float
for its whole scope, because numeric code mixes `10` and `3.14` constantly:

```crow
x = 10
x = 3.14
log[x]
```

```text
3.14
```

Multiple assignment and swap work as you'd hope:

```crow
a, b = 1, 2
a, b = b, a
log[a]
log[b]
```

```text
2
1
```

Each name takes a value of its own, as many values as names. A list is
never taken apart into names — that is a compile error, which shows the
form to write instead:

```crow,error
fn pair[]
    return [3, 4]
c, d = pair[]    --- error: Crow takes no list apart into names
```

Hold the list, then read its items by position:

```crow
fn pair[]
    return [3, 4]
r = pair[]
c = r[0]
d = r[1]
log[c + d]
```

```text
7
```

One rule you'll meet early: **names you define can't collide with names
the language owns**. `range = [1, 2]` is a compile error (`range` is a
core builtin), and after `use math`, so is `sum = 0`. The error always
says what to do — rename, or import the module under an alias. Chapter 8
covers the full story.
