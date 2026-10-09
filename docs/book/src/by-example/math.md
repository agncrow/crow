# Math: tensors

Tensors are Crow's numeric arrays: creating them, mapping functions over them, and reshaping them.

## Tensors and linear algebra

```crow
use math

a = [[2.0, 1.0], [1.0, 3.0]]
log[a * [[1.0], [1.0]]]
log[a .* a]
log[transpose[a]]
log[det[a]]
x = solve[a, [1.0, 2.0]]
log[round[x[0] * 100.0]]
log[sum[[1.0, 2.0, 3.0]]]
log[mean[[1.0, 2.0, 3.0]]]
log[zeros[2].shape]
log[eye[2]]
log[linspace[0.0, 1.0, 3]]
log[argmax[[3.0, 9.0, 1.0]]]
```

```text
[[3], [4]]
[[4, 1], [1, 9]]
[[2, 1], [1, 3]]
5
20
6
2
[2]
[[1, 0], [0, 1]]
[0, 0.5, 1]
1
```

Reference: spec section 13 (math) and section 4 (Tensor Arithmetic).

## Creating tensors

```crow
use math

log[zeros[2, 3]]
log[ones[3]]
log[full[2, 2, 7.0]]
log[eye[2]]
log[onehot[[0.0, 2.0, 1.0], 3]]
log[diag[[[1.0, 2.0], [3.0, 4.0]]]]
log[arange[0, 5, 1]]
log[linspace[0.0, 1.0, 5]]
t = tensor[[1, 2, 3]]
log[t.shape]
seed[42]
r = randn[2, 3]
log[r.shape]
```

```text
[[0, 0, 0], [0, 0, 0]]
[1, 1, 1]
[[7, 7], [7, 7]]
[[1, 0], [0, 1]]
[[1, 0, 0], [0, 0, 1], [0, 1, 0]]
[1, 4]
[0, 1, 2, 3, 4]
[0, 0.25, 0.5, 0.75, 1]
[3]
[2, 3]
```

`seed[n]` makes every random function reproducible for the rest of the
program; `randn` draws standard-normal values, so the example prints its
shape rather than its contents.

Reference: spec section 13 (math → Tensor Creation).

## Element-wise functions

Every function here maps over a tensor and returns a tensor of the same
shape; on a plain number it returns a number.

```crow
use math

x = [1.0, 4.0, 9.0]
log[sqrt[x]]
log[exp[[0.0, 1.0]]]
log[ln[[1.0, e]]]
log[log2[[8.0]]]
log[log10[[1000.0]]]
log[sign[[-2.0, 0.0, 5.0]]]
log[clip[[-5.0, 0.5, 5.0], 0.0, 1.0]]
log[mod[[7.0, 8.0, 9.0], 3.0]]
log[abs[[-1.5, 2.0]]]
log[floor[[1.7, -1.2]]]
log[round[[1.4, 1.6]]]
```

```text
[1, 2, 3]
[1, 2.718281828459045]
[0, 1]
[3]
[3]
[-1, 0, 1]
[0, 0.5, 1]
[1, 2, 0]
[1.5, 2]
[1, -2]
[1, 2]
```

Trigonometry and hyperbolics take tensors (wrap a single number in
brackets). Angles are radians:

```crow
use math

fn show[t]
    return fmt[t[0], decimals = 3]

log[show[sin[[pi / 2.0]]]]
log[show[cos[[0.0]]]]
log[show[tan[[pi / 4.0]]]]
log[show[asin[[1.0]]]]
log[show[acos[[1.0]]]]
log[show[atan[[1.0]]]]
log[show[atan2[[1.0], [1.0]]]]
log[show[sinh[[0.0]]]]
log[show[cosh[[0.0]]]]
log[show[tanh[[1.0]]]]
```

```text
1.000
1.000
1.000
1.571
0.000
0.785
0.785
0.000
1.000
0.762
```

Reference: spec section 13 (math → Element-wise Operations).

## Shape operations

```crow
use math

t = linspace[1.0, 6.0, 6]
m = reshape[t, 2, 3]
log[m]
log[m.shape]
log[flatten[m]]
log[transpose[m]]
log[stack[[[1.0, 2.0], [3.0, 4.0]]]]
log[concat[[1.0, 2.0], [3.0], 0]]
```

```text
[[1, 2, 3], [4, 5, 6]]
[2, 3]
[1, 2, 3, 4, 5, 6]
[[1, 4], [2, 5], [3, 6]]
[[1, 2], [3, 4]]
[1, 2, 3]
```

Reference: spec section 13 (math → Shape Operations, Combining Ops).

```crow
use math
x = [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]
labels = [0, 1, 0]
log[x[labels eq 0]]
log[mean[x[labels eq 0], 0]]
v = [10.0, 20.0, 30.0]
log[sum[v gt 15.0]]
log[where[v gt 15.0, v, 0.0 * v]]
```

```text
[[1, 2], [5, 6]]
[3, 4]
2
[0, 20, 30]
```
