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
baseline re-accepted once.

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
   correct when every column's type is known.

**First step.** Schemas through `select`, `filter` and `sort`, with a
parity program whose cells are typed without a database. *Built
(2026-09-25):* the type inference already carried a variable's columns
through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack` and
`join`; `group` now gives its key and a float per aggregate, and a frame
handed to a function keeps its columns there when every call agrees
(`param_frame_schemas`, found at the call sites each round and put in
view when the body is read and lowered). Parity 89. Decisions 1 and 3
(unknown cells dynamic; a nullable column's cells fallible) come next.

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
element-wise operators and `*`, with a unit test for the refusal.

## 4. Order

Typed frames' first step comes first, because it unblocks the lowering's
checks. Then capabilities' first step, a report that needs no syntax.
Then shapes. Fusion waits for the interpreter's typed values.
