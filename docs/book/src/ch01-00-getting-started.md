# Getting Started

Let's install Crow, meet the toolchain, and run your first program. By the
end of this chapter you'll have both execution modes working and a project
directory ready for the rest of the book.

## Installation

You need a Mac or a Linux machine (on Windows, run everything inside
WSL). One line installs Crow:

```bash
curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | bash
```

It puts one prebuilt binary in `~/.crow/bin`, checks it against its
published hash, runs a hello program, and prints the line that puts
`~/.crow/bin` on your `PATH`. Add that line to your shell profile, open a
new terminal, then verify:

```bash
crow -v
```

You should see the version. `crow` on its own starts the REPL, `crow
help` lists the main commands, and The Toolchain, next, has the rest.

Nothing else is installed: the interpreter, the tests, `crow check`, the
language server and a served app need only that binary. Crow compiles
your programs to Rust, so a native build needs a Rust toolchain too:
`crow setup` fetches a pinned one into `~/.crow`, with Crow's runtime,
once (about 175 MB), and the first `crow build` does the same if you
have not; `crow setup --check` says what is present. Until then `crow
run` says so and interprets, which answers the same, only slower.

### Building from a checkout

On a machine the release has no binary for, build crow from its source:

```bash
git clone https://github.com/agncrow/crow.git
cd crow
./install.sh --source
```

The script installs whatever is missing and skips whatever is present, in
order:

1. **A C toolchain.** The Xcode Command Line Tools on macOS (a dialog
   opens; accept it), `gcc` through your package manager on Linux.
2. **Rust**, through rustup — stable, no prompts. Crow needs 1.85 or newer.
3. **A permanent home.** The source is copied to `~/.crow/src`. Compiled
   programs link against the runtime crates there, so the clone can go once
   the script is done, and re-running the script on a newer checkout is the
   upgrade.
4. **The binary**, built in release mode into `~/.cargo/bin/crow`. The
   first build takes a few minutes.
5. **Proof.** A hello project is created, compiled, and run.

`./install.sh --dev` builds the checkout in place instead — for working
on Crow itself, since compiled programs then link against your working
tree. `--quick` skips the proof; `--uninstall` removes the binary and
`~/.crow` and leaves Rust alone. If this was your first Rust install, open
a new terminal (rustup put `~/.cargo/bin` on your `PATH` through your
shell profile) before `crow -v`.

### If something goes wrong

- **`crow: command not found`** right after the install: the binary is in
  `~/.crow/bin`. Add the `PATH` line the installer printed to your shell
  profile and open a new terminal. A build from a checkout is in
  `~/.cargo/bin`, which a fresh Rust install adds to your shell profile:
  open a new terminal, or run `source "$HOME/.cargo/env"` in this one.
- **No binary for this machine.** The release carries crow for macOS on
  Apple silicon and Linux on x86_64; anywhere else, build it from a
  checkout with `./install.sh --source`.
- **A dialog appeared on macOS** while building from a checkout. That is
  the Command Line Tools installer; accept it. The script waits for it to
  finish.
- **Linux asked for a password** while building from a checkout.
  Installing the C compiler goes through your package manager with
  `sudo`. Nothing else in the script needs it.
- **It seems stuck on "building crow".** A release build of the compiler
  takes a few minutes the first time and is quiet while it runs. Later
  runs reuse the cache in `~/.crow/target`.
- **Upgrading:** `crow upgrade` replaces the binary in `~/.crow/bin` with
  the latest release, verified; a crow built from a checkout upgrades by
  `git pull`, then `./install.sh --source` (or `--dev`) again.
  **Removing:** delete `~/.crow` and the `PATH` line; in a checkout,
  `./install.sh --uninstall` removes both binaries and `~/.crow`.

## Editor Support

Crow ships its own language server — diagnostics as you type, completions
with inferred signatures, hover types, go-to-definition — as a subcommand
of the same binary you just installed:

```bash
crow lsp
```

You never run that yourself; your editor does. For VS Code and the
editors built on it, install the extension "Crow" by `crow-lang`: Cursor
and VSCodium find it on Open VSX, and each release carries its `.vsix`
for VS Code, which `code --install-extension crow-lang-<version>.vsix`
installs. It starts `crow lsp` with the `crow` on your `PATH`; the
setting `crow.serverPath` names another.

Neovim, Helix, and Zed speak LSP natively — copy the snippet for your
editor from `docs/crow-lsp.md`. Everything the editor shows comes from the
real compiler pipeline, so the squiggles never disagree with `crow run`.
