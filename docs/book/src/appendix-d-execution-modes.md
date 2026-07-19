# Appendix D: The Two Execution Modes

Every Crow program runs two ways from the same source:

| | `crow run -i file.crow` | `crow run file.crow` |
|---|---|---|
| How | tree-walking interpreter | transpile → Rust → native binary |
| Startup | instant | first build compiles (cached after) |
| Speed | scripting-tier | Rust-tier |
| Use for | exploring, REPL, quick scripts | anything you ship or benchmark |

`crow build` produces the binary without running it.

**The parity promise:** both modes are required to produce identical
output. This is enforced mechanically — `crow test --parity` runs a
corpus of programs through both modes and diffs stdout, and every code
block in this book passes the same check (`crow test --book`). If you
ever find the modes disagreeing, it's a bug in Crow, not a semantics
you need to learn.

The shared-semantics design also means error behavior matches: both
modes reject the same invalid programs with the same messages, from the
same validation pipeline.
