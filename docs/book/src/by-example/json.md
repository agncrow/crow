# JSON

```crow
data = json['[1, 2, 3]']
log[data[0]]
obj = json['{"name": "crow", "n": 3}']
log[obj.name]
log[int[obj.n] + 1]
log[encode[[1, 2, 3]]]
bad = json['not json']
log[bad.ok]
log[bad.kind]
```

```text
1
crow
4
[1,2,3]
false
parse
```

Strings are inert outside backtick interpolation, so JSON braces and
double quotes need no escaping. A parsed field is open-schema data: convert
it with `int[]`, `float[]`, or `str[]` before arithmetic — the conversion
is an error value if the field isn't what you expected.

## Text for a header or a URL

```crow
pair = 'ada:secret'
coded = encode[pair, 'base64']
log['Basic `coded`']
log[decode[coded, 'base64']]
log[encode['crow & more', 'url']]
bad = decode['not base64!', 'base64']
log[bad.kind]
```

```text
Basic YWRhOnNlY3JldA==
ada:secret
crow%20%26%20more
parse
```

Given a scheme, `encode` writes text so something else can carry it —
base64 for an `authorization: Basic …` header, `'url'` for one piece of a
URL — and `decode` reads it back. `request` and the readers encode a query
and a form themselves.

Reference: spec section 11 (JSON), and Encodings.
