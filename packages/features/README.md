# features

A feature store written in Crow over Magpie (`use magpie`): a feature
group is a log keyed by its entity with its event-time column
declared, so one write is both stores — the history for training is
the log, the newest value per entity for serving is the sequencer's
image — and there is no materialisation job between them. A feature's
definition is one Crow function, run over windows for the backfill and
over the log's tail for the stream. A training set is the labels joined
against each group's history as of each label's moment. The registry is
a log of its own. Each answers the same in `crow run -i` and `crow run`,
and `training` answers what pandas's `merge_asof` answers.

```
deps = map[features = 'github.com/agncrow/crow/packages/features@main']
```

`crow install` checks the package out into `crow_modules/features`, and
`crow.lock` pins the commit it took and the hash of the files it
installed. A program imports it and calls it qualified:

```
use magpie, frame
use features
```

## A group is a log

`declare[dir, entity, time]` names a group: the log at `dir`, keyed by
the entity's columns (`['user']`, or several), each row's moment in the
column `time` (milliseconds, or a column of moments). It answers a
`Group` record and writes nothing; the first `append` declares the log
so (Magpie's keys and time column, declared once).

`append[g, f]` appends feature rows — the entity's columns, the time
column and the features — through Magpie's `put`: visible to `online`
at once, durable within ten milliseconds (`sync` of the log, for
sooner). A `_pos` column a read left on the rows is dropped. It answers
the first position the rows took.

## Two reads, one log

- `online[g, key]`: the newest feature row of one entity (its key's
  values, each as text), from the image Magpie's sequencer keeps in
  memory — the online store's read, in microseconds; a frame of one row,
  or of none.
- `history[g, from = 0, upto = -1, columns = []]`: a window of the
  group's history by position — the offline store, read a window at a
  time, with the columns named.

## One function for the backfill and the stream

A feature function takes a frame of source rows and answers a frame of
feature rows (the entity's columns, the time column, the features):

```
fn spend[rows]
    out = select[rows, ['user', 'at']]
    out.big = rows.amount gt 50.0
    out.cents = rows.amount * 100.0
    return out
```

`backfill[g, source, f, from = 0, upto = -1, per = 100000]` runs it
over the source log in windows of `per` positions and appends what it
answers, and answers how many rows it wrote. `stream[g, source, f,
after, seconds]` runs the same function over the source's tail — the
rows past `after`, waited for up to the seconds — and answers the last
position taken, the loop's next `after` (`after` itself where none
came). The same rows come out of either path: a feature computed one
way for training and another for serving has no place to arise. A
function that needs a row's past reads it with `online` (the entity's
newest row) or `history`.

## A training set is the labels as of their moments

`training[labels, groups, columns = [], upto = -1]` joins the labels — the
entity's columns and a moment in the groups' time column — against
each group: beside each label, the group's newest row at or before the
label's moment (by time, then by position; missing cells where none
precedes), the labels' columns first. `columns` names the features
taken from each group, every column the labels lack when none is named.
The labels carry a column named as each group's time column. It is
Magpie's `prior` per group, read from each key's run in the key-sorted
segments and the tail, the history never held as one frame. `upto` is
the position the set is as of (-1 is now): a dataset a position names
comes out the same at every run.

## Freshness and the registry

`freshness[g]` answers a frame of one row: `head` (the log's last
position), `rows` and `newest` (the time column's largest value
written), from the log's figures and a summary pushed below the reader.
`define[dir, feature, g, meaning, computed = '', at = -1]` registers a
feature — its name, its group's log, entity and time column, what it
means and which function computes it — in the registry log at `dir`,
keyed by the feature's name; `defined[dir]` answers every feature's
newest definition, and the log keeps their history.

## Held to

`held.co` against `reference/reference.py` (pandas 3.0.2): sixty feature
rows pandas computed from a seeded stream (a running count and sum per
user) appended to a group, twelve labels, and the training set pandas's
`merge_asof` builds from them (by user, backward, exact moments
matching), held cell for cell — the same rows, the same features beside
each label, a missing cell where no row precedes. `features.co`'s test
blocks: one write read as both stores, a backfill over windows and a
stream over the tail answering the same rows, and the registry;
`example.co` runs interpreted and compiled in CI with one output.
Embeddings as features wait on the column of vectors (a language
feature, docs/magpie-design.md).
