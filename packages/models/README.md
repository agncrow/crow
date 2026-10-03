# models

Small models written in Crow, on `math` and `math.nn`: least squares,
ridge, the lasso and the elastic net, logistic and softmax regression,
support vector machines (linear and with a radial kernel) and support
vector regression, k nearest neighbours, Gaussian naive Bayes, k-means,
DBSCAN, agglomerative clustering, a Gaussian mixture, principal
components, a decision tree, a random forest, gradient boosting, an
isolation forest and a one-hidden-layer network — four of them trained
by `grad` with no derivative written out — with the metrics that score
them, cross-validation and a parameter search, and the split, scaling,
polynomial features and encodings that prepare their data. Each one fits
the same way every run, and answers the same in `crow run -i` and
`crow run`.

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
| `mlp` | `fit[x, y, hidden, steps, rate, seed]` → `Net[w1, b1, w2, b2]`, by `grad` | `probability[net, x]`, `classify[net, x]` |
| `softmax` | `fit[x, y, classes, steps, rate]` → `Softmax[w, b, classes]`, `y` of int classes, by `grad` | `scores[m, x]` — a row of scores per sample; `classify[m, x]` — the greatest's class per row |
| `svm` | `fit[x, y, steps, rate]` → `Svm[w, b]`, `y` of −1s and 1s, by `grad` | `classify[m, x]` — −1 or 1 per row |
| `ridge` | `fit[x, y, penalty]` → `Ridge[coef, intercept]` | `predict[m, x]` — a number per row |
| `knn` | `fit[x, y, k]` → `Knn[x, y, k]` | `classify[m, r]` — the class most of the k nearest have; `predict[m, r]` — their mean target |
| `bayes` | `fit[x, y]` → `Bayes[classes, means, variances, priors]`, `y` of int classes | `classify[m, r]`, `probability[m, r]` — a probability per class |
| `pca` | `fit[x, k]` → `Pca[means, components, explained]` | `transform[p, x]` — each row's coordinates along the components |
| `forest` | `fit[x, y, trees, depth, seed]` → `Forest[trees]`, `y` of int classes | `classify[f, r]` — the class most trees give |
| `metrics` | — | `accuracy`, `precision`, `recall`, `f1` (the class 1 against 0), `rmse`, `mae`, `r2`, each `[truth, guess]` |
| `prepare` | `divide[x, y, share, seed]` → `Split[x_train, y_train, x_test, y_test]`; `fit_scale[x]` → `Scale[means, spreads]`; `labels[names]` → `Labels[names]` | `scale[s, x]` — the rows standardised; `polynomial[x, degree]` — the rows with their products of features; `onehot[y]` — int classes as columns of ones and zeros; `code[l, name]`, `codes[l, names]` — names as ints |
| `boost` | `fit[x, y, trees, depth, rate]` → `Boost[base, rate, trees]`, gradient-boosted regression trees | `predict[m, x]` — a number per row; `classify[m, x]` — a 0 or 1 per row, for targets of 0 and 1 |
| `lasso` | `fit[x, y, penalty, ratio, steps]` → `Lasso[coef, intercept]`, `ratio` 1 the lasso, 0 ridge, between them the elastic net | `predict[m, x]` — a number per row |
| `kernel_svm` | `fit[x, y, width, c, steps, rate]` → `KernelSvm[x, y, alpha, width]`, a radial kernel, `y` of −1s and 1s | `classify[m, x]` — −1 or 1 per row; `score[m, r]` — a sample's signed distance |
| `svr` | `fit[x, y, width, epsilon, penalty, steps, rate]` → `Svr[x, beta, width]`, by `grad` | `predict[m, x]` — a number per row |
| `dbscan` | `fit[x, eps, min_points]` → `Dbscan[labels, clusters]`, −1 for noise | — |
| `agglomerative` | `fit[x, k, linkage]` → `Merged[labels]`, linkage `'single'`, `'complete'` or `'average'` | — |
| `mixture` | `fit[x, k, rounds]` → `Mixture[weights, means, variances]`, by expectation–maximisation | `probability[m, r]` — a probability per curve; `classify[m, r]` — the likeliest curve |
| `isolation` | `fit[x, trees, sample, seed]` → `Isolation[trees, sample]` | `score[f, r]` — an anomaly score, 0 to 1; `anomalies[f, x, threshold]` — the rows scoring at least it |
| `crossval` | `crossval[x, y, folds, seed, fit, score]` → `Folds[scores, mean]`; `search[x, y, folds, seed, candidates, fit, score]` → `Search[best, score, scores]` | `fit` a function of (x, y) — or (x, y, candidate) — answering a model, `score` one of (model, x, y) answering a number (`fit_ridge`, `score_ridge` show the shape) |

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
- `ridge` solves (XᵀX + penalty·I) coef = Xᵀy on the centred inputs, the
  intercept exact and unpenalised; `penalty = 0` is least squares.
- `knn` keeps the data and does the work when asked: Euclidean distance,
  the smaller class on a tie.
- `bayes` fits one normal per feature per class, a variance floored so a
  constant feature still has a density, and classes by log prior plus log
  densities.
- `pca` takes the first `k` rows of Vᵀ from the SVD of the centred matrix;
  `explained` is each component's share of the variance.
- `forest` grows each tree on a bootstrap sample drawn from `seed`.
- `prepare.divide` shuffles the rows from `seed` and holds `share` of them
  out; `fit_scale`/`scale` standardise by the training rows' means and
  spreads (a spread of zero read as one).

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
