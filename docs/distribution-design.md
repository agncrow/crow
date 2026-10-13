# One binary, the compiler on demand — distributing Crow

How a person installs Crow with one line, runs it at once, and gets the
Rust toolchain only when a program is compiled, fetched into Crow's own
directory and pinned to the Crow version. This note is the design, to be
read before the code is written (Vijay, 2026-10-12: the alpha's shape).
The measurements are from this machine (an M-series Mac, rustc 1.93.1).

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
(o.o) fetching the crow 0.5.0 runtime (40 MB) … done
(o.o) building the runtime once (about a minute) … done
build ok -> ./main
```

`crow setup` does the fetching up front, for a laptop about to go
offline; `crow setup --check` says what is present. Re-running the
install line is the upgrade; `install.sh --uninstall` removes
`~/.crow`. `CROW_HOME` moves the directory.

## The pieces

| Artifact | Holds | Size | Lives at | Fetched |
|---|---|---|---|---|
| `crow` | The compiler, the interpreter, the checker, the language server, the servers | 51 MB | `~/.crow/bin/crow` | By the install line, per release |
| The toolchain | rustc, cargo and the standard library for the host, pinned to the Crow version (1.93.1 today) | 130 MB down, 500 MB installed | `~/.crow/toolchain/1.93.1/` | On the first compiled build, or `crow setup` |
| The runtime | `crow_runtime`, `crow_db`, `crow_web` sources (20 MB), their crates.io dependencies vendored (150 MB, 40 MB compressed), one `Cargo.lock` | 40 MB down | `~/.crow/runtime/0.5.0/` | With the toolchain |
| The runtime's build | The three crates compiled once, the cache every program's build starts from | 41 s for a script, 85 s for a web app, measured cold | `~/.crow/runtime/0.5.0/target/` | Built once on this machine, after the fetch |
| Packages | What a crowfile's `deps` name | — | `crow_modules/` per project | `crow install`, as now |
| The editor | The VS Code extension as a `.vsix` | — | — | From the release page |

The toolchain comes from Rust's own servers (`static.rust-lang.org`),
three standalone component tarballs — rustc, cargo, rust-std for the
host — with the hashes Rust's channel manifest publishes, and is
installed by the tarballs' own install script into Crow's directory, as
rustup would do into its own. rustup is not used and not touched: a
person who has it keeps it, and Crow's pinned copy never moves theirs.

The runtime bundle is built once per release in CI: the three crates'
source, `cargo vendor` of their dependencies with the Windows-only
crates pruned (130 of the 279 MB a plain vendor holds are `windows-sys`
and its targets), and the lockfile every generated program copies. With
it a build is offline and reproducible: cargo reads the vendor directory
by a source replacement in the generated project and never reaches the
registry.

## What changes in `crow`

- **The paths baked at build time become a lookup.** Today the binary
  carries `CARGO_MANIFEST_DIR` from where it was built, for the runtime
  crates and for `src/modules`; `install.sh` snapshots the source to
  `~/.crow/src` so the clone can go. Instead `crow::home()` is
  `$CROW_HOME` or `~/.crow`, the runtime lives under
  `home/runtime/<crow version>/`, and a developer's `install.sh --dev`
  writes `home/dev` naming the checkout, which wins when present — today's
  behaviour, kept for working on Crow itself.
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
- **The linker is the one thing asked of the machine.** Before a build,
  `xcode-select -p` on macOS and `cc` on Linux; the stop names the
  command that installs it. The interpreter never asks.
- **`crow run` with no toolchain present** says so and runs the program
  interpreted — both modes answer the same by design, and the difference
  is speed — and the spec's "compiled by default" becomes "compiled when
  the compiler is present, and told when not". (Decision for Vijay: this,
  or a stop naming `crow setup`.)

## The release

A workflow on a tag `v*`:

1. Build `crow` on `macos-14` (arm64) and `ubuntu-22.04` (x86_64, so the
   binary runs on any glibc from 2.35 up), strip it, and package it with a
   `sha256` file. The binary links only system libraries: TLS is rustls,
   the GPU is reached through Metal on macOS and a Vulkan loader opened at
   run time on Linux, so no OpenSSL and no distribution package is needed.
2. Build the runtime bundle once: sources, pruned vendor, lockfile, and
   the toolchain version the bundle is held to, in a `manifest.json` the
   binary reads.
3. Package the VS Code extension.
4. Attach all of it to the GitHub Release; `install.sh` reads the latest
   release, or the version it is given.

`install.sh` at the repository root becomes the fetcher: detect OS and
architecture, download, verify, extract to `~/.crow/bin`, print the PATH
line; no sudo at any point. `--dev` keeps today's path, building from the
checkout. A binary downloaded by curl carries no quarantine attribute,
so macOS does not prompt; signing and notarisation come with a Homebrew
formula, later.

## What holds it

- Unit tests of the fetch, the hash check, the layout and the manifest,
  against a local HTTP server in the test (the pattern of
  `tests/llm_server.rs`), including a corrupted download and a bundle for
  the wrong version.
- A fresh-machine job in CI on both runners: install from the artifacts
  the workflow just built, `crow new hello`, `crow run -i`, `crow test`,
  then `crow build` — which fetches the toolchain from Rust's servers and
  the runtime bundle from the workflow's artifact — then `crow install`
  of a package from the repository. The job proves that nothing but the
  binary and a linker is needed, which no amount of testing on a machine
  that already has everything can prove.
- The gate as it is, which runs the `--dev` path.

## Before the first release

- The repository public, `Cargo.lock` tracked (a build nobody can
  reproduce is not a release; `crow install` of a package from the
  repository needs the repository reachable).
- A `CHANGELOG.md` and the rule stated: alpha, syntax may change, `crow
  fix` migrates what it can, a breaking change bumps the version.
- The book and the spec on GitHub Pages (CI builds the book site already).
- The extension packaged; the marketplace later.
- Windows stated as unsupported: the symlinks of local packages, the
  install script and the linker check are all unix today.

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

**Built once here, not a prebuilt cache.** Considered: shipping the
runtime crates' compiled artifacts (rejected for now: cargo's
fingerprints hold absolute paths and the exact rustc, workable only with
the pinned layout, and the measured cold build is 41 s for a script and
85 s for a web app, paid once). Decided: build once at setup or first
use, say so; a cache if a measurement later says it matters.

**One runtime bundle, not one per module.** Considered: a bundle per
standard module, fetched when a program's `use` names it (rejected: the
pieces share dependencies, the versions must then match across bundles,
and the saving is tens of megabytes once). Decided: one bundle per
release; the program builds only the crates it names, which the emitter
already decides, so a script never compiles the web server or the
database drivers.

**A compiler inside the binary** — Cranelift compiling the IR in-process,
no toolchain ever — is the third option, a new backend after 1.0, as the
self-hosting note says.

## Sizes and times

| | Measured |
|---|---|
| The `crow` binary | 51 MB |
| The toolchain (rustc, cargo, rust-std, one target) | 130 MB download, 500 MB installed |
| The runtime sources | 20 MB |
| Vendored dependencies, plain | 279 MB, 216 crates |
| Of which Windows-only | 130 MB |
| The runtime's first build, a script | 41 s |
| The runtime's first build, a web app | 85 s |
| Every build after | 0.2 s for a script, 3 s for an app after an edit |

## Open decisions

- `crow run` with no toolchain: interpret with a note, or stop naming
  `crow setup`.
- The repository public as it is, or a public mirror.
- Linux x86_64 and macOS arm64 only for the alpha; macOS x86_64 and Linux
  aarch64 when a runner is cheap.
