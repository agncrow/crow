# Neural-network building blocks: `use math.nn`

```crow
use math, math.nn

log[relu[[-1.0, 0.5, 2.0]]]
log[sigmoid[[0.0]]]
log[softmax[[1.0, 1.0]]]
log[mse[[1.0, 2.0], [1.0, 4.0]]]
log[fmt[bce[[0.9, 0.1], [1.0, 0.0]], decimals = 3]]
log[fmt[crossentropy[[[2.0, 0.0]], [0.0]], decimals = 3]]
log[fmt[gelu[[1.0]][0], decimals = 3]]
log[linear[[[1.0, 2.0]], [[1.0], [1.0]], [0.5]]]
```

```text
[0, 0.5, 2]
[0.5]
[0.5, 0.5]
2
0.105
0.127
0.841
[[3.5]]
```

Reference: spec section 13 (math → Neural Networks).
