# Crow Toolchain Guide

This guide covers the tools: installing Crow, every `crow` command, project
layout, the crowfile, and the two execution modes. It does not teach the
language — that is [The Crow Book](book/src/SUMMARY.md); the reference is
[language-spec.md](language-spec.md).

## Installation

One line fetches the prebuilt binary for your machine from the latest
release into `~/.crow/bin`, held to its published hash:

```bash
curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | bash
```

Nothing else is installed: the interpreter, the tests, `crow check`, the
language server and served apps work at once. Compiling to a native
binary needs a Rust toolchain: the first `crow build` fetches a pinned one
from Rust's own servers into `~/.crow/toolchain` (a `cargo` already on the
PATH serves instead), with the release's runtime bundle into
`~/.crow/runtime`, once, each file verified; `crow setup` does it up
front, `crow setup --check` says what is present, and the one thing asked
of the machine is a linker (the Xcode command line tools, or
`build-essential`). Until the compiler is there, `crow run` says so and
interprets, which answers the same, only slower. `CROW_HOME` moves the
directory. The install script takes `--version X` for another release,
and `--uninstall` removes `~/.crow` and crow's binary in `~/.crow/bin`
and `~/.cargo/bin`, and leaves a Rust toolchain of your own (`curl -fsSL
…/install.sh | bash -s -- --uninstall`).

`crow upgrade` replaces that binary with the latest release, held to its
published hash as the install line holds it; `crow upgrade --version X`
installs a given release (a downgrade too), and `crow upgrade --check`
says whether a newer one is out. The new version fetches its own
toolchain and runtime the same way, on its first compiled build.

From a checkout, `./install.sh --source` builds Crow with the machine's
Rust (a C toolchain and Rust are installed first where they are missing),
installs it in `~/.cargo/bin` and snapshots the source to `~/.crow/src`
for compiled programs to link, and `./install.sh --dev` builds the
checkout in place for working on Crow itself (`cargo build --release`
leaves the binary at `target/release/crow` without installing it; it
links the checkout it was built from). Either upgrades by `git pull` and
the same command again: `crow upgrade` replaces only the binary the
install line put in `~/.crow/bin`, and says so.

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
| `crow run file.co` | Transpile to Rust, compile, run. It is a dev build — the runtime crates at optimization level 1, the program at 0, no LTO — so the edit loop stays short. The build is cached in `.crow_build/` in the directory you run it from; a second run of an unchanged file starts immediately. Words after the file are the program's arguments (`args[]`). On a machine with no compiler yet, it says so and interprets. |
| `crow run --opt N file.co` | The same, the program optimized at level N (0 to 3, 0 unless said; the runtime crates follow above 1): a program whose own loops are the work runs many times faster at 1. `crow build` is the shipped binary, level 3 with LTO. |
| `crow run -i file.co` | Run in the interpreter — instant start, same output (both modes are held to identical behavior). |
| `crow run` | Run the project in the current directory: the crowfile's `entry`, `main.co` by default. |
| `crow run <job>` | Run a named job from the crowfile's `jobs` map, compiled, e.g. `crow run train` (`crow run -i jobs/train.co` interprets it). Jobs run and exit; `main.co` is the thing that stays running. |
| `crow build [file.co]` | Compile without running: the release build, level 3 with LTO. The binary lands in the current directory, named after the file (`hello`; `main` for a project); for a file outside the directory the name is path-mangled (`examples_hello`). |
| `crow dev [file.co]` | Serve the program with the interpreter and reload it as its files change. A change is checked first, by the gate `crow run -i` passes; a program that passes replaces the one serving, and every tab reconnects and resumes its session. A change that breaks the program leaves the last good one serving, its pages showing the refusal until a change mends it. A change to what a page's shell carries (`css/app.css`, a widget's script) reloads the tabs, and a program that serves nothing runs again. Ctrl+Shift+D opens the dev panel over the page. Nothing is compiled. |
| `crow stop` | Stop the running app — by pidfile, then by the project binary's name, then a program the interpreter serves (`crow run -i`) and `crow dev`'s watcher with it; only processes running in this project's directory. `crow run` on a busy port tells you to use it. |
| `crow launch N [file.co]` | Build the release binary, then run it as N processes, each reading its rank with `env['CROW_RANK', '0']` (`CROW_WORLD_SIZE`, `CROW_MASTER_ADDR` and `CROW_MASTER_PORT` are set beside it). Without a file, the crowfile's entry. |
| `crow` | The REPL: variables and functions persist between lines, expression results print automatically. A paste of several lines runs as one cell; a block (`fn`, `view`, `test`, `if` with its `else`) ends at a blank line. The console serves no pages: a program that serves is told to run from its file (`crow run -i app.co`, or `crow dev app.co`). |
| `crow kernel install` | Make Crow a Jupyter kernel: notebooks, JupyterLab, VS Code's notebooks and `jupyter console` then offer *Crow*. Each notebook is one REPL session; a cell's logs stream under it, its last value is its result (a frame as a table), and interrupting stops a running cell. (`crow kernel <connection file>` is the kernel itself, as Jupyter starts it.) |
| `crow book [--port N]` | In the Crow repository: render the book with `docs/book/site.co` and serve it with every code block runnable in place: ▶ Run interprets, ⚙ Compile builds the native binary, ✓ Test runs the block's test blocks, every block is editable in place with line numbers (Tab indents, Ctrl/Cmd-Enter runs, ↺ Reset restores the book's text), ■ Stop kills a served app. A run of the unedited example reports whether its output matches the book. The runner answers only the book's own page on this machine. Default port 3008; `--write` renders without serving. Inside an app, its own book (below). |
| `crow help` | The commands, in brief. `crow -v` prints the version. |

Every `crow` command loads a `.env` file from the current directory if one
exists (`KEY=VALUE` lines, `#` comments, quotes around a value dropped;
real environment variables win).

### The book is a Crow program

The book's chapters are Markdown under `docs/book/src`. `docs/book/site.co` renders them with `use doc`: it reads the summary and every chapter, lays out each page with the layout vocabulary (menu left, chapter right, menu hidden below tablet width), and writes static HTML to `docs/book/crow-site`. Every runnable code block is rendered as the runner itself — the block is the editor with a line gutter, a toolbar row below it (▶ Run, ⚙ Compile, ✓ Test, ↺ Reset, ■ Stop), then the output panel and the book's expected output — all of it markup from the kernel, so `crow test docs/book/site.co` checks where those parts sit at two widths without a browser. A small script inside every page only carries the block's text to the server and the reply back.

```text
crow book                          # renders the site, then serves it with the runner live
crow test docs/book/site.co      # the page and runner layout at 1280px and 390px
crow run -i docs/book/site.co    # just the pages, in well under a second
crow run docs/book/site.co       # compiled: byte-identical pages
```

Without `crow book`, the pages still read fine from disk and the buttons say how to start a runner. Search runs over `search.json`, written by the same program; the top bar's `/` focuses it. The book has no other build.

### Verifying

| Command | What it does |
|---|---|
| `crow test [file.co]` | Run the `test` blocks you wrote (book chapter 9). Without a file, uses the crowfile entry; a part's tests run from the part. The blocks, and the apps they open with `app[view]`, run in the interpreter. |
| `crow test --compiled file.co` | The same blocks with every app they open compiled: each block runs in the interpreter, and its harness verbs reach the compiled program, started fresh for each run that opens one, so both modes answer the same test. |
| `crow check [file.co]` | Everything automatic on top of your tests: edge cases for pure functions (their answers kept as snapshots in `.crow/snapshots/`, a changed one reported), determinism contracts, live-loop hazards (a handler that blocks, a view serving a launch-time snapshot), scenarios — the named tests that open the app, run in a rehearsal that connects to nothing — and the app baseline — what every handler, view, fact and page does, derived from the source, recorded under `.crow/baseline/` on the first check and diffed on every check after, each difference with its cause and its reach. Interactive: `enter` rechecks, `s` skips, `t` tries the input a finding names, `a` accepts (a baseline change, a changed snapshot), `d` rewrites a launch-time snapshot as a derivation, `f` moves a handler's blocking call into `later[…]`, `q` quits. Without a terminal (a script, CI) it prints every finding without waiting, and exits 1 when there is one (a database finding only under `--strict`). |
| `crow check --accept` | Accept every baseline change at once — after a deliberate change, or from a script. Commit `.crow/baseline/` with the change; it is the record of what the app does. |
| `crow book [--port N]` | Inside an app: its own book, derived from the source on every start and served next to the app — on its `serve` port plus one, so finch on 3100 has its book on 3101 (3009 for an app that names no port) — the app and its parts, each page's composition tree and wireframes at desktop and phone widths, a chapter per part (facts, derivations, handlers as paths, views with trees and wireframes, tests as steps), the data wiring, and the changes since the accepted baseline. Nothing runs against a database. `--write` builds `.crow/book/site` without serving. In the crow repository, the Crow Book. |
| `crow stop book` | Stop the book this project is serving (`crow book` writes `.crow/book.pid`; Ctrl-C works too). `crow stop` stops the app and says if the book is still up. |
| `crow check --show [name]` | The app as a tree: each page, the views it composes under their guards, the inputs they bind and the controls with the handlers they fire; then every handler with what it writes and does, every fact with who binds, writes and reads it, every derivation with what it reads. With a name (`--show buy`), one declaration's full artifact: a handler's paths, a view's tree. |
| `crow check --db <url>` | Adds the schema-aware database checks — write skew, lost updates, one-way latches, assumed-unique reads, unpaired writes, plus column/table verification. Or put `db = '…'` in the crowfile; `migrations://dir` reads the schema from a directory's `.sql` files, offline. See [crow-check.md](crow-check.md). |
| `crow check --strict` | Exit 1 on any finding, a database finding included (for CI). `--verbose` prints purity and snapshot details, and every finding with its fix, without waiting. |
| `crow check --self` | In the Crow repository: Crow held to itself. About 100,000 cases written from the compiler's own tables — every builtin with the kinds it takes and their edges, every collection at every kind of index, every operator over every pair of operands, and families for served routes, views, pages, files, draws, gradients, the GPU and the standard modules — each run in five configurations that must agree (interpreted, walked, unfused, unmoved, compiled), its answer held to the spec's by `tests/self/reference.py`. `--only word`, `--lower-only`, `--bless`, `--clean`, `--memory GB`. See [crow-check.md](crow-check.md). |
| `crow fix file.co` | Apply, in place, the fixes the program's refusal names — a slip's nearest name (`did you mean`), a missing `use`, another language's habit in its Crow form (`==` → `eq`, `f(1)` → `f[1]`, `x += 1`, a dropped `push`, two statements on a line) — round after round, then list each change at its line and print what is left for you. The editor offers the same fixes one by one. A program that is not refused goes on to the database hazards. |
| `crow fix file.co --hazard N [--option M] [--db url]` | Apply a fix from the check report (writes the migration, applies it, re-checks). |
| `crow db diff [file.co]` | For each connection the program declares tables on (its literal `CREATE TABLE` statements) and opens with a url it writes, the SQL that would make that database hold them: each missing table's `CREATE`, each missing column's `ADD COLUMN`, and, commented out, what reconciling will not make. Reads the catalog only. Opening a connection checks the same tables: run from source it reconciles, a built program verifies; `schema = '…'` in the crowfile or `CROW_SCHEMA` says otherwise (spec, db, The schema a program declares). |
| `crow db apply <dir> <url>` | Run the `.sql` files of a directory that the database has not applied yet, in name order, each recorded in a `_crow_migrations` table, and say how many were new: the apply half of `crow db diff`. Migrations are the tool's, not a program's. |
| `crow db describe <url> <sql>` | The columns a statement answers on that database, as JSON name and kind pairs, without running it: what the compiler asks of a database whose driver is written in Crow, from a process of its own. |
| `crow test --parity [word…]` | The parity harness: every `tests/parity/*.co` runs in both modes and the outputs are diffed; words after it pick programs by name (`crow test --parity 97 shapes`). A program both modes stop at the same point, with the same last line on stderr, agrees too; a crash never does. |
| `crow test --served [word…]` | Every page in `tests/served/`, and every app under `examples/` with a `main.events` script beside its `main.co`, served by the compiled program and by the interpreter, driven with the same events, and the pages they settle on after each step compared; words pick pages by name. |
| `crow test --fuzz [n] [--seed s] [--keep]` | Differential fuzzing: n programs (ten by default), each of twenty generated cases with a module of its own beside it, run in both modes, and one page per ten programs served in both modes and driven alike. A program the modes disagree on is kept under `target/crow_fuzz/` with the first line they differ on, and the run fails; `--keep` keeps every program. Without `--seed` the seed is the time, which the report names, so any run can be repeated exactly. |
| `crow test --fuzz n --mistakes` | Generated programs with one mistake planted in each (a slip in a name, a call written with `( )`, another language's operator, a bracket left open, two statements on a line, an import left out), each refusal held to the rules every message keeps — refused once, at the mistake's line, naming only what the program wrote, the same in both modes and in the language server — and to the fix the mistake asks for. |
| `crow test --errors [--bless] [word…]` | In the Crow repository: the wrong programs of `tests/errors/`, each run in both modes and through the language server, its refusal held to what was reviewed (`<name>.expect`) and to the rules every message keeps; `--bless` writes what the programs answer now, for review. |
| `crow test --fixtures [file.co]` | The tests where the database servers and the model are not (CI, a laptop without the data): a connection to Postgres or ClickHouse reaches nothing, a read is answered from what `crow check --record-fixtures` kept in `.crow/fixtures`, one no fixture holds is the query's error (a failing test names the statement), and a write answers as one that landed; a request is answered from its fixture the same way, and one no fixture holds is the request's error value. A sqlite file opens as always. The clock stands at 2026-01-01 and random draws are seeded, as when the fixtures were recorded; with `--compiled`, the apps the tests open answer the same way. |
| `crow check --record-fixtures` | The scenarios read the real databases and send their requests, and what each statement answers first (its first 10,000 rows) and what each request answers are kept in `.crow/fixtures`, for the rehearsals after and for `crow test --fixtures`; the tests that open no app run too, for their fixtures alone. A write is recorded, never made. See [crow-check.md](crow-check.md). |
| `crow test --packages` | In the Crow repository: every package under `packages/` — each file's test blocks pass, and the package's `example.co` runs in both modes with the same output (a test block runs in the interpreter, so the example is what holds the package to the compiled program). `crow test --packages models` checks one. |
| `crow test --modules [word…]` | In the Crow repository: every standard module written in Crow (`src/modules/*.co`) runs its own test blocks; words pick modules by name. |
| `CROW_IR_SHOW=1 crow test file.co` | Print each test block's lowered program — the declarations it sees, then the block as the `Test` item it is — as the interpreter runs it; `crow build --ir` prints a whole file the same way. |
| `crow build --ir file.co` | Print the program's IR, the one typed form both execution modes are built from (`docs/ir-design.md`): types decided once, effects marked answered or held, propagation explicit, last reads as moves. Nothing is compiled. |
| `crow build --rs file.co` | Print the Rust the program compiles to: the IR above, printed by the Rust backend. Nothing is compiled. |
| `crow build --types file.co` | Print every type inference settled, in the compiler's own names, each body's locals included. Nothing is compiled. |
| `crow test --bench` | The benchmark programs in `tests/bench/` run in both modes (compiled as a release build) and each section's seconds are printed side by side; `--quick` runs small sizes, `--write` records the table in `docs/benchmarks.md` with the machine and the commit. |
| `crow test --book` | Executes every code block in the book in both modes and compares printed output with the book's text. |
| `crow test tests/layout/conformance.co` | The layout engine against a real browser: fixtures measured once in Chrome by `tests/layout/measure.py` (Playwright, developer-only) and checked in as CSV; the check itself needs no browser. Re-measure after a stylesheet or engine change. |
| `crow test --docs [--write]` | Holds the documentation to the compiler: every builtin, module function, and element must be in the spec; every spec reference row must exist in the compiler; every module and core builtin must have a verified book example; every example file must be indexed; and the standard library's pages (`docs/book/src/stdlib/`) must be as the spec generates them — `--write` regenerates them. Runs in CI. |

### Formatting

| Command | What it does |
|---|---|
| `crow fmt [file.co …]` | Format the files named, or the crowfile's entry: 2-space indentation, no trailing whitespace, at most one blank line, single trailing newline. Source-level, so comments and `then` chains are preserved exactly. A file that does not parse is left as it is, and the run stops at it with its error. |
| `crow fmt --check` | Exit 1 if anything needs formatting, naming each file; nothing is written. |
| `crow fmt --fix-interpolation` | Migrate pre-backtick `{expr}` strings to `` `expr` ``. |
| `crow fmt --fix-views` | Migrate ui-building `fn` declarations to `view` declarations. |

### Projects and packages

| Command | What it does |
|---|---|
| `crow new myapp` | Scaffold a project: `crowfile`, `main.co`, and a `.gitignore` that leaves out `crow_modules/` and `.crow_build/`. |
| `crow new myapp --web` | Adds `views/home.co`, wired into `main.co` with `page['/', home.render]`. |
| `crow new myapp --ml` | Adds `jobs/train.co`, `lib/model.co`. |
| `crow new myapp --db` | Adds `db/schema.co`. Flags combine. |
| `crow install` | Install the crowfile's `deps` into `crow_modules/`, each pinned by `crow.lock`; `--update [name]` moves them to what their versions name now. |
| `crow setup` | Fetch the pinned Rust toolchain and this release's runtime into `~/.crow` for compiled builds, each file verified; `--check` says what is present and whether compiled builds are ready. |
| `crow upgrade` | Replace the binary the install line put in `~/.crow/bin` with the latest release, held to its published hash (a mismatch is named and nothing is kept); `--version X` installs that release instead, a downgrade too; `--check` says whether a newer release is out and changes nothing. The new binary is written beside the old one and renamed over it once it runs. A crow installed any other way — a checkout's build, `--dev`, `--source` — is left as it is, and the command says where it is and how that install upgrades. The new version's toolchain and runtime come with its first compiled build, or `crow setup`. |

`crow run` succeeds right after `crow new`.

## Project structure

Any `.co` file is a program; a project is a directory with a `crowfile`.
Nothing is restructured as it grows — you add files:

```
myapp/
  crowfile              project config (see below)
  main.co               default entry point — the long-running thing (crow run)
  jobs/                 batch tasks: crow run train → jobs/train.co
  views/                web views: use views.home → page['/', home.render]
  lib/                  your code, any layout: use lib/accounts → accounts.open[...]
  db/schema.co          table definitions
  migrations/           .sql files drafted by crow db diff and applied by crow db apply
  css/app.css           optional stylesheet overrides (inlined after the defaults)
  tests/                integration tests
  crow.lock             the commit and the hash of each dependency as installed (commit it)
  crow_modules/         installed dependencies (crow install; not committed)
  .crow/                what crow check and the builds keep: snapshots, the app baseline, fixtures, the databases' answers (schema)
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
schema = 'verify'                   --- optional: what opening a connection does with the tables the program declares
cpu = 4                             --- optional: default thread count (integer)

deps = map[models = 'github.com/agncrow/crow-models@v0.1.0', utils = './libs/utils']
jobs = map[train = 'jobs/train.co', sync = 'jobs/sync.co']
```

`schema` is `'reconcile'`, `'verify'` or `'trust'`: a program run from
source reconciles and a built program verifies unless it says otherwise
(spec, db, The schema a program declares). `gpu`, like `cpu`, is a default
resource count (spec, Resource Control).

Dependencies (`deps`) take two forms:

| Form | Example | How it is installed |
|------|---------|---------------------|
| Repository and version | `models = 'github.com/agncrow/crow-models@v0.1.0'` | Checked out at the tag (or branch, or commit), the commit and the content hash pinned in `crow.lock`; `…@v0.1.0#sha256:…` holds the install to a hash you verified |
| Local path | `utils = './libs/utils'` | Linked into `crow_modules/` |

A package is a git repository and a version is its tag: there is no
registry. The first `crow install` of a dependency records the commit its
version names in `crow.lock`, with the hash of what was installed; every
install after that checks out that commit, so every machine runs the same
code, and every command refuses a package changed by hand under
`crow_modules/` until `crow install` restores it. A project holds one
version of a package: two requirements at tags of one major take the
higher, saying so. On a forge the repository is
the first two path segments and the rest is a directory inside it
(`github.com/agncrow/crow/packages/models@v0.6.0`). Installed modules are
imported like any user module — qualified: `use models.line` then
`line.slope[...]`.

## The two execution modes

| | `crow run -i file.co` | `crow run file.co` |
|---|---|---|
| How | the interpreter, over the program's IR | transpile → Rust → native binary |
| Startup | instant | first build compiles (cached after) |
| Speed | scripting-tier; parallel `for` uses every core | Rust-tier; parallel `for` uses every core (rayon) |
| Use for | exploring, the REPL, quick scripts, `crow test`, `crow dev` | anything you ship or benchmark |

The two modes are required to produce identical output; `crow test
--parity`, `--served`, `--fuzz` and `--book`, and `crow check --self`,
enforce it. If they ever disagree, that is a Crow bug, not a semantics to
learn. Both modes share one front end and one intermediate form: the
compiler infers a concrete type for every value before either mode runs,
so they accept and reject the same programs with the same messages.

The generated Rust lives in `.crow_build/` in the directory you run `crow`
from — you never need to read it, but it is there.

### Environment

| Variable | Effect |
|---|---|
| `CROW_BUILD_PROFILE=dev` | Make the release build of `crow build` and `crow launch` a dev build, the program unoptimised (the verification harnesses set it). A dev build carries no debug info, which keeps the build caches small: on macOS every unit's object file is kept for it, and the harness caches had grown to hundreds of GB. |
| `CROW_DEBUGINFO=1` | Compile the program with debug info — the dev profile as cargo defines it — into `.crow_build/target/debuginfo/`, its own directory, so the plain dev cache is neither rebuilt nor grown by it. For a Rust backtrace with file and line (`RUST_BACKTRACE=1`) when a program panics, which is a bug in Crow, never an error a Crow program can make. The first such build compiles the runtime's crates again with debug info; later ones are incremental. When the bug is fixed, unset it and delete that directory. |
| `CROW_NO_DESCRIBE=1` | Do not ask the program's databases what each query returns at compile time: a query's cells are typed from what the databases answered last (`.crow/schema`), and where nothing is kept they are dynamic, in both modes, read with `int[…]`, `float[…]` or `str[…]`. Nothing is written. |
| `CROW_SCHEMA` | What opening a connection does with the tables the program declares, for one run — `reconcile`, `verify` or `trust` (`CROW_SCHEMA=trust ./app`) — over the crowfile's `schema`. |
| `CROW_DROP_AUDIT=1` | With `crow build --ir file.co`, list on stderr each line whose value can fail and is dropped — decided on as a branch would be: a function hands the failure to its caller, the top level halts. |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` | The credentials an `s3://` path is read and written with (without them an object is read unsigned, as a public bucket's is). |
| `AWS_REGION` | The region of an `s3://` path's bucket (`us-east-1` when unset). |
| `AWS_ENDPOINT_URL` | An S3-compatible store (MinIO, R2) an `s3://` path is reached at, by path. |
