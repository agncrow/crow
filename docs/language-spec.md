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

Variables can be reassigned freely.

```
x = 10
x = 20
```

### Types

Crow is statically typed with full type inference — you never write type annotations, but every variable has a concrete type determined at compile time. The transpiler infers types from usage; if a type cannot be determined, the program is invalid.

The built-in types are:

| Type   | Example              |
|--------|----------------------|
| int    | `42`, `-7`, `0`      |
| float  | `3.14`, `-0.5`       |
| str    | `'hello'`, `'crow'`  |
| bool   | `true`, `false`      |
| list   | `[1, 2, 3]`          |
| map    | `['name' = 'crow', 'age' = 3]` |

### String Interpolation

Strings support inline expressions using `{}`.

```
name = 'crow'
greeting = 'hello, {name}'
result = '1 + 2 = {1 + 2}'
```

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

Variables are scoped to the block they are defined in. Inner blocks can access outer variables.

```
x = 10
if true
    y = 20
    log[x]    --- 10 — accessible ---

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
    log['hello, {name}']    --- no return value, status is 'done' ---

fn safe_div[a, b]
    if b eq 0
        return 0             --- early return ---
    a / b                    --- implicit return ---
```

### Calling

Functions are called using `[]`.

```
greet['crow']
result = add[3, 4]
```

### List Access

Lists are functions from index to value.

```
numbers = [1, 2, 3]
numbers[0]               --- 1 ---
```

### Map Access

Maps are functions from key to value.

```
person = ['name' = 'crow', 'age' = 3]
person['name']           --- 'crow' ---
```

### First-Class Functions

A function name without `[]` evaluates to the function itself. Functions can be passed as arguments, stored in lists and maps, and called later with `[]`.

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

### Empty Containers

`[]` is always an empty list. Use `map[]` for an empty map.

```
items = []               --- empty list ---
data = map[]             --- empty map ---
```

### Built-in Functions

#### Core

| Function | Description |
|----------|-------------|
| `log[value]` | Print a value to stdout |
| `range[stop]` | List of ints from 0 to stop (exclusive) |
| `range[start, stop]` | List of ints from start to stop (exclusive) |
| `range[start, stop, step]` | List of ints with custom step |
| `map[]` | Create an empty map |
| `assert[cond]` | Halt with error if condition is falsy |
| `assert[cond, msg]` | Halt with custom error message |
| `cpu[n]` | Set thread count for parallel loops |
| `gpu[n]` | Set GPU count for parallel compute |

#### Math

| Function | Description |
|----------|-------------|
| `abs[n]` | Absolute value |
| `sqrt[n]` | Square root (returns float) |
| `floor[n]` | Floor (returns int) |
| `ceil[n]` | Ceiling (returns int) |
| `round[n]` | Round to nearest int |
| `pow[base, exp]` | Exponentiation |
| `min[a, b]` | Minimum of two numbers |
| `max[a, b]` | Maximum of two numbers |

#### Strings

| Function | Description |
|----------|-------------|
| `split[s, delim]` | Split string into list |
| `trim[s]` | Remove leading/trailing whitespace |
| `replace[s, old, new]` | Replace all occurrences |
| `upper[s]` | Convert to uppercase |
| `lower[s]` | Convert to lowercase |
| `starts[s, prefix]` | Check if string starts with prefix |
| `ends[s, suffix]` | Check if string ends with suffix |
| `join[list, sep]` | Join list elements into string |

#### Conversion

| Function | Description |
|----------|-------------|
| `int[value]` | Convert to int (error-value on failure) |
| `float[value]` | Convert to float (error-value on failure) |
| `str[value]` | Convert to string |

#### Collections

| Function | Description |
|----------|-------------|
| `len[x]` | Length of list, string, or map |
| `push[list, value]` | Append to list (mutates) |
| `pop[list]` | Remove and return last element (mutates) |
| `append[list, other]` | Extend list with another list (mutates) |
| `sort[list]` | Sort list in place (mutates) |
| `reverse[list]` | Reverse list in place (mutates) |
| `contains[list, value]` | Check if list contains value |
| `slice[list, start, end]` | Extract sublist |

#### Map Operations

| Function | Description |
|----------|-------------|
| `keys[m]` | List of map keys |
| `values[m]` | List of map values |
| `has[m, key]` | Check if map contains key |
| `set[m, key, value]` | Set map entry (mutates) |
| `del[m, key]` | Delete map entry (mutates) |

#### I/O

| Function | Description |
|----------|-------------|
| `read[path]` | Read file contents (error-value on failure) |
| `write[path, content]` | Write string to file (error-value on failure) |
| `args[]` | CLI arguments as list of strings |
| `input[]` | Read a line from stdin |
| `input[prompt]` | Print prompt, then read a line from stdin |

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

### for as Expression

Assign a `for` loop to a variable to collect results into a list.

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

`while` loops run sequentially.

```
while x gt 0
    x = x - 1
```

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
    log['{bird.name} flies at {bird.speed}mph']

b = Bird['crow', 50]
fly[b]
```


## 8. Operators

### Arithmetic

```
+  -  /  *  %
```

`+` also concatenates strings:

```
'hello' + ' world'    --- 'hello world' ---
```

Division is type-determined — one operator, no ambiguity:

```
3 / 2        --- 1 (both int → int, floor division)
3 / 2.0      --- 1.5 (float involved → float)
7 / 3        --- 2
7.0 / 3      --- 2.3333...
```

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


## 9. Modules & Imports

### Importing

Use `use` to import a module. The entire module is imported — no selective imports.

```
use math
use myapp/utils
```

### Accessing

Access module functions with `.`

```
use math

result = math.sqrt[16]    --- 4 ---
```

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

Comments start with `---` and end with `---`. Used for both single and multi-line.

```
--- single line comment ---

x = 5  --- inline comment ---

---
multi line
comment
---
```


## 11. Error Handling

Every variable in Crow is a struct with `status`, `result`, and `reason`. Values auto-unwrap — you use them directly without worrying about the struct.

### Every Value is a Struct

```
x = 5
log[x]            --- 5, auto-unwraps ---
x.status          --- 'ok' ---
x.result          --- 5 ---
```

### Auto Unwrap

```
content = read['file.txt']
log[content]               --- just works, content unwraps to the result ---
```

If `read` fails, the error propagates automatically up the call chain. No try/catch needed.

### Explicit Handling

Check `.status` only when you want to handle errors yourself.

```
content = read['file.txt']

if content.status eq 'error'
    log['failed: {content.reason}']
else
    log[content.result]
```

### Value Struct

Every value has:

| Field    | Description                          |
|----------|--------------------------------------|
| `status` | `'ok'`, `'done'`, or `'error'`      |
| `result` | the return value (undefined on error or done)|
| `reason` | error message (undefined on ok or done)     |

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

`for` loops run iterations in parallel by default.

```
for x in items
    process[x]        --- each iteration runs in parallel ---
```

### While Loops

`while` loops run sequentially.

```
while x gt 0
    x = x - 1
```

### Resource Control

By default, Crow uses all available CPU cores. Use `cpu` or `gpu` to limit or control resources.

#### Function-level

```
fn heavy_work[]
    cpu[4]
    for x in items
        process[x]           --- 4 cores ---

fn gpu_work[]
    gpu[2]
    for x in items
        compute[x]           --- 2 GPUs ---
```

#### Global

Set program-wide defaults in the `crowfile`. Any function can override locally.

```
--- crowfile ---

cpu = 4
gpu = 1
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

## 13. Project Configuration

### crowfile

Every Crow project has a `crowfile` at the root. It uses Crow syntax.

```
--- crowfile ---

name = 'my-project'
entry = 'main.crow'
cpu = 4
gpu = 1

deps = ['math' = '1.0', 'http' = '2.3']
```

Dependencies can be:

| Format | Example | Description |
|--------|---------|-------------|
| Version | `'math' = '1.0'` | Registry package (resolved via central index) |
| Git URL | `'math' = 'https://github.com/user/crow-math'` | Cloned from git |
| Local path | `'utils' = './libs/utils'` | Symlinked from local directory |

### File Extension

Crow source files use the `.crow` extension.

```
main.crow
myapp/utils.crow
myapp/worker.crow
```

### CLI

```
crow                        --- start interactive REPL ---
crow init                   --- create new project with crowfile ---
crow install                --- install deps from crowfile into crow_modules/ ---
crow build                  --- transpile to Rust and compile to native binary ---
crow build file.crow        --- transpile a specific file ---
crow run                    --- transpile + compile + run (uses crowfile) ---
crow run file.crow          --- transpile + compile + run a specific file ---
crow run -i file.crow       --- run with interpreter (for debugging) ---
```

`crow build` transpiles `.crow` source files to Rust and compiles to a native binary. `crow run` builds and then executes the binary.

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
crow init
--- edit crowfile to add deps ---
crow install
crow run
```

**Registry packages** (version strings like `'1.0'`) are resolved via a central index hosted on GitHub. The registry maps package names to git repositories and available versions. When installing, Crow fetches the index, finds the repo URL, and clones it at the matching git tag (`v1.0`, `v2.3`, etc.). If the exact tag is not found, the latest commit is used with a warning.

**Git dependencies** are cloned directly from the URL. **Local path dependencies** are symlinked.

Installed modules are imported with `use`.

#### Registry

The default registry index is hosted at `crow-lang/registry` on GitHub. Override it with the `CROW_REGISTRY` environment variable:

```bash
CROW_REGISTRY=https://example.com/index.json crow install
```

The index is cached locally at `~/.crow/registry/index.json` and refreshed on each `crow install`.

## 14. Type System

Crow is **statically typed with inference**. No type annotations are needed — the transpiler infers concrete types from how values are used.

### Rules

- Every variable has exactly one type throughout its scope
- Types are inferred from assignment: `x = 5` makes `x` an `int`
- Function parameter and return types are inferred from call sites
- If the transpiler cannot determine a type, the program is invalid

### Execution Modes

| Mode | Command | Role |
|------|---------|------|
| Transpiler | `crow run file.crow` | **Primary.** Compiles to native Rust. This is the authority on valid Crow programs. |
| Interpreter | `crow run -i file.crow` | **Dev tool.** Runs code directly for quick iteration. Slower, but convenient. |
| REPL | `crow` | **Interactive.** Explore and test code interactively. Uses the interpreter. |

The transpiler is the source of truth. If a program works in the interpreter but fails the transpiler, the program has a type error and should be fixed. The interpreter is internally dynamic for convenience, but users should write code that the transpiler can handle.

### No `none`

Crow has no null, nil, or none value. Every variable always holds a concrete value. Missing or failed data is represented through the error-value system (`status`, `result`, `reason`) — see Error Handling.