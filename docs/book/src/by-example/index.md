# Crow by Example

One short, runnable program per area of the language — the reference in
example form. Every block is executed by `crow test --book` in both
execution modes, and `crow test --docs` fails the build if any builtin or
module has no example here or in the book's chapters. Looking for a
specific name? [Where Is It?](../where-is-it.md) maps every builtin to the
pages that show it.

| Page | Covers |
|---|---|
| [Numbers and conversion](numbers.md) | `abs sqrt floor ceil round min max type int float str` |
| [Strings](strings.md) | `trim upper lower replace starts ends repeat split join reverse len`, indexing |
| [Formatting](formatting.md) | `fmt` with `decimals commas percent width fill align` |
| [Regular expressions](regex.md) | `matches find sub` |
| [Arrays](arrays.md) | `sort reverse slice push pop concat contains range` |
| [Maps](maps.md) | `map keys values has set del` |
| [Files and directories](files.md) | `mkdir write exists read cp mv ls rm`, error values from I/O |
| [JSON](json.md) | `json encode`, converting open-schema fields |
| [The system: shell, environment, time, arguments](system.md) | `shell env time args hash` |
| [Moments and freshness](moments.md) | `moment calendar parse_time now captured`, `fmt[t, time =]` |
| [Math: tensors and linear algebra](math.md) | matmul, `.*`, `transpose det solve sum mean zeros eye linspace argmax` |
| [Math: creating tensors](math-creation.md) | `zeros ones full eye diag arange linspace tensor randn seed` |
| [Math: element-wise functions](math-elementwise.md) | `exp ln log2 log10 sign clip mod` and trigonometry |
| [Math: reductions, sorting, and selection](math-reductions.md) | `prod argmin cumsum cumprod topk argsort unique where`, `axis =` |
| [Math: shape operations](math-shape.md) | `reshape flatten transpose stack concat` |
| [Math: linear algebra](math-linalg.md) | `inv norm dot triu tril rank cond solve lstsq pinv qr svd cholesky eigen` |
| [Math: random numbers and distributions](math-random.md) | `normal uniform poisson bernoulli gamma_dist beta_dist exponential shuffle choice permutation` |
| [Math: signal processing](math-signal.md) | `fft ifft fft2 conv conv2d hamming hanning stft spectrogram lowpass highpass bandpass` |
| [Math: special functions and complex numbers](math-special.md) | `gamma erf bessel factorial choose complex real imag magnitude phase conjugate` |
| [Math: sparse matrices](math-sparse.md) | `sparse dense nnz spmv` |
| [Automatic differentiation](grad.md) | `grad` for one and many parameters |
| [Statistics: `use math.stats`](stats.md) | `median std var percentile corr linreg` |
| [Neural-network building blocks: `use math.nn`](nn.md) | `relu sigmoid softmax mse` |
| [Structured files: `use io`](io.md) | `csv lines` |
| [Frames: `use frame`](frames.md) | `frame.load filter sort select head group`, column reductions |
| [Databases: `use db`](db.md) | `connect exec query transact` |
| [Web: a JSON route](web.md) | `route serve`, a JSON response |
| [Web: responses, auth, sessions, and sockets](web-more.md) | `text html redirect respond msgpack arrow bytes sign verify sessions cache limit templates render uploads send recv close smtp` |
| [Views: the whole element vocabulary](views-elements.md) | all 32 elements, `fetch poll field` |
