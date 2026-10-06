# optimize

Optimisers written in Crow, each the algorithm scipy.optimize runs,
stopping where scipy stops and answering what scipy answers: the point,
the value there, the iterations and the calls. A function of one number
is bracketed and minimised (`scalar`) or solved for a root (`roots`); a
function of a vector is minimised by Nelder–Mead or by BFGS with the
More–Thuente line search (`multi`). Each answers the same in
`crow run -i` and `crow run`, and the same as scipy to the last digit.

```
deps = map[optimize = 'github.com/agncrow/crow/packages/optimize@main']
```

`crow install` checks the package out into `crow_modules/optimize`, and
`crow.lock` pins the commit it took. A program imports the parts it uses:

```
use optimize.scalar, optimize.roots, optimize.multi
```

## The optimisers

`f` is a function handed as a value: of one number for `scalar` and
`roots`, of a tensor answering a number for `multi`. `df` is its
derivative or gradient — `grad[f]` gives it where `f` is written over
what `grad` differentiates: arithmetic, `^`, the elementary functions
(`sqrt`, `exp`, `ln`, `sin`, `cos`, `tan`, `tanh`, `erf` and their kin)
and the tensor operations the spec lists.

| File | Function | Answer |
|------|----------|--------|
| `scalar` | `bracket[f, a, b]` | a `Bracket[a, b, c, fa, fb, fc, calls]` around a minimum, found from two points as scipy's `bracket` finds it; an error of kind `'optimize'` where none is |
| | `brent[f, a, b]`, `brent_in[f, bracket]` | a `Found[x, value, iterations, calls]`: Brent's method — parabolic steps where they help, golden-section steps otherwise — from two points (bracketed first) or within a bracket (scipy's `minimize_scalar`, method `'brent'`) |
| | `golden[f, a, b]`, `golden_in[f, bracket]` | the golden section (method `'golden'`) |
| | `bounded[f, lo, hi]` | the minimum between two bounds (method `'bounded'`); its iterations and calls are one count, as scipy reports them |
| `roots` | `bisect[f, a, b]`, `brentq[f, a, b]` | a `Root[x, iterations, calls]` from an interval whose ends differ in sign — halved, or by Brent's method (scipy's `bisect`, `brentq`); an error of kind `'optimize'` where the signs agree |
| | `newton[f, df, x0]`, `secant[f, x0]` | Newton's method by the derivative, or the secant through two points (scipy's `newton` with and without `fprime`) |
| `multi` | `nelder_mead[f, x0, xatol = 0.0001, fatol = 0.0001]` | a `Best[x, value, iterations, calls, gradient_calls]`: scipy's Nelder–Mead — its first simplex, its reflections, expansions, contractions and shrinks, its stopping rule |
| | `bfgs[f, df, x0, gtol = 0.00001]` | scipy's BFGS: the running inverse Hessian, each step's length by scipy's first Wolfe line search (MINPACK's dcsrch, ported step for step); where that search fails, scipy tries a second search and this answers what it has |

The tolerances are scipy's defaults, and named arguments change them.

```
use math
use optimize.roots, optimize.multi

fn cubic[x]
    return x ^ 3 - 2.0 * x - 5.0

fn sphere[x]
    return sum[x .^ 2]

r = roots.newton[cubic, grad[cubic], 2.5]      --- Root[2.0945514815423265, 5, 10]
b = multi.bfgs[sphere, grad[sphere], tensor[[3.0, -4.0, 5.0]]]
log[b.iterations]                              --- 3
```

## Against scipy

`compare.co` takes a folder holding scipy's answers (`reference.csv`,
written by a script kept with the PySR one, `opt_bench.py`: scipy 1.18
in a Python environment) and prints, for each case and method, how far
this package's point and value are from scipy's and whether its
iterations and calls are scipy's (2026-10-06):

| Method | Cases | Point | Value | Iterations and calls |
|---|---|---|---|---|
| `bracket`, `brent`, `bounded` | a wavy parabola from three starts, `exp(x) − 3x` | 0 | 0 | the same |
| `golden` | the same | 0, and 5e-9 on `exp(x) − 3x` | 0 | the same |
| `bisect`, `brentq`, `newton`, `secant` | `x³ − 2x − 5`, `cos x − x`, both roots of `exp(x) − 3x` | 0 | 0 | the same |
| `nelder_mead` | Rosenbrock in 2 and 4, a sphere in 3, Beale, Booth, Himmelblau | 0 to 8e-14 | 0 | the same |
| `bfgs` | the same, gradients by `grad` | 0 to 7e-15 | 0 | the same |

Newton's and BFGS's derivatives come from `grad` here where scipy was
given them written out — `cos x − x` and `exp(x) − 3x` included — and
the points still agree, which also holds Crow's automatic
differentiation to scipy's formulas. The one visible
difference, the golden section's point on `exp(x) − 3x`, is one
golden-section step: the two last points' values differ in the last bit
of `exp` between Rust and Python, and each side keeps the one it finds
lower. `crow run -i packages/optimize/compare.co <folder>` and `crow run`
print the same lines.

## Tests

Each file has its own `test` blocks holding small cases to scipy's
numbers, and `example.co` runs each optimiser once:

```
crow test packages/optimize/multi.co
crow run -i packages/optimize/example.co
crow run packages/optimize/example.co
```

In the Crow repository, `crow test --packages optimize` runs every file's
tests and the example in both modes, comparing the output.
