# Math: element-wise functions

Every function here maps over a tensor and returns a tensor of the same
shape; on a plain number it returns a number.

```crow
use math

x = [1.0, 4.0, 9.0]
log[sqrt[x]]
log[exp[[0.0, 1.0]]]
log[ln[[1.0, e]]]
log[log2[[8.0]]]
log[log10[[1000.0]]]
log[sign[[-2.0, 0.0, 5.0]]]
log[clip[[-5.0, 0.5, 5.0], 0.0, 1.0]]
log[mod[[7.0, 8.0, 9.0], 3.0]]
log[abs[[-1.5, 2.0]]]
log[floor[[1.7, -1.2]]]
log[round[[1.4, 1.6]]]
```

```text
[1, 2, 3]
[1, 2.718281828459045]
[0, 1]
[3]
[3]
[-1, 0, 1]
[0, 0.5, 1]
[1, 2, 0]
[1.5, 2]
[1, -2]
[1, 2]
```

Trigonometry and hyperbolics take tensors (wrap a single number in
brackets). Angles are radians:

```crow
use math

fn show[t]
    return fmt[t[0], decimals = 3]

log[show[sin[[pi / 2.0]]]]
log[show[cos[[0.0]]]]
log[show[tan[[pi / 4.0]]]]
log[show[asin[[1.0]]]]
log[show[acos[[1.0]]]]
log[show[atan[[1.0]]]]
log[show[atan2[[1.0], [1.0]]]]
log[show[sinh[[0.0]]]]
log[show[cosh[[0.0]]]]
log[show[tanh[[1.0]]]]
```

```text
1.000
1.000
1.000
1.571
0.000
0.785
0.785
0.000
1.000
0.762
```

Reference: spec section 13 (math → Element-wise Operations).
