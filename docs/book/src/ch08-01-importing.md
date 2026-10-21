# Importing with use

`use` brings a standard-library module's vocabulary into your file, bare:

```crow
use math

xs = [3.0, 1.0, 4.0, 1.0, 5.0]
log[sum[xs]]
log[mean[xs]]
log[math.sum[xs]]
log[pi]
```

```text
14
2.8
14
3.141592653589793
```

After `use math`, its functions (`sum`, `mean`, `exp`, `zeros`, `transpose`,
…) and constants (`pi`, `e`, `inf`, `nan`) work without a prefix — and
the prefixed form `math.sum[...]` *always* works too, if you prefer to
read the source of a name at its callsite.

Import several modules in one line: `use web, ui`. Submodules opt in
separately: `use math.stats` adds `median` and friends without bloating
plain `use math`.

The standard modules are `math` (+ `.stats`, `.nn`), `frame`, `io`, `db`,
`doc`, `bits`, `chart`, `llm`, `mcp`, `web` (+ feature submodules), and
`ui`; the standard library's pages in Part II list each one's names. Only
these get the bare-name privilege — everything else you import lives
behind a prefix, as you'll see in the last section.
