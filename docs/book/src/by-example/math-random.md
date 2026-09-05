# Math: random numbers and distributions

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
