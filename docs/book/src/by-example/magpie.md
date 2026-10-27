# The log as batch files: `use magpie`

```crow
use magpie
use frame

gone = rm['ex_magpie']
made = mkdir['ex_magpie']
first = batch['ex_magpie', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
log[first]
second = batch['ex_magpie', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]
log[second]
log[head['ex_magpie']]
log[latest['ex_magpie'].qty]
log[lookup['ex_magpie', ['id'], ['2']].qty]
stale = commit['ex_magpie', frame.new[id = [2], qty = [26]], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]
log[since['ex_magpie', 3].qty]
log[scan['ex_magpie', 0, 1].id]
log[len[batches['ex_magpie']]]
log[merge['ex_magpie']]
log[batches['ex_magpie'].form]
log[latest['ex_magpie'].qty]
log[batch['ex_magpie', frame.new[id = [4], qty = [40]], by = ['qty']].reason]
log[batch['ex_magpie', frame.new[id = [4], qty = [40]]]]
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
manifest to name it, which the same readers read. Reference: spec section 13 (magpie).

## Windows: a read is bounded, the log is walked

```crow
use magpie
use frame
use math

gone = rm['ex_windows']
made = mkdir['ex_windows']
a = batch['ex_windows', frame.new[day = ['d1', 'd1', 'd2', 'd2'], sym = ['A', 'B', 'A', 'B'], px = [1, 2, 3, 4]], by = ['day', 'sym']]
b = batch['ex_windows', frame.new[day = ['d2', 'd3'], sym = ['A', 'A'], px = [5, 6]]]
log[since['ex_windows', 2, 4].px]
log[scan['ex_windows', 1, 2, ['px']].px]
log[latest['ex_windows', lo = 'd2', hi = 'd2'].px]
log[latest['ex_windows', hi = 'd1'].sym]
total = 0
at = 0
last = head['ex_windows']
while at lte last
    window = since['ex_windows', at, at + 1, ['px']]
    total = total + int[sum[window.px]]
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
segments and its columns and nothing else. Reference: spec section 13 (magpie).

## The sequencer: put, a key's newest row, and a commit decided in memory

```crow
use magpie
use frame
use math

gone = rm['ex_tail']
made = mkdir['ex_tail']
first = put['ex_tail', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
log[first]
log[int[sum[batches['ex_tail'].rows]]]
log[since['ex_tail', 0].qty]
second = put['ex_tail', frame.new[id = [2, 3], qty = [25, 30]]]
log[latest['ex_tail', equal = ['2']].qty]
log[len[latest['ex_tail', equal = ['9']]]]
log[sync['ex_tail'] gte 1]
log[int[sum[batches['ex_tail'].rows]]]
stale = commit['ex_tail', frame.new[id = [2], qty = [26]], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]
fine = commit['ex_tail', frame.new[id = [2], qty = [26]], seen = 3, columns = ['id'], equal = ['2']]
log[fine]
log[latest['ex_tail', equal = ['2']].qty]
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
once, as the catalog's `pending` row; a thread flushes them as a batch
file within ten milliseconds, or `sync` does now, and `batch` is a put
and a flush, durable before it answers. `latest` of one key (`equal`) reads the newest row of a
key from the image the sequencer keeps in memory, and a `commit` whose
reads are the log's keys is decided against the sequencer's index in
memory. Reference: spec section 13 (magpie).

## A batch without the flush, and a sync

```crow
use magpie
use frame

gone = rm['ex_flush']
made = mkdir['ex_flush']
a = batch['ex_flush', frame.new[id = [1, 2], qty = [10, 20]], by = ['id'], flush = false]
b = batch['ex_flush', frame.new[id = [3], qty = [30]], by = ['id'], flush = false]
log[latest['ex_flush'].qty]
log[sync['ex_flush']]
gone = rm['ex_flush']
```

```text
[10, 20, 30]
2
```

With `flush = false` a batch is written but not flushed to the disk, so
a crash may lose the batches since the last `sync`: Redis's default
durability, for a log that chooses it; `sync` flushes every file of the
log and its directory, and a program calls it on a timer.

## A point read by two keys, and only the columns asked

```crow
use magpie
use frame

gone = rm['ex_keys']
made = mkdir['ex_keys']
a = batch['ex_keys', frame.new[day = [1, 1, 2, 2], sym = ['A', 'B', 'A', 'B'], px = [10, 11, 12, 13], qty = [1, 2, 3, 4]], by = ['day', 'sym']]
log[lookup['ex_keys', ['day', 'sym'], ['2', 'A']].px]
log[lookup['ex_keys', ['day'], ['1'], ['sym', 'px']].columns]
log[scan['ex_keys', 0, -1, ['qty']].qty]
log[since['ex_keys', 2, -1, ['sym', '_pos']].sym]
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

## Search over the log

```crow
use magpie
use frame

gone = rm['ex_search']
made = mkdir['ex_search']
a = batch['ex_search', frame.new[id = [1, 2, 3], text = ['the quick brown fox', 'a lazy dog', 'the fox and the dog']]]
b = batch['ex_search', frame.new[id = [4], text = ['brown bread']]]
hits = search['ex_search', 'text', ['brown', 'dog'], 3]
log[hits.id]
log[fmt[hits.score[0], decimals = 3]]
log[len[search['ex_search', 'text', ['zebra']]]]
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

## Search's forms: a phrase, a prefix, a fuzzy word, a snippet

```crow
use magpie
use frame

gone = rm['ex_forms']
made = mkdir['ex_forms']
a = batch['ex_forms', frame.new[id = [1, 2, 3, 4, 5], text = ['the kitchen table is set', 'a table in the garden', 'kitchens of the world', 'running the kitchen', 'tables and chairs']]]
log[search['ex_forms', 'text', ['kitchen table']].id]
log[search['ex_forms', 'text', ['kit*']].id]
log[search['ex_forms', 'text', ['kitchn~']].id]
log[search['ex_forms', 'text', ['table'], 2, snippet = 6].snippet]
m = merge['ex_forms']
log[search['ex_forms', 'text', ['kitchen table']].id]
log[search['ex_forms', 'text', ['tabel~', 'kitchen']].id]
s = batch['ex_stems', frame.new[id = [1, 2, 3], text = ['the tables', 'a table', 'no chairs']], stem = true]
log[search['ex_stems', 'text', ['tabling']].id]
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

## Aggregates below the reader, and the as-of join

```crow
use magpie
use frame

gone = rm['ex_summary']
made = mkdir['ex_summary']
a = batch['ex_summary', frame.new[day = [1, 1, 2, 2], sym = ['A', 'B', 'A', 'B'], px = [10.0, 20.0, 12.0, 22.0], vol = [1, 2, 3, 4]], by = ['day', 'sym']]
s = summary['ex_summary', ['sym'], ['px', 'vol'], ['mean', 'sum']]
log[s.columns]
log[s.px_mean]
log[s.vol_sum]
log[summary['ex_summary', ['day'], ['vol'], ['count'], on = ['sym'], ops = ['eq'], equal = ['A']].vol_count]
log[summary['ex_summary', [], ['px'], ['max'], lo = '2'].px_max]
gone = rm['ex_summary']
gone = rm['ex_asof']
made = mkdir['ex_asof']
b = batch['ex_asof', frame.new[sym = ['A', 'A', 'B'], t = [10, 20, 15], px = [1.0, 2.0, 5.0]], by = ['sym'], time = 't']
labels = frame.new[sym = ['A', 'B', 'A'], t = [25, 12, 5]]
j = prior['ex_asof', labels]
log[j.columns]
log[j.px[0]]
log[j.px[1].ok]
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
```

`summary` is the aggregate pushed below the reader: each row group is
pruned by its statistics, decoded for the columns named alone, filtered
by the predicates, grouped and reduced on its own core, and the partials
are combined, so the rows are never held as one frame. It answers what
`group` answers over `scan` of the same rows. `prior` is the as-of join
over the log: for each label's keys and moment, the newest row at or
before it, from the key's run.

## A view that follows the log

```crow
use magpie
use frame
use math

gone = rm['ex_tail']
made = mkdir['ex_tail']
first = batch['ex_tail', frame.new[sym = ['A', 'B'], qty = [10, 20]]]
seen = -1
total = 0
rows = tail['ex_tail', seen, 0.2]
total = total + sum[rows.qty]
seen = max[rows._pos]
second = batch['ex_tail', frame.new[sym = ['A'], qty = [5]]]
rows = tail['ex_tail', seen, 0.2]
total = total + sum[rows.qty]
seen = max[rows._pos]
log[total]
log[seen]
none = tail['ex_tail', seen, 0.05]
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

## Operations: the figures, a backup, a restore

```crow
use magpie
use frame

gone = rm['ex_ops']
gone = rm['ex_ops_copy']
made = mkdir['ex_ops']
a = batch['ex_ops', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
b = batch['ex_ops', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]
m = metrics['ex_ops']
log[m.files]
log[m.rows]
log[m.head]
log[m.bytes[0] gt 0]
log[backup['ex_ops', 'ex_ops_copy']]
gone = rm['ex_ops']
log[restore['ex_ops_copy', 'ex_ops']]
log[latest['ex_ops'].qty]
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

## The HTTP door: the verbs from another process

```crow
use magpie
use frame

gone = rm['ex_door']
made = mkdir['ex_door']
a = batch['ex_door/orders', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
served = door['ex_door', 0, rounds = 3]
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
in both modes is tests/magpie_door.rs. Reference: spec section 13
(magpie).

## A second node

```crow,norun
use magpie

--- the leader: its log served to followers, forever
served = share['orders', 3700]
```

```crow,norun
use magpie

--- a follower, on another machine: the files it lacks, fetched; then
--- the log read by position as any log is, and the pass run again
seen = -1
came = follow['orders', 'leader.local', 3700]
rows = since['orders', seen + 1]
```

A second node follows the log: it holds the same files and reads them
with the same verbs. A table too large for one node is split into logs
by a key, one on each node, and a query runs the same verbs on each and
stacks the answers.
