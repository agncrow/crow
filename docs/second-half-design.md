# The second half: capabilities, typed frames, shapes

*Proposed 2026-09-24, as step 6 of the rewrite (`ir-design.md` §8). Each
of the three is its own feature with its own gates. This record decides
what each is for, what its first step is, and what waits. Decisions open,
each with a recommendation.*

## 1. Capabilities

**What for.** A part reaches outside its values: a database, files, the
network, the clock, a shell, mail. Today that reach is visible only by
reading the code. The IR already knows it: `Fn.effects` is every effect a
function reaches, transitively (`ir.rs`, `EFFECTS`). Capabilities make
the reach declared, checked and substitutable. A reviewer sees in one
line what a part can touch, `crow check` refuses a part that reaches
further, and a test binds a double where the app binds the real thing.

**What exists.** A module function needs its import (`use db` before
`query`), so part of the reach is already declared. The core effects
(`read`, `write`, `shell`, `now`, `env`, `fetch`, `mail`, `sleep`, `exit`)
need nothing. The harness already substitutes doubles for the page
(`app`, `click`, `change`), and `change[a, 'now', t]` sets the clock.

**Decisions.**

1. *Declared, or derived and recorded.* Declaring needs syntax: a line
   per part, and a spec section on binding. Recording needs none: the
   baseline (`app-baseline-design.md`) already records what each declaration
   does and shows a change as a diff to accept, so it can record each
   part's reach the same way. *Recommendation:* derive and record first.
   `crow check` prints each part's reach, the baseline stores it, and a
   part that starts writing files or calling a shell is a change the
   author accepts. That delivers the review and the check without
   syntax. Declaration waits until a test needs a double the harness
   cannot give.
2. *Doubles.* When a declaration comes, binding follows the harness: a
   test binds a capability the way it sets the clock today, by a verb on
   the app (`bind[a, 'db', fake]`), not by new syntax in the part.
   *Recommendation:* no binding syntax in parts, ever. The one line a
   part gets is what it needs, and the app or the test binds.

**First step.** Each part's reach in `crow check` and the baseline,
derived from `Fn.effects`, with a harness test and the task board's
baseline re-accepted once. *Built (2026-09-26):* each file's reach is
the effects its functions, derivations (`Derive.effects`) and top level
(`Part.main_effects`, by the part a statement declares for) reach,
transitively, grouped by capability, recorded as `reach <file>` in the
baseline (`baseline::reach_entries`) and printed by `crow check` one line
a file; a file that touches nothing says so. `crow check` now reads it
from the one lowering it refuses by. The task board's six files and the
parts test's were recorded (`crow-check.md`, "The baseline").

## 2. Typed frames

**What for.** A frame's columns have types. `describe` knows them for a
query against a database the compiler can reach, and a literal knows
them for a frame built in the program. Elsewhere a cell is typed as text,
a guess. The guess is why the lowering's checks cannot read the
lowering's types yet (`core-rewrite-notes.md` §10, the third step), and
why a compiled program reads a number cell as a string when no database
was described.

**What exists.** `Ty::Frame(Option<Vec<(String, Ty)>>)` carries a schema
when one is known. `describe` fills it for literal queries, and a cell of
a described column has its type. Frame operations (`select`, `filter`,
`sort`, `join`, `group`) drop the schema: their result is a frame of
unknown columns.

**Decisions.**

1. *A cell of an unknown column.* Text is a guess that is sometimes wrong
   and never checked. `Unknown` is honest, but a compiled program must
   still hold the cell in some Rust type. *Recommendation:* a cell of an
   unknown column is `Json` (dynamic until read through `int[]`,
   `float[]` or `str[]`, as a parsed document's values are today), so no
   type is a guess. The checks can then read the lowering's types, and a
   program that assumed text says so with `str[]`.
2. *Schemas through operations.* *Recommendation:* each frame operation's
   result type is computed from its arguments' schemas: `select` keeps
   the named columns, `filter` and `sort` keep all of them, `join` unions
   the two, and `group` gives the keys and the aggregates. The lowering
   types these the way it types every node (the second step of the fold).
3. *A missing cell.* Read one at a time, a missing cell is its
   column's placeholder in both modes (`NaN`, `nan`, `0`, `false`; the
   spec, frame), because the IR types a cell as its column's type. For
   a number that is honest, since NaN compares as false as a filter does;
   for an int or a bool it is a guess of the kind the error rules refuse
   elsewhere. *Recommendation:* when frames are typed, a column the
   schema says can be missing (a nullable database column, a left join's
   right side) types its cells `Result`, so a missing one is an error in
   both modes and `.ok` guards it; a column that cannot be missing keeps
   plain cells. Until then the placeholder stands, and parity 65 holds
   both modes to it.
4. *Fusion.* A relational sublanguage in the IR (`filter`, `select`,
   `group`, `join`, `window`, `sort` over typed columns), fused into one
   pass by the Rust backend, with `derive` as the lazy boundary.
   *Recommendation:* after the first two above, and after the interpreter's
   typed columns (`typed-values-design.md`), because a fusion is only
   correct when every column's type is known. *Not built (2026-10-01):*
   the code there is chains almost no frame operations; what is fused is
   a filter's predicate (the end of §3). *The relational vocabulary grew
   instead (2026-10-02):* `group` by several keys, `pivot`, and `window`
   (running and trailing aggregates, lags, leads and ranks within
   partitions), each one runtime function both modes call, the keys and
   the window's cells typed in the frame's type (spec, frame).

**First step.** Schemas through `select`, `filter` and `sort`, with a
parity program whose cells are typed without a database. *Built
(2026-09-25):* the type inference already carried a variable's columns
through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack` and
`join`; `group` now gives its key and a float per aggregate, and a frame
handed to a function keeps its columns there when every call agrees
(`param_frame_schemas`, found at the call sites each round and put in
view when the body is read and lowered). Parity 89. *Decision 1 built (2026-09-25):* a cell of a column nothing
typed is `Json` in both modes — the interpreter's missing one JSON's null,
the compiled program's cell read with `get_json` — so arithmetic on it is
refused until `int[]`, `float[]` or `str[]` says what it is (parity 90;
the compiled program's JSON null, shown and tested as the error value the
spec says it is, came with it). *Decision 3 built (2026-09-25):* a column the schema says can have
missing cells has `Result` cells in both modes — the interpreter's missing
one an error of reason `missing`, the compiled program's a `CrowResult`
made from the column's validity. A database column's nullability comes
from the statement and the tables' declared NOT NULLs
(`sql_types::select_nulls`: outer-join sides, SQL's rules for expressions
and aggregates, the columns a WHERE compares), or from ClickHouse's
`Nullable`; the schema record keeps it as `float?`. A join's unmatched side
is typed the same way, and `fill` makes a column's cells plain (parity 65,
parity 92); a mask built from fallible cells is refused, as a decision on
values that can fail. A test's program now asks its databases too
(`ir_interp::prepare` had describing off, so a test read every query cell
as dynamic). Moving finch across found a handler's specialized copies
lowered as plain functions (their fact writes lost) and an int handed to
a float parameter left unconverted in the compiled program, both fixed.

**Columns in the type (2026-09-29, Vijay: "implement the full version:
frames get a new binding on each reassignment").** Until now a frame's
columns lived beside the types, in maps keyed by a variable's name
(`frame_schemas`, `fn_frame_schemas`, `param_frame_schemas`), computed on
the syntax tree (`frame_schema_of`, which typed `with`'s new column with
the tree's own typer). The IR's `Ty::Frame(Option<…>)` was never filled,
a function's frame parameter kept its columns only when every call
agreed, and a column the program misnamed stopped it as it ran.

- *The type.* A frame's type is its columns, `Frame{price: Float, n:
  Int?}`, each column's cell type, `?` where a cell can be missing. Where a
  column is named by a variable (`with[df, col, xs]`) the type keeps the
  columns it knows and is *open*, `Frame{price: Float, …}`: a column it
  cannot name may be there. Where nothing is known (a file loaded as the
  program runs, a query no database described) it is `Frame`, all open.
- *A reassignment binds anew.* A frame variable's type is its last
  assignment's: after `df = with[df, 'x', xs]` it is `Frame{…, x: Float}`
  from that line on, and before it the frame it was. In the block the
  variable was first bound in, a reassignment that changes the columns is
  a new `Let` of the name in the IR (Rust shadows a `let` the same way).
  Inside a block within it (an `if`'s, a loop's) it is the variable's
  assignment, and after that block the variable has the columns every way
  through gives it: the columns all paths share, their types joined, open
  where a path's columns differ. A loop's body reads, at its top, what the
  loop's entry and its end both give. A fact, which a view may read at
  any time, has the columns every assignment to it shares.
- *Operations* compute their result's columns from their arguments'
  types, by the rules the side map had: `filter`, `sort`, `head`, `tail`
  keep the columns; `select` gives the named ones (closed); `with` adds or
  replaces one, typed by the value it is given; `fill` makes a column's
  cells plain; `stack` keeps the first's, `?` where any side's can be
  missing; `join` the left's and the right's but the key (`_right` for a
  name both have), `?` on the side a row can miss; `group` its key and a
  float per aggregate; `rename` the columns renamed; `new[]` none (closed);
  a described query, `doc.chapters` and `chart.macd` theirs.
- *Functions.* A frame parameter's columns are its argument's. A function
  called with frames of different columns is a specialized copy per set
  of columns (the monomorphization every other type gets), and a returned
  frame's columns are the function's result type. So what a function
  reads of a frame, `df.amount`, is checked against the columns each call
  hands it.
- *What it catches.* A column a frame is known not to have — `df.x`, or
  `'x'` named where a column is taken — is refused before the program
  runs, in both modes, when the frame's columns are known and closed. An
  open or unknown frame's column is checked as it runs, as before.

*Built (2026-09-29; since 2026-10-07 the one type is `crate::ir::Ty`, so read `Ty::Frame` and `Ty::Tensor` for the names below).* `CrowType::Frame(Option<Arc<Cols>>)` and
`Ty::Frame(Option<Box<FrameCols>>)`; `lower_ir/frames.rs` computes each
operation's columns from its arguments' types (`frame_typed`), rebinds a
frame local at each assignment (`rebind_frame`, a new `Let` in the block
it was bound in), joins at the end of an `if` and a loop (`join_frames`,
a loop's body read once first where it rebinds a frame), and refuses a
column a closed frame lacks (`refuse_column_read`, `check_columns`).
`frame_schemas`, `fn_frame_schemas`, `param_frame_schemas` and
`frame_schema_of` are gone, and with them the last use of the syntax
tree's expression typer (`infer_expr`, deleted). Two kinds of unknown are
told apart: a frame whose columns inference has not worked out yet
(`Frame(None)`, which yields to what a later round learns, and makes no
specialization) and one whose columns show only as it runs (none known,
open: `load`, a query no database described). A value a guard knows has
failed (`if not x.ok` … `return x`) hands back its failure, not its
frame's columns. The generated Rust is byte-identical over the corpus but
for the new bindings (`let f = …` where it was `f = …`) and a frame read
inline (`with[g, 'day', 'today'].day[1]`, now read as the text it is).

Moving across found what the side map had hidden: it kept a branch's
columns after the branch, as though the branch had run (a column only one
branch adds read as present); it typed a column read only through a
variable; and a table's click handler took its key's type in the first
round of inference, before the frame's columns were known, and kept
text. Parity 123.

## 3. Shapes

**What for.** A tensor's rank and dimensions, when the program says them
(`zeros[2, 3]`, a nested literal), in its type. Then a mismatched
element-wise operation (`a + b` of `[2, 3]` and `[3, 2]`) is refused
before the program runs, and a chain of element-wise operations can be
fused.

**What exists.** `Ty::Tensor(elem)`: the element type only, with rank
known at run time. Shape errors are runtime errors in both modes.

**Decisions.**

1. *How much shape.* Rank only, or rank and literal dimensions.
   *Recommendation:* rank and literal dimensions (`Tensor(Float, [2, 3])`,
   with an unknown dimension where only rank is known), because rank alone
   catches few real mistakes. Every dimension a program computes stays
   unknown, and nothing is ever inferred by search.
2. *What is refused.* *Recommendation:* only what is certainly wrong: two
   known dimensions that differ where an element-wise operation or a
   matrix product needs them equal. Everything else stays a runtime check.

**First step.** Rank and literal dimensions for literals and the
constructors (`zeros`, `ones`, `eye`, `rand` of a shape), carried through
element-wise operators and `*`, with a unit test for the refusal. *Built
(2026-09-26):* the lowering works out a tensor's shape from what the
program states (a literal's nesting; `zeros`, `ones`, `full`, `rand`,
`randn`, `eye`, `linspace` given literal sizes; `transpose`; a local as
last assigned, forgotten where a later value's shape differs or a part
of it is assigned), through `+`, `-`, `.*`, `./`, `.^` by the runtime's
broadcasting and through `*` as the matrix product, and refuses the
certainly wrong with the message the program would stop with
(`lower_ir/shapes.rs`). The shape lives in that analysis, not yet in
`Ty::Tensor`: carrying it in the type waits for fusion, which needs it
there. Found on the way, and fixed: a broadcast mismatch the program
computes panicked the interpreter (the runtime's `broadcast_shape`)
while the compiled program reported it; both now stop with the words
the refusal uses (`CrowTensor::try_elementwise`; the compiled program's
`crow_stop`, which the matrix product's mismatch now uses too), and
`crow test --parity` counts two modes stopping alike as agreeing
(parity 96).

*In the type (2026-09-29):* the shape moved into `Ty::Tensor(elem,
dims)` (`CrowType::NumArray`), so it goes where the tensor goes: into a
function's parameter, as the dimensions all its calls agree on (joined as
types are: a dimension two calls disagree on is unknown, and nothing
unknown is refused), out of its result, through `reshape` given literal
sizes, and across a reassignment, which binds the local anew as a
frame's columns do. The rebinding and the joins where ways through meet
that typed frames built now serve any type with a detail, a frame's
columns or a tensor's shape. A function is not copied per shape, as it
is per set of a frame's columns: every tensor compiles alike, so a copy
would buy only earlier refusals (spec, Design Note "a function is one
function whatever shapes it is handed"). A refusal inside a function
names the parameters the tensors were handed as. Found on the way:
`transpose` had been read as reversing every dimension, where the
runtime swaps the last two, so a rank-3 tensor's transpose met by an
element-wise operation was refused though it runs (parity 129).

*Fusion of element-wise chains (2026-10-01; Vijay: finish the compiler
work).* A tree of `+`, `-`, `.*`, `./`, `.^`, `*` and `/` between a tensor
and a number, and negation, whose value is a tensor, with two operations
or more and operands that only read, is one call of
`crow_runtime::fused::run` (`lower_ir/fuse.rs`, a pass after the names
are written; the row `tensor.fused`, whose operands are a rest of tensors
and numbers). The kernel checks each operator's shapes in the operators'
order, with their words, then computes the answer block by block — 4,096
elements, across the cores from 65,536 — each element with the operators'
own formulas (a tensor over a number multiplies by its reciprocal; a
number minus a tensor adds the number to the negation), so the answer is
theirs to the bit; a tensor on the GPU, whose operators compute in f32
there, is left to the operators. A variable read twice is one operand.
The kernel is tested against the operators on 3,000 generated chains
(shapes broadcast, strided tensors, zeros and signs) and on large ones
across the cores; `tests/fusion.rs` runs programs fused and with
`CROW_NO_FUSE=1` in both modes and compares every line. The benchmark's
chain (`x * 2.0 + x .* x - 1.0`, ten million elements) takes 6 ms in each
mode where the operators took 34–38 ms.

*Fusion of a filter's predicate (2026-10-01).* The relational
sublanguage of §2's item 4 — `filter`, `select`, `group`, `join` and
`sort` fused into one pass, with `derive` as the lazy boundary — is not
built. A survey of the Crow code there is (finch, the examples, the
packages, the book) found 58 filters and two chains of frame operations
(`sort[filter[…]]`, `head[sort[…]]`), so a pipeline would fuse almost
nothing, and a lazy `derive` would move where a derivation's errors show.
What the survey found instead is the predicate: `filter[df, df.age gt 50
and df.name ne 'bob']` read each column into a list — a value per cell in
the interpreter — and made a mask per comparison and per `and`. A filter
whose predicate compares its frame's own columns with values that only
read, asks a column's `.ok`, and combines those with `and`, `or` and
`not` is now one call of `crow_runtime::mask::filter_where` (the row
`frame.filter_where`, in the pass that fuses tensor chains), which finds
each column and checks each comparison's kinds in the operators' order,
then decides the rows block by block and gathers them once. Settling it
found the masks themselves disagreeing between the modes: a missing cell
and NaN under `ne`, `eq` as nearness within 2⁻⁵², text parsed as numbers
compiled, an order of text answered compiled and stopped interpreted, a
column of bools comparing as nothing, and six forms that did not build
compiled (a list of text or of bools against a value, a value written
before its column, `eq` of a number with text, a grown list of ints).
A mask's cell is now decided by one function both modes call
(`crow_runtime::mask::holds`), and an order between kinds with none is
refused before the program runs where both kinds are known (spec,
Comparison and Boolean Masks; parity 149). The kernel is tested against
the unfused masks on 2,000 generated frames and predicates (missing
cells, NaN, ints, text, bools and moments, and a column the frame
lacks), and `tests/fusion.rs` runs parity 149 and 65 fused and unfused in
both modes. On a million rows a filter of three comparisons takes 10 ms
interpreted and 12 ms compiled, where the unfused masks took 83 ms and
35 ms.

## 4. Order

Typed frames' first step comes first, because it unblocks the lowering's
checks. Then capabilities' first step, a report that needs no syntax.
Then shapes. Fusion waits for the interpreter's typed values. *Fusion
built (2026-10-01):* element-wise tensor chains and a filter's predicate
(§3).
