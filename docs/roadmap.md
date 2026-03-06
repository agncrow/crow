# Crow Roadmap

What's done, what's remaining, and in what order.

## Status: What's Done ✅

| Area | Details |
|------|---------|
| Core language | Lexer, parser, AST, interpreter, transpiler, checker, formatter, REPL |
| Type system | Dynamic with inference — int, float, str, bool, array, map, tensor, frame, function, error |
| Modules | `math`, `math.stats`, `math.nn`, `io`, `frame`, `web` (17 sub-modules), `db` (SQLite, PostgreSQL, ClickHouse) |
| Math (~107 functions) | Creation (9), element-wise (12), trig (7), hyperbolic (3), reductions (9), sorting/selection (6), shape (3), linalg (15), random (12), signal (12), special (5), complex (6), sparse (3) |
| Statistics (13) | median, std, var, percentile, zscore, corr, cov, ttest, linreg, chisq, anova, kstest, fit |
| Neural networks (8) | relu, sigmoid, softmax, gelu, mse, crossentropy, bce, linear |
| DataFrames (16+) | load, save, select, filter, sort, group, join, stack, head, tail, fill, unique, rename, stats, datetime, tz |
| General-purpose (P3) | regex (find, matches, sub), date/time (now, date, elapsed), shell, hashing (sha256/sha512/md5) |
| Web framework (P4) | 17 modules: server, client, db, auth, cors, session, template, cache, rate, upload, sse, ws, live, static, mail, queue, docs — all with transpiler codegen + interpreter support |
| Autograd | `grad[f]` functional transformation |
| Parallel execution | Auto-parallel `for` loops (rayon), statement-level parallelism via dependency analysis |
| CLI | `crow run`, `crow build`, `crow fmt`, `crow check`, `crow new`, `crow install`, REPL |
| Project structure | `crow new` scaffolding (--web, --ml, --db), `crow run <job>`, convention-based views/jobs/layout/db, dot-path imports (`use views.home`), multi-import (`use web, ui`) |
| DX | Default welcome page for empty web apps, friendly error messages (e.g. "port 3000 is already in use") |
| Tests | 998 unit tests (+ 3 GPU benchmarks) |
| Documentation | language-spec.md, web-framework.md, ui-framework.md, math-library.md, feature-map.md, frame-module.md, usage.md, comparison.md, automatic-verification.md |

## Remaining Work

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

#### P8 — UI Framework — in progress

> **See [docs/ui-framework.md](ui-framework.md) for the full design document.**

Full-stack web applications written entirely in Crow. Server-rendered LiveView pattern: mutable state on the server, HTML diffs over WebSocket, tiny JS runtime on the client. No React, no npm, no node_modules.

##### Phase 1: Counter Works (foundation)

Goal: `page['/', render, handle]` with `serve[3000]` runs a LiveView counter in a browser. No explicit app variable needed.

- [ ] **VNode builder API** — `crow_web::view`: element functions (`vnode_el`, `vnode_text`) that build `CrowVNode` trees with attributes and event handler IDs
- [ ] **Upgrade live module** — `crow_web::live`: replace LCS string diff with `CrowVNode::diff` tree patching, add event registry (handler ID → closure map), per-connection mutable state
- [ ] **JS runtime v1** — `crow_web::live`: WebSocket + DOM patching (morphdom or custom) + event delegation + reconnect (~100 lines JS)
- [ ] **Transpiler codegen** — `src/transpiler.rs`: `div[class = 'x']` → `CrowVNode::el("div", ...)`, `button[click = fn[], '+']` → VNode + event registration, `use ui` module detection
- [ ] **Interpreter support** — `src/interpreter.rs`: UI element functions work in `crow run -i` mode (render to HTML string)
- [ ] **Counter example** — `examples/counter.crow`: working end-to-end in browser with click events

##### Phase 2: Forms Work (interaction)

- [ ] **`ui.input` element** — text, email, password, number inputs with value binding
- [ ] **`input` / `change` events** — send value on keystroke / blur
- [ ] **`form` + `submit`** — form submission handler
- [ ] **`select` element** — dropdown with `change` handler
- [ ] **Debounce** — `debounce = ms` option on input events (client-side)
- [ ] **Conditional / list rendering** — verify `if`/`for` in Crow produce correct VNode trees

##### Phase 3: Multi-Page App (routing + layout)

- [ ] **Multiple `live` routes** — several routes on one `web` app
- [ ] **URL parameters** — route params (`:id`), query params (`?q=foo`)
- [ ] **Live navigation** — `redirect['/path']`, `<a>` links without full page reload
- [ ] **Layout functions** — Crow functions wrapping route content
- [ ] **`title[]`, `meta[]`** — per-route head management
- [ ] **CSS** — `static_dir` for stylesheets (already exists)

##### Phase 4: Real App Features

- [ ] **Sessions** — `session.set/get/clear`, signed cookies
- [ ] **Auth** — `auth = true` on routes, redirect to login
- [ ] **Channels** — `channel.join/send/broadcast`, `on_channel` handler (server push)
- [ ] **Flash messages** — `flash[state, msg, timeout]`
- [ ] **Loading states** — `loading[state, fn[]]`
- [ ] **Keyboard events** — `keydown`, modifier keys
- [ ] **File upload** — binary over WebSocket
- [ ] **Downloads** — `download[content, filename, type]`
- [ ] **Table features** — sorting, filtering, row click
- [ ] **Modals, pagination, accessibility**
- [ ] **Error boundaries** — `on_error`, `state.error`

##### Phase 5: Developer Experience

- [ ] **`live.test`** — mount route in test, simulate events, assert HTML
- [ ] **`crow dev`** — hot reload server, error overlay
- [ ] **Dev tools** — state inspector, event log (JS panel in browser)
- [ ] **`live.screenshot`** — OS WebView rendering, pixel diff for visual regression
- [ ] **Chart engine** — `chart.live`, custom Canvas renderer, hooks
- [ ] **Component gallery** — `crow dev --gallery`

### P4.5 — Database module ✅

Top-level `db` module (`use db`) with separate `crow_db` crate. All three backends (SQLite, PostgreSQL, ClickHouse) fully implemented and live-tested.

- [x] **`connect[url, pool_size?]`** — Unified connection with URL-scheme dispatch (sqlite://, postgres://, clickhouse://). Automatic connection pooling.
- [x] **`query[conn, sql]`** — Returns a frame (not array of maps).
- [x] **`exec[conn, sql, params?]`** — DDL/DML execution, returns affected row count. Params are an array of bind values.
- [x] **`insert[conn, table, frame]`** — Bulk insert using backend-specific fast paths.
- [x] **`transact[conn, fn]`** — Named function executes inside a transaction with auto commit/rollback.
- [x] **`migrate[conn, dir]`** — Forward-only SQL migrations with `_crow_migrations` tracking table.
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
- [ ] **Editor support**: Syntax highlighting and LSP for VS Code, Vim, etc.

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

