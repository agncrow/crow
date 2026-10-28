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
- `flush[path]`: the file, or the directory, written through to the
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
effect kinds carry the rules: `memory` for a read, `memory.write` for
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

## 5. Open for Vijay

- A package's parts allowed `use sys` (2.7).
- The flush through `later`: a put under `crow run -i` is durable when
  it answers, and slower for it (2.4).
- `move` for a file's rename, since `rename` is the frame module's.
