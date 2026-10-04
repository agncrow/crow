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
| Web framework (P4) | 17 modules: server, client, db, auth, cors, session, template, cache, rate, upload, sse, ws, live, static, mail, queue, docs. Pages, routes, responses, auth for pages and mail work in both modes; the rest are audited for 1.0 (see The 1.0 bar): several are mocks in the interpreter, and a cache, a rate limiter or a session store can be made but not used |
| Autograd | `grad[f]` functional transformation |
| Parallel execution | Auto-parallel `for` loops (rayon), statement-level parallelism via dependency analysis |
| CLI | `crow run`, `crow build`, `crow stop`, `crow fmt`, `crow check` (+ `--db`, `--strict`), `crow fix`, `crow db diff`, `crow test` (blocks, `--parity`, `--book`), `crow lsp`, `crow new`, `crow install`, `crow launch`, REPL |
| Verification | `test` blocks (attached examples/laws, named scenarios, generated inputs, `app[]` harness for views); `crow check` hazards for the live loop (blocking, stale snapshots) and, with a schema, the concurrency catalog (write skew, lost update, latch, assumed-unique, unpaired writes) |
| Live apps | `view` blocks, reactive state, named-argument attributes, deferred event calls, `later[]`/`running[]`, `serve[port, refresh =, budget =]`, default stylesheet (8 classes + 8 tokens, `css/app.css` hatch), `chart[]` element |
| Book runner | `crow book`: the book served by the compiler with every block runnable in place (interpret / compile / test / stop) |
| Editor | `crow lsp` (diagnostics, completion, hover, go-to-definition, signature help) + VS Code extension in `editors/vscode` |
| Project structure | `crow new` scaffolding (--web, --ml, --db), `crow run <job>`, convention-based views/jobs/layout/db, dot-path imports (`use views.home`), multi-import (`use web, ui`) |
| DX | Default welcome page for empty web apps, friendly error messages (e.g. "port 3000 is already in use") |
| Tests | cargo unit tests, 139-file parity corpus (`crow test --parity`), generated programs in both modes (`crow test --fuzz`), a database stopped under a running program and a served app (`tests/lost_connection.rs`), the served pages in both modes (`crow test --served`), every book block in both modes (`crow test --book`), docs-vs-compiler coverage (`crow test --docs`), db-check fixtures in `tests/db_check`, chart harness; `.github/workflows/ci.yml` runs all of them, as the local gate does (GitHub's billing stops CI from starting today) |
| Documentation | The Crow Book (`docs/book`, 13 chapters + 5 appendices), language-spec.md, crow-for-llms.md, usage.md, crow-check.md, crow-lsp.md, module references, design records — indexed in `docs/README.md` |

## The 1.0 bar

Vijay (Sept 30 2026): no 1.0 until the language and its modules are bug-free and robust for production. What that takes, checked off as each lands:

- [x] **Route handlers** — a handler that stops answers 500 with the reason, as the interpreter's does (the compiled program dropped the connection), and a route's handler runs off the server's threads, as a page's does.
- [x] **No mocks, no half-features** — every builtin works the same in both modes, with a test and an example: the docs check's spec-only list holds only the keyword `test`, the removed `today`, and the test harness's `app` and `click`. The web audit (Vijay, Sept 30 2026): removed the queue, templates, the old SQLite handle, session stores, the SSE and WebSocket servers, the cache and API docs (Design Note "what web is"); CORS and files are `serve`'s options, a rate limit is a route's, a form's fields and files are `req.form` and `req.files`, and `sign`/`verify` work in both modes. Found on the way: a route read any size of body (now the uploads limit, 413), a query reached the program still encoded, and a CARGO_TARGET_DIR set elsewhere sent a compiled program's binary where `crow run` did not look. The WebSocket client is `socket[url]` with `send`, `recv` (and a wait) and `close`, `wss://` included — its `connect` had been the database's name, and the interpreter's calls stand-ins.
- [x] **A lost connection** — `connect` stays a stop at boot, and a connection that drops later answers errors while it is gone and reconnects on its own: `tests/lost_connection.rs` stops a scratch Postgres under a running program and under a served app, in both modes, and starts it again. While it is gone each query's reason says so — `the database does not answer: …` (it had said `postgres pool error: Backend error`) — and the first query after it is back reconnects.
- [x] **Differential fuzzing** — `crow test --fuzz`: programs generated from a seed (values of six kinds, maps and records, parsed documents read and spliced into text, the text, list and map builtins, conversions read with `.ok`, branches, loops, functions) run in both modes, and 3000 strings of random tokens go through the front end without a panic. Fifty programs from seed 1000 agreed on the first run.
- [x] **Failure tests for the server** — a served app's database stops and comes back: the route answers 500 with the reason while it is gone and 200 after, a slow handler holds up no other request, a malformed request is a 400, and the server never goes down (`tests/lost_connection.rs`); an oversized body is a 413 and a client over its limit a 429 (`tests/web_options.rs`). Found on the way: a route's handler that read a top-level connection was told to take it as a parameter or be wired as a handler, neither of which a route can do — it is told to open what it needs itself, which the spec now says (every `connect` to one URL is one handle, so the handlers share a pool).
- [x] **A fact holds no error** — a ui program's fact declared from a value that can fail (`trades = load['trades.csv']`) is refused where the facts are set, with the reason, and holds the value itself: compiled, such a program had not built at all (a session's state could not keep a value that can fail), and the interpreter had held the error in the fact (parity 142, served 23). Found on the way: a dynamic value in text — a parsed document's member, a cell of a frame loaded as the program runs — showed with JSON's quotes compiled (parity 143).
- [ ] **A soak test** — an app under load for a day, its memory and latency steady. `tests/soak.rs` (run on purpose: `CROW_SOAK_SECS=86400 cargo test --release --test soak -- --ignored --nocapture`) serves a compiled app — a route that queries, a route that answers JSON, a live page — to a steady hundred requests a second, samples its memory and latency, and fails on a request not answered, a slow request slowing, or memory that keeps growing (a line through the samples after the warm-up, judged on five minutes or more). A ten-minute run found a leak and, fixed, held steady: 52,895 requests, none failed, p99 near a millisecond, memory flat at 57 MB. The leak: the list of sessions the app can wake had kept every session a page load ended (pruned only when a table was written), some 7 KB a second at that load. Also found: a fact declared from a connection (`made = exec[book, …]`) stopped the compiled program as it started, the connection not yet set (served 24). An hour's run after the fix held as well: 306,468 requests, none failed, p99 under 2 ms, memory from 57.3 MB to 57.5 MB (7 MB a day by its trend). Left: the day-long run.
- [x] **A security review of crow_web** — sessions, cookies, origin checks, uploads, static paths, request size limits: `docs/security-review.md`. Found and fixed: static files handed out dot-named paths (`files = '.'` served `.env` and the key the sign-in cookies are sealed with); a page's socket took messages of 64 MB; a response holding a header with a line break answered `internal error` and said nothing; the key file was readable for a moment after it was written. The book's deployment section says what a proxy must pass on and why. A failed handler's 500 had told the client its reason (a database's address, a file's path): it tells an id, and the server's log has the reason beside it (Vijay, 2026-10-01). A page's handler that stopped had told its reader the reason as the notice; it tells that it stopped, that nothing changed, and an id, the reason in the log and under `crow dev` in the dev panel (Vijay, 2026-10-01).
- [x] **Fixture recording** — an app's tests run without live databases: `crow test --fixtures` answers each Postgres and ClickHouse read from what `crow check --record-fixtures` kept, a write as one that landed, the clock and the random draws as the recording had them, in both modes (`crow_db::fixtures`, shared with the rehearsals; `tests/fixture_replay.rs` records a scratch Postgres, stops it, and runs the tests from the fixtures, interpreted and compiled). A sqlite file is the program's own and opens as always — a test that writes and reads back needs its database, and a local file is one. A statement no fixture holds is named by the test that fails. Found on the way: a compiled view that refused a value still being computed (`error: loading`) lost its stale mark, so a compiled test read the page before the value landed, and the view's level was left open.
- [x] **S11** — the IR and the Rust say `x` where they said `x__s2`: the lowering's last step gives each block's variable the name the program wrote (the passes before it still name a later binding apart, which cost nothing to keep). Found on the way: a parameter the body changes had not built compiled.
- [x] **Every call names its row** — the calls on parsed JSON values (a parsed value is JSON in both modes), and the four whose arguments come from outside Crow (`grad`'s functions, the layout engine's nodes). Left by name: a call on a value nothing types, one in finch's parts lowered alone.
- [x] **The language server** — ranges under the name a diagnostic names, rename (scoped, and across the project's files for a part's name), quick fixes (a slip's nearest name, a missing import — both said by the refusals themselves now), and parts validated once per text with overtaken changes skipped. Found on the way: a part with an error of its own had made `crow lsp` exit.
- [x] **The ecosystem** — P7, below: packages (git tags and a lockfile), the Jupyter kernel, and the models and market packages; the data connectors are built into the language (every reader takes a URL or an S3 object, `load`/`save` read Parquet and JSON by extension).

- [x] **An HTTP client** (Vijay, 2026-10-01) — `request[url, method, body]`, with `headers = map[…]`: a GET, POST, PUT, PATCH or DELETE (HEAD and OPTIONS too), answered as a record of `status`, `body` and `headers` whatever the status, an error value of kind `'http'` only when no answer came; a JSON body goes as JSON; in a handler an unused answer waits for the commit; a rehearsal records it. One builtin with the method named, as `route` names the server's (Design Note: one `request`). Then the rest a client sends (Vijay, 2026-10-01): `query = map[…]` encoded into the URL, `timeout = seconds`, `form = map[…]` sent URL-encoded, `files = map[field = path]` sent as a multipart body, and `save = path` for a download's bytes; the readers take `query` and `timeout` beside `headers`; `encode[text, 'base64' | 'url']` and `decode[text, scheme]` for a Basic authorization and a URL's piece (Design Note: one `encode`). Both modes through `crow_runtime::sources` (`tests/http_request.rs`, `tests/reader_headers.rs`, parity 145).
- [x] **How far one process goes** (Vijay, 2026-10-02: "a scaling story, even if it's one process") — measured, not guessed: `tests/capacity.rs` serves a compiled app and reports what a session costs (4 KB parked, 140 KB as an open tab with its render), how fast tabs open (500 in 2 s, each sent its render) and what the routes answer (~30,000 requests a second at p99 under 0.4 ms on a laptop); the book's deployment chapter reads the numbers, says what bounds a process (file descriptors, the proxy, the renders' memory, the database) and how to run several behind a proxy with sticky tabs; spec Design Note "one process".
- [x] **The data-science gaps of 2026-10-02** (Vijay: "implement 2-5") — frames grouped by several keys, pivoted, and windowed (`group[df, ['a', 'b'], …]`, `pivot`, `window` with `n` and `by`; a group's key column keeps its kind, where the runtime had made it text); the models package grown to twelve files (ridge, k nearest neighbours, Gaussian naive Bayes, principal components, a random forest, the metrics, split and scaling), each tested in both modes; and three typing gaps the models exposed, fixed: a list of tensor rows is a matrix wherever a tensor is taken, `stack` and `tensor` state their shapes so a stacked matrix's columns walk as rows, and a call whose argument's kind shows only as the program runs is typed by what its rows agree on (an `abs` had picked the int row). The client-side widget the assessment had listed as missing exists (`widget`, spec; served tests 10 and 14). Reverse-mode autograd and the estimators that need it are the next item.
- [x] **Reverse-mode autograd** (Vijay, 2026-10-02: item 2 of the assessment) — `grad[f]` of a function of tensors answers the gradient of its number with respect to each parameter, from one run backward over a tape (spec, Automatic Differentiation: the list of differentiable operations, what is refused before the program runs, Design Note "one `grad`"); the same `grad`, forward over numbers as before. Both modes through `crow_runtime::autograd`: the interpreter runs the function on traced values, the compiled program a traced copy of it and of every function it reaches, the fusion pass leaving both alone (parity 155; the lowering's tests). The models package trains by it: `mlp` rewritten with its backward pass gone, `softmax` (cross-entropy) and `svm` (hinge) added. Found on the way: a matrix literal's type had been a tensor of row tensors beside the flat form every operation answers, and the two had not fit each other under the IR check (`m = m - zeros[2, 2] * 0.5` with `m` a literal was an internal error in both modes): a tensor's element is a number, its rank the count of its dimensions.
- [x] **Rank is in the type** (Vijay, 2026-10-03: "make it consistent across compiled vs interpreted") — a loop over a tensor walks rows or numbers, and an index reads a row or a number, as the type decides, in both modes: every builtin's answer has a rank (`row_dims`: an element-wise map keeps its argument's shape, a reduction along an axis drops one, a constructor has a dimension per size, `*` is a matrix), a gradient is shaped like its parameter, a fact's tensor like its declared value, and a rank one call states is kept where another knows nothing; where the rank is still unknown at a loop or an index the program is refused before it runs, with the fix (`reshape`, `flatten`); both backends read a tensor through `crow_runtime::core::walk_rows`/`walk_numbers`/`row_at`/`number_at`, which stop with the same words when a value's rank disagrees with its type (spec, Arrays, Design Note "rank is in the type"; parity 156). The interpreter had decided by the value — rows by its rank, a whole number as an int — so a loop over a matrix the type knew no rank for walked rows interpreted and numbers compiled, and `type[x]` of a float tensor's item was `int` interpreted and `float` compiled. Found on the way: a row assigned into a tensor (`m[1] = [7.0, 8.0]`) stopped interpreted and did not build compiled — refused before running.
- [x] **The fuzzer reaches tensors, frames and function values** (Vijay, 2026-10-03: "implement items 1,2" — the fuzzer first; the second item, a file's own function losing to a bare import, turned out not to exist: a user module's names are always qualified) — `crow test --fuzz` now generates vectors and matrices of fixed shape (their arithmetic, reductions, rows and numbers walked and indexed, a record holding them, element assignment), frames built in the program (`with` on `frame.new[]`; filtered by their own columns, sorted, cut, stacked, grouped by two keys, pivoted, windowed, their cells and types read), and functions handed as values through a relay. The first three hundred seeds found eleven bugs, all fixed with tests: a failed assertion read both its sides again for its message (a side that moved a local said `cannot access field`; parity 158); a list of rows made of a local moved it; a list or a map of ints handed a float — `push`, `set`, `concat`, `contains` — kept an int among floats interpreted and did not build compiled, now a holder of floats (`Convert::IntsToFloats`; parity 159); a record's float field given an int that can fail, and a function of floats returning one, were left ints; a function handed as a value beside one answering floats answers floats; the lowering's temporaries (`t1`) shadowed a function of that name compiled; `%` of a float stopped interpreted where the compiled program computed it — refused before running now, as the spec's ints-only remainder says; `len` of a frame's column was refused interpreted (the column reader was a stub); a tensor that can fail assigned to a variable holding a list, or pushed to, was typed a list (the conversions ran after a local bound anew returned early, and `push` read a failing holder as nothing); and `type[x]` compiled decided its word without reading `x`, so a stop inside `x` never came (parity 160). Three hundred seeds now agree.
- [x] **The standard library's own layer: `use sys`** (Vijay, 2026-10-03: "reduce the Rust dependency … implement it") — design rule 20: formulas in Crow, primitives in Rust, laws in Crow for both. The first primitives: bytes (`bytes`, `text`, `slice`, `find`, `concat`, `byte`, `len`, `eq`) and a connection to a host, plain or behind TLS (`open`, `send`, `recv`, `close`), in both modes through `crow_runtime::sys`. Only the compiler's own modules may import `sys` (a program's `use sys` is refused before it runs, with the way a program reaches what `sys` builds; the tests and the book have the harness's door, `CROW_SYS=1`). `tests/sys_stream.rs` runs a script against an echo server in both modes and checks the door. Next on this layer: the HTTP client and `io` rewritten in Crow as the proof, then the server and the drivers once interpreted speed and parity cost are measured — and the formula modules (`math.nn`, `chart`, `math.stats`) moved to Crow with laws, which need no new primitive at all.
- [x] **`math.nn` written in Crow** (Vijay, 2026-10-04: "ok implement it" — the first module moved, in the order recommended: nn, then the HTTP client with ClickHouse and S3 behind it, mail, the WebSocket client, stats and the indicators) — the eight functions are formulas in `src/modules/nn.co` over the math builtins; each keeps its row in the builtin table as its contract (a row whose body is Crow, `crow!`), inlined into every program that imports it, both modes alike, its stops naming the line of the call; the Rust module and its seven hand-written backward rules are gone, replaced by the kernels the formulas need under `grad` — `clip`, `erf`, `max` and `min` whole and along an axis, `floor`/`ceil`/`round` as constants, `onehot` (new, a math builtin; the models package's own one-hot is gone for it, one way) — each checked against finite differences; `halt[reason]` for a module's own precondition (Design Note: `halt` beside `assert`; `stop` is a trading program's stop-loss); the module's tests are its own, run by the gate. Found on the way: a function only a test called, or one whose parameter nothing had typed, kept a guessed type (`x - max[x]` a number) as its return — arithmetic on a value nothing has typed is now untyped, a module call with such an argument is typed by what its rows agree on, and a test's call widens the shape a program's call fixed; an indexed call (`f[x][0]`) never typed its callee; a gradient list's items took a vector's rank from one parameter beside matrices; `len` of a traced tensor read through its row interpreted; a numeric list literal in a traced copy did not build; `erf` was an approximation good to 1.5e-7 (libm's now, to the bit).
- [x] **The HTTP client written in Crow** (Vijay, 2026-10-04: the second module of the plan) — `request` is a formula in `src/modules/core.co` on the `sys` layer: the URL taken apart, a connection opened plain or behind TLS, the request line, headers and body sent (a JSON body as JSON, a form URL-encoded, files as a multipart body), the answer's head read to its blank line and its body by its length, by its chunks or until the server closes, redirects followed, a success's bytes saved; its row in the builtin table is the contract (the nine parameters, an error value of kind `'http'`), and `tests/http_request.rs` passes in both modes as it did against the Rust client, which is gone (`ureq` stays for the readers' URLs until they move). The layer grew what the client stands on — `gather`, `file`, `store`, `random`, `hex`, `text[bytes, fill]`, `hash` and `hmac` of bytes (S3's signature next), `reply` — and the registry gained scopes: a standard module written in Crow resolves its names in its own imports (`sys`, which a program never sees) and the program in its own; a core builtin written in Crow is inlined where a program calls it, and the library's own file may define its row's function (`fn request`, tested on its own). Found on the way, each fixed with its test: a parameter named like an effect, indexed (`query[0]`), was refused as the effect; a call of a name a module in scope also answers (`slice` of text beside `slice` of bytes) was typed by the module's row though both backends pick the core's; a value nothing had typed yet was typed by the module's row where the core's differ; bytes and a connection passed to a function, or placed in a list, were moved in the compiled program where any other value is cloned; the answer's record type was named where no Rust row made it; `\r` is an escape now (a wire protocol's line ends `\r\n`).
- [x] **The readers and S3 written in Crow** (Vijay, 2026-10-04: the third step of the plan — "the HTTP client with ClickHouse and S3 behind it") — the protocol is `sys`'s own, in `src/modules/sys.co` (`exchange`, `fetch`, `put`, redirects, an S3 object signed as Signature Version 4 says and checked against AWS's documented example), and every reader of a file, a URL or an S3 object is a few lines of Crow over it: `read` and `write` (core.co), `io`'s `csv` and `lines`, `frame`'s `load` and `save`, `math`'s `load`, the parsers as `sys` kernels; `ureq` is gone from the runtime. A module written in Crow is read in its own scope (a program names its functions as it likes; parity 162) and inlined only where the program reaches its rows. ClickHouse over HTTP waits: it is the session layer's driver, to be measured first.
- [x] **Tokens and msgpack written in Crow** (Vijay, 2026-10-04: "fix the mail, websocket, jwt, msgpack to use native" — the first two of the four) — `web.auth`'s `sign` and `verify` in `src/modules/auth.co` (HS256 over `hmac`, base64url and a constant-time compare, checked against jwt.io's example), `msgpack` in `src/modules/web.co` (each piece in its smallest form, nil for null); a row that takes a value JSON can hold hands its function the JSON text; the IR verifier holds a row written in Crow to its contract; `sys` gains `base64`, `base64url`, `unbase64`, `same`, `pack`, `ieee` and `binary`. jsonwebtoken, rmp-serde and the dead `web.client` module (reqwest's blocking client) are gone from crow_web. Next: the WebSocket client, then mail, then the note on what stays native.
- [x] **The WebSocket client written in Crow** (Vijay, 2026-10-04: the third of the four) — `web.ws`'s `socket`, `send`, `recv` and `close` in `src/modules/ws.co`: RFC 6455's framing over the stream the handshake upgraded (`sys.upgrade`, written in Crow in sys.co over a `read_head` split out of the answer reader), a message sent masked, the next read whole across its fragments, a ping answered, a close answered; the socket handle (`sys.socket_of`, `stream_of`, `leftover`, `unread`) and the kernels `mask` and `digest` (sha1 joins `hash`). A name two modules in scope provide is picked by the arguments' kinds, in the lowering, the checker and type inference (sys's `send` of a stream beside a socket's); a read given no seconds waits as its connection does. tungstenite is gone from crow_web (it stays in the compiler for the served harness, and in tests as the echo server). Next: mail, then the note on what stays native.
- [x] **The estimators people reach for** (Vijay, 2026-10-03: "implement the rest of them") — the models package grown to twenty-one files: gradient boosting (regression trees on residuals), the lasso and elastic net (coordinate descent), a radial-kernel SVM (projected gradient on the dual) and support vector regression (by `grad`), DBSCAN, agglomerative clustering (three linkages), a Gaussian mixture (EM from k-means' first rounds), an isolation forest, k-fold cross-validation and a parameter search over it (functions handed as values), and polynomial features, one-hot columns and label codes in `prepare`; each with its test, every one in the example, both modes. Not built: the interop bridge (a Python call, an ONNX model) — the in-language route was taken.
- [x] **The decisions of 2026-10-01** (Vijay, as recommended) — an item read refuses a value that failed, as a field read does (parity 144); a failed route answers an id, its reason in the server's log beside it; a package's own dependencies install beside it, one version of a package to a project; every reader takes `headers = map[…]` for a URL (`tests/reader_headers.rs`).

Decided and kept: `connect` stops rather than failing as a value, a function has one copy whatever the shapes it is handed, and `stream[feed, a, …]` hands its feed `put`. And (Vijay, 2026-10-01): the WebSocket client is `socket[url]`; `len` of a matrix is its rows, as a loop walks it; a form's fields are `req.form`; a JSON table's columns come in name order, as a map's keys show; under `crow test --fixtures` a sqlite file stays the program's own.

Not in 1.0: CI on every push waits (the local gate is the check), and so does the distributed runtime (P6).

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
`request[url, method, body]` (spec, Requests): any method, with headers, a query, a form, files, a timeout, and a download saved to a file, answered as a record of `status`, `body` and `headers`. The readers (`read`, `load`, `csv`, `lines`) take a URL with headers, a query and a timeout. (A `web.client` module with `get`, `post`, `put` and `del` had been planned here; Design Note: one `request`, its method named.)

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
- [x] **`msgpack[value]`** — MessagePack binary response: `CrowResponse::msgpack_of` packs the value as JSON holds it (`rmp-serde`, crow_web's `msgpack` feature), and both modes call it.
- [x] **`arrow[frame]`** — Arrow IPC response for DataFrames: `CrowResponse::arrow_of` writes `crow_runtime::frame::to_arrow_ipc()` (crow_web's `arrow_ipc` feature, which turns on crow_runtime's `arrow`), and both modes call it.
- [x] **`bytes[data]`** — Raw binary response with `application/octet-stream` content type.
- [x] **`stream[feed, a, …]`** — A text response sent as `feed[put, a, …]` puts it: `CrowResponse::stream` holds the feed, and the server runs it on a blocking thread when the response is sent, each `put` a chunk down a channel into `Body::from_stream` (crow_web's `http` feature). `put` answers whether the reader still listens.
- [x] **Both modes** (Sept 30 2026) — the interpreter answers all four through the same crow_web functions (it had stopped on `msgpack`, `arrow` and `stream`, and before that answered them with a map that only looked like a response); the compiled `stream` had not built; a feed's loop calling `put` had been marked parallel; a compiled route whose handler's value can fail had not built. `tests/served/21_formats` holds both modes to the same bytes.

#### Client-side WASM — rejected
Rejected in favor of the Crow UI framework (see below). Building a WASM frontend pipeline would require `wasm-bindgen`, `web-sys`, and a reactive runtime — essentially building React/Svelte from scratch. The UI framework approach achieves the same goal (one-language full-stack) with pure Crow libraries and server-rendered HTML.

#### P8 — UI Framework ✅ (Phases 1–5 done; `view` blocks, `later`, style and chart modules shipped since — see the status table above)

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

##### Phase 4: Real App Features ✅

- [x] **Sessions** — the session layer: per-reader facts, `reader`, `sign_in`/`sign_out` (docs/session-layer-design.md)
- [x] **Auth** — a page for signed-in readers checks `reader` in its view: it draws the sign-in form in place, offers none of the page's handlers and runs none of its queries (spec, Who the reader is). `auth = true` on a page is rejected (Design Note: no `auth` on a page)
- ~~**Channels**~~ — rejected (Design Note: no channels): a write already reaches every reader, and a process outside the program is seen through `fresh`
- ~~**Debounce**~~ — rejected (Design Note: no debounce): a derivation has one recompute in flight, the latest input winning, and a second click on a running handler is dropped
- [x] **Loading states, file upload** — `running[h]`, uploads
- [x] **Keyboard events** — a key presses a drawn element: `shortcut = 'ctrl+s'` (drawn as `aria-keyshortcuts`), Escape on a dialog's Close and the runtime's question, Enter in a form presses its first button, and a form never reloads the page; `keydown` handlers rejected (Design Note: keys press elements)
- [x] **Row click** — `table[frame, click = h, key = 'col']`, row actions
- [x] **Table features** — `sort = true`: a header sorts the rows by its column, and the session keeps the order as it keeps a page (spec, Tables over frames); filtering stays the program's (Design Note: no filter on a table)
- [x] **Modals, pagination** — `modal[child, open = var]`, `table[frame, page = 50]`
- [x] **Accessibility** — audited: a field's label names its input, a click on any element is pressable from the keyboard, a modal holds and returns focus, toasts are announced, charts are named, a reconnecting page takes no keyboard input, an `img` needs `alt`, and the light theme's accent meets AA contrast (spec, Accessibility)

##### Phase 5: Developer Experience ✅

- [x] **`crow dev`** — serves the program with the interpreter and reloads it as its files change: each change checked first, every tab's session kept across the restart, a refused change shown over the open pages while the last good program serves, a change to the page's shell reloading the tabs (spec, Execution Modes)
- [x] **Chart engine** — shipped as the `chart[frame, ...]` element (server-rendered SVG, `chart-module-design.md`)
- [x] **Dev tools** — the dev panel under `crow dev` (Ctrl+Shift+D): the session's facts live, each event with the facts it changed and its render's time and size, the renders' timing; it watches only (Design Note: the dev panel watches)

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

### P6 — Distributed runtime (after 1.0)

- [ ] **Multi-node execution**: `nodes[n]` directive exists but needs real multi-node testing.
- [ ] **Data sharding**: Verify automatic data partitioning across nodes.
- [ ] **Gradient synchronization**: Verify all-reduce for distributed training.
- [ ] **Strategy directives**: Test pipeline parallelism, tensor parallelism beyond data parallelism.

### P7 — Ecosystem (before 1.0)

- [x] **Packages** (Sept 30 2026, Vijay: git tags and a lockfile): a package is a git repository, a version its tag, and `crow.lock` pins the commit each dependency checked out; `crow install --update` moves them. No registry (the central index `crow install` had read is gone): a dependency names its repository, a directory inside it on a forge (`github.com/org/repo/sub/dir@v1`).
- [x] **Seed packages** (Sept 30 2026, Vijay: models, market data, data connectors), in `packages/`, each held to both modes by `crow test --packages`:
  - [x] `models`: least squares, logistic regression, k-means, a decision tree, a one-hidden-layer network.
  - [x] `market`: the US equity sessions and the NYSE calendar (holidays, early closes, `phase`), bars from trades and from finer bars.
  - [x] data connectors (built into the language, not a package): every reader takes a URL or an `s3://` object as it takes a file, and `load`/`save` read and write CSV, JSON records and Parquet by the extension (spec, Files, URLs and S3).
- [x] **Notebook integration** (Sept 30 2026, Vijay: a Jupyter kernel): `crow kernel install`; a notebook is one interpreter session, a cell's logs stream, its value is its result (a frame as a table), an interrupt stops a running cell (spec, Notebooks).
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
- **Stdlib (7/10):** JSON (`json[str]`, `encode[value]`), env vars (`env[name]`, `env[name, default]` + `.env` auto-loading), CLI args (`args[]`), logging (`log[]`), file I/O (`read`, `write`, `exists`, `mkdir`, `ls`, `rm`, `mv`, `cp`), regex (`find`, `matches`, `sub`), date/time (`now`, `date`, `elapsed`), shell (`shell[cmd]`), hashing (`hash[data, algo]`), input (`input[]`). Covers the practical needs for scripting and application development. Still below Python's 200+ modules — missing process management and some niche utilities (the HTTP client is `request`, 2026-10-01).
- **Web (8/10, 2026-10-02):** live pages with facts, derivations and handlers as transactions; routes with JSON, binary and streaming answers, CORS, static files, rate limits, uploads and tokens; `widget` as the one door to the program's own JavaScript; apps driven and laid out in tests without a browser, fixtures replayed, baselines diffed; one process, measured (above). Behind the best stacks on client-side reach (no other client code by design), auth beyond a password and a token, and running on more than one machine.
- **DB (8/10):** Top-level `db` module with connection pooling, parameterized queries, transactions, bulk insert, and forward-only migrations. All three backends (SQLite, PostgreSQL, ClickHouse) fully implemented and live-tested. No ORM (by design — Crow uses raw SQL). 8/10 because the core DB story is complete across OLTP and OLAP engines; missing only advanced features like connection-level event listeners or prepared statement caching.
- **Ecosystem (1/10):** No real package registry, no community packages, no third-party libraries. `crow install` exists as a command but there's nothing to install. This is the single biggest gap.

**What moves the needle:**
- Ecosystem → 5/10 would require a working registry + 20-30 community packages
- Web → 8/10 would require end-to-end app testing and the UI framework (P8)
- Stdlib → 8/10 would require process management and a few more utilities (the HTTP client landed 2026-10-01)

P0–P5 are complete. P6 (distributed verification), P7 (ecosystem), and P8 (UI framework) are what separate Crow from a production-ready tool.

