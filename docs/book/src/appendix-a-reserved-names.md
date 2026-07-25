# Appendix A: Reserved Names

A definition (variable, function, view, struct, parameter, loop variable)
may not take a name the language or an imported module owns. The compiler
error always includes the fix. Full rationale: chapter 8 and the spec's
Design Note "Names Are Never Ambiguous".

## Always reserved (core, frozen)

```text
log assert type
abs sqrt floor ceil round min max
split trim replace upper lower join starts ends repeat fmt
int float str
len contains slice push pop sort reverse concat range map
keys values has set del
args input read write exists mkdir ls rm mv cp
rand time sleep env exit json encode error shell hash later running
find matches sub any all now date elapsed cpu gpu nodes grad
```

## Reserved per import (bare imports only)

- `use math` — its ~86 exports (`sum`, `mean`, `std`, `zeros`, `eye`,
  `linspace`, …) and the constants `pi`, `e`, `inf`, `nan`
- `use frame` — `load`, `save`, `select`, `filter`, `sort`, `group`,
  `join`, `stack`, `head`, `tail`, `stats`, `fill`, `rename`, …
- `use db` — `connect`, `query`, `exec`, `insert`, `transact`, `migrate`
- `use web` — `route`, `serve`, `page`, `respond`, `redirect`, `html`,
  `text`, `send`, `recv`, `close`, …
- `use ui` — `fetch`, `poll`, `field`, `today` (element names like `div`
  and `p` are NOT reserved — they live only inside `view` blocks)

An aliased import (`use math as m`) reserves only the alias.

## Never reserved

Struct fields, map keys, JSON fields, and frame columns — `row.type`,
`account.sum` are always fine. Exception: `ok`, `reason`, `kind`,
`trace` cannot be struct field names (they're error metadata on every
value).
