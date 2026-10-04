# math.nn

Activation functions, losses, and a linear layer for small networks with autograd.


Import with `use math.nn`. Functions become top-level after import.

**Activations:**

| Function | Description | Example |
|---|---|---|
| `relu[x]` | Rectified linear unit: max(0, x) element-wise (NumArray) | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |
| `sigmoid[x]` | Sigmoid: 1/(1+exp(-x)) element-wise (NumArray) | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |
| `softmax[x]` | Softmax: exp(x)/sum(exp(x)) with numerical stability (NumArray) | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |
| `gelu[x]` | Gaussian error linear unit element-wise (NumArray) | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |

**Loss functions:**

| Function | Description | Example |
|---|---|---|
| `mse[pred, target]` | Mean squared error (Float) | [Automatic differentiation](../by-example/grad.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |
| `crossentropy[logits, targets]` | Cross-entropy loss with log-sum-exp stability (Float): logits `[classes]` with one target, or `[batch, classes]` with a target per row, each a class from 0 up to classes − 1 | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |
| `bce[pred, target]` | Binary cross-entropy with clamping (Float) | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |

**Layers:**

| Function | Description | Example |
|---|---|---|
| `linear[x, w, b]` | Linear layer: x @ w + b (NumArray) | [Neural-network building blocks: `use math.nn`](../by-example/nn.md) |

`math.nn` is written in Crow (`src/modules/nn.co`) over the math builtins, the first of the standard modules that are (design rule 20): each function is the formula its row above states — `relu` is `clip[x, 0.0, inf]`, `sigmoid` is `1.0 / (1.0 + exp[-x])`, `crossentropy` the mean over the rows of `log(sum(exp(row))) − row[target]` with the row's largest score taken out of the exponentials first — and the builtin table keeps each one's contract: what it takes (a list of numbers is read as a tensor, as any row's tensor is), what it answers, how it fails. The module is inlined into every program that imports it and runs in both modes as the program's own functions do; under `grad` its formulas are differentiated as any function is, so no rule of their own exists to disagree with them (Automatic Differentiation). Its tests are its own: `crow test src/modules/nn.co` runs the formulas at known values, their laws over a spread of inputs, and a gradient through every one against central differences.

Shapes `mse` cannot broadcast or `linear` cannot multiply (`x` and `w` matrices, `x`'s columns as many as `w`'s rows), two lengths given to `bce`, and a `crossentropy` target that is no class (or not a whole number, or one too few or too many for the rows of logits) stop the program with the reason, on the line of the call in the program (see When a Program Stops): `linear: x has 3 columns and w 2 rows — x * w needs them equal`, `crossentropy: target 7 is not a class of 3 (0 to 2)`.

**Design Note: a builtin written in Crow keeps its row.** Considered: inlining a Crow-written standard module as a user module is inlined, its functions reached by bare name (rejected: the call would lose the row's contract — a list of numbers converted to a tensor where the parameter takes one, a misuse refused before the program runs with the builtin's own words, `relu[] takes numbers, got an int` — and a mismatch inside the formula would be reported in the words of the formula's innards, `clip[] takes numbers`, naming a builtin the program never called). Decided (2026-10-04): the table states every builtin's contract whether its body is Rust or Crow; the lowering calls the Crow body where it would call the Rust function, with the arguments read as the row's parameters take them; the body's own stops name the line of the call in the program, never a line of the module's source. Private helpers of the module are renamed behind its prefix; a function only the module's tests use is not inlined. A module's function is read in the module's own scope: the program's functions, imports, aliases and elements are invisible in it — the protocol's local `lines`, indexed `lines[0]`, stays its local in a program with a `lines` of its own, and a program names its functions as it likes (parity 162) — and a module's rows are inlined only where the program reaches them, by a bare name, the module's, or as a value: a program that uses math without `load` carries none of the protocol. A row that takes a value JSON can hold (`sign`'s claims, `msgpack`'s value) hands its function the value's JSON text — the one form every such value takes in Crow, read back with `json[…]` — and the IR verifier holds every such function to its row: what it answers, and whether it can fail.

```
use math
use math.nn

x = linspace[-2, 2, 5]
log[relu[x]]              --- [0, 0, 0, 1, 2]
log[sigmoid[x]]           --- [0.119..., 0.269..., 0.5, 0.731..., 0.881...]
log[softmax[x]]           --- probabilities summing to 1

pred = linspace[0.1, 0.9, 3]
target = linspace[0, 1, 3]
log[mse[pred, target]]    --- 0.00667

input = reshape[linspace[1, 4, 4], 2, 2]
w = eye[2]
b = ones[2]
log[linear[input, w, b]]  --- input + bias
```



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
