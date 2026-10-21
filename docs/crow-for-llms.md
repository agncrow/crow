# Crow for LLMs

A compact, accurate reference for generating **Crow** code. Crow is an
indentation-based language that compiles to Rust (and also runs in an
interpreter). If you know Python, you already know 80% of Crow's shape — but
Crow deliberately rejects several Python/JS habits. This page is built to drop
into a system prompt: read the **Cheat Sheet**, then obey the **Reflexes to
Unlearn** table. The module sections after them cover web pages, frames,
databases and language models.

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
letters = split[word, '']   --- a text's characters; len, slice and s[i] count characters

--- Numbers: an int grows past 64 bits (2 ^ 100 is exact); 7 / 2 is 3, 7 / 2.0 is 3.5.
--- Exact base ten: decimal['0.1'] + decimal['0.2'] eq decimal['0.3']; exact ratios: fraction[1, 3].
--- A decimal meets ints and written numbers; a computed float is refused (decimal[f] or float[d]).

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
nums = push[nums, 4]        --- builtins answer a new value: assign it back
if 2 in nums                --- x in xs, x not in xs: an item, a part of text, a map's key
    log['found']

--- Maps are dictionaries: homogeneous VALUES, built with map[k = v].
scores = map[alice = 10, bob = 7]
a = scores.alice            --- or scores['alice']; a key the map lacks STOPS the program
if 'carol' not in scores
    scores = set[scores, 'carol', 9]   --- returns a new map
who = keys[scores]          --- also values[scores], del[scores, 'bob']

--- Structs are records: mixed named fields.
struct Point [x, y]
p = Point[3, 4]
log[p.x]

--- Control flow.
if score gte 10
    log['high']
else
    log['low']

--- No elif keyword: write else if.
if n eq 1
    log['one']
else if n eq 2
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
--- break and continue exist only inside while.

--- for as an EXPRESSION collects each iteration's last value
--- (in parallel when the body only computes).
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

--- Tests are part of the function. No framework, no imports, no test files.
--- fn fee[n]
---     return max[1.0, n * 0.001]
--- test[x]                     (attached: examples + laws for the fn above)
---     fee[100.0] eq 1.0       (bare boolean = a truth, checked once)
---     if x gte 0.0
---         fee[x] gte 1.0      (params = GENERATED inputs; law over many values)
--- test scenario_name[]        (named = standalone; assert[] for steps)
---     assert[fee[2000.0] eq 2.0]
--- Never write test_ prefixed fns, test directories, or verification helper fns.

--- Moments are a kind of their own: an instant, and the offset it was written in.
t = now[]                       --- UTC; in a live app, the moment the event began
bell = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']
d = moment['2026-07-14 09:30', tz = 'Europe/London']   --- ISO text; an offset in it wins
h = bell.hour                   --- also .year .month .day .minute .second .weekday (1 = Monday)
utc = moment[bell, tz = 'UTC']  --- the same instant in another zone
day = fmt[bell, style = 'date'] --- 'iso' 'date' 'time' 'datetime'; tz = ...
soon = bell + 90.0              --- a moment moved by seconds
secs = t - bell                 --- the seconds between two moments, a float
--- No duration type: seconds are numbers. float[t] gives epoch seconds; moment[secs] a moment.
--- Zones are always explicit; the default is UTC. A bad date or zone is an error value
--- of kind 'time'.
--- Data read from outside remembers when it arrived:
--- px = query[conn, '...']
--- if now[] - captured[px] gt 10.0  → refuse (staleness is plain arithmetic)
--- captured[x] on a computed value is an error value (kind 'no_capture').

--- Files and the outside world: each answers an error value when it fails.
notes = read['notes.txt']       --- a file, a URL or s3://bucket/key
write['out.txt', notes]         --- on a line of its own, a failure halts the program
cfg = json[read['config.json']] --- parse JSON; encode[value] writes a value as JSON
key = env['API_KEY', '']        --- also args[], and shell['ls'] with .status and .stdout
r = request['https://api.example.com/items', headers = map[authorization = 'Bearer `key`']]
--- r.status and r.body; use io gives csv[path] (a list of maps) and lines[path].

--- Operators are WORDS.
--- eq ne gt lt gte lte   and or not   in  not in
if a gt b and not done
    ...
--- and/or decide from the left: i lt len[xs] and xs[i] gt 0 reads xs[i] only in range.
--- A condition is a bool: write count gt 0, len[xs] gt 0, name ne '' (never if count).

--- Exponent is ^ (-x ^ 2 is -(x ^ 2)); % takes ints. Between two tensors (numeric
--- arrays) * is matrix multiply; .* ./ .^ work element by element.
sq = x ^ 2

--- Modules.
use math
total = sum[nums]           --- math's names are bare after use math
also = math.sum[nums]       --- module.fn also works

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

Most of the left column is refused before the program runs, usually with a
message that names the right column — but write the right column from the
start.

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
| `x in xs`, `if k in d`, `x not in xs` | the same words: an item, a part of text, a map's key |
| `def f(x):` | `fn f[x]` then indented body (no `:`) |
| `lambda x: x*2` | a named `fn` — no lambdas |
| `class Foo:` | `struct Foo [a, b]` + free functions |
| `match/case` | `if / else` — no pattern matching or sum types |
| `elif` | `else if` |
| `import math` | `use math` |
| `from pkg import x` | `use pkg` (then `pkg.x`), or `use pkg as p` |
| `print(x)` | `log[x]` |
| `None` / `null` | no null — use an error-value, check `.ok` |
| `True` / `False` | `true` / `false` (lowercase) |
| `try/except`, `raise` | errors are values: check `.ok`, or let it propagate |
| `a, b = f()` | `r = f[]` then `a = r[0]`, `b = r[1]` — no destructuring |
| `x = a if c else b` | `if c` / `else` statements with assignment — no ternary |
| `[k: v]`, `{k: v}` | `map[k = v]` |
| `map(f, xs)`, `filter(...)` | `for`-as-expression, `find`/`any`/`all` |
| `x = risky() or 0` | check `.ok` explicitly — no inline error fallback |
| `xs.append(v)` | `xs = push[xs, v]` — a builtin answers a new value |
| `list(s)` (its characters) | `split[s, '']` |
| `break` / `continue` in a `for` | a `while` loop, or `find`/`any`/`all` |
| `datetime.now()`, `datetime(2026, 7, 14)` | `now[]`, `moment[2026, 7, 14]` — a moment, not a number |
| `strptime`, `strftime`, `timedelta` | `moment[text, tz = zone]`, `fmt[t, style = 'date']`, seconds: `t + 60`, `t2 - t1` |
| `Decimal('0.1')`, `Fraction(1, 3)` | `decimal['0.1']`, `fraction[1, 3]` |
| pandas `df['c']`, `df['c'] = v` | `df.c`, `df.c = v` |
| pandas `df[df.a > 1]` | `df[df.a gt 1]` |
| pandas `df.a * df.b` | `df.a .* df.b` — `*` of two columns is a matrix product |
| `df.groupby('k').agg(...)` | `group[df, 'k', ['v', 'sum']]` — a column `v_sum` per aggregate |
| `cursor.execute(sql, args)` | `query[conn, sql, args]` — every statement, a frame back |

---

## Things that trip up generators specifically

- **The last expression in a function body is its return value.** `fn add[a, b]`
  then a line `a + b` returns the sum. `return` is optional but allowed. A
  function that returns a value returns one on every path: an `if` that
  returns, with no `else` and nothing after it, is refused.
- **`for` is an expression.** `doubled = for x in xs / x * 2` maps; a trailing
  `if` inside it filters. This replaces `map`/`filter`/list comprehensions —
  Crow has none of those. Its iterations run **in parallel** when the body
  only computes; a body that logs, writes an outer variable or reaches any
  other effect runs in order. Collect values in the loop, print after.
- **No `break`/`continue` in a `for`.** To find or test elements, use
  `find[xs, pred]`, `any[xs, pred]`, `all[xs, pred]` with a **named** predicate
  function. `break`/`continue` exist only inside a `while`, the loop for
  stopping early or skipping in order.
- **Functions see no top-level variables.** A function sees its parameters,
  its locals and other functions; pass in what it needs. (The views and
  handlers of a `use ui` page are the exception: they read and write the
  page's facts.)
- **Maps hold one value type; structs hold mixed named fields.** Reach for
  `struct` when fields differ in type (`struct Addr [street, zip]`), `map` for a
  uniform lookup table (`map[alice = 1, bob = 2]`). Maps are immutable-by-rebuild:
  `m2 = set[m, key, value]`. Reading a key the map lacks stops the program:
  ask `key in m` first.
- **Errors have no truth value.** You cannot compare or branch on an error. If a
  condition might be an error-value, check `.ok` first. A branch taken on an
  error propagates it (inside a function) or halts (at top level) — it never
  silently becomes `false`. A call that can fail, on a line of its own
  (`write[path, text]`), does the same when it fails; assign it
  (`kept = rm[tmp]`) to go on.
- **Text is not a number.** `'1' eq 1` is refused; convert first with
  `int[s]`, `float[s]` or `str[n]`.
- **Indentation defines blocks. There is no `:` anywhere in the language.**
- **Identifiers are ASCII**; comments and string contents may be any Unicode.

---

## A complete, idiomatic program

```
--- Word frequency: the three commonest words in speech.txt, with their counts.

use frame

speech = read['speech.txt']
if not speech.ok
    halt[speech.reason]

--- lowercased, each run of anything but letters made one space, then split
words = split[trim[sub[lower[speech], '[^a-z]+', ' ']], ' ']

counts = map[]
for w in words
    if w in counts
        counts = set[counts, w, counts[w] + 1]
    else if w ne ''
        counts = set[counts, w, 1]

top = frame.new[word = keys[counts], n = values[counts]]
    then sort['n', desc = true]
    then head[3]
for i in range[len[top]]
    log['`top.word[i]`: `top.n[i]`']
```

With `speech.txt` holding the two lines `The quick brown fox jumps over the
lazy dog. The dog sleeps; the fox runs.` and `A fox, a dog, and the end.`,
it prints:

```
the: 5
dog: 3
fox: 3
```

`w in counts` asks before `counts[w]` reads. The frame does the ranking, and
its sort is stable, so tied words stay in key order. The `then` lines feed
each step's value into the next call as its first argument.

## A reactive web page (LiveView)

```
use web, ui

count = 0

fn inc[]
    count = count + 1

view render[]
    return div[
        h1['Count: `count`'],
        button['+', click = inc]
    ]

serve[3000]
page['/', render]
```

**UI trees are built in `view` blocks, not `fn`.** Inside a view the HTML
element vocabulary (`div`, `p`, `a`, `form`, `button`…) is active with exact
HTML names; outside views those are ordinary names you can use freely
(`p = 0.95` is fine). Crow manages the app instance — no `app`
variable. `count` is reactive state: only views and event handlers see it;
handlers are passed by **name** (`click = inc`), never as lambdas.

More of `use web, ui` — facts, a derivation, inputs, routes, and a test that
drives the page:

```
use web, ui

qty = 10                    --- facts: the top-level names, one copy per session
side = 'buy'
speed = 1.0

derive total                --- recomputed when what it reads changes
    return qty * 2

fn buy[]                    --- a handler writes facts, as one transaction
    qty = qty + 1
    notice['bought `qty`']  --- a toast, sent when the handler commits

fn set_speed[s]
    speed = s

view render[]
    return stack[
        row[field['Qty', qty], select[['buy', 'sell'], side], gap = 'm'],   --- inputs bind facts
        p['total `total` at `speed`x', class = 'card'],
        button['Buy', click = buy, class = 'primary'],
        button['10x', click = set_speed[10.0]],   --- runs set_speed[10.0] when clicked
        gap = 'l'
    ]

fn health[req]              --- a route's handler: the request in, a response out
    return json[map[status = 'ok']]

route['/health', health]
page['/', render]
serve[3000]

test buying[]               --- no browser: a page is a tree
    a = app[render]
    change[a, 'qty', 5]     --- what typing into the bound field does
    click[a, buy]
    click[a, set_speed[10.0]]
    assert[a.qty eq 6]
    assert['total 12 at 10x' in html[a]]
    assert[elements[a, 'button.primary'][0].text eq 'Buy']   --- .tag .text .class, attributes
```

```
--- A view only shows state: a query goes in a derive, a write in a handler.
--- Inputs: field[label, fact] (type = 'date', lines = 3), select / radio / tabs[options, fact],
---   checkbox / toggle[label, fact], slider[label, fact, min = 0, max = 100, step = 1].
--- Routes: route['/orders', 'POST', add]; a path '/orders/:id' gives req.params.id; also
---   req.query.q, req.body. Responses: json[map, list or frame], text[s], html[s],
---   respond[404, s], redirect[url].
--- Harness: change[a, 'now', t] sets the session's clock; downloads[a] lists files handed out.
--- Tables: table[frame, sort = true, page = 50, click = pick, key = 'id'] (fn pick[id]).
--- Charts are elements: chart[bars, candles = ['o', 'hi', 'lo', 'c'], lines = ['vwap'],
---   bars_y = 'vol', x = 'ts', tz = 'UTC', height = 320, panes = [['rsi']],
---   levels = map[rsi = [30, 70]], marker = t]; use chart computes sma, ema, rsi, macd, vwap.
--- Layout, never CSS: stack row grid overlay scroll, with gap / pad = 'xs' 's' 'm' 'l' 'xl',
---   align, justify, width = 'fill', color = 'muted', below = 'phone'. Pages look right with
---   ZERO css. Classes: card stats controls primary up down stale. Tokens --crow-bg/surface/
---   text/muted/border/accent/up/down; override them in css/app.css (optional). NEVER write
---   css strings inside views.
--- Slow work in a handler: later[load_data] runs it without holding the session's other
---   clicks (its writes land in the facts, views re-render); running[load_data] is true while
---   it runs. A page's later[f] takes no arguments: f reads the facts it needs.
---   No async/await/threads/Promise.
```

---

## Frames: `use frame`

```
use frame, math

df = frame.load['trades.csv']           --- CSV, JSON or Parquet: a file, a URL or s3://…
log[df.columns]                         --- the column names; len[df] is the row count
log[df.sym[0]]                          --- a column is df.name, a cell df.name[i]
df.cost = df.qty .* df.px               --- write a column (* of two columns is matmul)
df.side = 'buy'                         --- one value repeated down the rows
big = df[df.qty gt 60 and df.sym eq 'AAPL']   --- a mask picks rows; also not (…), 'AA' in df.sym
top = head[sort[df, 'px', desc = true], 2]    --- also tail[df, n], select[df, ['sym', 'px']]
by_sym = group[df, 'sym', ['qty', 'sum'], ['px', 'mean']]   --- columns sym, qty_sum, px_mean
names = frame.new[sym = ['AAPL', 'MSFT'], name = ['Apple', 'Microsoft']]   --- written down
named = join[df, names, 'sym']          --- inner; 'left' 'right' 'outer' 'semi' 'anti' 'asof'
log[sum[df.qty]]                        --- a column of numbers is a tensor to math
for i in range[len[top]]
    log['`top.sym[i]` at `top.px[i]`']
saved = save[by_sym, 'by_sym.parquet']  --- the format is the extension's
--- A missing cell (an empty CSV cell, a NULL) stops arithmetic: fill[df, 'qty', 0] first.
--- Also: window[df, 'px', 'mean', n = 3, by = 'sym'], where[mask, a, b], unique[df, 'sym'].
```

## Databases: `use db`

```
use db

conn = connect['sqlite://app.db']       --- or postgres://user:pass@host/db, clickhouse://host/db
query[conn, 'CREATE TABLE IF NOT EXISTS orders (id INTEGER PRIMARY KEY, sym TEXT, qty INTEGER)']
query[conn, 'INSERT INTO orders (sym, qty) VALUES (?, ?)', ['AAPL', 100]]   --- ? binds, in order
rows = query[conn, 'SELECT sym, qty FROM orders WHERE qty > ?', [10]]   --- a frame: rows.sym
made = query[conn, 'INSERT INTO orders (sym, qty) VALUES (?, ?) RETURNING id', ['MSFT', 50]]
log[made.id[0]]                         --- a write says what it touched with RETURNING
some = query[conn, 'SELECT sym FROM orders WHERE sym IN ?', [['AAPL', 'NVDA']]]   --- IN ? a list

fn rebalance[tx]                        --- every statement through tx is one transaction
    query[tx, 'UPDATE orders SET qty = qty - 10 WHERE sym = ?', ['AAPL']]
    query[tx, 'UPDATE orders SET qty = qty + 10 WHERE sym = ?', ['MSFT']]

moved = transact[conn, rebalance]       --- committed; rolled back where rebalance fails
--- query is the one statement verb: every statement answers a frame (empty when it has no rows).
--- insert[conn, 'orders', frame] bulk-loads a frame. Migrations: crow db apply <dir> <url>.
```

## Language models and tools: `use llm`, `use mcp`

```
use llm

m = reach['anthropic://claude-sonnet-5']   --- the key comes from ANTHROPIC_API_KEY
--- (openai://localhost:11434/llama3.2 reaches Ollama, or any OpenAI-compatible server)
r = chat[m, ask['Capital of Mongolia?'], system = 'One word.', tokens = 50]
log[r.text]                     --- also r.calls, r.usage.input, r.stop: end, tool, length, other

--- A tool is a function, and the comment right above it is its description.

--- the current weather in a city
fn weather[city]
    return 'sunny in `upper[city]`'     --- upper[] reads city as text: its schema says string

--- a gate: the call (its name, args and id) in, a bool out
fn only_lisbon[call]
    return str[call.name] ne 'weather' or str[call.args.city] eq 'Lisbon'

done = act[m, ask['Is it warm in Lisbon?'], tools = [weather], allow = [only_lisbon], rounds = 8]
log[done.text]                  --- also done.stop, done.rounds, done.usage, done.talk
log[done.steps]                 --- a frame: a row per model call and tool call
--- A failed tool, a wrong argument, an unknown name and a gate's refusal are told to the model.
--- More options: budget = tokens, keep = n, compact = n, skills = 'skills/', system = '...'.
--- Go on from a run: act[m, asked[done.talk, 'And Porto?'], tools = [weather]].
```

```
use llm, mcp

m = reach['anthropic://claude-sonnet-5']
s = server['http://localhost:8000/mcp']
--- (or a process over stdio: server[command = 'npx -y @modelcontextprotocol/server-filesystem /tmp'])
log[catalog[s][0].name]                       --- the tools it offers; discover[s] says what it is
log[tool[s, 'weather', map[city = 'Lisbon']]] --- one called by hand: the text of its result
done = act[m, ask['What is in /tmp?'], servers = [s]]   --- its tools join the loop
--- A server's tool names must differ from those in tools = [...].
--- A Crow program as a server: fn served[req] answering answer[req, tools = [weather]],
--- with route['/mcp', 'POST', served] and serve[8000]; or listen[tools = [weather]] over its
--- standard streams, which a client starts as a command: server[command = 'crow run weather.co'].
```

---

## Names are never ambiguous (reserved names)

Crow rejects definitions that collide with names the language or an import
owns — the error tells you the fix. Rules of thumb:

- **Core builtin names are always taken**: don't name variables/params
  `range`, `map`, `set`, `type`, `input`, `find`, `sort`, `log`, `error`,
  `min`, `max`, `keys`, `values`… (pick `rows`, `kind`, `raw`, `total` instead).
- **A module imported bare takes its names** in that file: `use math` takes
  `sum`, `mean`, `prod`, `zeros`… and the constants `pi`, `e`, `inf`, `nan`;
  `use frame` takes `load`, `select`, `group`, `fill`…; `use web` takes
  `text`, `html`, `json`….
- **Want the name anyway? Alias the import**: `use math as m` imports
  qualified-only (Python `import x as y` semantics) — then `sum = 0` is
  legal and math is reached as `m.sum[xs]`.
- **User modules and packages are always qualified**: `use lib/scene as s`
  → `s.mesh[...]`. Only the prefix is reserved.
- **Fields are exempt**: `row.sum`, `account.type`, `df.mean` always work —
  except `ok`/`reason`/`kind`/`trace`, which are error-metadata on every
  value and can't be struct field names (a frame column named so is read
  after `rename[f, 'kind', 'k2']`).

## Where to look for more

- `docs/book/` — The Crow Book: the tutorial, every code block machine-verified in both
  execution modes. Chapter order is the recommended learning order.
- `docs/book/src/stdlib/` — a page per module listing every builtin, generated from the
  compiler's own tables.
- `docs/language-spec.md` — the full specification, including every builtin and
  a long series of "Design Note: No X" sections explaining each deliberate
  omission (they answer "why can't I just…?").
- `examples/` — runnable programs (`crow run file.co`).
- Commands: `crow run file.co` compiles and runs, `crow run -i file.co` interprets,
  `crow test file.co` runs its test blocks, `crow check` verifies a project, `crow stop`
  stops a served app; `crow help` lists the rest.
