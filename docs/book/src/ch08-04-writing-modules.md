# Writing Your Own Modules

A module is a `.crow` file of functions, structs, and constants. Your
modules are always **qualified** — importing one reserves a single bare
name (the prefix), and everything inside is reached through it:

```crow,ignore
--- geometry.crow ---
struct Circle [r]

fn circle_area[c]
    return 3.14159 * c.r * c.r
```

```crow,ignore
--- main.crow ---
use geometry

c = geometry.Circle[2.0]
log[geometry.circle_area[c]]
```

Subdirectories map through slashes (`use myapp/utils` → `utils.helper[...]`),
dots also work for view files (`use views.home`), and aliases apply as
everywhere (`use geometry as g` → `g.circle_area[...]`).

The asymmetry with the standard library is deliberate: **the bare
namespace belongs to the language.** Stdlib names are hand-curated and
frozen-or-slow-moving; your module can export a thousand names and take
none of anyone's vocabulary — the import costs exactly one word, the
prefix. That's also why two modules can never collide: same trailing
name, different aliases, done.

Next: put the last five chapters to work on real data.
