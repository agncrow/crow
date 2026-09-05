# Strings

```crow
s = '  Hello, Crow  '
log[trim[s]]
log[upper['crow']]
log[lower['CROW']]
log[replace['a-b-c', '-', '+']]
log[starts['crowfile', 'crow']]
log[ends['main.crow', '.crow']]
log[repeat['ab', 3]]
log[split['x,y,z', ',']]
log[join[['x', 'y', 'z'], '/']]
log[reverse['abc']]
w = 'hello'
log[len[w]]
log[w[0]]
log[w[-1]]
```

```text
Hello, Crow
CROW
crow
a+b+c
true
true
ababab
['x', 'y', 'z']
x/y/z
cba
5
h
o
```

Reference: spec section 11 (Strings) and section 1 (String Interpolation).
