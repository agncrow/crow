# Bug classes, removed by construction

Building finch in Crow found its bugs one at a time. Each fix came with a
test, but the next bug of the same kind waited in the next module. This
document records the fixes that remove a whole class instead (design rule
12: the modes agree by construction, not by discipline): what each class
is, the structure that closes it, what the structure found on its first
run, and what is left.

The four classes, in the order they were taken:

1. A failure means different things in the two modes.
2. The lowering decides a type wrongly, and the mistake surfaces late.
3. The runtime panics where it should stop.
4. A builtin is implemented twice.

## 1. A failure stops both modes the same way

**The class.** A program that indexes past its array, reads a key its map
lacks, or divides an int by zero cannot go on. Each mode handled such
failures its own way, and so did each build. Probes of thirty failure
paths (2026-09-27) found:

| Failure | `crow run -i` | `crow run` | `crow build` |
|---|---|---|---|
| index out of range | stop, its own words | panic hook, other words, no line | same as `crow run` |
| negative index past the start | stop | the index printed as a huge unsigned number | same |
| key a map lacks | stop, `key 'b' not found in map` | stop, `map has no key 'b'` | same |
| int divided by zero | an error value, the program went on | stop | stop |
| int past 64 bits | wrapped silently | stop | wrapped silently |
| `min_int / -1` | a Rust panic | stop | stop |
| `2 ^ -1` | `0.5`, where both modes type the value an int | overflow | overflow |
| `range` near the last int | looped until memory ran out | overflow | wrapped |
| stop inside a function | named the caller's line | named the function's line | same |
| stop inside a parallel loop | named the loop's line | one message per failing thread | same |
| `assert[c]` false | `assertion failed` | the generated Rust expression | same |
| deep recursion | Rust's abort message | Rust's abort message | same |

**The fix.** Every such failure is one check in `crow_runtime::stops`,
called by both modes: `item`, `char_at`, `row` and `key` for indexes,
`add`, `sub`, `mul`, `div`, `rem`, `pow`, `neg` and `abs` for ints, and
`no_column` and `size` for their words. Each returns the reason, so the
interpreter stops with it on the line it is running, and the compiled
program stops through `halt(line, reason)` on the line the emitter knows.

- Both print `runtime error: line N: <reason>` and exit with status 1.
- A stop off the main thread travels as a `Stop` payload to the thread
  that joins it. A parallel loop reports its earliest failing item once,
  in both modes, because rayon joins left before right and the
  interpreter's chunks are read in order. The live runtime reads a stop in
  a handler as the handler's (`panic_reason` uses `panic_words`).
- The interpreter attaches the line where a stop happened: in a function
  body, a derivation, a parallel loop's body.
- `overflow-checks = true` in the release profile of crow itself and of
  every generated program, so no build wraps an int that the checks miss.
- A panic that is not a stop — a check Crow is missing — is caught.
  - The interpreter runs every entry point under `guarded` (run, serve,
    render, handler, test, scenario, script, REPL), so the panic becomes
    a stop on the line it happened on.
  - The compiled program's `main_guard` and `install_hook` name it in the
    same words (`panic_words`).

The spec records the stops and their words: §10, When a Program Stops,
with the Design Note on why these failures stop rather than fail as
values.

**Tests.** Parity programs 97–104, one per stop (the harness counts two
modes stopping with the same last line as agreeing), and the unit tests
in `stops.rs`. `crow test --parity <words>` runs the programs whose names
hold the words.

**Measured.** The benchmark's `collect` rows, before and after, both
modes. Compiled (release): within noise on every row. Interpreted:
`for_collect` 82 → 58 ms, because each binary operator had formatted a
trace string it never used.

**Left.** Deep recursion still aborts with Rust's message in both modes.
A depth guard needs a limit both modes share, and the compiled program's
guard would cost on every call of a recursive function (the lowering
knows which ones are).

## 2. The IR is checked before a backend reads it

**The class.** The lowering decides every type once, and the backends
trust it. When it decided wrongly, the mistake surfaced late and far
away. The generated Rust did not build: `len[[]]`'s element type, a local
declared `()`. Or the interpreter carried a value its type said it could
not hold: an int in a float fact.

**The fix.** `ir_verify::verify` walks every body of a lowered program
with the bindings in scope, and checks that the parts agree:

- every local read is bound, and read at its binding's type;
- every value fits the slot it is stored in: a `let`, a `set`, a fact, a
  list's items;
- a decision is made on a boolean, and a loop walks something with items;
- a call hands a function as many arguments as it takes, each fitting its
  parameter, and an argument that can fail is handed on as the call's
  error;
- a call of a builtin's row hands each parameter the kind it takes (§4:
  a list of numbers or a column is converted to the tensor a parameter
  takes);
- a return fits the function's type;
- a conversion converts what it says;
- a render (a view or a derivation) calls nothing that writes facts.

It runs at the end of `lower_checked`, the one gate every pipeline passes
(run, run -i, build, test, check, the REPL, the layout engine). What it
does with a finding depends on `CROW_IR_VERIFY`:

- `strict` refuses the program as an internal error, a bug in Crow and
  not in the program.
- `report` prints each finding and goes on.
- `off` does not check.

When the variable is unset, a debug build is strict and a release build
is off. `cargo test` therefore checks every program a test lowers (1,052
tests), and a harness can run the release binary strict. A user's release
build pays nothing.

**Found on its first run.** Over the corpus (every tracked `.co` file,
every book block, finch: 1,053 files) the first version reported 442
findings. Most were conventions both backends already keep, which the
verifier now states:

- a view's tree placed where markup goes;
- a view rendering a failed argument as its text;
- a handler's value, which goes to nobody;
- `0` as a recursive link's empty sentinel;
- a query's parameter list holding values of every type;
- a loop over a value that can fail, refused where it runs;
- a failed value stored into a fact, which stops the handler;
- the original of a specialized function, which neither backend runs.

The rest were bugs, each fixed with a test:

- **An int default of a fact that holds floats** stayed an int in the
  interpreter until a handler stored a float, where the compiled program
  converted it. It is converted in the lowering now.
- **An int returned by a float function** was handed to the interpreter's
  caller as an int (the layout engine's `lay_lines`). It is converted in
  the lowering now.
- **A function whose returns are all of unknown type** was typed as
  returning nothing. A part built alone knows nothing of its parameters,
  so finch's `sim/autonext.co` alone declared `let e: ()` for a value, and
  a compiled build of the part would not have built. Such returns are now
  unknown.
- **Numbers held as a tensor, handed to a parameter that holds them as a
  list** (or the reverse) were not converted at the call. The generated
  Rust did not build. The lowering now converts where they meet, as it
  already did for `concat`.
- **A view calling a function that assigns a fact.** The interpreter
  wrote the fact mid-render, and the compiled program assigned a variable
  it never declared and did not build. It is refused in both modes
  (decision 2).

The corpus has no findings now.

## 3. No crash in the runtime

**The class.** A runtime function that panics instead of returning its
reason crashes the interpreter, or reaches the compiled program's hook
with no line.

**The fix, in two layers.**

- **The net** (§1): nothing in either mode crashes. A panic anywhere under
  a run is a stop, with the line where it happened (the interpreter) or
  its words (the compiled program).
- **The rule:** `crow_runtime`, `crow_db` and `crow_web` deny `unwrap`,
  `expect`, `panic!`, `unreachable!`, `todo!` and `unimplemented!` outside
  their tests. `cargo clippy --workspace --all-features` runs in CI and
  the gate and refuses a new one. The exceptions are allowed at the site,
  each with its reason:
  - `stops::halt`'s `panic_any`, which is how a stop travels off the main
    thread;
  - eleven math functions whose failures on a user's argument become
    their reasons when they move to the table (§4). These are `#[expect]`,
    so each warns once it is converted and the exception can go.

**Found on the audit** (61 sites at the start), each fixed:

- **CSV type inference** decided a column's type from its first hundred
  values and then read the rest as that type. An int column read `n/a`
  as 0, a float column read it as NaN, and a datetime column panicked. The
  whole column decides now, and a value that does not parse makes the
  column the next type.
- **`group` with a misspelled column** panicked a worker thread. It is
  refused with the column's name before the work is shared out.
- **A tensor on the GPU** panicked in every operation with no shader
  (`t[0]`, a broadcast, `diag`). Such operations read a downloaded copy
  now.
- **`tan` of a GPU tensor** asked the GPU for a shader it did not have,
  and panicked. The GPU's operations are enums now, so no operation can be
  named that has no shader; `tan` has one.
- **`to_gpu` on a machine with no GPU** panicked, where the spec says a
  tensor goes to the GPU "when a GPU is available". It stays on the CPU.
- **`fmt`'s kernel** panicked on a misuse the interpreter's adapter
  checked first. The kernel checks and answers the reason, and both modes
  stop with it.
- **Locks poisoned by a panic elsewhere** failed every later use. They
  recover their data now (`crow_runtime::lock`).
- The distributed runtime's network failures, a GPU that cannot hand
  results back, a queue's semaphore, a worker thread's panic, and the
  Arrow encoder now stop with a reason or pass the panic on as it was.

**Left.** The `assert!`s that check a user's arguments in the tensor,
linear algebra, signal and sparse kernels (shapes, axes, lengths: about
forty) are not caught by these lints. They move with their functions into
the table (§4), whose runtime functions answer `Result`.

## 4. One implementation per builtin

**The class.** A builtin whose interpreter adapter and compiled printing
are written separately drifts apart. Examples:

- `zeros[-2]` and `eye[-1]` stop the interpreter with a reason. The
  compiled program printed `capacity overflow` and `integer overflow in a
  multiplication`, because only the interpreter's math adapter checked
  the size.
- `fmt` checks `align` and the string-only options in the interpreter's
  adapter, while the compiled path relied on the kernel panicking.
- A bad time style is an error value interpreted, and the literal text
  `[fmt: …]` compiled.
- `calendar` fails as a value interpreted, and prints to stderr and
  answers an empty map compiled.

In all, 66 core builtins and 155 module functions (`math` 102, `frame`
18, `stats` 13, `nn` 8, `db` 6, `doc` 6, `io` 2, and `chart`) are
written twice: `builtins.rs` beside `emit_rs/builtins.rs`, and each
module's `call` beside its `emit_rust`.

**The design** (ir-design.md §3.5, typed-values-design.md §2 decided
the shape: "a builtin is a runtime function and a row naming it"):

- **One runtime function per builtin**, taking typed arguments (`i64`,
  `f64`, `&str`, `&CrowTensor`, `&[i64]`, …) and answering `Result<T,
  String>`. Its validation (sizes, shapes, axes, column names, offsets,
  styles) lives there and nowhere else.
- **One row per builtin** in the table. The row gives the Crow name, the
  module, the runtime path, the parameters' kinds, the answer's kind, and
  what a failure is: a stop (a misuse no value can carry, §1), an error
  value of a kind (`'time'`, `'parse'`, `'io'`), or impossible.
- **The interpreter's adapter is generic.** Each parameter kind reads
  its argument from a value (a float from an int or a float, a tensor
  from a tensor or a list of numbers), and each answer kind builds a
  value. So a row's call is the function applied to the read arguments,
  with its failure dispatched as the row says.
- **The emitter's printing is generic.** Each parameter kind prints its
  argument the way the function takes it (`(x) as f64`, `&*s`, `&t`).
  The failure prints as the row says: `.unwrap_or_else(|e| crow_stop(line,
  e))`, or a `CrowResult` of the row's kind.
- **A guard against drift:** a test that every builtin and module
  function the lowering accepts is a row, or is on the short list of
  names with syntax or state of their own. That list covers `log`,
  `type`, `grad`, `later`, the predicates that take a function, and the
  constructors of ui, web and db. A builtin added outside the table fails
  the test.

The two modes then agree by construction for every row. The only code
per mode is the adapter and the printer of each parameter kind, and
those are tested once per kind rather than once per builtin.

**The order.** `fmt`, `calendar` and the time builtins first (they
diverge today), then `math` (it diverges and is the largest), then
`frame`, `stats`, `nn`, `io`, `doc`, `chart`, and the remaining core
builtins. Each batch is gated by parity (one program per batch pinning
the edges both modes must now share), the book, the served pages and
the oracle.

**Built** (`src/table.rs`):

- The row type, the reading and printing of each kind, and a macro that
  makes the interpreter's call from the same tokens the emitter prints.
- A size kind for a rest of arguments (`zeros[2, 3]`, `poisson[1.5, 4]`).
- The module registry consults the table before a module's own arms, so
  a module function with a row is its runtime function for the
  interpreter, the emitter and inference alike.
- The drift test: every core builtin is a row, has a form of its own
  (`OWN_FORM`), or is still to move (`TO_MOVE`, which only shrinks).

**Batch 1**: `moment`, `calendar`, `parse_time`, `repeat`, `matches` and
`sub`, with parity 105.

- `calendar` of a zone that does not exist is an error value of kind
  `'time'`, as the spec lists it. The compiled program printed to stderr
  and answered an empty map, and its type said it could not fail.
- An invalid pattern given to `matches` or `sub` is an error value of kind
  `'regex'`, as `find`'s is. The interpreter answered one where the
  compiled program stopped. The pattern's message is one line now.
- A compiled time error carried an empty trace, where the interpreter's
  names its place. Both name it now.

**Batch 2**: the math constructors and distributions (`zeros`, `ones`,
`randn`, `full`, `eye`, `linspace`, `arange`, `diag`, `normal`,
`uniform`, `poisson`, `bernoulli`, `gamma_dist`, `beta_dist`,
`exponential`), in `crow_runtime::math`, with parity 106.

- A negative size stops both modes with one reason. The compiled program
  had cast it to a huge `usize`.
- A distribution's parameter is checked once, where both modes panicked.
- `diag` of a rank above two answers its reason.
- A negative `std` for `normal` stops the program. Both modes had
  flipped the samples' sign.

**Batch 3**: linear algebra (`det`, `inv`, `svd`, `qr`, `cholesky`,
`eigen`, `norm`, `dot`, `pinv`, `lstsq`, `triu`, `tril`, `rank`, `cond`),
with parity 107.

- A vector, a rank-3 tensor, or a rectangle where a square is needed is
  the program's reason. A kernel had panicked.
- `cholesky` of a matrix that is not positive definite answers its reason
  (it panicked both modes).
- `pinv` of an empty matrix read its first singular value unchecked.
- The sparse product's shapes are checked (`s * v` panicked).
- A name with several rows takes the one its arguments fit, by count
  (`norm[t]`, `norm[t, p]`) and by kind.

**Batches 4 and 5**: reductions, sorting, selection and shapes (`sum`,
`mean`, `min`, `max`, `prod` with and without an axis, `argmax`,
`argmin`, `cumsum`, `cumprod`, `argsort`, `topk`, `unique`, `where`,
`reshape`, `flatten`, `transpose`, `clip`, `atan2`), and the element-wise
functions of a number or a tensor, with parity 108.

- An axis out of range, a reshape that does not fill its shape, `where`'s
  lengths, and a transpose of a vector are the reason. The kernels had
  asserted them.
- A number and a tensor share one rule. `sign[0.0]` was 1 compiled, from
  Rust's `signum`, and 0 interpreted.

**Batch 6**: the rest of math (signal, special functions, complex
numbers, draws, sparse matrices, `solve`, `stack`, `tensor`, `mod`), with
parity 109. Every math function but `load`, an effect, is a row.

- A rank, a window, a count or a sparse entry outside its matrix is the
  reason. The kernels asserted them, and a sparse entry panicked the
  sparse library.
- `tensor[df]` unwrapped the frame's conversion compiled, a panic where
  the interpreter stopped with the reason.
- Kinds for sparse matrices, frames, lists of text, lists of tensors
  taken as arguments, and no answer. Rows are chosen by a list of tensors
  against a tensor, and by a sparse matrix against a dense one.

**Batch 7**: `stats`, `nn` and `chart`, with parity 110 and 111. Every
function of the three modules is a row, and a module whose functions are
all rows is its name and its list. Probes of eight misuses found the
modes apart on six and both wrong on the other two:

- A list of numbers or a frame's column handed to a row's tensor
  parameter is converted by the lowering where it is handed over, the
  same way in both modes. Compiled, a list built by `push` did not build
  (the interpreter read it); a column's missing cell was read as `0` and a
  column of text as zeros, where the interpreter stopped. A missing cell,
  or a column of text or of bools, now stops both modes with one reason:
  `CrowColumn::numbers`, and the interpreter's reading of the column's
  cells by the same rule, since the spec makes no missing cell a guess.
  The verifier checks every argument of a row against its parameter's
  kind.
- A chart window of no rows stops both modes. A negative one had been a
  huge window compiled (every value `NaN`) and one row interpreted (the
  values themselves). Series of two lengths stop too: the kernels had cut
  them to the shortest, so the answer was shorter than the frame it went
  into.
- `anova` of one group was `NaN` compiled and refused interpreted.
  Shapes `mse` or `linear` cannot meet panicked a kernel, and the
  compiled stop named no line.
- Both modes answered garbage for a `crossentropy` target that is no
  class (a logit of `0`, or class 0 for a negative target) and a
  distribution `kstest` does not know (a reference CDF of `0.5`
  everywhere); `fit` answered an empty map. Two lengths where one is
  needed (`corr`, `cov`, `linreg`, `chisq`, `bce`) were `NaN`, and a
  percent past 100 was the largest value. Each is the reason now.
- Kinds for a map of floats answered, a rest of tensors (`anova[a, b,
  c]`) and a frame answered. A count no row of a name takes names every
  count the name takes (`rsi[] takes 2 or 4 arguments, got 3`).

**Batch 8**: `frame`, with parity 112. Every frame function but `load`
and `save`, which read and write files, is a row, in
`crow_runtime::frames` over the frame kernels. Probes of fourteen calls
found twelve apart or wrong in both modes:

- A column the frame lacks (`select`, `sort`, `rename`, `join`), a mask
  of another length (`filter`) and frames whose columns differ (`stack`)
  were error values interpreted, which the type (a frame) said could not
  happen, and stops compiled, with the function named twice. They stop
  both modes now, as `df.col` of a missing column does (spec: Design
  Note, a frame operation's misuse stops the program).
- An aggregation `group` does not know panicked compiled with no line
  (`invalid agg`), as `datetime` of text that is no time did (`invalid
  datetime`), where the interpreter stopped and answered an error value.
  The first stops both modes; `datetime` and `tz` answer an error value
  of kind `'time'`, as `parse_time` does, typed as able to fail.
- `tz` read an offset by two rules: `'+5:30'` was refused interpreted and
  taken compiled. `'+HH:MM'` is the one form.
- `head[df, 1.5]` was refused interpreted and one row compiled; it is one
  row in both, as every row reads an int. `head[df, -1]` was the whole
  frame in both; it stops.
- `unique[df.name]` was refused interpreted and `[0]` compiled. A count
  no row of a module function takes is refused before the program runs,
  for a qualified call and for a name only a module answers.
- A join type the kernel did not know (`'lefty'`) was an inner join. It
  stops, naming the four.
- A column copied with `with` lost its missing cells in both modes: the
  cells became `0`, `NaN` or `false`. It keeps them (`with_valid`).
- `fill`'s value was written as text by each mode's own display and parsed
  back; it is a typed cell of the column's kind now.
- The emitter turned every column handed to a module function into a
  tensor, its missing cells `0`. The lowering converts a column where a
  row takes a tensor, including for a name the core shares with math
  (`max[df.age]`), and hands `with` the column itself.
- Kinds for a mask, a cell, a column's values and a rest of lists of text.
  A list of bools had been read as a list of numbers when a row was
  chosen.

Left: a column whose every cell is missing reads as text when the
interpreter copies it (it has no cell to read the kind from); `save`
fails as a value interpreted and stops compiled (the effects batch).

**Batch 9**: the pure core builtins — text (`split`, `trim`, `replace`,
`upper`, `lower`, `starts`, `ends`, `join`), `min` and `max`, `hash`,
`fmt`, `int`, `float` and `rand` — with parity 113. `abs`, `sqrt`,
`floor`, `ceil` and `round` have a form of their own: a function `grad`
differentiates takes them on dual numbers. `str`, `json` and `encode`
(any value's display and JSON), the collections and the effects are still
written twice.

- A builtin handed a count or a kind no row takes is refused before the
  program runs, in the table's words, for a core row as for a module's:
  `min['a', 'b']` is `min[] takes an int and an int or a number and a
  number, got text and text`. The compiled program had compared the text.
- `fmt` with a time style that is none was an error value interpreted and
  the text `[fmt: …]` compiled, typed as text in both. A moment's time
  style is a row of its own now, typed as able to fail (kind `'time'`).
- `int` and `float` of a JSON value: compiled, JSON text was trimmed and a
  JSON bool refused. One rule reads a scalar now (`crow_runtime::cell`).
- Indexing JSON:
  - a value indexed twice did not build compiled;
  - a failed parse, indexed, read `null` compiled, where the interpreter
    handed the error on;
  - a `null` element was a good value compiled and an error interpreted.
- The index rule was written three times in inference, and the copies
  disagreed. A column's cell was text in one and dynamic in another, and
  two dropped the failure of a value that can fail, so a failed
  `io.lines` indexed read an empty list compiled. There is one rule now
  (`index_type`).
- Core rows are chosen by their arguments' kinds in all three places —
  the interpreter, the emitter and inference — as module rows are: `min`
  of two ints is an int, of a float a float.

**Batch 10**: a frame's column as numbers, everywhere, with parity 114.

- A column in arithmetic (`+ - * / % ^ .* ./ .^`) is its numbers, as it
  is when handed to a function that takes numbers: the lowering converts
  it the same way in both modes, and a missing cell stops. The
  interpreter had refused `df.a + df.b` outright; the compiled program
  had bridged it through a tensor with missing cells read as `0`, and a
  column times a number through a column method that parsed a column of
  text as numbers. Both are gone.
- A missing cell's error value holds its column's placeholder (the safe
  default a failed value holds, spec §10). So a column the interpreter
  copies keeps its kind even when every cell is missing: it had become
  text interpreted and stayed an int compiled.
- A list's failed item read where numbers are taken is the reason, never
  its placeholder.

## Decisions (Vijay, 2026-09-28)

The questions the work raised, each decided as recommended.

1. **A handler passes a failed value to a helper.** By the spec's
   auto-propagation, the helper does not run and the call's value is the
   error. A helper called for what it writes has its value dropped, so the
   error vanished and the handler committed its other writes: finch's
   `start_session` calls `clock.begin[start, finish - start, open_at]`,
   where `open_at` can fail, and the session would start without its
   clock. **Decided:** a helper call whose value is dropped, handed a
   failed value, stops the handler with the error's reason, as an effect
   given a failed argument does (`effect_refused`): nothing the handler
   did lands, and the reason is the session's notice. **Built:** the
   lowering refuses such an argument before the call (a statement, or a
   handler's last line, whose value nobody reads), as it refuses an item
   pushed onto a list. Before, the modes disagreed on the click itself:
   the interpreter's `click` answered the dropped error while landing the
   click's writes, and the compiled program's answered success
   (`tests/failed_helper_argument_harness.co`, run in both modes by CI).
2. **A function that assigns a fact's name, called from a view.** The spec
   says "ordinary `fn` declarations are fully isolated even under
   `use ui`" (the assignment is a local), and also "a function a handler
   calls that assigns a module-level variable is a handler too". The
   lowering classes any function assigning a fact as a helper, which
   writes it. **Decided:** refused, in both modes, as now: a render reads
   state and never writes it.
3. **Int division.** The spec said `3 / 2` is floor division, but both
   modes have always rounded toward zero (`-7 / 2` is `-3`, as in Rust, C
   and Go; Python floors to `-4`). **Decided:** toward zero, as the spec
   now says.
4. **A frame operation's misuse.** The spec's table of error kinds listed
   `filter`, `sort`, `group` and `join` as failing with error values of
   kind `'frame'`, which the interpreter answered; their type said they
   could not fail and the compiled program stopped. **Decided:** a misuse
   stops the program, as `df.col` of a missing column does (spec: Design
   Note, a frame operation's misuse stops the program); `load` and
   `with`'s values stay error values.
