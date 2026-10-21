# Introduction

Crow is a programming language for people who work with data and ship what
they find. It looks like Python, compiles to Rust, and makes three promises:

1. **Parallel by default.** A `for` loop runs its iterations concurrently.
   You never write threads, async, or locks — you write loops.
2. **Errors are values.** No exceptions, no `try`/`catch`, no null. A failed
   operation returns a value that carries its own story (`.ok`, `.reason`),
   propagates safely, and refuses to be silently misread.
3. **One way to do things.** One call syntax (`f[x]`), one string quote
   (`'`), one loop, one way to build a map. When Crow rejects a feature you
   expect, a Design Note in the specification explains why — and the compile
   error tells you what to write instead.

The language ships as a single binary containing an interpreter (for
immediate feedback), a compiler to Rust (for production speed; the Rust
toolchain it needs is fetched once, on the first native build), tensors,
data frames, a database layer, and a reactive web framework. The final
chapter of this book builds a live dashboard using all of them. The same
binary talks to language models: a model is a connection, a program's
own functions are its tools, and an agent loop runs them (`use llm`,
`use mcp`; Crow by Example shows both).

Crow is in alpha: the language still changes between releases, the
changelog in the repository says what moved, and this book is rebuilt
for every change, so what you read matches the release you install.

## Who This Book Is For

You've written some Python, R, MATLAB, or JavaScript. You don't need to
know Rust — Crow compiles *to* Rust, you never read the output.

## How to Read It

Two parts, one language. Part I is the book: read it in order. Getting
Started installs Crow and meets the toolchain, the guessing game builds a
small program before any theory, and the chapters that follow build the
language up one idea at a time — concepts, functions and scope,
collections, parallelism, errors, modules, testing — each using only what
came before it. Then the applied layers: a hands-on data project,
databases, web UIs, and a final project that uses everything. Crow by
Example closes Part I with one runnable program per area of the language.
Whatever language you're arriving from — Python, JavaScript, R, MATLAB,
Julia, Go, Rust, Elixir, Ruby, Swift, Zig — Appendix B lists the habits to
unlearn, and Appendix A is the language on one page.

Part II is the reference: the standard library module by module, every
function linked to the example that shows it, and the language
specification — the source of truth (`docs/language-spec.md` in the
repository), with a Design Note for every feature Crow deliberately
leaves out. Consult it whenever the book says "the spec covers the rest",
and read its Design Notes whenever you wonder "why can't I…". The book
teaches; the spec specifies.

## Every Block Runs

Every code block in this book is executed by `crow test --book` in both of
Crow's execution modes on every change: what you read is what runs. And
every block can be run where it stands. From a checkout of the repository
run `crow book` and open the printed address: each block gets ▶ Run (the
interpreter), ⚙ Compile (the native binary), ✓ Test (its test blocks),
↺ Reset, and ■ Stop. The block itself is editable — change it and run
again. The same binary that verifies the book serves it.
