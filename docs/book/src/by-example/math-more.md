# Math: random, signal, special, and sparse

The specialised corners of `use math`: distributions, transforms and filters, special functions and complex numbers, and sparse matrices.

## Random numbers and distributions

Random draws print their shape and a property rather than their values —
`seed[n]` makes them reproducible, but the point of these examples is the
call shape.

```crow
use math

fn in_range[v]
    return v gte 2.0 and v lt 5.0

seed[7]
n = normal[0.0, 1.0, 4]
log[n.shape]
u = uniform[2.0, 5.0, 3]
log[all[u, in_range]]
p = poisson[3.5, 5]
log[p.shape]
b = bernoulli[0.5, 6]
log[b.shape]
g = gamma_dist[2.0, 1.0, 3]
log[g.shape]
bt = beta_dist[2.0, 2.0, 3]
log[bt.shape]
ex = exponential[1.5, 3]
log[ex.shape]
s = shuffle[linspace[1.0, 5.0, 5]]
log[sum[s]]
c = choice[linspace[1.0, 10.0, 10], 3]
log[len[c]]
perm = permutation[5]
log[sum[perm]]
log[rand[] lt 1.0]
```

```text
[4]
true
[5]
[6]
[3]
[3]
[3]
15
3
10
true
```

Reference: spec section 13 (math → Random & Distributions).

## Signal processing

```crow
use math

spec = fft[[1.0, 0.0, 0.0, 0.0]]
log[spec[0]]
log[spec[1]]
back = ifft[spec[0], spec[1]]
log[round[back[0]]]
f2 = fft2[[[1.0, 0.0], [0.0, 0.0]]]
log[f2[0]]
log[conv[[1.0, 2.0, 3.0], [1.0, 1.0]]]
log[conv2d[[[1.0, 1.0], [1.0, 1.0]], [[1.0]]]]
log[fmt[hamming[3][1], decimals = 2]]
log[fmt[hanning[3][1], decimals = 2]]

wave = linspace[0.0, 1.0, 64]
frames = stft[wave, 16, 8]
log[frames[0].shape]
log[spectrogram[wave, 16, 8].shape]
log[lowpass[wave, 5.0, 64.0].shape]
log[highpass[wave, 5.0, 64.0].shape]
log[bandpass[wave, 2.0, 10.0, 64.0].shape]
```

```text
[1, 1, 1, 1]
[0, 0, 0, 0]
1
[[1, 1], [1, 1]]
[1, 3, 5, 3]
[[1, 1], [1, 1]]
1.00
1.00
[7, 16]
[7, 16]
[64]
[64]
[64]
```

Reference: spec section 13 (math → Signal Processing).

## Special functions and complex numbers

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

## Sparse matrices

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
