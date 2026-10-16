# Changelog

Crow is in alpha: the syntax may change between releases, `crow fix`
migrates what it can, and a breaking change bumps the minor version.
Each release lists what breaks first. This file summarises; the commit
history says the rest.

## Unreleased

### Breaking

- A value that can fail, dropped by its line — `write[path, text]`,
  `exec[conn, sql]` or `rm[path]` on a line of its own — is decided on as
  a branch on it is: a function hands the failure to its caller, a
  handler stops, the top level halts with `cannot drop an error value —
  <reason>`. Its failure had gone by unseen in both modes. Assign the
  value to keep a failure on purpose (`kept = rm[tmp]`). To find the lines
  a program has, `CROW_DROP_AUDIT=1 crow build --ir file.co` lists them.
- `slice` counts a negative position from the end, as an index does:
  `slice[xs, 0, -1]` is all but the last (it was empty: a negative end had
  been taken as 0), `slice[xs, -2, len[xs]]` the last two.
- `f.kind` (or `.ok`, `.reason`, `.trace`) of a frame that holds a column
  of that name is refused, naming `rename[f, 'kind', 'another']`: the dot
  is the frame's status, never the column, and it had read the status
  where the column was meant.
- `describe[conn, sql]` answers a frame of `name` and `type`; its `kind`
  column had been unreachable as `d.kind`.
- `m[i] = x` on a matrix is refused (one position of a matrix is its row):
  it had written the flat number `i` while `m[i]` read the row. A number
  of a matrix is written `m[i][j] = x`, a row `m[i] = row`.

### Added

- A matrix's row is written in place, `m[i] = [7.0, 8.0]`, as a list of
  numbers or a tensor of the row's own length; a part of a tensor of more
  axes the same way (`c[i] = matrix`). A value of another shape is refused
  before the program runs, or stops where its shape shows only as it runs.
  It had been refused, the matrix built anew.
- A frame's column of bools is the mask it makes where a mask is taken:
  `filter[f, f.active]`, `where[f.active, a, b]`, `names[f.active]`.
- A list of lists of numbers is a matrix where a tensor is taken:
  `tensor[rows]` and `stack[rows]` (both had stopped interpreted and not
  built compiled).
- `crow upgrade` replaces the binary the install line put in
  `~/.crow/bin` with the latest release, held to its published hash as
  the install line holds it; `--version X` installs that release (a
  downgrade too), and `--check` says whether a newer one is out. A crow
  installed another way — a checkout's build, `install.sh --dev` or
  `--source` — is left as it is, and the command says how it upgrades.

### Changed

- The runtime bundle carries only what macOS arm64 and Linux x86_64
  build: every other vendored package (Windows', WebAssembly's and a few
  more) is a stub of its manifest, which cargo still reads, and the
  download goes from 84 MB to 43 MB.
- Arrow and Parquet are at 60, which reads Parquet's metadata without the
  Thrift crate.

### Fixed

- The Linux binary needed the system's OpenSSL 3 (`libssl.so.3`), which
  a slim container or an older distribution lacks: the live pages' HTTP
  client uses rustls now, as the rest of Crow does, and CI and the release
  check that the binary needs only the system's own libraries.
- A recursive function that adds its own answers into a local and returns
  it (a backtracking search's count) is held in 64 bits compiled; it had
  been a growing int, every addition paying for it.
- A function called with arguments of different types is checked at each
  signature it is compiled for: `set[cols, …]` where one call hands a list
  had run interpreted only to stop and not built compiled; it is refused
  before either mode runs.
- `stack` of a list of lists whose numbers can fail is refused in words;
  it had reached the compiler's own check as an internal error.
- `max`, `min` or `sum` of a tensor that can fail is a number that can
  fail; it had been typed as nothing known, and the compiled program did
  not build (found by the fuzzer).
- `m[i][j] = x` writes a matrix's number where it lies, in both modes; the
  interpreter had stopped on the chained index and the compiled program
  had not built. A matrix held in a record's field, a list's item or a
  map's value is written the same way (`g.cells[i][j] = x`), where both
  modes had failed too.
- `df.name[mask]` is refused, naming `filter[df, mask].name`; the
  interpreter had stopped as it ran and the compiled program had not
  built.
- `crow stop` stops a program the interpreter serves (`crow run -i`) and
  `crow dev` with what it serves; it had said nothing was running. A
  process a stale pidfile names is signalled only when it runs in the
  project's directory.
- The console runs a paste of several lines as one cell (it had waited
  behind its continuation prompt), reads a `view`, `test` or `derive`
  block whole and an `else` with its `if`, and tells a program that
  serves pages to run from its file.
- Dependencies with security advisories updated: rustls and its
  certificate checker, rand, and (through Parquet) Thrift; OpenSSL is gone.

## v0.6.1 — 2026-10-14

### Added

- The mark: the `(o.o)` every command prints, as `assets/crow.svg`, on
  the README, the book's pages (favicon, touch icon and top bar, their
  addresses versioned by the mark's own bytes) and the editor extension.
- The editor extension is published with each release: to Open VSX
  (Cursor, VSCodium, Gitpod) now, and to the Visual Studio Marketplace by
  trusted publishing once the Marketplace accepts it; the release page
  keeps the `.vsix` for VS Code meanwhile. Its version is Crow's.

### Fixed

- The book site made its directory before writing a page, so a fresh
  checkout published every chapter; the Pages site had answered 404 for
  all but the front page and the specification.

## v0.6.0 — 2026-10-13

The first release of the alpha: the one-line install, and everything
since v0.5.0 (2026-09-06).

### Breaking

- `contains[xs, x]` and `has[m, k]` are written `x in xs`; `gather[…]`
  is `concat[…]`; `pad` and `append` are gone.
- One moment kind: `now`, `moment`, `fields` and `fmt[style = …]` replace
  `calendar`, `parse_time`, `datetime`, `tz` and `time[]`.
- `then` chains are written as lines.
- `nodes[n]` is retired: `crow launch n` starts the processes and
  `env['CROW_RANK']` names each.
- Map literals are `map[k = v]`; a map holds one kind of value; a map's
  keys have a kind (text or int).

### Added

- One line installs Crow: a prebuilt binary for macOS arm64 and Linux
  x86_64 from the release, verified; the interpreter, the tests and the
  servers need nothing else. `crow setup` (or the first `crow build`)
  fetches the pinned Rust toolchain and the runtime into `~/.crow`; with
  no compiler present, `crow run` says so and interprets.
- `use llm`: a language model as a connection (`reach`, `ask`, `chat`,
  `follow`, `embed`), tools as functions, over Anthropic's and the
  OpenAI-compatible wire formats.
- Request fixtures: `crow check --record-fixtures` keeps what a request
  answered, `crow test --fixtures` replays it in both modes with no server
  and no model.
- The HTTP client, the readers, S3, mail, WebSockets, `math.stats` and
  the ClickHouse and Postgres drivers written in Crow on the `sys` layer.
- Ints that grow past 64 bits, decimals and fractions; `use bits`.
- Packages held to a reference: `symbolic` (PySR), `models/boost`
  (scikit-learn), `graph` (networkx), `optimize` (scipy), `retrieval`
  (scikit-learn, rank_bm25), `gpt` (nanoGPT), `vectors` (faiss); the frame
  verbs held to pandas.
- The `rag` reference app; the error-message overhaul and `crow fix`; the
  rehearsal, the app baseline and the scenarios in `crow check`; routes
  reading the app's derivations.

### Performance

- Compiled programs at Rust's speed on the benchmarks game set; the
  interpreter on an instruction form with typed registers (fannkuch from
  77 s to 1.3 s); frames faster than before by 2 to 10 times and ahead of
  polars on load, sort, join and unique; TechEmpower plaintext at Rust's
  speed.

## v0.5.0 — 2026-09-06

The version the alpha starts from.
