# The compiler holds the language; modules are written in it

Vijay, 2026-10-18: "a compiler is language specific and modules are built
using that language." Remove every module-specific line from the compiler,
then freeze it: a change to the compiler is a language feature, an
optimization or a bug fix, asked for before it is made, with a strong
reason. This note is the design: what was found, what the language needs
so that each piece can leave, the phases, and the questions each phase
settles. Design rule 24 states the rule; `src/freeze.rs` holds it
mechanically.

The process (Vijay, the same day: "I want to make sure everything is
properly designed before it's implemented"): phase 1 needs no language
change and begins at once; the code of each later phase waits until its
section here has been read and its open questions answered.

## 1. What the inventory found

Six read-only surveys of the sources at d1a1446, one per area. "Module-
specific" means a line that recognizes one module or one builtin by name,
row id or a type variant that exists for that module's values, to treat it
unlike the rest.

| Area | Module-specific | Arguable |
|---|---|---|
| Front end: `lower.rs` option tables, `derive.rs`, `states.rs`, the view rewrite | ≈1,650 | ≈380 |
| `lower_ir.rs` | ≈1,450 | ≈930 |
| `lower_ir/`: `checks.rs`, `uses.rs`, `frames.rs`, `shapes.rs`, `fuse.rs`, `schema.rs`, `grad.rs` | ≈2,300 (+560 in tests) | ≈1,190 |
| Types, IR, the table's lists, inference, the registry (incl. math's 800 lines of unit tests, the database's query typing) | ≈5,400 | ≈1,300 |
| The emitter and the interpreter | ≈5,900 | ≈3,700 |
| **Compiler** | **≈16,700 (about 30%)** | **≈7,500** |
| Tools in the binary: `crow check`, the baseline, wireframes, `db_check`, the self-check's families, the docs check | ≈10,000 | |

By what it is, across the compiler:

- The reactive UI and web: views, elements, controls, pages, facts,
  derivations, handlers, session effects, routes, responses, serving, the
  test harness, mail, sign-in: ≈8,000.
- The database: typing a query's answer while compiling (opening the
  connection, parsing the SQL), the schema, its verbs as forms of their
  own: ≈2,500.
- Math: shapes by name, fusion, `grad`'s lists, sparse, constants, its
  unit tests: ≈2,000, plus ≈2,200 of tensors as the language's arrays.
- Frames: each verb's columns, column forms, columns typed from use:
  ≈1,900, plus ≈650 arguable.
- Core forms with option tables (`fmt`, `moment`, `decimal`, `request`,
  `chart`, the readers), `llm`'s tool schemas, chart, bits, text: ≈1,100.
- Registration and hand-kept name lists: ≈400. A new module is registered
  by hand in about fourteen compiler places and sixteen tool places; the
  lists disagree (9, 10, 16 and 17 modules in different ones).
- Optimizations keyed on particular builtins (text builders, plain
  vectors, counted ranges, in-place forms): ≈1,500, arguable.
- Code that lists every module's types (clone, display, JSON, equality,
  type words): ≈400, arguable.

Defects found on the way, fixed in phase 1: `Checks.ui_mode` and
`Lowerer.in_ui` disagree on `use web.ui`; `READERS` is kept twice with
different contents; `EFFECTS` names builtins that no longer exist (`time`,
`listen`); `builtin_arity_ok` has fifteen arms no call reaches; `row_iv`
and `RUNTIME_TABLE` name row ids that no longer exist; dead code
(`imports_module`, `Stmt::Resource`, `note_db_backend`, `string_lit`,
`__crow_time_result`, `needs_chrono`); `WEB_EXTRA_FUNCTIONS` says its
names are not reserved while `WEB_RESERVED` holds all three.

## 2. The shape after

**Language.** Syntax; the types the spec defines (numbers, text, bools,
lists, maps, records, error values, tensors, frames and columns, moments,
parsed values, bytes); their operators, literals, indexing and masks; the
reactive model (views, derivations, facts, handlers, sessions, commits), as
Elm fixes its architecture in its runtime so that the checker can see the
app's shape; the `grad` transformation, since a compiled program cannot
trace at run time; test blocks and resources (`cpu`, `gpu`); and the
general machinery of sections 4 to 9: complete rows, the manifest, effect
kinds, functions as row parameters, handles, answer typing.

**Modules.** Everything else, each builtin's specifics declared on its own
row, each module's on its manifest: the element and control vocabulary,
routes and responses, the database, the readers, mail, sockets, auth, llm,
chart, doc, bits, and every verb's own rule (a frame verb's columns, a
math function's shape, a function's derivative).

**The boundary.** A module written in Crow carries no declaration: its
functions are its contract, inferred as every program's are. Native code
exists only where design rule 20 allows it (the operating system, a
validated numeric kernel, a byte loop the interpreter cannot afford), in a
runtime crate, behind `sys`, each primitive one row. A Crow file never
names a Rust type, function or attribute.

**The physical boundary.** The compiler references `crow_runtime`, the
language's runtime, and no module crate: `crow_web` and `crow_db` are
reached through rows alone. Today the compiler calls `crow_web::live` for
facts and commits and `crow_db::schema` for tables; section 10 moves the
session runtime into `crow_runtime` and the ratchet holds the module-crate
references at zero afterwards.

## 3. What a program sees

Nothing new to write. The complete list of visible changes, all phases:

1. A default may be an empty list or an empty map (phase 2).
2. A list may hold functions of different signatures, usable through
   description and call-by-name only, never called positionally (phase 4).
3. Options renamed where a function's label is a builtin's name: `chat`'s
   and `follow`'s `max` becomes `tokens`; `chart`'s `lines` (open: section
   5). Element attributes are keys of an attribute map, so a slider's
   `min`/`max` and a field's `type`/`lines` stay.
4. A column refused by name is refused in the kernel's words (phase 6),
   with the same content.

Removed from the language's definition: fifteen one-module type variants
become one handle category; the hand-kept vocabulary lists become a
module's exports; rules stated per builtin (a view has no effects, a
handler's writes wait, a loop with an effect keeps its order, mail not in
a render) become one effect rule; inconsistencies go (today `chart`,
`serve`, `fmt` and `moment` options work without an import while `chat`'s
need `use llm`).

No new core builtin: core is frozen by the spec's budget rule. The two
operations modules need over functions (section 7) are `sys` rows.

## 4. Phase 1 — complete rows, one manifest, the ratchet, the defects

No language change, no behavior change. Everything a hand-kept list says
about a builtin moves into its row; everything the registration places say
about a module moves into its manifest; every consumer reads from there;
the lists and the places are deleted.

### The complete row

`Row` (src/table.rs) gains, each field transcribed from the list it
replaces:

| Field | Replaces |
|---|---|
| `effect: bool` | `ir::EFFECTS` |
| `draws: bool` | `ir::DRAWS` |
| `reads_world: bool` (the capture stamp) | `table::READERS` and `checker::READERS` |
| `held: bool` (held to the handler's commit) | the emitter's `HELD`, the interpreter's `EFFECTS`, `SESSION_EFFECTS` |
| `blocks: bool` | `checker::BLOCKING_BUILTINS` |
| `outside: bool` (a rehearsal records it instead of running it) | `rehearsal::outside`'s names |
| `source: bool` (a page's data source: `fetch`, `poll`) | `derive::is_source`, `uses::is_load`, `is_source_call` |
| `binds: Option<usize>` (a bound input's fact) | `BOUND_INPUTS`, `BOUND` |
| `test_only: bool` | the harness-verb list in `checks.rs` |
| `taken: Option<&str>` (the in-place runtime path) | `table::taken_path`; `ast::IN_PLACE_BUILTINS` becomes "rows with `taken`" |
| `big: Option<&str>` (the growing-int form) | `table::big_form`, `written_name` |
| `range: IntRange` (the answer's int range) | `int_width::row_iv`, `range_items` |
| `features: &[(&str, &str)]` (crate and Cargo feature) | `runtime_features`, `web_features`, `db_features`, `note_sources`'s Parquet rule |
| `options: &[(&str, Preset)]` and `written` (section 4.1) | `lower::builtin_named_params`, `request_options`, `window_options`, `reader_options`, `llm_options`, the `fmt`/`moment`/`chart`/`serve`/`field`/`slider`/`progress`/`icon`/readers arms |
| `mask_at: Option<usize>` | `table::takes_mask_at` |
| `form: Option<&str>` (how a message writes the call) | `table::form`, `shown`, `miscount` |
| `differentiable: Option<Deriv>` (dual and traced paths) | `grad::DIFFERENTIABLE`, `builtins::HANDLED`, `DUAL_MATH`, the emitter's traced dispatch names, `autograd_bridge`'s name matches |
| `scope: Scope` (bare in every file; bare under its module's import; qualified only) | `CORE_RESERVED`, `WEB_RESERVED`, `UI_RESERVED`, `MODULE_BARE` |

Rows that today are not rows become rows with an empty runtime path and a
`form` of their own, so that one table lists every name a program can
call: the db verbs (`MODULE_OWN_FORM`), web's `route`, `serve`, `page`,
the responses, the ui controls, the harness verbs, and the core own forms
(`OWN_FORM`) that are not syntax. Their bodies stay where they are in this
phase (the lowering, the backends); what moves is the fact that they
exist, their arity and their options. `builtin_arity_ok` is deleted: a
row's parameters say its arity.

### 4.1 Options on rows, as built

A row's options are its last parameters, each with a `Preset`: a literal
default (`Int`, `Float`, `Str`, `Bool`, an empty `List` or `Map`), or
`Asked`, no default, which makes the row apply only to a call that names
the option (`sum[x, axis = 0]` reaches `sum_axis`; `sum[x, 0]` is left to
the row choice by kinds, as it was). One filler in `lower.rs`
(`row_options`) reads every row of the name, the forms included: the row
that takes what the call gives — positions in order, options by name, a
defaulted option by position too — fills in its defaults; of several, the
one filling in least (`read[path]` is the plain reader's); a count no row
takes, or an option none has, is the refusal, worded from the rows; a
call that names no option and fits some row by count is left as written.
The idempotence test (`lowering_twice_is_lowering_once`) found the one
subtlety: the time style's `fmt`, lowered to three positions, reads as a
number format given positionally when lowered again (a derivation's body
is), so a row an option picks may keep a hidden name of its own
(`written: "fmt"` on `__fmt_time`), which its lowered form is called by.

Forms live in `table::FORMS`, beside `ROWS`, with `IN_FORM` as their path:
their kinds are placeholders (`Shown`), read by no lookup by kind, and
the self-check writes no case from them; `arity_ok` counts them where
`builtin_arity_ok`'s table had. Left for later phases, each with its
reason: `route`'s `GET` is a positional parameter in the middle, which no
option says (phase 4 makes `route` a row taking a function and its two
forms two rows); `page`'s `url` check is a kinds check (phase 5); `fill`'s
value is a positional that `how` replaces (phase 6, frames); `decimal`'s
`digits` nests a conversion (a runtime function of its own, in phase 2);
the map and frame literals are language.

### The manifest, as built

`modules::MODULES: &[Module]`, one entry per module: `path` and `prefix`,
`source` (the Crow file, where there is one), `loads` (what its import
brings beside its parent: `db` its drivers), `needs` (the runtime crates
and their features), `internal` (`sys`, a driver), `docs` (the sentence
its book page opens with), `under` (the module whose section documents
it: web's modules), `driver` (a backend's name, URL schemes and row
prefix), `module` (its registry object: `Named`, every function a row by
name; math's with its constants; db's with its own arms). Four flags
transcribe lists the compiler had kept apart, each named for what its
readers make of it, to be unified when their readers are understood:
`registry` (a bare `use` makes its names), `known` (the tooling knows its
functions without an import), `pinned` (a qualified call is its bare call
in the emitter), `typed` (a qualified call is typed through the registry).
What reads it: `crow_source`, `crow_modules`, `module_by_path`,
`every_module_function`, `with_stdlib`, the inliner's parents and
prefixes, the `use` chain of `type_infer/classify.rs` (one loop: the
module, its parent, what it loads), the qualified-call rules of `rules.rs`,
`is_standard_module`, the suggestions and `known_module` of `checks.rs`,
`load_module` and `read_uses` of the emitter, its pinned modules, the
tables and page intros of `crow test --docs` (`names_of`: a module's rows
and forms and those documented under it), the table's drift test, the
drivers' bridge. Gone with it: the ten per-module Rust files that were
registration boilerplate (`bits.rs`, `chart.rs`, `doc.rs`, `frame.rs`,
`io.rs`, `llm.rs`, `nn.rs`, `stats.rs`, `sys.rs`, `web.rs`), the hand lists
`DB_FUNCTIONS`, `WEB_FUNCTIONS`, `WS_FUNCTIONS`, `MAIL_FUNCTIONS`,
`AUTH_FUNCTIONS`, `CLICKHOUSE_FUNCTIONS`, `POSTGRES_FUNCTIONS`. `crow test
--modules` runs every Crow-written module's own tests from the manifest.

### The ratchet

`src/freeze.rs`: a unit test scans the compiler's source files (listed
there; the tools are not among them) for the module names of the manifest
and the builtin names of the table written as string literals, and for
references to a module crate, and compares each file's count with
`tests/freeze/names.txt`. A count above its baseline fails. A count below
it fails too, naming the file, so the baseline is lowered
(`CROW_FREEZE_BLESS=1`) and committed: the file only shrinks. The rows
table itself is scanned outside its `ROWS` array.

### Held by

The whole gate, unchanged in outcome: no program behaves differently.

### As built (2026-10-18)

The ratchet went from 2,866 names to 2,069, and from 206 module-crate
references to 205. Every list named in the row table above is gone, with
`builtin_arity_ok`, `OWN_FORM`, `MODULE_OWN_FORM`, the hand-kept function
lists of db and web's modules, and ten per-module Rust files. What stays
in the compiler on purpose, each for a later phase: `route` (its `GET` is
a positional parameter in the middle), `page` (a kinds check), `fill` (a
positional that `how` replaces), `decimal` (a nested conversion), the
core's five dual forms and the language's `type`, `assert` and `error` in
the interpreter's own arms, the derivative implementations keyed by name
(a derivative per row, phase 6), `RUNTIME_TABLE`, and `form()`'s names for
the hidden rows the lowering writes. The manifest's four flags
(`registry`, `known`, `pinned`, `typed`) transcribe lists that disagreed;
their readers are to be understood and the flags unified.

## 5. Phase 2 — a Crow-written module's functions are its declaration

**Language changes.**

- A default may be `[]` or `map[]`. The spec's reason for literal-only
  defaults (no evaluation order, no scope) holds for an empty collection.
  Every non-scalar option in the standard library is one of these.
- The `Encoded` convention goes: a module function that takes a parsed
  value takes it as one. The `json[…]` at each function's door and the
  JSON-text hand-off are deleted.

**Resolution.** Nothing new. A module's function inlined into a program is
a named function, and named arguments with defaults already resolve for
named functions (spec, Named Arguments with Defaults). The 81 `crow!` rows
exist only because a row could hold neither a collection default nor a
parsed-value parameter.

**Exports.** A row list no longer says which functions a module exports.
A function whose name begins with one underscore is a helper, invisible
outside its module, as Python's convention has it; every other top-level
function is exported. Crow's double-underscore names stay the compiler's
hidden rows.

**Renames.** A parameter may not be named like a builtin, so an option
label that is one is renamed: `chat`'s and `follow`'s `max` → `tokens`.
`chart`'s `lines` is renamed when chart moves (section 9); candidates
`curves`, `series`. The note lists every collision the migration finds.

**What a module author writes.**

```
--- `chat[m, talk, system = '', tools = [], tokens = 1024, temperature = 1.0]`:
--- the model's reply to the conversation, in the one shape
fn chat[m, talk, system = '', tools = [], tokens = 1024, temperature = 1.0]
    …

--- the conversation as a list of turns, refused where it is none
fn _talk_of[talk]
    …
```

**Leaves the compiler.** The 81 `crow!` rows; `llm_options`,
`request_options`, `window_options`, `reader_options` and their arms;
`crow_call_args` and the hidden conversion rows (`__truncated`,
`__text_map`, `__sizes`, `__groups`); the doors (`talk_of`, `model_of`) in
the modules. Native rows keep their options in the row (phase 1).

**Open questions.**

1. `crow check --self` writes its cases from rows. For a Crow-written
   module it must write them from the functions' inferred signatures. A
   tool change, landed in the same phase.
2. A function a module's tests never call, whose parameter nothing types:
   refused as today, with the same advice. The module's tests are its
   contract's proof.
3. Specialization: a module's function called at two signatures is copied
   per signature as a program's function is. The inliner keeps the rows a
   program reaches today; it keeps the functions a program reaches after.

**Held by.** Each migrated module's own tests, unchanged; a parity program
per module; the errors corpus for the renamed options and the underscore
rule.

## 6. Phase 3 — effects declared on native rows, inferred elsewhere, contexts enforced

**Language change.** An effect is a property with a kind, not a name in a
list. A native row declares its kinds in the row; a Crow function's
effects are the union over its body, which the call graph already
computes; nothing is written in a program. Phase 1's booleans collapse
into these:

| Kind | Today's builtins |
|---|---|
| `files` | read, write, mkdir, ls, rm, mv, cp, csv, lines, load, save |
| `network` | request, fetch, poll, send, recv, socket, the readers of URLs |
| `db.read`, `db.write` | query / exec, insert, migrate, transact |
| `mail` | mail |
| `clock` | now |
| `random` | rand, seed, randn, normal, …, shuffle, choice, permutation |
| `input` | input |
| `process` | shell, exit |
| `session` | notice, sign_in, sign_out, download, later |
| `waits` | sleep, input, shell, recv, next |

Flags beside the kinds: `held` (lands at the handler's commit), `source`
(a page's data source), `test_only`.

**Contexts become rules of the language, stated once:**

| Context | May |
|---|---|
| view | no effects (console writes allowed) |
| derivation | `files`, `network`, `db.read`, `clock` (a clock read makes it fresh); no writes, no `session`, no `mail` |
| handler | anything; `session`, `db.write` and `mail` are held to its commit, and a failed argument of a held effect stops the handler |
| function | anything |
| test | `files`, `network`, `process`, `mail`, `db` replayed from fixtures where recorded |
| route handler | answered on the server's thread when it has no `waits` and no effect |

Loops: a body with an effect or a `random` draw keeps its order. Values
from `files`, `network`, `db.read`, `input`, `process` carry the capture
stamp. A rehearsal records `files`, `network`, `process`, `mail` instead of
running them.

**Leaves the compiler.** `EFFECTS`, `DRAWS`, both `READERS`,
`SESSION_EFFECTS`, `HELD` and its interpreter twin, `MAIL_IN_A_RENDER`,
`BLOCKING_BUILTINS`, rehearsal's list, the baseline's `EFFECTS`/`READS`/
`capability_of`, the source and bound-input lists, `quick_fns`' name
checks.

**Open questions.**

1. `log`: today an effect in some rules and allowed in views and under
   `grad`. Recommended: console writes are no effect anywhere; only
   `input` is.
2. Effects through function values (`any[xs, pred]` where `pred` mails):
   the union rule, following function values as typing does. Confirm the
   call graph does.
3. `later`: a `session` effect, or a kind of its own.

**Held by.** The errors corpus (every context refusal, worded once),
`crow check`'s fixtures, the self-check's purity cases, parity for held
effects.

## 7. Phase 4 — functions as values for native rows; description and call by name

**Language changes.**

- A native row may declare a parameter that takes a function, with its
  signature: `request → response` for `route`, `(put, …)` for `stream`'s
  feed, `tx → T` for `transact`, `item → bool` for `any`, `all`, `find`,
  `() → ()` for `later`. Both backends call it one way: a function pointer
  compiled, a function value interpreted. These become rows.
- A list literal may hold functions of different signatures. Such a list
  is described and called by name; a positional call of its items is
  refused, naming why.
- Two `sys` rows, for the standard modules: `describe[f]` answers a record
  with the function's name, its doc comment, its parameters (name, kind as
  the word errors use, default) and what it answers; `apply[f, args]`
  calls `f` with `args`, a parsed value or a map, each argument read by the
  parameter's kind; an unknown name or a missing argument is an error
  value of kind `'call'`; the function's own failure is its value. Both are
  computed from inference, so both modes agree by construction; a function
  whose parameter is typed by nothing is refused with today's advice.
  Compiled, the dispatch for a list of functions is a match the compiler
  writes for that list, knowing nothing about who asked for it.

**What a module author writes.** `llm.co`'s loop, in plain Crow:

```
fn act[m, talk, tools = [], allow = 0, rounds = 8]
    schemas = for t in tools / _schema_of[describe[t]]
    …
    for call in r.calls
        if _allowed[allow, call]
            talk = told[talk, call, apply[_named[tools, call.name], call.args]]
```

`mcp.co`'s `tools/list` and `tools/call` are the same two rows.

**Leaves the compiler.** Every special case for `any`, `all`, `find`,
`later`, `stream`, `route`, `transact` in checks, rules, calls, classify,
the width pass (`called_by_name`) and both backends; `llm_tools`,
`tool_schema`, `handed_schema`, `param_reads`; `chart_seek_handler`; the
ground for event handlers on elements (section 9).

**Open questions.**

1. A list of functions is a literal handed on through parameters, never
   stored in a fact. Recommended: yes at first.
2. `apply` by name only (a parsed value or a map). Positional lists later
   if a module needs them.
3. Whether a program outside the standard library may use `describe`:
   not in this phase; a package that needs it is a decision then.

**Held by.** Parity (a list described and applied in both modes), the
errors corpus, `tests/llm_server.rs` driving the loop against the stand-in
model, the self-check's routes family with `route` as a row.

## 8. Phase 5 — handles declared by modules; records declared in Crow

**Language change.** One type, `Ty::Handle(name)`, and one interpreter
value, replacing the variants that exist for one module each. A handle's
declaration sits beside its native rows: its name, its display word, its
Rust type, its default, and three properties: comparable, session-safe (a
fact may hold it; otherwise it is a process static, as a connection is
today), and how it shows (its word, or its text, as a page's node shows
as its HTML). One rule covers all: not indexable, not walkable, not JSON,
not in a mask, moved not copied.

A native row that answers a record names a record declared in Crow in its
module's file (`struct reply` with `status`, `body`, `headers` in
`core.co`), replacing the synthetic structs (`CrowShell`, `CrowReply`,
`CrowAnswer`, `CrowFindResult`) inside the compiler.

| Handles | Stay language types |
|---|---|
| db connection, transaction, socket, stream, mailer, response, app session, page node (markup) | tensor, frame, column, moment, parsed value, bytes |

`WebRequest` is a record (`method`, `path`, `body`, `headers`, `query`,
`params`, `form`, `files`), declared in `web.co`. `WebApp` goes: the app
is implicit.

**Leaves the compiler.** The per-variant arms in `Ty`, `Kind`, `Arg`,
`Value`, `totality.rs`, the emitter's imports and clone rules, the
interpreter's display, equality and type-word tables; the synthetic
records.

**Open questions.**

1. Sparse matrices: a handle (then `+` and `*` over them need the
   language's operators to know it) or a language type. Recommended:
   language type, beside tensors.
2. Handle equality: every handle non-comparable. Recommended: yes.
3. A handle in a view: shows as its word.

**Held by.** `totality.rs` reduced to one arm, the self-check's handle
cases, parity.

## 9. Phase 6 — answer typing: frames by evaluation, shapes by pattern, providers

**Frames, by evaluation.** A frame's type is an empty frame: columns with
kinds, no rows. When a frame verb is called with known columns and literal
arguments, the compiler builds that empty frame from the type, runs the
kernel both modes call, and reads the columns off the answer. A kernel's
refusal (`column 'x' not found — the frame's columns are a, b`) is the
compile-time refusal, so a module's checks come for free. Column forms
(`int[col]`, `upper[col]`, window aggregates) the same way on an empty
column. Two declared exceptions: `with`'s new column takes the
expression's type; `pivot`'s columns depend on data, so its answer is open.
Non-literal arguments give an open frame, as today. A frame whose columns
are unknown (a `load`) keeps today's typing from use: generic inference
over an open record, which stays language. Rows with effects are excluded
from evaluation.

**Tensors, by pattern.** Each row declares its shape rule from a fixed
set: `same(i)`, `scalar`, `matmul(i, j)`, `reduce(i, axis argument)`,
`sizes(arguments)`, `stack`, `pieces[…]` (`svd`, `eigen`), `vector(i)`,
`open`. Operators stay language.

**Providers.** A row may name a provider: a function in its runtime crate
that the compiler calls with the literal arguments it has and that answers
columns and kinds. `query`'s provider lives in `crow_db`: connecting,
describing, parsing SQL for nullability, caching, `.crow/schema`. The
compiler keeps the one hook. Non-literal SQL gives an open frame, as today.

**Chart and doc.** `chart`'s columns are checked by its own evaluation on
the empty frame once chart is a Crow module over its kernels (its 12
options move into its signature, phase 2; its seek handler is a function
parameter, phase 4). `doc.chapters`' record is declared in `doc.co`.

**Leaves the compiler.** `frame_rule`, `column_typed`, `check_columns`,
the chart and doc column tables, `row_dims`, `call_dims`, `piece_dims`,
`type_infer/frames.rs`, `sql_types.rs`, `schema.rs`'s attach (the DDL is
the provider's input): about 2,500 lines.

**Open questions.**

1. Kernels evaluated at compile time run with zero rows: they must never
   panic (the runtime crates deny panics) and must be pure (rows with
   effects are excluded).
2. A kernel's message is a compiler message: one wording convention for
   kernels' refusals. They mostly follow it.
3. The provider's cache keys and offline behavior: today's rules carried
   over.

**Held by.** A differential test during migration: every frame verb's type
by evaluation equals today's `frame_rule` answer across the corpus, before
the table is deleted; the pandas-held frame tests unchanged; the errors
corpus; the `crow db` tests.

## 10. Phase 7 — optimizations on properties, analyzers in crates, the session runtime

- Optimizations stay in the compiler (design rule 24 allows them) and key
  on declared row properties — element-wise, pure, has an in-place form,
  counted range, pure conversion — never on names: the text builders, the
  plain vectors, the counted loops, the fusions (`tensor.fused`,
  `__pick_where`, `__sub_len`).
- A module's analyzer is Rust in the module's crate, registered through
  the manifest; `crow check` runs what is registered. The database hazards
  (`db_check`) become `crow_db`'s analyzer, the UI rules the baseline's.
- The session runtime (facts, derivation specs, transactions, snapshots,
  `later`) moves from `crow_web::live` to `crow_runtime`: it is the
  language's. `crow_web` keeps transport: HTTP, WebSocket, HTML patches.
  The compiler's references to module crates reach zero, and the ratchet
  holds them there.

## 11. How it is held

- One gate per milestone (CLAUDE.md's list), the ratchet in it.
- Each phase ends with every count in `tests/freeze/names.txt` at or below
  where the phase began, and the deleted lines named in the commit.
- Each phase's section is read and its questions answered before its code
  begins; a change to the compiler outside the phase's scope is asked for
  on its own.
- A behavior that both modes must keep gets a parity program; a refusal
  gets its line in the errors corpus; a migrated module keeps its own
  tests green at every commit.
