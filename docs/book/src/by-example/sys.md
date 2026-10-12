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

The rest of the layer, for the modules written on it — a file's bytes and its lines read a block at a time, chunks gathered, digests, and a request's answer made from its parts:

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

Bytes as text and back, a signature compared, a number's bytes, and a response of bytes — what a token, a frame's header and a packed value are written over — and the request fixtures `request` asks for before it sends (`replaying`, `recording`, `replayed`, `recorded`: no replay and no recording run here, so nothing is held and nothing kept):

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
