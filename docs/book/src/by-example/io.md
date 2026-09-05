# Structured files: `use io`

```crow
use io

write['ex_people.csv', 'name,age\nann,30\nbob,25\n']
rows = csv['ex_people.csv']
log[len[rows]]
log[rows[1].name]
ls_lines = lines['ex_people.csv']
log[len[ls_lines]]
log[ls_lines[0]]
rm['ex_people.csv']
```

```text
2
bob
3
name,age
```

`csv[]` gives an array of maps (every cell a string) — handy for small
config-shaped files. For real tables use `frame.load` (chapter 10), which
types the columns and knows how to filter, sort, and group.

Reference: spec section 13 (io).
