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

`math.nn` is written in Crow over the math builtins (`src/modules/nn.co`): each function's row in the builtin table is its contract, and under `grad` the formulas differentiate as any function does. A misuse stops the program with the reason, on the line of the call (this block builds but is not run):

```crow,norun
use math, math.nn

w = [[1.0, 0.0, 0.0], [0.0, 1.0, 1.0]]
log[linear[[[1.0, 2.0, 3.0]], w, [1.0, 1.0, 1.0]]]
```

```text
runtime error: line 4: linear: x has 3 columns and w 2 rows — x * w needs them equal
```

Reference: spec section 13 (math → Neural Networks).
