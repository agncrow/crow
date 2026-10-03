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

A function of tensors is differentiated backward: it runs once on traced tensors, and every parameter's gradient comes from that one run. The gradient of several parameters is a list, one tensor per parameter in order.

```crow
use math, math.nn

fn loss[w, x, y]
    return mse[flatten[x * w], y]

x = [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
y = [1.5, 3.0, 4.5]
w = zeros[2, 1]
grads = grad[loss][w, x, y]
log[grads[0].shape]
log[grads[0]]
```

```text
[2, 1]
[[-4], [-5]]
```

Reference: spec section 14 (Automatic Differentiation).
