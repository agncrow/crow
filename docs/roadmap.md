# Crow Roadmap

What's done, what's remaining, and in what order.

## Status: What's Done ✅

| Area | Details |
|------|---------|
| Core language | Lexer, parser, AST, interpreter, transpiler, checker, formatter, REPL |
| Type system | Dynamic with inference — int, float, str, bool, array, map, tensor, frame, function, error |
| Modules | `math`, `math.stats`, `math.nn`, `io`, `frame` |
| Math (~100 functions) | Creation (9), element-wise (12), trig (7), hyperbolic (3), reductions (9), sorting/selection (6), shape (3), linalg (13), random (12), signal (7), special (5), complex (6), sparse (3) |
| Statistics (9) | median, std, var, percentile, zscore, corr, cov, ttest, linreg |
| Neural networks (8) | relu, sigmoid, softmax, gelu, mse, crossentropy, bce, linear |
| DataFrames (16+) | load, save, select, filter, sort, group, join, stack, head, tail, fill, unique, rename, stats, datetime, tz |
| Autograd | `grad[f]` functional transformation |
| Parallel execution | Auto-parallel `for` loops (rayon), statement-level parallelism via dependency analysis |
| CLI | `crow run`, `crow build`, `crow fmt`, `crow check`, `crow init`, `crow install`, REPL |
| Tests | 603 unit tests |
| Documentation | language-spec.md, math-library.md, feature-map.md, frame-module.md, usage.md, comparison.md, automatic-verification.md |

## Remaining Work

### P0 — Quick fixes (hours) ✅

- [x] **Fix flaky test**: Combined 4 seed-dependent interpreter tests into one `test_math_random_all` to avoid global RNG state races (same pattern as `distributed.rs::test_distributed_all`). Added `clear_seed()` cleanup.
- [x] **Spec cleanup**: Updated Design Note "No sum, min[arr], max[arr] in Core" — replaced stale "A future math library will provide..." with reference to `use math` reductions.

### P1 — Math/Stats completeness (days)

- [ ] **`rank`**: Matrix rank via SVD — last common linalg gap.
- [ ] **`cond`**: Condition number — last common linalg gap.
- [ ] **Chi-square test**: `chisq[observed, expected]` → `{chi2, p}`.
- [ ] **ANOVA**: `anova[group1, group2, ...]` → `{f, p}`.
- [ ] **KS test**: `kstest[data, 'normal']` → `{d, p}`.
- [ ] **Distribution fitting**: `fit[data, 'normal']` → `{mu, sigma}` (or appropriate params for each distribution).

### P2 — Signal processing depth (days–weeks)

- [ ] **STFT**: Short-time Fourier transform — `stft[signal, window_size]`.
- [ ] **Spectrogram**: `spectrogram[signal, window_size]` → 2D magnitude tensor.
- [ ] **Filter design**: `lowpass`, `highpass`, `bandpass` — basic IIR/FIR filters.

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
- [x] **`find[s, pattern]`** — regex match → `{ok, groups}` map. (`match` is a reserved keyword, so the builtin is named `find`.)
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

#### Client-side WASM — deferred
These require building a full WASM frontend pipeline (`wasm-bindgen`, `web-sys`, reactive runtime). Multi-month effort, deferred until server-side modules are fully usable from Crow source code.
- [ ] **`web.router`** — Client-side URL routing (History API, route matching, component swapping).
- [ ] **`web.store`** — Reactive state management (signals, computed values, fine-grained DOM updates).
- [ ] **Build modes** — `crow build --spa`, `--ssr`, `--live`, `--static` (see Rendering Modes in web-framework.md).

#### Response formats — remaining
- [ ] **`msgpack[value]`** — MessagePack binary response via `rmp-serde`.
- [ ] **`arrow[frame]`** — Arrow IPC response for DataFrames/tensors.
- [ ] **`stream[fn]`** — Streaming/chunked response.

### P5 — GPU runtime (weeks)

- [ ] **GPU execution verification**: `gpu[n]` directive exists but actual wgpu execution is unproven. Need to verify tensor ops actually dispatch to GPU.
- [ ] **GPU benchmarks**: Compare GPU vs CPU for common operations (matmul, conv2d, element-wise) at various sizes.
- [ ] **GPU memory management**: Verify tensor transfers, memory limits, out-of-memory handling.

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
| **Crow** | 6/10 | 5/10 | 9/10 | 6/10 |

### As a general-purpose / fullstack language

| Language | Stdlib | Web | DB | Ecosystem | Overall |
|----------|--------|-----|-----|-----------|---------|
| Python | 10/10 | 8/10 | 9/10 | 10/10 | 9/10 |
| Go | 9/10 | 9/10 | 7/10 | 8/10 | 8/10 |
| JavaScript/Node | 7/10 | 10/10 | 8/10 | 10/10 | 9/10 |
| **Crow** | 3/10 | 1/10 | 1/10 | 1/10 | 5/10 |

P3 and P4 are complete. P3 added regex, date/time, shell, and hashing (interpreter + transpiler). P4 wired up all 17 web modules with transpiler codegen, validator rules, and interpreter mock support (676 tests passing). P5–P7 are what separate a usable tool from a production ecosystem.

