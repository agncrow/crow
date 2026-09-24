# Notes toward a smaller core (2026-09-07)

*A record of a design conversation, kept so it can be resumed. Nothing
here is decided; the numbers are measured, the proposals are proposals.
Started from Vijay reading finch's book, `sim/clock.co` and
`sim/nav.co`, and asking why a few behaviours need so many paths, then
widening the question to the whole app and finally to the core of the
language itself: "if you had to rewrite the core with as little
complexity as possible, efficient, bug-proof, and without restricting
what people can build, how would you do it?"*

## 1. What the book showed

The app's book (`crow book`, app-baseline-design.md §8) prints every
handler as its paths. The clock part has 8 facts, 4 derivations, 7
handlers and every handler re-anchors the play-head. The skip handler in
the nav part has four paths for one behaviour, "skip this symbol and hop
to the next in-gate stock". The question was not presentation, since a
fold hides nothing; it was why the logic itself is that large.

Measured over finch's 28 handlers from the baseline artifacts:

| What a path is | Count |
|---|---|
| Paths, total | 79 |
| Paths that are a failed read or query ending in "say the reason, stop" | 37 |
| Paths that are the "start a session first" guard | 7 |
| Handlers whose every path writes two facts for a notice (`status`, `noticed_at`) | 17 of 28 |
| Handlers that re-anchor the clock (`anchor := now[]`, `base := head`) | 15 of 28 |
| Helper calls passing five or more facts as arguments | 15 |
| The time zone repeated in a `fmt` | 10 |

Of the skip handler's four paths, two are not the skip at all, and the
writes on the other two are mostly bookkeeping for other parts.

## 2. What removes each, at the language level

Ordered by paths removed. The first four need one policy, one checker
rule, one runtime value and one refactor; the fifth is a language round.

1. **A failed read says itself.** A part-level policy: a handler that
   stops on an error reports the reason as the app's notice. The
   behaviour stays; the branch goes. 37 paths.
2. **A control carries its guard.** The trio exists only in the session
   view, which exists only when there is a session; the checker's state
   machine (app-baseline-design.md, phase B2) can prove it, so the guard
   inside the handler is a duplicate defence. 7 paths, no syntax.
3. **A query helper is a derivation.** `next_gate[ch, day, head, ids,
   band[0], band[1], skipped]` reads state and writes none: by the
   spec's own rule that is `derive gate_episode`, computed on read, so a
   read only by a click costs one query per click. Needs no language
   change; finch is not yet written the Crow way here. 15 call sites.
4. **A notice is a runtime value.** `notice['…']` held by the runtime
   with its moment, read by `toast`. Removes two facts, the `say` helper,
   and a write from every path of 17 handlers.
5. **The play-head is a value over time.** With integration in the
   language, `head` is an equation over `speed`, `playing` and `now`,
   the runtime keeps the anchors, and a hop writes `head := at`. 15
   handlers lose their re-anchoring; the clock part loses six facts.

The skip logic after 1–4:

```crow
derive gate_episode
    return next_gate[ch, session.day, clock.head, session.dataset_ids, band, session.skipped]

fn next_gate_any[]
    if len[gate_episode] eq 0
        notice['no symbol enters the gate after the play-head']
        return
    session.sym = gate_episode.symbol[0]
    clock.jump[float[gate_episode.at[0]]]
    notice['`session.sym` enters the gate at `clock.head_time`']

fn skip_symbol[]
    session.skipped = session.skipped + [session.sym]
    next_gate_any[]
```

Two paths instead of four, one or two writes each instead of eight.

## 3. The whole app: where complexity lives between parts

Not in any part; in the seams:

- **A concept is re-expressed per tier.** A *session* is a SQLite row,
  facts in the session part, something four parts read, and columns in
  the history query. A *bar* is a ClickHouse row, a frame cell, a chart
  point, an RSI input, a tape line.
- **Writes fan out by hand.** A trade insert reaches the blotter, the
  history table and the P&L through a table-version bump: whole-table,
  in-process, stopping at the database boundary.
- **Three databases, no shared model.** Postgres (universe), ClickHouse
  (bars), SQLite (the book); cross-store questions are assembled in app
  code.
- **Reads flow one way, writes the other**, and the round trip is
  written, not declared.

Lineages that attack each seam, and what each removes:

| Seam | Lineage | Examples |
|---|---|---|
| Client/server glue | one language across tiers | Meteor, Phoenix LiveView, Blazor; Ur/Web, Links, Eliom (Ur/Web checks SQL, HTML, links end to end — the closest relative to what Crow reaches for) |
| Re-expression per tier | one data model across the store | Datomic and Datalog, LINQ, schema-first (Hasura, Prisma) |
| Invalidation glue | a reactive graph spanning the app | Skip (the language built for exactly this), Materialize and differential dataflow, Convex; Salsa, Adapton, Incremental in-process |
| Tier plumbing | effects and capabilities | Unison abilities, Koka, OCaml 5 effects |
| Build and deploy glue | a single medium | Unison (content-addressed code), Dark |
| Time bookkeeping | synchronous dataflow and FRP | Lustre, SCADE, Esterel, Céu; Yampa, Fran |
| Repeated guards and invariants | statecharts | Harel, Stateflow, SCXML, XState, `gen_statem` |
| Behaviours written alone | behavioural programming | Harel's b-threads |
| Invalid transitions | typestate, session types | Rust patterns |
| Multi-step flows | durable flows | Temporal, Céu's `await` |

What does not help: the Elm architecture and Redux gather handlers into
one function; same paths, one table.

Crow already holds three pillars: one language across client and
server, compile-time query typing (`describe`), and a reactive graph
inside the process (`derive`). The whole-app rounds, extending what
exists: `derive` through the database (the largest win), an entity
declared once, effects as capabilities (which also makes rehearsal
trivial).

## 4. The core, if rewritten

Complexity in the implementation and in programs comes from two
sources, and both must be attacked.

**Where it comes from now.** Two implementations of one meaning: the
interpreter walks the AST, the transpiler walks it again with its own
inference and thousands of lines of special cases, and the parity suite
exists to catch the drift. Bookkeeping the language makes programs
carry (§1). Heuristic typing: int widens to float by rule, results unify
by rule, cells are typed by asking a database.

**The kernel: seven concepts, everything else a library.**

1. **Values** are immutable: scalars, text, lists, maps, records,
   frames, tensors. Nothing shared is mutated.
2. **Facts** are the one mutable thing: a versioned store, written only
   by transitions, one transaction per event.
3. **Transitions** are what handlers mean: a function from state and
   event to a new state plus a list of **effects as values**. A handler
   still reads as imperative code; its writes and its `exec`, `fetch`,
   `notice`, `later` are collected and committed by the runtime after it
   returns. Rehearsal becomes free, the baseline exact, replay and undo
   possible, tests offline.
4. **Derivations** as now, memoized and incremental, extended through
   the database by versioned tables and later differential updates.
5. **Views** are pure functions of state to a tree, laid out and
   rendered by one kernel.
6. **Time is an input.** `now` is a fact the runtime advances; a value
   over time is a derivation over it.
7. **Capabilities**: a part declares what it needs (a bars store, a
   book, a clock); the app binds them; a test double is another
   binding; effects are typed by what a part declares.

Errors stay values; the default becomes "a transition meeting an error
stops and reports", with explicit handling opting in. Guards move onto
controls. Notices are a runtime value.

**One meaning, one intermediate form.** The front end lowers to a small
typed IR: canonical, explicit conversions, every type decided once. Dev
mode interprets the IR; production compiles the same IR to Rust. Parity
becomes a property of the design. Types inferred locally, declared at
part boundaries where inference cannot be sure, insertable by the tool.
Statement-level automatic parallelism goes; parallelism stays inside
frames, tensors and pure loops. Compilation per part, since parts are
already the unit of everything else.

**What stays:** the syntax; frames and tensors; the layout engine and
the kernel look; parts and their tests; the baseline and the book; raw
SQL, the stylesheet hatch, shell and HTTP as escape hatches, now visible
as effects.

**What it costs:** the interpreter and the transpiler replaced by a
front end, an IR and two small backends; the kernel crates survive.
Weeks. The transpiler's special cases encode lessons the IR must
reproduce by design; the acceptance suite exists for that: the parity
corpus, the book's 130 blocks, the layout fixtures, finch's baseline.

**What it buys:** about half the compiler, the parity suite retired,
rehearsal and the app's book falling out of the effect model, finch's
79 paths roughly halved with nothing lost.

**First move, if pursued:** not code. The IR written into the spec with
the clock and the skip handler as worked cases, and a decision on three
semantic changes: effects as values, errors that report, time as an
input.

## 5. Immutability and performance (Vijay's question)

Immutable values do not mean copying. Three mechanisms keep them fast:

- **Structural sharing.** A frame with one more column shares every
  other column; `CrowFrame` already holds columns in `Arc`. Persistent
  lists and maps share their unchanged parts (Clojure, Scala, Haskell).
- **In-place when uniquely owned.** A value nobody else holds can be
  mutated behind the immutable semantics: Rust's ownership does this by
  construction, Koka's Perceus and Roc do it by reference count. A
  `for` that collects, or an append to a list only this transition
  holds, becomes a push, not a copy. Today's `concat[out, [x]]` in a
  loop copies; the rewrite makes it a push.
- **Heavy work in the kernel.** Tensors, frames, SQL, layout and
  rendering are Rust code that mutates freely inside one operation; the
  language-level immutability is about *sharing between* operations.

What immutability buys back: no defensive copies, no locks, safe
parallel reads (shared derivations across sessions are safe because
values never change under a reader), and transactions that are a small
set of changed facts, not a snapshot. Erlang, OCaml at Jane Street,
Clojure and Rust all run demanding systems on immutable-by-default
values. The one habit to design for is the accumulate-in-a-loop
pattern, which is why `for` expressions and unique-ownership pushes are
part of the proposal.

## 6. Open threads

- Which of the three semantic changes to take first; how much of the
  gain is available without the rewrite (rounds 1–4 of §2 are).
- Whether `derive` through the database is a runtime feature or needs
  differential dataflow underneath.
- The IR's shape: how small it can be while typing frames and tensors.
- Per-part compilation: crates per part, or one program with cached
  parts.

## 7. Each concept: what it is today, why that is not enough

Written 2026-09-07 in answer to "go over each of the concepts, and why
you want to do it, why current is not enough". Four of the seven exist
already and would be made real rather than invented; three are new; the
eighth item is about the implementation rather than the language.

**1. Values.** *Today:* value semantics already. `push[xs, x]` returns
a new list and leaves `xs` alone (interpreter, `"push"` arm: the list
is cloned, then pushed). Frames share columns through `Arc`. *Not
enough:* the semantics are right but the representation does not commit
to them, so the cost is paid where it should not be: every push and
concat copies the whole list, a loop that collects is quadratic, and
the compiled program has its own representation with its own copying.
*Change:* persistent lists and maps (structural sharing), in-place
mutation when a value is uniquely owned, frames and tensors as
kernel-owned buffers. Nothing a program can observe changes; the cost
curve does.

**2. Facts as the one versioned store.** *Today:* facts are top-level
variables; the validator allows writes only in handlers and the helpers
they call. In the interpreter a write mutates the module variable at
once; in the compiled program facts are locals written through to the
state before and read back after each helper call (`flush_before`,
`reload_after`), machinery that exists only to imitate the
interpreter's immediacy. There is no store object, no version, no
transaction. *Not enough:* a handler that stops halfway has applied
half its writes; a derivation learns of a write through hooks at each
write site rather than from a version; "what changed in this event" is
not a value, so rehearsal, undo, replay and the runtime counterpart of
the baseline have nothing to read. *Change:* one store, one version per
event, writes applied at commit. Derivations compare versions; the
event log is the changed facts.

**3. Transitions: handlers return state and effects.** *Today:* `exec`,
`insert`, `fetch`, `later` and the notice writes run inline as the
handler executes. *Not enough:* finch's own tests show it: a test
clicks `start_session`, which inserts a row in the real SQLite book,
and then clicks `end_session` to tidy up; the session-id collision came
from a row inserted per click; `crow test` cannot honour the rule that
verification writes nothing; the baseline can only describe effects
statically; rehearsal (B3) needs a stub for every effect. *Change:* a
handler runs as a transaction. Reads are answered as they are asked and
recorded. Writes to facts are held until commit. Writes to stores go
through the store's capability, which is a real transaction in
production (so `RETURNING id` still answers mid-handler) and a double
under test. Outward effects that cannot be undone (mail, shell, HTTP
writes, `later`) are deferred to commit. A handler that stops commits
nothing and reports. The handler's text does not change; what changes
is when the world moves.

**4. Derivations.** *Today:* the strongest existing concept:
memoized, read-set-driven, recomputed on read once per render, shared
or per-session by inference, `fresh = N` for sources the app does not
write, errors held as values. *Not enough:* recompute is whole
(a derivation over a table reruns its query when any row changes);
table versions are app-wide and per table, not per row; a write from
another process is invisible except through `fresh` polling; a
derivation cannot be read inside a plain function, which forced
finch's `band` into a derive read only from views and handlers.
*Change:* keep the design; put it on the versioned store (2); extend it
later through the database with versioned tables and, if needed,
differential updates. This is the largest whole-app win and it needs
(2) and (7) underneath.

**5. Views.** *Today:* pure functions of state to a tree, enforced by
the validator; one layout kernel in Crow; the harness measures boxes
without a browser. *Not enough:* the tree is built twice (interpreter
and compiled program) and, for layout, rendered to HTML and parsed back
into boxes (`layout_html`) rather than passed as a value. *Change:*
a view produces one tree value consumed by layout, HTML, the harness and
the book alike. The closest to done of the seven.

**6. Time as an input.** *Today:* `now[]` is a builtin that reads the
system clock wherever it is called (`moments::now`); nothing can set it.
Derivations already half-treat it as an input (a derivation reading
`now[]` is dirty on each refresh tick), handlers do not. *Not enough:*
finch calls `now[]` fourteen times in handlers and helpers; the clock
part keeps `anchor` and `base` by hand in fifteen handlers because the
play-head is a function of wall time the language cannot express; ids
made from time collided; tests cannot freeze or advance time; the
baseline names each call as an opaque value. *Change:* `now` is a fact
the runtime advances once per event and per tick; every read in one
event sees one moment; tests and rehearsal set it; a value over time
(`head`) is a derivation the runtime integrates, and a hop writes it.

**7. Capabilities.** *Today:* a part connects itself:
`db.connect['clickhouse://localhost:9000/finch_trading']` appears in
four finch parts, the book in two, Postgres in one; the crowfile's `db`
line is a fourth place. The compiler learns schemas by asking the
database (`describe`), which is right, but binding and schema are
mixed into the part. *Not enough:* a part cannot be tested or
rehearsed against anything but its literal source; parts validated
alone still carry a connection string; a doubled store (B3) has no
seam to plug into; the checker cannot prove a part touches only what
it declares. *Change:* a part declares what it needs (a bars store, a
book, a clock, mail); the app binds them once; a test binds doubles
with declared data; effects (3) are typed by the capability they go
through. One declaration line per part is the only new syntax.

**8. The implementation: one meaning, one form.** *Today:* the
interpreter (8.6k lines) and the transpiler (8.4k lines, plus 2.8k of
inference) each give the language its meaning; the parity corpus (39
programs) catches drift. In the last finch step alone the compiled path
diverged six times: int facts not widened to float, `return` across
result types, a fallible condition in a handler, a consumed result in a
handler, a helper not seeing the caller's writes, a missing cast
parenthesis. *Not enough:* every feature is written twice and can be
right once; the parity suite is a tax that never ends. *Change:* one
front end lowering to a small typed IR (canonical, explicit conversions,
types decided once); an IR interpreter for `crow run -i`; an IR to Rust
backend for `crow run`. Parity becomes a property of the design, not
a test suite.

**What Vijay may push back on.** Deferring outward effects to commit
changes what a handler can do halfway through (it can read, it cannot
send mail and then branch on the reply). Time as an input means `now[]`
inside a long handler is the event's moment, not the instant. A
capability line per part is added ceremony, one line. These are the
three semantic changes named in §4, restated as costs.

## 8. Data, ML, performance: what the kernel moves and what it does not

Written 2026-09-07. Vijay asked how the kernel (§4, §7) helps the low
rows of rating-2026-09.md (data 6, math and ML 5, performance 6) and
whether the design should change because of them.

**The straight answer.** The kernel was designed for correctness and
complexity. On its own it moves data by nothing, ML by nothing,
performance by about one point. Three of its pieces are the foundations
those rows need (values with unique ownership, one IR, derivations as
the lazy boundary), but the rows move only if the rewrite is scoped to
carry a compute sublanguage, and only alongside work no kernel replaces:
measurements and library breadth.

**What is true today, checked.**

| Fact | Where |
|---|---|
| Frame runtime, columnar, columns shared by `Arc` | `crow_runtime/src/frame.rs`, 2.0k lines |
| Frame functions in the spec | 17 (Polars: hundreds) |
| Tensor: strided view over `Arc` storage; faer for linear algebra; wgpu for GPU; functional `grad[f]` | `tensor.rs` 2.2k, `linalg.rs`, `gpu.rs` |
| Neural-net module | `nn.rs`, 263 lines |
| Math, stats, signal, sparse, ta, distributed functions in the spec | 141 |
| Benchmark suite in the repo | none |
| Performance table in `math-library.md` | an estimate ("~0.5ms"), never measured |
| Benchmark in `frame-module.md` (csv 31.6s vs Polars 0.18s) | predates columnar storage; stale |
| Spec §14 "parallel by default": statement-level waves + parallel `for` | the waves are what §4 drops |

**Performance (6; reachable 8).** The row says "unproven under load"
and no design can prove anything: a benchmark suite is a prerequisite,
measured in both modes and tracked like the parity corpus. Candidates:
the 1 GB CSV, a 1000×1000 matmul, a training loop of the nn module,
finch under twenty concurrent sessions, a derivation over a growing
bars table. What the kernel then gives: (a) uniqueness, so loops that
collect stop copying (the largest single win, in both modes); (b) one
IR, so there is one optimizer: fusion of element-wise tensor chains,
loop fusion, hoisting of invariant reads, and a place to decide
parallelism from data size instead of statement position; (c) the
transaction, so an event's store writes commit as one batch; (d)
incremental derivations, so a changed row does not rerun a query.
Dropping statement waves costs nothing measurable that a parallel
`for`, parallel frame and tensor kernels, and fetches batched at
commit do not give back, and it removes a source of nondeterminism
that the parity suite has to police. That is a spec change and needs
a Design Note.

**Data (6; reachable 8).** The gap is breadth and lazy execution.
Polars' edge is a lazy plan fused into one pass over columns. Crow
already has the lazy boundary: `derive`. A derived frame is a plan
executed on read; with a relational sublanguage in the IR, `filter`
then `select` then `group` fuses into one pass, and Crow gets Polars'
lazy API without a second API. Incremental derivations over appends
(a window over a growing table) are something Polars does not do.
Frame schemas as static types at part boundaries extend what typed
query cells already do and let the Rust backend emit typed columns.
Breadth is a library workstream (the frame DSL, first in the July fix
plan); the rewrite helps it in one decisive way: every frame function
is written once against the IR instead of twice against two walkers.

**Math and ML (5; reachable 7, never 10).** PyTorch's ten is breadth
and ecosystem: pretrained models, CUDA kernels, loaders, distributed
training. Crow should not aim there. The kernel's help: immutable
tensors with unique-ownership updates fit functional autograd; the IR
fuses element-wise chains (the XLA win); capabilities bind compute
(threads, GPU, a remote worker) without touching the program; time as
input plus derivations make streaming features natural. Three
decisions: (1) shapes in the IR type, at least rank and literal dims,
so fusion and checking have something to work with; (2) rescope the
row to "ML on the data the app already has", tabular and time-series:
linear and logistic models, trees, small nets, TA features, rated
against scikit-learn and statsmodels rather than PyTorch; (3) one
interop seam, ONNX inference through a capability, so models trained
elsewhere run inside Crow. That is the largest gain per line and it is
a capability, so it fits the kernel as designed.

**Design changes to §4, four.**

1. The IR is not scalar-only: it carries a relational sublanguage
   (filter, select, group, join, window, sort over typed columns) and
   an array sublanguage (shaped tensor ops), so it can fuse and check.
2. `derive` is the lazy-plan boundary: a derived frame or tensor
   compiles to a fused plan run on read, incremental over appends.
3. Capabilities cover compute devices and model sources, not only
   stores, mail and the clock.
4. Parallelism is explicit and provable: parallel `for`, internal
   parallel kernels, effects batched at commit; statement waves go,
   with a Design Note; a benchmark suite joins the acceptance suite.

**What does not move by design.** The ecosystem row. Breadth of frame
and ML functions, which is library work whichever core exists. The
measured numbers, until they are measured.

**Order.** Benchmarks first, on today's runtime, so the rewrite has a
before. Then the IR with the two sublanguages and uniqueness. Then the
frame DSL on the IR. Then shapes, the ML rescope and the ONNX seam.

## 9. Recommendation (2026-09-07)

Asked for in plain words. **Rebuild the core, in stages, and decide
three meanings first.** Not a big bang: each stage keeps every suite
green and finch running, and the old path stays until the new one
passes.

**Why.** Every feature is built twice (122 builtin arms in the
interpreter, a separate typed emission per builtin in the transpiler,
two walkers), and every test runs against real databases. Both get
worse with each feature added. Everything wanted next, faster frames,
more data functions, tests that write nothing, less code in handlers,
is cheaper after the fix and dearer before it.

**Stages.**

1. Decide the three meanings in the spec, with the clock and the skip
   handler worked: effects commit when a handler ends; an error stops
   the handler and reports; time is one moment per event. Days.
2. Benchmarks on today's code, both modes, tracked like the parity
   corpus: the 1 GB CSV, a large matmul, a training loop, finch under
   twenty sessions, a derivation over a growing table. A week. Nothing
   is "proven" without numbers, and the rewrite needs a before.
3. One runtime for builtins: move the builtins into `crow_runtime` as
   functions both the interpreter and generated code call, one at a
   time, parity green at each step. Weeks. Removes the largest
   duplication before any IR exists.
4. The IR in the middle, in three moves: parser to IR; the interpreter
   walks the IR and the AST walker retires; the Rust backend emits from
   the IR and the transpiler retires. Uniqueness, the relational and
   array sublanguages, and the transaction ride on this. Weeks each.
5. Then the frame DSL written once on the IR; then tensor shapes, the
   ML rescope, the ONNX seam.

**Alongside, not waiting:** finch's query helpers become derivations
(no language change); the checker proves control guards (B2).

**Not to do:** write the frame DSL on the old core (twice, then
ported); chase PyTorch's breadth; add language features during stage
4 beyond bug fixes.

**Risks.** A stalled rewrite: each stage ships alone. A slower IR
interpreter: the benchmarks catch it. Meaning changes breaking finch:
the baseline names exactly what changed.

**What it buys, by the rating:** verification 6 to 9, tooling 6 to 8,
performance 6 to 8, data 6 to 8 with the DSL, ML 5 to 7 with the
rescope and ONNX; every handler smaller. Ecosystem unchanged; that is
people and time. The whole programme is months of steady work, not
weeks; the core stages are weeks each.

## 10. Progress log

- **2026-09-07, stage 1 done.** `core-design.md`: the three meanings in
  spec-ready wording, worked on the clock and the skip handler, three
  Design Notes, the spec sections each changes.
- **2026-09-07, stage 2 built.** `crow test --bench [--quick] [--write]`
  runs `tests/bench/{collect,frames,tensors,train}.co` in both modes
  (compiled as a release build), compares every non-timing line as the
  parity corpus does, prints the sections side by side and records
  `docs/benchmarks.md` with the machine and the commit; CI runs the
  quick sizes. First findings are in that file's Notes. Two bugs the
  programs found on their first run were fixed with tests: `*` between
  1-D tensors panicked (`try_matmul`), and `len[]` of a parsed JSON
  value did not compile (`json::len`, parity 40). A third finding fixed
  on the way: the extension rename had left `.crow/` being created as
  `.co` and the legacy checks testing the new extension.
- **2026-09-07, stage 3 batch 1 built.** `crow_runtime::core` holds
  the collection builtins once; both modes call it, and `x = push[x, v]`
  moves the collection in (`core-runtime-builtins.md`, log). The
  interpreter's parallel `for` was cloning the whole interpreter per
  item; scopes are now reference-counted and the clone is per chunk.
- **2026-09-07, `rand` on the seeded RNG** (`core-runtime-builtins.md`,
  log). Found on the way: statement waves make a seeded program's draw
  order depend on scheduling; parity 42 draws in one expression.
- **2026-09-08, statement waves dropped** (Vijay's decision after the
  measurement: five results in thirty runs of a seeded three-draw
  script). Statements run in the order written in both modes; the wave
  grouping, the interpreter's parallel top level and the transpiler's
  `rayon::join` emitter are gone; parallel `for`, the kernels and the
  search builtins stay. Spec §14 rewritten with a Design Note recording
  the reversal; parity 43 pins order; parity 42 is three plain
  statements again.
- **2026-09-08, stage 3 batch 2 (text) built:** `join`, `shell`,
  `input` in the runtime, compiled `str` of collections fixed
  (`core-runtime-builtins.md`, log; parity 44).
- **2026-09-08, stage 3 batch 3 (files) built:** the eight file
  builtins in `crow_runtime::io`, one meaning and one reason text
  (`core-runtime-builtins.md`, log; parity 45).
- **2026-09-08, stage 3 batches 4 and 5 (JSON, regex) built:** one
  encoder (`json::ToJson`) and one regex module (`text`);
  `find[text, pattern]` redefined as the groups or an error value
  (`core-runtime-builtins.md`, log; parity 46, 47). `min`/`max` were
  already one meaning.
- **2026-09-08, stage 3's builtin batches complete.** Every row of the
  audit is `runtime`, `keep` or `n/a`. Three residuals are recorded in
  the tracker and wait for the IR's typed values: the compiled
  `matches`/`sub` halt on an invalid pattern where the interpreter
  answers an error value (needs `Result` typing); `.ok` on a list is
  per element in the interpreter and not in the compiled program
  (needs typed columns); the interpreter's frame `filter` and column
  `sum` build a full value per row (needs typed columns and a compact
  boolean list).
- **2026-09-08, stage 4 opened.** Vijay took the four recommendations:
  status only where a value can fail; the IR interpreter first, behind
  a flag; the three meanings one step after the IR; capabilities and
  typed frames as the second half. `ir-design.md` is the record, for
  review before code: types, declarations, statements, expressions, the
  runtime table, the IR interpreter's values, the Rust backend, the
  session layer, the gates and order, the clock and skip handlers in
  the form.
- **2026-09-08, stage 4 steps 1 and 2 built:** `ir.rs` (the IR and its
  printer) and `lower_ir.rs` (the lowering on the existing front end:
  facts, derivations with read sets, functions classified pure, helper,
  handler or view with their reachable effects, effects marked
  answered or held, propagation explicit, int-to-float explicit,
  parallel loops decided, last reads as moves). `crow build --ir
  file.co` prints it; every parity program lowers, checked by a test.
- **2026-09-08, stage 4 step 3 started:** the state-free builtins and
  the operator, indexing and field semantics were moved out of the
  interpreter into `builtins.rs` and `ops.rs` first, so the IR
  interpreter (`ir_interp.rs`, behind `CROW_IR=1`) reimplements nothing
  it can share. Its first tally (47 of 47) was an artefact of the runner
  compiling instead of interpreting; the real first tally was 35 of 47,
  and after the day's fixes it runs all 47 parity programs and 115 of
  130 book blocks, the rest being views and the live layer. `crow test
  --parity --ir` and `--book --ir` are the advisory tallies until it is
  the default.
- **2026-09-08, the live layer in the IR interpreter:** views, bound
  inputs, the harness verbs, the mocks, `icon`, `progress`, the layout
  engine (lowered through the same pipeline) and test blocks
  (`CROW_IR=1 crow test file.co`, the block lowered as a `Test` with
  its inputs bound at run time). Tallies: 48 of 48 parity, 130 of 130
  book blocks with their test blocks, layout conformance 6 of 6. Three
  older bugs fixed with tests on the way: inference inferred call
  arguments twice (a 25-entry map literal hung `crow build`), the
  compiler's map index by a `keys[]` loop variable, and the
  compile-error translator's `stringing`.
- **2026-09-08, late — step 3 done:** the IR interpreter is `crow run
  -i`; `interpreter.rs` is deleted, its 381 tests ported and green
  through the walker; the checker, REPL, try, crowfile reader and
  wireframe tool run on the walker; parallel loops, copy-on-write
  scopes and in-place builtins keep the benchmark; found and fixed with
  tests: a function's local shadowing a module name was typed as the
  module's variable by the validator, inference and both lowerings
  (parity 49), the removed time builtins' teaching errors moved to the
  validator, resources and web stand-ins and `chart` ported.
- **2026-09-23, step 4 done:** the Rust backend prints the IR
  (`src/emit_rs/`), `crow build` and `crow run` compile through it, and
  `transpiler.rs` is deleted, its 270 unit tests ported. The IR now
  carries what the emitter needs that the walker also reads (derivation
  memo keys, tables written, markup and tensor element types,
  specialization clones), and four lowering bugs the emitter exposed are
  fixed (parity 14, 18, 33, 36), with two older ones found on the way: a
  value moved into `int[]` and five other places could not be read again
  compiled, and a failing `while` condition was tested once (parity 54,
  served 11). Every gate is green on the final binary, and finch built
  both ways behaves the same. The record, with the one
  decision still made twice (loop parallelism) and an older parity gap
  (`len` of an error), is under step 4 in `ir-design.md` §9.
- **2026-09-23, step 4's two open items closed.** Loop parallelism is
  the IR's decision alone, by Vijay's rule "no side effects, then
  parallel" (spec §14, For Loops): the lowering decides once every
  function's reach is known, and both backends only read the flag. The
  interpreter runs a loop over a pure function 3.4 times faster, and a
  seeded loop of `rand[]` draws the same numbers on every run again.
  `len` of an error is the error in both modes, and `and`/`or` evaluate
  both sides in both modes, Vijay's choice of the spec as written over
  short-circuiting (Design Note "`and` and `or` Evaluate Both Sides").
  A `for` over a list that failed is refused like a condition, his choice
  over running it zero times, and controls render a failed argument as
  its reason.
  Found on the way and fixed with tests: a collecting loop could not
  hand back an error from its condition (parity 55), and `push[]` and
  `set[]` took a variable by move (parity 56). The record is under step
  4 in `ir-design.md` §9.
- **2026-09-24, a guard narrows.** After `if not x.ok` whose branch
  leaves, `x` is its value for the rest of the block (spec, Errors in
  Conditions), so guarded code is not typed as able to fail: finch's
  `symbols` is a list again, not a failing list. Done once, before
  inference, as a rewrite in `lower.rs`: a narrowed read becomes the
  hidden `__known[x]`, which inference types as the value and the lowering
  makes `Convert::Known`. That node is new in the IR: known good, so nothing
  propagates from it, unlike `Unwrap`. Every pass reads the same type, where
  threading the narrowing through each of inference's walks would have let
  them drift. The fold below replaces the rewrite with the scoped
  environment's own narrowing.
- **2026-09-24, the fold's first step: names resolved with their blocks.**
  The type checker kept one type per variable name for a whole function.
  So two blocks setting `x` to an int and to text were refused as a type
  change, although the spec makes them different variables, and a read
  after the block that set a name failed at run time in the interpreter
  and at Rust's compile in the other mode. `lower.rs` now resolves names
  once, block by block: a later variable of an ended name is renamed apart
  (`x__s2`, shown as `x` in every message), and a read after its block is
  refused before the program runs, saying what to write instead.
- **2026-09-24, the fold's second step: the lowering types bottom-up.**
  The type checker's rule for each kind of node is now a function of its
  children's types (`binop_type`, `named_call_type`, `field_type`, …), and
  the lowering types each IR node from its lowered children with those
  rules. It used to ask the type checker to walk the whole subtree again
  at every node. The check that nothing moved: the IR and the emitted Rust
  of every tracked program, every doc block and finch (640 files) are
  byte for byte what they were, as are 3,492 mutants of them. A per-node
  comparison found one node typed differently, a record built from a
  value that can fail (`[LSlot[kw, li]]` in the layout module): it is the
  record the list holds, where the walk called it a failed record.
- **Next:** the fold's later steps. Validation moves into the lowering,
  and then named arguments, deleting `validator.rs` and `lower.rs`. Last,
  the type checker's whole-program analyses (specialisation, frame
  schemas, signatures) become the lowering's pre-pass, deleting
  `type_infer.rs`.
