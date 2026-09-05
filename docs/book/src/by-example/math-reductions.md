# Math: reductions, sorting, and selection

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
