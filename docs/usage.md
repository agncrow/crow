# Crow User Guide

Crow is a simple, expressive programming language that compiles to native code via Rust. It features minimal syntax, automatic parallelism, and two execution modes: a fast transpiler for production and an interpreter for development.

## Installation

### From Source

```bash
git clone https://github.com/your-repo/crow.git
cd crow
```

**Build** (compile in release mode, binary stays in the project):

```bash
cargo build --release
```

Binary output: `target/release/crow`

**Install** (copy binary to Cargo bin directory, available globally):

```bash
cargo install --path .
```

Binary installed to: `~/.cargo/bin/crow`

Make sure `~/.cargo/bin/` is in your `PATH`.

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

### `crow new <name>`

Create a new Crow project. Generates a `crowfile` and `main.crow` with working starter code.

```bash
crow new myapp
cd myapp
crow run
```

Add flags to include specific capabilities:

```bash
crow new myapp --web            # adds views/home.crow, wires page[] route in main.crow
crow new myapp --ml             # adds jobs/train.crow, lib/model.crow
crow new myapp --db             # adds db/schema.crow, jobs/migrate.crow
crow new myapp --web --ml --db  # all of the above
```

Every generated file has real working code — `crow run` works immediately after `crow new`.

### `crow run` (no file)

Run the project defined by the `crowfile` in the current directory. Runs `main.crow` by default.

```bash
crow run
```

### `crow run <job>`

Run a named job defined in the `crowfile`'s `[jobs]` section.

```
--- crowfile ---
name = 'myapp'

[jobs]
train = 'jobs/train.crow'
migrate = 'jobs/migrate.crow'
```

```bash
crow run train        # runs jobs/train.crow
crow run migrate      # runs jobs/migrate.crow
```

Jobs are batch tasks — they run and exit. `main.crow` is the long-running entry point (e.g., web server).

### `crow install`

Install dependencies listed in the `crowfile` into `crow_modules/`.

```
--- crowfile ---
name = 'myapp'
deps = map[math = '1.0', utils = 'https://github.com/user/crow-utils']
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

The default registry is the git repo at `https://github.com/vsriram/registry.git`. It is cloned to `~/.crow/registry/` on first install and updated via `git pull` on subsequent runs.

To use a custom registry, set the `CROW_REGISTRY` environment variable:

```bash
CROW_REGISTRY=https://github.com/myorg/my-registry.git crow install
```

## Project Structure

A Crow program can be a single file or a full project directory. You never need to restructure as the project grows — just add files.

### Single File

Any `.crow` file is a valid program:

```
--- hello.crow ---
log['hello, world']
```

```bash
crow run hello.crow
```

### Full Project

Use `crow new` to create a project:

```bash
crow new finch --web --ml --db
```

```
finch/
  crowfile                    # project config + job definitions
  main.crow                   # default entry point (crow run)

  jobs/                       # batch tasks (crow run <name>)
    train.crow                # crow run train
    migrate.crow              # crow run migrate

  views/                      # web views — imported and wired in main.crow
    home.crow                 # use views.home → page['/', home_render, home_handle]
    dashboard.crow            # use views.dashboard → page['/dashboard', ...]

  components/                 # reusable UI functions
    nav.crow

  layout.crow                 # shared UI wrapper for all pages

  lib/                        # your code — organized however you want
    accounts.crow
    ml/
      features.crow
      model.crow

  db/
    schema.crow               # table definitions (auto-discovered)

  tests/                      # integration tests (crow test)
    accounts_test.crow
```

### Conventions

| Folder/File | What it does | Magic? |
|---|---|---|
| `main.crow` | Default entry point. `crow run` runs this | Convention |
| `views/` | Web views. Import with `use views.home` and wire with `page[]` | Explicit |
| `layout.crow` | Wraps all pages in shared UI | Auto-applied |
| `db/schema.crow` | Table definitions | Auto-discovered |
| `jobs/` | Batch tasks. `crow run train` | Listed in crowfile |
| `tests/` | Integration tests. `crow test` | Auto-discovered |
| `lib/` | Shared code | Just files — import with `use` |
| `components/` | UI functions | Just files — import with `use` |

Rule: **`main.crow` stays running (web server). Jobs run and exit (training, migrations).**

The user is free to organize code however they want — the conventions are shortcuts, not requirements. You can define routes manually, write SQL directly, or put code anywhere and import it with `use`.

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
| array  | `[1, 2, 3]`                      |
| map    | `map[name = 'crow', count = 3]`  |

### String Interpolation

Use paired backticks inside strings to embed expressions:

```
name = 'crow'
log['hello, `name`']
log['1 + 2 = `1 + 2`']
```

### Functions

Functions are declared with `fn` and called with `[]`:

```
fn greet[name]
    log['hello, `name`']

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
    log['`bird.name` flies at `bird.speed`mph']

b = Bird['crow', 50]
fly[b]
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
log['sequential: `seq`']

cpu[8]
par = for limit in tasks
    count_primes[limit]
log['parallel: `par`']
```

### Error Handling

Every value in Crow carries error metadata: `.ok`, `.reason`, `.kind`, `.trace`. Values auto-unwrap so you use them directly.

```
x = 5
log[x]            --- 5, auto-unwraps
x.ok              --- true
x.reason          --- ''
```

Check `.ok` when you want explicit error handling:

```
content = read['file.txt']
if not content.ok
    log['failed: `content.reason`']
else
    log[content]
```

Errors have no truth value: comparing or branching on an error-value propagates it (inside a function) or halts (at top level) — it never silently becomes `false`. See the spec's "Errors in Conditions".

### Comments

Comments run from `---` to the end of the line — no closing delimiter:

```
--- this is a comment
x = 5  --- inline comment

--- multi line comments
--- are per-line
```

### Arrays

```
items = [1, 2, 3]
items[0]             --- 1
len[items]           --- 3
grown = push[items, 4]   --- [1, 2, 3, 4]
```

### Maps

Maps hold homogeneous values and are built with the `map[...]` constructor:

```
data = map[name = 'crow', kind_of = 'bird']
data['name']         --- 'crow'
data.name            --- dot access works too
empty = map[]
```

(For mixed named fields, use a struct — `struct Bird [name, speed]`.)

### Import Aliases

Any import can be aliased — the module then lives behind the alias only (Python `import x as y` semantics), freeing its bare names:

```
use math as m

sum = 0              --- legal: math's names were never dumped
t = m.sum[[1, 2, 3]]
```

Names you define may not collide with core builtins or a bare import's exports — the compiler errors at the definition line with the fix (rename, or alias the import). See the spec's Design Note "Names Are Never Ambiguous".

### Views (web UI)

UI trees are built in `view` blocks — inside a view the HTML element vocabulary (`div`, `h1`, `button`, ...) is active; outside views those names are free for your own code:

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

## Examples

### Fibonacci

```
fn fib[n]
    if n lte 1
        return n
    return fib[n - 1] + fib[n - 2]

log['fib[10] = `fib[10]`']
log['fib[20] = `fib[20]`']
log['fib[30] = `fib[30]`']
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