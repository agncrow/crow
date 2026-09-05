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
immediate feedback), a Rust transpiler (for production speed), tensors,
data frames, a database layer, and a reactive web framework. The final
chapter of this book builds a live dashboard using all of them.

## Who This Book Is For

You've written some Python, R, MATLAB, or JavaScript. You don't need to
know Rust — Crow compiles *to* Rust, you never read the output.

## How to Read It

Chapter 1 installs Crow and chapter 2 builds a small game before any
theory. Chapters 3–9 then build the language up one idea at a time —
concepts, functions and scope, collections, parallelism, errors, modules,
testing — each chapter using only what came before it. Chapter 10 is a
hands-on data project, and chapters 11–13 cover the applied layers:
databases, web UIs, and the final project that uses everything. Whatever
language you're arriving from — Python, JavaScript, R, MATLAB, Julia, Go,
Rust, Elixir, Ruby, Swift, Zig — Appendix B lists the habits to unlearn.
Appendix A is the whole language on one page — a good bookmark once the
chapters are behind you — and Part II is the full language specification,
the document to keep open while you write real programs.
