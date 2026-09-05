# Math: creating tensors

```crow
use math

log[zeros[2, 3]]
log[ones[3]]
log[full[2, 2, 7.0]]
log[eye[2]]
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
