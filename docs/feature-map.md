# Crow Feature Inventory

A quick lookup: what exists, where it is documented, and what was left out on
purpose. **Source of truth:** `docs/language-spec.md`. Every keyword, operator
and form is on one page in the book's
[Appendix A](book/src/appendix-a-language-at-a-glance.md); every name of the
standard library, with an example of each, is in
[The Standard Library](book/src/stdlib/index.md), generated from the
compiler's own tables.

## Operators

| Syntax | What it does | Tensor behavior |
|--------|-------------|-----------------|
| `+` `-` | Add / subtract | Element-wise (always) |
| `*` | Multiply | **Matmul** (tensor×tensor), scalar multiply (tensor×scalar) |
| `.*` | — | Element-wise multiply |
| `/` | Divide (int÷int rounds toward zero: `-7 / 2` is `-3`) | Scalar divide only (tensor÷tensor → error) |
| `./` | — | Element-wise divide |
| `^` | Power (right-associative; `-x ^ 2` is `-(x ^ 2)`) | Scalar power |
| `.^` | — | Element-wise power |
| `%` | Remainder of ints, with the left side's sign | — |
| `eq ne gt lt gte lte` | Comparison (words, never symbols); two kinds are not compared | Column/tensor vs scalar → boolean mask |
| `and or not` | Logical; `and` and `or` decide from the left | Masks combine element-wise |
| `in` `not in` | Membership: an item of a list, a part of text, a key of a map | A number in a vector; over a text column, a mask cell by cell |
| `then` | Pipe: `x then f[a]` = `f[x, a]`; as lines, one step a line under an assignment | — |
| `` 'text `expr`' `` | String interpolation (paired backticks) | — |
| `---` | Comment to end of line | — |

## Core builtins (no import needed) — spec section 11

| Area | Names |
|------|-------|
| Output, checks | `log` `assert` `halt` `type` `error` |
| Numbers | `abs` `sqrt` `floor` `ceil` `round` `min` `max` `rand` |
| Strings | `split` (`split[s, '']` is its characters) `trim` `replace` `occurs` `translate` `upper` `lower` `join` `starts` `ends` `repeat` `fmt` `reverse` |
| Regex | `matches` `find[s, pattern]` `sub` |
| Conversion | `int` `float` `decimal` `fraction` `str` |
| JSON, encodings, hashing | `json` `encode` `decode` `hash` |
| Collections | `len` `slice` `push` `pop` `sort` `reverse` `concat` `range`; `x in xs` |
| Maps | `map[k = v]` `map[keys, values]` `keys` `values` `set` `del`; `k in m` |
| Search (named predicates, parallel early exit) | `find[arr, pred]` `any` `all` |
| Moments (an instant, and the offset it was written in) | `now` `moment`, fields `t.year` … `t.weekday` `t.offset`, `fmt[t, style =, tz =]` `bucket` `float[t]`; `t + s` and `t2 - t1` in seconds |
| Freshness | `captured[x]` |
| Background work | `later[fn, args…]` `running[fn]`; in a page's handler, `notice[text]` tells the reader |
| Verification | `test` blocks; `assert`; in tests of live apps `app` `click` `change` `html` `elements` |
| Files, process, system | `read` `write` `exists` `mkdir` `ls` `rm` `mv` `cp` `args` `input` `env` `exit` `shell` `sleep` |
| HTTP | `request[url, method, body]` with `headers =` `query =` `form =` `files =` `save =` `timeout =`; `read`, `load`, `csv` and `lines` take a URL or an S3 object as their path |
| Resources, autograd | `cpu` `gpu` `grad` |
| Charts (inside views) | `chart[frame, x =, candles =, lines =, bars_y =, marker =, tz =, height =, panes =, levels =, points =, seek =]` |

## Modules — reserved only when imported bare (`use math as m` reserves just `m`)

| Import | Provides | Reference |
|--------|----------|-----------|
| `use math` | tensor creation, element-wise ops, reductions with `axis =`, sorting/selection, shape ops, linear algebra, random, FFT and filters, special functions, complex, sparse, `load` (a table of numbers as a matrix: CSV, JSON or Parquet), constants `pi e inf nan` | spec §13 math, `math-library.md` |
| `use math.stats` | `median std var percentile zscore corr cov ttest linreg chisq anova kstest fit` | spec §13 math → Statistics |
| `use math.nn` | `relu sigmoid softmax gelu mse crossentropy bce linear` | spec §13 math → Neural Networks |
| `use frame` | `frame.new` `load save select sort group pivot window join stack head tail fill unique rename unpivot complete sample where bucket stats`; a mask picks rows (`df[df.col gt x]`); a column is written by the dot (`df.name = values`); `sort[df, 'col', desc = true]`; `join[a, b, 'key', 'left']` | spec §13 frame, `frame-module.md` |
| `use io` | `csv[path]` → array of maps, `lines[path]` | spec §13 io |
| `use doc` | `markdown chapters menu pager plain document` — Markdown made into pages, as the book is | spec §13 doc |
| `use bits` | `bitand bitor bitxor bitcount shift code char` | spec §13 bits |
| `use chart` | `sma ema rsi macd vwap rolling_sum cvd` — the indicators a `chart` draws | spec §13 chart, `chart-module-design.md` |
| `use db` | `connect query insert transact describe` (SQLite, PostgreSQL, ClickHouse); migrations are `crow db apply` | spec §13 db, `db-module.md`, `crow-check.md` |
| `use web` | `serve page route` (`serve[port, cors =, files =, refresh =, budget =]`, `route[path, handler, limit =]`); responses `text html json respond redirect msgpack arrow bytes stream`; pages `download title meta sign_in sign_out reader` | spec §13 web, `web-framework.md` |
| `use web.auth` | `sign verify` — tokens for an API | spec §13 web → Auth |
| `use web.ws` | `socket send recv close` — a WebSocket client | spec §13 web → WebSocket |
| `use web.mail` | `smtp mail` | spec §13 web → Mail |
| `use ui` | `fetch poll field slider checkbox toggle radio tabs select progress icon` + the element vocabulary inside `view` blocks: the HTML names, the containers `stack row grid overlay scroll`, and `details summary dropdown modal drawer toast divider widget`; classes `card stats controls primary up down stale` | spec §13 web → View Blocks, Layout, Styling |
| `use llm` | `reach ask asked said told chat follow next reply embed act` — a model as a connection (Anthropic's API, or any OpenAI-compatible server); tools are the functions themselves (`tools = [f, g]`); `act` is the agent loop, with limits (`rounds`, `budget`), gates (`allow = [g]`), a frame of its steps, context (`keep`, `compact`), skills (`skills = folder`) and MCP servers (`servers = [s]`) | spec §13 llm, `llm-design.md`, `agents-design.md` |
| `use mcp` | `server discover catalog tool answer listen` — the Model Context Protocol over HTTP and stdio, as a client and as a server | spec §13 mcp, `agents-design.md` |
| `use sys` | the standard library's own layer — bytes, connections, a process (`start`), `describe` and `apply` over functions; only a module of the standard library imports it, and a program's `use sys` is refused | spec §13 sys, `extending-crow.md` |

## Language constructs

| Feature | Syntax | Spec section |
|---------|--------|--------------|
| Functions, implicit return, named args with defaults | `fn f[x, lr = 0.01]` | §7 |
| Function scope isolation (module vars invisible in fns) | — | §7 "Function Scope" |
| First-class functions (by name, no lambdas) | `apply[double, 5]` | §7 |
| `for` as a parallel expression, trailing `if` filter | `xs = for x in a / if p / x` | §6, §14 |
| `while` with `break`/`continue` (sequential) | — | §6 |
| `else if` chains | — | §5 |
| Membership | `x in xs`, `x not in xs` | §4 "Membership" |
| `then` chains as lines | `x = e`, then indented lines `then f[a]` | §4 "Then Chaining" |
| Ints of any size; exact decimals and fractions | `2 ^ 100`, `decimal['12.30']`, `fraction[1, 3]` | §1 "Numbers That Keep Every Digit" |
| Structs (records, no methods) | `struct P [x, y]` | §9 |
| A record or none (a tree's links) | `0` for none; `t.left eq 0` asks | §9 "A Record or None" |
| Maps (homogeneous dictionaries, text or int keys) | `map[k = v]`, `map[keys, values]` | §8 "Maps" |
| Arrays = tensors when numeric, nested = matrices | `[[1, 2], [3, 4]]` | §8, Design Note "Unified Arrays" |
| A mask picks items, rows and cells | `xs[xs gt 2]`, `df[df.age gt 30]` | §8 "Arrays", §13 frame "Boolean Masks" |
| A frame's column written by the dot | `df.name = values`, ``df.`name` = values`` | §13 frame |
| Errors as values | `.ok .reason .kind .trace`, `error[reason, kind]` | §10 |
| Modules, aliases, reservation | `use x`, `use x as y`, qualified user modules | §12, Design Note "Names Are Never Ambiguous" |
| View blocks, reactive state, deferred event calls | `view render[]`, `click = inc`, `click = set_speed[10.0]` | §13 web |
| Derivations (values computed from facts and tables) | `derive total` | §13 web "Derivations" |
| Background work, periodic refresh | `later[f]`, `serve[p, refresh = 5, budget = 50]` | §15 |
| Test blocks | `test[n]` under a fn, `test name[]` | §16 |
| Monomorphization at call sites | automatic | §17 |
| Resource control, GPU, distributed | `cpu[n] gpu[]`, `crow launch`, `env['CROW_RANK', '0']` | §14 |
| `.env` auto-loading | `crow run` / `crow build` | §18 |

## Not in Crow, on purpose — each has a Design Note in the spec

| Missing | Use instead |
|---------|-------------|
| Lambdas, closures | a named `fn`, passed by name |
| `map[arr, f]`, `filter`, comprehensions, iterators | `for`-as-expression (with a trailing `if`); a mask picks: `xs[xs gt 2]`, `df[df.age gt 30]` |
| `reduce` | named reductions: `sum min max prod join` (`use math` for the array forms) |
| `enumerate`, `zip`, destructuring | `for i in range[len[a]]`, `r[0]`/`r[1]`; a map from two lists is `map[keys, values]` |
| `+=` and friends | `x = x + 1` |
| `"double quotes"` | `'single quotes'` (`\'` to nest) |
| `'a' + 'b'` | interpolation or `join[parts, sep]` |
| `{expr}` / format mini-language | `` `expr` `` in strings; `fmt[x, decimals = 2]` |
| Ternary, `match`, sum types, `elif` keyword | `if`/`else if`/`else` |
| `null`/`None`, `try`/`catch`, inline fallback (`or 0`) | error values; check `.ok` |
| `break` in `for` | `any`/`all`/`find`, or `while` |
| Bare column names in frame calls | strings for names, `df.col` expressions for computation |
| A column written by index (`df['c'] = v`) | `df.c = v` |
| Heterogeneous maps, map literals `[k = v]` | structs; `map[k = v]` |
| A duration type | seconds, as a number: `t + 60`, `t2 - t1` |
| Bit operators (`&`, `\|`, `<<`) | `use bits`: `bitand[a, b]`, `bitor[a, b]`, `shift[a, n]` (`^` is the power) |
| Timers, `every[]`, async/await, threads | `later[f]`, `serve[..., refresh = N]`, parallel `for` |
| Ambient globals inside functions | parameters in, return values out |
| `chars[s]` | `split[s, '']` |
