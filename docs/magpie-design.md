# Magpie: the design

Written 2026-10-26, before any of it is built. Magpie is the data platform
that was built as `lake` (phases 0–6 of the first architecture, commits
c642869 → 61c20d9): one store that takes the stream, holds the table,
indexes the text, keeps the current state and orders the transactions,
in one process, written in Crow on the sys layer with kernels in Rust.
The first architecture holds at tens of millions of rows and was never
released. Vijay's decisions, 2026-10-26:

- **The name is `magpie`**: the corvid that gathers everything into one
  nest. `lake` goes entirely (no alias, no message: a retired name is an
  unknown name). The Kafka door keeps its name, `kafka`.
- **Nothing is released until the architecture holds** at a billion rows
  and a thousand columns on one machine, and until the four capabilities
  it stands in for — an analytics engine, a search index, a current-state
  store, a transactional store — are its own, at their engines' cost
  class, verified by measure.
- **All of it is Crow or our own Rust.** No third-party code in Magpie:
  its files are its own format, written straight from the frame's
  columns, with its own encodings and its own codec; the Kafka door
  decodes gzip and snappy with our own decoders. `flate2` and `snap`
  leave crow_runtime. `arrow` and `parquet` stay for the frame module
  (`frame_of_parquet`, the web's Arrow responses) and are not Magpie's:
  a program that wants a segment as Parquet exports it through a frame.
- **The compiler is untouched** (design rule 24). Anything new is a
  module, a row, a kernel, or a runtime component.

The spec's section (`docs/language-spec.md`, magpie) is updated as each
phase lands; this note records what was decided and why. Phases are in
build order; each is one gated commit series that keeps every test
before it and ends with a measure.

## What is wrong with the first architecture

Five things, none of them a verb:

1. **The merge is in memory.** `batch::merge` stacks every live file into
   one frame, sorts, deduplicates and writes the segment into one buffer.
   It ends at RAM.
2. **A file's head is one JSON of every column's statistics**, parsed
   whole by every query of any column. A thousand columns is tens of
   megabytes per file, read by a lookup that wants one.
3. **Row groups are 8,192 rows**, and the Parquet footer keeps an entry
   per column per group: the wide-table pathology.
4. **The directory is the index.** A read lists the log and reads every
   head to decide what to open.
5. **The producer chooses what is searchable** (`texts` on `batch`), and
   a column it did not name is unsearchable until the log is rewritten.

And one limit of the language's shape, kept and made explicit: a read
answers a frame, so a read over a billion rows is not one call.

## What stays

- The log of positions: a batch is the unit of ingestion, sorted by the
  log's keys, its rows at consecutive positions in `_pos`; a segment is
  the compacted form; `latest` is the newest version of each key.
- The verbs and their answers: `batch`, `commit`, `head`, `since`, `tail`,
  `scan`, `latest`, `lookup`, `search`, `sync`, `merge`, `metrics`,
  `backup`, `restore`, `share`, `follow`, `batches`. Signatures change
  where a phase says and nowhere else.
- Both modes through the same kernels (design rule 20: a kernel where the
  cost is per byte, the protocol in Crow).
- The Kafka door and the shipping, which move files and positions.
- `commit` as the transaction: optimistic, over any number of keys, its
  precondition the snapshot the writer read at.
- BM25 as Lucene scores it, held to `tests/magpie/reference.py`.
- The frame as the unit a program reads and writes: Magpie's file
  format is designed around the frame's columns (phase 2), not the
  frame around a format, and the frame verbs — `join[…, 'asof']` among
  them — run on what a read answers.

## Phase 0 — the rename

`src/modules/lake.co` → `magpie.co`; `kafka.co`'s `lake.` → `magpie.`;
the registry entry; the rows' `value "lake"` → `"magpie"`; the spec
section; the book pages (`by-example/magpie.md`, `stdlib/magpie.md`);
parity 294–297's titles and bodies; `tests/lake_share.rs` →
`tests/magpie_share.rs`; `tests/lake/` → `tests/magpie/`;
`examples/lake` → `examples/magpie`; the self-check lists; the
CHANGELOG; the article's sample and prose. One commit; `lake` appears
nowhere afterwards (`grep -rn lake` finds only the word in prose about
other things).

## Phase 1 — a manifest, and keys declared once

`dir/manifest`: the list of live segments, one row each — `path`,
`level`, `first`, `last` (its position range), `rows`, `bytes`, `keys`
(the log's key columns) and `ranges`, each key's `[lo, hi]` in the
segment as a JSON text. It is a Magpie file like any other (phase 2;
until then an Arrow IPC file), written beside and renamed over the old
one: one atomic replacement, readers see the old list or the new.
Batches are not in it: they are the tail, found by listing `*.batch`,
which is short because the merge runs.

A read prunes by the manifest before opening anything: `since` by
`first`/`last`, `lookup` by `ranges`, `latest` and `scan` by `last`
against `upto`. A reader that finds a listed file gone (a merge replaced
it between the listing and the open) reads the manifest once more and
goes again; the second miss is an error. `batches[dir]` answers the
manifest's rows and the tail's in its present shape plus `level`.
`backup` copies the manifest last; `restore` rewrites it from the files
restored; `follow` ships by position and does not change.

**The log's keys are declared once.** The first `batch` of a log fixes
its `by`; the manifest records them; a later `batch` or `commit` with
other keys is refused (`the log 'orders' is keyed by id, not by item`).
`merge[dir]`, `latest[dir]` and `commit[dir, f, …]` read the keys from
the manifest and take no `by`. **A log may declare its event-time
column the same way** (`batch[dir, f, by = ['entity'], time = 'ts']`, a
moment column, recorded in the manifest): the as-of join of phase 8,
the expiry of phase 6 and the windows by time read it, so no later call
names the time column again. Considered: keys per call, as now
(rejected: a log sorted by one key at write and merged by another is a
mess nobody wants, and `latest` by a column that is no key is a group
over `scan`, in Crow). Considered: a second declared order, so a
reverse-edges log for a graph is the merge's work (rejected for now: a
second order is a second log the program writes, two `batch` calls, and
a package over Magpie does that; a declared order would double every
merge for every log).

Gets: reads that open only the files that can hold their answer; a
merge that is one atomic switch; recovery with a definition (what the
manifest does not name is a leftover); a log that is a table with a
primary key.

Holds: the module's tests and parity 294–297 unchanged in meaning; a
parity program where a listed file goes missing under a read; a refused
second key set.

## Phase 2 — Magpie's own file format, statistics read on demand

One format for batches, segments and the manifest. A file is:

- **Row groups.** Per group, per column, a chunk: an encoding byte, a
  validity bitmap where any cell is missing, the payload. Encodings, by
  the column's kind and chosen per chunk by what is smaller: ints by
  frame of reference and bit packing, or delta where the column is
  monotone (positions, timestamps); floats raw; text by a sorted
  dictionary with bit-packed codes where the distinct values are at most
  half the rows, else offsets and bytes; bools as a bitmap; moments as
  their ints. A chunk is then passed through Magpie's codec — an LZ
  byte codec of our own, hash-chain matches over a 64 KB window,
  literals and matches — and kept coded where that shrinks it by a
  tenth or more.
- **The index section.** Per column per group: kind, rows, missing, min,
  max, distinct (capped at 65,536, the bloom given up past it), the
  bloom filter. Per text column per group, from phase 4: the token
  filter, the document count, the word count. From phase 7: the postings
  and the term dictionary. From phase 10: the vector index.
- **The footer.** A table of sections and, within the index section, an
  offset table by column then group, so a reader seeks to one column's
  one entry; the file's own facts (format, rows, first, last, keys); the
  footer's offset and the magic as the last bytes.

A reader reads the last bytes, the footer, then only what it asks for. A
batch is written whole to a temporary name and linked into place, as
now; a segment is written by the merge group by group (phase 3). Row
groups are sized by bytes, `GROUP_BYTES` (4 MB: a lookup decodes one
group, so a group is small, and the index is read per column, so many
groups cost a reader of two columns nothing; settled by the phase 5
measure); the 8,192-row constant goes. Built 2026-10-26, before phase 1,
so the manifest is a Magpie file from its first version.

**The frame is the unit on both sides.** A write takes the frame's
columns as they lie — an int or float column is its vector, bit-packed
or raw; a text column's dictionary is the dictionary the frame keeps;
the validity bitmap as it is — and a read decodes into the same layout,
so the frame verbs run on what was read with no conversion, and a read
of two columns of a thousand decodes two. The frame's kinds (int, float,
text, bool, moment) are the file's kinds; a kind the frame gains (the
vector column, below) gets an encoding here. A float column may be
stored as 32 bits where the program says so at the first batch
(`narrow = ['x', 'y']`, recorded in the manifest), since that is lossy
and a model's memory is the only reason; the frame keeps 64 bits in
memory.

The Kafka door's gzip and snappy decoding is our own (an inflate by
RFC 1951 with the gzip wrapper of RFC 1952, and a snappy decoder);
the door answers fetches uncompressed, which the protocol allows.

Format 1 (the first architecture's Arrow and Parquet files) is not read:
nothing was released, and the month of finch's bars is regenerated from
ClickHouse by its script.

Gets: a query's cost independent of the table's width (a lookup on one
column of a thousand reads one entry per file, then one group's pages);
no conversion through Arrow on either side of a write or a read; no
sidecars; files whose every part is found through one footer.

Holds: round trips of every column kind and every encoding with missing
cells; the codec against its own decoder on random and on real bytes; a
segment's size within 1.2× of the Parquet-with-snappy segment of the
same month (336 MB) and its decode no slower, measured; the door's
decoders against gzip's and snappy's reference vectors.

Measured 2026-10-26: the bars' segment 287 MB against Parquet's 336, the
quotes' 248 against 325; the five questions from the files, interpreted,
8 → 6, 7 → 4, 27 → 18, 220 → 73 and 80 → 76 ms against the first
architecture. The first run found the reader stacking each group onto
the growing frame (16 s for the month's scan): `CrowFrame::stack_all`
builds each column once.

## Phase 3 — a streaming merge, bounded segments, levels

`merge[dir]` compacts in two moves, each a k-way merge over inputs that
are already sorted by (keys, `_pos`), holding one row group per input
and one for the output:

1. **The tail into a segment.** Every batch, in key order, into segments
   of at most `SEGMENT_BYTES` (1 GB), the newest version of each key kept
   (a key's versions are adjacent in the merged order, the last of a run
   the newest) unless `every`.
2. **A level into the next.** Where a level holds more than `FANOUT` (8)
   segments, its oldest eight neighbours in position order merge into one
   segment of the next level, the same way.

A segment is named by a counter (`S000123.segment`); its position range
lives in the manifest, since a merge that splits by size makes two
segments over one range. Within a segment the order is the keys', so a
lookup by the first key reads one group (the 41 → 9 ms finding of the
first architecture stands); `since` over a merged range reads the
segments whose range covers it, which is a scan of that range as in
every store — a consumer that keeps up reads the tail. Across segments
the newest position wins at read, which `lookup` (every version, in
position order) and `latest` (the max of `_pos` per key) already do.
Considered: segments in position order with blooms alone for keys
(rejected: gives up the one-group lookup). Considered: key-ordered
levels with disjoint ranges, as LevelDB (rejected: a log compacts by
time, and disjoint key ranges make `since` read every segment).

The old files are deleted after the manifest names the new; a crash
between leaves both, and the next merge removes what the manifest does
not name. Memory is `(inputs + 1) × GROUP_BYTES`.

Gets: a merge in constant memory, so the log's size stops being bounded
by RAM; compaction cost proportional to what arrived; a billion rows as
a few dozen segments over three levels.

Holds: a merge of a log larger than the process's memory budget (the
self-check's 1 GB) in both modes; the crash harness (`tests/magpie_crash.rs`:
an environment switch ends the merge after each step — segment written,
manifest renamed, old files half deleted — and the next read and merge
answer whole).

## Phase 4 — indexes built by the merge, never by the producer

`texts` leaves `batch` and `commit`. The merge tokenises every text
column of every segment it writes and keeps, per group, the token filter
and the two counts in the index section; a column whose distinct tokens
exceed half its documents (an identifier column) gets none and is
scanned. `search` on a segment reads the index and scores from the
groups whose filters admit a word; on a batch it reads the column and
tokenises it, the counts from what it read. The answer is the same
either way: the index changes what is read, not what is scored.

Gets: ingestion knows nothing about queries; any text column is
searchable the moment it lands and cheap once merged; the price paid by
the background job.

Holds: one parity program searches a log before and after its merge and
asserts one ranking; `tests/magpie/reference.py` unchanged.

## Phase 5 — reads bounded by what a frame holds, and the measure

- `since[dir, from, upto = -1, columns = []]`: a position window.
- `scan[dir, from = 0, upto = -1, columns = []]`: the same.
- `latest[dir, upto = -1, columns = [], lo = '', hi = '']`: the current
  state over a range of the log's first key, each bound a value as text
  as `lookup` takes it; empty bounds mean every key. Groups are in key
  order, so the range reads the groups it covers.
- `lookup` and `search` are bounded by their keys and `k`.

An aggregate over the whole log is a loop over windows in Crow until
phase 8. Considered: a read that answers a stream (rejected: the
language has one shape for rows, a frame; a stream of frames is the loop
the program writes).

**The measure that ends the storage plan**: the present tests and the
month of finch's bars unchanged in answer and no slower, then a generated
log of a hundred million rows by a thousand columns (not kept): lookup
and search flat in the width, merge linear in the rows, memory flat.
`GROUP_BYTES` and `SEGMENT_BYTES` are settled here.

## Phase 6 — the sequencer: a memtable and its write-ahead log

A runtime component of crow_runtime, process-global per directory as
the process registry is, that both modes share. It holds:

- **The tail buffer**: rows written since the last flush, each with its
  position assigned the moment it is written.
- **The key index**: for every key, the position of its last write.
- **The image**: the latest row per key, built from `latest` the first
  time it is asked for and kept current by every write after.

Verbs: `put[dir, f]` appends rows to the buffer and answers their first
position (visible at once to `since`, `tail`, `latest`, `lookup` and `get`
in this process; flushed as one batch file, fsynced, within 10 ms by the
sequencer's thread, or at `sync`); `get[dir, equal]` answers the newest
row for a key from the image, a frame of one row or none; `commit`
validates its keys against the key index in memory and appends under the
sequencer's lock, so two commits over one key serialise with no file
read; `batch[dir, f, flush = true]` is `put` and a flush, durable before
it returns, its contract unchanged. A log may declare an expiry column
at its first batch (`expires = 'until'`, a moment column, recorded in
the manifest): `get`, `latest` and `lookup` skip a row whose moment has
passed, and the merge drops it.

In a served app the handlers call `put`, `get` and `commit`, and the
runtime's state is the process's, so every handler and every thread
sees one tail; a `use web` quick handler answers `get` on the server's
own thread. Another process sees a write after its flush; `follow` ships
the files as before.

Gets: Redis's model — a single thread ordering writes, applying them to
memory and appending them to a log — so single-key writes, counters and
reads run at memory speed, with expiry; Postgres's group commit, so
small transactions over several keys run at thousands to tens of
thousands a second with concurrent writers serialised by construction;
the log files unchanged, now the write-ahead log.

Holds: a parity program where threads of one served app write and read
one log; a durability test that ends the process between `put` and the
flush and reads back what `sync` had made durable; `commit` conflicts
decided in memory agreeing with the file-read decision of phase 1.

## Phase 7 — search's long tail

In the segment's index section, per text column, built by the merge:

- **Postings**: for each term, the rows that hold it (group and row),
  the count, the positions; deltas bit-packed in blocks of 128.
- **A sorted term dictionary** with each term's postings offset: a
  prefix query is a run of it; a fuzzy query walks it with an
  edit-distance automaton (Levenshtein, distance 1 or 2).
- **Block-max WAND**: each block carries the best score it can reach, so
  a top-k over a common word skips the blocks that cannot beat the k-th.
- **Snippets** around the hits (`snippet = 0` characters by default; a
  `snippet` column when asked), **stemming** by a Porter stemmer for
  English (`stem = false` by default, since Lucene's standard analyser
  does not stem; on at both index and query when asked, and the log's
  choice recorded in the manifest), **facets** as a `group` over the hits
  in Crow.

Query forms, Lucene's own, inside the words: a word with a space is a
phrase (`'kitchen table'`, the tokens in order from the positions), a
word ending in `*` a prefix, a word ending in `~` fuzzy. One analyser:
runs of letters and digits, lowercased, stemmed when the log says so.
Batches stay scanned.

Gets: Elasticsearch's queries on one node — term, phrase, prefix, fuzzy,
stemmed — with a common word's top-k in milliseconds over a billion rows,
scored as Lucene scores.

Holds: every query form against the scan's ranking over the same log
(the index changes what is read, never the score); the reference script
extended to phrase and prefix; a measure of a common word's top-10 over
the hundred-million log with and without WAND.

## Phase 8 — aggregates pushed below the reader

`summary[dir, by, of, how, on = [], ops = [], equal = [], lo = '', hi = '',
upto = -1]`: one kernel over the segments and the tail. Per row group,
on its own core: prune by the group's statistics (ranges, blooms for
equality), decode only the columns named, filter by the predicates (`on`,
`ops`, `equal` as three lists: a column, an operator — `eq`, `ne`, `lt`,
`le`, `gt`, `ge` — and a value as text; the same column repeated is any
of its values, columns are all of them), group by `by` with hashing,
reduce `of` by `how` (`count`, `sum`, `min`, `max`, `mean`, `var`, `std`,
`first`, `last` by position) into partials; then the partials combined
(sum and count for the mean, Welford's merge for the variance, as the
frame kernels held to pandas do). `metrics` reports bytes per column.

**Built as operators, not as one kernel.** `summary` is a pipeline of
separate operators over row groups — scan (prune by statistics, decode
the columns named), filter (predicates evaluated over column vectors),
project, group, reduce, combine — each a function of its own over the
frame's columns, and the verb a fixed composition of them. This costs
nothing in phase 8 and keeps any later door a translator into existing
operators rather than an engine of its own; the frame verbs (join, sort,
window, pivot) are the in-memory operators beside them.

**The as-of join over the log**, `asof[dir, f, columns = []]`: for each
row of `f` — the log's keys and a moment in the log's time column — the
newest row of the log with the same keys at or before that moment,
read from the key's run in the key-sorted segments and from the tail,
answered beside `f`'s row (a missing row where none precedes). It is
the frame module's `join[…, 'asof']` with the log as the right side,
and the same answer: a training set of millions of labels against a
billion-row history without materialising the history.

Gets: ClickHouse's behaviour on filter, group, aggregate and sort, the
shape most analytics takes: rows never materialised, cost proportional
to the bytes of the columns touched, every core busy; the point-in-time
join a feature store is built on, at the log's scale.

Holds: `summary` against the frame verbs over `scan` of the same rows
(held to pandas already), every operator and every `how`; `asof` against
`join[f, scan[dir], keys, 'asof']` over the same rows; a measure of
finch's five questions and the hundred-million log against ClickHouse on
the same machine, in the article's form.

## Phase 9 — doors

A log is a directory path or a URL. `magpie.serve[dir, port]` is a
`use web` app whose routes are the verbs over JSON, and the verbs given
a URL (`batch['http://host:9000/orders', f]`) reach it, so a program in
another process or another language reaches a Magpie by HTTP. A RESP
door (`redis.co`, the Kafka door's pattern): GET, SET, DEL, INCR,
EXPIRE, TTL, EXISTS, MGET, MSET over `get`/`put`, XADD and XREAD over
`put`/`tail`, so Redis clients speak to a log. The Kafka door stays.

Gets: the four products' clients reaching one store, for the questions
the doors carry.

Holds: the door tests as the Kafka door has them (a client script per
door, both modes).

## Before phase 10 — a column of vectors (a language feature, Vijay's call)

A frame holds ints, floats, text, bools and moments per cell, and
nothing else: an embedding per row has no place. The fix is a column of
fixed-width vectors — `Column::Vectors(width, Vec<f64>)` in the runtime,
stored flat, `f.embedding[i]` a tensor, a loop over it yielding tensors,
Magpie encoding it as a float chunk of `rows × width`. A new cell kind
threads through the type rules (`src/totality.rs` names the places), so
it is a language feature under the freeze (design rule 24), not a part
of a phase: asked for here, built when Vijay says so, before phase 10.
Until then an embedding is one float column per dimension, which
Magpie's column projection makes workable and no one would call good.

## Phase 10 — a graph and a vector index

A knowledge graph is two logs and a package: nodes keyed by `id`, edges
keyed by `src`, `rel`, `dst` with provenance columns, a reverse edges log
written beside. A merged segment sorted by `src` is a node's out-edges
as one run (compressed sparse rows on disk, built by the merge); the
image is the adjacency in memory; `upto` is the graph as of a position.
The package (in `packages/`, named later): `node`, `link`, `unlink` (a
tombstone), `neighbors`, `walk` (breadth-first, a kernel for the
frontier where a hop over thousands of nodes should cost per byte),
`paths`, `match` (joins over the edges frame), `subgraph` answering a
frame `packages/graph` takes for PageRank and components, `asof`. Rules
are Crow functions that derive edges and write them back. Extraction is
the `llm` module writing triples.

The vector index: a tensor column's index per segment built by the
merge (an HNSW of our own; the tail scanned exactly), `nearest[dir,
column, vector, k]`, held to the vectors package's exact search for
recall. It is what finds an entity by meaning before the edges expand
it.

Gets: a property graph and a triple store with history, text and vector
search over entities, traversal at disk or memory speed, analytics over
subgraphs in memory.

## Phase 11 — a feature store

A feature store today is three systems and a job: an offline store of
history for training, an online store of latest values for serving, a
materialisation job copying one into the other, and a registry. On
Magpie it is a package in Crow (in `packages/`, named later) over what
the phases built, and the job is gone:

- **A feature group is a log** keyed by the entity, with its event-time
  column declared (phase 1). One write is both stores: the history is
  the log, the latest per entity is the image (phase 6), and `get`
  serves it in microseconds.
- **A feature's definition is a Crow function** from source rows to
  feature rows. The same function runs in the backfill over `scan`
  windows and in the streaming path over `tail` from the Kafka door, in
  both modes: training-serving skew is gone by construction. The
  registry is the functions' signatures and comments, which `describe`
  answers.
- **A training set is a position.** The labels frame as-of joined
  against the history (phase 8's `asof`), `upto` naming the dataset, so
  it is reproducible exactly; lineage is the positions and the
  provenance columns each feature row carries.
- **Embeddings are features** through the vector column.

The package: `group[dir, entity, time]` (declares the log), `write`
(`put`), `history` (`scan` windows), `training[labels, groups]` (`asof`
per group, joined), `serve[group, entity]` (`get`), `backfill[group, f,
from, upto]` (the function over windows, written back), `freshness`
(`head`, `metrics`), expiry through phase 6's column.

Gets: an offline and an online feature store that are one log, with
point-in-time correctness and reproducible training sets, and one
definition of each feature for every path.

Holds: `training` against pandas's `merge_asof` over the same labels
and history; a backfill and a streaming run of one feature function
answering the same rows; a served app reading `serve` while a writer
runs.

## What Magpie is not doing

- **A query language.** No SQL, Cypher or SPARQL: a question is a verb,
  a pattern is joins, a scan is windows. Arbitrary joins of two large
  logs, subqueries and window functions in one plan stay loops in Crow;
  a planner and a vectorised executor of our own is the boundary, not a
  phase. (A SQL subset as a `db` driver over phase 8's operators is
  possible later, in weeks, and is set aside for now.)
- **MVCC.** No isolation levels, no long transactions holding reads, no
  transactions across logs. Latest-wins by key, optimistic commits, one
  sequencer per process.
- **In-place updates.** A new version is a row; the merge keeps the
  newest.
- **A second machine.** No shards, no election, no cross-shard
  transactions; `follow` gives read replicas, a shard is a log per key
  range the program routes to.
- **The products' full surfaces.** Not Redis's command set, Elasticsearch's
  query DSL or aggregations framework, ClickHouse's codecs and skipping
  indexes; English stemming only until another language is asked for.
- **Other tools reading Magpie's files.** Export through the frame module
  (CSV, Parquet) instead.
- **A migration** from the first architecture's files, which were never
  released.
- **Any change to the compiler.**

## What holds the whole

- Every present test, renamed: module tests, parity 294–297,
  `tests/magpie_share.rs`, `tests/kafka.rs`, `examples/magpie`, finch's
  queries from the log.
- A parity program per phase as listed; the crash harness; the
  durability test; the door clients.
- The references: `tests/magpie/reference.py` for BM25 and its query
  forms; the frame tests' pandas answers for `summary`; ClickHouse's
  answers for finch's questions; the vectors package for recall.
- The measures, in the article's form, on the same hardware: the month
  of bars, the hundred million by a thousand, finch's five questions
  against ClickHouse, a common word's top-10, writes and reads through
  the sequencer against Redis and Postgres on loopback.
- The gate as CLAUDE.md lists it, after every phase.

## Size

Phases 0–5 are about one and a half times the first architecture
(the format is the half). Phase 6 is medium. Phase 7 is the largest
single phase. Phase 8 is a kernel round the size of the frame verbs'.
Phase 9 is small. Phase 10 is a package plus one index. All of it is
ours.

## Open for Vijay

1. `put`'s flush every 10 ms with an fsync each time (Postgres's default
   durability at Redis's write path), or an fsync only at `sync` and at
   `batch` (Redis's everysec): my recommendation is the fsync each
   flush, a hundred a second is nothing.
2. Stemming off by default (Lucene's default), as written.
3. The verbs taking a URL as a log (phase 9), as written.
4. The graph package's name, and the feature store package's.
5. Measure after phase 5 before phases 6–11, as written.
6. The column of vectors: a language feature, built before phase 10 when
   Vijay says so; the feature store and the vector index wait on it,
   nothing before them does.
7. `narrow` (32-bit floats on disk, opt-in at the first batch), as
   written in phase 2.
