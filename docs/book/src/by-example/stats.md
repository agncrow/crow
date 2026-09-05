# Statistics: `use math.stats`

```crow
use math, math.stats

xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]
log[median[xs]]
log[std[xs]]
log[var[xs]]
log[percentile[xs, 50.0]]
ys = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
log[fmt[corr[xs, ys], decimals = 3]]
fit_line = linreg[ys, xs]
log[fmt[fit_line.slope, decimals = 3]]
log[fmt[cov[xs, ys], decimals = 3]]
log[zscore[[1.0, 2.0, 3.0]]]
t = ttest[xs, ys]
log[t.p lt 1.0]
a = anova[[1.0, 2.0, 3.0], [2.0, 3.0, 4.0]]
log[a.p lt 1.0]
c = chisq[[10.0, 20.0, 30.0], [20.0, 20.0, 20.0]]
log[c.chi2]
f = fit[xs, 'normal']
log[f.mu]
k = kstest[xs, 'normal']
log[k.d lt 1.0]
```

```text
4.5
2
4
4.5
0.927
0.810
4.250
[-1.224744871391589, 0, 1.224744871391589]
true
true
10
5
true
```

Reference: spec section 13 (math → Statistics).
