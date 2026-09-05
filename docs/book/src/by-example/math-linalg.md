# Math: reductions and linear algebra

Collapsing a tensor to numbers, ordering it, and the matrix operations.

## Reductions, sorting, and selection

```crow
use math

x = [3.0, 1.0, 4.0, 1.0, 5.0]
log[sum[x]]
log[prod[[1.0, 2.0, 3.0, 4.0]]]
log[mean[x]]
log[min[x]]
log[max[x]]
log[argmax[x]]
log[argmin[x]]
log[cumsum[[1.0, 2.0, 3.0]]]
log[cumprod[[1.0, 2.0, 3.0]]]
log[topk[x, 2]]
log[argsort[x]]
log[unique[x]]
log[where[[1.0, 0.0, 1.0], [10.0, 20.0, 30.0], [0.0, 0.0, 0.0]]]

m = [[1.0, 2.0], [3.0, 4.0]]
log[sum[m, axis = 0]]
log[sum[m, axis = 1]]
```

```text
14
24
2.8
1
5
4
1
[1, 3, 6]
[1, 2, 6]
[5, 4]
[1, 3, 0, 2, 4]
[1, 3, 4, 5]
[10, 0, 30]
[4, 6]
[3, 7]
```

Reference: spec section 13 (math → Reductions, Sorting & Selection, Cumulative Ops).

## Linear algebra

`*` between two tensors is matrix multiplication; the functions below are
the rest of the toolkit. Decompositions return arrays of tensors, so the
examples check them by reconstruction rather than printing every element.

```crow
use math

a = [[4.0, 1.0], [2.0, 3.0]]
log[a * eye[2]]
log[det[a]]
log[inv[a]]
log[fmt[norm[[3.0, 4.0]], decimals = 1]]
log[dot[[1.0, 2.0], [3.0, 4.0]]]
log[triu[a]]
log[tril[a]]
log[rank[a]]
log[fmt[cond[eye[2]], decimals = 1]]
x = solve[a, [1.0, 2.0]]
log[fmt[x[1], decimals = 1]]
x2 = lstsq[a, [1.0, 2.0]]
log[fmt[x2[1], decimals = 1]]
log[pinv[eye[2]]]

qr_parts = qr[a]
back = qr_parts[0] * qr_parts[1]
log[round[sum[back]]]
svd_parts = svd[a]
log[svd_parts[1].shape]
spd = [[4.0, 2.0], [2.0, 3.0]]
l = cholesky[spd]
log[round[sum[l * transpose[l]]]]
ev = eigen[[[2.0, 0.0], [0.0, 3.0]]]
log[sort[ev[0]]]
```

```text
[[4, 1], [2, 3]]
10
[[0.3, -0.1], [-0.2, 0.4]]
5.0
11
[[4, 1], [0, 3]]
[[4, 0], [2, 3]]
2
1.0
0.6
0.6
[[1, 0], [0, 1]]
10
[2]
11
[2, 3]
```

Reference: spec section 13 (math → Linear Algebra) and section 4 (Tensor Arithmetic).
