# Data Types

The scalar types are `int`, `float`, `bool`, and `str` (text). Arithmetic
is familiar, with two things worth pinning down.

**Division between ints rounds toward zero; a float anywhere makes it float:**

```crow
log[3 / 2]
log[3 / 2.0]
```

```text
1
1.5
```

**Exponentiation is `^`** (not `**`), and `%` is remainder:

```crow
log[2 ^ 10]
log[7 % 3]
```

```text
1024
1
```

**An int is a whole number of any size.** It grows as far as its value
needs, so int arithmetic is exact, as Python's is — nothing is written to
ask for it:

```crow
log[2 ^ 100]
top = 9223372036854775807
log[top + 1]
fn fact[n]
    r = 1
    for i in range[2, n + 1]
        r = r * i
    return r
log[len[str[fact[100]]]]
```

```text
1267650600228229401496703205376
9223372036854775808
158
```

The compiled program keeps an int in 64 bits wherever it can show the int
stays within them — a loop's counter, a position, a count — so a program
pays for the size only where a number can have it. A number past 64 bits
handed where 64 bits are taken, a position or a count, is refused there:
`range[2 ^ 70]` stops with `a whole number past 64 bits where range[] takes
an int of 64 bits: 1180591620717411303424`.

**Exact numbers.** A float is a nearby binary number — `0.1 + 0.2` is
`0.30000000000000004` — which is right for measurement and wrong for money.
A `decimal` is an exact base-ten number that keeps its places, and a
`fraction` an exact ratio in lowest terms:

```crow
a = decimal['0.1']
log[a + decimal['0.2']]
price = decimal['12.30']
log[price * 3]
log[decimal[1] / decimal[3]]
log[fraction[1, 3] + fraction[1, 6]]
```

```text
0.3
36.90
0.3333333333333333333333333333
1/2
```

`+`, `-` and `*` of decimals are exact; a quotient has 28 significant
digits, rounded half away from zero, unless a decimal was given more with
`decimal[x, digits = n]` — which makes it the language's big float:

```crow
log[sqrt[decimal[2, digits = 30]]]
log[round[decimal['2.345'], 2]]
```

```text
1.41421356237309504880168872421
2.35
```

An int meeting a decimal is the decimal it is, and a literal with a point is
the number it writes (`price * 1.1` is exactly 13.530). A float computed
elsewhere meeting a decimal is refused before the program runs, with both
conversions named — `decimal[rate]` or `float[price]` — since the exact
answer would be a guess.

Convert between types with `int[...]`, `float[...]`, `str[...]`. Converting
a string can fail — which returns an error *value*, not an exception (you
met this in the guessing game; chapter 7 goes deep):

```crow
log[int[3.9]]
log[float['2.5']]
log[str[42]]
```

```text
3
2.5
42
```

Collections — arrays, tensors, maps, structs — get their own chapter (5).
