# Math: special functions and complex numbers

```crow
use math

log[round[gamma[[1.0, 5.0]]]]
log[fmt[erf[[1.0]][0], decimals = 3]]
log[fmt[bessel[[1.0], 0][0], decimals = 3]]
log[round[factorial[[0.0, 5.0]]]]
log[choose[[5.0], [2.0]]]

z = complex[3.0, 4.0]
log[real[z][0]]
log[imag[z][0]]
log[magnitude[z][0]]
log[fmt[phase[complex[0.0, 1.0]][0], decimals = 3]]
log[conjugate[z]]
```

```text
[1, 24]
0.843
0.765
[1, 120]
[10]
3
4
5
1.571
[[3], [-4]]
```

A complex value is a `[real, imag]` pair of one-element tensors, which is
why the parts are indexed with `[0]` and the conjugate prints as a pair.

Reference: spec section 13 (math → Special Functions, Complex Numbers).
