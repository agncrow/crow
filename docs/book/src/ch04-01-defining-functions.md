# Defining and Calling Functions

`fn`, a name, parameters in square brackets, an indented body:

```crow
fn area[w, h]
    return w * h

log[area[3, 4]]
```

```text
12
```

The last expression in a body is the return value, so `return` on the
final line is optional:

```crow
fn cube[x]
    x * x * x

log[cube[3]]
```

```text
27
```

## Functions Are Values

A function name without brackets *is* the function — store it, pass it,
call it later. This is how Crow does everything other languages use
lambdas for (Crow has no lambdas — a deliberate choice; named functions
keep every callsite greppable and every behavior testable):

```crow
fn cube[x]
    x * x * x

op = cube
log[op[2]]

fn apply_twice[f, x]
    return f[f[x]]

log[apply_twice[cube, 2]]
```

```text
8
512
```
