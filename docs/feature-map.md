# Crow Feature Inventory

A quick lookup: what exists, where it is documented, and what was left out on
purpose. **Source of truth:** `docs/language-spec.md`; the complete name
census is the book's [Appendix A](book/src/appendix-a-language-at-a-glance.md).

## Operators

| Syntax | What it does | Tensor behavior |
|--------|-------------|-----------------|
| `+` `-` | Add / subtract | Element-wise (always) |
| `*` | Multiply | **Matmul** (tensor×tensor), scalar multiply (tensor×scalar) |
| `.*` | — | Element-wise multiply |
| `/` | Divide (int÷int floors) | Scalar divide only (tensor÷tensor → error) |
| `./` | — | Element-wise divide |
| `^` | Power (right-associative) | Scalar power |
| `.^` | — | Element-wise power |
| `%` | Remainder | — |
| `eq ne gt lt gte lte` | Comparison (words, never symbols) | Column/tensor vs scalar → boolean mask |
| `and or not` | Logical | Masks combine element-wise |
| `then` | Pipe: `x then f[a]` = `f[x, a]` | — |
| `` 'text `expr`' `` | String interpolation (paired backticks) | — |
| `---` | Comment to end of line | — |

## Core builtins (no import needed) — spec section 11

| Area | Names |
|------|-------|
| Output, checks | `log` `assert` `type` `error` |
| Numbers | `abs` `sqrt` `floor` `ceil` `round` `min` `max` `rand` |
| Strings | `split` `trim` `replace` `upper` `lower` `join` `starts` `ends` `repeat` `fmt` `reverse` |
| Regex | `matches` `find[s, pattern]` `sub` |
| Conversion | `int` `float` `str` `json` `encode` `hash` |
| Collections | `len` `contains` `slice` `push` `pop` `sort` `reverse` `concat` `range` |
| Maps | `map[k = v]` `keys` `values` `has` `set` `del` |
| Search (named predicates, parallel early exit) | `find[arr, pred]` `any` `all` |
| Moments (a moment is epoch seconds) | `now` `moment` `calendar` `parse_time` `fmt[t, time =]` |
| Freshness | `captured[x]` |
| Background work | `later[fn, args…]` `running[fn]` |
| Verification | `test` blocks; `assert`; in tests of live apps `app` `click` `html` |
| Files, process, system | `read` `write` `exists` `mkdir` `ls` `rm` `mv` `cp` `args` `input` `env` `exit` `shell` `sleep` `time` |
| Resources, autograd | `cpu` `gpu` `nodes` `grad` |
| Charts (inside views) | `chart[frame, x =, candles =, lines =, bars_y =, marker =, tz =, height =]` |

## Modules — reserved only when imported bare (`use math as m` reserves just `m`)

| Import | Provides | Reference |
|--------|----------|-----------|
| `use math` | tensor creation, element-wise ops, reductions with `axis =`, sorting/selection, shape ops, linear algebra, random, FFT and filters, special functions, complex, sparse, `load` (numeric CSV), constants `pi e inf nan` | spec §13 math, `math-library.md` |
| `use math.stats` | `median std var percentile zscore corr cov ttest linreg chisq anova kstest fit` … | spec §13 math → Statistics |
| `use math.nn` | `relu sigmoid softmax gelu mse crossentropy bce linear` … | spec §13 math → Neural Networks |
| `use frame` | `load save select filter sort group join stack head tail fill unique rename stats bucket` …; masks via `df.col gt x`; `sort[df, 'col', desc = true]`; `join[a, b, 'key', 'left']` | spec §13 frame, `frame-module.md` |
| `use io` | `csv[path]` → array of maps, `lines[path]` | spec §13 io |
| `use db` | `connect query exec insert transact migrate` (SQLite, PostgreSQL, ClickHouse) | spec §13 db, `db-module.md`, `crow-check.md` |
| `use web` (+ `web.auth` `web.cors` …) | `serve page route respond redirect html text send recv close` … | spec §13 web, `web-framework.md` |
| `use ui` | `fetch poll field` + the HTML element vocabulary inside `view` blocks; classes `card stats controls row primary up down stale` | spec §13 web → View Blocks, Styling |

## Language constructs

| Feature | Syntax | Spec section |
|---------|--------|--------------|
| Functions, implicit return, named args with defaults | `fn f[x, lr = 0.01]` | §7 |
| Function scope isolation (module vars invisible in fns) | — | §7 "Function Scope" |
| First-class functions (by name, no lambdas) | `apply[double, 5]` | §7 |
| `for` as a parallel expression, trailing `if` filter | `xs = for x in a / if p / x` | §6, §14 |
| `while` with `break`/`continue` (sequential) | — | §6 |
| `else if` chains | — | §5 |
| Structs (records, no methods) | `struct P [x, y]` | §9 |
| Maps (homogeneous dictionaries) | `map[k = v]` | §8 "Maps" |
| Arrays = tensors when numeric, nested = matrices | `[[1, 2], [3, 4]]` | §8, Design Note "Unified Arrays" |
| Errors as values | `.ok .reason .kind .trace`, `error[reason, kind]` | §10 |
| Modules, aliases, reservation | `use x`, `use x as y`, qualified user modules | §12, Design Note "Names Are Never Ambiguous" |
| View blocks, reactive state, deferred event calls | `view render[]`, `click = inc`, `click = set[10]` | §13 web |
| Background work, periodic refresh | `later[f]`, `serve[p, refresh = 5, budget = 50]` | §15 |
| Test blocks | `test[n]` under a fn, `test name[]` | §16 |
| Monomorphization at call sites | automatic | §17 |
| Resource control, GPU, distributed | `cpu[n] gpu[]`, `crow launch` | §14 |
| `.env` auto-loading | `crow run` / `crow build` | §18 |

## Not in Crow, on purpose — each has a Design Note in the spec

| Missing | Use instead |
|---------|-------------|
| Lambdas, closures | a named `fn`, passed by name |
| `map[arr, f]`, `filter`, comprehensions, iterators | `for`-as-expression (with a trailing `if`) |
| `reduce` | named reductions: `sum min max prod join` (`use math` for the array forms) |
| `enumerate`, `zip`, destructuring | `for i in range[len[a]]`, `r[0]`/`r[1]` |
| `+=` and friends | `x = x + 1` |
| `"double quotes"` | `'single quotes'` (`\'` to nest) |
| `'a' + 'b'` | interpolation or `join[parts, sep]` |
| `{expr}` / format mini-language | `` `expr` `` in strings; `fmt[x, decimals = 2]` |
| Ternary, `match`, sum types, `elif` keyword | `if`/`else if`/`else` |
| `null`/`None`, `try`/`catch`, inline fallback (`or 0`) | error values; check `.ok` |
| `break` in `for` | `any`/`all`/`find`, or `while` |
| Bare column names in frame calls | strings for names, `df.col` expressions for computation |
| Heterogeneous maps, map literals `[k = v]` | structs; `map[k = v]` |
| Timers, `every[]`, async/await, threads | `later[f]`, `serve[..., refresh = N]`, parallel `for` |
| Ambient globals inside functions | parameters in, return values out |
| `chars[s]` | `s[i]` in a `for` |
