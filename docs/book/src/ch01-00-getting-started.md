# Getting Started

Let's install Crow, meet the toolchain, and run your first program. By the
end of this chapter you'll have both execution modes working and a project
directory ready for the rest of the book.

## Installation

You need a Mac or a Linux machine (on Windows, run everything inside WSL),
`git`, and about ten minutes, most of it compiling. One command from a
fresh clone does the rest:

```bash
git clone https://github.com/agncrow/crow.git
cd crow
./install.sh
```

Crow compiles your programs to Rust, so the Rust toolchain is a real
dependency, not just a build-time one — and the script takes care of it.
It installs whatever is missing and skips whatever is present, in order:

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

`./install.sh --dev` builds from the checkout in place instead — for
working on Crow itself, since compiled programs then link against your
working tree. `--quick` skips the proof; `--uninstall` removes the binary
and `~/.co` and leaves Rust alone. On Windows, run it inside WSL.

If this was your first Rust install, open a new terminal (rustup put
`~/.cargo/bin` on your `PATH` through your shell profile), then verify:

```bash
crow -v
```

You should see the version. `crow` on its own starts the REPL, and `crow
help` lists `run`, `build`, `new`, `fmt`, `check`, and `test`.

### If something goes wrong

- **`crow: command not found`** right after the install: the binary is in
  `~/.cargo/bin`, which a fresh Rust install adds to your shell profile.
  Open a new terminal, or run `source "$HOME/.cargo/env"` in this one.
- **A dialog appeared on macOS.** That is the Command Line Tools installer;
  accept it. The script waits for it to finish.
- **Linux asked for a password.** Installing the C compiler goes through
  your package manager with `sudo`. Nothing else in the script needs it.
- **It seems stuck on "building crow".** A release build of the compiler
  takes a few minutes the first time and is quiet while it runs. Later
  runs reuse the cache in `~/.crow/target`.
- **Upgrading:** `crow upgrade` replaces the binary in `~/.crow/bin` with
  the latest release, verified; a crow built from a checkout upgrades by
  `git pull`, then `./install.sh --source` (or `--dev`) again.
  **Removing:** `./install.sh --uninstall`.

## Editor Support

Crow ships its own language server — diagnostics as you type, completions
with inferred signatures, hover types, go-to-definition — as a subcommand
of the same binary you just installed:

```bash
crow lsp
```

You never run that yourself; your editor does. For VS Code, build and
install the extension once:

```bash
cd editors/vscode && npm install && npx @vscode/vsce package --allow-missing-repository
code --install-extension crow-lang-0.1.0.vsix
```

Neovim, Helix, and Zed speak LSP natively — copy the snippet for your
editor from `docs/crow-lsp.md`. Everything the editor shows comes from the
real compiler pipeline, so the squiggles never disagree with `crow run`.
