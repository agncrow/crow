# Names Are Never Ambiguous

The rule behind everything in this chapter: a definition may not take a
name the language or an import already owns. Not shadowing, not
precedence — an error, at the definition line, with the fix included.

```crow,error
--- error: 'range' is a core builtin — pick another name for this variable
range = [10, 20, 30]
```

```crow,error
use math
--- error: 'sum' is provided by math (imported on line 1) — rename the
--- variable, or import qualified: use math as m
sum = 0
```

Why so strict? Because Crow's brackets serve both calling and indexing.
In a language where `range[0]` could mean *index my variable* or *call
the builtin*, silent shadowing produces silently wrong answers — delete
the variable during a refactor and the same expression changes meaning
without an error. Crow refuses the ambiguity at the source.

What this costs you: an occasional rename (`total` instead of `sum`,
`kind` instead of `type`) — the error always suggests one, and the alias
form is there when you'd rather keep your word. What it buys: any bare
name in any Crow file means one thing, for you, for readers, and for
every AI assistant generating Crow.

Four softenings keep it livable:

- **Core is frozen** — the ~70 always-reserved names never grow, so code
  that compiles today can't be broken by a future builtin.
- **Fields are exempt** — `row.type`, `account.sum`, `df.mean` are always
  fine; reservation governs bare definitions, not the insides of your
  records. (Only `ok`/`reason`/`kind`/`trace` are off-limits as struct
  fields — every value already carries those.)
- **Modules reserve only where imported** — no `use math`, no reserved
  `sum`.
- **Inside a view, the element wins.** With `use frame, math` loaded,
  `stack[...]` in a view is the layout container, not `frame.stack` or
  `math.stack`; the prefix (`frame.stack[a, b]`) still reaches the module
  version from inside a view.
