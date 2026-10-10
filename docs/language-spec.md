# Crow Language Specification


This is the reference: every construct, every builtin, and a Design Note for every deliberately rejected feature. New to Crow? Start with The Crow Book (`docs/book`; this document is Part II of the same site) — it teaches in order and every code block there is machine-verified. The other documents are indexed in `docs/README.md`.

## Contents

| | |
|---|---|
| **The language** | [1. Variables and Types](#1-variables-and-types) · [2. Comments](#2-comments) · [3. Blocks](#3-blocks) · [4. Operators](#4-operators) · [5. Control Flow](#5-control-flow) · [6. Loops](#6-loops) · [7. Functions](#7-functions) · [8. Collections](#8-collections) · [9. Structs](#9-structs) · [10. Error Handling](#10-error-handling) |
| **The library** | [11. Core Builtins](#11-core-builtins) · [12. Modules and Imports](#12-modules-and-imports) · [13. Standard Library](#13-standard-library) — math, io, frame, web, db |
| **The runtime** | [14. Parallelism](#14-parallelism) · [15. Background Work](#15-background-work-later) · [16. Verification](#16-verification-test-blocks) · [17. Type System](#17-type-system) |
| **The toolchain** | [18. Project Configuration](#18-project-configuration) |
| **Why not X?** | [19. Design Notes](#19-design-notes) — every deliberately rejected feature, with its reason |

Design Notes about one feature sit next to that feature; the language-wide ones are collected in section 19.


## 1. Variables and Types

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
x = 20          --- ok, same type
x = 3.14        --- ok, int/float compatible
x = 'hello'     --- error: cannot reassign from int to string
```

**A variable whose first value could not fail holds no error.** A later value that can fail lands when it is good; when it failed, the assignment refuses it, as a decision refuses one (§10, Errors in Conditions): a function hands the error back to its caller, so a function that can fail this way can fail, as one with such a condition can; a handler stops; a view renders `error: reason` in its own place; and the top level halts with `runtime error: line N: cannot assign an error value to 'x' — <reason>`. To keep a failure to look at, give it a name whose first value can fail: `r = float[s]`, then `if not r.ok`.

```
x = 1.0
x = float['2.5']     --- 2.5 lands
x = float['abc']     --- halts: cannot assign an error value to 'x' — cannot convert 'abc' to float
```

An empty list cannot fail either: `xs = []`, then `xs = push[xs, float[s]]`, holds numbers, and a failed item is refused where it is pushed.

**Design Note: a plain variable refuses an error.** Considered: letting the variable hold the error, its type widening to one that can fail, which the interpreter did while the compiled program returned at the assignment. Decided (Vijay, 2026-09-25): the compiled program's rule, in both modes. A variable's type does not change (above), and an error that silently took the variable's place would surface lines later, far from its cause; refused where it is assigned, it names the line and the variable.

**A variable that is ever assigned a float is a float for its whole scope.** Assigning an int to it stores the float value (`x = 1` above stores `1.0`), and so does handing an int to a parameter that is a float: `half[3]`, where `fn half[x]` is also called with `2.5`, divides `3.0`. (The compiled program handed the function its int and did not build.) So does a record's field that holds floats — `P[1, 2]` beside `P[1, 2.5]` holds `2.0` — and a list, a map or a comparison of floats given an int: `push[xs, 2]` onto floats pushes `2.0`, an item of a list of floats assigned an int holds its float (`xs[0] = 2` stores `2.0`; compiled, it had not built), and `map[lr = 0.01, epochs = 100]` holds `100.0`. The other way round too: a list or a map of ints handed a float holds floats from then on — `push[range[3], 2.5]` is `[0.0, 1.0, 2.0, 2.5]`, `set[counts, 'k', 2.5]` a map of floats, `concat` and `contains` likewise — and a record's float field given an int that can fail holds a float that can fail. (Compiled, those did not build; interpreted, the int was kept among the floats.) (Compiled, those did not build; interpreted, the int was kept, so `type[m.epochs]` said `int` beside a float division.) Note for interpreted mode: the widening applies from the first float assignment onward, so an int division performed *before* any float assignment may differ from compiled mode, where the variable is float from its declaration — don't depend on int division of variables that later hold floats.


### Types

Crow is statically typed with full type inference — you never write type annotations, but every variable has a concrete type determined at compile time. The compiler infers types from usage; if a type cannot be determined, the program is invalid.

The built-in types are:

| Type   | Example              |
|--------|----------------------|
| int    | `42`, `-7`, `0` — a whole number of any size |
| float  | `3.14`, `-0.5`       |
| decimal | `decimal['12.30']`, `decimal[2, digits = 50]` — an exact base-ten number |
| fraction | `fraction[1, 3]` — an exact ratio of two ints |
| str    | `'hello'`, `'crow'`  |
| bool   | `true`, `false`      |
| array  | `[1, 2, 3]`, `[[1, 2], [3, 4]]` |
| map    | `map[home = 78701, office = 78702]` |

Arrays are homogeneous — all elements must be the same type. Use maps for mixed-type named data.

```
[1, 2, 3]                    --- int array
['a', 'b', 'c']              --- str array
[[1, 2], [3, 4]]             --- 2D int array
[1, 'hello', true]           --- error: mixed types
```


### Numbers That Keep Every Digit

**An int is a whole number of any size.** Arithmetic on ints gives the exact answer however large it grows: `2 ^ 100` is `1267650600228229401496703205376`, a factorial of a thousand has its 2,568 digits, and an int read from text keeps every digit it was written with. Nothing is written to ask for it, in either mode. The compiled program keeps an int in 64 bits where it can show the int never grows past them — a loop's counter below its bound, a position in a list, a value taken `% n`, a sum a loop adds to — and in a form that grows where it cannot, so a program pays for the size only where a number can have it. In showing it, the compiler counts no part of a program as running 2^48 times (days of computing at a billionth of a second a round): a sum taken in a loop, or over a recursion's calls, grows by fewer bits than that past what it adds, and a program that ran longer and grew such a sum past 64 bits would stop with an integer overflow, never with a wrong digit. Where an int past 64 bits is handed to what takes a 64-bit one — a position, a size, a count (`range[n]`, `xs[i]`, `repeat[s, n]`), a map's key, a frame's cell, a tensor's number — it is refused where it is handed, with what took it and what it was (its first twenty digits and how many there are, past forty): `a whole number past 64 bits where range[] takes an int of 64 bits: 1267650600228229401496703205376`. What shows or holds any value takes one as it is — `str`, `log`, text, `type`, a list's items, a map's values, a record's fields, `encode` — and so do `abs`, `min`, `max` and `sort`; `float[n]` is the nearest float, and `int[x]`, `floor[x]`, `ceil[x]` and `round[x]` of a float are the whole number it holds, every digit (`int[10.0 ^ 30]` is `1000000000000000019884624838656`, the float's own value; of NaN `0`, and of an infinite float a stop). A literal list of whole numbers that holds one growing past 64 bits is a list of ints, as one past 2^53 is (a tensor's numbers are floats). A function handed as a value — to another function, kept in a variable — is called through it as a function of ints of 64 bits: an answer past them is refused where it crosses (`a whole number past 64 bits where a function handed as a value answers an int of 64 bits: …`), while a call of the function by its name keeps every digit (the roadmap holds function values that grow). An int past 64 bits encodes as JSON text of its digits — `"123456789012345678901234567890"` — as JSON's readers hold numbers in 64 bits or as floats, which would lose them; one within 64 bits is a JSON number.

**A decimal is an exact base-ten number.** `decimal['12.30']` holds 12.30 as written, its two places kept (`str` shows `12.30`); `decimal[12]` and `decimal[0.1]` hold 12 and 0.1 (a float as the shortest text that reads back as it). `+`, `-` and `*` of decimals are exact and keep their places (`decimal['12.30'] + decimal['1.1']` is `13.40`, `decimal['1.10'] * decimal['2.0']` is `2.200`), so `decimal['0.1'] + decimal['0.2'] eq decimal['0.3']` holds, and a sum of money never drifts. `/` of decimals has 28 significant digits, the last rounded half away from zero, as `round` rounds, unless a decimal in it holds more: `decimal[x, digits = n]` is `x` rounded to `n` significant digits, and every result it takes part in — `+`, `-`, `*`, `/`, `^`, and `sqrt`, `exp`, `ln`, `sin`, `cos`, `tan`, `atan` of it — is rounded to the most digits of the decimals in it. That is the language's big float: `sqrt[decimal[2, digits = 50]]` is √2 to fifty digits. `round[d, places]` rounds a decimal to `places` places after the point, half away from zero (`round[decimal['2.345'], 2]` is `2.35`); `round`, `floor` and `ceil` of a decimal are ints; `int[d]` cuts toward zero and `float[d]` is the nearest float.

**A fraction is an exact ratio of two ints.** `fraction[1, 3]` is one third, held in lowest terms with its sign on the top (`fraction[2, -4]` is `-1/2`); `fraction[x]` of an int or a decimal is that number exactly (`fraction[decimal['0.25']]` is `1/4`). `+`, `-`, `*`, `/` and `^` of fractions are exact. A fraction shows as `n/d`, a whole one as its int (`str[fraction[4, 2]]` is `2`); `int`, `float`, `round`, `floor` and `ceil` read it as a number; `fraction[n, 0]` is an error value of kind `'math'`, as an int divided by zero is a stop.

**Kinds meet the exact way.** An int meeting a decimal is the decimal it is, and an int or a decimal meeting a fraction is the fraction it is: the answer is the exact one. A float meeting a decimal or a fraction is refused before the program runs, in both modes — a float holds a nearby binary number, and the exact answer would be a guess: `price * rate` with `rate` a float is refused with `a decimal and a float: decimal[rate] reads the float as the decimal it shows, float[price] reads the decimal as a float`. A literal written with a point is the number it writes, so `price * 1.1` is `price` times eleven tenths. Two decimals, two fractions, or a decimal and a fraction compare by value in either order (`decimal['2.50'] eq decimal['2.5']`, `fraction[1, 2] eq decimal['0.5']`), and with an int; with a float, the comparison is refused as arithmetic is.

A decimal and a fraction encode as JSON text of what they show — `"12.30"`, `"1/3"` — every digit kept, as an int past 64 bits does (JSON's readers hold a number as a float, which would lose digits, and JSON has no ratio); a list, a map or a record holds decimals and fractions as it holds any value, and a fact keeps one as that text. A frame's columns hold ints, floats, text, truths and moments: a column of decimals or fractions is refused where it is made, and a decimal column a database answers is a column of floats (the roadmap holds columns of decimals).

**Design Note: ints grow.** Considered: stopping on overflow (Crow until 2026-10-09: `integer overflow: … does not fit in 64 bits`), wrapping (Rust's release build, Java, Go), and a kind of its own the program asks for where it starts a number (`q = big[1]`). Decided (Vijay, 2026-10-09, asked whether Crow could tell by itself which numbers need to be big): the int grows and nothing is written, so the answer is the exact one as Python's is. Whether a number outgrows 64 bits depends on the data and on how long a loop runs, so the compiler cannot know it before the program runs — what it can show before the program runs is the opposite, that an int stays within 64 bits, and there it keeps the 64-bit form, so a loop over positions costs what it did.

**Design Note: one decimal for exact money and for big floats.** Considered: a binary big float beside the decimal (Julia's `BigFloat`, MPFR), and a precision set for the whole program (Python's `getcontext().prec`). Decided (Vijay asked for exact decimals, rationals and big floats, 2026-10-09): one decimal kind, exact where exactness is possible — `+`, `-`, `*` — and rounded to the digits its values ask for where it is not. A precision belongs to the values, never to a mode of the program, so a function rounds the same wherever it is called; a binary big float would be a second way to hold a number with chosen digits, whose digits print in base ten anyway.

**Design Note: a float meets no exact number silently.** Considered: a float meeting a decimal makes a float (C#'s `decimal` refuses it; Python's `Decimal` refuses it), or a decimal of the float's exact binary value (`Decimal(0.1)` is `0.1000000000000000055511151231257827…`). Decided: refused before the program runs, in both modes, with both conversions named — a computed float is a nearby binary number, and either answer would be a guess the program did not make — while a literal written with a point is the number it writes, which a decimal or a fraction reads exactly.


### String Escape Sequences

Strings support escape sequences with backslash:

| Escape | Character |
|--------|-----------|
| `\n` | Newline |
| `\t` | Tab |
| `\r` | Carriage return (a line of a wire protocol ends `\r\n`) |
| `\\` | Literal backslash |
| `\'` | Literal single quote |
| `` \` `` | Literal backtick (not interpolation) |

A backslash before any other character is itself, the backslash kept, so a regular expression's escapes are written as they are: `'\d+'`, `'\.'`, `sub[s, '\|[^|]*\|', '-']`. (Until 2026-10-09 any other character after a backslash was refused as an unknown escape, against strings being inert, and a regex needed `[|]` for `\|`.)

```
log['line1\nline2']
log['col1\tcol2']
log['it\'s here']
log['a \`code span\` in prose']    --- prints: a `code span` in prose
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
cfg = json['`"a": 1, "hosts": ["x"]`']   --- braces and brackets: plain text
m = matches[s, '[a-z]`2,4`']             --- regex: fully raw
```

Rules:
- `` `expr` `` evaluates any Crow expression and splices its value. Its names are read like any others: an undefined one, or a module variable read inside a function, is refused before the program runs.
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

Variables are scoped to the block they are defined in. Inner blocks can access outer variables — but **function boundaries are opaque**: module variables are not visible inside functions (see Function Scope, section 7).

```
x = 10
if true
    y = 20
    log[x]    --- 10 — accessible: blocks see outer variables

log[y]        --- error — y is not in scope
```

Reading `y` there is refused before the program runs, in both modes, with what to write instead: set `y` before the block and assign it inside. Two blocks may each set a variable of the same name, and a name set again after its block has ended is a new variable. They are different variables and may hold different types:

```
if ok
    x = 1
else
    x = 'one'     --- a different x
```


## 2. Comments

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


## 3. Blocks

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

A line holds one statement: `x = 1 y = 2` is refused — the second starts a line of its own (it had run as two). A line is indented only under one that opens a block; one indented under a plain statement is refused, and a block's first line with no indented body under it is too.


## 4. Operators

### Arithmetic

```
+  -  /  *  %  ^
```

`^` is the power operator (right-associative):

```
2 ^ 10       --- 1024
2 ^ 3 ^ 2   --- 512 (right-associative: 2^(3^2) = 2^9)
2.0 ^ 0.5   --- 1.4142...
```

Division is type-determined — one operator, no ambiguity:

```
3 / 2        --- 1 (both int → int, rounded toward zero)
-7 / 2       --- -3
-7 % 2       --- -1 (the remainder has the sign of the left side; `%` takes ints — a float on either side is refused before the program runs)
3 / 2.0      --- 1.5 (float involved → float)
7 / 3        --- 2
7.0 / 3      --- 2.3333...
```

An int divided by zero stops the program (When a Program Stops, §10); a float divided by zero is infinity.

The operands are numbers: text is refused before the program runs, and so is a list of text (`['1', '2'] + 1` is `cannot use '+' with array[str] and int`). (Compiled, a list of text had been parsed as numbers, where the interpreter made each item an error.)


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
y = W * x                   --- matmul
a = ones[3] .* full[3, 2]   --- element-wise multiply
b = ones[3] ./ full[3, 2]   --- element-wise divide
c = full[3, 2] .^ 3    --- element-wise power
d = ones[3] * 2        --- scalar multiply
```

**A tensor's shape is in its type.** A shape the program states is
known before it runs — a literal (`[[1.0, 2.0], [3.0, 4.0]]` is 2×2), a
constructor given literal sizes (`zeros[2, 3]`, `ones`, `full`, `randn`,
`eye[3]`, `linspace`), `reshape` given literal sizes, and what
`transpose` (its last two dimensions swapped), the element-wise operators
and `*` make of them. The type carries the shape where the tensor goes: a
reassignment binds the variable anew, as a frame's does (see Frames), so
after `x = zeros[3, 3]` it is 3×3 from that line on; where branches, or a
loop's rounds, meet, it has the dimensions they agree on; a function's
parameter has the dimensions its calls agree on, and a call has the shape
of what the function returns. An operation two known dimensions make
certainly wrong is refused before it runs, in both modes: `*` of tensors
that are not both 2-D, or whose inner dimensions differ (`matmul:
incompatible shapes [2, 2] × [3, 3] — …`), and an element-wise operation
of two shapes that do not broadcast (each pair of dimensions, aligned
from the innermost, equal or one of them 1: `` `+` of tensors of shapes
[2, 2] and [2, 3]: dimension 2 is 2 in one and 3 in the other — … ``),
read in a function with the parameters the tensors were handed as (``(the
tensors add[] is handed as `a` and `b`)``). A dimension the program
computes (`zeros[n, 3]`), one a function's calls disagree on, and any of
a tensor's once a cell or a row of it is assigned (`m[0] = …`) are not
known, and nothing about them is refused before the program runs: an
operation whose shapes do not meet stops the program where it runs, with
the same words, in both modes.

**A chain of element-wise operations is computed in one pass.** `x * 2.0
+ x .* x - 1.0` reads each element of `x` once and writes one tensor,
where each operator would make a tensor of its own — across the machine's
cores for a large one. Each element is what the operators compute, in
their order and with their formulas, so the answer is the same to the
bit, and a shape that does not meet stops with the operator's words at
the first operator whose operands do not meet. A chain's operands are what
only read — a variable, a fact, a derivation, a number written; one that
calls something is computed where the program calls it, and its chain by
the operators. The same in both modes.

**Design Note: a function is one function whatever shapes it is
handed.** Considered: a copy of a function for each shape its calls hand
it, as a frame's columns make one (see Frames), so that each copy's
operations are checked against its own shapes. Rejected: a frame's
columns are its row type, but a tensor compiles alike whatever its
shape, so a copy per shape would grow the build for nothing but earlier
refusals inside the function. Decided: a parameter has the dimensions all
its calls agree on; a call whose shapes the others do not share is
checked where it runs.

Scalar-array operations broadcast the scalar:

```
[1, 2, 3] + 10              --- [11, 12, 13]
[1, 2, 3] * 2               --- [2, 4, 6]
10 - [1, 2, 3]              --- [9, 8, 7]
-[1, 2, 3]                  --- [-1, -2, -3]
```

Internally, all-numeric arrays use a fast tensor representation for element-wise operations. This is transparent — numeric arrays behave identically to regular arrays for indexing, iteration, comparison and all builtins: two lists of numbers are `eq` when they hold the same numbers in the same shape, however each was built (a list grown by `push` and a literal, ints and the same floats, a list of rows and a matrix literal). A list of rows — tensors of one shape, grown by `push` from a matrix's rows — is a matrix wherever a tensor is taken (`transpose[rows]`, `stack[rows]`, `mean`), as a list of numbers is a vector; rows of two shapes stop the program with the reason, in both modes (compiled, a list of rows had been refused where a tensor was taken). A matrix `stack` makes, or `tensor` of a list of lists, has a rank the program knows, so its columns walk as rows and each is numbers (its columns had been read as numbers, and `mean` of one refused).

A tensor holds its numbers as floats, and a float holds a whole number exactly only up to 2^53 (9007199254740992): a literal list of ints past it is a list of ints, which holds each as it is (a tensor had held `9007199254740993` as `…992`), and a matrix literal of them is refused, in both modes. A literal list of whole numbers that a function goes on to hold as a list of ints it builds — `q = [1]`, then `q = grow[q, k]` with `grow` pushing ints — is a list of ints from its first line (it had held floats, converted to ints and back at every call and assignment); one used as numbers stays a tensor.


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

`eq` compares two values whole, in both modes alike: numbers by value (an int beside a float as a float: `1 eq 1.0`), text by its characters, a list item by item, a map entry by entry, and a record field by field — two records are equal when they are the same struct and every field is (`Point[1, 2] eq Point[1, 2]`). An item or a field that failed equals nothing, not even another failure. `contains` compares a list's items the same way. A frame's column compared whole — with a column, a list or a tensor — is compared as a list is, cell by cell, a missing cell equal to nothing. A list compared with a single value is a mask instead (see Arrays). (Compiled, comparing two records did not build, and the interpreter answered `false` for records and maps with equal contents; a column of text with a missing cell had equalled itself, and a column compared with a list had not built.)

**Two values of two kinds are not compared.** Each of the six compares a value with one of its kind: numbers with numbers (an int beside a float as a float), text with text, bools with bools, and lists, maps and records with their own kind. A number is not text, nor a bool either: `'1' eq 1`, `true eq 1`, a list of text with a list of numbers, a column of text with a number (`df.name eq 2`), `1 in ['a']` and `push[['a'], 1]` are refused before the program runs — `eq compares values of one kind, not text with a number` — and where a kind shows only as the program runs — a parsed value, a cell of a file's column — the comparison stops the program with the same words. A parsed value is compared with a value of its kind (`j.status eq 'ok'`), and JSON's null equals nothing. (Both modes had answered `false` for a number and text, and `contains` had too; compiled, `'a' eq 1` had not built; interpreted, a parsed `1` had not equalled `1.0`.)

`gt`, `lt`, `gte` and `lte` order numbers with numbers and text with text, by character code — the order `sort` gives text: `'b' gt 'a'`, `'B' lt 'a'`, and `'10' lt '9'`, text and not numbers. Numbers compare exactly, and NaN is equal to nothing and unordered, so of the six only `ne` holds for it. Bools have no order (`gt compares numbers with numbers or text with text, not a bool with a bool`), nor two lists, and a parsed value is read as the kind the other side says, as arithmetic on one reads it as a number (see JSON). (Compiled, `true gt false` and `'b' gt 'a'` had answered where the interpreter stopped.)

**Design Note: a number is not text.** Considered: answering `false` for values of two kinds, as Python's `==` does and as both modes had; converting one side, as JavaScript, R and MySQL do (`'10' < 9`). Decided (Vijay, 2026-10-01: "a string is not a number"): a comparison of two kinds is the program's mistake, not a question with an answer — an identifier read as text and compared with a number matched nothing, and a filter dropped every row without a word. So all six refuse it where both kinds are known, as a statically typed language does, and stop the program where a kind shows only as it runs.

**Design Note: text has an order.** Considered: refusing an order between texts, as the interpreter had by stopping, so that `gt` is for numbers alone. Decided (2026-10-01; Vijay: text with text and numbers with numbers are compared): text is ordered by character code, the order `sort` already gives it — one order for text, not a second — so `filter[df, df.day gte '2026-01-01']` keeps the rows of ISO dates written as text from that day on, as the compiled program had. It is not a locale's alphabetical order (`'B' lt 'a'`); `lower` first compares without case.


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

`and` and `or` evaluate both sides, the left first: there is no short-circuit. A call on the right runs even when the left side already decides, and an error on either side is the value, because an error has no truth value (Errors in Conditions). A guard that must not touch a failed value is its own `if`:

```
if not r.ok
    return 'unreadable'
if len[r] eq 0              --- r is known good here
    return 'empty'
```

`if not r.ok or len[r] eq 0` is not that guard: when `r` failed, `len[r]` is the error, so the condition is the error and the decision is refused.


### Design Note: `and` and `or` Evaluate Both Sides

**Considered:** short-circuiting, as most languages do: `or` skipping its right side when the left is true, `and` when the left is false, so `not r.ok or len[r] eq 0` would work as a guard.

**Decision (Vijay, 2026-09-23):** both sides are evaluated. Every operator treats an error the same way, and `and` and `or` are operators like the rest: an error on either side is the value, and nothing is evaluated conditionally except by `if` and `while`. The guard that must not touch a failed value is written as the `if` it is, and reads the same in both modes. The compiled program had short-circuited plain booleans, so `false and check[x]` ran `check` in the interpreter only; now both modes run it.


### Membership

`x in xs` is true where `xs` holds `x`, and `x not in xs` where it does not — the one way to ask, in words: an item of a list, compared as `eq` compares; a number in a tensor; a part of text; a key of a map or of a parsed value; and over a frame's column, cell by cell, a mask (`filter[t, 'AA' in t.sym]`). It binds as a comparison does — tighter than `and`, `or` and `not`, looser than arithmetic — so `n + 1 in [3, 4] and n in [2]` reads as written. A value of another kind than the items is refused before the program runs, in both modes: `1 in ['a']` is "`x in xs` takes a list and an item of its kind: its items are text, not a number".

```
if node.tag not in ['button', 'input', 'select', 'summary']
    lines = lay_lines[node, inner_w]
```

**Design Note: membership is an operator.** Considered: keeping `contains[xs, x]` and `has[m, key]` as functions (rejected: `node.tag ne 'button' and node.tag ne 'input' and …` was written in the layout engine where a list would do, because `not contains[list, node.tag]` reads backwards from how it is said, and `has` beside `contains` was two words for one question); adding `in` beside them (rejected: two ways to write one call). **Decision (Vijay, 2026-10-08):** `in` and `not in`, a pair as `eq` and `ne` are, and the functions retired — `contains[…]` or `has[…]` written in a program is refused with the operator to write. The parser writes the operator as a call of the row named `contains` (a map's and a parsed value's rows are `has`'s functions under that name), so nothing below it knows an operator exists, as with `then`.

### Then Chaining

`then` pipes the left value as the first argument to the right function, enabling left-to-right composition instead of nested calls.

```
--- x then f = f[x]
s = '  hello  '
result = s then trim                --- trim[s]

--- x then f[a] = f[x, a]
result = 'hello world' then split[' '] then join['-']
--- split['hello world', ' '] then join[..., '-'] → 'hello-world'
```

Chains of any length:

```
result = s then trim then split[' '] then join[', ']
```

`then` binds tighter than comparisons and logical operators, but looser than arithmetic:

```
x + 1 then str          --- str[x + 1]
x then str eq '5'       --- (str[x]) eq '5'
```

Since all builtins are immutable and return new values, `then` works with every builtin:

```
nums = nums then sort then reverse then slice[0, 3]
```

**A chain as lines.** An assignment's chain may be written one step a line, each line starting with `then`, indented under the assignment. A step that is a call takes the value so far as its first argument, as above. A step that names the variable being assigned reads the value so far where it says, as often as it says, so a step may be any expression over it, and the value's kind may change along the way:

```
pr = frame.new[node = keys[m], value = values[m]]
    then join[pr, table, 'node']
    then max[abs[pr.value - pr.pagerank]]
```

`pr` is the frame through the join and the number at the end. Each step is a statement of its own — a refusal names the step's line, and a step whose value failed hands the failure on, as a call of a failed value does (`n = '42'` then `int` then `n + 1` is `43`; from `'x'` it is the error, held by `n`). The lines belong to an assignment: a `then` line under anything else, or under a loop used as a value, is refused before the program runs, in both modes. Style: name a value you use across lines or want to explain, nest a value used once as a first argument, and write `then` lines for two or more steps.

**Design Note: a chain's steps name the variable.** Considered: a word for the value flowing through — `it`, `self` — as Kotlin's scope functions and magrittr's `.` have (rejected: a keyword on every line of every chain); the dot form, `x.f[a]` for `f[x, a]` (rejected: a second way to write every call, and the dot already reads a field — a frame's `ok` column is unreachable by dot today, and every function name would join it); a pipe symbol (rejected: the shift key, and the same second way); a step continued by indentation alone (rejected: indentation would mean a body under `for` and a step under a value); bare column names inside a step over a frame, as pandas's `eval` reads them (rejected: where a name came from would go unsaid, and a variable of a column's name would collide); a dotted statement that replaces its receiver, `pr.join[…]` (rejected: a statement that changes a variable without `=`, the confusion `push` as a statement had). **Decision (Vijay, 2026-10-08):** the variable being assigned names the value so far, and a step that does not name it is a call that takes it first. One rule, read either way, and a reader can expand the chain into `pr = …` lines knowing nothing more. The parser does exactly that, each step assigned to a hidden name so the kind may change, which is why nothing below the parser — the lowering, the IR, either backend — knows a chain exists.


## 5. Control Flow

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

A condition is true or false. A number, text or a list decided on — `if count`, `while name` — is refused before the program runs: compare it, `count gt 0`, `len[xs] gt 0`, `name ne ''`. (A number had reached the compiled program as "the lowered program disagrees with itself", a bug in Crow's words.) A value that can fail is decided on as Errors in Conditions says.


### Design Note: No Pattern Matching or Sum Types

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

**Why no sum types?** Sum types shine in systems programming and compiler design, where you model distinct variants (e.g. `Shape = Circle | Rectangle`). Crow targets data science and practical programming, where data lives in collections, tables, and structs — not algebraic types. Crow's error-value system (section 10) handles the most common sum type use case (success vs failure) without additional syntax.

**If both were added, they would need to come together.** Pattern matching without sum types is pointless sugar. Sum types without pattern matching are unusable. Neither earns its place alone in a language optimized for simplicity and LLM-friendliness.


## 6. Loops

### for

`for` loops run their iterations in parallel when nothing the body does can show the order they ran in, and in order when something can (section 14). Results are always collected in the original order.

```
for x in [1, 2, 3]
    log[x]              --- prints 1, 2, 3: output keeps a loop in order
```

A loop walks a list, a tensor (its rows, or its numbers when it has one axis), a frame's column (its cells) or a parsed list. A map, text, a frame or a number has no items, and a loop over one is refused before the program runs, in both modes: walk `keys[m]` or `values[m]`, `split[s, '']`, or a column of the frame.

When a `for` loop is used as a statement (not assigned to a variable), the results of each iteration are discarded. The loop runs only for its side effects:

```
for x in [1, 2, 3]
    log[x]               --- prints 1, 2, 3; results discarded
```


### for as Expression

Assign a `for` loop to a variable to collect results into an array.

```
--- transform each item
doubled = for x in [1, 2, 3]
    x * 2
--- doubled eq [2, 4, 6]

--- filter items
evens = for x in [1, 2, 3, 4, 5]
    if x % 2 eq 0
        x
--- evens eq [2, 4]

--- filter and transform
result = for x in [1, 2, 3, 4, 5]
    if x gt 2
        x * 10
--- result eq [30, 40, 50]
```

An item that fails is kept, not dropped: `nums = for s in ['1', 'b', '3'] / int[s]` is a list of three whose second item is the error, shown by its line and reason — `[1, error: line 1: cannot convert 'b' to int, 3]` — as a list literal holding a failed value is. The list itself did not fail (`nums.ok` is `true`, see Error Handling); each item's own `.ok` says which did. Handing a failed value to a function is another matter: `push[xs, int['b']]` is that error, by auto-propagation. A list whose items can fail, handed whole to a function that takes numbers (`sum[nums]`, `mean[nums]`), is its numbers, as a column's cells are: a failed item stops the program (`item 1 of the list failed: cannot convert 'b' to int`), in both modes. (Compiled, it did not build.) Compared whole with a tensor or a list of numbers (`nums eq [1, 2]`, each item a map's read or a function's answer that can fail) it is its numbers the same way, and the comparison is one truth (the interpreter had compared the items; the compiled program had not built).


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

In a `for` loop and outside every loop `break` and `continue` are refused before the program runs: a `for` loop's rounds run at once, in no order, so there is no round after this one to skip to and none before it that stopping would keep — loop with `while` to stop early or skip in order, skip an item of a `for` with `if`, and search with `find`, `any` or `all`. (They had run in a `for`, the loop quietly made sequential; Vijay, 2026-10-08: "for is parallel, while is for sequential".)

> **Design Note:** `break` and `continue` are only valid in `while` loops. `for` loops are parallel by default (section 14), and `break`/`continue` are inherently sequential concepts. Using `break` in a parallel loop would silently force sequential execution — a hidden performance trap. Use `if` inside `for` to skip items, `any`/`all`/`find` for searches that should stop early (they early-exit *in parallel* — see the Search Builtins Design Note), or `while` for genuine sequential iteration.


## 7. Functions

### Declaration

Functions are declared with `fn`. The last expression in the body is the implicit return value. Use explicit `return` for early returns.

```
fn add[a, b]
    a + b                    --- implicit return

fn greet[name]
    log['hello, `name`']

fn safe_div[a, b]
    if b eq 0
        return 0             --- early return
    a / b                    --- implicit return
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
train[my_data]                     --- uses defaults
train[my_data, lr = 0.001]         --- override one
train[my_data, 0.001, 50]          --- positional also works
```

Named arguments are required for axis parameters in math operations:

```
s = sum[x, axis = 0]              --- sum along axis 0
mx = max[x, axis = 1]             --- max along axis 1
```

Named arguments are resolved at compile time (a lowering pass rewrites them to positional form before execution), so they work only when the callee is a directly named function, struct, builtin with named parameters, or ui element. A function received as a value (`fn apply[f, x]` calling `f[...]`) must be called with positional arguments.


### Calling

Functions are called using `[]`.

```
greet['crow']
result = add[3, 4]
```


### Function Scope

A function sees its parameters, its own locals, and the program's vocabulary — other functions, structs, and builtin constants (`pi`, `e`). **Module variables are invisible inside functions.** The only way data enters a function is through its parameters; the only way it leaves is through its return value.

```
count = 5

fn build[]
    return 2 + 3          --- ok: self-contained

fn scaled[n]
    count = n * 2         --- ok: a fresh LOCAL named count —
    return count          ---     unrelated to the module variable

fn bad[x]
    return x + count      --- error: 'count' is a module variable, which a
                          --- function does not see — pass it as a parameter

count = 6                 --- ok: top-level code mutates top-level state freely
log[scaled[count]]        --- 12: pass it in
```

Because module variables were never visible, a local reusing a module name shadows nothing — it is simply a new binding. There are no `global` declarations and no scope-resolution rules to learn: inside a function, every name is a parameter, a local, or a function/struct/constant.

**Exception — views and event handlers:** in a ui program, top-level assignments become managed reactive state. **Only `view` declarations and event handlers** (functions referenced from views or `page[]`) access that state — through the state machinery, rewritten to `state.get`/`state.set`. Ordinary `fn` declarations are fully isolated even under `use ui`. That is the sanctioned mutable-state channel — owned by the runtime and synchronized, like LiveView assigns.

**Exception — a route's handler reads what the app holds:** a function a top-level `route[…]` answers with reads, by name, a derivation the app holds — one over tables, the clock and facts no session moves (Derivations, A route reads what the app holds). It still sees no module variable, and a function it calls sees no derivation: what a helper needs, the handler hands it.

**A fact gets one value at the top level.** Its top-level assignment declares it, and that value is the one every session starts from; a test runs the declarations and nothing else of the top level, and each declaration's value is computed before the top level's other statements run, in the order written, so a declaration may read a fact or use a connection declared above it (`tripled = base * 3`, `made = exec[book, 'CREATE TABLE …']`; compiled, the first had not built and the second had stopped the program as it started, the connection not yet set). A fact holds no error: a declaration whose value can fail — a file loaded, a query, a document parsed — is refused where the facts are set, so the program stops before anything is served, `runtime error: line 8: cannot assign an error value to 'trades' — load: No such file or directory (os error 2)`, and the fact holds the value itself: `trades = load['trades.csv']` is a frame, which a page reads as one. (The interpreter had held the error in the fact, and a compiled program with such a fact had not built: a session's state could not keep it.) So a statement of the top level that changes a fact after its declaration — a second assignment, a loop that pushes into it, `cfg['a'] = 2` — would reach only the rest of the top level under `crow run`, and no session, no test, and no declaration after it (`n = len[xs]` after such a loop would be the declared `xs`'s length). It is refused: `'xs' is state, declared on line 2: it gets one value at the top level, which every session starts from, and handlers move it after that — give it its whole value where it is declared (a function can build it), or derive it`. A loop of the top level may bind names of its own, which are not facts (the book's site generator writes its pages in one). A `use web` program that declares a derivation holds state the same way, for its routes (Derivations): its top-level names are facts, each given one value, which its derivations read — `'xs' is state, declared on line 2: a program with derivations gives each top-level name one value, which its derivations read — …`.

The state is the file's that imports ui, or that imports web and declares a derivation: **an entry file that does neither keeps its top-level variables plain**, whatever its parts import, so a script that imports a part — to call its functions over real data — runs its top level in order, as any program does.

#### Design Note: No Ambient Mutability

**What was considered:** letting functions read (and possibly write) module variables, as in Python (`global` keyword), Julia, Go, or Lua.

**Why it was rejected:**

1. **Same reason closures were rejected.** Reading ambient state is capture — a hidden data dependency that makes a function's behavior depend on things not in its signature. Purity detection, auto-testing (`crow check`), and refactoring all rely on parameters telling the whole story.
2. **Parallel `for` requires it.** Iterations run concurrently, and the loop rule is sound because every read and write in the body is visible at the call site. An ambient read inside a function would be invisible, so an iteration calling `f[]` could race with another that writes what `f` secretly reads. Immutability-by-construction is the only zero-cost fix; the alternatives are interprocedural analysis or locks.
3. **The permissive languages document the damage.** Python's `global`/`UnboundLocalError` is its most-asked scoping question; Julia re-fought its scope rules for two years; JavaScript needed strict mode to stop accidental globals. The concurrency-serious languages — Rust (no unsynchronized mutable statics), Erlang/Elixir (no shared mutable state at all) — sit exactly where Crow now sits.

**What to use instead:** parameters in, return values out. For shared mutable state across events, use the explicit managed channels: ui `state`, the database, files.


### First-Class Functions

A function name without `[]` evaluates to the function itself. Functions can be passed as arguments, stored in arrays and maps, and called later with `[]`.

```
fn double[x]
    return x * 2

fn apply[f, x]
    return f[x]

result = apply[double, 5]    --- 10

ops = [double, square]
ops[0][3]                     --- 6
```

A function handed as a value is typed by what the parameter it is handed to is applied with — `apply[double, 5]` types `double`'s `x` as an int, by `f[x]` inside `apply` — however far it is handed on: through a function that only passes it along to the one that applies it, and to a module's function called qualified (`crossval.crossval[x, y, 3, 1, fit_ridge, score_ridge]` in the models package types `fit_ridge` by the rows `crossval` fits it to). A function handed as a value that nothing types — one never called directly, handed to a parameter that is only copied into a variable — is refused before the program runs, with the fix: apply it in the function it is handed to, or call it directly once. (Compiled, such a function had been left out of the program, and one handed on had been read as text.) A gradient handed as a value (`bfgs[f, grad[f], x0]`) is a function of what `f` takes, answering `f`'s gradient — a tensor, or a float — and `f` is typed by what the parameter is applied with, through however many hands; a function only a test hands to `grad` is typed the same way. (Compiled, the parameter's answers had been read as text, and `f`'s parameters had been guessed floats.)


### No Lambdas

Crow has no anonymous functions. Every function must be declared with `fn` and a name. To pass behavior, define a named function and pass it.

```
--- correct
fn is_big[x]
    return x gt 100

hit = find[items, is_big]

--- not supported
hit = find[items, fn[x] x gt 100]    --- no anonymous functions
```

This follows Crow's "one way to do things" principle — there is exactly one way to define a function.


## 8. Collections

Arrays hold ordered values of one type — an all-numeric array is a tensor (see Tensor Arithmetic and the math module). Strings index like arrays. Maps are dictionaries: string keys, values of one type. Records with mixed, named fields are structs (section 9); tables are frames (section 13).


### Arrays

Arrays are functions from index to value. Negative indices count from the end: `-1` is the last element, `-2` is second-to-last, etc.

```
numbers = [1, 2, 3]
numbers[0]               --- 1
numbers[-1]              --- 3 (last)
numbers[-2]              --- 2 (second-to-last)
```

Negative indices also work for assignment:

```
numbers[-1] = 99         --- sets last element
```


**A mask picks.** Where the index is a list of bools — what a comparison over a list or a tensor answers — the result is the items where it is true: `names[labels eq j]`. Of a matrix it is the rows, so `mean[x[labels eq j], 0]` is the centre of the rows labelled `j`; of a vector, the numbers: `v[v gt 25.0]`. A mask of another length than the items stops. Frames keep `filter[df, mask]`, which also reads a column's `.ok`.

**Design Note: a mask picks, and counts.** Considered: `filter[t, mask]` for tensors and lists (rejected: a second verb where the index already picks — `x[i]` one row, `x[mask]` the rows); `any[mask]` and `all[mask]` (rejected: `sum[mask] gt 0` and `sum[mask] eq len[mask]` say it, once a mask counts). **Decision (Vijay, 2026-10-08):** the index takes a mask, and a mask where numbers are asked is numbers — `true` 1, `false` 0 — so `sum[v gt 2.0]` counts, and math's `where[mask, a, b]` takes one as it takes numbers. The k-means centre, a loop with an index and a count, is one line.

Out-of-bounds indices (positive or negative) are a runtime error.


### Multidimensional Arrays

Nested brackets create multidimensional arrays. All inner arrays must be the same length.

```
m = [[1, 2, 3],
     [4, 5, 6]]              --- 2x3 array

m[0]                          --- [1, 2, 3] (first row)
m[1][2]                       --- 6

--- creation by shape
z = zeros[3, 4]               --- 3x4 of zeros
r = uniform[0.0, 1.0, 2, 3]   --- 2x3 random in [0, 1)

--- growing (immutable: returns a new tensor)
m2 = push[m, [7, 8, 9]]       --- append one row (shape-checked)
c = concat[m, m2, 0]          --- concatenate along an axis

--- errors: shape and type are the tensor's contract
push[m, [1, 2]]               --- error: row length 2 vs rows of length 3
push[m, 'x']                  --- error: arrays are homogeneous
[[1, 2], [3]]                 --- error: all inner arrays must be the same length
```

Iterating a 2-D array yields its rows; comparisons with a scalar broadcast to a boolean mask:

```
totals = for row in m
    sum[row]                  --- per-row sums

x = [1, 2, 3, 4]
mask = x gt 2                 --- [false, false, true, true]
big = (x gt 1) and (x lt 4)   --- masks combine element-wise
```

A tensor's rank is in its type, and a loop over a tensor walks rows or numbers — an index reads a row or a number — as the type decides, so both modes read a value the same way: a literal states its rank by its nesting, a constructor by its sizes, every builtin by what it answers (`relu[m]` is shaped like `m`, `sum[m, 1]` has one dimension fewer, `flatten` is a vector, `transpose` and `*` a matrix), a parameter by its calls, a record's field by what builds it, a gradient by its parameter. An item of a tensor of floats is a float, whole or not (`type[x]` of `[2.0, 3.0]`'s items is `float`; the interpreter had read `2.0` as the int `2`). Where the rank is not known at a loop or an index — a parameter no call types, a piece of `svd` read by a position the program computes (`parts[1]` has its piece's shape, a vector of singular values; `parts[i]` has none) — the program is refused before it runs, with the fix: `reshape[t, rows, columns]` states a matrix, `flatten[t]` a vector. A tensor's item assigned (`v[0] = 9.0`) is a number; a row cannot be assigned one at a time — a matrix is built anew with `concat`, `stack` or `reshape`.

**Design Note: rank is in the type.** Considered: the interpreter deciding by the value, as it did (rows where the value had two dimensions, a whole number as an int), with the compiled program deciding by the type — the two walked the same matrix differently wherever the type knew no rank, and `type[x]` differed on a float tensor's items; a dynamic walk in the compiled program (rejected: the loop variable would need a type decided as the program runs, and the body with it); a vector assumed where the rank is unknown (rejected: a guess either way is the same bug with another sign). Decided (Vijay, 2026-10-03): the type decides, every builtin's answer has a rank, and where none is known the loop or index is refused before the program runs; both backends read a tensor through one runtime walk that checks the value's rank against the type's and stops with the reason, so a wrong shape rule can never make the modes differ silently.


### String Access

Strings support indexing just like arrays. Each index returns a single-character string. Negative indices count from the end.

```
word = 'hello'
word[0]                  --- 'h'
word[-1]                 --- 'o'
word[2]                  --- 'l'
```

Out-of-bounds indices are a runtime error.

`len[s]`, `slice[s, start, end]`, and indexing count characters, never bytes, in both execution modes: `len['a—b']` is 3 and `slice['a—b', 1, 2]` is `'—'`.


### Maps

Maps are created with the `map[...]` constructor — a variadic call with named arguments. `[]` is always an empty array.

```
items = []                               --- empty array
data = map[]                             --- empty map
counts = map[crow = 3, raven = 7]        --- map with entries
headers = map['content-type' = 'json']   --- quoted keys for special characters
by_id = map[1 = 'a', 2 = 'b']            --- keys written as ints key the map by ints
```

**A map's keys are one kind: text, or ints.** `2` and `'2'` are different keys, and a map holds one or the other, as it holds values of one kind. The literal's names and quoted keys are text; a key written as an int, `map[2 = 'b']`, keys the map by ints; `map[keys, values]` keys it by what the keys are (`map[[1, 2, 3], ['a', 'b', 'c']]`, a tensor of ints too); an empty `map[]` is open, and its first `set` decides. A key of the other kind is refused before the program runs, in both modes — `m['2']` of a map keyed by ints: "the map's keys are ints, not text: `m[2]` reads one, `'2'` is text" — as is a float (`map[[1.5], …]`: a map's keys are text or ints, not floats), and the dot form, which names a text key, is refused on a map keyed by ints (`m.a`: "a map keyed by ints is read by `m[2]`"). `keys[m]` answers the keys as they are, ints in order; `in` asks for a key of the map's kind. A parsed value's object has text keys, so a map decoded from JSON is keyed by text, and a map keyed by ints encodes its keys as text, as `json.dumps` does, since JSON's objects have no other kind.

```
scores = map[[7, 3], [0.5, 0.25]]
scores[3]                --- 0.25
keys[scores]             --- [3, 7]
3 in scores              --- true
scores['3']              --- refused: the map's keys are ints, not text
```

**Design Note: a map's keys are typed.** Considered: text keys only, a number used as a key read as its text, as JavaScript's objects and JSON do (rejected, Vijay, 2026-10-08: "2 is different from '2'", and a key put in as a number came back as text from `keys[m]`); keys of any kind in one map, as Lua's tables (rejected: the map's type would say nothing of its keys, and `2` beside `'2'` is a bug waiting). **Decision (Vijay, 2026-10-08):** one key kind per map, text or ints, as Python's `dict` and Go's `map[K]V` are used — the map's type carries it (`Ty::Map(Key, value)`), the compiled program's map is `BTreeMap<String, _>` or `BTreeMap<i64, _>` by it, the interpreter holds the two apart (`Value::Map`, `Value::IntMap`), and the eight map rows bridge both by hand, since one function cannot be instantiated twice in one closure. Floats are not keys, as in Go and for the same reason. JSON stays text-keyed both ways.

`map[key = value, ...]` desugars to a chain of `set[]` calls — it is ordinary Crow, not special syntax. For fully dynamic keys, build with `set[]`:

```
m = map[]
m = set[m, key_var, value]
```

**Maps are homogeneous: every value must have the same type.** Int and float
unify (like arrays), and numbers held as a list unify with numbers held as a
tensor: the map holds one form, and a value in the other is converted where
it is set — `map[a = [1, 2], c = []]` holds lists, as a list pushed onto a
list of rows is a row (it had stopped the interpreter at the tensor,
`expected a list, got array`, and the compiled program had not built).
Mixed named data belongs in a struct — and since the value
type can be *any* type, including structs, the two compose:

```
struct Address [street, city, zip]

addresses = map[
    home = Address['12 Oak St', 'Austin', 78701],
    office = Address['500 Main St', 'Austin', 78702]
]

addresses.home.city                       --- 'Austin'
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
--- error: a map holds values of one kind: the entry 'b' holds text, and the map's other values are an int —
--- a record (struct) holds fields of several kinds
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


### Map Access

Maps use bare names as keys. Access with dot notation or string key.

```
zips = map[home = 78701, office = 78702]
zips.home                --- 78701
zips['home']             --- 78701 (string key also works)
```

Bare-name keys are stored as strings internally. Use quoted string keys when the key is dynamic or contains special characters:

```
headers = map['content-type' = 'json']
key = 'home'
zips[key]                --- dynamic lookup
```


## 9. Structs

Structs are pure data — no methods. Define with `struct`, create with `[]`.


### Declaration

```
struct Bird [name, speed]
```


### Creation

```
b = Bird['crow', 50]
```

A field holds what it is given, a failed value included: `Bird['crow', int[s]]` is a bird whose `speed` may be an error, not an error itself, so a list gathered from such records is a list, and `b.speed.ok` asks the field.


### Field Access

```
b.name                   --- 'crow'
b.speed                  --- 50
```


### A Record or None

A field may hold a record of its own kind, or `0` where there is none — a
tree's children, a chain's next link:

```
struct Node [op, a, b]
leaf = Node['leaf', 0, 0]
t = Node['+', Node['*', leaf, leaf], leaf]
```

What holds a record or `0` is *a record or none*: such a field, a variable
first given `0` and later a record (or the other way), a function's
parameter handed either, and a function's value answering either
(`fn child[n, side]` answering `n.a` or `n.b`). It passes through all of
them as it is, in both modes. Reading a field of none, or handing none to
a function that takes a record and reads it, stops the program:
`cannot access field 'op' on this value`. A program tells a leaf from a
branch by a field of its own (`op eq 'leaf'`, a `leaf` flag), not by
comparing the record with `0`: two values of two kinds are not compared.


### Structs with Functions

Structs have no methods. Pass structs to regular functions.

```
struct Bird [name, speed]

fn fly[bird]
    log['`bird.name` flies at `bird.speed`mph']

b = Bird['crow', 50]
fly[b]
```


## 10. Error Handling

Every value in Crow is a result. Fallible operations set `ok` to `false` with a `reason`, `kind`, and `trace`. Values auto-unwrap — you use them directly without worrying about the struct.


### Fallible Builtins

These builtins can fail and return an error-value:

| Builtin | When it fails |
|---------|---------------|
| `read[path]` | File doesn't exist or can't be read; a URL or an S3 object that does not answer (Files, URLs and S3) |
| `write[path, content]` | File or S3 object can't be written; a URL, which is read, not written |
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
| `load[path]` (frame, math) | File, URL or S3 object missing or unparsable |
| `csv[path]` / `lines[path]` (io) | File, URL or S3 object missing or unreadable |
| `query[conn, sql]` | SQL syntax error or query execution fails |
| `exec[conn, sql]` | SQL syntax error or execution fails |
| `insert[conn, table, frame]` | Bulk insert fails (schema mismatch, connection error) |
| `transact[conn, fn]` | The function failed (its error, the transaction rolled back), or the commit was refused |
| `migrate[conn, dir]` | Migration fails (bad SQL, directory not found) |

Non-fallible conversions (e.g. `int[3.14]`, `int[true]`) return raw values with `ok: true`.


### Every Value Has Status

```
x = 5
log[x]            --- 5
x.ok              --- true
x.reason          --- ''
x.kind            --- ''
x.trace           --- []
```

Non-fallible values always have `ok` of `true`, `reason` and `kind` are `''`, and `trace` is `[]` — decided before the program runs, from the value's type. What is read for them is still read: `loud[4].ok` calls `loud`, and `xs[9].ok` past a list's end stops, in both modes (compiled, the value read for them had been skipped).

A list is a value like any other: `names.ok` is `true` for a list that did not fail, whatever it holds, so `if not listing.ok` guards a listing that failed and nothing else. The one per-row reading is a frame column's: `df.age.ok` is its cells' validity, `false` where a cell is missing (see frame).


### Auto Unwrap

```
content = read['file.txt']
log[content]               --- just works, auto-unwraps to the value

val = int['42']
doubled = val + val        --- auto-unwraps in arithmetic: 84
```

When an error-value is used in arithmetic, string interpolation, or `log[]`, the inner value is automatically accessed.

A value that failed, shown — logged, in text, in a list — says the line it failed on and why: `error: line 4: cannot read 'trades.csv': No such file or directory`, the first place its `trace` names (inside a standard module's function, the line of the program's call). It had shown the reason alone, and a program that logged a failed conversion did not say which line had failed. A frame's missing cell and a parsed JSON null, which no line made, show as `error: missing` and `error: null`.


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

**An error has no truth value, and no items to walk.** A comparison with an error-value operand does not produce true or false — it propagates the error, like every other operator. And a branch cannot be taken on an error, nor a loop run over one: when an `if` or `while` condition evaluates to an error-value, or the list a `for` walks does (`for i in range[len[r]]` when `r` failed), the decision is refused —

- **inside a function**: the function immediately returns the error to its caller (auto-propagation, trace appended — a place the trace already ends at is not named twice, as when the refusal is on the line the error came from) — the same rule as error arguments, applied to decisions
- **at top level**: the program halts: `runtime error: line N: cannot branch on an error value — <reason>`, or `cannot loop over an error value` for a loop

A variable whose first value could not fail refuses an error the same way where it is assigned (see Reassignment): `cannot assign an error value to 'x'`.

**A value that failed has no fields or items to read.** A frame's column, a record's field, a map's key or a parsed value's field, read from a value that failed, is refused the same way — a function hands the error back, a handler stops, the top level halts: `runtime error: line N: cannot read 'price' of an error value — load: No such file or directory (os error 2)` — and so is an item: of a list, a text, a map by its key, a column, a parsed value (`lines[path][0]` when the file is not there: `cannot read an item of an error value — lines: No such file or directory`; it had been the error, carried on). A guard (below) makes the read an ordinary one. A record holding a field that failed did not fail itself: `p.ok` is true, and `p.x.ok` asks the field (see Structs). (Compiled, the read had found the failed value's placeholder — `frame has no property or column 'price'` for a load that found no file, JSON's `null` for text that did not parse — where the interpreter went on with the reason; and a record's `.ok` had not built.)

**Design Note: a read refuses a value that failed.** Considered: carrying the error through a read as an operator carries it — a field of a failed value the error, an item of a failed list the error. Decided: a read refuses it, as a decision does, field and item alike (Vijay, 2026-10-01: one rule for reading into a value, and failing where the value is first read is safer in production than carrying the error on). A frame's column read from a load that failed would make every column of the frame a result, carried through each mask, sum and join that reads it, where the reason belongs at the line that first reads the frame; and the interpreter's carrying on and the compiled program's placeholder were where the two modes had parted. An operator still computes with an error (`x + 1` of a failed `x` is that error): it reads no part of it.

```
fn check[s]
    n = int[s]
    if n gt 5            --- if n is an error: check returns it here
        return 'big'
    return 'small'

r = check['abc']
log[r.ok]                --- false — the parse error propagated out
log[r.reason]            --- cannot convert 'abc' to int
```

The same holds when a function hands back an error of another shape — `if not b.ok` / `return b` in a function whose value is a number, where `b` is a frame: the caller receives the error with its reason, kind and trace, and the value is the function's default. A handler has no caller to hand an error to. **A handler stops when an error value reaches a decision, a fact, or an effect** — an `if` or `while` condition, the list a `for` walks, an assignment to reactive state or to a variable whose first value could not fail, or an argument of `query`, `exec`, `insert`, `transact`, `read`, `write`, `fetch`, `shell`, `later`, `notice` or `sign_in`, or of a function that writes facts called for what it writes (a statement, or the handler's last line, whose value nobody reads) — and nothing it did lands: its fact writes are discarded, its database writes are rolled back, its held effects are dropped. The reader is told, as the session's notice, that it stopped and that nothing changed, with an id — `apply stopped, and nothing changed — the server's log has why, as 3f9a1c07` — and the served program's log has the reason beside the id (`crow live: apply stopped [3f9a1c07]: cannot convert 'x' to float`): a stop's reason is the runtime's own words, which can name a database's address or a file's path, so it goes no further, as a failed route's does (Vijay, 2026-10-01). Under `crow dev` the dev panel lists the stopped event with its reason and id. A local whose first value could fail, holding an error and guarded later, computes as it always has: `.ok`, `.reason`, `.kind` and `.trace` never propagate, so `if not r.ok` is how a handler does something other than stop. In the harness, `click[a, apply]` evaluates to that error value and the session's facts are as they were before the click. An event that does not fit the program — no handler of its name, or not the arguments the handler takes (`click[a, apply]` where `apply` takes one) — stops the same way, on a served page and in a test, in both modes. A fact never holds an error: a derivation, which is a belief, may; a fact, which the reader acts on, keeps its last good value.

**Design Note: a loop over an error is refused.** Considered: running such a loop zero times, which the interpreter did for the statement form (the collecting form made a one-item list of the error). Decided (Vijay, 2026-09-23): refused like a condition, so an error never vanishes into an empty loop. A function whose loop walks a list that can fail can fail itself, as one with a condition that can; a guard before the loop narrows (below), so a guarded loop is an ordinary one.

The metadata fields `.ok`, `.reason`, `.kind`, `.trace` never propagate — they are always readable, so the guard idiom is always available and is the one bridge from error-space back to decisions:

```
age = int[input['age: ']]
if not age.ok
    log['invalid: `age.reason`']
else if age gte 18
    log['adult']
```

Checking is optional: the happy path needs nothing, and unguarded errors fail loudly at the decision instead of silently taking a branch. (Frame masks are unrelated: missing cells excluded by `filter` are validity-bitmap missingness, not error-values.)

**A guard narrows.** After `if not x.ok` with no `else`, whose branch always leaves (it ends in `return`, `break` or `continue`), `x` is known good for the rest of the block: reading it reads its value, which cannot fail. `if not a.ok or not b.ok` narrows both. Assigning `x` again ends it, and so does the end of the block; a loop's body does not inherit a name the body assigns, since an earlier pass through it may have made the value fail again.

```
fn size[path]
    t = read[path]
    if not t.ok
        return 0
    return len[t]            --- t is known good: len[t] is a number, not a result
```

So a function whose only failures are guarded is not typed as able to fail, and a loop over `range[len[t]]` after the guard is an ordinary loop.

#### Design Note: Errors Have No Truth Value

**What was considered:** treating error-values as false in comparisons and conditions (SQL NULL / IEEE NaN style: `err gte 18` → false).

**Why it was rejected:** fabricating an answer silently misclassifies data — bad input would always take the `else` branch (an unparseable age becomes a 'minor'). An error cannot be compared with a value; pretending otherwise is the Go zero-value bug with different spelling. Instead: **you can compute with an error, but you cannot decide with one.** Propagation is implicit through dataflow (like Odin's `or_return`, applied automatically); the refusal happens at the last moment that doesn't fabricate. A future `crow check` lint can flag unguarded fallible conditions statically.


### Creating Errors

Use the `error[]` builtin to create custom error-values. An error made this way has no value, only its reason and kind, so whatever it meets hands it on: `error['boom'] gt 0` is that error, in both modes.

```
fn validate[age]
    if age lt 0
        return error['age cannot be negative', 'validation']
    return age

result = validate[-5]
log[result.ok]          --- false
log[result.reason]      --- 'age cannot be negative'
log[result.kind]        --- 'validation'
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

The kinds the language itself produces — branch on `.kind` when a category matters, not on the text of `.reason`:

| `kind` | Produced by |
|--------|-------------|
| `'parse'` | `int[]`, `float[]`, `json[]` |
| `'io'` | `read`, `write`, `mkdir`, `ls`, `rm`, `mv`, `cp`, `input`, `csv`, `lines` |
| `'db'` | `connect`, `query`, `exec`, `insert`, `transact`, `migrate` |
| `'frame'` | `load`, `save`, and `with` given values of another length than the frame's rows |
| `'load'` | `math.load` |
| `'env'` | `env[name]` when the variable is not set |
| `'shell'` | `shell[cmd]` when the command cannot be started (a non-zero exit is not an error — read `.status`) |
| `'time'` | `moment`, `parse_time`, `calendar`, `fmt[t, time =]`, and `use frame`'s `datetime` and `tz` |
| `'regex'` | `matches`, `find`, `sub` with a pattern that is not one |
| `'no_capture'` | `captured[x]` on a value that was computed, not read |
| `'conflict'` | a write refused by a database constraint (see `crow-check.md`) |
| `''` | `error[reason]` with no kind; `find` with no match |

Your own `error[reason, kind]` values use whatever kind you choose (`'validation'`, `'network'`, …).

```
x = int['hello']
log[x.ok]         --- false
log[x.reason]     --- 'cannot convert ...'
log[x.kind]       --- 'parse'
log[x.trace]      --- ['main:1']
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

data = int['bad']           --- error-value
result = pipeline[data]     --- error propagates through pipeline → process
log[result.ok]              --- false
log[result.reason]          --- 'cannot convert ...'
log[result.trace]           --- ['main.validate:5', 'main:6', ...]
```

**Exception:** `log[]` does not propagate — it prints error-values directly for debugging. `len[]` is no exception: the length of an error is the error.

**Exception:** elements, controls and views render what they are given. An error-value handed to an element renders as the text `error: the server's log has why, as <id>` in place of that child, and one handed to a control (`field`, `select`, `slider`, `checkbox`, `toggle`, `radio`, `tabs`, `progress`, `icon`), a `chart`, a `widget`'s props or a `view` renders as that text in place of the control or the view; the tree is never wrapped in a Result. A page with one failed part is still a page. The reader is told an id, not the reason: a reason names what the program reaches — a database's address, a file's path — and a page is read by whoever has it open, as a handler that stopped tells its reader an id (Reactive State, The notice). The server's log has the reason beside the id, once: the id is the same for the same reason, so a page that renders the failure on every tick is stable. A derivation still being computed shows `error: loading` until its value lands, as before — not a failure, and nothing is logged.

**Collections:** a `map[...]` built with a fallible value is a fallible map of plain values (the map is built from the unwrapped value or not at all), and `encode[]` of an array of fallible values writes each ok element's payload and `null` for each error.

Errors also propagate through operators (`+`, `-`, etc.) and unary operations. If either operand of a binary op is an error, the result is that error, its trace unchanged: a call that receives an error, and a branch refused inside a function, are the places a trace records. A trace names the file by its stem (`main.check:14`), in both modes.


### Error Messages

What is not a program — an undefined name, a builtin's name taken, a call with the wrong number of arguments — is refused before anything runs, every such error at once. Each is printed as the file, line and column of what the program wrote, the message, and the line with that text underlined:

```
ex1.co:1:8: frame.new[] is frame's: add `use frame`
  1 │ ours = frame.new[node = keys[m], value = values[m]]
    │        ^^^^^^^^^
ex1.co:1:30: undefined variable 'm'
  1 │ ours = frame.new[node = keys[m], value = values[m]]
    │                              ^                  ^
2 mistakes
```

A refusal is the same in both modes (it is printed before either runs) and in the language server, which underlines the same text and offers the fix. What fails while the program runs stops it with the line it failed on, `runtime error: line 7: assertion failed: expected positive` (When a Program Stops).

A message is held to five rules, which `crow test --errors` checks over a corpus of wrong programs (`tests/errors/`), in both modes and in the language server, and `crow test --fuzz n --mistakes` over programs the fuzzer generates, one mistake planted in each — a slip in a builtin's name, a call written with `( )`, another language's operator, a bracket left open, two statements on a line, an import left out — each refusal held to the line of the mistake and the fix it asks for:

1. **It names what the program wrote**, or what to write instead — never a form the compiler made. `frame.new[…]` and `map[…]` are rewritten before they are checked; a mistake in one is refused as written.
2. **It says the fix.** A function a module provides that the program does not import, called bare or through the module's name, says what to import (``frame.new[] is frame's: add `use frame` ``); one a different module provides says which (``frame has no 'mean' — mean[] is math's: add `use math` ``); a name two modules provide names both, with what each answers; a name the core has whose form for this kind of value is a module's says that module (``abs[] of a column is math's: add `use math` ``). A name one slip from a name in view — one edit in a name of up to five characters, two in a longer one, a swapped pair counting as one — says which, whether the name is the program's, a keyword or any module's: `undefined function 'scroe' — did you mean 'score'?`. A `use` of a module that does not exist is refused.
3. **One mistake, one message.** A name the program never defines is refused where it is read, once per line however often the line reads it, and what is computed from it is not refused again for being unknown; a module the program lacks is named once, at the first line that needs it, with every name that line calls of it.
4. **It points at the mistake**: the line and column of the text it is about, not the statement around it or the line after it.
5. **It is never the compiler's own.** A program refused by Crow is refused as a program; "a bug in Crow" is for Crow's bugs.

The parser speaks in the program's words, never its tokens': `f(1)` says calls use square brackets, a bracket never closed points at where it opened, a line indented under nothing says so, a name one slip from a keyword says which (`esle` — did you mean `else`?), and the habits of other languages are taught their Crow form where they are met — `==`, `&&`, `**`, `++`, `{…}`, `#`, `;`, `:` (after a block's first line, in an index, after `lambda`), `def`, `print`, `None`, `True`, a method call (`df.join(…)` is `join[df, …]`, `xs.append(v)` is `xs = push[xs, v]`).

Some mistakes run quietly unless refused, and are: a builtin that answers a new value of the kind it was handed — `push`, `set`, `sort`, `upper`, `with` — called as a statement of its own (`push[xs, v]` alone answers the longer list and drops it: `xs = push[xs, v]` keeps it; a collecting loop's last line and a console cell's hand their value on, and are not dropped), a `return` outside every function, view and derivation, `break` or `continue` outside a `while` loop, and a `use` of a module there is none of.

The language server offers each fix the message names (an import added, a slip corrected, a habit in its Crow form) as a quick fix, and `crow fix file.co` applies them all in place, round after round — a parse error hides the mistakes after it — then lists each change at its line and prints what is left for the author: a name the program never defines, a module two ways out of (`add `use frame` or `use math``). One reading of the messages (`diagnostic::fixes`) serves both.


### When a Program Stops

Some failures no value can carry. An index reads an element and an int operation makes an int, and neither an element nor an int can say it failed. These failures stop the program on the line they happen on, with the same words in both modes: `runtime error: line N: <reason>` on stderr, exit status 1. Inside a function the line is the function's own. In a loop whose iterations run in parallel it is the body's line, for the earliest item that failed, printed once. A handler that meets one stops as it does at a decision: nothing it did lands, and the reader is told so, the reason kept for the server's log.

| What | The reason |
|------|------------|
| `xs[i]` outside the array (a negative index counts from the end: `-1` is the last) | `index 5 is out of range for an array of 3 items` |
| `s[i]` outside the string, counted in characters | `index 9 is out of range for a string of 3 characters` |
| `m[i]` outside a matrix's rows | `index 4 is out of range for a tensor of 2 rows` |
| `xs[i] = v` outside the array | as reading it |
| `m[k]` or `m.k` of a key the map lacks | `map has no key 'k'` |
| `df.col` of a column the frame lacks | `frame has no property or column 'col'` |
| an int `/` or `%` by zero | `division by zero`, `modulo by zero` |
| an int past 64 bits handed where a 64-bit one is taken (a position, a size, a count, a map's key, a frame's cell) | `a whole number past 64 bits where range[] takes an int of 64 bits: 1267650600228229401496703205376` |
| `int`, `floor`, `ceil` or `round` of an infinite float | `inf is no whole number: an int of an infinite float` |
| an int to a negative power | `2 ^ -1: an int to a negative power is a fraction — write 2.0 ^ -1 for a float` |
| `assert[c]` or `assert[c, message]` false | `assertion failed`, `assertion failed: message`; a comparison, `assert[a eq b]`, says both sides: `assert failed: eq`, then `left: …` and `right: …` as they show, in both modes (compiled, it had said only `assertion failed`) |
| `halt[reason]` | the reason, as written — a program's own precondition, and a standard module's written in Crow (`math.nn`), on the line of the call in the program |
| a recursive function's calls nested past 10000 deep (the line is the function's first) | `recursion deeper than 10000 calls — a function that calls itself needs a case that returns without calling` |
| a builtin given what it cannot take: a negative size, an axis or a window outside its range, shapes or lengths that do not meet, a frame's column with a missing cell where numbers are taken | the builtin's own reason: `zeros: a size cannot be negative (got -2)`, `sma: a window is 1 row or more (got 0)`, `corr: x and y must be one length, got 3 and 2` |
| a database connection that cannot open (a bad url, a server that does not answer), or whose database does not hold the tables the program declares on it (§13 db, The schema a program declares) | the database's reason: `unknown database URL scheme: nosuch://x`, `the database at sqlite://board.db does not hold what the program declares (schema = 'verify'):` and each difference (compiled, a connection that could not open had stopped on a Rust unwrap) |

An int never wraps round and never stops for its size — it grows (Numbers That Keep Every Digit) — and a float never stops: a float divided by zero is IEEE's infinity, and `int[]` of a float past 64 bits is the whole number it holds. A `range` that would step past the last int ends there. A check Crow is missing, a panic inside the runtime, stops the program the same way and never crashes it.

#### Design Note: The Failures No Value Carries Stop the Program

**What was considered:** making an index outside its array, or an int divided by zero, an error value, as `int['x']` is.

**Why it was rejected:** every `xs[i]` and `a / b` would be fallible, so every function that indexes or divides would be typed as able to fail. The two modes had drifted apart instead of choosing. The interpreter answered an error value for a division by zero where the compiled program stopped. It wrapped an overflowing int where `crow run` stopped and `crow build`'s binary wrapped. And each mode used its own words for an index out of range. Now each failure is one check in the runtime (`crow_runtime::stops`), called by both modes, so they agree by construction (design rule 12).


## 11. Core Builtins

The functions available in every file without an import. These names are reserved — a definition may not reuse them (see Design Note: Names Are Never Ambiguous). Module functions (`use math`, `use frame`, …) are in section 13.

A builtin handed a number of arguments, or a kind of argument, it does not take is refused before the program runs, in both modes, naming what it takes: `min['a', 'b']` is `min[] takes an int and an int or a number and a number, got text and text`. (The interpreter had stopped on the value while the compiled program did not build — or, for `min` of text, compared it.) A module's function is refused alike: `mean['a']` is `mean[] takes numbers, got text`. A parsed JSON value is read as one type before a builtin computes with it (see JSON). An argument whose kind shows only as the program runs — an item of a list of parsed values, a cell no description typed — is read then, by the same rule.


### Core

| Function | Description |
|----------|-------------|
| `log[value]` | Print a value to stdout, as it shows: text bare, a number as it displays (a whole float as an int), a list as `[1, 'a']` with its text quoted, a map as `[key = value]`, a record as `Point[x = 1, y = 2]` (its fields in name order; a module's record by the name it was declared by, as `type` names it), a value that failed as `error: reason` — the same in both modes, through one display (`crow_runtime::display`). To a terminal, each line as it is logged; a script whose output goes to a file or a pipe writes it in blocks — before the program sleeps, at least every tenth of a second whatever else it is doing (so a reader watching it sees each line within that), at the end, before a stop is reported, at `exit` and before `input` reads (a line at a time had been a write to the system per line). A program that serves pages writes each line as it is logged |
| `range[stop]` | Array of ints from 0 to stop (exclusive); every argument of `range` is an int (`range[2.5]` is refused) |
| `range[start, stop]` | Array of ints from start to stop (exclusive) |
| `range[start, stop, step]` | Array of ints with custom step; a negative step counts down (`range[5, 0, -2]` is `[5, 3, 1]`); a zero step is an error |
| `map[]` | Create an empty map |
| `map[keys, values]` | A map from a list of keys and a list of values, in order — the key first, as the literal and `set` write it. Lists of different lengths, or a key that repeats, stop; a tensor of values makes a map of numbers (see Design Note: a map from two lists) |
| `assert[cond]` | Halt with error if condition is falsy |
| `assert[cond, msg]` | Halt with custom error message |
| `error[reason]` | Create an error-value with the given reason |
| `error[reason, kind]` | Create an error-value with reason and category |
| `cpu[n]` | Set thread count for parallel loops |
| `gpu[n]` | Set GPU count (`gpu[]` for all GPUs) |
| `nodes[n]` | Set machine count for distributed compute |

### Math

| Function | Description |
|----------|-------------|
| `abs[n]` | Absolute value |
| `sqrt[n]` | Square root (returns float) |
| `floor[n]` | Floor (returns int) |
| `ceil[n]` | Ceiling (returns int) |
| `round[n]` | Round to nearest int, a half away from zero (`round[2.5]` is `3`, `round[-2.5]` is `-3`); `floor`, `ceil` and `round` of a float past 64 bits are the whole number it holds, every digit, of NaN `0`, of an infinite float a stop |
| `base ^ exp` | Exponentiation (use `^` operator) |
| `min[a, b]` | Minimum of two numbers |
| `max[a, b]` | Maximum of two numbers |

### Strings

| Function | Description |
|----------|-------------|
| `split[s, delim]` | Split string into array |
| `trim[s]` | Remove leading/trailing whitespace |
| `replace[s, old, new]` | Replace all occurrences |
| `occurs[s, part]` | How many times `part` occurs in `s`, not overlapping, as Python's `str.count` counts (`occurs['aaaa', 'aa']` is 2; a part that cannot overlap itself counts every place it is). The search reads the text once at the machine's speed, so counting in a long text costs what reading it does — where `len[s] - len[replace[s, part, '']]` had made a new text for every count. An empty part is refused |
| `translate[s, from, to]` | Each character of `from` in `s` replaced by the character at its place in `to`, all in one pass — `translate[dna, 'ACGT', 'TGCA']` is the complement, where `replace` in turn needs a stand-in letter to keep one replacement from meeting another's (Unix's `tr`, Python's `str.translate`). `from` and `to` have as many characters, or the program stops with both counts |
| `upper[s]` | Convert to uppercase |
| `lower[s]` | Convert to lowercase |
| `starts[s, prefix]` | Check if string starts with prefix |
| `ends[s, suffix]` | Check if string ends with suffix |
| `join[arr, sep]` | Join array elements into string: text as it is, a number as it displays, a bool as `true` or `false`; a frame's column's cells too, where a missing cell stops the program, as it does where numbers are taken; a list whose items can fail (a collecting loop's over a value that can) item by item, a failed item stopping the program with its reason — `item 1 of the list failed: cannot convert 'x' to int` |
| `repeat[s, n]` | Repeat string `n` times |
| `fmt[x, ...]` | Format a value with named arguments (see Formatting) |
| `reverse[s]` | Reverse a string |

**A value handed to a function is the function's own, and a function that only reads it is lent the caller's.** A parameter the body never assigns (nor an item of), never binds anew and never hands on — it is indexed, measured, walked, compared, given to a builtin that reads it, or handed to a function that only reads it in turn — reads the caller's list, map or text where the caller holds it, in both modes; any other parameter gets its own copy, so what the function does to it the caller never sees. (A call had copied every list it was handed: a long list given to a function called once per item was quadratic — revcomp's lines ran 130 times slower than the same loop without the call.)

**A value read for the last time moves; one read again is copied.** A variable's last read — returned, pushed, assigned, handed to a function or a builtin that keeps it — hands its value on without a copy, in both modes, and so does a local of a loop's body, or the loop's variable, at its last read in a round (the next round binds it anew: spec, Scope). A text nothing else holds is changed where it lies: `s = replace[s, old, new]` of one ASCII character by another, and `upper`, `lower` and `reverse` of a text read for the last time or of another call's answer. Any other read copies, so a name still holds what it held. A value that can fail, held by a name, is read where it lies to be checked and handed on. (A function's `return out` had copied the list it returned — `lines` copied every line it read — a loop body's local was copied into the list that kept it, a file's lines held at the top level were copied whole twice by every call they were handed to, and each of twelve `replace` calls made a new text.)

**A text read by position costs what is read.** A slice, a character and a length count characters, and a text holds UTF-8, where a character is one to four bytes. A text a function reads by position — a parameter, or a local it does not give a new text while the call runs — keeps, in both modes, how far its start is known to be ASCII, checked only as far as a read has needed: within that start a character is a byte, so `slice[seq, i, i + k]` reads `k` characters wherever `i` is, and a loop over a long text's windows is linear (each read had walked the text from its start, and counting k-mers by slices was quadratic). Past the ASCII start, a read walks only from where it ends. A slice compared with a text (`slice[seq, i, i + k] eq pat`) is compared where it lies.

### Formatting

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

#### Design Note: No Format Mini-Language

**What was considered:** Python-style format specs inside interpolation (`{pnl:,.2f}`) or as spec strings (`fmt[x, '.2f']`).

**Why rejected:** format codes are a second grammar — cryptic single-character commands with their own parser, embedded in strings. That violates the no-DSL rule and requires memorization; nobody writes `%-+8.2e` without a lookup. Named arguments are ordinary Crow: self-documenting, validated at compile time (unknown names and arity are lowering errors), and guessable without docs.

**The trade:** one-off formatting is wordier than `{pnl:,.2f}`. The sanctioned recovery is the named formatter (`money[x]`), which is *shorter* than the Python spec at the call site, has a domain name, and keeps every money field in the project formatted identically — the consistency that per-slot codes never provide.

### Conversion

| Function | Description |
|----------|-------------|
| `int[value]` | Convert to int: a float is cut toward zero (the whole number it holds, every digit, past 64 bits; an infinite float stops), a decimal or a fraction toward zero, a bool is 1 or 0, text is parsed as written — no spaces around it, every digit kept — and text or a JSON value that names no int is an error value of kind `'parse'` |
| `float[value]` | Convert to float, as `int` reads its value; an int past 64 bits, a decimal and a fraction are the nearest float |
| `decimal[value]` | An exact base-ten number (Numbers That Keep Every Digit): of text as written, its places kept (`decimal['12.30']`; text that writes no number is an error value of kind `'parse'`), of an int exactly, of a float as the shortest text that reads back as it (`decimal[0.1]` is 0.1). `decimal[x, digits = n]` rounds it to `n` significant digits, a half away from zero, and every answer it takes part in to the most digits of the decimals in it: the language's big float (`sqrt[decimal[2, digits = 50]]`) |
| `fraction[n, d]` | The exact ratio n/d in lowest terms, its sign on the top (`fraction[2, -4]` is `-1/2`); a bottom of 0 is an error value of kind `'math'`. `fraction[x]` of an int or a decimal is that number exactly (`fraction[decimal['0.25']]` is `1/4`) |
| `str[value]` | The text a value shows as, as `log` shows it (`str[[1, 2]]` is `'[1, 2]'`). (Compiled, a record holding a list, a list of records and a map of them had shown in Rust's debug form, and a JSON list as its JSON text.) |
| `type[value]` | Type name as string: `'int'`, `'float'`, `'str'`, `'bool'`, `'array'`, `'map'`, `'fn'`, `'none'`, `'error'`, `'frame'`, `'sparse'`, `'db'`, or struct name — the same in both modes: a frame's column is an `'array'`, a parsed JSON value what it maps to, and `shell`'s and `find`'s records `'map'` |

```
type[42]          --- 'int'
type[3.14]        --- 'float'
type['hello']     --- 'str'
type[[1, 2]]      --- 'array'
type[Point[1, 2]] --- 'Point'
type[error['x']]  --- 'error'
```

### Collections

| Function | Description |
|----------|-------------|
| `len[x]` | Length of a list, of text (in characters), of a map (its entries), of a tensor (what a loop walks and an index reads: a vector's numbers, a matrix's rows — `len[flatten[m]]` counts every number), of a frame (its rows), of a frame's column (its cells, a missing one too). `x.len` is the same length; a record's field, a map's key or a frame's column named `len` is its own |
| `push[arr, value]` | Return new array with value appended; onto a tensor, a number onto a vector or a row onto a matrix. A list holds one kind: an item of another (`push[['a'], 1]`) is refused |
| `pop[arr]` | Return new array without last element; an empty array stays empty |
| `concat[a, b]` | Return new array merging two arrays: `a`'s items, then `b`'s. Given one list written down, `concat[[a, b, c]]`, its items joined in order, whatever two of them join — lists, text, tensors, bytes (see Design Note: one word for joining). An empty `a` takes its items from `b` (`idx = []`, then `idx = concat[idx, [i]]` makes a list of ints) |
| `concat[a, b, axis]` | Concatenate two tensors along an axis |
| `sort[arr]` | Return new sorted array: numbers ascending, NaN last; text by character code (`'B'` before `'a'`). A list of anything else — bools, records, lists — has no order, and is refused |
| `reverse[arr]` | Return new reversed array; text reversed by character |
| `contains[arr, value]` | The row behind `value in arr` (see Membership; the name is not written): whether an item is `value`, compared as `eq` compares (a record field by field), so a value of another kind than the items is refused; of text, whether it holds the part (`'ell' in 'hello'`); of a map or a parsed value, whether it has the key |
| `any[arr, pred]` | True if any element passes the predicate (parallel, early exit) |
| `all[arr, pred]` | True if every element passes (parallel, early exit) |
| `find[arr, pred]` | First element passing the predicate; error-value if none |
| `slice[arr, start, end]` | Elements from `start` to `end` exclusive, both clamped to the array, empty when `start` is not before `end`; a string slices by character |

### Map Operations

| Function | Description |
|----------|-------------|
| `keys[m]` | The map's keys as they are — text, or ints in order |
| `values[m]` | Array of map values |
| `has[m, key]` | The row behind `key in m` (see Membership; the name is not written) |
| `set[m, key, value]` | Return new map with entry added/updated; a value of another kind than the map's is refused, as is a key of the other kind (an empty map takes its first key's kind) |
| `del[m, key]` | Return new map with entry removed (a key of the other kind is refused) |

**Design Note: one word for joining.** Considered: `gather[chunks]` beside `concat[a, b]` (rejected: two words, and `concat[a, concat[b, concat[c, …]]]` eight brackets deep in the Postgres driver where a list of pieces would do). **Decision (Vijay, 2026-10-08):** `concat` given one list written down joins its items two at a time, in the first lowering, so every kind two pieces join is joined; a list of bytes held in a variable is `concat[chunks]`, the row `gather` was. A list of lists held in a variable is not joined: its items' kind is the lists', which one function over any item kind cannot hold in both modes — write the pieces, or a loop.

**Design Note: a map from two lists.** Considered: a named form, `map[keys = ks, values = vs]` (rejected: the literal already means a map with a key called `keys`); `zip` (rejected, see No Enumerate or Zip). **Decision (Vijay, 2026-10-08):** `map[keys, values]`, positional, the key first as everywhere a key is written — `map[k = v]`, `set[m, key, value]`. The loop that set one entry per word, in the retrieval package and twenty-four other places, is one line.

A collection builtin takes the kinds its forms above say — a list, a map, text, a tensor — and any other is refused before the program runs, in both modes, naming the forms it takes: `1 in [1, 2]` is `has[] takes a map and text, got numbers and an int`, and `len[5]`, `keys[[1, 2]]` and `'a' in m` of a map are refused alike. A JSON value's kind shows only as the program runs, so `len` of one that is a number or a bool stops the program with the same words. (Compiled, those calls did not build, where the interpreter stopped as it ran; and `len` of a JSON number was 0.)

**All builtins are immutable.** No builtin modifies its arguments. To update a variable, reassign: `nums = push[nums, 4]`. Both modes turn `x = push[x, v]` (and `pop`, `sort`, `reverse`, `concat`, `set`, `del` in the same shape) into an in-place update: the collection is moved into the operation, not copied, when the right-hand side reads `x` exactly once.

### I/O

| Function | Description |
|----------|-------------|
| `read[path]` | The text of a file, a URL or an S3 object (error-value on failure; Files, URLs and S3) |
| `write[path, content]` | Write text to a file or an S3 object (error-value on failure) |
| `exists[path]` | Check if file/directory exists (returns bool) |
| `mkdir[path]` | Create directory (and parents). Error-value on failure |
| `ls[path]` | List directory entries as sorted array of strings. Error-value on failure |
| `rm[path]` | Remove file or directory (recursive). Error-value on failure |
| `mv[src, dst]` | Move/rename file or directory. Error-value on failure |
| `cp[src, dst]` | Copy file. Error-value on failure |
| `args[]` | CLI arguments as array of strings: those after the source file when `crow` runs it, after the program when it runs compiled |
| `input[]` | Read a line from stdin (error-value on failure) |
| `input[prompt]` | Print prompt, then read a line (error-value on failure) |

A file operation that answers nothing — `write`, `mkdir`, `rm`, `mv`, `cp`, and `use frame`'s `save` — answers empty text when it succeeds, as its type says, in both modes. (The interpreter had answered `done`, and the compiled program `''`.) What reads the world outside the program — `read`, `ls`, `shell`, and the modules' `load`, `csv` and `lines` — is stamped with the moment it was read (`captured[x]`), in both modes, and nothing else is: the compiled program had stamped `rm`'s answer.

#### Files, URLs and S3

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

The readers are written in Crow (design rule 20): the protocol is `sys`'s own, in `src/modules/sys.co` — a URL got with the options and its redirects followed, an S3 object signed with the environment's credentials as Signature Version 4 says and reached by path at an endpoint — and each reader is a few lines in its module's source (`src/modules/core.co`, `io.co`, `frame.co`, `math.co`) over the layer's parsers; every row above keeps its contract in both modes. Their tests are their own (`crow test src/modules/sys.co` checks the signature against AWS's documented example), and `tests/reader_headers.rs` and `tests/s3_endpoint.rs` run them against servers of their own in both modes.

**Design Note: one `load`, wherever the data is.** Considered: an HTTP client (`get`, `post`), an `s3` module and a `parquet` module, each with its own readers. Decided: where the data is and what shape it is in are the path's, not the program's, so every reader takes a URL or an S3 object as it takes a file, and the extension names the format — one way, which a new source or format joins without a new name. A key an API asks for is part of where the data is, so a reader sends headers with its URL (`headers = map[…]`, Vijay, 2026-10-01), and so is the question a URL asks (`query = map[…]`, 2026-10-01); other methods, a body, a form and files are `request`'s (### Requests).

### JSON

| Function | Description |
|----------|-------------|
| `json[str]` | Parse JSON string into a Crow value (error-value on failure). Indexing the value reads its elements; a parse that failed, indexed, hands its error on, and an element that is JSON's `null` — or one the value lacks: past a list's end, a negative position, a key an object does not hold — is an error value of reason `null` (kind `'parse'`), in both modes |
| `encode[value]` | Encode a Crow value as a JSON string: a record as an object of its fields, a tensor's whole numbers as ints (a float alone keeps its point: `encode[2.0]` is `2.0`), a NaN or infinite float as `null`, an error value as `null`; a function, a frame and a sparse matrix have no JSON and are refused. (The interpreter had refused to encode a record, and the compiled program did not build one.) |

A parsed value is JSON in both modes, held as it was parsed: `type` says what it holds — `map`, `array`, `str`, `int`, `float`, `bool`, or `error` for `null`. It shows as a Crow value of that kind would (`[1, 2.0]` shows as `[1, 2]`), encodes back as that value would (`[1,2]`), and a parsed object answers `keys`, `values` and `has`, in both modes (compiled, those did not build). A field a parsed object lacks, `j.missing`, is JSON's `null`: its `.ok` is false and its `.reason` is `null`. (The interpreter had converted a parsed value to lists, maps and tensors: an item past a parsed list's end, or a field a parsed object lacked, stopped the program where the compiled one read `null`.)

A parsed value is dynamic until the program reads it. In text it shows as the one display shows it — `` 'name: `j.name`' `` is `name: crow`, a list or a map as it prints, JSON's `null` as `error: null` — as a dynamic cell of a frame does (compiled, text had shown with JSON's quotes, `name: "crow"`). **A parsed value is read as the kind its use implies**, in both modes: a position (`xs[j.i]`) an int, a key (`m[j.k]`) text, one met in arithmetic or compared in order with a number a float (`j.n + 1`, `j.n gt 1`; `%` an int), one compared with text text, one decided on, negated or joined with `and`/`or` a bool, one met beside a moment a moment, one handed to a builtin the kind its rows take (`upper[j.name]` text, `abs[j.n]` a number), one assigned to a variable of a kind that kind (a number as a float: the variable widens, as one assigned a float does), and one handed to a function what that function's own body reads of its parameter. Where the value is not that, the program stops with what it holds and the read that asked — `a parsed value holds text 'many' where the program reads a number (met in add at line 39)` — the same words in both modes; text that reads as a number is one (`j.s + 1` is 8 for `"s": "7"`), and a number read as text is its text. `int[…]`, `float[…]`, `str[…]` and `moment[…]` still read one by hand where the program means the conversion. A parsed whole number is held as it is: a loop walks it and `encode` writes it with every digit, and `int[…]` reads it exactly; read as a float — met in arithmetic or compared in order — one past 2^53 stops the program rather than lose digits, as a frame's column of them does at its load (a loop and `encode` had read whole numbers through a float, and `9007199254740993` became `…992`). A loop walks a parsed list's items (`for x in j.items`), each a parsed value; the builtins that navigate one take it as it is — indexing, `.field`, `len`, `keys`, `values`, `has`, `encode` and `type`; and two parsed values compared (`j.a eq j.b`, in order too: `j.a gt j.b`) compare as the program runs, two numbers or two texts as such and another pair stopping (see Comparison), while two met in arithmetic are both read as numbers. (Until 2026-10-08 each such use was refused until the value was read by hand — `upper[] takes text, got a JSON value — a value whose type the program does not know is read as one first` — and a parsed value decided on, negated or joined with `and` had run interpreted and not built compiled.) Parsed JSON is dynamic data — the map homogeneity rule applies to maps you construct, not to values parsed from JSON (open-schema data is exactly what `json[]` is for). An element that is JSON's `null` is an error-value with `ok: false` and `reason: 'null'`, which auto-propagates through the error-value system (section 10).

```
items = json['[1, 2, 3]']
log[items[0]]              --- 1

n = json['null']
log[n.ok]                  --- false
log[n.reason]              --- 'null'

s = encode[items]
log[s]                     --- [1,2,3]
```

### System

| Function | Description |
|----------|-------------|
| `rand[]` | Random float in [0, 1) from the program's RNG; `seed[n]` (math) makes every draw repeatable, in both modes |
| `rand[n]` | Random int between 0 and n-1 (n must be positive) |
| `rand[min, max]` | Random int between min and max (inclusive) |
| `sleep[ms]` | Sleep for ms milliseconds (int) or seconds (float); a negative time stops the program (in both modes it had wrapped to a sleep without end, or panicked) |
| `env[name]` | Get environment variable (error-value if not set) |
| `env[name, default]` | Get environment variable, return default if not set — as text, as the variable is (`env['PORT', 3000]` is `'3000'` when `PORT` is unset; the interpreter had answered the int) |
| `exit[]` | Exit with code 0 |
| `exit[code]` | Exit with given code |
| `halt[reason]` | Halt the program with the reason, on the line of the call, as a builtin given what it cannot take does: a program's own precondition (When a Program Stops) |

**Design Note: `halt` beside `assert`.** Considered: `assert[false, reason]` for a precondition a program or a standard module written in Crow finds broken (rejected: it says `assertion failed: …`, an assertion's words, where a module's refusal must read as a builtin's does — `crossentropy: target 7 is not a class of 3 (0 to 2)`); `exit[1]` after `log[reason]` (rejected: an exit is not a stop — it lands no error, and a handler would not answer 500 with the reason). Decided (2026-10-04, with `math.nn` written in Crow): one unconditional halt, `halt[reason]`, the reason as written, on the line of the call; `assert` remains the check that says it checked. Named as `assert`'s own description has it ("halt with error"), and not `stop`: a trading program's stop-loss and a bus's stop are variables a program writes, and a builtin's name is reserved everywhere.

### Shell

| Function | Description |
|----------|-------------|
| `shell[cmd]` | Run a shell command; returns a record with `status`, `stdout`, `stderr` (error-value with kind `'shell'` only if the command cannot start) |

```
r = shell['echo hello']
log[r.stdout]    --- 'hello'
log[r.status]    --- 0
```

### Requests

| Function | Description |
|----------|-------------|
| `request[url]` | An HTTP GET; returns a record with `status`, `body` and `headers` (their names in lower case), whatever the status (error-value with kind `'http'` only when no answer came) |
| `request[url, method]` | A request without a body: `'DELETE'`, `'HEAD'`, `'OPTIONS'` or `'GET'` |
| `request[url, method, body]` | A request with a body, as text: `'POST'`, `'PUT'`, `'PATCH'` or `'DELETE'` |

Each takes these options by name, any of them:

| Option | What it sends |
|--------|---------------|
| `headers = map[…]` | Its headers: an `authorization`, a `content-type` naming the body's type |
| `query = map[…]` | Pairs added to the URL, each name and value URL-encoded, as a reader's are (#### Files, URLs and S3) |
| `form = map[…]` | A form, as a browser sends one: `application/x-www-form-urlencoded`, its fields URL-encoded |
| `files = map[field = path]` | Files with the form, as `multipart/form-data`: each file's bytes under its field, named by its file name, typed by its extension (`.csv` as `text/csv`, `.png` as `image/png`, any other as `application/octet-stream`), and the form's fields as text beside them |
| `save = path` | Where a success's bytes go: written to the file as they came — an image, an archive — when the status is a success (2xx), the record's `body` then `''`; any other answer's body stays in `body`, saying why, and the file is not written |
| `timeout = seconds` | How long it waits for its answer: unless told, thirty seconds to connect and ten minutes in all (`0` is as none) |

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

`request` is written in Crow (`src/modules/core.co`) on the `sys` layer — the second standard module moved (design rule 20) and the first that reaches the operating system: its row keeps the contract above, in both modes, and its own tests (`crow test src/modules/core.co`) read a URL apart, add a query, name a part's type, decode a chunk's size and spell a timeout; `tests/http_request.rs` runs it against a server in both modes. The Rust client it replaced (`crow_runtime::sources::request`, over `ureq`) is gone, and so is `ureq`: the readers' remote branch (`read[url]`, `load[url]`, S3) moved after it (Files, URLs and S3).

**Design Note: one `request`, its method named.** Considered: a word for each method — `get`, `post`, `put`, `delete` — as many HTTP clients spell them. Decided: one `request` with its method as text, as `route['/up', 'POST', handler]` names the server's (Vijay, 2026-10-01): `get` would be a second `read`, `put` is the name a stream's feed is handed (`fn numbered[put, n]`), and `delete` would stand beside the map's `del`; a method as text reaches PATCH, HEAD and OPTIONS with no more names.

### Hashing

| Function | Description |
|----------|-------------|
| `hash[s, algo]` | Hash string with algorithm. Supported: `'sha256'`, `'sha512'`, `'md5'`, and `'password'` |
| `hash[s, stored]` | Hash `s` the way the password hash `stored` was made — its algorithm, cost and salt — so `hash[attempt, stored] eq stored` checks a password |

```
h = hash['hello', 'sha256']
log[h]    --- '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824'
```

`hash[pw, 'password']` is for storing a password: a salted, deliberately slow hash (Argon2id), written as one string that carries its algorithm, cost and salt (`$argon2id$v=19$m=19456,t=2,p=1$…`). Its salt is random, so hashing the same password twice gives two different strings; checking a password therefore hashes the attempt with the stored string in place of the algorithm and compares the two. The fast hashes (`sha256`, `sha512`, `md5`) are for fingerprints and checksums, never for passwords: they can be guessed at billions a second.

### Encodings

| Function | Description |
|----------|-------------|
| `encode[text, scheme]` | Text written so something else can carry it: `'base64'` (the standard alphabet, padded with `=`), as an `authorization: Basic …` header or a binary field wants it; `'url'`, one component of a URL — every byte but a letter, a digit and `-` `.` `_` `~` written `%XX`, so a space is `%20` and `&`, `=`, `/` and `?` stand for themselves no more |
| `decode[text, scheme]` | The text `encode[…, scheme]` was given: base64 in the standard or the URL-safe alphabet, padded or not, whitespace between skipped; a URL component's `%XX` each the byte it stands for (a `+` stays a `+`). Error-value with kind `'parse'` when the text is not in the scheme or decodes to bytes that are not text |

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

### Regex

| Function | Description |
|----------|-------------|
| `matches[s, pattern]` | Check if string matches regex pattern (returns bool); an invalid pattern is an error value of kind `'regex'` |
| `find[s, pattern]` | The first match, as a record whose `groups` holds the whole match followed by each capture group. No match is an error value (`.ok` false, reason `no match`), as `find[arr, pred]` answers; an invalid pattern is one with kind `'regex'` |
| `sub[s, pattern, replacement]` | Replace all regex matches with replacement string; an invalid pattern is an error value of kind `'regex'` |

```
matches['hello', '[a-z]+']       --- true

m = find['2024-01-15', '([0-9]+)-([0-9]+)-([0-9]+)']
log[m.ok]        --- true
log[m.groups]    --- ['2024-01-15', '2024', '01', '15']
log[m.groups[1]] --- 2024

sub['hello world', 'world', 'crow']  --- 'hello crow'
```

### Moments

A moment is a kind of its own: an instant, and the offset it was written
in. It is the same kind in both modes and from every source — `now[]`,
`moment[…]`, a frame's moment column, a database's — so one vocabulary
reads it, moves it, renders it and computes with it, and no program meets
a time as a float here and as text there.

| Function | Description |
|----------|-------------|
| `now[]` | The current moment, in UTC — in a live program, the moment the current event began |
| `moment[text]`, `moment[text, tz = zone]` | A moment read from ISO 8601 text — a date, a date and time, with `Z`, an offset or neither, a fraction kept to the millisecond; a space may part the date and the time, and seconds may be left out. Text without an offset is UTC's, or the named zone's; an offset in the text wins. Text that is no time is an error value of kind `'time'` |
| `moment[year, month, day, hour = 0, minute = 0, second = 0, tz = 'UTC']` | A moment from its parts in a zone, the zone's offset at that instant kept; a date that does not exist, a time in a DST gap, or a zone it does not know is an error value of kind `'time'` |
| `moment[t, tz = zone]` | The same instant in another zone — an IANA name (`'America/New_York'`), an offset (`'+09:00'`) or `'UTC'`; a zone it does not know is an error value of kind `'time'` |
| `moment[seconds]`, `moment[t]` | The moment `seconds` after the epoch, in UTC (the reverse of `float[t]`); a moment is itself, so `moment[x]` is the moment of a number, of text or of a moment alike |
| `t.year`, `.month`, `.day`, `.hour`, `.minute`, `.second`, `.weekday`, `.offset` | Its fields, read in its own zone, as ints; `weekday` 1 is Monday and 7 Sunday; `offset` the minutes east of UTC it was written in |
| `str[t]` | Its ISO 8601 text — `2026-07-14T13:30:00Z`, `2026-07-14T09:30:00-04:00`, a bare date at midnight UTC, a fraction where there is one — which interpolation, `log`, a CSV and JSON write |
| `fmt[t, style = 'iso' \| 'date' \| 'time' \| 'datetime', tz = zone]` | The moment rendered in a style, in its own zone or in `tz`; a style or a zone it does not know is an error value of kind `'time'` (`fmt[t]` is `str[t]`) |
| `float[t]`, `int[t]` | Seconds since the epoch, as a number — whole for `int` |
| `bucket[t, seconds]` | The start of the bucket `t` falls in, `seconds` wide — `bucket[t, 300]` the five minutes, `bucket[t, 86400]` the day — the buckets aligned to midnight in the moment's own offset (a day's bucket starts at the local midnight; from UTC, at the epoch's); a span that is not a positive number stops the program. Over a frame's column, a column of moments (see frame) |
| `t + s`, `t - s` | The moment `s` seconds on or back, in its zone (`s` an int or a float) |
| `t2 - t1` | The seconds between two moments, a float |
| `t1 lt t2`, `eq`, `ne`, `gt`, `gte`, `lte` | Compared by instant: two moments of one instant are equal whatever zone each was written in |
| `min[a, b]`, `max[a, b]` | The earlier of two moments, the later |

```
open_bell = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']
log[open_bell]                                   --- 2026-07-14T09:30:00-04:00
log[fmt[open_bell, style = 'datetime']]          --- 2026-07-14 09:30:00
log[moment[open_bell, tz = 'UTC'].hour]          --- 13
log[open_bell - moment['2026-07-14T13:00:00Z']]  --- 1800

--- subtraction is the seconds between; comparison is comparison
if print_time lt open_bell - 10.0
    log['stale print — not driving price']
```

A moment multiplied, divided or taken a remainder of, added to a moment,
or compared with a number or text is refused before the program runs: a
moment adds or subtracts seconds, subtracts a moment, and compares with a
moment. A duration is a number of seconds, as `sleep`, `later` and every
timeout take one; there is no duration kind. A list of moments is a list.
A moment handed to `with` makes a moment column (`datetime`, as `.dtypes`
names it), `fill` fills one with a moment, a moment column's cell read one
at a time is a moment — from a file, a query or `with` alike — and a moment
column compares with a moment: `filter[df, df.at gt open_bell]`.

**One moment per event.** In a script `now[]` is the clock at the call. In a live program it is the moment the current event began — a click or a change from the page, a render, a `refresh` tick, a `later` schedule — everywhere inside it: a handler and every helper it calls, a render and every view and derivation in it. Time advances between events, never inside one, so `base, anchor = head, now[]` reads one clock twice and gets one answer, and two rows a handler stamps with `now[]` share a stamp. In the test harness the clock is the test's: it starts at the moment the first `app[…]` opened and moves only when the test moves it with `change[a, 'now', t]`.

Zones are IANA names (`'America/New_York'`, `'Europe/London'`) or offsets
(`'+09:00'`, `'UTC'`) and are **always explicit** — the default everywhere
is UTC. Invalid dates, unknown zones, DST gaps and text that is no time are
error values of kind `'time'`.

**Design Note: a moment is one kind.** Considered: the three moments Crow had grown — the core's, a float of epoch seconds with `calendar`, `parse_time` and `fmt[t, time = …]`; the frame module's, ISO text with `datetime`, `tz` and fields read from the text; and a database column's, a float again — so that `df.at[0]` was text from a file and a float from a query, and the functions of one refused the other. Decided (Vijay, 2026-10-06): one kind, carrying its instant and the offset it was written in, so `t.year` is the year it was written in and `moment[t, tz = …]` moves it; one name for each thing a program does with a moment — `now`, `moment`, the fields, `str`, `fmt` and the operators — and the rest parameters. So `calendar`, `parse_time`, `datetime`, `tz` and the duplicate `time[]` are gone, outright, and `fmt`'s `time =` option is `style =`: the one renderer's option for one more kind, not a renderer per kind. Rejected: a float everywhere, which loses the zone a text was written in; text everywhere, on which nothing computes; a duration kind, where seconds already serve. `float[t]` and `moment[seconds]` are the bridges for a program that keeps a time as a number, as finch's simulated clock does.

### Freshness: captured

Values read from the outside world remember when they arrived:

| Function | Description |
|----------|-------------|
| `captured[x]` | The moment `x` was read from outside (a moment, so ordinary arithmetic applies). A value computed here has no capture moment: the answer is an error value with `kind = 'no_capture'` |

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

### Design Note: Freshness Lives at the Boundary

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

### Design Note: Adopting External Ground Truth

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

### Design Note: A Moment Is a Number

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
(subsumed by `moment`), `today` (`fmt[now[], style = 'date']`) — all three
now raise teaching errors naming their replacement.


## 12. Modules and Imports

### Importing

Use `use` to import a module. The entire module is imported — no selective imports. Multiple modules can be imported on a single line with commas.

```
use math
use views.home
use web, ui
```

Dot-separated paths map to directories: `use views.home` loads `views/home.co`, and its exports are reached qualified through the last segment — `home.render`, `home.message`. Slash paths (`use lib/utils` → `utils.helper[...]`) work the same way. This is how you import view files, library modules, etc.

Comma-separated imports expand at parse time: `use web, ui` is equivalent to writing `use web` and `use ui` on separate lines.

**Each file's imports are its own.** A file sees the modules it imports and no other: a part's `use frame` serves the part, not the file that uses the part, so an entry that calls `frame.new[…]` or `join[…]` with no `use frame` of its own is refused for the import (``frame.new[] is frame's: add `use frame` ``), whatever its parts import — as every file is when it is checked alone. (The entry had reached a module through a part's import, and lost it when the part's import went; found in October 2026, writing the error corpus.) The program still links every module its parts import; only each file's names resolve against its own imports. What the part declares is reached as before, qualified: `helpers.table_of[…]`. A part that does not parse is refused by its own file and line (`views/sim.co: line 60, col 5: …`), as a part that fails its checks is; it had been dropped, and the entry told its name was undefined.

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

result = sqrt[16]    --- 4
x = zeros[3, 3]     --- 3x3 tensor of zeros
```


### Conflict Resolution

When two imported modules export the same function name, bare calls are ambiguous. Crow throws an error and requires the module prefix to disambiguate.

```
use math
use frame

--- ERROR: 'stack' is ambiguous — provided by math and frame
t = stack[[a, b], 0]

--- Correct: use module prefix
t = math.stack[[a, b], 0]
both = frame.stack[top, bottom]

--- No conflict: only frame exports group
g = group[sales, 'region', ['amount', 'sum']]
```

Rules:
- If only one module exports a name, bare calls work.
- If multiple modules export the same name, bare calls error with a list of qualified alternatives.
- Inside a `view`, an element name is the element: `stack[...]` is the layout container even with `frame` and `math` (which both export `stack`) imported. The prefix (`frame.stack[a, b]`) reaches the module function from inside a view.
- A `frame` function given something other than a frame is the core builtin of the same name: with `use frame`, `sort[xs]` sorts the array and `join[parts, sep]` joins strings, while `sort[df, 'col']` and `join[a, b, 'key']` are the frame operations. The first argument decides.
- The element-wise math functions (`exp ln log2 log10 sign sin cos tan asin acos atan sinh cosh tanh`) take a plain number as well as a tensor and return a number for a number.
- Module-prefixed calls (`module.fn[args]`) always work regardless of conflicts.


### User Modules: File Structure

A source file ends in `.co`, and its bytecode in `.coc`; no other file is a Crow source. Each file is a module. The file path is the module path, rooted at the project: the nearest directory above the entry file that holds a `crowfile`, or the entry file's own directory when there is none. So `views/sim.co` reaches `sim/clock.co` with `use sim.clock` whichever file is the entry. A part is validated alone as it is expanded — its own names unprefixed, its imports inlined — so a name the part reserves by what it imports (`derive progress` beside `use ui`), or a handler nothing in the part wires, is refused whichever file is the entry, under the part's path: `crow test part.co` and `crow run main.co` see the same part. Any error found in a part — alone or in the whole program — names the part and its own line (`sim/session.co: line 60: …`), never a line of the entry. In the whole program a part keeps its own names: its facts and derivations carry its path, so a local or parameter of one of its functions, views, handlers or derivations is that body's own, whatever another file holds or imports under the same name — a local `ch` beside the entry's connection `ch`, a parameter `secs` beside the entry's `derive secs`, a list `fills` it builds beside the entry's `derive fills` (a derivation that calls the function reads none of them), a list `rsi` where another part imports `chart` bare, a derivation's local `c` or a handler's local `r` or parameter `s` beside the entry's top-level `c`, `r` and `s`. (The expansion gives such a local the part's path as a suffix — `c__sim_session` — which an error inside the part may show.)

#### Design Note: The Extension Is `.co`

**What was considered:** `.crow` (the name, four letters), `.cw`, `.w`, `.cr`, `.o`, `.ro`, `.caw`, `.co`.

**Why `.co`:** two letters that say themselves, the head of the name, and no language owns it — its only other users are a few 3D and colour-profile data formats that never meet an editor's language mode or GitHub's detection. `.cr` is Crystal's, `.w` is OpenEdge's and CWEB's, `.o` is every compiled object file on Unix, `.cw` cannot be said aloud, `.ro` reads as read-only. `.crow` was fine and unambiguous; it lost to brevity in a file tree of short part names (`scan.co`, `nav.co`). The bytecode file follows: `.coc`. The Markdown fence language stays `crow`, since that names the language, not the file. Decided September 2026, while the tree was one repository and one app. `.crow` and `.crowc` were read for a release after the rename, with a note, so a tree could be renamed at leisure; that went in October 2026 (one extension, not two — design rule 1), with the editor association.

```
--- file: myapp/utils.co

fn double[x]
    return x * 2
```

```
--- file: main.co

use myapp/utils

result = utils.double[5]    --- 10
```

**A module is a part.** It may hold facts, derivations, handlers, views and `test` blocks, not only functions — the clock of a simulation, its tape, its blotter — and the page that composes them imports each one. The front end expands every module file once, whichever files import it: its names take the module's path as a prefix (`sim/clock.co`'s `head` is `sim_clock_head` inside the program) and every importer's qualified spelling — `clock.head`, or an alias's — is rewritten to that name, in code and inside interpolation spans. A module reached through two importers is one copy with one state. A handler in one part may assign another part's fact by its qualified name (`session.day = d`), and it is checked like any fact write. A part's `test` blocks run with `crow test sim/clock.co`, with the parts it imports inlined; `crow check main.co` checks the whole app. A module's top-level statements other than declarations are not part of the program.


## 13. Standard Library

The modules that ship with Crow. Their exports are bare names in files that import them (`use math`), qualified-only under an alias (`use math as m`), and always reachable with the prefix (`math.sum[...]`) — see section 12.

**Design Note: what stays native, and why.** Considered: the standard library wholly in Crow, Rust kept for the compiler alone. Decided (design rule 20; Vijay, 2026-10-04): native code stays where one of four reasons holds, and every crate the manifests name (`Cargo.toml`, `crow_runtime/`, `crow_web/`, `crow_db/`) is accounted for here — a crate added gets its line, with its reason.

- *A mechanism of the runtime, which no formula can be:* the server and its pages (axum, tokio, tower, tokio-stream, futures-util) and what runs inside its request handling — multipart uploads (multer), CORS and static files (tower-http), the signed sign-in cookie and session ids (hmac, sha2, base64, uuid), a live page's `fetch[]` sources polled by the server (reqwest); the database handles — their pools, a handler's transaction, the race of a query against a stop (`crow_db::driven`); the drivers themselves are written in Crow (docs/db-in-crow-design.md); the seeded process-global random source (rand); the autograd tape; parallel loops (rayon).
- *A validated kernel:* TLS (rustls, webpki-roots); the digests, the keyed digests and the password hash (sha2, sha1, md-5, hmac, argon2), randomness from the operating system (getrandom); regular expressions (regex); the byte search the CSV reader splits lines and fields with, vectorised (memchr); the formats a table travels in (arrow, parquet, serde_json, bytes); linear algebra, the FFT, sparse matrices and the GPU (faer, rustfft, sprs, wgpu, pollster, bytemuck); time zones (chrono, chrono-tz); markdown (pulldown-cmark); a number written and read exactly (itoa, ryu), `erf` to the last bit (libm); whole numbers of any size, decimals to any digits with their roots, powers, logarithms and trigonometry correctly rounded, and fractions (dashu-int, dashu-float, dashu-ratio); the distributions drawn by rejection or a ziggurat — Poisson, Bernoulli, gamma, beta and exponential (rand_distr).
- *A C library:* SQLite (rusqlite); the memory allocator of the interpreter and of every compiled program (mimalloc): the system allocator on macOS reads the clock in its `free` and frees a value another thread made slowly, and a program allocates for nearly every text and list it makes — revcomp and k-nucleotide run on every core spent half their time there, interpreted, and compiled binarytrees nine times its CPU.
- *The compiler's own tooling:* the Jupyter kernel (zeromq, libc), the REPL (rustyline), the language server (lsp-server, lsp-types), `crow check`'s SQL facts (sqlparser), snapshots of a program (bincode, serde), the served harness's client (tungstenite, in the compiler alone).

Written in Crow on the `sys` layer, their crates gone: the HTTP client and every reader of a URL or an S3 object (ureq), tokens (jsonwebtoken), msgpack (rmp-serde), the WebSocket client (tungstenite, from the web runtime) and mail (lettre); and, with no crate to drop, the statistics (`math.stats`) and the normal draws (`randn` and `normal`: Box–Muller over uniform draws). The database drivers too (klickhouse; tokio-postgres, deadpool-postgres and `crow_db`'s tokio), measured first (2026-10-05, on one machine, three columns of a million rows from `numbers` and `generate_series`): the protocol written in Crow with the rows decoded straight into a frame by a kernel matches the native drivers or beats them, in both modes: ClickHouse over its HTTP interface — `exchange` in Crow, `frame_of_rowbinary` for the rows — read a million rows in 52 ms against klickhouse's 85 ms (two hundred thousand in 16 ms against 22 ms; five thousand in 2 ms against 1.4 ms, a connection opened per request), every column typed as the server names it; the Postgres wire protocol in Crow — the startup, a simple query, the answer read in large pieces and decoded by `frame_of_postgres` — read a million rows in 0.25 s against tokio-postgres's 0.24 s, the server's own time. The rows parsed by a parser written in Crow instead took 1.3 s compiled and 3.7 s interpreted (CSV through the `frame_of_csv` kernel, 77 ms): per-cell work in Crow pays the language's price per operation — a copy, an allocation, a Result — where a kernel pays per byte, which is design rule 20's line. The drivers are called from Rust — the schema modes at `connect`, a handler's transaction, a derivation's race of a query against a stop, fixture replay, the rehearsal, `crow db diff` — through a table of calls (`crow_db::driven::Calls`) a compiled program registers at its start and the interpreter answers from a child of its own; the compiler's `describe` asks a `crow db describe` process (docs/db-in-crow-design.md). What runs inside the Rust server's request handling moves only once the server can call Crow, or the design changes — a live page's sources polled by the program itself, with `request`.


### math

The `math` module is built in — no file needed. It provides tensor creation, element-wise operations, reductions, shape operations, and linear algebra. After `use math`, all functions are called directly (e.g. `zeros[3]`, not `math.zeros[3]`).

#### Tensor Creation

| Function | Description |
|---|---|
| `zeros[d1, d2, ...]` | Tensor of zeros with given shape |
| `ones[d1, d2, ...]` | Tensor of ones with given shape |
| `full[d1, d2, ..., value]` | Tensor filled with `value` (last arg) |
| `eye[n]` | n×n identity matrix |
| `onehot[targets, classes]` | A row per target, 1 in its class's column and 0 elsewhere: the targets a vector of whole numbers from 0 to classes − 1 (a target that is no class stops the program: `onehot: 7 is not a class of 3 (0 to 2)`) |
| `diag[tensor]` | Extract diagonal from a 2D tensor |
| `linspace[start, end, n]` | `n` evenly spaced values from `start` to `end` |
| `arange[start, end, step]` | Values from `start` to `end` by `step` |
| `randn[d1, d2, ...]` | Normal random values (mean=0, std=1); written in Crow (see Random Numbers) |
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
| `sum[t]` | Sum of all elements (float), added as numpy adds them — pairwise, so the error grows as log n and `sum[x]` is `numpy.sum(x)` to the last digit |
| `mean[t]` | Mean of all elements (float): the sum over the count |
| `min[t]` | Minimum element (float) |
| `max[t]` | Maximum element (float) |
| `prod[t]` | Product of all elements (float) |
| `argmax[t]` | Index of maximum element (int) |
| `argmin[t]` | Index of minimum element (int) |

All reductions except `argmax`/`argmin` accept an optional axis parameter: `sum[t, axis]` reduces along that axis and returns a tensor instead of a scalar.

#### Sorting & Selection

| Function | Returns |
|---|---|
| `argsort[t]` | The indices that would sort the numbers, ascending: a list of ints, as `range` answers one, so `xs[order[0]]` reads the smallest (it had answered a tensor, whose items read as floats and indexed nothing) |
| `topk[t, k]` | Top k largest values, sorted descending (NumArray) |
| `unique[t]` | Unique values, sorted (NumArray) |
| `where[cond, a, b]` | Element-wise: a where cond≠0, else b (NumArray); `cond` a mask too — `where[v gt 15.0, v, 0.0 * v]` — as a mask is numbers where numbers are asked (true 1, false 0; `sum[v gt 15.0]` counts) |

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
a = reshape[linspace[1, 3, 3], 3, 1]   --- shape [3, 1]
b = reshape[linspace[10, 40, 4], 1, 4]  --- shape [1, 4]
c = a + b                                          --- shape [3, 4]
```

Broadcasting works with all element-wise operators: `+`, `-`, `.*`, `./`, `.^`.

Incompatible shapes (e.g. `[3] + [4]`) are refused before the program runs where both shapes are known (see Tensor Arithmetic), and stop the program where it runs where they are not, with the same words in both modes.

#### Advanced Indexing

Tensors can be indexed with another tensor to gather elements by index:

```
a = linspace[10, 50, 5]
indices = linspace[0, 2, 3]
y = a[indices]                    --- gathers elements at indices 0, 1, 2
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
| `rank[a]` | Matrix rank (count of singular values above tolerance) |
| `cond[a]` | Condition number (largest singular value / smallest) |

#### Random & Distributions

| Function | Description |
|---|---|
| `seed[n]` | Set the program's RNG seed: every later draw, tensors and the core `rand[]` alike, is repeatable and identical in both modes. Statements run in the order written, and a loop that draws runs in order (§14), so a seeded program draws the same numbers on every run |
| `randn[d1, d2, ...]` | Standard normal (mean=0, std=1): Box–Muller over two uniform draws a value, written in Crow (`src/modules/math.co`, design rule 20); a negative size stops the program |
| `normal[mean, std, d1, d2, ...]` | Normal with custom mean/std: `mean + std · randn`, written in Crow; a negative std stops the program |
| `uniform[low, high, d1, d2, ...]` | Uniform in [low, high): `uniform[0.0, 1.0, 2, 3]` is a 2×3 tensor of values in [0, 1) |
| `poisson[lambda, d1, d2, ...]` | Poisson distribution |
| `bernoulli[p, d1, d2, ...]` | Bernoulli (0.0 or 1.0) |
| `gamma_dist[alpha, beta, d1, ...]` | Gamma distribution |
| `beta_dist[alpha, beta, d1, ...]` | Beta distribution |
| `exponential[lambda, d1, ...]` | Exponential distribution |
| `shuffle[x]` | Random permutation of tensor elements |
| `choice[x, n]` | Random sample of n elements (no replacement) |
| `permutation[n]` | Random permutation of 0..n |

`rand` is core's: `rand[]` is one float in [0, 1), and `rand[n]` and `rand[min, max]` are ints (see Core Builtins). A tensor of values in [0, 1) is `uniform[0.0, 1.0, d1, …]`.

**Design Note: no tensor `rand`.** Considered: `rand[d1, d2, …]` for a tensor of uniform values in [0, 1), as NumPy spells it. The math module had it until March 2026, when its functions became callable without `math.` and core's `rand` kept the name; this table went on listing it, and its examples made ints (`rand[2, 3]` is 2 or 3). Rejected: `rand[5]` and `rand[2, 3]` are core's, an int below 5 and an int from 2 to 3, so the same call would mean two things, and `use math` would change what a program computes. Decided: `uniform[0.0, 1.0, …]` makes the tensor, the one way to.

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
| `erf[x]` | Error function erf(x), of a number or element-wise over a tensor (it had taken a tensor only) |
| `fma[a, b, c]` | a × b + c rounded once: the fused multiply-add a compiled loop accumulates with (pandas's rolling kernels, scikit-learn's row norms, scipy's sparse products), so a sum held to one of them runs with it — `total = fma[x, x, total]` |
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
| `nnz[s]` | Number of stored non-zero entries |
| `spmv[s, v]` | Sparse matrix × dense vector (the same product as `s * v`) |
| `s * v` | Sparse × dense matmul (vector or matrix) |
| `s + s2` | Sparse + sparse addition |
| `solve[s, b]` | Solve sparse linear system s·x = b |

```
use math

--- dense to sparse
s = sparse[eye[3]]
log[s]                              --- sparse[3x3, nnz=3]

--- COO creation
s2 = sparse[[0, 1, 2], [0, 1, 2], [5, 6, 7], [3, 3]]

--- sparse matmul
r = s * [1, 2, 3]

--- sparse add
s3 = s + s2

--- back to dense
d = dense[s3]

--- sparse solve
x = solve[s2, [10, 12, 14]]
```

#### Data Loading

| Function | Description |
|---|---|
| `load[path]` | Load a table of numbers directly into a 2D tensor: a CSV file, or JSON records or Parquet, from a file, a URL or an S3 object (error-value on failure) |

`load[path]` reads a CSV file with a header row and parses all values as floats into a 2D tensor (rows × columns). This is much faster than `csv[path]` (io module) for numeric data because it avoids creating intermediate string maps.

```
use math

data = load['data.csv']
log[data.shape]        --- [1000, 10]
log[sum[data]]         --- total of all values
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

A statistic of no values is `NaN`. The series a function pairs up (`corr`, `cov`, `linreg`, `chisq`) are one length, `percentile`'s `p` is from 0 to 100, `anova` compares two groups or more, and `kstest` and `fit` know the distributions above; anything else stops the program with the reason (see When a Program Stops). They had answered `NaN`, `0.5` or an empty map.

The module is written in Crow (`src/modules/stats.co`, design rule 20): each statistic is a formula over the math builtins — a median the middle of `sort`, a variance the mean of squared distances, a regression its sums of products — and the tests' p-values are the t, F and chi-square distributions written over the `sys` layer's `betainc` and `gammainc`, which stay validated kernels. Its rows keep the contract above in both modes; its tests are the module's own (`crow test src/modules/stats.co`): each statistic at known values, and its laws over a spread of inputs (a median is the 50th percentile, a series' covariance with itself is its variance, a correlation is symmetric and within −1 and 1, a z-score keeps nothing of the scale, a line through its own points has r² 1).

```
use math
use math.stats

x = linspace[1, 10, 10]
log[median[x]]            --- 5.5
log[std[x]]               --- 2.872...
log[var[x]]               --- 8.25
log[percentile[x, 25]]    --- 3.25

z = zscore[x]             --- mean=0, std=1

y = linspace[2, 20, 10]
log[corr[x, y]]           --- 1.0 (perfect correlation)
log[cov[x, y]]            --- 16.5

result = ttest[x, y]
log[result.t]                   --- t-statistic
log[result.p]                   --- p-value
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
| `crossentropy[logits, targets]` | Cross-entropy loss with log-sum-exp stability (Float): logits `[classes]` with one target, or `[batch, classes]` with a target per row, each a class from 0 up to classes − 1 |
| `bce[pred, target]` | Binary cross-entropy with clamping (Float) |

**Layers:**

| Function | Description |
|---|---|
| `linear[x, w, b]` | Linear layer: x @ w + b (NumArray) |

`math.nn` is written in Crow (`src/modules/nn.co`) over the math builtins, the first of the standard modules that are (design rule 20): each function is the formula its row above states — `relu` is `clip[x, 0.0, inf]`, `sigmoid` is `1.0 / (1.0 + exp[-x])`, `crossentropy` the mean over the rows of `log(sum(exp(row))) − row[target]` with the row's largest score taken out of the exponentials first — and the builtin table keeps each one's contract: what it takes (a list of numbers is read as a tensor, as any row's tensor is), what it answers, how it fails. The module is inlined into every program that imports it and runs in both modes as the program's own functions do; under `grad` its formulas are differentiated as any function is, so no rule of their own exists to disagree with them (Automatic Differentiation). Its tests are its own: `crow test src/modules/nn.co` runs the formulas at known values, their laws over a spread of inputs, and a gradient through every one against central differences.

Shapes `mse` cannot broadcast or `linear` cannot multiply (`x` and `w` matrices, `x`'s columns as many as `w`'s rows), two lengths given to `bce`, and a `crossentropy` target that is no class (or not a whole number, or one too few or too many for the rows of logits) stop the program with the reason, on the line of the call in the program (see When a Program Stops): `linear: x has 3 columns and w 2 rows — x * w needs them equal`, `crossentropy: target 7 is not a class of 3 (0 to 2)`.

**Design Note: a builtin written in Crow keeps its row.** Considered: inlining a Crow-written standard module as a user module is inlined, its functions reached by bare name (rejected: the call would lose the row's contract — a list of numbers converted to a tensor where the parameter takes one, a misuse refused before the program runs with the builtin's own words, `relu[] takes numbers, got an int` — and a mismatch inside the formula would be reported in the words of the formula's innards, `clip[] takes numbers`, naming a builtin the program never called). Decided (2026-10-04): the table states every builtin's contract whether its body is Rust or Crow; the lowering calls the Crow body where it would call the Rust function, with the arguments read as the row's parameters take them; the body's own stops name the line of the call in the program, never a line of the module's source. Private helpers of the module are renamed behind its prefix; a function only the module's tests use is not inlined. A module's function is read in the module's own scope: the program's functions, imports, aliases and elements are invisible in it — the protocol's local `lines`, indexed `lines[0]`, stays its local in a program with a `lines` of its own, and a program names its functions as it likes (parity 162) — and a module's rows are inlined only where the program reaches them, by a bare name, the module's, or as a value: a program that uses math without `load` carries none of the protocol. A row that takes a value JSON can hold (`sign`'s claims, `msgpack`'s value) hands its function the value's JSON text — the one form every such value takes in Crow, read back with `json[…]` — and the IR verifier holds every such function to its row: what it answers, and whether it can fail.

```
use math
use math.nn

x = linspace[-2, 2, 5]
log[relu[x]]              --- [0, 0, 0, 1, 2]
log[sigmoid[x]]           --- [0.119..., 0.269..., 0.5, 0.731..., 0.881...]
log[softmax[x]]           --- probabilities summing to 1

pred = linspace[0.1, 0.9, 3]
target = linspace[0, 1, 3]
log[mse[pred, target]]    --- 0.00667

input = reshape[linspace[1, 4, 4], 2, 2]
w = eye[2]
b = ones[2]
log[linear[input, w, b]]  --- input + bias
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
| `.len` | Its length, as `len[t]` gives it: a vector's numbers, a matrix's rows |
| `.dtype` | Data type (`'float'`) |
| `.device` | Where the tensor lives (`'cpu'` or `'gpu'`) |
| `.grad` | Gradient status (`'none'` if no gradient) |

```
use math

a = zeros[3, 3]
b = eye[3]
c = a * b
d = det[b]           --- 1.0

--- constants
log[pi]               --- 3.141592653589793

--- properties
log[a.shape]               --- [3, 3]
log[a.dtype]               --- float
log[a.device]              --- cpu
```


### io

Import with `use io`. Provides structured file parsing beyond the built-in `read[path]` and `write[path, content]`.

| Function | Description |
|----------|-------------|
| `csv[path]` | Parse CSV — a file, a URL or an S3 object — into array of maps (error-value on failure) |
| `lines[path]` | Read a file, a URL or an S3 object as array of lines (error-value on failure) |

**Note:** Basic file I/O (`read[path]`, `write[path, content]`) are builtins available without any import. The io module adds structured parsing only.

```
use io

write['/tmp/data.csv', 'name,age\nalice,30\nbob,25\n']

rows = csv['/tmp/data.csv']
log[len[rows]]         --- 2
log[rows[0].name]      --- alice
log[rows[0].age]       --- 30

ls = lines['/tmp/data.txt']
log[len[ls]]           --- number of lines
log[ls[0]]             --- first line

--- error handling
bad = csv['/tmp/missing.csv']
log[bad.ok]            --- false
log[bad.reason]        --- error message
```


### doc

Import with `use doc`. Documentation from Markdown, rendered by the kernel so the interpreter and compiled programs produce the same pages. The Crow Book is built with it (`docs/book/site.co`).

| Function | Description |
|----------|-------------|
| `markdown[text]` | Markdown to HTML: headings get anchor ids, links to `.md` files point at `.html` pages, fenced `crow` blocks are highlighted with Crow's own token rules and tagged `data-mode` (`run`, `error`) |
| `chapters[summary]` | An mdBook-style summary (`# Part` headings, `- [Title](path)` items nested by indentation) as a frame with columns `title path depth part` |
| `menu[chapters, current]` | Navigation markup for the chapters frame: a brand link, parts as headings, collapsible groups for chapters with sub-chapters (open around the current page), the current page marked, links relative to the current page |
| `pager[chapters, current]` | Previous and next chapter links for the current page, in summary order |
| `plain[text]` | The words of a chapter with markup removed and whitespace collapsed — what a search index holds |
| `document[title, body]` | A complete HTML page: the kernel stylesheet (tokens, elements, responsive rules) plus the documentation rules, then the body |

`markdown`, `menu` and `pager` make markup, which a view places as markup, so a rendered chapter or a menu can be a child of `stack[...]` or `row[...]`; every other string is text and is escaped (§ Views: *Text is text*). A view's tree handed to `document[...]` is its markup in both modes.

```
use web, ui, doc

view chapter[title, body, nav]
    return row[stack[nav, width = '240px', hidden_below = 'tablet'],
               stack[body, fill = 'true', class = 'doc'], gap = 'xl']

toc = doc.chapters[read['docs/book/src/SUMMARY.md']]
for i in range[len[toc]]
    path = toc.path[i]
    body = doc.markdown[read['docs/book/src/`path`']]
    page = doc.document[toc.title[i], chapter[toc.title[i], body, doc.menu[toc, path]]]
    html_path = replace[path, '.md', '.html']
    write['docs/book/crow-site/`html_path`', page]
```


### chart

Import with `use chart`. Technical indicators for the chart element, computed by the runtime (`crow_runtime::indicators`, over the kernels in `crow_runtime::ta`) so the interpreter and compiled programs draw the same numbers. Every function takes series — frame columns or arrays of numbers — and returns a series of the same length, `NaN` where the value is not yet defined (a 14-period RSI has no value on row 3), so a chart draws a gap instead of a guess. Put a result back into the frame with `with[frame, name, series]` and name the column in `lines = [...]` or `panes = [...]`.

| Function | Description |
|----------|-------------|
| `sma[values, n]` | Simple moving average over the last `n` rows |
| `ema[values, n]` | Exponential moving average, smoothing `2/(n+1)`, seeded by the first `n` rows' mean |
| `rsi[values, n]` | Wilder's relative strength index over `n` deltas (0–100; 50 when flat) |
| `rsi[ts, values, n, secs]` | RSI on clock-aligned buckets of `secs` seconds over finer rows — a 14-period RSI on 3-minute candles drawn over 1-second bars: completed buckets feed the smoothing once, and rows inside the forming bucket see a provisional value from the latest close, so history never changes and the newest value moves with the tape |
| `macd[values, fast, slow, signal]` | A frame with columns `line` (fast EMA − slow EMA), `signal` (its EMA), `hist` (their difference) |
| `vwap[price, vol]` | Cumulative volume-weighted average price from the first row |
| `vwap[ts, price, vol, secs]` | VWAP over the trailing `secs` seconds (rows with `ts >= t − secs`) |
| `rolling_sum[ts, values, secs]` | Sum over the trailing `secs` seconds |
| `cvd[close, vol]` | Cumulative volume delta: volume counted positive when the close rose, negative when it fell |

`ts` is a column of moments (seconds). A window is 1 row or more, a trailing window 0 seconds or more and a bucket more than 0, and the series an indicator reads row by row are one length; anything else stops the program with the reason (see When a Program Stops), as does a wrong number of arguments, naming the counts the function takes (`rsi[] takes 2 or 4 arguments, got 3`). A negative window had been a huge one compiled and one row interpreted, and a shorter series cut the others.

```
use chart, frame

bars = frame.new[]
bars = with[bars, 'ts', [0.0, 60.0, 120.0, 180.0, 240.0, 300.0]]
bars = with[bars, 'c', [10.0, 11.0, 10.5, 11.5, 12.0, 11.0]]
bars = with[bars, 'vol', [100.0, 200.0, 150.0, 300.0, 250.0, 100.0]]
bars = with[bars, 'sma3', sma[bars.c, 3]]
bars = with[bars, 'rsi3', rsi[bars.c, 3]]
bars = with[bars, 'vwap', vwap[bars.c, bars.vol]]
log[bars.sma3[5]]      --- 11.5
log[bars.rsi3[5]]      --- 50
log[bars.vwap[1]]      --- 10.666666666666666
```

In a view, the columns become panes with reference levels:

```
chart[bars, candles = ['o', 'hi', 'lo', 'c'], lines = ['vwap'], bars_y = 'vol',
      panes = [['rsi3'], ['cvd']], levels = map[rsi3 = [30, 70], cvd = [0]],
      points = fills, seek = jump, marker = clock, tz = 'America/New_York']
```


### sys

Import with `use sys` — **only in a module of the standard library.** `sys` is the layer the standard library's own Crow-written modules stand on: bytes, and a connection to a host read and written as bytes. A program may not import it; a program serves with `serve[…]`, reads with `request[…]`, `read[…]` and `socket[…]`, and connects with `connect[…]`, which are built on this layer (design rule 20). In a program, `use sys` is refused before it runs: `` `sys` is the standard library's own layer — a program serves with serve[…], reads with request[…], read[…] and socket[…], and connects with connect[…] ``. The book's page shows the layer through the harness's door (`CROW_SYS=1`), which the tests and the book have and a program has not.

| Function | Description |
|----------|-------------|
| `bytes[text]` | The text's UTF-8 as bytes. A bytes value shows as `<n bytes>`, has a `len`, and compares with `eq` and `ne` to another; `type[b]` is `bytes` |
| `text[bytes]` | The text the bytes spell in UTF-8; an error value of kind `'parse'` where they spell none |
| `slice[bytes, from, to]` | The bytes from `from` to `to` (excluded), counted from the end when negative; a span outside stops |
| `find[bytes, needle]` | Where the needle first occurs, or -1 |
| `concat[a, b]` | The two in order |
| `byte[bytes, i]` | The byte at `i` as an int, counted from the end when negative; outside stops |
| `open[host, port, tls, seconds]` | A connection to the host's port — each address the host's name resolves to tried in turn (`localhost` is `::1` and `127.0.0.1`, and a server may listen on one) — behind TLS when `tls` is true (the system's roots); each read and write gives up after `seconds` (none when 0). An error value of kind `'sys'` when none can be made: a host that does not resolve, a port nobody listens on, a certificate that does not fit |
| `send[stream, bytes]` | The bytes written, all of them; the count. An error value of kind `'sys'` on a stream that is closed or broke |
| `recv[stream, most, seconds]` | Up to `most` bytes as they arrive — none when the other side closed — waiting at most `seconds` (the connection's own wait when 0). The wait ends with a stop — of the work it is in (a derivation nobody waits for any more) or of the program — with the reason `cancelled: nobody waits for this query any more`, which a driver answers by telling its server. An error value of kind `'sys'`: `no data from host:port in 2 s`, `the stream is closed` |
| `close[stream]` | The connection let go; every call after is refused |
| `concat[chunks]` | A list of bytes joined in order, as one (what a connection read, piece by piece), as `concat[[a, b, c]]` joins pieces written down |
| `file[path]` | A file's bytes, as they are; an error value of kind `'sys'` where it cannot be read |
| `store[path, bytes]` | The bytes written to a file, as they are; an error value of kind `'sys'` where it cannot be written |
| `random[n]` | `n` bytes from the operating system's randomness |
| `hex[bytes]` | The bytes as lowercase hex, two digits each |
| `text[bytes, fill]` | The text the bytes spell, each run that spells none replaced by `fill` (`'�'`, as a browser shows one) — never an error |
| `hash[bytes, algo]` | The digest as lowercase hex: `'sha256'`, `'sha512'`, `'sha1'` or `'md5'` |
| `digest[bytes, algo]` | The digest as bytes: `'sha1'` (a socket's accept key), `'sha256'`, `'sha512'` or `'md5'` |
| `hmac[key, message, algo]` | The keyed digest as bytes: `'sha256'` (an S3 signature's chain) or `'sha512'` |
| `reply[status, body, headers]` | A request's answer from its parts: the record `request` answers, made by the client written in Crow |
| `answer[status, phrase, headers, body]` | A server's answer from its parts, its body as bytes: the record `exchange` and `fetch` answer — `status`, `phrase`, `headers` (names in lower case) and `body` |
| `csv_rows[text]` | CSV text as rows, each a map from its column's name to its text; an error value of kind `'io'` where it does not parse |
| `frame_of_csv[text]`, `frame_of_csv_bytes[bytes]`, `frame_of_json[text]`, `frame_of_parquet[bytes]`, `frame_of_rowbinary[bytes]`, `frame_of_postgres[bytes]` | A table as a frame, from CSV text (or a CSV file's bytes read where they are, which `load` uses: no copy of the text), JSON records, Parquet bytes, ClickHouse's `RowBinaryWithNamesAndTypes`, or the messages Postgres answers a query with (its row description, rows, completion and ready-for-query). Each column is typed as the server names it: integers an int column (a ClickHouse `UInt64` past an int cell's 64 bits refuses the whole, naming its column — `CAST` it to `String` to keep its digits; it had wrapped round to a negative number), floats, decimals and numerics floats (a decimal the float nearest its value, every digit of it read — a 256-bit decimal's high bits had been dropped), booleans bools, dates and timestamps a datetime column (a fraction of a second kept to the millisecond, a zone's offset kept), a NULL a missing cell, ClickHouse's enums by name and its UUIDs, IPs, 128- and 256-bit integers (their exact digits and sign) and arrays as text, anything else text; a ClickHouse type no cell holds, or a Postgres cell sent in the binary format, refuses the whole; a Postgres error is the reason in the server's words, and bytes that end before the ready-for-query "end early" (the driver reads more); an error value of kind `'frame'` |
| `frame_csv[frame]`, `frame_json[frame]`, `frame_parquet[frame]` | A frame as CSV text, as JSON records, or as Parquet bytes |
| `tensor_of_frame[frame]` | A frame's cells as a matrix, a row each; an error value of kind `'load'` where a cell is no number |
| `base64[bytes]` | The bytes as base64, the standard alphabet padded with `=` (HTTP's Basic authorization, SMTP's AUTH PLAIN) |
| `base64url[bytes]` | The bytes as base64 in the URL-safe alphabet, unpadded, as a token's parts are written |
| `unbase64[text]` | Base64's bytes, either alphabet, padded or not; an error value of kind `'parse'` where the text is no base64 |
| `same[a, b]` | Whether two byte strings are equal, decided in time that does not depend on where they differ (a signature checked against the one computed) |
| `pack[n, width]` | An int as `width` bytes, big-endian — 1, 2, 4 or 8, the low bytes of its two's complement; another width stops |
| `ieee[x]` | A number as the 8 bytes of its IEEE 754 double, big-endian |
| `binary[bytes, media]` | A response of the bytes, as the media type names them (`msgpack`'s) |
| `mask[bytes, key]` | The bytes XORed with the 4-byte key, cycling: a client's frame masked, a masked frame read |
| `read_head[stream, deadline]` | A connection's answer's head read to its blank line — the status and phrase, the headers — and, as the answer's `body`, what came after it (a body's start, a socket's first frame); written in Crow |
| `upgrade[url, timeout]` | A connection to a `ws://` or `wss://` URL's host upgraded to a WebSocket as RFC 6455 says — the key sent, the server's accept checked — or the reason; the socket holds what came after the head; written in Crow |
| `socket_of[stream, url, unread]`, `stream_of[socket]` | A socket over an upgraded stream, holding bytes unread; the stream a socket talks over |
| `leftover[socket]`, `unread[socket, bytes]` | The bytes a socket holds unread, taken; bytes put back for its next read (what a read took past a frame's end) |
| `mailer_field[mailer, name]` | What a mailer holds: its `host`, `port` (as text), `user` or `pass`; another name stops |
| `secure[stream, host]` | A plain connection put behind TLS from here on (SMTP's STARTTLS), the certificate checked against the host's name; the same stream for every value that holds it, or an error value of kind `'sys'` |
| `civil[t]` | A moment's civil date and time, UTC, as a list: the year, month and day, the hour, minute and second, and the day of the week (0 is Sunday); written in Crow |
| `betainc[x, a, b]` | The regularized incomplete beta function I_x(a, b): the t and F distributions are written over it |
| `gammainc[a, x]` | The lower regularized incomplete gamma function P(a, x): the chi-square distribution is written over it |
| `lgamma[x]` | The natural log of the gamma function |
| `note[stream, key, text]`, `noted[stream, key]` | A note kept on a connection under a key, read back (empty when there is none; empty text lets the key go): a query's id and a backend's process and key — what a cancel needs later — how long the server keeps the connection, the statements prepared on it |
| `machine[]`, `process[]`, `alive[pid]` | This machine's name (letters, digits, dots and dashes, or `local`), this process's number, and whether a process of this machine is running (a number that is none counts as running) |
| `xor[a, b]` | The bytes XORed one with the other, of one length (a SCRAM proof) |
| `copy_of_frame[frame]` | The frame's rows in Postgres's COPY text format: a tab between cells, a line per row, `\N` for a missing cell, a backslash, a tab, a line break and a return escaped; a whole float without its decimals, a truth as `t` or `f`, a moment as it is written |
| `rowbinary_of_frame[frame, types]` | A frame as ClickHouse's RowBinary, each column encoded as the table's type for it names (one per frame column, in the frame's order): what `INSERT … FORMAT RowBinary` takes after the statement; a missing cell NULL where the type holds one, else the type's own nothing; a decimal as its digits at the type's scale, a half away from zero (a float as the shortest text that reads back as it); a cell a type cannot hold — a number past its range, a fraction where a whole number is taken — is an error value of kind `'frame'` naming the column (a number had been cut to the type's width: 300 written as a `UInt8` had been 44) |
| `shaped[t, dims]` | The tensor's values in the shape the list of sizes names: `reshape` given its sizes as a list, as a Crow function's rest arrives |
| `remote[path]` | Whether a path names a remote source: a URL (`http://`, `https://`) or an S3 object (`s3://`) |
| `exchange_on[stream, hostline, method, path, lines, body, deadline, keep]` | One request on a connection already open, written in Crow — the method, the path, the host line, `connection: keep-alive` when the connection is kept for the next request else `close`, the body's length and the header lines — and its answer read: what `exchange` does on a connection of its own, and a driver on one it keeps |
| `exchange[url, method, lines, body, timeout]` | One HTTP exchange, written in Crow: the request sent to the URL's host with its method, the header lines and the body, its answer read whole — an `answer`, or an error value of kind `'sys'` where none came in `timeout` seconds (0 as thirty to connect and ten minutes in all) |
| `redirected[from, location]` | Where a redirect sends the request: its `location`, made absolute against the URL it came from |
| `with_query[url, pairs]` | The URL with the pairs added, each name and value URL-encoded, after the `?` it has or one it is given |
| `fetch[path, headers, query, timeout]` | A remote source's answer, written in Crow: a URL got with a reader's options and its redirects followed, or an S3 object got signed — a success, or the reason as an error value of kind `'sys'` (the options on a path that is no URL, a timeout below 0, no answer, an answer that is no success) |
| `put[path, bytes]` | The bytes written to an S3 object, signed; a URL is refused — it is read, not written |
| `format_of[path]` | A table's format by its path's extension (a URL's query left out): `'csv'`, `'json'`, `'parquet'`, or `''` |
| `format_sniffed[media, bytes]` | The format of what a URL with no extension answered: the media type its server named, then the bytes themselves (Parquet's `PAR1`, JSON's opening bracket), and CSV otherwise |

A stream is one connection however many values hold it, as a socket is; `type[s]` is `stream`. Every function works identically in both modes (`crow_runtime::sys`). The HTTP protocol is written on this layer, in `src/modules/sys.co`, as rows of `sys` itself: a URL taken apart, a connection opened plain or behind TLS, the request line, headers and body sent, the answer's head read to its blank line and its body by its length, by its chunks, or until the server closes (`exchange`), redirects followed and a reader's options applied (`fetch`), an S3 object signed as AWS's Signature Version 4 says and read or written by path at an endpoint (`fetch`, `put`). On the protocol stand the HTTP client — `request` (Requests), in `src/modules/core.co` — and every reader of a URL or an S3 object: `read` and `write` (core.co), `use io`'s `csv` and `lines` (io.co), `use frame`'s `load` and `save` (frame.co) and `use math`'s `load` (math.co), each a few lines over the layer's parsers (`csv_rows`, `frame_of_csv`, `tensor_of_frame`). A program never sees the layer: it sees `request` and `read`, whose rows in the builtin table are the contracts the Crow bodies keep (math.nn, Design Note "a builtin written in Crow keeps its row").

```
use sys

b = bytes['héllo']
log[len[b]]                      --- 6 (two bytes for é)
log[text[slice[b, 3, 6]]]        --- llo
log[find[b, bytes['llo']]]       --- 3
bad = text[slice[b, 1, 2]]       --- half of é: not text
log[bad.kind]                    --- parse
s = open['127.0.0.1', 1, false, 0.2]
log[s.kind]                      --- sys: nobody listens on port 1
```

**Design Note: the standard library's layer.** Considered: writing the standard library's modules in Rust only, as they were (rejected: a module both backends share is a bug both share, which no parity or fuzz run can see; the week of 2026-10-03 found eleven bugs where the two backends differ and none in the kernels they share, because nothing can look there); giving every program bytes, sockets and threads (rejected: a program would gain the bug classes the language exists to exclude — data races, half-read buffers, protocol state spread over user code — and a second way to serve and connect beside `serve` and `connect`). Decided (Vijay, 2026-10-03): one `sys` module, the smallest native layer the rest of the library can be written on, importable by the compiler's own modules alone; a program sees one way, and every module written on `sys` is compiled and run twice, fuzzed, and takes laws.

### frame

Import with `use frame`. Column-oriented DataFrames for tabular data. See `docs/frame-module.md` for design details.

**Column references: strings for names, expressions for computation.** Naming a column is a string; computing on a column is a normal Crow expression over `df.col`. Every line is plain Crow — there is no special evaluation inside frame calls.

| Function | Description |
|---|---|
| `new[]`, `new[col = values, ...]` | The empty frame (`frame.new[]`, since `new` reads best qualified), or a frame written down: each named argument a column, in order — `frame.new[sym = ['AAPL', 'MSFT'], qty = [100, 50.5]]` — its values a list, a tensor or one value repeated, as `with` takes them (it is `frame.new[sym = …, qty = …]`, typed column by column; until 2026-10-09 only that chain could write a frame down) |
| `with[frame, col, values]` | A frame with one more (or replaced) column: `values` is an array, a tensor, or a scalar repeated for every row; the length must match the rows (error-value otherwise). Ints held as a tensor (`[1, 2]`) make a column of ints |
| `load[path]` | Load a table → frame, in the format its extension names — CSV, JSON records (columns in name order), Parquet — from a file, a URL or an S3 object (error-value on failure; Files, URLs and S3). A CSV column of whole numbers is ints, of numbers floats, of `true`/`false` (or `True`/`FALSE`, as pandas writes them) bools, of ISO 8601 moments a moment column, anything else text; an empty cell is missing; a cell in quotes may hold a comma or a quote written twice (RFC 4180), though not a line break |
| `save[frame, path]` | Write frame → a file or an S3 object, as CSV, JSON records or Parquet by its extension (CSV otherwise); a text cell holding a comma, a quote or a line break is written in quotes, as `load` reads it; a failure is an error value of kind `'frame'`, as `load`'s is (compiled, it had stopped the program) |
| `select[frame, cols]` | Pick columns: `select[df, ['name', 'age']]` |
| `filter[frame, mask]` | Keep rows where mask is true: `filter[df, df.age gt 50]` |
| `sort[frame, col]`, `sort[frame, [cols]]` | Sort ascending: `sort[df, 'age']`. Descending: `sort[df, 'age', desc = true]`. By several keys, `sort[df, ['symbol', 'qty']]`: by the first, ties by the next; `desc = true` turns every key, `desc = [false, true]` one each. Stable — rows whose cells are equal keep their order — with a missing cell, or a float that is not a number, last either way |
| `group[frame, col, aggs...]` | Group + aggregate: `group[df, 'dept', ['salary', 'mean'], ['age', 'max']]`; by several keys, `group[df, ['dept', 'city'], ['salary', 'mean']]` — one row per combination of the keys' cells, in the order first seen, each key column as it was (a row with a missing key cell is in no group). Each aggregate is named `col_how` and is in the kind it has: `count` the present cells, an int; `sum` an int over ints or bools and a float over floats, 0 where a group has no present cell; `mean` a float; `min` and `max` a cell of the column's own kind (text by character code, moments by their instant), missing where a group has no present cell. A missing cell is skipped, as SQL's aggregates skip NULL; a `sum` or `mean` over text stops the program |
| `pivot[frame, rows, cols, values]` | The long frame made wide: a row per cell of `rows`, a column per cell of `cols` — named by the cell's text, in ascending order — holding the `values` cell of the row that has the pair, and a missing cell where no row does. A pair two rows share stops the program: aggregate with `group` first |
| `window[frame, col, how, n = 0, by = [], span = 0, on = '']` | A column computed over each row's *partition* — the rows sharing the `by` columns' cells (one name or a list; the whole frame without) — in the frame's order: an aggregate (every word `group` takes) over the trailing `n` rows, over every row so far when `n` is 0, or, with `span = seconds` and `on = column`, over the rows whose `on` cell lies within the span before the row's own (a column of moments, or of numbers in the span's units; the partition in ascending order of it, or the program stops: `sort` first) — `(t - span, t]`, as pandas and polars close a time window; a `sum`, `min` or `max` over ints is an int, a `mean`, `median`, `std`, `var` or percentile a float, a `count` or `distinct` an int, `first` and `last` the column's own cells; `'lag'` and `'lead'` the cell `n` rows back or ahead, missing where there is none; `'rank'` the cell's rank in its partition, ascending, equal cells sharing a rank and the next rank skipping (none of the three takes a span). Add it to the frame with `with`: `with[df, 'avg3', window[df, 'close', 'mean', n = 3, by = 'symbol']]`, `with[df, 'vwap_5m', window[df, 'px', 'mean', span = 300, on = 'at', by = 'symbol']]` |
| `join[f1, f2, key]`, `join[f1, f2, [keys]]` | Join on a key column, or on several: `join[df1, df2, 'id']`, `join[df1, df2, ['symbol', 'day']]`. Its kind: `join[df1, df2, 'id', 'left']` — `'inner'` (the default), `'left'`, `'right'`, `'outer'`; `'semi'` the left's rows that have a match and `'anti'` those that have none, the left's columns alone; `'asof'` each left row with the right row whose last key is the latest at or before the left's, the other keys matching exactly — a column of moments, or of numbers — missing where there is none (a left join by time; the right's clock is the frame's own order, sorted as needed) |
| `stack[f1, f2]` | Stack frames vertically |
| `head[frame, n]` / `tail[frame, n]` | First/last n rows (returns a frame) |
| `fill[frame, col, value]`, `fill[frame, col, how = 'forward' \| 'backward']` | The column's missing cells given `value`, of the column's kind; or each given the nearest present cell before it (`'forward'`) or after it (`'backward'`), one with none staying missing |
| `unique[frame, col]`, `unique[frame, [cols]]` | The column's distinct present cells, first seen first, in the column's own kind (an int column's are ints): `unique[df, 'symbol']`. With a list of columns, a frame: the first row of each distinct combination of their cells, every column as it was, a missing cell a value of its own |
| `rename[frame, old, new]` | Rename a column: `rename[df, 'salary', 'pay']` |
| `unpivot[frame, keys, cols]` | The wide frame made long, the reverse of `pivot`: a row per key row and column, the key columns (one name or a list, `[]` for none) repeated, `variable` the column's name and `value` its cell — the columns' cells one kind, ints beside floats floats |
| `complete[frame]`, `complete[frame, cols]` | The rows whose every cell is present, or every cell of the named columns (one name or a list): `filter` by `.ok` over all of them |
| `sample[frame, n]` | `n` of the rows, drawn without replacement by the seeded generator (see Random Numbers), kept in the frame's order; `n` beyond the rows stops the program |
| `where[mask, a, b]` | A column that takes `a`'s cell where the mask is true and `b`'s elsewhere — each a column, a list or one value for every row, both of one kind (ints beside floats floats); a missing cell stays missing. (`math.where` is the same word over tensors.) |
| `bucket[column, seconds]` | Each moment's bucket start (see Moments, `bucket[t, seconds]`), a column of moments: `group[with[df, 'hour', bucket[df.at, 3600]], ['symbol', 'hour'], ['px', 'first'], ['px', 'max'], ['px', 'min'], ['px', 'last'], ['qty', 'sum']]` is the hourly bars |
| `stats[frame]` | Each numeric column's count, mean, population standard deviation, min, lower quartile, median, upper quartile and max, as text |

A `datetime` column holds moments (see Moments): `with` makes one from a list of moments or one repeated, `fill` fills one with a moment, a cell read one at a time is a moment, a mask compares the column with a moment, and `load` reads one from ISO 8601 text and `save` writes it back as such.

Properties: `.shape`, `.columns`, `.dtypes`. Column access: `df.age`. `len[df]` returns the row count. Aggregation names for `group` and `window`: `'sum'`, `'mean'`, `'min'`, `'max'`, `'count'`, `'median'`, `'std'`, `'var'`, `'first'`, `'last'`, `'distinct'` and a percentile `'p1'` to `'p99'` (`'p90'`). A `median` and a percentile are interpolated between the two cells they fall between (as pandas, numpy and Julia interpolate), a float; `std` and `var` the population's (as `stats` and `std` of a tensor answer), floats; `first` and `last` the first and the last present cell, in the column's kind; `distinct` the count of distinct present cells, an int. Join kinds: `'inner'` (the default), `'left'`, `'right'`, `'outer'`, `'semi'`, `'anti'`, `'asof'`.

**Text, casts and a moment's fields over a column.** `lower`, `upper`, `trim`, `replace` and `sub` of a frame's column answer a column of text, each present cell as the text function answers it, a missing cell staying missing (a `sub` pattern that is none stops the program: a cell cannot carry the reason); `contains`, `starts` and `ends` of a column answer a mask, a missing cell false. `int`, `float`, `str` and `moment` of a column convert every cell as the one-value form does — text read as a number or a moment, a number as seconds, a moment as its seconds, anything as the text it shows — a cell that cannot be converted missing, which the column's `.ok` says (`int[df.price]` cuts toward zero; `str[df.day]` is text that `load` would read back as ints). `df.at.hour`, `.year`, `.month`, `.day`, `.minute`, `.second`, `.weekday` and `.offset` of a column of moments are a column of ints, each read in its own zone. (`len[df.col]` stays the column's length: a cell's length is `len[df.col[i]]`.)

A column the frame lacks, an aggregation or a join it does not know, a mask of another length than the rows, a negative count for `head` or `tail`, a `fill` value of another kind than the column's (an int fills a float column), and a count of arguments the function does not take stop the program with the reason (see When a Program Stops), in both modes, as `df.col` of a missing column does. A column copied with `with` keeps its missing cells.

**Several keys, pivots and windows.** `group[df, ['symbol', 'day'], ['qty', 'sum']]` has a row per symbol and day, `symbol` and `day` as they were and `qty_sum` an int, as `qty` is. `pivot[df, 'symbol', 'day', 'qty']` lays the days out as columns, one row per symbol, with `qty` in the cells — the columns are the days' text (`'1'`, `'2'`), so they show as the program runs and their cells are read as a loaded file's are, with `int[…]`, `float[…]` or `str[…]`; a symbol and day two rows share stops the program (`pivot: 'b' and '1' appear 2 times — one row per pair: aggregate first, group[frame, ['sym', 'day'], ['qty', 'mean']]`). `window[df, 'qty', 'sum', by = 'symbol']` is each row's running total within its symbol, `window[df, 'px', 'lag', n = 1, by = 'symbol']` the symbol's previous price, missing on its first row, and `window[df, 'px', 'rank']` each price's rank over the whole frame; the trailing forms (`n = 3`) take the rows there are at a partition's start. A window's cells are typed by what it computes — a `sum`, `min` or `max` in the column's kind, a float for `mean`, an int for `count` and `rank`, the column's own cells for `lag` and `lead`, which can be missing — and a column a window or a pivot names that the frame lacks is refused before the program runs, as a key is. A column handed to `sum`, `mean`, `min` or `max` holds numbers, and a missing cell stops the program as it does where numbers are taken. The same in both modes.

**Design Note: pivot reshapes, group aggregates.** Considered: `pivot[df, rows, cols, values, how = 'sum']`, a pivot table that aggregates the rows sharing a pair, as a spreadsheet's does and pandas's `pivot_table`. Decided (2026-10-02, with the frame work): one way to aggregate — `group` — and one way to reshape; a pair two rows share stops the program, naming the `group` that aggregates it. Rows and columns come in ascending order of their cells, where `group` keeps the order first seen: a pivot is read as a table, and its days and symbols are looked for by their order.

**Design Note: percentiles are words, keys are lists, and the other names are the old ones.** Considered, for a percentile, `['px', 'quantile', 0.9]` — a list holds one kind, and `'quantile 0.9'` as text — a name with a space in it. Decided: `'p90'`, one word for the 90th percentile, as a column is named by it (`px_p90`); `'median'` is `'p50'` by its own name. Considered, for a join on several keys with an as-of clock, pandas's and polars's `by =` beside `on =`. Decided: one list of keys, the last the clock — `join[bars, signals, ['symbol', 'at'], 'asof']` reads as the match it is. Considered, for a conditional column, a new word (`pick`, `choose`); decided: `where`, math's word for the same operation over tensors — one name, the kinds picking the row. Considered, for the reverse of `pivot`, pandas's `melt` and polars's `unpivot`; decided: `unpivot`, naming its two new columns `variable` and `value` as both do, so a program ported from either keeps its names. Considered, for rows without a missing cell, `dropna`; decided: `complete`, the word the masks section already used for them. A bucket's edge is the local midnight of the moment's own offset (a day in New York starts at 04:00Z), where pandas floors a zone-aware moment in UTC: a bar is drawn on the exchange's clock, and a moment carries the offset it was written in for this. A fill by direction is `fill[df, col, how = 'forward']`, since `fill[df, col, 'forward']` would be the text in a column of text. A text function over a column keeps its name, as the arithmetic operators keep theirs; `len` of a column stays its row count.

**Design Note: the frame module is held to pandas.** `tests/parity/185_frames_held_to_pandas.co` computes 59 cases over four tables (`tests/frames/data/`) and compares each with what pandas 3.0 answered (`tests/frames/expected/`, written once by `tests/frames/reference.py`): loading and its kinds, `select`, six filters, five sorts, `head` and `tail`, four groups, a pivot, running, trailing, shifted and ranked windows, the four joins, `stack`, three fills, `unique`, `rename`, a computed column, reductions, moments moved between offsets, and quoted cells read and written back — every column, kind and cell the same, in both modes (2026-10-06); and, since 2026-10-07, the other aggregates (a median, percentiles, the deviation, first and last, distinct), windows of them trailing and running and over a span of six hours, sorts on two and three keys, joins on two keys of every kind, distinct rows, an as-of join, six-hour buckets and the bars over them, text and a moment's fields over a column, an unpivot, fills by direction, complete rows, a conditional column and casts. What that round found: pandas's rolling kernels compile to fused multiply-adds, so a deviation or a percentile agreed to twelve digits and not the last — the runtime's `mul_add` now computes the same rounding, and a window whose squared sum collapsed is computed afresh as pandas's is. What it found and changed: a CSV column of `true`/`false` had loaded as text (so `save` then `load` lost the kind) and a quoted cell had been split at its comma; the sort was unstable and a missing int cell sorted as 0 — now stable, missing cells last either way, as pandas sorts them (polars puts them first, Postgres first or last by direction: one rule was needed, and the one that keeps the present cells together was taken); every aggregate of `group` was a float and a sum over text silently 0; a window's sum over ints was a float; `unique` answered text for every column, and gave up scanning after a long stretch without a new cell; a right or outer join's unmatched row had lost its key; a sum over floats differed from pandas and numpy in the last digit — a `group` or `window` total is now compensated as pandas keeps one (Kahan, with pandas's separate compensations for the cells added and taken off, and its exact answer for a window whose cells are all one value), and `sum[t]` adds as numpy does, pairwise; and, in the compiled program alone, a moment cell read one at a time was seconds where the interpreter read its text, a field of a moment that can fail (`tz`'s answer) did not build, `[1, 2]` given to `with` made a column of floats, a list a function started empty and pushed onto did not build, and a list of a loaded file's cells, read one at a time, did not `join`. And in `crow test` alone, `window[…, by = …]` and `request[…, timeout = …]` were refused in every test block: the runner desugared the program twice, and the options filled in once were too many arguments the second time. Where pandas has a choice, the spec's is taken, and the script says so: groups in the order first seen (`sort=False`), a trailing window over the rows there are (`min_periods=1`), `rank` sharing a rank between equal cells (`method='min'`), the population deviation (`ddof=0`), a right frame's duplicate column named `_right`, and a right or outer join compared sorted, since pandas orders those rows by the right frame or by key where Crow keeps the left frame's order and appends the unmatched. Not compared: `stats`, a display, and `fill` of a moment column, which neither side needed.

**Design Note: a window is a frame's, an indicator a series's.** Considered: leaving trailing aggregates to the market package's indicators (`sma[bars.c, 3]` is a trailing mean over one series), so that a frame has no second way to a moving average. Decided: `window` computes within a frame's partitions, in its order — one column holding a series per symbol, each windowed on its own — where an indicator takes one series; over one series, `sma[df.close, 3]` and `window[df, 'close', 'mean', n = 3]` agree. A trailing window's first rows are the rows there are, as SQL's window frames give them (pandas leaves them empty until the window fills).

**A frame's type is its columns.** A frame built in the program (`frame.new[]`, `with` given a column's name), a query a database described (see db, Typed cells), and a frame derived from either through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack`, `join`, `rename`, `group` or `pivot` carries its columns in its type, each with its cells' type (a column `with` copies keeps its cells' type; a pivot's rows column, and its others as they show). A column named by a variable (`with[df, col, xs]`) leaves the columns the type knows and makes the frame *open*: a column it cannot name may be there. A reassignment binds the frame anew: after `df = with[df, 'x', xs]`, `df` has `x` from that line on, and before it the frame it was. Where branches, or a loop's rounds, meet, a frame has the columns they all give it; a column only some add may be there, and is read as the program runs. A function handed frames of different columns is a copy per set of columns, so it reads the columns each call hands it. Where a frame's columns are all known, a column it lacks — read as `df.x`, named where `select`, `sort`, `fill`, `rename`, `group` or `join` takes a column, or named by a table (`key`, `mark`, `delta`, `hide`) or a chart (`candles`, `lines`, `bars_y`) — is refused before the program runs, in both modes, with the columns the frame has (`frame has no property or column 'cost' — its columns are 'price'`, and, read in a function, the parameter the frame was handed as); the stop above is for a frame whose columns show only as it runs (a file `load` reads then, a query no database described, a column a variable names).

**A frame loaded from a file has the columns the program reads.** `frame.load` cannot see a file's columns before the program runs, and neither can the program state them: the reads of the frame's cells say what they are. A cell walked and handed to `upper`, or to a function that reads it as text, is text; one met as a number — compared with one, added to one, halved — is a number, held as floats whichever the file wrote, so `q gt 20` holds for a file's `20.5` and `q / 2` of its `7` is `3.5`, as a variable that may hold a float divides (§1); one handed to what takes nothing but an int — a position, `xs[q]`, or `head`, `repeat`, `slice` — is an int; one decided on is a bool; one met beside a moment is a moment. A function the cell is handed to says what its own body reads the parameter as. The frame is held to that shape where it is bound, before any cell is read: a column missing, or holding another kind — numbers where the program reads text, text where it reads numbers, floats where a position reads ints — stops the program there with the column, what it holds, what the program reads and the read that asked (`column 'slot' holds floats where the program reads ints (a position in labels at line 12)`), the same words in both modes. A column of whole numbers the program reads as numbers is widened to floats at the load, exactly: a whole number beyond 2^53 stops the load there rather than lose digits (read it with `int[…]` where it is compared). From then on the frame's cells are typed, in both modes, and nothing is read first. A column no read says anything of stays as it comes, a parsed value read as its use implies where it is used (see JSON). A column read as two kinds — text here, a number there — is refused before the program runs, naming both reads. A page's data source (`fetch[url]`, `poll[url, seconds]`, ui) is such a frame too: its reads say its columns, the fact carries them to the session, and the answer is held to them where it arrives — its records a frame, a column of whole numbers read as numbers floats, an answer of no records the table with no rows yet — before the page shows; what is not what the page reads (no answer, no JSON, a column missing or of another kind) stops the session there, told to the reader as a handler's stop is, the reason in the server's log with the column, what it holds, what the program reads and the read that asked — the same in both modes.

```
fn label[t]
    return upper[t]

df = frame.load['orders.csv']    --- sym is text, qty a number: the reads below say so
labels = for t in df.sym
    label[t]
for q in df.qty
    if q gt 20
        log[q]
```

**Design Note: a loaded frame's columns are what the program reads, not what it writes.** Considered: the program stating a column's type (`columns = map[sym = 'str']`, in the words `type` answers) or showing it by a sample value (`map[sym = '', qty = 0]`). Rejected: each is a second way of saying what a value already says, and the sample asks the programmer to invent one. Considered: reading every cell first, as a parsed value's (the way until 2026-10-08). Rejected: the program did nothing wrong, and the compiled program had not held a function handed such a cell. Decided: the reads say the columns, the file is held to them where it is loaded, and a column no read types stays as it comes. Considered, and built for a day: a cell compared with an int held as ints, so a file's `20.5` stopped the load where the program wrote `q gt 20`. Rejected (Vijay, 2026-10-08: "q gt 20 when q = 20.5 should work. as a human being, it is obvious ... you are comparing two numbers. whether it is a float or int, it does not matter"): the program says a number, which kind the file's are is the file's business, and a program's meaning must not turn on whether the file wrote `7` or `7.0` — the language's own rule, that a variable which may hold a float is a float, says what the column is. Only a use that takes nothing but an int asks the file for ints, and the stop names that use. A page's data source is held the same way where its answer arrives, in the session both modes share (2026-10-08: the interpreter had answered an empty map and registered no source at all, and the compiled program had turned the records into a frame of its own at every render).

**Design Note: a frame's reassignment is a new binding.** Considered: one type per variable, as every other variable has, the frame's columns joined over its assignments — which loses exactly the columns a program adds (`df = with[df, 'x', xs]`, then `df.x`); and a name per shape (`df2 = with[df, …]`), which asks the program to rename what it changes, and makes a loop that adds columns impossible to write. Decided (Vijay, 2026-09-29): a frame variable's type is its last assignment's — a new binding where its columns change, as a Rust `let` shadows one — and where ways through meet it has the columns they share.

**Design Note: a frame operation's misuse stops the program.** Considered: error values of kind `'frame'`, which the table of kinds listed for `filter`, `sort`, `group` and `join`, and which the interpreter answered. Rejected: the operations are typed as giving a frame, which cannot fail, and the compiled program stopped — so a program that checked `.ok` worked in one mode only — and the same column the frame lacks, read as `df.col`, stops the program. A misuse is the program's mistake, not something it reads: an error value would carry it to wherever a decision first met it. Reading a file, `load`, and values from elsewhere, `with`, stay error values.

#### Boolean Masks

Filtering uses ordinary Crow expressions that evaluate to boolean arrays. These operations are vectorized (element-wise):

- `eq ne gt lt gte lte` — column vs scalar → boolean array (the scalar may come first: `50 lt df.age`)
- `'late' in df.note`, `starts[df.symbol, 'A']`, `ends[df.note, 'k']` — a text column tested cell by cell → boolean array, a missing cell false
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

Each cell is compared with the value as two values compare (see Comparison): numbers by value, exactly — NaN holds only `ne` — text by its characters and ordered by character code, bools equal or not, moments by instant. A value of another kind than the cells — `df.name eq 2` of a column of text — and an order between kinds with none — `gt` on a column of bools — are refused where the column's type is known, and stop the program where it shows only as it runs (a file `load` reads). Missing values compare as `false` for every operator, `ne` included — they are excluded by any filter, and `not (df.age gt 50)` is what keeps them. A list compared with one value is a mask the same way. (Compiled, a missing or NaN cell had been `false` for `ne`, `eq` had been nearness within 2⁻⁵², a column of text had been parsed as numbers and a column of bools had compared as nothing; the interpreter had counted a missing cell as `ne` and stopped on an order of text; a list of values that can fail, compared with one value, had not built.)

**A filter's predicate is computed in one pass.** `filter[df, df.age gt 50 and df.name ne 'bob']` — a predicate that compares the frame's own columns with values that only read, asks a column's `.ok`, and combines those with `and`, `or` and `not` — reads no column into a list and makes no mask per comparison: the rows are decided block by block, across the cores for a large frame, each cell as its comparison decides it. The rows kept are the mask's, and a column the frame lacks or an order between kinds with none stops with the operator's words at the first comparison that meets it. The same in both modes.

A column's `.ok` is its rows' validity — `true` where the cell is present, `false` where it is missing — so `filter[df, df.age.ok]` keeps the complete rows. (Every other value's `.ok` is its own status: a list that did not fail is ok.) A frame shows a missing cell as `missing`, and a column shown as a list shows it as the error it is read as, `error: missing`; a NaN a number column holds is a value, and shows as `nan` (until 2026-10-08 a missing cell had shown as `nan` too, in a text column as well). Read one at a time (`df.age[3]`), a cell of a column that can have missing cells is fallible: a missing one is an error value of reason `missing`, which computes and cannot be decided on (§10), so `.ok` guards it and a guard narrows it to its value. The columns that can are a database column that can hold NULL (see db, Typed cells) and the side of a `join` a row can have no match on — a left join's right columns, a right join's left ones, all of an outer join's. `fill[df, 'age', 0]` gives a column a value where it had none, and its cells are plain again, as are a column's that cannot have missing cells. A mask decides every row, so a list of values that can fail — built from such cells — is refused as one, in both modes: fill the column first, or guard each value with `.ok`. Handed whole to a function that takes numbers — `mean[df.age]`, `median`, `sma[bars.c, 3]` — or used in arithmetic — `df.price * 2`, `df.a + df.b` — a column is its numbers, as a list of numbers is, and the answer is a tensor; a missing cell stops the program (`row 3 of the column is missing — fill the column first: fill[frame, 'column', value]`), and so does a column of text or of bools, in both modes. (Compiled, a missing cell had been read as `0` and a column of text parsed, where the interpreter stopped — or, for `df.a + df.b`, refused the sum.) A comparison is a mask, as above. A column copied with `with` keeps its kind and its missing cells, even when every cell is missing.

**Design Note: a missing cell is an error, not a placeholder.** Considered: reading a missing cell as its column's placeholder in both modes — `NaN` in a number column, `0` in an int one, `false` in a bool one, `nan` in text — as a filter reads it, which both modes did. Honest for a number, since NaN compares as false; for an int or a bool it is a guess the program cannot tell from a value, of the kind §10 refuses elsewhere (an unmatched row's count read as `0`). Decided (Vijay, 2026-09-25, typed frames decision 3): where the schema says a column can have missing cells its cells are fallible, and where it says it cannot they are plain, so no cell is a guess.

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


### web

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

**Inside a view, the standard HTML element vocabulary is active** — plus the vocabulary's own elements: the containers `stack`, `row`, `grid`, `overlay`, `scroll`, the layers `modal`, `drawer`, `toast`, and `details`, `summary`, `dropdown`, `divider` (Revealing and framing, below) — `div`, `span`, `p`, `h1`, `h2`, `h3`, `h4`, `h5`, `h6`, `button`, `ul`, `ol`, `li`, `form`, `a`, `nav`, `header`, `footer`, `section`, `tr`, `td`, `th`, `thead`, `tbody`, `option`, `textarea`, `label`, `pre`, `code`, `input`, `img`, `table` — with their exact HTML names. **Outside a view, those names don't exist**: `p = 0.95`, `fn add[a, b]`, and `form = req.body` are ordinary code in any ui program, and `use ui` reserves nothing in your file. Calling an element outside a view is an error that points at the fix.

**A child that is a list contributes each item.** Markup a view or an element made is markup, anything else is text — so `rows = for s in sessions` over a view, then `stack[h2['Sessions'], rows]`, renders one row per session. This is how a page shows many of something with their own controls; `table[frame]` is for cells that are values. **A click can ask first**: `button['End session', click = end_session, confirm = 'End this session?']` shows the question in a dialog the runtime draws, and runs the handler only when the reader says yes. The server enforces it — the button offers the question, not the handler, and the dialog's own button offers the handler while the dialog is open — so the handler cannot be reached without the answer. On a table the question is a map from action to text: `actions = map[Delete = delete_session], confirm = map[Delete = 'Delete this session?']`. **A key presses an element**: `button['Save', click = save, shortcut = 'ctrl+s']` is pressed by Ctrl+S as a click presses it — the same handler with the same arguments, the question first when it asks one — and only while the page draws it and it can be pressed (shown, not disabled, not busy), so a key reaches only what the reader could click. A shortcut names its modifiers, `ctrl`, `alt`, `shift` and `meta`, then a key: a character (`'?'`, `'/'`, `'g'`) or one of `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown` and `f1` to `f12`. Alternatives are separated by a space (`'ctrl+s meta+s'`, for the Mac's Command key), and a character shift makes is written as that character (`'?'`, not `'shift+/'`). It is drawn as the browser's `aria-keyshortcuts`, which a screen reader announces. A key without `ctrl`, `alt` or `meta` is the reader's while they type in a field, Escape excepted, and Enter and Space are the focused control's; of two elements that take the same key, the one in an open dialog is pressed, else the first. The element must do something when pressed — a `click`, or it is a button or a link — or the program is refused, as a shortcut that names no key is, in both modes. A dialog's Close says `shortcut = 'escape'`, and the runtime's own question cancels on Escape. **A form submits once and never reloads the page**: Enter in a field presses the form's first button, and `form[field['Name', name], button['Add'], submit = add]` runs `add` when the form is submitted, by its button or by Enter, with the fields' facts already bound. **Boolean attributes follow state**: `button['Start', disabled = starting]` writes `disabled=""` when true and nothing when false, for `disabled`, `hidden`, `readonly`, `required`, `checked`, `selected`, `multiple`, `autofocus`.

**Design Note: keys press elements.** Considered: `keydown = h` on an element or the page, the handler handed the key's name to decide what it means (ui-framework.md, Keyboard Events). Rejected: every shortcut would be an if-chain over key names, in a second kind of handler with arguments of its own, and a key could reach work no button offers, where a session accepts only what a render offered. A key that presses a drawn element keeps one handler, one set of arguments, and the reader's view of what they can do.

Views compose: a view calls other views (components) and ordinary functions freely. A module can export views — importers reach them qualified, like any module name:

```
--- cards.co
view card[title, body_text]
    div[h2[title], p[body_text]]

--- main.co
use cards as c

view render[]
    div[h1['Home'], c.card['Status', 'All good']]
```

This is the JSX/SwiftUI boundary: the lowercase vocabulary is fixed and language-owned; your components are module-exported views. There is no way to add elements or define new vocabulary zones — see the Design Note "Names Are Never Ambiguous".

Views and event handlers are the only functions that see reactive state (see Function Scope). `crow fmt --fix-views` migrates pre-view code (`fn` declarations that build ui trees) automatically.

**The view tree is target-independent.** A view describes a tree; what renders it is a build target. Web today; a desktop or mobile shell renders the same tree without a language change. Platforms are build targets, never dialects — there will never be a `screen` or `window` vocabulary fork.

#### Reactive Data and Inputs (`use ui`)

Top-level variables in a ui program are reactive state (see Function Scope). Three builtins declare state that the runtime keeps current, and bind inputs to it:

| Function | Description |
|---|---|
| `fetch[url]` | A frame loaded once by HTTP GET when the page connects — from a URL, or from a path of this app's own (`fetch['/api/rows']`) — held where it arrives to the columns the page reads of it (Frames: a frame loaded from a file has the columns the program reads) |
| `poll[url, seconds]` | The same, re-fetched every `seconds`; every reader re-renders when it changes |
| `field[label, var, type = '', lines = 0]` | A labeled input two-way bound to reactive variable `var` (inside a view). `type` is the input's kind — `'text'`, `'number'`, `'date'`, `'time'`, `'password'`, `'file'` — and defaults to `'number'` for an int or float variable; `lines = 3` makes it a textarea |
| `slider[label, var, min = 0, max = 100, step = 1]` | A range input bound to a number |
| `checkbox[label, var]` | A checkbox bound to a bool |
| `toggle[label, var]` | A switch bound to a bool — the same binding as `checkbox`, drawn as a toggle |
| `radio[options, var]` | One of several strings, as radio buttons: `radio[['buy', 'sell'], side]` |
| `tabs[options, var]` | One of several strings, as a tab strip; the view shows the selected tab's content with `if mode eq 'Practice'` |
| `select[options, var]` | One of several strings, as a dropdown list |
| `progress[value, max = 1.0]` | A progress bar |
| `icon[name, size = 'm', color = '']` | An icon from the closed set (below), sized by the type scale, colored by a token |

Inputs are state: the reader's change lands in the variable, the handlers read it, and the next render draws it. There is no "value of the input" apart from the variable.

**Files in and out.** A file is an input like any other: `field['Trades', upload, type = 'file']` binds `upload`, declared `upload = map[]`. When the reader picks a file the page sends it to the server, and `upload` becomes a map of strings — `name` (the file's name on the reader's computer), `type` (its media type), `size` (its bytes, as text) and `path`, where the server keeps the copy — so a handler reads it as it reads any file: `csv[upload.path]`, `read[upload.path]`. The copy is the session's, kept under `.crow/uploads/` (a directory that ignores itself in version control) and removed when the session ends; a handler that keeps a file copies it (`cp[upload.path, …]`). A file bigger than the app allows — 10 MB, or `serve[…, uploads = 50]` in megabytes — is refused before it is sent, and the reader is told the limit. The other way, `download[value, name]` hands the reader's browser a file: a frame as CSV, a string as text, or a picked file (a map with a `path`) as that file's bytes, saved under `name`. It is an effect held to the handler's commit, as `notice` is, so a handler that stops sends nothing. In a test, `change[a, 'upload', map[name = 'trades.csv', type = 'text/csv', size = '120', path = 'fixtures/trades.csv']]` stands for a file the reader picked, and `downloads[a]` is the list of what the session was handed, each a map of `name` and `text`.

#### Derivations — `derive`

Facts are stored; beliefs are derived. A derivation is a value computed from facts and data sources, read like a fact, never assigned, and recomputed only when something it read changed:

```
qty = 100
sym = 'IMRN'

derive total
    return qty * 2

derive trades[fresh = 300]
    return query[book, 'SELECT * FROM trades WHERE symbol = ?', [sym]]

view render[]
    return stack[p['total `total`'], table[trades]]
```

`derive name` opens a block with a `return`, like a view; it never takes parameters. The name reads as a variable anywhere a fact can be read — views, handlers, tests, other derivations, and the program's top level, once, as it starts, over the facts as declared (parity 168) — and in a route's handler when the app holds it (below); assigning it is an error that says what to change instead. A derivation writes no fact: only handlers write.

**What it reads is decided once, from the source**, and both execution modes obey the same sets: the facts and derivations it names (transitively through the functions it calls, and inside interpolated strings), `now[]` if it or a callee reads the clock, and the tables its query literals read. A derivation recomputes, on its next read, when

1. a handler wrote a fact it reads;
2. a derivation it reads recomputed and its value changed — an equal number, string or bool cuts the chain (frames always propagate);
3. a statement in this program wrote a table it reads — an `exec`, an `insert`, or a `query` whose statement writes (`INSERT … RETURNING id` reads back the row it wrote, and the table is written all the same);
4. it reads `now[]` and a tick of `serve refresh` passed — a click's render is not a tick, so a click recomputes only what it moved;
5. it reads a table this program never writes — an external source — and its freshness window elapsed. `fresh = N` is seconds; the default `fresh = 0` means *as fresh as the loop*: recomputed every tick, which is what a query inside a view did before. Declaring `fresh` is the relaxation.

A derivation that fails — a query error — holds the error value; readers see `.ok` and `.reason` and a view renders the reason in place. `captured[x]` on a derivation is the moment it last ran. Derivations live in a program that serves: a `use ui` program with a page, whose views and handlers read them, or a `use web` program whose routes read them — `derive x needs a program that serves — …` in a script, which computes a value where it needs it. In the test harness `html[a]` is a tick and `click[a, h]` marks what the handler wrote. Design: `derive-design.md`.

**Who holds a derivation is inferred from what it reads.** A fact is a top-level variable; a database or socket handle is not one, and neither is a `fetch`/`poll` binding. A derivation that reads a fact a session writes — assigned by a handler or a helper a handler calls, bound to an input like `field` or `select`, or carried in a page's address (a `:name` of its path, a name of its `url = […]`) — or a data source, whose answers land in each session that asks, or another derivation held per session, is computed per session. One that reads only constants, tables and the clock — the day's universe, the latest bar in a table — is computed once for the app and shared by every session and route, and a table write in any session, or any route, reaches every derivation over that table. (A derivation over a fact of the address, or over a source, had been counted the app's: one session's address, or the source's first answer, served to all.) `crow check` prints each derivation's read set and who holds it — `held by the app`, or `held by each session (it reads 'picked', which a session writes)`.

**A render never waits for a query it does not need.** A derivation that reaches out — its read set includes a table or an external source — is computed off the render in a served page: a render reads its latest value, and when that value is stale the runtime starts a recompute on a worker thread and the render goes ahead with what it has; the page renders again when the new value lands. Staleness has a cause, and the cause decides what waits. A derivation over a table the current event wrote is recomputed before that event's render, so a click's own writes are on the page in its answer — after `buy`, the blotter shows the trade. One that is stale because time moved, because a fact it reads changed, or because another session wrote its table shows its last value until the recompute lands. A handler that reads a derivation always gets the recomputed value; a derivation over facts and the clock alone is computed where it is read.

Before its first value, a derivation whose value can be an error — a query's frame, anything that can fail — reads as an error of kind `'pending'` with the reason `loading`, which a view renders in place like any failed read; one that cannot fail is computed where it is first read. One recompute of a derivation is in flight at a time for each holder — the session, or the app for a shared one, so readers arriving together run a shared query once — and a value whose inputs moved while it was computed is dropped and computed once more with the latest inputs. `running[x]` is true while a recompute of `x` is in flight. The root element of a view that read a value still being recomputed carries `data-crow-stale` while the value is behind something the reader did — it was computed before their last change (a click or an input that moved a fact), or there is none yet — and the default stylesheet dims it after 300 ms, so a fast recompute never flashes. A value behind only the clock is not marked: a query over a playing replay's play-head recomputes each tick and is always one query behind, where marking it dimmed every pane it feeds once a second. In the test harness every event settles before the next line runs: the handler's writes, then every recompute they made stale, so a test reads the page a reader sees once the data is in.

**A write reaches every reader.** When a handler writes a table, every session whose page read a derivation over that table renders again, recomputing it off the render: a reader sees another reader's change without waiting for a refresh tick, and without a line of code to publish it. A route's write is the same: it lands as the route's handler ends — a handler that stops too, since a store without transactions kept what it was given — and the pages over the table render again. A write made outside the program — another service, a person at a SQL prompt — is seen through `fresh` and the ticks, as above.

**A route reads what the app holds.** A route has no session, so its handler reads, by name, the derivations the app holds — the one value every session reads too — and the cached rows a web service keeps in memory are a derivation:

```
use web, db

world_db = connect['postgres://app@127.0.0.1/hello_world']

derive worlds
    return query[world_db, 'SELECT id, randomnumber FROM world ORDER BY id']

fn cached_h[req]
    picked = for i in range[20]
        k = rand[0, len[worlds] - 1]
        map[id = worlds.id[k], randomNumber = worlds.randomnumber[k]]
    return json[picked]

fn update_h[req]
    conn = connect['postgres://app@127.0.0.1/hello_world']
    exec[conn, 'UPDATE world SET randomnumber = ? WHERE id = ?', [rand[1, 10000], rand[1, 10000]]]
    return text['ok']

route['/cached-queries', cached_h]
route['/update', update_h]
serve[8080]
```

A route's handler reads each derivation it names once, as its request begins — the app's value, computed then when there is none yet or something it reads moved — and reads that value wherever the body names it after, so a loop over the rows reads them where they lie; the same request reading it twice reads one value. A value is computed once for the requests that arrive together: a burst of requests after a write runs one query, and the rest wait for its answer. The rules above decide when it is computed again, with each request a moment of its own: a derivation over tables this program writes is kept until it writes one — `worlds` above until `/update` runs, whose write lands as its handler ends, so the next request reads the new rows — one over the clock, or over a table outside the program at `fresh = 0`, is computed for each request that reads it, and `fresh = N` keeps one over an outside table N seconds. A route that reads a derivation a session holds is refused, with what makes it the session's: `'band' is held by each session, and a route's handler has no session — it reads 'picked', which a session writes: …`. Both modes keep the app's values in the one runtime (`crow_web::live`) and read them by the same rule.

**Design Note: a route's cache is a derivation.** Considered: a cache of its own for routes — `cache[key, secs, compute]`, or a module-level value a route may write — as web frameworks offer beside their database layer, and an external cache (Redis) as part of the language. Rejected: a derivation already says what a value is computed from, so the runtime knows when it is stale — the part a cache by hand gets wrong (a key forgotten at a write, a value served after the row changed). A second form would hold the same rows with the invalidation written out, and a value a route writes would be state a route can move, which routes do not have. A store shared by several processes is a database the program reads, and comes as a driver: what it holds is a table a derivation reads. Decided too: a route's handler reads a derivation once, as its request begins, as a view does, where a page's handler reads it where it uses it — a route writes no fact, so nothing it does moves the value under it, and a loop over the rows reads one value instead of asking the cache each round.

**Design Note: no channels.** Considered: `channel.join[name]`, `channel.send[name, message]` and an `on_channel` handler, so that a route, a job or another process could push to the pages that joined a channel (ui-framework.md, Server Push). Rejected: a write already reaches every reader. The database is the channel, and a page learns of another's change the way it learns of its own, by reading a derivation over the table; a channel would be a second way to say that something changed, and a message pushed past the database could say what the database does not. A process outside the program is seen through `fresh`.

**Rows keep their identity through `id`.** The live client patches the page in place: an element with an `id` is matched to the element with that `id` in the previous render, so a row rendered from data — `p['`s.name`', id = 's-`s.id`']` — moves when its list is reordered and is dropped when its item is gone, while its neighbours are untouched. Elements without an `id` match by position. A node that survives a render keeps its focus, caret and scroll, so the field a reader is typing in is never rebuilt under them.

**The checker teaches the form.** A top-level variable assigned from a query and read by a view (a launch-time snapshot), and a query against a remote database inside a view, are errors in `crow check`; `d` in the check loop rewrites the first into `derive x` and hoists the second above its view as `derive rows[fresh = 60]`.

#### Tables over frames

**A big table pages.** `table[frame, page = 50]` shows fifty rows at a time, says which (`1–50 of 12000`) and offers the previous and the next page; the frame stays on the server and only the page shown is drawn, so a hundred thousand rows cost the reader one page. Each paged table keeps its place for its session: new rows arriving keep the reader on their page, and a frame that shrinks brings them to its last one. A table of more than 1,000 rows without `page` is refused, as a chart over its point budget is — in its place the view shows `table: 12000 rows — a table shows at most 1000 at a time: table[frame, page = 50]`, never a silent cut. A test sees the first page.

`table[frame]` renders a frame's rows. Numeric columns are typed: their cells carry the class `num` and the stylesheet right-aligns them with tabular figures. Six named arguments make it interactive and legible: `click = handler` sends the row's key to the handler when a row is clicked (`fn focus[symbol]`), `key = 'col'` names the column whose cell is sent (the first column by default; a numeric key travels as a number, a text key as a string), `mark = 'col'` names a column whose truthy cells put the class `marked` on their row, tinted by the stylesheet, `delta = ['pnl', 'pct']` names columns whose cells are colored by sign (`up` / `down`), `actions = map[Replay = replay, Delete = ask_delete]` adds a trailing cell of buttons to every row, each sending the row's key to its handler, and `hide = ['id']` keeps columns out of the table that stay in the frame — the key column, a helper. In the test harness `click[a, ask_delete[3]]` presses a row's button.

**A reader can sort a table.** `table[frame, sort = true]` makes each header a button: pressing it shows the rows in the order of its column, ascending, and pressing it again, descending. Numbers sort as numbers and text alphabetically; missing cells come last either way, and equal cells keep the frame's order. The session keeps each sorted table's order, as it keeps a paged table's page, and a sorted table that pages pages through the sorted rows. The frame itself is untouched: `sort[df, 'col']` orders the program's data, and the table's sort orders only what the reader sees. The sorted header carries `aria-sort` and an arrow. `sort` is written `true` or `false`, and a test sees the frame's own order.

**Design Note: no filter on a table.** Considered: `table[frame, find = true]`, a search box that keeps the rows holding the typed text in any shown column (ui-framework.md, Table Features). Rejected: filtering is the program's, over its frame — a field bound to a fact and a derivation that filters by it — which says which columns to search and how to match, where a table's own box would decide both for every table and be a second way to filter a frame. Sorting is the table's because its headers are: nothing in the program draws them.

#### Sliders

`slider[label, var, min = 0, max = 100, step = 1]` is a range input bound to a reactive variable, exactly as `field` binds a text input. A handler reads the variable when the reader acts (a `Seek` button beside the scrubber).

A bound input keeps its variable's type: text typed into a field bound to an integer becomes an integer (a float is truncated), a float variable gets a float, a bool variable gets a bool, and text that does not parse leaves a number alone. When a handler assigns to a bound variable, every field bound to it shows the new value on the next render; only the input the reader is typing in keeps its draft.

#### Handlers and their helpers

A view sees reactive state, a handler moves it, and a function a handler calls that assigns a module-level variable is a handler too — its own part's, or one of a part it uses (`session.picked = symbol` in the scanner's part) — it joins the handler class in both renderers, so `fn restart[]` may call `fn rewind_to[t]` and the write lands in state. So does one that acts for the session — says a `notice`, signs the reader in or out, hands a `download`, sends `mail` — itself or through a function it calls, so what it does waits for its handler's commit. A pure helper (one that assigns no module variable) keeps its own scope. A helper a handler calls sees what the handler has written so far, and the handler sees what the helper wrote when it returns, in both modes — derivations included: a derivation the handler reads after a write to what it reads, its own or a helper's, is the recomputed one (`clock.jump[t]`, then `clock.head` is `t`). A helper that reads state and assigns none is a derivation, not a function: `derive band` computes the price band from the scanner's config, and the handlers read `band`. A function reading a derivation is refused with that advice — by `crow run`, `crow test` and `crow check` alike, which pass every program through the same checks before doing anything else. Reactive state may be an array, a map, a record or a list of records, or a frame; the compiled program keeps each fact in a typed field of its own, which its handlers, helpers, views and derivations read and write in place — one copy, so a helper reads what its caller wrote — and writes a fact as JSON only where it leaves the program: the harness, a bound input, the dev panel, a session kept across a restart.

**A handler is a transaction.** It runs on a working copy of the session's facts, sees its own writes at once — as a helper it calls does — and when it ends its writes land as one; a render never sees a handler halfway, and two sessions never see each other's copies. Effects follow one rule, decided from the text: **an effect whose answer the handler uses runs when asked; one whose answer it does not use runs when the handler commits, in program order.** `r = query[…]`, `n = shell['wc -l x']` and `row = query[book, 'INSERT … RETURNING id']` answer at once; `notice['saved']` and `later[reload]` wait for the commit, and a stop drops them. A function's last line is its value, but a handler's value goes to nobody, so an effect on a handler's last line waits for the commit too, as does one a helper returns to a caller that calls it as a statement; a caller that reads the helper's answer (`r = notify[…]`) has it at once. Database writes run in a transaction: the first write a handler makes through a connection begins one on it, every later read and write on that connection is inside it, and it commits when the handler does and rolls back when it stops — `RETURNING` answers inside it. A handler that writes two databases commits them in program order; the language promises no atomicity across stores, and ClickHouse, which has no transactions, keeps a write the moment it is made.

**The notice.** `notice[text]` tells the reader something: an effect held to the commit, so a handler that stops never leaves a message saying it succeeded. The session keeps one notice, with the moment it landed; `notice` reads as a record with `text` and `at`, provided by the runtime like a derivation, so a view writes `toast[notice.text]`. A handler's stop is delivered the same way, as what a reader can act on — `skip_symbol stopped, and nothing changed — the server's log has why, as 3f9a1c07` — its reason in the server's log beside the id, never on the page (A handler stops, §10). A page that reads no `notice` gets the runtime's own toast, which fades after a few seconds; the program keeps no flag and clears nothing.

**Who the reader is.** `reader` is the page's signed-in reader: a map of strings, empty until someone signs in. A handler signs the reader in with `sign_in[record]` and out with `sign_out[]` — effects held to the handler's commit, like `notice`, so a handler that stops signs nobody in:

```
fn log_in[]
    found = query[book, 'SELECT id, role, hash FROM people WHERE name = ?', [name]]
    if len[found] eq 0
        notice['no one called `name`']
        return
    if hash[password, found.hash[0]] ne found.hash[0]
        notice['wrong password']
        return
    sign_in[map[id = str[found.id[0]], name = name, role = found.role[0]]]

fn log_out[]
    sign_out[]
```

Signing in sets `reader` for the session and gives the browser a cookie, so the reader is signed in on every page and tab they open next — after a restart too — for thirty days, or until they sign out. Signing out clears `reader` and the cookie, and ends that sign-in everywhere: every session holding it, another tab included, is signed out and redrawn at once, and the cookie is refused if it is ever sent again. A tab already open when someone signs in shows it the next time it opens the page. The program never sees the cookie: the runtime reads it when a page opens and fills `reader`. Nothing on the page can write `reader` — no input binds it and no handler assigns it; in a test, `change[a, 'reader', record]` stands for a reader who signed in. Authorization is ordinary code in the handler that needs it: `if reader.role ne 'admin'`. A signed-out reader has no `role`: reading it is an error, and a handler that meets one stops — nothing it did lands, and the reader is told it stopped — so the check fails closed (in a test, `click` reports the error); `'role' in reader` asks first. A derivation that reads `reader` is computed per session; a plain function is handed the reader as a parameter (`is_admin[reader]`), as it is handed a module variable. The record travels in the cookie, so it stays small: at most 1 KB — an id, a name, a role; look the rest up. The cookie is `HttpOnly`, `SameSite=Lax`, and `Secure` when the page came over https through a proxy (`X-Forwarded-Proto: https`). It is signed with the app's secret: `CROW_SECRET` (at least 32 characters) when set, otherwise a key the runtime writes to `.crow/keys/` on first use, a directory that ignores itself in version control. Changing the secret signs everyone out. Passwords are stored as `hash[password, 'password']` and checked with `hash[attempt, stored] eq stored` (### Hashing).

**A page for signed-in readers** checks `reader` in its view:

```
view orders[]
    if 'id' not in reader
        return sign_in_form[]
    return stack[
        table[open_orders, actions = map[Refund = refund]],
        button['Log out', click = log_out]
    ]
```

While the reader is signed out the page draws the sign-in form, and nothing else of the page reaches them: a session accepts only the events a render it sent offered, so `refund` is refused even in a message written by hand, and a derivation is computed only where it is read, so `open_orders` is never queried for them. When `log_in` commits, the same session draws the orders, at the same address, so nothing has to send the reader back. A page for one role checks the role the same way (`reader.role ne 'admin'`, after `'role' in reader`).

**Design Note: no `auth` on a page.** Considered: `page['/admin', admin, auth = true]`, sending a signed-out reader to a login page and back once they sign in (ui-framework.md, Authentication). Rejected: the view's check already does all of it and fails closed. It draws the sign-in form in place, offers none of the page's handlers and runs none of its queries, and a second way to guard a page would leave two places to look for why a page is shown. The page keeps its address while the reader signs in, so there is nowhere to send them back to.

**A handler runs off the page's loop.** While it runs the page stays live — refresh ticks render and landed values render, from the committed facts — and `running[h]`, true while a `later` schedule of `h` is in flight, is also true while `h` runs as a handler: every element that offers `h` renders `disabled` with `aria-busy="true"`, so a button shows it is working with no flag in the program. Events are applied one at a time, in arrival order, bound inputs included: a change that arrives while a handler runs waits its turn, and the browser keeps showing what the reader typed. A second click on the handler that is running is dropped, as `later` skips a schedule in flight. `later[f]` is the one way to say the session's other clicks need not wait for this work (Design Note: *Handlers and later*).

**Design Note: no debounce.** Considered: `debounce = ms` on an input or a button, holding its events in the browser until the reader pauses (ui-framework.md, Debounce). Rejected: what it guards against is already kept small where it happens. A derivation over a field's fact has one recompute in flight, and a value its inputs moved past is dropped for the latest, so a search typed a key at a time runs at most one query at a time, always for the latest text; a second click on a handler that is running is dropped; and the events that arrive while a render runs are answered by one render. A delay the program chose would slow every reader to spare a server the runtime already spares.

#### Views render wherever they are called

A `view` returns its markup from any call site — a `page[]`, the test harness, another view, or a program writing pages to disk (`doc.document[title, chapter[...]]`). **Text is text.** Markup is what a view, an element, a chart, an icon, a bound input, or `doc`'s `markdown`, `menu` and `pager` make; it stays markup however it travels — held in a local, collected by a `for`, put in a list, handed to another view as an argument. Every other string is text and is escaped, whatever it holds: a reader who types `<img src=x onerror=…>` into a field sees those characters on every page that shows them, never an image and never a script. A view given markup in one call and text in another is compiled once for each. Both renderers apply the same rule.

### Layout — the vocabulary

Pages are composed from layout primitives, never styled with CSS. Five containers join the element vocabulary inside views, and a fixed set of layout attributes applies to any element. Each has one written rule; the kernel table (`crow_runtime::layout`) turns it into the CSS a browser lays out with, and the layout engine — written in Crow, shipped inside the compiler — implements the same rule for tests and the desktop shell. Design and rationale: `ui-engine-design.md`.

| Container | Rule |
|---|---|
| `stack[a, b, ...]` | children top to bottom, each the container's full content width |
| `row[a, b, ...]` | children left to right on one line; overflow is a failure `fits[]` reports, never a silent wrap (`wrap = true` opts in) |
| `grid[a, b, ..., columns = 2]` | cells left-to-right, top-to-bottom, equal columns; `columns = '240px fill'` names tracks |
| `overlay[base, top]` | `top` positioned over `base` by `anchor = 'center' \| 'top_right' \| ...` |
| `scroll[child, height = '360px']` | a fixed-height viewport; the platform scrolls |

| Attribute | Values |
|---|---|
| `gap`, `pad` (`'m'` or `'m l'`), `margin` | the spacing scale `xs s m l xl` = 4 8 16 24 32 px |
| `align` | `start center end stretch` (cross axis) |
| `justify` | `start center end between around` (main axis) |
| `width`, `height`, `min_width`, `max_width` | `'240px'`, `'50%'` (width only), `'fill'` for remaining space; `aspect = '16/9'` |
| `size`, `weight`, `font` | the type scale `xs s m l xl xxl` = 12 14 16 20 24 32 px; `regular medium bold`; `font = 'mono'` sets the monospace face with tabular digits — a figure in a stat tile, a code-like value |
| `surface = 'card'`, `radius`, `border = true`, `shadow` | surfaces: radius `none s m l round`, shadow `s m l` |
| `color`, `background` | a colour token: `text muted accent up down surface bg` — never a literal colour |
| `lines = 2` | clamp a wrapping paragraph |
| `below = 'phone' \| 'tablet'`, `becomes = 'stack'`, `columns_then = 1`, `hidden_below` | responsive rules at named widths (phone < 600, tablet < 1024) — no media queries |
| `open = true`, `side = 'left' \| 'right'` | a disclosure's or layer's state (any bool expression) and a drawer's or dropdown's side |
| `tip = 'text'`, `fit = 'cover' \| 'contain'`, `active = '/path'` | a tooltip on anything; how an `img` fills its `aspect` box; the current page of a `nav` |

Layout attributes are literals; a wrong value is a compile error naming the scale (`gap = 'huge' is not a spacing step (xs s m l xl)`), the same in both modes. Attributes outside the vocabulary (`class`, `href`, `name`, ...) pass through as HTML. Labels, headings, buttons, and cells are single-line; only `p` wraps.

#### Revealing and framing

The vocabulary's own elements beyond the containers, each earning its place by a capability a stack of divs cannot compose:

| Element | Renders |
|---|---|
| `details[summary[...], ..., open = true]` | A disclosure: the summary always shows, the rest only when open. `open` follows a bool — `open = show` toggles it from state |
| `dropdown[summary[...], panel, side = 'right']` | A button that opens a floating panel — its second child, usually a `stack` of buttons or links. `side` anchors the panel to the button's right edge |
| `modal[child, open = var]` | A dialog layer over a dimmed page while `open` is true; the child (a `stack` with a heading, text, and a `button['Close', click = close, shortcut = 'escape']`) is the dialog |
| `drawer[..., open = var, side = 'left']` | A side panel layer, sliding from the right by default |
| `toast[..., open = var]` | A brief notice at the bottom corner while `open` is true; it fades on its own, and the program clears the flag |
| `divider[]` | A rule between siblings: horizontal in a `stack`, vertical in a `row` |
| `nav[a[...], ..., active = path]` | A navigation row; the link whose `href` equals `active` gets `aria-current="page"` and the stylesheet's active look |
| `header[...]` | The app's bar: a surface with a border holding its name, its `nav` and a status chip, laid out as a row (`gap`, `justify`, `align` as on any row) |
| `widget[name, props = value, on = map[event = handler]]` | A piece of the page the program's own JavaScript draws — a map, a rich editor, a calendar: the module `widgets/name.js` (below) |

Three attributes any element takes: `tip = 'text'` is the browser's tooltip, `fit = 'cover'` (or `'contain'`) sizes an `img` into its box together with `aspect`, and `open` as above. The layers take no room in the flow and are hidden until opened; the layout engine lays an open layer out as a stack where it sits.

**One way out of the vocabulary: `widget`.** The vocabulary is closed, so a map, a rich-text editor or a calendar is a widget: `widget['map', props = map[lat = 51.5, lng = -0.1], on = map[pick = choose]]` draws the module `widgets/map.js` of the program's folder — plain JavaScript that exports `mount(el, props, send)` and, optionally, `update(el, props)`. The page loads the module once, calls `mount` with the element and the props (the value, as JSON), and calls `update` with the same element whenever a render changes them; the element's contents are the module's, and no render touches them. `send('pick', value)` runs the handler `on` names for `pick` — `fn choose[place]` receives `value` — through the offers like any click: a page offers exactly the events its widgets name, and a handler takes a widget's value as it takes a field's, so it checks what it is given. A module name is letters, digits, `_` and `-`; the server serves only `widgets/*.js`. In a test a widget is its element, and `click[a, choose['Paris']]` stands for what it would send. There is no other client code.

**The icon set**, closed like the rest of the vocabulary: `check x plus minus chevron_down chevron_up chevron_left chevron_right arrow_up arrow_down arrow_left arrow_right search menu more settings user home calendar clock refresh play pause stop upload download trash edit copy link external info warning error success star filter sort chart table bell mail lock eye eye_off dollar bookmark flag tag circle dot loader`. An unknown name is an error that lists them. Icons are inline SVG in the current text color, so `color = 'muted'` and the theme both apply.

#### Charts — `chart[frame, ...]`

A chart is a view over a frame. `chart` is an element like `table`: valid only inside views, it takes a frame plus named arguments that say which columns to draw, and renders server-side as SVG:

```
view render[]
    bars = query[conn, 'SELECT ts, o, hi, lo, c, vol FROM bars ORDER BY ts']
    return div[
        chart[bars, candles = ['o', 'hi', 'lo', 'c'], bars_y = 'vol',
              marker = open_bell, tz = 'America/New_York', height = 200]
    ]
```

| Named argument | Default | Meaning |
|---|---|---|
| `x` | `'ts'` | Column of moments for the x axis |
| `candles` | `[]` | Four column names — open, high, low, close — drawn as candlesticks |
| `lines` | `[]` | Column names drawn as line series |
| `bars_y` | `''` | Column drawn as a volume pane under the chart |
| `marker` | `0` | A moment marked with a vertical play-head |
| `tz` | `'UTC'` | Timezone for the axis ticks |
| `height` | `320` | Height in pixels |
| `panes` | `[]` | Lists of column names, each list drawn as its own strip under the volume with its own scale: `[['rsi'], ['cvd']]` |
| `levels` | `0` | Reference lines keyed by column, drawn dashed in the pane that holds the column: `map[rsi = [30, 70], cvd = [0]]` |
| `points` | `0` | A frame of markers on the main pane — columns `ts`, `y`, and optionally `label` and `kind` (`'buy'` an up triangle in the up color, `'sell'` a down triangle in the down color, anything else a dot) |
| `seek` | `0` | A handler: every bucket becomes a click target and the handler receives the bucket's moment — `seek = jump` with `fn jump[t]` |

Colors come from the style tokens (`--crow-up`, `--crow-down`, `--crow-accent`, and `--crow-series-2` … `--crow-series-6` for the second line onward, counted across the whole chart), so every chart is theme-correct with no color plumbing. Every line series is named in a legend at the top of its pane. The indicators a pane usually draws come from `use chart` (`### chart`). An empty frame renders a placeholder rather than an error; a frame over the point budget (1000 points) refuses with an error that says to aggregate first — coarser rows from the query, or `head[frame, n]` — never a silent downsample. The geometry and the SVG assembly live in `crow_runtime::chart`, so both modes emit identical markup. Design and roadmap: `chart-module-design.md`.

#### Styling — the default stylesheet

Pages look right with zero CSS: every page shell carries a default stylesheet (light and dark, following the system theme). The whole styling vocabulary is seven classes and eight tokens:

```
classes   card  stats  controls  primary  up  down  stale
tokens    --crow-bg  --crow-surface  --crow-text  --crow-muted
          --crow-border  --crow-accent  --crow-up  --crow-down
```

Use the classes as ordinary attributes — `div[..., class = 'card']`. To restyle, put plain CSS in `css/app.css` in the project; it is inlined after the defaults, and overriding a token (`:root { --crow-accent: #c60; }`) recolors every element and chart that uses it. `crow check` flags unknown `--crow-*` names with a did-you-mean. CSS never appears inside `.co` files. Design: `style-module-design.md`. Where this is going — a closed layout vocabulary with an engine written in Crow, so tests can state where things are and what colour they have without a browser — is `ui-engine-design.md`.

#### Accessibility

A page works for a reader on a keyboard or with a screen reader without a line of the program saying so, the same in both modes:

- A field's label names its input (`aria-label`), as a checkbox's and a radio's label wraps its input. A `select` is named by a `label[...]` around it: `label['Symbol', select[symbols, sym]]`.
- An element with a `click` that the browser does not press itself — a `div`, a card, a table row — is a tab stop with a button's role (a row keeps its own), and Enter and Space press it.
- A `modal` is a dialog (`role="dialog"`, `aria-modal`): when it opens, focus moves into it and Tab stays inside, and when it closes, focus goes back where it was. The runtime's question behaves the same way. A `drawer` is a dialog that does not hold focus, and a `toast` is a status a screen reader announces, as the runtime's own notice is.
- A chart is an image with a name that says what it draws (`a chart of candles, vwap and volume, 390 points`); an icon is hidden from screen readers, since words go beside it.
- While the page is reconnecting it takes no input from the mouse or the keyboard, and a screen reader is told.
- A key presses only what a click could (see Keys press elements), and a shortcut is announced (`aria-keyshortcuts`).
- An `img` says what it shows, `alt = 'the logo'`, or that it only decorates, `alt = ''`; one that says neither is refused before the program runs, in both modes.
- The default stylesheet's text meets WCAG AA contrast (4.5:1) in both themes, focus is always visible, and a drawer's motion stops for a reader who asks for less (`prefers-reduced-motion`). The page's language is English (`lang="en"`).

What stays the program's: headings in order, a `main` for the page's content, and words on every button — a button that shows only an icon names itself with `tip`.

#### Core (Server)

```
use web, ui
use views.home

serve[3000]
page['/', home.render]
```

Crow manages the application instance implicitly — no `app` variable needed. `page[path, view]` registers a LiveView page rendered by a view; event handlers are bound inside the view (`click = inc`), so there is no separate handler argument. `serve[port]` starts the server (`serve[port, refresh = N, budget = M]` adds periodic re-rendering and a latency budget — see Background Work). `page[path, view, url = ['day', 'sym']]` puts those facts in the page's address: a session starts from the values in the query string (`/?day=2026-09-04&sym=IMRN`), each read in its fact's shape, and the address follows them as they change (`history.replaceState`), so a reload or a shared link opens the same view — a fact of a part is named by its path, `url = ['session.day']`, and appears by its last name. A `:name` segment of a page's path is a fact in its address too: `page['/card/:card', card_page]` opens a session with the fact `card` read from the path (`/card/7` is `7`, in the fact's shape; a value the fact cannot hold, `/card/x`, is no page — a 404, where a query value that does not fit is passed over), and the address follows the fact as it changes (`/card/8`) — the path holds the facts that say which thing the page shows, the query string (`url = […]`) the facts that tune how. A path's name, like a `url` name, must name a fact (a part's by its last name, `:day`), or the program is refused before it runs, in both modes. A program serves every page it registers, and a page is a session of its own: a link to another page (`a['Card 7', href = '/card/7']`, or a `nav` of them) opens that page's session, and what pages share is the database, the signed-in reader, and the address. (`crow run -i` had served only the last page a program registered.) The look of the whole app is set there too: `serve[port, theme = 'dark', density = 'compact', accent = '#3b82f6', wide = true]` — `theme` is `'light'`, `'dark'`, or `'system'` (the default, following the reader's setting), `density = 'compact'` tightens the element defaults for dense screens, `accent` overrides the accent token with a hex colour, and `wide = true` lets the page use the full window instead of the reading column. A wrong value is a startup error naming the choices. The order in source doesn't matter: the server starts only after every route is registered.

If no routes are registered, `serve[]` shows a default welcome page so the user always sees something; once a route or a page is, an address none of them answers is a 404, `no page at /cards` (the welcome page had answered it).

If the port is already in use, the server prints a friendly error: `error: port 3000 is already in use`.

| Function | Description |
|---|---|
| `serve[port]` | Start the server on the given port (implicit app) |
| `page[path, view]` | Add a LiveView page rendered by `view` |
| `route[path, handler]` | Add a route; `handler` is a named `fn handler[req]` that receives the request and returns a response (implicit app; `route[path, method, handler]` for other methods; `limit = 60` answers each client at most 60 times a minute) |

A route's handler is handed the request, and nothing else: it is a function, so no module variable is visible inside it (Function Scope), and no parameter can carry one in, since the route calls it with the request alone. What it needs it opens itself — `conn = connect[env['DATABASE_URL']]` — and every `connect` to one URL in a program is one handle, so the handlers that connect share one pool (### db). The request's `req.method`, `req.path` and `req.body` are text, and `req.headers` (their names in lower case), `req.query` and `req.params` are maps of text, so `/hello/:name` reads its name as `req.params.name` and `/find?q=crow` its query as `req.query.q` — decoded, so `?q=hello%20world` is `hello world` (a query had reached the program still encoded). A form's fields are `req.form`, a map of text, from an urlencoded body or a multipart one, and a multipart body's files are `req.files`, a list of maps of text as a page's picked file is — `field` (the form's name for it), `name`, `type`, `size` and `path`, where the server keeps a copy under `.crow/uploads/` while the handler runs and removes it once the handler has answered (`cp[f.path, …]` keeps one). A body bigger than the app takes — 10 MB, or `serve[…, uploads = N]` megabytes, the limit a page's uploads keep — is answered `413 Payload Too Large` before the handler sees it (a body had been read whatever its size), and a multipart body that does not parse `400 Bad Request`.

`serve[port, cors = ['https://app.example.com'], files = 'public']` sets the server's options. `cors` names the sites whose pages may call the app — `'*'` any — and answers their preflights; a page on any other site is told nothing, so its browser refuses the answer. `files` is a directory whose files answer the addresses no route or page does — `/logo.png` is `public/logo.png`, `/` its `index.html` — and a file it does not hold is the app's 404, as is one whose name, or a directory's on its path, starts with a dot: `.env`, `.git/`, `.crow/`, however the address spells the dot (`files = '.'` had handed out `.crow/keys/`, whose key seals the sign-in cookies); `.well-known`, where a certificate authority looks, is served. A route's `limit = n` answers each client at most n times a minute, a client being its address (a proxy's on this machine, the first address its `X-Forwarded-For` names); a request over it is `429 Too Many Requests`, its `Retry-After` the seconds to wait. All three are the server's, the same in both modes. A route that names a method answers only it, and another method at its path is `405 Method Not Allowed`. Both modes serve routes and make every response below through the same functions, so they answer alike, to the byte of a `msgpack` or an `arrow` body. A handler whose value fails — `int[req.query.n]` of a query that is no number, a frame that did not load — answers `500` and an id, `the request could not be answered — the server's log has why, as 3f9a1c07`, and the server's log names the handler and the reason beside it, `crow: flow stopped [3f9a1c07]: cannot convert 'x' to int`; so does one that stops (`boom stopped [91c0d2e4]: line 5: index 9 is out of range for an array of 2 items`). The client is told no more, since a reason names what the program reaches — a database's address, a file's path (it had been told the reason; Vijay, 2026-10-01) — in both modes, and the server goes on. A response HTTP cannot carry — a status outside 100 to 999, a header whose value holds a line break or another control character, as one a handler built from what a request sent may — answers `500` and an id as a failed handler does, the log saying why beside it (`a response could not be sent [5e07aa1b]: the header 'x-name' holds a line break or another control character`); text beyond ASCII in a header's value is sent as its bytes. (It had answered `internal error`, saying nothing, and refused text beyond ASCII the same way.) A route's handler that cannot wait on anything — no query, no read, no log, no loop, nothing it calls that does one — is answered on the server's own thread as its request is read (a hop to another thread and back had cost more than answering `text['Hello, World!']`); any other runs off the server's threads, as a page's does, so a slow one delays only its own request, on at most four threads a core (64 at least) at once: a request past them waits its turn, as one past a database's pool waits for a connection. Both modes decide which handlers cannot wait by the same rule. A route and its responses are web's: a program that calls one without `use web` is refused before it runs. (The interpreter had served no route at all, and then none of `msgpack`, `arrow` and `stream`; a compiled route that read a param or named a method had not built, nor one whose handler's value can fail, and the compiled `stream` had not built either; a compiled handler that stopped had dropped the connection, saying nothing.)

#### Responses

| Function | Description |
|---|---|
| `text[s]` | Plain text response |
| `html[s]` | HTML response |
| `json[map]` | JSON response from a map; from a record, an object of its fields; from a list, an array — `json[rows]` of a list of maps or of records answers an array of objects. Each value is written as `encode` writes it: a value that failed is `null`, an int past 64 bits the text of its digits |
| `json[frame]` | JSON response from a frame: its records, a list of objects (a missing cell `null`, a moment its ISO text) — what a page's `fetch` of the route reads back as a frame |
| `respond[status, body]` | Response with custom status code |
| `redirect[url]` | HTTP redirect response |
| `msgpack[value]` | MessagePack binary response: the value as JSON holds it, packed (`application/msgpack`) — written in Crow (`src/modules/web.co`), each piece in its smallest form |
| `arrow[frame]` | The frame as an Arrow IPC stream (`application/vnd.apache.arrow.stream`), which pandas, polars and every Arrow reader take |
| `bytes[data]` | Raw binary response (`application/octet-stream`) |
| `stream[feed]` | A text response sent as `feed` puts it, chunk by chunk; `stream[feed, a, …]` hands `feed` the rest |

**A stream is sent as its feed puts it.** `stream[feed, a, …]` answers at once, and when the response is sent it calls `feed[put, a, …]` on a thread of its own. Each `put[text]` sends that chunk to the reader as it is put, and answers whether the reader still listens: `false` once they have gone, so a long feed can stop early. The arguments after the feed are the values they have when the handler answers, which is how a feed reads the request. A feed puts text — a value is written into text first, ``put['`row`\n']`` — and a `put` handed anything else is refused before the program runs; the chunks go out in the order they are put, a loop's included. A feed that fails or stops ends the body where it is, and the log says why. The body is `text/plain`, sent chunked.

```
use web

fn numbered[put, n]
    for i in range[n]
        if not put['line `i`\n']
            return

fn flow[req]
    return stream[numbered, int[req.query.n]]

route['/flow', flow]
serve[3000]
```

**Design Note: a stream's feed.** Considered: a feed that returns a list the runtime sends (it holds the whole body, which a stream exists not to); a channel value a handler writes to (a new kind of value for one builtin); `send[out, chunk]` on a handle (WebSocket's `send` would name two things); and chunks of any value, sent as JSON lines. Decided: the feed is a named function, as Crow has no lambdas, handed `put`, a function value the program already knows how to call; `stream[feed, a, …]` hands it what it needs from the request, as `later[f, a, …]` does. Text chunks keep one way to write a value (interpolation), and `put` answering whether the reader listens lets a feed stop with the `return` it already has.

A response is what a route returns. A page shows elements and text, so a view that places one — `row[text['3 items']]` — is refused in both modes: place the text itself, or `p[…]` or `span[…]`.

#### Auth (`use web.auth`)

| Function | Description |
|---|---|
| `sign[claims, secret]` | A token — a JSON Web Token signed with HS256 — carrying the claims (a map, or a record), issued now (`iat`) and expiring in an hour (`exp`) unless the claims say when |
| `verify[token, secret]` | The claims a token carries, as a parsed value (`str[claims.user]`), when it was signed with this secret and has not expired |

A token is how an API, which has no page session, says who calls it: `sign` gives one to a client that signed in, and each request carries it back for `verify` to read. A refusal is an error value of kind `'auth'`, its reason saying why: `the token has expired`, `the token's signature does not match`, `not a token`. A secret shorter than 32 characters signs nothing (`a token's secret is 32 characters or more`), as the sign-in cookie's `CROW_SECRET` keeps the same floor; keep it out of the source, in the environment (`env['TOKEN_SECRET']`). A page keeps `sign_in`, which needs no token. Both modes sign and verify through the same functions. (Neither had built them: the compiled program named a type the web runtime did not have, and the interpreter had no such builtins.) They are written in Crow (`src/modules/auth.co`) on the `sys` layer: the header and the claims as base64url, the signature an HMAC-SHA256 of the two, compared in constant time; the claims reach `sign` as the JSON text they encode to (Design Note "a builtin written in Crow keeps its row"). Their tests are the module's own (`crow test src/modules/auth.co`), one of them a token another library signed, read with its secret.

#### WebSocket (`use web.ws`)

| Function | Description |
|---|---|
| `socket[url]` | A connection to a WebSocket server, `ws://` or `wss://` |
| `send[s, text]` | Send a text message; answers empty text |
| `recv[s]` / `recv[s, secs]` | The next message, as text — waiting for it, or at most `secs` seconds |
| `close[s]` | Close the connection, for every value that holds it |

A socket is a live feed a program reads — a market's trades, a service's events — in a script, a job, or a handler's background work. Each failure is the call's error value, of kind `'socket'`: a server that does not answer is `socket`'s (`cannot open a socket to …`), a wait that passes its seconds is `recv`'s (`no message in 5 s`), and a server that closed answers `closed`, after which every call says `the socket is closed`. A binary message reads as text, and a ping is answered and passed over. A socket is one connection however many values hold it — a function handed it, a loop — and `close` closes it for all of them. Both modes talk through the same client, which is written in Crow (`src/modules/ws.co`) on the `sys` layer: the handshake is the protocol's (`sys.upgrade`), and the framing — a message sent masked, the next read whole across its fragments, a ping answered, a close answered and the connection let go — a formula over `send`, `recv`, `pack` and `mask`. Its rows keep the contract above in both modes; its tests are the module's own (`crow test src/modules/ws.co`, the RFC's masked example among them), and `tests/socket.rs` runs it against a server in both modes.

```
use web, web.ws

feed = socket['wss://feed.example.com/stocks']
send[feed, '{"action": "subscribe", "params": "T.AAPL"}']
while true
    m = recv[feed, 30]
    if not m.ok
        log['the feed stopped: `m.reason`']
        break
    log[m]
```

**Design Note: a socket's name.** Considered: `connect[url]`, the client's first name, which the database's `connect` already is — one name two ways, told apart by the URL's scheme; and `dial`, `open` and `ws`, which name a mechanism, a verb many things do, or the protocol's initials. Decided: `socket`, the thing a program holds, as `smtp` names a mailer and `connect` a database; `send`, `recv` and `close` act on it.

#### LiveView / Pages

| Function | Description |
|---|---|
| `page[path, view]` | Add a LiveView page (server-rendered with live updates via WebSocket); `:name` in `path` is a fact in its address (Core, above) |
| `download[value, name]` | Hand the reader's browser a file when the handler commits: a frame as CSV, a string as text, a picked file's bytes |
| `title[text]` | The page's title, which the browser's tab and history show: a view places it anywhere in the page, reading the page's state like any element (`title['Card `card`']`); a render that changes it changes the tab |
| `meta[name = 'description', content = text]` | A tag in the page's head: its attributes pass through as HTML (`property = 'og:title'` for a link preview) |

**A page's head is what its render places.** `title` and `meta` take no room on the page: a served page carries them in its head from its first render, and a later render that changes them changes the head — the last `title` the render places is the page's (a layout's placed before the page's own gives way to it), and a page that places none is titled `Crow App`. In a test they are elements of the render, where `elements[a, 'title']` finds them; the layout engine gives them no box.

**Design Note: a page's address and its head.** Considered: a path's names as the view's parameters (`view card[id]`), as Express and Rails hand a route its parameters; and a page's title as an argument of `page` (`page['/card/:card', card, title = 'Card']`). Decided: a path's name is a fact. A page's state is its facts and the address already carried facts (`url = […]`), so one rule holds the path and the query string, a handler that changes the fact moves the address, and nothing new reaches a view. A title is an element the render places, so it reads the page's state (a card's own name) and changes with it, where an argument of `page` would be text fixed when the page is registered. A route keeps `req.params`: a route's handler is handed its request, and a page is handed none.

**A page and its session.** The page request renders the reader's session once, and the page's socket adopts that session — its facts, its derivations and the render already on screen — so a page loads with one render. **A session outlives its connection.** When the socket closes — a laptop sleeps, the network drops, a rebuild restarts nothing but the page — the server keeps the session for ten minutes, and the page, which holds the session's token for its tab, resumes it on reconnect: the same facts, a replay still where it was, sent its render at once. A waiting session keeps its facts and its derivations' last values, so it comes back without a flash of loading; at most a thousand sessions wait at once. The token is a credential and is treated as one: it travels only over the socket, it is good once, and every resume replaces it, so a copied token resumes nothing after its tab has. Reloading the tab resumes it too, and so does coming back to it with the browser's back button; a second tab is a session of its own. Following a link opens the session of the address it names, even to the page the tab is on: `/?day=2` from `/?day=1` starts from `day = 2` (the tab's session had been resumed, and the address followed it back to `day = 1`). A socket that brings no live token — a session past its ten minutes, a server that restarted — starts a fresh session from the facts its address carries and is sent its render at once: the page never keeps showing a session the server no longer holds. `serve[…, keep = true]` keeps sessions across a restart as well: each session's facts are written to `.crow/sessions/` as they change, and a tab that comes back to a restarted program finds its session there for the same ten minutes (a kept session had been removed before it was read, and the tab started afresh); a fact whose name or type changed since starts from its declared value, which the log says. The directory ignores itself in version control. While the socket is down the page dims, says *reconnecting…* and takes no input, so a click is never silently lost. Events that arrive while a render runs are applied together and answered by one render: a slider dragged across its range is one render, not one per step. A render sends what changed, not the page: the server reads the render as the browser builds it — a chart's marks and a table's rows included — compares it with the render the page holds, matching elements by `id` and then by position as the page does, and sends the changes (a text, an attribute, an element added, gone or replaced) when they are smaller than the page, the whole page when they are not. A moving play-head on a chart is a few attributes, not the drawing again, and a row added to a list whose items carry ids is one row. Each change names the element it expects to find; a page that meets one it cannot place — its shape changed under it — asks for the whole page back, and a field being typed in keeps the reader's draft. Markup the browser would have to repair (a block inside a paragraph, a row outside a table, a tag closed by its parent's end tag) is sent as whole pages, and the server log says which repair, once. Handlers and renders run on worker threads, never on the threads that carry the sockets, so a session's slow work delays only its own page. A clicked element is marked `data-crow-busy` the moment it is clicked, before the server answers, and the mark clears when the answer to that click arrives — the reader sees the click taken in the same frame, with no program code in the browser.

**A page sends only what it was offered.** A session accepts a click only when a render it sent carried that handler with exactly those arguments (`click = set_speed[10.0]` offers `set_speed` with `10.0`, a table row its key), a bound input or select only when a render drew it, and a form only when a render drew it. A helper no element names, a fact no input binds, and the button of a closed modal or a hidden element are refused — a confirmation's second step is accepted only after its first opened it. A refusal changes nothing and is logged: `crow live: refused 'name' — the page did not offer it`. The socket opens only from the app's own page (a page on another site is refused before it can send anything), a message on it is at most 4 MB — an event is a click or a field's text, and a file comes by a request of its own, under the uploads limit — so a bigger one closes the socket, which the page opens again (the bound had been 64 MB), and a served app listens on this machine only; set `CROW_HOST=0.0.0.0` where the network must reach it — a container, a server behind a proxy.

Both modes serve pages. `crow run file.co` compiles and serves; `crow run -i file.co` serves from the interpreter at once, with no Rust compile between an edit and the page — the live runtime keeps the sessions (their facts, the commit, the notice, the questions, the address, the reconnects) in both, and only who renders the views and runs the handlers differs. One difference is of timing, not of meaning: the interpreter computes a derivation where it is read, so a page served by it waits for its queries, where the compiled page shows the last value and catches up. `crow test --served` drives every page in `tests/served/`, and every app under `examples/` with a `main.events` script beside its `main.co` (served from a fresh copy, so its database starts empty in each mode), with the same events in both modes and compares the pages they settle on.

#### Mail (`use web.mail`)

| Function | Description |
|---|---|
| `smtp[host, port, user, pass]` | Create an SMTP mailer; its messages come from `user` |
| `mail[mailer, to, subject, body]` | Send a plain-text message: held to a handler's commit, so a handler that stops sends nothing; `r = mail[…]` sends at once and `r.ok` says whether the server took it |

`mail` is an effect like `notice` and `download`: in a handler, or a function a handler calls, a message whose answer is not used waits for the commit, and a stop drops it; used (`sent = mail[…]`), it is sent when asked, and a failure is the answer's reason. Outside a handler it is sent at once. A view or a derivation sends none, itself or through a function it calls, and is refused if it tries: it runs whenever the page needs it. A mailer is made once for the whole program, as a connection is, and prints as its host, never its password. In a test nothing is sent: `mails[a]` is the list of what the session sent, each a map of `to`, `subject` and `body`, and `crow test --compiled` records the compiled program's the same way.

The sending is written in Crow (`src/modules/mail.co`) on the `sys` layer, as RFC 5321 says: the greeting, `EHLO`, the connection secured with STARTTLS where the server offers it (port 465 is TLS from the start; a password travels only encrypted, so a remote server that offers neither is refused — a local relay on the loopback address is not), `AUTH PLAIN`, the envelope, the message's head (`From`, `To`, a `Subject` beyond ASCII as an encoded word, `Date`) and its body with its dots stuffed (a body beyond ASCII as base64), `QUIT`. A recipient the server refuses, an address that is none (`name@host`), a server that does not answer or answers anything but what the step wants are the reasons (`mail to bob@example.com failed: the server answered 550 no such user at RCPT TO`). The session layer holds a handler's message to its commit in both modes and hands it to the row's function to send; a script sends it at once. Its tests are the module's own (`crow test src/modules/mail.co`), and `tests/smtp_server.rs` runs it against a server in both modes.

#### Client (`use web.client`)

A program reads a URL with `read` and `load` (Files, URLs and S3), sends any other request with `request` (Requests), and a page pulls remote data with `fetch[url]` / `poll[url, seconds]` (reactive, see above). `web.client` is no module: the client is the language's own, written in Crow.


**Design Note: one process.** Considered: sessions any process can resume from a shared store, and a table's version shared across processes, so that an app could run on several machines as a Phoenix cluster does. Decided (session-layer-design.md §6; measured 2026-10-02): an app is one process — its sessions, its derivations' values and its tables' versions live in it — and how far one goes is measured, not guessed: a connected tab costs about 140 KB while it is open, a parked session a few kilobytes, and a trivial route answers tens of thousands of requests a second on a laptop (the book, Polish and Deployment, "How far one process goes"; `tests/capacity.rs` measures it on any machine). An app that outgrows one process runs several behind a proxy that keeps each tab on its process, sharing the database; a write one process makes reaches another's derivations by their `fresh` window, as an external source's does. Sessions across processes wait for an app that needs them.

**Design Note: what web is.** Considered: the modules the web framework had gathered — a task queue (`web.queue`, `push`), templates (`templates`, `render`), an embedded SQLite handle (`web.db`'s `sqlite`), session stores (`sessions[]`), Server-Sent Events and WebSocket servers (`sse`, `ws`), an LRU cache (`cache`), generated API docs (`web.docs`), and CORS, static files, rate limits and uploads as calls on an `app` value the language no longer has (`cors[app]`, `static_dir[app, …]`, `limit[max, window]`, `uploads[req]`). Decided (Vijay, 2026-09-30): each was a second way to do what the language already does, or a stand-in the interpreter answered with a placeholder (`render` gave `[rendered: name]`), and each is gone. Background work is `later[f, …]`; a page is a view, not a template; a database is `db.connect`; a reader's session is a page's session, with `sign_in` for who they are; a server reaches a page through its live session and a client through `stream`; what a program computes once and reads often is a derivation. `use web` serves pages and routes, and its modules are `web.auth`, `web.mail` and `web.ws`; another is refused by name.

### db

The `db` module provides database connectivity. Import with `use db`. Supports SQLite, PostgreSQL, and ClickHouse via URL-scheme dispatch. See `docs/db-module.md` for full architecture details.

#### Connection

```
use db

--- SQLite
conn = connect['sqlite://data.db']

--- PostgreSQL (default pool: 10 connections)
conn = connect['postgres://user:pass@host/mydb']

--- ClickHouse (its HTTP interface: port 8123 when none is given)
conn = connect['clickhouse://default@localhost:8123/default']

--- Custom pool size
conn = connect['postgres://user:pass@host/mydb', 20]
```

| Function | Description |
|---|---|
| `connect[url]` | Open a database connection (pool) with default pool size, checking its database holds the tables the program declares on it (The schema a program declares, below) |
| `connect[url, pool_size]` | Open a database connection (pool) with custom pool size; a pool size of 0 is the default |
| `describe[conn, sql]` | The columns a statement answers, without running it: a frame of `name` and `kind` (`int`, `float`, `str`, `bool`, `datetime`, with `?` after a kind where the column can hold NULL); a `?` placeholder stands in as NULL. What the compiler asks at build time (Typed cells, below), asked by the program |

**The ClickHouse driver is written in Crow** (`src/modules/clickhouse.co`, design rule 20, docs/db-in-crow-design.md), over its HTTP interface: `clickhouse://user:pass@host:port/database` names that interface, port 8123 when none is given; the native protocol's port, 9000, is no longer spoken, and a URL that names it is read as the HTTP port, which the log says once. Each statement is sent with an id (`crow-<machine>-<process>-<draw>`), its rows come back as RowBinary and are decoded by the `frame_of_rowbinary` kernel, typed as the server names each column; an insert goes as RowBinary encoded by the table's types; a statement a stop abandoned is killed by its id (tests/query_cancel.rs); at connect, the statements a dead run of this machine left running are swept. The server says how long it keeps a connection open (its `Keep-Alive` header), and the driver notes it: an idle connection past that is let go before the server closes it, not found closed under the next statement. The handle, its pool of kept connections (four, since ClickHouse runs one query on every core already), the reconnect and the cancel are `crow_db`'s, which calls the driver through a table of calls the program registers (the compiled program's generated functions; the interpreter's child over the program's functions). A program never imports the driver: `use db` carries the drivers its connection URLs name.

**The Postgres driver is written in Crow too** (`src/modules/postgres.co`), speaking the wire protocol over `sys`: a connection started and authenticated — trust, a clear password, md5, or SCRAM-SHA-256 as RFC 7677 says, its proof held to the RFC's own example in the module's tests — and its backend's process and key noted for a cancel; a statement without parameters as a simple query, one with parameters through the extended protocol, each parameter typed by its kind (text, int8, float8, bool; a cast like `?::date` is a cast from text) and bound as text; the answer decoded by the `frame_of_postgres` kernel, typed as the server names each column; a command's count from its completion; `insert` as `COPY … FROM STDIN` in the text format (`copy_of_frame`); `describe` from Parse and Describe without an Execute; a statement a stop abandoned cancelled by a CancelRequest on a connection of its own. tokio-postgres and deadpool are gone; the pool (ten), a handler's transaction on one connection, `transact`, the advisory lock around reconciling a schema and the reconnect are `crow_db`'s.

A connection is not the program's lifetime. When the server closes it — a restart drops every one — the next query reopens it and runs, and the server log says so; **a program told to stop stops the statements it had in flight on their servers** (ClickHouse by the statement's id, Postgres by a CancelRequest) and waits for that, three seconds at most, before it ends: a restarted app does not leave its predecessor's queries running; Postgres and ClickHouse connections are pooled, so queries from different sessions run side by side, and sqlite is a file. Every `connect` to one Postgres or ClickHouse URL in a program is one handle: parts that each connect to the same server share its pool.

**A query nobody waits for is stopped.** A background recompute whose session has gone, or whose inputs moved while it ran, stops its query on the server (ClickHouse and Postgres); so does every query in flight when a served program is told to stop — `crow stop`, a rebuild, Ctrl-C. A program killed outright cannot stop anything; the next run that connects to a ClickHouse server on the same machine stops the queries the dead run left behind, and the log says so. Each ClickHouse statement carries a leading comment naming the machine, the process and a number, which is how it is found again.

#### Query & Execute

| Function | Description |
|---|---|
| `query[conn, sql]` | Execute a SELECT and return results as a frame |
| `exec[conn, sql]` | Execute DDL/DML (CREATE, INSERT, UPDATE, DELETE), return affected row count |
| `exec[conn, sql, params]` | Execute parameterized DDL/DML with an array of bind values |

```
use db
conn = connect['postgres://user:pass@host/mydb']

--- Query returns a frame
users = query[conn, 'SELECT * FROM users']
log[len[users]]
filtered = filter[users, users.age gt 25]

--- Execute returns affected row count
exec[conn, 'CREATE TABLE users (id INT, name TEXT)']
n = exec[conn, 'DELETE FROM users WHERE age < ?', [18]]
log[n]
```

**List parameters.** An array bound as a parameter stands for a list: its `?` becomes `(?, ?, …)` with one placeholder per element, on every backend. Write `WHERE id IN ?` and pass `[[1, 3]]`; an empty list matches nothing. A list goes after `IN` and nowhere else: handed to another `?` — `x = ANY(?)` — it would be sent as a row where the database wants an array, so a literal statement that does so is refused, in both modes, before it runs. The compiler asks a database about `IN ?` as the list of one it is sent as, `IN (?)`.

**Typed cells in compiled programs.** The compiler asks the program's databases what each literal query returns — a `prepare` on Postgres, `DESCRIBE` on ClickHouse — and types every column: `rows.price[0]` is a float, `rows.n[0]` an int, in the compiled program as it always was in the interpreter. A column that can hold NULL has fallible cells, in both modes: a NULL one is an error value of reason `missing` (see frame). The compiler reads which can from the statement and the tables' declarations — a column read from a table can hold NULL unless it is declared `NOT NULL` (or is SQLite's `INTEGER PRIMARY KEY`); every column of the side of an outer join a row can have no match on can; an expression can where what it reads can, except `count`, `coalesce` with an argument that cannot, `IS NULL` and a literal, and an aggregate over no rows (no `GROUP BY`) can; a function it does not know can; and a column the `WHERE` clause compares or asks `IS NOT NULL` of cannot, in the rows it keeps. ClickHouse says it itself, by a `Nullable` type. A statement it cannot read that way can hold NULL in every column. Connections bound to a literal URL (`pg = db.connect['postgres://…']`) are asked in declaration order; `?` placeholders and interpolated spans stand in as NULL. A database that cannot be reached at compile time is skipped. What each database answers is kept in the project's `.crow/schema` — a file per statement and the connection it was asked on, holding the statement and its columns' types, a column that can hold NULL marked `?` (`float?`) — and when no database answers a statement its cells are typed from that record, and the build says so once; with no record they are dynamic, as below. A project may keep `.crow/schema` in version control, so a machine without the databases — CI, a laptop without the data — builds the program typed as it was. The frames the standard modules build are typed as they are built: `doc.chapters`' `title`, `path` and `part` are text and its `depth` an int, and `chart.macd`'s columns are floats. Frames derived through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack`, and `join` keep their column types; `group` gives its key columns as they were and a float per aggregate (`group[df, 'dept', ['salary', 'mean']]` has `dept` and `salary_mean`); `pivot` gives its rows column as it was, the rest as they show; a `window` is typed by what it computes (frame). A frame handed to a function keeps its column types there — each call's, the function a copy per set of columns (see frame, A frame's type is its columns) — so a function reads its cells as the numbers or text they are. A cell of a column whose type nothing tells the compiler — a file loaded as it runs, a query no database answered and no record holds — is dynamic, as a value `json[]` parsed is, in both modes: `int[c]`, `float[c]` and `str[c]` read it as one type (and fail as they do on text), arithmetic or a comparison with a number on it is refused until the program says what it is (`cannot use '*' with json and float — … read as one first: int[…], float[…] or str[…]`), as is giving one to a variable that holds another type (`cannot reassign 'next' from str to json — … read as one first`), and a missing one is JSON's `null`, an error value of reason `null`. (Compiled, such a cell was read as text while the interpreter read the number: the same line meant two things.) On sqlite, a computed column (`count(*)`, `round(...)`) is typed by stepping the SELECT once with null parameters; when no row comes back — `WHERE session_id = ?`, an empty table — it is typed from the statement itself, by the declared types of the columns it reads and SQLite's rules (`round`, `avg` and arithmetic on a `REAL` are floats, `count` and comparisons ints, `||` and `datetime` text; a parameter or `NULL` alone stays text). The cells of a `RETURNING` clause — `query[book, 'INSERT ... RETURNING id', [...]]`, the way to read back a row the database numbered — are typed from the table's declared columns; asking never runs a write, and never creates a sqlite file that is not there yet. Every command asks the same way — `crow run`, `crow run -i`, `crow test`, `crow check` and the app's book — so a program refused for its columns' types (a local given an int cell, then text) is refused by all of them. Set `CROW_NO_DESCRIBE=1` to build without asking: no database answers then, so the records type the cells, and nothing is written.


#### Bulk Insert

| Function | Description |
|---|---|
| `insert[conn, table, frame]` | Bulk insert a frame into a table using native fast paths |

Uses backend-specific protocols for maximum performance:
- **PostgreSQL:** `COPY … FROM STDIN`, the text format
- **ClickHouse:** RowBinary over the HTTP interface, encoded by the table's types (`DESCRIBE` first)
- **SQLite:** Batched prepared statements in a transaction

```
use db
conn = connect['clickhouse://localhost/default']
insert[conn, 'events', events_frame]
```

#### Transactions

| Function | Description |
|---|---|
| `transact[conn, fn]` | The function run inside a transaction: its value, committed; or its error, rolled back |

One connection is checked out, `BEGIN` run on it, and the function handed it as its one parameter; its statements through that parameter are the transaction's. **`transact` answers what the function answers.** A function that fails — its value an error (`return error['no', 'db']`, or a failed value it returns), or a stop inside it — rolls the transaction back, and `transact` answers that error (a stop stops, as a stop does); a function that succeeds commits, and a commit the server refuses answers an error of kind `'db'` with the server's words. Inside a handler, whose writes through a connection are one transaction already (A handler is a transaction), `transact` is a savepoint on it: released when the function succeeds, rolled back to when it fails, the handler's other writes untouched. The same in both modes (parity 167). No `begin`/`commit`/`rollback` builtins — one way to do transactions.

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

#### The schema a program declares

**A program's tables are the ones its own statements make.** Every literal `CREATE TABLE` a program hands a connection — `exec[book, 'CREATE TABLE IF NOT EXISTS cards (…)']`, directly or through a function the connection is handed to (`setup[book]`) — declares that table, with its columns, on that connection. A table the program also drops there (`DROP TABLE IF EXISTS scratch`) is one it remakes each run, and declares nothing. A connection the program declares no table on is one it does not own: nothing below reads or changes it, and the tables and columns a database has beyond the declared ones are its own.

**Opening a connection checks that the tables the database has hold what the program declares**, in one of three modes, the same in both modes of running. A declared table the database does not have yet is not a difference: the program's own statement makes it as it runs, so a fresh database starts as it always has.

| Mode | At connect |
|---|---|
| `reconcile` | What a table lacks is made: a declared column it does not have is added (`ALTER TABLE … ADD COLUMN`, as the statement declares it), under the database's lock (Postgres: an advisory lock, in one transaction; SQLite: an immediate transaction), so two processes starting at once make it once. A column SQLite cannot add in place — a key, or `NOT NULL` without a default — is made by making a table with no rows anew. A difference reconciling cannot make without losing data or guessing — a column of another kind than declared, a `NOT NULL` column without a default for a table that has rows — stops the program with each difference and the statements that would make it, and nothing is made. |
| `verify` | Nothing is changed. The database's catalog is read (reading it needs no right beyond reading), and every declared column of every table the database has must be there, of the declared kind; a difference stops the program at connect with each difference and the statements that would mend it. |
| `trust` | Nothing is read, and every start says so: `schema not verified — schema = 'trust'`. |

A column's kind is the kind Crow reads its cells as (int, float, str, bool, datetime): `TEXT` and `VARCHAR(20)` are one kind, `INTEGER` and `TEXT` two. A column declared without a type is of any kind.

A program run from its source — `crow run`, `crow run -i`, `crow test` — reconciles: it is being developed, against a database of its own. A program `crow build` makes verifies: it runs where the database is someone else's to change. The crowfile's `schema = 'verify'` (or `'reconcile'`, `'trust'`) says otherwise for both, and is what a built program starts with; the environment's `CROW_SCHEMA` says otherwise for one run (`CROW_SCHEMA=trust ./app`).

`crow db diff [file.co]` prints, for each connection a program declares tables on and opens with a url it writes, the SQL that would make its database hold what the program declares: the `CREATE TABLE` of each declared table it lacks, the `ADD COLUMN` statements reconciling would run and, commented out, the changes reconciling will not make (a column of another kind, a `NOT NULL` column a table with rows cannot take without a default) — a draft for a migration to review. It only reads the catalog; a SQLite file that is not there is not made.

```
$ crow db diff app.co
-- sqlite://board.db (book)
-- table 'cards' lacks column 'due'
ALTER TABLE "cards" ADD COLUMN due TEXT;
-- table 'labels' is missing
CREATE TABLE IF NOT EXISTS labels (card INTEGER, name TEXT);
```

**Design Note: schema modes.** Considered: migrations alone (`migrate[conn, dir]`), which leave a table the program changed — a new column — to fail at the first query that reads it, in development and in production alike; making the schema everywhere (Prisma's `db push`, Django's old `syncdb`), which alters production databases the program's user has no right to alter and must not; and a check only in production. Decided (Vijay, July 2026; approved September 2026): a program's `CREATE` statements declare its tables; development makes what is missing under a lock, a built program checks it at connect, and `crow db diff` drafts the migration for what making would lose (as Terraform's `plan` drafts before an `apply`, and Prisma's `db push` in development meets `migrate deploy` in production). At connect a missing table is left to the program's own `CREATE`, which runs moments later: checking it there stopped every built program on its first run against a fresh database, and making it there broke a program whose own `CREATE TABLE` (without `IF NOT EXISTS`) then found it made — the drift worth stopping for is the table that exists with other columns, which `CREATE TABLE IF NOT EXISTS` silently keeps.

#### Pool Defaults

| Backend | Default Pool Size | Notes |
|---|---|---|
| SQLite | 1 | File-based, concurrent writes lock |
| PostgreSQL | 10 | Standard for web/data apps; the driver written in Crow keeps the connections it opened |
| ClickHouse | 4 | The HTTP interface, kept alive; opened as queries need them. One query already runs on every core of the server, so more at once contend rather than help |


## 14. Parallelism

Crow is parallel where the work is: a `for` loop runs its iterations on every core, a frame or tensor operation uses every core inside itself, and a database query is parallel on its server. No explicit threading, async/await, or channels needed.


### Statements Run in Order

Statements execute in the order they are written, in both modes. Two statements that look independent may still share something no variable names — the random generator, the clock, the output, a file, a table — and the order written is the order that happens:

```
seed[1]
a = rand[]                   --- the first number of the seeded sequence
b = rand[]                   --- the second
write['out.csv', text]
df = frame.load['out.csv']   --- reads what the line above wrote
```

Work that should overlap is written as a `for` over the pieces: ten fetches in a for-expression run at once (below).


### Design Note: Statements Run in Order (a reversal)

**What shipped first:** the runtime grouped independent assignments — statements sharing no variable — into waves and ran each wave in parallel, in both modes, so `a = fetch['url1']` and `b = fetch['url2']` overlapped with nothing written.

**Evidence:** a seeded script drawing three numbers in three independent statements gave five different results in thirty runs; a `write` followed by a `load` of the same file was one wave, so the read could run before the write. The analysis saw variables only. The random generator, the clock, the output stream, files, tables, and every function that touches them were invisible to it, and a miss was a timing-dependent bug that no test catches reliably.

**What was considered:** making the analysis exact by ending a wave at every statement that is not pure. That is a few days of work, and it leaves nothing worth running in parallel: what remains is pure arithmetic and the frame and tensor operations, which are parallel inside already. Keeping "unordered by default" with an explicit sequencing form was also considered; it asks every programmer to know which builtins carry hidden state, the opposite of never thinking about parallelism.

**Why this design:** the programmer never thinks about order, because the order is the one written, as in every language they already know. Parallelism stays where the time goes — `for`, the kernels, the database — with nothing to write. `crow test --bench` is the measure of what the change costs, and the benchmark programs did not slow down when the waves went.


### For Loops

A `for` loop runs its iterations in parallel when nothing its body does can show the order they ran in, and in order when something can. Both modes decide the same way, once, from the program; a collecting `for` keeps its results in input order either way.

A loop runs **in order** when its body:

- writes a variable from outside the loop, or a fact;
- reaches an effect: output, files, the database, the network, the clock, `later`, `notice`, `download`, a sign-in;
- draws from the random generator (`rand`, and `randn`, `shuffle` and the other draws of `math`), so a seeded program draws the same numbers on every run;
- renders a view or a paged table, which a page numbers in the order it draws them;
- asserts, or asks whether background work is `running`;
- can leave the function: a `return`, or an error handed back from a condition, where the first failure in input order decides;
- calls a function that does any of these, however deep. A function handed to a builtin (`any[xs, check]`) counts as called, and a call through a variable counts as a call of any function the program holds as a value.

Everything else runs **in parallel** on every core: arithmetic, text, lists, maps, records, frames and tensors, reads of facts and derivations, elements, and calls to functions that do only these.

```
for x in items
    process[x]         --- parallel when process only computes
total = 0
for x in items
    total = total + x  --- in order: writes the outer 'total'
for x in items
    log[x]             --- in order: output is an effect
```

`cpu[1]` runs every loop that starts after it on one core (Resource Control).


### Design Note: When a Loop Runs in Parallel

**What shipped first:** each mode decided for itself. The interpreter ran a loop in order as soon as its body called a function the program defined; the compiled program ran it in parallel unless the function reached an effect. Neither looked at the random generator, rendering, or a way out of the function, so a seeded loop of `rand[]` drew different numbers on every run, and two views rendering at once could mix up the page's marks.

**Why this design:** "no side effects, then parallel" is the rule a reader can predict, and it is decided once, where every function's reach is known, so the modes cannot drift: the IR carries the answer (`crow build --ir` shows `parallel` on each loop that has it) and both backends only read it. The cost is a loop over a function that logs or draws running on one core, and the idiom that keeps it parallel is the one the book teaches anyway: compute in the loop, print after.


### While Loops

`while` loops run sequentially.

```
while x gt 0
    x = x - 1
```


### Resource Control

By default, Crow uses all available CPU cores. Use `cpu`, `gpu`, or `nodes` to control resources.

```
cpu[4]                   --- 4 CPU cores
gpu[]                    --- all GPUs on this machine
gpu[2]                   --- 2 GPUs on this machine
nodes[4]                 --- 4 machines
nodes[4, gpu = 2]        --- 4 machines, 2 GPUs each
```

`cpu[n]` bounds every parallel loop that starts after it runs — in the code after it and in every function that code calls, whichever was written first — until another `cpu[…]` changes it; written in a function, it holds until the function returns, and the caller's count is back. A loop nested in a bounded loop runs on the same cores. A count below 1 stops the program (`cpu[] requires a positive integer`). Both modes read the count where a loop starts, so the same loops are bounded in both. (The compiled program had applied `cpu[1]` while it wrote the program, so a loop in a function written above the call ran on every core, and `cpu[4]` bounded nothing; the interpreter had sized one pool for the whole process at the first `cpu[…]`, and ignored every later one.)

#### Function-level

```
fn heavy_work[]
    cpu[4]
    for x in items
        process[x]           --- 4 cores

fn gpu_work[]
    gpu[]
    for x in items
        compute[x]           --- all GPUs
```

#### Global

Set program-wide defaults in the `crowfile` (integers). Any function can override locally.

```
--- crowfile

cpu = 4
gpu = 1
```

```
--- file: main.co

fn task_a[]
    for x in items
        process[x]           --- 4 cores (from crowfile)

fn task_b[]
    cpu[8]                   --- override: 8 cores for this function
    for x in items
        heavy[x]
```


### GPU Support

`gpu[n]` specifies intent — "use n GPUs." `gpu[]` uses all available GPUs. The language does not expose any GPU vendor API. The compiler decides what is GPU-eligible (numeric operations on arrays) and what falls back to CPU (string ops, I/O, map lookups).

#### Implementation

When `gpu[]` or `gpu[n]` is called, a global GPU-enabled flag is set. This has two effects:

1. **Tensor auto-placement**: Tensor creation functions (`zeros`, `ones`, `full`, `arange`, `linspace`, `eye`, `randn`, `uniform` and the other distributions) automatically place data on GPU memory when the flag is set and a GPU is available.

2. **Op auto-dispatch**: Element-wise operations (`+`, `-`, `*`, `/`, `neg`, `abs`, `exp`, `log`, `sqrt`, `sin`, `cos`) and scalar operations automatically dispatch to GPU compute shaders (WGSL via wgpu) when both operands are on GPU. Reductions (`sum`) also dispatch to GPU. If tensors are on CPU, the CPU path is used as fallback.

GPU uses f32 internally (wgpu limitation). CrowTensor uses f64. Conversion happens transparently during GPU upload/download, with ~7 significant digits of precision on GPU vs ~15 on CPU.


### Automatic Differentiation (grad)

`grad[f]` takes a function and returns a function of the same parameters that computes the gradient of `f`: its derivative with respect to each parameter. A function of numbers is differentiated forward, with dual numbers; a function of tensors backward, over a tape its run records. The `grad` builtin works identically in both modes.

#### Numbers

When `f` takes one number and returns a number, `grad[f]` returns a function that computes the derivative:

```
fn f[x]
  x * x

df = grad[f]
log[df[3]]       --- 6 (derivative of x^2 at x=3 is 2*3=6)
```

When `f` takes several numbers, `grad[f]` returns a function that computes the gradient (a list of partial derivatives):

```
fn g[x, y]
  x * x + y * y

dg = grad[g]
log[dg[3, 4]]    --- [6, 8] (∂g/∂x=2x=6, ∂g/∂y=2y=8)
```

All arithmetic operations (`+`, `-`, `*`, `/`, `^`) and `sqrt`, `abs`, `exp`, `ln`, `log2`, `log10`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `erf`, `floor`, `ceil`, `round` and `sign` are differentiable (the last four have derivative 0) — every function of a number the math module has (`gamma`, `factorial`, `choose`, `bessel` and `atan2` take tensors), and `%` is refused. (The dual numbers had carried arithmetic, `sqrt`, `abs` and the roundings only, so a function with `sin` or `exp` in it stopped as it ran.) Forward mode carries a value and a derivative in each number; for N parameters, N passes compute the gradient. A helper `f` calls — a function of numbers answering one — is differentiated too, through its own run on the dual numbers, in both modes (compiled, the helper had only its float version, and the program had not built). A function handed to `grad` is a function of floats: its parameters are floats wherever it is called, so an int handed to it is a float, as it is to any float parameter, and the builtins its body calls are the ones for floats. (Its parameters had no type, `grad[f][3]` left them so even where `df = grad[f]` did not, and a compiled program that called such a function with an int had not built.)

#### Tensors

When a call of the gradient hands `f` tensors, `f` is a function of tensors and `grad[f]` computes the gradient of the number `f` answers with respect to each tensor parameter — one tensor for one parameter, a list of tensors, one per parameter in order, for several:

```
use math, math.nn

fn loss[w, b, x, y]
    p = sigmoid[x * w + b]
    return bce[flatten[p], y]

g = grad[loss]
grads = g[w, b, x, y]        --- [∂loss/∂w, ∂loss/∂b, ∂loss/∂x, ∂loss/∂y]
w = w - grads[0] * 0.1
```

`f` runs once on *traced* tensors, each remembering how it was computed from the parameters, and the tape is walked backwards from `f`'s number, so every parameter's gradient comes from one run however many parameters there are (reverse mode: what training needs). Each operation computes its value as the operator does, so `f`'s number is the same whether traced or not, to the bit. The operations a function `grad` differentiates may use: `+`, `-`, `*` (a number, or the matrix product), `/`, `.*`, `./`, `.^`, `^`, negation; `sum`, `mean`, `max` and `min`, whole or along an axis (an extreme's gradient reaches the first element holding it); `exp`, `ln`, `log2`, `log10`, `sqrt`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `abs`, `erf`, and `clip` (its bounds numbers that take no gradient; the gradient passes strictly inside them); `floor`, `ceil`, `round` and `sign`, constants of the tape as they are for numbers; `transpose`, `reshape`, `flatten`; `len`, `.shape` and `onehot`, read and never differentiated; the constructors (`zeros`, `ones`, `full`, `eye`, `randn`, `tensor`, `linspace`), which make constants; and every function of `math.nn`, each a formula over these (relu is `clip`, sigmoid is `exp`, the losses are `ln`, `mean` and `onehot`), differentiated as a helper is. A helper `f` calls is differentiated too, through its own run. A value `f` reads that is no parameter — a fact, a tensor from outside — is a constant, with no gradient. A loop in `f` runs in order, since one tape records the run. `log` inside `f` prints the value. `f` answers a number (a tensor of one element counts); the gradient of a parameter the answer does not depend on is zeros of its shape. A shape that does not meet stops the program where it runs, with the operator's words, as it does untraced.

What is refused before the program runs, in both modes: an operation on a traced value that is not in the list (`grad: loss calls \`prod\`, which is not differentiable`), an index into a tensor or a loop over one inside `f`, a comparison of tensors (a comparison of two numbers decides as it would, with no gradient through the decision), a parameter that is a record or a list, parameters of two kinds (numbers beside tensors: all numbers, or all tensors), and `grad` called inside a function `grad` differentiates. A function `grad` differentiates is typed by the calls of its gradient: `g[w, b, x, y]` types `loss`'s parameters as those tensors, as a direct call would.

**Design Note: one `grad`.** Considered: a `backward` on tensors that remember their history, as PyTorch's `requires_grad` and `.backward()` do (rejected: a tensor would carry a history the program never wrote, and a value's meaning would depend on where it came from); a separate `gradient[f]` for tensors beside `grad[f]` for numbers (rejected: two names for one operation, decided by the parameters' kinds that the call already shows). Decided (2026-10-02): one `grad`, forward over numbers and backward over tensors, the kind decided by what its gradient is handed; the list of differentiable operations is the list, and anything outside it is refused before the program runs rather than silently a constant.

### Distributed Support

`nodes[n]` specifies distributed computing across multiple processes. The compiler handles process coordination, and the `crow launch` CLI command spawns and manages all processes.

```
nodes[4]                 --- 4 processes
```

#### Implementation

When `nodes[n]` is called, the runtime initializes distributed mode using TCP-based networking:

1. **Process discovery**: Each process reads `CROW_RANK`, `CROW_WORLD_SIZE`, `CROW_MASTER_ADDR`, and `CROW_MASTER_PORT` from environment variables.
2. **Ring topology**: Processes form a ring — each connects to the next rank via TCP.
3. **Available operations**: `allreduce_mean` (ring allreduce), `broadcast`, `barrier`, `shard` (split data by rank).
4. **Finalize**: The compiled program calls `finalize()` at the end of main to clean up connections.

In interpreter mode, `nodes[n]` always simulates single-process (rank 0) — no TCP connections are made.

#### CLI: crow launch

```
crow launch <n> <file.co>
```

Builds the binary via `crow build`, then spawns `n` processes with the correct environment variables. All processes run on `127.0.0.1` with base port `29500`. Output from all ranks is interleaved to stdout/stderr.

```
crow launch 4 train.co     --- build + run on 4 processes
```

#### Backend Detection

The toolchain auto-detects the available GPU runtime:

- NVIDIA → CUDA
- Apple Silicon → Metal
- Cross-platform → Vulkan / WebGPU

The backend can be pinned in the `crowfile`:

```
gpu_backend = 'auto'        --- default: detect at build time
gpu_backend = 'cuda'
gpu_backend = 'metal'
gpu_backend = 'vulkan'
```

#### Decoupled from Language Version

New GPU vendors, new CUDA versions, or new backends are **toolchain updates** (`crow update`), not language changes. The `.co` source files never change. This follows the same model as LLVM — Rust and Swift don't change when LLVM adds a new target.

The language specifies *intent*. The toolchain handles *implementation*.


## 15. Background Work: later

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
| `running[f]` | is a schedule of `f` still in flight? (spinners, disabled buttons); for a derivation, is a recompute of it in flight? |

Semantics, in order of importance:

1. **State is the only mailbox.** In live apps the fn reads and writes
   reactive state; its writes are applied on the loop between renders, one
   schedule at a time, as one — state never mutates concurrently, and
   landing writes re-render the page. In scripts (function-scope isolation
   applies) the fn communicates through its arguments and the database.
2. **Skip-if-running.** A second `later[f]` while `f` is in flight is a
   no-op. Double-clicks and overlapping polls cannot stack.
3. **A failure stops it.** A failing query inside the fn stops it as it
   stops a handler: nothing it wrote lands, the state keeps its last
   value, and the notice says why.
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
jobs. *Delivered with the session layer (session-layer-design.md §3.4):*
a derivation that queries is computed off the render and a superseded one
is stopped — the automatic offload, for derivations, and the
cancel-and-restart; handlers follow in the layer's next phase.


### Design Note: Effects Commit When the Handler Ends

**What was considered:** leaving effects immediate (the rule before the
session layer); making every effect deferred, including reads (rejected:
`RETURNING`, a shell status, a query the handler branches on all need an
answer, and a handler that cannot read is not a handler); two-phase
commit across stores (rejected: three databases with three transaction
models cannot promise it, and pretending would be the worst kind of
promise).

**Why this design:** one rule, decidable from the text, that a reader can
apply without the runtime: needed the answer, it ran; did not, it waits.
Elm's commands and Redux's thunks return effects as values but make the
handler write them as data; here the handler's text is unchanged and the
runtime does the holding. Design: `core-design.md` §1.

### Design Note: A Fact Never Holds an Error

**What was considered:** errors landing in facts (the rule before, and
`later`'s); a per-fact choice.

**Why this design:** a fact is what the reader acts on. An error in a
fact is a page that lost its data to a string, and a handler that
half-succeeded. Stopping keeps the last good value and tells the reader
why; derivations, which are beliefs, keep holding errors, so a page with
one failed part is still a page. Design: `core-design.md` §2.

### Design Note: One Moment Per Event

**What was considered:** a wall-clock read per call (the rule before); a
frozen clock for the whole process (rejected: `refresh` and derivations
over `now[]` need it to move); a `clock` capability only (deferred:
capabilities bind what a part needs; the per-event moment is what the
binding delivers).

**Why this design:** an event is one thing that happened at one time.
Two readings inside it are two answers to one question. Tests set it
with the verb that sets any input, because it is one. Design:
`core-design.md` §3.

### Design Note: Handlers and later

**What was considered:** dropping `later` once handlers run off the loop
(rejected: a thirty-second import would hold every other click of the
session); a second kind of handler (rejected: two ways to handle an
event).

**Why this design:** a programmer never chooses where a handler runs; the
runtime does, invisibly. The only choice left is whether the session's
other clicks should wait for this work, and `later[f]` is the one way to
say they should not. Design: `session-layer-design.md` §3.3.

### Design Note: Text Is Text

**What was considered:** keeping the old rule — a string that begins with
`<` and ends with `>` is markup — which made composing views free (a view
returned a string, and a string that looked like markup was placed as
markup); rejected: whatever a reader types or a database holds can look
like markup, and a page placed it as markup, so one reader's text ran as
script in every other reader's page. A `raw[s]` for markup built by hand
(deferred: nothing in the corpus builds markup from strings; if it is
needed, it gets a name that says what it does).

**Why this design:** markup has a source — a view, an element, `doc` —
and only what came from one is markup. The interpreter carries the
source with the value; the compiler carries it in the type (`markup`, a
string wherever a string is read), and compiles a view once for markup
and once for text when it is given both, so the two modes place every
child alike. Reading a string never makes it markup.

### Design Note: One Widget Element

**What was considered:** growing the vocabulary for each thing a page
might draw (rejected: a map, an editor, a chart library — every one a
language change, and never enough); script tags or inline JavaScript in
views (rejected: two ways to put code on the page, and text a reader
wrote could become one of them); widgets that write facts directly
(rejected: the page would write state no handler checked).

**Why this design:** one element and one module shape. The module draws;
everything it tells the program arrives as an event the page offered, to
a handler, like a click — so the session's rules (one event at a time, a
transaction, a notice) hold for widgets too. Design:
`session-layer-design.md` §4.4.

### Design Note: Files Are Inputs and Effects

**What was considered:** an `upload` element and an event carrying the
file's bytes (rejected: a second way to bind a value, and a socket that
carries megabytes between clicks); a file record as a struct the runtime
declares (rejected: the reader's map of strings is already the shape a
record the runtime fills takes, and a program declares `upload =
map[]` as it declares any fact); `download` as a response a handler
returns (rejected: handlers return nothing to the page; everything a
handler does to the page is an effect held to its commit).

**Why this design:** a file field is a field whose value is a map; the
bytes travel beside the socket, by a one-time ticket, as a sign-in's
cookie does, and are limited before they are sent. A download is an
effect like `notice`: a handler that stops hands the reader nothing.

### Design Note: The Runtime Says Who the Reader Is

**What was considered:** a `reader` fact the program declares and a login
handler assigns (rejected: anything a handler can assign, a bug can
assign, and nothing would keep a bound input or a stray event away from
it); handing the program the cookie, or `web.auth`'s `sign` and `verify`,
to build sessions itself (rejected: every app would rebuild the same
security-critical plumbing, and most would get one part wrong); a record
type for `reader` (rejected: the runtime reads it from a cookie before the
program runs and cannot know the program's struct; maps are the one
shape that crosses that line, and the values are strings because
identity is — an id, a name, a role); a built-in users table and login
page (rejected: who may sign in, and how, is the app's; only the password
hash is not, so it is `hash[pw, 'password']`); a sign-in carried over the
socket (rejected: a socket cannot set an `HttpOnly` cookie, and a cookie
script can read is one a script can steal — the page fetches it from the
server with a one-time ticket instead).

**Why this design:** the reader is a fact the runtime provides, like the
clock: read everywhere, written by two effects, and checked by ordinary
code. Signing out ends the sign-in everywhere because that is what a
person means on a shared computer. Design: `session-layer-design.md` §4.1.

## 16. Verification: test Blocks

Verification is part of writing a function, not a separate discipline. A
`test` block states what a function does at points you chose, and what it
can never do anywhere:

```
fn fee[notional]
    if notional lt 0
        return error['negative notional', 'validation']
    return max[1.0, notional * 0.001]
test[n]
    fee[50000.0] eq 50.0                --- example: exact fact, checked once
    fee[100.0] eq 1.0                   --- the floor decision
    fee[-5.0].kind eq 'validation'      --- the error path
    if n gte 0.0
        fee[n] gte 1.0                  --- law: n is generated, many values
```

| Form | Means |
|------|-------|
| `test` after a fn | that function's examples and laws |
| `test name[…]` | a standalone scenario, any file, any position |
| `test[a, b]` | params are **generated inputs** — boundaries first |
| no params | runs once |

A param's type is what its use in the body gives it: the parameter of the
function it is handed to (as the test's other calls of that function type
it, when nothing else does), or a number, when it meets one in arithmetic.

A bare boolean line is a truth to check. `assert` is for steps inside
scenarios, and reports both operands on failure. Preconditions are plain
`if`s, and a bare boolean line under one is a truth too, checked where the
precondition holds. Bindings are plain assignments. A test body is
ordinary Crow. A block that reaches no truth and no `assert` — in its body
or in a function it calls — fails with `asserts nothing`: a test that
cannot fail tests nothing.

A test block sees declarations (functions, views, structs, imports) and two kinds of top-level assignment: literal values — a number, text, a bool, a list or a map of them, a frame built from them, a moment of them (`moment[0.0]`, `moment['2026-01-01']`) — which are reactive-state defaults, and connections (`db.connect[...]`), which are resources a render-time view needs. Computed top-level values are invisible, as inside any function.

Live apps are tested through a headless session — `app[view]` opens one,
`click[a, handler]` runs a handler on it, `a.name` reads reactive state,
`html[a]` renders the view to a string, and `elements[a, selector]`
returns the rendered elements that match a selector, in document order:

```
test refresh_flow[]
    a = app[render]
    click[a, go]
    assert[a.status eq 'updated']
    assert['count: 1' in html[a]]
```

A named test is a scenario and reads the program back: the facts and derivations, as the harness's clicks left them — `assert[session_id gt 0]`, `assert[len[rows] gt 0]`. Each session holds its own facts, as each served page does: the first `app[…]` takes the facts as the test has them, and every later one starts from where the first started. A bare fact name reads the session a verb acted on last, so a one-session test reads its session directly; `a.name` reads any session's — a fact, or a derivation as that session sees it (`len[bob.cards]`). An attached test (`test[p]` after a fn) sees what a caller of that fn sees and nothing of the program's state; its inputs are generated.

| Harness | Meaning |
|---------|---------|
| `app[view]` | a headless session rendering `view` against its own copy of the program's reactive state |
| `click[a, handler]` | run `handler` on the session as one transaction (deferred calls: `click[a, set_speed[10.0]]`, whose arguments may be numbers, text, lists, maps, tensors, frames or records, in both modes); when it stops, `click` evaluates to the error and the session is as it was |
| `html[a]` | the rendered view as one HTML string, as the browser builds it (each element's attributes in name order), the same text in both modes |
| `elements[a, sel]` | matching elements as records — `tag`, `text`, `class`, and every attribute (`href`, `name`, …) as keys; `sel` is `tag`, `.class`, `tag.class`, or a space-separated descendant chain (`'div.controls button'`) |
| `change[a, 'var', value]` | what typing into a bound `field` does: set the reactive variable, so the next render and handler see it. A part's fact is named as the code reads it: `change[a, 'session.day', '2026-09-04']` |
| `change[a, 'now', t]` | set the session's clock: `now[]` in its handlers, renders and derivations is `t` until the test moves it again |
| `change[a, 'reader', record]` | a reader who signed in: the session's `reader` is `record` (`map[]` signs them out), as if a handler had called `sign_in` |
| `downloads[a]` | what the session's handlers handed the reader with `download`, in order: maps of `name` and `text` (a frame's CSV, a string, a file's contents) |
| `app[view, width = 390, wide = true]` | a session at a viewport width (default 1280) — responsive rules are testable at every named width; `wide = true` lays the page out across the whole window, as `serve[..., wide = true]` serves it, instead of the reading column |
| `box[a, sel]` | the layout engine's boxes for matching elements: `x y width height overflow`, plus `tag class path` |
| `style[a, sel]` | resolved tokens: `background color size weight` |
| `fits[a, width]` | true when nothing overflows at that viewport width |

Structure is a truth like any other: `len[elements[a, 'div.stats .card']] eq 2`
says there are two cards in the stats panel; `elements[a, 'button'][0].text
eq 'Reset'` pins the first button; an element that must sit *inside* a
panel is selected through it. Layout that only CSS decides (pixels,
columns) is not a tree property and is not what a unit test checks.

Background work scheduled with `later[]` runs inline in tests, so a
handler's eventual state is observable on the next line — no waiting, no
flakiness. Two sessions are two independent pages, which is how event
interleavings are tested deterministically: one session's handler never
touches another's facts, and a table one writes is the table the other
reads, as in the served program (session-layer-design.md §3.1).

**Verification never creates methods.** Test bodies call production
functions — that is what verification is — but no function exists in the
program's namespace because verification needed it. Anything a truth needs
beyond calling the program lives inside the block.

`test` blocks never run in production: `crow build` strips them, `crow run`
ignores them. `crow test` runs them; `crow check` runs them plus everything
automatic — determinism contracts, edge cases, database hazards. `crow test
--compiled` runs them with every app they open (`app[view]`) compiled: the
block runs in the interpreter, and its harness verbs reach the compiled
program, started fresh for each run that opens one, so both modes answer
the same test. A test reads a fact or a derivation of the compiled program
by name, a record's fields included (`stats.items.names`), and the page it
renders is the settled one: a render that read a value still being computed
waits for it and renders again, as the interpreter computes it before it
answers. A handler a test clicks with arguments is a handler, as one
clicked by name is, and its arguments type the parameters nothing in the
program types.


### Design Note: Verification Is Part of the Function

**What was considered:** a conventional test framework (files by naming
convention, `assert` helpers, fixtures); separate keywords for examples
and laws (`verify` + `rule`, or `where` + `prop_`); helper functions
holding multi-line laws.

**Why this design:** every alternative created a boundary the language then
had to police — test files that drift from code, two words with identical
semantics, "is this function logic or test scaffolding?". One keyword whose
*position* carries the distinction (attached vs named) and whose *params*
carry quantification needs no policing: a test block cannot drift from the
function it sits under, and a helper function for verification is rejected
by the checker. The runtime/verification split is drawn by the vocabulary
itself: `assert` is a builtin that ships; `test` is a keyword that never
executes in a build. Deferred: compiled-mode execution of test blocks
(today they run in the interpreter) and richer input generation for
structs and frames.


## 17. Type System

Crow is **statically typed with inference**. No type annotations are needed — the compiler infers concrete types from how values are used.


### Rules

- Every variable has exactly one type throughout its scope
- Types are inferred from assignment: `x = 5` makes `x` an `int`
- Function parameter and return types are inferred from call sites
- If the compiler cannot determine a type, the program is invalid


### Call-Site Inference (No Generics Needed)

Crow has no generics, no type parameters, and no type annotations — but you can still write functions that work across different types. The compiler infers concrete types from each call site, effectively specializing the function for the types used.

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

evens = filter[[1, 2, 3, 4], is_even]         --- items: array[int], f: fn[int] -> bool
positives = filter[[-1, 0, 3], is_positive]    --- same types, different function
```

This works because `filter[[1, 2, 3, 4], is_even]` tells the compiler that `items` is an array of ints and `f` is a function taking an int and returning a bool. No annotations needed — the call site *is* the type information.

When one function is called with *different* type signatures, the compiler **monomorphizes** it — generating one specialized native version per signature, invisibly:

```
fn first[arr]
    return arr[0]

log[first[[1, 2, 3]]]      --- compiles a tensor version
log[first[['a', 'b']]]     --- compiles a string-array version
```

A parameter nothing in the program types — a function only tests call — takes the type every test call agrees on, and stays unknown where they disagree, so a test never makes the program refuse what it did not. A part's tests type its functions the same way in a program that imports the part, whose build leaves the part's tests out: a part's function has the same types whichever file is the entry. (Such a function had its calls run by name, chosen as the program ran.) A tensor's shape is not a signature: a function handed tensors of different shapes is one function, whose parameter has the dimensions its calls agree on (see Tensor Arithmetic).

Specialization propagates through call chains and works with recursion and struct arguments. A version is what its function is: a view's versions are views, and a handler's are handlers, moving state as the handler does. (A handler's version was made a plain function, so its fact writes landed in locals the page never saw — `scan.focus` handed a symbol from a query and one from a click.) Two documented limits: a function used at multiple signatures cannot itself be passed *as a value* (a function value has one type — wrap it in a single-type function), and a generic function with *function-typed parameters* can be reused across predicates of the same element type but not across element types (specializing the predicate per clone is future work).


### Execution Modes

| Mode | Command | Role |
|------|---------|------|
| Transpiler | `crow run file.co` | **Primary.** Compiles to native Rust. This is the authority on valid Crow programs. |
| Interpreter | `crow run -i file.co` | **Dev tool.** Runs code directly for quick iteration. Slower, but convenient. |
| REPL | `crow` | **Interactive.** Explore and test code interactively. Uses the interpreter. |
| Dev server | `crow dev [file.co]` | **The edit loop.** Serves the program with the interpreter, and reloads it as its files change. |

**`crow dev`** serves the program with the interpreter and reloads it as its files change. A change is checked first, by the gate `crow run -i` passes. A program that passes replaces the one serving, and every tab reconnects and resumes its session, drawn by the new code: `crow dev` keeps sessions as `serve[…, keep = true]` does. A change that breaks the program leaves the last good one serving, and every open page shows the refusal, with its file and line, until a change mends it; a click on it uncovers the last good page. A change to what a page's shell carries (`css/app.css`, a widget's module) reloads the tabs, and a program that serves nothing runs again. The program `crow dev` started stops with it, and nothing of it reaches a build: its routes and its part of the page exist only under `crow dev`, taken only from the `crow dev` that started the program.

**The dev panel.** Under `crow dev`, Ctrl+Shift+D opens a panel over the page: the session's facts, live; each event (a click by its handler, an input by its fact) with the facts it changed, before and after, and its render's time and size — a handler that stopped with its reason and the id its reader was told (`apply stopped [3f9a1c07]: cannot convert 'x' to float`); and the renders' timing, last, average and slowest. The panel watches and changes nothing: a value is tried through the page.

**Design Note: the dev panel watches.** Considered: editing a fact and resetting the session from the panel (ui-framework.md, Dev Tools). Rejected: it writes facts that no input and no handler wrote, so the page would show a state the program cannot reach and a bug found there might be the panel's. A value is tried through the page, and a test sets one with `change[a, …]`.

The compiler's front end is the source of truth for what a valid program is: it infers a concrete type for every value. Both modes run the same front end (parse → expand → lower → infer → validate) and then the same intermediate form, which the interpreter walks and the Rust backend prints, so they reject the same invalid programs with the same messages.

**The parity promise:** both modes are required to produce identical output. This is enforced mechanically — `crow test --parity` runs `tests/parity/` through both modes and diffs stdout, and `crow test --book` does the same for every code block in the book. A disagreement between the modes is a bug in Crow, never a semantics to learn. Interpret while exploring (instant start, the REPL, `crow test`); compile when shipping (`crow build` produces the binary without running it). `crow run` builds in the dev profile — the runtime crates at optimization level 1, the program at 0, no LTO — so the edit loop stays short; `crow run --opt 1 file.co` optimizes the program too (0 to 3, the runtime crates following above 1) for a run that needs the speed, and `crow build` is the shipped binary, level 3 with LTO. The level is a trade the program decides: a program whose own loops are the work runs many times faster at 1 (fannkuch: 3.85 s at 0, 0.32 s at 1, 0.12 s from `crow build`), and a large app rebuilds many times slower (finch's 10,541 lines of generated Rust: 3 s of compile after an edit at 0, 14 s at 1; 4 s and 26 s from scratch) — so 0 is the default, and `crow dev` builds nothing.


### No `none`

Crow has no null, nil, or none value. Every variable always holds a concrete value. Missing or failed data is represented through the error-value system (`ok`, `reason`, `kind`, `trace`) — see Error Handling.


## 18. Project Configuration

### Project Structure

A Crow program can be a single file or a full project. No restructuring is needed as a project grows.

#### Single File (No Project)

Any `.co` file is a valid program:

```
crow run hello.co
```

#### Project Structure

A project is a directory with a `crowfile` at the root. Create one with `crow new`:

```
crow new myapp
```

Full project structure (all optional except `crowfile` and `main.co`):

```
myapp/
  crowfile                    --- project identity + config
  main.co                   --- default entry point (crow run)

  jobs/                       --- batch tasks (crow run <name>)
    train.co                --- crow run train
    migrate.co              --- crow run migrate
    sync.co                 --- crow run sync

  views/                       --- web views (imported and wired in main.co)
    home.co                 --- use views.home → page['/', home.render]
    dashboard.co            --- use views.dashboard → page['/dashboard', ...]
    accounts.co             --- /accounts

  components/                 --- reusable UI functions
    nav.co
    chart.co

  layout.co                 --- shared UI wrapper for all pages

  lib/                        --- shared code (organized however you want)
    accounts.co
    ml/
      features.co
      model.co

  db/
    schema.co               --- source of truth for tables

  tests/                      --- integration tests
    accounts_test.co

  config/                     --- configuration files
  models/                     --- trained model artifacts
  data/                       --- data files
```

#### Conventions

| Folder/File | Purpose | Auto-discovered? |
|---|---|---|
| `crowfile` | Project identity, entry points, deps | Required for projects |
| `main.co` | Default entry point. `crow run` runs this | Yes — convention |
| `views/` | Web views. Import with `use views.X`, wire with `page[]` | Explicit |
| `layout.co` | Shared UI wrapper for all pages | **Yes** — auto-applied |
| `db/schema.co` | Table definitions, auto-migrations | **Yes** — auto-discovered |
| `jobs/` | Batch tasks. `crow run <name>` | **Yes** — listed in crowfile |
| `tests/` | Integration tests. `crow test` | **Yes** — auto-discovered |
| `lib/` | Your code, any organization | No — import with `use` |
| `components/` | Reusable UI functions | No — import with `use` |
| `config/` | Configuration files | No — import with `use` |

Only three magic folders: `views/`, `db/`, `tests/`. Plus `layout.co` and `main.co` by convention. Everything else is just files you import.

#### Entry Points

`main.co` is the long-running entry point (e.g., web server). `jobs/` contains batch tasks that run and exit.

```
crow run              --- runs main.co (long-running)
crow run train        --- runs jobs/train.co (batch, exits)
crow run migrate      --- runs jobs/migrate.co (batch, exits)
```

Rule: **`main.co` is the thing that stays running. Everything else runs and exits.**

#### Growth Path

| Stage | What you have |
|---|---|
| Day 1 | `hello.co` — one file, no project |
| Week 1 | `crow new myapp` — `crowfile` + `main.co` |
| Week 2 | Add `views/`, `lib/` — split code out |
| Month 1 | Add `db/schema.co`, `jobs/train.co` |
| Month 6 | Full project — multiple entry points, lib files, pages, components |

No restructuring at any step. You just add files.


### crowfile

Every Crow project has a `crowfile` at the root. It is a plain Crow file — the toolchain evaluates it and reads these variables:

```
--- crowfile

name = 'myapp'
entry = 'main.co'                 --- optional: main.co is the default
db = 'sqlite://dev.db'              --- optional: lets crow check read the schema
cpu = 4                             --- optional: default thread count

deps = map[models = 'github.com/agncrow/crow-models@v0.1.0', utils = './libs/utils']
jobs = map[train = 'jobs/train.co', migrate = 'jobs/migrate.co']
```

| Key | Meaning |
|-----|---------|
| `name` | Project name (written by `crow new`) |
| `entry` | Entry point for `crow run`; defaults to `main.co` |
| `db` | Database url used by `crow check` and the editor for schema-aware checks |
| `schema` | `'reconcile'`, `'verify'` or `'trust'`: what opening a connection does with the tables the program declares (see db, The schema a program declares); run from source reconciles, a built program verifies |
| `cpu`, `gpu` | Default resource counts (integers) — see Resource Control |
| `deps` | Map of dependency name → a git repository and its version (`github.com/org/repo@v1.0.0`), or a local path (`crow install`; see Package Manager) |
| `jobs` | Map of job name → file, run with `crow run <name>` |

`crow new` writes only `name`; add keys as you need them. No need to list `main.co` — it is the default entry point by convention.

Dependencies can be:

| Format | Example | Description |
|--------|---------|-------------|
| Repository and version | `models = 'github.com/agncrow/crow-models@v0.1.0'` | A git repository at a tag (or a branch, or a commit), pinned by `crow.lock` |
| Local path | `utils = './libs/utils'` | Linked from a directory on this machine |


### File Extension

Crow source files use the `.co` extension.

```
main.co
lib/accounts.co
views/dashboard.co
jobs/train.co
```


### CLI

```
crow                        --- start interactive REPL
crow new myapp              --- create new project (crowfile + main.co)
crow new myapp --web        --- create web project (+ views/home.co)
crow new myapp --ml         --- create ML project (+ jobs/train.co, lib/model.co)
crow new myapp --db         --- create DB project (+ db/schema.co, jobs/migrate.co)
crow new myapp --web --ml --db  --- all capabilities
crow install                --- install deps into crow_modules/, pinned by crow.lock (--update [name] moves them)
crow build                  --- transpile to Rust and compile to native binary
crow build file.co        --- transpile a specific file
crow build --ir file.co   --- print the program's one typed form, the IR (docs/ir-design.md); nothing is compiled
crow build --rs file.co   --- print the Rust the program compiles to, printed from the IR; nothing is compiled
crow run                    --- transpile + compile + run main.co
crow run file.co          --- transpile + compile + run a specific file (a dev build: the runtime crates at optimization level 1, the program at 0, no LTO)
crow run --opt 1 file.co  --- the same, the program at level 1 (0 to 3, 0 unless said; the runtime crates follow above 1; crow build is 3 with LTO)
crow run train              --- run a named job from the crowfile jobs map
crow run -i file.co       --- run with interpreter (instant start, for exploring)
crow stop                   --- stop the running app (pidfile, then project binary)
crow stop book              --- stop the book this project is serving
crow book [--port N]        --- in the crow repository: serve the Crow Book with every code block runnable in place;
                                inside an app: derive and serve the app's own book — pages with trees and wireframes,
                                a chapter per part, the data, the changes since the accepted baseline (--write: build only);
                                the app's book serves next to the app, on its serve port plus one (finch on 3100 → 3101);
                                its runner answers only the book's own page on this machine
crow test                   --- run the test blocks of the project (or: crow test file.co)
crow test --parity          --- run tests/parity/ in both modes and diff output
crow test --packages        --- in the crow repository: each package's tests, and its example.co in both modes
crow test --fuzz [n]        --- n programs generated from a seed, each run in both modes; one they disagree on is kept (--seed s repeats a run)
crow test --fixtures        --- the tests where the database servers are not: each read is what crow check --record-fixtures kept
crow test --book            --- execute every code block of the book in both modes
crow test --docs            --- the docs cover every compiler name, and name nothing the compiler lacks
crow test --bench           --- run tests/bench/ in both modes and print each section's seconds side by side (--quick: small sizes; --write: record docs/benchmarks.md)
crow check                  --- automatic verification (uses crowfile; or: crow check file.co); records and diffs the app baseline
crow check --accept         --- accept every baseline change at once (the loop's `a`, for a script or a deliberate sweep)
crow check --show [name]    --- the app as a tree: pages, views under their guards, controls and handlers, facts; or one declaration's artifact
crow check --db <url>       --- + schema-aware database checks (or db = '…' in the crowfile)
crow check --record-fixtures --- the scenarios read the real databases, and what each statement answers is kept in .crow/fixtures
crow check --strict         --- exit 1 on any finding (CI)
crow fix file.co             --- apply the fixes the program's refusal names (a slip, an import, a habit), then list them
crow fix file.co --hazard N [--option M]   --- apply a fix from the check report
crow fmt                    --- format project (uses crowfile; or: crow fmt file.co)
crow fmt --check            --- check if formatting needed (exit 1 if not)
crow fmt --fix-interpolation  --- migrate {expr} strings to `expr`
crow fmt --fix-views        --- migrate ui-building fns to view declarations
crow lsp                    --- language server over stdio (see crow-lsp.md)
crow kernel install         --- make Crow a Jupyter kernel (notebooks, JupyterLab, VS Code, jupyter console)
crow kernel <connection>    --- the kernel itself, as Jupyter starts it
crow launch 4 file.co     --- build + run on 4 distributed processes
```

`crow test` runs the `test` blocks you wrote (see Verification). `crow test --parity` is the parity harness: every `tests/parity/*.co` file runs in both execution modes and any difference in output (or exit status) is a failure — this enforces the parity rule mechanically. `crow test --book` does the same for every code block in the book. `crow test --fixtures` runs the tests where the database servers are not — CI, a laptop without the data — from what `crow check --record-fixtures` kept in `.crow/fixtures`: a connection to a Postgres or ClickHouse server reaches nothing; a statement that reads is answered from its fixture, and one no fixture holds is the query's error, which a test that fails names (``no fixture held `SELECT …` — `crow check --record-fixtures` records it where the database is``); a write answers as one that landed and changes nothing, so a test that reads back what it wrote to a server needs the server. A sqlite file is the program's own and opens as always. Each test runs as the recording ran it — the clock at 2026-01-01 00:00 UTC until the test moves it, the random draws seeded — so a statement that splices in the time matches what was kept, and with `--compiled` the apps the tests open answer the same way. A statement matches its fixture by its text as written, a list handed to `IN ?` as that one `?`, whatever its parameters. `crow test --fuzz` does it for programs no one wrote: generated from a seed — ints, floats, text, lists and maps, records, parsed documents read and spliced into text, arithmetic and comparisons, the text, list and map builtins, conversions read with `.ok`, branches, loops and functions — and run by the interpreter and compiled; a program the two disagree on, or one either stops on with a bug in Crow, is kept under `target/crow_fuzz/` with the first line they differ on, and the same seed makes the same programs on every machine. `crow test --docs` reads the compiler's own name tables and fails if a builtin, module function, or element is missing from this specification, if a reference row here names something the compiler doesn't have, if a module or core builtin has no verified example in the book, or if an example file is not indexed — so the language and its documentation cannot drift apart. `crow check` runs your test blocks plus everything automatic — edge cases, determinism contracts, live-loop hazards, and (with a database url) the schema-aware concurrency checks documented in `crow-check.md`.

`crow build` transpiles `.co` source files to Rust and compiles to a native binary. `crow run` builds and then executes the binary.

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

### Notebooks

`crow kernel install` makes Crow a Jupyter kernel: it writes the kernel's spec where Jupyter looks (`jupyter --data-dir`'s `kernels/crow`), so JupyterLab, the notebook, VS Code's notebooks and `jupyter console` offer *Crow*. A notebook is one interpreter session, the REPL's: a cell sees what the cells before it defined, what it logs appears under it as it runs, and its last expression's value is its result — a frame shown as a table of its first 50 rows. A cell that fails shows why, and the session goes on. Interrupting the kernel stops the running cell at its next loop turn, and the session keeps what it had. `crow kernel <connection-file>` is the kernel itself, which Jupyter starts with the file naming its sockets and the key each message is signed with (the Jupyter messaging protocol, 5.3). A cell is a fragment as a REPL line is, and a name it defines outlives it however many statements the cell has (a list a cell defined and read had reached the next cell as `done`).


### Package Manager

A package is a git repository, a version is one of its tags (or a branch, or a commit), and `crow.lock` pins the commit each dependency checked out. The crowfile names each dependency by its repository and version, or by a path for a package beside the program:

```
deps = map[models = 'github.com/agncrow/crow-models@v0.1.0', mine = '../mine']
```

`crow install` checks each one out into `crow_modules/<name>`, and the program imports its files with `use` (`use models.line` reads `crow_modules/models/line.co`). A package is rooted at itself: its files reach each other by their paths inside it, so `use line` in one of its files is the package's `line.co`, whatever the program beside it holds (it had been the program's own `line.co` when there was one). The first install of a dependency takes the commit its version names and writes it to `crow.lock`; every install after that — on another machine, in CI — checks out that commit, whatever the tag names by then, and says so when a tag has moved. `crow install --update models` takes what `models`'s version names now, and `--update` alone every dependency's; both rewrite the lock. A package's own dependencies — its crowfile's `deps` — are installed beside it in the same `crow_modules/`, pinned by the same lock, and its files reach them as the program does (`use market.bars` in one of `models`'s files reads `crow_modules/market/bars.co`); a path a package names is read from the package's directory. A project holds one version of a package: one wanted at two sources — the program at one version and a package at another, or two packages apart — is refused, naming who wanted each, `market is wanted at two sources — '…@v1' by the project and '…@v2' by the package models: a project holds one version of a package, so name the one they can share`. (A package's dependencies had not been installed at all.) Commit `crow.lock` with the program, and leave `crow_modules/` out of version control (`crow new` writes that `.gitignore`). A program run before its dependencies are installed is told which one is missing and to run `crow install`.

A repository on a forge (`github.com`, `gitlab.com`, `codeberg.org`, `bitbucket.org`) is the first two segments of its path, and what follows is the directory inside it that is the package: `github.com/agncrow/crow/packages/models@v0.6.0`. Any other place is a url, with the package's directory after a `//`: `https://git.example.com/pkgs.git//models@v1`, `file:///srv/pkgs.git@v2`. A local path is linked as it is, for working on a package beside a program.

**Design Note: no registry.** Considered: a central index mapping package names to repositories and versions (the `index.json` repository `crow install` had read), and a registry server that packages are published to, as crates.io and npm are. Decided: a dependency names its repository, as Go modules do. There is nothing to run or keep up, a name cannot be taken by someone else, and a tag is the one version there is; the lock is what makes every install the same, which a registry's immutable versions otherwise give.


## 19. Design Notes

The language-wide rejections, each with what was considered and why it lost. Notes about a single feature sit beside that feature (search this document for "Design Note").


### Names Are Never Ambiguous

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

**6. The backend's words are not reserved.** Crow compiles to Rust, but Rust's keywords are not Crow's: a name Crow does not reserve — `move`, `impl`, `match`, `loop`, `final` — is yours as a variable, function, parameter, loop variable or struct field, in both modes. The compiled program renames it for Rust behind the scenes and still names it as you wrote it in a trace.

**Why not shadowing (Python/Go-style)?** Because Crow's brackets unify calling and indexing, shadowing produces *silent wrong answers*: `range = [10, 20, 30]` then `range[0]` — delete the assignment during a refactor and the same expression silently becomes `range(0)` = `[]`. MATLAB, the one mainstream language sharing this syntax collision, chose shadowing — and it has been a documented footgun for forty years. Languages where shadowing is safe (Python, Go, Julia) are exactly the ones whose call syntax is distinct from indexing. Crow's is not, so a collision must be an error, at the earliest possible moment, with the fix in the message.

**Why this survives growth:** every new stdlib export visibly costs a name taken from every importer, which enforces the budget rule structurally; packages can never collide with anything; and `crow fmt --fix-collisions`-style migrations handle the rare stdlib addition that lands on someone's variable.


### Unified Arrays (No Separate List and Tensor Types)

**What was considered:** Having separate `list` (heterogeneous, general) and `tensor` (homogeneous, numeric, N-dimensional) types, with `tensor[[1, 2, 3]]` as the creation syntax.

**Decision:** One type — `array`. `[1, 2, 3]` is an array. Arrays are homogeneous (all elements same type). Nested brackets create multidimensional arrays: `[[1, 2], [3, 4]]`. No `tensor` keyword.

```
--- arrays
[1, 2, 3]                    --- int array
['a', 'b', 'c']              --- str array
[[1, 2], [3, 4]]             --- 2D int array
zeros[3, 4]                  --- 3x4 of zeros
uniform[0.0, 1.0, 2, 3]      --- 2x3 random in [0, 1)

--- mixed named data → use a struct
struct User [name, age]
User['alice', 25]            --- named, mixed-type fields

--- error
[1, 'hello', true]           --- error: mixed types
```

A literal of two kinds is refused before the program runs, in both modes: `a list holds one kind: this one has text and a number — a record (struct) holds fields of several kinds`. The one list of several kinds is a query's parameters (`query[conn, sql, ['a', 1.5]]`): a list of what each `?` takes, sent one by one.

**Why:** Python has both `list` and `numpy.array` — the "two type problem." You constantly convert between them. MATLAB, Julia, and R have one array type and it works. For data science, columns are homogeneous arrays. For tabular data, use a **frame** (`use frame`) — the dedicated type with named, per-column-typed columns, missing-value tracking, and relational operations.

**Tensors vs frames:** a tensor is positional numbers (one dtype, axes interchangeable — matmul, transpose, reductions); a frame is named observations (typed columns, filter/sort/group/join). An all-numeric frame and a matrix can hold the same grid, but intent picks the type — convert deliberately with `tensor[df]`. Neither grows the other's features: no named axes on tensors, no linear algebra on frames.

**Mixed-type data uses structs, not arrays.** `struct User [id, name, active]` then `User[1, 'alice', true]` gives named, typed fields with dot access. This is clearer than a mixed-type list anyway — fields are named and self-documenting. (Maps are homogeneous — see Design Note: Maps Are Dictionaries, Not Records.)

**What this eliminates:** `tensor` keyword, `list` vs `tensor` confusion, `tensor[[1,2,3]]` wrapper syntax, mixed-type arrays.


### Tensor Arithmetic Operators

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
h = W * x + b               --- matmul
scaled = W .* mask           --- element-wise multiply
normed = values ./ total     --- element-wise divide
squared = x .^ 2             --- element-wise power
```

**Shift-key tradeoff:** `^` and `.^` require the shift key, which normally violates Crow's principle #3 (shift-key minimization, `design-rules.md`). This tradeoff is accepted because data scientists type power operations constantly, and `pow[x, 2]` is too clunky. There is no non-shift alternative for power that isn't worse.

**Why not `matmul[]` builtin?** `W * x + b` is what every data scientist expects. A `matmul[W, x] + b` builtin adds noise to the most common operation in ML. `*` for matmul follows MATLAB and Julia — the two most established scientific computing languages.

**Lexing:** `.*`, `./`, and `.^` are lexed as single tokens (`DotStar`, `DotSlash`, `DotCaret`), not `Dot` + `Star`. This avoids ambiguity with field access (`model.w`).


### Map Arithmetic Requires All-Numeric Leaves

**Considered:** When doing arithmetic on maps (`model - lr * g`), what happens if a map contains non-numeric values like strings or bools?

Three options were evaluated:

1. **Error** — reject if any leaf is non-numeric
2. **Skip** — silently ignore non-numeric leaves
3. **Apply where possible** — multiply numbers, skip strings

**Decision:** Error. Map arithmetic requires every leaf to be numeric (tensor, int, float). If any leaf is a string, bool, or other non-numeric type, it's an error.

- **Compile time:** The compiler checks map leaf types and rejects the program when they are statically known, in both modes and with the same message.
- **Runtime:** The interpreter checks before applying arithmetic and errors on non-numeric leaves.

```
--- works: all leaves are numeric
model = map[w = [1, 2, 3], b = [4, 5, 6]]
result = 2 * model
```

Since maps are homogeneous (Design Note: Maps Are Dictionaries, Not Records),
a map mixing numeric weights with string config can no longer be constructed
at all — the question resolves at the type level.

**What to use instead:** Weights in a map (all tensors), config in a struct.

```
struct Config [lr, optimizer]

weights = map[w = randn[784, 128] * 0.01, b = zeros[128]]
config = Config[0.01, 'adam']

g = dloss[weights, x, y]
weights = weights - config.lr * g
```

**Why:** Silent skipping hides bugs. "Apply where possible" is ambiguous — does `2 * 'adam'` skip or error? Every other language (JAX, Julia/Flux, PyTorch, Mojo) either errors or requires explicit marking of trainable fields. Erroring is the simplest approach and matches Crow's philosophy of no hidden behavior. Separating weights from config is good practice regardless.


### No Lambdas or Closures

Crow intentionally has no anonymous functions (lambdas), closures, or inline function expressions.

**Why they were rejected:**

1. **Violates "one way to do things."** If lambdas existed, there would be two ways to define behavior: `fn double[x]` and `fn[x] x * 2`. Every code review becomes a style debate. One syntax for functions means zero ambiguity.

2. **Closures add hidden complexity.** Closures capture variables from their enclosing scope, which creates hidden data dependencies. In a language designed for automatic parallelism (section 14), captured mutable state would silently break parallel execution or require complex borrow-checking rules that Crow deliberately avoids.

3. **Named functions are more readable.** `find[items, is_big]` is clearer than `find[items, fn[x] x gt 100]`. The name `is_big` documents intent. Anonymous functions save a line of code but cost readability — a bad trade for a language optimized for clarity.

4. **LLM-unfriendly.** Lambda syntax varies wildly across languages (`=>`, `->`, `lambda`, `\x ->`, `fn[x]`). A single `fn name[params]` declaration is unambiguous for both humans and language models.

5. **First-class named functions cover every use case.** Crow supports passing named functions as values — `find[items, is_big]`, `transact[conn, transfer]`, `later[load_book]`, storing functions in variables. There is no expressiveness gap.

**What to use instead:**

```
--- define the behavior once, with a name
fn is_big[x]
    return x gt 100

hit = find[items, is_big]
```

For transformations, `for`-as-expression is what other languages use `map` + lambda for:

```
result = for x in items
    x * 2
```


### No Iterator Builtins

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

4. **Laziness isn't needed.** Crow's automatic parallelism (section 14) handles performance at the statement level. Lazy evaluation adds complexity without meaningful benefit for Crow's target audience.

**What to use instead:**

| Old (iterators) | New (for + slice) |
|---|---|
| `collect[each[iter[arr], fn]]` | `for x in arr \n    fn[x]` |
| `collect[filter[iter[arr], pred]]` | `for x in arr \n    if pred[x] \n        x` |
| `collect[take[iter[arr], n]]` | `slice[arr, 0, n]` |
| `collect[skip[iter[arr], n]]` | `slice[arr, n, len[arr]]` |


### No map Higher-Order Function

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


### No Enumerate or Zip

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


### Search Builtins (any, all, find) — a Reversal

**Original position:** `any`/`all`/`reduce` were rejected ("the loop versions are the same length"), and `find` was rejected ("finding a position is straightforward with a loop").

**Why it was reversed:** the recommended flag-loop replacement was worse than the thing it replaced, on three axes at once: it cannot stop early (always O(n)); mutating the flag variable triggers the automatic sequential fallback — the exact "hidden performance trap" the `break` rejection warns about; and it is more code. Worse, the *parallel early-exit* search (all cores scanning, all stopping at the first hit) is impossible to express in user code at all — parallel `for` cannot break, and `break`-able `while` cannot parallelize. That is the same "cannot be written cleanly without a builtin" test that justified `contains`, passed harder.

**The design:** predicates are named functions (no lambdas — name the question):

```
fn over_90[x]
    return x gt 90

any[scores, over_90]      --- true if at least one passes
all[scores, over_90]      --- true if every one passes
hit = find[scores, over_90]   --- the first passing ELEMENT
if not hit.ok                 --- error-value when nothing matches
    log['none found']
```

`find` dispatches by argument types: `find[str, pattern]` is the regex builtin; `find[array, function]` is the search. Both modes agree; compiled mode runs searches in parallel with deterministic first-match semantics (rayon `find_first`).

**Division of labor:** operators and verbs eat most loops; `for` computes over everything that is left, in parallel; `any`/`all`/`find` ask the questions that deserve to stop early; `while` handles genuine sequential iteration. `reduce` stays rejected — the named reductions (`sum`, `min`, `max`, `prod`, `join`) cover it.


### No sum, min[arr], max[arr] in Core

Crow intentionally omits `sum[arr]`, `min[arr]`, and `max[arr]` from core builtins.

**What was considered:** `sum[[1, 2, 3]]` → `6`, `min[[3, 1, 2]]` → `1`, `max[[3, 1, 2]]` → `3`.

**Why they were removed/rejected:** All three are straightforward loops:

```
total = 0
for x in nums
    total = total + x
```

Including `sum` but not `min[arr]`/`max[arr]` created an inconsistency — all three are the same pattern (reduce an array to a single value). Rather than adding more, we removed `sum` to keep core honest: if you can write it with a loop, write it with a loop.

**What to use instead:** `use math` provides `sum`, `min`, `max`, `prod`, and other tensor/array reductions (see section 13, math). For plain arrays without `use math`, use a loop.


### No chars Builtin

Crow intentionally has no `chars` builtin to split a string into an array of characters.

**Why:** With string indexing (`s[0]`, `s[-1]`) and `for`-as-expression, splitting a string into characters is a one-liner:

```
chars = for i in range[len[s]]
    s[i]
```

Adding a dedicated `chars` builtin would create a second way to do the same thing.


### Named Arguments with Defaults (Revised)

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

train[my_data]                     --- uses defaults
train[my_data, lr = 0.001]         --- override one
train[my_data, 0.001, 50]          --- positional also works
```


### No Augmented Assignment

Crow intentionally has no `+=`, `-=`, `*=`, `/=`, or any compound assignment operators.

**Why:** `x += 1` is syntactic sugar for `x = x + 1`. Adding it creates two ways to write the same thing, violating "one way to do things." Every language that supports both spawns style debates about when to use which form.

**What to use instead:**

```
x = x + 1
total = total + item
```

The explicit form is clear, consistent, and only a few characters longer. The compiler can optimize `x = x + op` patterns into in-place operations automatically — the sugar adds no performance benefit.


### No Destructuring

Crow intentionally has no destructuring assignment from function returns or collections (e.g. `a, b = f[]`).

**What was considered:** Allowing `a, b = divmod[7, 3]` where a function returns an array and multi-assign unpacks it.

**Why it was rejected:** Multi-assign already exists for literal values (`a, b = 1, 2`). Extending it to destructure arrays would create two ways to access array elements — destructuring and indexing (`result[0]`, `result[1]`). This violates "one way to do things."

**What to use instead:** Return an array and access elements by index:

```
result = divmod[7, 3]
a = result[0]
b = result[1]
```


### No Ternary Expression

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


### No Inline Error Fallback

Crow intentionally has no inline error recovery syntax (e.g. `x = int['bad'] rescue 0` or `x = int['bad'] or 0`).

**What was considered:** A keyword (`rescue`, `catch`, `alt`, `fail`, etc.) that silently replaces an error-value with a default, turning a 3-line check into a one-liner.

**Why it was rejected:** Inline fallbacks hide errors. The program continues with a value that may be wrong, and the error's `.reason` and `.trace` are silently discarded. This contradicts Crow's error philosophy — errors are values that propagate visibly, carrying context, until you consciously handle them.

Crow's errors don't crash the program. They propagate safely through function calls and operators:

```
x = int['bad']          --- error-value, no crash
y = x + 10              --- still error-value, no crash
log[y.ok]               --- false
log[y.reason]           --- original reason preserved
```

**What to use instead:** Check errors explicitly. The verbosity is intentional — it forces you to acknowledge the error:

```
x = int[input[]]
if not x.ok
    log['invalid: `x.reason`']
    x = 0
```

This may be revisited in the future if a design emerges that preserves error visibility while reducing verbosity.


### No Double-Quote Strings

Crow uses single quotes for all strings. There is no double-quote alternative.

**Why:** Adding a second string delimiter would create two ways to write the same thing, violating "one way to do things." Every language that supports both `'...'` and `"..."` spawns style debates and inconsistency.

**Nested quotes:** Use the `\'` escape sequence. Most strings don't contain quotes. When they do, `\'` handles it cleanly:

```
msg = 'he said \'hello\' to me'
```

For heavy quoting (SQL, JSON, HTML), use interpolation or load from a file with `read[]`.


### No String Concatenation with `+`

Crow does not allow `+` to concatenate strings. String interpolation is the only way to combine strings.

**What was considered:** `'hello' + ' world'` producing `'hello world'`.

**Why it was rejected:** `+` is numeric. For combining pieces, interpolation (`` 'hello `name`' ``) shows the result's shape; for building from many pieces, collect parts and `join[parts, sep]` — the O(n) idiom every `+`-language teaches as best practice anyway; for a rare pairwise merge, `concat[a, b]` works on strings. Adding `+` as a fourth way violates "one way to do things".

**What to use instead:**

```
greeting = 'hello `name`'
full = '`first` `last`'
```
