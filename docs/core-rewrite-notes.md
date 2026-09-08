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
