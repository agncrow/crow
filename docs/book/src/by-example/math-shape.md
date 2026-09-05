# Math: shape operations

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
