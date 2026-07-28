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

## F-2. Freshness — PARTIAL: propagation is architecturally blocked compiled

**Implemented (dormant, in-tree):** `CrowValue.captured: Option<f64>`,
reader stamping (query/read/fetch/recv/input/load/env/shell/ls), and
oldest-input propagation through binops, unary ops, field access, and
builtin calls. `age[]` is NOT in the vocabulary — withdrawn so the two
modes cannot diverge.

**Benchmark (the ruling's gate):** 400k scalar binops in a tight
interpreter loop — 1.42s baseline vs 1.49s with propagation, ~4-5%.
Acceptable in the interpreter.

**The blocker:** compiled mode has no metadata slot on plain scalars —
`let x: f64 = …` by design, for speed. Only fallible values carry
CrowResult. So option-2 propagation (age flows through arithmetic) is
architecturally infeasible compiled without boxing every scalar, which
would forfeit the compiled mode's whole point. Parity is the repo's
hard rule, so age[] cannot ship with interpreter-only propagation.

**The honest paths:** (a) option 1 — `age[]` reads capture on values
that already carry a metadata header (reader results), no arithmetic
propagation, parity-clean in both modes, plus a `crow check` warning
when a reader value flows into a decision without an age guard (the
checker catches statically what the runtime declines to carry);
(b) box scalars compiled (rejected — costs the language its
performance story); (c) age as an explicit second return / wrapper type
(new vocabulary, needs its own round). **Recommend (a).**

Original proposal below.

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

## F-6. Verification story ✅ SHIPPED (core) — `test` blocks

Shipped: the `test` keyword (attached + named forms), params-are-generated
-inputs, bare-boolean truths, assert operand introspection, build/run
stripping, `crow test` verb, `crow check` superset, Layer-0 determinism
contract, parity file 26. Deferred: compiled-mode test execution, struct/
frame input generation, seeds/skip-ratio reporting, the verification-
helper boundary nudge. `app[]` harness SHIPPED (app/click/html + inline
later + interpreter ui rendering). Original design below.

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
- **Layers 2+3 — FINAL: one keyword, `test` (verify deleted — Vijay's
  merge):** after rule's deletion, verify and test had IDENTICAL
  semantics (statements + bare-boolean truths, params = generated,
  stripped from builds) differing only in attachment — and same
  semantics must mean same word. Attached form (follows a fn, unnamed)
  holds that fn's examples and laws; standalone form is named:

      fn fee[notional] …
      test[n]
          fee[100.0] eq 1.0            --- example (ground)
          fee[n] gte 1.0               --- law (generated n)

      test webhook_idempotent[fill]    --- standalone: named, stateful
          …

  Teaching line: "attach tests to a function; name tests that stand
  alone." Bonus deletions: the verify/web-JWT contextual-keyword
  machinery (no verify keyword exists), and the which-block
  classification decision (position decides). Verb story: `crow test`
  runs authored test blocks (universal muscle memory); `crow check` =
  the full audit (contracts + tests + hazards) — test ⊂ check.
  Final user surface: **test, assert.**
- **Delivery (the C loops):** editor = incremental affected-run on save
  (conservative call graph; fn-values → run everything); CI/pre-commit
  = always complete. Stale-green contained by policy, not cleverness.
- assert is the only assertion everywhere, with native operand
  introspection (builtin, no AST rewriting). No matcher DSL, no fixture
  framework, no describe/it, no config.

Open rulings: does user-facing `crow test` exist at all, or does check
subsume it (--parity/--book stay internal); where-block scope (lean:
what callers see, F1-consistent); auto-contract battery default-on;
block-keyword naming: full shortlist surveyed — verify (lean) > check
(strong second: unifies with the verb, "you write checks, crow check
runs them"; but the command checks far more than these blocks, and the
word carries mild runtime ambiguity) > spec (honest noun, slight
jargon) > where (math prose, SQL-clash) > must (punchy modal, novelty
risk); rejected: assert (dual lifetime), ensure/contract (Eiffel
runtime connotation — these strip), expect (framework matcher baggage),
given (means setup), proof (overclaim). Original note: `verify` vs `where` (lean: verify — SQL-WHERE
connotation clash for a data-language audience, self-describing, matches
the brand; contextual keyword so web.verify/JWT stays callable — the
`as`/`view` precedent; assert must NOT be taken for the block: it stays
the single runtime/test-step assertion, or one word would carry both
ship-and-strip semantics); reserve `test` (CORE_RESERVED);
crow check runs all layers
while `crow test` stays internal (--parity/--book) — revisit whether a
user-facing "just my tests" subset verb is wanted post-v1.
Persona-analysis gaps to design: (a) fullstack harness — handlers write
reactive state that test fns cannot read under F1; scenario tests need a
sanctioned way to drive a handler and observe state (headless
app-under-test or a test-scoped state accessor); (b) route testing needs
a request-construction helper; (c) view output testing (tree/html
compare) is the natural future home of user snapshot[].

## F-7. Datetime ✅ SHIPPED — moments are numbers

Shipped exactly as proposed: `now[]` → float, `moment[…, tz =]`,
`calendar[t, tz =]`, `parse_time[text, tz =]`, `fmt[t, time =, tz =]`;
kernel `crow_runtime::moments` (chrono-tz), both modes, parity 27.
UTC default, machine-local time removed entirely. `date`/`elapsed`/
`today` deleted with teaching errors. Spec section + Design Note
"A Moment Is a Number". Audit + proposal kept below.

**What exists:** `now[]` → map of components in MACHINE-LOCAL time +
epoch timestamp; `date[y,m,d]` → map with UTC-midnight timestamp (note:
now-local vs date-UTC is an inconsistency that already off-by-hours any
mixed arithmetic); `elapsed[a,b]` → seconds/minutes/hours/days map;
`time[]` → epoch float; `today[]` → local 'YYYY-MM-DD'; frames parse
datetime columns internally. Usage is tiny (2 examples + 1 parity file
— break cost near zero).

**What cannot be expressed:** any named timezone ("09:30 ET"), parsing a
timestamp string, adding a duration to a moment, weekday, comparing two
date maps directly, formatting a moment. The finch stale-print question
("was this print >10s before 09:30 ET?") is unwritable.

**Proposal — a moment is a number (epoch seconds, float):**
comparisons and arithmetic come free from numbers (`b - a` IS elapsed;
`t lt open - 10` IS the stale check); the type system stays untouched.
Surface: `now[]` → float (BREAKING: drops the map — migration trivial
given usage); `moment[y, m, d, h, min, s, tz = 'America/New_York']` →
float (named args, the tz-aware constructor); `parts[t, tz = …]` →
components + weekday; `parse_time[s, tz = …]` → float or error value;
`fmt` gains `time =` / `tz =` named args for display. Timezones via
chrono-tz (named zones). DELETIONS (vocab rule paid): `elapsed`
(subtraction does it), `date` (moment subsumes), `today` (fmt does it).

**Rulings needed:** (1) moments-are-numbers + the now[] break; (2)
default tz — UTC default vs explicit-always (lean: UTC default, loudly
documented; machine-local is the deploy-time footgun and is removed
entirely); (3) bless the three deletions; (4) fmt named-arg form for
time display. The finch question after: two lines.

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
