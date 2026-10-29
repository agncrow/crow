# Magpie in Crow: the kernels rewritten, and what the language gains for it

Decided by Vijay, 2026-10-27: a package is written in Crow, and its
author writes no Rust. Magpie is the first package held to that rule,
and it is the hardest case, because two thirds of it is Rust today:
the kernels under `crow_runtime/src/magpie/` (7,500 lines: the file
format, the codec, the encodings, the merge, the sequencer, the
executor, the postings and the search, the nearest scan, the bloom
filters, the gzip and snappy decoders, the stemmer) and the Kafka
records kernel (`crow_runtime/src/kafka.rs`), reached from the Crow
modules through nineteen rows of the sys layer. This document is the
plan to rewrite every one of them in Crow, held to the Rust original
as the oracle, and the five things the language lacks for that, each
checked against the spec for an existing way (design rules 1 and 24).

What stays Rust is the language's own: frames, tensors, text, sockets,
the core pool, the compiler. crow_runtime keeps its kernels; a package
gets none.

## 1. The rule, and what it costs

A kernel written in Crow runs at the interpreter's speed under
`crow run -i` and at the compiled program's under `crow run`. For
per-byte work the interpreter is two orders slower than Rust, so a
package's kernels must be compiled ahead of the programs that use
them: the assembly step of the package plan (docs/magpie-repo-design,
to be written) compiles the functions a crowfile names and registers
them as rows the interpreter calls natively, so both modes run the
same compiled code. Until assembly exists, Magpie's kernels inside the
standard module run interpreted under `crow run -i`: correct, held by
the same tests, and slow. The measures in docs/magpie-design.md are
re-run in compiled mode at the end of the port and the interpreted
numbers stated as they are, which is the case for assembly.

Compiled Crow over bytes and ints is the evidence the port stands on:
revcomp, k-nucleotide and fasta run within a small factor of Rust,
after the emitter's work on text builders, plain vectors and
known-ASCII reads. The CSV parser that measured 20 to 45 times slower
than the runtime's did per-cell work through values that can fail; the
primitives below are chosen so a kernel never does that.

The first kernel ported is the codec, and it is measured against its
Rust original on the same input before the second is begun: if
compiled Crow lands within about two times, the port goes on; if not,
the emitter is where the work is, not the kernels.

## 2. What Crow lacks, and the form each takes

Every item below was searched for in the spec first. Where a form
exists it is extended by kind or by an option, never doubled.

### 2.1 Bytes written in place

What exists: `bytes[text]`, `text[b]`, `len[b]`, `byte[b, i]`,
`slice[b, from, to]`, `find[b, needle]`, `concat[a, b]`, `pack`,
`unpack`, `ieee[x]`, `hex`, `hash`, `digest`, `hmac`, `random[n]`,
`mask`, `xor`, `same`. A bytes value is `Bytes(Vec<u8>)` in the runtime,
held plain in both backends. What a kernel lacks is writing: a decoder
builds its output a byte at a time, an encoder writes lengths into
what it wrote, a hash table of positions is a vector of ints.

- `bytes[n]`: `n` zero bytes (a second `bytes` row, by kind, beside
  `bytes[text]`).
- `set[b, i, v]`: the byte at `i` replaced by `v`, 0 to 255, counted
  from the end when negative, outside stops (the `set` the lists and
  maps have, for bytes).
- `push[b, v]`: the byte appended (the `push` the lists have).
- `concat[a, b]` with `a` moved: appended where `a` lies.

All three in place through what exists: `Prop::InPlace` on the row, a
take bridge for the interpreter's moved argument, a `taken` path the
emitter prints when the first argument is a moved local
(`taken_in_place` splits `b = set[b, i, byte[b, j]]` as it splits a
list's). Reads stay `byte[b, i]`: a kernel's loop reads and writes by
position, and the vector of ints a hash table needs is a one-axis
tensor, which the emitter already keeps as `Vec<i64>`.

### 2.2 Numbers in bytes

What exists: `pack[n, width]` and `unpack[b, at, width]` are
big-endian and signed, as Kafka's wire is; `ieee[x]` is a float's eight
bytes, big-endian. Magpie's format is little-endian with unsigned
lengths, and a decoder reads floats.

- `pack[n, width, little = false]`, `unpack[b, at, width, little =
  false, signed = true]`: the options, defaults as today.
- `ieee[x, width = 8, little = false]`, and `ieee[b, at, width, little]`:
  the bytes at `at` as the float they spell (a second `ieee` row, by
  kind, the inverse of the first), 4 for single precision; the reader's
  width and byte order are positions, since an option may be given by
  position and `ieee[b, at, 8]` would be the writer's row with a width
  and a byte order.
- `digest[b, 'crc32']` and `hash[b, 'crc32']`: the checksum gzip and
  Kafka's message sets carry, four bytes, written by us as the other
  digests are not (a table-driven CRC-32 is twenty lines; the spec's
  "what stays native" note names no crate for it).
- `wrapmul[a, b]`, `wrapadd[a, b]`, `wrapsub[a, b]` in `use bits`: the
  product, sum or difference of two 64-bit patterns wrapped to 64 bits,
  the int of the low bits. A hash (FNV's multiply per byte, murmur's
  mix), a linear congruence and a checksum are written on wrapping
  arithmetic, which a language whose ints grow has no way to say: the
  product of two ints past 2^32 is a growing int, exact and slow, and
  `bitand` of it with a 64-bit mask is a growing int's operation too.
  The three rows answer a 64-bit int, so the width pass keeps what is
  computed from them narrow.

What the width pass needs from a kernel's loop, found porting the codec
and the packing: bits gather by `bitor`, never by `+`, since a sum in a
loop is a growing int to the pass and `bitor` of two non-negative ints
is bounded by the larger's power of two; a value wider than 32 bits is
packed as its low 32 bits and its high bits apart, so no shift passes
64 bits; and a 64-bit pattern at or past 2^63 is held as the negative
int of the same bits, assembled as `lo + shift[hi, 32]` where `hi` is
read as signed in a branch the pass bounds. `CROW_WIDTH_DEBUG=1` shows
the settled ranges; a kernel is not done while one of its loop's ints
is wide.

### 2.3 Files

What exists: `file[path]` (bytes, whole), `store[path, bytes]`,
`filesize[path]`, `read`, `write`, `exists`, `mkdir`, `ls`, `rm`. The
write path of a batch is: write a part file, flush it to the disk,
link it into place (never over an existing name), flush the directory.
A reader opens a file's footer, then one chunk. The footer cache is
keyed by path, size and modification time.

- `file[path, from, n]`: the bytes of a region (a third `file` row,
  by arity).
- `modified[path]`: when the file was last written, a moment.
- `flushed[path]`: the file, or the directory, written through to the
  disk (`F_FULLFSYNC` on macOS, `fsync` elsewhere); answers the path.
- `link[from, to]`: a hard link, refused with the system's reason
  where `to` exists; answers `to`.
- `move[from, to]`: renamed over `to`, atomically, as a manifest is
  replaced; answers `to`. (`rename` is the frame module's.)

### 2.4 Process memory

What exists: top-level variables, invisible to functions; facts, a
served app's state, typed by their declaration and held by the session
layer; `env`; the notes a connection keeps (`note`, `noted`: text, per
stream). Nothing a module's functions can hold between calls for the
process, shared by its threads. The sequencer is that: the rows put
since the last flush, the key index, the image of each key's newest
row; so is the footer cache, and the registry of what a process has
opened.

- `remember[name, v]`: the value held for the process under the name,
  replacing what was held.
- `recall[name, default]`: the value held, or the default where none
  is; typed by the default, as `env[name, default]` is answered by its
  default's shape.
- `forget[name]`: nothing held under the name.
- `revise[name, default, f, x]`: `f[held, x]` run under the name's
  lock, with `held` the value held (the default where none), the value
  `f` answers held afterwards and answered; `f` owns the value it is
  handed, so a map's insert or a list's push inside it is in place.
  One schedule at a time per name, so two handlers writing one log
  serialise by construction, as the Rust sequencer's lock serialises
  them.

The store is per process and per mode: the compiled program keeps
typed slots, the interpreter its values; the semantics are one. Two
effect kinds carry the rules: `held` for a read, `held.write` for
the rest, allowed and refused where `db.read` and `db.write` are (a
view reads neither; a derivation reads, never writes), ordered in a
loop for the writes, recorded by no rehearsal since nothing leaves the
program. One kind of the row vocabulary is new: `Kind::Any`, the kind
of the argument handed where it is taken, which types `recall`'s
answer by its default and `revise`'s function by its default and the
rest of its arguments, as `later[f, a, b]` hands its rest to `f`.

Design Note, recorded here for the spec: a timer was considered for
the sequencer's flush and rejected again (the spec's "Periodicity Is
Declared, Not Written"). A `put` appends to the process's memory and
schedules `later[flush_all]`, with no argument, which is allowed in a
handler too: coalesced, so a burst of puts shares one flush; waited
for at exit, so nothing put is lost to a clean end; run inline by the
interpreter, so under `crow run -i` a put is durable when it answers,
which is a legal schedule and a slower one. The ten milliseconds of
the Rust thread become "as soon as the loop is free".

### 2.5 Columns as their parts

What exists: a loop over a column yields its cells, each a value that
can fail where the cell can be missing; `df.name` reads a column and
`df.name = values` writes one from a list, a tensor or a column; a
frame's `columns`. What the writer and the reader of a file do instead
is read a column's rows `start..end` as the vectors they are held in and
build a column from such vectors, with no value per cell: the Rust
writer encodes `Vec<i64>` and a validity bitmap; the reader hands the
frame a `Column` it decoded whole. The CSV parser's factor came from
the per-cell path, and the file format's chunk is the vector's shape.

Rows of the sys layer, each over a column and a range of its rows,
answering what a chunk holds: `ints_of[col, from, to]` (missing as 0),
`floats_of`, `bools_of`, `millis_of` and `zones_of` for a moment column,
`vectors_of` (the rows' floats laid flat) with `vector_width[col]`,
`text_bytes_of` with `text_ends_of` (each cell's end offset, as the
text is held), `present_of` (the validity as a bitmap's bytes, bit i of
byte i / 8, the chunk's own layout, empty where every cell is present),
and `cell_kind[col]` (`int`, `float`, `text`, `bool`, `moment`,
`vector`). The inverse builds a column from its parts: `ints_column[ints,
present]`, `floats_column`, `bools_column[bits, present]`,
`moments_column[millis, zones, present]`, `vectors_column[width, floats,
present]`, `text_column[bytes, ends, present]`, each a `Column` the
program writes onto a frame with `df.`name` = …`, which exists. A
distinct count is exact: a map keyed by a 64-bit hash of the cell's
bytes holding the byte strings that share it, compared with `same`.

### 2.6 Parallel decoding

Nothing new. A loop over file reads is ordered, since a read is an
effect; a kernel reads the groups it needs first, then decodes them in
a loop with no effect, which the compiler runs across the cores.

### 2.7 Packages and the sys layer

`use sys` is refused in a program and allowed in a standard module. A
package that holds kernels needs it: bytes, files, hashing, memory.
Proposed, for Vijay: a package's parts may `use sys`, as the standard
library's may, since a package is library code under the same rules;
a program still may not. Not needed until Magpie leaves the repository.

## 3. The port

Order, each kernel a milestone with its oracle, a commit after the
gate:

| Kernel | Rust | Crow module | Oracle |
|---|---|---|---|
| bit packing (`pack.rs`) | 211 | `magpie.pack` | the Rust rows, test-only, byte for byte |
| codec (`codec.rs`) | 217 | `magpie.codec` | byte for byte; the measurement of section 1 |
| encodings (`encode.rs`) | 580 | `magpie.encode` | a chunk's bytes identical for every kind, round trips |
| bloom (`bloom.rs`) | 95 | `magpie.bloom` | the same bits for the same values |
| file (`file.rs`) | 841 | `magpie.file` | a file written by both is identical; each reads the other's |
| manifest (`manifest.rs`) | 174 | `magpie.manifest` | identical bytes |
| sequencer (`tail.rs`) | 570 | `magpie.tail` | the module's tests, the crash harness, the pending listing |
| merge (`merge.rs`) | 380 | `magpie.merge` | the segment identical for the same inputs |
| postings, search, stem (`postings.rs`, `search.rs`, `stem.rs`) | 1,045 | `magpie.search` | the same ranking and scores to the last digit; Lucene's forms |
| executor (`exec.rs`) | 740 | `magpie.exec` | `summary` as `group` over `scan`; `prior` as the as-of join |
| nearest (`nearest.rs`) | 268 | `magpie.nearest` | the flat index's positions and cosines |
| inflate, snappy (`inflate.rs`, `snappy.rs`) | 497 | `magpie.inflate`, `magpie.snappy` | the reference streams the Rust tests hold |
| records (`kafka.rs`) | 334 | `magpie.records` | message sets identical both ways; kafka-python |
| the verbs (`mod.rs`) | 1,594 | `magpie` | the module's tests; parity 302, 303; the doors' tests |

During the port the Rust kernels stay, reached by test-only rows
(`Prop::TestOnly`) a module test compares against; at the end of each
milestone the Crow kernel is what the verbs call, and when the last is
through, `crow_runtime/src/magpie/`, `kafka.rs`, the sys bridges and
the nineteen rows go, and `tests/freeze/names.txt` shrinks by every
name of Magpie's the compiler held.

The modules are registered under `magpie` (`under: Some("magpie")`, as
`web.auth` is under `web`), each a file beside `magpie.co`, imported by
the verbs with `use magpie.file` and by nothing else. A kernel's
function is exported from its module and helpers keep their
underscore, as every Crow-written module does.

## 4. What is measured, and when

After the codec: compress and decompress of 50 MB, compiled at
`crow build`, against the Rust codec on the same bytes. After the
file module: a month's segment written and read by both. At the end:
every number of docs/magpie-design.md's measured section, compiled,
beside the Rust numbers it replaces, and the interpreted numbers
beside them as the case for assembly. The article is revised then.

### 3.1 Ported so far (2026-10-27)

`magpie.codec`, `magpie.packing`, `magpie.encode` (the chunk: every
column kind, the kernel's bytes byte for byte, tested in both modes,
the long columns through the codec), `magpie.bloom` (the filter, bit
for bit, its positions held to the kernel's over ten thousand probes),
`magpie.stem` (word for word) and `magpie.postings` (the index of a
row group, byte for byte, blocks included). Two traps a kernel in Crow meets, recorded for the next ones:
a list literal of ints (`[h, g]`) is a tensor, which holds a 64-bit
pattern as a float and rounds it — two hashes travel in a record; and
`%` keeps the sign of its left side, so a pattern read as unsigned is
`x % m`, made non-negative, plus 2^64 modulo m, modulo m again. What the port added to the
language beyond section 2: `wrap[x]` (a growing int's low 64 bits),
`float_bytes`/`float_values` (a chunk's floats at once), and
`text_dict_column` (a text column kept as its dictionary, as the chunk
holds it). What it found: a module row's call printed without the
owned-kind rule (a list moved at every call, E0382), `encode` and
`decode` being the core's names (the exports are `chunk` and `unchunk`),
and the interpreter's placeholder for a missing float cell (NaN) differing
from the compiled program's (what was written) — the column rows read a
missing cell as one placeholder now, in both modes.

`magpie.file` (2026-10-28): the file — the writer in parts
(`columns_of`, `group_bytes`, `finish`, and `file_of` over them, so a
merge can stream groups), the statistics and the token index with the
kernel's rules, the reader by region (`opened`, `entries`, `groups_of`,
the filters, `about`) — held to the kernel through `file_rs` byte for
byte (plain, narrow and stemmed) and to `batch_read`/`batch_stats` on
the way back, the torn and foreign files refused in the kernel's words,
the same program run in both modes. What it added to the language:
`stack[frames]` of a list of frames (a `Frames` kind beside `Chunks`,
as `stack[tensors]` takes a list: the groups a reader decodes apart are
stacked in one pass, where `stack[a, b]` in a loop copied the whole each
round), `filled[filter, items]` in `magpie.bloom` (a group's distinct
values added in one pass; `add` per item copied the bits each time),
and `filesize` read for the file's tail. Traps: a record's field may
not be named `kind` (reserved with `ok`, `reason` and `trace` on every
value) — a `lo.kind eq 0` had compared an error's kind, text, with a
number, and `push[out, b.kind]` had typed the bytes as a list of text;
`split['', ',']` is `['']`, not an empty list — an empty list of text
is `slice[['x'], 0, 0]`; `set[xs, i, v]` is a map's and bytes' form,
a list's write is `xs[i] = v`; `reader` is a web form's name; the
rows-per-group rule is written by the frame's bytes as a plain text
column holds them (the kernel read the column's in-memory size, which a
program cannot see), so the oracle takes `per`. What it found in the compiler: a program's top-level
name made a function's branch-local of that name the program's in the
scoping pass (lower.rs `scoped`), so two sibling branches binding
`plain` to two kinds — the encoder's — were refused as a reassignment
only in programs holding a `plain`; a function's body now binds its own
locals whatever the program names, unless the program holds state
(parity 304); a list under a map's key grew by a copy of
the whole list at every push (`m[k] = push[m[k], v]`, and the `set` form
the builder had used): the postings of a 200,000-row column took 9.7 s,
the kernel's 0.1 s — a place now grows where it lies, in both modes
(parity 306), and the builder writes at the key; a record's field given
`[]` was never typed by what was pushed onto it, and a map of empty maps
was declared a map of text (both did not build compiled — parity 307); a
function taking a column shared one copy between an int column and a
float column (parity 305). Measured on 200,000 rows of five columns, one
indexed, compiled: the writer 265 ms against the kernel's 90, the reader
13 ms against 3 — the rest is per-cell work (a bound, a distinct value
looked up) the kernel does in a tight loop. Still the kernel's: the
footer cache per process (2.4, process memory) and the pending rows read
as a file in memory (the tail's).

`magpie.manifest` and `magpie.merge` (2026-10-28), and the verbs' reads:
the manifest byte for byte (`listed`, `declare`, `write_listed` beside
and over the old file, `catalog_of`); the streaming merge — one group
per input held, the rows decided up to the least of the inputs' last
loaded rows, a key's run held back until another key follows — held to
the kernel's segments and manifest at the production sizes and, through
`stream_rs`, at sizes small enough to cut several groups and segments,
in both the keep-latest and keep-every modes. The verbs now read files
in Crow: `lookup`, `latest`, `since`, `between` and `find` prune by the
blocks' bounds and filters and decode only the groups that may hold a
row (the pending rows of the tail are still read through the kernel),
`batches` reads the catalog and each file's `about`, `sync` flushes each
file and the directory, and `merge` folds through `merge_files`. What
the port added: a row that makes a column says its cells' kind (`cells`
on the row: `ints_column` answers a column of ints), so a column built
from its parts and written onto a frame keeps its kind; an empty frame's
column is an empty column of its kind in the interpreter as compiled.
What it found in the compiler: a failure pushed into a list whose items
were not known yet (parity 308), a module's local named like one of its
exports (309), a literal of items typed late (310), a computed column
name inside a function read as the program's (311), a function whose
second signature showed only after a round of copies left uncopied, its
calls answering nothing known (312), a function handed only an empty
list's items not compiled while its call was (313), and `select` of a
frame compiled as the dropdown in a page.

The executor, nearest, search and the Kafka door's bytes (2026-10-28):
`magpie.exec` (`summary_of` with the kernel's pruning, partials and
merge order; `asof_rows`), `magpie.nearest`, `magpie.search` (BM25 with
Lucene's query forms, the indexed groups' block skipping mirrored so a
tie at the threshold resolves as the kernel's does), and
`magpie.inflate`, `magpie.snappy` and `magpie.records` — each held to
its kernel's answers in both modes (inflate and snappy to the kernels'
own test streams), the verbs and the Kafka door rewired onto them. What the port found in the compiler: a part's computed column
name renamed apart from the entry's variable (parity 314), `keys` of a
map with no key yet guessed as text (315), an empty map widened with no
type Rust could read (316) or into the wrong kind (317), a plain value
written beside values that can fail refused or printed bare (318), and
a returned empty local declared a map of text (319). Left: the
sequencer and its process memory (2.4), then the kernels, their rows
and bridges deleted.

The sequencer, then the kernels deleted (2026-10-28): `magpie.tail` —
the memtable and its write-ahead log in Crow over process memory (section
2.4 as built: `use memory`, records held by name, one kind per name,
revised under the name's lock, a path's records forgotten by `rm`) — on
which `put`, `batch`, `sync`, `latest` of one key and every reader's
pending rows run; the crash harness's cuts (`pending`, `placed`,
`listed`) are the modules' own. Then the kernels went:
`crow_runtime/src/magpie/` and `kafka.rs` (7,580 lines), the sys rows
that reached them (`batch_*`, `tail_*`, `records`, `records_of`) and the
test-only oracles (`*_rs`, `stream_rs`). Their answers were recorded
first: each module's test holds its bytes to the kernel's digests (the
first sixteen hex digits of a SHA-256), its frames to the digests of
their columns' chunks, its refusals to the kernel's words, all read off
the run that checked them against the kernel one last time; the batch
files those tests write are written in Crow now (`write_batch`), and
were compared with the kernel's byte for byte in that run. What the
self-check found in the compiler once the ports were exports it calls
with every kind (92 cases, 14 groups): a column a function walks moved
by the walk (parity 321); a list of nothing read by a row typed by
nothing (322), negated as no mask (errors j82); a parameter handed only
the items of an empty list read as nothing where it is compiled as text
(323), a value of no known kind not copied, an effect's growing int cast;
a parameter its body widens kept at the calls' kind, a caller's answer
an earlier round's guess (324), a parameter reassigned another kind
taken for its type (j84); a body's guess of text standing against a
call's number (j83); and a list of frames answered as JSON in neither
mode (served 25). Once the sequencer and process memory were in, the
check's second pass (206 cases, 4 groups) found `remember` and `recall`
taking any value (errors j85), an answer of no value handed on making
a copy of `place` whose bytes were nothing (parity 328), and `len` of an
item of a list nothing fills compiled as a list's length of a text —
every program that only followed a log or read its metrics (parity 327).
The served pages found the last: a page calling `batch` with two frames
typed apart refused to start, since the sequencer's function handed to
`revise` was copied per frame and then refused as a value of several
kinds — a row that calls a function with its own arguments now hands
the copy they reach (parity 329).

## 4.1 Measured: the codec, 2026-10-27

The LZ77 codec ported (`src/modules/codec.co`), held to the Rust kernel
byte for byte on every input of the kernel's own tests, then both run
over the same 50 MB (a text repeated, one byte in 64 replaced by a
linear congruence's low byte): the Crow module compiled at `crow build`
against the kernel, warm, second of two runs.

| | Rust kernel | Crow, compiled | Factor |
|---|---|---|---|
| compress, 50 MB → 7,033,168 bytes | 68 ms | 153 ms | 2.2 |
| decompress | 9 ms | 34 ms | 3.5 |

Three things stood between the first number and this one, each a
compiler fault the port found, each fixed with its test: the emitter
asked the taken path (a buffer changed where it lies) for core rows
alone, so a module row's `push` and `concat` copied the buffer at every
byte, and the first run was quadratic; a bytes parameter a function
only reads was passed by value, so the hash of four bytes cloned the
50 MB input once per byte (770 ms once the first was fixed); and the
width pass bounded a right shift by the value shifted, so `shift[v,
-16]` of a 32-bit int was 32 bits to it, the hash's multiply of it a
growing int, and the hash ran on big ints. Compress is within the two
times the plan asks; decompress is not yet: its overlapping matches copy
a byte at a time through two rows, and its plain matches through a
`slice` that allocates, where the kernel extends the buffer from within
itself. A form for that, or a fusion of `concat[out, slice[out, …]]` in
the lowering, is the next optimization, after the file reader shows
how much of a read it is.

## 4.2 Measured: the port, 2026-10-28

The programs of `tests/magpie/measure/` built with `crow build` (the
port is Crow now, so the compiled program is the comparison), each run
once at the sizes its README names, every number the program's own (the
median of its warm runs where it takes several). Apple M4 Max, 64 GB;
other sessions paused, macOS's media analysis running (load 4 to 5).
The kernel column is docs/magpie-design.md's "Measured 2026-10-27",
taken interpreted on a quiet machine while the Rust kernels did the
work. "Before" is the port as committed (49f9ad2); "after" is with the
fixes below.

| Measure | Kernel | Before | After | After ÷ kernel |
|---|---|---|---|---|
| `put`, 2,000 then `sync` (per put) | 77.5 µs | 36,404 µs | 108 µs | 1.4 |
| `latest` by key | 13.5 µs | 109.5 µs | 14.0 µs | 1.0 |
| `batch`, durable | 12.0 ms | 69.4 ms | 13.9 ms | 1.2 |
| summary of 2 columns by key, 200,000 rows | 2 ms | 18,958 ms | 109 ms | 55 |
| a common word's top 10, indexed | 7 ms | 204 ms | 21 ms | 3.0 |
| a rare and a common word, indexed | 8 ms | 208 ms | 26 ms | 3.3 |
| a common word's top 10, scanned | 11 ms | 133 ms | 105 ms | 9.5 |
| merge 20,000 / 100,000 vectors of 64 | 74 / 311 ms | 420 / 2,157 ms | 111 / 531 ms | 1.5 / 1.7 |
| `nearest`, 20,000 / 100,000 | 2.0 / 9.0 ms | 15 / 54 ms | 8 / 22 ms | 4 / 2.4 |
| 200,000 × 102 written / merged | 2.67 / 2.34 s | 10.5 / 10.9 s | 5.3 / 5.6 s | 2.0 / 2.4 |
| graph: walk of 3 hops (1,022 nodes) | 222 ms | 1,443 ms | 1,372 ms | 6.2 |
| graph: a node's neighbors | 3.0 ms | 12 ms | 12 ms | 4 |
| finch's bars copied / merged | 17 / 14.5 s | 45 / 58 s | 42 / 51 s | 2.5 / 3.5 |
| finch's NBBO copied / merged | 18.5 / 15 s | 76 / 64 s | 70 / 60 s | 3.8 / 4.0 |
| finch's five questions, from the files | 9 / 4 / 17 / 73 / 72 ms | 17 / 15 / 43 / 172 / 72 | 16 / 14 / 42 / 174 / 70 | 1.0 to 3.5 |
| finch's five questions, rows held | 3 / 1 / 3 / 24 / 54 ms | 1 / 1 / 2 / 22 / 18 | 1 / 1 / 2 / 23 / 18 | 0.3 to 1.0 |

The answers agree with ClickHouse's in every run; the copies of the
month met ClickHouse dropping connections and the driver reconnecting,
which their times include.

What the measures found, each fixed with its test:

- **The sequencer flushed one put at a time.** A flush wrote and synced
  its file inside `revise`, holding the tail's lock, and a put waits on
  that lock: each put waited out the last one's fsync, then asked a
  flush of its own row (2,000 files of one row; 61 s of system time).
  The kernel held the lock for its write too, but flushed from a timer
  every 10 ms, so puts gathered between flushes. Now the flush claims
  the pending parts under the lock, writes them outside it and lets
  them go under it (`_claimed`, `_written`, `_settled`): the puts that
  arrive during a write join the next flush, with no timer. A listing
  that met a flush between its two reads begins the pending rows past
  the file that covers them (`_uncovered`), and `pending_rows` cuts at
  both ends, so no row is read twice or missed.
- **A revision that can fail copies the record first.** A put's and an
  image's revisions answered errors, so every put and every `latest`
  copied the tail with its pending parts; they answer a refusal in the
  record now (`refused`), and `latest` is the kernel's again.
- **An item of an item was copied whole.** `by_cells[k][i]` compiled to a
  copy of `by_cells[k]` for each read — the summary copied a group's
  cells once per row and column (quadratic in the group's size). The
  emitter reads the inner item where it lies (`item_place`).
- **A value a guard made known good was copied where it was lent.** A
  reader handed to `entries[r, c]` carried its whole footer into a copy
  at every call, and a search's positions were copied per row it scored.
- **A search read a whole group for each of its k rows**, reopening the
  file (its footer parsed again) each time, and opened the file again
  for every group it scored. The kernel kept its readers and a map of
  the groups it had read; the port does now.
- **A 64-bit hash ran on growing ints.** The width pass read `bitxor` of
  a side spanning [-2^63, 2^63 - 1] as 65 bits, so the bloom filter's
  FNV and murmur over every cell a merge writes were big-int
  arithmetic; and a vector column's distinct count formatted 512 bytes
  of hex per row through `format!`.
- **`nearest` indexed rows through a growing int**: `row * w` is past 64
  bits to the width pass, so every number of every row was read through
  `Int::add`. It walks the flat list a width at a time now, within the
  list's length.

What is left, and why:

- **Every read parses the file's footer again**, and every `entries`
  call its block: the kernel kept both for the process (keyed by path,
  size and change time). A graph's walk is a `lookup` per node, each
  reading the manifest, the directory and every footer: that is its
  6.2. Decided (Vijay, 2026-10-28): nothing is kept between calls —
  process memory is the standard library's and a program's never, and a
  form a derivation could call to keep a record was declined — so the
  reads are made cheap instead: the parsers narrowed to 64 bits, a hop
  of the graph one call for its whole frontier, a read parsing only the
  groups and columns it asks.
- **The encoders and decoders run on growing ints.** Positions count up
  without a bound the width pass can see, and products of counts read
  from bytes (`n * width`) pass 64 bits to it; a function has one
  signature for all its callers, so one such caller makes the whole
  chain growing — 38 of the modules' functions take a growing int
  (packing, encode, file, merge, postings). Narrowing them at their
  sources, as section 2.2 asks of a kernel, is the next step for the
  writes, the merges and the reads from the files.
- **The decompressor copies each run twice**: `slice` makes the bytes,
  then `concat` appends them (section 4.1). A fusion of `concat[out,
  slice[…]]` in the lowering would make it one copy.
- **The summary is still per cell in Crow** (a record per cell, a map
  read and written per row): 55 times the kernel at 200,000 rows, where
  a grouping that reads the columns as vectors would not be.

## 5. Open for Vijay

- A package's parts allowed `use sys` (2.7).
- The flush through `later`: a put under `crow run -i` is durable when
  it answers, and slower for it (2.4).
- `move` for a file's rename, since `rename` is the frame module's.
