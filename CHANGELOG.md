# Changelog

Crow is in alpha: the syntax may change between releases, `crow fix`
migrates what it can, and a breaking change bumps the minor version.
Each release lists what breaks first. This file summarises; the commit
history says the rest.

## v0.7.0 — 2026-10-18

Crow held to itself: `crow check --self` runs about 100,000 cases from the
compiler's own tables in five configurations, each answer held to the
spec's, and what it found is fixed. Breaking changes first.

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
- `filter` is removed: a mask picks a frame's rows as it picks a list's
  items, `f[f.age gt 30]` (`filter[a, b]` is `a[b]`). A mask that compares
  the frame's own columns is still computed in one pass. A frame's index
  takes a mask alone: `f[0]` and `f['name']` stay refused.
- `pad`, `append`, `date`, `elapsed`, `today` and `nodes` are unknown
  names like any other: the messages that named their replacements are
  gone, and a program may give the names to functions of its own.
- `-x ^ 2` is `-(x ^ 2)`: the power binds before the sign (`-3 ^ 2` is
  -9; it had been 9).
- A matrix's items are its rows: `slice`, `reverse` and `pop` of a matrix
  take rows (they had counted its numbers), and `sort`, `x in m` and
  `join` of a matrix are refused.
- A route's handler answers a response, and a function answers on every
  path: a handler that answers none is refused in a program that serves
  (compiled it had been an empty 200), and a function that can end
  without its value is refused at the line (it had answered `done`
  interpreted and not built compiled).
- A response's body is text and its status HTTP's: `text`, `html`,
  `redirect` and `respond` of another kind are refused (compiled they had
  not built), and `respond`'s status is 100 to 999 — refused where it is
  written outside, a stop where it is computed (it had answered 599
  interpreted and 200 compiled).
- `later[f, a, b]` in a page's handler is refused: a page's `later[f]`
  runs `f` on the page's state, which keeps what `f` needs (compiled, the
  arguments had been dropped and the page had not built). A script's or a
  server's `later[f, a, b]` is as it was.
- A select bound to a fact takes only a value it offers (a radio's and a
  tab's too): a forged event's value is refused, unless a free input binds
  the same fact.
- Refused where they are written, in both modes (one had stopped
  interpreted and not built compiled, or answered differently): a write
  into text, a frame, a column or a parsed value; a mask as a write's
  position; an item, a map's value or a record's field of another kind; a
  function value nothing types, wherever it is handed; a function that
  takes a function called at two kinds; a field asked of a list that can
  fail (`csv[…].b`), as one of a list is; `encode` of bytes (it had
  answered null); handles compared; `tensor / tensor`.
- A trailing window over times that go back stops, saying where (it had
  summed rows by no reading of "trailing"); `onehot`'s classes are 1 or
  more; `moment[seconds]` past 100,000,000 days of 1970 stops (it had
  clamped to the year 292278994).

### Added

- A matrix's row is written in place, `m[i] = [7.0, 8.0]`, as a list of
  numbers or a tensor of the row's own length; a part of a tensor of more
  axes the same way (`c[i] = matrix`). A value of another shape is refused
  before the program runs, or stops where its shape shows only as it runs.
  It had been refused, the matrix built anew.
- A frame's column of bools is the mask it makes where a mask is taken:
  `f[f.active]`, `where[f.active, a, b]`, `names[f.active]`, and negated
  or joined, `not f.active`, `f.active and f.age gt 30` (negated it had
  been one bool, and joined it had not built compiled).
- A frame's column is picked by a mask, `f.name[f.age gt 30]`: a column of
  the same kind with its missing cells kept, as `f[mask].name` reads it.
- A list of lists of numbers is a matrix where a tensor is taken:
  `tensor[rows]` and `stack[rows]` (both had stopped interpreted and not
  built compiled).
- `crow upgrade` replaces the binary the install line put in
  `~/.crow/bin` with the latest release, held to its published hash as
  the install line holds it; `--version X` installs that release (a
  downgrade too), and `--check` says whether a newer one is out. A crow
  installed another way — a checkout's build, `install.sh --dev` or
  `--source` — is left as it is, and the command says how it upgrades.
- `crow check --self` holds Crow to itself, mechanically: about 100,000
  cases written from the compiler's own tables — every builtin with the
  kinds it takes and their edges (one parameter at a time and two at
  once), every collection at every kind of index, every operator over
  every pair of operands, the shapes and the paths a value takes — and
  families for served routes, views drawn as pages, pages driven by their
  events, files, random draws, gradients, the GPU, the standard library's
  own layer, both database drivers and a WebSocket server, a mail server
  and a language model of the check's own, with the standard modules' and
  the packages' own tests. Each runs in five configurations that must
  agree (interpreted, walked, unfused, unmoved, compiled), and each answer
  is held to the spec's by a reference written in Python
  (tests/self/reference.py: libm, numpy, scipy, pyarrow, hashlib, hmac,
  re, Python's HTML parser). Any finding fails the gate, and the lists of
  builtins no case calls, no reference answers and forms no case reaches
  only shrink (docs/crow-check.md).
- `json` of a parsed value in a program that serves answers it as it is:
  `json[json[req.body]]` echoes a body.
- A view called outside views answers its markup, in both modes: logged,
  held in a variable, measured.
- A tool only handed to a model is typed by what its body reads: a
  parameter handed to `upper` is text, and its schema says so (only a
  call had typed one).

### Changed

- A call evaluates every argument, left to right, before the leftmost
  failure is its value, in both modes (the compiled program had skipped
  the arguments after a failed one).
- A tensor divided by a number divides each number (it had multiplied by
  the reciprocal and rounded twice: `[3.0] / 2.5` was 1.2000000000000002).
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
- `xs[mask]` of a list read from a value that can fail (a file's text
  split into lines) builds compiled; it had run interpreted only.
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
- `follow` and `next` lost a piece of a model's reply whenever one of its
  events straddled two reads of the connection, and `recv` of a socket
  lost a message when a wait ended inside it: what a reader held is put
  back before it reads more.
- With the GPU on (`gpu[]`), a view of a tensor placed there — a
  transpose, a slice — is its own numbers: it had been read, multiplied
  and taken `exp` of as the storage under it (`transpose[m]` was `m`,
  `slice[v, -2, 3]` the first two). A number taken from or divided by a
  tensor there is one operation in both modes (they had shown different
  digits, and `sigmoid` and `softmax` with them), and a small number
  reaches the device as itself (`t * 1e-12` had multiplied by 0).
- A name two imported modules give, called with a kind neither module's
  row takes — `load` of a tensor under `use frame, math` — is refused as
  one module's call is; it had stopped interpreted and not built compiled.
- `grad` of a function through `floor`, `ceil`, `round` or `abs` runs in
  both modes (the interpreter had not run the whole number's form on a
  dual number, and the compiled program had not built).
- `factorial` and `gamma` of a whole number are exact (`factorial[3]` had
  been 6.000000000000007).
- A compiled page draws a child of any kind — a tensor, a map, a record, a
  moment, a parsed value, a frame, a column — as the text `log` writes for
  it (it had not built); the interpreter takes as attributes only the map
  written first in an element (it had taken every map child so), draws a
  missing cell as missing (it had drawn 0), and both escape a quote alike;
  `class = ''` writes no class in both.
- A record fact's field written in a handler is stored (interpreted it
  had stopped, compiled it had changed nothing the page saw).
- `stream[feed, 2 ^ 100]` and `later[f, 2 ^ 70]` hand the whole number on
  (they had stopped at 64 bits); `msgpack` of a decimal builds; `json` of a
  map with int keys answers (it had been a 500 interpreted); a tensor
  holding 2^63 is encoded as 2^63.
- `linspace` ends at its end; `norm` of nothing is 0 (it had been -0);
  `conv` of nothing stops with its reason (it had overflowed); `floor`,
  `ceil` and `round` of an int past 64 bits is itself; `push` or `set` of
  one onto 64-bit ints holds it; a wait past what a clock holds is none
  (it had panicked).
- Programs that had not built compiled now do: an int literal past 64
  bits, `.^` with a number on its left, a list or a map of ints written a
  float, a write into a map with int keys, a response logged, a tensor
  beside a failed number, `describe` inside a function, and a function
  handed the answer of another called at two kinds.
- One mistake is one message: a function value nothing types, and a rank
  the unknown value reached, are no longer said beside the misspelling
  that caused them.

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
