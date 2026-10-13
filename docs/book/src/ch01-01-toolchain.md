# The Toolchain

One binary, a handful of verbs. Every one of them runs the same compiler
pipeline the editor uses, so nothing you see in one place disagrees with
another.

```text
crow new name [--web --ml --db]  scaffold a project (a Crowfile, main.co, css/)
crow run [-i] [file]             execute: compiled when the compiler is here, else interpreted and told; -i interprets
crow setup [--check]             fetch the Rust toolchain and the runtime for compiled builds, once; --check says what is present
crow run --opt N file            the program at optimization level N (0 to 3; 0 unless said, crow build is 3 with LTO)
crow stop                        stop the served app started by crow run
crow build [file]                compile to a native binary, ./main
crow check [file]                verify: tests, generated edge cases, contracts, and the app baseline — what changed since the accepted one
crow fix [file]                  apply the fixes the error messages name, and those crow check offers
crow fmt                         format
crow test [file]                 run the test blocks
crow test --parity | --book      the two execution modes must agree
crow test --docs [--write]       the documentation must cover the compiler
crow book [--port N]             render this book and serve it, every block runnable
crow lsp                         the language server (your editor runs it)
crow launch N                    distributed workers
```

Two execution modes, one behaviour. `crow run -i` interprets: instant
start, the mode to use while you explore. `crow run` transpiles to Rust
and builds a native binary: the mode to ship. The binary you installed
holds the interpreter and needs nothing else; the compiler is a Rust
toolchain that `crow setup`, or the first `crow build`, fetches into
`~/.crow` once, and until it is there `crow run` says so and interprets. They are held to identical
output by `crow test --parity` over a corpus of programs and by
`crow test --book` over every code block in this book. Interpret while you
explore; compile when you ship. That is the whole workflow.

A project's Crowfile names the entry point and the resources: keys
`name`, `entry`, `db`, `cpu`, `gpu`, `deps`, `jobs`. The next page makes
one.
