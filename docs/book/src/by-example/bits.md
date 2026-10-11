# Bits and character codes: `use bits`

```crow
use bits

log[bitand[12, 10]]
log[bitor[12, 10]]
log[bitxor[12, 10]]
log[bitcount[255]]
log[shift[1, 10]]
log[shift[-7, -1]]
log[shift[1, 100]]
log[code['A']]
log[char[233]]
```

```text
8
14
6
8
1024
-4
1267650600228229401496703205376
65
é
```

An int is a whole number of any size, so its bits are as many as it needs:
`shift[1, 100]` keeps every digit, and a negative number's high bits are
ones (`bitand[-1, 255]` is 255). A set of small numbers fits in one int's
bits — here the columns an n-queens search may still use, cut to the
board's width with `bitand` after each shift so the numbers stay small:

```crow
use bits

fn queens[n, mask, row, cols, d1, d2]
    if row eq n
        return 1
    count = 0
    free = bitand[bitxor[bitor[bitor[cols, d1], d2], -1], mask]
    while free ne 0
        bit = bitand[free, -free]
        free = bitxor[free, bit]
        count = count + queens[n, mask, row + 1, bitor[cols, bit], bitand[shift[bitor[d1, bit], 1], mask], shift[bitor[d2, bit], -1]]
    return count

log[queens[8, shift[1, 8] - 1, 0, 0, 0, 0]]
```

```text
92
```

A character's code and back, for a parser or a hash that works on codes:

```crow
use bits

fn fnv[s]
    h = 2166136261
    for c in split[s, '']
        if c ne ''
            h = bitxor[h, code[c]]
            h = h * 16777619 % 4294967296
    return h

log[fnv['crow']]
word = ''
for n in [99, 114, 111, 119]
    word = '`word``char[n]`'
log[word]
```

```text
2472741726
crow
```
