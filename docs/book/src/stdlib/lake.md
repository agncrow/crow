# lake

The log as a directory of batch files: each commit one file written once, holding its rows' positions, sorted by key, with the statistics a reader prunes by; appends, reads from a position, the latest version of each key, lookups, and a sequencer that refuses a stale read.


Import with `use lake`. The log as a directory of batch files, written in Crow on the sys layer (design rule 20): each commit is one file, written once by the kernel (sys, `batch_write`) — a frame given a position per row in a column `_pos`, its rows sorted by the key columns, and in the file's head the statistics a reader prunes by (min, max and a bloom filter per column, a filter over the words of each text column indexed). One file is at once the log entry (positions), the level-0 segment (sorted, with statistics) and the search index (word filters). The numbering, the sequencer, the reads and the catalog's state are the module's; a log is a directory path, a snapshot a position.

| Function | Description | Example |
|----------|-------------|---|
| `batch[dir, f, by = [], texts = []]` | The frame written as the log's next batch file, its rows sorted by the `by` columns, the words of the `texts` columns indexed; answers the first position its rows got (the rows take consecutive positions in the order they arrive). An error value of kind `'lake'` where the directory is no log, the frame has no rows, or a column named is missing. Two writers that claim one number: the kernel links a file into place only where no file of the number exists, so the second claims the next | [The log as batch files: `use lake`](../by-example/lake.md) |
| `commit[dir, f, by = [], texts = [], seen = -1, columns = [], equal = []]` | The sequencer: a transaction that read at snapshot `seen` the keys `columns[i] = equal[i]` (each value as text) and writes `f`. Appended, as `batch` does, where none of those keys was written after `seen`; refused with an error value of kind `'lake'` naming the key and the position where one was (`conflict: id = 2 was written at position 2, after snapshot 1`). Validation covers every file before the number the write claims; a number another writer took first means the files are read again | [The log as batch files: `use lake`](../by-example/lake.md) |
| `head[dir]` | The log's last position; -1 for an empty log | [Frames: `use frame`](../by-example/frames.md) · [The log as batch files: `use lake`](../by-example/lake.md) |
| `since[dir, from]` | Every row at position `from` or later, as one frame in position order: a consumer's read, its offset the last position it saw | [The log as batch files: `use lake`](../by-example/lake.md) |
| `scan[dir, upto = -1]` | Every row of the log up to position `upto` (every row with -1), in position order: each version of each key | [The log as batch files: `use lake`](../by-example/lake.md) |
| `latest[dir, by, upto = -1]` | The newest version of each key (the `by` columns) up to the snapshot, in position order: the table the log holds | [The log as batch files: `use lake`](../by-example/lake.md) |
| `lookup[dir, column, value]` | The rows whose `column` holds the value, written as text (`'3'`, `'2.5'`, `'true'`; a moment as its milliseconds), in position order: each file asked through its filter (sys, `batch_find`), and a segment through its row groups' statistics, so a key the segment is sorted by reads one group of 8192 rows. An error value of kind `'lake'` for a column the log lacks or a value that is no value of its kind | [The log as batch files: `use lake`](../by-example/lake.md) |
| `merge[dir, by = []]` | Every batch file and segment of the log folded into one segment (sys, `batch_merge`): the rows sorted by the `by` columns, the latest version of each key kept (every row without keys), the inputs removed once the segment is in place, a stale batch a crash left behind with them. Answers the segment's path, or `''` where the log holds fewer than two files. The merge is the one other write a row pays: a version a merge folded is gone from `scan`, so time travel reaches back to the last merge, and a log merged without keys keeps every row | [The log as batch files: `use lake`](../by-example/lake.md) |
| `batches[dir]` | The catalog's state: a frame of the log's files, a row each — `number` and `high` (the batch numbers it covers), `form` (`batch`, `segment`, or `stale` for a batch a segment already holds), `path`, `rows`, `first` and `last` (positions) — and one row with an empty path and `last` -1 for an empty log | [The log as batch files: `use lake`](../by-example/lake.md) |

```
use lake
use frame

made = mkdir['orders']
first = batch['orders', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
log[first]                                   --- 0
second = batch['orders', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]
log[head['orders']]                          --- 3
log[latest['orders', ['id']].qty]            --- [10, 25, 30]
log[lookup['orders', 'id', '2']._pos]        --- [1, 2]
stale = commit['orders', frame.new[id = [2], qty = [26]], by = ['id'], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]                            --- conflict: id = 2 was written at position 2, after snapshot 1
log[merge['orders', by = ['id']]]            --- orders/000000000000-000000000001.segment
log[latest['orders', ['id']].qty]            --- [10, 25, 30]
```

**Design Note: one file per commit, and the module over it.** Considered: a write-ahead log of row-shaped entries, a memtable flushed to sorted segments and postings built beside them — three writes per row, three structures (rejected: every durable database pays them, and no reader of this log needs them apart: a batch file sorted by key with statistics in its head is the segment a scan prunes, the index a search prefilters, and the entry a consumer reads by position, so the row is written once at commit and again only when files are merged). Considered, for the kernel's reach: the sequencer in the runtime under a lock (rejected: the validation is policy, which is Crow's; the kernel claims a number atomically — a file linked into place only where none exists — and the Crow module validates every file before the number it claims, so a slip is a retry, not a hole). Considered, for the name: `log` (taken by the console), `store` (sys's `store` row), `journal` (rejected: long) — `lake`, since the merged form is the lakehouse. The position column is `_pos`, a name no table of a program is likely to use; a frame that holds one is refused. A value looked up is written as text so that one function serves every column kind, compared with `str` of the column on both sides and read by the kernel as the column's kind for its filter.


Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
