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

Reference: spec section 11 (JSON).
