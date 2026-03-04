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

### P3 — General-purpose language (days–weeks)

Currently Crow has basic I/O (`read`, `write`, `args`, `input`, `env`, `time`, `sleep`, `exit`, `json`, `encode`) but lacks standard library features expected of a general-purpose language.

#### File system
- [ ] **`exists[path]`** — check if file or directory exists (returns bool).
- [ ] **`mkdir[path]`** — create directory (and parents). Error-value on failure.
- [ ] **`ls[path]`** — list directory contents → array of strings.
- [ ] **`delete[path]`** — remove file or empty directory. Error-value on failure.
- [ ] **`rename[old, new]`** — rename/move file or directory.
- [ ] **`copy[src, dst]`** — copy file.

#### Networking / HTTP client
- [ ] **`fetch[url]`** — HTTP GET → `{status, body, headers}`. Error-value on failure.
- [ ] **`fetch[url, method, body, headers]`** — full HTTP request (POST, PUT, DELETE, etc.).

#### Regex
- [ ] **`match[s, pattern]`** — regex match → `{ok, groups}` or error-value.
- [ ] **`matches[s, pattern]`** — does string match pattern? (returns bool).
- [ ] **`sub[s, pattern, replacement]`** — regex substitution.

#### Date/time
- [ ] **`now[]`** — current datetime struct with `.year`, `.month`, `.day`, `.hour`, `.minute`, `.second`.
- [ ] **`date[y, m, d]`** — construct date.
- [ ] **`elapsed[start, end]`** — duration between two timestamps.
- [ ] **`format[dt, pattern]`** — format datetime to string.

#### Process / shell
- [ ] **`shell[cmd]`** — run shell command → `{status, stdout, stderr}`. Error-value on failure.

#### Hashing
- [ ] **`hash[s, 'sha256']`** — compute hash digest → hex string. Support sha256, sha512, md5.

### P4 — Fullstack / web (weeks)

#### HTTP server
- [ ] **`serve[port]`** — start HTTP server on port.
- [ ] **`route[method, path, handler]`** — register route handler. Handler receives request map, returns response map.
- [ ] **Request/response model** — request: `{method, path, headers, body, query}`. Response: `{status, body, headers}`.
- [ ] **Static files** — `static[path]` to serve a directory.

#### Database
- [ ] **`connect[url]`** — connect to database (SQLite, PostgreSQL). Returns connection handle.
- [ ] **`query[conn, sql, params]`** — execute SQL query → array of maps (rows). Error-value on failure.
- [ ] **`execute[conn, sql, params]`** — execute SQL statement (INSERT, UPDATE, DELETE) → `{rows_affected}`.

#### WebSocket
- [ ] **`ws[url, handler]`** — WebSocket client/server support.

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
| **Crow** | 3/10 | 0/10 | 0/10 | 1/10 | 2/10 |

Completing P0–P1 would bring the data science rating to **7/10**. Completing P3 (general-purpose) would bring the fullstack rating from 2/10 to **5/10**. P4 (web/DB) would bring it to **7/10**. P5–P7 are what separate a usable tool from a production ecosystem.

