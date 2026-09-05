# Getting Started

Let's install Crow, meet the toolchain, and run your first program. By the
end of this chapter you'll have both execution modes working and a project
directory ready for the rest of the book.

## Installation

Crow builds from source with the Rust toolchain (it compiles your programs
to Rust, so `cargo` is a real dependency, not just a build-time one):

```bash
git clone <the crow repository>
cd crow
cargo install --path .
```

The binary lands in `~/.cargo/bin/crow` — make sure that directory is on
your `PATH`, then verify:

```bash
crow
```

You should see the usage help listing `run`, `build`, `new`, `fmt`,
`check`, and `test`.

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
