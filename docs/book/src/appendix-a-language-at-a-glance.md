# Appendix A: The Language at a Glance

The entire language on one page — every keyword, every core builtin,
every element. The counts come from the compiler's own tables, so this
page is also the honest answer to "how big is Crow?":

| Layer | Size |
|---|---|
| Keywords | **25** (11 statement words + 13 word-operators + `_`) |
| Symbolic operators | **11** |
| Core builtins (always reserved) | **76** |
| Modules | **6** (`math` 102 · `frame` 16 · `math.stats` 13 · `math.nn` 8 · `db` 6 · `io` 2) |
| UI elements (inside `view` only) | **32** + `field` + `chart` |
| Event attributes | **4** (`click` `input` `submit` `change`) |
| Style vocabulary | **8 classes + 8 tokens** |
| Error metadata (on every value) | **4** (`.ok` `.reason` `.kind` `.trace`) |
| Test harness (test blocks only) | **5** (`app` `click` `change` `html` `elements`) |

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

## Core builtins — all 76, always reserved

A definition (variable, function, view, struct, parameter, loop variable)
may not take one of these names — the compile error includes the fix.
Struct fields, map keys, JSON fields, and frame columns are *never*
reserved: `row.type`, `account.sum` are always fine. Rationale: chapter 8
and the spec's Design Note "Names Are Never Ambiguous".

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
charts      chart  (an element: valid inside views, like table)
background  later  running
errors      error
io          read  write  exists  mkdir  ls  rm  mv  cp  args  input
            env  exit  shell  sleep  time
compute     cpu  gpu  nodes  grad
harness     test  (app  click  change  html  elements — valid only
            inside test blocks; see chapter 12)
```

Removed and teaching their replacements: `date`, `elapsed`, `today`.

## Modules (reserved only when imported bare)

```text
use math        102: tensor constructors (zeros ones eye linspace …),
                element-wise and trig fns, reductions (sum mean min max
                prod …), sorting, shape ops, linear algebra (inv det solve
                svd …), random, FFT and filters, special fns, complex,
                sparse, load. Constants pi e inf nan.
use math.stats  13: median std var percentile zscore corr cov ttest
                linreg chisq anova kstest fit
use math.nn     8:  relu sigmoid softmax gelu mse crossentropy bce linear
use frame       16: load save select filter sort group join stack head
                tail fill unique rename stats datetime tz
use db          6:  connect query exec insert transact migrate
use io          2:  csv lines
use web         route serve page respond redirect html text send recv
                close sse ws cors sessions cache limit uploads sign
                verify static_dir smtp + the test harness app click
use ui          fetch poll field (the elements below are NOT reserved —
                they exist only inside view blocks)
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

`chart[frame, x = 'ts', candles = [...], lines = [...], bars_y = '',
marker = 0, tz = 'UTC', height = 320]` renders a frame as SVG — the one
element whose content is data rather than text.

## Style vocabulary (the whole styling story)

```text
classes   card  stats  controls  primary  up  down  stale
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
crow stop                    stop the app
crow build                   compile       crow check      verify
crow fix                     apply fixes   crow fmt        format
crow test --parity|--book    infrastructure tests
crow lsp                     editor server crow launch N   distributed
```

Crowfile keys: `name`, `entry`, `db`, `cpu`, `gpu`, `deps`, `jobs`.

Two execution modes, one behavior: `crow run -i` interprets (instant
start), `crow run` compiles to Rust. `crow test --parity` and
`crow test --book` hold them to identical output.

---

That is the complete surface. The size is governed by the vocabulary
rule — every addition needs a deletion or an impossibility proof — so
this page is expected to grow slowly and shrink occasionally; when it
disagrees with the compiler, the compiler is right and this page has a
bug.
