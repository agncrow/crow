# Math: tensors and linear algebra

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
