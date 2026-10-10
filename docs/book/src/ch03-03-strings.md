# Strings and Interpolation

Strings use **single quotes, only**. There is no double-quote form — one
way to write a string, no style debates. Escape a quote with `\'`:

```crow
msg = 'he said \'hi\''
log[msg]
```

```text
he said 'hi'
```

## Interpolation

Any expression between **paired backticks** is evaluated into the string:

```crow
name = 'crow'
n = 4
log['hello, `name`']
log['`n` squared is `n * n`']
```

```text
hello, crow
4 squared is 16
```

Outside backticks, strings are completely inert — braces, JSON, and regex
punctuation need no escaping. For a literal backtick, write ``\` ``. A
backslash before any other character stays as it is, so a regular
expression is written as it is: `'\d+'`, `'\|'`.

## Length and slicing count characters

`len[]`, `slice[]`, and indexing all work in characters, never bytes, so
accented letters and dashes are one step each in both execution modes:

```crow
s = 'naïve — café'
log[len[s]]
log[slice[s, 6, 7]]
log[s[-1]]
log[s[2]]
```

```text
12
—
é
ï
```

## Counting and mapping characters

`occurs` counts how many times a part occurs, not overlapping (as Python's
`str.count` counts), and `translate` maps each character of one text to
the character at its place in another, all in one pass — the complement
of a strand of DNA:

```crow
dna = 'GATTACA'
log[occurs[dna, 'A']]
log[occurs['aaaa', 'aa']]
log[translate[dna, 'ACGT', 'TGCA']]
```

```text
3
2
CTAATGT
```

## No `+` on strings

`+` is for numbers. To combine strings, interpolate; to build from many
pieces, collect and `join`:

```crow
parts = split['a,b,c', ',']
log[parts]
log[join[parts, '-']]
log[upper['crow']]
```

```text
['a', 'b', 'c']
a-b-c
CROW
```

```crow,error
s = 'a' + 'b'    --- error: cannot use '+' with strings
```

The error message itself tells you the two idioms above. For value
formatting (decimals, widths, percents) there's `fmt[...]` with named
arguments — chapter 10 uses it heavily.
