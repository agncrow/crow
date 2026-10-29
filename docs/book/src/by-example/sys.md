# The standard library's layer: `use sys`

`sys` is the layer the standard library's own modules are written on —
bytes, and a connection to a host. A program may not import it (a program
serves with `serve`, reads with `request`, `read` and `socket`, connects
with `connect`); these blocks run through the book harness's door, which
a program has not.

```crow
use sys

b = bytes['héllo']
log[len[b]]
log[type[b]]
log[b]
log[text[slice[b, 3, 6]]]
log[find[b, bytes['llo']]]
log[byte[b, 0]]
log[text[concat[slice[b, 0, 1], slice[b, 3, 6]]]]
bad = text[slice[b, 1, 2]]
log[bad.ok]
log[bad.kind]
log[b eq bytes['héllo']]
```

```text
6
bytes
<6 bytes>
llo
3
104
hllo
false
parse
true
```

A connection nobody answers is the call's error value, of kind `sys`:

```crow
use sys

s = open['127.0.0.1', 1, false, 0.2]
log[s.ok]
log[s.kind]
late = send[s, bytes['x']]
log[late.ok]
```

```text
false
sys
false
```

A process the program talks to is a socket too (`start`): the command
runs as `shell` runs one, what is sent is its input, what is read its
output, and the socket keeps what a read took past a message. One process
per command on each thread: a second `start` of the command answers the
running one, and starts it again once it has ended. A process ends when
its stream is closed — its input closed, then, if it stays, a signal —
and a command the shell does not find says so on standard error, then
ends: a read answers nothing, a write that the process ended.

```crow
use sys

p = start['/bin/cat']
log[type[p]]
log[p]
log[send[stream_of[p], bytes['hello\n']]]
log[trim[text[recv[stream_of[p], 64, 5.0], '?']]]
again = start['/bin/cat']
log[type[again]]
close[stream_of[p]]
late = send[stream_of[p], bytes['x']]
log[late.reason]
gone = start['no-such-command-for-the-book']
log[len[recv[stream_of[gone], 64, 5.0]]]
log[send[stream_of[gone], bytes['x\n']].reason]
log[start[''].reason]
```

```text
socket
<socket /bin/cat>
6
hello
socket
the stream is closed
0
the process ended
a command is text, and '' is none
```

The rest of the layer, for the modules written on it — a file's bytes and its lines read a block at a time, chunks joined (`concat`), digests, and a request's answer made from its parts:

```crow
use sys

store['sys-note.txt', bytes['one\ntwo']]
log[len[file['sys-note.txt']]]
log[file_lines['sys-note.txt']]
log[text[concat[[bytes['ab'], bytes['cd']]], '?']]
log[hex[bytes['hi']]]
log[hash[bytes['hello'], 'sha256']]
log[hex[hmac[bytes['key'], bytes['hello'], 'sha256']]]
log[len[random[8]]]
log[text[slice[bytes['héllo'], 0, 2], '?']]
r = reply[404, 'no such thing', map[]]
log[r.status]
log[len[file['no-such-file'].reason] gt 0]
```

```text
7
['one', 'two']
abcd
6869
2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
9307b3b915efb5171ff14d8cb55fbcc798c6c0ef1456d66ded1a6aa723a58b7b
8
h?
404
true
```

The parsers the readers stand on, and the protocol's pure pieces — a table's format, a URL's query, a redirect's target, an answer from its parts:

```crow
use sys, frame

df = frame_of_csv['sym,qty\nAAPL,100\nMSFT,200\n']
log[df.qty]
log[frame_of_csv_bytes[bytes['sym,qty\nAAPL,100\n']].sym]
log[len[frame_csv[df]]]
log[frame_of_json[frame_json[df]].sym]
log[len[frame_of_parquet[frame_parquet[df]]]]
rows = csv_rows['a,b\n1,2\n']
log[rows[0]['b']]
m = tensor_of_frame[frame_of_csv['x,y\n1,2\n3,4\n']]
log[m.shape]
log[tensor_of_frame[df].kind]
log[remote['s3://bucket/key']]
log[format_of['https://x.example.com/bars.csv?day=1']]
log[format_sniffed['application/json', bytes['']]]
log[format_sniffed['', bytes['PAR1...']]]
log[with_query['https://x.example.com/q?a=1', map[q = 'crow lang']]]
log[redirected['https://x.example.com/a/b', '/c']]
a = answer[200, 'OK', map[], bytes['hi']]
log[a.status]
log[text[a.body, '?']]
```

```text
[100, 200]
['AAPL']
26
['AAPL', 'MSFT']
2
2
[2, 2]
load
true
csv
json
parquet
https://x.example.com/q?a=1&q=crow%20lang
https://x.example.com/c
200
hi
```

Bytes as text and back, a signature compared, a number's bytes, and a response of bytes — what a token, a frame's header and a packed value are written over — and the request fixtures `request` asks for before it sends (`replaying`, `recording`, `rehearsing`, `replayed`, `recorded`: no replay, recording or rehearsal runs here, so nothing is held and nothing kept):

```crow
use sys, web.mail, math

b = bytes['hi']
log[base64[b]]
log[base64url[bytes['hi?']]]
log[text[unbase64['aGk=']]]
log[same[b, bytes['hi']]]
log[same[b, bytes['ho']]]
log[hex[pack[258, 2]]]
log[hex[pack[-1, 1]]]
log[hex[ieee[1.0]]]
log[type[binary[b, 'text/plain']]]
log[hex[digest[bytes['abc'], 'sha1']]]
log[hex[mask[bytes['Hello'], pack[939139389, 4]]]]
log[civil[1369353600]]
m = smtp['smtp.example.com', 587, 'ada@example.com', 'secret']
log[mailer_field[m, 'host']]
log[fmt[betainc[0.3, 1.0, 1.0], decimals = 3]]
log[fmt[gammainc[1.0, 2.0], decimals = 3]]
log[fmt[lgamma[5.0], decimals = 3]]
log[shaped[arange[0, 6, 1], push[push[[], 2], 3]]]
log[frame_of_rowbinary[unbase64['AgFuAXgGVUludDY0EU51bGxhYmxlKEZsb2F0NjQpAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAE=']]]
log[frame_of_postgres[unbase64['VAAAAC4AAm4AAAAAAAAAAAAAFwAE/////wAAcwAAAAAAAAAAAAAZ////////AABEAAAAEAACAAAAATEAAAABYUQAAAAPAAIAAAABMv////9DAAAADVNFTEVDVCAyAFoAAAAFSQ==']]]
log[machine[] ne '']
log[process[] gt 0]
log[alive[process[]]]
log[len[rowbinary_of_frame[frame_of_csv['n,s\n1,a\n2,b\n'], ['UInt64', 'String']]]]
log[hex[xor[bytes['ab'], bytes['  ']]]]
log[len[copy_of_frame[frame_of_csv['n,s\n1,a\n2,b\n']]]]
log[replaying[]]
log[recording[]]
log[rehearsing[]]
miss = replayed['GET', 'http://x.test/a', '']
log[miss.kind]
kept = recorded['GET', 'http://x.test/a', '', 200, 'hello', map[]]
log[kept.ok]
```

```text
aGk=
aGk_
hi
true
false
0102
ff
3ff0000000000000
response
a9993e364706816aba3e25717850c26c9cd0d89d
7f9f4d5158
[2013, 5, 24, 0, 0, 0, 5]
smtp.example.com
0.300
0.865
3.178
[[0, 1, 2], [3, 4, 5]]
n        x
int    float
-  -------
0        0
1  missing

[2 rows x 2 cols]
n        s
int      str
-  -------
1        a
2  missing

[2 rows x 2 cols]
true
true
true
20
4142
8
false
false
false
http
true
```

The protocol itself reaches a host — one exchange, a reader's fetch with its options and redirects, an S3 object put signed with the environment's credentials — so these lines are not run here:

```crow,norun
use sys

got = exchange['https://example.com/', 'GET', ['accept: text/html'], bytes[''], 5.0]
log[got.status]
page = fetch['https://example.com/data.csv', map[authorization = 'Bearer k'], map[day = '2026-07-14'], 10.0]
log[page.ok]
kept = put['s3://reports/aapl.parquet', bytes['PAR1']]
log[kept.ok]
s = upgrade['wss://feed.example.com/stocks', 0.0]
first = leftover[s]
unread[s, first]
head = read_head[stream_of[s], now[] + 30.0]
same_socket = socket_of[stream_of[s], 'wss://feed.example.com/stocks', bytes['']]
relay = open['smtp.example.com', 587, false, 30.0]
secured = secure[relay, 'smtp.example.com']
```

Reference: spec section 13 (sys).

A driver written in Crow keeps a connection and sends statements on it
(docs/db-in-crow-design.md): notes on the connection by key — a query's id
for a cancel later — and one request on the connection already open, kept
alive for the next.

```crow,norun
use sys

s = open['db.example.com', 8123, false, 5.0]
note[s, 'query', 'crow-local-4242-1a2b']
log[noted[s, 'query']]
a = exchange_on[s, 'db.example.com:8123', 'POST', '/?default_format=TabSeparated', [], bytes['SELECT 1'], now[] + 30.0, true]
log[a.status]
```

A standard module that hands functions to a model, or runs one by its
name, has two operations over functions (spec section 13, sys; the
compiler lowers each to what inference knows, so both modes agree):
`describe` answers a function's description — its name, the comment above
it, its parameters with the word errors use for each kind and its default,
what it answers — and `apply` calls a function with its arguments read by
name from a parsed value, answering the value encoded as text, or an error
of kind `call` where an argument is missing or of another kind. A list
literal of functions is a group, each held by its name, so a loop over it
describes and calls each.

```crow
use sys

--- the forecast for a city
fn forecast[city, days = 2]
    return '`days` days of sun in `upper[city]`'

--- the time somewhere
fn clock[zone]
    return 'noon in `upper[zone]`'

d = describe[forecast]
log[str[d.name]]
log[str[d.doc]]
log[str[d.params[1].name]]
log[str[d.params[1].takes]]
log[int[d.params[1].default]]
log[apply[forecast, json['{"city": "Oslo", "days": 3}']]]
missing = apply[forecast, json['{"city": "Oslo"}']]
log[missing.kind]
for tool in [forecast, clock]
    log[str[describe[tool].name]]
log[apply[clock, json['{"zone": "Rome"}']]]
```

```text
forecast
the forecast for a city
days
an int
2
"3 days of sun in OSLO"
call
forecast
clock
"noon in ROME"
```

A program listens too: `listen` binds a port (0 for one the system picks,
`address` says which), and `accept` answers each connection that arrives
as a stream of its own. A server written in Crow over the sys layer, as
the Kafka door is, stands on these.

```crow
use sys

l = listen['127.0.0.1', 0]
port = int[split[address[l], ':'][1]]
client = open['127.0.0.1', port, false, 2.0]
server = accept[l, 2.0]
sent = send[client, bytes['ping']]
log[text[recv[server, 100, 2.0]]]
sent = send[server, bytes['pong']]
log[text[recv[client, 100, 2.0]]]
log['is a listener: it is accepted on, not read' in recv[l, 10, 0.1].reason]
log[accept[client, 0.1].kind]
late = listen['127.0.0.1', 0]
log[accept[late, 0.05].kind]
```

```text
ping
pong
true
sys
sys
```

## Magpie's codec, in Crow

Magpie's kernels were rewritten in Crow (docs/magpie-in-crow.md), the
codec first: `use magpie.codec` brings `compress` and
`decompress`, an LZ77 byte codec of our own over the sys layer's bytes,
written with the bytes a kernel builds in place — `bytes[n]`, `set`,
`push` and `concat` where the bytes lie. A stream that is cut short, or
decodes to another count than promised, is an error value saying why.
The module's tests hold it to the streams the Rust kernel wrote, byte
for byte, recorded before the kernel was deleted.

```crow
use sys, magpie.codec

phrase = 'the quick brown fox jumps over the lazy dog; '
data = bytes[concat[phrase, phrase]]
small = compress[data]
log[len[data]]
log[len[small]]
back = decompress[small, len[data]]
log[same[back, data]]
log[decompress[small, 50].reason]
log[decompress[slice[small, 0, 10], 90].reason]
```

```text
90
57
true
the codec stream decodes past the 50 bytes promised
the codec stream is cut short
```

## Bits and varints, in Crow

`use magpie.packing` is the second kernel ported: values packed at a
width of bits each, little-endian within and across bytes, and LEB128
varints for counts — what a Magpie file writes its columns and its
footer with. A value is a 64-bit pattern held as an int, so one at or
past 2^63 is the negative int of the same bits.

```crow
use sys, magpie.packing

codes = []
for c in [3, 0, 7, 7, 1, 5]
    codes = push[codes, int[c]]
width = width_of[7]
log[width]
b = packed[codes, width]
log[len[b]]
log[hex[b]]
log[unpacked[b, 0, 6, width]]
counts = put_var[put_var[bytes[0], 300], 5]
log[hex[counts]]
log[read_var[counts, 0]]
log[read_var[counts, 2]]
log[unpacked[bytes[1], 0, 4, 3].reason]
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

## Bytes, files and columns for a kernel in Crow

What a file format's writer and reader need beyond bytes written in
place (docs/magpie-in-crow.md, section 2): numbers in bytes at either
byte order, a float's bits in 8 or 4 bytes, the checksum gzip and Kafka
carry; a file read by region, flushed through to the disk, linked into
its name and moved over another; and a column read as the vectors it
is held in and built from them, with no value per cell.

```crow
use sys, frame

--- numbers written into bytes, and read back
log[hex[pack[258, 2]]]
log[hex[pack[258, 2, little = true]]]
log[unpack[pack[-2, 2], 0, 2]]
log[unpack[pack[65534, 2], 0, 2, signed = false]]
log[hex[ieee[1.5]]]
log[hex[ieee[1.5, width = 4, little = true]]]
log[ieee[ieee[2.25, width = 4, little = true], 0, 4, true]]
log[hash[bytes['123456789'], 'crc32']]
log[len[digest[bytes['123456789'], 'crc32']]]

--- a file placed the way a kernel places one: written beside its name,
--- flushed, linked into the name, the directory flushed; read by region
kept = store['part.bin', bytes['0123456789']]
synced = flushed['part.bin']
placed = link['part.bin', 'placed.bin']
synced = flushed['.']
log[hex[file['placed.bin', 3, 4]]]
log[file['placed.bin', 8, 4].reason]
log[link['part.bin', 'placed.bin'].ok]
moved = move['part.bin', 'moved.bin']
log[exists['part.bin']]
log[filesize['moved.bin']]
log[type[modified['moved.bin']]]

--- a column as its parts, and from them: no value per cell
df = frame.new[id = [3, 1, 2], name = ['Apple', 'banana', 'cherry'], price = [1.5, 2.0, 0.25], flag = [true, false, true], at = [moment[0.0], moment[1.0], moment[2.0]], e = [[1.0, 0.0], [0.0, 1.0], [0.5, 0.5]]]
log[cell_kind[df.name]]
log[ints_of[df.id, 1, 3]]
log[floats_of[df.price, 0, 3]]
log[bools_of[df.flag, 0, 3]]
log[text[text_bytes_of[df.name, 0, 3]]]
log[text_ends_of[df.name, 0, 3]]
log[millis_of[df.at, 0, 3]]
log[zones_of[df.at, 0, 3]]
log[vector_width[df.e]]
log[vectors_of[df.e, 2, 3]]
log[len[present_of[df.id, 0, 3]]]
log[missing_in[df.id, 0, 3]]
back = frame.new[]
back.id = ints_column[ints_of[df.id, 0, 3], bytes[0]]
back.name = text_column[text_bytes_of[df.name, 0, 3], text_ends_of[df.name, 0, 3], bytes[0]]
back.price = floats_column[floats_of[df.price, 0, 3], present_of[df.price, 0, 3]]
back.at = moments_column[millis_of[df.at, 0, 3], zones_of[df.at, 0, 3], bytes[0]]
back.e = vectors_column[2, vectors_of[df.e, 0, 3], bytes[0]]
bits = bytes[1]
bits = set[bits, 0, 5]
back.flag = bools_column[bits, 3, bytes[0]]
log[back.columns]
log[back.flag]
present = bytes[1]
present = set[present, 0, 5]
partial = ints_column[ints_of[df.id, 0, 3], present]
log[missing_in[partial, 0, 3]]
log[hex[present_of[partial, 0, 3]]]
```

```text
0102
0201
-2
65534
3ff8000000000000
0000c03f
2.25
cbf43926
4
33343536
file: 4 bytes at 8 lie outside 'placed.bin', which holds 10
false
false
10
moment
text
[1, 2]
[1.5, 2, 0.25]
[true, false, true]
Applebananacherry
[5, 11, 17]
[0, 1000, 2000]
[0, 0, 0]
2
[0.5, 0.5]
0
0
['id', 'name', 'price', 'at', 'e', 'flag']
[true, false, true]
1
05
```

A chunk's floats at once, where `ieee` writes and reads one: a list of
floats as their bits one after another, 8 bytes each or 4 of single
precision, and read back. A region outside the bytes stops.

```crow
use sys

xs = []
xs = push[xs, 1.5]
xs = push[xs, -2.0]
packed = float_bytes[xs, 4, true]
log[hex[packed]]
log[float_values[packed, 0, 2, 4, true]]
log[float_values[packed, 4, 1, 4, true]]
log[len[float_bytes[xs, 8, false]]]
```

```text
0000c03f000000c0
[1.5, -2]
[-2]
16
```

## A column chunk, in Crow

`use magpie.encode` is the third kernel ported: the cells of one column
over one row group as a Magpie file writes them — ints by frame of
reference or delta, floats as their bits, text plain or as a dictionary,
bools as a bitmap, moments, vectors — through the codec where it
shrinks them, the kernel's bytes byte for byte. `chunk` writes one,
`unchunk` reads it back as a column with its missing cells marked, and a
chunk cut short or of another kind is an error value saying why.

```crow
use sys, frame, magpie.encode

df = frame.new[qty = [3, 1, 2, 1000, -7], name = ['a', 'b', 'a', 'a', 'b']]
ints = chunk[df.qty, 0, 5, false]
log[len[ints]]
log[byte[ints, 0]]
log[header[]]
back = unchunk[ints, 'int']
log[ints_of[back, 0, 5]]
names = chunk[df.name, 0, 5, false]
log[byte[names, 0]]
log[text[text_bytes_of[unchunk[names, 'text'], 0, 5]]]
log[unchunk[slice[ints, 0, 10], 'int'].reason]
log[unchunk[ints, 'float'].reason]
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

f = sized[3]
for w in ['apple', 'banana', 'cherry']
    f = add[f, bytes[w]]
log[admits[f, bytes['banana']]]
log[admits[f, bytes['durian']]]
raw = filter_bytes[f]
log[len[raw]]
log[byte[raw, 0]]
back = filter_of[raw]
log[admits[back, bytes['cherry']]]
log[filter_of[bytes[2]].reason]
log[same[filter_bytes[filled[sized[3], [bytes['apple'], bytes['banana'], bytes['cherry']]]], raw]]
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

`use magpie.stem` is the fifth kernel ported: the Porter stemmer as the
paper gives it, for a log that declares `stem`, so `running`, `runs` and
`runner` all `run`; and `tokens` splits a text as an index and a query
split it, each run of letters and digits lowercased.

```crow
use sys, magpie.stem

for w in ['running', 'runs', 'runner', 'generalization', 'ponies', 'Fox', 'the']
    log['`w` → `stem[w]`']
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
made = built[df.note, 0, 3, false]
log[made.docs]
log[made.words]
log[len[made.blob]]
ix = parsed[made.blob]
log[ix.lengths]
log[ix.terms]
the = term_of[ix, 'the']
log[ix.dfs[the]]
blocks = blocks_of[ix, the]
log[blocks[0].posts[1].row]
log[blocks[0].posts[1].positions]
log[with_prefix[ix, 'br']]
log[within[ix, 'fix', 1]]
log[term_of[ix, 'cat']]
log[levenshtein['kitchen', 'kitten', 2]]
log[parsed[bytes[3]].reason]
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

`use magpie.file` is the seventh kernel ported: the file itself — row
groups of chunks with their statistics and filters, a block per column,
the footer — written as the kernel writes it and read back by region.

```crow
use sys, frame, magpie.file, magpie.bloom

fn _floats[a, b, c, d, e]
    out = []
    out = push[out, a]
    out = push[out, b]
    out = push[out, c]
    out = push[out, d]
    return push[out, e]

df = frame.new[id = [3, 1, 2, 1, 9], name = ['Apple pie', 'banana split', 'Cherry tart', 'apple crumble', 'durian'], ok = [true, false, true, true, false], _pos = [0, 1, 2, 3, 4]]
gap = bytes[0]
gap = push[gap, 23]
df.price = floats_column[_floats[2.5, 1.0, 0.0 / 0.0, 0.5, 4.0], gap]
nothing = slice[['x'], 0, 0]
made = file_of[df, 3, 0, 4, ['id'], ['name'], nothing, false]
log[len[made]]
log[rows_per_group[df]]
kept = store['ex.batch', made]
r = opened['ex.batch']
log[r.footer.groups]
log[r.footer.columns[1].cells]
ids = entries[r, 0]
log[ids[0].lo.whole]
log[ids[0].distinct]
prices = entries[r, 4]
log[prices[1].absent]
names = entries[r, 1]
log[names[0].docs]
log[admits[token_filter[r, names[0]], bytes['cherry']]]
log[admits[value_filter[r, ids[0]], pack[9, 8, little = true]]]
both = []
both = push[both, 0]
both = push[both, 1]
whole = groups_of[r, both, wanted[r, nothing]]
log[whole.name]
log[hex[present_of[whole.price, 0, 5]]]
two = group_of[r, 1, wanted[r, ['name', '_pos']]]
log[two._pos]
log[len[empty_of[r, wanted[r, nothing]]]]
log[about['ex.batch'].docs]
log[opened['nosuch.batch'].reason]
log[ints_of[chunk_of[r, 0, ids[1]], 0, 2]]
log[len[index_bytes[r, names[0]]]]
--- the writer in its parts: what file_of does, for a writer that streams groups
cols = columns_of[df, ['name']]
wide = [false, false, false, false, false]
g1 = group_bytes[df, 0, 3, cols, wide, false, 8]
g2 = group_bytes[df, 3, 5, cols, wide, false, 8 + len[g1.data]]
out = concat[concat[bytes['MAGPIE'], push[push[bytes[0], 0], 2]], concat[g1.data, g2.data]]
blocks = []
for c in range[5]
    blocks = push[blocks, concat[g1.entries[c], g2.entries[c]]]
groups = []
groups = push[groups, g1.rows]
groups = push[groups, g2.rows]
log[same[finish[out, blocks, groups, cols, 0, 4, ['id'], [], []], made]]
log[len[magic[]]]
log[shape_of[cols.names, cols.kinds, cols.indexed].indexed]
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
log[listed[dir].found]
m = declare[dir, ['day', 'symbol'], ['time', 'narrow'], ['at', 'price']]
log[m.note_names]
back = listed[dir]
log[back.keys]
log[note_of[back, 'time']]
log[narrow_of[back]]
log[stems_of[back]]
log[len[catalog_of[dir]]]
log[segment_columns[]]
log[declare[dir, ['id'], [], []].reason]
log[batch_named[17]]
log[segment_named[3]]
log[number_of['000000000017.batch']]
log[counter_of['S000000000003.segment']]
log[counter[back]]
log[key_words[['a', 'b', 'c']]]
log[len[listed_bytes[back]] eq len[file['`dir`/manifest']]]
written = write_listed[dir, with_note[back, 'stem', 'true']]
log[stems_of[listed[dir]]]
log[unlisted[].found]
log[len[with_segments[back, catalog_of[dir]].segments]]
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

`use magpie.merge` is the ninth kernel ported: the streaming merge,
batches and segments folded into segments of bounded size, one row group
per input held at a time, the latest version of each key kept. A batch
is written as the sequencer's flush writes one: `write_batch` gives each
row its position in the order it arrives and sorts the rows by the keys,
and `numbered_bytes` answers the bytes of rows already numbered.

```crow
use sys, frame, magpie.file, magpie.manifest, magpie.merge

dir = 'ex_merge'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
m = declare[dir, ['id'], [], []]
none = slice[['x'], 0, 0]
a = frame.new[id = [1, 2, 3], qty = [10, 20, 30], _pos = [0, 1, 2]]
pa = place[dir, batch_named[0], file_of[a, 1024, 0, 2, ['id'], none, none, false]]
pb = write_batch[dir, 1, 3, frame.new[id = [4, 2], qty = [40, 25]], ['id'], none]
log[pb]
b = frame.new[id = [2, 4], qty = [25, 40], _pos = [4, 3]]
log[same[file[pb], numbered_bytes[dir, b, 3, ['id'], none]]]
seg = merge_files[dir, [pa, pb], false]
log[seg]
back = listed[dir]
log[back.segments.name]
log[back.segments.rows]
log[key_ranges[seg, ['id']]]
r = opened[seg]
log[group_of[r, 0, wanted[r, none]].qty]
cut = merge_into[dir, [pa, pb], ['id'], none, none, false, true, 7, 48, 120]
log[cut.counter]
log[cut.written[0].name]
log[cut.written[0].rows]
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

`use magpie.exec` is the executor in Crow: `summary_of`, an aggregate
over a log's files pushed below the reader — each group pruned by its
statistics, its rows masked and grouped, the partials merged in order —
and `asof_rows`, each label's newest row at or before its moment.

```crow
use sys, frame, magpie.file, magpie.merge, magpie.exec

dir = 'ex_exec'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
a = frame.new[sym = ['a', 'b', 'a'], px = [1.5, 2.0, 2.5], ts = [10, 20, 30], _pos = [0, 1, 2]]
p = place[dir, '000000000000.batch', file_of[a, 1024, 0, 2, ['sym'], none, none, false]]
s = summary_of[[p], ['sym'], ['px', 'px'], ['mean', 'max'], none, none, none, '', '', -1]
log[s.sym]
log[s.px_mean]
log[s.px_max]
labels = frame.new[sym = ['a', 'b'], ts = [25, 25]]
j = asof_rows[[p], labels, ['sym'], 'ts', none, -1]
log[j.columns]
log[j.px]
```

```text
['a', 'b']
[2, 2]
[2.5, 2]
['sym', 'ts', 'px']
[1.5, 2]
```

`use magpie.nearest` finds the rows nearest a query vector by the
cosine, every present row scored and the best k kept, an earlier
position first between equals.

```crow
use sys, frame, magpie.file, magpie.merge, magpie.nearest

dir = 'ex_nearest'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
v = frame.new[id = [1, 2, 3], e = [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]], _pos = [0, 1, 2]]
p = place[dir, '000000000000.batch', file_of[v, 1024, 0, 2, ['id'], none, none, false]]
hits = nearest_rows[[p], 'e', [1.0, 0.0], 2]
log[hits.id]
log[hits.score]
```

```text
[1, 3]
[1, 0.7071067811865475]
```

`use magpie.search` is BM25 as Lucene scores it over a log as one
corpus: a word, a phrase, a prefix (`kit*`), a fuzzy word (`kitchn~`),
from a group's index where it has one and its text where it has none,
with a snippet around the first hit.

```crow
use sys, frame, magpie.file, magpie.merge, magpie.search

dir = 'ex_search'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
d = frame.new[id = [1, 2, 3], note = ['a crate of apples', 'pears for the kitchen table', 'the kitchen table holds apples'], _pos = [0, 1, 2]]
p = place[dir, '000000000000.batch', file_of[d, 1024, 0, 2, ['id'], ['note'], none, false]]
hits = search_rows[[p], 'note', ['kitchen table'], 3, false, 10]
log[hits.id]
log[hits.snippet]
log[len[terms_of[['kit*', 'kitchn~1', 'apples'], false]]]
log[snippet_of['a crate of apples for the shop', terms_of[['apples'], false], 4, false]]
```

```text
[2, 3]
['…s for the kitchen table', 'the kitchen table holds app…']
3
… of apples for…
```

`use magpie.inflate`, `use magpie.snappy` and `use magpie.records` are
the Kafka door's bytes in Crow: a gzip member or a raw deflate stream
inflated, a snappy block decoded, and a legacy message set read into a
frame and written from one, each message's CRC-32 checked.

```crow
use sys, frame, magpie.inflate, magpie.snappy, magpie.records

data = bytes['hello hello hello hello']
log[text[gunzipped[gzip_stored[data]]]]
log[crc32[bytes['123456789']]]
log[text[unsnapped[snappy_literal[data]]]]
raw = bytes[0]
for b in [203, 72, 205, 201, 201, 87, 200, 64, 39, 1]
    raw = push[raw, int[b]]
log[text[inflated[raw]]]
f = frame.new[offset = [0, 1], at = [5, 6], key = ['a', 'b'], value = ['one', 'two']]
set_bytes = message_set[f]
log[len[set_bytes]]
log[message_rows[set_bytes].value]
```

```text
hello hello hello hello
3421780262
hello hello hello hello
hello hello hello hello
76
['one', 'two']
```

`use magpie.tail` is the sequencer in Crow: the rows put since the last
flush, numbered as they arrive and held for the process in its memory
with each key's last position and newest row, flushed as one batch file
by the work a put asks for — what this process reads at once, and
another once flushed.

```crow
use sys, frame, magpie.manifest, magpie.tail

dir = 'ex_tail'
if exists[dir]
    gone = rm[dir]
made = mkdir[dir]
none = slice[['x'], 0, 0]
m = declare[dir, ['id'], none, none]
log[put_rows[dir, frame.new[id = [1, 2], qty = [10, 20]]]]
log[put_rows[dir, frame.new[id = [2], qty = [25]]]]
log[last_position[dir, ['2']]]
log[newest_row[dir, ['2']].qty]
done = flush_rows[dir]
log[pending_of[dir]]
log[len[pending_rows[dir, 2, 2]] gt 0]
log[pending_at['ex_tail/.pending@0-0'].ok]
log[forget_log[dir]]
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
