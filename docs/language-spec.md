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
| map    | `[name = 'crow', age = 3]` |

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

```
log['line1\nline2']
log['col1\tcol2']
log['it\'s here']
```

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
    log['hello, {name}']

fn safe_div[a, b]
    if b eq 0
        return 0             --- early return ---
    a / b                    --- implicit return ---
```

### Named Arguments with Defaults

Parameters with `= value` are named arguments with default values. Named arguments must come after all positional arguments.

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
person = [name = 'crow', age = 3]
person.name              --- 'crow' ---
person['name']           --- 'crow' (string key also works) ---
```

Bare-name keys are stored as strings internally. Use quoted string keys when the key is dynamic or contains special characters:

```
data = ['content-type' = 'json']
key = 'name'
person[key]              --- dynamic lookup ---
```

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

### Empty Containers

`[]` is always an empty array. Use `map[]` for an empty map.

```
items = []               --- empty array ---
data = map[]             --- empty map ---
```

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
| `pad[s, n]` | Right-pad string to width `n` with spaces |
| `reverse[s]` | Reverse a string |

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
| `append[arr, other]` | Return new array with other's elements added |
| `sort[arr]` | Return new sorted array |
| `reverse[arr]` | Return new reversed array |
| `contains[arr, value]` | Check if array contains value |
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
| `args[]` | CLI arguments as array of strings |
| `input[]` | Read a line from stdin (error-value on failure) |
| `input[prompt]` | Print prompt, then read a line (error-value on failure) |

#### JSON

| Function | Description |
|----------|-------------|
| `json[str]` | Parse JSON string into a Crow value (error-value on failure) |
| `encode[value]` | Encode a Crow value as a JSON string |

JSON type mapping: objects → maps, arrays → arrays, strings → str, numbers → int/float, booleans → bool, `null` → error-value. JSON `null` maps to an error-value with `ok: false` and `reason: 'null'`, which auto-propagates through the error-value system (§11).

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
| `env[name]` | Get environment variable (empty string if not set) |
| `exit[]` | Exit with code 0 |
| `exit[code]` | Exit with given code |

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

> **Design Note:** `break` and `continue` are only valid in `while` loops. `for` loops are parallel by default (§12), and `break`/`continue` are inherently sequential concepts. Using `break` in a parallel loop would silently force sequential execution — a hidden performance trap. Use `if` inside `for` to skip items, or use `while` when you need early exit.

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
W = math.eye[2]
x = math.ones[2, 1]
y = W * x                   --- matmul ---
a = math.ones[3] .* math.full[3, 2]   --- element-wise multiply ---
b = math.ones[3] ./ math.full[3, 2]   --- element-wise divide ---
c = math.full[3, 2] .^ 3    --- element-wise power ---
d = math.ones[3] * 2        --- scalar multiply ---
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

### Built-in Module: `math`

The `math` module is built in — no file needed. It provides tensor creation, element-wise operations, reductions, shape operations, and linear algebra.

#### Tensor Creation

| Function | Description |
|---|---|
| `math.zeros[d1, d2, ...]` | Tensor of zeros with given shape |
| `math.ones[d1, d2, ...]` | Tensor of ones with given shape |
| `math.full[d1, d2, ..., value]` | Tensor filled with `value` (last arg) |
| `math.eye[n]` | n×n identity matrix |
| `math.diag[tensor]` | Extract diagonal from a 2D tensor |
| `math.linspace[start, end, n]` | `n` evenly spaced values from `start` to `end` |
| `math.arange[start, end, step]` | Values from `start` to `end` by `step` |
| `math.rand[d1, d2, ...]` | Uniform random values in [0, 1) |
| `math.randn[d1, d2, ...]` | Normal random values (mean=0, std=1) |
| `math.tensor[array]` | Convert an array to a tensor |

#### Element-wise Operations

| Function | Description |
|---|---|
| `math.exp[t]` | Element-wise exponential |
| `math.log[t]` | Element-wise natural log |
| `math.sqrt[t]` | Element-wise square root |
| `math.sin[t]` | Element-wise sine |
| `math.cos[t]` | Element-wise cosine |
| `math.abs[t]` | Element-wise absolute value |

#### Reductions

| Function | Returns |
|---|---|
| `math.sum[t]` | Sum of all elements (float) |
| `math.mean[t]` | Mean of all elements (float) |
| `math.min[t]` | Minimum element (float) |
| `math.max[t]` | Maximum element (float) |
| `math.prod[t]` | Product of all elements (float) |
| `math.argmax[t]` | Index of maximum element (int) |
| `math.argmin[t]` | Index of minimum element (int) |

All reductions except `argmax`/`argmin` accept an optional axis parameter: `math.sum[t, axis]` reduces along that axis and returns a tensor instead of a scalar.

#### Sorting & Selection

| Function | Returns |
|---|---|
| `math.argsort[t]` | Indices that would sort the tensor (NumArray) |
| `math.topk[t, k]` | Top k largest values, sorted descending (NumArray) |
| `math.unique[t]` | Unique values, sorted (NumArray) |
| `math.where[cond, a, b]` | Element-wise: a where cond≠0, else b (NumArray) |

Note: `sort[t]` is a core builtin (not in the math module) that works on both arrays and tensors.

#### Cumulative Ops

| Function | Returns |
|---|---|
| `math.cumsum[t]` | Cumulative sum (NumArray, same shape) |
| `math.cumprod[t]` | Cumulative product (NumArray, same shape) |

#### Combining Ops

| Function | Returns |
|---|---|
| `math.concat[a, b]` | Concatenate two tensors along axis 0 (NumArray) |
| `math.concat[a, b, axis]` | Concatenate two tensors along given axis (NumArray) |
| `math.stack[tensors]` | Stack array of tensors along new axis 0 (NumArray) |
| `math.stack[tensors, axis]` | Stack array of tensors along new axis (NumArray) |

Note: `slice[t, start, end]` is a core builtin (not in the math module) that works on arrays, tensors, and strings.

#### Shape Operations

| Function | Description |
|---|---|
| `math.reshape[t, d1, d2, ...]` | Reshape to new dimensions |
| `math.flatten[t]` | Flatten to 1D |
| `math.transpose[t]` | Transpose (swap last two axes) |

#### Linear Algebra

| Function | Description |
|---|---|
| `a * b` | Matrix multiplication (use `*` operator) |
| `math.solve[a, b]` | Solve linear system Ax = b |
| `math.inv[a]` | Matrix inverse |
| `math.det[a]` | Determinant (returns float) |
| `math.svd[a]` | SVD → returns [U, S, Vt] |
| `math.qr[a]` | QR decomposition → returns [Q, R] |
| `math.cholesky[a]` | Cholesky (L*L^T) for symmetric positive-definite |
| `math.eigen[a]` | Eigenvalues → returns [real, imag] |

#### Constants

| Constant | Description |
|---|---|
| `math.pi` | π = 3.14159... |
| `math.e` | Euler's number e = 2.71828... |
| `math.inf` | Positive infinity |
| `math.nan` | Not a number |

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

a = math.zeros[3, 3]
b = math.eye[3]
c = a * b
d = math.det[b]           --- 1.0 ---

--- constants ---
log[math.pi]               --- 3.141592653589793 ---

--- properties ---
log[a.shape]               --- [3, 3] ---
log[a.dtype]               --- float ---
log[a.device]              --- cpu ---
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

Every value in Crow is a result. Fallible operations set `ok` to `false` with a `reason`, `kind`, and `trace`. Values auto-unwrap — you use them directly without worrying about the struct.

### Fallible Builtins

These builtins can fail and return an error-value:

| Builtin | When it fails |
|---------|---------------|
| `read[path]` | File doesn't exist or can't be read |
| `write[path, content]` | File can't be written |
| `input[]` / `input[prompt]` | Stdin read fails |
| `int[string]` | String can't be parsed as integer |
| `float[string]` | String can't be parsed as float |
| `json[string]` | String is not valid JSON, or value is `null` |

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
    log['failed: {content.reason}']
    log['kind: {content.kind}']
    log['trace: {content.trace}']
else
    log[content]
```

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

`nodes[n]` specifies distributed training across multiple machines. The compiler handles data sharding, gradient synchronization, and communication automatically — no explicit `allreduce`, `broadcast`, `scatter`, or `gather` calls.

```
nodes[4]                 --- 4 machines ---
nodes[4, gpu = 2]        --- 4 machines, 2 GPUs each ---
```

The user writes the same training loop. The compiler inserts the right communication pattern. Same philosophy as `gpu` — specify intent, compiler decides how.

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

### crowfile

Every Crow project has a `crowfile` at the root. It uses Crow syntax.

```
--- crowfile ---

name = 'my-project'
entry = 'main.crow'
cpu = 4
gpu = 'all'

deps = [math = '1.0', http = '2.3']
```

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
crow fmt                    --- format project (uses crowfile) ---
crow fmt file.crow          --- format a specific file ---
crow fmt --check            --- check if formatting needed (exit 1 if not) ---
```

`crow build` transpiles `.crow` source files to Rust and compiles to a native binary. `crow run` builds and then executes the binary.

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

This means higher-order functions like `filter`, `map`, and `reduce` can be written as normal Crow functions. You don't need generics, and you don't need builtins — the type system handles it automatically.

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
    log['{i + 1}. {players[i]}']
```

Find index of something:
```
for i in range[len[scores]]
    if scores[i] eq max[scores]
        log['best at index {i}']
```

Combine parallel arrays (instead of zip):
```
for i in range[len[names]]
    log['{names[i]}: {scores[i]}']
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


## Design Note: No Reduce, Any, or All

Crow intentionally omits `reduce`, `any`, and `all` builtins.

**Why they were rejected:**

1. **`reduce` is redundant.** Every common reduction already has a dedicated builtin: `sum[arr]` for addition, `join[arr, sep]` for string concatenation, `max[arr]` and `min[arr]` for extremes. The remaining cases are rare enough for a loop.

2. **`any` and `all` need lambdas to be ergonomic.** In Python: `any(x > 90 for x in scores)`. In Rust: `scores.iter().any(|x| x > 90)`. Without lambdas, Crow would require defining a named function for every one-off predicate — no savings over the loop version.

3. **The loop versions are the same length.** Defining a throwaway predicate function for `any[arr, pred]` takes 3 lines. The loop version also takes 3 lines but uses concepts Crow already has.

**What to use instead:**

Instead of `reduce` (product):
```
acc = 1
for x in nums
    acc = acc * x
```

Instead of `any` (is any score above 90?):
```
found = false
for x in scores
    if x gt 90
        found = true
```

Instead of `all` (are all passwords long enough?):
```
valid = true
for x in passwords
    if len[x] lt 8
        valid = false
```

## Design Note: No find/index Builtin

Crow intentionally has no `find` or `index` builtin for locating the position of an element in an array or a substring in a string.

**Why:** Finding a position is straightforward with a loop:

```
pos = -1
for i in range[len[items]]
    if items[i] eq target
        pos = i
```

This is clear, explicit, and doesn't require a new builtin. Adding `find` would create a second way to accomplish the same task. Unlike `contains` — which genuinely can't be written cleanly without a builtin (Crow has no `break`, so a loop needs a flag variable and always runs to completion) — `find` maps directly to a simple loop pattern.

**What to use instead:** A `for` loop over `range[len[x]]` with an `if` check.

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

**Why it was rejected:** String interpolation (`'hello {name}'`) already handles every string-combining use case. Adding `+` as a second way violates "one way to do things" and creates style debates about when to use which form.

**What to use instead:**

```
greeting = 'hello {name}'
full = '{first} {last}'
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

**What to use instead:** Loops for now. A future `math` library will provide `sum`, `min`, `max`, `product`, and other array reductions.

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

**Why it was rejected:** Inline fallbacks hide errors. The program continues with a value that may be wrong, and the error's `.reason` and `.line` are silently discarded. This contradicts Crow's error philosophy — errors are values that propagate visibly, carrying context, until you consciously handle them.

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
    log['invalid: {x.reason}']
    x = 0
```

This may be revisited in the future if a design emerges that preserves error visibility while reducing verbosity.

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
| `crow init` | ✅ Resolved | Project scaffolding creates `crowfile` and `main.crow` |
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
model = [w = [1, 2, 3], b = [4, 5, 6]]
result = 2 * model

--- compile error: non-numeric leaf ---
config = [lr = 0.01, name = 'adam']
result = 2 * config
--- error: cannot multiply map with non-numeric value at key 'name' (str) ---
```

**What to use instead:** Keep weights and config in separate maps.

```
weights = [w = rand[784, 128], b = zeros[128]]
config = [lr = 0.01, name = 'adam']

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

--- mixed types → use a map ---
[name = 'alice', age = 25]   --- map (heterogeneous, named) ---

--- error ---
[1, 'hello', true]           --- error: mixed types ---
```

**Why:** Python has both `list` and `numpy.array` — the "two type problem." You constantly convert between them. MATLAB, Julia, and R have one array type and it works. For data science, columns are homogeneous arrays. For tabular data, a DataFrame is a map of arrays — which is exactly what Crow's bare-name maps provide:

```
df = [
  name = ['alice', 'bob'],
  age = [25, 30],
  salary = [50000, 60000]
]

mean[df.salary]              --- works because arrays support math ---
df.salary * 1.1              --- element-wise, no wrapper needed ---
```

**Mixed-type data uses maps, not arrays.** `[id = 1, name = 'alice', active = true]` is a map with named fields and dot access. This is clearer than a mixed-type list anyway — fields are named and self-documenting.

**What this eliminates:** `tensor` keyword, `list` vs `tensor` confusion, `tensor[[1,2,3]]` wrapper syntax, mixed-type arrays.