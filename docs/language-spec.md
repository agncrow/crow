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

Variables can be reassigned freely, including to a different type.

```
x = 10
x = 'hello'
```

### Types

Crow is dynamically typed. Types are inferred at runtime. The built-in types are:

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

Functions are declared with `fn`. Use explicit `return`. Functions without `return` have status `'done'`.

```
fn greet[name]
    log['hello, {name}']    --- no return, status is 'done' ---

fn add[a, b]
    return a + b             --- returns value, status is 'ok' ---
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

### Empty Containers

`[]` is always an empty list. Use `map[]` for an empty map.

```
items = []               --- empty list ---
data = map[]             --- empty map ---
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

## 6. Pattern Matching

```
match value
    0
        log['zero']
    1..10
        log['small']
    _
        log['big']
```


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

### File Extension

Crow source files use the `.crow` extension.

```
main.crow
myapp/utils.crow
myapp/worker.crow
```

### CLI

```
crow init                   --- create new project with crowfile ---
crow build                  --- compile .crow files to bytecode, check for errors ---
crow run                    --- build + run ---
```

`crow build` compiles `.crow` source files to bytecode and reports any errors. `crow run` builds and then executes the bytecode.