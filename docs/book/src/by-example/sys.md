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

Reference: spec section 13 (sys).
