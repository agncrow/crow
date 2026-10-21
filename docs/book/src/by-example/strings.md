# Strings

```crow
s = '  Hello, Crow  '
log[trim[s]]
log[upper['crow']]
log[lower['CROW']]
log[replace['a-b-c', '-', '+']]
log[starts['crowfile', 'crow']]
log[ends['main.co', '.co']]
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

A text's characters are `split[s, '']`, counted as `len` counts them;
`occurs` counts a part without overlap, and `translate` maps each
character of one set to the character at its place in another, in one
pass:

```crow
log[split['héllo', '']]
log[len['héllo']]
log[occurs['banana', 'an']]
log[occurs['aaaa', 'aa']]
log[translate['GATTACA', 'ACGT', 'TGCA']]
```

```text
['h', 'é', 'l', 'l', 'o']
5
2
2
CTAATGT
```

Reference: spec section 11 (Strings) and section 1 (String Interpolation).
