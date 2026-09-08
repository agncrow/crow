# The app baseline — design round (2026-09-07)

*What a program does, derived from its source in a canonical form, kept
beside it, and diffed on every `crow check`. Refactors leave it
unchanged; a change in behaviour or composition shows as a diff with its
cause and its reach. Nobody writes a rule, and nobody forgets one.*

## 1. The question

The simulation page was rebuilt as parts, and the question became how to
know that a change to one part did not break another — without a manual
walk through the app, without writing a test per control, and without
running anything against a database or a service. Vijay's framing, which
this design follows:

- *Understanding.* A reader should see the whole app — its facts,
  derivations, handlers, views, pages and parts, and how the UI is
  composed — without reading every file.
- *Like unit tests, but derived.* "If we change something, we should be
  able to tell if something else is broken", the way git tells you what
  text changed. Not a text diff: a diff of what the app does.
- *No rules.* Laws that someone has to write and remember are the thing
  to avoid. Intent lives in an accepted baseline, and `crow check` shows
  what moved so a person can decide.
- *Nothing written, nothing called.* A check is about the UI and its
  functionality, not integration: no database write, no external call,
  ever.

## 2. Why it is possible in Crow and not elsewhere

Three things are explicit in the model and implicit everywhere else:
the only writers of facts are handlers and the only readers are views
and derivations; every effect goes through a handle (`exec`, `insert`,
`fetch`, mail, shell); and every control is declared in a view with the
handler it fires. The checker can therefore compute, from source alone,
what every control does in every state, and compare two versions of that
computation. The prior art and the rating are in `rating-2026-09.md`;
the short version is that types (Elm, Rust) give this for signatures,
approval testing (insta, Jest) gives the accept workflow, hardware
equivalence checking gives the technique, and no application language
has the model that lets all three meet.

## 3. The artifacts

`crow check` derives these from the expanded, lowered program and writes
them under `.crow/baseline/<entry>/`, one file per declaration, tracked
like the snapshot store:

| Artifact | One per | What it holds |
|---|---|---|
| `fact` | top-level variable | its type; who writes it; which inputs bind it; who reads it |
| `derive` | derivation | what it reads (facts, derivations, tables, the clock); who holds it; who reads it |
| `handler` | event handler (and the helpers it inlines) | its **signature**: every path through it as a condition over facts and inputs, the facts it writes and from what, the effects it would perform with their payloads, what it may do inside loops; where it is wired; what it reaches |
| `view` | view | its **conditional tree**: elements, attributes, guards, sub-views, bound inputs and controls, per path through the view; which pages compose it |
| `page` | `page[...]` | its path and its view |

Everything is in a **canonical form**: locals are substituted by their
definitions, parameters are numbered, guards are printed as normalised
expressions over facts and inputs, writes are `fact := value` sorted by
fact, paths are sorted by their conditions. Two programs that do the same
thing produce the same text. Renaming a local, introducing a temporary,
reordering independent statements, moving a handler between parts,
splitting a function: no change. Flipping `lte` to `lt` in a guard,
writing one fact more or less, wiring a button to a different handler,
adding an element under a guard: a change.

### 3.1 The handler signature

Computed by walking the body symbolically. An `if` forks the path with
the guard on one side and its negation on the other; an early `return`
ends a path; a call into a helper that writes facts is inlined with its
arguments substituted (depth-limited); a loop's writes and effects are
recorded as *may*, since the checker does not count iterations. A query
in a handler is a read whose result is a symbol, so a later `r.id[0]`
prints as the query's cell.

```text
handler sim_session_start_session (sim/session.crow)
  wired: sim_session_scanner_setup button 'Start' · sim_session_practice_setup button 'Start'
  reaches: views_sim_render · page /
  when not at_time[day, win_from].ok:
    noticed_at := now[] · status := 'bad date or time: `day` `win_from`'
  when at_time[day, win_to] lte at_time[day, win_from]:
    noticed_at := now[] · status := 'the window ends before it starts'
  when not query[book, 'INSERT INTO sessions (...) RETURNING id', [...]].ok:
    session_id := 0 · starting := false · status := 'could not record the session: ...'
    effect query book INSERT INTO sessions (trade_date, mode, ...) RETURNING id [day, mode_code[mode], ...]
  ...
```

A signature is the thing a functionality change moves. "Start now
accepts an empty window", "Buy is enabled with no session", "End session
no longer stops the clock" are each a path whose condition, writes or
effects differ from the accepted text.

### 3.2 The conditional tree

A view's return expression, canonicalised: elements with their
attributes in name order, text literals as they are, dynamic text as its
canonical expression, sub-views as references, controls as `event →
handler[args]`, bound inputs as `binds fact`. Guards fork the tree the
same way they fork a signature, so "the modal shows only under `ending`"
is readable from the artifact, and a control that moves under a new
guard is a change.

### 3.3 Reach

Each changed artifact carries where it reaches, computed from the
wiring: a handler reaches the views that wire it and the views that read
what it writes, and through them the pages; a view reaches the pages
that compose it; a derivation reaches its readers. Reach is what turns
"something changed" into "something else may be broken".

## 4. The check

1. Derive the artifacts.
2. If no baseline exists for the entry, record it and say so. This is
   the first commit.
3. Otherwise diff, per declaration. Each difference is a finding with:
   - **what**: the declaration and the changed lines, was and now;
   - **cause**: the file the declaration lives in;
   - **reach**: the views and pages it reaches;
   - **expected or not**: whether the cause is among the files git says
     changed since the last commit. Differences inside the files you
     edited are shown as consequences of your edit; differences reached
     elsewhere are the headline.
4. Accept with `a` in the check loop, as function snapshots are accepted
   today; the baseline moves and travels with the change in review.
   Without a terminal the check reports and exits non-zero, so CI blocks
   on an unaccepted change. New declarations are recorded; removed ones
   are reported.

## 5. Kernel rules

A baseline cannot say that a modal should be closable. A few rules are
the compiler's, generic, and nobody's to maintain:

- an `open = fact` on a modal, drawer or toast with no handler setting
  it true, or none setting it false;
- a `disabled = fact` that no handler ever writes;
- a control whose handler writes nothing, performs nothing and calls
  nothing that does;
- a function that assigns a fact's name but is wired by nothing and
  called by nothing — a handler nobody can reach.

They are findings, not errors, like the live-loop hazards.

## 6. What stays out, by decision

- **Laws over facts** ("playing implies a session"). Rejected as rules to
  maintain; a change that makes such a state reachable shows in the
  signature diff and is judged there.
- **Database writes and external calls in a check.** Never. Where a
  branch depends on data — a row exists, a gate is true — the signature
  says *may*, and that remainder is covered later by rehearsal with
  declared data (§8), still with no connection.
- **Pixels.** The engine's boxes and the kernel's tokens are the render;
  the browser gap is bounded by the measured fixtures.

## 7. Phases

| Phase | Ships | Status |
|---|---|---|
| B1 | Artifacts for facts, derivations, handlers (signatures), views (conditional trees), pages; the baseline store; the diff with cause, reach and the git split; accept in the check loop; the four kernel rules; finch as the acceptance case | this round |
| B2 | The reachable state machine over abstract facts (small domains: booleans, options, zero-or-a-session), so "Buy is enabled with no session" is a state, not only a path; layout bounds per view at each width from the engine | next |
| B3 | Rehearsal: reads answered from declared data generated from each query's described schema or from a recorded fixture; writes and calls recorded as effects; `now[]` frozen; the transition snapshots per scenario | after B2 |
| B4 | The app's book in `crow book`: overview, pages with trees and wireframes, a chapter per part, data, changes (§8) | this round, ahead of B2 and B3 at Vijay's ask |

## 8. The app's book (B4) — built in this round, ahead of B2 and B3

Vijay's ask: one command, every detail organised step by step, easy to
view. Not SVG files a developer has to find. So `crow book`, run inside
an app, serves the app's own book, derived from the source the way the
Crow Book is rendered — by Crow's `doc` module, with the same page
shell — and nothing runs against a database.

**Chapters, in reading order.**

| Chapter | What it holds |
|---|---|
| The app | The crowfile's name, the counts, and every part with the description its file opens with (the leading comment) and what it holds |
| Pages | For each page: its composition tree — views under their guards, the inputs they bind, the controls and the handlers they fire — and its wireframe at 1280 and 390, one per state of the page |
| One chapter per part | Its facts (type, bound by, written by, read by), derivations (reads, tables, holder), handlers (wired or called by; every path with its writes and effects), views (composed on; the tree; the wireframe of every path at both widths), and its named tests as steps |
| Data | Facts across the app with writers and readers; derivations and who holds them; queries and tables by handle; the effects: which handler writes which table |
| Changes | The diff against the accepted baseline as `crow check` reports it, with cause and reach; or "the baseline is current" |

**Wireframes are static.** A view's conditional tree is laid out by the
kernel's own layout engine, the one the harness uses, with placeholders
where the tree has values: dynamic text prints as the value's short name
(‹status›), a table draws its header from the query's described columns
and three rows, a select lists its literal options, a chart is a box of
its height, a sub-view is its default path inlined. Every path of every
view is drawn, so every state is visible, and nothing runs. Geometry is
exact where text is literal and approximate where it is dynamic; when
rehearsal (B3) supplies declared data the same drawing becomes exact.

**Rendering.** `crow book` writes Markdown under `.crow/book/src/`, a
summary, and a `site.crow` with the Crow Book's own page shell (menu,
pager, search, the kernel stylesheet); runs it with the interpreter;
serves `.crow/book/site/` next to the app, on its `serve` port plus one
(`--port` overrides), and records its pid so `crow stop book` stops it.
`crow book --write` builds without serving, for CI. The Markdown and the wireframes are derived on
every start, so the book is never stale; the runner's `▶ Run` stays
available on any code block a part's own documentation carries.

**What it is not.** Not a place to write documentation by hand (a part's
leading comment is the one authored line), not a diagram tool, and not
a replacement for `crow check --show`, which is the same tree in the
terminal.

## 9. Rehearsal (B3), recorded so the shape is agreed

A scenario is the initial state plus the state each named test leaves
behind. In rehearsal a `query` returns a frame for that query — generated
from the schema the compiler cached at build time, or a fixture written
or recorded once on purpose — and an `exec`, `insert`, HTTP call, mail
or shell command becomes a recorded effect with its payload while the
handler sees success. No connection is opened. The check runs offline in
seconds. Behaviour that turns on data needs the data declared; the
static layer names exactly which controls those are.

## 10. Open questions

- Text in views: a renamed label is a visible change and is reported;
  dynamic text is printed as its expression. Whether long literal text
  should be elided in the artifact.
- Loops: *may* is coarse. A bounded unrolling for loops over small literal
  lists would sharpen the signature at little cost.
- Parameters of handlers wired with frozen arguments (`click = set_speed[10.0]`)
  are inputs; the wiring records each frozen value as its own control.
