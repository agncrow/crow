# Numbers and conversion

```crow
log[abs[-3]]
log[sqrt[16.0]]
log[floor[2.7]]
log[ceil[2.1]]
log[round[2.5]]
log[min[3, 7]]
log[max[3, 7]]
log[type[42]]
log[type['x']]
log[type[[1, 2]]]
log[int['12'] + 1]
log[float['2.5'] * 2.0]
log[str[7]]
```

```text
3
4
2
3
3
3
7
int
str
array
13
5
7
```

An int is a whole number of any size: past 64 bits it keeps every digit.
A float is a nearby binary number; `decimal` is an exact base-ten number
that keeps its places, `fraction` an exact ratio in lowest terms, and
`float[…]` of either is the nearest float:

```crow
log[2 ^ 64]
log[int['123456789012345678901234567890'] + 1]
log[0.1 + 0.2]
log[decimal['0.1'] + decimal['0.2']]
log[decimal['12.30'] * 3]
log[fraction[1, 3] + fraction[1, 6]]
log[float[fraction[1, 4]]]
```

```text
18446744073709551616
123456789012345678901234567891
0.30000000000000004
0.3
36.90
1/2
0.25
```

Reference: spec section 11 (Math, Conversion) and section 1 (Numbers That
Keep Every Digit).
