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

Reference: spec section 13 (sys).
