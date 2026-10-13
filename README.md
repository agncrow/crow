<p align="center"><img src="assets/crow.svg?v=2" width="128" alt="Crow"></p>

# Crow

A programming language for data work and web applications. Looks like
Python, compiles to Rust, runs parallel by default, treats errors as values.

> **Alpha.** Crow is open source under the MIT or Apache-2.0 licence, at
> your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
> The language still changes between releases ([CHANGELOG](CHANGELOG.md));
> it takes no pull requests during the alpha, and bug reports are welcome
> in Issues ([CONTRIBUTING](CONTRIBUTING.md)).

```crow
use math

prices = [101.5, 102.1, 99.8, 103.4]
avg = mean[prices]

returns = for p in prices
    (p - avg) / avg

log['average: `fmt[avg, decimals = 2]`']
```

An operation that fails answers a value that says why, and the program
decides what to do with it:

```crow
prices = read['prices.csv']
if not prices.ok
    log['no prices today: `prices.reason`']
```

- **Parallel by default** — `for` loops run their iterations concurrently; no threads, no async
- **Errors are values** — no exceptions, no null; failures carry `.ok` / `.reason` and refuse to be silently misread
- **One way to do things** — one call syntax `f[x]`, one string quote, one loop; every rejected feature has a documented reason
- **Batteries included** — tensors, data frames, SQL, a reactive web framework (LiveView-style `view` blocks), and a language model as a connection (`use llm`), in one binary
- **Verified** — `crow check` runs your tests plus generated edge cases, replays recorded database and model answers, and diffs what the app does against the accepted baseline; the two execution modes are held to one behaviour on every block of the book
- **Held to references** — frames to pandas and models to scikit-learn to the last digit, optimisation to scipy step for step, retrieval to rank_bm25 and faiss, a GPT to nanoGPT; each package ships with the reference script it is held to

## Quick start

One line on a Mac or a Linux machine (on Windows, use WSL):

```bash
curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | bash
```

It puts one prebuilt binary in `~/.crow/bin`, checks it against its
published hash, runs a hello program, and prints the PATH line. Nothing
else is installed: the interpreter, the tests, `crow check`, the language
server and served apps work at once.

```bash
crow new hello && cd hello && crow run
```

Compiling to a native binary needs a Rust toolchain. The first `crow
build` fetches a pinned one into `~/.crow`, with the runtime, once (about
215 MB); `crow setup` does it up front, and `crow setup --check` says
what is present. Until then `crow run` says so and interprets, which
answers the same, only slower.

`crow help` lists every command. To upgrade, run the line again; to
remove Crow, delete `~/.crow` and the PATH line. Working on Crow itself?
`git clone https://github.com/agncrow/crow && cd crow && ./install.sh --dev`
builds your checkout in place; `--source` builds from it and snapshots the
source for compiled programs.

## Learn

- **[The Crow Book](https://agncrow.github.io/crow/)** ([source](docs/book/src/SUMMARY.md)) — step-by-step, from install to a live dashboard. Every code block is executed in both execution modes by CI (`crow test --book`).
- **[Toolchain guide](docs/usage.md)** — every `crow` command, project layout, the crowfile.
- **Run the book's examples in place** — from a checkout, `crow book` serves the book with ▶ Run / ⚙ Compile / ✓ Test / ■ Stop on every code block.
- **Learning with an AI assistant?** Paste [`docs/crow-for-llms.md`](docs/crow-for-llms.md) into it and ask it to teach you interactively — the file is written to make any LLM an accurate Crow tutor.
- **[Language specification](docs/language-spec.md)** — the reference, including the Design Notes explaining every deliberately rejected feature.
- Runnable programs in [`examples/`](examples/) (indexed in its README), and the full documentation map in [`docs/README.md`](docs/README.md).

## Two execution modes, one behavior

`crow run -i file.co` interprets instantly; `crow run file.co`
transpiles to Rust and runs a native binary. The two modes are held to
identical output on a growing parity corpus (`crow test --parity`).
Interpret while exploring, compile when shipping.

## Licence

Crow is licensed under either of the MIT licence ([LICENSE-MIT](LICENSE-MIT))
or the Apache licence, version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at
your option. The runtime crates a compiled program links are under the
same terms, so a program built with Crow carries only its author's licence.
