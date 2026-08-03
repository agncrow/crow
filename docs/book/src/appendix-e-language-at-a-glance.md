# Appendix E: The Language at a Glance

The entire language on one page — every keyword, every core builtin,
every element. The counts come from the compiler's own tables, so this
page is also the honest answer to "how big is Crow?":

| Layer | Size |
|---|---|
| Keywords | **25** (11 statement words + 13 word-operators + `_`) |
| Symbolic operators | **12** |
| Core builtins (always reserved) | **75** |
| Modules | **6** (`math` ~200 · `frame` 36 · `stats` 27 · `nn` 16 · `db` 12 · `io` 6) |
| UI elements (inside `view` only) | **32** + `field` |
| Event attributes | **4** (`click` `input` `submit` `change`) |
| Style vocabulary | **8 classes + 8 tokens** |
| Error metadata (on every value) | **4** (`.ok` `.reason` `.kind` `.trace`) |
| Test harness (test blocks only) | **3** (`app` `click` `html`) |

Everything below is the complete listing.

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
(Appendix C).

## Operators

```text
+  -  *  /  %  ^        arithmetic (/ floors between ints; ^ is power)
.*  ./  .^              element-wise on tensors
a..b                    range
`expr`                  interpolation (inside 'single-quoted strings')
---                     comment to end of line
```

No `==` `!=` `<` `>` `&&` `||` `!` `+=` or ternary — every one is a
teaching error naming the Crow spelling. Parallel assignment exists:
`a, b = x, y`.

## Core builtins — all 75, always reserved

```text
output      log  fmt  type  assert
numbers     abs  sqrt  floor  ceil  round  min  max  rand
strings     split  join  trim  replace  upper  lower  starts  ends
            repeat  sub  matches
convert     int  float  str  json  encode  hash
collections len  contains  slice  push  pop  sort  reverse  concat
            range  map  keys  values  has  set  del
search      find  any  all
moments     now  moment  calendar  parse_time
freshness   captured
background  later  running
errors      error
io          read  write  exists  mkdir  ls  rm  mv  cp  args  input
            env  exit  shell  sleep  time
compute     cpu  gpu  nodes  grad
harness     test  (app  click  html — valid only inside test blocks)
```

Removed and teaching their replacements: `date`, `elapsed`, `today`.

## Modules (reserved only when imported bare)

```text
use math    ~200 exports: reductions (sum mean std var median …),
            tensor constructors (zeros ones eye linspace …), linear
            algebra (matmul inv det solve …), FFT, random, activation
            fns, autograd. Constants pi e inf nan.
use frame   36: load save select filter sort group join stack head
            tail stats fill rename bucket …
use stats   27: distributions, tests, correlation
use nn      16: layers, losses, optimizers
use db      12: connect query exec insert transact migrate …
use io      6:  file helpers beyond the core set
use web     serve page route respond redirect send recv close sse ws
            cache cors …
use ui      fetch poll field + the 32 elements below
```

`use math as m` reserves only `m`.

## UI elements — valid only inside `view` blocks

```text
div  span  p  h1  h2  h3  h4  h5  h6
button  a  nav  header  footer  section  form
ul  ol  li
table  thead  tbody  tr  td  th
input  textarea  option  label  field
pre  code  img
```

Event attributes: `click` `input` `submit` `change` — value is a
handler name or a deferred call (`click = set_speed[10.0]`, arguments
frozen at render). Other attributes pass through (`class`, `href`, …).

## Style vocabulary (the whole styling story)

```text
classes   card  stats  controls  row  primary  up  down  stale
tokens    --crow-bg  --crow-surface  --crow-text  --crow-muted
          --crow-border  --crow-accent  --crow-up  --crow-down
```

Everything else is the default stylesheet (zero code) or plain CSS in
`css/app.css` (the escape hatch). No CSS ever appears in `.crow` files.

## Error metadata — on every value

```text
.ok  .reason  .kind  .trace
```

Errors are values; they propagate through use and have no truth value.
These four names are consequently forbidden as struct fields.

## The toolchain

```text
crow new [--web --ml --db]   scaffold      crow run [-i]   execute
crow build                   compile       crow check      verify
crow fix                     apply fixes   crow fmt        format
crow test --parity|--book    infrastructure tests
crow lsp                     editor server crow launch N   distributed
```

Crowfile keys: `name`, `db`, `jobs`.

---

That is the complete surface. The size is governed by the vocabulary
rule — every addition needs a deletion or an impossibility proof — so
this page is expected to grow slowly and shrink occasionally; when it
disagrees with the compiler, the compiler is right and this page has a
bug.
