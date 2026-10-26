# The log as batch files: `use lake`

```crow
use lake
use frame

gone = rm['ex_lake']
made = mkdir['ex_lake']
first = batch['ex_lake', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]
log[first]
second = batch['ex_lake', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]
log[second]
log[head['ex_lake']]
log[latest['ex_lake', ['id']].qty]
log[lookup['ex_lake', 'id', '2'].qty]
stale = commit['ex_lake', frame.new[id = [2], qty = [26]], by = ['id'], seen = 1, columns = ['id'], equal = ['2']]
log[stale.reason]
log[since['ex_lake', 3].qty]
log[scan['ex_lake', 1].id]
log[len[batches['ex_lake']]]
log[merge['ex_lake', by = ['id']]]
log[batches['ex_lake'].form]
log[latest['ex_lake', ['id']].qty]
gone = rm['ex_lake']
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
ex_lake/000000000000-000000000001.segment
['segment']
[10, 25, 30]
```

Each `batch` is one file in the directory, written once: the rows take
the next positions, sorted by the `by` columns, with the statistics a
reader prunes by in the file's head. `latest` is the table the log
holds, one row per key; `lookup` reads only the files whose filters admit
the value; `commit` is the sequencer, refusing a write whose reads are
stale; `merge` folds the files into one Parquet segment holding the latest
version of each key, which the same readers read. Reference: spec section 13 (lake).
