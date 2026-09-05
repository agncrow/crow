# Appendix A: The Language at a Glance

The language on one page: every keyword, every operator, and the four
names every value carries. The names of the standard library — core
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
_                                    discard
```

There is no `class`, `lambda`, `async`, `await`, `try`, `catch`,
`match`, `elif`, `null`, or `import` — each reflex has a Crow form
(Appendix B).

## Operators

```text
+  -  *  /  %  ^        arithmetic (/ floors between ints; ^ is power)
.*  ./  .^              element-wise on tensors
x then f[a]             pipe: f[x, a], chains left to right
`expr`                  interpolation (inside 'single-quoted strings')
---                     comment to end of line
```

No `==` `!=` `<` `>` `&&` `||` `!` `+=` or ternary — every one is a
teaching error naming the Crow spelling. Parallel assignment exists:
`a, b = x, y`.

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
