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

`For` is parallel when the body writes no outer variable and calls
nothing with an effect; the lowering decides, not the runtime. Order
of statements is the order written (spec §14).

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
   programs printed.
2. **Lowering.** AST to IR using the three existing passes; the
   parity corpus is lowered and printed, and the printouts are read.
3. **The IR interpreter**, behind `CROW_IR=1`, run against every gate
   until green; then it is `crow run -i` and `interpreter.rs` goes.
4. **The Rust backend**, the same way; then `transpiler.rs` goes.
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
`Value` type with status on every value.

Kept: the parser, the three front-end passes, every runtime crate, the
layout kernel, the harness verbs, the baseline and the book, the
parity and book corpora, the escape hatches (raw SQL, `css/app.css`,
shell, HTTP) as visible effects.

## 12. Open questions, to settle while building

- The printer's notation for the book's app chapter: should
  `crow check --show` print the IR form instead of its own?
- Whether `Json` cells become typed columns at the first `int[]` read
  or stay dynamic until the second half.
- How much of `lower.rs`, `type_infer.rs` and `validator.rs` folds into
  the lowering pass once the IR exists, and in what order.
