# Typed values: the interpreter's values, the compiled facts, one builtin table

*Proposed 2026-09-24; every decision in §3 taken as recommended
2026-09-25; under way (§5). The last three things `ir-design.md` §11 lists
as still standing after step 4: the interpreter's `Value` with a status
on every value, the compiled handlers' write-through of facts, and the
emitter's chain of per-builtin printing.*

## 1. What stands, and what it costs

**Every interpreter value carries a status.** `CrowValue` is 192 bytes:
the value (96) and, on every value, `ok`, `reason`, `kind`, `trace`,
`captured` and `markup`. A list of a million ints is 192 MB in `crow run
-i` and 8 MB compiled. The IR already knows which values can fail (only
a `Result` can), which ones carry a capture moment, and which strings are
markup. The interpreter re-derives all three at run time, on every value,
and the benchmark's two slow rows (the plain collecting loop, the small
sort) are that cost (`benchmarks.md`).

**Compiled handlers imitate the interpreter's facts.** A compiled session
keeps its facts as JSON (`CrowLiveState.data`, one `serde_json::Value` per
fact). A handler reads the facts it uses into Rust locals, so every call
of a helper that handles facts too is wrapped. `flush_before` writes the
handler's facts back to the JSON first, `reload_after` reads the helper's
writes back after, and `refresh_derivations` re-reads a derivation after a
write it depends on. That machinery exists only so compiled code means
what the interpreter means (`core-rewrite-notes.md` §2).

**The emitter prints each builtin by name.** `emit_rs/builtins.rs` is a
match on about 110 names that prints Rust text per argument type. Batch 6
(2026-09-24) moved the string builtins into `crow_runtime::text`, and the
audit (`core-runtime-builtins.md`) shows every state-free builtin either
shared or the same in both modes. What remains is the shape: the Rust
backend should print a call from a table row, and the interpreter should
call the same function (`ir-design.md` §3.5, §6). It cannot while the
interpreter's values are untyped, because a runtime function takes a
`&str` or a `Vec<i64>`, not a `CrowValue`.

## 2. The target

```
Val ::= Int(i64) | Float(f64) | Bool(bool) | Str(Arc<str>) | Markup(Arc<str>) | Unit
      | Ints(Vec<i64>) | Floats(Vec<f64>) | Bools(Vec<bool>) | Strs(Vec<Arc<str>>)
      | List(Vec<Val>) | Map(BTreeMap<Arc<str>, Val>) | Record(Arc<[Val]>)
      | Tensor(CrowTensor) | Frame(CrowFrame) | Column(CrowColumn) | Json(serde_json::Value)
      | Result(Box<Res>) | Fn(Arc<str>) | Handle(..) | Node(..)
Res ::= { value: Option<Val>, reason, kind, trace, captured: Option<f64> }
```

- A value's status is its `Res`, and only a `Result` has one. `x.ok` of a
  value typed otherwise is `true` by the IR (`Meta::Ok` on a non-result
  type prints `true`), and nothing is checked at run time.
- The compiled session's facts are a struct the emitter generates, one
  typed field per fact, behind a small trait the live runtime is written
  against. They are JSON only at the edges: the wire, the store a session
  outlives its connection in, the harness's `a.count`, and a bound
  input's text.
- A builtin is a runtime function and a row naming it, with how each
  argument is passed. The Rust backend prints the call from the row, and
  the interpreter calls the function on its `Val`s.

## 3. Decisions

1. **Where the capture moment lives.** The spec: readers stamp, a field of
   a captured value keeps the capture (`px.last` is as old as `px`), and
   arithmetic does not. Every reader can fail, so its value is a `Result`,
   and the compiled program already keeps the moment in `CrowResult`.
   *Recommendation:* `Res.captured`. A field read of a `Result` value is
   typed `Result` and keeps it, as compiled does now. `captured[x]` of a
   value not typed `Result` is `no_capture`, decided by the lowering.
2. **Markup.** The IR types it (`Ty::Markup`), and the interpreter carries
   a flag on every string. *Recommendation:* a `Markup` variant, so a view
   places a value by what it is, not by a flag beside it.
3. **Scalar lists.** `Ints`, `Floats`, `Bools` and `Strs` beside the
   general `List` make a million-row mask one bitmap, and `sort` and `sum`
   run on plain vectors. They cost a second path in every list operation.
   *Recommendation:* take them. The IR says a list's element type, so the
   lowering picks the variant, and the runtime's generic functions
   (`core::push<T>`) serve both.
4. **The module boundary.** Every module (`math`, `frame`, `db`, `io`,
   `doc`, `chart`, `stats`, `nn`) is called with `&[CrowValue]`.
   *Recommendation:* an adapter at the registry converts both ways while
   the interpreter's core moves to `Val`. Then each module moves in its
   own commit, and the adapter goes with the last one.
5. **Facts as a struct.** The live runtime (`crow_web::live`, 4.5k lines)
   is written against JSON facts: deltas, persistence, bound inputs, the
   harness and version counters. *Recommendation:* a `Facts` trait
   (clone for the working copy, a version per field, `to_json`/`from_json`
   for the edges, `get`/`set` by name for the harness and bound inputs)
   and a generated struct implementing it, with the runtime generic over
   it. Handlers and helpers then read and write the working copy's fields,
   and the write-through is deleted.
6. **Order.** *Recommendation:* the interpreter first (steps 1 to 3
   below), because the benchmark measures it and every test runs on it,
   then the compiled facts (step 4). Each step gates as step 4 of the
   rewrite did: workspace tests, parity, served pages, the book, layout,
   the task board, finch's baseline and harness, and the benchmark, which
   must not slow down and should gain on the two rows above.

## 4. Steps

1. **`Val` in the interpreter's core.** Locals, expressions, statements,
   calls between Crow functions. Builtins and modules through the adapter.
   The largest step: `ir_interp.rs`, `ops.rs`, the harness and the
   interpreter's tests (`interp_tests.rs`, 3.4k lines, mostly unchanged
   sources with new value assertions).
2. **Builtins on `Val`.** Each state-free builtin calls its runtime
   function on `Val`s, and the emitter prints the same function from the
   table, so the per-builtin chain shrinks to what has its own syntax
   (`log`'s formatting, `grad`'s closures, the web and ui constructors).
3. **Modules on `Val`,** one per commit. Then `CrowValue` is deleted.
4. **Facts as a struct** in the compiled session, the write-through
   deleted, and the harness and persistence through JSON at the edges.

A step that changes what a program prints fails parity and is fixed
before it lands, as in stage 4. What `ir-design.md` §5 sketched is this
document's target; this document decides the parts it left open.

## 5. Progress

The interpreter's value moves to the target in place rather than beside
an adapter: `CrowValue` is changed step by step until it is `Val`, so the
builtins and the modules move with it, and every step keeps what every
program prints (parity, the book, the harnesses) and is timed on the
benchmark.

- **2026-09-26, A: a value's status lives apart.** What a value carries
  besides itself (whether it failed, its reason, kind and trace, its
  capture moment, whether it is markup) is one boxed `Status`, absent on
  a plain value; `CrowValue` is 104 bytes where it was 192 (its `Value`
  is 96, held up by the frame, tensor and sparse variants). Reads go
  through accessors (`is_ok`, `reason`, `trace`, `captured`,
  `is_markup`), writes through `status_mut`. Interpreter times on
  `collect`, before and after, full sizes: the collecting loop 121 → 88
  ms, the loop through a function 216 → 184 ms, join/split 34 → 27 ms,
  sort 3.3 → 2.0 ms, the JSON round trip 13 → 8.6 ms.
- **2026-09-26, B: the large variants held apart.** A tensor, a frame,
  a sparse matrix, a record and a test's app session are boxed, and a
  mailer names its host by a shared string: a `Value` is 32 bytes and a
  `CrowValue` 40, from 96 and 104. On `collect`, against A: the
  collecting loop 82 → 67 ms, the loop through a function 187 → 168 ms,
  join/split 26 → 22 ms, the JSON round trip 8.7 → 5 ms. Against the
  start: the collecting loop and sort each nearly twice as fast, the
  JSON round trip over twice.
- **2026-09-26, the walker's scopes hash names cheaply** (FxHash's
  rotate and multiply: their keys are the program's names). The loop
  through a function 181 → 171 ms, the map-building loop 6.4 → 5.8 ms,
  within the parallel loops' noise elsewhere. A frame naming its
  function by a shared string was tried and undone: in a parallel loop
  every call of every thread wrote that string's reference count, and
  the function loop slowed by a sixth.
- **C deferred.** With a value's status held apart (A), markup is a flag
  in a box a plain string does not have, so it costs a plain value
  nothing; a variant of its own would buy the form decision 2 asked for
  (a view places a value by what it is) at the price of every one of
  the 230 places a string is read deciding whether markup is one there.
  Taken up when the builtins move to the table (§4 step 2), which reads
  each of those places anyway.
- **2026-09-26, a compiled body reads only the facts it uses.** Every
  handler, render and derivation body read every fact of the program
  from the session's JSON up front, used or not. Each now reads the
  facts it loads, stores or assigns into (`facts_used`): finch's
  generated Rust has 320 such reads where it had 10,501, and 43 JSON
  decodings of a list, map or record where it had 1,870; the file is
  8,709 lines where it was 18,783. The served pages, the task board's
  and finch's compiled tests and finch's build all pass. The facts as a
  struct (§4 step 4) take the rest of the JSON away.
- Next: the status held only
  where the IR types a value as able to fail (decision 1), the scalar
  lists (decision 3), builtins from the table (§4 step 2), and the
  compiled facts as a struct (§4 step 4).
