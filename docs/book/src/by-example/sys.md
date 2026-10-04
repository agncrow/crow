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

The rest of the layer, for the modules written on it — a file's bytes, chunks gathered, digests, and a request's answer made from its parts:

```crow
use sys

store['sys-note.txt', bytes['one\ntwo']]
log[len[file['sys-note.txt']]]
log[text[gather[[bytes['ab'], bytes['cd']]], '?']]
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

The protocol itself reaches a host — one exchange, a reader's fetch with its options and redirects, an S3 object put signed with the environment's credentials — so these lines are not run here:

```crow,norun
use sys

got = exchange['https://example.com/', 'GET', ['accept: text/html'], bytes[''], 5.0]
log[got.status]
page = fetch['https://example.com/data.csv', map[authorization = 'Bearer k'], map[day = '2026-07-14'], 10.0]
log[page.ok]
kept = put['s3://reports/aapl.parquet', bytes['PAR1']]
log[kept.ok]
```

Reference: spec section 13 (sys).
