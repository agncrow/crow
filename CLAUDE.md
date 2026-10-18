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
./target/release/crow test --errors
./target/release/crow test --parity
./target/release/crow test --packages
./target/release/crow test --fuzz 20
./target/release/crow test --fuzz 50 --mistakes
./target/release/crow check --self
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
./target/release/crow test src/modules/llm.co
./target/release/crow test src/modules/stats.co
./target/release/crow test src/modules/clickhouse.co
./target/release/crow test src/modules/postgres.co
(cd examples/rag && ../../target/release/crow test chat/corpus.co && ../../target/release/crow test chat/retrieve.co && ../../target/release/crow test --fixtures chat/page.co && ../../target/release/crow test --fixtures --compiled chat/page.co)
scripts/coverage.sh
```

Use the release binary for the layout and site suites: the debug binary
overflows its stack there. CI (`.github/workflows/ci.yml`) runs the same
steps on every push and must be green.

## Things that have bitten

- `*.csv`, `*.txt`, `*.json` are gitignored. A fixture needs a negation
  rule in `.gitignore` or CI never sees it.
- `Cargo.lock` is tracked (the release builds `--locked`): a version bump
  or a dependency change commits it too, and `cargo install` re-resolves
  unless given `--locked`.
- The seeded RNG is process-global; tests that draw random numbers take
  the lock in `crow_runtime/src/tensor.rs`'s tests.
- Map literals are `map[k = v]`; nested numeric literals must be
  rectangular (ragged ones are refused in both modes).
- Generated programs link the runtime crates by the absolute path baked in
  at build time (`CARGO_MANIFEST_DIR`); `install.sh` snapshots the source
  to `~/.crow/src` for that reason.
- After verifying a served app (finch on 3100), run `crow stop` so the
  port is free for the user.
- The editor's language server is a process: VS Code's `crow lsp` kept
  the binary it started from for two days after `install.sh` replaced it,
  and refused a query's cell pushed onto a list long after the installed
  crow accepted it (parity 233). The server now leaves when its binary
  changes and the editor starts it again (`lsp::Binary`); its output pane
  names the binary it runs and when it was written. The extension's installed copy
  is a second process to keep current: any change under `editors/vscode`
  is repackaged and reinstalled (docs/crow-lsp.md), then the window
  reloaded. The extension is plain JavaScript over VS Code's API alone —
  no npm package, packaged by `editors/vscode/package_vsix.py`, and no
  workflow runs Node.js (2026-10-18) — and `editors/vscode/test.sh` drives
  it inside VS Code: run from VS Code's own terminal or an extension, the
  app inherits `ELECTRON_RUN_AS_NODE=1` and starts as Node ("bad option"
  for every flag), its profile directory's path must be short (a Unix
  socket's is at most 103 bytes), and a document's URI is normalized (a
  `//` in the path the test asks about had found no diagnostics).
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
- The fuzzer (`crow test --fuzz n --seed s`; `--keep` keeps every
  program under target/crow_fuzz/) writes a user module beside each
  program and one page per ten programs (ports 3470–3489, through
  `served_parity::drive`). A generator slip shows as "refused alike":
  a name a module owns (`rename`, `where`) or the predicates' names
  (`p0`, `p1`) must not be reused; `encode` of a document literal is a
  value that can fail, which a list literal then holds. The pass renaming
  names Rust reserves (`rust_names.rs`) is scoped: a body's names in that
  body, the top level's everywhere — the ClickHouse driver's local
  `where` had renamed every program's `where[…]`.
- Performance of the compiled program is decided by what the emitter writes
  per operation, not by the toolchain (opt-level 3 + fat LTO already) — with
  one trap: fat LTO re-optimizes the standard library's own byte loops worse
  (`str::replace` of one ASCII byte ran 3× slower than unlinked; thin LTO
  cost binarytrees 5–35%), so a runtime kernel over bytes writes its own
  loop (`text::replace`'s ASCII pair). A
  `for` over `range[n]`/`range[a, b]` is a counted loop (`emit_for`), and
  `stops::item`/`char_at` take their common case first (`#[inline(always)]`,
  the slow path `#[cold]`). Measure by rewriting the generated main.rs by
  hand one change at a time ($S/fk/exp.py did) before touching the emitter.
  The interpreter: block scopes come from a pool (`push_scope`/`pop_scope`),
  a `for` runs its rounds in ONE scope (the variable rebound, the round's
  bindings `retain`ed away), `xs[i]` and `xs[i] = v` of a local have fast
  paths before the general in-place path, `for` over `range[…]` counts
  without building the list (`counted_for`, the emitter's twin: any bound
  that is not an int goes to the row call the list path makes), and
  `Expr::ty_ref` lends a type where `ty()` would clone a boxed one.
  Profile with `sample <pid> 3`: the "Sort by top of stack" list is what
  matters (the release binary has no frame pointers, so call chains are
  blank). And time the right thing: `crow run file.co` is a DEV build by
  design (the runtime crates at opt-level 1, the program at 0 unless
  `--opt N` says — the spec says so; fannkuch is 30× slower there than
  from `crow build`'s binary, 12× at `--opt 1`; and a level's BUILD cost
  is measured on a big crate, not on fannkuch: finch's rebuilds after an
  edit in 3 s at level 0 against 14 s at 1, which is why 0 is the
  default), so
  the compiled number is `crow build`, then the binary; the interpreter's
  number is CPU time (`/usr/bin/time -p crow run -i …`): wall time under
  a running suite was 3× off.
- A function's body runs on its PREPARED form (`src/ir_interp/prepared.rs`):
  instructions over registers (`Ins`, compiled on the first call), and a
  TYPED program (`TIns`, one per `Ins`, at the same positions) keeping int
  and float registers bare. (1) A new walker arm that gives a SHAPE its own
  meaning (as `assert`'s message, a bound input, `len` in place and an
  in-place `push` have) must stay an `Eval`/`Exec` fallback in
  `Comp::compute` (the `own` list, `moves_first`), or the prepared body runs
  the general path for it and the modes drift. (2) A new `Ins` needs its
  mapping in `typed()`: a typed form, or `Boxed { pre, post }` naming EVERY
  typed register it reads (`pre`) and writes (`post`) — one left out is read
  stale or written and lost; an instruction that can lend the registers to
  the walker boxes all of them both ways (`lend()`). (3) A typed instruction
  checks before it writes, so a hand-over at its own position re-runs it in
  the boxed program; after a boxed instruction ran, the hand-over is at the
  next position and skips boxing what it wrote. (4) A register has one kind
  for the whole function (`RKind`; temporaries from per-kind pools), or a
  hand-over would box a stale int over a live value. (5) A list of ints is
  an `IntList` register only where every instruction that meets it is a list
  instruction or makes it (`list_use_ok`, `reads_writes`): an instruction
  that READS one otherwise would convert the whole list each time it runs,
  so a new `Ins` that reads registers must say so in `reads_writes`. A
  PARAMETER is never one: its list arrives boxed, and making it bare at
  every call read the whole list (a binary search called 80,000 times). The
  slots follow the
  walker's scoping (`declare` reuses a name bound in the current block,
  `fresh` is a loop variable's): a nested `for i` restores the outer `i`.
  Measure the interpreter with instructions retired (`/usr/bin/time -l`),
  which load does not move.
- A map's type carries its keys' kind: `Ty::Map(Key, value)` with `Key::{Text,
  Int, Open}` (open = empty until a first key). Where a map's type is REBUILT
  from another (settle, held_as, `as_inferred`, the emitter's inner types),
  carry the key — writing `Key::Text` there made the first assignment of
  an int-keyed map look like a reassignment ("cannot reassign 'm' from
  map[str] to map[str]"). The interpreter holds `Value::IntMap` beside
  `Value::Map`; the eight map rows are `maprow!`s with hand-written
  bridges (`table::maps`), one branch per key kind, since one closure
  cannot instantiate a generic runtime function twice; a row answering
  `Kind::Keys` is typed by the rules (`Kind::generic()` lists it) — a kind
  not listed there is answered as its `ty()`, Unknown. `Kind::Lists` is a
  VARIADIC group of lists of text, not a list of lists.
- `x in xs` / `x not in xs` are desugared by the PARSER (parse_comparison)
  to a call of the row `contains` with the collection first; `contains[`
  and `has[` written in SOURCE are refused there (parse_postfix) — the
  rows stay (a map's and a parsed value's `contains` rows are `has`'s
  functions), the lowering still emits `has` calls by AST (states.rs
  handles their text), `uses::row_says` reads nothing for an argument a
  row takes as Json, the docs check lists both under SPEC_ONLY and the
  LSP skips them in completions. `concat[[a, b, c]]` is folded in lower.rs
  (a LITERAL list only; a variable list of lists has no row — `Kind::Lists`
  is a variadic group of lists of text, not a list of lists); bytes held in
  a variable are `sys.concat_all (Chunks)`. `map[keys, values]` is the hidden
  row `__map_of` (lower_map_call), a tensor of values converted to a list
  at the builtin-args site. An index by a mask lowers to `__pick_rows` /
  `__pick_items` BEFORE check_callee at the index site; `Kind::Tensor`
  takes `Arg::Truths`, and `Convert::ToTensor` has a bool arm in both
  backends (true 1, false 0). `table::form(name)` names a hidden row in a
  refusal as the program writes it (`x in xs`, `map[keys, values]`,
  `xs[mask]`). The corpus rewriter is in the scratchpad (in/rewrite.py):
  bracket- and quote-aware, `not contains[a, b]` → `b not in a`.
- A `then` chain written as lines is desugared by the PARSER (`then_lines`
  in parser.rs): `x = e` then `then f[…]` lines become statements over
  hidden names (`x__then1 = e`, `x__then2 = f[x__then1, …]`, the last
  step assigns `x`), because a variable may not change its kind and a
  chain's does. A step that names `x` has `x` renamed to the hidden
  name (`rust_names::rename_expr`, spans included); one that does not
  gets the value as its first argument. Nothing below the parser knows a
  chain exists; the formatter is line-based, so it keeps the lines. The
  installed `crow` is the OLD binary: probe a parser change with
  `./target/release/crow`, or the probe tests nothing.
- A missing cell shows as `missing` in a frame's table and as `error:
  missing` in a column shown as a list (runtime `fmt_cell`, the frame's
  Display, `CrowDisplay for CrowColumn`); the interpreter's missing cell
  is an error of reason `missing` (ops.rs `column_cell`) and its plain
  placeholder is empty text, as the compiled `get_str` answers. A NaN a
  number column holds is a value and shows as `nan`; arithmetic over a
  column with a missing cell stops the program (`row 1 of the column is
  missing — fill the column first`), as the spec's Frames section says.
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
- Crow has no `pass` or `lambda` (the parser names each), `continue` and
  `break` only inside a `while`, and no `chars[]`: characters are `split[text, '']`
  (no empty piece at either end since 2026-10-18). A module written in Crow sees another module's ROWS, never its
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
- A Crow-written row's parameter that is a parsed value, a map or a
  record is `Encoded` in the table: the lowering writes the argument as its
  JSON text (`crow_call_args`) and the Crow body reads it back with
  `json[…]` at its door (`llm.co`'s `talk_of`, `model_of`). A `Json` kind
  on a `crow!` row is "a bug in Crow" as the program runs. Such a row is
  NOT listed in `ir::EFFECTS`: the effect path re-encodes the argument
  through the by-name bridge, and the text arrived quoted (`"{\"base\"…`);
  a Crow row's effects are its body's (`request`, `send`, `recv`), which
  the call graph carries. Inside a module, an interpolation holds no quote
  and no backslash (`` `join[xs, ', ']` `` and `` `join[xs, '\n']` `` are
  hoisted into a variable first), a variable set in a branch is not in
  scope after it, `text`, `input`, `args` and (with `use math`) `max` are
  builtins and so no parameter's name, and `a.ok and a.field` reads the
  field of a failed value — nest the `if`s. A list of float lists is not a
  matrix for `tensor` or `stack`: a list of row tensors is (`stack[rows]`
  with `rows = push[rows, tensor[numbers]]`); a list of lists of values
  that can fail handed to `stack` passes the lowering and fails only in the
  verifier (open).
- A request's fixture (`crow check --record-fixtures` keeps it, `crow test
  --fixtures` answers from it) is asked for and kept by `request`'s OWN Crow
  body (core.co: `replaying[]`, `recording[]`, `replayed`, `recorded` — sys
  rows over `crow_runtime::fixtures`), so both modes agree by construction;
  the replay's switch is `CROW_REPLAY`, the recording's a runtime global the
  rehearsal sets (`record_to`, since a recording runs interpreted in the
  process that began it), a rehearsal's a third (`rehearse_in`: a request
  a recording kept is answered from its fixture, any other `200` with
  nothing to say — in the body, so a scenario that asks a model comes out
  as its recording did and the baseline stands; `rehearsal::outside` no
  longer answers `request` itself); a recording runs the tests that open
  no app too, for their fixtures alone (the replay runs every test). `fnv`, `note_miss` and `take_misses` are the runtime's — crow_db
  reads them from there; a second copy is the bug class. The key is the
  method, the URL with its query and what is sent: a multipart boundary is
  random, so files are keyed by field and path (`sent_as`). The miss is
  noted as the whole sentence the run prints. A `crow!` row's body may
  `return held` of a fallible local, and a local may not be named like a
  sys row (`answer`).
- Where the runtime crates come from is `home::runtime()` (src/home.rs),
  in order: a checkout named by `~/.crow/dev` (`install.sh --dev`), the
  checkout the binary was built from where it still exists (the gate's
  `target/release/crow`; `CROW_IGNORE_CHECKOUT=1` skips it, for a
  release-shaped run), the release's bundle `~/.crow/runtime/<version>`
  (`crow setup`: toolchain from static.rust-lang.org by the channel
  manifest's hashes into `~/.crow/toolchain/<rust version>`, the bundle
  from the GitHub Release; curl and tar do the fetching), then an
  install's snapshot `~/.crow/src`. A bundle build is offline: the
  generated project gets `.cargo/config.toml` (vendored sources) and the
  bundle's lockfile. cargo is `home::cargo()` (Crow's own when fetched).
  `crow run` with no compiler interprets and says so; `crow build` fetches
  first. A version bump renames the bundle: `scripts/bundle_runtime.sh`
  and the release workflow read Cargo.toml's; `RUST_VERSION` in home.rs
  is pinned by hand and must match the release workflow's.
- A standard module's function inlined into a `use ui` program is read in
  the module's scope at EVERY site that reads a bare name: the ui-element
  check (checks.rs) and the element lowering (lower_ir.rs `module.is_none()
  && in_ui`) had read llm's `stack[rows]` (math's) as the element outside a
  view; both skip `modules::is_crow_fn(current_fn)` now, as `calls_fn` and
  `is_fact` do, and so does inference's first-assignment rule
  (`fresh_binding`: the module local's type had come from a program fact
  of the same name — `rows = 0` beside `embed`'s `rows = []`: "push[]
  takes a list … got an int"). A part is named by the path the program
  reaches it through (`crow_modules/vectors/flat.co`), never the path a
  link resolves to: the baseline's reach keys had held the machine's
  absolute path, and a linked package's own imports had lost its prefix. The compiled harness answers a test's read
  of a derivation by `__crow_derived`'s arms (emit_rs/program.rs `as_json`):
  a kind without an arm is "no fact or readable derivation named" compiled
  and fine interpreted — a list of records has one now. The checker's
  defined names are every module's (`modules::every_module_function`), not
  `with_stdlib`'s core set alone; a dependency's functions
  (`main::from_dependency`, by `EXPANSION_PARTS`'s `crow_modules/` path)
  are not probed with edge cases. An example app's `crow_modules/` holds
  RELATIVE symlinks to packages/ (committed; `crow install` would write
  absolute ones). `crow test main.co` runs the entry's own blocks: a part's
  tests run from the part, and a part holding derivations must read them.
- Writing a package over `grad` (packages/gpt): a function sees no
  top-level variable (inputs are the loss's parameters, gradients zeros);
  `reshape[t, rows, 1]` takes its sizes as arguments and makes the column
  a reduction broadcasts with; a literal list of tensors is a matrix, so
  tensors of three shapes are a list grown by `push`; a list of ints
  handed to `crossentropy` reaches a row `grad` cannot take (hand a
  tensor); `halt` in a guard does not narrow (`return` does); `fit`'s
  answer of two kinds is a record (a list holds one kind: `push` refuses
  another kind against the kind the items were first known to be, noted
  in the scan on `ctx.first_items` — a map on the Lowerer does not survive
  to the final pass, which runs on a Lowerer of its own; `ctx.param_reads`
  moved there for the same reason). A test's call refines a parameter the
  program's calls typed with an unknown inside (`type_by_tests`): an
  uncalled `fit` had handed `step` a `List(Unknown)` that a test's list of
  tensors could not replace, and `grad` refused the weights. A helper's
  assigned parameter in a traced copy is `mut` now (parity 245).
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
- A prepared function (`ir_interp/prepared.rs`) is a `Prepared` handle on
  a shared `Body`, and a call clones the handle it runs: what a call
  clones must be the thread's own. A parallel loop's chunk gets its own
  handles (`handle`); one reference count every thread wrote on every
  call had made 16 threads run no faster than one (spectral norm).
- The emitter keeps a function's one-axis float tensor that the body only
  reads and sets by index as a plain `Vec<f64>` (`emit_rs/vectors.rs`) —
  `Vec<i64>` when every binding and every number written is an int
  (`int_vectors`; a write goes through `core::held_int`, the float's nearest
  past 2^53, so it holds what the tensor would) — a vector PARAMETER it only
  reads by index and measures through a view made as the body starts
  (`read_views`, `{p}__v = p.numbers()`, borrowed where contiguous), and a
  list of text it only empties, appends to and joins as a `String` and a
  count (`emit_rs/builders.rs`). A new emitter path that prints a local
  goes through `local()`, `plain_vector`, `read_view` or the builders'
  intercepts, or it prints a vector or a text where a tensor or a list is
  expected. (A brainfuck interpreter's tape held as floats turned each cell
  into a float and back every step: 5.1 s against Rust's 0.98, 2.0 now.)
- `cpu[n]` is a count both backends read where a parallel loop starts
  (`crow_runtime::cores`: per thread, the crowfile's the default, a
  function's put back by `Restore`). A new parallel form — a builtin that
  runs on rayon — goes through `cores::run`, or `cpu[1]` does not hold it.
- A text read by position keeps a `crow_runtime::text::Known` (how far
  its start is checked ASCII): beside a local in the compiled program
  (`emit_rs/texts.rs`, reset at every assignment) and in a frame slot in
  the interpreter only for a register no instruction writes twice
  (`text_slots`): another text can lie where a freed one did, and a
  `Known` of the first would misread it. A new read of text by position
  (a builtin over characters) takes the `Known`, or it is quadratic again.
- `x = set[x, k, f[x]]` (any `IN_PLACE_BUILTINS` call whose later
  arguments read `x`) is split by the lowering (`taken_in_place`): the
  arguments first into `_tN`, then the call takes `x` moved. A backend
  that copies such a collection is missing that split, not a rule of its
  own.
- A parameter the body only reads is lent (`ir::lendable_params`, both
  backends): compiled `p: &T` read as `(*p)`, interpreted moved into the
  callee and back (`CallArgs`/`FromRegs`), its moves compiled as reads
  (`ir::reading_lent`). A new way to call a user function from generated
  code must pass `by_ref_params` by reference, or exclude the function
  (a Crow-written row's function is excluded: its row calls it with
  values; so is one handed as a value — its signature is the function type
  that holds it, records included — and `apply_fn_value` hands its
  arguments as owned values). A register named twice in one call is lent
  once, never twice. A call through a function value is lowered on the
  INDEX path (`f[x]` of a local): a rule for positions (`decided_index`)
  must skip it, or it unwraps the call's arguments and retypes the value.
- `log` writes a script's output in blocks when stdout is not a terminal
  (`display::buffer_output`; a thread writes what is held every 100 ms, so
  a reader watching the pipe or the log sees each line soon — the lost-
  connection test reacts to them): a new way for a program to end or to
  wait on its reader (an exit, a stop, a prompt) calls
  `display::flush_output` first, or what was logged is lost or comes late.
- An int is a whole number of any size (2026-10-09): the interpreter grows
  (`Value::Big`; its typed registers hand over on overflow), the compiled
  program holds an int in 64 bits where `src/int_width.rs` shows its range
  fits — an interval per place (local, answer, field, fact, a list's
  items), flow-sensitive for locals (`cur`, narrowed by `if`/`while`
  conditions, relations like `done lt count` kept in `rels`), sums bounded
  by `ROUNDS` = 48 bits of rounds ("no part of a program runs 2^48 times")
  — and as `Ty::Big` (`crow_runtime::int::Int`) elsewhere. The pass runs on
  the lowered IR, so both backends see its conversions (`ToBig`,
  `ToSmall(Who)` — refused past 64 bits in one wording; the interpreter's
  rows refuse by `table::no_big_taken`). A write to a local a condition
  only NARROWED (not written yet this round) is a sum unless the condition
  bounds the side the write moves toward (`bounds`, `bound_toward`: `lt`
  above, `gt` below, `eq` both, `ne` the end it cuts), and then only its
  bounded sides go into where the sums start (`write_as`): the narrowed
  range holds the loop's own drift, and joining it there each round had
  grown the range to unbounded — n-queens' `k` (`if k eq 0 … else k = k -
  1`) was a growing int, 4.4× slower than Rust's. A new expression shape that makes
  an int needs its range in `iv_of` (else it reads as any 64-bit int), a
  new write site goes through `write` (else a sum there diverges to Big
  and the hot loop pays the growing form: fannkuch had run 5× slower), and
  a builtin that keeps every digit gets a hidden `__…_big` row in
  `table::big_form`. `CROW_WIDTH_DEBUG=1` prints the settled ranges. A
  list handed to a function flows ONE way into its parameter
  (`flow_into`): joined, `report[ones]` and `report[twos]` (twos made of
  ones' items) shared one range that grew round every pass, and every
  count of k-nucleotide became a big int. `int[text]` always answers the
  growing form; `range[n]` clamps its items below 2^48 (soft). Inside a
  function `grad` differentiates, a dual number or a traced value that
  reaches a growing form's row is answered by the name the program wrote
  (`table::written_name`): `floor[x]` of a float parameter is
  `__floor_big` in the IR, which no dual had reached (parity 261).
- A row can grow what it is given (`Row { take: …, ..row!(…) }`: the
  tensor `push`'s bridge pushes in place when nothing else holds the
  storage). The interpreter hands a row its arguments by value only on
  its `take` path (a moved argument): a new in-place row needs that bridge,
  or it copies at every call.
- The emitter honours the IR's `move` (`moved_local`): a last read prints
  bare, so it moves. Exceptions copy: a lent parameter (`(*p)`), a handle, a
  plain vector, a text builder, and a statement loop's item it borrows from
  what it walks (`Local::borrowed`, `declare_borrowed` in `emit_for`). A new
  path that keeps a local's value goes through `owned_expr`,
  `moved_or_cloned` or checks `moved_local`, or it copies every last read
  again. The move pass (`place_moves_in`) kills an inner block's own names at
  its end (spec, Scope), but the OUTERMOST block's names are what its caller
  says is live: a notebook's next cell reads them (moving them had emptied
  `prices` between cells). A `move` the prepared interpreter does not compile
  is a walker fallback in a loop's body: `len[move x]` had been one.
- A text row whose runtime function can change the text where it lies has
  an entry in `table::taken_path` (the emitter prints it when the first
  argument is a moved local, the statement's target or a call's answer) and a
  `texts` take bridge (the interpreter's moved argument): both, or the modes
  copy differently.
- The interpreter's lists, maps, parsed values and records are
  copy-on-write: `Value::Array(Arc<Vec<CrowValue>>)`, `Value::Map`/`IntMap`
  (`Arc<BTreeMap>`, made with `Value::map`/`Value::int_map`), `Value::Json(
  Arc<serde_json::Value>)` (write through `Arc::make_mut`, take the items
  with `Arc::unwrap_or_clone`) and `Value::StructInstance(Arc<StructInstance>)`
  over an interned `&'static Layout` (`Layout::intern`, the interpreter's
  `Shape`s). A map had been copied whole at every top-level `key in m`
  (primes: 6.5 s at 100k), a parsed document at every `c[i]` (JSON: 109 s
  for 20,000 objects). A read of either is a count: the walker's reads had copied a
  400,000-line list for each chunk of a parallel loop, and a record copied
  every node below it at each field read (binarytrees: 4.6 GB). A count
  every thread writes is the spectral-norm trap again: the record's layout
  had been an `Arc` cloned per record, and sixteen threads ran slower than
  one — interned, it is read with no count.
- mimalloc is the allocator of the interpreter and of every generated
  program (`dep_list`, the `#[global_allocator]` before `fn main`): macOS's
  allocator reads the clock in `free` (`mach_absolute_time` near the top of
  a parallel profile is the allocator, not the program).
- The prepared form compiles a record's field read (`Ins::Field`) and a
  record literal (`Ins::Record`, its shape resolved at the first run); a
  row's call lends its large arguments unless the row makes its answer of a
  moved one (`table::keeps_storage`): a moved list had made every other
  large argument a copy.
- A compiled index is `crow_runtime::stops::at` (emitter: `index_at`):
  pure arithmetic, its stop never returning. A check whose slow path
  builds its message as a value (`stops::item(…).unwrap_or_else(…)`, the
  interpreter's form) is a call the optimizer cannot see through, so a
  loop's reads of a list it never writes were made again every round:
  the many-body n-body ran at half Rust's speed.
- A generated program's release and dev builds check overflow in the
  program and in Crow's own crates only (`generated_cargo_toml`); the
  crates beneath them build unchecked, as their authors release them. A
  runtime loop meant to run a vector's width at a time adds with
  `wrapping_add` where it cannot overflow (`text::counted`), or the
  build's checks keep it to one item at a time (`occurs` ran 25× slower).
- The emitter emits in the program's scope (`emit_with` enters it, each
  function its own): the registry's scope is a thread-local, and lowering
  a standard module's function leaves the thread in that module's —
  `find[text, p]` after a `read` had reached sys's `find` of bytes, and
  the program did not build.
- A `return` clears what is live where moves are placed: nothing after it
  runs on its path (`return slice[pieces, 0, n - 1]` in a branch had
  copied every line of a file, because `return pieces` after the branch
  read them).
- A route's handler that cannot wait (`ir::quick_fns`: no effect, no
  loop, no recursion, no call through a value, in it or in what it calls)
  is answered on the server's own thread (`CrowHandler::quick`, both
  modes); any other goes through `spawn_blocking`, on at most
  `cores::blocking_threads()` threads (tokio's 512 had let hundreds of
  handlers wait on one pool, and plaintext ran at a sixth of Rust's). A new
  builtin that can block (a socket, a sleep, a read) is listed in
  `ir::EFFECTS`, or a handler calling it stalls the server's thread.
- A kernel that shares its work across the cores goes through
  `cores::map_range` / `map_items` / `sort_unstable` / `sort_by`, parallel
  only past `SHARED_MIN` items: thirteen rows handed to rayon from a
  handler's thread had waited on the pool at every sort of a frame.
- The interpreter runs a route's handler on its thread's copy of the
  program, kept from request to request (`served.rs` `ROUTE_COPY`): a
  handler writes nothing a later request reads, and a stop drops the copy
  (a copy per request had prepared the handler again every time).
- `sys.leftover[s]` TAKES what a socket holds, and `unread[s, b]` puts
  bytes back in FRONT of it: a reader that took the leftover and must read
  more puts back what it holds first. llm's `next` lost every event that
  straddled two reads, and ws's `recv` lost a message's head when its wait
  ended inside the message (the next read took the payload for a frame).
  A test server that writes a whole body at once never shows it:
  tests/llm_server.rs writes each chunk after a pause, and the check's
  model (src/selfcheck/llm.rs) writes each chunk's parts apart.
- A connection's notes are keyed (`note[s, key, text]`, `noted[s, key]`):
  the Postgres driver prepares a statement with parameters once per
  connection under its kinds and text (`statement_sent`, at most 64 names,
  a name never used twice), ClickHouse notes its query id and keep-alive
  apart.
- A route's handler reads the derivations the app holds (spec,
  Derivations): the lowering binds each one it names to a local of its
  name as the handler starts (`route_derived`, `lower_fn`), and both
  backends keep the values in `crow_web::live` — compiled through the state
  main holds (`hold_app`, read with `app()`), interpreted through one per
  served program (`app_state_from`, its specs' names tagged with the
  program's `app_tag`). A new reader of the app's values goes through
  `derive`/`derive_bg`/`derive_with` on such a state, never a cache of its
  own, or the modes keep values by different rules; a shared value computed
  for a reader who waits is one flight (`FLIGHTS`) for every reader arriving
  with the same inputs. A route's writes are taken as its handler ends
  (`__CrowWrites`, `route_answer`) and published (`publish_writes`): crow_db's
  thread-local record of writes had grown with every write a route made. A
  `use web` program that declares a derivation holds state (`holds_state`):
  its top-level names are facts. Who holds a derivation is
  `derive::read_sets` alone — a fact of a page's address and a data source
  are the session's (`session_written`; both had been shared).
- A local is let go after its last read (`place_moves_in`'s frees): where
  no read moves it — an indexed base, a loop's list, `len[x]` — the move
  pass puts `move x` alone (`Stmt::Do(Expr::Move)`) after the statement;
  the emitter prints `drop(x)` (`emit_do`), the prepared interpreter
  `Ins::Clear`, the walker takes the slot. Never after a block's last
  statement (a collecting loop's item, a function's implicit return), never
  a parameter (it may be lent) and never a binding the block did not make
  before the read (a later `let` of the name shadows an outer one). A
  collecting loop over a list it reads for the last time walks it by value
  (`consumable`: a moved local, a `_list_N` bound by value, a call's
  answer), and a fallible argument that is a moved local hands its payload
  on (`_prop_N.value`, not `.clone()`), the taken path judging the
  payload's type: revcomp 1249 → 924 MB, k-nucleotide 645 → 356 MB.
- What a compiled program links is what every one of them pays at start:
  wgpu brought Metal, QuartzCore and Foundation, chrono's default `clock`
  CoreFoundation (iana-time-zone) — 4 of a one-line program's 6.4 MB. The
  GPU is crow_runtime's `gpu` feature, asked for by the emitter only where
  `gpu[…]` is written (`runtime_features`); the crow binary enables it.
  `otool -L` on a generated binary shows what a new dependency brings.
  Measuring memory: vmmap labels mimalloc's pages "IOAccelerator" (VM tag
  100 is mimalloc's default `os_tag`), and a 24-byte allocation takes a
  32-byte slot in both mimalloc and macOS malloc (16-byte alignment): a
  three-field record costs twice a two-field one.
- The interpreter's walker (the top level, a collecting loop's rounds,
  what a prepared body hands it) keeps a `Known` per local text only while
  it runs a loop (`walker_loop`, `loop_text`, keyed by frame and name):
  `bind`, `assign` and `slot` let a name's go, and a loop's end lets its
  frame's go however it ends (prepared code may write the name after the
  walker hands back). A new walker path that reads a text by position
  takes it from `loop_text`, or a long text is walked from its start at
  every read (revcomp's pieces had taken 106 s interpreted).
- A record's link to a record of its kind (`Ty::OptRecord`) is
  `Option<crow_runtime::link::Link<T>>` compiled — triomphe's `Arc`, one
  count and no weak one (the 32-byte slot a box took) — never a `Box`: a
  box copied the subtree at every `t.left` (a treap ran 1000× slower than
  Rust's). A write through a link goes through `link::make_mut`
  (`emit_set_path_with`'s `through`); a builtin takes the link as it is
  (`type[t.left]`, `str[t.left]` of none had unwrapped it, a panic); a
  user function taking the record gets `as_deref().unwrap_or_else(stop)`.
  An int handed where a record or none is held (a field only ever given
  0) goes through the hidden row `core.__none` (`none_of` in lower_ir.rs),
  then as the literal 0 both backends read as none.
- `and`/`or` decide from the left (2026-10-11): the right side runs only
  where the left does not decide — `ir::short_circuits` (neither side a
  list, a tensor, a column or a frame: a mask joins item by item, both
  sides computed) is the one rule all three backends read: the emitter's
  `&&`/`||` (a lazy right side where either can fail), the walker's early
  return, the prepared form's `Ins::JumpDecided` (the left in its own
  register, copied into the destination when it decides). Statements the
  RIGHT side's lowering pushes (`self.pre`: a failed value's refusal, a
  position's check) wait under the left's decision (`right_side_waits`:
  the left into `_tN`, then `if <left goes on>` around them and `_tN =
  right`), or `r.ok and r.x gt 0` refused `r` before asking `r.ok`.
- A record or none compared with the literal 0 is the none test (`t.left
  eq 0`): the lowering lets exactly that comparison past `check_order` and
  types the 0 as the link (`Ty::OptRecord`); the emitter's
  `optbox_comparison` asks `is_none()`/`is_some()` where the link lies (a
  record: always one); the interpreter's `values_equal` already answers.
- `use bits` (bitand, bitor, bitxor, bitcount, shift, code, char) is a
  module like `doc`: rows in table.rs, `crow_runtime::bits`, the module
  registered at the dozen places `doc` is (modules/mod.rs `module_by_path`
  and `with_stdlib`, emit_rs/program.rs `load_module` and the `use` arm,
  type_infer/classify.rs, checks.rs `REGISTRY_MODULES`, emit_rs/exprs.rs's
  pinned list, main.rs's docs tables and stdlib pages). Its growing forms
  are hidden CORE rows (`core.__bitand_big`, …: a module's `__` rows would
  be listed as its functions), reached through `table::big_form`; a shift
  whose range may pass 64 bits is `wide` (`wide_arith`) and takes
  `__shift_big`. `row_iv` gives and/or/xor/shift their ranges: a mask cut
  with `bitand` after a shift stays 64-bit (`shift[1, 13]` is exactly
  8192 — an end from 0 had made every mask look negative).
- `scripts/coverage.sh` had been a zsh script, and GitHub's Linux runner
  has no zsh: coverage never ran on CI, and every run after the repository
  went public failed there while every suite before it passed. A script is
  bash as macOS ships it (3.2: no associative arrays; `mktemp` given its
  whole template, which GNU's needs); coverage is a CI job of its own,
  since it runs every suite again instrumented. The Postgres tests start a
  scratch cluster with `initdb` from the PATH and skip without it: the
  runner ships PostgreSQL with its server tools in
  `/usr/lib/postgresql/<n>/bin`, put on the PATH by both CI jobs — until
  then the Postgres driver had never been tested on CI. The GPU tests skip
  where no adapter answers: both CI jobs install Mesa's lavapipe (a Vulkan
  device on the CPU) so they run there, since gpu.rs alone was the 3.7
  points crow_runtime's coverage stood lower on Linux (85.28% against its
  floor of 87).
- A value that can fail, dropped by its line, is `Decision::Dropped`
  (lower_ir's `S::ExprStmt` arm, 2026-10-16): a function hands it back,
  the top level halts `cannot drop an error value`. Exempt: what a line
  hands on (`self.yields`: a collecting loop's last, a console cell's), a
  truth (truth syntax, or a bool in a test block — a first version
  decided `len[v] eq 59` of a value that can fail, and the test lost its
  truth unseen), a `mail` (its unread answer is what holds it for the
  commit: reading it sends at once — the mail-waits test caught it), and a
  call of a handler or a fact-writing helper (`kind_of`: its stop is the
  handler's, and compiled it answers `()` — served 02 did not build). A standard module's best-effort call keeps its value in a
  named local (`swept = sweep[…]`), or its failure now fails the caller.
  `CROW_DROP_AUDIT=1 crow build --ir file.co` lists the lines the rule
  covers (stderr); the parity corpus compares the modes, not outputs over
  time, so a semantic change shows only in an old-vs-new run of it.
- `read(x)` and `full(x)` forget a column's cells (`as_inferred`): ask
  `x.ty()` to see them — the bool-column mask rewrite (`is_bool_column`)
  never fired until it did. A rewrite that happens only once types settle
  still meets the scan pass's refusal: accept a column whose cells are not
  known yet where the rewrite would apply.
- A function called at several signatures is lowered once per signature
  (`specializations`), each checked at its own types — the copies had been
  `muted`, and a copy whose list met `set` (a map's row) ran interpreted
  only to stop and did not build. The printer shows a refusal both make
  once.
- A name the core and math share (`max`, `min`, `sum`, …) is typed by
  what its arguments hold (`payload()`): a tensor that can fail had failed
  inference's "is it numbers" test, the call was typed as nothing known,
  and the compiled program did not build (fuzz seed 1792144449, parity
  250). The outer step wraps the answer as one that can fail.
- A tensor is written at the positions that read it: `m[i][j] = x` lowers
  (S::IndexAssign) to `m = core.__set_at[m, [i, j], x]` and `m[i] = row` to
  `core.__set_part` (a part of exactly the shape the positions leave:
  `part_disagrees` refuses statically where both shapes are known, the
  runtime stops in the same words), hidden rows in `IN_PLACE_BUILTINS` with
  take bridges and `taken_path`s, so both modes write where the matrix lies;
  `taken_in_place` splits a tensor's self-reading write too (`d[k] = d[k -
  1] + 1.0`). Through a record's field, a list's item or a map's value
  (`tensor_lead`) it is `g.cells = __set_at[g.cells, …]` as a `SetPath`, a
  computed position hoisted once — a copy of that tensor per write.
  `crow_runtime::core::set_flat_at` (`v[i] = x`, both backends) refuses a
  tensor of two axes or more. The interpreter's `assign_in_place` declines
  a row whose collection is no list or map BEFORE evaluating its arguments:
  evaluating first had read a moved row twice ("expected numbers, got
  done").
- A mask picks a frame's rows: `f[mask]` (`filter` removed from the
  language 2026-10-16, an unknown name like any other) is lowered at the
  index site (call_inner) to the hidden row `core.__pick_frame`
  (`frames::pick_rows`), which `fuse.rs` turns into `core.__pick_where`
  (`mask::pick_where`) where the mask compares the frame's own columns;
  any other index of a frame is refused.
  A column picked by a mask is `__pick_cells` (mask_pick), a column of its
  kind. A pick's call goes through `picked`: it propagates (and is typed as
  one that can fail) where an argument is an unwrap mark, because the
  emitter reads such a mark only in a propagating call (`raw`) — `xs[mask]`
  of a list read from a file had not built. A column of bools under `not`,
  `and` or `or` is lowered as `c eq true`. `uses::frame_of` follows a
  frame through its index.
- A list of lists of numbers is `Arg::Rows`, not `Arg::Tensors`: no list
  of tensors' parameter takes it (`Kind::Tensors`), a tensor's does as a
  matrix (`stack[rows]` is `math.stack_rows` of it). Both backends make the
  matrix through `CrowTensor::from_elements` of the rows.
- The standard modules (`src/modules/*.co`) are compiled into the crow
  binary: an edit reaches a program only after a rebuild (`crow test
  src/modules/x.co` reads the file itself, so the module's own tests pass
  first).
- `crow_web`'s reqwest takes rustls (`default-features = false`): its
  default TLS is OpenSSL on Linux, which the 0.6.1 release binary needed
  (`libssl.so.3`) while the docs said otherwise. CI and the release check
  the Linux binary's NEEDED libraries. The runtime bundle keeps what the
  `TARGETS` line in `scripts/bundle_runtime.sh` builds (by `cargo metadata
  --filter-platform`, which reads build dependencies for the target, as
  `cargo tree --target` does not) and stubs the rest: a new platform is a
  word there.
- `crow check --self` (src/selfcheck.rs) holds Crow to itself: cases
  generated from the compiler's tables (every row × the kinds it takes,
  from `tried_first`, a `Kind` match with no wildcard; every collection ×
  every index; every operator × every operand pair; value shapes; a helper
  at two kinds; ints past 64 bits; linked records), lowered in process
  (`program_of_source`, a thread each: a panic is a finding), then the ones
  that lower run in batches — a function per case (`self_case_N`, each case
  with its OWN copy of every helper, `self_may_N`, so a batch is its cases
  alone and a rustc error is placed in its case by the number) and a
  dispatch from `CROW_FROM` to `CROW_TO` — under five configurations:
  interpreted, walked (`CROW_WALK=1`: the interpreter's traced path, every
  body walked and every loop in order), unfused (`CROW_NO_FUSE=1`), unmoved
  (`CROW_NO_MOVES=1`: `place_moves_in` returns at once) and compiled. Every
  process is held to a memory budget (RAM less 9 GB in all, `--memory GB`
  says another; 1 GB each, footprint polled through libc; over it the case
  is a `memory` finding),
  30 s and 1 MB of output: the first version ran sixteen `range[2^63]`s at
  once and took the machine down. A run stopped from outside may lose its
  buffered markers, so the cases from the last marker run one per process
  until the culprit is found. Findings: `target/crow_self/findings.txt`,
  grouped in `summary.txt`; refusals in `refusals.txt`; rows not run in
  `unexercised.txt`. `tests/self/known.txt` is the ratchet (a new finding
  or a gone one fails; `--bless` rewrites it). `--only word`,
  `--lower-only` (seconds), `--clean` (the builders' caches under
  target/crow_self/build/w*, kept between runs). Three more lists, changed
  only by `--bless` (which keeps the reasons people wrote), hold what is
  added later: `tests/self/not-called.txt` (builtins no case calls),
  `agreement-only.txt` (builtins whose cases run, none answered by
  `tests/self/reference.py`) and `forms.txt` (the AST's statements,
  expressions and operators no case reaches, read from `src/ast.rs` by
  `include_str!`) — a new builtin, module or form is tested or the gate
  names it. The operator family is the AST's own (`operators`, by serde
  from the variant names), and a row's import is the registry's
  (`modules::module_by_path`).
- A call evaluates every argument, left to right, before the leftmost
  failure is its value (spec, Auto-Propagation; 2026-10-17): the emitter's
  `propagation_block` binds each argument that can fail (`_prop_N`) and
  each that can stop or act (`_pa_N`, all but the `quiet` ones) in their
  order, then checks — a failed argument had skipped the ones after it.
  A builtin that prints an argument again from the IR reads the bound
  name (`unwrap_as`, consulted at the top of `expr()`), or it runs twice.
  A cast beside a comparison is parenthesized (`((x) as f64) < y`: Rust
  reads `f64<` as a type's arguments).
- A function called at several signatures answers nothing known until its
  copies exist (`awaits_clones`, the one rule `new_clones` reads too), and
  the program is scanned again after copying: the first rounds' guess —
  one copy's answer — had typed the parameter of a function handed
  another copy's answer, and stayed. A function taking a function as a
  value is not copied (v1), so it never awaits.
- A row written in Crow takes its arguments as its function's parameters
  are typed, with none of the printing a Rust row's kind does
  (`Kind::print`): `crow_call_args`' `read_as` converts each — an int to
  a float, a float to its whole part (`__truncated`), any map to a map of
  text (`__text_map`) — and gathers the rest of the arguments into a list
  where one can fail through a row (`__sizes`, `__groups`), so the call
  hands that failure on as any call does. A new kind on a `crow!` row's
  parameter needs its conversion there, or the compiled call does not build
  where the interpreter converted (table.rs's own conversion by kind).
- A function copied per signature is lowered muted at its own joined
  types (every call reaches a copy, which is checked), and a vector and a
  matrix are two signatures (`same_signature`'s ranks). A database
  connection and a response are argument classes of their own
  (`Arg::Db`, `Arg::Response`): a value shows them, and the kinds check
  refuses them elsewhere — classed as not known, `len[conn]` had passed.
- The self-check's routes family (`src/selfcheck/web.rs`, 2026-10-17)
  serves each batch per configuration: ports come from a counter below the
  system's ephemeral range (a port bound to 0, let go and handed to the
  server had gone to another batch first, whose server the readiness
  check then reached), and a server is a batch's only when it answers the
  run's token at `/self-ready`. What it found: the response forms took any
  value (`text[3]` did not build compiled; `respond[1000, …]` was 599
  interpreted, 65736 was 200 compiled) — now text, an HTTP status, a
  frame for `arrow`, refused in the lowering (`web_form`, `web_form_reads`
  in checks.rs; words in `crow_web::core::response`); a handler answering
  no response (an empty 200 compiled) and a function that can end without
  its value (`done` interpreted, no build compiled) are refused
  (`check_answered`); a `crow!` row's `Encoded` argument that can fail
  moves its unwrap mark onto the encode's answer (`crow_call_args`), or
  the mark is read by no one; `stream[feed, a]` and `later[f, a]` call
  their function BY NAME — the width pass flows their arguments into its
  parameters (`called_by_name`), never treating it as a value with an
  adapter. And a ui control named like a module's function (`select` of
  options beside `use frame`) skips that module's kinds check, as the
  lowering reads it (finch's dropdowns had been refused).
- The views family (`view_cases`, 2026-10-17) draws each case as a page
  and reads the page root's markup (`page_root`). It found the compiled
  page unable to draw most kinds of child (a tensor, a map, a record, a
  moment, a parsed value, a column: handed over raw) — a child is now the
  text `crow_fmt` writes, a list's items each (`child_items`, nested lists
  flattened as the interpreter splices them), a column's cells each
  (`crow_web::view::column_children`, a missing cell failed); the
  interpreter's `render_element` takes the element's own attributes as its
  FIRST value, and had read every map child as attributes; the two
  renderers escaped `"` in text differently (one escape now:
  `crow_web::view::escape_text`/`escape_attr`); classes join once, an empty
  one adds none, a computed one merges at run time (`with_class`). A view
  called outside views answers its markup (the book hands one to
  `doc.document`, a derivation builds rows of one): compiled, a node held
  where markup is — `log`, a local or a top-level name — is rendered
  (`CrowDisplay for CrowVNode`, `emit_let_value`, both facts' paths); a
  refusal of such calls (2aa84fc) broke three book blocks and was undone.
- The state family (`state_cases`, 2026-10-17) serves each case as a
  page with top-level lines of its own (facts, handlers, derivations,
  named `@N`) and drives its events through `served_parity::page_steps`
  (split out of `drive`, which starts the server itself). It found a
  record fact's field written in a handler lowered as a local's
  `SetPath` (interpreted "undefined variable", compiled it changed): a
  path write to a fact is `let _t = load f; _t.… = …; store f ← _t`
  (`path_write`) — `store` is the one write to state; and a select bound
  to a fact taking any value a forged event sent: `Offers` keeps each
  choice's values (a select's options, a radio's value, in elements and in
  markup) and refuses another, unless a free input binds the same fact.
- `tests/self/reference.py`'s keys are the cases' `what`: a module's row
  by the path it is imported from and its name (`math.stats.median`,
  `math.abs` for the tensor form), a core row by its bare name (`encode`,
  `sub`, `hash`, `fraction` — one name for several rows, told apart by
  arity or kind) — not the row's id (`stats.median`, `math.abs_t`), which
  the agreement-only list is keyed by. A reference that answers within a
  tolerance returns `Near` (8 ulps) or `Close` (1e-9 of the answer's
  largest number); a size no case could hold is `Skip`, or numpy raises and
  the whole reference run fails. Its first findings: `factorial` of a whole
  number was inexact (a Lanczos gamma; libm's `tgamma` now, exact to 22!).
- The check's own servers (`Servers::start`): an echo/HTTP server
  (`CROW_SELF_PORT`), a ClickHouse stand-in (`CROW_SELF_CH`), a scratch
  Postgres (`CROW_SELF_PG`), a WebSocket echo (`CROW_SELF_WS`) and a mail
  server (`CROW_SELF_SMTP`, RFC 5321's replies, `AUTH PLAIN` offered); a
  live case's id names its server (`ws-live/`, `mail-live/`) in the live
  check, and `live_rows` says which rows its answers retire from the
  agreement-only list. A mailer's user is the sender's address (`'u'` is
  refused, `the mailer's user 'u' is no address`). Two self-checks must not
  run at once in one tree (they share target/crow_self), and one beside a
  gate in its worktree halves both: run them in turn.
- What the reference found once it reached the modules (2026-10-17):
  `factorial`/`gamma` were a Lanczos approximation (3! was
  6.000000000000007; libm's `tgamma` now, exact to 22!); a trailing window
  (`rolling_sum`, `vwap` over seconds, `rsi` on buckets) over times that
  go back summed rows by no reading of "trailing" — it stops now, saying
  where (`in_order`, crow_runtime::indicators). The spec gained what the
  cases asked and it had not said: `onehot`'s classes are 1 or more,
  `env`'s default is held as a cell (an int past 64 bits refused). The
  draws family holds random draws to their distributions with a fixed seed
  (deterministic, so never flaky); the self-check's batch builds keep no
  debug info (`CARGO_PROFILE_DEV_DEBUG=0`) so their programs stay a few
  tens of MB and are kept for the next run, the cache let go past 80 GB.
- The io family (`io_cases`) gives each case a directory of its own
  (`c<N>/`, the run's files copied in; `Case.own_dir`, a batch of its own):
  the row family's io cases shared one directory, so an answer depended on
  the cases before it. It found a field asked of a value that can fail
  holding a list (`csv[…].b`) refused nowhere — a runtime stop
  interpreted, no build compiled: `check_field` reads the payload now.
- A page's `later[f]` runs f on the page's state (`spawn_later` takes a
  plain `fn(&mut CrowLiveState)`), so `later[f, a]` in a page's handler
  is refused in both modes (errors j45); a script's carries its arguments
  (parity 25, 259). Compiled, the arguments had been dropped and the page
  did not build, where the interpreter ran the call inline.
- `nodes[n]` is removed (2026-10-11; its message of its own went
  2026-10-16, as `filter`'s did — a replaced name is an unknown name): the
  process count is `crow launch <n>`'s, a process reads its rank with
  `env['CROW_RANK', '0']`, and `nodes[…]` is "undefined
  function 'nodes'". The runtime's ring (`crow_runtime::distributed`) has no
  builtin yet.

- `crow check --self` holds its cases to the right answer, not only to
  each other: a case's `refer` names what it computes, the agreed outputs
  go to `target/crow_self/answers.jsonl`, and `tests/self/reference.py`
  (run `python3 -E`: numpy lives in the user's site-packages, which `-I`
  hides) answers from the spec — libm for a number's functions, numpy for
  math's reductions, Rust's float display — skipping what it has no answer
  for, never guessing. A value is written into a case as its text, so
  `big` is `2 ^ 100`: `2 ^ 100 ^ 3` is `2 ^ (100 ^ 3)`. The `pair/` family
  varies two of a row's parameters at once over `PAIRED` (edges: 0, a
  negative, past the end, past 64 bits, a float, empty text, `[]`, a
  vector, a matrix, a value that can fail): one at a time had never met
  `slice[m, -1, 3]` of a matrix. The check runs servers of its own on the
  loopback (an echo/HTTP server, a ClickHouse stand-in, a scratch Postgres
  where `initdb` is on the PATH), and a `live` case runs without the
  replay, which would answer `connect` in the server's place.
- A tensor's `len()` is its NUMBER of elements; a collection builtin
  walks a tensor's items — a vector's numbers, a matrix's rows — so it
  counts `shape()[0]` (`core::items_of`): slice, reverse and pop had
  counted a matrix's numbers and answered one axis its type did not say,
  and sort, `x in m` and join of a matrix are refused (rows have no
  order, a number is no row, a row shows as no text). A power binds
  tighter than a leading minus (`-x ^ 2` is `-(x ^ 2)`, parse_unary →
  parse_power). A tensor divided by a number divides each number
  (`div_scalar`): a multiply by the reciprocal rounds twice. A Rust `sum`
  of no floats is -0: a sum a program sees folds from +0.
