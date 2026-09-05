# Maps

```crow
m = map[b = 2, a = 1]
log[keys[m]]
log[values[m]]
log[has[m, 'a']]
m2 = set[m, 'c', 3]
log[len[m2]]
log[del[m2, 'b']]
log[m2['c']]
```

```text
['a', 'b']
[1, 2]
true
3
[a = 1, c = 3]
3
```

Reference: spec section 8 (Maps) and section 11 (Map Operations).
