# Named Arguments

Trailing parameters can declare defaults, which makes them optional and
nameable at the call site:

```crow
fn greet[name, greeting = 'hello']
    return '`greeting`, `name`'

log[greet['crow']]
log[greet['crow', 'hey']]
log[greet['crow', greeting = 'welcome']]
```

```text
hello, crow
hey, crow
welcome, crow
```

The rules are exactly four:

1. Named parameters come **after** all positional ones
2. A named parameter **must have a default** — that's what makes it optional
3. A default is a literal — a number, text, a bool — or an empty `[]` or
   `map[]`, never an expression, so when and where it is computed never
   comes up
4. Callers may use position or name for those slots — both are fine

You'll use this constantly with builtins: `fmt[x, decimals = 2]`,
`sort[df, 'age', desc = true]`, `sum[m, axis = 0]`. It's the same
mechanism everywhere — there is no separate "keyword argument" system for
builtins versus your functions.
