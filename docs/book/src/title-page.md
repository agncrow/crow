# The Crow Programming Language

*by the Crow project*

Two documents, one language:

- **Part I, the book** — teaches Crow from the ground up: a language for
  data work and web applications that compiles to Rust, runs parallel by
  default, and treats errors as values. Read it in order. Every code block
  is executed by `crow test --book` in both of Crow's execution modes on
  every change: what you read is what runs.
- **Part II, the reference** — [the standard library](stdlib/index.md)
  module by module, [where every name is shown](where-is-it.md), and the
  [language specification](language-spec.md), the source of truth
  (`docs/language-spec.md` in the repository): every construct, every
  builtin, and a Design Note for every feature Crow deliberately leaves out. Consult it whenever the book says
  "the spec covers the rest", and read its Design Notes whenever you
  wonder "why can't I…".

The book teaches; the spec specifies.

Every code block on this site can be run where it stands. From a checkout
of the repository run `crow book` and open the printed address: each block
gets ▶ Run (interpreter), ⚙ Compile (native), ✓ Test (its test blocks),
↺ Reset (the block itself is editable — change it and run again), and ■ Stop — the same binary that
verifies the book serves it.
