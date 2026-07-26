# Discovered features — the finch mining queue

Findings from walking a real production trading system's git history (finch,
1,565 commits) and asking, for each fixed bug: *could Crow have made this
unwritable, and what would that take?* Some answers became `crow check`
detectors immediately; the ones below need **language design** first. Each is
written as a reviewable proposal: the evidence, the gap, a sketch, and the
questions a design round must answer. Nothing here is decided.

Per the vocabulary rule, each needs a deletion or an impossibility proof
before it ships.

---

## F-1. Background work — a sanctioned "off the loop" primitive ✅ SHIPPED

Implemented as `later[f]` / `running[f]` — see the language spec's
"Background Work: later" section for final semantics (state-as-mailbox,
skip-if-running, inline interpreter schedule, exit wait). The blocking-in-
view check now teaches `later[…]` and applies it interactively (`f` in the
crow check loop). Deferred to future rounds: auto-offload (the no-verb
endgame), cancel-and-restart, `serve` latency budget. Original proposal
kept below for the record.

**Evidence:** finch `188772df` (event-loop hang: ~700ms model load + sync DB
reads directly on the loop froze the entire scanner) and `8c15d8ef`
(event-loop deadlock). Python's fix was `asyncio.to_thread`.

**The gap:** Crow has no equivalent. `crow check` now *detects* blocking work
in views/handlers, but the only expressible fix is "do it before `serve[]`".
There is no way to say "run this without freezing the interaction path" —
no spawn, no task, no timer. Parallel `for` is data-parallelism inside a
computation, not background work. The checker can point at the bug; the
language cannot yet express the cure.

**Sketch (deliberately loose):** a single verb — working name `later[fn]` —
that schedules a named fn off the live loop; its result arrives by writing
reactive state (the sanctioned channel), so views update when it lands.
Errors follow errors-as-values: the state var holds the error value, views
render `.reason`. No handles, no join, no cancellation in v1.

**Design questions:** result delivery (state-write only, or a value?); can
`later` fns take by-ref state (scheduler implications); interaction with F1
scope isolation; interpreter/transpiler parity (threads vs a task queue);
does this subsume or conflict with `derive … cache[30s]` from the
concurrency design; what gets *deleted* in exchange.

---

## F-2. Freshness — staleness as a first-class property of data

**Evidence:** four independent finch fixes for the same class: `4e90b882`
("stale prints must never drive price"), `792e2e4e` (stale-breakout guard),
`da9020d6` (digest supersedes only 2h+ stale coverage), `53c71e5a`
(crossing-freshness guard). The pattern: data captured at time T drives a
decision at time T+Δ as if Δ were zero. Every fix hand-rolled a timestamp
comparison.

**The gap:** Crow values carry error metadata (`.ok/.reason/.kind/.trace`)
but no notion of *when the value was true*. Staleness checks are ad-hoc
arithmetic the developer must remember, per call site — exactly the shape of
discipline-dependent safety this project exists to remove.

**Sketch:** values produced by external readers (`query`, feeds) carry a
capture time the way all values carry `.ok` — readable as `.age`. A decision
guard idiom (`if px.age gt 5 …`) or a declarative form on the read. The
`crow check` counterpart (see check roadmap below) can ship before the
language feature and inform it.

**Design questions:** metadata channel cost (every value? only reader
outputs?); wall-clock in pure functions (purity analysis interaction);
compiled-mode representation; is `.age` a fifth metadata name (reserved-field
implications — struct fields named `age` are common!); maybe `.at` +
user-side arithmetic instead.

---

## F-3. Language-owned periodicity — polls without overlap ✅ SHIPPED

Implemented as two named arguments on serve: `serve[port, refresh = N,
budget = M]` — runtime-owned re-render interval (missed ticks skipped,
never stacked) and a loop latency budget that logs slow handlers/renders.
No `every[]`, no timers: periodicity is declared, not written (see the
spec's Design Note). Original proposal kept below.

**Evidence:** finch `a082820b` (1s orders poll needed a hand-written
in-flight guard so slow polls don't overlap), plus the digest/TTL-refresh
family (`b3b6b034`). Every hand-rolled poll loop eventually needs the same
three fixes: overlap guard, jitter/backoff, shutdown handling.

**The gap:** Crow has no timers at all today, which has kept this bug class
out — but F-1 (background work) will create the demand, and users will
hand-roll polls with `while + sleep` the moment they can. The concurrency
design round already rejected a bare `every[30s]` in favor of
`derive … cache[30s]`; this evidence says: if periodicity ever ships, the
*runtime* must own the no-overlap/backoff discipline, never the user.

**Design questions:** is `derive … cache[TTL]` sufficient for all observed
uses (finch's poll updates a panel — that's a derive!); if not, what's the
minimal periodic form; overlap semantics (skip vs queue); parity.

---

## F-4. External ground truth — adopting beliefs about systems Crow doesn't own

**Evidence:** finch `75a33fbe` (fill-settlement race: a transiently-flat
position row was adopted as "trade closed" — fix was a 15s grace window) and
`e4765f38` (order-truth: derive closure from order status + fills, not from
the position row — a wrong-source belief). This is the derive-from-ground-
truth doctrine colliding with *distributed* ground truth: the authoritative
fact lives in the broker, observations of it are delayed and transiently
wrong.

**The gap:** no language can make a remote system atomic, but Crow currently
offers nothing to express "this belief needs N seconds of stability before
it's actionable" or "this value's authoritative source is X, not its cache."
Today this is the least-shaped item — possibly doctrine + a Design Note, not
a feature.

**Design questions:** is there any primitive here at all, or is this a
documented pattern (debounced adoption) plus a `crow check` heuristic;
relationship to F-2 (a grace window is freshness inverted: *minimum* age
before acting on an absence).

---

## F-5. Durable jobs — claim semantics for work that must survive a crash

**Evidence:** `later` (F-1) is deliberately in-process and best-effort: a
process death loses in-flight work. Fine for a refresh; unacceptable for
"place this order." The durable version of background work is a persisted
queue with **claim semantics** — the claim/release design from the
concurrency arc (see crow-check.md and the design memory): work items are
rows, a worker `claim`s one atomically (the unique-constraint machinery
makes double-processing unwritable), completes it, releases/marks it; a
crashed worker's claim expires and the item is retried. This is the
composition of `later` with the deferred store/claim design, and it is the
correct home for finch-class order placement.

**Design questions:** claim leases/expiry (crash recovery without
double-fire); at-least-once vs exactly-once honesty; whether the queue is
a `store` with `claim` or its own noun; retry/backoff policy ownership;
how `crow check` distinguishes work that needs durability (order
placement) from work that doesn't (refresh) — likely a teach, not a proof.

**Status: revisit after the store/claim (Level-3/persist) design round —
explicitly queued at Vijay's request.**

---

# Fundamentals audit (July 25) — gaps vs. other languages, not from finch

A deliberate sweep: what do mainstream languages consider table stakes that
Crow neither has nor has documented-rejected? Five items survived the
filter; everything else (generators, channels, generics, closures, match)
is either covered by existing design or already rejected on the record.

## F-6. Verification story — "one verb, layered curriculum" (design settled in discussion, awaiting go)

Not a test framework: an extension of crow check. There is one question
("is my program right?"), one verb asking it (crow check), and user
artifacts are *curriculum* that teach the checker the domain:

- **Layer 0 — auto-contracts, free on every fn:** determinism for pure
  fns (run twice, compare), error-propagation contract (error in →
  error out with trace), both-mode parity on generated + example
  inputs, no-panic on edges. Zero user input; no other language runs a
  contract battery on every function by default.
- **Layer 1 — `where` blocks:** canonical examples attached to the fn.
  Lines are bare booleans (implicit asserts). They are simultaneously
  verified docs (LSP hover/renderer), inference call-sites, and
  generator seeds for layer 2. Editor folds them by default (kills file
  bloat); convention: doc-worthy examples only, overflow → test files;
  transpiler strips them from release builds.
- **Layer 2 — laws, unified into where (prop_ DELETED):** a where line
  with a free variable is a law: `fee[n] gte 1.0` — unbound names are
  universally quantified, typed from their usage position, generated
  minimal-first. Ground lines = examples (checked once); free-var lines
  = laws (checked across generated inputs). The upgrade path from
  example to law is literally "generalize a constant." Typo risk
  (misspelled name silently becomes a quantified var) mitigated by
  did-you-mean against visible fns + type constraints; LSP annotates
  generated vars. Relational two-fn laws attach to either fn.
- **Layer 3 — scenarios (directory/suffix conventions DELETED):** a
  `test_` fn may live in ANY file — discovery is by name prefix alone;
  tests/ and *_test.crow are organizational suggestions, not tool
  requirements. Colocated scenarios get affected-selection trivially.
  A test_ fn WITH parameters is a stateful property: params are
  generated inputs (same rule as where — unbound = generated), enabling
  idempotency/invariant laws that need setup (db, app harness).
  Division of labor: where = truths of one fn; test_ = truths of fns
  working together.
- **Delivery (the C loops):** editor = incremental affected-run on save
  (conservative call graph; fn-values → run everything); CI/pre-commit
  = always complete. Stale-green contained by policy, not cleverness.
- assert is the only assertion everywhere, with native operand
  introspection (builtin, no AST rewriting). No matcher DSL, no fixture
  framework, no describe/it, no config.

Open rulings: does user-facing `crow test` exist at all, or does check
subsume it (--parity/--book stay internal); where-block scope (lean:
what callers see, F1-consistent); auto-contract battery default-on;
implicit quantification (free vars, with did-you-mean guard — lean) vs
an explicit marker word; bless the two deletions (prop_ prefix, required
file conventions).
Persona-analysis gaps to design: (a) fullstack harness — handlers write
reactive state that test fns cannot read under F1; scenario tests need a
sanctioned way to drive a handler and observe state (headless
app-under-test or a test-scoped state accessor); (b) route testing needs
a request-construction helper; (c) view output testing (tree/html
compare) is the natural future home of user snapshot[].

## F-7. Datetime depth — audit required, blocks F-2

`now/date/elapsed` + frame datetime columns exist; timezones, parsing,
durations, market-calendar arithmetic likely don't. The domain is
time-obsessed (stale prints, ET market hours) and `age[x]` (F-2) is time
arithmetic. First step is an audit, then a small principled surface.

## F-8. Constrained values — the stringly-typed status problem

`if status eq 'actve'` compiles and is silently never true. Sketch:
`choice status = ['active', 'closed', 'pending']` — validator rejects
literals outside the set at compare/assign sites. No match, no payloads —
just "the compiler knows the legal values." Needs the vocabulary-rule
audit.

## F-9. Money — decide the decimal question deliberately

All P&L examples are IEEE floats in a language whose dogfood is a trading
system. Options: real decimal type (heavy) / int-cents as blessed idiom +
Design Note + checker nudge (cheap, my lean) / documented float
acceptance. Currently undecided by omission — that's the gap.

## F-10. Interop — formalize or reject the Python bridge

Today's answer is `shell[]` (finch's own sklearn call), now non-blocking
via `later[]`. Options: blessed subprocess-JSON bridge / real FFI /
documented 1.x rejection. Any is defensible; drifting isn't. Related
kernel work, not language design: parquet/arrow IO for frames — data-
science table stakes.

# Check roadmap (no language change needed)

Detectors buildable on current machinery, from the same mining:

| Check | Finch evidence | Status |
|---|---|---|
| Blocking in views/handlers (transitive) | 188772df, 8c15d8ef | ✅ shipped |
| Startup snapshot served forever (query at top level, read in views) | da9020d6, 4e90b882 class | ✅ shipped |
| Remote-DB query inside views (a network hop per render stalls every client) | 188772df root cause #2 | ✅ shipped |
| Poll loop deciding on a pre-loop snapshot (while+sleep reading launch-time query data) | 4e90b882, 53c71e5a class | ✅ shipped |
| Debounce-free adoption of external absence | 75a33fbe | needs F-4 design first |

Further evidence logged during the complete scan: `8748874c` (impossible
cross-source data — float > shares outstanding — silently consumed signals)
is direct evidence for the **declared data invariants** feature from the
persist design (arbitrary-predicate `invariant` checked at commit); and
`b3b6b034` (news poll starving the universe refresh through a shared rate
gate) strengthens F-3's case that the runtime, not the user, must own
periodicity and its resource arbitration.

Classes finch fixed that Crow makes unwritable by existing design: re-entrant
lock deadlock (no locks), signature-mismatch loop crash (static types),
check-then-act entry claims (db_check 1a), consumed stop-latches (1c).
