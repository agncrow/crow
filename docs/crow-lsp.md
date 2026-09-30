# crow lsp — the Crow language server

`crow lsp` speaks the Language Server Protocol over stdio. Every editor with
an LSP client gets live Crow support from the same binary that compiles the
code — diagnostics come from the real pipeline (parse → expand → lower →
infer → lower to the IR, which checks), so the editor never disagrees
with the compiler.

## What it does

- **Diagnostics as you type** — parse errors, the lowering's checks (the
  same teaching messages `crow run` prints), with one broken statement never
  hiding the rest of the file (the parser recovers per statement). A
  squiggle sits under the name the message names (`undefined function
  'lenn'` underlines `lenn`), else under the line's text. A part the file
  imports that has an error of its own is a diagnostic on the `use` line
  that imports it, its file and line in the message.
- **Quick fixes** — a slip's nearest name (`undefined function 'lenn' —
  did you mean 'len'?`) put in its place, and an import a function needs
  (``mean[] is math's: add `use math` ``) added below the file's imports.
- **Rename** — a local or a parameter within its function (a parameter's
  name in a named argument of a call to it too), a top-level name across
  the file and wherever the project's files call it by the part's name
  (`calc.total`, `c.total` under `use lib.calc as c`). A builtin, a field or
  a name already in use is refused with the reason.
- **Database hazards, live** — when the workspace crowfile sets
  `db = 'sqlite://dev.db'`, the schema-aware checks run on every edit:
  the racy check-then-act gets a warning squiggle with its headline
  ("two programs can pass this check at the same time") the moment you
  type it. Details and fixes stay in `crow check` / `crow fix`.
- **Completions** — your functions, structs and variables, plus builtins
  and keywords, with inferred signatures as detail.
- **Hover** — inferred types (`name: str`, `fn add[a: int, b: int] -> int`),
  struct fields.
- **Signature help** inside a call's brackets, the active parameter marked.
- **Go to definition** — in the file, and across parts: on `calc.total`
  it opens the part `use lib.calc` names at `total`'s declaration, on
  `calc` the part itself.
- **Find references** — every read and write of the name in the file, its
  declaration included; a part's name only where the part qualifies it,
  and never a record's field of the same name (`p.f` is not `f`).
- **Document outline.**

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
- **Whole-file re-analysis per change, parts kept.** Each change re-runs
  the pipeline on the file, and a change that another change to the same
  file has overtaken is not analysed at all. The parts it imports are
  validated once per text: a part is validated again only when it, or a
  part it imports, changes (finch's `views/sim.co`: 4.2 s to open, 0.9 s
  per edit after). The server types a query's cells from what the
  project's databases answered last (`.crow/schema`), never asking a
  database per keystroke.
- **Ranges from the message.** A diagnostic's range is the name its
  message quotes or calls, found on its line as a whole word; the line's
  text otherwise. (Spans from the syntax tree would be finer still, for
  what the messages do not name.)
- **When in doubt, show nothing.** Hover and completion answer from the last
  good analysis; the server never invents information the pipeline didn't
  compute.
