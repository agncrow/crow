# Core builtins

Always available, no import. The tables below are the spec's, with the book pages whose verified code uses each name.


The functions available in every file without an import. These names are reserved — a definition may not reuse them (see Design Note: Names Are Never Ambiguous). Module functions (`use math`, `use frame`, …) are in section 13.


## Core

| Function | Description | Example |
|----------|-------------|---|
| `log[value]` | Print a value to stdout | [Arrays](../by-example/arrays.md) · [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Files and directories](../by-example/files.md) · [Formatting](../by-example/formatting.md) · [Frames: `use frame`](../by-example/frames.md) · [Automatic differentiation](../by-example/grad.md) · [Structured files: `use io`](../by-example/io.md) · [JSON](../by-example/json.md) · [Maps](../by-example/maps.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Regular expressions](../by-example/regex.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Hello, World!](../ch01-02-hello-world.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Variables](../ch03-01-variables.md) · [Data Types](../ch03-02-data-types.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Comments](../ch03-04-comments.md) · [Control Flow](../ch03-05-control-flow.md) · [Defining and Calling Functions](../ch04-01-defining-functions.md) · [Named Arguments](../ch04-02-named-arguments.md) · [Function Scope Isolation](../ch04-03-scope-isolation.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Structs Are Records](../ch05-03-structs.md) · [for as an Expression](../ch06-01-for-as-expression.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) · [Importing with use](../ch08-01-importing.md) · [Aliased Imports](../ch08-02-aliased-imports.md) · [Writing Your Own Modules](../ch08-04-writing-modules.md) · [Testing](../ch09-00-testing.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |
| `range[stop]` | Array of ints from 0 to stop (exclusive) | [Arrays](../by-example/arrays.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [for as an Expression](../ch06-01-for-as-expression.md) |
| `range[start, stop]` | Array of ints from start to stop (exclusive) | [Arrays](../by-example/arrays.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [for as an Expression](../ch06-01-for-as-expression.md) |
| `range[start, stop, step]` | Array of ints with custom step | [Arrays](../by-example/arrays.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [for as an Expression](../ch06-01-for-as-expression.md) |
| `map[]` | Create an empty map | [Maps](../by-example/maps.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Testing](../ch09-00-testing.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
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
| `round[n]` | Round to nearest int | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Numbers and conversion](../by-example/numbers.md) |
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
| `join[arr, sep]` | Join array elements into string | [Frames: `use frame`](../by-example/frames.md) · [Strings](../by-example/strings.md) · [Strings and Interpolation](../ch03-03-strings.md) |
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
| `int[value]` | Convert to int (error-value on failure) | [JSON](../by-example/json.md) · [Numbers and conversion](../by-example/numbers.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) · [Data Types](../ch03-02-data-types.md) · [Every Value Has Status](../ch07-01-every-value-has-status.md) · [Propagation](../ch07-02-propagation.md) · [Errors Have No Truth Value](../ch07-03-no-truth-value.md) · [Handling Errors Explicitly](../ch07-04-explicit-handling.md) |
| `float[value]` | Convert to float (error-value on failure) | [Numbers and conversion](../by-example/numbers.md) · [Data Types](../ch03-02-data-types.md) |
| `str[value]` | Convert to string | [Numbers and conversion](../by-example/numbers.md) · [Data Types](../ch03-02-data-types.md) |
| `type[value]` | Type name as string: `'int'`, `'float'`, `'str'`, `'bool'`, `'array'`, `'map'`, `'fn'`, `'none'`, `'error'`, or struct name | [Numbers and conversion](../by-example/numbers.md) |

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
| `len[x]` | Length of array, string, or map | [Databases: `use db`](../by-example/db.md) · [Documentation: `use doc`](../by-example/doc.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Maps](../by-example/maps.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Strings](../by-example/strings.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Strings and Interpolation](../ch03-03-strings.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `push[arr, value]` | Return new array with value appended | [Arrays](../by-example/arrays.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `pop[arr]` | Return new array without last element | [Arrays](../by-example/arrays.md) |
| `concat[a, b]` | Return new array merging two arrays | [Arrays](../by-example/arrays.md) · [Math: tensors](../by-example/math.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `concat[a, b, axis]` | Concatenate two tensors along an axis | [Arrays](../by-example/arrays.md) · [Math: tensors](../by-example/math.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `sort[arr]` | Return new sorted array | [Arrays](../by-example/arrays.md) · [Frames: `use frame`](../by-example/frames.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Views and Live Updates](../ch13-03-views.md) |
| `reverse[arr]` | Return new reversed array | [Arrays](../by-example/arrays.md) · [Strings](../by-example/strings.md) |
| `contains[arr, value]` | Check if array contains value | [Arrays](../by-example/arrays.md) · [Documentation: `use doc`](../by-example/doc.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `any[arr, pred]` | True if any element passes the predicate (parallel, early exit) | [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `all[arr, pred]` | True if every element passes (parallel, early exit) | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `find[arr, pred]` | First element passing the predicate; error-value if none | [Regular expressions](../by-example/regex.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `slice[arr, start, end]` | Extract subarray | [Arrays](../by-example/arrays.md) · [Strings and Interpolation](../ch03-03-strings.md) |

## Map Operations

| Function | Description | Example |
|----------|-------------|---|
| `keys[m]` | Array of map keys | [Maps](../by-example/maps.md) · [Maps Are Dictionaries](../ch05-02-maps.md) |
| `values[m]` | Array of map values | [Maps](../by-example/maps.md) |
| `has[m, key]` | Check if map contains key | [Maps](../by-example/maps.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Testing](../ch09-00-testing.md) |
| `set[m, key, value]` | Return new map with entry added/updated | [Maps](../by-example/maps.md) · [Maps Are Dictionaries](../ch05-02-maps.md) · [Testing](../ch09-00-testing.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `del[m, key]` | Return new map with entry removed | [Maps](../by-example/maps.md) |

**All builtins are immutable.** No builtin modifies its arguments. To update a variable, reassign: `nums = push[nums, 4]`. The transpiler optimizes `x = f[x]` patterns into in-place operations automatically.

## I/O

| Function | Description | Example |
|----------|-------------|---|
| `read[path]` | Read file contents (error-value on failure) | [Files and directories](../by-example/files.md) · [Moments and freshness](../by-example/moments.md) · [View Blocks](../ch12-01-views.md) |
| `write[path, content]` | Write string to file (error-value on failure) | [Databases: `use db`](../by-example/db.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Moments and freshness](../by-example/moments.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) |
| `exists[path]` | Check if file/directory exists (returns bool) | [Files and directories](../by-example/files.md) |
| `mkdir[path]` | Create directory (and parents). Error-value on failure | [Databases: `use db`](../by-example/db.md) · [Files and directories](../by-example/files.md) |
| `ls[path]` | List directory entries as sorted array of strings. Error-value on failure | [Files and directories](../by-example/files.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `rm[path]` | Remove file or directory (recursive). Error-value on failure | [Databases: `use db`](../by-example/db.md) · [Files and directories](../by-example/files.md) · [Frames: `use frame`](../by-example/frames.md) · [Structured files: `use io`](../by-example/io.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Moments and freshness](../by-example/moments.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) |
| `mv[src, dst]` | Move/rename file or directory. Error-value on failure | [Files and directories](../by-example/files.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) |
| `cp[src, dst]` | Copy file. Error-value on failure | [Files and directories](../by-example/files.md) |
| `args[]` | CLI arguments as array of strings | [The system: shell, environment, time, arguments](../by-example/system.md) |
| `input[]` | Read a line from stdin (error-value on failure) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `input[prompt]` | Print prompt, then read a line (error-value on failure) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |

## JSON

| Function | Description | Example |
|----------|-------------|---|
| `json[str]` | Parse JSON string into a Crow value (error-value on failure) | [JSON](../by-example/json.md) · [Web: a JSON route](../by-example/web.md) |
| `encode[value]` | Encode a Crow value as a JSON string | [JSON](../by-example/json.md) |

JSON type mapping: objects → maps, arrays → arrays, strings → str, numbers → int/float, booleans → bool, `null` → error-value. Parsed JSON is dynamic data — the map homogeneity rule applies to maps you construct, not to values parsed from JSON (open-schema data is exactly what `json[]` is for). JSON `null` maps to an error-value with `ok: false` and `reason: 'null'`, which auto-propagates through the error-value system (section 10).

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
| `rand[]` | Random float between 0.0 and 1.0 | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `rand[n]` | Random int between 0 and n-1 | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `rand[min, max]` | Random int between min and max (inclusive) | [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `time[]` | Current unix timestamp as float (seconds) | [Frames: `use frame`](../by-example/frames.md) · [Moments and freshness](../by-example/moments.md) · [The system: shell, environment, time, arguments](../by-example/system.md) |
| `sleep[ms]` | Sleep for ms milliseconds (int) or seconds (float) | [The system: shell, environment, time, arguments](../by-example/system.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) |
| `env[name]` | Get environment variable (error-value if not set) | [The system: shell, environment, time, arguments](../by-example/system.md) |
| `env[name, default]` | Get environment variable, return default if not set | [The system: shell, environment, time, arguments](../by-example/system.md) |
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

## Hashing

| Function | Description | Example |
|----------|-------------|---|
| `hash[s, algo]` | Hash string with algorithm. Supported: `'sha256'`, `'sha512'`, `'md5'` | [The system: shell, environment, time, arguments](../by-example/system.md) |

```
h = hash['hello', 'sha256']
log[h]    --- '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824'
```

## Regex

| Function | Description | Example |
|----------|-------------|---|
| `matches[s, pattern]` | Check if string matches regex pattern (returns bool) | [Regular expressions](../by-example/regex.md) |
| `find[s, pattern]` | Find first match with capture groups. Returns map with `ok` (bool) and `groups` (array) | [Regular expressions](../by-example/regex.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `sub[s, pattern, replacement]` | Replace all regex matches with replacement string | [Regular expressions](../by-example/regex.md) |

```
matches['hello', '[a-z]+']       --- true

m = find['2024-01-15', '([0-9]+)-([0-9]+)-([0-9]+)']
log[m.ok]        --- true
log[m.groups]    --- ['2024-01-15', '2024', '01', '15']

sub['hello world', 'world', 'crow']  --- 'hello crow'
```

## Moments

A moment is a number: seconds since the Unix epoch, as a float. Comparison
and arithmetic therefore come free — `end - start` *is* elapsed seconds,
`t lt open - 10.0` *is* a staleness check — and no separate duration or
date type exists.

| Function | Description | Example |
|----------|-------------|---|
| `now[]` | The current moment (epoch seconds) | [Moments and freshness](../by-example/moments.md) |
| `moment[y, m, d, hour = 0, minute = 0, second = 0, tz = 'UTC']` | Build a moment from calendar components in a named timezone | [Moments and freshness](../by-example/moments.md) |
| `calendar[t, tz = 'UTC']` | Calendar components of a moment: `year month day hour minute second weekday` (weekday 1 = Monday) | [Moments and freshness](../by-example/moments.md) |
| `parse_time[text, tz = 'UTC']` | Parse RFC3339, or `'YYYY-MM-DD[ HH:MM[:SS]]'` read in the named zone | [Moments and freshness](../by-example/moments.md) |
| `fmt[t, time = 'iso' \| 'date' \| 'time' \| 'datetime', tz = 'UTC']` | Render a moment | [Formatting](../by-example/formatting.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Moments and freshness](../by-example/moments.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Statistics: `use math.stats`](../by-example/stats.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |

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
