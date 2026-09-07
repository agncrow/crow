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
echo "log['hello, world']" > hello.crow
crow run hello.crow           # transpile → compile → run (native speed)
crow run -i hello.crow        # interpret (instant start)
```

## Commands

### Running and building

| Command | What it does |
|---|---|
| `crow run file.crow` | Transpile to Rust, compile, run. The build is cached in `.crow_build/`; a second run of an unchanged file starts immediately. |
| `crow run -i file.crow` | Run in the interpreter — instant start, same output (both modes are held to identical behavior). |
| `crow run` | Run the project in the current directory: the crowfile's `entry`, `main.crow` by default. |
| `crow run <job>` | Run a named job from the crowfile's `jobs` map, e.g. `crow run train`. Jobs run and exit; `main.crow` is the thing that stays running. |
| `crow build [file.crow]` | Compile without running. The binary lands in the current directory, named after the file (`hello`); for a file outside the directory the name is path-mangled (`examples_hello`). |
| `crow stop` | Stop the running app — by pidfile, then by the project binary's name. `crow run` on a busy port tells you to use it. |
| `crow launch N file.crow` | Build, then run on N distributed processes (`nodes[N]` programs). |
| `crow` | The REPL: variables and functions persist between lines, expression results print automatically. |
| `crow book [--port N]` | Render the book with `docs/book/site.crow` and serve it with every code block runnable in place: ▶ Run interprets, ⚙ Compile builds the native binary, ✓ Test runs the block's test blocks, every block is editable in place with line numbers (Tab indents, Ctrl/Cmd-Enter runs, ↺ Reset restores the book's text), ■ Stop kills a served app. A run of the unedited example reports whether its output matches the book. Builds the HTML with `mdbook` if needed. Default port 3008. |

### The book is a Crow program

The book's chapters are Markdown under `docs/book/src`. `docs/book/site.crow` renders them with `use doc`: it reads the summary and every chapter, lays out each page with the layout vocabulary (menu left, chapter right, menu hidden below tablet width), and writes static HTML to `docs/book/crow-site`. Every runnable code block is rendered as the runner itself — the block is the editor with a line gutter, a toolbar row below it (▶ Run, ⚙ Compile, ✓ Test, ↺ Reset, ■ Stop), then the output panel and the book's expected output — all of it markup from the kernel, so `crow test docs/book/site.crow` checks where those parts sit at two widths without a browser. A small script inside every page only carries the block's text to the server and the reply back.

```text
crow book                          # renders the site, then serves it with the runner live
crow test docs/book/site.crow      # the page and runner layout at 1280px and 390px
crow run -i docs/book/site.crow    # just the pages, in well under a second
crow run docs/book/site.crow       # compiled: byte-identical pages
```

Without `crow book`, the pages still read fine from disk and the buttons say how to start a runner. Search runs over `search.json`, written by the same program; the top bar's `/` focuses it. The book has no other build: mdBook is gone.

Both `crow run` and `crow build` load a `.env` file from the current
directory if one exists (`KEY=VALUE` lines, `#` comments; real environment
variables win).

### Verifying

| Command | What it does |
|---|---|
| `crow test [file.crow]` | Run the `test` blocks you wrote (book chapter 4). Without a file, uses the crowfile entry. |
| `crow check [file.crow]` | Everything automatic on top of your tests: edge cases for pure functions, determinism contracts, live-loop hazards (blocking calls in views, stale snapshots), and the app baseline — what every handler, view, fact and page does, derived from the source, recorded under `.crow/baseline/` on the first check and diffed on every check after, each difference with its cause and its reach. Interactive: `enter` rechecks, `s` skips, `f` applies a fix, `a` accepts a baseline change. |
| `crow check --accept` | Accept every baseline change at once — after a deliberate change, or from a script. Commit `.crow/baseline/` with the change; it is the record of what the app does. |
| `crow book [--port N]` | Inside an app: its own book, derived from the source on every start and served on port 3008 — the app and its parts, each page's composition tree and wireframes at desktop and phone widths, a chapter per part (facts, derivations, handlers as paths, views with trees and wireframes, tests as steps), the data wiring, and the changes since the accepted baseline. Nothing runs against a database. `--write` builds `.crow/book/site` without serving. In the crow repository, the Crow Book. |
| `crow check --show [name]` | The app as a tree: each page, the views it composes under their guards, the inputs they bind and the controls with the handlers they fire; then every handler with what it writes and does, every fact with who binds, writes and reads it, every derivation with what it reads. With a name (`--show buy`), one declaration's full artifact: a handler's paths, a view's tree. |
| `crow check --db <url>` | Adds the schema-aware database checks — write skew, lost updates, one-way latches, assumed-unique reads, unpaired writes, plus column/table verification. Or put `db = '…'` in the crowfile. See [crow-check.md](crow-check.md). |
| `crow check --strict` | Exit 1 on any finding (for CI). `--verbose` prints purity and snapshot details. |
| `crow fix file.crow --hazard N [--option M]` | Apply a fix from the check report (writes the migration, applies it, re-checks). |
| `crow test --parity` | The interpreter–transpiler harness: every `tests/parity/*.crow` runs in both modes and the outputs are diffed. |
| `crow test --book` | Executes every code block in the book in both modes and compares printed output with the book's text. |
| `crow test tests/layout/conformance.crow` | The layout engine against a real browser: fixtures measured once in Chrome by `tests/layout/measure.py` (Playwright, developer-only) and checked in as CSV; the check itself needs no browser. Re-measure after a stylesheet or engine change. |
| `crow test --docs` | Holds the documentation to the compiler: every builtin, module function, and element must be in the spec; every spec reference row must exist in the compiler; every module and core builtin must have a verified book example; every example file must be indexed. Runs in CI. |

### Formatting

| Command | What it does |
|---|---|
| `crow fmt [file.crow]` | Format the project (via crowfile) or one file: 2-space indentation, no trailing whitespace, at most one blank line, single trailing newline. Source-level, so comments and `then` chains are preserved exactly. |
| `crow fmt --check` | Exit 1 if anything needs formatting. |
| `crow fmt --fix-interpolation` | Migrate pre-backtick `{expr}` strings to `` `expr` ``. |
| `crow fmt --fix-views` | Migrate ui-building `fn` declarations to `view` declarations. |

### Projects and packages

| Command | What it does |
|---|---|
| `crow new myapp` | Scaffold a project: `crowfile` + `main.crow`. |
| `crow new myapp --web` | Adds `views/home.crow`, wired into `main.crow` with `page['/', home.render]`. |
| `crow new myapp --ml` | Adds `jobs/train.crow`, `lib/model.crow`. |
| `crow new myapp --db` | Adds `db/schema.crow`, `jobs/migrate.crow`. Flags combine. |
| `crow install` | Install the crowfile's `deps` into `crow_modules/`. |

Every scaffolded file is working code — `crow run` succeeds right after
`crow new`.

## Project structure

Any `.crow` file is a program; a project is a directory with a `crowfile`.
Nothing is restructured as it grows — you add files:

```
myapp/
  crowfile              project config (see below)
  main.crow             default entry point — the long-running thing (crow run)
  jobs/                 batch tasks: crow run train → jobs/train.crow
  views/                web views: use views.home → page['/', home.render]
  lib/                  your code, any layout: use lib/accounts → accounts.open[...]
  db/schema.crow        table definitions
  migrations/           .sql files applied by migrate[] and written by crow fix
  css/app.css           optional stylesheet overrides (inlined after the defaults)
  tests/                integration tests
  crow_modules/         installed dependencies (crow install)
  .crow_build/          the generated Rust project — safe to delete
```

Modules are files. `use views.home` loads `views/home.crow` and its
exports are reached qualified (`home.render`); slash paths work the same
(`use lib/utils` → `utils.helper[...]`). Only the standard library gets
bare names — see book chapter 8.

## The crowfile

The crowfile is a Crow file; the toolchain evaluates it and reads these
variables. `crow new` writes only `name`.

```
name = 'myapp'
entry = 'main.crow'                 --- optional: main.crow is the default
db = 'sqlite://dev.db'              --- optional: schema for crow check and the editor
cpu = 4                             --- optional: default thread count (integer)

deps = map[utils = './libs/utils', http = '2.3']
jobs = map[train = 'jobs/train.crow', migrate = 'jobs/migrate.crow']
```

Dependencies (`deps`) take three forms:

| Form | Example | How it is installed |
|------|---------|---------------------|
| Version | `http = '2.3'` | From the registry, cloned at git tag `v2.3` |
| Git URL | `lib = 'https://github.com/user/crow-lib'` | Cloned from the URL |
| Local path | `utils = './libs/utils'` | Symlinked into `crow_modules/` |

Installed modules are imported like any user module — qualified:
`use utils` then `utils.helper[...]`. The registry is the git repo at
`https://github.com/vsriram/registry.git`, cloned to `~/.crow/registry/`;
override it with the `CROW_REGISTRY` environment variable.

## The two execution modes

| | `crow run -i file.crow` | `crow run file.crow` |
|---|---|---|
| How | tree-walking interpreter | transpile → Rust → native binary |
| Startup | instant | first build compiles (cached after) |
| Speed | scripting-tier | Rust-tier; parallel `for` uses every core (rayon) |
| Use for | exploring, the REPL, quick scripts, `crow test` | anything you ship or benchmark |

The two modes are required to produce identical output; `crow test
--parity` and `crow test --book` enforce it. If they ever disagree, that is
a Crow bug, not a semantics to learn. The transpiler is the authority on
what is a valid program: it infers a concrete type for every value, and a
program the interpreter accepts but the transpiler rejects has a type error.

The generated Rust lives in `.crow_build/` next to your source — you never
need to read it, but it is there.

### Environment

| Variable | Effect |
|---|---|
| `CROW_BUILD_PROFILE=dev` | Compile checked programs without optimisation (the verification harnesses set it). |
| `CROW_NO_DESCRIBE=1` | Do not ask the program's databases what each query returns at compile time; frame cells then read as text in the compiled program. |
