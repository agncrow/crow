# Maps

```crow
m = map[b = 2, a = 1]
log[keys[m]]
log[values[m]]
log['a' in m]
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

```crow
words = ['the', 'cat', 'sat']
at = map[words, range[len[words]]]
log[at['cat']]
log['dog' in at]
```

```text
1
false
```
