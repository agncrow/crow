# math

Tensors, element-wise math, reductions, linear algebra, random numbers, signal processing, sparse arrays, and numeric loading.


The `math` module is built in — no file needed. It provides tensor creation, element-wise operations, reductions, shape operations, and linear algebra. After `use math`, all functions are called directly (e.g. `zeros[3]`, not `math.zeros[3]`).

### Tensor Creation

| Function | Description | Example |
|---|---|---|
| `zeros[d1, d2, ...]` | Tensor of zeros with given shape | [Math: tensors](../by-example/math.md) |
| `ones[d1, d2, ...]` | Tensor of ones with given shape | [Math: tensors](../by-example/math.md) |
| `full[d1, d2, ..., value]` | Tensor filled with `value` (last arg) | [Math: tensors](../by-example/math.md) |
| `eye[n]` | n×n identity matrix | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) |
| `diag[tensor]` | Extract diagonal from a 2D tensor | [Math: tensors](../by-example/math.md) |
| `linspace[start, end, n]` | `n` evenly spaced values from `start` to `end` | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) |
| `arange[start, end, step]` | Values from `start` to `end` by `step` | [Math: tensors](../by-example/math.md) |
| `randn[d1, d2, ...]` | Normal random values (mean=0, std=1) | [Math: tensors](../by-example/math.md) |
| `tensor[array]` | Convert an array to a tensor | [Math: tensors](../by-example/math.md) |

### Element-wise Operations

| Function | Description | Example |
|---|---|---|
| `exp[t]` | Element-wise exponential | [Math: tensors](../by-example/math.md) |
| `ln[t]` | Element-wise natural log | [Math: tensors](../by-example/math.md) |
| `log2[t]` | Element-wise base-2 logarithm | [Math: tensors](../by-example/math.md) |
| `log10[t]` | Element-wise base-10 logarithm | [Math: tensors](../by-example/math.md) |
| `sqrt[t]` | Element-wise square root | [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
| `abs[t]` | Element-wise absolute value | [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `sign[t]` | Element-wise sign (-1, 0, or 1) | [Math: tensors](../by-example/math.md) |
| `floor[t]` | Element-wise floor | [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
| `ceil[t]` | Element-wise ceiling | [Numbers and conversion](../by-example/numbers.md) |
| `round[t]` | Element-wise round to nearest | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
| `clip[t, lo, hi]` | Clamp values to [lo, hi] | [Math: tensors](../by-example/math.md) |
| `mod[t, divisor]` | Element-wise modulo | [Math: tensors](../by-example/math.md) |
| `sin[t]` | Element-wise sine | [Math: tensors](../by-example/math.md) |
| `cos[t]` | Element-wise cosine | [Math: tensors](../by-example/math.md) |
| `tan[t]` | Element-wise tangent | [Math: tensors](../by-example/math.md) |
| `asin[t]` | Element-wise arcsine | [Math: tensors](../by-example/math.md) |
| `acos[t]` | Element-wise arccosine | [Math: tensors](../by-example/math.md) |
| `atan[t]` | Element-wise arctangent | [Math: tensors](../by-example/math.md) |
| `atan2[y, x]` | Element-wise two-argument arctangent | [Math: tensors](../by-example/math.md) |
| `sinh[t]` | Element-wise hyperbolic sine | [Math: tensors](../by-example/math.md) |
| `cosh[t]` | Element-wise hyperbolic cosine | [Math: tensors](../by-example/math.md) |
| `tanh[t]` | Element-wise hyperbolic tangent | [Math: tensors](../by-example/math.md) |

### Reductions

| Function | Returns | Example |
|---|---|---|
| `sum[t]` | Sum of all elements (float) | [Frames: `use frame`](../by-example/frames.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `mean[t]` | Mean of all elements (float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) · [Importing with use](../ch08-01-importing.md) |
| `min[t]` | Minimum element (float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Numbers and conversion](../by-example/numbers.md) |
| `max[t]` | Maximum element (float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Testing](../ch09-00-testing.md) |
| `prod[t]` | Product of all elements (float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `argmax[t]` | Index of maximum element (int) | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) |
| `argmin[t]` | Index of minimum element (int) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |

All reductions except `argmax`/`argmin` accept an optional axis parameter: `sum[t, axis]` reduces along that axis and returns a tensor instead of a scalar.

### Sorting & Selection

| Function | Returns | Example |
|---|---|---|
| `argsort[t]` | Indices that would sort the tensor (NumArray) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `topk[t, k]` | Top k largest values, sorted descending (NumArray) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `unique[t]` | Unique values, sorted (NumArray) | [Frames: `use frame`](../by-example/frames.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `where[cond, a, b]` | Element-wise: a where cond≠0, else b (NumArray) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |

Note: `sort[t]` is a core builtin (not in the math module) that works on both arrays and tensors.

### Cumulative Ops

| Function | Returns | Example |
|---|---|---|
| `cumsum[t]` | Cumulative sum (NumArray, same shape) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `cumprod[t]` | Cumulative product (NumArray, same shape) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |

### Combining Ops

| Function | Returns | Example |
|---|---|---|
| `stack[tensors]` | Stack tensors along new axis 0 (NumArray) | [Frames: `use frame`](../by-example/frames.md) · [Math: tensors](../by-example/math.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `stack[tensors, axis]` | Stack tensors along new axis (NumArray) | [Frames: `use frame`](../by-example/frames.md) · [Math: tensors](../by-example/math.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |

Note: `concat[a, b, axis]` is a core builtin (not in the math module) — one merge operation for all arrays and tensors. It replaced the former `append`.

Note: `slice[t, start, end]` is a core builtin (not in the math module) that works on arrays, tensors, and strings.

### Broadcasting

Element-wise operations on tensors with different shapes automatically broadcast following NumPy rules:
- Dimensions are right-aligned
- Size-1 dimensions expand to match the other tensor
- Missing dimensions are treated as size 1

```
use math
a = reshape[linspace[1, 3, 3], 3, 1]   --- shape [3, 1]
b = reshape[linspace[10, 40, 4], 1, 4]  --- shape [1, 4]
c = a + b                                          --- shape [3, 4]
```

Broadcasting works with all element-wise operators: `+`, `-`, `.*`, `./`, `.^`.

Incompatible shapes (e.g. `[3] + [4]`) produce a runtime error.

### Advanced Indexing

Tensors can be indexed with another tensor to gather elements by index:

```
a = linspace[10, 50, 5]
indices = linspace[0, 2, 3]
y = a[indices]                    --- gathers elements at indices 0, 1, 2
```

Negative indices are supported (wrap around from the end). The result tensor has the same shape as the index tensor.

### Shape Operations

| Function | Description | Example |
|---|---|---|
| `reshape[t, d1, d2, ...]` | Reshape to new dimensions | [Math: tensors](../by-example/math.md) |
| `flatten[t]` | Flatten to 1D | [Math: tensors](../by-example/math.md) |
| `transpose[t]` | Transpose (swap last two axes) | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) |

### Linear Algebra

| Function | Description | Example |
|---|---|---|
| `a * b` | Matrix multiplication (use `*` operator) | [JSON](../by-example/json.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `solve[a, b]` | Solve linear system Ax = b | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) |
| `inv[a]` | Matrix inverse | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `det[a]` | Determinant (returns float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) |
| `svd[a]` | SVD → returns [U, S, Vt] | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `qr[a]` | QR decomposition → returns [Q, R] | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `cholesky[a]` | Cholesky (L*L^T) for symmetric positive-definite | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `eigen[a]` | Eigenvalues → returns [real, imag] | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `norm[x]` | L2 (Euclidean) norm (returns float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `norm[x, p]` | Lp norm (p=1 for L1, inf for L∞) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `dot[a, b]` | Vector dot product (inner product, returns float) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `pinv[a]` | Moore-Penrose pseudo-inverse | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `lstsq[a, b]` | Least-squares solve: minimize ‖Ax − b‖² | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `triu[a]` | Upper triangular part of matrix | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `tril[a]` | Lower triangular part of matrix | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `rank[a]` | Matrix rank (count of singular values above tolerance) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `cond[a]` | Condition number (largest singular value / smallest) | [Math: reductions and linear algebra](../by-example/math-linalg.md) |

### Random & Distributions

| Function | Description | Example |
|---|---|---|
| `seed[n]` | Set global RNG seed for reproducibility | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) |
| `rand[d1, d2, ...]` | Uniform random in [0, 1) | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `randn[d1, d2, ...]` | Standard normal (mean=0, std=1) | [Math: tensors](../by-example/math.md) |
| `normal[mean, std, d1, d2, ...]` | Normal with custom mean/std | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `uniform[low, high, d1, d2, ...]` | Uniform in [low, high) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `poisson[lambda, d1, d2, ...]` | Poisson distribution | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `bernoulli[p, d1, d2, ...]` | Bernoulli (0.0 or 1.0) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `gamma_dist[alpha, beta, d1, ...]` | Gamma distribution | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `beta_dist[alpha, beta, d1, ...]` | Beta distribution | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `exponential[lambda, d1, ...]` | Exponential distribution | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `shuffle[x]` | Random permutation of tensor elements | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `choice[x, n]` | Random sample of n elements (no replacement) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `permutation[n]` | Random permutation of 0..n | [Math: random, signal, special, and sparse](../by-example/math-more.md) |

### Signal Processing

| Function | Description | Example |
|---|---|---|
| `fft[x]` | 1D FFT → returns [real, imag] tensors | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `ifft[real, imag]` | Inverse 1D FFT → returns real tensor | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `fft2[m]` | 2D FFT → returns [real, imag] tensors | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `conv[a, b]` | 1D convolution (full), length = len(a)+len(b)-1 | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `conv2d[a, b]` | 2D convolution (full) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `hamming[n]` | Hamming window of length n | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `hanning[n]` | Hanning (Hann) window of length n | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `stft[x, window_size, hop_size]` | Short-Time Fourier Transform → [real, imag] 2D tensors [frames × window_size] | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `spectrogram[x, window_size, hop_size]` | Power spectrogram (magnitude² of STFT) → 2D tensor | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `lowpass[x, cutoff_hz, sample_rate]` | FFT-based lowpass filter | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `highpass[x, cutoff_hz, sample_rate]` | FFT-based highpass filter | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `bandpass[x, low_hz, high_hz, sample_rate]` | FFT-based bandpass filter | [Math: random, signal, special, and sparse](../by-example/math-more.md) |

### Special Functions

| Function | Description | Example |
|---|---|---|
| `gamma[x]` | Gamma function Γ(x), element-wise | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `erf[x]` | Error function erf(x), element-wise | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `bessel[x, n]` | Bessel function J_n(x), element-wise | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `factorial[x]` | Factorial x! = Γ(x+1), element-wise | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `choose[n, k]` | Binomial coefficient C(n,k), element-wise | [Math: random, signal, special, and sparse](../by-example/math-more.md) |

### Complex Numbers

Complex values are represented as `[real_tensor, imag_tensor]` pairs.

| Function | Description | Example |
|---|---|---|
| `complex[re, im]` | Create complex value → [real, imag] | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `real[c]` | Extract real part tensor | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `imag[c]` | Extract imaginary part tensor | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `magnitude[c]` | Magnitude \|c\| = sqrt(re² + im²) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `phase[c]` | Phase angle = atan2(im, re) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `conjugate[c]` | Complex conjugate → [re, -im] | [Math: random, signal, special, and sparse](../by-example/math-more.md) |

### Sparse Arrays

Sparse matrices store only non-zero entries, using CSR (Compressed Sparse Row) format internally. They use the same operators as dense arrays — the runtime dispatches to sparse-optimized algorithms automatically.

| Function | Description | Example |
|---|---|---|
| `sparse[dense_array]` | Convert dense 2D array to sparse | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `sparse[rows, cols, vals, shape]` | Create sparse from COO triplets (row indices, column indices, values, [nrows, ncols]) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `dense[s]` | Convert sparse matrix back to dense array | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `nnz[s]` | Number of stored non-zero entries | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `spmv[s, v]` | Sparse matrix × dense vector (the same product as `s * v`) | [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `s * v` | Sparse × dense matmul (vector or matrix) | [Arrays](../by-example/arrays.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Controlling Resources](../ch06-03-resources.md) · [Testing](../ch09-00-testing.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `s + s2` | Sparse + sparse addition | [Arrays](../by-example/arrays.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Controlling Resources](../ch06-03-resources.md) · [Testing](../ch09-00-testing.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `solve[s, b]` | Solve sparse linear system s·x = b | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) |

```
use math

--- dense to sparse
s = sparse[eye[3]]
log[s]                              --- sparse[3x3, nnz=3]

--- COO creation
s2 = sparse[[0, 1, 2], [0, 1, 2], [5, 6, 7], [3, 3]]

--- sparse matmul
r = s * [1, 2, 3]

--- sparse add
s3 = s + s2

--- back to dense
d = dense[s3]

--- sparse solve
x = solve[s2, [10, 12, 14]]
```

### Data Loading

| Function | Description | Example |
|---|---|---|
| `load[path]` | Load numeric CSV file directly into a 2D tensor (error-value on failure) | [Databases: `use db`](../by-example/db.md) · [Frames: `use frame`](../by-example/frames.md) · [Web: responses and submodules](../by-example/web-more.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) |

`load[path]` reads a CSV file with a header row and parses all values as floats into a 2D tensor (rows × columns). This is much faster than `csv[path]` (io module) for numeric data because it avoids creating intermediate string maps.

```
use math

data = load['data.csv']
log[data.shape]        --- [1000, 10]
log[sum[data]]         --- total of all values
```

### Constants

| Constant | Description | Example |
|---|---|---|
| `pi` | π = 3.14159... |  |
| `e` | Euler's number e = 2.71828... | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `inf` | Positive infinity |  |
| `nan` | Not a number |  |

### Properties

| Property | Description | Example |
|---|---|---|
| `.shape` | Array of dimension sizes | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Controlling Resources](../ch06-03-resources.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Names Are Never Ambiguous](../ch08-03-names.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `.len` | Total number of elements | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Controlling Resources](../ch06-03-resources.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Names Are Never Ambiguous](../ch08-03-names.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `.dtype` | Data type (`'float'`) | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Controlling Resources](../ch06-03-resources.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Names Are Never Ambiguous](../ch08-03-names.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `.device` | Where the tensor lives (`'cpu'` or `'gpu'`) | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Controlling Resources](../ch06-03-resources.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Names Are Never Ambiguous](../ch08-03-names.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `.grad` | Gradient status (`'none'` if no gradient) | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Controlling Resources](../ch06-03-resources.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Names Are Never Ambiguous](../ch08-03-names.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |

```
use math

a = zeros[3, 3]
b = eye[3]
c = a * b
d = det[b]           --- 1.0

--- constants
log[pi]               --- 3.141592653589793

--- properties
log[a.shape]               --- [3, 3]
log[a.dtype]               --- float
log[a.device]              --- cpu
```



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
