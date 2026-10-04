# sys

The standard library's own layer: bytes, and a connection to a host, for the modules written in Crow — a program may not import it.


Import with `use sys` — **only in a module of the standard library.** `sys` is the layer the standard library's own Crow-written modules stand on: bytes, and a connection to a host read and written as bytes. A program may not import it; a program serves with `serve[…]`, reads with `request[…]`, `read[…]` and `socket[…]`, and connects with `connect[…]`, which are built on this layer (design rule 20). In a program, `use sys` is refused before it runs: `` `sys` is the standard library's own layer — a program serves with serve[…], reads with request[…], read[…] and socket[…], and connects with connect[…] ``. The book's page shows the layer through the harness's door (`CROW_SYS=1`), which the tests and the book have and a program has not.

| Function | Description | Example |
|----------|-------------|---|
| `bytes[text]` | The text's UTF-8 as bytes. A bytes value shows as `<n bytes>`, has a `len`, and compares with `eq` and `ne` to another; `type[b]` is `bytes` | [The standard library's layer: `use sys`](../by-example/sys.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `text[bytes]` | The text the bytes spell in UTF-8; an error value of kind `'parse'` where they spell none | [The standard library's layer: `use sys`](../by-example/sys.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `slice[bytes, from, to]` | The bytes from `from` to `to` (excluded), counted from the end when negative; a span outside stops | [Arrays](../by-example/arrays.md) · [The standard library's layer: `use sys`](../by-example/sys.md) · [Strings and Interpolation](../ch03-03-strings.md) |
| `find[bytes, needle]` | Where the needle first occurs, or -1 | [Regular expressions](../by-example/regex.md) · [The standard library's layer: `use sys`](../by-example/sys.md) · [Searching with any, all, and find](../ch06-02-search-builtins.md) |
| `concat[a, b]` | The two in order | [Arrays](../by-example/arrays.md) · [Math: tensors](../by-example/math.md) · [The standard library's layer: `use sys`](../by-example/sys.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |
| `byte[bytes, i]` | The byte at `i` as an int, counted from the end when negative; outside stops | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `open[host, port, tls, seconds]` | A connection to the host's port, behind TLS when `tls` is true (the system's roots); each read and write gives up after `seconds` (none when 0). An error value of kind `'sys'` when none can be made: a host that does not resolve, a port nobody listens on, a certificate that does not fit | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `send[stream, bytes]` | The bytes written, all of them; the count. An error value of kind `'sys'` on a stream that is closed or broke | [The standard library's layer: `use sys`](../by-example/sys.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `recv[stream, most, seconds]` | Up to `most` bytes as they arrive — none when the other side closed — waiting at most `seconds` (the connection's own wait when 0). An error value of kind `'sys'`: `no data from host:port in 2 s`, `the stream is closed` | [Web: responses and submodules](../by-example/web-more.md) |
| `close[stream]` | The connection let go; every call after is refused | [Web: responses and submodules](../by-example/web-more.md) |
| `gather[chunks]` | A list of bytes joined in order, as one (what a connection read, piece by piece) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `file[path]` | A file's bytes, as they are; an error value of kind `'sys'` where it cannot be read | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `store[path, bytes]` | The bytes written to a file, as they are; an error value of kind `'sys'` where it cannot be written | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `random[n]` | `n` bytes from the operating system's randomness | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `hex[bytes]` | The bytes as lowercase hex, two digits each | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `text[bytes, fill]` | The text the bytes spell, each run that spells none replaced by `fill` (`'�'`, as a browser shows one) — never an error | [The standard library's layer: `use sys`](../by-example/sys.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `hash[bytes, algo]` | The digest as lowercase hex: `'sha256'`, `'sha512'` or `'md5'` | [The standard library's layer: `use sys`](../by-example/sys.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `hmac[key, message, algo]` | The keyed digest as bytes: `'sha256'` (an S3 signature's chain) or `'sha512'` | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `reply[status, body, headers]` | A request's answer from its parts: the record `request` answers, made by the client written in Crow | [The standard library's layer: `use sys`](../by-example/sys.md) |

A stream is one connection however many values hold it, as a socket is; `type[s]` is `stream`. Every function works identically in both modes (`crow_runtime::sys`). The HTTP client — `request` (Requests) — is written on this layer, in `src/modules/core.co`: a URL taken apart, a connection opened plain or behind TLS, the request line, headers and body sent, the answer's head read to its blank line and its body by its length, by its chunks, or until the server closes, redirects followed, the answer made by `reply`. A program never sees the layer: it sees `request`, whose row in the builtin table is the contract the Crow body keeps (math.nn, Design Note "a builtin written in Crow keeps its row").

```
use sys

b = bytes['héllo']
log[len[b]]                      --- 6 (two bytes for é)
log[text[slice[b, 3, 6]]]        --- llo
log[find[b, bytes['llo']]]       --- 3
bad = text[slice[b, 1, 2]]       --- half of é: not text
log[bad.kind]                    --- parse
s = open['127.0.0.1', 1, false, 0.2]
log[s.kind]                      --- sys: nobody listens on port 1
```

**Design Note: the standard library's layer.** Considered: writing the standard library's modules in Rust only, as they were (rejected: a module both backends share is a bug both share, which no parity or fuzz run can see; the week of 2026-10-03 found eleven bugs where the two backends differ and none in the kernels they share, because nothing can look there); giving every program bytes, sockets and threads (rejected: a program would gain the bug classes the language exists to exclude — data races, half-read buffers, protocol state spread over user code — and a second way to serve and connect beside `serve` and `connect`). Decided (Vijay, 2026-10-03): one `sys` module, the smallest native layer the rest of the library can be written on, importable by the compiler's own modules alone; a program sees one way, and every module written on `sys` is compiled and run twice, fuzzed, and takes laws.


Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
