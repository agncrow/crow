# Crow — notes for Claude Code

Read `docs/design-rules.md` first: nine principles, the implementation and
documentation rules, and the feature workflow. The spec and the code cite
them by number. The short version:

- **One way to do things.** Before adding syntax, an operator, or a
  builtin, search `docs/language-spec.md` for an existing way; if one
  exists, don't add another. Rejected ideas become Design Notes in the spec.
- **The spec is the source of truth.** Read it before designing, update it
  after implementing. Then the book chapter and `crow test --docs`.
- **Both modes, one behavior.** Every feature works identically in
  `crow run -i` and `crow run`. Shared logic lives in `crow_runtime` so
  both call the same code; new behavior gets a `tests/parity/` program.
- **Builtins are short lowercase words**, every call is `f[x]`, blocks are
  indentation, and shift-key syntax needs a recorded tradeoff.
- **Every bug fix ships with the test that reproduces it.** No stubs: a
  feature isn't done until every planned backend works and is documented.

## Before calling anything done

```
cargo test --workspace
cargo clippy --workspace --all-features
./target/release/crow test --parity
./target/release/crow test --packages
./target/release/crow test --fuzz 20
./target/release/crow test --served
./target/release/crow test --docs
./target/release/crow test --book
./target/release/crow test tests/layout/conformance.co
./target/release/crow test src/modules/nn.co
./target/release/crow test src/modules/core.co
./target/release/crow test src/modules/sys.co
./target/release/crow test src/modules/io.co
./target/release/crow test src/modules/frame.co
./target/release/crow test src/modules/math.co
./target/release/crow test src/modules/auth.co
./target/release/crow test src/modules/web.co
./target/release/crow test src/modules/ws.co
./target/release/crow test src/modules/mail.co
```

Use the release binary for the layout and site suites: the debug binary
overflows its stack there. CI (`.github/workflows/ci.yml`) runs the same
steps on every push and must be green.

## Things that have bitten

- `*.csv`, `*.txt`, `*.json` are gitignored. A fixture needs a negation
  rule in `.gitignore` or CI never sees it.
- `Cargo.lock` is not tracked, and `cargo install` re-resolves unless
  given `--locked`.
- The seeded RNG is process-global; tests that draw random numbers take
  the lock in `crow_runtime/src/tensor.rs`'s tests.
- Map literals are `map[k = v]`; nested numeric literals must be
  rectangular (ragged ones are refused in both modes).
- Generated programs link the runtime crates by the absolute path baked in
  at build time (`CARGO_MANIFEST_DIR`); `install.sh` snapshots the source
  to `~/.crow/src` for that reason.
- After verifying a served app (finch on 3100), run `crow stop` so the
  port is free for the user.
- A debug build checks the IR of every program it lowers
  (`CROW_IR_VERIFY`, strict by default there; the harnesses set it for the
  release binary). A finding is a bug in the lowering, not in the program.
- A standard module written in Crow (`src/modules/*.co`) is read in its own
  scope: inside its inlined functions the program's names are invisible
  (`calls_fn` in lower_ir.rs, lower.rs, checks.rs and type_infer/calls.rs).
  A new resolution site that consults `ctx.functions` by a bare name needs
  the same guard, or a module local named like a program function becomes
  a call. The inliner keeps only the rows a program reaches (by name), and
  the modules' scopes are read from the program's own functions
  (`crow_modules_carried`), never from whatever was inlined last on the
  thread: a harness lowers a part's tests and the app in turn.
- `crow_runtime`, `crow_db` and `crow_web` deny panics under clippy: a
  runtime function answers its reason, and a failure no value can carry
  stops through `crow_runtime::stops` (docs/bug-classes.md).
