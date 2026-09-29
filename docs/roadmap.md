# Crow Roadmap

What's done, what's remaining, and in what order.

## Status: What's Done ✅

| Area | Details |
|------|---------|
| Core language | Lexer, parser, AST, IR, IR interpreter, Rust backend from the IR, checker, formatter, REPL |
| Type system | Dynamic with inference — int, float, str, bool, array, map, tensor, frame, function, error |
| Modules | `math`, `math.stats`, `math.nn`, `io`, `frame`, `web` (17 sub-modules), `db` (SQLite, PostgreSQL, ClickHouse) |
| Math (~107 functions) | Creation (9), element-wise (12), trig (7), hyperbolic (3), reductions (9), sorting/selection (6), shape (3), linalg (15), random (12), signal (12), special (5), complex (6), sparse (3) |
| Statistics (13) | median, std, var, percentile, zscore, corr, cov, ttest, linreg, chisq, anova, kstest, fit |
| Neural networks (8) | relu, sigmoid, softmax, gelu, mse, crossentropy, bce, linear |
| DataFrames (16+) | load, save, select, filter, sort, group, join, stack, head, tail, fill, unique, rename, stats, datetime, tz |
| General-purpose (P3) | regex (find, matches, sub), moments (now, moment, calendar, parse_time — a moment is a number), freshness (captured), shell, hashing (sha256/sha512/md5) |
| Web framework (P4) | 17 modules: server, client, db, auth, cors, session, template, cache, rate, upload, sse, ws, live, static, mail, queue, docs — all with transpiler codegen + interpreter support |
| Autograd | `grad[f]` functional transformation |
| Parallel execution | Auto-parallel `for` loops (rayon), statement-level parallelism via dependency analysis |
| CLI | `crow run`, `crow build`, `crow stop`, `crow fmt`, `crow check` (+ `--db`, `--strict`), `crow fix`, `crow db diff`, `crow test` (blocks, `--parity`, `--book`), `crow lsp`, `crow new`, `crow install`, `crow launch`, REPL |
| Verification | `test` blocks (attached examples/laws, named scenarios, generated inputs, `app[]` harness for views); `crow check` hazards for the live loop (blocking, stale snapshots) and, with a schema, the concurrency catalog (write skew, lost update, latch, assumed-unique, unpaired writes) |
| Live apps | `view` blocks, reactive state, named-argument attributes, deferred event calls, `later[]`/`running[]`, `serve[port, refresh =, budget =]`, default stylesheet (8 classes + 8 tokens, `css/app.css` hatch), `chart[]` element |
| Book runner | `crow book`: the book served by the compiler with every block runnable in place (interpret / compile / test / stop) |
| Editor | `crow lsp` (diagnostics, completion, hover, go-to-definition, signature help) + VS Code extension in `editors/vscode` |
| Project structure | `crow new` scaffolding (--web, --ml, --db), `crow run <job>`, convention-based views/jobs/layout/db, dot-path imports (`use views.home`), multi-import (`use web, ui`) |
| DX | Default welcome page for empty web apps, friendly error messages (e.g. "port 3000 is already in use") |
| Tests | cargo unit tests, 128-file parity corpus (`crow test --parity`), the served pages in both modes (`crow test --served`), every book block in both modes (`crow test --book`), docs-vs-compiler coverage (`crow test --docs`), db-check fixtures in `tests/db_check`, chart harness; CI runs all of them (`.github/workflows/ci.yml`) |
| Documentation | The Crow Book (`docs/book`, 13 chapters + 5 appendices), language-spec.md, crow-for-llms.md, usage.md, crow-check.md, crow-lsp.md, module references, design records — indexed in `docs/README.md` |

## Remaining Work

> **Design review fixes:** [docs/design-review.md](design-review.md) tracks the July 2026 review pushbacks (frame DSL uniformity, semantics gaps, early-exit, LLM pack, syntax taxes, parity architecture). Being fixed in that order.

### P0 — Quick fixes (hours) ✅

- [x] **Fix flaky test**: Combined 4 seed-dependent interpreter tests into one `test_math_random_all` to avoid global RNG state races (same pattern as `distributed.rs::test_distributed_all`). Added `clear_seed()` cleanup.
- [x] **Spec cleanup**: Updated Design Note "No sum, min[arr], max[arr] in Core" — replaced stale "A future math library will provide..." with reference to `use math` reductions.

### P1 — Math/Stats completeness ✅

- [x] **`rank`**: Matrix rank via SVD (counts singular values above tolerance).
- [x] **`cond`**: Condition number (ratio of largest to smallest singular value).
- [x] **Chi-square test**: `chisq[observed, expected]` → `{chi2, p}`.
- [x] **ANOVA**: `anova[group1, group2, ...]` → `{f, p}`.
- [x] **KS test**: `kstest[data, 'normal']` → `{d, p}`.
- [x] **Distribution fitting**: `fit[data, 'normal']` → `{mu, sigma}` (or appropriate params for each distribution).

### P2 — Signal processing depth ✅

- [x] **STFT**: Short-time Fourier transform — `stft[signal, window_size, hop_size]` → [real, imag] 2D tensors.
- [x] **Spectrogram**: `spectrogram[signal, window_size, hop_size]` → 2D power tensor.
- [x] **Filter design**: `lowpass[x, cutoff, sr]`, `highpass[x, cutoff, sr]`, `bandpass[x, low, high, sr]` — FFT-based frequency-domain filters.

### P3 — General-purpose language ✅

Crow now has regex, date/time, shell, and hashing builtins in addition to file system and HTTP client.

#### File system ✅
- [x] **`exists[path]`** — check if file or directory exists (returns bool).
- [x] **`mkdir[path]`** — create directory (and parents). Error-value on failure.
- [x] **`ls[path]`** — list directory entries as sorted array of strings. Error-value on failure.
- [x] **`rm[path]`** — remove file or directory (recursive). Error-value on failure.
- [x] **`mv[src, dst]`** — move/rename file or directory. Error-value on failure.
- [x] **`cp[src, dst]`** — copy file. Error-value on failure.

#### Networking / HTTP client ✅
Provided by `web.client` module (`use web.client`): `get[url]`, `post[url, body]`, `put[url, body]`, `del[url]` — all return `{status, body, headers, ok, json}`. No separate `fetch` builtin needed (Design Note: one way to do things).

#### Regex ✅
- [x] **`find[s, pattern]`** — regex match → `{ok, groups}` map.
- [x] **`matches[s, pattern]`** — does string match pattern? (returns bool).
- [x] **`sub[s, pattern, replacement]`** — regex substitution.

#### Date/time ✅
- [x] **`now[]`** — current datetime map with `year`, `month`, `day`, `hour`, `minute`, `second`, `timestamp`.
- [x] **`date[y, m, d]`** — construct date map with `year`, `month`, `day`, `timestamp`.
- [x] **`elapsed[start, end]`** — duration between two date maps → `{seconds, minutes, hours, days}`.

#### Process / shell ✅
- [x] **`shell[cmd]`** — run shell command → `{status, stdout, stderr}`. stdout/stderr are trimmed.

#### Hashing ✅
- [x] **`hash[s, 'sha256']`** — compute hash digest → hex string. Supports sha256, sha512, md5.

### P4 — Fullstack / web ✅

> **See [docs/web-framework.md](web-framework.md) for the full design document.**

#### Rust crate (`crow_web`) — ✅ Done
All 17 server-side modules implemented with 119 tests passing:
- [x] `web` (server), `web.client`, `web.ws`, `web.sse`, `web.cors`, `web.static`, `web.upload`, `web.rate`
- [x] `web.auth`, `web.session`, `web.template`, `web.db`, `web.mail`, `web.cache`, `web.queue`
- [x] `web.docs` (OpenAPI + Swagger UI), `web.live` (LCS diff + WS push), `web.view` (VNode diff + JS module gen)
- [x] Transpiler wiring — `use web.X` → feature flags in generated Cargo.toml

#### Transpiler codegen — ✅ Done
- [x] **Web builtins codegen** — transpiler emits Rust code for all web builtins (cors, sessions, templates, render, cache, limit, uploads, sse, ws, connect, send, recv, close, live, static_dir, smtp, web.queue, web.docs).
- [x] **Interpreter support** — all `web.*` builtins work in `crow run -i` mode with mock implementations.
- [x] **Unit tests** — 21 transpiler tests + 12 interpreter tests for P4 codegen.

#### Response formats ✅
- [x] **Binary body support** — `CrowResponse` extended with `binary_body: Option<Vec<u8>>` for binary responses; Axum converter prioritizes binary body over string body.
- [x] **`msgpack[value]`** — MessagePack binary response via `rmp-serde`. Transpiler emits `CrowResponse::msgpack(rmp_serde::to_vec(...))`. Feature-gated (`msgpack` feature in crow_web).
- [x] **`arrow[frame]`** — Arrow IPC response for DataFrames. `crow_runtime::frame::to_arrow_ipc()` converts `CrowFrame` → Arrow `RecordBatch` → IPC stream bytes. Feature-gated (`arrow` feature in crow_runtime, `arrow_ipc` feature in crow_web).
- [x] **`bytes[data]`** — Raw binary response with `application/octet-stream` content type.
- [x] **`stream[fn]`** — Streaming/chunked response via `tokio::sync::mpsc` channel + `axum::body::Body::from_stream`. Feature-gated (`stream` feature in crow_web).
- [x] **Interpreter mocks** — All four builtins (`msgpack`, `arrow`, `bytes`, `stream`) work in `crow run -i` mode.
- [x] **Tests** — 10 new tests (4 transpiler codegen + 4 interpreter + 2 arrow IPC unit tests). Total: 702 tests passing.

#### Client-side WASM — rejected
Rejected in favor of the Crow UI framework (see below). Building a WASM frontend pipeline would require `wasm-bindgen`, `web-sys`, and a reactive runtime — essentially building React/Svelte from scratch. The UI framework approach achieves the same goal (one-language full-stack) with pure Crow libraries and server-rendered HTML.

#### P8 — UI Framework — in progress (Phases 1–3 done; `view` blocks, `later`, style and chart modules shipped since — see the status table above)

> **See [docs/ui-framework.md](ui-framework.md) for the full design document.**

Full-stack web applications written entirely in Crow. Declarative reactive pattern: `page[path, render, handle]` with `serve[port]`, reactive data loading (`fetch`, `poll`), two-way binding (`field`, `select`), and `table[frame]` auto-rendering. No React, no npm, no node_modules.

##### Phase 1: Counter Works (foundation) ✅

Goal: `page['/', render, handle]` with `serve[3000]` runs a LiveView counter in a browser.

- [x] **VNode builder API** — `crow_web::view`: `CrowVNode` trees with attributes and event handler IDs
- [x] **Live module** — `crow_web::live`: VNode tree diffing, event registry, per-connection mutable state
- [x] **JS runtime v1** — WebSocket + DOM patching + event delegation via `data-crow-*` attributes
- [x] **Transpiler codegen** — `div[h1['hello']]` → `CrowVNode::el("div", ...)`, `use ui` detection
- [x] **`page[]` / `serve[]` API** — implicit app, `serve[port]` deferred to end of main
- [x] **`state.get` / `state.set`** — type-inferred: `state.get['count', 0]` → `get_i64()`
- [x] **`use web, ui`** — comma-separated multi-import syntax
- [x] **Counter example** — `examples/counter.co`

##### Phase 2: Reactive Data & Two-Way Binding ✅

Goal: `fetch[url]`, `poll[url, seconds]`, `field[label, var]`, `select[options, var]`, `table[frame]` — the reactive glue layer.

- [x] **`fetch[url]`** — one-time reactive HTTP GET, registered on `CrowLiveState`, executed on page load and WS connect
- [x] **`poll[url, seconds]`** — repeating reactive HTTP GET with per-source interval tracking via `tokio::select!`
- [x] **`field[label, var]`** — two-way bound labeled input (`data-crow-input="__field:key"`)
- [x] **`select[options, var]`** — two-way bound dropdown (`data-crow-change="__select:key"`)
- [x] **`table[frame]`** — auto-render table from headers + rows via `CrowVNode::table_from_data`
- [x] **`today[]`** — returns today's date via `chrono::Local`
- [x] **Validator/interpreter/transpiler** — all builtins registered with correct signatures
- [x] **JS runtime** — `morphChildren` preserves input focus and values during re-render
- [x] **VNode constructors** — all HTML elements in transpiler have matching `CrowVNode` methods

##### Phase 3: Multi-Page App (routing + layout) ✅

- [x] **Multiple `page` routes** — several routes on one app (`page[path, view]` per route), each a session of its own, served in both modes (`crow run -i` had served only the last); an address no page answers is a 404
- [x] **URL parameters** — a path's `:name` is a fact in the page's address (`page['/card/:card', card_page]`), a query's facts are `url = […]`; the address follows both, and a link to another address of the page opens that address's session
- [x] **Layout functions** — `layout.co`, applied to every page
- [x] **`title[]`, `meta[]`** — per-route head management: elements a view places, which a served page carries in its head and a render keeps current
- [x] **CSS** — `static_dir` for stylesheets, and the default stylesheet (`css/app.css` hatch)

##### Phase 4: Real App Features

- [x] **Sessions** — the session layer: per-reader facts, `reader`, `sign_in`/`sign_out` (docs/session-layer-design.md)
- [x] **Auth** — a page for signed-in readers checks `reader` in its view: it draws the sign-in form in place, offers none of the page's handlers and runs none of its queries (spec, Who the reader is). `auth = true` on a page is rejected (Design Note: no `auth` on a page)
- ~~**Channels**~~ — rejected (Design Note: no channels): a write already reaches every reader, and a process outside the program is seen through `fresh`
- ~~**Debounce**~~ — rejected (Design Note: no debounce): a derivation has one recompute in flight, the latest input winning, and a second click on a running handler is dropped
- [x] **Loading states, file upload** — `running[h]`, uploads
- [x] **Keyboard events** — a key presses a drawn element: `shortcut = 'ctrl+s'` (drawn as `aria-keyshortcuts`), Escape on a dialog's Close and the runtime's question, Enter in a form presses its first button, and a form never reloads the page; `keydown` handlers rejected (Design Note: keys press elements)
- [x] **Row click** — `table[frame, click = h, key = 'col']`, row actions
- [ ] **Table features** — sorting, filtering
- [x] **Modals, pagination** — `modal[child, open = var]`, `table[frame, page = 50]`
- [ ] **Accessibility** — an audit (nav's `aria-current` and tabs' roles are in)

##### Phase 5: Developer Experience

- [ ] **`crow dev`** — hot reload server, error overlay
- [x] **Chart engine** — shipped as the `chart[frame, ...]` element (server-rendered SVG, `chart-module-design.md`)
- [ ] **Dev tools** — state inspector, event log

### P4.5 — Database module ✅

Top-level `db` module (`use db`) with separate `crow_db` crate. All three backends (SQLite, PostgreSQL, ClickHouse) fully implemented and live-tested.

- [x] **`connect[url, pool_size?]`** — Unified connection with URL-scheme dispatch (sqlite://, postgres://, clickhouse://). Automatic connection pooling.
- [x] **`query[conn, sql]`** — Returns a frame (not array of maps).
- [x] **`exec[conn, sql, params?]`** — DDL/DML execution, returns affected row count. Params are an array of bind values.
- [x] **`insert[conn, table, frame]`** — Bulk insert using backend-specific fast paths.
- [x] **`transact[conn, fn]`** — Named function executes inside a transaction with auto commit/rollback.
- [x] **`migrate[conn, dir]`** — Forward-only SQL migrations with `_crow_migrations` tracking table.
- [x] **Schema modes** (Sept 2026) — the literal `CREATE TABLE` statements a program hands a connection declare its tables; opening it reconciles (run from source: missing columns added under a lock), verifies (a built program: the catalog read, a difference stops at connect with the mending SQL) or trusts (warns every start). Crowfile `schema`, `CROW_SCHEMA`; `crow db diff` drafts the migration. Parity 124–128 (127–128: a compiled program using Postgres or ClickHouse without the web layer had not built).
- [x] **Conflict resolution** — Language-level disambiguation for `module.fn[]` when multiple modules export the same name.
- [x] **Interpreter support** — All 6 builtins work in `crow run -i` mode.
- [x] **Transpiler codegen** — `use db` → `needs_crow_db` flag, `crow_db` in generated `Cargo.toml`, Rust codegen for all builtins.
- [x] **Tests** — 9 crow_db unit tests + 8 interpreter tests + 8 transpiler tests + integration example.

### P5 — GPU runtime ✅

- [x] **GPU matmul shader**: WGSL compute shader for matrix multiplication. `matmul` auto-dispatches to GPU when both tensors are on GPU, falls back to CPU (faer) otherwise.
- [x] **GPU parallel reduction**: Real tree-based parallel reduction shader for `sum`. Uses workgroup shared memory with 256-element workgroups. Falls back to CPU for small inputs (<1024 elements).
- [x] **GPU execution verification**: All GPU ops tested end-to-end (upload → compute → download): matmul, sum, all unary ops (neg/abs/exp/log/sqrt/sin/cos), all binary ops (add/sub/mul/div), scalar ops.
- [x] **GPU benchmarks**: `cargo test -p crow_runtime bench_gpu -- --nocapture --ignored` runs matmul, elementwise, and reduction benchmarks at multiple sizes.
- [x] **GPU memory management**: `gpu_memory_info()` queries device limits, `gpu_try_alloc()` checks max_buffer_size before allocation for graceful OOM handling.

### P6 — Distributed runtime (weeks)

- [ ] **Multi-node execution**: `nodes[n]` directive exists but needs real multi-node testing.
- [ ] **Data sharding**: Verify automatic data partitioning across nodes.
- [ ] **Gradient synchronization**: Verify all-reduce for distributed training.
- [ ] **Strategy directives**: Test pipeline parallelism, tensor parallelism beyond data parallelism.

### P7 — Ecosystem (months)

- [ ] **Package registry**: `crow install` command exists but needs a real registry backend (GitHub-based or custom).
- [ ] **Community packages**: Seed the ecosystem with useful packages (plotting, data connectors, model zoo).
- [ ] **Pretrained models**: Common models (linear regression, decision trees, simple NNs) as installable packages.
- [ ] **Notebook integration**: Jupyter kernel or similar interactive environment for exploratory data science.
- [x] **Editor support**: `crow lsp` + VS Code extension (Neovim/Helix/Zed via native LSP clients) — see `crow-lsp.md`.

## Overall Rating (current)

### As a data science language

| Language | Breadth | Depth | Ergonomics | Overall |
|----------|---------|-------|------------|---------|
| Python (NumPy+SciPy+pandas) | 10/10 | 10/10 | 6/10 | 9/10 |
| Julia | 9/10 | 9/10 | 8/10 | 9/10 |
| MATLAB | 9/10 | 10/10 | 7/10 | 8/10 |
| R | 8/10 | 9/10 (stats) | 5/10 | 7/10 |
| **Crow** | 7/10 | 7/10 | 9/10 | 7/10 |

### As a general-purpose / fullstack language

| Language | Stdlib | Web | DB | Ecosystem | Overall |
|----------|--------|-----|-----|-----------|---------|
| Python | 10/10 | 8/10 | 9/10 | 10/10 | 9/10 |
| Go | 9/10 | 9/10 | 7/10 | 8/10 | 8/10 |
| JavaScript/Node | 7/10 | 10/10 | 8/10 | 10/10 | 9/10 |
| **Crow** | 7/10 | 7/10 | 8/10 | 1/10 | 6/10 |

### Rating Rationale

**Data science (7/10):** Crow has a strong math foundation — ~107 tensor/linalg/stats/signal functions, auto-parallel for loops, autograd, DataFrames, and GPU support. P1 added statistical tests (chi-square, ANOVA, KS, distribution fitting) and linalg completeness (rank, cond). P2 added STFT, spectrogram, and frequency-domain filters (lowpass, highpass, bandpass). P5 verified GPU execution end-to-end with matmul, parallel reduction, and element-wise shaders. Breadth is 7/10 — covers core operations plus statistical testing and signal processing. Depth is 7/10 — individual areas are solid with proper implementations (regularized gamma/beta functions, SVD-based rank, FFT-based filtering, verified GPU dispatch with memory management). Ergonomics is 9/10 — the syntax is genuinely cleaner than Python+NumPy for common operations.

**General-purpose (6/10):**
- **Stdlib (7/10):** JSON (`json[str]`, `encode[value]`), env vars (`env[name]`, `env[name, default]` + `.env` auto-loading), CLI args (`args[]`), logging (`log[]`), file I/O (`read`, `write`, `exists`, `mkdir`, `ls`, `rm`, `mv`, `cp`), regex (`find`, `matches`, `sub`), date/time (`now`, `date`, `elapsed`), shell (`shell[cmd]`), hashing (`hash[data, algo]`), input (`input[]`). Covers the practical needs for scripting and application development. Still below Python's 200+ modules — missing HTTP client as builtin, process management, and some niche utilities.
- **Web (7/10):** P4 provides 17 web modules plus binary/streaming response formats (msgpack, arrow, bytes, stream). Comparable to Express.js/Flask for server-side coverage. 7/10 because: no ORM (by design), no middleware composition, and end-to-end app testing is limited. P8 (UI Framework) will add full-stack frontend generation via pure Crow libraries — no WASM needed.
- **DB (8/10):** Top-level `db` module with connection pooling, parameterized queries, transactions, bulk insert, and forward-only migrations. All three backends (SQLite, PostgreSQL, ClickHouse) fully implemented and live-tested. No ORM (by design — Crow uses raw SQL). 8/10 because the core DB story is complete across OLTP and OLAP engines; missing only advanced features like connection-level event listeners or prepared statement caching.
- **Ecosystem (1/10):** No real package registry, no community packages, no third-party libraries. `crow install` exists as a command but there's nothing to install. This is the single biggest gap.

**What moves the needle:**
- Ecosystem → 5/10 would require a working registry + 20-30 community packages
- Web → 8/10 would require end-to-end app testing and the UI framework (P8)
- Stdlib → 8/10 would require HTTP client builtin, process management, and a few more utilities

P0–P5 are complete. P6 (distributed verification), P7 (ecosystem), and P8 (UI framework) are what separate Crow from a production-ready tool.

