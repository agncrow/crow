# Crow — notes for Claude Code

Read `docs/design-rules.md` first: nine principles, the implementation and
documentation rules, and the feature workflow. The spec and the code cite
them by number. The short version:

- **One way to do things.** Before adding syntax, an operator, or a
  builtin, search `docs/language-spec.md` for an existing way; if one
  exists, don't add another. Rejected ideas become Design Notes in the spec.
- **The spec is the source of truth.** Read it before designing, update it
  after implementing. Then the book chapter and `crow test --docs`.
- **Both modes, one behavior.** Every feature works identically in
  `crow run -i` and `crow run`. Shared logic lives in `crow_runtime` so
  both call the same code; new behavior gets a `tests/parity/` program.
- **Builtins are short lowercase words**, every call is `f[x]`, blocks are
  indentation, and shift-key syntax needs a recorded tradeoff.
- **Every bug fix ships with the test that reproduces it.** No stubs: a
  feature isn't done until every planned backend works and is documented.

## Before calling anything done

```
cargo test --workspace
cargo clippy --workspace --all-features
./target/release/crow test --parity
./target/release/crow test --packages
./target/release/crow test --fuzz 20
./target/release/crow test --served
./target/release/crow test --docs
./target/release/crow test --book
./target/release/crow test tests/layout/conformance.co
./target/release/crow test tests/parity/185_frames_held_to_pandas.co
./target/release/crow test src/modules/nn.co
./target/release/crow test src/modules/core.co
./target/release/crow test src/modules/sys.co
./target/release/crow test src/modules/io.co
./target/release/crow test src/modules/frame.co
./target/release/crow test src/modules/math.co
./target/release/crow test src/modules/auth.co
./target/release/crow test src/modules/web.co
./target/release/crow test src/modules/ws.co
./target/release/crow test src/modules/mail.co
./target/release/crow test src/modules/stats.co
./target/release/crow test src/modules/clickhouse.co
./target/release/crow test src/modules/postgres.co
scripts/coverage.sh
```

Use the release binary for the layout and site suites: the debug binary
overflows its stack there. CI (`.github/workflows/ci.yml`) runs the same
steps on every push and must be green.

## Things that have bitten

- `*.csv`, `*.txt`, `*.json` are gitignored. A fixture needs a negation
  rule in `.gitignore` or CI never sees it.
- `Cargo.lock` is not tracked, and `cargo install` re-resolves unless
  given `--locked`.
- The seeded RNG is process-global; tests that draw random numbers take
  the lock in `crow_runtime/src/tensor.rs`'s tests.
- Map literals are `map[k = v]`; nested numeric literals must be
  rectangular (ragged ones are refused in both modes).
- Generated programs link the runtime crates by the absolute path baked in
  at build time (`CARGO_MANIFEST_DIR`); `install.sh` snapshots the source
  to `~/.crow/src` for that reason.
- After verifying a served app (finch on 3100), run `crow stop` so the
  port is free for the user.
- Every build checks the IR of every program it lowers (`CROW_IR_VERIFY`:
  strict unless set to `off` or `report`; a release binary had skipped
  the check until 2026-10-08, so a lowering slip reached the backends
  silently). A finding is a bug in the lowering, not in the program:
  three unit tests looked flaky for an hour until the variable, exported
  in some shells and not others, turned out to be what differed — and the
  finding was real. `scripts/coverage.sh --arms` writes
  `target/coverage/arms.txt`: the lines no suite reaches, the match arms
  among them named — a backend's arm for what the lowering must never
  produce is where the next slip hides.
- One type: `crate::ir::Ty` (re-exported by `type_infer`) is the only type
  enum since 2026-10-07 — `CrowType` and the IR's `Ty` were one thing
  twice with a lossy map between them, and `where` of a mask was math's
  to inference and the frame's to the lowering. `Value` is the
  interpreter's value, `Kind` a row's contract, `Arg` a type's class for
  choosing a row. `lower_ir::as_inferred` is the one projection (a
  column's cells and a view's node forgotten), not a second vocabulary. A
  new kind (`Ty::Moment`, `Value::Moment`, `Kind::Moment`/`Arg::Moment`)
  still threads through the table's rows, rules (binop_type, fields),
  lower_ir/checks, the emitter, ir_verify, the interpreter's ops and the
  runtime's Cell/mask::Cell/CrowDisplay/ToJson — `src/totality.rs` lists
  the places that must answer for every variant, and refuses to compile
  until a new variant is placed (design rule 21).
- The row a bare name reaches is decided once, in
  `crate::table::module_of_bare`: inference (rules.rs), the lowering
  (`module_for`, `builtin_row`) and both backends' by-name paths (the
  interpreter's `runtime_call`, the emitter's `builtin` and
  `base_is_result`) all call it. A second rule in any of them is the bug
  class that produced "the lowered program disagrees with itself" and a
  compiled program the Rust compiler refused; the scalar-overlap list
  (abs, sqrt, …) had been written four times.
- A builtin's name lives in the table and nowhere else: `CORE_RESERVED`,
  `BUILTINS` and every module's function list (`MATH_FUNCTIONS`,
  `STATS_FUNCTIONS`, `NN_FUNCTIONS`, `IO_FUNCTIONS`, `DOC_FUNCTIONS`,
  `CHART_FUNCTIONS`, `FRAME_FUNCTIONS`, `SYS_FUNCTIONS`) are derived from
  `ROWS` (plus `OWN_FORM`, the web and ui lists, `MODULE_BARE` and
  `INTERNAL_FORMS` in checks.rs; db's and web's lists stay written, being
  own forms), and the checker's purity reads `ir::EFFECTS` and
  `ir::DRAWS`. A hand-kept copy had held `pad` and `append` long after
  they went, lacked `bucket` the day it came, and lacked `fma` the hour
  it came: "undefined function" in a package, "documented but the
  compiler has no such builtin" in the docs check.
- A package's files call each other qualified (`words.counts[…]`, with
  `use words`); a bare sibling name is undefined even inside the package.
  A parameter may not be named like a builtin (`values`), a module
  (`words`) or a function of its own file (`scores` reads as a call), and
  a module needs its own `use math` for `ln`, `sqrt`, `fma`. A `return`
  of a collecting loop is written as an assignment first.
- A reference compiled with clang contracts `sum += x * x` into a fused
  multiply-add: scikit-learn's row norms and scipy's sparse products, as
  pandas's rolling kernels. A sum held to one runs with `fma[x, x, total]`
  (the row exists for that); a plain sum differs in the last digit a third
  of the time.
- Whether a type holds an unknown is `Ty::has_unknown()` /
  `is_concrete()`, never the type's debug text; `src/totality.rs` holds
  them to every variant.
- `no_row_is_shadowed_by_an_earlier_row` (table.rs) fails when a row's
  position makes it unreachable: `str[column]` listed after the form that
  shows any value had shown every column.
- A test block that reaches no truth and no `assert` fails with "asserts
  nothing" (finch's `sim/autonext.co` had a line meant as a truth that
  was not one, and passed for weeks; its `sim/clock.co` law stood both
  its truths under a precondition `if`, and nested bare booleans had
  never been truths — they are now, by syntax before the lowering and by
  their bool type after it); a parity program that prints nothing in
  either mode fails.
- A loaded frame's columns come from the program's reads
  (`lower_ir/uses.rs`): the shapes are read off the declarations before
  any pass types anything (a parameter once typed as a parsed value stays
  one through every later round), and applied where the frame is bound,
  BEFORE the scan pass types the variable — the scan settles a variable
  from the lowered value's type ahead of `assign`, so a hook in `assign`
  alone reached only the final pass. A frame from `load` is
  `Frame(Some(Cols::dynamic()))` (open, no columns), not `Frame(None)`:
  test for both (`uses::dynamic_frame`).
- A column held in a variable keeps its cells' kind: the lowering wrote
  `as_inferred` (cells forgotten) into `ctx.vars` at FOUR sites — the
  scan's note, the settle pass, the final rebind and the `let` — so
  `texts = str[t.at]` then `moment[texts]` then `back[2] eq t.at[2]`
  compared ISO text with a moment compiled (parity 189). `kept`/
  `keep_cells` in lower_ir.rs keep a column's cells; `Ty::compatible` and
  `unify` have a Column arm (by cells, as lists by items).
- Inside a view, a function named like an element (`label`, `div`, …) IS
  the element; a Crow map holds one kind (`map[sym = 'A', qty = 1]` is
  refused); a route's `json[text]` PARSES the text (it is `json.parse`),
  `json[map]` and `json[frame]` answer responses; the emitter's
  `base_is_result` found the parse row for `json` by name and skipped the
  `CrowResult { ok: true, … }` wrap of a propagating call (the response
  forms are own forms: listed there now).
- `fetch`/`poll` are EFFECTS in the IR (`fetch!now(…)`), not calls: a
  retype or a wrap of a source's binding must match `Expr::Effect` too, or
  it silently does nothing (the probe showed the hook running and the
  variable staying dynamic).
- A missing cell shows as `missing` in a frame's table and as `error:
  missing` in a column shown as a list (runtime `fmt_cell`, the frame's
  Display, `CrowDisplay for CrowColumn`); the interpreter's missing cell
  is an error of reason `missing` (ops.rs `column_cell`) and its plain
  placeholder is empty text, as the compiled `get_str` answers. A NaN a
  number column holds is a value and shows as `nan`; arithmetic over a
  column with a missing cell answers a tensor holding NaN there.
- A parsed value's reads are inserted by the lowering: `read_json`
  (lower_ir.rs) at six sites — a position or key, an operator, not/neg, a
  condition, an assignment to a variable of a kind, a builtin's argument
  (by `uses::row_says`). A NEW site that meets `Ty::Json` must call it, or
  the compiled program refuses or does not build (`if j.flag`, `not
  j.flag`, `j.flag and true` had run interpreted and not built). The rows
  are `core.__read_*(Json, Text)`; the second argument names the use and
  line for the stop.
- Never `crow run` a program IN PLACE under tests/parity or tests/served
  while a harness runs: the directory's `.crow_build` is shared, and the
  binary you execute may be another program's (a run printed parity 190's
  output for 198, and once "crow_web unresolved"). Copy the program to the
  scratchpad and run it there.
- A loop whose body's failure leaves the function (an unwrap into a value
  that cannot hold it — a bool — in a function that answers a result)
  keeps its order: a round on another thread cannot return from the
  function (`keeps_order`, fuzz seed 1791422055, parity 198). A failure
  assigned to a variable that can hold it (`day: Result(Moment)`) is
  held, and the function goes on — in both modes.
- A served page's first render runs `fetch_sources` before rendering; the
  stop's reason goes to the server's log only (Vijay, 2026-10-01) and the
  page shows "<who> stopped, and nothing changed — the log has why"; the
  served parity harness normalises the id. `crow test --served <word>`
  runs only the pages whose names hold the word.
- A backend's fallback arm for what the lowering must never produce
  (the interpreter's int-plus-float arithmetic, the verifier's loop over a
  map) is where a lowering slip hides: the coverage run lists them as
  arms no suite reaches. Refuse in the lowering, in both modes, or
  convert there; do not widen the backend.
  And four places the gate found afterwards: its serde form is ONE JSON
  text both modes read (`served::json_to_typed`/`json_to_fact` read a fact
  back by `Ty`, the emitter's `by_serde` list decodes a handler's
  argument); `int[]`/`float[]` of it are `never` rows listed BEFORE the
  `(Cell)` rows in table.rs (or a derivation over it is `Result`); a fact
  declared from it is a literal default only if `checker::declarations`
  says so (`moment[…]` of literals is; any other call is "undefined
  variable" in every test and fine under `crow run`); and an option
  desugaring in lower.rs passes its own lowered form through unchanged
  (`lowering_twice_is_lowering_once`).
- A removed builtin a standard module (`src/modules/*.co`) still calls
  fails only in programs that reach that row: `time[]` in auth.co and
  mail.co broke two parity programs, two book blocks, a harness and six
  unit tests while every module test passed.
- A `sys` row a standard module may call is listed in `SYS_FUNCTIONS`
  (`src/modules/sys.rs`) beside its row in the table; without the entry the
  module's call is "undefined function" in both modes.
- `push[xs, v]` answers a new list: as a bare statement it does nothing.
  Write `xs = push[xs, v]` (a check's list of results stayed empty, and
  every test passed).
- Crow has no `pass`, `continue` or `lambda` (the parser names each) and
  no `chars[]`: characters are `split[text, '']` with its empty ends
  skipped. A module written in Crow sees another module's ROWS, never its
  helpers (`exchange_on` had to become a sys row for the driver to call it).
  A driver's rows are named `ch_*` so they collide with no program name
  (`db`'s `query` beside a driver's `query` was "ambiguous" in every
  program). A row called from Crow converts its arguments by its
  signature: JSON text handed to an `Encoded` parameter arrives as a JSON
  string (`'[]'` as `"[]"`), so a module's rows call helpers, never each
  other (`pg_check` pinging through `pg_query` failed every connect).
- Per-cell work written in Crow pays the language's price per operation
  (a copy, an allocation, a Result) where a kernel pays per byte: a parser
  of a million rows was 20–45× slower than `frame_of_csv`. The rows a
  database answers are decoded by a kernel (`frame_of_rowbinary`,
  `frame_of_postgres`); the protocol around them is Crow's.
- A standard module written in Crow (`src/modules/*.co`) is read in its own
  scope: inside its inlined functions the program's names are invisible
  (`calls_fn` in lower_ir.rs, lower.rs, checks.rs and type_infer/calls.rs).
  A new resolution site that consults `ctx.functions` by a bare name needs
  the same guard, or a module local named like a program function becomes
  a call. The inliner keeps only the rows a program reaches (by name), and
  the modules' scopes are read from the program's own functions
  (`crow_modules_carried`), never from whatever was inlined last on the
  thread: a harness lowers a part's tests and the app in turn.
- A crate added to a manifest gets its line in the spec's Design Note
  "what stays native, and why" (section 13), with its reason under design
  rule 20; a formula over existing builtins is written in Crow instead.
- `crow_runtime`, `crow_db` and `crow_web` deny panics under clippy: a
  runtime function answers its reason, and a failure no value can carry
  stops through `crow_runtime::stops` (docs/bug-classes.md).
- A kernel held to pandas "to the last digit" reproduces its arithmetic,
  not its formula: pandas's rolling kernels (`roll_var`, `roll_quantile`,
  `group_var`) compile to fused multiply-adds on arm64, so the runtime
  uses `f64::mul_add` where pandas writes `a + b * c`; `roll_var` also
  restarts a window whose squared sum collapsed (its `InvCondTol`). Read
  the `.pyx` itself (curl the raw file) — a summary of it cost an hour.
- A function a test does not call is still lowered, its parameters
  untyped: a comparison over an untyped column is a guessed `Bool`, which
  the kinds check and the row choice read as unknown (`guessed_truth`),
  or `where[df.filled eq true, …]` was refused in every test that did not
  call the function and resolved to math's row in the one that did.
