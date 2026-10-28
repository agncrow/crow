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

Reference: spec section 13 (bits).

Arithmetic wrapped to 64 bits, for a hash whose multiply wraps, a linear
congruence or an offset from a base — what ints that grow have no other
way to say: the exact product of two ints past 2^32 is a growing int.
`wrapmul`, `wrapadd` and `wrapsub` answer the low 64 bits of the exact
answer, as the int of those bits.

```crow
use bits

--- FNV-1a of 64 bits: a hash whose multiply wraps, written as it is
fn fnv64[s]
    h = -3750763034362895579
    for c in split[s, '']
        h = bitxor[h, code[c]]
        h = wrapmul[h, 1099511628211]
    return h

log[fnv64['crow']]
log[wrapadd[9223372036854775807, 1]]
log[wrapsub[-9223372036854775808, 1]]
log[wrapmul[4294967296, 4294967296]]
log[wrap[18446744073709551615]]
log[wrap[2 ^ 64 + 5]]
```

```text
1330361311932775454
-9223372036854775808
9223372036854775807
0
-1
5
```
