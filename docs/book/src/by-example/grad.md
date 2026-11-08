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

A function of numbers may use arithmetic and `^`, and `abs`, `sign`, `sqrt`, `exp`, `ln`, `log2`, `log10`, `log1p`, `expm1`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `erf`, `floor`, `ceil` and `round`:

```crow
use math

fn h[x]
    return sin[x] * exp[x]

dh = grad[h]
log[dh[0.0]]
```

```text
1
```

A function of tensors is differentiated backward: it runs once on traced tensors, and every parameter's gradient comes from that one run. The gradient of several parameters is a list, one tensor per parameter in order.

```crow
use math

fn loss[w, x, y]
    d = flatten[x * w] - y
    return mean[d .* d]

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

Inside the function a tensor is read as it is anywhere: a `slice` of its items, a number or a row by its position, the numbers at a tensor or list of positions, two tensors joined by `concat`. A model can so keep all its weights in one vector, each layer's read out of it, and one gradient covers them all:

```crow
use math

fn loss[theta, x, y]
    w = reshape[slice[theta, 0, 2], 2, 1]
    b = theta[2]
    d = flatten[x * w] + b - y
    return mean[d .* d]

x = [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
y = [1.5, 3.0, 4.5]
log[grad[loss][zeros[3], x, y][0]]
```

```text
[-4, -5, -6]
```

A tensor compared with a number is its mask, a constant of the tape, and
`where[mask, a, b]` takes a side: the gradient reaches `a` where the mask
holds and `b` where it does not. A rectifier is written so:

```crow
use math

fn loss[w]
    z = w - 1.0
    r = where[z gt 0.0, z, zeros[3]]
    return sum[r .* r]

log[grad[loss][tensor[[0.5, 2.0, 3.0]]]]
```

```text
[0, 2, 4]
```

A draw inside the function — `bernoulli`'s mask, `uniform`'s noise — is
drawn as the function runs and is a constant of the tape, so a dropout
keeps the gradient of the weights it kept:

```crow
use math

fn loss[w]
    keep = bernoulli[0.5, 4]
    return sum[w .* keep * 2.0]

seed[7]
log[grad[loss][tensor[[1.0, 2.0, 3.0, 4.0]]]]
```

```text
[2, 2, 2, 0]
```

Reference: spec section 14 (Automatic Differentiation).
