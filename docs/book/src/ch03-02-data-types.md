# Data Types

The scalar types are `int`, `float`, `bool`, and `string`. Arithmetic is
familiar, with two things worth pinning down.

**Division between ints floors; a float anywhere makes it float:**

```crow
log[3 / 2]
log[3 / 2.0]
```

```text
1
1.5
```

**Exponentiation is `^`** (not `**`), and `%` is remainder:

```crow
log[2 ^ 10]
log[7 % 3]
```

```text
1024
1
```

Convert between types with `int[...]`, `float[...]`, `str[...]`. Converting
a string can fail — which returns an error *value*, not an exception (you
met this in the guessing game; chapter 7 goes deep):

```crow
log[int[3.9]]
log[float['2.5']]
log[str[42]]
```

```text
3
2.5
42
```

Collections — arrays, tensors, maps, structs — get their own chapter (6).
