# Installation

Crow builds from source with the Rust toolchain (it compiles your programs
to Rust, so `cargo` is a real dependency, not just a build-time one):

```bash
git clone <the crow repository>
cd crow
cargo install --path .
```

The binary lands in `~/.cargo/bin/crow` — make sure that directory is on
your `PATH`, then verify:

```bash
crow
```

You should see the usage help listing `run`, `build`, `new`, `fmt`,
`check`, and `test`.
