# Math: sparse matrices

```crow
use math

s = sparse[[[1.0, 0.0, 0.0], [0.0, 0.0, 2.0]]]
log[nnz[s]]
log[dense[s]]
log[spmv[s, [1.0, 1.0, 1.0]]]
log[s * [1.0, 1.0, 1.0]]
```

```text
2
[[1, 0, 0], [0, 0, 2]]
[1, 2]
[1, 2]
```

Reference: spec section 13 (math → Sparse Arrays).
