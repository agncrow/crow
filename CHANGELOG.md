# Changelog

Crow is in alpha: the syntax may change between releases, `crow fix`
migrates what it can, and a breaking change bumps the minor version.
Each release lists what breaks first. This file summarises; the commit
history says the rest.

## Unreleased

### Breaking

- One statement verb (spec, db, Statements): `query` runs every statement
  and answers a frame — the rows a SELECT answers, the rows a RETURNING
  clause reads back, an empty frame for a statement that answers none.
  `exec` is gone: write `query`, and a write that must say what it
  touched reads it back with `RETURNING`; the count of rows affected is
  not answered. Whether a call reads or writes is the statement's to say:
  a literal SELECT, VALUES, SHOW, EXPLAIN, DESCRIBE or PRAGMA asked is a
  read, anything else a write (RETURNING included), and a statement
  built as the program runs is a write — a derivation that hands one is
  refused, the fix being the literal with `?` for its parameters.
  `migrate[conn, dir]` is gone too: migrations are the tool's, `crow db
  apply <dir> <url>`, beside `crow db diff`. The db module is `connect`,
  `query`, `insert`, `transact` and `describe`. On the way: a ClickHouse
  statement that answers no rows had failed through `query` ("the rows
  end early"); it answers an empty frame now.

- Effects have kinds (spec, Effects Have Kinds): each builtin's row says
  what it reaches — `console`, `files`, `files.write`, `network`,
  `db.read`, `db.write`, `mail`, `clock`, `random`, `input`, `process`,
  `exit`, `shell`, `session`, `waits`, `serving` — and every rule about effects
  is stated by kind, once. What changes for a program: a view may reach
  nothing but the console, the clock (one moment per event) and a draw,
  and a derivation only reads (`files`, `network`, `db.read`, `clock`,
  `process`) —
  a view reading a file or querying a table (the book had taught "query
  at render time"; a derivation is where a page reads a table), a
  derivation sending a notice, had passed and were performed at every
  render; a call the context cannot hold is refused in one wording
  whatever the builtin, naming the function it is reached through
  (`mail`'s own wording goes). In a handler, a failed argument stops the
  handler only for an effect held to its commit (`session`, `db.write`,
  `mail`); a read, a request, a query or a command with a failed argument
  answers the failure, as any call does. `exists` is an effect (`files`),
  so a loop over it keeps its order and a derivation may read it; `args`
  and `env` are stamped with their capture moment, as reads of the world
  are. `crow check`'s reach names a file's effects by kind (`db.read`,
  `db.write`, `files.write`, `shell`, `input`, `waits` where it said
  `database`, `files`, `shell`, `process`).

- `chat`'s and `follow`'s `max` option is `tokens` (`chat[m, talk, tokens
  = 200]`): the module's options are its functions' defaults now, and a
  parameter may not be named like a builtin. A conversation's next user
  turn is `asked[talk, text]`, beside `said[talk, reply]` and `told[talk,
  call, text]`; `ask[text]` still begins one. A program that bound `asked`
  as a name of its own under `use llm` is refused, as any clash with an
  import is: rename it.
- A tensor compared with a number is a mask of its shape: a matrix's is
  its rows of bools (`[[1.0, 2.0], [3.0, 4.0]] gt 2.0` is `[[false,
  false], [true, true]]`), as numpy's is. `where` over it keeps the matrix
  (it had answered a vector), `not`, `and` and `or` keep its rows, `sum`
  counts it, and `m[m gt 2.0]` is the matrix's numbers where it is true
  (it had stopped). A tensor whose rank the program does not show,
  compared with a number, is refused, naming it.
- `min` and `max` answer NaN where a NaN is held — of a tensor, along an
  axis, of two numbers — as numpy's do and as `sum` carries one (a NaN had
  been passed over), and of no numbers the program stops (`max of
  nothing: an empty tensor has no largest number`; they had answered
  `-inf` and `inf`).
- `split[s, '']` is the text's characters, with no empty piece at either
  end (`['', 'a', 'b', 'c', '']` is `['a', 'b', 'c']`), and of empty text
  `[]`. A walk that skipped the ends by position (`slice[cs, 1, len[cs] -
  1]`) now drops real characters: walk `split[s, '']` as it is.
- `gamma_dist[α, β, n]`'s β is the rate, the mean α/β — the textbooks',
  R's, Stan's and PyTorch's (α, β) — so `gamma_dist[1.0, λ, n]` is
  `exponential[λ, n]`. It had been the scale, the mean α·β.
- A `later[f]` asked again while `f` is in flight runs `f` once more when
  it ends, from the state as it then stands, however many arrive
  meanwhile; it had been skipped, and a page could keep the earlier run's
  answer where the interpreter, running each inline, showed the latest.

### Added

- The package lock holds the bytes (spec, Package Manager): `crow.lock`
  records the hash of each dependency as installed beside its commit,
  every command that reads a program refuses a package changed by hand
  under `crow_modules/` until `crow install` restores it, and an install
  refuses a fetch of a locked commit that hashes otherwise. A requirer
  may attest the hash it verified after the version
  (`…@v0.1.2#sha256:…`), in a program's crowfile or a package's, and the
  install holds the bytes to every attestation it meets. Two requirements
  of one package at tags of one major take the higher, saying so, where
  any two sources had been refused; tags of two majors, branches and
  commits that differ still are.

### Changed

- The shape of the tensor a builtin answers is declared on its row
  (`table::Shape`, a fixed vocabulary: an argument's own shape, a
  broadcast, a dimension per size, each dimension stated, a reduction, a
  join along an axis, a transpose, rows stacked, a decomposition's
  pieces), read where the call is typed; the compiler's rule by name is
  gone (docs/extending-crow.md, phase 6). What a program sees: the rank
  of every builtin's answer as before, and a list handed where a tensor
  is taken keeps the rank it had as a list. A row that answers a frame
  declares its columns where its kernel fixes them (doc's `chapters`,
  chart's `macd`), and `query`'s row names its provider: what a database
  says a literal statement answers — asked, kept, typed from the
  statement where SQLite does not say, marked where it can hold NULL —
  is crow_db's (`crow_db::describe`), handed the connection's literal
  URL, the statement and the tables the program declares; the form that
  opens a connection says so on its row (`Prop::Connects`). What a
  program sees is as before.

- A handle — a database connection and a transaction's, a socket, a
  stream, a mailer, a response, the server's app, a test's app session —
  is one kind to the language (spec, Type System: Handles), declared
  once beside its module's rows with its name, its word, the type the
  compiled program holds it as and whether a session may hold it; every
  rule about handles (moved not copied, not indexed, not JSON, equal to
  no other, shown as its word, a fact of one the process's) reads the
  one declaration where it had listed the kinds (docs/extending-crow.md,
  phase 5). What a program sees: `type[x]` and the words of a refusal are
  as before.

- A builtin that takes a function declares it on its row, with its
  signature (`any`, `all`, `find`, `later`, `running`, `stream`, `route`,
  `transact`, a chart's `seek`): inference hands the function its
  parameters from the declaration, the kinds check refuses anything else
  in the row's words, the width pass and the classification read the
  same declaration — each had been a list of the builtins' names in the
  compiler (docs/extending-crow.md, phase 4). A list literal of functions
  is a group, held by name: a call through one is a dispatch the compiler
  writes (the members take and answer one kind), and members of several
  signatures are described and called by name alone (spec,
  Specialization). The standard modules describe and call functions
  through sys's `describe[f]` and `apply[f, args]`; llm's tools are
  described in the module (`chat`'s and `follow`'s schemas were the
  compiler's), so a tools list may hold functions of several signatures.

- The compiler holds the language and nothing of any module (design rule
  24; docs/extending-crow.md): what a builtin is — its options and their
  defaults, its effects, its in-place and growing forms, the range of the
  int it answers, the crate feature it needs — is declared on its row, and
  what a module is on its manifest (`modules::MODULES`); the compiler's
  hand-kept lists of names, which had disagreed with each other, are
  gone, and `src/freeze.rs` fails when a compiler file learns a module's
  or a builtin's name. No program behaves differently, except where a
  call's options are refused: the wording comes from the rows now
  (`window[] takes 3 arguments, then n = 0, by = [], span = 0.0, on = ''
  by name`), and `fmt[t, 'HH:mm']` with the style given by position is
  the time style's, where it had been refused. `use web.ui` is a ui
  program to every check, as it was to the lowering.
- `crow test --modules` runs every standard module written in Crow
  against its own test blocks, from the manifest.
- A standard module written in Crow declares its functions by writing
  them (phase 2 of docs/extending-crow.md): a top-level function with no
  leading underscore is what a program calls, typed by its use as any
  function is, its options its defaults — which may now be `[]` or
  `map[]` — and a helper begins with an underscore. The statistics, the
  neural-network formulas, the token functions, a socket's `socket` and
  `close`, the model functions and the protocol's own are such functions
  now; 45 rows of the builtin table are gone with the JSON text the
  values had crossed them as. A misuse inside one is refused
  naming it and the program's line (`in relu: clip[] takes numbers, got
  an int`). The effects (`read`, `write`, `request`, `mail`, the readers,
  a socket's `send` and `recv`), the variadics (`randn`, `normal`,
  `anova`), `msgpack` (a value of any kind, kept as it is) and the
  database drivers keep their rows until their phases.

- The VS Code extension depends on no npm package: it speaks LSP to `crow
  lsp` itself, in one file of plain JavaScript over VS Code's own API, and
  is packaged by Python's standard library. No workflow runs Node.js: the
  release, CI and the book's site use git, rustup, `gh`, Python and curl
  where they used GitHub's JavaScript actions, npm and `vsce`. The
  extension is published to Open VSX with each release; the Marketplace
  takes the release's `.vsix` by its web page.
- `rsi`'s warm-up is the spec's: from the second row, the mean gain and
  loss of the deltas there are until `n` have come, then Wilder's
  smoothing — as it was, now written down and held to a reference.

### Fixed

- `find['a', 'b']` under `use sys` had reached the layer's `find` of
  bytes (the one resolver read the core's runtime rows alone, and the
  core's `find` is a form), and the lowered program disagreed with
  itself: a core form that declares its kinds comes before a module's
  row, as a core row does.
- A derivation indexed (`trades[trades.qty gte min_qty]`, a mask over a
  derived frame) read as a call of the derivation, "'trades' expects 0
  arguments, got 1": a derivation is read like a fact and indexed like
  one. And a call written with its module was read for its effects by
  its bare name, so `ui.input[…]`, the element, counted as the core's
  `input`, which waits on the reader: the module's own row says.
- A function called from a copy of its caller, with an argument whose
  kind settles only once another copy answers (`inner[may[m, false]]`
  inside a function called with text and with a parsed value), was never
  copied for that kind, and the compiled program did not build; a loop's
  variable whose item kind was not yet known kept a binding of the same
  name from outside the loop; a standard module's local named like a
  page's fact made the module's function a handler; a value that can
  fail, converted where a function's parameter takes another form and
  lent to it, was handed by value, so the compiled program did not
  build; and a loop's item from a frame's column whose cells can be
  missing was typed as nothing known by the width pass (the rule for a
  loop's item knew no column), so a function writing the item into text
  did not build compiled, where the interpreter stopped on the missing
  cell (parity 268). Each fixed.
- Three misuses the lowering had let through to a backend that could not
  build them are refused before the program runs, in both modes, each
  naming what it found: a list whose items no type says compared with
  text (two kinds are not compared; bytes compare with bytes alone), a
  function returning two kinds (`return 'a'` beside `return x` of an int:
  `returns an int where the function answers text — a function answers
  one kind`, where it had been an internal error), and a field read of a
  value that has none or that a tensor or a moment lacks (`text has no
  field 'provider'`, `numbers has no field 'id' — its fields are shape,
  dtype, device`; a map's field is its key, read as the program runs).
  And a parameter handed `[]` that the body reads where a builtin takes
  numbers is a list of numbers (`mse[[], m]` stops as the row had: `pred
  and target must be one length`), where the compiled program had held
  the list as text and not built. Found by crow check --self once the
  modules' rows were gone.

## v0.7.0 — 2026-10-18

Crow held to itself: `crow check --self` runs about 100,000 cases from the
compiler's own tables in five configurations, each answer held to the
spec's, and what it found is fixed. Breaking changes first.

### Breaking

- A value that can fail, dropped by its line — `write[path, text]`,
  `query[conn, sql]` or `rm[path]` on a line of its own — is decided on as
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
