# Automatic differentiation

```crow
fn f[x]
    return x * x + 3.0 * x

df = grad[f]
log[df[2.0]]

fn g[x, y]
    return x * y

dg = grad[g]
log[dg[3.0, 4.0]]
```

```text
7
[4, 3]
```

Reference: spec section 14 (Automatic Differentiation).
