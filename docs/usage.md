# Crow Toolchain Guide

This guide covers the tools: installing Crow, every `crow` command, project
layout, the crowfile, and the two execution modes. It does not teach the
language — that is [The Crow Book](book/src/SUMMARY.md); the reference is
[language-spec.md](language-spec.md).

## Installation

Crow builds from source with the Rust toolchain. `cargo` stays a real
dependency after installation: `crow run` compiles your program to Rust.

```bash
git clone <the crow repository>
cd crow
cargo install --path .        # installs ~/.cargo/bin/crow
```

Make sure `~/.cargo/bin` is on your `PATH`, then run `crow` with no
arguments — you should see the usage help. (`cargo build --release` instead
leaves the binary at `target/release/crow` without installing it.)

Editor support is a subcommand of the same binary: `crow lsp`. See
[crow-lsp.md](crow-lsp.md) for VS Code, Neovim, Helix, and Zed.

## Quick start

```bash
echo "log['hello, world']" > hello.co
crow run hello.co           # transpile → compile → run (native speed)
crow run -i hello.co        # interpret (instant start)
```

## Commands

### Running and building

| Command | What it does |
|---|---|
| `crow run file.co` | Transpile to Rust, compile, run. The build is cached in `.crow_build/`; a second run of an unchanged file starts immediately. |
| `crow run -i file.co` | Run in the interpreter — instant start, same output (both modes are held to identical behavior). |
| `crow run` | Run the project in the current directory: the crowfile's `entry`, `main.co` by default. |
| `crow run <job>` | Run a named job from the crowfile's `jobs` map, e.g. `crow run train`. Jobs run and exit; `main.co` is the thing that stays running. |
| `crow build [file.co]` | Compile without running. The binary lands in the current directory, named after the file (`hello`); for a file outside the directory the name is path-mangled (`examples_hello`). |
| `crow stop` | Stop the running app — by pidfile, then by the project binary's name. `crow run` on a busy port tells you to use it. |
| `crow launch N file.co` | Build, then run on N distributed processes (`nodes[N]` programs). |
| `crow` | The REPL: variables and functions persist between lines, expression results print automatically. |
| `crow kernel install` | Make Crow a Jupyter kernel: notebooks, JupyterLab, VS Code's notebooks and `jupyter console` then offer *Crow*. Each notebook is one REPL session; a cell's logs stream under it, its last value is its result (a frame as a table), and interrupting stops a running cell. |
| `crow book [--port N]` | Render the book with `docs/book/site.co` and serve it with every code block runnable in place: ▶ Run interprets, ⚙ Compile builds the native binary, ✓ Test runs the block's test blocks, every block is editable in place with line numbers (Tab indents, Ctrl/Cmd-Enter runs, ↺ Reset restores the book's text), ■ Stop kills a served app. A run of the unedited example reports whether its output matches the book. Builds the HTML with `mdbook` if needed. Default port 3008. |

### The book is a Crow program

The book's chapters are Markdown under `docs/book/src`. `docs/book/site.co` renders them with `use doc`: it reads the summary and every chapter, lays out each page with the layout vocabulary (menu left, chapter right, menu hidden below tablet width), and writes static HTML to `docs/book/crow-site`. Every runnable code block is rendered as the runner itself — the block is the editor with a line gutter, a toolbar row below it (▶ Run, ⚙ Compile, ✓ Test, ↺ Reset, ■ Stop), then the output panel and the book's expected output — all of it markup from the kernel, so `crow test docs/book/site.co` checks where those parts sit at two widths without a browser. A small script inside every page only carries the block's text to the server and the reply back.

```text
crow book                          # renders the site, then serves it with the runner live
crow test docs/book/site.co      # the page and runner layout at 1280px and 390px
crow run -i docs/book/site.co    # just the pages, in well under a second
crow run docs/book/site.co       # compiled: byte-identical pages
```

Without `crow book`, the pages still read fine from disk and the buttons say how to start a runner. Search runs over `search.json`, written by the same program; the top bar's `/` focuses it. The book has no other build: mdBook is gone.

Both `crow run` and `crow build` load a `.env` file from the current
directory if one exists (`KEY=VALUE` lines, `#` comments; real environment
variables win).

### Verifying

| Command | What it does |
|---|---|
| `crow test [file.co]` | Run the `test` blocks you wrote (book chapter 4). Without a file, uses the crowfile entry. |
| `crow check [file.co]` | Everything automatic on top of your tests: edge cases for pure functions, determinism contracts, live-loop hazards (blocking calls in views, stale snapshots), and the app baseline — what every handler, view, fact and page does, derived from the source, recorded under `.crow/baseline/` on the first check and diffed on every check after, each difference with its cause and its reach. Interactive: `enter` rechecks, `s` skips, `f` applies a fix, `a` accepts a baseline change. |
| `crow check --accept` | Accept every baseline change at once — after a deliberate change, or from a script. Commit `.crow/baseline/` with the change; it is the record of what the app does. |
| `crow book [--port N]` | Inside an app: its own book, derived from the source on every start and served next to the app — on its `serve` port plus one, so finch on 3100 has its book on 3101 — the app and its parts, each page's composition tree and wireframes at desktop and phone widths, a chapter per part (facts, derivations, handlers as paths, views with trees and wireframes, tests as steps), the data wiring, and the changes since the accepted baseline. Nothing runs against a database. `--write` builds `.crow/book/site` without serving. In the crow repository, the Crow Book. |
| `crow stop book` | Stop the book this project is serving (`crow book` writes `.crow/book.pid`; Ctrl-C works too). `crow stop` stops the app and says if the book is still up. |
| `crow check --show [name]` | The app as a tree: each page, the views it composes under their guards, the inputs they bind and the controls with the handlers they fire; then every handler with what it writes and does, every fact with who binds, writes and reads it, every derivation with what it reads. With a name (`--show buy`), one declaration's full artifact: a handler's paths, a view's tree. |
| `crow check --db <url>` | Adds the schema-aware database checks — write skew, lost updates, one-way latches, assumed-unique reads, unpaired writes, plus column/table verification. Or put `db = '…'` in the crowfile. See [crow-check.md](crow-check.md). |
| `crow check --strict` | Exit 1 on any finding (for CI). `--verbose` prints purity and snapshot details. |
| `crow fix file.co` | Apply, in place, the fixes the program's refusal names — a slip's nearest name (`did you mean`), a missing `use`, another language's habit in its Crow form (`==` → `eq`, `f(1)` → `f[1]`, `x += 1`, a dropped `push`, two statements on a line) — round after round, then list each change at its line and print what is left for you. The editor offers the same fixes one by one. A program that is not refused goes on to the database hazards. |
| `crow fix file.co --hazard N [--option M]` | Apply a fix from the check report (writes the migration, applies it, re-checks). |
| `crow db diff [file.co]` | For each connection the program declares tables on (its literal `CREATE TABLE` statements) and opens with a url it writes, the SQL that would make that database hold them: each missing table's `CREATE`, each missing column's `ADD COLUMN`, and, commented out, what reconciling will not make. Reads the catalog only. Opening a connection checks the same tables: run from source it reconciles, a built program verifies; `schema = '…'` in the crowfile or `CROW_SCHEMA` says otherwise (spec, db, The schema a program declares). |
| `crow test --parity` | The parity harness: every `tests/parity/*.co` runs in both modes and the outputs are diffed. A program both modes stop at the same point, with the same last line on stderr, agrees too; a crash never does. |
| `crow test --fuzz [n] [--seed s]` | Differential fuzzing: n programs (ten by default), each of twenty generated cases, run in both modes. A program the modes disagree on is kept under `target/crow_fuzz/` with the first line they differ on, and the run fails. Without `--seed` the seed is the time, which the report names, so any run can be repeated exactly. The compiled side builds with the dev profile, so an integer overflow stops instead of wrapping. |
| `crow test --fixtures [file.co]` | The tests where the database servers are not (CI, a laptop without the data): a connection to Postgres or ClickHouse reaches nothing, a read is answered from what `crow check --record-fixtures` kept in `.crow/fixtures`, one no fixture holds is the query's error (a failing test names the statement), and a write answers as one that landed. A sqlite file opens as always. The clock stands at 2026-01-01 and random draws are seeded, as when the fixtures were recorded; with `--compiled`, the apps the tests open answer the same way. |
| `crow check --record-fixtures` | The scenarios read the real databases, and what each statement answers first is kept in `.crow/fixtures` (its first 10,000 rows), for the rehearsals after and for `crow test --fixtures`. A write is recorded, never made. See [crow-check.md](crow-check.md). |
| `crow test --packages` | In the Crow repository: every package under `packages/` — each file's test blocks pass, and the package's `example.co` runs in both modes with the same output (a test block runs in the interpreter, so the example is what holds the package to the compiled program). `crow test --packages models` checks one. |
| `CROW_IR_SHOW=1 crow test file.co` | Print each test block's lowered program — the declarations it sees, then the block as the `Test` item it is — as the interpreter runs it; `crow build --ir` prints a whole file the same way. |
| `crow build --ir file.co` | Print the program's IR, the one typed form both execution modes are built from (`docs/ir-design.md`): types decided once, effects marked answered or held, propagation explicit, last reads as moves. Nothing is compiled. |
| `crow build --rs file.co` | Print the Rust the program compiles to: the IR above, printed by the Rust backend. Nothing is compiled. |
| `crow test --bench` | The benchmark programs in `tests/bench/` run in both modes (compiled as a release build) and each section's seconds are printed side by side; `--quick` runs small sizes, `--write` records the table in `docs/benchmarks.md` with the machine and the commit. |
| `crow test --book` | Executes every code block in the book in both modes and compares printed output with the book's text. |
| `crow test tests/layout/conformance.co` | The layout engine against a real browser: fixtures measured once in Chrome by `tests/layout/measure.py` (Playwright, developer-only) and checked in as CSV; the check itself needs no browser. Re-measure after a stylesheet or engine change. |
| `crow test --docs` | Holds the documentation to the compiler: every builtin, module function, and element must be in the spec; every spec reference row must exist in the compiler; every module and core builtin must have a verified book example; every example file must be indexed. Runs in CI. |

### Formatting

| Command | What it does |
|---|---|
| `crow fmt [file.co]` | Format the project (via crowfile) or one file: 2-space indentation, no trailing whitespace, at most one blank line, single trailing newline. Source-level, so comments and `then` chains are preserved exactly. |
| `crow fmt --check` | Exit 1 if anything needs formatting. |
| `crow fmt --fix-interpolation` | Migrate pre-backtick `{expr}` strings to `` `expr` ``. |
| `crow fmt --fix-views` | Migrate ui-building `fn` declarations to `view` declarations. |

### Projects and packages

| Command | What it does |
|---|---|
| `crow new myapp` | Scaffold a project: `crowfile` + `main.co`. |
| `crow new myapp --web` | Adds `views/home.co`, wired into `main.co` with `page['/', home.render]`. |
| `crow new myapp --ml` | Adds `jobs/train.co`, `lib/model.co`. |
| `crow new myapp --db` | Adds `db/schema.co`, `jobs/migrate.co`. Flags combine. |
| `crow install` | Install the crowfile's `deps` into `crow_modules/`, each pinned by `crow.lock`; `--update [name]` moves them to what their versions name now. |

Every scaffolded file is working code — `crow run` succeeds right after
`crow new`.

## Project structure

Any `.co` file is a program; a project is a directory with a `crowfile`.
Nothing is restructured as it grows — you add files:

```
myapp/
  crowfile              project config (see below)
  main.co             default entry point — the long-running thing (crow run)
  jobs/                 batch tasks: crow run train → jobs/train.co
  views/                web views: use views.home → page['/', home.render]
  lib/                  your code, any layout: use lib/accounts → accounts.open[...]
  db/schema.co        table definitions
  migrations/           .sql files applied by migrate[] and written by crow fix
  css/app.css           optional stylesheet overrides (inlined after the defaults)
  tests/                integration tests
  crow.lock             the commit each dependency checked out (commit it)
  crow_modules/         installed dependencies (crow install; not committed)
  .crow_build/          the generated Rust project — safe to delete
```

Modules are files. `use views.home` loads `views/home.co` and its
exports are reached qualified (`home.render`); slash paths work the same
(`use lib/utils` → `utils.helper[...]`). Only the standard library gets
bare names — see book chapter 8.

## The crowfile

The crowfile is a Crow file; the toolchain evaluates it and reads these
variables. `crow new` writes only `name`.

```
name = 'myapp'
entry = 'main.co'                 --- optional: main.co is the default
db = 'sqlite://dev.db'              --- optional: schema for crow check and the editor
cpu = 4                             --- optional: default thread count (integer)

deps = map[models = 'github.com/agncrow/crow-models@v0.1.0', utils = './libs/utils']
jobs = map[train = 'jobs/train.co', migrate = 'jobs/migrate.co']
```

Dependencies (`deps`) take two forms:

| Form | Example | How it is installed |
|------|---------|---------------------|
| Repository and version | `models = 'github.com/agncrow/crow-models@v0.1.0'` | Checked out at the tag (or branch, or commit), the commit pinned in `crow.lock` |
| Local path | `utils = './libs/utils'` | Linked into `crow_modules/` |

A package is a git repository and a version is its tag: there is no
registry. The first `crow install` of a dependency records the commit its
version names in `crow.lock`; every install after that checks out that
commit, so every machine runs the same code. On a forge the repository is
the first two path segments and the rest is a directory inside it
(`github.com/agncrow/crow/packages/models@v0.6.0`). Installed modules are
imported like any user module — qualified: `use models.line` then
`line.slope[...]`.

## The two execution modes

| | `crow run -i file.co` | `crow run file.co` |
|---|---|---|
| How | tree-walking interpreter | transpile → Rust → native binary |
| Startup | instant | first build compiles (cached after) |
| Speed | scripting-tier | Rust-tier; parallel `for` uses every core (rayon) |
| Use for | exploring, the REPL, quick scripts, `crow test` | anything you ship or benchmark |

The two modes are required to produce identical output; `crow test
--parity` and `crow test --book` enforce it. If they ever disagree, that is
a Crow bug, not a semantics to learn. Both modes share one front end and
one intermediate form: the compiler infers a concrete type for every value
before either mode runs, so they accept and reject the same programs with
the same messages.

The generated Rust lives in `.crow_build/` next to your source — you never
need to read it, but it is there.

### Environment

| Variable | Effect |
|---|---|
| `CROW_BUILD_PROFILE=dev` | Compile checked programs without optimisation (the verification harnesses set it). A dev build carries no debug info, which keeps the build caches small: on macOS every unit's object file is kept for it, and the harness caches had grown to hundreds of GB. |
| `CROW_DEBUGINFO=1` | Compile the program with debug info — the dev profile as cargo defines it — into `.crow_build/target/debuginfo/`, its own directory, so the plain dev cache is neither rebuilt nor grown by it. For a Rust backtrace with file and line (`RUST_BACKTRACE=1`) when a program panics, which is a bug in Crow, never an error a Crow program can make. The first such build compiles the runtime's crates again with debug info; later ones are incremental. When the bug is fixed, unset it and delete that directory. |
| `CROW_NO_DESCRIBE=1` | Do not ask the program's databases what each query returns at compile time; frame cells then read as text in the compiled program. |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` | The credentials an `s3://` path is read and written with (without them an object is read unsigned, as a public bucket's is). |
| `AWS_REGION` | The region of an `s3://` path's bucket (`us-east-1` when unset). |
| `AWS_ENDPOINT_URL` | An S3-compatible store (MinIO, R2) an `s3://` path is reached at, by path. |
