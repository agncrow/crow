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
  as its whole call, each parameter a placeholder to tab through. A
  part's function is offered as the program calls it (`calc.total[x]`),
  never by the name it is inlined under (`lib·calc·total`).
- **Hover** — inferred types (`name: str`, `fn add[a: int, b: int] -> int`),
  struct fields.
- **Signature help** inside a call's brackets, the active parameter marked.
- **Go to definition** — in the file, and across parts: on `calc.total`
  it opens the part `use lib.calc` names at `total`'s declaration, on
  `calc` the part itself.
- **Find references** — every read and write of the name in the file, its
  declaration included; a part's name only where the part qualifies it,
  and never a record's field of the same name (`p.f` is not `f`).
- **Document outline** — the file's own functions, records and
  variables; a part's are its own file's outline.
- **Semantic highlighting** — each name told by what it is, for the
  editor to colour it by over the grammar's colours: a parameter, a local
  or a fact, a function (declared, called, or handed on as a value:
  `draw` in `sys.frames[draw]`), a struct (`App[3, 2]` makes one), a
  field (`a.copies`, and a key of a struct or a map made: `P[x = 1]`,
  `map[k = v]`), a parameter named in a call (`go[1, far = 3]`), a module
  (`gui` in `gui.row[…]`). A builtin, a constant (`pi`) and a standard
  module's function are marked as Crow's own (`defaultLibrary`), a
  derivation and a module's value as read-only, each declaration as one;
  the names in a string's backtick pieces are told where they stand.
  Where a grammar sees `b[0]` as a call, the server knows `b` is a
  parameter indexed. VS Code asks through the extension, which turns
  semantic highlighting on for Crow's files (`"[crow]":
  {"editor.semanticHighlighting.enabled": true}`, where a theme would
  otherwise decide) and colours a `keyword` the server tells (`test`
  where a test starts, `as`) as the grammar colours `fn`
  (`semanticTokenScopes`: VS Code's own default for it is a control
  word's colour, `if`'s); Neovim (0.9+) colours by them unasked.

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
python3 editors/vscode/package_vsix.py 0.9.0 crow-lang-0.9.0.vsix
code --install-extension crow-lang-0.9.0.vsix --force
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
rename, code actions, each name's kind — then kills the server and waits
for the extension to start it again and the diagnostics and the names'
kinds to come back. A window opens for the run. The crow it tests is the checkout's `target/release/crow` unless
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
  when it was written, as it starts (`crow lsp: crow 0.9.0 at …/crow,
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
- **Names read off the tokens.** Semantic tokens (`src/lsp/semantic.rs`)
  are read from the document's tokens, as a rename reads them, never from
  the program lowered, whose names a part's inlining and the lowerings
  rename. A declaration runs to the next line at column 1 (a `fn`,
  `struct` or other declaration's word there starts one whatever brackets
  a line above left open, so a bracket being typed does not run the file
  together); its parameters are its brackets' names, its locals the names
  its statements assign and its loops bind, and the top level's facts the
  names it assigns. A name read is then what it names there: a parameter
  or a local of its declaration first, then what the file declares, then
  a module it uses, then Crow's own (`BUILTINS`), then the analysis's
  functions and structs (a part's); a name that names nothing known is a
  function where it is called, a struct where that name is capitalised,
  and a variable elsewhere. The legend is the server's
  (`semantic::legend`: namespace, struct, parameter, variable, property,
  function, keyword; declaration, readonly, defaultLibrary), and the
  extension registers with the legend the server announces, so a crow
  with a new kind needs no change to the extension. A lexer's refusal is
  the whole file's, so a document that does not lex is answered
  `ContentModified`: VS Code keeps the colours it has, moved with the
  edits, until the document lexes again.
