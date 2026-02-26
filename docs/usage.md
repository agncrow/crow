# Crow User Guide

Crow is a simple, expressive programming language that compiles to native code via Rust. It features minimal syntax, automatic parallelism, and two execution modes: a fast transpiler for production and an interpreter for development.

## Installation

### From Source

```bash
git clone https://github.com/your-repo/crow.git
cd crow
cargo install --path .
```

This installs the `crow` binary to `~/.cargo/bin/`. Make sure it's in your `PATH`.

### Verify Installation

```bash
crow
```

You should see the usage help.

## Quick Start

Create a file called `hello.crow`:

```
log['hello, world']
```

Run it:

```bash
crow run hello.crow
```

This transpiles your Crow code to Rust, compiles it to a native binary, and runs it — all in one step.

## CLI Commands

### `crow run file.crow`

Transpile → compile → run. This is the primary way to run Crow programs. The output binary runs at native Rust speed.

```bash
crow run hello.crow
crow run examples/bench_fibonacci.crow
```

### `crow run -i file.crow`

Run with the interpreter. Slower, but useful for debugging and development.

```bash
crow run -i hello.crow
```

### `crow build file.crow`

Transpile and compile to a native binary without running it. The binary is placed in the current directory.

```bash
crow build hello.crow
./hello          # run the compiled binary directly
```

### `crow init`

Create a new Crow project with a `crowfile` and `main.crow`.

```bash
mkdir myproject
cd myproject
crow init
crow run
```

### `crow run` (no file)

Run the project defined by the `crowfile` in the current directory.

```bash
crow run
```

### `crow install`

Install dependencies listed in the `crowfile` into `crow_modules/`.

```
--- crowfile ---
name = 'myapp'
entry = 'main.crow'
deps = ['math' = '1.0', 'utils' = 'https://github.com/user/crow-utils']
```

```bash
crow install
```

Three dependency formats are supported:

| Format | Example | How it works |
|--------|---------|--------------|
| Version | `'math' = '1.0'` | Resolved from the package registry, cloned at git tag `v1.0` |
| Git URL | `'lib' = 'https://...'` | Cloned directly from the git URL |
| Local path | `'utils' = './libs/utils'` | Symlinked into `crow_modules/` |

After installing, import with `use`:

```
use math
log[math.double[5]]
```

To use a custom registry, set the `CROW_REGISTRY` environment variable:

```bash
CROW_REGISTRY=https://example.com/index.json crow install
```

## Language Basics

### Variables

Variables are declared by assigning a value. No keywords needed.

```
name = 'crow'
age = 3
pi = 3.14
active = true
```

Multiple assignment and swap:

```
x, y, z = 1, 2, 3
a, b = b, a
```

### Types

| Type   | Example                          |
|--------|----------------------------------|
| int    | `42`, `-7`, `0`                  |
| float  | `3.14`, `-0.5`                   |
| str    | `'hello'`, `'crow'`              |
| bool   | `true`, `false`                  |
| list   | `[1, 2, 3]`                      |
| map    | `['name' = 'crow', 'age' = 3]`  |

### String Interpolation

Use `{}` inside strings to embed expressions:

```
name = 'crow'
log['hello, {name}']
log['1 + 2 = {1 + 2}']
```

### Functions

Functions are declared with `fn` and called with `[]`:

```
fn greet[name]
    log['hello, {name}']

fn add[a, b]
    return a + b

greet['crow']
result = add[3, 4]
log[result]
```

Functions without `return` return `done`. Functions with `return` return the value with status `ok`.

### Control Flow

#### if / else

```
if x gt 5
    log['big']
else if x gt 2
    log['medium']
else
    log['small']
```

#### for loops

`for` loops iterate over lists and run in parallel by default:

```
for x in [1, 2, 3]
    log[x]
```

Assign a `for` to a variable to collect results (like map/filter):

```
doubled = for x in [1, 2, 3]
    x * 2
--- doubled = [2, 4, 6] ---
```

#### while loops

`while` loops run sequentially:

```
i = 0
while i lt 10
    log[i]
    i = i + 1
```

### Operators

#### Arithmetic
```
+  -  *  /  %
```

Division is type-determined:

```
3 / 2        --- 1 (both int, floor division) ---
3 / 2.0      --- 1.5 (float involved, float result) ---
```

#### Comparison (word-based, no symbols)

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

#### Logical

```
and  or  not
```

```
if x gt 0 and y lt 10
    log['in range']
if not active
    log['inactive']
```

### Structs

Structs are pure data — no methods.

```
struct Bird [name, speed]

b = Bird['crow', 50]
log[b.name]          --- 'crow' ---
log[b.speed]         --- 50 ---
```

Pass structs to functions:

```
struct Bird [name, speed]

fn fly[bird]
    log['{bird.name} flies at {bird.speed}mph']

b = Bird['crow', 50]
fly[b]
```

### Pattern Matching

```
match value
    0
        log['zero']
    1..10
        log['small']
    _
        log['other']
```

### Modules

Each `.crow` file is a module. Import with `use`:

```
--- file: mymath.crow ---
fn double[x]
    return x * 2
```

```
--- file: main.crow ---
use mymath

result = mymath.double[5]
log[result]                  --- 10 ---
```

Subdirectory modules:

```
use myapp/utils
result = utils.helper[42]
```

### Parallelism

Crow is parallel by default. `for` loops automatically run iterations in parallel.

#### Control thread count with `cpu[n]`

```
--- use 8 threads for this for loop ---
cpu[8]
results = for x in items
    heavy_work[x]

--- use 1 thread (sequential) ---
cpu[1]
for x in items
    process[x]
```


#### Example: parallel vs sequential

```
fn count_primes[limit]
    count = 0
    n = 2
    while n lte limit
        is_prime = true
        d = 2
        while d * d lte n
            if n % d eq 0
                is_prime = false
            d = d + 1
        if is_prime
            count = count + 1
        n = n + 1
    return count

tasks = [15000, 12000, 18000, 10000]

cpu[1]
seq = for limit in tasks
    count_primes[limit]
log['sequential: {seq}']

cpu[8]
par = for limit in tasks
    count_primes[limit]
log['parallel: {par}']
```

### Error Handling

Every value in Crow has `status`, `result`, and `reason` fields. Values auto-unwrap so you use them directly.

```
x = 5
log[x]            --- 5, auto-unwraps ---
x.status          --- 'ok' ---
x.result          --- 5 ---
```

Check `.status` when you want explicit error handling:

```
content = read['file.txt']
if content.status eq 'error'
    log['failed: {content.reason}']
else
    log[content.result]
```

### Comments

Comments use `---`:

```
--- this is a comment ---
x = 5  --- inline comment ---

---
multi line
comment
---
```

### Lists

```
items = [1, 2, 3]
items[0]             --- 1 ---
len[items]           --- 3 ---
push[items, 4]       --- [1, 2, 3, 4] ---
```

### Maps

```
data = ['name' = 'crow', 'age' = 3]
data['name']         --- 'crow' ---
empty = map[]
```

## Examples

### Fibonacci

```
fn fib[n]
    if n lte 1
        return n
    return fib[n - 1] + fib[n - 2]

log['fib[10] = {fib[10]}']
log['fib[20] = {fib[20]}']
log['fib[30] = {fib[30]}']
```

### Hello World

```
log['hello, world']
```

## Execution Modes

| Mode | Command | Speed | Use Case |
|------|---------|-------|----------|
| Transpiled | `crow run file.crow` | Native (Rust speed) | Production, benchmarks |
| Interpreter | `crow run -i file.crow` | Slower (~100x) | Debugging, development |
| Build only | `crow build file.crow` | — | Produce distributable binary |

The transpiler infers concrete types from your Crow code and generates optimized Rust. Parallel `for` loops with `cpu[n]` are compiled to use Rayon for real multi-threaded execution.