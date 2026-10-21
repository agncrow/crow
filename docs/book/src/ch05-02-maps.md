# Maps Are Dictionaries

A map is a lookup table: keys of one kind — text, or ints
(`map[1 = 'a']`) — and values of **one** type. Build one with the
`map[...]` constructor; read with dot or bracket; `set` returns an
updated copy:

```crow
scores = map[alice = 10, bob = 7]
log[scores.alice]
log[scores['bob']]

s2 = set[scores, 'carol', 9]
log[s2]
log[keys[s2]]
log['alice' in scores]
```

```text
10
7
[alice = 10, bob = 7, carol = 9]
['alice', 'bob', 'carol']
true
```

The one-type rule is enforced — a map of ints won't accept a string. If
you're tempted to mix types under named keys, what you want is not a map.
It's the next section.
