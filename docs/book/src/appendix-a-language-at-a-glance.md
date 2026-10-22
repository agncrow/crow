# Appendix A: The Language at a Glance

The language on one page: every keyword, every operator, every form, and
the four names every value carries. The names of the standard library — core
builtins, module functions, elements, and layout attributes — are in
[The Standard Library](stdlib/index.md), one page per module, generated
from the compiler's own tables with an example for each. The commands are
in [The Toolchain](ch01-01-toolchain.md).

## Keywords

```text
fn  view  struct  use  test          declarations
if  else  for  in  while             control flow
return  break  continue              flow exits
true  false                          literals
eq  ne  gt  lt  gte  lte            comparison (words, not symbols)
and  or  not  then                   logic and chaining
in  not in                           membership: an item, a part, a key, a mask over a column
```

There is no `class`, `lambda`, `async`, `await`, `try`, `catch`,
`match`, `elif`, `null`, or `import` — each reflex has a Crow form
(Appendix B).

## Operators

```text
+  -  *  /  %  ^        arithmetic (/ floors between ints; ^ is power)
.*  ./  .^              element-wise on tensors
x then f[a]             pipe: f[x, a], chains left to right
    then g[x]           a chain as lines: x = …, then one step a line;
                        a step that names x reads it where it says
`expr`                  interpolation (inside 'single-quoted strings')
---                     comment to end of line
```

No `==` `!=` `<` `>` `&&` `||` `!` `+=` or ternary — every one is a
teaching error naming the Crow spelling. Parallel assignment exists:
`a, b = x, y`.

## Forms

One line for every shape the parser knows. Blocks are indented; there
are no braces, semicolons, or parentheses for calls.

```text
x = 1                      bind (rebinding is fine; values never mutate in place)
_kept = rm[tmp]            a value kept unread on purpose: crow check asks nothing of a name beginning with _
a, b = x, y                parallel assignment
s.field = v   xs[i] = v    field and index assignment (a fresh value, not a mutation)
fn name[a, b = 1.0]        function; defaults are literals, callers may name them
    return a * b
view name[a]               a view: elements are functions inside it, and it sees reactive state
struct Name [a, b]         a record; create with Name[1, 2], read with n.a
test name[x]               a scenario; params are generated inputs; a bare fn's test needs no name
use math, frame            import; use math as m aliases; m.f[...] always works
if c ... else ...          branch (an error value in c is itself an error)
for x in xs                loop — a parallel expression when its body is pure
ys = for x in xs           collect: the body's last expression is the element
    x * x
while c                    loop
cpu[4]  gpu[]              resources: where the program runs
f[a, b]  f[a, k = v]       call, positional then named; m.f[...] for a module
xs[0]  xs[-1]  s[i]        index (strings by character); slice[xs, a, b]
[1, 2, 3]  map[k = v]      array; map (keys are names or 'strings')
'text `expr` more'         string: single quotes only, backticks interpolate
x then f[a] then g         pipe: g[f[x, a]]
```

Values: int, float, string, bool, array, tensor (`use math`), map, struct,
function, and every one of them carries the four fields below. A
function sees its parameters and other functions — never a module
variable; views and event handlers are the one exception, for reactive
state.

## Error metadata — on every value

```text
.ok  .reason  .kind  .trace
```

Errors are values; they propagate through use and have no truth value.
These four names are consequently forbidden as struct fields.

The size of the language is governed by the vocabulary rule — every
addition needs a deletion or an impossibility proof — so this page is
expected to grow slowly and shrink occasionally; when it disagrees with
the compiler, the compiler is right and this page has a bug.
