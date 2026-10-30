# Magpie's parts at work

Each file of the package is a part a program may import on its own —
`use magpie.lz77`, `use magpie.fileformat` — and call qualified
(`lz77.compress[…]`): the codec, the bit packing, a column chunk, a bloom
filter, the stemmer, a text column's postings, the file itself, the
manifest, the merge, the executor, the nearest rows, the search, the
Kafka wire's bytes and the sequencer. Each block below is a program that
depends on the package, with what it prints; `example.co` runs every one
in both modes. The functions are listed in [REFERENCE.md](REFERENCE.md);
the log they make up is the tour in [README.md](README.md).

## Magpie's codec, in Crow

Magpie's kernels were rewritten in Crow (docs/magpie-in-crow.md), the
codec first: `use magpie.lz77` brings `compress` and
`decompress`, an LZ77 byte codec of our own over the sys layer's bytes,
written with the bytes a kernel builds in place — `bytes[n]`, `set`,
`push` and `concat` where the bytes lie. A stream that is cut short, or
decodes to another count than promised, is an error value saying why.
The part's tests hold it to the streams the Rust kernel wrote, byte
for byte, recorded before the kernel was deleted.


```crow
use sys, magpie.lz77

phrase = 'the quick brown fox jumps over the lazy dog; '
data = bytes[concat[phrase, phrase]]
small = lz77.compress[data]
log[len[data]]
log[len[small]]
back = lz77.decompress[small, len[data]]
log[same[back, data]]
log[lz77.decompress[small, 50].reason]
log[lz77.decompress[slice[small, 0, 10], 90].reason]
```

```text
90
57
true
the codec stream decodes past the 50 bytes promised
the codec stream is cut short
```


## Bits and varints, in Crow

`use magpie.bitpack` is the second kernel ported: values packed at a
width of bits each, little-endian within and across bytes, and LEB128
varints for counts — what a Magpie file writes its columns and its
footer with. A value is a 64-bit pattern held as an int, so one at or
past 2^63 is the negative int of the same bits.


```crow
use sys, magpie.bitpack

codes = []
for c in [3, 0, 7, 7, 1, 5]
    codes = push[codes, int[c]]
width = bitpack.width_of[7]
log[width]
b = bitpack.packed[codes, width]
log[len[b]]
log[hex[b]]
log[bitpack.unpacked[b, 0, 6, width]]
counts = bitpack.put_var[bitpack.put_var[bytes[0], 300], 5]
log[hex[counts]]
log[bitpack.read_var[counts, 0]]
log[bitpack.read_var[counts, 2]]
log[bitpack.unpacked[bytes[1], 0, 4, 3].reason]
```

```text
3
3
c39f02
[3, 0, 7, 7, 1, 5]
ac0205
[300, 2]
[5, 3]
4 nums of 3 bits need 2 bytes, and 1 are there
```


## A column chunk, in Crow

`use magpie.columnchunk` is the third kernel ported: the cells of one column
over one row group as a Magpie file writes them — ints by frame of
reference or delta, floats as their bits, text plain or as a dictionary,
bools as a bitmap, moments, vectors — through the codec where it
shrinks them, the kernel's bytes byte for byte. `chunk` writes one,
`unchunk` reads it back as a column with its missing cells marked, and a
chunk cut short or of another kind is an error value saying why.


```crow
use sys, frame, magpie.columnchunk

df = frame.new[qty = [3, 1, 2, 1000, -7], name = ['a', 'b', 'a', 'a', 'b']]
ints = columnchunk.chunk[df.qty, 0, 5, false]
log[len[ints]]
log[byte[ints, 0]]
log[columnchunk.header[]]
back = columnchunk.unchunk[ints, 'int']
log[ints_of[back, 0, 5]]
names = columnchunk.chunk[df.name, 0, 5, false]
log[byte[names, 0]]
log[text[text_bytes_of[columnchunk.unchunk[names, 'text'], 0, 5]]]
log[columnchunk.unchunk[slice[ints, 0, 10], 'int'].reason]
log[columnchunk.unchunk[ints, 'float'].reason]
codes = []
for c in [0, 1, 0]
    codes = push[codes, int[c]]
few = text_dict_column[bytes['ab'], range[1, 3], codes, bytes[0]]
log[text_ends_of[few, 0, 3]]
log[cell_kind[few]]
```

```text
30
0
14
[3, 1, 2, 1000, -7]
4
abaab
the file ends 4 bytes early
a float chunk is not encoding 0
[1, 2, 3]
text
```


## A bloom filter, in Crow

`use magpie.bloom` is the fourth kernel ported: `k` hashes into `m`
bits, FNV-1a and a mix of it, the kernel's bits bit for bit — what a
file keeps per column chunk over its distinct values. A filter never
refuses what it holds, and admits a stranger now and then.


```crow
use sys, magpie.bloom

f = bloom.sized[3]
for w in ['apple', 'banana', 'cherry']
    f = bloom.add[f, bytes[w]]
log[bloom.admits[f, bytes['banana']]]
log[bloom.admits[f, bytes['durian']]]
raw = bloom.filter_bytes[f]
log[len[raw]]
log[byte[raw, 0]]
back = bloom.filter_of[raw]
log[bloom.admits[back, bytes['cherry']]]
log[bloom.filter_of[bytes[2]].reason]
log[same[bloom.filter_bytes[bloom.filled[bloom.sized[3], [bytes['apple'], bytes['banana'], bytes['cherry']]]], raw]]
```

```text
true
false
17
6
true
a filter needs its hash count and its size
true
```


## The stemmer and the tokenizer, in Crow

`use magpie.stemmer` is the fifth kernel ported: the Porter stemmer as the
paper gives it, for a log that declares `stem`, so `running`, `runs` and
`runner` all `run`; and `tokens` splits a text as an index and a query
split it, each run of letters and digits lowercased.


```crow
use sys, magpie.stemmer

for w in ['running', 'runs', 'runner', 'generalization', 'ponies', 'Fox', 'the']
    log['`w` → `stemmer.stem[w]`']
log[terms['The quick, brown Fox-42 jumps; über naïve 東京!']]
```

```text
running → run
runs → run
runner → runner
generalization → gener
ponies → poni
Fox → Fox
the → the
['the', 'quick', 'brown', 'fox', '42', 'jumps', 'über', 'naïve', '東京']
```


## A search index over a row group, in Crow

`use magpie.postings` is the sixth kernel ported: the lengths of a text
column's documents, a sorted dictionary of its terms, and each term's
postings in blocks with their bounds, the kernel's bytes byte for byte;
terms found by word, by prefix and within an edit distance.


```crow
use sys, frame, magpie.postings

df = frame.new[note = ['the quick brown fox', 'the fox and the dog', 'brown bread']]
made = postings.built[df.note, 0, 3, false]
log[made.docs]
log[made.words]
log[len[made.blob]]
ix = postings.parsed[made.blob]
log[ix.lengths]
log[ix.terms]
the = postings.term_of[ix, 'the']
log[ix.dfs[the]]
blocks = postings.blocks_of[ix, the]
log[blocks[0].posts[1].row]
log[blocks[0].posts[1].positions]
log[postings.with_prefix[ix, 'br']]
log[postings.within[ix, 'fix', 1]]
log[postings.term_of[ix, 'cat']]
log[postings.levenshtein['kitchen', 'kitten', 2]]
log[postings.parsed[bytes[3]].reason]
```

```text
3
11
258
[4, 5, 2]
['and', 'bread', 'brown', 'dog', 'fox', 'quick', 'the']
2
1
[0, 3]
[1, 2]
[4]
-1
2
the file ends 2 bytes early
```


`use magpie.fileformat` is the seventh kernel ported: the file itself — row
groups of chunks with their statistics and filters, a block per column,
the footer — written as the kernel writes it and read back by region.


```crow
use sys, frame, magpie.fileformat, magpie.bloom

fn _floats[a, b, c, d, z]
    out = []
    out = push[out, a]
    out = push[out, b]
    out = push[out, c]
    out = push[out, d]
    return push[out, z]

df = frame.new[id = [3, 1, 2, 1, 9], name = ['Apple pie', 'banana split', 'Cherry tart', 'apple crumble', 'durian'], ok = [true, false, true, true, false], _pos = [0, 1, 2, 3, 4]]
gap = bytes[0]
gap = push[gap, 23]
df.price = floats_column[_floats[2.5, 1.0, 0.0 / 0.0, 0.5, 4.0], gap]
nothing = slice[['x'], 0, 0]
made = fileformat.file_of[df, 3, 0, 4, ['id'], ['name'], nothing, false]
log[len[made]]
log[fileformat.rows_per_group[df]]
kept = store['ex.batch', made]
r = fileformat.opened['ex.batch']
log[r.footer.groups]
log[r.footer.columns[1].cells]
ids = fileformat.entries[r, 0]
log[ids[0].lo.whole]
log[ids[0].distinct]
prices = fileformat.entries[r, 4]
log[prices[1].absent]
names = fileformat.entries[r, 1]
log[names[0].docs]
log[bloom.admits[fileformat.token_filter[r, names[0]], bytes['cherry']]]
log[bloom.admits[fileformat.value_filter[r, ids[0]], pack[9, 8, little = true]]]
both = []
both = push[both, 0]
both = push[both, 1]
whole = fileformat.groups_of[r, both, fileformat.wanted[r, nothing]]
log[whole.name]
log[hex[present_of[whole.price, 0, 5]]]
two = fileformat.group_of[r, 1, fileformat.wanted[r, ['name', '_pos']]]
log[two._pos]
log[len[fileformat.empty_of[r, fileformat.wanted[r, nothing]]]]
log[fileformat.about['ex.batch'].docs]
log[fileformat.opened['nosuch.batch'].reason]
log[ints_of[fileformat.chunk_of[r, 0, ids[1]], 0, 2]]
log[len[fileformat.index_bytes[r, names[0]]]]
--- the writer in its parts: what file_of does, for a writer that streams groups
cols = fileformat.columns_of[df, ['name']]
wide = [false, false, false, false, false]
g1 = fileformat.group_bytes[df, 0, 3, cols, wide, false, 8]
g2 = fileformat.group_bytes[df, 3, 5, cols, wide, false, 8 + len[g1.data]]
out = concat[concat[bytes['MAGPIE'], push[push[bytes[0], 0], 2]], concat[g1.data, g2.data]]
blocks = []
for c in range[5]
    blocks = push[blocks, concat[g1.entries[c], g2.entries[c]]]
groups = []
groups = push[groups, g1.rows]
groups = push[groups, g2.rows]
log[same[fileformat.finish[out, blocks, groups, cols, 0, 4, ['id'], [], []], made]]
log[len[fileformat.magic[]]]
log[fileformat.shape_of[cols.names, cols.kinds, cols.indexed].indexed]
gone = rm['ex.batch']
```

```text
1530
89240
[3, 2]
text
1
3
1
3
true
false
['Apple pie', 'banana split', 'Cherry tart', 'apple crumble', 'durian']
17
[3, 4]
0
[0, 5, 0, 0, 0]
magpie: cannot read 'nosuch.batch': No such file or directory (os error 2)
[1, 9]
216
true
8
[false, true, false, false, false]
```


`use magpie.manifest` is the eighth kernel ported: a log's list of live
segments and its declarations, a file of one group replaced whole.


```crow
use sys, frame, magpie.manifest

dir = 'ex_manifest'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
log[manifest.listed[dir].found]
m = manifest.declare[dir, ['day', 'symbol'], ['time', 'narrow'], ['at', 'price']]
log[m.note_names]
back = manifest.listed[dir]
log[back.keys]
log[manifest.note_of[back, 'time']]
log[manifest.narrow_of[back]]
log[manifest.stems_of[back]]
log[len[manifest.catalog_of[dir]]]
log[manifest.segment_columns[]]
log[manifest.declare[dir, ['id'], [], []].reason]
log[manifest.batch_named[17]]
log[manifest.segment_named[3]]
log[manifest.number_of['000000000017.batch']]
log[manifest.counter_of['S000000000003.segment']]
log[manifest.counter[back]]
log[manifest.key_words[['a', 'b', 'c']]]
log[len[manifest.listed_bytes[back]] eq len[file['`dir`/manifest']]]
written = manifest.write_listed[dir, manifest.with_note[back, 'stem', 'true']]
log[manifest.stems_of[manifest.listed[dir]]]
log[manifest.unlisted[].found]
log[len[manifest.with_segments[back, manifest.catalog_of[dir]].segments]]
gone = rm['ex_manifest']
```

```text
false
['narrow', 'time']
['day', 'symbol']
at
['price']
false
0
['name', 'level', 'number', 'high', 'first', 'last', 'rows', 'bytes', 'ranges']
batch: the log at 'ex_manifest' is keyed by day and symbol, not by id
000000000017.batch
S000000000003.segment
17
3
0
a, b and c
true
true
false
0
```


`use magpie.merger` is the ninth kernel ported: the streaming merge,
batches and segments folded into segments of bounded size, one row group
per input held at a time, the latest version of each key kept. A batch
is written as the sequencer's flush writes one: `write_batch` gives each
row its position in the order it arrives and sorts the rows by the keys,
and `numbered_bytes` answers the bytes of rows already numbered.


```crow
use sys, frame, magpie.fileformat, magpie.manifest, magpie.merger

dir = 'ex_merge'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
m = manifest.declare[dir, ['id'], [], []]
none = slice[['x'], 0, 0]
a = frame.new[id = [1, 2, 3], qty = [10, 20, 30], _pos = [0, 1, 2]]
pa = merger.place[dir, manifest.batch_named[0], fileformat.file_of[a, 1024, 0, 2, ['id'], none, none, false]]
pb = merger.write_batch[dir, 1, 3, frame.new[id = [4, 2], qty = [40, 25]], ['id'], none]
log[pb]
b = frame.new[id = [2, 4], qty = [25, 40], _pos = [4, 3]]
log[same[file[pb], merger.numbered_bytes[dir, b, 3, ['id'], none]]]
seg = merger.merge_files[dir, [pa, pb], false]
log[seg]
back = manifest.listed[dir]
log[back.segments.name]
log[back.segments.rows]
log[merger.key_ranges[seg, ['id']]]
r = fileformat.opened[seg]
log[fileformat.group_of[r, 0, fileformat.wanted[r, none]].qty]
cut = merger.merge_into[dir, [pa, pb], ['id'], none, none, false, true, 7, 48, 120]
log[cut.counter]
log[cut.written[0].name]
log[cut.written[0].rows]
gone = rm['ex_merge']
```

```text
ex_merge/000000000001.batch
true
ex_merge/S000000000001.segment
['S000000000001.segment']
[4]
{"id":[1,4]}
[10, 25, 30, 40]
9
S000000000008.segment
4
```


`use magpie.executor` is the executor in Crow: `summary_of`, an aggregate
over a log's files pushed below the reader — each group pruned by its
statistics, its rows masked and grouped, the partials merged in order —
and `asof_rows`, each label's newest row at or before its moment.


```crow
use sys, frame, magpie.fileformat, magpie.merger, magpie.executor

dir = 'ex_exec'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
a = frame.new[sym = ['a', 'b', 'a'], px = [1.5, 2.0, 2.5], ts = [10, 20, 30], _pos = [0, 1, 2]]
p = merger.place[dir, '000000000000.batch', fileformat.file_of[a, 1024, 0, 2, ['sym'], none, none, false]]
s = executor.summary_of[[p], ['sym'], ['px', 'px'], ['mean', 'max'], none, none, none, '', '', -1]
log[s.sym]
log[s.px_mean]
log[s.px_max]
labels = frame.new[sym = ['a', 'b'], ts = [25, 25]]
j = executor.asof_rows[[p], labels, ['sym'], 'ts', none, -1]
log[j.columns]
log[j.px]
gone = rm['ex_exec']
```

```text
['a', 'b']
[2, 2]
[2.5, 2]
['sym', 'ts', 'px']
[1.5, 2]
```


`use magpie.knn` finds the rows nearest a query vector by the
cosine, every present row scored and the best k kept, an earlier
position first between equals.


```crow
use sys, frame, magpie.fileformat, magpie.merger, magpie.knn

dir = 'ex_nearest'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
v = frame.new[id = [1, 2, 3], e = [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]], _pos = [0, 1, 2]]
p = merger.place[dir, '000000000000.batch', fileformat.file_of[v, 1024, 0, 2, ['id'], none, none, false]]
hits = knn.nearest_rows[[p], 'e', [1.0, 0.0], 2]
log[hits.id]
log[hits.score]
gone = rm['ex_nearest']
```

```text
[1, 3]
[1, 0.7071067811865475]
```


`use magpie.searcher` is BM25 as Lucene scores it over a log as one
corpus: a word, a phrase, a prefix (`kit*`), a fuzzy word (`kitchn~`),
from a group's index where it has one and its text where it has none,
with a snippet around the first hit.


```crow
use sys, frame, magpie.fileformat, magpie.merger, magpie.searcher

dir = 'ex_search'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
d = frame.new[id = [1, 2, 3], note = ['a crate of apples', 'pears for the kitchen table', 'the kitchen table holds apples'], _pos = [0, 1, 2]]
p = merger.place[dir, '000000000000.batch', fileformat.file_of[d, 1024, 0, 2, ['id'], ['note'], none, false]]
hits = searcher.search_rows[[p], 'note', ['kitchen table'], 3, false, 10]
log[hits.id]
log[hits.snippet]
log[len[searcher.terms_of[['kit*', 'kitchn~1', 'apples'], false]]]
log[searcher.snippet_of['a crate of apples for the shop', searcher.terms_of[['apples'], false], 4, false]]
gone = rm['ex_search']
```

```text
[2, 3]
['…s for the kitchen table', 'the kitchen table holds app…']
3
… of apples for…
```


`use magpie.inflate`, `use magpie.snappy` and `use magpie.recordsets` are
the Kafka door's bytes in Crow: a gzip member or a raw deflate stream
inflated, a snappy block decoded, and a legacy message set read into a
frame and written from one, each message's CRC-32 checked.


```crow
use sys, frame, magpie.inflate, magpie.snappy, magpie.recordsets

data = bytes['hello hello hello hello']
log[text[inflate.gunzipped[inflate.gzip_stored[data]]]]
log[inflate.crc32[bytes['123456789']]]
log[text[snappy.unsnapped[snappy.snappy_literal[data]]]]
raw = bytes[0]
for b in [203, 72, 205, 201, 201, 87, 200, 64, 39, 1]
    raw = push[raw, int[b]]
log[text[inflate.inflated[raw]]]
f = frame.new[offset = [0, 1], at = [5, 6], key = ['a', 'b'], value = ['one', 'two']]
set_bytes = recordsets.message_set[f]
log[len[set_bytes]]
log[recordsets.message_rows[set_bytes].value]
```

```text
hello hello hello hello
3421780262
hello hello hello hello
hello hello hello hello
76
['one', 'two']
```


`use magpie.sequencer` is the sequencer in Crow: the rows put since the last
flush, numbered as they arrive and held for the process in its memory
with each key's last position and newest row, flushed as one batch file
by the work a put asks for — what this process reads at once, and
another once flushed.


```crow
use sys, frame, magpie.manifest, magpie.sequencer

dir = 'ex_tail'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
m = manifest.declare[dir, ['id'], none, none]
log[sequencer.put_rows[dir, frame.new[id = [1, 2], qty = [10, 20]]]]
log[sequencer.put_rows[dir, frame.new[id = [2], qty = [25]]]]
log[sequencer.last_position[dir, ['2']]]
log[sequencer.newest_row[dir, ['2']].qty]
done = sequencer.flush_rows[dir]
log[sequencer.pending_of[dir]]
log[len[sequencer.pending_rows[dir, 2, 2]] gt 0]
log[sequencer.pending_at['ex_tail/.pending@0-0'].ok]
log[sequencer.forget_log[dir]]
gone = rm['ex_tail']
```

```text
0
2
2
[25]
[0, -1, -1]
true
true
ex_tail
```
