# Hello, World!

Create a file named `hello.crow`:

```crow
log['hello, world']
```

Two things already differ from the languages you know: **calls use square
brackets** (`log[...]`, never `log(...)`) and **strings use single
quotes**. These are the two most common reflexes to retrain, and the
compiler will remind you if you slip.

Run it compiled — Crow transpiles to Rust, builds a native binary, and
executes it:

```bash
crow run hello.crow
```

```text
hello, world
```

Or run it interpreted for instant startup while you're iterating:

```bash
crow run -i hello.crow
```

The two modes are guaranteed to agree — the project's test suite diffs
their outputs on a growing corpus of programs, including every code block
in this book. Interpret while you explore; compile when you ship. That's
the whole workflow.

One more taste of what's ahead — interpolation uses paired backticks, and
a `for` loop is an expression that runs in parallel:

```crow
name = 'crow'
log['hello, `name`']

squares = for x in [1, 2, 3, 4]
    x * x
log[squares]
```

```text
hello, crow
[1, 4, 9, 16]
```
