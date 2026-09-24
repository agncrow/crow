# One meaning in one form: the IR (design round)

*Decided 2026-09-08 as the first move of stage 4 in
`core-rewrite-notes.md` §9, after the four choices in that discussion:
status lives only where a value can fail; the IR interpreter is built
first, behind a flag, then the Rust backend; the three meanings of
`core-design.md` land one step after the IR; capabilities and typed
frames are the second half. This record is the contract the code is
built to. It is written in the spec's voice so that its sections move
into `language-spec.md` when the code lands; until then the spec
describes what runs today.*

## 1. The problem in one paragraph

The parser produces a tree. Two programs then each decide what the
tree means: the interpreter while running it (`interpreter.rs`, 8.7k
lines) and the transpiler while writing Rust (`transpiler.rs`, 8.4k
lines, on top of `type_infer.rs`, 2.8k). Every rule exists twice, and
every rule they disagree on is a bug found by running programs in both
modes: the parity corpus is 47 programs long because of it, and stage
3 found eleven more disagreements in a week. The fix is one more step
after the parser in which every decision is made once, and two small
programs that only carry it out.

## 2. The pipeline

```
source ─ lexer ─ parser ─▶ AST
                            │  lower.rs      named arguments → positional (exists)
                            │  type_infer.rs types, schemas, specialisations (exists)
                            │  validator.rs  scope, purity, wiring (exists)
                            ▼
                          lowering ─▶ IR      (new: ir.rs, lower_ir.rs)
                                       │
                          ┌────────────┴────────────┐
                    ir_interp.rs               ir_rust.rs
                    walks the IR               prints Rust from the IR
                    `crow run -i`, tests,      `crow run`, `crow build`
                    check, book, harness
                                       │
                                  crow_runtime, crow_web, crow_db
                                  (every operation, once)
```

The three existing front-end passes stay: they already decide types
once. What changes is that their decisions are written into the IR and
never re-derived downstream. The interpreter and the transpiler are
deleted when their replacements pass the acceptance suite (§9).

## 3. The IR

The IR is a typed tree. Every node carries its type; every conversion
is a node; every call names the runtime function it means; every
effect is marked answered or held; every last use of a variable is a
move. There is no inference, no defaulting and no dispatch on runtime
type anywhere below the lowering.

### 3.1 Types

```
Ty ::= Int | Float | Bool | Str | Unit
     | List(Ty) | Map(Ty)                      -- one element type, as the spec requires
     | Tensor(dtype)                           -- rank known at run time, dtype f64 today
     | Frame(Schema?)                          -- columns and their types when describe knew them
     | Column(Ty)                              -- a frame column read
     | Json                                    -- a parsed JSON value
     | Record(name, [(field, Ty)])             -- a struct, or a runtime record (shell, find)
     | Result(Ty)                              -- a value that can fail
     | Fn(sig) | Handle(kind) | Node           -- function value, connection, view tree
```

**Status lives only where a value can fail** (decision 1). A `Result`
appears where the type checker knows the value can fail: readers,
queries, conversions from text, `find`, `pop`-free arithmetic never.
`.ok`, `.reason`, `.kind` and `.trace` on a non-result are constants:
true, empty, empty, empty. A column's `.ok` is its validity bitmap, as
documented. `.ok` on a list is the list's own status; the statuses of
its items are `for x in xs` `x.ok`. This settles the residuals in
`core-runtime-builtins.md`: `matches` and `sub` become `Result(Bool)`
and `Result(Str)`, and a list of booleans is `List(Bool)`, compact.

### 3.2 Declarations

```
Program  ::= { parts: [Part], entry: PartId }
Part     ::= { prefix, facts: [Fact], derives: [Derive], fns: [Fn], views: [Fn],
               handlers: [Fn], tests: [Test], serve: Serve? }
Fact     ::= { name, ty, default: Expr, session: bool }        -- session: written by a handler
Derive   ::= { name, ty, body: Block, reads: ReadSet, fresh: secs? }
Fn       ::= { name, params: [(name, Ty)], ret: Ty, body: Block,
               kind: Pure | Helper | Handler | View, effects: [EffectKind] }
ReadSet  ::= { facts: [name], derives: [name], tables: [name], clock: bool }
```

A function called at two signatures is two `Fn`s (the transpiler's
specialisation, now visible). A `Helper` is a function a handler calls
that writes a fact; it runs in the handler's transaction. `effects`
lists what a function can do, computed transitively, and is what
capabilities will check later.

### 3.3 Statements

```
Stmt ::= Let   { var, ty, expr }                    -- first binding
       | Set   { var, expr }                        -- rebinding a local
       | Store { fact, expr }                       -- the only write to a fact
       | If    { cond: Expr(Bool), then: Block, else: Block? }
       | While { cond, body }
       | For   { var, iter: Expr(List|Tensor), body, collect: var?, parallel: bool }
       | Return(Expr?) | Break | Continue
       | Do(Expr)                                   -- an expression for its effect
       | Propagate { values: [var], trace }         -- return the first error among values
       | Stop     { value: var, trace }             -- a handler meets an error: stop and report
       | Line(n)                                    -- source line, for traces and the checker
```

`For` is parallel when nothing its body does can show the order its
iterations ran in (spec §14, For Loops): no write to an outer variable
or a fact, no way out of the function, no effect, draw, render or
assertion, directly or through a function it calls. The lowering
decides once every function is known, not the runtime. Order of
statements is the order written (spec §14).

`Propagate` is the spec's auto-propagation made visible: before a call
whose arguments include a result, the lowering emits `Propagate` for
them; inside a function it returns the error, in a handler it becomes
`Stop`, at top level it halts. Both backends do exactly this and
nothing else with errors.

### 3.4 Expressions

```
Expr ::= Const(value, ty)
       | Local(var) | Move(var)                     -- read; read for the last time (moves)
       | Load(fact) | Derived(name)
       | Call    { target: Runtime(name) | User(name) | Module(mod, name), args, ty }
       | Effect  { kind, args, ty, answered: bool }  -- query exec insert read write fetch shell mail later notice now
       | Convert { how: IntToFloat | Unwrap | ToStr | ..., expr }
       | Binary  { op, l, r, ty } | Unary { op, expr }
       | Index { base, index } | Field { base, name } | Slice {..}
       | Interp([Str | Expr])                        -- string interpolation
       | ListLit([Expr]) | MapLit([(Str, Expr)]) | RecordLit(name, [Expr])
       | Tree { element, attrs: [(name, Expr)], children: [Expr] }   -- a view node
       | ForExpr { var, iter, body, yield }          -- the collecting loop
       | FnRef(name)
```

`Move` is placed by a liveness pass in the lowering: a variable read
for the last time in its block moves. That is the general form of the
in-place rule the collections got in stage 3: every runtime call that
takes its collection by value gets it moved when nothing reads it
after, in both backends, with no special cases in the emitters.

`Effect.answered` is the rule of `core-design.md` §1.2: the value is
used, so it runs now; unused, so it is held to commit. The lowering
decides from the text; the session layer (§7) does the holding.

### 3.5 Runtime calls

The IR never says how to push, sort, slice, join, encode, format or
match: it says `Call(Runtime("core.push"), ...)`. One table maps every
runtime name to its Rust path and signature; the Rust backend prints
the path, the IR interpreter dispatches through the same table with
its typed values. A builtin added later is one row and one function.

## 4. Typing decided once

The lowering fixes, in one place, the rules that today live in two:

- **Int and float.** A variable or fact that ever holds a float is a
  float from its declaration; every int written to it gets
  `Convert(IntToFloat)`. A literal's type is what the checker found;
  an empty `[]` or `map[]` takes the type its later writes give it.
- **Results.** A result consumed by an operation that needs the payload
  gets `Propagate` before and `Convert(Unwrap)` at the use. Storing a
  result into a fact is `Stop` on error, then the payload (rule 2 of
  `core-design.md`, landing in §7).
- **Frames.** Cell types come from `describe` when the query is a
  literal, otherwise the frame is `Frame(None)` and cells are `Json`
  until read through `int[]`, `float[]`, `str[]`, exactly as today's
  transpiler does. Typed columns and fusion are the second half (§8).
- **Views.** A view's body is a `Tree`; a child that is a list
  contributes each item; a string starting and ending with a tag is
  markup; anything else is text. The kernel renders the tree in both
  backends.
- **Facts and sessions.** A fact a handler writes is per session; the
  read sets of derivations decide who holds them, as `derive-design.md`
  says; the lowering computes both.

## 5. The IR interpreter (`crow run -i`)

Values are typed, immutable, and small:

```
Val ::= Int(i64) | Float(f64) | Bool(bool) | Str(Arc<str>) | Unit
      | Ints(Vec<i64>) | Floats(Vec<f64>) | Bools(Vec<bool>) | Strs(Vec<String>)   -- lists of scalars
      | List(Vec<Val>) | Map(BTreeMap<String, Val>)
      | Tensor(CrowTensor) | Frame(CrowFrame) | Column(CrowColumn) | Json(serde_json::Value)
      | Record([Val]) | Result(Box<Res>) | Fn(name) | Handle(..) | Node(..)
Res ::= { ok, value: Val, reason, kind, trace, captured }
```

A list of booleans is a `Vec<bool>`, a column read is a `Column`, a
million-row filter allocates one bitmap. Scopes are reference-counted
and copied on write, as the interpreter does since stage 3. `Move`
takes the value out of its slot; everything else clones, and clones of
tensors, frames and strings are pointer bumps. There is no dynamic
dispatch on the shape of a value: the IR said the shape.

## 6. The Rust backend (`crow run`, `crow build`)

Types map one to one: `Int`→`i64`, `List(Str)`→`Vec<String>`,
`Result(T)`→`CrowResult<T>`, `Tensor`→`CrowTensor`, and so on, the
table today's `to_rust_type` already has. Expressions print directly;
`Move(x)` prints `x`, `Local(x)` of a non-copy type prints `x.clone()`.
The backend has no `infer_expr` and no `if name == "..."` chain: it
has the runtime table and the printing of each node kind. The
generated crate keeps its shape (`.crow_build`, the runtime crates by
path, `--release` for `crow build`), and the warm-up rule in the
benchmark stays.

## 7. The session layer (one step after the IR)

*Design and plan, decided 2026-09-23 after measuring finch:
`session-layer-design.md` — what the layer adds beyond the list below
(derivations and handlers off the loop, one session's facts in both the
harness and the server, writes that reach every reader, sessions that
outlive a connection, the URL, confirmation, patches, then sign-in,
files, big tables and widgets) and the order to build it in. It moves
the session layer ahead of step 4.*

`crow_runtime::session` implements `core-design.md` for both backends:

- **Facts**: a versioned store per app and per session; a handler runs
  on a working copy; `Store` writes the copy; commit lands the copy as
  one and advances the version; derivations compare versions.
- **Effects**: answered effects run when asked; held effects go to a
  log the commit performs in program order and a stop discards. Store
  writes open one transaction per connection, committed at the end.
- **Stop**: `Stop` discards the copy and the log and delivers the
  reason as the session's notice; `notice[text]` is a held effect and
  `notice` a runtime-provided derivation with `text` and `at`.
- **Moment**: `now()` is a field of the event; the harness sets it
  with `change[a, 'now', t]`.

The harness (`app`, `click`, `change`, `html`, `box`, `fits`) is a
client of this layer, so `click` is one transaction and rehearsal is
the same layer with the effect log recorded instead of performed.

## 8. The second half

After the IR and the session layer pass the suite:

- **Capabilities.** A part declares what it needs; the app binds; a
  test binds a double. `Fn.effects` is what the checker verifies
  against the declaration. One line per part is the whole syntax.
- **Typed frames and fusion.** `Frame(Schema)` everywhere `describe` or
  a literal knows it; a relational sublanguage in the IR (`filter`,
  `select`, `group`, `join`, `window`, `sort` over typed columns) that
  the Rust backend fuses into one pass; `derive` as the lazy boundary.
- **Shapes.** `Tensor` with rank and literal dimensions in its type so
  element-wise chains fuse and mismatches are refused before running.

## 9. Building it: gates and order

Each step lands only when every gate is green, and the old code is
deleted in the same commit that makes the new path the default.

| Gate | What it proves |
|---|---|
| `cargo test --workspace` | the units |
| `crow test --parity`, 47 programs | both modes agree, and agree with before |
| `crow test --book`, 130 blocks | the documented language runs in both modes |
| `tests/layout/conformance.co` | the layout engine is untouched |
| finch's baseline, 96 declarations | the app's meaning is unchanged, or changed exactly as intended |
| `crow test --bench` | no program slowed down |

1. **This record reviewed.** Then `ir.rs` (the types of §3, about a
   thousand lines) and a printer, so `crow build --ir` shows a
   program's form; the clock and the skip handler are the first
   programs printed. *Built 2026-09-08: `src/ir.rs`.*
2. **Lowering.** AST to IR using the three existing passes; the
   parity corpus is lowered and printed, and the printouts are read.
   *Built 2026-09-08: `src/lower_ir.rs`; every parity program lowers
   (a unit test keeps it so), and the clock fixture and finch's nav
   part print as §10 has them, with the propagation and the float
   conversion of a cell the sketch had elided.*
3. **The IR interpreter**, behind `CROW_IR=1`, run against every gate
   until green; then it is `crow run -i` and `interpreter.rs` goes.
   *Started 2026-09-08: `src/ir_interp.rs` walks the IR over the
   interpreter's values, calling the modules both walkers share:
   `builtins.rs` (the sixty-four state-free builtins), `ops.rs` (the
   operators, indexing, field access, literals and loop items) and the
   module registry. `crow test --parity --ir` and `crow test --book
   --ir` count what it runs identically. The first tally was worthless:
   the runner put the interpreter flag after the file, so it compiled
   the program and compared the compiled mode with itself; with the
   flag in its place the walker ran 35 of 47 parity programs, and after
   a day of fixes (qualified effects keep their module, bare module
   names resolve at run time by the interpreter's own rule, function
   values applied through indexing, regex `find`, `captured`, `grad`,
   `later`, `transact`, specialised names, and a liveness slip that
   moved a value read again in a branch) it runs all 47, and 115 of the
   book's 130 blocks. The 15 left were views, `page` and the harness.
   Later the same day the live layer followed: views render through
   `ops::render_element` and `ops::render_input` (the element and
   bound-input renderers both walkers share), the harness verbs (`app`,
   `html`, `elements`, `change`, `click` with a deferred call,
   `box`/`fits`/`style` with the layout engine lowered through the
   same pipeline into the walker's functions), the served-mode mocks,
   `icon` and `progress`, and test blocks: `CROW_IR=1 crow test
   file.co` runs a block under the walker, lowered as the `Test` item
   it is with its inputs bound at run time (lowered at the top of a ui
   program its statements would become facts, evaluated before main —
   the first attempt's stale render). Tallies: 48 of 48 parity
   programs, 130 of 130 book blocks including their test blocks, the
   layout conformance suite, all identical. Three older bugs surfaced
   on the way, each fixed with a test: type inference inferred every
   call argument twice, so a map literal's nested `set[]` chain cost
   doubled per entry and the engine's 95-entry width tables never
   finished (`crow build` on any 25-entry literal hung; arguments are
   inferred once now); the compiler indexed a map by a `keys[]` loop
   variable as `&&String` (parity 48); and the compile-error
   translator turned `&String` into `stringing`. Left for later: the
   engine does not compile as a standalone program through the old
   transpiler (a `k.tag` on a list item and a `float` return), which
   nothing needs — the Rust backend will compile it from the IR.*
   *Built 2026-09-08, late: the IR interpreter is `crow run -i`, and
   `interpreter.rs` (6,664 lines) is deleted. Its 381 behaviour tests
   moved to `interp_tests.rs` and run through the walker; the checker's
   probes (determinism, snapshots, test blocks), the REPL, `crow
   check`'s try command, the crowfile reader and the wireframe tool run
   on it through a small API (`prepare`, `run_script`, `Session`,
   `call`, `layout_html`). The harness's own pieces (HTML parsing,
   selectors, escapes, the engine's source, the box) are `harness.rs`.
   The walker's function tables are shared references, its scopes are
   copied on write, and a loop the lowering marked parallel runs in
   chunks on clones of it, for the collecting form too; `x = push[move
   x, v]` and the other in-place builtins grow the value in its slot,
   as the compiled mode does. Everything found on the way was fixed
   with a test: the validator judged a function's local against the
   module variable of the same name, and inference and both lowerings
   typed it that way (the type context's body scope now makes a body's
   first assignment of a non-fact name a new binding, spec §7 — parity
   49); the removed time builtins' teaching errors were the AST
   walker's alone and are the validator's now, for both modes; the
   resource statements `gpu[]`/`nodes[]`/`cpu[]` and the web module's
   stand-ins (`cors`, `sessions`, `templates`, `render`, `cache`,
   `limit`, `uploads`, `sse`, `ws`, `connect`, `send`, `recv`,
   `close`, `static_dir`, `smtp`, `msgpack`, `arrow`, `bytes`,
   `stream`) and `chart` were ported; a test block lowered without a
   name was judged as one attached to a function. The gates: workspace
   tests, parity 49 of 49, docs, book 130, layout, chart harness,
   finch's baseline unchanged, and the benchmark: against the last
   AST-interpreter binary on the same machine, every whole program is
   level or faster (the collect program by a tenth, frames by a
   twelfth); by section, the map-set loop, join/split and the push loop
   are faster (the in-place path, and a string-building loop the old
   effect test never let run in parallel — an interpolation reaches
   only what its spans reach now), and two are slower at their sizes,
   the plain collecting loop by an eighth and the small sort by a
   fifth, which the typed values of §5 are the answer to, not this
   walker.*
4. **The Rust backend**, the same way; then `transpiler.rs` goes.
   *Hand-off, 2026-09-08 night, for the session that starts it — read
   this before anything else.*

   *Where things stand.* Steps 1–3 are done and committed (3944c4c):
   `ir.rs` is the form, `lower_ir.rs` produces it from the front end,
   `ir_interp.rs` walks it as `crow run -i`, and `interpreter.rs` is
   gone. `transpiler.rs` (8,285 lines, 267 unit tests) still compiles
   programs from the AST with its own inference passes; it is the only
   consumer left of the AST after validation. The runtime crates the
   generated code links are the ones the walker calls
   (`crow_runtime::{core, frame, display, io, tensor, stats, layout,
   moments, later, text, json, distributed, sparse, gpu}`,
   `crow_web::{view, live, server, session, rate, mail, docs, cors,
   core}`, `crow_db::{connect, DbHandle}`), so the backend emits calls
   into what exists; it invents no runtime.

   *The shape.* One file, `src/emit_rs.rs`: `pub fn emit(program:
   &Program, source_file: &str) -> Crate` where `Crate` holds the
   `main.rs` text and what the crate needs (`rayon`, `serde_json`,
   `crow_runtime` with its `arrow` feature, `crow_web`, `crow_db`),
   decided from the program's `uses`, `Fn.effects` and types instead
   of the transpiler's `needs_*` flags. `transpile_and_compile` in
   `main.rs` swaps `Transpiler::new(ctx).transpile(&stmts)` for
   `lower_ir::lower` then `emit_rs::emit`; the `.crow_build` crate, the
   runtime crates by path, `--release` for `crow build` and
   `CROW_BUILD_PROFILE=dev` for the runners all stay. The emitter is
   the printing of each node kind of §3 and the type map of §6
   (`to_rust_type` today), nothing else: no `infer_expr`, no name
   chains. `Move(x)` prints `x`; `Local(x)` of a non-copy type prints
   `x.clone()`; `Convert` prints the cast or the unwrap; `Propagate`
   prints the early return of the error in a function and the halt at
   the top level; `Stop` prints the handler's stop; a held `Effect`
   prints in place until the session layer (step 5) gives it a log;
   `For … parallel` prints the rayon chunk loop the walker's
   `run_parallel` mirrors and `ForExpr … parallel` its collecting
   form; `Tree` prints `crow_web::view` construction; a `Call` to a
   `Target::Runtime` prints through `RUNTIME_TABLE` (31 entries:
   `push`→`crow_runtime::core::push` …) and a `Target::Module` through
   the registry's Rust names, which `lower_ir` should record on the
   node (the walker resolves bare module names at run time by the
   registry rule; the emitter cannot, so resolve them at lowering and
   print the resolution).

   *What the IR must carry that the transpiler computes for itself.*
   Put these into the lowering, not the emitter: which parameters pass
   by reference (`compute_by_ref_params`: non-copy, never mutated),
   which locals are `mut` (a `Set` on a `Let`), which structs are
   `Copy`, the recursive-struct-with-sentinel case (`OptBoxStruct`),
   the functions `grad` differentiates (`emit_dual_function`: a
   duplicate over `CrowDual`), `later` targets, the filter shape of a
   collecting loop (`is_filter_pattern` → an iterator filter), the
   search builtins that need rayon, the database calls' parameter
   guarding (`db_params_to_json_guarded`, `cell_getter` from
   `describe`), and the per-type display of `log`
   (`emit_log_call` → `crow_runtime::display`). Each is a field on the
   node or the declaration, printed by `crow build --ir`, so the walker
   and the emitter read the same decision.

   *Order of work, each slice gated by the parity programs it covers,
   behind `CROW_RS=1` until every gate is green (the pattern of step 3:
   `crow test --parity --rs` as the advisory tally, and put the flag
   before the file in the runner — the first tally of step 3 was an
   artefact of the flag after it).* (a) Scripts: scalars, strings and
   interpolation, lists, maps, structs, functions, loops, results and
   propagation, the state-free builtins — parity 01 to 20, 26, 41,
   43–49. (b) The runtime modules: frames, tensors, json, text, io,
   moments, regex, rand — the rest of the parity suite and
   `tests/bench`. (c) `use ui` programs: facts as `CrowLiveState`
   (`state.get`/`state.set`, the transpiler's state preamble), views
   through `crow_web::view::CrowVNode` and `children_of`, handlers,
   derivations as `__derive_x(state)` with `DeriveSpec`, `page`/`serve`
   through `crow_web::live::configure`, the web extras (cors, sessions,
   rate, mail, docs, static, arrow/msgpack/bytes/stream, ws/sse), and
   the database through `crow_db` — the book's ui chapters and finch's
   baseline are the gates; test blocks never compile (the walker runs
   them). (d) Delete `transpiler.rs` in the commit that makes the
   emitter the default; port the 267 unit tests that check generated
   code's shape to the emitter and drop the ones the parity suite
   already proves; then fold `lower.rs`, `type_infer.rs` and
   `validator.rs` as §12 decided.

   *What the emitter must not inherit (building finch, 2026-09-22).*
   Four compiled-only bugs were fixed in `transpiler.rs`; the emitter
   must print the same meanings. A handler reads a derivation at the
   point of use, after its writes land (`refresh_derivations`: the
   transpiler flushes the handler's own writes and reads the derivation
   again before each statement that reads it; a snapshot at the
   handler's start announced stale values). A Crow name Rust reserves
   (`move`, `impl`, `match`) is renamed for Rust after the gate
   (`rust_names.rs`; struct fields are written raw, `r#type`), and a trace
   still names it as written. An operator hands a failed operand on with
   its trace unchanged; only a call receiving an error and a branch
   refused in a function add a place, and a top-level halt reads
   `runtime error: line N: cannot branch on an error value — …`. And
   sqlite's computed columns are typed from the statement when the
   describe probe returns no row (`sql_types.rs`). Parity 50–52 hold the
   script-level ones; the handler read is pinned by the transpiler test
   `a_handler_reads_a_derivation_after_a_write` and needs its emitter
   twin.

   *Gotchas kept from step 3.* The book runner's worker caches
   (`$TMPDIR/crow_book_test/w*/.crow_build/target`) can rot (a missing
   libsqlite3-sys `bindgen.rs` shows as "does not build" on database
   blocks): wipe them and rerun. `crow build` from the repo root
   creates 0-byte sqlite files named by the program (the compiler asks
   the database): build scratch programs in a scratch directory.
   Debug-mode unit tests lower every parity program: a literal nested
   more than about a hundred deep overflows the test thread's stack.
   For an A/B against the last committed binary, `git worktree add
   <scratch>/crow_ab <commit>` and `cargo build --release` there, then
   run both binaries back to back; remove the worktree after.
   `CROW_IR_SHOW=1 crow test file.co` prints a test block's lowered
   program. The benchmark's columns compare within a recording, not
   across recordings.*

   *Done, 2026-09-23.* The Rust backend is `src/emit_rs/`, six files
   and about 5,200 lines against the transpiler's 6,700, with its 270
   unit tests ported and one added: `mod.rs` holds the emitter's state
   and its typing helpers, `program.rs` the whole-program analyses and
   the crate's layout, `stmts.rs` functions, handlers, derivations and
   statements, `exprs.rs` expressions and calls, `builtins.rs` the
   builtins and modules, `ui.rs` trees and controls. `crow build`,
   `crow run` and every runner compile through it, and `crow build --rs
   file.co` prints the Rust. `transpiler.rs` is deleted, and with it the
   `CROW_RS` switch, three helpers only it used (`ident_count`,
   `in_place_call_on`, `infer_call_base`) and `CrowLiveViewLegacy`, a
   runtime type no generated program had used since the no-app web API;
   `chart_seek_handler` moved to `lower.rs`.

   *How it was built: fidelity first.* The emitter prints the Rust the
   transpiler printed, keyed on the IR's types instead of types
   re-inferred from the AST, so the generated programs kept their shape
   and every gate compared like with like. A local is typed by the
   emitter's own declaration of it (the IR's type of a variable is the
   union of its bindings, `Result(Frame)` where the first binding made a
   `Frame`); a call's argument is read through its `Convert::Unwrap`;
   propagation is decided as the transpiler decided it. Finch built
   both ways prints the same 135 items with the same signatures and the
   same 22 derivation specs; the bodies differ in clones, and in branch
   conditions hoisted into temporaries, and driven through every handler
   in a browser the two builds behave alike, the emitter's as fast or
   faster on every click.

   *What moved into the lowering.* A derivation's memo key
   (`Derive.spec`: the read set, and whether it is shared) and the
   tables each function writes (`Fn.tables_written`), so the walker and
   the emitter read one decision; `Ty::Markup`, `Ty::Tensor(elem)` and
   `Ty::OptRecord`, so the IR's types map one to one onto the compiled
   program's (`lower_ir::crow_type` is the inverse of `ty_of`); each
   specialization clone lowered as its own `Fn`; a `while` whose
   condition needs a prelude lowered as `while true` with the prelude and
   a break. Four lowering bugs the emitter exposed, each pinned by the
   parity program that found it: a function's assignment to a name the
   top level also uses is a local of its own, not a `Set` of the top
   level's (parity 14); a loop variable's type is bound while its body
   lowers (18); an interpolation with a failing span is a `Result(Str)`
   (33); a new local's type is its first value's (36).

   *What the emitter still decides itself.* Decisions only Rust needs —
   which parameters pass by reference, which locals are `mut`, which
   structs are `Copy`, the dual-number copies `grad` differentiates, the
   filter form of a collecting loop, the database calls' parameter
   guard, the per-type display of `log` — are made over the IR in
   `program.rs`'s `analyse`, not recorded on it; the walker needs none of
   them. Whether a loop runs in parallel was the one decision both modes
   made each for itself; since 2026-09-23 it is the IR's alone
   (`decide_loops` in `lower_ir.rs`, run once every function's kind and
   reach are known, Vijay's rule: no side effects, then parallel). A loop
   runs in order when its body writes an outer variable or a fact, leaves
   the function, reaches an effect, draws from the seeded generator,
   renders a view or a paged table, asserts, or calls — or hands to a
   builtin, or calls through a value — a function that does; the emitter
   prints `par_iter` exactly where the flag says so and the walker runs
   its chunks exactly there. Three older bugs came out with it: a seeded
   loop of `rand[]` drew differently on every run (parity 55); a
   collecting loop inside a function could not hand back an error from
   its condition, the walker stopping with "cannot return from inside a
   for loop" and the compiled program not building (the walker now
   unwinds to the call, the emitter gathers such a loop in a `for`, and
   inference reads a collecting loop's conditions when it types the
   function; parity 55); and `push[]` and `set[]` took a variable's text
   by move, so reading it again, or pushing a loop's variable, did not
   build (parity 56). The interpreter runs a loop over a pure function
   3.4 times faster (`collect`'s new `for_fn` section, 893 ms to 259 ms).

   *Found on the way.* A derivation read into a local at the top of a
   view was moved into the first call that took it and gone for the
   second (finch's clock tile; the emitter test
   `a_derivation_read_up_front_is_cloned_into_a_call`). An int widened to
   a float around a failing operand is unwrapped before the widening
   (parity 39). Two older bugs, found while checking the step's own
   fixes: the compiled program moved a value into `int[]`, `float[]`,
   `not`, `assert[]`, a view's child or a view's argument, so reading it
   again did not build (`n = int[s]` then `log[s]`; the transpiler did the
   same); each of those places now borrows the value (parity 54, served
   11). And the lowering had hoisted a failing `while` condition out of
   the loop, so it was tested once and the walker never left the loop;
   now it is tested before every pass (parity 54). Older than the
   emitter, and fixed on 2026-09-23: `len` of an error was the error in
   the interpreter and `0` compiled, because inference and the emitter
   kept `len` off the calls that hand an error on. It is typed and printed
   like every call now (parity 57). Fixing it showed that the modes also
   disagreed on `and` and `or`: the compiled program short-circuited
   plain booleans with Rust's `&&` and `||`, and with a failing operand
   it evaluated the right side only when the left succeeded, while the
   interpreter evaluates both sides, the left first. Vijay chose the
   interpreter's rule, the spec's as written (Design Note "`and` and `or`
   Evaluate Both Sides"): the emitter prints `&` and `|`, and binds both
   operands before either's error decides. A guard such as `if not r.ok
   or len[r] eq 0` is refused when `r` failed, in both modes; the guard
   is its own `if`. With `len` typed honestly, finch's
   `for i in range[len[u]]` became a loop over a list that can fail, and
   what that does was unspecified: the interpreter ran the statement
   form zero times and made the collecting form a one-item list of the
   error, and the emitter could not print either. Vijay chose to refuse
   it like a condition: the lowering binds the list, a `Propagate` that
   names the loop (`Decision::Loop`) refuses it, the loop walks the value,
   and inference widens the function's type as it does for a failing
   condition (parity 57). Controls (`select`, `field` and the rest) now
   render a failed argument as its reason, as elements and views do
   (served 12), and an index that failed halts in the compiled program as
   it does in the interpreter.
5. **The session layer** with the three meanings; the harness; finch
   rewritten to them and its baseline re-accepted, whose diff is the
   measure of what the meanings delete.
6. **Capabilities, typed frames, shapes.**

Estimates, honest: one to two weeks for 1 and 2 together, three to
four for 3, three to four for 4, one to two for 5, with 6 open-ended.

## 10. Worked cases

The notation is the printer's: one statement per line, types after a
colon, `←` a store, `!` an effect with `now` or `held`, `move` a last
read.

### 10.1 The clock (`sim/clock.co`), the part and two handlers

```
part sim_clock
  fact base: Float = 0.0            session
  fact anchor: Float = 0.0          session
  fact speed: Float = 1.0           session
  fact playing: Bool = false        session
  fact win_end: Float = 0.0         session

  fn clock_at(b: Float, a: Float, s: Float, moving: Bool, w_end: Float) -> Float  pure
    if moving
      return core.min(b + (now!now - a) * s, w_end)
    return b

  derive head: Float  reads {facts: base anchor speed playing win_end, clock}
    return call clock_at(load base, load anchor, load speed, load playing, load win_end)

  handler toggle_play()  effects {}
    if load playing
      store base ← derived head
      store playing ← false
    else
      store anchor ← now!now
      store playing ← true

  handler set_speed(s: Float)
    store base ← derived head
    store anchor ← now!now
    store speed ← s
```

What the form shows: `now!now` twice in one handler is one moment
(§7); the three stores in `set_speed` land as one; `head` is a
derivation over the clock, so a render recomputes it. Nothing in the
handler's text changed.

### 10.2 The skip (`sim/nav.co`), as written today

```
part sim_nav
  handler skip_symbol()  effects {query, notice}
    t1: List(Str) = core.push(move (load session.skipped), load session.sym)
    store session.skipped ← t1
    call next_gate_any()

  handler next_gate_any()  effects {query, notice}
    if load session.session_id == 0
      call session.say("start a session first")
      return
    b: List(Float) = derived band
    after: Str = interp("", derived clock.head)
    r: Result(Frame{symbol: Str, at: Int}) =
        query!now(load ch, "SELECT ...", [load session.day, after, load session.dataset_ids, b[0], b[1], load session.skipped])
    if not r.ok
      call session.say(interp("next in gate failed: ", r.reason))
      return
    propagate r
    if core.len(unwrap r) == 0
      call session.say("no symbol enters the gate after the play-head")
      return
    call session.focus((unwrap r).symbol[0])
    call clock.jump(convert float((unwrap r).at[0]))
    when: Str = fmt!(derived clock.head, time = "time", tz = "America/New_York")
    call session.say(interp("", (unwrap r).symbol[0], " enters the gate at ", when))
```

And after the session layer lands (§7), with the guard left to the
checker:

```
  handler next_gate_any()  effects {query, notice}
    r: Result(Frame{symbol: Str, at: Int}) = query!now(...)
    stop r                                   -- commits nothing, notice "next_gate_any stopped: …"
    if core.len(r) == 0
      notice!held("no symbol enters the gate after the play-head")
      return
    call session.focus(r.symbol[0])
    call clock.jump(convert float(r.at[0]))
    notice!held(interp("", r.symbol[0], " enters the gate at ", derived clock.clock_str))
```

Four paths become two; the skip's list write and the hop land
together or not at all.

## 11. What this deletes and what it keeps

Deleted: `interpreter.rs` and `transpiler.rs`, the parity suite's role
as a meaning check (it stays as a regression suite), the `flush_before`
and `reload_after` write-through, the per-builtin emission chain, the
`Value` type with status on every value. Where that stands on
2026-09-23: both files are gone; the write-through, the per-builtin
printing (now `emit_rs/builtins.rs`) and `Value` remain until the
session layer's meanings and §8's typed values replace them.

Kept: the parser, the three front-end passes, every runtime crate, the
layout kernel, the harness verbs, the baseline and the book, the
parity and book corpora, the escape hatches (raw SQL, `css/app.css`,
shell, HTTP) as visible effects.

## 12. Three more decisions (2026-09-08, Vijay: "go with your recommendation")

- **`crow check --show` prints the IR's form** once the lowering is the
  default (step 3 of §9), and the baseline's artifacts derive from the
  IR from then on, so the stored artifacts change exactly once and the
  app's book and the checker read one form.
- **`Json` cells stay dynamic** until the typed-frames half (§8): the
  IR keeps them as today's transpiler does, so no behaviour changes
  with the IR itself.
- **`lower.rs`, `type_infer.rs` and `validator.rs` stay as they are**
  through steps 1 to 4; they fold into the lowering after the
  transpiler is gone, starting with the type checker's single map of
  variables by name, which the lowering replaces with a scoped
  environment of its own.
