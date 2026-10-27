# Changelog

Crow is in alpha: the syntax may change between releases, `crow fix`
migrates what it can, and a breaking change bumps the minor version.
Each release lists what breaks first. This file summarises; the commit
history says the rest.

## Unreleased

### Breaking

- One value for several names is refused before the program runs, in
  both modes (spec, Multiple Assignment; Design Note: No Destructuring):
  `a, b = xs` and `c, d = pair[]` had run interpreted and not built
  compiled. The refusal gives the form in the program's names (`r =
  pair[]`, then `c = r[0]`, `d = r[1]`). Values of another count than
  the names are refused too: a value past the names had been dropped,
  and a name past the values left unset.
- A loaded file's empty cell is missing in a column the program's reads
  type: read one at a time it is an error value of reason `missing`,
  `.ok` false, in both modes and for a page's data source too, as the
  spec says. The reads had typed the cells plain, and a missing cell had
  read as `''` or NaN with `.ok` true. A program that decides on one now
  meets the refusal a failed value meets, and a list of such cells used
  as a mask is refused: fill the column first. On the way, a call
  answering a list of values that can fail, given an argument that can
  fail, builds compiled, and an item read from such a list keeps its
  failure.
- A module of the program's takes no standard module's name (spec,
  Importing): `use math` where a `math.co` of the program's lies — or a
  package `math` under `crow_modules/` — names two modules and is
  refused at the import, naming the file and how to rename it. The
  import had reached the standard module silently, and the file's
  functions failed as the program ran. The standard library's own
  sources are not the program's.
- A column is written the way it is read (spec, Frames): `df.name =
  values` writes a column where the frame lies — a new column, or one
  replaced; a scalar repeated down the rows — and `with` is gone from
  the language (an unknown name now). After a dot a column's name reads
  as text does, a backtick inside it an interpolation: `df.lag_`k` = …`
  and `df.`name` = …` write a column by a computed name, and read one
  the same way; a header no dot can write is reached as `df.`'in px'``.
  `frame.new[a = xs, b = ys]` stays, each column such a write. Values of
  another length than the rows stop the program where `with` had
  answered an error value. `crow fix` has no rule: write `x = with[x,
  'n', v]` as `x.n = v`, a chain over `frame.new[]` as the literal, and
  a `with` in expression position as a write on the line before.
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

- Magpie, the data platform (`use magpie`; first built as `lake`, renamed
  2026-10-26 before any release: a lake is raw files other engines query,
  and this store holds its state and runs its own engines — the magpie is
  the corvid that gathers everything into one nest). Its second
  architecture is docs/magpie-design.md, built phase by phase below.
- The log as batch files, `use magpie` (spec, magpie): each `batch` one file
  written once by the kernel (sys, `batch_write`) — the log entry, the
  sorted level-0 segment with its statistics and the word index at once —
  `head`, `since`, `tail` (a view that follows the log), `scan`, `latest`
  (the table the log holds), `lookup` (every key at once, so a segment
  sorted by day then symbol reads one row group for one symbol's day;
  files pruned by their filters), `columns` on every reader (a segment
  decodes only the columns asked), `batch[…, flush = false]` and
  `sync[dir]` (Redis's default durability, when asked for),
  `commit` (the sequencer: a stale read refused), `merge` (the files
  folded into one Parquet segment of 8192-row groups, the latest version
  of each key kept, or every row with `every = true`), `batches` (the
  catalog's state), `search` (BM25 as Lucene scores it, over the whole
  log as one corpus, the files whose filters admit no word unread; held
  to the formula computed apart, parity 296), `metrics`, `backup` and
  `restore` (a directory or an `s3://` prefix, the manifest written
  last), `share` and `follow` (replication by log shipping, two
  processes in tests/magpie_share.rs). Every file's head carries the
  format's version, so an older Crow refuses a newer file by name. Held
  to SQLite (parity 294, 295), to a month of finch's ClickHouse and
  Postgres rows, and to the self-check. The fifth reference app,
  `examples/magpie`, is a page over a log.
- The Kafka door, `use kafka` (spec, kafka): `serve[root, port]` answers
  a 0.10.1 broker's protocol in Crow over the sys layer's listener
  (`listen`, `accept`, `address`), a topic a log under the root, a
  producer's set one batch, a consumer's fetch `since` its offset, groups
  of several members and committed offsets two logs of the door's own,
  a fetch with nothing to answer waiting up to the client's `max_wait_ms`;
  legacy message sets with their CRC-32 read and written by a kernel
  (sys, `records`, `records_of`, `unpack`), gzip and snappy sets decoded.
  Held to the pure-Python client in both modes (tests/kafka.rs).
- `sys.filesize[path]`: a file's bytes from its metadata.

- MCP over stdio (spec, mcp: The stdio transport): `server[command =
  'npx -y @modelcontextprotocol/server-filesystem /tmp']` names a server
  by the command that runs it, as `shell` takes a command; the process
  starts at the first message, stays while the thread that started it
  lives — one per command on each thread, so a served program's handlers
  never share one — and is started again if it ends; each request is one line on its input with
  the `_meta` every request carries, the response the line carrying the
  request's id, notifications and late answers passed over, a line that
  is no message refused with its words; the server's standard error is
  the program's. `discover`, `catalog`, `tool` and the loop's `servers =
  [s]` are the same calls over either transport. `listen[tools = [f,
  g]]` makes a Crow program such a server over its own standard streams,
  answering as `answer` answers a route — and a legacy client's
  `initialize` is refused naming the revision spoken, on both transports.
- A process the program talks to, in the sys layer (spec, sys):
  `start[command]` runs the command through `sh -c` with its standard
  input and output piped and answers a socket — `send` and `recv` on its
  stream, `leftover` and `unread` for what a read took past a message —
  one process per command on each thread; `close` of its stream ends
  it as the MCP stdio transport says a client ends its server: the input
  closed, its exit waited for, then TERM and KILL to its process group.
  No new handle kind: a process is a socket, as a WebSocket's connection
  is.
- The agent loop, `act` (spec, llm: The loop is the module's):
  `act[m, talk, tools = [f, g]]` runs the model's tools until it answers
  or a limit stops it — each reply's calls answered by the tool of its
  name through `apply`, a name no tool has, an argument of the wrong
  kind and a tool's own failure each told to the model as a failed
  result that says why, so every call is answered; `rounds` (8) and
  `budget` (tokens in all) as the limits, stopping after a reply's calls
  are answered so the conversation stays well-formed. Answers a record:
  `text`, `stop`, `rounds`, `usage`, `talk` and `steps`, a frame with a
  row per model call and tool call. `told[…, failed = true]` tells a
  failure by hand; Anthropic's wire carries it as `is_error`. Written in
  Crow in the module (docs/agents-design.md).
- The loop's gate: `allow = [g, h]` names functions of the call, each
  asked before a tool runs and answering a bool; a call one refuses is
  told to the model as a failed result (`remove is not allowed by the
  program`, `how` `refused` in the steps frame) rather than run, and a
  gate that takes another count of parameters or answers another kind is
  the call's error before any model is asked.
- A subagent is a tool that runs the loop (spec, llm): a function the
  parent model calls, whose body runs `act` with a model and a tool list
  of its own and answers the inner run's text — nothing new in the
  language, verified in both modes against a server that scripts a
  delegating run and against a local model. A run that failed fails the
  tool, so the parent model hears a failed result that says why; the
  parent's steps frame shows the delegation as one tool row whose `ms`
  covers the inner run, and the inner run's steps are the tool's own to
  read.
- The loop's context, skills and servers: `keep = n` cuts the tool
  results of rounds older than the last n to a stub naming their size,
  `compact = n` asks the model for a summary once a reply has read n
  tokens and goes on from the summary alone, `skills = folder` reads a
  folder of skills in the Agent Skills format (agentskills.io: each
  skill's `SKILL.md` frontmatter held to the standard's rules), names
  them in the system prompt and lets the model load one, and the files
  it refers to, through the tools `skill` and `skill_file`; `servers =
  [s]` adds the tools MCP servers offer, called on the server.
- `use mcp` (spec, section 13): the Model Context Protocol, revision
  2026-07-28, over Streamable HTTP — `server[url]` names an endpoint,
  `discover[s]`, `tools[s]` and `tool[s, name, given]` ask it, and
  `answer[req, tools = [f, g]]` in a POST route makes a Crow program a
  server over the functions themselves (`server/discover`, `tools/list`,
  `tools/call`, the errors the protocol names). Written in Crow on
  `request`; tests/mcp.rs drives both directions in both modes.
- An agent with traces, evals and a page, `examples/agent` (the fourth
  reference app; docs/agents-design.md, Traces, evals and a page): a
  support agent over a shop's orders in SQLite — three tools, the refund
  behind the program's gate — every run written to the database as a
  trace (a row per run with its question, model, answer, stop, rounds,
  tokens and milliseconds; a row per step, the loop's own frame handed to
  `insert`) and a page over the traces, the tokens and the latency of
  each run charted; evals as `test` blocks replayed from recorded
  fixtures in both modes — exact checks on the steps frame (the tools
  called in order, the gate's refusal, the stop) and a model-as-judge
  asked for PASS or FAIL — and a page that asks the agent and shows the
  run's steps. Nothing new in the module: the judge is a `chat` with a
  rubric, kept in the example.
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

- `crow fmt` indents at four spaces, the unit every document, the book
  and the standard modules write (it had normalized every file to two),
  and reads levels from the nesting: a line inside a bracket moves with
  its statement, keeping its alignment, and the lines a string written
  across lines runs onto are left as written. One continuation aligned
  at an odd width had multiplied every level of a file
  (`tests/layout/conformance.co` came out four times as deep), and a
  string's lines had been re-indented, changing its text.
- `crow help` lists every command and flag the binary takes, one line
  each, on standard output, held by a test to the flags the binary
  reads; `crow file.co` is documented as the short form of `crow run
  file.co`.
- `crow new --db` declares the program's tables as the db section does:
  `db/schema.co` holds a `setup[conn]` running `CREATE TABLE IF NOT
  EXISTS users (…)`, and `main.co` opens `sqlite://<name>.db` and hands
  it to `schema.setup`; `crow db diff` drafts what a database lacks of
  them. The scaffold lists no migrate job.
- `crow check`'s rules for a view that queries, reads a file or waits
  are gone — a remote query in a view, its `--- crow: remote-render`
  acknowledgment, and blocking rooted at a view — since the compiler
  refuses such a view first; blocking in a handler stays.
- A name beginning with `_` is one `crow check` asks nothing of: `_kept
  = rm[tmp]` keeps a failure it ignores, and the unused-variable advice
  names that form (it had said "use _", which parses nowhere).
- `.coc` bytecode is no longer read (nothing wrote it since `crow
  compile` went): a project's `main.coc` and `crow x.coc` are not Crow
  files.
- A call a view may not make is sent where its kind belongs (spec,
  Effects Have Kinds): a read a derivation may make — a file, the
  network, a database, the process's environment — to a derivation the
  view reads (``read it in a derivation the view reads — `derive x`
  above the view, returning it``), as the book teaches, and a write or
  any other kind to a handler, as before; a call of several kinds goes
  where the one a derivation cannot hold says (`recv`, which waits, is a
  handler's). Every refusal had sent the call to a handler.
- A method call written as another language writes one is taught the
  Crow form there is, never a function Crow lacks (spec, Error
  Messages): `datetime.now()` names `now[]` (it had said `now[datetime,
  …]`), `xs.includes(1)` and its kin name `1 in xs` (it had said
  `includes[xs, …]`), a call through an imported module is that call in
  square brackets, a value's method is the builtin of its name with the
  value first (`upper[s]`), and anything else is told that a call is
  `f[x]`.
- The examples reach `claude-sonnet-5-5` (they had named
  `claude-sonnet-5`): llm's hint, its tests, the spec, the book and the
  design notes.
- The core's names are held, not frozen (spec, Three namespace tiers;
  the book's chapter 8): until 1.0 a core name is added only as a
  breaking change the changelog lists first, since a name it reserves
  may be one a program used, and from 1.0 the core does not grow. The
  spec had said the core never grows while `occurs`, `translate`,
  `decimal`, `fraction`, `halt` and `bucket` joined it this month.
- `packages/models`: `search` answers `Search[best, score, scores]`, the
  best candidate's mean in `score`; the field had been `scorer`, a slip
  of the rename of the function parameter beside it.
- The comparison's ratings follow the measurements of 2026-10-21: Go's
  performance is 9 beside Crow's (comparable on the seven programs),
  Crow's parallelism 8 (data-parallel only: requests and queries never
  overlap) and its data structures 7 (frames, tensors, exact numbers,
  moments).
- The documentation no test reads describes the language as it is
  (2026-10-21): `docs/crow-for-llms.md` (moments as a kind, `in`, ints
  that grow, and the modules a generator reaches for — frames, db, web,
  llm, mcp — every block run), the feature map, the by-example index,
  Appendix B, the comparison (the measurements of 2026-10-21, a section
  on language models), every command and flag in `docs/usage.md` and the
  book's toolchain page, `crow check`'s and the language server's notes,
  the db, frame and math notes, the roadmap's status, and the docs index
  (ten notes it had not listed). In the spec: the error kinds the
  modules answer (`'http'`, `'socket'`, `'auth'`, `'mail'`, `'llm'`,
  `'mcp'`) and what answers `'time'`; a handler's stop by effect kind; a
  dynamic cell read as its use implies; the project layout as the
  toolchain reads it (no folder is discovered but `jobs/`, through the
  crowfile); the note "a moment was a number" marked superseded.
- A parameter read by its fields, and typed by nothing else, is a parsed
  value (spec, Tools are the functions): a function called by name —
  the loop's gate, a tool taking an object, an MCP server's — had been
  refused as "typed by nothing — read it in the body", which its body
  did. A parameter any call types keeps that type; the rule only fills
  the case the compiler refused.
- The book's standard-library pages (`docs/book/src/stdlib/`) are
  generated for every module the manifest lists that has a section of
  its own in the spec, in the manifest's order; a hand-kept list in the
  docs check had lacked a module's page while the book's index linked it.
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

- Five shapes over a log's open frames that one mode had accepted and
  the other refused (parity 297, found by the magpie's benchmark): a frame
  literal built from an open frame's cells — `frame.new[last =
  [rows.close[i]]]` — builds compiled (the runtime's `Cells` takes a
  list of parsed values: whole numbers as ints, numbers as floats,
  bools, texts, a null a missing cell); a mask conjunction over a frame
  that can fail (`ds[ds.day eq day and ds.status eq 'ready']` of a
  `latest`) is joined truth by truth where the emitter had printed `&`
  of two lists; `sum[where[mask, column, 0.0]]` over a column that can
  fail converts the marked column for math's sum (the verifier had
  refused the call); a helper handed a frame begun as `frame.new[]` and
  filled in a branch is one copy with the open frame's (a frame of no
  known columns is no signature of its own, and the recorded frames are
  joined); and a fallible local assigned in a branch under its own
  guard (`if hit.ok` then `out = hit`) copies its payload out where the
  local is read again (compiled, the frame had been moved: E0382).
- The interpreter compares a column of one kind with a value in one
  loop — numbers, moments or text — where it had asked `holds` of each
  cell: a day's 871,527 timestamps against an instant 12 → 7 ms. A
  column is still read into its cells first (6 ns a row); a column
  value kind in the interpreter is the fix behind this one.

- Ending a process `start` began signals its own group and nothing else
  on Linux: the group had been written `kill -TERM -<pid>`, which
  procps's `kill` (4.0.4, Ubuntu's) reads as a signal to every process
  the user may signal when the pid begins with 1 — on CI it ended the
  unit tests' step and a coverage runner, where macOS's `kill` had read
  it right. The group is named after `--`, and a pid of 0 or 1 is
  signalled not at all.
- A line that holds only a comment opens and closes no block, whatever
  its indentation, as a blank line does (spec, Blocks), in a file and in
  the console. A trailing comment continued aligned under itself had
  been refused ("this line is indented, but the line above it opens no
  block"), a comment at the margin inside a block had ended the block,
  and one before a chain's first `then` step left the step indented
  under nothing.
- A string written across lines counts its lines: a stop, an error's
  trace and a refusal after one had named a line short by every line
  break the string held, in both modes (`line 4` for line 6). A refusal
  on the line such a string closes on is placed and underlined there, in
  the terminal and the editor.
- A crowfile holding `cpu` or `gpu`, keys the spec documents, stopped
  every command ("'cpu' is a core builtin — pick another name for this
  variable"): the crowfile's names are keys, not a program's variables.
  A key another key reads is no longer read back as `done`. Both modes
  take both counts, for the entry and for a job (a compiled run had
  taken `cpu` alone), `crow build --ir` and `--rs` print the entry with
  them, and `crow run -i <job>` runs the job interpreted (it had read
  the job's name as a file's).
- In the console and a notebook, a function one cell defines is the next
  cell's to call (an internal error: "calls 'double', which the program
  does not define"), and a cell that defines a function again runs its
  new body.
- `crow new`: `--db` wrote a `table users` block that does not parse and
  listed a `jobs/migrate.co` it never wrote; `--ml`'s job called `train`
  bare, which is undefined. Every file a flavour scaffolds runs now,
  held by a test of each flavour.
- `crow fix --hazard`'s constraint migration is written as the next
  numbered file and applied as `crow db apply` applies it, recorded in
  `_crow_migrations` (the next `crow db apply` had run it again: "index
  … already exists"). With `--db migrations://dir` it goes in that
  directory ("unknown database URL scheme"). The check prints the exact
  command, the `--db` it was handed included, and the exact file name.
  No message names `crow migrate`.
- `crow check` reported a control whose handler only runs `CREATE TABLE`
  (a setup button, as the book writes one) as a dead control: a database
  write is something a control does. Its reach named a writing query
  `__query_write`; it names it `query`. A failed example truth was
  reported "(at n = 0)", naming an input it never read (the book's
  chapter 9 `fee`): a failure names the inputs only where its line reads
  one.
- A program's own `db/schema.co` is imported by its dot path, `use
  db.schema`, as `views/home.co` is: every `use db.…` had been refused
  as a driver's own module (a driver's, `db.postgres`, still is).
- A second derivative is refused before the program runs, as the spec
  says (Automatic Differentiation): `fn df[x]` answering `grad[f][x]`,
  then `grad[df][2.0]`, had answered 0 in both modes — the inner
  gradient ran on the dual numbers' values alone — where the second
  derivative of x³ at 2 is 12, and `grad[grad[f]]` had stopped as it
  ran, interpreted, and not built compiled. `grad` inside a function
  `grad` differentiates, or inside one it reaches, is refused at its
  line (`grad: df calls grad, and a function grad differentiates
  cannot`), and `grad` handed a gradient where it is written; a function
  that computes a derivative and is differentiated by nothing runs as
  before.
- A gradient held in a variable takes what a call takes: `dloss =
  grad[loss]`, then `dloss[w, xs]` with `xs` read by `load`, had been
  refused on its only assignment ("cannot reassign 'dloss'"), where
  `grad[loss][w, xs]` written inline ran. A gradient applied to a value
  that failed answers that failure, the leftmost, in both modes (the
  interpreter had handed it in, and the compiled call had stopped or
  answered it by whether the failure was the first argument), and a
  function value applied twice to a local that can fail builds compiled.
- `y = 1.0`, then `y = log[3]`, was an internal error ("the lowered
  program disagrees with itself"). The answer of a call that answers
  nothing — a builtin whose row declares no answer (`log`, `sleep`,
  `exit`, `seed` …), or a function of the program's whose every return
  hands on nothing — given to a variable, a field, an item or names
  assigned together is refused where it is written, in both modes:
  `log[] answers nothing, which 'y' cannot hold — call it on a line of
  its own`. A new variable given one had held `done`, and a record's
  field had changed kind.
- A line that drops an index by a mask (`df[df.age gt 50]` alone) is
  refused naming the form the program wrote — `` `f[mask]` answers a
  frame, and this line drops it — keep it: df = df[…] `` — where it had
  named the hidden `__pick_frame[]`; `crow fix` and the language server
  apply the keep.
- A call through a standard module of a name it does not have (`use
  math`, then `math.double[5]`) is refused before the program runs, the
  nearest of its names offered where one is a slip away (`math has no
  'meen' — did you mean 'mean'?`): `math`, a name its own import makes
  so that its constants read, had let any `math.f[…]` by, typed as
  nothing known, to stop as the program ran, interpreted, and not build
  compiled.
- A column the program writes on a loaded frame (`df.when = …`) is the
  program's from then on: what it reads of it afterwards asks nothing of
  the file. A column of moments written and compared had stopped the
  load ("the frame has no column 'when'").
- A statement handed a parameter that failed answers the failure, as any
  call does: the interpreter had stopped (`cannot bind done as SQL
  param`), inside `transact` too, where the compiled program answered
  it. A list parameter holding an item that failed builds compiled. In a
  handler a read answers the failure and a write stops the handler, in
  both modes; a read had stopped it in both.
- `insert` answers the database's refusal (rows that break a constraint,
  a column the table lacks) as an error value of kind `'db'`, in both
  modes: the compiled program had unwrapped it and panicked, and the
  interpreter had stopped.
- `describe[conn, sql]` answers what the compiler reads: a computed
  column typed from the statement, a column that can hold NULL marked
  `?` on SQLite and Postgres, `IN ?` described as a list of one. It had
  answered the database's word alone.
- `crow test --docs` reads a call of a name only where `name[` begins a
  name: the coverage check could have counted a builtin as called inside
  a longer name, and the standard library's pages had linked `area[…]`
  to the `a * b` row and `greet[…]` to the moment's `t` rows.
- The task board's committed baseline is the app as it is (`query` where
  it had `exec`, reach by effect kind): it had been accepted before both
  changed, and no suite diffs it.
- The book's site is built again (`docs/book/site.co`): its page view
  read a file for the mark's version and the harness's sample chapter
  was read inside a view, which a view may not do since effects have
  kinds — CI's test job and the Pages workflow had failed at that step
  on every push since 2026-10-20, so the live book stayed at
  2026-10-18's. The version is worked out once and handed to the view,
  the sample's chapter is written in the program, and the gate's list
  in `CLAUDE.md` runs the step as CI does.
- The book's contents list the `chart` and `sys` reference pages: the
  site writes the pages its contents list, and the standard library's
  index had linked two it never wrote.
- Examples that did not run: `examples/dashboard.co` queried inside its
  view, refused since effects have kinds (it reads through a derivation,
  as the book's last chapter does); `examples/multifile` named its own
  module `math`, which `use math` never reaches (it is `arith`, and a
  function is handed `pi` as a parameter, which it does not see
  otherwise); and the parallelism examples described statements run in
  waves and a loop that logs running at once — they show a loop whose
  body only computes, timed at one thread and at eight. Five of the
  book's error blocks were refused for an indented comment rather than
  the mistake they show; each is refused for its own now.
- The interpreter had run two to four times slower since the compiler's
  freeze began (n-body 2.0 s to 8.0 s, k-nucleotide and reverse
  complement twice their time; the compiled program untouched): the
  questions it asks of a builtin's name after every call — whether the
  answer is stamped, whether a list grows in place, whether a rehearsal
  records the call — had become scans of the whole table by name when the
  hand-kept lists became the table's. The table is indexed by name, built
  once, a row's stamp is read by its place in the table, and a rehearsal
  is asked about first; n-body is back to 2.2 s. A test holds the index to
  the scan's answers and to being far faster than one.
- A parameter reassigned in its body from a value that can fail (`words
  = maybe[1]`) keeps the kind its calls hand it: the body had typed an
  untyped parameter as a value that can fail before any call typed it,
  and a call's plain argument — a function's answer, a local, a value
  that can fail — was then handed bare to a `Result` parameter; the
  interpreter ran it and the compiled program did not build (E0308),
  while a literal argument, which types the parameter first, built. The
  type had depended on the order inference met the body and the call.
  `act[…, system = prompt[]]` is such a call: the loop's `system` is
  reassigned from the skills' prompt (parity 278; found building
  examples/agent).
- A part's function is named as the program wrote it wherever `describe`
  names it — the name a model is told, calls back and the steps frame
  records, and the name a refusal uses: it had been the part-qualified
  internal name (`agent_support_order_status`) when an entry imported the
  part and the written name when the part was the entry, so a trace and a
  recorded fixture differed by which file was run (tests/described.rs).
- `crow check`'s snapshot store keeps an answer holding a line break: it
  had written the answer whole, one line per answer, and read it back as
  its first line, so a function answering several lines was reported
  changed by every check after the first. A backslash, a line break and a
  carriage return are escaped on the line now; a stored answer holding a
  backslash is rewritten in that form at the next check, and reads the
  same.
- A program's function in a group handed to a standard module's function
  (`tools = [weather]` to `act`) was never noted as called where the
  module applied it by name, since a program's functions are invisible
  inside a module: the function stayed untyped and the compiled program
  called one it did not hold ("cannot find function `weather`"). The
  note records the signature itself now, and the dispatch's call of such
  a function is lowered as the program's call it is, where the interpreter
  had met a by-name call of a function it does not run (parity 274,
  tests/llm_server.rs).
- A program's own top-level value named with a standard module's prefix
  (`mcp_s = server[…]` under `use mcp`) was printed as a Rust constant,
  by the rule that hoists a user module's values, and the compiled program
  did not build: a standard module hoists no value, so its prefix names
  none now.
- A function copied per signature was scanned without entering its own
  scope: its first assignment to a name was not a local of its own, and
  inside a standard module's copy the program's top-level names were
  visible. The copy of llm's `act` made for a second tool list read the
  program's `one` — the record the first call answered — as the loop's
  local `one`, and refused the module's own push in both modes (parity
  275). A copy's scan enters the function's scope as the function's own
  does.
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
