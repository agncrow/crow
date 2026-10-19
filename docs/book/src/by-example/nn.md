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

`math.nn` is written in Crow over the math builtins (`src/modules/nn.co`): each function is its own declaration, typed by its use, and under `grad` the formulas differentiate as any function does. A misuse is refused where the formula first cannot take it, naming the function and the line of the call — before the program runs where the shapes are written, as here, and as a stop with the same reason where they are only known as it runs:

```crow,error
use math, math.nn

w = [[1.0, 0.0, 0.0], [0.0, 1.0, 1.0]]
log[linear[[[1.0, 2.0, 3.0]], w, [1.0, 1.0, 1.0]]]
```

```text
nn.co:4:1: in linear: matmul: incompatible shapes [1, 3] × [2, 3] — the first's columns (3) must be the second's rows (2)
```

Reference: spec section 13 (math → Neural Networks).
