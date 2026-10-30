# magpie

The log as a directory of batch files, written in Crow. Each commit is
one file, written once: its rows take the log's next positions, sorted
by the log's keys, with the statistics a reader prunes by. Over the
files are the reads a program asks of a log — the rows since a position,
the newest version of each key, a lookup, a window scanned, a search
ranked as Lucene ranks it, the nearest vectors, an aggregate pushed
below the reader and an as-of join. A sequencer orders every write of a
process and refuses a stale commit; a merge folds the files into sorted
segments; a second node follows the log by its files. Two doors let
other products' clients in: Kafka's protocol and Redis's. Every answer
is the same in `crow run -i` and `crow run`.

```
deps = map[magpie = 'github.com/agncrow/crow/packages/magpie@main']
```

`crow install` checks the package out into `crow_modules/magpie`, and
`crow.lock` pins the commit it took and the hash of the files it
installed. A program imports it and calls it qualified; the doors are
the package's parts:

```
use magpie, frame
use magpie.kafka
```

Magpie stands on the standard library's own layer — `use sys` for bytes,
files and sockets, `use memory` for the sequencer's state, which the
process holds — as any package that needs it may. Its file format is its
own ([docs/magpie-design.md](../../docs/magpie-design.md)). [REFERENCE.md](REFERENCE.md) has every
function and part, and the design notes that decided them;
[PARTS.md](PARTS.md) shows each part at work alone; `example.co` runs
the tour below and every part's block in both modes.

## A tour

### Batches, reads, a commit and a merge

```crow
use magpie
use frame

gone = rm['ex_magpie']
made = mkdir['ex_magpie']
first = magpie.batch['ex_magpie', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
log[first]
second = magpie.batch['ex_magpie', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]
log[second]
log[magpie.head['ex_magpie']]
log[magpie.latest['ex_magpie'].qty]
log[magpie.lookup['ex_magpie', ['id'], ['2']].qty]
stale = magpie.commit['ex_magpie', frame.new[id = [2], qty = [26]], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]
log[magpie.since['ex_magpie', 3].qty]
log[magpie.scan['ex_magpie', 0, 1].id]
log[len[magpie.batches['ex_magpie']]]
log[magpie.merge['ex_magpie']]
log[magpie.batches['ex_magpie'].form]
log[magpie.latest['ex_magpie'].qty]
log[magpie.batch['ex_magpie', frame.new[id = [4], qty = [40]], by = ['qty']].reason]
log[magpie.batch['ex_magpie', frame.new[id = [4], qty = [40]]]]
gone = rm['ex_magpie']
```

```text
0
2
3
[10, 25, 30]
[20, 25]
conflict: id = 2 was written at position 2, after snapshot 1
[30]
[1, 2]
2
ex_magpie/S000000000001.segment
['segment']
[10, 25, 30]
the log at 'ex_magpie' is keyed by id, not by qty
4
```

Each `batch` is one file in the directory, written once: the rows take
the next positions, sorted by the log's keys, with the statistics a
reader prunes by in the file's index. The first batch fixes the keys
(`by`), and the manifest beside the files records them: a later batch
names the same keys or none, and another set is refused naming both.
`latest` is the table the log holds, one row per key; `lookup` reads
only the files whose filters admit the value; `commit` is the sequencer,
refusing a write whose reads are stale; `merge` folds the files into one
segment holding the latest version of each key, then switches the
manifest to name it, which the same readers read.

### Windows: a read is bounded, the log is walked

```crow
use magpie
use frame
use math

gone = rm['ex_windows']
made = mkdir['ex_windows']
a = magpie.batch['ex_windows', frame.new[day = ['d1', 'd1', 'd2', 'd2'], sym = ['A', 'B', 'A', 'B'], px = [1, 2, 3, 4]], by = ['day', 'sym']]
b = magpie.batch['ex_windows', frame.new[day = ['d2', 'd3'], sym = ['A', 'A'], px = [5, 6]]]
log[magpie.since['ex_windows', 2, 4].px]
log[magpie.scan['ex_windows', 1, 2, ['px']].px]
log[magpie.latest['ex_windows', lo = 'd2', hi = 'd2'].px]
log[magpie.latest['ex_windows', hi = 'd1'].sym]
total = 0
at = 0
last = magpie.head['ex_windows']
while at lte last
    piece = magpie.since['ex_windows', at, at + 1, ['px']]
    total = total + int[sum[piece.px]]
    at = at + 2
log[total]
gone = rm['ex_windows']
```

```text
[3, 4, 5]
[2, 3]
[4, 5]
['A', 'B']
21
```

A read answers a frame, so every read is bounded: `since` and `scan`
take a position window, `latest` a range of the log's first key, read
through the groups the range covers. A read over the whole log is a
loop over windows, as the sum above walks the positions two at a time;
an aggregate over a billion rows is that loop, each window reading its
segments and its columns and nothing else.

### The sequencer: put, a key's newest row, and a commit decided in memory

```crow
use magpie
use frame
use math

gone = rm['ex_tail']
made = mkdir['ex_tail']
first = magpie.put['ex_tail', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
log[first]
log[int[sum[magpie.batches['ex_tail'].rows]]]
log[magpie.since['ex_tail', 0].qty]
second = magpie.put['ex_tail', frame.new[id = [2, 3], qty = [25, 30]]]
log[magpie.latest['ex_tail', equal = ['2']].qty]
log[len[magpie.latest['ex_tail', equal = ['9']]]]
log[magpie.sync['ex_tail'] gte 1]
log[int[sum[magpie.batches['ex_tail'].rows]]]
stale = magpie.commit['ex_tail', frame.new[id = [2], qty = [26]], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]
fine = magpie.commit['ex_tail', frame.new[id = [2], qty = [26]], seen = 3, columns = ['id'], equal = ['2']]
log[fine]
log[magpie.latest['ex_tail', equal = ['2']].qty]
gone = rm['ex_tail']
```

```text
0
2
[10, 20]
[25]
0
true
4
conflict: id = 2 was written at position 2, after snapshot 1
4
[26]
```

Every write of a process goes through the log's sequencer: `put` gives
the rows their positions as they arrive and the process reads them at
once, as the catalog's `pending` row; the flush the put asks for writes
them as a batch file (`later`: at once under `crow run -i`, on a thread
compiled), or `sync` does now, and `batch` is a put
and a flush, durable before it answers. `latest` of one key (`equal`) reads the newest row of a
key from the image the sequencer keeps in memory, and a `commit` whose
reads are the log's keys is decided against the sequencer's index in
memory.

### A batch without the flush, and a sync

```crow
use magpie
use frame

gone = rm['ex_flush']
made = mkdir['ex_flush']
a = magpie.batch['ex_flush', frame.new[id = [1, 2], qty = [10, 20]], by = ['id'], flush = false]
b = magpie.batch['ex_flush', frame.new[id = [3], qty = [30]], by = ['id'], flush = false]
log[magpie.latest['ex_flush'].qty]
log[magpie.sync['ex_flush'] gt 0]
gone = rm['ex_flush']
```

```text
[10, 20, 30]
true
```

With `flush = false` a batch is written but not flushed to the disk, so
a crash may lose the batches since the last `sync`: Redis's default
durability, for a log that chooses it; `sync` flushes every file of the
log and its directory, and a program calls it on a timer.

### A point read by two keys, and only the columns asked

```crow
use magpie
use frame

gone = rm['ex_keys']
made = mkdir['ex_keys']
a = magpie.batch['ex_keys', frame.new[day = [1, 1, 2, 2], sym = ['A', 'B', 'A', 'B'], px = [10, 11, 12, 13], qty = [1, 2, 3, 4]], by = ['day', 'sym']]
log[magpie.lookup['ex_keys', ['day', 'sym'], ['2', 'A']].px]
log[magpie.lookup['ex_keys', ['day'], ['1'], ['sym', 'px']].columns]
log[magpie.scan['ex_keys', 0, -1, ['qty']].qty]
log[magpie.since['ex_keys', 2, -1, ['sym', '_pos']].sym]
gone = rm['ex_keys']
```

```text
[12]
['sym', 'px']
[1, 2, 3, 4]
['A', 'B']
```

`lookup` takes every key at once, so a segment sorted by day then
symbol reads one row group for one symbol's day instead of the symbol's
whole month; `columns` on `lookup`, `scan`, `since` and `latest` names
the columns to read, and a segment decodes only those, which is what a
month's scan pays for two columns and not ten. A lookup by a column the
log is not sorted by scans the files its filters admit: where that
recurs, keep a second log of the same rows sorted by that column, fed
from the first with `tail` (a sorted projection: one more write per row,
and the point read reads one group again).

### Search over the log

```crow
use magpie
use frame

gone = rm['ex_search']
made = mkdir['ex_search']
a = magpie.batch['ex_search', frame.new[id = [1, 2, 3], text = ['the quick brown fox', 'a lazy dog', 'the fox and the dog']]]
b = magpie.batch['ex_search', frame.new[id = [4], text = ['brown bread']]]
hits = magpie.search['ex_search', 'text', ['brown', 'dog'], 3]
log[hits.id]
log[fmt[hits.score[0], decimals = 3]]
log[len[magpie.search['ex_search', 'text', ['zebra']]]]
gone = rm['ex_search']
```

```text
[4, 2, 1]
0.382
0
```

A `texts` column is indexed as it is written: each file's head holds a
filter of its words, and `search` reads only the files whose filters
admit a word asked, scoring the rows as Lucene scores them (BM25) over
the whole log as one corpus.

### Search's forms: a phrase, a prefix, a fuzzy word, a snippet

```crow
use magpie
use frame

gone = rm['ex_forms']
made = mkdir['ex_forms']
a = magpie.batch['ex_forms', frame.new[id = [1, 2, 3, 4, 5], text = ['the kitchen table is set', 'a table in the garden', 'kitchens of the world', 'running the kitchen', 'tables and chairs']]]
log[magpie.search['ex_forms', 'text', ['kitchen table']].id]
log[magpie.search['ex_forms', 'text', ['kit*']].id]
log[magpie.search['ex_forms', 'text', ['kitchn~']].id]
log[magpie.search['ex_forms', 'text', ['table'], 2, snippet = 6].snippet]
m = magpie.merge['ex_forms']
log[magpie.search['ex_forms', 'text', ['kitchen table']].id]
log[magpie.search['ex_forms', 'text', ['tabel~', 'kitchen']].id]
s = magpie.batch['ex_stems', frame.new[id = [1, 2, 3], text = ['the tables', 'a table', 'no chairs']], stem = true]
log[magpie.search['ex_stems', 'text', ['tabling']].id]
gone = rm['ex_stems']
gone = rm['ex_forms']
```

```text
[1]
[1, 3, 4]
[1, 3, 4]
['…tchen table is se…', 'a table in th…']
[1]
[1, 2, 5, 4]
[1, 2]
```

A word with a space is a phrase (the tokens in order), `kit*` a prefix,
`kitchn~` a fuzzy word within two edits (`~1` within one), and
`snippet` asks for that many characters each side of the first hit. A
log declared with `stem = true` stems its words with the Porter
stemmer at the index and at the query, so `tabling` finds `tables` and
`table`. A batch is read whole and a merged segment through the
postings the merge built, and the ranking is the same.

### Vectors in the log: the nearest rows

```crow
use magpie
use frame

gone = rm['ex_nearest']
made = mkdir['ex_nearest']
a = magpie.batch['ex_nearest', frame.new[id = [1, 2, 3], e = [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]], by = ['id']]
hits = magpie.nearest['ex_nearest', 'e', [1.0, 0.1], 2]
log[hits.id]
log[fmt[hits.score[0], decimals = 3]]
log[magpie.latest['ex_nearest', equal = ['3']].e[0]]
gone = rm['ex_nearest']
```

```text
[1, 3]
0.995
[1, 1]
```

A column of vectors (an embedding per row) is held as any column —
written as a float chunk of rows × width, `narrow` keeping 32 bits,
carried through `latest`, `since` and the merge — and `nearest` finds
the `k` rows nearest a query vector by the cosine, the best first,
exact over every row of the log. An entity is found by meaning first,
and the edges expand it (packages/kg's `embed` and `similar`).

### Aggregates below the reader, and the as-of join

```crow
use magpie
use frame

gone = rm['ex_summary']
made = mkdir['ex_summary']
a = magpie.batch['ex_summary', frame.new[day = [1, 1, 2, 2], sym = ['A', 'B', 'A', 'B'], px = [10.0, 20.0, 12.0, 22.0], vol = [1, 2, 3, 4]], by = ['day', 'sym']]
sums = magpie.summary['ex_summary', ['sym'], ['px', 'vol'], ['mean', 'sum']]
log[sums.columns]
log[sums.px_mean]
log[sums.vol_sum]
log[magpie.summary['ex_summary', ['day'], ['vol'], ['count'], on = ['sym'], ops = ['eq'], equal = ['A']].vol_count]
log[magpie.summary['ex_summary', [], ['px'], ['max'], lo = '2'].px_max]
gone = rm['ex_summary']
gone = rm['ex_asof']
made = mkdir['ex_asof']
b = magpie.batch['ex_asof', frame.new[sym = ['A', 'A', 'B'], t = [10, 20, 15], px = [1.0, 2.0, 5.0]], by = ['sym'], time = 't']
labels = frame.new[sym = ['A', 'B', 'A'], t = [25, 12, 5]]
j = magpie.prior['ex_asof', labels]
log[j.columns]
log[j.px[0]]
log[j.px[1].ok]
log[magpie.prior['ex_asof', labels, [], 0].px[0]]
gone = rm['ex_asof']
```

```text
['sym', 'px_mean', 'vol_sum']
[11, 21]
[4, 6]
[1, 1]
[22]
['sym', 't', 'px']
2
false
1
```

`summary` is the aggregate pushed below the reader: each row group is
pruned by its statistics, decoded for the columns named alone, filtered
by the predicates, grouped and reduced on its own core, and the partials
are combined, so the rows are never held as one frame. It answers what
`group` answers over `scan` of the same rows. `prior` is the as-of join
over the log: for each label's keys and moment, the newest row at or
before it, from the key's run; with `upto`, as of that position, so a
training set a position names comes out the same at every run.

### A view that follows the log

```crow
use magpie
use frame
use math

gone = rm['ex_tail']
made = mkdir['ex_tail']
first = magpie.batch['ex_tail', frame.new[sym = ['A', 'B'], qty = [10, 20]]]
seen = -1
total = 0
rows = magpie.tail['ex_tail', seen, 0.2]
total = total + sum[rows.qty]
seen = max[rows._pos]
second = magpie.batch['ex_tail', frame.new[sym = ['A'], qty = [5]]]
rows = magpie.tail['ex_tail', seen, 0.2]
total = total + sum[rows.qty]
seen = max[rows._pos]
log[total]
log[seen]
none = magpie.tail['ex_tail', seen, 0.05]
log[len[none]]
gone = rm['ex_tail']
```

```text
35
2
0
```

`tail` answers the rows past a position, waiting up to the seconds for
some to arrive: a view is a loop over it, each round a batch of new rows
folded into the state the view keeps, the position of the last row read
kept with it, and its results written back to a log with `batch` for
other views and other nodes to read.

### Operations: the figures, a backup, a restore

```crow
use magpie
use frame

gone = rm['ex_ops']
gone = rm['ex_ops_copy']
made = mkdir['ex_ops']
a = magpie.batch['ex_ops', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
b = magpie.batch['ex_ops', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]
figures = magpie.metrics['ex_ops']
log[figures.files]
log[figures.rows]
log[figures.head]
log[figures.bytes[0] gt 0]
log[magpie.backup['ex_ops', 'ex_ops_copy']]
gone = rm['ex_ops']
log[magpie.restore['ex_ops_copy', 'ex_ops']]
log[magpie.latest['ex_ops'].qty]
gone = rm['ex_ops']
gone = rm['ex_ops_copy']
```

```text
[2]
[4]
[3]
true
3
3
[10, 25, 30]
```

`metrics` is the log's figures as a frame of one row, a table a program
keeps by writing it to a log of its own. `backup` copies the live files
and the manifest under a directory or an `s3://` prefix and writes a
`listing` last, so a backup is whole only when its listing is; `restore` copies them back,
and the batches written since are the rest of the log.

### The HTTP door: the verbs from another process

```crow
use magpie
use frame

gone = rm['ex_door']
made = mkdir['ex_door']
a = magpie.batch['ex_door/orders', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
served = magpie.door['ex_door', 0, rounds = 3]
log[served]
gone = rm['ex_door']
```

```text
0
```

`door[root, port]` serves every log under the root over HTTP: a
request's path names the log and the verb (`/orders/latest`,
`/orders/lookup`, `/orders/search`, `/orders/summary`, …), the query its
arguments, the body the rows of a `/orders/put` or `/orders/batch` as
JSON records, and the answer a frame as JSON records. A program in
another process reaches a log through the same verbs given its URL —
`latest['http://host:9000/orders']`, `batch['http://host:9000/orders',
rows]` — and so does any HTTP client
(`curl 'http://host:9000/orders/lookup?on=id&equal=2'`). The block above runs
the door's loop three rounds with nobody connecting; a client's session
in both modes is tests/magpie_door.rs.

### A second node

A second node follows the log: it holds the same files and reads them
with the same verbs. A table too large for one node is split into logs
by a key, one on each node, and a query runs the same verbs on each and
stacks the answers.

```crow
use magpie

--- the leader: its log served to followers, forever
served = magpie.share['orders', 3700]
```

```crow
use magpie

--- a follower, on another machine: the files it lacks, fetched; then
--- the log read by position as any log is, and the pass run again
seen = -1
came = magpie.follow['orders', 'leader.local', 3700]
rows = magpie.since['orders', seen + 1]
```

### The Kafka door: `use magpie.kafka`

```crow
use magpie.kafka
use frame

gone = rm['ex_kafka']
served = kafka.serve['ex_kafka', 0, rounds = 3]
log[served]
log[exists['ex_kafka']]
gone = rm['ex_kafka']
```

```text
0
true
```

`kafka.serve[root, port]` is the door: a Kafka client that connects to the port
is spoken to in the protocol a 0.10.1 broker speaks, its records landing
in the magpie's logs under the root, one log per topic. The block above
runs the door's loop three rounds with nobody connecting; a producer and a
consumer group driven against a running door are tests/kafka.rs's, with
the pure-Python client.

### The RESP door: `use magpie.redis`

```crow
use magpie.redis

gone = rm['ex_redis']
made = mkdir['ex_redis']
served = redis.serve['ex_redis', 0, rounds = 3]
log[served]
gone = rm['ex_redis']
```

```text
0
```

`redis.serve[root, port]` is the door: a Redis client that connects to the
port speaks RESP2 as redis-cli does, and the door answers from Magpie —
`SET` a put into a log under the root keyed by the key, `GET` a read of
the sequencer's image in memory, `INCR`, `MGET`, `DEL`, `KEYS` and
`DBSIZE` over the same log; `XADD` a put into a stream's own log, the
entry's id its position, `XRANGE` and `XREAD` reads by position. The
block above runs the door's loop three rounds with nobody connecting; a
client's session — `redis-cli -p 6379`, or any Redis library — is
tests/redis_door.rs, driven in both modes.
