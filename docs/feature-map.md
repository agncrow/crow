# Crow Feature Inventory

Quick-reference lookup table. **Source of truth:** `docs/language-spec.md`.

## Operators

| Syntax | What it does | Tensor behavior |
|--------|-------------|-----------------|
| `+` `-` | Add / subtract | Element-wise (always) |
| `*` | Multiply | **Matmul** (tensor×tensor), scalar multiply (tensor×scalar) |
| `.*` | — | Element-wise multiply |
| `/` | Divide | Scalar divide only (tensor÷tensor → error) |
| `./` | — | Element-wise divide |
| `^` | Power | Scalar power |
| `.^` | — | Element-wise power |
| `%` | Modulo | — |
| `eq ne gt lt gte lte` | Comparison | — |
| `and or not` | Logical | — |
| `then` | Pipe/chain | `x then f` = `f[x]` |

## Core Builtins (no import needed)

| Function | Category |
|----------|----------|
| `log[v]` | I/O |
| `range[...]` | Iteration |
| `map[]` | Collections |
| `assert[cond]` `assert[cond, msg]` | Control |
| `error[reason]` `error[reason, kind]` | Errors |
| `cpu[n]` `gpu[n]` `nodes[n]` | Resources |
| `abs[n]` `sqrt[n]` `floor[n]` `ceil[n]` `round[n]` | Math |
| `min[a, b]` `max[a, b]` | Math |
| `len[x]` | Collections |
| `push[a, v]` `pop[a]` `append[a, b]` | Array ops |
| `sort[a]` `reverse[a]` `contains[a, v]` `slice[a, s, e]` | Array ops |
| `keys[m]` `values[m]` `has[m, k]` `set[m, k, v]` `del[m, k]` | Map ops |
| `split[s, d]` `trim[s]` `replace[s, o, n]` `upper[s]` `lower[s]` | Strings |
| `starts[s, p]` `ends[s, p]` `join[a, s]` `repeat[s, n]` `pad[s, n]` `reverse[s]` | Strings |
| `int[v]` `float[v]` `str[v]` `type[v]` | Conversion |
| `read[p]` `write[p, c]` `args[]` `input[]` `input[prompt]` | I/O |
| `exists[p]` `mkdir[p]` `ls[p]` `rm[p]` `mv[s, d]` `cp[s, d]` | File System |
| `json[s]` `encode[v]` | JSON |
| `rand[...]` `time[]` `sleep[ms]` `env[name]` `exit[]` `exit[code]` | System |

## `use math` — Core Math Module

| Function | Sub-category |
|----------|-------------|
| `zeros` `ones` `full` `eye` `diag` `linspace` `arange` `randn` `tensor` | Creation |
| `exp` `ln` `log2` `log10` `sqrt` `abs` `sign` `floor` `ceil` `round` `clip` `mod` | Element-wise |
| `sin` `cos` `tan` `asin` `acos` `atan` `atan2` | Trigonometric |
| `sinh` `cosh` `tanh` | Hyperbolic |
| `sum` `mean` `min` `max` `prod` `argmax` `argmin` | Reductions |
| `cumsum` `cumprod` | Cumulative |
| `argsort` `topk` `unique` `where` | Sorting/selection |
| `concat` `stack` | Combining |
| `reshape` `flatten` `transpose` | Shape |
| `solve` `inv` `det` `svd` `qr` `cholesky` `eigen` `norm` `dot` `pinv` `lstsq` `triu` `tril` | Linear algebra |
| `seed` `randn` `normal` `uniform` `poisson` `bernoulli` `gamma_dist` `beta_dist` `exponential` `shuffle` `choice` `permutation` | Random |
| `fft` `ifft` `fft2` `conv` `conv2d` `hamming` `hanning` | Signal |
| `gamma` `erf` `bessel` `factorial` `choose` | Special |
| `complex` `real` `imag` `magnitude` `phase` `conjugate` | Complex |
| `sparse` `dense` `spmv` | Sparse |
| `load` | Data loading |
| `pi` `e` `inf` `nan` | Constants |
| `.shape` `.len` `.dtype` `.device` `.grad` | Properties |

**Note:** `sort[t]` and `slice[t, s, e]` are core builtins — they work on arrays AND tensors without `use math`.

## `use math.stats` — Statistics

| Function | Description |
|----------|-------------|
| `median` | Median of 1D array |
| `std` | Population standard deviation |
| `var` | Population variance |
| `percentile` | p-th percentile (0–100) |
| `zscore` | Z-score normalization |
| `corr` | Pearson correlation |
| `cov` | Population covariance |
| `ttest` | Welch's two-sample t-test → `{t, p}` |
| `linreg` | Simple linear regression → `{slope, intercept, r2}` |

## `use math.nn` — Neural Networks

| Function | Description |
|----------|-------------|
| `relu` `sigmoid` `softmax` `gelu` | Activations |
| `mse` `crossentropy` `bce` | Loss functions |
| `linear` | Layer: `x @ w + b` |

## `use frame` — DataFrames

| Function | Description |
|----------|-------------|
| `load[path]` | Load CSV → frame |
| `save[df, path]` | Write frame → CSV |
| `select[df, [cols]]` | Pick columns |
| `filter[df, mask]` | Filter rows by boolean mask |
| `sort[df, col]` | Sort by column (default asc, `sort[df, col, desc]` for desc) |
| `group[df, col, [col, agg]...]` | Group and aggregate (sum, mean, min, max, count) |
| `join[df1, df2, key]` | Join frames on key (default inner; `join[df1, df2, key, left]`) |
| `stack[df1, df2]` | Stack frames vertically |
| `head[df, n]` | First N rows |
| `tail[df, n]` | Last N rows |
| `fill[df, col, val]` | Fill missing values in column |
| `unique[df.col]` | Unique values in column |
| `rename[df, old, new]` | Rename column |
| `stats[df]` | Summary statistics |
| `datetime[s]` | Construct datetime |
| `tz[d, offset]` | Convert timezone |

Properties: `.shape`, `.columns`, `.dtypes`, `df.colname` (column access), `df.col.ok` (validity)

Column arithmetic: `df.salary * 1.1`, `df.age > 50` (returns boolean mask for `filter`)

Frame → tensor: `tensor[df]` or `tensor[df, [col1, col2]]`

## `use io` — Structured I/O

| Function | Description |
|----------|-------------|
| `csv[path]` | Parse CSV → array of maps |
| `lines[path]` | Read file → array of lines |

## Advanced Features

| Feature | Syntax | Location |
|---------|--------|----------|
| Matmul | `*` operator | Operator (no function needed) |
| Power | `^` / `.^` operators | Operator (no function needed) |
| Autograd | `grad[f]` | Core builtin |
| GPU | `gpu[n]` | Core builtin |
| Distributed | `nodes[n]` + `crow launch` | Core builtin + CLI |
| Parallel loops | `for x in items` | Automatic (rayon) |
| Error values | `.ok` `.reason` `.kind` `.trace` | Every value |
| Structs | `struct Name [fields]` | Language syntax |
| String interpolation | `'{expr}'` | Language syntax |
| Pipe chaining | `x then f` | Operator |

## NOT Implemented (by design — see Design Notes in spec)

| Feature | Reason | Use instead |
|---------|--------|-------------|
| Lambdas | One way to define functions | Named `fn` |
| `enumerate` / `zip` | Homogeneous arrays | `range[len[arr]]` |
| `reduce` / `any` / `all` | Loops cover it | `for` / `while` |
| `find` / `index` | Loop covers it | `for i in range[len[x]]` |
| `+=` `-=` etc. | One way to assign | `x = x + 1` |
| `"strings"` | One string delimiter | `'strings'` |
| String `+` | One way to combine | `'{a}{b}'` |
| Ternary | One way to branch | `if/else` |
| Pattern matching | No sum types | `if/else` |
| `map[arr, fn]` | `for`-as-expression | `for x in arr / x * 2` |
| `chars[s]` | Indexing covers it | `for i in range[len[s]] / s[i]` |
| Iterators | `for` + `slice` cover it | `for`-as-expression |
| Destructuring | One way to access | `result[0]`, `result[1]` |
| Inline error fallback | Errors should be visible | `if not x.ok` |

