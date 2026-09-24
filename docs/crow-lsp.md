# crow lsp — the Crow language server

`crow lsp` speaks the Language Server Protocol over stdio. Every editor with
an LSP client gets live Crow support from the same binary that compiles the
code — diagnostics come from the real pipeline (parse → expand → lower →
infer → lower to the IR, which checks), so the editor never disagrees
with the compiler.

## What it does

- **Diagnostics as you type** — parse errors, the lowering's checks (the
  same teaching messages `crow run` prints), with one broken statement never
  hiding the rest of the file (the parser recovers per statement).
- **Database hazards, live** — when the workspace crowfile sets
  `db = 'sqlite://dev.db'`, the schema-aware checks run on every edit:
  the racy check-then-act gets a warning squiggle with its headline
  ("two programs can pass this check at the same time") the moment you
  type it. Details and fixes stay in `crow check` / `crow fix`.
- **Completions** — your functions, structs and variables, plus builtins
  and keywords, with inferred signatures as detail.
- **Hover** — inferred types (`name: str`, `fn add[a: int, b: int] -> int`),
  struct fields.
- **Go to definition** and **document outline**.

## Editor setup

**VS Code** — the extension lives in `editors/vscode`:
```
cd editors/vscode && npm install && npm run package   # then install the .vsix
```
(or open the folder and press F5 for a dev session). Set `crow.serverPath`
if `crow` is not on PATH.

**Neovim (0.11+)**
```lua
vim.filetype.add({ extension = { crow = "crow" } })
vim.lsp.config("crow", {
  cmd = { "crow", "lsp" },
  filetypes = { "crow" },
  root_markers = { "crowfile", ".git" },
})
vim.lsp.enable("crow")
```

**Helix** (`~/.config/helix/languages.toml`)
```toml
[language-server.co]
command = "crow"
args = ["lsp"]

[[language]]
name = "crow"
scope = "source.co"
file-types = ["crow"]
roots = ["crowfile"]
language-servers = ["crow"]
```

**Zed** — add a local language extension pointing at `crow lsp`, or use the
generic LSP configuration with file type `crow`.

## Design notes

- **One binary.** The server is a subcommand of `crow` itself — no separate
  install, no version skew between the compiler and the editor.
- **Whole-file re-analysis per change**, debounced by the editor. The
  pipeline runs in single-digit milliseconds at typical file sizes;
  incremental analysis is deliberately deferred until a real file is slow.
- **Line-granular ranges in v1.** Diagnostics highlight the offending line
  (narrowed by token columns where cheap). Precise sub-line spans arrive
  with AST span enrichment, tracked as follow-up work.
- **When in doubt, show nothing.** Hover and completion answer from the last
  good analysis; the server never invents information the pipeline didn't
  compute.
