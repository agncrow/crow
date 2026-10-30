# The standard library's layer: `use sys`

`sys` is the layer the standard library's own modules are written on —
bytes, files, and a connection to a host — and so is a package that needs
a kernel's speed. It is meant for library code: a program serves with
`serve`, reads with `request`, `read` and `socket`, and connects with
`connect`, which are built on it. Keeping it to library code is a
suggestion, not a rule — any program or package may import it.

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
the Magpie package's Kafka door is, stands on these.

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

A text's words as a search index and a query split them — each run of
letters and digits, lowercased (`terms`) — and a column of text from a
dictionary: its distinct texts once, end to end, and a code per row
(`text_dict_column`).

```crow
use sys

log[terms['The quick, brown Fox-42 jumps; über naïve 東京!']]
codes = []
for c in [0, 1, 0]
    codes = push[codes, int[c]]
few = text_dict_column[bytes['ab'], range[1, 3], codes, bytes[0]]
log[text_ends_of[few, 0, 3]]
log[cell_kind[few]]
```

```text
['the', 'quick', 'brown', 'fox', '42', 'jumps', 'über', 'naïve', '東京']
[1, 2, 3]
text
```

Magpie's parts — its codec, bit packing, column chunks, filters, file,
manifest, merge, executor, search and sequencer — are written on these,
in Crow, and are the Magpie package's, a repository of its own (its
PARTS.md shows each at work, its example runs every one in both modes).
