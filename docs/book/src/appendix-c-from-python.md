# Appendix C: Coming from Python

The one-page retraining list. Every wrong reflex below produces a compile
error that names the fix — this table just saves you the round-trips.
(The same material, formatted for pasting into an AI assistant, lives in
`docs/crow-for-llms.md`.)

| Python | Crow |
|---|---|
| `f(x)` | `f[x]` — brackets call |
| `"text"` / `f"hi {name}"` | `'text'` / `` 'hi `name`' `` |
| `"a" + "b"` | interpolate, or `join[parts, sep]` |
| `x += 1` | `x = x + 1` |
| `==  !=  <  >=` | `eq  ne  lt  gte` |
| `and or not` | same words, same meaning |
| `def f(x):` | `fn f[x]` + indentation (no `:`) |
| `lambda` | a named `fn`, passed by name |
| `class` | `struct` + free functions |
| `elif` | `else` + nested `if` |
| `match/case` | `if`/`else` |
| `import numpy as np` | `use math` (bare) or `use math as m` |
| `print(x)` | `log[x]` |
| `None` | doesn't exist — error values + `.ok` |
| `True/False` | `true/false` |
| `try/except/raise` | errors are values (chapter 7) |
| `a, b = f()` | `r = f[]; a = r[0]; b = r[1]` |
| `[x*2 for x in xs]` | `for`-as-expression |
| `dict` | `map[k = v]` (one value type) or a `struct` |
| global variables in functions | invisible — pass parameters (chapter 4) |
| threads / asyncio | delete them — `for` is already parallel |
