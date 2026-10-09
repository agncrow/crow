# Arrays

```crow
xs = [5, 3, 9, 1]
log[sort[xs]]
log[reverse[xs]]
log[slice[xs, 1, 3]]
log[push[xs, 7]]
log[pop[xs]]
log[concat[xs, [0]]]
log[9 in xs]
log[range[3]]
log[range[2, 5]]
log[range[0, 10, 3]]
```

```text
[1, 3, 5, 9]
[1, 9, 3, 5]
[3, 9]
[5, 3, 9, 1, 7]
[5, 3, 9]
[5, 3, 9, 1, 0]
true
[0, 1, 2]
[2, 3, 4]
[0, 3, 6, 9]
```

Reference: spec section 8 (Collections) and section 11 (Collections).

```crow
xs = ['b', 'c', 'a']
log['c' in xs]
log['z' not in xs]
log['ell' in 'hello']
log[concat[[[1, 2], [3], [4, 5]]]]
labels = [0, 1, 0, 1]
names = ['p', 'q', 'r', 's']
log[names[labels eq 1]]
```

```text
true
true
true
[1, 2, 3, 4, 5]
['q', 's']
```
