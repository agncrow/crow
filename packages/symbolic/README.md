# symbolic

Symbolic regression written in Crow, as PySR searches
(SymbolicRegression.jl): populations of equations evolve by tournament
selection with age regularisation, the mutations and crossover of PySR,
simulated annealing on the cost, constant optimisation and simplification
each iteration, and a hall of fame of the best equation at each
complexity. The Pareto front of what was found is a frame of complexity,
loss, score and equation, as PySR's `equations_`. The search is serial and
seeded: the same answer every run, in both `crow run -i` and `crow run`.

```
deps = map[symbolic = 'github.com/agncrow/crow/packages/symbolic@main']
```

## The search

```
use symbolic.search

o = search.defaults[]
f = search.fit[x, y, o]        --- x: a row of features per sample; y: a target each
log[f.equations]               --- complexity, loss, score, equation
log[search.equation_at[f, 4]]  --- the front's equation of complexity 4
p = search.predict[f, x]       --- the best equation's prediction, by score
```

`Options[binary, unary, iterations, populations, population_size, cycles,
maxsize, parsimony, alpha, tournament_n, tournament_p,
optimize_probability, seed]`; `defaults[]` is `['+', '-', '*', '/']`,
`['sin', 'cos', 'exp']`, 20 iterations, 4 populations of 33, 60 cycles per
iteration, equations of 20 nodes at most, no parsimony, alpha 3.17, a
tournament of 15 won by its best with probability 0.982, constants
optimised with probability 0.14, seed 0. The operators: `+ - * / ^` and
`sin cos exp log sqrt abs square cube neg inv tanh`.

## The trees

`symbolic.expr` holds the expression trees: a tree is a list of nodes in
preorder — the root, then its first child's subtree, then its second's —
each a record of its operator, its constant, its feature and the size of
its subtree (`Node[op, value, feature, size]`), so a subtree is a slice
of the list. A tree is evaluated over a dataset with tensor operations,
printed as an equation, measured by node count, simplified by folding
constants, and changed by PySR's mutations — `mutate_constant`,
`mutate_operator`, `mutate_feature`, `swap_operands`, `append_random_op`,
`insert_random_op`, `delete_random_op`, `rotate`, `random_tree` — and
`crossover`, each a new tree.

## Against PySR

The parts that are deterministic agree with Python exactly: every
equation the search prints, evaluated by sympy on the same data, has the
loss Crow reported (to 1e-12), and the scores follow PySR's rule. The
search itself is stochastic, so the two find different equations from a
seed and are compared on what they recover. `compare.co` runs four known
formulas (sixty samples each, four seeds, 40 iterations of 8 populations
of 33 with 100 cycles, equations of 14 nodes at most) against PySR at the
same budget (2026-10-06, one machine):

| formula | Crow, compiled | PySR |
|---|---|---|
| `2.5 x0² − sin(x1)` | exact, 4 of 4 seeds (`((x0 * x0) * 2.5) - sin(x1)`), ~22 s a run | exact, 4 of 4, 0.3 s a run |
| Nguyen-1, `x³ + x² + x` | exact, 4 of 4, ~18 s | exact, 4 of 4, 0.2 s |
| Nguyen-5, `sin(x²) cos(x) − 1` | loss 1e-5 to 5e-5, ~20 s | exact in 3 of 4, 0.3 s |
| Nguyen-7, `ln(x+1) + ln(x²+1)` | loss 2e-5 to 7e-4, ~23 s | loss 1e-5 to 2e-5, 0.3 s |

Re-measured 2026-10-11, Crow's side only, after `and` and `or` began
deciding from the left — the tournament's `place lt n - 1 and rand[] gt p`
no longer draws once the left decides, so a seed is a different stream —
on data drawn anew (sixty samples each, Nguyen's ranges; the files of
2026-10-06 are gone) with eight seeds: `2.5 x0² − sin(x1)` exact in 5 of
8, Nguyen-1 in 7 of 8, Nguyen-5 median loss 2e-4, Nguyen-7 median 6e-4,
2–3 s a run compiled. The build before the change, on the same data: 6 of
8, 7 of 8, 6e-5 and 5e-4, the same worst runs — the same search within the
noise of eight seeds.

PySR's Julia runs its evolution some seventy times faster per step, so
its default budget (380 cycles) costs it under a second where Crow's
compiled search takes a minute and a half; the interpreter is ten times
slower again. Where the two differ in what they find, PySR's constant
optimiser (BFGS with gradients) and its adaptive parsimony are the likely
reasons: Crow's optimiser steps along central differences, and the
frequency term is not implemented. Crow's search is the place to spend a
compiled program's time, not the interpreter's.
