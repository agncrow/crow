# models

Five small models written in Crow, on `math` and `math.nn`: least squares,
logistic regression, k-means, a decision tree and a one-hidden-layer
network. Each one fits the same way every run, and answers the same in
`crow run -i` and `crow run`.

```
deps = map[models = 'github.com/agncrow/crow/packages/models@main']
```

`crow install` checks the package out into `crow_modules/models`, and
`crow.lock` pins the commit it took. A program imports the models it uses:

```
use models.regression, models.tree
```

## The models

`x` holds a row of features per sample, as a nested array or a matrix; `y`
holds a target (or a class) for each.

| File | Fit | Ask |
|------|-----|-----|
| `regression` | `fit[x, y]` → `Linear[coef, intercept, r2]` | `predict[m, x]` — a number per row |
| `logistic` | `fit[x, y, steps, rate]` → `Logistic[coef, intercept]`, `y` of 0s and 1s | `probability[m, x]`, `classify[m, x]` — a 0 or 1 per row |
| `kmeans` | `fit[x, k, rounds]` → `Clusters[centres, labels]` | `assign[c, r]` — the nearest centre's index |
| `tree` | `fit[x, y, depth]` → the root `Node`, `y` of int classes | `classify[t, r]` — a class for one sample |
| `mlp` | `fit[x, y, hidden, steps, rate, seed]` → `Net[w1, b1, w2, b2]` | `probability[net, x]`, `classify[net, x]` |

- `regression` solves the centred inputs by least squares (`lstsq`), so the
  intercept is exact and `r2` says how much of `y`'s variance the fit explains.
  `linreg` in `math.stats` is the one-input case.
- `logistic` descends the mean binary cross-entropy, with the weights held
  as a column so that `x * w` is the matrix product.
- `kmeans` takes the first `k` samples as its first centres and stops early
  when no sample changes centre.
- `tree` splits on the feature and threshold that leave the least Gini
  impurity, down to `depth` levels or a pure node.
- `mlp` has ReLU hidden units and a sigmoid output, with backpropagation
  written out; its weights start from `seed`.

```
use models.regression

m = regression.fit[[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]], [1.0, 3.0, -2.0, 0.0]]
log[regression.predict[m, [[2.0, 1.0]]]]
```

## Tests

Each file has its own `test` block, and `example.co` fits every model once
and prints what each answers:

```
crow test packages/models/regression.co
crow run -i packages/models/example.co
crow run packages/models/example.co
```

In the Crow repository, `crow test --packages models` runs every file's
tests and runs `example.co` in both modes, comparing the output: a test
block runs in the interpreter, so the example is what holds the models to
the compiled program.
