# Design Review — July 2026

An honest external assessment of Crow's syntax, features, and gaps, plus the fix plan. Each pushback below gets resolved (fix or explicit Design Note rejecting the fix) and is then checked off.

## Ratings

| Axis | Score | Why |
|---|---|---|
| Design coherence | 9/10 | Documented rejections, consistent principles — rare at any scale |
| Syntax | 7/10 | Clean and uniform; loses points for `[[...]]` ambiguity, comment syntax, string building |
| Semantics | 7/10 | Error-values and for-as-expression are strong; early-exit hole and parallel-fallback trap cost it |
| Feature breadth | 8/10 | Tensors + frames + db + web + LiveView in one binary |
| Feature depth | 6/10 | A mile wide; each vertical is ~20% of its Python counterpart |
| Tooling | 4/10 | fmt/check/REPL exist; no LSP, no debugger, no syntax highlighting |
| Ecosystem | 1/10 | Registry exists but is empty (matches roadmap self-rating) |

**Overall: ~6.5/10 as a usable language today; top-decile as a language design project.**

## Strengths (keep, invest)

1. **Design discipline** — the Design Notes documenting *rejected* features are the project's most impressive artifact. Every decision traces to stated principles.
2. **Error-value system** — `ok/reason/kind/trace` with auto-propagation and auto-unwrap is a genuine contribution between exceptions and `Result`.
3. **`for`-as-expression** — one construct covering map, filter, map+filter, with parallelism built in.
4. **Vertical integration** — `examples/dashboard.co` (a reactive trading dashboard in ~50 lines, no npm) is the product.
5. **`crow check`** — the most differentiated feature; currently the least developed relative to its potential.

## Pushbacks and Fix Plan

Ordered by "gets more expensive to fix every day" first.

### 1. Frame API breaks syntax uniformity (fix first)

`sort[filtered, pnl, desc]` and `group[today, playbook_code, [pnl, sum]]` use bare unquoted identifiers (`pnl`, `desc`) that are not variables in scope — a special evaluation form, i.e. a DSL. This violates design rule 19 ("every line must be valid Crow, no DSL"). `docs/frame-module.md` even shows `>` and `==` operators that don't exist in Crow. UI Phase 3 (`table[]`, routing) builds directly on the frame API, so this decision compounds daily.

**Fix:** decide one of: (a) column references become real, specified syntax with defined semantics; (b) columns are strings (`sort[df, 'pnl', 'desc']`); (c) columns are expressions over `df.col` returning boolean/sort-key arrays (already how `filter[df, df.age gt 50]` is specced). Update spec, frame-module doc, both implementations, and all examples together.

- [x] **Decision: strings for names + expressions for computation** (b + c). Specced in `language-spec.md` ("Built-in Module: frame" section — previously absent from the spec entirely) with a Design Note documenting the removal.
- [x] Bare-name evaluation layer removed from validator, interpreter, and transpiler. What it was hiding: (1) the interpreter preferred an in-scope variable while the transpiler force-converted the same argument to a string — a live parity violation; (2) `dashboard.co` had a latent capture bug (`sort[filtered, pnl, desc]` where `pnl` was a local variable); (3) dynamic column names (`sort[df, col_var]`) were impossible in compiled mode — they work now.
- [x] Mask expressions completed in both modes: vectorized `eq`/`ne` (incl. string columns), `and`/`or`, `not` — these were only implemented for `gt/lt/gte/lte`, so `filter[df, df.name eq 'alice']` and compound masks previously failed. Also fixed while in there: `len[frame]` missing in the interpreter, whole-array equality missing for non-numeric arrays.
- [x] `docs/frame-module.md` and examples corrected (no `>`/`==`, no bare names); 15 regression tests added (734 passing).

### 2. Underspecified semantics + spec–implementation drift (next)

Edge semantics the spec does not answer:

- **Error-value in a conditional**: what does `if err gt 5` do? (Propagate? Falsy? Runtime error?)
- **int/float reassignment**: after `x = 10` then `x = 3.14`, what is `x`'s type? Interacts with `/` floor-vs-float division.
- **Output ordering in parallel `for`**: is `log[]` order defined? (Currently nondeterministic — surprising in a beginner's first program.)

Spec–implementation drift discovered during the frame fix (July 2026):

- **Named arguments are documented but entirely unimplemented.** `fn train[data, lr = 0.01]` and `sum[x, axis = 0]` are both parse errors ("expected ']', got '='), yet the spec has a full section plus a revised Design Note claiming they exist, and other docs lean on them. Either implement (parser → both modes) or strip from the spec. Blocks the planned `sort[df, 'col', desc = true]` migration.
- **Comparison broadcasting is asymmetric**: `df.age gt 50` (frame column = plain Array) broadcasts, but `[1, 2, 3] gt 2` fails — numeric array literals become NumArray, and `compare_values` has no NumArray arm. Decide: masks for all arrays, or frames only.
- **Display parity**: transpiled binaries print string arrays as `["a", "b"]`, the interpreter as `['a', 'b']`.

**Fix:** pick and document each in `language-spec.md`; add regression tests locking behavior in both modes.

- [x] Error-in-conditional specced + tested (see F4/#2 batch below — "errors have no truth value")
- [x] int/float reassignment specced + tested (float-for-scope rule, both modes)
- [x] Parallel `log[]` ordering documented as unspecified (§12)
- [x] **Named arguments: implemented** (July 2026). Parser accepts defaults in `fn` params (literals only) and `name = expr` at call sites; a single lowering pass (`src/lower.rs`) rewrites every call to positional form before inference/validation/execution — one implementation for both modes (kernel principle). Covers user functions (defaults filled), structs, builtins with named params (`axis` on reductions, `desc` on sort — reorder only, no filling), the variadic `map[...]` constructor (desugars to a `set[]` chain), and ui elements (named args become the attribute map). Limitation (documented): named args need a directly named callee — function-as-value calls are positional-only. `sort[df, 'col', desc = true]` migration done; parity file `13_named_args.co` locks both modes.
- [x] NumArray comparison broadcasting (resolved in F3: tensor masks)
- [x] String display parity between modes (resolved via the shared display kernel, pushback 6)

### 3. The early-exit gap (the self-inconsistency)

Crow rejects `break` in `for` because silent sequentialization is "a hidden performance trap" — but the recommended replacement (mutable-flag loops for any/all/find) triggers exactly that trap: mutating an outer variable silently forces the loop sequential, *and* it can't exit early (always O(n)), *and* it's more code.

**Fix:** either bless a small set of search/reduction builtins (`find`, `any`, `all` — loops the compiler can parallelize with early exit, which users cannot write themselves; same justification as `contains`), or allow `break` in `for` and retire the rationale. Update the three related Design Notes coherently.

- [x] **Decision: search builtins** (July 2026, settled through the backtest/parallelism discussion). `any[arr, pred]`, `all[arr, pred]`, `find[arr, pred]` — named-function predicates, `find` returns the first element (error-value on miss, flowing into the errors-at-decisions machinery); `find` dispatches by argument types against the regex builtin.
- [x] Implemented: interpreter sequential early-exit; **compiled mode parallel early-exit** (rayon `any`/`all`/`find_first` — deterministic first-match), the capability no user code can express. Predicate param types inferred from the array's element type; by-ref-param aware calls. Two latent codegen bugs found and fixed along the way: missing rayon prelude gating for body-generated flags, and `x.len() as i64 < n` parsing as Rust generics (any `len[x] lt n` comparison was broken compiled).
- [x] Spec reconciled: the "No Reduce, Any, All" and "No find/index" Design Notes replaced by "Search Builtins — a Reversal" recording the self-contradiction (flag loops triggered the exact trap the `break` ban warns about); the `break` note now points to the trio. `reduce` stays rejected. Parity `17_search.co`; 791 tests, 17 parity files green.

### 4. LLM adoption pack — ✅ DONE (July 2026): the final review item

The LLM-friendliness claim is half right: consistency helps, but models are trained on zero lines of Crow and will emit Python-isms (`+=`, f-strings, `"..."`, `lambda`, `()` calls).

**Fix:**
- [x] **`docs/crow-for-llms.md`** — a compact canonical cheatsheet designed to paste into system prompts: the bracket rule up top, a single dense cheat-sheet block covering every core construct, a 24-row "Reflexes to Unlearn" table (Python/JS form → Crow form), a "things that trip up generators" section (implicit return, parallel `for`-as-expression, no-lambda predicates, maps-vs-structs, errors-have-no-truth-value), and two complete idiomatic programs (word count + LiveView counter). Every code idiom in the doc was executed in both modes with matching output before shipping.
- [x] **Python-ism-aware errors** (lexer + parser, so both modes share them). 18 distinct reflexes now produce a message that teaches the Crow form instead of "unexpected character":
  - Lexer: `"..."` → single quotes; `+=`/`-=`/`*=`/`/=`/`%=` → write it out; `==`/`!=`/`<`/`>`/`<=`/`>=` → `eq`/`ne`/`lt`/`gt`/`lte`/`gte`; `&&`/`||`/`!` → `and`/`or`/`not`; `:` → indentation defines blocks; `f'...'`/`r'...'` string prefixes → backtick interpolation.
  - Parser (statement position): `def`, `lambda`, `class`, `match`, `elif`, `pass`, `import`/`from`, `print`, `async`/`await`, `try`/`except`/`catch`/`raise`/`throw` — each redirects to the Crow construct (with Design Note pointers). Guarded: a variable that merely shares a Python keyword's spelling (`from = 'alice'`, `print = 5`) still works — the hint only fires when the name isn't being assigned to.
  - Parser (expression position): `True`/`False` → lowercase; `None`/`null`/`nil`/`undefined` → no null, error-values + `.ok`.
  - 10 new unit tests (lexer + parser); 803 total.
- Found & fixed while validating the cheatsheet: **interpolated calls with named args never lowered** — `` '`fmt[x, decimals = 2]`' `` failed with "undefined function 'fmt'" in both modes because the interpolation re-parse skipped the lowering pass. Both modes now lower interpolated expressions; regression lines added to `16_fmt.co` parity file.

### 5. Syntax readability taxes

Deliberate re-evaluation, not automatic change (these are breaking):

- `button[[click = inc], '+']` — double brackets where meaning depends on nesting depth
- `--- inline comment ---` — requiring a closing delimiter inline is error-prone
- String accumulation without concatenation (`s = '{s}{line}'`) is ugly in loops

**Fix:** evaluate each; either change with migration via `crow fmt`, or write a Design Note explicitly accepting the cost.

- [x] **Attribute-map syntax: resolved via trailing named arguments.** `button['+', click = inc]` and `div[children..., class = 'x']` replace `button[[click = inc], '+']` — the lowering pass converts named args on ui elements into the attribute map internally, so ui codegen is unchanged. All examples, unit tests, and `ui-framework.md` migrated. `ui_demo.co` was found to be an unrunnable Phase 3–5 mockup (never parsed in any Crow version) and moved to `docs/ui-demo-sketch.co` with an honest header.
- [x] **Bare map literal removed** (July 2026). `[k = v]` is gone from the grammar — `map[key = value, ...]` (with quoted-string keys for special characters) is the one way to build maps, desugaring to `set[]` chains. Bare `[...]` always means an array; `name = value` in call brackets always means a named argument; `map[]` is now the ordinary zero-argument case rather than a special form. Old syntax gets a pointed error: "map literals were removed — write map[key = value, ...] instead". Design Note added to the spec; all docs/examples/tests migrated. (Found during migration: `examples/stdlib_test.co` and `examples/test.co` were already broken before this work — they use the removed `pow` builtin and `match` statement — stale examples worth a cleanup pass. The F1 sweep widened this: ~12 more examples fail to compile for pre-existing reasons — old `.status`/`.line` error-value fields, `math_zeros` referencing, moved-value codegen bugs in `binary_search`/`check_demo`, `operators_test` syntax. An examples-rot cleanup pass is overdue and would double as a compile-error-quality audit.)
- [x] **Comment syntax: comments end at the newline** (July 2026). The paired `---`...`---` design's unclosed-comment failure (silently swallowing the next line into code — two first-party incidents during this review) is now unrepresentable. Old trailing closers parse as inert text; `crow fmt` strips them; bare-fence block comments become per-line `---`. The last silent failure mode in the syntax is gone. Design Note in spec section 10.
- [x] String-building decision (resolved in the string-story arc: collect + join blessed, concat on strings, no `+`, teaching error message)

### 6. Interpreter–transpiler parity tax (architectural)

Two ~6,000-line implementations of every feature, forever; already produced the "transpiler is the source of truth, interpreter is internally dynamic" hedge.

- [x] **Architecture direction chosen (July 2026): shared semantics kernel, not a VM.**
  - Semantics migrate into `crow_runtime` as single-source functions; the interpreter delegates to them, the transpiler emits calls to them. The registry's `emit_rust`/`call` split collapses toward one implementation per builtin.
  - The transpiler keeps its **typed scalar fast path** (native Rust for statically-known scalar arithmetic, loops, control flow) — that's the benchmark story, and scalar semantics don't drift. Type inference becomes the fast-path/kernel router.
  - Rejected: compile-only (kills the REPL), interpret-only binaries (kills performance), bytecode VM + AOT (multi-year project; loses readable generated Rust).
  - Migration is opportunistic: whenever a fix or feature touches an operation, move it into the kernel. First tenant: `crow_runtime::display` (`CrowDisplay`), the shared value-formatting rules both modes now use.
- [x] **Parity harness: `crow test --parity`** runs every `tests/parity/*.co` in both modes and diffs stdout; any divergence is a failing exit. 12-file corpus covering scalars, strings, arrays, maps, control flow, for-expressions, functions, structs, error values, math, frames, and conversions. All 12 pass.

**What the harness caught on its first run** (7 of 12 files diverged; all fixed):

| Bug | Severity | Fix |
|---|---|---|
| `not (x eq 8)` compiled to Rust `!x == 8` (bitwise-not, wrong result) — every `not <comparison>` was silently wrong when compiled | correctness | parenthesize: `(!(...))` |
| Implicit returns (`fn add[a, b]` / `a + b`) — spec §3's headline feature — never worked in compiled mode | spec drift | parser now desugars last expression statement to `return` (one AST for both modes) |
| Interpreter's parallel `for` silently **discarded outer-variable mutations** (`total = total + x` left total at 0); spec §12 promises sequential fallback | correctness | interpreter now detects outer mutation and runs sequentially, like the transpiler |
| Compiled `find[]` never compiled at all (bool + array shoved into one typed BTreeMap) despite roadmap "P3 ✅" | spec drift | synthetic `CrowFindResult` struct, typed in inference, emitted on use |
| `e = eye[2]` — compiled mode resolved `e` to Euler's constant, interpreter to the variable | correctness | user variables now shadow module constants in codegen |
| `nums[0] = 99` on a numeric array failed to compile (CrowTensor not indexable) | gap | rebuild-with-element-replaced codegen |
| Numeric array literal in a for-expression emitted `CrowTensor` with no import | gap | fully-qualified paths |
| Display divergences: string arrays (`["a"]` vs `['a']`), 2-D tensors (`tensor(...)` vs flat), json values (compact vs spaced) | polish | shared display kernel; both modes now identical |

**Still open (structural, discovered by the harness):**

- [x] **Heterogeneous maps: resolved — "maps are dictionaries, structs are records"** (July 2026). Decision analysis: sorting map uses by keys-known-vs-dynamic × values-uniform-vs-mixed shows the true map uses (counts, caches, headers — dynamic keys) are naturally homogeneous, mixed-value uses have static keys (anonymous structs in disguise), and the one dynamic-and-mixed case is open JSON — already the `Json` type's job. Every typed language separates these; every dynamic one that didn't grew record types anyway (Python dataclasses, Ruby `Data.define` 2022, Elixir structs) or pays Julia's `Dict{String,Any}` performance cliff. Implementation: interpreter now enforces homogeneity at every insert (parity with the transpiler's typed maps — same friendly error in both modes: "map values must all be the same type… use a struct for mixed named data"); validator reports `set[]` type changes at compile time in Crow terms; int/float unify. Spec: new Design Note "Maps Are Dictionaries, Not Records", map-of-structs addresses example as the teaching pattern, Unified-Arrays and map-arithmetic notes corrected (they recommended heterogeneous maps). Parity corpus exercises map-of-structs with dynamic keys in both modes. No boxed value type was added — the gap closed by subtraction.
- [x] **No brace escape in strings** — resolved: interpolation is paired backticks, so a brace is plain text (`json['{"a": 1}']` is written as it reads; spec, String Interpolation).
- [x] **Lexer rejects non-ASCII** — resolved: comments and string literals accept any Unicode text; identifiers are ASCII (spec, Lexical structure).

## Fundamental audit (July 2026, after the maps decision)

A deliberate sweep for more maps-class issues — structural promises the implementation can't keep — each verified empirically in both modes.

### F1 — Functions cannot see module globals; the two modes disagree three ways (SEVERE)

```
count = 0
fn inc[]
    count = count + 1
inc[]
inc[]
log[count]      --- interpreter: 0 (!!) — silently creates a local, discards it
                --- compiled: error, undefined variable 'count'
```

Reads fail too: `fn add_base[x] x + base` → works interpreted, "undefined variable 'base'" compiled. The spec's scope section ("inner blocks can access outer variables") implies access; the interpreter silently produces wrong results on write; compiled mode rejects both read and write. `counter.co`'s `fn inc[] count = count + 1` only works because the UI transpiler rewrites globals into reactive state. **The language has no defined story for module-level state, and the interpreter's silent no-op write is the worst failure mode in the language.**

- [x] **Decision (July 2026, sharpened through design review with Vijay): full isolation — module variables are invisible inside functions.** Functions see parameters, locals, and the program's vocabulary (functions, structs, builtin constants) — nothing else. Stricter than the initial "reads allowed" proposal, and better: no shadowing rules needed (a local reusing a module name shadows nothing), the wave scheduler needs zero interprocedural analysis (an ambient read invisible to hazard detection is exactly what would make parallelism unsound), and relaxing later (e.g. single-assignment constants readable) stays backward-compatible while the reverse would not. Precedent: Ruby's method scope gate, Rust's static discipline, Erlang's no-shared-state.
- [x] Validator: module-variable reference inside a function errors with the teaching hint ("module variables are not visible inside functions — pass 'count' as a parameter"); `use ui` programs exempt (reactive-state transform is the sanctioned channel, documented in the spec).
- [x] Interpreter: global-scope lookups during calls now resolve only functions/structs/modules — the silent-local corruption is dead; reads error with the same hint; index/field-assign to module variables blocked. Locals may freely reuse module names (fresh bindings).
- [x] Spec: "Function Scope" section + Design Note "No Ambient Mutability" (closure-rejection consistency, scheduler soundness, language survey); §1 Scope clarified (blocks see outer, functions don't).
- [x] Parity file `14_fn_scope.co` + 7 unit tests. 761 tests, 14 parity files green.
- Known approximation (logged): type inference still uses one flat variable namespace, so a function-local reusing a module name with a *different type* can trip a spurious reassignment error — scoped inference is future kernel work.

### F2 — "Call-site specialization" does not exist; inference unifies one signature (SEVERE)

```
fn first[arr]
    return arr[0]
log[first[[1, 2, 3]]]     --- interpreter: 1
log[first[['a', 'b']]]    --- interpreter: a
                          --- compiled: type mismatch — expected CrowTensor but got array
```

Spec §14 claims "the transpiler infers concrete types from each call site, effectively specializing the function for the types used… like C++ templates or Julia" — the basis for "no generics needed." The implementation *unifies* all call sites into a single signature, so any helper used at two types fails to compile. Either implement real monomorphization (clone the function per distinct argument-type signature — the call-site collection machinery half-exists) or walk the claim back in the spec. Recommend monomorphization: it's the honest version of the documented design.

- [x] **Monomorphized** (July 2026). Inference collects distinct concrete call signatures per function (compatibility-grouped, fixpoint for transitive chains); functions with multiple signatures get per-signature clones (`first__1`, `first__2`) with independently inferred bodies and returns; call sites route to the matching clone; single-signature functions are completely untouched (zero risk to existing code). Specialization-aware callee return resolution + a settling pass handle transitive chains regardless of clone order. Documented v1 limits: multi-signature functions can't be passed as values (loud error), and generic *higher-order* functions specialize across predicates but not across element types. Spec §14 updated from claim to description. Parity `18_generics.co` (multi-type, transitive, recursive, structs); 793 tests, 18 parity files green.

### F3 — ✅ FIXED (July 2026): 2-D literals build real tensors

```
use math
m = [[1, 2], [3, 4]]
log[sum[m]]     --- interpreter: "sum: expected numarray, got Array(...)"
                --- compiled: no method `sum` on Vec<CrowTensor>
```

Nested numeric literals became an *array of 1-D tensors*, not a 2-D tensor — so reductions, matmul, transpose failed on the spec's own literal form. **Fixed:**

- [x] Nested all-numeric literals → single shaped CrowTensor in both modes (rank tracked in inference as nested `NumArray` inner types); ragged rows error ("all inner arrays must be the same length"); `m[0]` is a row tensor, `for row in m` iterates rows, negative row indexing works.
- [x] Comparison broadcasting on tensors: `x gt 2` → boolean mask in both modes, composing with the existing `and`/`or`/`not` mask machinery (retires the NumArray-asymmetry item in #2).
- [x] Growth contract: `push[m, row]` appends along axis 0 with strict shape/type errors (no NumPy flatten-guessing, no MATLAB auto-grow, no R recycling, no promotion — survey in the spec's multidimensional-arrays section); tensor mutation stays immutable-rebuild (the JAX model).
- [x] **`append` removed; `concat` is core** — one merge operation for all arrays and tensors (`concat[a, b, axis?]`), replacing the core/math `append`/`concat` duplication Vijay spotted. `stack` accepts tensors (array-of-tensor literals now auto-build one).
- [x] Spec: growth/mask examples, "Tensors vs frames" boundary paragraph, and the stale "a DataFrame is a map of arrays" line replaced with a pointer to `frame`. Parity file `15_tensors.co`; 767 tests, 15 parity files green.
- Example-rot addendum: `numarray_test.co` fails at HEAD too (`a * b` on 1-D tensors predates `*`-is-matmul) — added to the cleanup-pass list.

### F4 / #2 batch — ✅ FIXED (July 2026): the five-item semantics pass

- [x] **Errors have no truth value** (design sharpened by Vijay: "an error cannot be compared with a value" — errors-are-falsy rejected as silent misclassification). Comparisons propagate; a branch on an error is refused: in-function → the function returns the error (trace appended); top level → halt with reason. `.ok`/metadata never propagate — the guard idiom is the one bridge. Both modes agree; fn return types widen to Result when conditions are fallible. Spec section + Design Note "Errors Have No Truth Value" (Odin/Go/Rust/SQL survey informed).
- [x] **int/float**: a variable ever assigned a float is float for its whole scope — compiled declares from the unified type and coerces; interpreter coerces from the first float assignment on (residual interp-only edge documented). This also fixed a fresh discovery: compiled mode previously *rejected* int-then-float reassignment outright, despite the spec calling it compatible.
- [x] **`\{` `\}` escapes**: JSON object literals are finally writable (`json['\{"a": 1\}']`); fixing this surfaced and closed another gap — Json *field access* (`j.a`) had never been implemented in compiled mode. Both modes agree.
- [x] **Parallel `log[]` ordering**: documented as unspecified in §12 with the for-as-expression alternative.
- [x] **Non-ASCII: re-diagnosed.** Comments and strings already accept any Unicode; the audit's em-dash failure was actually the `---` comment-pairing footgun (unclosed inline comment leaking into code) — more ammunition for the pushback-#5 comment-syntax decision. Spec now states the rule (Unicode in comments/strings, ASCII identifiers).

776 tests, 15 parity files green. Corpus extended: error propagation through conditions (09), JSON objects + brace escapes (12), float unification (01).

Minor tensions noted, not blocking: the Unified-Arrays note's "a DataFrame is a map of arrays" now conflicts with `Frame` being its own type (two ways); `select` is exported by both `ui` and `frame` and the conflict rules make `dashboard.co`'s bare `select[...]` ambiguous in principle.

### String story — resolved by design (July 2026)

The syntax re-review identified strings as the weakest remaining area. Resolved in two parts:

- [x] **`fmt` builtin** — value formatting via named arguments (`decimals`, `commas`, `width`, `align`, `fill`, `percent`), implemented once in the display kernel and used by both modes; `pad` absorbed (`fmt[s, width = n]`); named-formatter idiom (`money[x]`) blessed as the per-project answer to Python's `{:,.2f}` compactness. Design Note "No Format Mini-Language". Parity file `16_fmt.co`; 780 tests, 16 parity files green. Known v1 gap: no locale (separators hardcoded); extensible via future named args.
- [x] **Backtick interpolation migration — IMPLEMENTED** (July 2026). Details as decided below; additionally fixed during implementation: the parallel-wave scheduler and call-site type inference both had brace-scanning string readers that would have missed backtick interpolation reads (the F1 hidden-dependency class — caught by tests before shipping); nested quotes inside interpolation follow the you-are-still-in-a-string rule (escape with \'). `crow fmt --fix-interpolation` migrates user code. 785 tests, 16 parity files green; counter + dashboard build. Original decision record: `` `expr` `` (paired backticks, markdown-style) replaces `{expr}`; strings otherwise inert (JSON/regex/prose raw, deletes the `\{` escapes and today's silent `{4}` regex-quantifier corruption); `` \` `` escapes a literal backtick; string building = collect + `join` (+ `concat` extended to strings); no `+`. Full design lineage recorded through the sessions: `{}` → `\[expr]` → bare `` `name `` → bare `` `path `` → paired `` `expr` `` (each step user-driven; the paired form restores full expressions with zero special rules). Implementation: lexer/interpolators both modes, `crow fmt` auto-migration, examples/docs/corpus sweep, `+`-error message teaching collect+join.

### Examples cleanup — ✅ DONE (July 2026): every example builds or is marked by design

The overdue rot pass. End state: **every `examples/*.co` compiles** except `mymath.co`, which is a module imported by `module_test.co` (not a standalone program — header note says so). `check_demo.co` (deliberate `crow check` fodder) and `p4_web_test.co` (interpreter-only Phase-4 stub on the removed `live[]`/app-passing API) were **deleted** rather than kept as excluded files. 793 tests, **20 parity files** green.

Real compiler/inference bugs found and fixed by the sweep (each one a user-facing class, not just example rot):

- [x] **Moved-value class**: non-Copy variable args (tensors/arrays/maps/strings/frames/structs) to user fns are cloned at the call site — `binary_search.co`'s second call no longer hits Rust's move checker. Value semantics match the interpreter.
- [x] **`shell`/`elapsed` returned heterogeneous maps** (illegal under maps-are-dictionaries, and uncompilable). Now synthetic structs `CrowShell{status, stdout, stderr}` / `CrowElapsed{seconds, minutes, hours, days}` (CrowFindResult pattern); interpreter `elapsed.seconds` widened Int→Float for value parity.
- [x] **Recursive-struct field access** (`node.left.hash` through `Option<Box<T>>`): OptBoxStruct field reads unwrap the box in both inference and codegen ("field access on an empty (0) node" on the sentinel) — `bench_merkletrees` compiles.
- [x] **`db` compiled support was hollow**: `use db` now loads the registry for inference/validation, `connect` gets a real `DbHandle` type (new CrowType variants `DbHandle`/`DbTx`), mixed-type SQL bind params (`['alice', 30]`) emit element-wise `json!` values, `transact[conn, fn]` types its callback param as a transaction handle and passes `&dyn DbTx` straight through. `db.query[...]` module-prefix routing works (registry-prefix fix extended to db in inference + validator; transpiler keeps its dedicated branch). `db_demo.co` builds *and runs* against in-memory SQLite.
- [x] **For-expression inference didn't bind the loop variable** — `for f in [double, square] → f[10]` inferred the element type (the fn!) instead of the call result; the elem-type fallback is gone entirely, and a trailing `if` (the filter pattern) now infers through its branches.
- [x] **Uncalled fns referenced as values kept Unknown params and were skipped from emission.** Indirect-call typing extended: fns iterated in a for-expression get params from calls through the loop var; `propagate_indirect_fn_types` now sees through maps of fns (`ops = map[add = add]; r = ops.add; r[3, 4]` types `add`).
- [x] **Map dot-access had no inference rule** (`m.key` → Unknown → String). Now returns the map's value type.
- [x] **Display-kernel gaps**: maps and simple structs now print through `crow_runtime::display` in compiled mode (log + interpolation), matching the interpreter exactly — structs get a generated `CrowDisplay` impl (`Point[x = 3, y = 4]`, fields alphabetical to match the interpreter's BTreeMap). Synthetic/recursive structs keep Debug.
- [x] **`map[k = fn]` values cast to fn-pointer type** in the set[]-chain emission (distinct Rust fn items now unify).
- [x] **Fn-type `compatible`** recurses through params/returns (Unknown params are wildcards) — kills a spurious "variable type cannot change" on fn-valued vars.
- [x] **No-app `route[path, handler]` / `route[path, method, handler]`** — the docs promised it since the no-app API commit but only the legacy app-first form existed. Plain `use web` programs get the implicit `__crow_app` (new prescan). `web_test.co` migrated off `web.new[]`.
- [x] **User-module imports compiled**: `mymath.pi` (module *variable* access) validates via the prefixed name; `use myapp/utils` computes the module prefix correctly for slash paths. `module_test`/`module_test2` build with full parity.
- [x] `now`/`date` typed as `map[int]` in inference (were Unknown→String).

Example migrations (rot, not bugs): `pow` → `^` (stdlib_test), `match` → if/else (test.co), `.status` → `.ok`, elementwise `a / b` → `a ./ b` and `a * b` → `a .* b` (numarray_test), missing `use math` (sum_test), `final` renamed (Rust keyword — logged: transpiler should escape Rust reserved words), flat-namespace param renames (first_class_test `reduce[g, ...]`, math_test `spec`/`z`) — the flat type-inference namespace remains the documented approximation.

New parity corpus: `19_fn_values.co` (fn values in arrays/for-exprs/maps), `20_display.co` (map/struct/empty-map display). `test.co` stays out of the corpus (parallel-for log order is documented-unspecified).

## Not pushbacks, but the adoption gap (post-fix priorities)

1. Editor support (LSP + syntax highlighting) — adoption blocker #1
2. One real dogfood app (the trading dashboard, live, against a real DB)
3. Deepen `crow check`
4. Ecosystem/registry last — packages follow users

## The 1.0 namespace round — DECIDED and implemented (July 2026)

Post-campaign design session triggered by the question "can we call it 1.0?" — the last open
semantic (builtin shadowing) plus everything it pulled in. All rulings are Vijay's; the
discussion record is summarized here so the implementation has one source of truth.

### The problem

Shadowing was never decided — it was an accident of implementation order, and inconsistent:
call position let builtins silently beat user variables AND user functions (`range = [10,20,30];
range[0]` → `[]` silently; `fn sum` under `use math` silently ignored), while value position let
variables beat constants. Crow's unified `[]` (calls = indexing) makes shadowing uniquely
dangerous — MATLAB, the only mainstream language with the same collision, chose shadowing and it
became a 40-year footgun. Our own cleanup month (ls, e = eye[2], fn sum) was the local evidence.

### The rulings

1. **Strict scoped reservation.** A bare name means exactly one thing. Any user definition
   (variable, function, struct, param) colliding with a core builtin or an imported module's
   export is a compile/validation error at the definition line, with rename suggestions.
   Identical in both modes. Constants included: `use math` reserves `pi`, `e`, `inf`, `nan`
   (reverses the vars-shadow-constants patch from the examples-cleanup week).
2. **Three namespace tiers.** Core: bare, reserved always, frozen by the budget rule. Stdlib
   modules (`use math` etc.): bare + reserved in importing files only. Everything else (user
   modules today, registry packages tomorrow): qualified-only — the import reserves exactly one
   name, the prefix. "The language owns the bare namespace; packages get prefixes."
   (User modules are already qualified-only in the implementation — tier 3 codifies it.)
3. **`as` aliasing, universal, Python-semantics.** `use X as y` works for any module; an
   aliased import is qualified-only under its alias (exactly `import pandas as pd` — no bare
   names). For stdlib this doubles as the opt-out: `use math as m` frees `sum` for your
   variables. Contextual keyword (only special after a use-path) — zero reserved-word growth.
   Fixes the otherwise-impossible case of two packages with the same trailing name.
4. **`view` blocks — the ui zone.** New declaration keyword, sibling of `fn`. Inside a view,
   the standard HTML vocabulary (current 31 elements, exact HTML names — kept for learnability)
   is active; outside views the names don't exist, so `use ui` reserves NOTHING — `p = 0.95`,
   `fn add[a, b]`, `form = req.body` are fine everywhere. Replaces the transpiler's heuristic
   render-fn detection with a declarative marker. Precedent: the JSX/SwiftUI/Phoenix boundary;
   like JSX, the lowercase vocabulary is fixed — components are module-exported views called as
   `prefix.component`. Reactive-state access narrows from "any fn under use ui" to views +
   event handlers (tightens F1: `fn` is now ALWAYS fully isolated). Migration `fn render` →
   `view render` via crow fmt, accepted pre-1.0.
5. **No user-defined zones.** Vocabulary zones are language-owned and budget-priced, same
   principle as bare names. Modules participate by exporting functions (qualified) or views
   (components). A future external-fixed-vocabulary case (e.g. SVG) extends the view
   vocabulary rather than adding a zone.
6. **Target-independence pre-commitment.** "The view tree is target-independent; platforms
   (desktop/mobile) are build targets, never language dialects." One zone, one tree model;
   the natural desktop path is a Tauri-style wry shell around the existing compiled server
   (Crow already emits Rust), where LiveView latency goes to zero in-process.
7. **Non-view trees need no mechanism**: recursive structs (data), builder modules with
   named args (scene-graph style, `use scene as s` → `s.group[s.mesh['tree', x = 10]]`),
   `json` (open data).
8. **Deferred**: whether 1.0 scopes to core-language with web/ui marked preview — revisit
   after the dogfood app (which is confirmed as a 1.0 gate).

### Bugs found while stress-testing the design (fix before/with implementation)

- **Struct-args parity break (core, worst)**: `pair[leaf[1], leaf[2]]` — struct-returning
  calls as struct arguments fail compiled validation with "expected N but got N" (interp runs
  fine). Likely the Int+Struct→OptBoxStruct sentinel unify misfiring; message must print
  wrapper types regardless.
- **Named args on qualified calls don't lower** (`scene.mesh['tree', x = 10]`) — documented
  v1 limit, now blocking the tier-3 builder pattern.
- **Import expander doesn't rewrite intra-module references** — a module fn can't construct
  the module's own struct (renamed out from under it). User modules only work flat.

### Implementation — ✅ ALL FIVE ITEMS SHIPPED (July 2026)

1. [x] **Struct-arg parity break** — by-ref struct params cloned in owned contexts
   (array literals, struct fields, returns, assignments; `owned_expr_str`); plus the
   root of the "expected N but got N" repro turned out to double: struct fields named
   `ok`/`reason`/`kind`/`trace` were silently shadowed by error metadata in one mode —
   both modes now reject them at declaration (first enforcement of this round).
   Parity `21_struct_trees`.
2. [x] **User modules real**: expander rewrites intra-module references (module fns can
   use their own structs/helpers); lowerer resolves named args on qualified calls and
   canonicalizes to the prefixed name; interpreter file path runs the same
   expand→lower→validate pipeline as the compiled path. Parity `22_modules` (+ lib/ module).
3. [x] **`view` blocks**: keyword + is_view through the whole pipeline; element
   vocabulary active only inside views (`use ui` reserves nothing — `p = 0.95`,
   `fn add[a, b]` legal in ui files); elements outside views teach; reactive-state
   exemption narrowed to views + handlers in validator AND per-call-frame in the
   interpreter; `crow fmt --fix-views` migrates; counter/dashboard/scaffold/fixtures
   migrated.
4. [x] **Reservation + `as`**: CORE_RESERVED split from the dispatch list (web/db names
   are not core — `render` deliberately freed for views); bare stdlib imports reserve
   exports + constants per file with registry attribution; prefixes/aliases reserved;
   `use X as y` = qualified-only (Python semantics), contextual keyword, alias-aware
   routing in all four resolution sites; bare calls to aliased modules error with the
   qualified form. Both modes reject identical programs (interp now validates).
   Fallout renames in examples were themselves reservation catches (`today`, `sub`,
   `e`, `qr`, `input`). Parity `23_aliases`.
5. [x] **Docs**: spec — View Blocks section, Aliased Imports section, narrowed
   Function Scope exception, Design Note "Names Are Never Ambiguous" (tiers, zones,
   fields exemption, the MATLAB argument, target-independence); crow-for-llms —
   view example, reserved-names section, import-as row.

End state: 803 tests, 23 parity files, every example builds, both modes share one
enforcement pipeline. Deferred as agreed: 1.0 scoping decision (after dogfood app);
`crow fmt --fix-collisions` (reserve for the first stdlib addition that needs it).
