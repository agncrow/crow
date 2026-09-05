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
punctuation need no escaping. For a literal backtick, write ``\` ``.

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
