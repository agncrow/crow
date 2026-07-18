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
4. **Vertical integration** — `examples/dashboard.crow` (a reactive trading dashboard in ~50 lines, no npm) is the product.
5. **`crow check`** — the most differentiated feature; currently the least developed relative to its potential.

## Pushbacks and Fix Plan

Ordered by "gets more expensive to fix every day" first.

### 1. Frame API breaks syntax uniformity (fix first)

`sort[filtered, pnl, desc]` and `group[today, playbook_code, [pnl, sum]]` use bare unquoted identifiers (`pnl`, `desc`) that are not variables in scope — a special evaluation form, i.e. a DSL. This violates design rule 19 ("every line must be valid Crow, no DSL"). `docs/frame-module.md` even shows `>` and `==` operators that don't exist in Crow. UI Phase 3 (`table[]`, routing) builds directly on the frame API, so this decision compounds daily.

**Fix:** decide one of: (a) column references become real, specified syntax with defined semantics; (b) columns are strings (`sort[df, 'pnl', 'desc']`); (c) columns are expressions over `df.col` returning boolean/sort-key arrays (already how `filter[df, df.age gt 50]` is specced). Update spec, frame-module doc, both implementations, and all examples together.

- [x] **Decision: strings for names + expressions for computation** (b + c). Specced in `language-spec.md` ("Built-in Module: frame" section — previously absent from the spec entirely) with a Design Note documenting the removal.
- [x] Bare-name evaluation layer removed from validator, interpreter, and transpiler. What it was hiding: (1) the interpreter preferred an in-scope variable while the transpiler force-converted the same argument to a string — a live parity violation; (2) `dashboard.crow` had a latent capture bug (`sort[filtered, pnl, desc]` where `pnl` was a local variable); (3) dynamic column names (`sort[df, col_var]`) were impossible in compiled mode — they work now.
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

- [ ] Error-in-conditional specced + tested
- [ ] int/float reassignment specced + tested
- [ ] Parallel `log[]` ordering specced (define or document nondeterminism prominently)
- [x] **Named arguments: implemented** (July 2026). Parser accepts defaults in `fn` params (literals only) and `name = expr` at call sites; a single lowering pass (`src/lower.rs`) rewrites every call to positional form before inference/validation/execution — one implementation for both modes (kernel principle). Covers user functions (defaults filled), structs, builtins with named params (`axis` on reductions, `desc` on sort — reorder only, no filling), the variadic `map[...]` constructor (desugars to a `set[]` chain), and ui elements (named args become the attribute map). Limitation (documented): named args need a directly named callee — function-as-value calls are positional-only. `sort[df, 'col', desc = true]` migration done; parity file `13_named_args.crow` locks both modes.
- [ ] NumArray comparison broadcasting decision
- [ ] String display parity between modes

### 3. The early-exit gap (the self-inconsistency)

Crow rejects `break` in `for` because silent sequentialization is "a hidden performance trap" — but the recommended replacement (mutable-flag loops for any/all/find) triggers exactly that trap: mutating an outer variable silently forces the loop sequential, *and* it can't exit early (always O(n)), *and* it's more code.

**Fix:** either bless a small set of search/reduction builtins (`find`, `any`, `all` — loops the compiler can parallelize with early exit, which users cannot write themselves; same justification as `contains`), or allow `break` in `for` and retire the rationale. Update the three related Design Notes coherently.

- [ ] Decision made (builtins vs `break`) with Design Note
- [ ] Implemented in interpreter + transpiler with parallel early-exit
- [ ] Spec Design Notes reconciled

### 4. LLM adoption pack

The LLM-friendliness claim is half right: consistency helps, but models are trained on zero lines of Crow and will emit Python-isms (`+=`, f-strings, `"..."`, `lambda`, `()` calls).

**Fix:**
- [ ] A compact canonical cheatsheet (`docs/crow-for-llms.md`) designed to paste into system prompts
- [ ] Transpiler/interpreter errors that recognize Python-isms: "Crow has no `+=` — write `x = x + 1`", double quotes → single quotes, `def`/`lambda` → `fn`, `(...)` call → `[...]`

### 5. Syntax readability taxes

Deliberate re-evaluation, not automatic change (these are breaking):

- `button[[click = inc], '+']` — double brackets where meaning depends on nesting depth
- `--- inline comment ---` — requiring a closing delimiter inline is error-prone
- String accumulation without concatenation (`s = '{s}{line}'`) is ugly in loops

**Fix:** evaluate each; either change with migration via `crow fmt`, or write a Design Note explicitly accepting the cost.

- [x] **Attribute-map syntax: resolved via trailing named arguments.** `button['+', click = inc]` and `div[children..., class = 'x']` replace `button[[click = inc], '+']` — the lowering pass converts named args on ui elements into the attribute map internally, so ui codegen is unchanged. All examples, unit tests, and `ui-framework.md` migrated. `ui_demo.crow` was found to be an unrunnable Phase 3–5 mockup (never parsed in any Crow version) and moved to `docs/ui-demo-sketch.crow` with an honest header.
- [x] **Bare map literal removed** (July 2026). `[k = v]` is gone from the grammar — `map[key = value, ...]` (with quoted-string keys for special characters) is the one way to build maps, desugaring to `set[]` chains. Bare `[...]` always means an array; `name = value` in call brackets always means a named argument; `map[]` is now the ordinary zero-argument case rather than a special form. Old syntax gets a pointed error: "map literals were removed — write map[key = value, ...] instead". Design Note added to the spec; all docs/examples/tests migrated. (Found during migration: `examples/stdlib_test.crow` and `examples/test.crow` were already broken before this work — they use the removed `pow` builtin and `match` statement — stale examples worth a cleanup pass. The F1 sweep widened this: ~12 more examples fail to compile for pre-existing reasons — old `.status`/`.line` error-value fields, `math_zeros` referencing, moved-value codegen bugs in `binary_search`/`check_demo`, `operators_test` syntax. An examples-rot cleanup pass is overdue and would double as a compile-error-quality audit.)
- [ ] Comment syntax decision
- [ ] String-building decision

### 6. Interpreter–transpiler parity tax (architectural)

Two ~6,000-line implementations of every feature, forever; already produced the "transpiler is the source of truth, interpreter is internally dynamic" hedge.

- [x] **Architecture direction chosen (July 2026): shared semantics kernel, not a VM.**
  - Semantics migrate into `crow_runtime` as single-source functions; the interpreter delegates to them, the transpiler emits calls to them. The registry's `emit_rust`/`call` split collapses toward one implementation per builtin.
  - The transpiler keeps its **typed scalar fast path** (native Rust for statically-known scalar arithmetic, loops, control flow) — that's the benchmark story, and scalar semantics don't drift. Type inference becomes the fast-path/kernel router.
  - Rejected: compile-only (kills the REPL), interpret-only binaries (kills performance), bytecode VM + AOT (multi-year project; loses readable generated Rust).
  - Migration is opportunistic: whenever a fix or feature touches an operation, move it into the kernel. First tenant: `crow_runtime::display` (`CrowDisplay`), the shared value-formatting rules both modes now use.
- [x] **Parity harness: `crow test --parity`** runs every `tests/parity/*.crow` in both modes and diffs stdout; any divergence is a failing exit. 12-file corpus covering scalars, strings, arrays, maps, control flow, for-expressions, functions, structs, error values, math, frames, and conversions. All 12 pass.

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
- [ ] **No brace escape in strings** — `{` always starts interpolation, so JSON object literals (`json['{"a": 1}']`) are unwritable. Needs `\{` or equivalent.
- [ ] **Lexer rejects non-ASCII** (e.g. an em-dash in a comment is a syntax error). Decide and document intended behavior.

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

Reads fail too: `fn add_base[x] x + base` → works interpreted, "undefined variable 'base'" compiled. The spec's scope section ("inner blocks can access outer variables") implies access; the interpreter silently produces wrong results on write; compiled mode rejects both read and write. `counter.crow`'s `fn inc[] count = count + 1` only works because the UI transpiler rewrites globals into reactive state. **The language has no defined story for module-level state, and the interpreter's silent no-op write is the worst failure mode in the language.**

- [x] **Decision (July 2026, sharpened through design review with Vijay): full isolation — module variables are invisible inside functions.** Functions see parameters, locals, and the program's vocabulary (functions, structs, builtin constants) — nothing else. Stricter than the initial "reads allowed" proposal, and better: no shadowing rules needed (a local reusing a module name shadows nothing), the wave scheduler needs zero interprocedural analysis (an ambient read invisible to hazard detection is exactly what would make parallelism unsound), and relaxing later (e.g. single-assignment constants readable) stays backward-compatible while the reverse would not. Precedent: Ruby's method scope gate, Rust's static discipline, Erlang's no-shared-state.
- [x] Validator: module-variable reference inside a function errors with the teaching hint ("module variables are not visible inside functions — pass 'count' as a parameter"); `use ui` programs exempt (reactive-state transform is the sanctioned channel, documented in the spec).
- [x] Interpreter: global-scope lookups during calls now resolve only functions/structs/modules — the silent-local corruption is dead; reads error with the same hint; index/field-assign to module variables blocked. Locals may freely reuse module names (fresh bindings).
- [x] Spec: "Function Scope" section + Design Note "No Ambient Mutability" (closure-rejection consistency, scheduler soundness, language survey); §1 Scope clarified (blocks see outer, functions don't).
- [x] Parity file `14_fn_scope.crow` + 7 unit tests. 761 tests, 14 parity files green.
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

- [ ] Monomorphize user functions per call-site signature in the transpiler (parity file: generic helper at int/str/array types)

### F3 — "Unified arrays": 2-D literals are not tensors (MAJOR)

```
use math
m = [[1, 2], [3, 4]]
log[sum[m]]     --- interpreter: "sum: expected numarray, got Array(...)"
                --- compiled: no method `sum` on Vec<CrowTensor>
```

Nested numeric literals become an *array of 1-D tensors*, not a 2-D tensor — so reductions, matmul, transpose, and every math-module function fail on the spec's own "2x3 array" literal form; users must launder through `reshape`. Related seam: `[1, 2, 3] gt 2` fails in both modes while frame columns broadcast comparisons. The "internally tensors, transparently" claim needs to be made true: all-numeric nested literals should build one shaped tensor, and tensors need comparison broadcasting.

- [ ] Nested all-numeric literals → single shaped CrowTensor (both modes)
- [ ] Comparison broadcasting on tensors (subsumes the NumArray item in #2)

### F4 — Error-values in conditionals: modes disagree (confirmed, already queued in #2)

`if err_value gt 5` runs interpreted (falsy) but is a compile error ("expected bool but got result"). One semantics must win; recommend: comparisons auto-unwrap like arithmetic does, errors propagate.

Minor tensions noted, not blocking: the Unified-Arrays note's "a DataFrame is a map of arrays" now conflicts with `Frame` being its own type (two ways); `select` is exported by both `ui` and `frame` and the conflict rules make `dashboard.crow`'s bare `select[...]` ambiguous in principle.

## Not pushbacks, but the adoption gap (post-fix priorities)

1. Editor support (LSP + syntax highlighting) — adoption blocker #1
2. One real dogfood app (the trading dashboard, live, against a real DB)
3. Deepen `crow check`
4. Ecosystem/registry last — packages follow users
