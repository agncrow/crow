# One binary, the compiler on demand — distributing Crow

How a person installs Crow with one line, runs it at once, and gets the
Rust toolchain only when a program is compiled, fetched into Crow's own
directory and pinned to the Crow version. Designed 2026-10-12 (Vijay: the
alpha's shape), built 2026-10-13 with the repository public: `src/home.rs`,
`crow setup`, `install.sh`, `scripts/bundle_runtime.sh`, the release and
Pages workflows. The measurements are from this machine (an M-series Mac,
rustc 1.93.1).

## The user's view

```
curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | sh
```

One prebuilt binary lands in `~/.crow/bin/crow`, and the script prints
the one PATH line. Nothing else is installed. At once, with no toolchain:

```
crow new hello && cd hello
crow run -i main.co     --- the interpreter
crow test               --- the tests, the checker, crow check
crow dev                --- a served app, reloaded as it changes
```

The first compiled build fetches what it needs, says so, and goes on:

```
$ crow build main.co
(o.o) fetching the Rust toolchain 1.93.1 for aarch64-apple-darwin (130 MB) … done
(o.o) fetching the crow 0.6.0 runtime (43 MB) … done
(o.o) building the runtime once (about a minute) … done
build ok -> ./main
```

`crow setup` does the fetching up front, for a laptop about to go
offline; `crow setup --check` says what is present. `crow upgrade` is the
upgrade: the latest release's binary in place of the one there, held to
its published hash as the install line holds it (`--version X` names a
release, `--check` says whether a newer one is out); `install.sh
--uninstall` removes `~/.crow`. `CROW_HOME` moves the directory. Until
the compiler is there, `crow run` says so and interprets:

```
$ crow run main.co
(o.o) no compiler here yet: running interpreted — `crow setup` fetches the Rust toolchain and the runtime (about 175 MB, once), and `crow run` compiles from then on
hello, hello
```

## The pieces

| Artifact | Holds | Size | Lives at | Fetched |
|---|---|---|---|---|
| `crow` | The compiler, the interpreter, the checker, the language server, the servers | 51 MB | `~/.crow/bin/crow` | By the install line, per release |
| The toolchain | rustc, cargo and the standard library for the host, pinned to the Crow version (1.93.1 today) | 130 MB down, 500 MB installed | `~/.crow/toolchain/1.93.1/` | On the first compiled build, or `crow setup` |
| The runtime | `crow_runtime`, `crow_db`, `crow_web` sources (2 MB), their crates.io dependencies vendored for the supported targets (219 MB; a package neither builds is a stub of its manifest), one `Cargo.lock` | 43 MB down | `~/.crow/runtime/<version>/` | With the toolchain |
| The runtime's build | The three crates compiled from the bundle's source, per project directory as today (`.crow_build/target`) | 41 s for a script, 85 s for a web app, measured cold; 0.2 s and 3 s after | each project's `.crow_build/` | On a project's first compiled build |
| Packages | What a crowfile's `deps` name | — | `crow_modules/` per project | `crow install`, as now |
| The editor | The VS Code extension as a `.vsix` | — | — | From the release page |

The toolchain comes from Rust's own servers (`static.rust-lang.org`),
three standalone component tarballs — rustc, cargo, rust-std for the
host — with the hashes Rust's channel manifest publishes, and is
installed by the tarballs' own install script into Crow's directory, as
rustup would do into its own. rustup is not used and not touched: a
person who has it keeps it, and Crow's pinned copy never moves theirs.

The runtime bundle is built once per release in CI
(`scripts/bundle_runtime.sh`): the three crates' source, `cargo vendor` of
their dependencies from a probe manifest that names every feature, and
the lockfile every generated program copies. With it a build is offline
and reproducible: cargo reads the vendor directory by a source
replacement in the generated project and never reaches the registry.

Only what the supported targets build is vendored whole. cargo reads the
manifest of every package the lockfile names, whatever the platform — a
directory source loads every directory that holds a `Cargo.toml` — but
the source only of what it compiles, and it verifies only the files a
package's `.cargo-checksum.json` lists. Every other package is a stub, as
cargo-vendor-filterer makes them: its `Cargo.toml`, an empty
`src/lib.rs`, and a checksum file that lists no files and keeps the
package's checksum, which the lockfile's still matches. What the targets
build is read by `cargo metadata --filter-platform`, once for each target
in the script's `TARGETS`: it reads every dependency's platform for that
target, build dependencies and proc macros included, where `cargo tree
--target` reads theirs for the machine it runs on (run on a Mac, it leaves
out a crate a build dependency needs only on Linux). The script then
resolves the pruned directory offline for each target before packing it.
Of the 459 packages vendored on 2026-10-16, 79 are stubs — the 32 Windows
crates (285 MB), 13 of WebAssembly, WASI and UEFI (41 MB), and 34 that
neither target's graph reaches (Android's, Redox's, loom's, …; 34 MB) —
and the download halved, from 84 MB to 43 MB.

## What changes in `crow`

- **The paths baked at build time become a lookup** (`src/home.rs`).
  The binary had carried `CARGO_MANIFEST_DIR` from where it was built, for
  the runtime crates and for `src/modules`. `home::runtime()` looks, in
  order: a checkout named by `~/.crow/dev` (`install.sh --dev`); the
  checkout the binary was built from, where it still exists — the gate's
  `target/release/crow` and a `--source` install keep working unchanged,
  and `CROW_IGNORE_CHECKOUT=1` skips it for a release-shaped run; the
  release's bundle `~/.crow/runtime/<version>`; an install's snapshot
  `~/.crow/src`. `CROW_HOME` moves the directory.
- **cargo is Crow's own.** `Command::new("cargo")` becomes
  `home/toolchain/<rust version>/bin/cargo`, with `CARGO_HOME` under
  `home` so no registry index or cache lands in `~/.cargo`; a `--dev`
  install uses the PATH's cargo as now.
- **The generated project is offline.** Its `.cargo/config.toml` replaces
  crates.io with the bundle's vendor directory and sets `net.offline`; its
  `Cargo.lock` is the bundle's. The emitter's manifest is unchanged: it
  already names only the crates a program uses and the features it needs.
- **`crow setup`** fetches the toolchain and the runtime for this Crow
  version, verifies each file against its published hash, builds the
  runtime once, and reports; `--check` reports without fetching. `crow
  build` and `crow run` call the same code lazily, saying what they fetch.
  A fetch that fails stops with the URL, the hash expected and the one
  seen; nothing half-fetched is kept.
- **`crow upgrade`** (added 2026-10-16) replaces the binary the install
  line put at `~/.crow/bin/crow` with the latest release — GitHub's API
  names it, as for the install line, which with `CROW_DIST_URL` set asks
  for `--version` instead — or with the release `--version` names, a
  downgrade too. The binary's tarball is held to the `.sha256` beside it,
  extracted under the home, copied beside the old binary, run once
  (`crow -v`), and renamed over it: atomic, and a crow running from the
  old file keeps it; a failure at any step leaves the old binary and
  nothing fetched. `--check` reports and changes nothing. A binary
  anywhere else — a checkout's `target/`, a `--dev` or `--source` install
  in `~/.cargo/bin` — is left alone, and the command says where it is and
  how that install upgrades (`git pull` and the install script again, or
  the install line). The new version's toolchain and runtime are fetched
  by its first compiled build, or by `crow setup`, as on a fresh machine;
  the old version's stay under `~/.crow` until removed.
- **The linker is the one thing asked of the machine.** Before a build,
  `xcode-select -p` on macOS and `cc` on Linux; the stop names the
  command that installs it. The interpreter never asks.
- **`crow run` with no compiler present** says so and runs the program
  interpreted — both modes answer the same by design, and the difference
  is speed — and the spec's "compiled by default" became "compiled when
  the compiler is here, and told when not". `crow build`, and every path
  that compiles (`crow test --compiled`), fetch first and say so.

## The release

The repository is public, so the files live on GitHub: the binaries and
the bundle as release assets, the book and the specification on GitHub
Pages, the install script at its raw URL on `main`. A domain in front of
the same files (`dl.aggnr.com`, a subdomain pointed at a bucket) is
branding to add later; the install script reads `CROW_DIST_URL` for any
other home of the files, which the fresh-machine job uses too.

`.github/workflows/release.yml`, on a tag `v*`:

1. Build `crow` on `macos-14` (arm64) and `ubuntu-22.04` (x86_64, so the
   binary runs on any glibc from 2.35 up), strip it, and package it with a
   `sha256` file. The binary links only system libraries: TLS is rustls,
   the GPU is reached through Metal on macOS and a Vulkan loader opened at
   run time on Linux, so no OpenSSL and no distribution package is needed.
2. Build the runtime bundle once: sources, pruned vendor, lockfile, and
   the toolchain version the bundle is held to, in a `manifest.json` the
   binary reads.
3. Package the VS Code extension (Python's standard library alone:
   `editors/vscode/package_vsix.py`).
4. On a fresh machine of each platform, install from those very files
   (`CROW_DIST_URL=file://…`), run interpreted, see `crow run` say there is
   no compiler yet, `crow setup` against Rust's real servers, and `crow
   build` a program — nothing but the binary and a linker.
5. Publish the GitHub Release each job put its files on as a draft (`gh`,
   GitHub's CLI); `install.sh` reads the latest
   release, or the version it is given. The release is immutable once
   published (the repository setting), so a tag's files never change.

`install.sh` at the repository root becomes the fetcher: detect OS and
architecture, download, verify, extract to `~/.crow/bin`, print the PATH
line; no sudo at any point. `--dev` keeps today's path, building from the
checkout. A binary downloaded by curl carries no quarantine attribute,
so macOS does not prompt; signing and notarisation come with a Homebrew
formula, later.

## What holds it

- `src/home.rs`'s unit tests: the channel manifest read, the lookup's
  order, the vendored configuration; for the upgrade, the order of
  versions, the latest release read off GitHub's answer, a published hash
  read, which install a binary is, and a new binary renamed over the old
  only once it runs.
- `tests/upgrade.rs`: against a stand-in release of the test's own, the
  test's binary copied to a home's `bin/crow` upgrades itself to a stub
  release and downgrades to another, each verified; a release whose hash
  does not match, whose binary does not run, or that is not there leaves
  it untouched and is named; the version running is current; and the
  binary run from its own `target/` replaces nothing and says where it
  is.
- `tests/setup.rs`: against a server of the test's own standing in for
  the release and for Rust's dist — a channel manifest, three archives
  whose installers write stub tools, a runtime bundle, each with its hash
  — `crow run` with nothing fetched says there is no compiler and
  interprets; `crow setup` lays the toolchain and the runtime out under a
  home of the test's own and verifies each file; a bundle that does not
  match its hash is named and not kept.
- The fresh-machine job in the release workflow, on both platforms, which
  proves that nothing but the binary and a linker is needed — what no
  test on a machine that already has everything can prove.
- An offline build from a real bundle on this machine, pruned
  (2026-10-16): a one-line script, tensors and a frame, SQLite, Parquet,
  the GPU, a web app and a live page, each built by the machine's cargo
  with an empty cargo home, reading only the vendored sources, and run.
- `scripts/bundle_runtime.sh` itself, which resolves the pruned vendor
  directory offline for each supported target before packing it.
- The gate as it is, which runs the checkout's path.

## Before the first release

Done 2026-10-12: the repository public under MIT or Apache-2.0, no pull
requests during the alpha, `Cargo.lock` tracked, `CHANGELOG.md` with the
alpha rule, secret scanning and push protection on. With this note's
build: the Pages workflow for the book and the specification, the
extension packaged by the release workflow. Windows stays unsupported:
the symlinks of local packages, the install script and the linker check
are all unix.

## Design notes

**Crow's own toolchain directory, not rustup.** Considered: having the
install line run rustup, as `install.sh` does today (rejected: it changes
the person's PATH and `~/.cargo`, moves a toolchain they may already
pin, and ties Crow's version to whatever `stable` is the day they
install). Decided: a pinned toolchain under `~/.crow`, fetched from
Rust's own servers with Rust's own hashes, untouched by and not touching
rustup.

**Vendored dependencies, not a registry fetch at first build.**
Considered: letting cargo fetch the runtime's dependencies from
crates.io on the first build (rejected: a network dependency at build
time is a failure mode, and the resolve can drift). Decided: the bundle
carries them, the build is offline, and the lockfile is the release's.

**Built per project, not a prebuilt cache.** Considered: shipping the
runtime crates' compiled artifacts (rejected for now: cargo's
fingerprints hold absolute paths and the exact rustc, workable only with
the pinned layout), and one shared target directory for every project
(rejected: the generated binaries are named by file stem, and two
projects' `main` would overwrite each other — the harness trap of
`.crow_build`). Decided: the bundle's source compiles in each project's
`.crow_build`, as the snapshot's did; the measured cold build is 41 s for
a script and 85 s for a web app, paid once per project.

**One runtime bundle, not one per module.** Considered: a bundle per
standard module, fetched when a program's `use` names it (rejected: the
pieces share dependencies, the versions must then match across bundles,
and the saving is tens of megabytes once). Decided: one bundle per
release; the program builds only the crates it names, which the emitter
already decides, so a script never compiles the web server or the
database drivers.

**The upgrade replaces only the install line's binary.** Considered:
re-running the install line as the only upgrade (rejected: a person
should not need the line again, and the binary already knows its home,
its host and how to hold a file to its hash); and replacing whichever
binary runs (rejected: a `--dev` install links its checkout through
`~/.crow/dev` and a `--source` install the snapshot in `~/.crow/src`, so
a release's binary in their place would link the source of another
version; a checkout's `target/` is cargo's to write, and a binary put
elsewhere by hand is its installer's to replace).
Decided: `crow upgrade` replaces `~/.crow/bin/crow` alone, and only where
it is a file of its own; the new binary is written beside it, run once,
and renamed over it, never written into, so the process running the old
one keeps its file and a failure at any step leaves the old binary in
place. Every other install is told where it is and how it upgrades.
Without `--version` a crow newer than the latest release (a pre-release,
a build ahead of the tag) is left as it is: an upgrade never moves back
unasked.

**A compiler inside the binary** — Cranelift compiling the IR in-process,
no toolchain ever — is the third option, a new backend after 1.0, as the
self-hosting note says.

## Sizes and times

| | Measured |
|---|---|
| The `crow` binary | 51 MB |
| The toolchain (rustc, cargo, rust-std, one target) | 130 MB download, 500 MB installed |
| The runtime sources | 2 MB |
| Vendored dependencies, every platform's | 578 MB, 459 crates; 84 MB as the bundle, gzipped |
| Of which no supported target builds | 359 MB in 79 crates, 285 MB of it Windows' |
| Vendored dependencies, pruned (shipped) | 219 MB: 380 crates whole, 79 stubs; 43 MB as the bundle, gzipped |
| The runtime's first build, a script | 41 s |
| The runtime's first build, a web app | 85 s |
| Every build after | 0.2 s for a script, 3 s for an app after an edit |

## Decided

- `crow run` with no compiler interprets and says so; `crow build` fetches.
- The repository is public as it is (2026-10-12); the files live on
  GitHub, a domain in front of them later.
- Linux x86_64 and macOS arm64 for the alpha; macOS x86_64 and Linux
  aarch64 when a runner is cheap, each then one more word in the bundle
  script's `TARGETS`.

**No Node.js in the pipeline (2026-10-18).** Every workflow step is a shell
step: the commit fetched with git, Rust installed with the runner's
rustup, the release's files gathered on a draft of the release with `gh`,
the extension packaged by Python and sent to Open VSX with curl, and the
book pushed to the `gh-pages` branch the repository's Pages serve. GitHub's
JavaScript actions — checkout, setup-node, setup-python, the artifact
upload and download, the Pages deploy, rust-cache — each ran on the
runner's Node.js, which GitHub retires a major version at a time; the
extension's packaging ran npm and vsce. What is given up: the build cache
(each run compiles its dependencies), and the Marketplace's command-line
publishing (its `.vsix` is uploaded by its web page).
