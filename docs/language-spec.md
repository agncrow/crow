# Crow Language Specification

## 1. Variables

### Declaration & Assignment

Variables are declared by simply assigning a value. No keywords needed.

```
name = 'crow'
age = 3
pi = 3.14
active = true
```

### Reassignment

Variables can be reassigned, but the type cannot change. `int` and `float` are compatible with each other.

```
x = 10
x = 20          --- ok, same type ---
x = 3.14        --- ok, int/float compatible ---
x = 'hello'     --- error: cannot reassign from int to string ---
```

**A variable that is ever assigned a float is a float for its whole scope.** Assigning an int to it stores the float value (`x = 1` above stores `1.0`). Note for interpreted mode: the widening applies from the first float assignment onward, so an int division performed *before* any float assignment may differ from compiled mode, where the variable is float from its declaration — don't depend on int division of variables that later hold floats.

### Types

Crow is statically typed with full type inference — you never write type annotations, but every variable has a concrete type determined at compile time. The transpiler infers types from usage; if a type cannot be determined, the program is invalid.

The built-in types are:

| Type   | Example              |
|--------|----------------------|
| int    | `42`, `-7`, `0`      |
| float  | `3.14`, `-0.5`       |
| str    | `'hello'`, `'crow'`  |
| bool   | `true`, `false`      |
| array  | `[1, 2, 3]`, `[[1, 2], [3, 4]]` |
| map    | `map[home = 78701, office = 78702]` |

Arrays are homogeneous — all elements must be the same type. Use maps for mixed-type named data.

```
[1, 2, 3]                    --- int array ---
['a', 'b', 'c']              --- str array ---
[[1, 2], [3, 4]]             --- 2D int array ---
[1, 'hello', true]           --- error: mixed types ---
```

### String Escape Sequences

Strings support escape sequences with backslash:

| Escape | Character |
|--------|-----------|
| `\n` | Newline |
| `\t` | Tab |
| `\\` | Literal backslash |
| `\'` | Literal single quote |
| `` \` `` | Literal backtick (not interpolation) |

```
log['line1\nline2']
log['col1\tcol2']
log['it\'s here']
log['a \`code span\` in prose']    --- prints: a `code span` in prose ---
```

Comments and string literals accept any Unicode text. Identifiers are ASCII — a deliberate restriction that keeps the grammar and tooling simple.

### String Interpolation

Backticks delimit an expression inside a string — like a markdown code span. Everything else in a string is inert text:

```
name = 'crow'
greeting = 'hello, `name`'
result = '1 + 2 = `1 + 2`'
li['`item.name` — qty `item.qty`']
log['best: `max[scores]`']
cfg = json['`"a": 1, "hosts": ["x"]`']   --- braces and brackets: plain text ---
m = matches[s, '[a-z]`2,4`']             --- regex: fully raw ---
```

Rules:
- `` `expr` `` evaluates any Crow expression and splices its value.
- Inside an interpolation you are still inside the string — quotes stay escaped: `'up: `upper[\'hi\']`'`.
- An unterminated or empty backtick span is an error (never silent).
- For repeated formatting, name it: `fn money[x]` + `'P&L: `money[pnl]`'` (see Formatting).

#### Design Note: Strings Are Inert; Backticks Delimit Code

**What was considered:** brace interpolation `{expr}` (the original design), `\[expr]` (Swift-style escape activation), bare `` `name `` (marker without terminator).

**Why paired backticks won:** any *bare* delimiter collides with real text — `{}` silently corrupted regex quantifiers (`'[0-9]{4}'` interpolated the `{4}`), bare `[]` would have destroyed regex character classes and `[INFO]`-style prose, and bare `` `name `` could not delimit (`` `names ``? `` `name.csv ``?) or express `a + b`. Pairing the marker gives full expressions, zero termination rules, and one form. Strings become inert by default — JSON, regex, CSS, and prose need no escapes — and interpolation is explicitly activated, unshifted, with the one notation everyone already reads as "this is code". The design lineage (braces → escape-activation → bare marker → paired) is preserved in docs/design-review.md.

### Multiple Assignment

Multiple variables can be assigned in a single line.

```
x, y, z = 1, 2, 3
```

### Swap

```
a, b = b, a
```

### Scope

Variables are scoped to the block they are defined in. Inner blocks can access outer variables — but **function boundaries are opaque**: module variables are not visible inside functions (see Function Scope, §3).

```
x = 10
if true
    y = 20
    log[x]    --- 10 — accessible: blocks see outer variables ---

log[y]        --- error — y is not in scope ---
```

## 2. Blocks

Blocks are defined by indentation. A new block starts after a statement (e.g. `if`, `fn`, `for`) and ends when the indentation returns to the previous level.

```
if x gt 5
    log['big']
    log['done']

next_thing[]
```

Blocks can be nested.

```
if x gt 5
    if y gt 10
        log['both big']

next_thing[]
```

## 3. Functions

### Declaration

Functions are declared with `fn`. The last expression in the body is the implicit return value. Use explicit `return` for early returns.

```
fn add[a, b]
    a + b                    --- implicit return ---

fn greet[name]
    log['hello, `name`']

fn safe_div[a, b]
    if b eq 0
        return 0             --- early return ---
    a / b                    --- implicit return ---
```

### Named Arguments with Defaults

Parameters with `= value` are named arguments with default values. Named arguments must come after all positional arguments. Default values must be literals (`int`, `float`, `str`, `bool`) — expressions are not allowed, so evaluation order and scope never come into question.

```
fn train[data, lr = 0.01, epochs = 100]
    for i in range[epochs]
        update[data, lr]
```

Callers can omit named arguments (uses default), provide them by name, or by position:

```
train[my_data]                     --- uses defaults ---
train[my_data, lr = 0.001]         --- override one ---
train[my_data, 0.001, 50]          --- positional also works ---
```

Named arguments are required for axis parameters in math operations:

```
s = sum[x, axis = 0]              --- sum along axis 0 ---
mx = max[x, axis = 1]             --- max along axis 1 ---
```

Named arguments are resolved at compile time (a lowering pass rewrites them to positional form before execution), so they work only when the callee is a directly named function, struct, builtin with named parameters, or ui element. A function received as a value (`fn apply[f, x]` calling `f[...]`) must be called with positional arguments.

### Calling

Functions are called using `[]`.

```
greet['crow']
result = add[3, 4]
```

### Array Access

Arrays are functions from index to value. Negative indices count from the end: `-1` is the last element, `-2` is second-to-last, etc.

```
numbers = [1, 2, 3]
numbers[0]               --- 1 ---
numbers[-1]              --- 3 (last) ---
numbers[-2]              --- 2 (second-to-last) ---
```

Negative indices also work for assignment:

```
numbers[-1] = 99         --- sets last element ---
```

Out-of-bounds indices (positive or negative) are a runtime error.

### Multidimensional Arrays

Nested brackets create multidimensional arrays. All inner arrays must be the same length.

```
m = [[1, 2, 3],
     [4, 5, 6]]              --- 2x3 array ---

m[0]                          --- [1, 2, 3] (first row) ---
m[1][2]                       --- 6 ---

--- creation by shape ---
z = zeros[3, 4]               --- 3x4 of zeros ---
r = rand[2, 3]                --- 2x3 random ---

--- growing (immutable: returns a new tensor) ---
m2 = push[m, [7, 8, 9]]       --- append one row (shape-checked) ---
c = concat[m, m2, 0]          --- concatenate along an axis ---

--- errors: shape and type are the tensor's contract ---
push[m, [1, 2]]               --- error: row length 2 vs rows of length 3 ---
push[m, 'x']                  --- error: arrays are homogeneous ---
[[1, 2], [3]]                 --- error: all inner arrays must be the same length ---
```

Iterating a 2-D array yields its rows; comparisons with a scalar broadcast to a boolean mask:

```
totals = for row in m
    sum[row]                  --- per-row sums ---

x = [1, 2, 3, 4]
mask = x gt 2                 --- [false, false, true, true] ---
big = (x gt 1) and (x lt 4)   --- masks combine element-wise ---
```

### String Access

Strings support indexing just like arrays. Each index returns a single-character string. Negative indices count from the end.

```
word = 'hello'
word[0]                  --- 'h' ---
word[-1]                 --- 'o' ---
word[2]                  --- 'l' ---
```

Out-of-bounds indices are a runtime error.

### Map Access

Maps use bare names as keys. Access with dot notation or string key.

```
zips = map[home = 78701, office = 78702]
zips.home                --- 78701 ---
zips['home']             --- 78701 (string key also works) ---
```

Bare-name keys are stored as strings internally. Use quoted string keys when the key is dynamic or contains special characters:

```
headers = map['content-type' = 'json']
key = 'home'
zips[key]                --- dynamic lookup ---
```

### Function Scope

A function sees its parameters, its own locals, and the program's vocabulary — other functions, structs, and builtin constants (`pi`, `e`). **Module variables are invisible inside functions.** The only way data enters a function is through its parameters; the only way it leaves is through its return value.

```
count = 5

fn build[]
    return 2 + 3          --- ok: self-contained ---

fn scaled[n]
    count = n * 2         --- ok: a fresh LOCAL named count —
    return count          ---     unrelated to the module variable ---

fn bad[x]
    return x + count      --- error: module variables are not visible
                          --- inside functions — pass 'count' as a parameter ---

count = 6                 --- ok: top-level code mutates top-level state freely ---
log[scaled[count]]        --- 12: pass it in ---
```

Because module variables were never visible, a local reusing a module name shadows nothing — it is simply a new binding. There are no `global` declarations and no scope-resolution rules to learn: inside a function, every name is a parameter, a local, or a function/struct/constant.

**Exception — views and event handlers:** in a ui program, top-level assignments become managed reactive state. **Only `view` declarations and event handlers** (functions referenced from views or `page[]`) access that state — through the state machinery, rewritten to `state.get`/`state.set`. Ordinary `fn` declarations are fully isolated even under `use ui`. That is the sanctioned mutable-state channel — owned by the runtime and synchronized, like LiveView assigns.

#### Design Note: No Ambient Mutability

**What was considered:** letting functions read (and possibly write) module variables, as in Python (`global` keyword), Julia, Go, or Lua.

**Why it was rejected:**

1. **Same reason closures were rejected.** Reading ambient state is capture — a hidden data dependency that makes a function's behavior depend on things not in its signature. Purity detection, auto-testing (`crow check`), and refactoring all rely on parameters telling the whole story.
2. **Parallel-by-default requires it.** The wave scheduler orders statements by the reads and writes it can see at each call site. An ambient read inside a function is invisible to it, so `a = f[]` and `x = 2` would race whenever `f` secretly read `x`. Immutability-by-construction is the only zero-cost fix; the alternatives are interprocedural analysis or locks.
3. **The permissive languages document the damage.** Python's `global`/`UnboundLocalError` is its most-asked scoping question; Julia re-fought its scope rules for two years; JavaScript needed strict mode to stop accidental globals. The concurrency-serious languages — Rust (no unsynchronized mutable statics), Erlang/Elixir (no shared mutable state at all) — sit exactly where Crow now sits.

**What to use instead:** parameters in, return values out. For shared mutable state across events, use the explicit managed channels: ui `state`, the database, files.

### First-Class Functions

A function name without `[]` evaluates to the function itself. Functions can be passed as arguments, stored in arrays and maps, and called later with `[]`.

```
fn double[x]
    return x * 2

fn apply[f, x]
    return f[x]

result = apply[double, 5]    --- 10 ---

ops = [double, square]
ops[0][3]                     --- 6 ---
```

### No Lambdas

Crow has no anonymous functions. Every function must be declared with `fn` and a name. To pass behavior, define a named function and pass it.

```
--- correct ---
fn double[x]
    return x * 2

result = map[items, double]

--- not supported ---
result = map[items, fn[x] x * 2]    --- no anonymous functions ---
```

This follows Crow's "one way to do things" principle — there is exactly one way to define a function.

### Map Construction

Maps are created with the `map[...]` constructor — a variadic call with named arguments. `[]` is always an empty array.

```
items = []                               --- empty array ---
data = map[]                             --- empty map ---
counts = map[crow = 3, raven = 7]        --- map with entries ---
headers = map['content-type' = 'json']   --- quoted keys for special characters ---
```

`map[key = value, ...]` desugars to a chain of `set[]` calls — it is ordinary Crow, not special syntax. For fully dynamic keys, build with `set[]`:

```
m = map[]
m = set[m, key_var, value]
```

**Maps are homogeneous: every value must have the same type.** Int and float
unify (like arrays). Mixed named data belongs in a struct — and since the value
type can be *any* type, including structs, the two compose:

```
struct Address [street, city, zip]

addresses = map[
    home = Address['12 Oak St', 'Austin', 78701],
    office = Address['500 Main St', 'Austin', 78702]
]

addresses.home.city                       --- 'Austin' ---
addresses = set[addresses, 'work', Address['9 Elm Rd', 'Dallas', 75001]]

for label in keys[addresses]
    a = addresses[label]
    log['`label`: `a.street`, `a.city`']
```

Mixed field types live inside the struct; dynamic keys live in the map. Adding
a value of a different type is an error in both modes:

```
m = map[a = 1]
m = set[m, 'b', 'x']
--- error: map values must all be the same type — this map holds int values,
--- cannot add str (use a struct for mixed named data) ---
```

#### Design Note: Maps Are Dictionaries, Not Records

**What was considered:** heterogeneous maps (`map[name = 'crow', age = 3]`) —
mixed value types per key, as in Python dicts or Elixir maps.

**Why it was rejected:** sort the use cases by whether keys are known at
compile time and whether values are mixed:

| | Homogeneous values | Mixed values |
|---|---|---|
| Keys known in code | map or struct — both fine | an anonymous struct in disguise |
| Keys are runtime data | counts, caches, headers — the true map uses | open-schema JSON — the `Json` type |

The genuinely map-shaped uses (dynamic keys) are naturally homogeneous; the
mixed-value uses have static keys — if every key gets its own type, the keys
are code, and a struct gives them typo-checking and native compiled speed.
The one dynamic-and-mixed case, open JSON, already has the `Json` type.

Every statically typed language lands here (Go's `map[K]V` vs `struct`, Rust's
`HashMap` vs `struct`); every dynamic language that conflated them grew record
types anyway (Python dataclasses, Ruby's `Data.define`, Elixir structs), and
Julia's `Dict{String, Any}` shows the performance cliff heterogeneous maps
cost a compiled language. Crow makes the idiom the rule: **heterogeneity gets
its shape from structs; keys get their freedom from maps; compose them.**

#### Design Note: No Map Literals

**What was removed:** the bare map literal `[name = 'crow']`, which shared the
array's delimiters and was distinguished only by the `=` inside.

**Why:** one bracket form with two meanings created a cascade of special cases —
the empty map needed the special form `map[]`, named arguments were ambiguous
against map literals inside call brackets, and UI attribute maps forced double
brackets (`button[[click = inc], '+']`). With the constructor: bare `[...]`
always means an array, `name = value` inside call brackets always means a named
argument, and `map[]` is just the zero-argument case of the one map constructor.
The grammar has one rule per position. Precedent: Go writes `map[string]int{...}`,
Julia uses `Dict(...)`, Python's `dict(k=v)` is this exact form.

Writing the old literal produces a helpful error:
`map literals were removed — write map[key = value, ...] instead`.

### Built-in Functions

#### Core

| Function | Description |
|----------|-------------|
| `log[value]` | Print a value to stdout |
| `range[stop]` | Array of ints from 0 to stop (exclusive) |
| `range[start, stop]` | Array of ints from start to stop (exclusive) |
| `range[start, stop, step]` | Array of ints with custom step |
| `map[]` | Create an empty map |
| `assert[cond]` | Halt with error if condition is falsy |
| `assert[cond, msg]` | Halt with custom error message |
| `error[reason]` | Create an error-value with the given reason |
| `error[reason, kind]` | Create an error-value with reason and category |
| `cpu[n]` | Set thread count for parallel loops |
| `gpu[n]` | Set GPU count (`gpu[]` for all GPUs) |
| `nodes[n]` | Set machine count for distributed compute |

#### Math

| Function | Description |
|----------|-------------|
| `abs[n]` | Absolute value |
| `sqrt[n]` | Square root (returns float) |
| `floor[n]` | Floor (returns int) |
| `ceil[n]` | Ceiling (returns int) |
| `round[n]` | Round to nearest int |
| `base ^ exp` | Exponentiation (use `^` operator) |
| `min[a, b]` | Minimum of two numbers |
| `max[a, b]` | Maximum of two numbers |

#### Strings

| Function | Description |
|----------|-------------|
| `split[s, delim]` | Split string into array |
| `trim[s]` | Remove leading/trailing whitespace |
| `replace[s, old, new]` | Replace all occurrences |
| `upper[s]` | Convert to uppercase |
| `lower[s]` | Convert to lowercase |
| `starts[s, prefix]` | Check if string starts with prefix |
| `ends[s, suffix]` | Check if string ends with suffix |
| `join[arr, sep]` | Join array elements into string |
| `repeat[s, n]` | Repeat string `n` times |
| `fmt[x, ...]` | Format a value with named arguments (see Formatting) |
| `reverse[s]` | Reverse a string |

#### Formatting

`fmt[value, ...]` formats numbers and strings using named arguments — plain Crow, no format mini-language:

| Named argument | Meaning | Default |
|---|---|---|
| `decimals = n` | Fixed decimal places (numbers) | off |
| `commas = true` | Thousands separators | false |
| `width = n` | Minimum width (pads with `fill`) | off |
| `align = 'left'/'right'` | Alignment within width | numbers right, strings left |
| `fill = c` | Pad character | `' '` |
| `percent = true` | Multiply by 100, append `%` (numbers) | false |

```
fmt[1234.567, decimals = 2]                  --- 1234.57 ---
fmt[1234.567, decimals = 2, commas = true]   --- 1,234.57 ---
fmt[0.123, percent = true, decimals = 1]     --- 12.3% ---
fmt[42, width = 8]                           --- '      42' ---
fmt[42, width = 5, fill = '0']               --- 00042 ---
fmt['alice', width = 8]                      --- 'alice   ' ---
```

**Name your formats.** For repeated formats, define a function once and use it everywhere — shorter and clearer than any per-slot format code:

```
fn money[x]
    return fmt[x, decimals = 2, commas = true]

log['P&L: `money[pnl]`']
```

`decimals`/`percent` on a string is an error. `fmt` replaces the removed `pad` builtin (`pad[s, n]` → `fmt[s, width = n]`).

##### Design Note: No Format Mini-Language

**What was considered:** Python-style format specs inside interpolation (`{pnl:,.2f}`) or as spec strings (`fmt[x, '.2f']`).

**Why rejected:** format codes are a second grammar — cryptic single-character commands with their own parser, embedded in strings. That violates the no-DSL rule and requires memorization; nobody writes `%-+8.2e` without a lookup. Named arguments are ordinary Crow: self-documenting, validated at compile time (unknown names and arity are lowering errors), and guessable without docs.

**The trade:** one-off formatting is wordier than `{pnl:,.2f}`. The sanctioned recovery is the named formatter (`money[x]`), which is *shorter* than the Python spec at the call site, has a domain name, and keeps every money field in the project formatted identically — the consistency that per-slot codes never provide.

#### Conversion

| Function | Description |
|----------|-------------|
| `int[value]` | Convert to int (error-value on failure) |
| `float[value]` | Convert to float (error-value on failure) |
| `str[value]` | Convert to string |
| `type[value]` | Type name as string: `'int'`, `'float'`, `'str'`, `'bool'`, `'array'`, `'map'`, `'fn'`, `'none'`, `'error'`, or struct name |

```
type[42]          --- 'int' ---
type[3.14]        --- 'float' ---
type['hello']     --- 'str' ---
type[[1, 2]]      --- 'array' ---
type[Point[1, 2]] --- 'Point' ---
type[error['x']]  --- 'error' ---
```

#### Collections

| Function | Description |
|----------|-------------|
| `len[x]` | Length of array, string, or map |
| `push[arr, value]` | Return new array with value appended |
| `pop[arr]` | Return new array without last element |
| `concat[a, b]` | Return new array merging two arrays |
| `concat[a, b, axis]` | Concatenate two tensors along an axis |
| `sort[arr]` | Return new sorted array |
| `reverse[arr]` | Return new reversed array |
| `contains[arr, value]` | Check if array contains value |
| `any[arr, pred]` | True if any element passes the predicate (parallel, early exit) |
| `all[arr, pred]` | True if every element passes (parallel, early exit) |
| `find[arr, pred]` | First element passing the predicate; error-value if none |
| `slice[arr, start, end]` | Extract subarray |

#### Map Operations

| Function | Description |
|----------|-------------|
| `keys[m]` | Array of map keys |
| `values[m]` | Array of map values |
| `has[m, key]` | Check if map contains key |
| `set[m, key, value]` | Return new map with entry added/updated |
| `del[m, key]` | Return new map with entry removed |

**All builtins are immutable.** No builtin modifies its arguments. To update a variable, reassign: `nums = push[nums, 4]`. The transpiler optimizes `x = f[x]` patterns into in-place operations automatically.

#### I/O

| Function | Description |
|----------|-------------|
| `read[path]` | Read file contents (error-value on failure) |
| `write[path, content]` | Write string to file (error-value on failure) |
| `exists[path]` | Check if file/directory exists (returns bool) |
| `mkdir[path]` | Create directory (and parents). Error-value on failure |
| `ls[path]` | List directory entries as sorted array of strings. Error-value on failure |
| `rm[path]` | Remove file or directory (recursive). Error-value on failure |
| `mv[src, dst]` | Move/rename file or directory. Error-value on failure |
| `cp[src, dst]` | Copy file. Error-value on failure |
| `args[]` | CLI arguments as array of strings |
| `input[]` | Read a line from stdin (error-value on failure) |
| `input[prompt]` | Print prompt, then read a line (error-value on failure) |

#### JSON

| Function | Description |
|----------|-------------|
| `json[str]` | Parse JSON string into a Crow value (error-value on failure) |
| `encode[value]` | Encode a Crow value as a JSON string |

JSON type mapping: objects → maps, arrays → arrays, strings → str, numbers → int/float, booleans → bool, `null` → error-value. Parsed JSON is dynamic data — the map homogeneity rule applies to maps you construct, not to values parsed from JSON (open-schema data is exactly what `json[]` is for). JSON `null` maps to an error-value with `ok: false` and `reason: 'null'`, which auto-propagates through the error-value system (§11).

```
items = json['[1, 2, 3]']
log[items[0]]              --- 1 ---

n = json['null']
log[n.ok]                  --- false ---
log[n.reason]              --- 'null' ---

s = encode[items]
log[s]                     --- [1,2,3] ---
```

#### System

| Function | Description |
|----------|-------------|
| `rand[]` | Random float between 0.0 and 1.0 |
| `rand[n]` | Random int between 0 and n-1 |
| `rand[min, max]` | Random int between min and max (inclusive) |
| `time[]` | Current unix timestamp as float (seconds) |
| `sleep[ms]` | Sleep for ms milliseconds (int) or seconds (float) |
| `env[name]` | Get environment variable (error-value if not set) |
| `env[name, default]` | Get environment variable, return default if not set |
| `exit[]` | Exit with code 0 |
| `exit[code]` | Exit with given code |

#### Shell

| Function | Description |
|----------|-------------|
| `shell[cmd]` | Run a shell command, returns map with `status`, `stdout`, `stderr` |

```
r = shell['echo hello']
log[r.stdout]    --- 'hello' ---
log[r.status]    --- 0 ---
```

#### Hashing

| Function | Description |
|----------|-------------|
| `hash[s, algo]` | Hash string with algorithm. Supported: `'sha256'`, `'sha512'`, `'md5'` |

```
h = hash['hello', 'sha256']
log[h]    --- '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824' ---
```

#### Regex

| Function | Description |
|----------|-------------|
| `matches[s, pattern]` | Check if string matches regex pattern (returns bool) |
| `find[s, pattern]` | Find first match with capture groups. Returns map with `ok` (bool) and `groups` (array) |
| `sub[s, pattern, replacement]` | Replace all regex matches with replacement string |

```
matches['hello', '[a-z]+']       --- true ---

m = find['2024-01-15', '([0-9]+)-([0-9]+)-([0-9]+)']
log[m.ok]        --- true ---
log[m.groups]    --- ['2024-01-15', '2024', '01', '15'] ---

sub['hello world', 'world', 'crow']  --- 'hello crow' ---
```

#### Date/Time

| Function | Description |
|----------|-------------|
| `now[]` | Current date/time as map with `year`, `month`, `day`, `hour`, `minute`, `second`, `timestamp` |
| `date[y, m, d]` | Create date map with `year`, `month`, `day`, `timestamp` |
| `elapsed[start, end]` | Time difference between two date maps. Returns map with `seconds`, `minutes`, `hours`, `days` |

```
t = now[]
log[t.year]        --- 2026 ---
log[t.timestamp]   --- unix timestamp ---

d1 = date[2024, 1, 15]
d2 = date[2024, 1, 16]
e = elapsed[d1, d2]
log[e.seconds]     --- 86400 ---
log[e.days]        --- 1 ---
```

## 4. Control Flow

### if / else

```
if x gt 5
    log['big']

if x gt 5
    log['big']
else
    log['small']

if x gt 5
    log['big']
else if x gt 2
    log['medium']
else
    log['small']
```

## 5. Loops

### for

`for` loops run in parallel by default. Order of results is always preserved — iterations may run in any order, but results are collected in the original order.

```
for x in [1, 2, 3]
    log[x]
```

When a `for` loop is used as a statement (not assigned to a variable), the results of each iteration are discarded. The loop runs only for its side effects:

```
for x in [1, 2, 3]
    log[x]               --- prints 1, 2, 3; results discarded ---
```

### for as Expression

Assign a `for` loop to a variable to collect results into an array.

```
--- transform each item ---
doubled = for x in [1, 2, 3]
    x * 2
--- doubled eq [2, 4, 6] ---

--- filter items ---
evens = for x in [1, 2, 3, 4, 5]
    if x % 2 eq 0
        x
--- evens eq [2, 4] ---

--- filter and transform ---
result = for x in [1, 2, 3, 4, 5]
    if x gt 2
        x * 10
--- result eq [30, 40, 50] ---
```

### while

`while` loops run sequentially. They support `break` and `continue`.

```
while x gt 0
    x = x - 1
```

#### break and continue

`break` exits the loop immediately. `continue` skips to the next iteration.

```
x = 0
while x lt 100
    if x eq 10
        break
    x = x + 1

total = 0
i = 0
while i lt 10
    i = i + 1
    if i eq 5
        continue
    total = total + i
```

> **Design Note:** `break` and `continue` are only valid in `while` loops. `for` loops are parallel by default (§12), and `break`/`continue` are inherently sequential concepts. Using `break` in a parallel loop would silently force sequential execution — a hidden performance trap. Use `if` inside `for` to skip items, `any`/`all`/`find` for searches that should stop early (they early-exit *in parallel* — see the Search Builtins Design Note), or `while` for genuine sequential iteration.

## 6. Design Note: No Pattern Matching or Sum Types

Crow intentionally omits pattern matching (`match`) and sum types (enums with variants).

**Why no pattern matching?** Without sum types, `match` is syntactic sugar for `if/else`. It adds a second way to branch, violating "one way to do things." Use `if/else` instead:

```
if value eq 0
    log['zero']
else if value gt 0 and value lt 10
    log['small']
else
    log['big']
```

**Why no sum types?** Sum types shine in systems programming and compiler design, where you model distinct variants (e.g. `Shape = Circle | Rectangle`). Crow targets data science and practical programming, where data lives in collections, tables, and structs — not algebraic types. Crow's error-value system (§11) handles the most common sum type use case (success vs failure) without additional syntax.

**If both were added, they would need to come together.** Pattern matching without sum types is pointless sugar. Sum types without pattern matching are unusable. Neither earns its place alone in a language optimized for simplicity and LLM-friendliness.


## 7. Structs

Structs are pure data — no methods. Define with `struct`, create with `[]`.

### Declaration

```
struct Bird [name, speed]
```

### Creation

```
b = Bird['crow', 50]
```

### Field Access

```
b.name                   --- 'crow' ---
b.speed                  --- 50 ---
```

### Structs with Functions

Structs have no methods. Pass structs to regular functions.

```
struct Bird [name, speed]

fn fly[bird]
    log['`bird.name` flies at `bird.speed`mph']

b = Bird['crow', 50]
fly[b]
```


## 8. Operators

### Arithmetic

```
+  -  /  *  %  ^
```

`^` is the power operator (right-associative):

```
2 ^ 10       --- 1024 ---
2 ^ 3 ^ 2   --- 512 (right-associative: 2^(3^2) = 2^9) ---
2.0 ^ 0.5   --- 1.4142... ---
```

Division is type-determined — one operator, no ambiguity:

```
3 / 2        --- 1 (both int → int, floor division)
3 / 2.0      --- 1.5 (float involved → float)
7 / 3        --- 2
7.0 / 3      --- 2.3333...
```

### Tensor Arithmetic

For tensors (numeric arrays), operators follow MATLAB/Julia semantics:

| Operator | Meaning |
|---|---|
| `*` | Matrix multiply (matmul) for tensor×tensor; scalar multiply for tensor×scalar |
| `.*` | Element-wise multiply |
| `/` | Scalar divide (tensor÷scalar only; tensor÷tensor is an error — use `./`) |
| `./` | Element-wise divide |
| `^` | Scalar power |
| `.^` | Element-wise power |
| `+`, `-` | Element-wise (always) |

```
use math
W = eye[2]
x = ones[2, 1]
y = W * x                   --- matmul ---
a = ones[3] .* full[3, 2]   --- element-wise multiply ---
b = ones[3] ./ full[3, 2]   --- element-wise divide ---
c = full[3, 2] .^ 3    --- element-wise power ---
d = ones[3] * 2        --- scalar multiply ---
```

Scalar-array operations broadcast the scalar:

```
[1, 2, 3] + 10              --- [11, 12, 13] ---
[1, 2, 3] * 2               --- [2, 4, 6] ---
10 - [1, 2, 3]              --- [9, 8, 7] ---
-[1, 2, 3]                  --- [-1, -2, -3] ---
```

Internally, all-numeric arrays use a fast tensor representation for element-wise operations. This is transparent — numeric arrays behave identically to regular arrays for indexing, iteration, and all builtins.

### Expression Grouping

Use `()` to group expressions and control evaluation order. Parentheses are **only** for grouping — function calls always use `[]`.

```
mid = (low + high) / 2
result = (a + b) * (c - d)
```

### Comparison

No symbols — use words only.

| Operator | Meaning          |
|----------|------------------|
| `eq`     | equal            |
| `ne`     | not equal        |
| `gt`     | greater than     |
| `lt`     | less than        |
| `gte`    | greater or equal |
| `lte`    | less or equal    |

```
if x eq 5
    log['equal']

if x gt 10
    log['big']
```

### Logical

```
and  or  not
```

```
if x gt 0 and y lt 10
    log['in range']

if not active
    log['inactive']

if x eq 1 or x eq 2
    log['one or two']
```


### Then Chaining

`then` pipes the left value as the first argument to the right function, enabling left-to-right composition instead of nested calls.

```
--- x then f = f[x] ---
s = '  hello  '
result = s then trim                --- trim[s] ---

--- x then f[a] = f[x, a] ---
result = 'hello world' then split[' '] then join['-']
--- split['hello world', ' '] then join[..., '-'] → 'hello-world' ---
```

Chains of any length:

```
result = s then trim then split[' '] then join[', ']
```

`then` binds tighter than comparisons and logical operators, but looser than arithmetic:

```
x + 1 then str          --- str[x + 1] ---
x then str eq '5'       --- (str[x]) eq '5' ---
```

Since all builtins are immutable and return new values, `then` works with every builtin:

```
nums = nums then sort then reverse then slice[0, 3]
```

## 9. Modules & Imports

### Importing

Use `use` to import a module. The entire module is imported — no selective imports. Multiple modules can be imported on a single line with commas.

```
use math
use views.home
use web, ui
```

Dot-separated paths map to directories: `use views.home` loads `views/home.crow` and prefixes its exports with `home_` (the last segment). This is how you import view files, library modules, etc.

Comma-separated imports expand at parse time: `use web, ui` is equivalent to writing `use web` and `use ui` on separate lines.

#### Aliased Imports: `use X as y`

Any import may take an alias. **An aliased import is qualified-only under its alias** — exactly Python's `import pandas as pd`: nothing enters your bare namespace except the alias itself.

```
use math as m

sum = 0                --- legal: math's names were never dumped
t = m.sum[xs]          --- the module is reached through the alias

use lib/scene as s     --- user modules too: s.mesh[...], s.group[...]
```

For the stdlib this doubles as the opt-out when you want a name the module owns (`sum`, `mean`, `e`). For packages and user modules — which are always qualified-only — the alias shortens long prefixes and resolves the otherwise-impossible case of two modules sharing a trailing name (`use alice/stats as astats`). `as` is contextual: it only has meaning after a use path, so a variable named `as` stays legal.

### Accessing

After `use`, all module functions become top-level — no prefix needed.

```
use math

result = sqrt[16]    --- 4 ---
x = zeros[3, 3]     --- 3x3 tensor of zeros ---
```

### Conflict Resolution

When two imported modules export the same function name, bare calls are ambiguous. Crow throws an error and requires the module prefix to disambiguate.

```
use db
use web

--- ERROR: ambiguous — use db.connect or web.connect ---
conn = connect['postgres://...']

--- Correct: use module prefix ---
db_conn = db.connect['postgres://user:pass@host/mydb']
ws_conn = web.connect['ws://localhost/ws']

--- No conflict: only one module exports exec ---
exec[db_conn, 'SELECT 1']
```

Rules:
- If only one module exports a name, bare calls work.
- If multiple modules export the same name, bare calls error with a list of qualified alternatives.
- Module-prefixed calls (`module.fn[args]`) always work regardless of conflicts.

### Built-in Module: `math`

The `math` module is built in — no file needed. It provides tensor creation, element-wise operations, reductions, shape operations, and linear algebra. After `use math`, all functions are called directly (e.g. `zeros[3]`, not `math.zeros[3]`).

#### Tensor Creation

| Function | Description |
|---|---|
| `zeros[d1, d2, ...]` | Tensor of zeros with given shape |
| `ones[d1, d2, ...]` | Tensor of ones with given shape |
| `full[d1, d2, ..., value]` | Tensor filled with `value` (last arg) |
| `eye[n]` | n×n identity matrix |
| `diag[tensor]` | Extract diagonal from a 2D tensor |
| `linspace[start, end, n]` | `n` evenly spaced values from `start` to `end` |
| `arange[start, end, step]` | Values from `start` to `end` by `step` |
| `randn[d1, d2, ...]` | Normal random values (mean=0, std=1) |
| `tensor[array]` | Convert an array to a tensor |

#### Element-wise Operations

| Function | Description |
|---|---|
| `exp[t]` | Element-wise exponential |
| `ln[t]` | Element-wise natural log |
| `log2[t]` | Element-wise base-2 logarithm |
| `log10[t]` | Element-wise base-10 logarithm |
| `sqrt[t]` | Element-wise square root |
| `abs[t]` | Element-wise absolute value |
| `sign[t]` | Element-wise sign (-1, 0, or 1) |
| `floor[t]` | Element-wise floor |
| `ceil[t]` | Element-wise ceiling |
| `round[t]` | Element-wise round to nearest |
| `clip[t, lo, hi]` | Clamp values to [lo, hi] |
| `mod[t, divisor]` | Element-wise modulo |
| `sin[t]` | Element-wise sine |
| `cos[t]` | Element-wise cosine |
| `tan[t]` | Element-wise tangent |
| `asin[t]` | Element-wise arcsine |
| `acos[t]` | Element-wise arccosine |
| `atan[t]` | Element-wise arctangent |
| `atan2[y, x]` | Element-wise two-argument arctangent |
| `sinh[t]` | Element-wise hyperbolic sine |
| `cosh[t]` | Element-wise hyperbolic cosine |
| `tanh[t]` | Element-wise hyperbolic tangent |

#### Reductions

| Function | Returns |
|---|---|
| `sum[t]` | Sum of all elements (float) |
| `mean[t]` | Mean of all elements (float) |
| `min[t]` | Minimum element (float) |
| `max[t]` | Maximum element (float) |
| `prod[t]` | Product of all elements (float) |
| `argmax[t]` | Index of maximum element (int) |
| `argmin[t]` | Index of minimum element (int) |

All reductions except `argmax`/`argmin` accept an optional axis parameter: `sum[t, axis]` reduces along that axis and returns a tensor instead of a scalar.

#### Sorting & Selection

| Function | Returns |
|---|---|
| `argsort[t]` | Indices that would sort the tensor (NumArray) |
| `topk[t, k]` | Top k largest values, sorted descending (NumArray) |
| `unique[t]` | Unique values, sorted (NumArray) |
| `where[cond, a, b]` | Element-wise: a where cond≠0, else b (NumArray) |

Note: `sort[t]` is a core builtin (not in the math module) that works on both arrays and tensors.

#### Cumulative Ops

| Function | Returns |
|---|---|
| `cumsum[t]` | Cumulative sum (NumArray, same shape) |
| `cumprod[t]` | Cumulative product (NumArray, same shape) |

#### Combining Ops

| Function | Returns |
|---|---|
| `stack[tensors]` | Stack tensors along new axis 0 (NumArray) |
| `stack[tensors, axis]` | Stack tensors along new axis (NumArray) |

Note: `concat[a, b, axis]` is a core builtin (not in the math module) — one merge operation for all arrays and tensors. It replaced the former `append`.

Note: `slice[t, start, end]` is a core builtin (not in the math module) that works on arrays, tensors, and strings.

#### Broadcasting

Element-wise operations on tensors with different shapes automatically broadcast following NumPy rules:
- Dimensions are right-aligned
- Size-1 dimensions expand to match the other tensor
- Missing dimensions are treated as size 1

```
use math
a = reshape[linspace[1, 3, 3], 3, 1]   --- shape [3, 1] ---
b = reshape[linspace[10, 40, 4], 1, 4]  --- shape [1, 4] ---
c = a + b                                          --- shape [3, 4] ---
```

Broadcasting works with all element-wise operators: `+`, `-`, `.*`, `./`, `.^`.

Incompatible shapes (e.g. `[3] + [4]`) produce a runtime error.

#### Advanced Indexing

Tensors can be indexed with another tensor to gather elements by index:

```
a = linspace[10, 50, 5]
indices = linspace[0, 2, 3]
y = a[indices]                    --- gathers elements at indices 0, 1, 2 ---
```

Negative indices are supported (wrap around from the end). The result tensor has the same shape as the index tensor.

#### Shape Operations

| Function | Description |
|---|---|
| `reshape[t, d1, d2, ...]` | Reshape to new dimensions |
| `flatten[t]` | Flatten to 1D |
| `transpose[t]` | Transpose (swap last two axes) |

#### Linear Algebra

| Function | Description |
|---|---|
| `a * b` | Matrix multiplication (use `*` operator) |
| `solve[a, b]` | Solve linear system Ax = b |
| `inv[a]` | Matrix inverse |
| `det[a]` | Determinant (returns float) |
| `svd[a]` | SVD → returns [U, S, Vt] |
| `qr[a]` | QR decomposition → returns [Q, R] |
| `cholesky[a]` | Cholesky (L*L^T) for symmetric positive-definite |
| `eigen[a]` | Eigenvalues → returns [real, imag] |
| `norm[x]` | L2 (Euclidean) norm (returns float) |
| `norm[x, p]` | Lp norm (p=1 for L1, inf for L∞) |
| `dot[a, b]` | Vector dot product (inner product, returns float) |
| `pinv[a]` | Moore-Penrose pseudo-inverse |
| `lstsq[a, b]` | Least-squares solve: minimize ‖Ax − b‖² |
| `triu[a]` | Upper triangular part of matrix |
| `tril[a]` | Lower triangular part of matrix |

#### Random & Distributions

| Function | Description |
|---|---|
| `seed[n]` | Set global RNG seed for reproducibility |
| `rand[d1, d2, ...]` | Uniform random in [0, 1) |
| `randn[d1, d2, ...]` | Standard normal (mean=0, std=1) |
| `normal[mean, std, d1, d2, ...]` | Normal with custom mean/std |
| `uniform[low, high, d1, d2, ...]` | Uniform in [low, high) |
| `poisson[lambda, d1, d2, ...]` | Poisson distribution |
| `bernoulli[p, d1, d2, ...]` | Bernoulli (0.0 or 1.0) |
| `gamma_dist[alpha, beta, d1, ...]` | Gamma distribution |
| `beta_dist[alpha, beta, d1, ...]` | Beta distribution |
| `exponential[lambda, d1, ...]` | Exponential distribution |
| `shuffle[x]` | Random permutation of tensor elements |
| `choice[x, n]` | Random sample of n elements (no replacement) |
| `permutation[n]` | Random permutation of 0..n |

#### Signal Processing

| Function | Description |
|---|---|
| `fft[x]` | 1D FFT → returns [real, imag] tensors |
| `ifft[real, imag]` | Inverse 1D FFT → returns real tensor |
| `fft2[m]` | 2D FFT → returns [real, imag] tensors |
| `conv[a, b]` | 1D convolution (full), length = len(a)+len(b)-1 |
| `conv2d[a, b]` | 2D convolution (full) |
| `hamming[n]` | Hamming window of length n |
| `hanning[n]` | Hanning (Hann) window of length n |
| `stft[x, window_size, hop_size]` | Short-Time Fourier Transform → [real, imag] 2D tensors [frames × window_size] |
| `spectrogram[x, window_size, hop_size]` | Power spectrogram (magnitude² of STFT) → 2D tensor |
| `lowpass[x, cutoff_hz, sample_rate]` | FFT-based lowpass filter |
| `highpass[x, cutoff_hz, sample_rate]` | FFT-based highpass filter |
| `bandpass[x, low_hz, high_hz, sample_rate]` | FFT-based bandpass filter |

#### Special Functions

| Function | Description |
|---|---|
| `gamma[x]` | Gamma function Γ(x), element-wise |
| `erf[x]` | Error function erf(x), element-wise |
| `bessel[x, n]` | Bessel function J_n(x), element-wise |
| `factorial[x]` | Factorial x! = Γ(x+1), element-wise |
| `choose[n, k]` | Binomial coefficient C(n,k), element-wise |

#### Complex Numbers

Complex values are represented as `[real_tensor, imag_tensor]` pairs.

| Function | Description |
|---|---|
| `complex[re, im]` | Create complex value → [real, imag] |
| `real[c]` | Extract real part tensor |
| `imag[c]` | Extract imaginary part tensor |
| `magnitude[c]` | Magnitude \|c\| = sqrt(re² + im²) |
| `phase[c]` | Phase angle = atan2(im, re) |
| `conjugate[c]` | Complex conjugate → [re, -im] |

#### Sparse Arrays

Sparse matrices store only non-zero entries, using CSR (Compressed Sparse Row) format internally. They use the same operators as dense arrays — the runtime dispatches to sparse-optimized algorithms automatically.

| Function | Description |
|---|---|
| `sparse[dense_array]` | Convert dense 2D array to sparse |
| `sparse[rows, cols, vals, shape]` | Create sparse from COO triplets (row indices, column indices, values, [nrows, ncols]) |
| `dense[s]` | Convert sparse matrix back to dense array |
| `s * v` | Sparse × dense matmul (vector or matrix) |
| `s + s2` | Sparse + sparse addition |
| `solve[s, b]` | Solve sparse linear system s·x = b |

```
use math

--- dense to sparse ---
s = sparse[eye[3]]
log[s]                              --- sparse[3x3, nnz=3] ---

--- COO creation ---
s2 = sparse[[0, 1, 2], [0, 1, 2], [5, 6, 7], [3, 3]]

--- sparse matmul ---
r = s * [1, 2, 3]

--- sparse add ---
s3 = s + s2

--- back to dense ---
d = dense[s3]

--- sparse solve ---
x = solve[s2, [10, 12, 14]]
```

#### Data Loading

| Function | Description |
|---|---|
| `load[path]` | Load numeric CSV file directly into a 2D tensor (error-value on failure) |

`load[path]` reads a CSV file with a header row and parses all values as floats into a 2D tensor (rows × columns). This is much faster than `csv[path]` (io module) for numeric data because it avoids creating intermediate string maps.

```
use math

data = load['data.csv']
log[data.shape]        --- [1000, 10] ---
log[sum[data]]         --- total of all values ---
```

#### Statistics (sub-module: `math.stats`)

Import with `use math.stats`. Functions become top-level after import.

| Function | Description |
|---|---|
| `median[x]` | Median of a 1D array |
| `std[x]` | Population standard deviation |
| `var[x]` | Population variance |
| `percentile[x, p]` | p-th percentile (0-100) with linear interpolation |
| `zscore[x]` | Z-score normalization: (x - mean) / std → returns array |
| `corr[x, y]` | Pearson correlation coefficient between two arrays |
| `cov[x, y]` | Population covariance between two arrays |
| `ttest[x, y]` | Welch's two-sample t-test → returns map with `t` and `p` |
| `linreg[x, y]` | Simple linear regression → returns map with `slope`, `intercept`, `r2` |
| `chisq[observed, expected]` | Chi-square goodness-of-fit test → map with `chi2`, `p` |
| `anova[g1, g2, ...]` | One-way ANOVA → map with `f`, `p` |
| `kstest[data, dist_name]` | Kolmogorov-Smirnov test → map with `d`, `p`. Distributions: `"normal"`, `"uniform"` |
| `fit[data, dist_name]` | Distribution fitting (MLE) → map of parameters. Distributions: `"normal"` → `{mu, sigma}`, `"exponential"` → `{lambda}`, `"uniform"` → `{a, b}`, `"poisson"` → `{lambda}` |

```
use math
use math.stats

x = linspace[1, 10, 10]
log[median[x]]            --- 5.5 ---
log[std[x]]               --- 2.872... ---
log[var[x]]               --- 8.25 ---
log[percentile[x, 25]]    --- 3.25 ---

z = zscore[x]             --- mean=0, std=1 ---

y = linspace[2, 20, 10]
log[corr[x, y]]           --- 1.0 (perfect correlation) ---
log[cov[x, y]]            --- 16.5 ---

result = ttest[x, y]
log[result.t]                   --- t-statistic ---
log[result.p]                   --- p-value ---
```

#### Neural Networks (sub-module: `math.nn`)

Import with `use math.nn`. Functions become top-level after import.

**Activations:**

| Function | Description |
|---|---|
| `relu[x]` | Rectified linear unit: max(0, x) element-wise (NumArray) |
| `sigmoid[x]` | Sigmoid: 1/(1+exp(-x)) element-wise (NumArray) |
| `softmax[x]` | Softmax: exp(x)/sum(exp(x)) with numerical stability (NumArray) |
| `gelu[x]` | Gaussian error linear unit element-wise (NumArray) |

**Loss functions:**

| Function | Description |
|---|---|
| `mse[pred, target]` | Mean squared error (Float) |
| `crossentropy[logits, targets]` | Cross-entropy loss with log-sum-exp stability (Float) |
| `bce[pred, target]` | Binary cross-entropy with clamping (Float) |

**Layers:**

| Function | Description |
|---|---|
| `linear[x, w, b]` | Linear layer: x @ w + b (NumArray) |

```
use math
use math.nn

x = linspace[-2, 2, 5]
log[relu[x]]              --- [0, 0, 0, 1, 2] ---
log[sigmoid[x]]           --- [0.119..., 0.269..., 0.5, 0.731..., 0.881...] ---
log[softmax[x]]           --- probabilities summing to 1 ---

pred = linspace[0.1, 0.9, 3]
target = linspace[0, 1, 3]
log[mse[pred, target]]    --- 0.00667 ---

input = reshape[linspace[1, 4, 4], 2, 2]
w = eye[2]
b = ones[2]
log[linear[input, w, b]]  --- input + bias ---
```


#### Constants

| Constant | Description |
|---|---|
| `pi` | π = 3.14159... |
| `e` | Euler's number e = 2.71828... |
| `inf` | Positive infinity |
| `nan` | Not a number |

#### Properties

| Property | Description |
|---|---|
| `.shape` | Array of dimension sizes |
| `.len` | Total number of elements |
| `.dtype` | Data type (`'float'`) |
| `.device` | Where the tensor lives (`'cpu'` or `'gpu'`) |
| `.grad` | Gradient status (`'none'` if no gradient) |

```
use math

a = zeros[3, 3]
b = eye[3]
c = a * b
d = det[b]           --- 1.0 ---

--- constants ---
log[pi]               --- 3.141592653589793 ---

--- properties ---
log[a.shape]               --- [3, 3] ---
log[a.dtype]               --- float ---
log[a.device]              --- cpu ---
```

### io Module

Import with `use io`. Provides structured file parsing beyond the built-in `read[path]` and `write[path, content]`.

| Function | Description |
|----------|-------------|
| `csv[path]` | Parse CSV file into array of maps (error-value on failure) |
| `lines[path]` | Read file as array of lines (error-value on failure) |

**Note:** Basic file I/O (`read[path]`, `write[path, content]`) are builtins available without any import. The io module adds structured parsing only.

```
use io

write['/tmp/data.csv', 'name,age\nalice,30\nbob,25\n']

rows = csv['/tmp/data.csv']
log[len[rows]]         --- 2 ---
log[rows[0].name]      --- alice ---
log[rows[0].age]       --- 30 ---

ls = lines['/tmp/data.txt']
log[len[ls]]           --- number of lines ---
log[ls[0]]             --- first line ---

--- error handling ---
bad = csv['/tmp/missing.csv']
log[bad.ok]            --- false ---
log[bad.reason]        --- error message ---
```

### Built-in Module: frame

Import with `use frame`. Column-oriented DataFrames for tabular data. See `docs/frame-module.md` for design details.

**Column references: strings for names, expressions for computation.** Naming a column is a string; computing on a column is a normal Crow expression over `df.col`. Every line is plain Crow — there is no special evaluation inside frame calls.

| Function | Description |
|---|---|
| `load[path]` | Load CSV → frame (error-value on failure) |
| `save[frame, path]` | Write frame → CSV |
| `select[frame, cols]` | Pick columns: `select[df, ['name', 'age']]` |
| `filter[frame, mask]` | Keep rows where mask is true: `filter[df, df.age gt 50]` |
| `sort[frame, col]` | Sort ascending: `sort[df, 'age']`. Descending: `sort[df, 'age', desc = true]` |
| `group[frame, col, aggs...]` | Group + aggregate: `group[df, 'dept', ['salary', 'mean'], ['age', 'max']]` |
| `join[f1, f2, key]` | Join on key column: `join[df1, df2, 'id']`. Type: `join[df1, df2, 'id', 'left']` |
| `stack[f1, f2]` | Stack frames vertically |
| `head[frame, n]` / `tail[frame, n]` | First/last n rows (returns a frame) |
| `fill[frame, col, value]` | Fill missing values in a column |
| `unique[col]` | Unique values in a column |
| `rename[frame, old, new]` | Rename a column: `rename[df, 'salary', 'pay']` |
| `stats[frame]` | Summary statistics |
| `datetime[s]` | Parse ISO 8601 datetime |
| `tz[d, offset]` | Convert timezone: `tz[d, '+09:00']` |

Properties: `.shape`, `.columns`, `.dtypes`. Column access: `df.age`. `len[df]` returns the row count. Aggregation names for `group`: `'sum'`, `'mean'`, `'min'`, `'max'`, `'count'`.

#### Boolean Masks

Filtering uses ordinary Crow expressions that evaluate to boolean arrays. These operations are vectorized (element-wise):

- `eq ne gt lt gte lte` — column vs scalar → boolean array
- `and` / `or` — two boolean arrays → boolean array
- `not` — boolean array → boolean array

```
use frame
df = load['people.csv']

old = filter[df, df.age gt 50]
alices = filter[df, df.name eq 'alice']
rich_old = filter[df, df.age gt 50 and df.salary gt 100000]
others = filter[df, not (df.name eq 'alice')]
```

Missing values compare as `false` — they are excluded by any filter.

Dynamic column names need no special syntax — a string variable works everywhere a column name does:

```
col = 'age'
sorted = sort[df, col]
```

#### Design Note: No Bare-Name Column References

**What was considered (and originally shipped):** Bare identifiers as column names — `sort[df, age]`, `group[df, dept, [salary, mean]]` — where an identifier that isn't a defined variable is treated as a column-name string.

**Why it was removed:**

1. **Scope-dependent meaning.** Adding a local variable named `age` silently changed `sort[df, age]` from a column reference into a value argument. Code broke at a distance with no error.
2. **Interpreter–transpiler divergence.** The interpreter preferred the variable when one existed; the transpiler force-converted certain argument positions to strings regardless. The same program meant different things in the two modes, violating the parity rule.
3. **Violates rule 19 (no DSL).** A second evaluation rule inside frame calls means a line of Crow can't be read without knowing which builtin it appears in.

**What to use instead:** Strings for column names, expressions for computation — as specified above. One mechanism, no sugar, dynamic columns free.

### Built-in Module: web

Import sub-modules with `use web.X`. Each sub-module enables only its feature — minimal binary, minimal compile time. See `docs/web-framework.md` for full architecture details.

#### View Blocks — the UI Vocabulary Zone

UI trees are built in `view` blocks — a declaration form, sibling of `fn`:

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

**Inside a view, the standard HTML element vocabulary is active** — `div`, `span`, `p`, `h1`–`h6`, `button`, `ul`, `ol`, `li`, `form`, `a`, `nav`, `header`, `footer`, `section`, `tr`, `td`, `th`, `thead`, `tbody`, `option`, `textarea`, `label`, `pre`, `code`, `input`, `img`, `table` — with their exact HTML names. **Outside a view, those names don't exist**: `p = 0.95`, `fn add[a, b]`, and `form = req.body` are ordinary code in any ui program, and `use ui` reserves nothing in your file. Calling an element outside a view is an error that points at the fix.

Views compose: a view calls other views (components) and ordinary functions freely. A module can export views — importers reach them qualified, like any module name:

```
--- cards.crow ---
view card[title, body_text]
    div[h2[title], p[body_text]]

--- main.crow ---
use cards as c

view render[]
    div[h1['Home'], c.card['Status', 'All good']]
```

This is the JSX/SwiftUI boundary: the lowercase vocabulary is fixed and language-owned; your components are module-exported views. There is no way to add elements or define new vocabulary zones — see the Design Note "Names Are Never Ambiguous".

Views and event handlers are the only functions that see reactive state (see Function Scope). `crow fmt --fix-views` migrates pre-view code (`fn` declarations that build ui trees) automatically.

**The view tree is target-independent.** A view describes a tree; what renders it is a build target. Web today; a desktop or mobile shell renders the same tree without a language change. Platforms are build targets, never dialects — there will never be a `screen` or `window` vocabulary fork.

#### Core (Server)

```
use web, ui
use views.home

serve[3000]
page['/', home_render, home_handle]
```

The transpiler manages the application instance implicitly — no `app` variable needed. `page[]` registers a LiveView route with a render function and event handler. `serve[port]` starts the server. The order in source doesn't matter: the transpiler always emits `serve[]` after all routes are registered.

If no routes are registered, `serve[]` shows a default welcome page so the user always sees something.

If the port is already in use, the server prints a friendly error: `error: port 3000 is already in use`.

| Function | Description |
|---|---|
| `serve[port]` | Start the server on the given port (implicit app) |
| `page[path, render, handle]` | Add a LiveView page with render/handle functions |
| `route[path, handler]` | Add a route with a handler function (implicit app; `route[path, method, handler]` for other methods) |

#### Responses

| Function | Description |
|---|---|
| `text[s]` | Plain text response |
| `html[s]` | HTML response |
| `json[map]` | JSON response from a map |
| `respond[status, body]` | Response with custom status code |
| `redirect[url]` | HTTP redirect response |
| `msgpack[value]` | MessagePack binary response (requires `rmp-serde`) |
| `arrow[frame]` | Arrow IPC binary response for DataFrames |
| `bytes[data]` | Raw binary response (`application/octet-stream`) |
| `stream[handler_fn]` | Streaming/chunked response via channel |

#### Auth (`use web.auth`)

| Function | Description |
|---|---|
| `sign[claims, secret]` | Sign a JWT from a claims map |
| `verify[token, secret]` | Verify a JWT and return claims map |

#### CORS (`use web.cors`)

| Function | Description |
|---|---|
| `cors[app]` | Apply permissive CORS (allow all origins) |
| `cors[app, origins]` | Apply CORS with specific allowed origins |

#### Sessions (`use web.session`)

| Function | Description |
|---|---|
| `sessions[]` | Create an in-memory session store |

#### Templates (`use web.template`)

| Function | Description |
|---|---|
| `templates[glob]` | Load templates matching a glob pattern |
| `render[templates, name, data]` | Render a template with data |

#### Cache (`use web.cache`)

| Function | Description |
|---|---|
| `cache[max]` | Create an LRU cache with max entries |
| `cache[max, ttl]` | Create an LRU cache with max entries and TTL in seconds |

#### Rate Limiting (`use web.rate`)

| Function | Description |
|---|---|
| `limit[max, window]` | Create a rate limiter (max requests per window seconds) |

#### File Uploads (`use web.upload`)

| Function | Description |
|---|---|
| `uploads[req]` | Extract uploaded files from a request |

#### SSE (`use web.sse`)

| Function | Description |
|---|---|
| `sse[app, path, handler]` | Add a Server-Sent Events endpoint |

#### WebSocket (`use web.ws`)

| Function | Description |
|---|---|
| `ws[app, path, handler]` | Add a WebSocket endpoint |
| `connect[url]` | Connect to a WebSocket server |
| `send[sock, msg]` | Send a message on a WebSocket |
| `recv[sock]` | Receive a message from a WebSocket |
| `close[sock]` | Close a WebSocket connection |

#### LiveView / Pages

| Function | Description |
|---|---|
| `page[path, render_fn, handle_fn]` | Add a LiveView page (server-rendered with live updates via WebSocket) |

#### Static Files (`use web.static`)

| Function | Description |
|---|---|
| `static_dir[app, url_path, dir_path]` | Serve static files from a directory |

#### Mail (`use web.mail`)

| Function | Description |
|---|---|
| `smtp[host, port, user, pass]` | Create an SMTP mailer |

#### Queue (`use web.queue`)

| Function | Description |
|---|---|
| `web.queue[n]` | Create a background task queue with n workers |
| `push[queue, fn]` | Push a task onto the queue |

#### API Docs (`use web.docs`)

| Function | Description |
|---|---|
| `web.docs[title, version]` | Generate OpenAPI docs with Swagger UI |

#### Client (`use web.client`)

| Function | Description |
|---|---|
| `get[url]` | HTTP GET request |
| `post[url, body]` | HTTP POST request |
| `put[url, body]` | HTTP PUT request |
| `del[url]` | HTTP DELETE request |

### Built-in Module: db

The `db` module provides database connectivity. Import with `use db`. Supports SQLite, PostgreSQL, and ClickHouse via URL-scheme dispatch. See `docs/db-module.md` for full architecture details.

#### Connection

```
use db

--- SQLite ---
conn = connect['sqlite://data.db']

--- PostgreSQL (default pool: 10 connections) ---
conn = connect['postgres://user:pass@host/mydb']

--- ClickHouse ---
conn = connect['http://localhost:8123/default']

--- Custom pool size ---
conn = connect['postgres://user:pass@host/mydb', 20]
```

| Function | Description |
|---|---|
| `connect[url]` | Open a database connection (pool) with default pool size |
| `connect[url, pool_size]` | Open a database connection (pool) with custom pool size |

Note: `connect` is also exported by `web.ws`. If both `use db` and `use web` are active, bare `connect[...]` is ambiguous — use `db.connect[...]` or `web.connect[...]`.

#### Query & Execute

| Function | Description |
|---|---|
| `query[conn, sql]` | Execute a SELECT and return results as a frame |
| `exec[conn, sql]` | Execute DDL/DML (CREATE, INSERT, UPDATE, DELETE), return affected row count |
| `exec[conn, sql, params]` | Execute parameterized DDL/DML with an array of bind values |

```
use db
conn = connect['postgres://user:pass@host/mydb']

--- Query returns a frame ---
users = query[conn, 'SELECT * FROM users']
log[len[users]]
filtered = filter[users, age gt 25]

--- Execute returns affected row count ---
exec[conn, 'CREATE TABLE users (id INT, name TEXT)']
n = exec[conn, 'DELETE FROM users WHERE age < ?', [18]]
log[n]
```

#### Bulk Insert

| Function | Description |
|---|---|
| `insert[conn, table, frame]` | Bulk insert a frame into a table using native fast paths |

Uses backend-specific protocols for maximum performance:
- **PostgreSQL:** COPY protocol
- **ClickHouse:** Native RowBinary
- **SQLite:** Batched prepared statements in a transaction

```
use db
conn = connect['http://localhost:8123/default']
insert[conn, 'events', events_frame]
```

#### Transactions

| Function | Description |
|---|---|
| `transact[conn, fn]` | Execute a named function inside a transaction (auto commit/rollback) |

Checks out one connection from the pool, passes it to the function, commits on success, rolls back on error. No `begin`/`commit`/`rollback` builtins — one way to do transactions.

```
use db
conn = connect['postgres://user:pass@host/mydb']

fn transfer[t]
  exec[t, 'UPDATE accounts SET balance = balance - 100 WHERE id = 1']
  exec[t, 'UPDATE accounts SET balance = balance + 100 WHERE id = 2']

transact[conn, transfer]
```

#### Migrations

| Function | Description |
|---|---|
| `migrate[conn, dir]` | Run forward-only SQL migrations from a directory |

Reads `.sql` files sorted by filename, executes new ones in order, tracks applied migrations in `_crow_migrations` table. Returns count of newly applied migrations.

```
use db
conn = connect['postgres://user:pass@host/mydb']
n = migrate[conn, 'migrations/']
log['`n` migrations applied']
```

#### Pool Defaults

| Backend | Default Pool Size | Notes |
|---|---|---|
| SQLite | 1 | File-based, concurrent writes lock |
| PostgreSQL | 10 | Standard for web/data apps |
| ClickHouse | 10 | Native protocol |

### File Structure

Each file is a module. The file path is the module path.

```
--- file: myapp/utils.crow ---

fn double[x]
    return x * 2
```

```
--- file: main.crow ---

use myapp/utils

result = utils.double[5]    --- 10 ---
```


## 10. Comments

A comment starts with `---` and runs to the end of the line. There is no closing delimiter.

```
--- a comment line

x = 5  --- inline comment

--- multi-line commentary is just
--- consecutive comment lines
```

### Design Note: Comments End at the Newline

**What was replaced:** paired `---`...`---` comments (open and close delimiters, spanning lines).

**Why:** an unclosed paired comment silently swallowed the following line into code — the comment ran until it found the `---` that *opened* the next comment, and the remainder of that line executed. This failure mode produced real incidents during development (corrupted test files; one bug misdiagnosed as a Unicode issue). End-of-line comments make the failure *unrepresentable*: every line is locally, independently correct, and commenting out a line is just prepending `---`.

**Migration:** old single-line comments (`--- text ---`) still parse — the trailing `---` is now inert text — and `crow fmt` strips these cosmetic closers. Old *block* comments with bare `---` fence lines are no longer supported; write one `---` per line.


## 11. Error Handling

Every value in Crow is a result. Fallible operations set `ok` to `false` with a `reason`, `kind`, and `trace`. Values auto-unwrap — you use them directly without worrying about the struct.

### Fallible Builtins

These builtins can fail and return an error-value:

| Builtin | When it fails |
|---------|---------------|
| `read[path]` | File doesn't exist or can't be read |
| `write[path, content]` | File can't be written |
| `mkdir[path]` | Directory can't be created |
| `ls[path]` | Path doesn't exist or can't be read |
| `rm[path]` | Path doesn't exist or can't be removed |
| `mv[src, dst]` | Source doesn't exist or move fails |
| `cp[src, dst]` | Source doesn't exist or copy fails |
| `env[name]` | Environment variable not set (no default provided) |
| `input[]` / `input[prompt]` | Stdin read fails |
| `int[string]` | String can't be parsed as integer |
| `float[string]` | String can't be parsed as float |
| `json[string]` | String is not valid JSON, or value is `null` |
| `connect[url]` | Database connection fails (bad URL, server unreachable) |
| `query[conn, sql]` | SQL syntax error or query execution fails |
| `exec[conn, sql]` | SQL syntax error or execution fails |
| `insert[conn, table, frame]` | Bulk insert fails (schema mismatch, connection error) |
| `transact[conn, fn]` | Transaction fails (rolled back on error) |
| `migrate[conn, dir]` | Migration fails (bad SQL, directory not found) |

Non-fallible conversions (e.g. `int[3.14]`, `int[true]`) return raw values with `ok: true`.

### Every Value Has Status

```
x = 5
log[x]            --- 5 ---
x.ok              --- true ---
x.reason          --- '' ---
x.kind            --- '' ---
x.trace           --- [] ---
```

Non-fallible values always have `ok` of `true`, `reason` and `kind` are `''`, and `trace` is `[]`.

### Auto Unwrap

```
content = read['file.txt']
log[content]               --- just works, auto-unwraps to the value ---

val = int['42']
doubled = val + val        --- auto-unwraps in arithmetic: 84 ---
```

When an error-value is used in arithmetic, string interpolation, or `log[]`, the inner value is automatically accessed.

### Explicit Handling

Check `.ok` only when you want to handle errors yourself.

```
content = read['file.txt']

if not content.ok
    log['failed: `content.reason`']
    log['kind: `content.kind`']
    log['trace: `content.trace`']
else
    log[content]
```

### Errors in Conditions

**An error has no truth value.** A comparison with an error-value operand does not produce true or false — it propagates the error, like every other operator. And a branch cannot be taken on an error: when an `if` or `while` condition evaluates to an error-value, the decision is refused —

- **inside a function**: the function immediately returns the error to its caller (auto-propagation, trace appended) — the same rule as error arguments, applied to decisions
- **at top level**: the program halts: `runtime error: line N: cannot branch on an error value — <reason>`

```
fn check[s]
    n = int[s]
    if n gt 5            --- if n is an error: check returns it here ---
        return 'big'
    return 'small'

r = check['abc']
log[r.ok]                --- false — the parse error propagated out ---
log[r.reason]            --- cannot convert 'abc' to int ---
```

The metadata fields `.ok`, `.reason`, `.kind`, `.trace` never propagate — they are always readable, so the guard idiom is always available and is the one bridge from error-space back to decisions:

```
age = int[input['age: ']]
if not age.ok
    log['invalid: `age.reason`']
else if age gte 18
    log['adult']
```

Checking is optional: the happy path needs nothing, and unguarded errors fail loudly at the decision instead of silently taking a branch. (Frame masks are unrelated: missing cells excluded by `filter` are validity-bitmap missingness, not error-values.)

#### Design Note: Errors Have No Truth Value

**What was considered:** treating error-values as false in comparisons and conditions (SQL NULL / IEEE NaN style: `err gte 18` → false).

**Why it was rejected:** fabricating an answer silently misclassifies data — bad input would always take the `else` branch (an unparseable age becomes a 'minor'). An error cannot be compared with a value; pretending otherwise is the Go zero-value bug with different spelling. Instead: **you can compute with an error, but you cannot decide with one.** Propagation is implicit through dataflow (like Odin's `or_return`, applied automatically); the refusal happens at the last moment that doesn't fabricate. A future `crow check` lint can flag unguarded fallible conditions statically.

### Creating Errors

Use the `error[]` builtin to create custom error-values:

```
fn validate[age]
    if age lt 0
        return error['age cannot be negative', 'validation']
    return age

result = validate[-5]
log[result.ok]          --- false ---
log[result.reason]      --- 'age cannot be negative' ---
log[result.kind]        --- 'validation' ---
```

`error[reason]` creates an error with an empty `kind`. `error[reason, kind]` creates an error with a category.

### Value Struct

Every value has:

| Field    | Description                          |
|----------|--------------------------------------|
| `ok`     | `true` or `false`                    |
| `reason` | error message (empty on success)      |
| `kind`   | error category (empty on success or if not specified) |
| `trace`  | array of locations the error passed through (empty on success) |

On failure, the value contains a safe default (`0` for int, `0.0` for float, `''` for string), `reason` describes what went wrong, `kind` categorizes the error, and `trace` records the call chain. Each trace entry uses the format `"file.function:line"` inside a function or `"file:line"` at top-level (e.g. `"math.divide:5"`, `"main:3"`).

```
x = int['hello']
log[x.ok]         --- false ---
log[x.reason]     --- 'cannot convert ...' ---
log[x.kind]       --- 'parse' ---
log[x.trace]      --- ['main:1'] ---
```

### Auto-Propagation

Error-values propagate automatically through function calls. When a function (builtin or user-defined) receives an error-value as an argument, it returns that error immediately without executing. The original error's `.reason` and `.kind` are preserved, and the new location is appended to `.trace`.

```
fn process[x]
    return x * 2

fn pipeline[x]
    a = process[x]
    b = process[a]
    return b

data = int['bad']           --- error-value ---
result = pipeline[data]     --- error propagates through pipeline → process ---
log[result.ok]              --- false ---
log[result.reason]          --- 'cannot convert ...' ---
log[result.trace]           --- ['main.validate:5', 'main:6', ...] ---
```

**Exception:** `log[]` does not propagate — it prints error-values directly for debugging.

Errors also propagate through operators (`+`, `-`, etc.) and unary operations. If either operand of a binary op is an error, the result is that error.

### Error Messages

Runtime errors include line numbers for easy debugging:

```
runtime error: line 3: undefined variable 'z'
runtime error: line 7: assertion failed: expected positive
```


## 12. Parallelism

Crow is parallel by default. The runtime analyzes dependencies between statements and runs independent work in parallel automatically. No explicit threading, async/await, or channels needed.

### Automatic Parallelism

Independent statements run in parallel. The runtime waits only when a value is actually needed.

```
a = fetch['url1']
b = fetch['url2']
c = process['data']
d = a + b             --- waits for a and b ---
log[d]
log[c]
```

`a`, `b`, and `c` run in parallel. `d` waits for `a` and `b` to resolve.

### For Loops

`for` loops run iterations in parallel by default using `rayon::par_iter`.

```
for x in items
    process[x]        --- each iteration runs in parallel ---
```

Output order inside a parallel `for` is unspecified — `log[]` calls from different iterations may interleave in any order. When order matters, collect results with `for`-as-expression (results are always in input order) and log afterward.

If the loop body mutates a variable from the outer scope, Crow automatically falls back to sequential execution for safety:

```
total = 0
for x in items
    total = total + x  --- sequential: mutates outer 'total' ---
```

### While Loops

`while` loops run sequentially.

```
while x gt 0
    x = x - 1
```

### Resource Control

By default, Crow uses all available CPU cores. Use `cpu`, `gpu`, or `nodes` to control resources.

```
cpu[4]                   --- 4 CPU cores ---
gpu[]                    --- all GPUs on this machine ---
gpu[2]                   --- 2 GPUs on this machine ---
nodes[4]                 --- 4 machines ---
nodes[4, gpu = 2]        --- 4 machines, 2 GPUs each ---
```

#### Function-level

```
fn heavy_work[]
    cpu[4]
    for x in items
        process[x]           --- 4 cores ---

fn gpu_work[]
    gpu[]
    for x in items
        compute[x]           --- all GPUs ---
```

#### Global

Set program-wide defaults in the `crowfile`. Any function can override locally.

```
--- crowfile ---

cpu = 4
gpu = 'all'
```

```
--- file: main.crow ---

fn task_a[]
    for x in items
        process[x]           --- 4 cores (from crowfile) ---

fn task_b[]
    cpu[8]                   --- override: 8 cores for this function ---
    for x in items
        heavy[x]
```

### GPU Support

`gpu[n]` specifies intent — "use n GPUs." `gpu[]` uses all available GPUs. The language does not expose any GPU vendor API. The compiler decides what is GPU-eligible (numeric operations on arrays) and what falls back to CPU (string ops, I/O, map lookups).

#### Implementation

When `gpu[]` or `gpu[n]` is called, a global GPU-enabled flag is set. This has two effects:

1. **Tensor auto-placement**: Tensor creation functions (`zeros`, `ones`, `full`, `arange`, `linspace`, `eye`, `rand`, `randn`) automatically place data on GPU memory when the flag is set and a GPU is available.

2. **Op auto-dispatch**: Element-wise operations (`+`, `-`, `*`, `/`, `neg`, `abs`, `exp`, `log`, `sqrt`, `sin`, `cos`) and scalar operations automatically dispatch to GPU compute shaders (WGSL via wgpu) when both operands are on GPU. Reductions (`sum`) also dispatch to GPU. If tensors are on CPU, the CPU path is used as fallback.

GPU uses f32 internally (wgpu limitation). CrowTensor uses f64. Conversion happens transparently during GPU upload/download, with ~7 significant digits of precision on GPU vs ~15 on CPU.

### Automatic Differentiation (grad)

`grad[f]` takes a function and returns a new function that computes the gradient of `f`. Uses forward-mode automatic differentiation with dual numbers.

#### Single Parameter

When `f` takes one parameter and returns a scalar, `grad[f]` returns a function that computes the derivative:

```
fn f[x]
  x * x

df = grad[f]
log[df[3]]       --- 6 (derivative of x^2 at x=3 is 2*3=6) ---
```

#### Multiple Parameters

When `f` takes multiple parameters, `grad[f]` returns a function that computes the gradient (array of partial derivatives):

```
fn g[x, y]
  x * x + y * y

dg = grad[g]
log[dg[3, 4]]    --- [6, 8] (∂g/∂x=2x=6, ∂g/∂y=2y=8) ---
```

#### Supported Operations

All arithmetic operations (`+`, `-`, `*`, `/`) and these math builtins are differentiable:

- `sqrt` — derivative: 1/(2√x)
- `abs` — derivative: sign(x)
- `^` (power) — derivative: n·x^(n-1) for constant exponent
- `floor`, `ceil`, `round` — derivative: 0 (piecewise constant)

#### Implementation

Uses forward-mode AD with dual numbers. Each number carries a value and a derivative component. For N parameters, N forward passes compute the full gradient. The `grad` builtin works identically in both the interpreter and transpiler.

### Distributed Support

`nodes[n]` specifies distributed computing across multiple processes. The compiler handles process coordination, and the `crow launch` CLI command spawns and manages all processes.

```
nodes[4]                 --- 4 processes ---
```

#### Implementation

When `nodes[n]` is called, the runtime initializes distributed mode using TCP-based networking:

1. **Process discovery**: Each process reads `CROW_RANK`, `CROW_WORLD_SIZE`, `CROW_MASTER_ADDR`, and `CROW_MASTER_PORT` from environment variables.
2. **Ring topology**: Processes form a ring — each connects to the next rank via TCP.
3. **Available operations**: `allreduce_mean` (ring allreduce), `broadcast`, `barrier`, `shard` (split data by rank).
4. **Finalize**: The transpiler auto-inserts `finalize()` at end of main to clean up connections.

In interpreter mode, `nodes[n]` always simulates single-process (rank 0) — no TCP connections are made.

#### CLI: crow launch

```
crow launch <n> <file.crow>
```

Builds the binary via `crow build`, then spawns `n` processes with the correct environment variables. All processes run on `127.0.0.1` with base port `29500`. Output from all ranks is interleaved to stdout/stderr.

```
crow launch 4 train.crow     --- build + run on 4 processes ---
```

#### Backend Detection

The toolchain auto-detects the available GPU runtime:

- NVIDIA → CUDA
- Apple Silicon → Metal
- Cross-platform → Vulkan / WebGPU

The backend can be pinned in the `crowfile`:

```
gpu_backend = 'auto'        --- default: detect at build time ---
gpu_backend = 'cuda'
gpu_backend = 'metal'
gpu_backend = 'vulkan'
```

#### Decoupled from Language Version

New GPU vendors, new CUDA versions, or new backends are **toolchain updates** (`crow update`), not language changes. The `.crow` source files never change. This follows the same model as LLVM — Rust and Swift don't change when LLVM adds a new target.

The language specifies *intent*. The toolchain handles *implementation*.

## 13. Project Configuration

### Project Structure

A Crow program can be a single file or a full project. No restructuring is needed as a project grows.

#### Single File (No Project)

Any `.crow` file is a valid program:

```
crow run hello.crow
```

#### Project Structure

A project is a directory with a `crowfile` at the root. Create one with `crow new`:

```
crow new myapp
```

Full project structure (all optional except `crowfile` and `main.crow`):

```
myapp/
  crowfile                    --- project identity + config
  main.crow                   --- default entry point (crow run)

  jobs/                       --- batch tasks (crow run <name>)
    train.crow                --- crow run train
    migrate.crow              --- crow run migrate
    sync.crow                 --- crow run sync

  views/                       --- web views (imported and wired in main.crow)
    home.crow                 --- use views.home → page['/', home_render, home_handle]
    dashboard.crow            --- use views.dashboard → page['/dashboard', ...]
    accounts.crow             --- /accounts

  components/                 --- reusable UI functions
    nav.crow
    chart.crow

  layout.crow                 --- shared UI wrapper for all pages

  lib/                        --- shared code (organized however you want)
    accounts.crow
    ml/
      features.crow
      model.crow

  db/
    schema.crow               --- source of truth for tables

  tests/                      --- integration tests
    accounts_test.crow

  config/                     --- configuration files
  models/                     --- trained model artifacts
  data/                       --- data files
```

#### Conventions

| Folder/File | Purpose | Auto-discovered? |
|---|---|---|
| `crowfile` | Project identity, entry points, deps | Required for projects |
| `main.crow` | Default entry point. `crow run` runs this | Yes — convention |
| `views/` | Web views. Import with `use views.X`, wire with `page[]` | Explicit |
| `layout.crow` | Shared UI wrapper for all pages | **Yes** — auto-applied |
| `db/schema.crow` | Table definitions, auto-migrations | **Yes** — auto-discovered |
| `jobs/` | Batch tasks. `crow run <name>` | **Yes** — listed in crowfile |
| `tests/` | Integration tests. `crow test` | **Yes** — auto-discovered |
| `lib/` | Your code, any organization | No — import with `use` |
| `components/` | Reusable UI functions | No — import with `use` |
| `config/` | Configuration files | No — import with `use` |

Only three magic folders: `views/`, `db/`, `tests/`. Plus `layout.crow` and `main.crow` by convention. Everything else is just files you import.

#### Entry Points

`main.crow` is the long-running entry point (e.g., web server). `jobs/` contains batch tasks that run and exit.

```
crow run              --- runs main.crow (long-running)
crow run train        --- runs jobs/train.crow (batch, exits)
crow run migrate      --- runs jobs/migrate.crow (batch, exits)
```

Rule: **`main.crow` is the thing that stays running. Everything else runs and exits.**

#### Growth Path

| Stage | What you have |
|---|---|
| Day 1 | `hello.crow` — one file, no project |
| Week 1 | `crow new myapp` — `crowfile` + `main.crow` |
| Week 2 | Add `views/`, `lib/` — split code out |
| Month 1 | Add `db/schema.crow`, `jobs/train.crow` |
| Month 6 | Full project — multiple entry points, lib files, pages, components |

No restructuring at any step. You just add files.

### crowfile

Every Crow project has a `crowfile` at the root. It uses Crow syntax.

```
--- crowfile ---

name = 'myapp'
cpu = 4
gpu = 'all'

deps = map[math = '1.0', http = '2.3']

[jobs]
train = 'jobs/train.crow'
migrate = 'jobs/migrate.crow'
sync = 'jobs/sync.crow'
```

No need to list `main.crow` — it is the default entry point by convention.

Dependencies can be:

| Format | Example | Description |
|--------|---------|-------------|
| Version | `math = '1.0'` | Registry package (resolved via central index) |
| Git URL | `math = 'https://github.com/user/crow-math'` | Cloned from git |
| Local path | `utils = './libs/utils'` | Symlinked from local directory |

### File Extension

Crow source files use the `.crow` extension.

```
main.crow
lib/accounts.crow
views/dashboard.crow
jobs/train.crow
```

### CLI

```
crow                        --- start interactive REPL ---
crow new myapp              --- create new project (crowfile + main.crow) ---
crow new myapp --web        --- create web project (+ views/home.crow) ---
crow new myapp --ml         --- create ML project (+ jobs/train.crow, lib/model.crow) ---
crow new myapp --db         --- create DB project (+ db/schema.crow, jobs/migrate.crow) ---
crow new myapp --web --ml --db  --- all capabilities ---
crow install                --- install deps from crowfile into crow_modules/ ---
crow build                  --- transpile to Rust and compile to native binary ---
crow build file.crow        --- transpile a specific file ---
crow run                    --- transpile + compile + run main.crow ---
crow run file.crow          --- transpile + compile + run a specific file ---
crow run train              --- run a named job from crowfile [jobs] ---
crow run -i file.crow       --- run with interpreter (for debugging) ---
crow fmt                    --- format project (uses crowfile) ---
crow fmt file.crow          --- format a specific file ---
crow fmt --check            --- check if formatting needed (exit 1 if not) ---
crow test --parity          --- run tests/parity/ in both modes and diff output ---
```

`crow test --parity` is the interpreter–transpiler parity harness: every `tests/parity/*.crow` file runs in both execution modes and any difference in output (or exit status) is a failure. This enforces rule 11 (parity) mechanically. (A fuller `crow test` that runs `tests/` and verify statements is planned; only `--parity` is implemented today.)

`crow build` transpiles `.crow` source files to Rust and compiles to a native binary. `crow run` builds and then executes the binary.

**`.env` auto-loading:** Both `crow run` and `crow build` automatically load a `.env` file from the current directory if it exists. Each line should be `KEY=VALUE`. Lines starting with `#` are comments. Quoted values (`'...'` or `"..."`) are unquoted. Real environment variables take priority over `.env` values — the file only sets variables that aren't already set.

`crow fmt` auto-formats Crow source files. Formatting rules:
- 2-space indentation (normalizes tabs, 4-space, or mixed indentation)
- No trailing whitespace
- At most one consecutive blank line
- Single newline at end of file
- No leading blank lines

The formatter validates syntax before formatting. It works at the source level (not AST), so comments, `then` chains, and all syntax are preserved exactly. `crow fmt --check` exits with code 1 if any file needs formatting (useful for CI).

### REPL

Running `crow` with no arguments starts an interactive REPL. Variables and functions persist between lines. Expression results are printed automatically.

```
> x = 10
> x + 5
15
> fn double[n]
      n * 2

> double[x]
20
```

Multi-line blocks (functions, if, for, while, struct) are entered by typing the first line, then indenting the body. An empty line or a non-indented line ends the block.

### Package Manager

`crow install` reads deps from the `crowfile` and installs them into `crow_modules/`.

```bash
crow new myapp
cd myapp
--- edit crowfile to add deps ---
crow install
crow run
```

**Registry packages** (version strings like `'1.0'`) are resolved via a central index hosted on GitHub. The registry is a git repository containing an `index.json` that maps package names to git repositories and available versions. When installing, Crow clones the registry repo locally, reads the index, finds the package repo URL, and clones it at the matching git tag (`v1.0`, `v2.3`, etc.). If the exact tag is not found, the latest commit is used with a warning.

**Git dependencies** are cloned directly from the URL. **Local path dependencies** are symlinked.

Installed modules are imported with `use`.

#### Registry

The default registry is the git repo at `https://github.com/vsriram/registry.git`. On first `crow install`, the registry is cloned to `~/.crow/registry/`. On subsequent runs, it is updated via `git pull`.

Override the registry repo with the `CROW_REGISTRY` environment variable:

```bash
CROW_REGISTRY=https://github.com/myorg/my-registry.git crow install
```

#### Registry Format

The registry repo must contain an `index.json` at its root:

```json
{
  "packages": {
    "mylib": {
      "repo": "https://github.com/user/crow-mylib",
      "versions": ["0.1", "0.2", "1.0"]
    }
  }
}
```

Each package entry has a `repo` (git URL) and `versions` (list of available version strings). Version strings correspond to git tags prefixed with `v` (e.g., version `1.0` maps to tag `v1.0`).

## 14. Type System

Crow is **statically typed with inference**. No type annotations are needed — the transpiler infers concrete types from how values are used.

### Rules

- Every variable has exactly one type throughout its scope
- Types are inferred from assignment: `x = 5` makes `x` an `int`
- Function parameter and return types are inferred from call sites
- If the transpiler cannot determine a type, the program is invalid

### Call-Site Inference (No Generics Needed)

Crow has no generics, no type parameters, and no type annotations — but you can still write functions that work across different types. The transpiler infers concrete types from each call site, effectively specializing the function for the types used.

```
fn filter[items, f]
    result = for x in items
        if f[x]
            x
    return result

fn is_even[x]
    return x % 2 eq 0

fn is_positive[x]
    return x gt 0

evens = filter[[1, 2, 3, 4], is_even]         --- items: array[int], f: fn[int] -> bool ---
positives = filter[[-1, 0, 3], is_positive]    --- same types, different function ---
```

This works because `filter[[1, 2, 3, 4], is_even]` tells the transpiler that `items` is an array of ints and `f` is a function taking an int and returning a bool. No annotations needed — the call site *is* the type information.

When one function is called with *different* type signatures, the transpiler **monomorphizes** it — generating one specialized native version per signature, invisibly:

```
fn first[arr]
    return arr[0]

log[first[[1, 2, 3]]]      --- compiles a tensor version ---
log[first[['a', 'b']]]     --- compiles a string-array version ---
```

Specialization propagates through call chains and works with recursion and struct arguments. Two documented limits: a function used at multiple signatures cannot itself be passed *as a value* (a function value has one type — wrap it in a single-type function), and a generic function with *function-typed parameters* can be reused across predicates of the same element type but not across element types (specializing the predicate per clone is future work).

### Execution Modes

| Mode | Command | Role |
|------|---------|------|
| Transpiler | `crow run file.crow` | **Primary.** Compiles to native Rust. This is the authority on valid Crow programs. |
| Interpreter | `crow run -i file.crow` | **Dev tool.** Runs code directly for quick iteration. Slower, but convenient. |
| REPL | `crow` | **Interactive.** Explore and test code interactively. Uses the interpreter. |

The transpiler is the source of truth. If a program works in the interpreter but fails the transpiler, the program has a type error and should be fixed. The interpreter is internally dynamic for convenience, but users should write code that the transpiler can handle.

### No `none`

Crow has no null, nil, or none value. Every variable always holds a concrete value. Missing or failed data is represented through the error-value system (`ok`, `reason`, `kind`, `trace`) — see Error Handling.


## Design Note: No Iterator Builtins

Crow previously had a set of iterator builtins: `iter`, `collect`, `take`, `skip`, `filter`, `each`, and `next`. These converted an array to a lazy iterator, transformed/filtered it through chained calls, and materialized it back into an array with `collect`.

**Why they were removed:**

1. **Too verbose.** Chained iterator calls read inside-out and require excessive nesting:
   `collect[each[filter[take[iter[nums], 5], is_even], double]]` — hard to read, hard to write.

2. **Violates "one way to do things."** Crow's `for`-as-expression already handles mapping, filtering, and combined map+filter more readably:
   ```
   result = for x in nums
       if x % 2 eq 0
           x * 2
   ```

3. **`slice` covers take/skip.** `slice[arr, start, end]` replaces `take` and `skip` without needing an iterator abstraction.

4. **Laziness isn't needed.** Crow's automatic parallelism (§12) handles performance at the statement level. Lazy evaluation adds complexity without meaningful benefit for Crow's target audience.

**What to use instead:**

| Old (iterators) | New (for + slice) |
|---|---|
| `collect[each[iter[arr], fn]]` | `for x in arr \n    fn[x]` |
| `collect[filter[iter[arr], pred]]` | `for x in arr \n    if pred[x] \n        x` |
| `collect[take[iter[arr], n]]` | `slice[arr, 0, n]` |
| `collect[skip[iter[arr], n]]` | `slice[arr, n, len[arr]]` |


## Design Note: No Lambdas or Closures

Crow intentionally has no anonymous functions (lambdas), closures, or inline function expressions.

**Why they were rejected:**

1. **Violates "one way to do things."** If lambdas existed, there would be two ways to define behavior: `fn double[x]` and `fn[x] x * 2`. Every code review becomes a style debate. One syntax for functions means zero ambiguity.

2. **Closures add hidden complexity.** Closures capture variables from their enclosing scope, which creates hidden data dependencies. In a language designed for automatic parallelism (§12), captured mutable state would silently break parallel execution or require complex borrow-checking rules that Crow deliberately avoids.

3. **Named functions are more readable.** `map[items, double]` is clearer than `map[items, fn[x] x * 2]`. The name `double` documents intent. Anonymous functions save a line of code but cost readability — a bad trade for a language optimized for clarity.

4. **LLM-unfriendly.** Lambda syntax varies wildly across languages (`=>`, `->`, `lambda`, `\x ->`, `fn[x]`). A single `fn name[params]` declaration is unambiguous for both humans and language models.

5. **First-class named functions cover every use case.** Crow supports passing named functions as values — `map[items, double]`, `sort[arr, compare]`, storing functions in variables. There is no expressiveness gap.

**What to use instead:**

```
--- define the behavior once, with a name ---
fn double[x]
    return x * 2

result = map[items, double]
```

For one-off transformations, `for`-as-expression is often simpler than `map` + lambda anyway:

```
result = for x in items
    x * 2
```


## Design Note: No Enumerate or Zip

Crow intentionally omits `enumerate` and `zip` builtins.

**Why they were rejected:**

1. **Mixed-type problem.** `enumerate[['a','b']]` would need to return `[[0,'a'], [1,'b']]` — arrays mixing int and string. Crow arrays are homogeneous, so this doesn't fit the type system. `zip[[1,2], ['a','b']]` has the same problem.

2. **`range[len[arr]]` already covers every use case.** Index-based iteration with `for i in range[len[arr]]` handles every scenario that `enumerate` or `zip` would address, with no new concepts.

3. **No new concepts needed.** Adding `enumerate` or `zip` would require either tuples, anonymous structs, or mixed-type arrays — none of which exist in Crow. The cost of adding a new type concept outweighs saving one line of code.

**What to use instead:**

Numbered output:
```
for i in range[len[players]]
    log['`i + 1`. `players[i]`']
```

Find index of something:
```
for i in range[len[scores]]
    if scores[i] eq max[scores]
        log['best at index `i`']
```

Combine parallel arrays (instead of zip):
```
for i in range[len[names]]
    log['`names[i]`: `scores[i]`']
```

Dot product (instead of zip + map):
```
products = for i in range[len[a]]
    a[i] * b[i]
total = sum[products]
```

Build a map from two arrays:
```
m = map[]
for i in range[len[keys]]
    m = set[m, keys[i], values[i]]
```

The pattern is always the same: iterate over indices with `range[len[arr]]`, access elements by index. One pattern, zero new concepts.


## Design Note: Search Builtins (any, all, find) — a Reversal

**Original position:** `any`/`all`/`reduce` were rejected ("the loop versions are the same length"), and `find` was rejected ("finding a position is straightforward with a loop").

**Why it was reversed:** the recommended flag-loop replacement was worse than the thing it replaced, on three axes at once: it cannot stop early (always O(n)); mutating the flag variable triggers the automatic sequential fallback — the exact "hidden performance trap" the `break` rejection warns about; and it is more code. Worse, the *parallel early-exit* search (all cores scanning, all stopping at the first hit) is impossible to express in user code at all — parallel `for` cannot break, and `break`-able `while` cannot parallelize. That is the same "cannot be written cleanly without a builtin" test that justified `contains`, passed harder.

**The design:** predicates are named functions (no lambdas — name the question):

```
fn over_90[x]
    return x gt 90

any[scores, over_90]      --- true if at least one passes ---
all[scores, over_90]      --- true if every one passes ---
hit = find[scores, over_90]   --- the first passing ELEMENT ---
if not hit.ok                 --- error-value when nothing matches ---
    log['none found']
```

`find` dispatches by argument types: `find[str, pattern]` is the regex builtin; `find[array, function]` is the search. Both modes agree; compiled mode runs searches in parallel with deterministic first-match semantics (rayon `find_first`).

**Division of labor:** operators and verbs eat most loops; `for` computes over everything that is left, in parallel; `any`/`all`/`find` ask the questions that deserve to stop early; `while` handles genuine sequential iteration. `reduce` stays rejected — the named reductions (`sum`, `min`, `max`, `prod`, `join`) cover it.

## Design Note: No Augmented Assignment

Crow intentionally has no `+=`, `-=`, `*=`, `/=`, or any compound assignment operators.

**Why:** `x += 1` is syntactic sugar for `x = x + 1`. Adding it creates two ways to write the same thing, violating "one way to do things." Every language that supports both spawns style debates about when to use which form.

**What to use instead:**

```
x = x + 1
total = total + item
```

The explicit form is clear, consistent, and only a few characters longer. The transpiler can optimize `x = x + op` patterns into in-place operations automatically — the sugar adds no performance benefit.

## Design Note: No Double-Quote Strings

Crow uses single quotes for all strings. There is no double-quote alternative.

**Why:** Adding a second string delimiter would create two ways to write the same thing, violating "one way to do things." Every language that supports both `'...'` and `"..."` spawns style debates and inconsistency.

**Nested quotes:** Use the `\'` escape sequence. Most strings don't contain quotes. When they do, `\'` handles it cleanly:

```
msg = 'he said \'hello\' to me'
```

For heavy quoting (SQL, JSON, HTML), use interpolation or load from a file with `read[]`.

## Design Note: No String Concatenation with `+`

Crow does not allow `+` to concatenate strings. String interpolation is the only way to combine strings.

**What was considered:** `'hello' + ' world'` producing `'hello world'`.

**Why it was rejected:** `+` is numeric. For combining pieces, interpolation (`` 'hello `name`' ``) shows the result's shape; for building from many pieces, collect parts and `join[parts, sep]` — the O(n) idiom every `+`-language teaches as best practice anyway; for a rare pairwise merge, `concat[a, b]` works on strings. Adding `+` as a fourth way violates "one way to do things".

**What to use instead:**

```
greeting = 'hello `name`'
full = '`first` `last`'
```

## Design Note: No Destructuring

Crow intentionally has no destructuring assignment from function returns or collections (e.g. `a, b = f[]`).

**What was considered:** Allowing `a, b = divmod[7, 3]` where a function returns an array and multi-assign unpacks it.

**Why it was rejected:** Multi-assign already exists for literal values (`a, b = 1, 2`). Extending it to destructure arrays would create two ways to access array elements — destructuring and indexing (`result[0]`, `result[1]`). This violates "one way to do things."

**What to use instead:** Return an array and access elements by index:

```
result = divmod[7, 3]
a = result[0]
b = result[1]
```

## Design Note: No chars Builtin

Crow intentionally has no `chars` builtin to split a string into an array of characters.

**Why:** With string indexing (`s[0]`, `s[-1]`) and `for`-as-expression, splitting a string into characters is a one-liner:

```
chars = for i in range[len[s]]
    s[i]
```

Adding a dedicated `chars` builtin would create a second way to do the same thing.

## Design Note: No map Higher-Order Function

Crow intentionally has no `map[items, fn]` higher-order function.

**What was considered:** `map[items, double]` to apply a function to each element.

**Why it was rejected:**

1. **`for`-as-expression already does this.** `doubled = for x in items / x * 2` is mapping. Adding `map` creates a second way.
2. **Naming conflict.** `map[]` already creates an empty map. Overloading the name with a completely different concept (higher-order function application) would be confusing.
3. **Requires first-class functions.** `map[items, fn]` needs function references as arguments, which pulls toward lambdas and closures — features Crow intentionally omits.

**What to use instead:** `for`-as-expression:

```
doubled = for x in items
    x * 2
```

## Design Note: Named Arguments with Defaults (Revised)

Crow originally rejected default parameters because `greet['alice']` vs `greet['alice', 'hi']` created two ways to call the same function.

**Why the decision was revised:** Data science operations like `sum[x]` vs `sum[x, axis = 0]` are fundamentally different operations — one sums everything, the other sums along an axis. Forcing separate function names (`sum` vs `sum_axis`) or requiring the argument every time makes the language worse for its target audience. Every data science language (Python, Julia, MATLAB, R) has this.

**Rules:**
1. Named arguments must come **after** all positional arguments
2. Named arguments **must have defaults** — that's what makes them optional
3. Callers can use name or position: `sum[x, 0]` and `sum[x, axis = 0]` both work
4. Named arguments without a value at the call site use the default

```
fn train[data, lr = 0.01, epochs = 100]
    ...

train[my_data]                     --- uses defaults ---
train[my_data, lr = 0.001]         --- override one ---
train[my_data, 0.001, 50]          --- positional also works ---
```

## Design Note: No sum, min[arr], max[arr] in Core

Crow intentionally omits `sum[arr]`, `min[arr]`, and `max[arr]` from core builtins.

**What was considered:** `sum[[1, 2, 3]]` → `6`, `min[[3, 1, 2]]` → `1`, `max[[3, 1, 2]]` → `3`.

**Why they were removed/rejected:** All three are straightforward loops:

```
total = 0
for x in nums
    total = total + x
```

Including `sum` but not `min[arr]`/`max[arr]` created an inconsistency — all three are the same pattern (reduce an array to a single value). Rather than adding more, we removed `sum` to keep core honest: if you can write it with a loop, write it with a loop.

**What to use instead:** `use math` provides `sum`, `min`, `max`, `prod`, and other tensor/array reductions (see §10). For plain arrays without `use math`, use a loop.

## Design Note: No Ternary Expression

Crow intentionally has no ternary or inline conditional expression (e.g. `x = if val gt 0 then val else -val`).

**What was considered:** An expression-level `if/else` that returns a value inline.

**Why it was rejected:** Crow already has `if/else` as a statement. Adding an expression form creates two ways to write conditionals. The statement form is always clear and never ambiguous about precedence.

**What to use instead:** `if/else` with assignment:

```
x = 0
if val gt 0
    x = val
else
    x = -val
```

## Design Note: No Inline Error Fallback

Crow intentionally has no inline error recovery syntax (e.g. `x = int['bad'] rescue 0` or `x = int['bad'] or 0`).

**What was considered:** A keyword (`rescue`, `catch`, `alt`, `fail`, etc.) that silently replaces an error-value with a default, turning a 3-line check into a one-liner.

**Why it was rejected:** Inline fallbacks hide errors. The program continues with a value that may be wrong, and the error's `.reason` and `.trace` are silently discarded. This contradicts Crow's error philosophy — errors are values that propagate visibly, carrying context, until you consciously handle them.

Crow's errors don't crash the program. They propagate safely through function calls and operators:

```
x = int['bad']          --- error-value, no crash ---
y = x + 10              --- still error-value, no crash ---
log[y.ok]               --- false ---
log[y.reason]           --- original reason preserved ---
```

**What to use instead:** Check errors explicitly. The verbosity is intentional — it forces you to acknowledge the error:

```
x = int[input[]]
if not x.ok
    log['invalid: `x.reason`']
    x = 0
```

This may be revisited in the future if a design emerges that preserves error visibility while reducing verbosity.

## Background Work: later

A live Crow app is one loop serving every connected client; a script's poll
loop is one loop feeding everything downstream. Slow work on the loop
freezes everyone behind it. `later` is the one verb that moves work off the
loop:

```
fn load_book[]
    book = query[pg, 'SELECT symbol, pnl FROM trades']
    status = 'updated'

fn refresh[]
    status = 'loading'
    later[load_book]
```

| Form | Meaning |
|------|---------|
| `later[f]` | run `f` off the current loop; returns immediately |
| `later[f, a, b]` | scripts/servers: capture `a, b` by value, pass to `f` |
| `running[f]` | is a schedule of `f` still in flight? (spinners, disabled buttons) |

Semantics, in order of importance:

1. **State is the only mailbox.** In live apps the fn reads and writes
   reactive state; its writes are applied on the loop between renders, one
   schedule at a time — state never mutates concurrently, and landing
   writes re-render every client. In scripts (function-scope isolation
   applies) the fn communicates through its arguments and the database.
2. **Skip-if-running.** A second `later[f]` while `f` is in flight is a
   no-op. Double-clicks and overlapping polls cannot stack.
3. **Errors are values.** A failing query inside the fn lands in state as
   an ordinary error value; views render its `.reason`.
4. **Nothing vanishes at exit.** A script waits for in-flight work before
   ending.
5. **The interpreter runs the work inline.** `later` promises
   "eventually, atomically" — immediate is a legal schedule, so
   `crow run -i` and `crow test` are deterministic.

`later` is for responsiveness; `for` is for throughput. Crunching data is
parallel already — `later` exists so an interaction (a click, a tick) never
waits on slow work: remote queries, `shell` calls, model loads.

`later` is in-process and best-effort: if the process dies mid-flight, the
work is gone. Work that must survive a crash (placing an order) belongs in
a durable queue the fn drains — see the claim-semantics proposal in
`discovered-features.md`.

### Periodic Freshness and the Loop Budget

```
serve[3000, refresh = 5, budget = 50]
```

| Named arg | Meaning |
|-----------|---------|
| `refresh = N` | re-render every connected session every N seconds. Render-time queries re-run, so pages stay current with zero user code. Missed ticks are skipped, never stacked — a slow render coalesces. |
| `budget = M` | log any handler or render that takes more than M ms, with its name — the loop ships its own heartbeat. |

There is no `every[]`, no timer builtin, and no way to write periodic user
code. Periodicity is a property you declare on `serve`, not a loop you
write.

### Design Note: Periodicity Is Declared, Not Written

**Evidence:** in the same production system that motivated `later`, a 1s
status poll needed a hand-written in-flight guard so slow rounds couldn't
stack out-of-order updates, and a 3s news poll starved a 60s universe
refresh through a shared rate gate — four symptoms, one root cause: user
code owned the clock. Every hand-rolled poll loop eventually needs the
same three fixes (overlap guard, backoff, shutdown), and every codebase
writes them badly once first.

**What was considered:** an `every[N] { … }` block (rejected: it is the
poll loop with nicer clothes — overlap, starvation, and shutdown become
the user's problem again); a timer builtin (rejected: same, smaller).

**Why this design:** the runtime owns the clock, so no-overlap
(missed-tick skipping) and lifecycle come built in, and there is nothing
to hand-roll badly. The `while + sleep` poll shape remains for scripts —
`crow check` steers UI-shaped uses toward `refresh` (the stale-loop-read
finding). Deferred: per-source refresh intervals, backoff policy for
failing render queries.

### Design Note: The Loop Never Blocks

**Evidence:** a production trading scanner froze in premarket because a
~700ms model load and synchronous database reads ran directly on its event
loop — every websocket and poll stalled behind them. The fix (offload to
threads) exists in every ecosystem, and in every ecosystem it is *opt-in
discipline*: Python's `to_thread`, JS's workers, Go's `go`, Swift's
detached tasks. Discipline failed there and fails everywhere.

**What was considered:** async/await (rejected: function colors infect
every caller); a bare spawn verb with return values (rejected: Go's leak
and race classes arrive with it); nothing (rejected: `crow check` can
detect blocking handlers but the language must be able to express the
cure).

**Why this design:** Elixir/LiveView's mailbox model — background work
communicates by messages the loop applies serially — is the only surveyed
approach with no colors, no locks, and no shared-state races, and Crow's
reactive state is already exactly such a mailbox. Skip-if-running is built
in because the overlap guard is the first thing every production system
bolts on. The interpreter's inline schedule keeps the parity principle:
both modes satisfy the same contract, tests see the simplest legal
schedule. Deferred deliberately: automatic offload of slow handlers (the
no-verb endgame — needs the decision-span machinery), cancel-and-restart
for typeahead-shaped work, a render latency budget on `serve`, and durable
jobs.

## Appendix: Implementation Status

All core language features have interpreter–transpiler parity. The following items were identified during development and have been resolved:

| Feature | Status | Notes |
|---------|--------|-------|
| First-class function return types | ✅ Resolved | Transpiler correctly infers return types for variable-stored functions |
| For-as-expression rayon dependency | ✅ Resolved | `ForExpr` in assignments correctly sets `needs_rayon` |
| `while true` → `loop {}` | ✅ Resolved | Transpiler emits `loop {` instead of `while true {` |
| Modules (`use`) in transpiler | ✅ Resolved | Module code is flattened into generated Rust with prefixed names |
| Functions in collections type inference | ✅ Resolved | `arr[i]` and `map[k]` return correct element types in `infer_call_base` |
| Automatic statement-level parallelism | ✅ Resolved | Independent assignments grouped into `rayon::join` waves via dependency analysis |
| REPL (`crow` with no args) | ✅ Resolved | Interactive interpreter REPL |
| `crow new` | ✅ Resolved | Project scaffolding with --web, --ml, --db flags; replaces `crow init` |
| `crow run <job>` | ✅ Resolved | Named jobs from crowfile [jobs] section |
| `crow install` | ✅ Resolved | Package manager with registry, git, and local path support |

## Design Note: Map Arithmetic Requires All-Numeric Leaves

**Considered:** When doing arithmetic on maps (`model - lr * g`), what happens if a map contains non-numeric values like strings or bools?

Three options were evaluated:

1. **Error** — reject if any leaf is non-numeric
2. **Skip** — silently ignore non-numeric leaves
3. **Apply where possible** — multiply numbers, skip strings

**Decision:** Error. Map arithmetic requires every leaf to be numeric (tensor, int, float). If any leaf is a string, bool, or other non-numeric type, it's an error.

- **Compile time:** The transpiler checks map literal leaf types and rejects at compile time when types are statically known.
- **Runtime:** The interpreter checks before applying arithmetic and errors on non-numeric leaves.

```
--- works: all leaves are numeric ---
model = map[w = [1, 2, 3], b = [4, 5, 6]]
result = 2 * model
```

Since maps are homogeneous (Design Note: Maps Are Dictionaries, Not Records),
a map mixing numeric weights with string config can no longer be constructed
at all — the question resolves at the type level.

**What to use instead:** Weights in a map (all tensors), config in a struct.

```
struct Config [lr, optimizer]

weights = map[w = rand[784, 128], b = zeros[128]]
config = Config[0.01, 'adam']

g = dloss[weights, x, y]
weights = weights - config.lr * g
```

**Why:** Silent skipping hides bugs. "Apply where possible" is ambiguous — does `2 * 'adam'` skip or error? Every other language (JAX, Julia/Flux, PyTorch, Mojo) either errors or requires explicit marking of trainable fields. Erroring is the simplest approach and matches Crow's philosophy of no hidden behavior. Separating weights from config is good practice regardless.

## Design Note: Tensor Arithmetic Operators

Crow uses MATLAB/Julia-style operators for tensor arithmetic. Plain operators are matrix operations; dot-prefixed operators are element-wise.

| Operation | Operator | Meaning |
|---|---|---|
| `*` | matrix multiply | `W * x` — matmul |
| `.*` | element-wise multiply | `A .* mask` — Hadamard product |
| `/` | matrix divide | `A / B` |
| `./` | element-wise divide | `values ./ total` |
| `^` | scalar power | `x ^ 2` |
| `.^` | element-wise power | `x .^ 2` |

`+` and `-` are already element-wise for matrices (adding two matrices is always element-wise). No dot versions needed.

```
h = W * x + b               --- matmul ---
scaled = W .* mask           --- element-wise multiply ---
normed = values ./ total     --- element-wise divide ---
squared = x .^ 2             --- element-wise power ---
```

**Shift-key tradeoff:** `^` and `.^` require the shift key, which normally violates Crow's principle #3 (shift-key minimization). This tradeoff is accepted because data scientists type power operations constantly, and `pow[x, 2]` is too clunky. There is no non-shift alternative for power that isn't worse.

**Why not `matmul[]` builtin?** `W * x + b` is what every data scientist expects. A `matmul[W, x] + b` builtin adds noise to the most common operation in ML. `*` for matmul follows MATLAB and Julia — the two most established scientific computing languages.

**Lexing:** `.*`, `./`, and `.^` are lexed as single tokens (`DotStar`, `DotSlash`, `DotCaret`), not `Dot` + `Star`. This avoids ambiguity with field access (`model.w`).

## Design Note: Unified Arrays (No Separate List and Tensor Types)

**What was considered:** Having separate `list` (heterogeneous, general) and `tensor` (homogeneous, numeric, N-dimensional) types, with `tensor[[1, 2, 3]]` as the creation syntax.

**Decision:** One type — `array`. `[1, 2, 3]` is an array. Arrays are homogeneous (all elements same type). Nested brackets create multidimensional arrays: `[[1, 2], [3, 4]]`. No `tensor` keyword.

```
--- arrays ---
[1, 2, 3]                    --- int array ---
['a', 'b', 'c']              --- str array ---
[[1, 2], [3, 4]]             --- 2D int array ---
zeros[3, 4]                  --- 3x4 of zeros ---
rand[2, 3]                   --- 2x3 random ---

--- mixed named data → use a struct ---
struct User [name, age]
User['alice', 25]            --- named, mixed-type fields ---

--- error ---
[1, 'hello', true]           --- error: mixed types ---
```

**Why:** Python has both `list` and `numpy.array` — the "two type problem." You constantly convert between them. MATLAB, Julia, and R have one array type and it works. For data science, columns are homogeneous arrays. For tabular data, use a **frame** (`use frame`) — the dedicated type with named, per-column-typed columns, missing-value tracking, and relational operations.

**Tensors vs frames:** a tensor is positional numbers (one dtype, axes interchangeable — matmul, transpose, reductions); a frame is named observations (typed columns, filter/sort/group/join). An all-numeric frame and a matrix can hold the same grid, but intent picks the type — convert deliberately with `tensor[df]`. Neither grows the other's features: no named axes on tensors, no linear algebra on frames.

**Mixed-type data uses structs, not arrays.** `struct User [id, name, active]` then `User[1, 'alice', true]` gives named, typed fields with dot access. This is clearer than a mixed-type list anyway — fields are named and self-documenting. (Maps are homogeneous — see Design Note: Maps Are Dictionaries, Not Records.)

**What this eliminates:** `tensor` keyword, `list` vs `tensor` confusion, `tensor[[1,2,3]]` wrapper syntax, mixed-type arrays.
## Design Note: Names Are Never Ambiguous

A bare name in Crow means exactly one thing. Collisions are never resolved silently — not by shadowing, not by precedence, not by context. This is one rule applied at every level of the language:

**1. Definitions cannot take reserved names.** A variable, function, view, struct, parameter, or loop variable that collides with a name the language or an imported module owns is a compile error at the definition line:

```
range = [10, 20, 30]     --- error: 'range' is a core builtin
use math
sum = 0                  --- error: 'sum' is provided by math (line 2)
e = 100                  --- error: 'e' is a math constant
math = 5                 --- error: 'math' is a module name
```

Every error names the source and both fixes: rename your definition, or import the module qualified (`use math as m`).

**2. Three namespace tiers.** *Core* (~70 names: `log`, `len`, `range`, `map`, `set`, `type`…) is reserved in every file, frozen by the budget rule — core does not grow, so this reservation never breaks existing programs. *Stdlib modules* (`math`, `frame`, `io`, `db`, `web`, `ui`) reserve their exports only in files that import them bare. *Everything else* — user modules today, registry packages tomorrow — is **qualified-only**: the import reserves exactly one name (the prefix), and the module can export ten thousand names without taking any of yours. The language owns the bare namespace; packages get prefixes.

**3. Aliased imports are qualified-only** (`use math as m` — see Aliased Imports). This is the escape valve when you want a name a module owns: rename what you own, or alias what you don't.

**4. Fields are exempt.** Struct fields, map keys, JSON fields, and frame columns live inside their containers, not in the file's namespace — `account.sum`, `row.type`, `df.mean` are always fine. The one exception: the four error-metadata names (`ok`, `reason`, `kind`, `trace`) are a fixed vocabulary on *every* value, so struct fields may not use them.

**5. Vocabulary zones are language-owned.** `view` blocks activate the HTML element vocabulary — the only zone, granted because HTML is an external, universal, unchangeable vocabulary that could not be renamed, reserved, or qualified without wrecking either learnability or ordinary code. User modules cannot define zones (that mechanism is a macro system, and Crow rejects macros); they export functions and views instead, reached by prefix — exactly how JSX fixes lowercase tags while components are imported references.

**Why not shadowing (Python/Go-style)?** Because Crow's brackets unify calling and indexing, shadowing produces *silent wrong answers*: `range = [10, 20, 30]` then `range[0]` — delete the assignment during a refactor and the same expression silently becomes `range(0)` = `[]`. MATLAB, the one mainstream language sharing this syntax collision, chose shadowing — and it has been a documented footgun for forty years. Languages where shadowing is safe (Python, Go, Julia) are exactly the ones whose call syntax is distinct from indexing. Crow's is not, so a collision must be an error, at the earliest possible moment, with the fix in the message.

**Why this survives growth:** every new stdlib export visibly costs a name taken from every importer, which enforces the budget rule structurally; packages can never collide with anything; and `crow fmt --fix-collisions`-style migrations handle the rare stdlib addition that lands on someone's variable.
