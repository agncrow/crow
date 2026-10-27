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
log[latest['ex_magpie', ['id']].qty]
log[lookup['ex_magpie', ['id'], ['2']].qty]
stale = commit['ex_magpie', frame.new[id = [2], qty = [26]], by = ['id'], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]
log[since['ex_magpie', 3].qty]
log[scan['ex_magpie', 1].id]
log[len[batches['ex_magpie']]]
log[merge['ex_magpie', by = ['id']]]
log[batches['ex_magpie'].form]
log[latest['ex_magpie', ['id']].qty]
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
ex_magpie/000000000000-000000000001.segment
['segment']
[10, 25, 30]
```

Each `batch` is one file in the directory, written once: the rows take
the next positions, sorted by the `by` columns, with the statistics a
reader prunes by in the file's head. `latest` is the table the log
holds, one row per key; `lookup` reads only the files whose filters admit
the value; `commit` is the sequencer, refusing a write whose reads are
stale; `merge` folds the files into one Parquet segment holding the latest
version of each key, which the same readers read. Reference: spec section 13 (magpie).

## A batch without the flush, and a sync

```crow
use magpie
use frame

gone = rm['ex_flush']
made = mkdir['ex_flush']
a = batch['ex_flush', frame.new[id = [1, 2], qty = [10, 20]], by = ['id'], flush = false]
b = batch['ex_flush', frame.new[id = [3], qty = [30]], by = ['id'], flush = false]
log[latest['ex_flush', ['id']].qty]
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
log[scan['ex_keys', -1, ['qty']].qty]
log[since['ex_keys', 2, ['sym', '_pos']].sym]
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
a = batch['ex_search', frame.new[id = [1, 2, 3], text = ['the quick brown fox', 'a lazy dog', 'the fox and the dog']], texts = ['text']]
b = batch['ex_search', frame.new[id = [4], text = ['brown bread']], texts = ['text']]
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
log[latest['ex_ops', ['id']].qty]
gone = rm['ex_ops']
gone = rm['ex_ops_copy']
```

```text
[2]
[4]
[3]
true
2
2
[10, 25, 30]
```

`metrics` is the log's figures as a frame of one row, a table a program
keeps by writing it to a log of its own. `backup` copies the live files
under a directory or an `s3://` prefix and writes the manifest last, so a
backup is whole only when its manifest is; `restore` copies them back,
and the batches written since are the rest of the log.

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
