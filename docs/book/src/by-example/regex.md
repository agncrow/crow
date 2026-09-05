# Regular expressions

```crow
log[matches['crow-2026', '^[a-z]+-[0-9]+$']]
m = find['2026-09-04', '([0-9]+)-([0-9]+)-([0-9]+)']
log[m.ok]
log[m.groups[2]]
log[sub['a1b22c333', '[0-9]+', '#']]
```

```text
true
true
09
a#b#c#
```

Reference: spec section 11 (Regex).
