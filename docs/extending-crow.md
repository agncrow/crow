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

### As built (2026-10-18)

**What moved.** 45 of the 81 `crow!` rows are gone: every row that was a
pure contract — stats (12), nn (8), sys (11), auth (2), the socket's
`socket` and `close`, and llm (10). A program's call of one is
the inlined function's call, typed by its use, a copy per signature, its
answer its body's. The inliner (`modules::inline_crow_modules`) is the
one place that knows a module's exports (`modules::exports`: the
source's top-level functions with no leading underscore, less those a
row still names): it rewrites the program's calls — bare, by the
module's path, by its alias, handed as a value, inside an interpolation
— to the inlined names, and each module's calls of what it imports the
same way; the stages below it see a program function. The `Encoded`
hand-off and the `json[…]` doors are gone from the migrated modules: a
function takes the value the program holds and reads it as its JSON where
it needs to (`encode[claims]`). A refusal inside such a function names it
and the program's line (`in relu: clip[] takes numbers, got an int`),
and every message names the function as the program writes it
(`modules::unprefixed`, `written_as`). Options are the functions'
defaults, `[]` and `map[]` allowed (the parser's one change); the
language gained nothing else.

**What stayed, and why — each a later phase's.** The 36 rows left are
of four kinds. The session-effect rows keep the properties the session
layer, the rehearsal, the checker's staleness analysis and the loaded
frame's columns read at the call's name — `Effect`, `Reads`, `Held`,
`Outside`, `Session`, `Source`, `Table` — which a function has only once
effects are declared on the layer's own rows and inferred through the
call graph (phase 3): `read`, `write`, `request`, `mail`, `fetch`, `send`,
`recv`, `csv`, `lines`, `load`, `save` (18 rows, the `_asked` pairs among
them). The variadic rows take a rest parameter no function of the
language can (phase 4): `randn`, `normal`, `anova`. `msgpack` takes a value
of any kind and must keep it — an int beside a float would be one function
of floats under the number rule, and `msgpack[3]` packed `3.0` (the
self-check's route family) — a parameter of no kind the language can
declare yet (phase 4): 1. The drivers' rows are a contract crow_db calls
from Rust, not a program (phase 6): 14. With
them stay `crow_call_args`, `crow_target`, the hidden conversion rows
and the row-typed contract in inference, each serving only those rows.

**Decisions taken** (open for Vijay's veto, each a module's change). A
helper begins with one underscore (`_talk_of`), as designed; a record a
module declares is its own until phase 5. `chat`'s and `follow`'s `max`
is `tokens` (a parameter may not be named like a builtin). The one kind
overload among the rows, `ask[text]` beside `ask[talk, text]`, became
`ask[text]` and `asked[talk, text]` — the verb family `asked`, `said`,
`told` adds a turn — since a function has one signature per name; a
program that bound `asked` as a variable under `use llm` is refused as
any clash with an import is (the rag example's was renamed). The
functions behind the rows that stayed keep their names (`read_asked`,
`recv_for`), excluded from the exports.

**Open question 1, answered.** `crow check --self` writes a case family
from signatures (`Sig`): a row's from the table, an export's read off the
module lowered with its own tests (`export_sigs`); a parameter the tests
leave untyped, or that takes a function, is listed as not called with
why. The reference's keys are unchanged (`math.stats.median`); the
agreement-only list's ids follow the module path (`stats.median` is
`math.stats.median`), blessed once. Open question 2 holds as stated;
question 3 is the specialization the IR shows (`_math_nn_relu__2` for a
matrix beside the vector's).

**The ratchet** no longer counts the tests of a compiler file (a test
names what it holds the compiler to); the baseline was blessed on that
rule, and parser.rs rose by the one literal `map` the default needs.

**Six compiler bugs the phase met, fixed as bug fixes.** A function
called from a copy of its caller, with an argument whose kind settles
only once another copy answers (`inner[may[m, false]]` in `outer__2`), was
never copied for that kind: the clones' calls are now read again after
their bodies settle (`specialize`, parity 259). A double underscore
inside a name read as a copy's mark (`_llm__model_of`, a helper behind
its prefix), so such a function was never copied at all: one predicate,
`ir::copy_base`, names a copy (`name__N`) everywhere. A loop's variable
whose item kind was not known yet kept a binding of the same name from
outside the loop — a module's `for t in tools` read the program's
top-level `t` — so a loop variable is now its own binding whatever is
known of its item (`scan_loop_var`, `bind_loop_var`). And a module's
local named like a fact of a page (`turns`) had made the module's
function a handler, compiled to answer nothing: the handler class is
read from the program's functions alone (classify.rs), as the lowering
already read it. And, in the emitter, a failed value's payload converted
where a copy's parameter takes another form (a failed tensor's numbers
as the list of floats a copy of `ask` takes) was handed by value to a
parameter the callee borrows, and the program did not build: the
conversion is lent as a value a name holds is (`&(…)`, exprs.rs's
propagating path; the self-check's `fail:vector` cases, which build
alone and failed only in their batch's company — a no-build finding now
carries the compiler's own detail and the generated line). And the last,
found by the export cases handing a frame's column with missing cells
where sys's `exchange` takes its head lines: a loop's item from such a
column was typed as nothing known by the width pass, because
`TypeContext::loop_item` — the rule every pass reads for a loop's item —
knew no column, while the lowering and the emitter each had a column arm
of their own; a function writing the item into text declared the text's
temporary plain and assigned it a value that can fail, and did not build
compiled, where the interpreter stopped on the missing cell. The rule
answers a column's cells now, and the lowering reads it (parity 268).

**Four more the self-check found once the rows were gone, each older
than the phase and fixed in the lowering (the compiled program had not
built where the interpreter ran or stopped).** Three are rules both modes
now refuse before the program runs: a list whose items no type says,
compared with text (`m eq 'GET'` with `m: List(?)`, from a call handing
`[]`) — two kinds are not compared, and bytes compare with bytes alone; a
function returning two kinds (`return 'a'` beside `return x` of an int) —
a function answers one kind, refused naming both where it had been the
verifier's internal error; a field read of a value with no fields (text,
a number, bytes, a handle) or a field a tensor or a moment lacks (`c.id`
with `c` a tensor, guarded by `type[c]` at run time) — refused naming the
kind's own fields, which the runtime names (`tensor::FIELDS`,
`Moment::FIELDS`); a map's field is its key, read as the program runs.
The fourth is typed by use: a parameter handed `[]`, read where a row
takes a tensor (`tensor[pred]` in `mse`), is a list of numbers
(`retype_unknown_lists`, settled in the scan), so the call's `[]` is an
empty tensor where the formula computes with it, as the row had read it
(a first version refused it, and refused `argsort[xs]` over a list the
values type in three packages with it: a list a body makes stays the
values' to type). **One found and left:** a function called only with
`[]` hands items of no kind to a function it calls, which is then not
emitted (`fn k[s] return json[s]` reached only through `for x in xs: k[x]`
with `xs = []`); `chat` shows each tool's schema as text (`'`t`'`) so its
`tools = []` has a kind. The language's (typed by use), not a module's.

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

### As built (2026-10-19)

The questions, decided by recommendation (Vijay: "go with your
recommendation"): (1) `log` is an effect of its own kind, `console` —
not no effect, since output has an order and the spec's loop rule keeps
it (a loop that logs runs in order in both modes); a view and a derivation
may write it. (2) Effects through function values: the call graph already
counts a function handed to a call as called (`handed` in lower_ir.rs),
so a view calling `any[xs, pred]` where `pred` mails is refused naming
`pred`; confirmed by errors j55. (3) `later` is `session`: it acts for
the session as a notice does, held to the commit. Two more the code
asked: a view may read the clock, since the spec gives a live program
one moment per event, the render's included (`now[]` in a view is the
event's moment), and a view or a derivation may draw — a draw is no
effect on the world (the seeded generator is the program's own: lowered
as a call, not an `Expr::Effect`; it orders a loop and makes a function
impure to `crow check`). And a query in a view, which the book taught as
"query at render time", is refused: a derivation is where a page reads a
table (recomputed off the render when the table is written or a tick
passes), and the book's dashboard and chart examples derive theirs now —
which found a derivation indexed (`trades[mask]`) read as a call of it,
fixed in the lowering's index path. A call written with its module reads
its effects from the module's own row (`table::effects_of_in`): the bare
name had made `ui.input[…]`, the element, the core's `input`.

Three kinds the table of section 6 lacked, and two flags: `exit` beside
`process` (a derivation may read the environment — the rag example's
derivation embeds through llm, which reads its key from `env` — and may
not end the process), `files.write`
beside `files` (a derivation reads a file and may not write one; a
rehearsal records a write and runs a read), `shell` beside `process` (a
rehearsal records a command and answers a quiet run, and the reach names
it), `serving` for `route`, `page` and `serve` (effects today, declared
at the top level); and `Prop::Ordered` (`assert`, `running`: no effect,
but a loop over it keeps its order) with `Prop::Varies` (`running`,
`captured`: answers by the run, so a function `crow check` probes as pure
cannot call it) — the three names the loop rule and the checker's purity
had kept as names. `Prop::Effect`, `Draws`, `Reads`, `Held`, `Session`,
`Blocks` and `Outside` are gone: one `Prop::Does(Effect)`, and the kind
answers `held` (session, db.write, mail), `stamps` (files, network,
db.read, input, process, shell), `recorded` (files.write, shell),
`acts_for_session` (session, mail), `waits`. The rules live in
`src/effects.rs` (`Effect`, `Context::allows`, one `refusal` wording) and
the table's helpers (`effects_of`, `is_effect`, `held`, `stamps`,
`waits`, `ordered`, `impure`); the baseline's `EFFECTS` and
`capability_of` and the checks' `MAIL_IN_A_RENDER` are gone, the
capabilities are the kinds' words.

Behavior that changed with the rules: a view reaching any effect but the
console, and a derivation reaching a write, a session effect or mail, are
refused (they had been performed at every render); a handler's failed
argument stops it only for a held effect (`read`, `request`, `query`,
`shell`, `fetch` had stopped it too — a read with a failed argument now
answers the failure, as any call does); `exists` is a `files` effect;
`args` and `env` are stamped. Held by errors j54–j56, the suites, and
the self-check.

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

### As built (2026-10-19)

The questions, decided by recommendation: (1) a list of functions is a
literal flowed through parameters, never a fact's value — a group
(`Ty::Fns(names)`), each value its name compiled, typed before its items
are lowered; (2) `apply` by name only, from a parsed value or a map; (3)
`describe` and `apply` are sys's (the self-check's door, `CROW_SYS`, and
the standard modules reach them); a program's own packages do not, this
phase. Decided beyond the questions: `describe` answers a parsed value
(a record would need phase 5's declaration in Crow) whose parameter
field is `takes`, not `kind`, since a value's `kind` is its error's;
`apply` answers the value encoded as text, as text that can fail — the
branches of a dispatch answer one kind; a positional call through a
group member is a dispatch too, allowed where the members take and
answer one kind a position can hold, refused otherwise ("call it by
name"); a group member is no function value, so a row taking a function
(`any`, `later`) refuses it.

Built: `Kind::FnOf(&Sig)` with the row's declaration (`PRED`, `DEFERRED`,
`PUT`/`FEED`, `HANDLER`, `TX`, `SEEK`; `Walked`, `Request`, `Db`, `Tx`,
`Found` beside it; `Prop::Answers` for a form typed by its row;
`Preset::Event` for an option the page calls); `table::fn_params`,
`walked_param`, `event_positions`, `deferred_positions`; inference's one
rule `hand_to_row`; `lower::event_handlers` and `deferred_targets` in the
classification and the checker; `called_by_name` by signature; the
dispatch (`described`, `dispatch`, `applied`, `__same_fn`, the `__arg_*`
and `__ok_text` rows); llm.co's `_schema_of` and `_json_type` over
`describe`. Gone: the name arms for any/all/find/later/running/stream/
route/transact in rules.rs, calls.rs and checks.rs; `chart_seek_handler`;
`llm_tools`, `tool_schema`, `handed_schema`, `Handed`; the width pass's
names. Kept for a later phase: the backends' arms for these builtins —
`any`/`all`/`find` run by name in both backends until a function-taking
row can call back into the interpreter (a `Caller` the runtime's kernels
take), and `later`/`stream`/`route`/`transact` are the session's, the
server's and the database's, which phase 7 moves.

The call-by-name lowering is `src/lower_ir/named.rs`, its names (the
two forms, the hidden rows, `json`, `encode`, `error`) counted there by
the ratchet; lower_ir.rs ends the phase at the count it began with. The
self-check's pairs found the one resolver (`module_of_bare`) reading the
core's runtime rows alone: `find['a', 'b']` under `use sys` reached the
layer's row of bytes — a core form that declares its kinds comes before
a module's row now.

Held by: parity 269 (a group's dispatch, both modes), errors j57 (a
positional call through a group of several signatures) and j58 (a group
member where a function is taken), llm.co's own tests (`describe` and
`apply`), tests/llm_server.rs (tools through `describe` in both modes),
and the gate.

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

### As built (2026-10-19)

The questions, decided by recommendation: (1) sparse matrices stay a
language type (their operators are the language's); (2) every handle is
non-comparable (`a eq b` of two handles is refused, in the one wording,
whatever the handle); (3) a handle in a view shows as its word. Built:
`table::Handle` (name, crow name, word, Rust type, default, session,
and whether it is read by its fields — a test's app session is, its
fields the page's facts; no other handle has any) and the eight
declarations `DB`, `TX`, `SOCKET`, `STREAM`, `MAILER`,
`RESPONSE`, `WEBAPP`, `APP` beside the rows; `Ty::Handle(&Handle)` for
the eight variants `Db`, `Tx`, `WebApp`, `WebResponse`, `Mailer`,
`Socket`, `Stream`, `App`; `Arg::Handle(&Handle)` for the five classes;
the row kinds (`Kind::Mailer`, …) stay as the rows' bridges to the
runtime's own types, reading the declaration for their type, word and
default; every list of handle variants in the compiler (the clone rules,
the display rules, the comparison, the fields a value lacks, the static
facts, the type words) is `Ty::Handle(_)` or reads the declaration;
the ratchet (src/freeze.rs) reads the table without the declarations, as
it reads it without the rows — a declaration names its module and the
runtime type by right.
Deferred, each a design of its own: `WebRequest` as a record declared in
web.co and the synthetic records (`CrowShell`, `CrowReply`, `CrowAnswer`,
`CrowFindResult`) declared in Crow — the emitter's generated structs and
the server's `CrowRequest` must agree, so a declared record would need
the record's layout to be the runtime's; a page's node stays the
reactive model's (language), as the note's language paragraph says; one
interpreter value for every handle — the `Value` variants are the
runtime's own types, which the bridges read by the declaration, and a
single boxed value would make each row's bridge a downcast.

Held by: parity 270 (a handle shown, held, moved, in a list, both
modes), errors j59 (two handles compared), totality (the eight handles
as samples of the one variant), and the gate.

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

### As built (2026-10-19)

**Tensors, by pattern: built.** `table::Shape` is the vocabulary — `Open`,
`Same(i)`, `Broadcast(is)`, `Sizes(from, but)`, `Dims([Dim])` (a dimension
`Open`, `Size(i)` the literal at an argument, `Of(i, j)` an argument's
dimension), `Reduce(i, axis)`, `Like(i, first)`, `Along(i, axis)`,
`Flat(i)`, `Transposed(i)`, `Stacked(i)`, `Restacked(i)`, `Diag(i)`,
`Items(i)`, `Pieces([[Dim]])` — and 118 rows declare theirs; the one
reader is `lower_ir/shapes.rs`'s `declared` (`declared_pieces` for a
decomposition's parts), applied where the call's row is chosen, with
the filled arguments. Where the lowering picks no row (one its values
pick as the program runs), the shape is what the rows of the name that
answer a tensor agree on (`agreed`). The declarations were held to the
rule by name over the whole corpus — parity, errors, book, packages,
layout, pandas, modules, fuzz, rag, served, and every case the
self-check lowers — with `CROW_SHAPE_DIFF` writing each difference, until
the differences were only the ones chosen: a list handed where a tensor
is taken is read as the list it was (`seen`, through the conversion the
row's arguments insert), a frame handed to `tensor[]` is a matrix where
the rule by name had said nothing, and a sparse system's solution and a
least-squares solution are vectors. `row_dims`, `call_dims` and
`piece_dims` are gone: 117 names off `shapes.rs`. Not a pattern: the six
functions of the standard modules written in Crow the rule by name had
shaped (`relu`, `sigmoid`, `softmax`, `gelu`, `zscore`, `linear`) — their
bodies' inference shapes their answers, as any Crow function's does.

**Providers: built.** `table::Provider` (the connection argument, the
statement argument, a function of the row's runtime crate handed the
connection's literal URL, the literal statement and the `CREATE TABLE`s
the program declares on that connection, answering name and kind pairs)
is declared on `db.query`'s form; `crow_db::describe` is `query`'s
provider — asking with one connection per URL for the build and one
answer per statement for a minute, the scratch database a program's own
statements build, `crow db describe` for a driver written in Crow,
`sql_types` for what SQLite does not type and for which columns can hold
NULL — moved whole from the compiler with its tests (`sqlparser` is
crow_db's too; both behind the `sqlite` feature, which a generated
program does not enable). The compiler keeps the hook (`describe_sql`:
which connections the program opened, scrubbing spans to NULL, the
`.crow/schema` record read when no database answers, the notes), the
kind-to-type reading (`cell_type_of_kind`) and the schema the program
declares (`lower_ir/schema.rs`, which knows a connection by the form
that opens it, `Prop::Connects`, or by the handle it answers). A row
whose kernel fixes its frame's columns declares them (`columns`: doc's
`chapters`, chart's `macd`). `sql_types.rs` leaves the compiler (71
names), `type_infer/frames.rs` keeps 4 of 10.

**Frames, by evaluation: not built.** The rule per verb stays in
`lower_ir/frames.rs` (`frame_rule`, `column_typed`, `check_columns`: 77
names, 786 lines). What the design needs, found in reading it against
the code: a frame verb's rule today is more than a kernel's answer on an
empty frame — `unpivot`'s value column is the kinds of the named columns
joined by a rule (ints, else floats, else one kind, else a parsed
value), `complete` turns the named columns' fallible cells certain,
`with`'s column takes its expression's type, and each verb's refusal of
a column the frame lacks names the verb and the frame's columns in the
errors corpus's words; most frame verbs are written in Crow now (phase
2), so evaluating one at compile time means running the interpreter on
the module's function inside the lowering. Recommended: a vocabulary of
column patterns declared on the rows, as the shapes are (`Same(i)`,
`Named(i)`, `Without(i)`, `Renamed(i)`, `With(i, expression)`,
`Joined(i, j, keys)`, `Grouped(by, aggregates)`, `Open`), with the
refusal of a missing column one rule over the arguments that name
columns — a phase of its own, after the chart module moves (its 12
options, its seek handler) so that `check_chart_columns` goes with it.

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
