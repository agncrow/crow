# Appendix B: Coming from Another Language

Every language you already know has habits that Crow deliberately breaks.
Most wrong reflexes below produce a compile error that names the fix, so
you would find those out anyway — this appendix saves you the round-trips,
and warns of the few that run and mean something else: `^` is the power,
not exclusive or, and an index counts from 0. Read the six facts, then the
section for your language.

## The six facts that cover most of it

1. **Calls use square brackets.** `f[x]`, `sqrt[9]`, `log['hi']`. Parentheses only group arithmetic.
2. **One string quote, and interpolation is paired backticks.** `'hello `name`'`. No `+` on strings: interpolate, or `join[parts, sep]`.
3. **Comparison and logic are words.** `eq ne gt lt gte lte`, `and or not`. No `==`, `<`, `&&`, `!`, and no `+=`.
4. **Blocks are indentation; there is no `:`, `{}`, `end`, or `;`.** `fn f[x]`, `if x gt 0`, `else if`, `while`, `for x in xs`.
5. **No lambdas, no null, no exceptions.** Behavior is a named `fn` passed by name; failure is a value with `.ok` and `.reason` that propagates through use and refuses to decide a branch.
6. **`for` is an expression that runs in parallel.** `ys = for x in xs` … `x * 2` is your `map`; a trailing `if` is your `filter`; `any`/`all`/`find` with a named predicate are your early-exit searches. A loop whose body prints, writes or draws keeps its order, decided for you. There are no threads to write.

A seventh, for anyone with a data background: an all-numeric array *is* a
tensor (`*` is matmul, `.*` is element-wise), and a table is a *frame*
(`use frame`), not a matrix.

(The same material, phrased for pasting into an AI assistant, lives in
`docs/crow-for-llms.md`.)

## From Python

| Python | Crow |
|---|---|
| `f(x)` | `f[x]` — brackets call |
| `"text"` / `f"hi {name}"` | `'text'` / `` 'hi `name`' `` |
| `"a" + "b"` | interpolate, or `join[parts, sep]` |
| `x += 1` | `x = x + 1` |
| `x ** 2` | `x ^ 2` (`-x ^ 2` is `-(x ^ 2)`, as in Python) |
| `a ^ b`, `a & b`, `a << n` | `bitxor[a, b]`, `bitand[a, b]`, `shift[a, n]` after `use bits` — `^` is the power |
| `==  !=  <  >=` | `eq  ne  lt  gte` |
| `and or not` | same words, same meaning |
| `x in xs`, `x not in xs` | the same — an item of a list, a part of text, a key of a map |
| `def f(x):` | `fn f[x]` + indentation (no `:`) |
| `lambda` | a named `fn`, passed by name |
| `class` | `struct` + free functions |
| `elif` | `else if` |
| `match/case` | `if`/`else if`/`else` |
| `import numpy as np` | `use math` (bare) or `use math as m` |
| `print(x)` | `log[x]` |
| `None` | doesn't exist — error values + `.ok` |
| `True/False` | `true/false` |
| `try/except/raise` | errors are values (chapter 7); `raise` is `return error[reason, kind]` |
| `a, b = f()` | `r = f[]`, then `a = r[0]` and `b = r[1]` |
| `[x*2 for x in xs]` | `for`-as-expression |
| `xs.append(v)` | `xs = push[xs, v]` — a builtin answers a new value |
| `for c in s`, `for k in d` | `for c in split[s, '']`, `for k in keys[m]` — text and maps are not walked |
| `break` in a `for` loop | `while`, or `find`/`any`/`all` — a `for`'s rounds run at once |
| `dict` | `map[k = v]` (one value type) or a `struct` |
| `# comment` | `--- comment` |
| global variables in functions | invisible — pass parameters (chapter 4) |
| threads / asyncio / multiprocessing | delete them — `for` is already parallel where its body only computes; `later[f]` for background work |
| `pandas.DataFrame` | `use frame`; `df[df.age > 30]` → `df[df.age gt 30]`, `df['c'] = v` → `df.c = v` |
| `numpy` arrays | plain numeric arrays; `@` → `*`, `*` → `.*` |
| `datetime.now()`, `datetime(2026, 7, 14)`, `timedelta` | `now[]`, `moment[2026, 7, 14]`, seconds: `t + 60` |
| `Decimal('0.1')`, `Fraction(1, 3)` | `decimal['0.1']`, `fraction[1, 3]`; an int grows as Python's does |
| `json.loads(s)`, `json.dumps(v)` | `json[s]`, `encode[v]` |
| `requests.get(url)` | `request[url]` |
| the `anthropic` or `openai` SDK, an agent framework | `use llm`: `chat[m, talk]`; `act[m, talk, tools = [f]]` runs the tool loop |
| `pytest` files | `test` blocks under the function (chapter 9) |

## From JavaScript and TypeScript

| JavaScript | Crow |
|---|---|
| `const x = 5;` / `let` / `var` | `x = 5` — no keyword, no semicolon |
| `f(x)` | `f[x]` |
| `"str"` / `` `hi ${name}` `` | `'str'` / `` 'hi `name`' `` (backticks go *inside* the quotes) |
| `"a" + b` | interpolate, or `join` |
| `===  !==  <  >=` | `eq  ne  lt  gte` |
| `&&  \|\|  !` | `and  or  not` |
| `x++`, `x += 1` | `x = x + 1` |
| `x ** 2` | `x ^ 2` |
| `a ^ b`, `a & b`, `a << n` | `bitxor[a, b]`, `bitand[a, b]`, `shift[a, n]` after `use bits` — `^` is the power |
| `function f(x) { }` / `(x) => x * 2` | `fn f[x]` + indented body; no arrow functions |
| `{ }` blocks | indentation |
| `x ? a : b` | `if`/`else` statements |
| `console.log(x)` | `log[x]` |
| `null` / `undefined` | don't exist — error values + `.ok` |
| `try/catch/throw` | errors are values; `throw` is `return error[reason, kind]` |
| `async/await`, Promises | none — `later[f]` runs work off the loop; `for` is parallel |
| `xs.map(f)`, `.filter`, `.reduce` | `for`-as-expression; named reductions (`sum` …) |
| `xs.find(p)`, `.some`, `.every` | `find[xs, p]`, `any[xs, p]`, `all[xs, p]` with a named `fn` |
| `xs.includes(x)`, `'k' in obj` | `x in xs`, `'k' in m` |
| `xs.push(x)` | `xs = push[xs, x]` |
| `{ k: v }` object | `map[k = v]` (one value type) or a `struct` |
| `class` | `struct` + free functions |
| `import x from 'y'` | `use y` (then `y.x`) |
| `JSON.parse(s)`, `JSON.stringify(v)` | `json[s]` (error value on bad input), `encode[v]` |
| `fetch(url)` | `request[url]`; in a page, `fetch[url]` loads a frame when the page connects |
| `new Date()`, `Date.parse(s)` | `now[]`, `moment[s]` |
| `BigInt`, `123n` | plain ints: an int grows past 64 bits |
| `xs.length` | `len[xs]` |
| `// comment` | `--- comment` |
| React/Vue components + fetch + REST | `view` blocks over reactive state, rendered on the server (chapter 12) |
| `npm test`, Jest | `test` blocks under the function; `crow test` |
| Vercel's AI SDK, LangChain.js | `use llm`: `chat[m, talk]`; `act[m, talk, tools = [f]]` runs the tool loop |

## From R

| R | Crow |
|---|---|
| `x <- 5` | `x = 5` |
| `f(x)` | `f[x]` |
| `c(1, 2, 3)` | `[1, 2, 3]` |
| 1-based indexing, `x[1]` | 0-based, `x[0]`; `x[-1]` is the *last* element, not "all but the first" |
| `function(x) x * 2` | a named `fn`, passed by name |
| `xs %>% f() %>% g(a)` | `xs then f then g[a]` |
| `sapply(xs, f)`, `lapply`, `purrr::map` | `for`-as-expression |
| `df[df$age > 30, ]`, `dplyr::filter` | `df[df.age gt 30]` |
| `df$col`, `df$c <- v`, `mutate` | `df.col`, `df.c = v` |
| `nrow(df)` | `len[df]` |
| `group_by %>% summarise` | `group[df, 'dept', ['salary', 'mean']]` |
| `NULL`, `NA` | no null — error values: a NULL cell read from a database, or a frame's missing cell, is an error of reason `missing`; a mask excludes missing cells |
| `TRUE/FALSE` | `true/false` |
| `==  !=  <=` | `eq  ne  lte` |
| `&&  \|\|  !`, `&  \|` | `and  or  not` (they work element-wise on columns too) |
| `ifelse(c, a, b)` | `if`/`else`; over columns, `where[mask, a, b]` |
| `paste0("a", x)`, `sprintf` | interpolation `` '`x`' ``, `fmt[x, decimals = 2]` |
| `print(x)`, `cat` | `log[x]` |
| `library(x)` | `use x` |
| `stop("msg")`, `tryCatch` | `return error['msg', kind]`; check `.ok` |
| `%in%` | `v in xs` |
| `# comment` | `--- comment` |
| `mean(x)`, `sd(x)` | `mean[x]` after `use math`, `std[x]` after `use math.stats` |
| `Sys.time()`, `as.POSIXct(s)` | `now[]`, `moment[s]` |

## From MATLAB and Octave

| MATLAB | Crow |
|---|---|
| `f(x)` and `x(2)` — parentheses for both | `f[x]` and `x[1]` — brackets for both, 0-based |
| `[1 2 3]`, `[1, 2, 3]` | `[1, 2, 3]` (commas required) |
| `[1 2; 3 4]` | `[[1, 2], [3, 4]]` |
| `A * B`, `A .* B` | the same: `*` matmul, `.*` element-wise (also `./`, `.^`) |
| `A'` | `transpose[A]` |
| `A \ b` | `solve[A, b]` |
| `zeros(3, 4)`, `eye(3)`, `linspace(0, 1, 5)` | `zeros[3, 4]`, `eye[3]`, `linspace[0, 1, 5]` after `use math` |
| `x(end)` | `x[-1]` |
| `length(x)`, `numel(A)`, `size(A)` | `len[x]` (a matrix's rows), `len[flatten[A]]`, `A.shape` |
| `function y = f(x) … end` | `fn f[x]` + indented body; last expression returns |
| `if … elseif … else … end` | `if` / `else if` / `else`, indentation, no `end` |
| `for i = 1:10 … end` | `for i in range[10]` (0–9); the loop is parallel |
| `while … end` | `while`, indentation |
| `~=`, `==`, `<=` | `ne`, `eq`, `lte` |
| `&&  \|\|  ~` | `and  or  not` |
| `% comment` | `--- comment` |
| `disp(x)`, `fprintf('%.2f', x)` | `log[x]`, `` log['`fmt[x, decimals = 2]`'] `` |
| `'text'` | the same single quotes; `"text"` is not accepted |
| `struct('a', 1)` | `struct Name [a]` then `Name[1]` |
| `cellfun`, `arrayfun` | `for`-as-expression |
| `try … catch … end` | errors are values; check `.ok` |
| `parfor` | plain `for` — already parallel |
| tables (`readtable`) | `use frame`; `frame.load['file.csv']` |
| `bitand(a, b)`, `bitshift(a, n)` | `bitand[a, b]`, `shift[a, n]` after `use bits` |

## From Julia

| Julia | Crow |
|---|---|
| `f(x)` | `f[x]` |
| `function f(x) … end` / `f(x) = x * 2` | `fn f[x]` + indented body |
| `x -> x * 2` | a named `fn`, passed by name |
| `"hi $name"`, `"$(x + 1)"` | `` 'hi `name`' ``, `` '`x + 1`' `` |
| 1-based, `x[end]` | 0-based, `x[-1]` |
| `A * B`, `A .* B`, `A .^ 2` | the same operators |
| `A'` | `transpose[A]` |
| `[x^2 for x in xs]`, `map(f, xs)` | `for`-as-expression |
| `Dict("a" => 1)` | `map[a = 1]` |
| `struct P; x; y; end` | `struct P [x, y]` |
| `x::Int` annotations | none — inferred |
| `using Statistics` | `use math, math.stats` |
| `nothing`, `missing` | no null — error values |
| `big(2)^100`, `1//3`, `BigFloat` | `2 ^ 100` (an int grows), `fraction[1, 3]`, `decimal[x, digits = 50]` |
| `try/catch`, `error("…")` | errors are values; `return error['…', kind]` |
| `Threads.@threads for` | plain `for` — already parallel |
| `push!(v, x)` | `v = push[v, x]` — a builtin answers a new value, and leaves what it was handed as it was |
| `println(x)` | `log[x]` |
| `# comment` | `--- comment` |
| `==  !=  <=  &&  \|\|  !` | `eq  ne  lte  and  or  not` |
| `1:10` | `range[1, 11]` (exclusive end) |
| `@time`, `@test` | `t0 = now[]`, then `now[] - t0` in seconds; `test` blocks |

## From Go

| Go | Crow |
|---|---|
| `x := 5`, `var x int` | `x = 5` |
| `func f(x int) int { }` | `fn f[x]` + indented body — no types written |
| `f(x)` | `f[x]` |
| `if err != nil { return err }` | nothing — a failed value propagates through use automatically; check `.ok` only where you handle it |
| `value, err := f()` | `v = f[]` — one value that carries its own `.ok` and `.reason` |
| `errors.New("…")`, `fmt.Errorf` | `error['…', kind]` |
| `nil` | doesn't exist |
| `for i := 0; i < n; i++` | `for i in range[n]` (parallel) or `while` (sequential) |
| `for _, x := range xs` | `for x in xs` |
| `go f()`, channels, `sync.WaitGroup` | `for` is already parallel; `later[f]` for background work |
| `map[string]int{}` | `m = map[]`, then `m = set[m, k, v]` |
| `append(xs, x)` | `xs = push[xs, x]` |
| `type P struct { … }` + methods | `struct P [x, y]` + free functions |
| `switch` | `if` / `else if` |
| `fmt.Println(x)`, `fmt.Sprintf("%.2f", x)` | `log[x]`, `fmt[x, decimals = 2]` |
| `==  !=  <=  &&  \|\|  !` | `eq  ne  lte  and  or  not` |
| `x++`, `x += 1` | `x = x + 1` |
| `a ^ b`, `a & b`, `a << n` | `bitxor[a, b]`, `bitand[a, b]`, `shift[a, n]` after `use bits` — `^` is the power |
| `math/big` | nothing to import: an int grows past 64 bits; `fraction[n, d]` for a ratio |
| `// comment` | `--- comment` |
| `package` / `import "x"` | `use x` |
| `_test.go` files, `testing.T` | `test` blocks under the function; `crow test` |
| `defer` | not needed — a file is read or written in one call, and a database connection is the program's pool; a WebSocket is closed with `close[s]` |
| `net/http` handlers | `use web`; `route['/api', handler]`, `serve[3000]` |
| `http.Get(url)` | `request[url]` |

## From Rust

| Rust | Crow |
|---|---|
| `let x = 5;` / `let mut` | `x = 5` — no keyword, no `mut`, no semicolon |
| `fn f(x: i64) -> i64 { }` | `fn f[x]` — types inferred at each call site (section 17 of the spec) |
| `f(x)` | `f[x]` |
| `Result<T, E>`, `?` | every value is a result already; propagation is automatic; `.ok`/`.reason`/`.kind` where you handle it |
| `Option<T>`, `None`, `unwrap()` | no null; values auto-unwrap on use; a miss is an error value; a tree's `Option<Box<Node>>` is a field holding `0` for none, asked `t.left eq 0` |
| `match` | `if` / `else if` — no pattern matching, no enums |
| `\|x\| x * 2` closures | a named `fn`, passed by name |
| `xs.iter().map(f).collect()` | `for`-as-expression |
| `rayon::par_iter` | plain `for` — already parallel |
| `String` vs `&str`, `&`, `&mut`, lifetimes | one string type; no references — collections are immutable-by-rebuild (`push` returns a new array) |
| `Vec<T>`, `HashMap<K, V>` | arrays, `map[k = v]` |
| `struct` + `impl` | `struct P [x, y]` + free functions; no methods, no traits |
| `enum` | none — structs, error kinds, or strings |
| `println!("{x}")`, `format!` | `log['`x`']`, `fmt[x, decimals = 2]` |
| `"str"`, `'c'` | `'str'` only; characters are 1-length strings, and `s.chars()` is `split[s, '']` |
| `==  !=  <=  &&  \|\|  !` | `eq  ne  lte  and  or  not` |
| `x += 1` | `x = x + 1` |
| `a ^ b`, `a & b`, `a << n` | `bitxor[a, b]`, `bitand[a, b]`, `shift[a, n]` after `use bits` — `^` is the power |
| `i64` overflow, `num-bigint` | an int grows past 64 bits; the compiled program keeps 64 bits where it can show they suffice |
| `// comment`, `/// doc` | `--- comment` |
| `use x::y;`, `mod` | `use x` (then `x.y`); every file is a module |
| `#[test]`, `assert_eq!` | `test` blocks with bare boolean truths; `assert[cond]` in scenarios |
| `cargo run/build/test` | `crow run/build/test` (and `crow check`) |
| `async/await`, `tokio::spawn` | none — `later[f]`; the live loop is owned by the runtime |

## From Elixir and Erlang

| Elixir | Crow |
|---|---|
| `def f(x), do: …` / `fn x -> … end` | `fn f[x]` + indented body; no anonymous fns |
| `f(x)`, `f.(x)` | `f[x]` |
| `x \|> f() \|> g(a)` | `x then f then g[a]` |
| `{:ok, v}` / `{:error, reason}` + `case` | one value: `.ok`, `.reason`, `.kind`; no tuples, no pattern matching |
| `with`, `case`, `cond` | `if` / `else if`; failures propagate on their own |
| `Enum.map`, `Enum.filter`, `Enum.reduce` | `for`-as-expression; named reductions |
| `Enum.any?`, `Enum.find` | `any[xs, p]`, `find[xs, p]` |
| `%{a: 1}` | `map[a = 1]` (one value type); records are `struct` |
| `defstruct` | `struct P [x, y]` |
| `nil` | doesn't exist |
| `IO.puts`, `"#{x}"` | `log[x]`, `` '`x`' `` |
| `# comment` | `--- comment` |
| `Task.async`, `spawn`, GenServer | `for` is parallel; `later[f]` for off-loop work; reactive state is the mailbox |
| Phoenix LiveView `render`/`handle_event`/`assign` | `view render[]`, handlers as named fns bound with `click = inc`, top-level reactive variables (chapter 12) — the same model, without the boilerplate |
| `ExUnit` | `test` blocks under the function |
| immutability everywhere | the same — `push`, `set` return new values |

## From Ruby and Crystal

| Ruby | Crow |
|---|---|
| `def f(x) … end` | `fn f[x]` + indentation, no `end` |
| `f(x)`, `f x` | `f[x]` — always brackets |
| `xs.each { \|x\| … }`, `.map`, `.select` | `for`-as-expression (parallel); no blocks or procs |
| `xs.include?(x)` | `x in xs` |
| `xs << x`, `xs.push(x)` | `xs = push[xs, x]` |
| `"#{x}"` | `` '`x`' `` |
| `'a' + 'b'`, `<<` | interpolate, or `join` |
| `nil` | doesn't exist |
| `begin/rescue/raise` | errors are values; `return error[reason, kind]` |
| `unless`, `x if c` | `if not c`; no modifiers |
| `elsif` | `else if` |
| `x ? a : b` | `if`/`else` |
| `class` + methods, `attr_accessor` | `struct P [x, y]` + free functions |
| `{ a: 1 }`, symbols | `map[a = 1]`; no symbols — strings |
| `puts x` | `log[x]` |
| `==  !=  <=  &&  \|\|  !` | `eq  ne  lte  and  or  not` |
| `x += 1` | `x = x + 1` |
| `# comment` | `--- comment` |
| `require`, `require_relative` | `use x` |
| RSpec / Minitest | `test` blocks under the function |
| Crystal's `x : Int32` annotations | none — inferred |

## From Swift

| Swift | Crow |
|---|---|
| `let x = 5` / `var` | `x = 5` |
| `func f(_ x: Int) -> Int { }` | `fn f[x]` + indented body |
| `f(x)`, `f(x: 1)` | `f[x]`, `f[x = 1]` (named args need a default) |
| `"hi \(name)"` | `` 'hi `name`' `` |
| `Optional`, `?`, `!`, `if let`, `guard let` | no optionals — a failed value carries `.ok`; check it once, at the boundary |
| `throws`, `try`, `do/catch` | errors are values; `return error[reason, kind]` |
| `{ x in x * 2 }` closures | a named `fn`, passed by name |
| `xs.map { }`, `.filter`, `.first(where:)` | `for`-as-expression, `find[xs, p]` |
| `xs.contains(x)` | `x in xs` |
| `xs.append(x)` | `xs = push[xs, x]` |
| `struct` with methods, `class`, `enum` | `struct P [x, y]` + free functions; no enums |
| `[1, 2, 3]`, `["a": 1]` | `[1, 2, 3]`, `map[a = 1]` |
| `switch` | `if` / `else if` |
| `print(x)` | `log[x]` |
| `==  !=  <=  &&  \|\|  !` | `eq  ne  lte  and  or  not` |
| `x += 1` | `x = x + 1` |
| `// comment` | `--- comment` |
| `import Foundation` | `use math`, `use frame`, … |
| `async/await`, `Task {}` | none — `later[f]`; `for` is parallel |
| SwiftUI `View` | `view` blocks — a tree, rendered by the runtime (chapter 12) |
| XCTest | `test` blocks under the function |

## From Zig

| Zig | Crow |
|---|---|
| `const x = 5;` / `var` | `x = 5` |
| `fn f(x: i64) i64 { }` | `fn f[x]` — no types written, inferred per call site |
| `!T` error unions, `try`, `catch`, `orelse` | every value is a result; propagation is automatic; `.ok`/`.reason` where you handle it |
| `?T` optionals, `null` | none — error values |
| `switch` | `if` / `else if` |
| `for (xs) \|x\| { }` | `for x in xs` — parallel, and an expression |
| `while (i < n) : (i += 1)` | `while i lt n` … `i = i + 1` |
| `std.debug.print("{d}\n", .{x})` | `log[x]` |
| `[_]i64{1, 2, 3}`, slices, allocators | `[1, 2, 3]` — no allocation to manage |
| `struct { }` with methods | `struct P [x, y]` + free functions |
| `defer`, `comptime` | neither exists; nothing to free, nothing to compute at compile time |
| `==  !=  <=  and  or  !` | `eq  ne  lte  and  or  not` |
| `x += 1` | `x = x + 1` |
| `a ^ b`, `a & b`, `a << n` | `bitxor[a, b]`, `bitand[a, b]`, `shift[a, n]` after `use bits` — `^` is the power |
| `// comment` | `--- comment` |
| `@import("std")` | `use math`, `use io`, … |
| `test "name" { }` | `test name[]`, a named scenario; a `test` block written under a function is that function's examples and laws |
| `zig build`, `zig test` | `crow build`, `crow test`, `crow check` |
