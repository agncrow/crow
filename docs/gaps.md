# What Crow Does Not Have

What Crow does not have yet, gathered on 2026-10-29 so each gap can be
weighed for the [roadmap](roadmap.md). Most of it was found by writing the
696 tasks of Rosetta Code in Crow ([rosetta/](../rosetta/README.md)): 6 could
not be written, 23 were written in part ([REVIEW.md](../rosetta/REVIEW.md)),
and the notes of many more in `rosetta/tasks.tsv` say what stood in for a
missing piece. The rest comes from the spec's own "not built" sentences, the
roadmap's open items and the decisions recorded in
[rosetta/QUESTIONS.md](../rosetta/QUESTIONS.md).

Each gap gives the tasks or documents that met it, what stands in today, and
a size by what building it touches:

- **S**: a row in the table (a builtin or an option) and its runtime
  function, with its spec line and a parity test.
- **M**: a module, a kind, or a rule in the compiler. A change to the
  compiler is a language change, which Vijay decides (design rule 24).
- **L**: a subsystem: a backend, a platform, a debugger, a toolkit.

What Crow leaves out on purpose is not a gap: it was rejected on the record,
most of it in a Design Note of the spec, and
[feature-map.md](feature-map.md) says what to use instead. The last section
lists those briefly, so the roadmap does not take one up without revisiting
its decision.

## Numbers and text

| Not built | Met by | Today | Size |
|---|---|---|---|
| An exponent in a number literal (`2.7e-16`) | Apply a digital filter, the NKTg law tasks | refused by the lexer ("Crow writes no exponent in a number"): `2.7 * 10.0 ^ -16`, or `float['2.7e-16']`; the spec does not say why | S, language |
| Hex, binary and octal literals; digit separators (`0xff`, `1_000`) | SHA-1, Pseudo-random numbers/Xorshift star | decimal literals only; `int[text]` reads decimal digits | S, language |
| `fmt` in scientific notation, hex or binary | the NKTg tasks (a helper writes the exponent), SHA-1 (hex by hand), Two identical strings (base 2 by hand) | `fmt` has `decimals`, commas, width, align, fill and percent; `encode` knows base64 and url | S each |
| Complex arithmetic: `*`, `/`, `^`, `exp`, `log` of a complex number | Euler's identity, Fast Fourier transform, Conjugate transpose | math's complex numbers are `[re, im]` pairs with `complex`, `real`, `imag`, `magnitude`, `phase` and `conjugate`; the arithmetic is written by hand | M |
| SHA-1 (and other hash schemes) for programs | SHA-1 | `hash` takes sha256, sha512, md5 and password; `sys.hash` has sha1, but `sys` is the library's own | S |
| Text encodings other than UTF-8 (Latin-1, UTF-16) | I before E except after C (a Latin-1 word list, converted with iconv through `shell`), Unicode strings (UTF-16 by hand) | `read` takes UTF-8 only | S once bytes are readable |
| A leading `+` (`+x`) | Exponentiation with infix operators | refused; write `x` | S, language |

## Files, processes and the system

| Not built | Met by | Today | Size |
|---|---|---|---|
| Binary file input and output: bytes 128 to 255 | Fixed length records, Musical scale, Sine wave, Bitwise IO, Base64 decode data, Random number generator (device), Find duplicate files; the roadmap's byte-level BPE tokenizer | `read` and `write` take UTF-8 text (`read` refuses other bytes); `request[…, save = path]` saves downloaded bytes. Deferred on 2026-10-29 until a program needs it; the spec's Design Note "the standard library's layer" rejected handing programs `sys`'s bytes, so this needs a design of its own | M |
| A file's size, times and kind (stat) | File size distribution, Find duplicate files, Truncate a file | `sys`'s `filesize` and `modified`, which programs cannot import; `wc -c` and `ls` through `shell` | S |
| Walking a directory tree; glob patterns | File size distribution | `ls[path]` lists one directory | S |
| Appending to a file, and truncating one | Stream merge (appends through `shell`), Truncate a file (the system's `truncate`) | `write` replaces the file | S |
| Reading a file a line or a block at a time | Stream merge (reads through `tail` and `head` in `shell`) | `read` and io's `lines` take the file whole. The spec's Design Note "reads bounded by what a frame holds" keeps Magpie's reads bounded, so a reader that keeps a position needs a design | M |
| A monotonic clock, finer than a millisecond | Rate counter | `now[]` is UTC wall time to the millisecond | S |
| Terminal control: a key without Enter, the cursor, clearing, flushing pending input | Keyboard input/Obtain a Y or N response, Metronome; the games read lines | `input[]` reads a line; colours work as escape codes in text | M (a module) |
| Starting and talking to a process | the roadmap ("process management") | `shell[cmd]` answers a command's output; `sys.start` runs a process with pipes, for the library only | M |
| Windows itself | Write to Windows event log | macOS and Linux; Windows inside WSL (the book's Getting Started; docs/distribution-design.md keeps Windows unsupported) | L |

## Media and the desktop

| Not built | Met by | Today | Size |
|---|---|---|---|
| A desktop window, mouse and keyboard events, the clipboard | Clipboard manager/display (not possible), Animated spinners, Colour bars, Greyscale bars, Plasma effect, Honeycombs, Particle fountain, Draw a clock | views are pages served to a browser; the drawings are SVG, plain PPM and PGM files, or characters. The layout engine's desktop shell, font metrics and `theme[]` are not built (docs/ui-engine-design.md) | L |
| Sound | Metronome, Musical scale, Nautical bell, Sine wave | none: Sine wave writes a WAV whose every byte is below 128 and hands it to the system's player through `shell` | M for files once bytes are writable, L for a device |
| Images in binary formats (PNG, JPEG) | the same drawing tasks | plain PPM, PGM and SVG, written as text | M, after binary I/O |
| XML, YAML, TOML | (no task in the run) | tables are CSV, JSON and Parquet; YAML and TOML as configuration are out by design (environment variables, docs/web-framework.md) | M per format, in Crow |
| Control of the display's mode | Video display modes (not possible) | none | not worth a row |

## The language and its compiler

Each of these changes the compiler, so each is a language change.

| Not built | Met by | Today | Size |
|---|---|---|---|
| A function that takes a function, copied per kind; a function used at several signatures passed as a value | Monads/List monad (`bind` written once per kind), Extend your language | the spec's two documented limits of Specialization: such a function serves one kind | M |
| Variadic parameters for a program's own functions | Function prototype | builtins take rest arguments (`concat`, `randn`, `anova`); a program passes a list (docs/extending-crow.md, phase 4) | M |
| Arithmetic over maps (`2 * model`, a map plus a map) | the spec | refused; its rule is recorded in the spec's note for when it is built | M |
| Second derivatives (`grad` of a gradient) | the spec | refused: "a second derivative is not built" | M |
| Columns of decimals or fractions in a frame | the spec | refused where made; a database's decimal column arrives as floats | M |
| A function held as a value answering an int past 64 bits | the spec (Numbers That Keep Every Digit) | refused where the answer crosses: such a call is typed with 64-bit ints | M |
| Test blocks run compiled | the spec (test blocks) | they run in the interpreter; `crow test --compiled` compiles the apps they open | M |
| The interpreter's remaining overhead | the roadmap: about 30 machine instructions per interpreter instruction, against 10 to 15 for a lean one | measured, not built | M |
| The Rosetta run's open findings | `weiler_atherton_polygon_clipping-3` (a function nothing calls is not checked for answers of two kinds), `hash_join-1` and `gaussian_elimination-1` (a second, cascading refusal after the right one) | listed in rosetta/findings/README.md | S each |

## Tools

| Not built | Today | Size |
|---|---|---|
| A debugger: breakpoints, stepping, a look at the locals | `log`, the REPL, and `crow build --ir`, `--rs` and `--types` | L |
| A profiler | the repository profiles with macOS's `sample` | M |
| Coverage of a program's tests | `scripts/coverage.sh` measures the compiler's crates only | M |
| Building for another platform | `crow build` builds for the machine it runs on; releases are built per platform | M |
| Compiled builds without the Rust toolchain | `crow setup` fetches the pinned toolchain; an in-process backend is "after 1.0" (docs/distribution-design.md) | L |
| A lint for a condition on a value that can fail, unguarded | the spec names it a future `crow check` lint | S |

## Domains the roadmap already holds

Kept here so the list is whole; each is an open item of [roadmap.md](roadmap.md)
or a design document.

- **Distributed runtime (P6, after 1.0):** the ring in
  `crow_runtime::distributed` has no builtin, and nothing has run across real
  machines; sharding, gradient synchronization and the parallelism strategies
  wait on it.
- **The LLM domain (P8):** AdamW, a byte-level BPE tokenizer (it needs a
  text's bytes) and `grad` over a list of tensors; durable runs (nothing built,
  a decision for Vijay, docs/agents-design.md); serving; parallel tool calls
  ("not in this round").
- **GPU:** it computes in 32-bit floats, about 7 significant digits against
  the CPU's 15 (wgpu's limit, the spec's GPU section).
- **Magpie:** a SQL subset was set aside (docs/magpie-design.md).
- **The server:** a cap on sessions per client ("a candidate after 1.0",
  docs/security-review.md); sign-in beyond a password and a token.
- **The runtime's deferred items** (spec): freshness thresholds, a source's
  refresh backoff, handing a slow handler off, durable jobs, a `clock`
  capability, `raw[s]`.

## No decision recorded

Absent, with no Design Note for or against. Each wants a decision before a
row on the roadmap: build it, or record why not.

- **`goto` and labels** (Jump anywhere): control flow is structured; the
  spec never says so.
- **Operator overloading** (Tropical algebra overloading): records take
  named functions.
- **`eval` of text at run time** (Runtime evaluation): the task writes the
  text to a file and runs `crow run -i` on it through `shell`.
- **Foreign calls, raw memory, pointers and memory layout** (Machine code,
  Memory layout of a data structure, Arena storage pool): no FFI;
  docs/discovered-features.md lists FFI as undecided, and the roadmap took
  the in-language route over Python and ONNX interop.
- **A record whose field holds a different kind in each use** (Euler's
  identity: one `Complex` cannot hold floats in one place and decimals in
  another): functions are copied per kind, records are not.
- **Compile-time evaluation** (Compile-time calculation): the book's
  Appendix B says there is nothing to compute at compile time, but no note
  weighs it.
- **A topic variable** (`$_`, `it`; Topic variable): none, in the REPL too.

The two literal forms and the leading `+` above belong here as well: the
lexer refuses them, but the spec records no reason.

## Absent by design

Each was rejected on the record, with its reason and the way instead: in a
Design Note of the spec, or (client-side WASM, an ORM, YAML and TOML) in the
roadmap or docs/web-framework.md. [feature-map.md](feature-map.md) tables
most of them. Taking one up means revisiting that decision first.

- Lambdas and closures; iterators, laziness and generators; `map`, `filter`,
  `reduce`, `enumerate`, `zip`.
- Classes and methods (a struct is data); macros and user-defined syntax;
  generics and written types (types come from use); shadowing; tuples and
  destructuring.
- `match` and sum types; ternaries; `null`, `try`/`catch` and inline
  fallbacks (errors are values).
- Second forms of one thing: `+=`, double quotes, `+` of texts, a format
  mini-language, bit operators (`use bits` instead), `x.f[a]` method calls.
- `break` and `continue` in a `for` (a `for` may run in parallel).
- Threads, channels, locks, raw sockets and bytes for programs (the `sys`
  layer is the standard library's own); async/await, timers and `every[]`.
- The machine's local time zone, and a duration kind (seconds serve).
- Script tags, inline JavaScript and client-side WASM; a package registry
  (a dependency names its repository); an ORM (SQL is written);
  configuration in YAML or TOML (environment variables).
- Non-ASCII identifiers; selective imports.
