# Core builtins

Always available, no import. The tables below are the spec's, with the book pages whose verified code uses each name.


The functions available in every file without an import. These names are reserved — a definition may not reuse them (see Design Note: Names Are Never Ambiguous). Module functions (`use math`, `use frame`, …) are in section 13.

A builtin handed a number of arguments, or a kind of argument, it does not take is refused before the program runs, in both modes, naming what it takes: `min['a', 'b']` is `min[] takes an int and an int or a number and a number, got text and text`. (The interpreter had stopped on the value while the compiled program did not build — or, for `min` of text, compared it.) A module's function is refused alike: `mean['a']` is `mean[] takes numbers, got text`. A parsed JSON value is read as one type before a builtin computes with it (see JSON). An argument whose kind shows only as the program runs — an item of a list of parsed values, a cell no description typed — is read then, by the same rule.


## Core

| Function | Description | Example |
|----------|-------------|---|
| `log[value]` | Print a value to stdout, as it shows: text bare, a number as it displays (a whole float as an int), a list as `[1, 'a']` with its text quoted, a map as `[key = value]`, a record as `Point[x = 1, y = 2]` (its fields in name order; a module's record by the name it was declared by, as `type` names it), a value that failed as `error: reason` — the same in both modes, through one display (`crow_runtime::display`) | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |
| `range[stop]` | Array of ints from 0 to stop (exclusive); every argument of `range` is an int (`range[2.5]` is refused) | [Arrays](../by-example/arrays.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Web: responses and submodules](../by-example/web-more.md) · [for as an Expression](../ch06-01-for-as-expression.md) |
| `range[start, stop]` | Array of ints from start to stop (exclusive) | [Arrays](../by-example/arrays.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Web: responses and submodules](../by-example/web-more.md) · [for as an Expression](../ch06-01-for-as-expression.md) |
| `range[start, stop, step]` | Array of ints with custom step; a negative step counts down (`range[5, 0, -2]` is `[5, 3, 1]`); a zero step is an error | [Arrays](../by-example/arrays.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Web: responses and submodules](../by-example/web-more.md) · [for as an Expression](../ch06-01-for-as-expression.md) |
| `map[]` | Create an empty map | [Maps](../by-example/maps.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Testing](../ch09-00-testing.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `assert[cond]` | Halt with error if condition is falsy | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Testing](../ch09-00-testing.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `assert[cond, msg]` | Halt with custom error message | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Testing](../ch09-00-testing.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `error[reason]` | Create an error-value with the given reason | [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Testing](../ch09-00-testing.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) |
| `error[reason, kind]` | Create an error-value with reason and category | [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Testing](../ch09-00-testing.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) |
| `cpu[n]` | Set thread count for parallel loops | [Controlling Resources](../ch06-03-resources.md) |
| `gpu[n]` | Set GPU count (`gpu[]` for all GPUs) | [Controlling Resources](../ch06-03-resources.md) |
| `nodes[n]` | Set machine count for distributed compute | [Controlling Resources](../ch06-03-resources.md) |

## Math

| Function | Description | Example |
|----------|-------------|---|
| `abs[n]` | Absolute value | [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `sqrt[n]` | Square root (returns float) | [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
| `floor[n]` | Floor (returns int) | [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
| `ceil[n]` | Ceiling (returns int) | [Numbers and conversion](../by-example/numbers.md) |
| `round[n]` | Round to nearest int, a half away from zero (`round[2.5]` is `3`, `round[-2.5]` is `-3`); `floor`, `ceil` and `round` of a float past 64 bits are cut to the largest int, of NaN `0` | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
| `base ^ exp` | Exponentiation (use `^` operator) |  |
| `min[a, b]` | Minimum of two numbers | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Numbers and conversion](../by-example/numbers.md) |
| `max[a, b]` | Maximum of two numbers | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: tensors](../by-example/math.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Testing](../ch09-00-testing.md) |

## Strings

| Function | Description | Example |
|----------|-------------|---|
| `split[s, delim]` | Split string into array | [Strings](../by-example/strings.md) · [Strings and Interpolation](../ch03-03-strings.md) |
| `trim[s]` | Remove leading/trailing whitespace | [Strings](../by-example/strings.md) |
| `replace[s, old, new]` | Replace all occurrences | [Strings](../by-example/strings.md) |
| `upper[s]` | Convert to uppercase | [Strings](../by-example/strings.md) · [Strings and Interpolation](../ch03-03-strings.md) |
| `lower[s]` | Convert to lowercase | [Strings](../by-example/strings.md) |
| `starts[s, prefix]` | Check if string starts with prefix | [Documentation: `use doc`](../by-example/doc.md) · [Strings](../by-example/strings.md) |
| `ends[s, suffix]` | Check if string ends with suffix | [Strings](../by-example/strings.md) |
| `join[arr, sep]` | Join array elements into string: text as it is, a number as it displays, a bool as `true` or `false`; a frame's column's cells too, where a missing cell stops the program, as it does where numbers are taken | [Frames: `use frame`](../by-example/frames.md) · [Strings](../by-example/strings.md) · [Strings and Interpolation](../ch03-03-strings.md) |
| `repeat[s, n]` | Repeat string `n` times | [Strings](../by-example/strings.md) |
| `fmt[x, ...]` | Format a value with named arguments (see Formatting) | [Formatting](../by-example/formatting.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `reverse[s]` | Reverse a string | [Arrays](../by-example/arrays.md) · [Strings](../by-example/strings.md) |

## Formatting

`fmt[value, ...]` formats numbers and strings using named arguments — plain Crow, no format mini-language:

| Named argument | Meaning | Default | Example |
|---|---|---|---|
| `decimals = n` | Fixed decimal places (numbers) | off |  |
| `commas = true` | Thousands separators | false |  |
| `width = n` | Minimum width (pads with `fill`) | off |  |
| `align = 'left'/'right'` | Alignment within width | numbers right, strings left |  |
| `fill = c` | Pad character | `' '` | [Frames: `use frame`](../by-example/frames.md) |
| `percent = true` | Multiply by 100, append `%` (numbers) | false |  |

```
fmt[1234.567, decimals = 2]                  --- 1234.57
fmt[1234.567, decimals = 2, commas = true]   --- 1,234.57
fmt[0.123, percent = true, decimals = 1]     --- 12.3%
fmt[42, width = 8]                           --- '      42'
fmt[42, width = 5, fill = '0']               --- 00042
fmt['alice', width = 8]                      --- 'alice   '
```

**Name your formats.** For repeated formats, define a function once and use it everywhere — shorter and clearer than any per-slot format code:

```
fn money[x]
    return fmt[x, decimals = 2, commas = true]

log['P&L: `money[pnl]`']
```

`decimals`/`percent` on a string is an error. `fmt` replaces the removed `pad` builtin (`pad[s, n]` → `fmt[s, width = n]`).

### Design Note: No Format Mini-Language

**What was considered:** Python-style format specs inside interpolation (`{pnl:,.2f}`) or as spec strings (`fmt[x, '.2f']`).

**Why rejected:** format codes are a second grammar — cryptic single-character commands with their own parser, embedded in strings. That violates the no-DSL rule and requires memorization; nobody writes `%-+8.2e` without a lookup. Named arguments are ordinary Crow: self-documenting, validated at compile time (unknown names and arity are lowering errors), and guessable without docs.

**The trade:** one-off formatting is wordier than `{pnl:,.2f}`. The sanctioned recovery is the named formatter (`money[x]`), which is *shorter* than the Python spec at the call site, has a domain name, and keeps every money field in the project formatted identically — the consistency that per-slot codes never provide.

## Conversion

| Function | Description | Example |
|----------|-------------|---|
| `int[value]` | Convert to int: a float is cut toward zero (the largest int past its range), a bool is 1 or 0, text is parsed as written — no spaces around it — and text or a JSON value that names no int is an error value of kind `'parse'` | [JSON](../by-example/json.md) · [Numbers and conversion](../by-example/numbers.md) · [Web: responses and submodules](../by-example/web-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Data Types](../ch03-02-data-types.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) |
| `float[value]` | Convert to float, as `int` reads its value | [Numbers and conversion](../by-example/numbers.md) · [Data Types](../ch03-02-data-types.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `str[value]` | The text a value shows as, as `log` shows it (`str[[1, 2]]` is `'[1, 2]'`). (Compiled, a record holding a list, a list of records and a map of them had shown in Rust's debug form, and a JSON list as its JSON text.) | [Numbers and conversion](../by-example/numbers.md) · [Web: responses and submodules](../by-example/web-more.md) · [Data Types](../ch03-02-data-types.md) |
| `type[value]` | Type name as string: `'int'`, `'float'`, `'str'`, `'bool'`, `'array'`, `'map'`, `'fn'`, `'none'`, `'error'`, `'frame'`, `'sparse'`, `'db'`, or struct name — the same in both modes: a frame's column is an `'array'`, a parsed JSON value what it maps to, and `shell`'s and `find`'s records `'map'` | [Numbers and conversion](../by-example/numbers.md) |

```
type[42]          --- 'int'
type[3.14]        --- 'float'
type['hello']     --- 'str'
type[[1, 2]]      --- 'array'
type[Point[1, 2]] --- 'Point'
type[error['x']]  --- 'error'
```

## Collections

| Function | Description | Example |
|----------|-------------|---|
| `len[x]` | Length of a list, of text (in characters), of a map (its entries), of a tensor (what a loop walks and an index reads: a vector's numbers, a matrix's rows — `len[flatten[m]]` counts every number), of a frame (its rows), of a frame's column (its cells, a missing one too). `x.len` is the same length; a record's field, a map's key or a frame's column named `len` is its own | [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Maps](../by-example/maps.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `push[arr, value]` | Return new array with value appended; onto a tensor, a number onto a vector or a row onto a matrix. A list holds one kind: an item of another (`push[['a'], 1]`) is refused | [Arrays](../by-example/arrays.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `pop[arr]` | Return new array without last element; an empty array stays empty | [Arrays](../by-example/arrays.md) |
| `concat[a, b]` | Return new array merging two arrays: `a`'s items, then `b`'s. An empty `a` takes its items from `b` (`idx = []`, then `idx = concat[idx, [i]]` makes a list of ints) | [Arrays](../by-example/arrays.md) · [Math: tensors](../by-example/math.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `concat[a, b, axis]` | Concatenate two tensors along an axis | [Arrays](../by-example/arrays.md) · [Math: tensors](../by-example/math.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `sort[arr]` | Return new sorted array: numbers ascending, NaN last; text by character code (`'B'` before `'a'`). A list of anything else — bools, records, lists — has no order, and is refused | [Arrays](../by-example/arrays.md) · [Frames: `use frame`](../by-example/frames.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Views and Live Updates](../ch13-03-views.md) |
| `reverse[arr]` | Return new reversed array; text reversed by character | [Arrays](../by-example/arrays.md) · [Strings](../by-example/strings.md) |
| `contains[arr, value]` | Whether an item is `value`, compared as `eq` compares (a record field by field), so a value of another kind than the items is refused; of text, whether it holds the part (`contains['hello', 'ell']`) | [Arrays](../by-example/arrays.md) · [Documentation: `use doc`](../by-example/doc.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `any[arr, pred]` | True if any element passes the predicate (parallel, early exit) | [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `all[arr, pred]` | True if every element passes (parallel, early exit) | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `find[arr, pred]` | First element passing the predicate; error-value if none | [Regular expressions](../by-example/regex.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `slice[arr, start, end]` | Elements from `start` to `end` exclusive, both clamped to the array, empty when `start` is not before `end`; a string slices by character | [Arrays](../by-example/arrays.md) · [Strings and Interpolation](../ch03-03-strings.md) |

## Map Operations

| Function | Description | Example |
|----------|-------------|---|
| `keys[m]` | Array of map keys | [Maps](../by-example/maps.md) · [Maps Are Dictionaries](../ch05-02-maps.md) |
| `values[m]` | Array of map values | [Maps](../by-example/maps.md) |
| `has[m, key]` | Check if map contains key | [Maps](../by-example/maps.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Testing](../ch09-00-testing.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `set[m, key, value]` | Return new map with entry added/updated; a value of another kind than the map's is refused | [Maps](../by-example/maps.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Testing](../ch09-00-testing.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `del[m, key]` | Return new map with entry removed | [Maps](../by-example/maps.md) |

A collection builtin takes the kinds its forms above say — a list, a map, text, a tensor — and any other is refused before the program runs, in both modes, naming the forms it takes: `has[[1, 2], 1]` is `has[] takes a map and text, got numbers and an int`, and `len[5]`, `keys[[1, 2]]` and `contains[m, 'a']` of a map are refused alike. A JSON value's kind shows only as the program runs, so `len` of one that is a number or a bool stops the program with the same words. (Compiled, those calls did not build, where the interpreter stopped as it ran; and `len` of a JSON number was 0.)

**All builtins are immutable.** No builtin modifies its arguments. To update a variable, reassign: `nums = push[nums, 4]`. Both modes turn `x = push[x, v]` (and `pop`, `sort`, `reverse`, `concat`, `set`, `del` in the same shape) into an in-place update: the collection is moved into the operation, not copied, when the right-hand side reads `x` exactly once.

## I/O

| Function | Description | Example |
|----------|-------------|---|
| `read[path]` | The text of a file, a URL or an S3 object (error-value on failure; Files, URLs and S3) | [Files and directories](../by-example/files.md) · [Moments and freshness](../by-example/moments.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [View Blocks](../ch12-01-views.md) |
| `write[path, content]` | Write text to a file or an S3 object (error-value on failure) | [Databases: `use db`](../by-example/db.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Moments and freshness](../by-example/moments.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `exists[path]` | Check if file/directory exists (returns bool) | [Files and directories](../by-example/files.md) |
| `mkdir[path]` | Create directory (and parents). Error-value on failure | [Databases: `use db`](../by-example/db.md) · [Files and directories](../by-example/files.md) |
| `ls[path]` | List directory entries as sorted array of strings. Error-value on failure | [Files and directories](../by-example/files.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `rm[path]` | Remove file or directory (recursive). Error-value on failure | [Databases: `use db`](../by-example/db.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Moments and freshness](../by-example/moments.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |
| `mv[src, dst]` | Move/rename file or directory. Error-value on failure | [Files and directories](../by-example/files.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `cp[src, dst]` | Copy file. Error-value on failure | [Files and directories](../by-example/files.md) |
| `args[]` | CLI arguments as array of strings: those after the source file when `crow` runs it, after the program when it runs compiled | [The system: shell, environment, time, arguments](../by-example/system.md) |
| `input[]` | Read a line from stdin (error-value on failure) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `input[prompt]` | Print prompt, then read a line (error-value on failure) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |

A file operation that answers nothing — `write`, `mkdir`, `rm`, `mv`, `cp`, and `use frame`'s `save` — answers empty text when it succeeds, as its type says, in both modes. (The interpreter had answered `done`, and the compiled program `''`.) What reads the world outside the program — `read`, `ls`, `shell`, and the modules' `load`, `csv` and `lines` — is stamped with the moment it was read (`captured[x]`), in both modes, and nothing else is: the compiled program had stamped `rm`'s answer.

### Files, URLs and S3

A path is a file on this machine, a URL (`http://…`, `https://…`), or an S3 object (`s3://bucket/key`) wherever the program reads one — `read`, `use io`'s `csv` and `lines`, and both modules' `load` — and a file or an S3 object wherever it writes one (`write`, `use frame`'s `save`):

```
bars = load['https://data.example.com/aapl/2026-07-14.parquet']
trades = load['s3://market-archive/2026/07/14/trades.csv']
r = save[bars, 's3://reports/aapl.parquet']
token = env['API_TOKEN']
fills = load['https://api.example.com/fills.json', headers = map[authorization = 'Bearer `token`']]
day = load['https://api.example.com/bars.csv', query = map[symbol = 'AAPL', day = '2026-07-14'], timeout = 20]
```

Every reader takes three options for a URL, in both modes: `headers = map[…]`, sent with its request — an API's key, a token; `query = map[…]`, its pairs added to the URL, each name and value URL-encoded (`query = map[q = 'crow lang']` asks `?q=crow%20lang`, after the URL's own `?…` if it has one); and `timeout = seconds`, how long the read waits for its answer (unless told, thirty seconds to connect and ten minutes in all; `0` is as none). A map's values go as text, each as it shows, so a map of numbers is one too (`query = map[page = 2]`). Without the key a server's refusal is the read's error (`cannot read 'https://…': the server answered 401 Unauthorized`), and so is a server that does not answer in time (`cannot read 'https://…': no answer in 20 s`). A file has none to send and an S3 object is signed with the environment's credentials, so either given an option is refused: `cannot read 'trades.csv' with headers, a query or a timeout: they go with an http:// or https:// URL`; a timeout below 0 is refused too (`a timeout is a number of seconds, 0 or more — not -1`).

A table's format is its extension: `.csv`, `.json` or `.parquet`. A URL with none is read as the media type its server names (`application/json`, `text/csv`, Parquet's), and failing that as its bytes show — Parquet's `PAR1`, JSON's opening bracket, CSV otherwise. A JSON table is a list of records, a row each, or an object holding exactly one list of records, as an API's answer `{"status": "OK", "results": [...]}` does; an object holding two is refused, naming them, and `json[read[url]]` reads it whole. An S3 object is read and written with the environment's credentials — `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` when there is one, `AWS_REGION` (`us-east-1` when unset) — signed as AWS's Signature Version 4 says; `AWS_ENDPOINT_URL` points it at an S3-compatible store (MinIO, R2) by path. Without credentials an object is read unsigned, as a public bucket's is. A URL is read, never written.

What goes wrong is the read's error value, as a missing file's is, its reason naming the path: a server that answers an error (`cannot read 'https://…': the server answered 404 Not Found`, and S3's own words after it), one that does not answer within 30 seconds (or its `timeout`), a transfer longer than ten minutes (or its `timeout`), more than 4 GB. A program's own URL, built from what a reader typed, reaches wherever it names: a served program checks it as it checks any input.

**Design Note: one `load`, wherever the data is.** Considered: an HTTP client (`get`, `post`), an `s3` module and a `parquet` module, each with its own readers. Decided: where the data is and what shape it is in are the path's, not the program's, so every reader takes a URL or an S3 object as it takes a file, and the extension names the format — one way, which a new source or format joins without a new name. A key an API asks for is part of where the data is, so a reader sends headers with its URL (`headers = map[…]`, Vijay, 2026-10-01), and so is the question a URL asks (`query = map[…]`, 2026-10-01); other methods, a body, a form and files are `request`'s (### Requests).

## JSON

| Function | Description | Example |
|----------|-------------|---|
| `json[str]` | Parse JSON string into a Crow value (error-value on failure). Indexing the value reads its elements; a parse that failed, indexed, hands its error on, and an element that is JSON's `null` — or one the value lacks: past a list's end, a negative position, a key an object does not hold — is an error value of reason `null` (kind `'parse'`), in both modes | [JSON](../by-example/json.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) |
| `encode[value]` | Encode a Crow value as a JSON string: a record as an object of its fields, a tensor's whole numbers as ints (a float alone keeps its point: `encode[2.0]` is `2.0`), a NaN or infinite float as `null`, an error value as `null`; a function, a frame and a sparse matrix have no JSON and are refused. (The interpreter had refused to encode a record, and the compiled program did not build one.) | [JSON](../by-example/json.md) · [Web: responses and submodules](../by-example/web-more.md) |

A parsed value is JSON in both modes, held as it was parsed: `type` says what it holds — `map`, `array`, `str`, `int`, `float`, `bool`, or `error` for `null`. It shows as a Crow value of that kind would (`[1, 2.0]` shows as `[1, 2]`), encodes back as that value would (`[1,2]`), and a parsed object answers `keys`, `values` and `has`, in both modes (compiled, those did not build). A field a parsed object lacks, `j.missing`, is JSON's `null`: its `.ok` is false and its `.reason` is `null`. (The interpreter had converted a parsed value to lists, maps and tensors: an item past a parsed list's end, or a field a parsed object lacked, stopped the program where the compiled one read `null`.)

A parsed value is dynamic until the program reads it. In text it shows as the one display shows it — `` 'name: `j.name`' `` is `name: crow`, a list or a map as it prints, JSON's `null` as `error: null` — as a dynamic cell of a frame does (compiled, text had shown with JSON's quotes, `name: "crow"`). `int[…]`, `float[…]` and `str[…]` read it as one type; a loop walks a parsed list's items (`for x in j.items`), each a parsed value; and the builtins that navigate one take it as it is — indexing, `.field`, `len`, `keys`, `values`, `has`, `encode` and `type`. Compared with `eq` or `ne`, one is compared with a value of its kind, and one of another kind stops the program (see Comparison); an order asks it to be read first. A builtin that computes with one — `split`, `upper`, `sort`, `push`, `contains`, `mean` — is refused until it is read, in both modes, as arithmetic on one is: `upper[j.name]` is `upper[] takes text, got a JSON value — a value whose type the program does not know is read as one first: int[…], float[…] or str[…], a list item by item (for x in …)`, and `upper[str[j.name]]` reads it. A list's items are read one by one: `names = for x in j.names / str[x]` is a list of text. (The interpreter had run those builtins on the value it parsed to, where the compiled program did not build them, and a loop over a parsed list did not build compiled.) Parsed JSON is dynamic data — the map homogeneity rule applies to maps you construct, not to values parsed from JSON (open-schema data is exactly what `json[]` is for). An element that is JSON's `null` is an error-value with `ok: false` and `reason: 'null'`, which auto-propagates through the error-value system (section 10).

```
items = json['[1, 2, 3]']
log[items[0]]              --- 1

n = json['null']
log[n.ok]                  --- false
log[n.reason]              --- 'null'

s = encode[items]
log[s]                     --- [1,2,3]
```

## System

| Function | Description | Example |
|----------|-------------|---|
| `rand[]` | Random float in [0, 1) from the program's RNG; `seed[n]` (math) makes every draw repeatable, in both modes | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `rand[n]` | Random int between 0 and n-1 (n must be positive) | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `rand[min, max]` | Random int between min and max (inclusive) | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `time[]` | Current unix timestamp as float (seconds) | [Frames: `use frame`](../by-example/frames.md) · [Moments and freshness](../by-example/moments.md) · [The system: shell, environment, time, arguments](../by-example/system.md) |
| `sleep[ms]` | Sleep for ms milliseconds (int) or seconds (float); a negative time stops the program (in both modes it had wrapped to a sleep without end, or panicked) | [The system: shell, environment, time, arguments](../by-example/system.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) |
| `env[name]` | Get environment variable (error-value if not set) | [The system: shell, environment, time, arguments](../by-example/system.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `env[name, default]` | Get environment variable, return default if not set — as text, as the variable is (`env['PORT', 3000]` is `'3000'` when `PORT` is unset; the interpreter had answered the int) | [The system: shell, environment, time, arguments](../by-example/system.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `exit[]` | Exit with code 0 | [The system: shell, environment, time, arguments](../by-example/system.md) |
| `exit[code]` | Exit with given code | [The system: shell, environment, time, arguments](../by-example/system.md) |

## Shell

| Function | Description | Example |
|----------|-------------|---|
| `shell[cmd]` | Run a shell command; returns a record with `status`, `stdout`, `stderr` (error-value with kind `'shell'` only if the command cannot start) | [The system: shell, environment, time, arguments](../by-example/system.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |

```
r = shell['echo hello']
log[r.stdout]    --- 'hello'
log[r.status]    --- 0
```

## Requests

| Function | Description | Example |
|----------|-------------|---|
| `request[url]` | An HTTP GET; returns a record with `status`, `body` and `headers` (their names in lower case), whatever the status (error-value with kind `'http'` only when no answer came) | [Web: responses and submodules](../by-example/web-more.md) |
| `request[url, method]` | A request without a body: `'DELETE'`, `'HEAD'`, `'OPTIONS'` or `'GET'` | [Web: responses and submodules](../by-example/web-more.md) |
| `request[url, method, body]` | A request with a body, as text: `'POST'`, `'PUT'`, `'PATCH'` or `'DELETE'` | [Web: responses and submodules](../by-example/web-more.md) |

Each takes these options by name, any of them:

| Option | What it sends | Example |
|--------|---------------|---|
| `headers = map[…]` | Its headers: an `authorization`, a `content-type` naming the body's type |  |
| `query = map[…]` | Pairs added to the URL, each name and value URL-encoded, as a reader's are (#### Files, URLs and S3) | [Databases: `use db`](../by-example/db.md) · [Web: responses and submodules](../by-example/web-more.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `form = map[…]` | A form, as a browser sends one: `application/x-www-form-urlencoded`, its fields URL-encoded | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `files = map[field = path]` | Files with the form, as `multipart/form-data`: each file's bytes under its field, named by its file name, typed by its extension (`.csv` as `text/csv`, `.png` as `image/png`, any other as `application/octet-stream`), and the form's fields as text beside them |  |
| `save = path` | Where a success's bytes go: written to the file as they came — an image, an archive — when the status is a success (2xx), the record's `body` then `''`; any other answer's body stays in `body`, saying why, and the file is not written | [Frames: `use frame`](../by-example/frames.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `timeout = seconds` | How long it waits for its answer: unless told, thirty seconds to connect and ten minutes in all (`0` is as none) |  |

```
order = map[symbol = 'AAPL', side = 'buy']
r = request['https://api.example.com/orders', 'POST', encode[order], headers = map[authorization = 'Bearer k']]
if r.status eq 201
    log['placed: `r.body`']
found = request['https://api.example.com/search', query = map[q = 'crow lang'], timeout = 5]
signed = request['https://example.com/login', 'POST', form = map[user = 'ada', password = pw]]
sent = request['https://example.com/upload', 'POST', form = map[title = 'July'], files = map[report = 'july.csv']]
got = request['https://example.com/logo.png', save = 'logo.png']
```

A request is answered, whatever its status: a 404 or a 500 is the record's `status`, read as `shell`'s is, and only a request that got no answer is an error value, of kind `'http'` — a server that does not answer (`cannot request 'https://…': …`), a URL that is none, a server that does not answer in time (`cannot request 'https://…': no answer in 5 s`), a URL that is none, a method that is not one (`'FETCH' is not an HTTP method — GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS`), a body, a form or files on a method that sends none (`a GET request sends no body — send it with POST, PUT or PATCH`; a form's fields can go as `query = map[…]` instead), a body beside a form or files (`a request sends a body, or a form and its files — not both`), a timeout below 0, a file to send that cannot be read and one to `save` that cannot be written. The method is a word in either case (`'post'` is `'POST'`). A body that is JSON — `encode[…]`, which opens with `{` or `[` — goes as `application/json`, any other as text, a form and files as their own types say, unless a `content-type` header names the type. A map's values go as text, each as it shows. A body answered is text — its bytes read as UTF-8, any that are not as `�` — so bytes that are not text are `save`d, not held. `request` is an effect: in a handler, one whose answer is used runs when asked, and one whose answer is not — a webhook told what happened — runs when the handler commits, so a handler that stops sends nothing (A handler is a transaction). A rehearsal (`crow check`) records a request instead of sending it. Both modes send it through the one client the readers use (`crow_runtime::sources`). `read` and `load` stay the readers: they want data, so a server's refusal is their error, where a request's record says it.

**Design Note: one `request`, its method named.** Considered: a word for each method — `get`, `post`, `put`, `delete` — as many HTTP clients spell them. Decided: one `request` with its method as text, as `route['/up', 'POST', handler]` names the server's (Vijay, 2026-10-01): `get` would be a second `read`, `put` is the name a stream's feed is handed (`fn numbered[put, n]`), and `delete` would stand beside the map's `del`; a method as text reaches PATCH, HEAD and OPTIONS with no more names.

## Hashing

| Function | Description | Example |
|----------|-------------|---|
| `hash[s, algo]` | Hash string with algorithm. Supported: `'sha256'`, `'sha512'`, `'md5'`, and `'password'` | [The system: shell, environment, time, arguments](../by-example/system.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `hash[s, stored]` | Hash `s` the way the password hash `stored` was made — its algorithm, cost and salt — so `hash[attempt, stored] eq stored` checks a password | [The system: shell, environment, time, arguments](../by-example/system.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |

```
h = hash['hello', 'sha256']
log[h]    --- '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824'
```

`hash[pw, 'password']` is for storing a password: a salted, deliberately slow hash (Argon2id), written as one string that carries its algorithm, cost and salt (`$argon2id$v=19$m=19456,t=2,p=1$…`). Its salt is random, so hashing the same password twice gives two different strings; checking a password therefore hashes the attempt with the stored string in place of the algorithm and compares the two. The fast hashes (`sha256`, `sha512`, `md5`) are for fingerprints and checksums, never for passwords: they can be guessed at billions a second.

## Encodings

| Function | Description | Example |
|----------|-------------|---|
| `encode[text, scheme]` | Text written so something else can carry it: `'base64'` (the standard alphabet, padded with `=`), as an `authorization: Basic …` header or a binary field wants it; `'url'`, one component of a URL — every byte but a letter, a digit and `-` `.` `_` `~` written `%XX`, so a space is `%20` and `&`, `=`, `/` and `?` stand for themselves no more | [JSON](../by-example/json.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `decode[text, scheme]` | The text `encode[…, scheme]` was given: base64 in the standard or the URL-safe alphabet, padded or not, whitespace between skipped; a URL component's `%XX` each the byte it stands for (a `+` stays a `+`). Error-value with kind `'parse'` when the text is not in the scheme or decodes to bytes that are not text | [JSON](../by-example/json.md) |

```
pair = 'ada:secret'
coded = encode[pair, 'base64']
log['Basic `coded`']               --- 'Basic YWRhOnNlY3JldA=='
log[decode[coded, 'base64']]       --- 'ada:secret'
log[encode['crow & more', 'url']]  --- 'crow%20%26%20more'
bad = decode['Zm9v!', 'base64']
log[bad.reason]                    --- 'Zm9v!' is not base64: '!' is not in its alphabet
```

A scheme neither knows is refused before the program runs when the call writes it (`encode[] knows no scheme 'hex' — base64 or url`); one the program computes stops `encode`, and is `decode`'s error value, as text it cannot read is. A query and a form are encoded by `request` and the readers themselves (`query = map[…]`, `form = map[…]`), so a program rarely encodes a URL's piece by hand.

**Design Note: one `encode`, its scheme named.** Considered: a builtin for each encoding — `base64`, `unbase64`, `urlencode` — as most libraries spell them. Decided: `encode` writes a value as text something else carries, and `decode` reads it back: JSON with no scheme, base64 or a URL's component with one, as `hash` names its algorithm (2026-10-01, for a Basic authorization). A new scheme joins without a new name. JSON is read back by `json[…]`, which parses text into a value; `decode` always takes a scheme, since what it gives back is text.

## Regex

| Function | Description | Example |
|----------|-------------|---|
| `matches[s, pattern]` | Check if string matches regex pattern (returns bool); an invalid pattern is an error value of kind `'regex'` | [Regular expressions](../by-example/regex.md) |
| `find[s, pattern]` | The first match, as a record whose `groups` holds the whole match followed by each capture group. No match is an error value (`.ok` false, reason `no match`), as `find[arr, pred]` answers; an invalid pattern is one with kind `'regex'` | [Regular expressions](../by-example/regex.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `sub[s, pattern, replacement]` | Replace all regex matches with replacement string; an invalid pattern is an error value of kind `'regex'` | [Regular expressions](../by-example/regex.md) |

```
matches['hello', '[a-z]+']       --- true

m = find['2024-01-15', '([0-9]+)-([0-9]+)-([0-9]+)']
log[m.ok]        --- true
log[m.groups]    --- ['2024-01-15', '2024', '01', '15']
log[m.groups[1]] --- 2024

sub['hello world', 'world', 'crow']  --- 'hello crow'
```

## Moments

A moment is a number: seconds since the Unix epoch, as a float. Comparison
and arithmetic therefore come free — `end - start` *is* elapsed seconds,
`t lt open - 10.0` *is* a staleness check — and no separate duration or
date type exists.

| Function | Description | Example |
|----------|-------------|---|
| `now[]` | The current moment (epoch seconds) — in a live program, the moment the current event began | [Moments and freshness](../by-example/moments.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) |
| `moment[y, m, d, hour = 0, minute = 0, second = 0, tz = 'UTC']` | Build a moment from calendar components in a named timezone | [Moments and freshness](../by-example/moments.md) |
| `calendar[t, tz = 'UTC']` | Calendar components of a moment: `year month day hour minute second weekday` (weekday 1 = Monday) | [Moments and freshness](../by-example/moments.md) |
| `parse_time[text, tz = 'UTC']` | Parse RFC3339, or `'YYYY-MM-DD[ HH:MM[:SS]]'` read in the named zone | [Moments and freshness](../by-example/moments.md) |
| `fmt[t, time = 'iso' \| 'date' \| 'time' \| 'datetime', tz = 'UTC']` | Render a moment; a style or a zone it does not know is an error value of kind `'time'` (compiled, it had been the text `[fmt: …]`), so the call can fail as any moment builtin can | [Formatting](../by-example/formatting.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |

```
open_bell = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']
log[fmt[open_bell, time = 'datetime', tz = 'America/New_York']]   --- 2026-07-14 09:30:00
log[fmt[open_bell, time = 'iso']]                                 --- 2026-07-14T13:30:00Z

--- subtraction is elapsed seconds; comparison is comparison
if print_time lt open_bell - 10.0
    log['stale print — not driving price']

c = calendar[open_bell, tz = 'UTC']
log[c.hour]                                                       --- 13
```

**One moment per event.** In a script `now[]` is the clock at the call. In a live program it is the moment the current event began — a click or a change from the page, a render, a `refresh` tick, a `later` schedule — everywhere inside it: a handler and every helper it calls, a render and every view and derivation in it. Time advances between events, never inside one, so `base, anchor = head, now[]` reads one clock twice and gets one answer, and two rows a handler stamps with `now[]` share a stamp. In the test harness the clock is the test's: it starts at the moment the first `app[…]` opened and moves only when the test moves it with `change[a, 'now', t]`.

Timezones are IANA names (`'America/New_York'`, `'Europe/London'`, `'UTC'`)
and are **always explicit** — the default everywhere is UTC. Invalid dates,
unknown zones, DST gaps, and unparseable strings return error values with
`kind = 'time'`.

## Freshness: captured

Values read from the outside world remember when they arrived:

| Function | Description | Example |
|----------|-------------|---|
| `captured[x]` | The moment `x` was read from outside (a moment, so ordinary arithmetic applies). A value computed here has no capture moment: the answer is an error value with `kind = 'no_capture'` | [Moments and freshness](../by-example/moments.md) |

```
px = query[conn, 'SELECT last FROM ticks WHERE symbol = ?', [sym]]

if now[] - captured[px] gt 10.0
    return error['price is stale — not trading on it', 'stale']

place_order[sym, px.last[0]]
```

Readers stamp: `query`, `read`, `input`, `ls`, `env`, `shell`, `csv`,
`lines`, `load` (frame and math), `fetch`, `recv`. Only a successful read
is stamped — `captured[]` of a failed read propagates the read's own
error. Reading a field of a captured value keeps the capture (`px.last` is
as old as `px`); **arithmetic does not** — a computed number is a new value, not a
remembered one. `crow check` covers that gap statically: a reader value
that flows through computation into a decision without any freshness guard
is reported (see `crow-check.md`).

## Design Note: Freshness Lives at the Boundary

**What was considered:** propagating capture time through every operation
so a derived value is as old as its oldest input — provenance as a sibling
of the error system.

**Why it was rejected:** compiled Crow stores plain numbers as bare machine
values with nowhere to keep a header; carrying age through arithmetic would
mean boxing every scalar, forfeiting exactly what compiled mode is for. The
interpreter could do it (measured: ~4-5% on a scalar-heavy loop), but the
two modes must agree, so the feature would have been interpreter-only.

**Why this design:** freshness is tracked where data enters and checked
before it is trusted — which is what every system that got burned by stale
data already does: HTTP's `Age` header, ROS message stamps, dbt source
freshness, stream-processing watermarks. The flow-through-computation case
moves to `crow check`, which is the same answer compiled languages reached
for taint tracking: runtime labels stayed in interpreted languages, while
compiled ones (Java's Checker Framework, FlowDroid) moved propagation into
static analysis. Deferred: declarative thresholds on the reader itself
(dbt's `warn_after` shape), which would fit a future `store` declaration.

## Design Note: Adopting External Ground Truth

A remote system's state cannot be made atomic by any language. Observations
of it are delayed and transiently wrong — a position row reads flat for a
moment while a fill settles, an order looks missing before it appears. The
doctrine: **never act on an external absence until it has been stably
absent.** A grace window is freshness inverted — where `captured` asks "is
this recent enough to trust?", adopting an absence asks "has this been
true long enough to believe?":

```
if flat[position] and now[] - first_seen_flat gt 15.0
    mark_closed[sym]
```

The authoritative source is the external system, not a cached row derived
from it: derive closure from order status and fills, not from a position
snapshot. No primitive can enforce this — it is a modeling rule, and the
reason it is written down here is that every system that skipped it
rediscovered it as an incident.

## Design Note: A Moment Is a Number

**What was considered:** the previous design — `now[]`/`date[]` returning
component maps and `elapsed[a, b]` returning a duration map. It could not
express a named timezone, parse a timestamp, add a duration, or compare two
moments directly; worse, `now[]` built its components in *machine-local*
time while `date[]` timestamped at UTC midnight, so mixed arithmetic was
already silently wrong by hours.

**Why this design:** representing a moment as epoch seconds makes the
number line do the work — comparison, subtraction, and "ten seconds before
the bell" are ordinary arithmetic on ordinary floats, with no new type, no
operator overloads, and no duration vocabulary. Calendars and zones are
where the real complexity lives, so they are named-argument functions
(`moment`, `calendar`) rather than hidden defaults. Machine-local time is
absent by design: "correct on my laptop, wrong on the UTC server" is the
classic deploy-day bug, and a language that makes the timezone explicit
never has it. **Deleted in exchange:** `elapsed` (subtraction), `date`
(subsumed by `moment`), `today` (`fmt[now[], time = 'date']`) — all three
now raise teaching errors naming their replacement.



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
