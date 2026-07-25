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

## F-1. Background work — a sanctioned "off the loop" primitive

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

## F-3. Language-owned periodicity — polls without overlap

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

# Check roadmap (no language change needed)

Detectors buildable on current machinery, from the same mining:

| Check | Finch evidence | Status |
|---|---|---|
| Blocking in views/handlers (transitive) | 188772df, 8c15d8ef | ✅ shipped (sample 1) |
| Startup snapshot served forever (query at top level, read in views) | da9020d6, 4e90b882 class | sample 2 |
| Remote-DB query inside views (latency-aware refinement of the blessed render-time-query idiom) | 188772df root cause #2 | proposed; needs backend-awareness at check time |
| Debounce-free adoption of external absence | 75a33fbe | needs F-4 design first |

Classes finch fixed that Crow makes unwritable by existing design: re-entrant
lock deadlock (no locks), signature-mismatch loop crash (static types),
check-then-act entry claims (db_check 1a), consumed stop-latches (1c).
