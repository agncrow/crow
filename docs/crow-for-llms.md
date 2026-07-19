# Crow for LLMs

A compact, accurate reference for generating **Crow** code. Crow is an
indentation-based language that compiles to Rust (and also runs in an
interpreter). If you know Python, you already know 80% of Crow's shape — but
Crow deliberately rejects several Python/JS habits. This page is built to drop
into a system prompt: read the **Cheat Sheet**, then obey the **Reflexes to
Unlearn** table.

---

## The one rule that catches everyone

**Calls use square brackets, not parentheses.** `log[x]`, `sqrt[9]`,
`double[5]`. Parentheses are *only* for grouping arithmetic: `(a + b) * c`.

---

## Cheat Sheet

```
--- Comments run from --- to the end of the line. No closing delimiter.

--- Variables: no keyword, no type annotation.
name = 'alice'
count = 0
pi_ish = 3.14
ready = true                --- lowercase true/false

--- Strings: single quotes only. Interpolate with `backticks`.
greeting = 'hello `name`'
line = '`first` `last` is `age` years old'
--- Build from many pieces: collect + join (no + on strings).
parts = for x in items
    '`x`'
sentence = join[parts, ', ']

--- Functions: fn name[params], body indented, last expression returns.
fn double[x]
    return x * 2

fn add[a, b]
    a + b                   --- implicit return of the last expression

--- Named arguments (must come after positional; must have a default).
fn train[data, lr = 0.01, epochs = 100]
    ...
train[my_data]
train[my_data, lr = 0.001]

--- Calling.
y = double[5]
z = add[3, 4]

--- Collections.
nums = [1, 2, 3]            --- array (homogeneous)
first = nums[0]
last = nums[-1]
n = len[nums]

--- Maps are dictionaries: homogeneous VALUES, built with map[k = v].
scores = map[alice = 10, bob = 7]
a = scores.alice           --- or scores['alice']
scores2 = set[scores, 'carol', 9]   --- returns a new map

--- Structs are records: mixed named fields.
struct Point [x, y]
p = Point[3, 4]
log[p.x]

--- Control flow.
if score gte 10
    log['high']
else
    log['low']

--- No elif: nest else + if.
if n eq 1
    log['one']
else
    if n eq 2
        log['two']
    else
        log['other']

--- Loops.
for x in nums
    log[x]

i = 0
while i lt 5
    log[i]
    i = i + 1               --- no += ; write it out

--- for as an EXPRESSION collects each iteration's last value (runs in parallel).
squares = for x in nums
    x * x
--- with a filter (trailing if):
evens = for x in range[10]
    if x % 2 eq 0
        x

--- Early-exit searches take a NAMED predicate function (no lambdas):
fn is_big[x]
    return x gt 100
found = find[nums, is_big]   --- first matching element (error-value if none)
any_big = any[nums, is_big]  --- true/false, stops early
all_big = all[nums, is_big]

--- Operators are WORDS.
--- eq ne gt lt gte lte   and or not
if a gt b and not done
    ...

--- Exponent is ^. Element-wise tensor ops are dotted: .* ./ .^
sq = x ^ 2

--- Modules.
use math
r = sqrt[2]
s = math.sum[nums]           --- module.fn also works

--- Errors are VALUES, not exceptions. Every value has .ok / .reason / .kind.
content = read['file.txt']   --- auto-unwraps when used
if not content.ok
    log['failed: `content.reason`']
else
    log[content]

--- Formatting via named args (no format mini-language).
log['total: `fmt[amount, decimals = 2, commas = true]`']
```

---

## Reflexes to Unlearn

Every left column produces a **compile error that names the right column** — but
write the right column from the start.

| You might type (Python/JS) | Write in Crow instead |
|---|---|
| `f(x)`, `sqrt(9)` | `f[x]`, `sqrt[9]` — square brackets for calls |
| `"text"` | `'text'` — single quotes only (`\'` to nest) |
| `f"hello {name}"` | `` 'hello `name`' `` — backticks, no `f` prefix |
| `"a" + "b"` | `` '`a``b`' `` or `join[parts, sep]` — no `+` on strings |
| `x += 1` | `x = x + 1` — no augmented assignment |
| `a == b`, `a != b` | `a eq b`, `a ne b` |
| `a < b`, `a >= b` | `a lt b`, `a gte b` (also `gt`, `lte`) |
| `a and b` / `a && b`, `!a` | `a and b`, `not a` |
| `def f(x):` | `fn f[x]` then indented body (no `:`) |
| `lambda x: x*2` | a named `fn` — no lambdas |
| `class Foo:` | `struct Foo [a, b]` + free functions |
| `match/case` | `if / else` — no pattern matching or sum types |
| `elif` | `else` then an indented `if` |
| `import math` | `use math` |
| `from pkg import x` | `use pkg` (then `pkg.x`) |
| `print(x)` | `log[x]` |
| `None` / `null` | no null — use an error-value, check `.ok` |
| `True` / `False` | `true` / `false` (lowercase) |
| `try/except`, `raise` | errors are values: check `.ok`, or let it propagate |
| `a, b = f()` | `r = f[]` then `a = r[0]`, `b = r[1]` — no destructuring |
| `x = a if c else b` | `if c` / `else` statements with assignment — no ternary |
| `[k: v]`, `{k: v}` | `map[k = v]` |
| `map(f, xs)`, `filter(...)` | `for`-as-expression, `find`/`any`/`all` |
| `x = risky() or 0` | check `.ok` explicitly — no inline error fallback |

---

## Things that trip up generators specifically

- **The last expression in a function body is its return value.** `fn add[a, b]`
  then a line `a + b` returns the sum. `return` is optional but allowed.
- **`for` is an expression.** `doubled = for x in xs / x * 2` maps; a trailing
  `if` inside it filters. This replaces `map`/`filter`/list comprehensions —
  Crow has none of those. It also runs iterations **in parallel**, so don't
  rely on side-effect ordering; collect values instead.
- **No `break`/`continue`-driven search.** To find or test elements, use
  `find[xs, pred]`, `any[xs, pred]`, `all[xs, pred]` with a **named** predicate
  function. `break`/`continue` exist only for plain imperative `while`/`for`.
- **Maps hold one value type; structs hold mixed named fields.** Reach for
  `struct` when fields differ in type (`struct Addr [street, zip]`), `map` for a
  uniform lookup table (`map[alice = 1, bob = 2]`). Maps are immutable-by-rebuild:
  `m2 = set[m, key, value]`.
- **Errors have no truth value.** You cannot compare or branch on an error. If a
  condition might be an error-value, check `.ok` first. A branch taken on an
  error propagates it (inside a function) or halts (at top level) — it never
  silently becomes `false`.
- **Indentation defines blocks. There is no `:` anywhere in the language.**
- **Identifiers are ASCII**; comments and string contents may be any Unicode.

---

## A complete, idiomatic program

```
--- Word frequency from a file, top 3 by count.

use math

text = read['speech.txt']
words = split[lower[text], ' ']

counts = map[]
for w in words
    if len[w] gt 0
        seen = counts[w]
        if seen.ok
            counts = set[counts, w, seen + 1]
        else
            counts = set[counts, w, 1]

fn report[word, n]
    log['`word`: `n`']

--- (illustrative: iterate the map, format each line)
log['counted `len[words]` words']
```

## A reactive web page (LiveView)

```
use web, ui

count = 0

fn inc[]
    count = count + 1

fn render[]
    return div[
        h1['Count: `count`'],
        button['+', click = inc]
    ]

serve[3000]
page['/', render]
```

The transpiler manages the app instance — no `app` variable. `count` is a
reactive variable (module-level under `use ui`); event handlers are passed by
**name** (`click = inc`), never as lambdas.

---

## Where to look for more

- `docs/language-spec.md` — the full specification, including every builtin and
  a long series of "Design Note: No X" sections explaining each deliberate
  omission (they answer "why can't I just…?").
- `examples/` — runnable programs (`crow run file.crow`).
