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
  'lenn'` underlines `lenn`), else under the token at the column the
  refusal gives, else under the statement — what the terminal underlines.
  A part the file imports that has an error of its own is a diagnostic on
  the `use` line that imports it, its file and line in the message.
- **Quick fixes** — a slip's nearest name (`undefined function 'lenn' —
  did you mean 'len'?`) put in its place, and an import a function needs
  (``mean[] is math's: add `use math` ``) added below the file's imports.
- **Rename** — a local or a parameter within its function (a parameter's
  name in a named argument of a call to it too), a top-level name across
  the file and wherever the project's files call it by the part's name
  (`calc.total`, `c.total` under `use lib.calc as c`). A builtin, a field or
  a name already in use is refused with the reason.
- **Database hazards, live** — when the workspace crowfile sets
  `db = 'sqlite://dev.db'` (read as the server starts), the schema-aware
  checks run on every edit: the racy check-then-act gets a warning
  squiggle with its headline ("two programs can pass this check at the
  same time") the moment you type it. Details and fixes stay in `crow
  check` / `crow fix`.
- **Completions** — your functions, structs and variables, plus builtins
  and keywords, with inferred signatures as detail; a function completes
  as its whole call, each parameter a placeholder to tab through.
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

**VS Code** — the extension is published as "Crow" by `crow-lang` to Open
VSX with each release (the token in the secret `OVSX_PAT`; Cursor,
VSCodium and Gitpod read from there), and each release carries the
`.vsix`, installed with `code --install-extension crow-lang-<version>.vsix`
— the Visual Studio Marketplace takes it by its web page
(marketplace.visualstudio.com/manage). The extension is one file of plain
JavaScript, `editors/vscode/extension.js`, which speaks LSP to `crow lsp`
with VS Code's own API: it depends on no npm package, and packaging it
needs neither npm nor `vsce` — nor Node.js, which nothing in Crow installs
(`vscode-languageclient` had been its one dependency, and with it npm in
the release; 2026-10-18). From a checkout:
```
python3 editors/vscode/package_vsix.py 0.8.0 crow-lang-0.8.0.vsix
code --install-extension crow-lang-0.8.0.vsix --force
```
(or open the folder and press F5 for a dev session), then reload the
window. VS Code runs the copy it installed, not the folder: after any
change under `editors/vscode`, package and install again. (The `.co` rename
reached the folder in September and the installed copy a month later; until
then VS Code opened every `.co` file as plain text, so it neither
highlighted it nor started the server.) Set `crow.serverPath` if `crow` is
not on PATH; a change to it starts the server again.

`editors/vscode/test.sh [path to crow]` drives the extension inside VS Code
itself: VS Code's extension test host loads `editors/vscode/test/index.js`,
which asks every feature through VS Code's own commands — diagnostics,
hover, completion, definition, symbols, references, signature help,
rename, code actions — then kills the server and waits for the extension
to start it again and the diagnostics to come back. A window opens for the
run. The crow it tests is the checkout's `target/release/crow` unless
named, and `CROW_VSCODE` names the VS Code binary (macOS's application by
default).

**Neovim (0.11+)**
```lua
vim.filetype.add({ extension = { co = "crow" } })
vim.lsp.config("crow", {
  cmd = { "crow", "lsp" },
  filetypes = { "crow" },
  root_markers = { "crowfile", ".git" },
})
vim.lsp.enable("crow")
```

**Helix** (`~/.config/helix/languages.toml`)
```toml
[language-server.crow]
command = "crow"
args = ["lsp"]

[[language]]
name = "crow"
scope = "source.crow"
file-types = ["co"]
roots = ["crowfile"]
language-servers = ["crow"]
```

**Zed** — add a local language extension pointing at `crow lsp`, or use the
generic LSP configuration for the language `crow` on files ending in `.co`.

## Design notes

- **One binary.** The server is a subcommand of `crow` itself — no separate
  install, no version skew between the compiler and the editor.
- **A server outlives no install.** The server watches the binary it runs
  from: when `crow upgrade`, `install.sh` or `cargo install` replaces
  `crow`, it says so and stops at its next message, or within two seconds
  when idle, and the editor starts it again on the new binary. In VS Code
  the server's output pane, *Crow Language Server*, shows `the server left
  (exit 0)` and `starting crow lsp`, and each server names its binary, and
  when it was written, as it starts (`crow lsp: crow 0.8.0 at …/crow,
  written Oct 21 14:02`); its `serverInfo` carries the same. The extension
  starts a server that leaves at most five times in three minutes, then
  says so and waits for a reload. (A server that had outlived its install
  kept refusing, for two days, programs the installed crow accepted — a
  query's cell pushed onto a list — until the window was reloaded.) A
  reload is still needed after the extension itself is reinstalled.
- **Whole-file re-analysis per change, parts kept.** Each change re-runs
  the pipeline on the file, and a change that another change to the same
  file has overtaken is not analysed at all. The parts it imports are
  validated once per text: a part is validated again only when it, or a
  part it imports, changes (finch's `views/sim.co`: 4.2 s to open, 0.9 s
  per edit after). The server types a query's cells from what the
  project's databases answered last (`.crow/schema`), never asking a
  database per keystroke.
- **Ranges from the message.** A diagnostic's range is the name its
  message quotes or calls, found on its line as a whole word; else the
  token at the column the refusal gives; else the statement, past its
  indentation — one rule with the terminal's underline
  (`src/diagnostic.rs`). (Spans from the syntax tree would be finer still,
  for what the messages do not name.)
- **When in doubt, show nothing.** Hover and completion answer from the
  latest analysis, which keeps every statement that parsed; the server
  never invents information the pipeline didn't compute.
