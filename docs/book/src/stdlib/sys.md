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
| `hash[bytes, algo]` | The digest as lowercase hex: `'sha256'`, `'sha512'`, `'sha1'` or `'md5'` | [The standard library's layer: `use sys`](../by-example/sys.md) · [The system: shell, environment, time, arguments](../by-example/system.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `digest[bytes, algo]` | The digest as bytes: `'sha1'` (a socket's accept key), `'sha256'`, `'sha512'` or `'md5'` | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `hmac[key, message, algo]` | The keyed digest as bytes: `'sha256'` (an S3 signature's chain) or `'sha512'` | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `reply[status, body, headers]` | A request's answer from its parts: the record `request` answers, made by the client written in Crow | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `answer[status, phrase, headers, body]` | A server's answer from its parts, its body as bytes: the record `exchange` and `fetch` answer — `status`, `phrase`, `headers` (names in lower case) and `body` | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `csv_rows[text]` | CSV text as rows, each a map from its column's name to its text; an error value of kind `'io'` where it does not parse | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `frame_of_csv[text]`, `frame_of_json[text]`, `frame_of_parquet[bytes]` | A table as a frame, from CSV text, JSON records, or Parquet bytes; an error value of kind `'frame'` where it does not parse | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `frame_csv[frame]`, `frame_json[frame]`, `frame_parquet[frame]` | A frame as CSV text, as JSON records, or as Parquet bytes | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `tensor_of_frame[frame]` | A frame's cells as a matrix, a row each; an error value of kind `'load'` where a cell is no number | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `base64[bytes]` | The bytes as base64, the standard alphabet padded with `=` (HTTP's Basic authorization, SMTP's AUTH PLAIN) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `base64url[bytes]` | The bytes as base64 in the URL-safe alphabet, unpadded, as a token's parts are written | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `unbase64[text]` | Base64's bytes, either alphabet, padded or not; an error value of kind `'parse'` where the text is no base64 | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `same[a, b]` | Whether two byte strings are equal, decided in time that does not depend on where they differ (a signature checked against the one computed) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `pack[n, width]` | An int as `width` bytes, big-endian — 1, 2, 4 or 8, the low bytes of its two's complement; another width stops | [The standard library's layer: `use sys`](../by-example/sys.md) · [Web: responses and submodules](../by-example/web-more.md) |
| `ieee[x]` | A number as the 8 bytes of its IEEE 754 double, big-endian | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `binary[bytes, media]` | A response of the bytes, as the media type names them (`msgpack`'s) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `mask[bytes, key]` | The bytes XORed with the 4-byte key, cycling: a client's frame masked, a masked frame read | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `read_head[stream, deadline]` | A connection's answer's head read to its blank line — the status and phrase, the headers — and, as the answer's `body`, what came after it (a body's start, a socket's first frame); written in Crow | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `upgrade[url, timeout]` | A connection to a `ws://` or `wss://` URL's host upgraded to a WebSocket as RFC 6455 says — the key sent, the server's accept checked — or the reason; the socket holds what came after the head; written in Crow | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `socket_of[stream, url, unread]`, `stream_of[socket]` | A socket over an upgraded stream, holding bytes unread; the stream a socket talks over | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `leftover[socket]`, `unread[socket, bytes]` | The bytes a socket holds unread, taken; bytes put back for its next read (what a read took past a frame's end) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `remote[path]` | Whether a path names a remote source: a URL (`http://`, `https://`) or an S3 object (`s3://`) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `exchange[url, method, lines, body, timeout]` | One HTTP exchange, written in Crow: the request sent to the URL's host with its method, the header lines and the body, its answer read whole — an `answer`, or an error value of kind `'sys'` where none came in `timeout` seconds (0 as thirty to connect and ten minutes in all) | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `redirected[from, location]` | Where a redirect sends the request: its `location`, made absolute against the URL it came from | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `with_query[url, pairs]` | The URL with the pairs added, each name and value URL-encoded, after the `?` it has or one it is given | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `fetch[path, headers, query, timeout]` | A remote source's answer, written in Crow: a URL got with a reader's options and its redirects followed, or an S3 object got signed — a success, or the reason as an error value of kind `'sys'` (the options on a path that is no URL, a timeout below 0, no answer, an answer that is no success) | [The standard library's layer: `use sys`](../by-example/sys.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) |
| `put[path, bytes]` | The bytes written to an S3 object, signed; a URL is refused — it is read, not written | [The standard library's layer: `use sys`](../by-example/sys.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Programming a Guessing Game](../ch02-00-guessing-game.md) |
| `format_of[path]` | A table's format by its path's extension (a URL's query left out): `'csv'`, `'json'`, `'parquet'`, or `''` | [The standard library's layer: `use sys`](../by-example/sys.md) |
| `format_sniffed[media, bytes]` | The format of what a URL with no extension answered: the media type its server named, then the bytes themselves (Parquet's `PAR1`, JSON's opening bracket), and CSV otherwise | [The standard library's layer: `use sys`](../by-example/sys.md) |

A stream is one connection however many values hold it, as a socket is; `type[s]` is `stream`. Every function works identically in both modes (`crow_runtime::sys`). The HTTP protocol is written on this layer, in `src/modules/sys.co`, as rows of `sys` itself: a URL taken apart, a connection opened plain or behind TLS, the request line, headers and body sent, the answer's head read to its blank line and its body by its length, by its chunks, or until the server closes (`exchange`), redirects followed and a reader's options applied (`fetch`), an S3 object signed as AWS's Signature Version 4 says and read or written by path at an endpoint (`fetch`, `put`). On the protocol stand the HTTP client — `request` (Requests), in `src/modules/core.co` — and every reader of a URL or an S3 object: `read` and `write` (core.co), `use io`'s `csv` and `lines` (io.co), `use frame`'s `load` and `save` (frame.co) and `use math`'s `load` (math.co), each a few lines over the layer's parsers (`csv_rows`, `frame_of_csv`, `tensor_of_frame`). A program never sees the layer: it sees `request` and `read`, whose rows in the builtin table are the contracts the Crow bodies keep (math.nn, Design Note "a builtin written in Crow keeps its row").

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
