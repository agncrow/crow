# Crow

A programming language for data work and web applications. Looks like
Python, compiles to Rust, runs parallel by default, treats errors as values.

```crow
use math

prices = [101.5, 102.1, 99.8, 103.4]
avg = mean[prices]

returns = for p in prices
    (p - avg) / avg

log['average: `fmt[avg, decimals = 2]`']
```

- **Parallel by default** — `for` loops run their iterations concurrently; no threads, no async
- **Errors are values** — no exceptions, no null; failures carry `.ok` / `.reason` and refuse to be silently misread
- **One way to do things** — one call syntax `f[x]`, one string quote, one loop; every rejected feature has a documented reason
- **Batteries included** — tensors, data frames, SQL, and a reactive web framework (LiveView-style `view` blocks) in one binary

## Install

```bash
cargo install --path .
crow new hello && cd hello && crow run
```

## Learn

- **[The Crow Book](docs/book/src/SUMMARY.md)** — step-by-step, from install to a live dashboard. Every code block is executed in both execution modes by CI (`crow test --book`).
- **[Toolchain guide](docs/usage.md)** — every `crow` command, project layout, the crowfile.
- **Run the book's examples in place** — `crow book` serves the book with ▶ Run / ⚙ Compile / ✓ Test / ■ Stop on every code block.
- **Learning with an AI assistant?** Paste [`docs/crow-for-llms.md`](docs/crow-for-llms.md) into it and ask it to teach you interactively — the file is written to make any LLM an accurate Crow tutor.
- **[Language specification](docs/language-spec.md)** — the reference, including the Design Notes explaining every deliberately rejected feature.
- Runnable programs in [`examples/`](examples/) (indexed in its README), and the full documentation map in [`docs/README.md`](docs/README.md).

## Two execution modes, one behavior

`crow run -i file.crow` interprets instantly; `crow run file.crow`
transpiles to Rust and runs a native binary. The two modes are held to
identical output on a growing parity corpus (`crow test --parity`).
Interpret while exploring, compile when shipping.
