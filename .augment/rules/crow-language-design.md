# Crow Language Design Rules

## Core Principles

Crow is a minimalist programming language for data science with automatic verification.

1. One way to do things. One obvious way per task. No aliases, no alternative syntax. Ask: "Does this duplicate something existing?" If yes, don't add it.
2. Minimal syntax. Simplest unambiguous syntax. Fewer keywords, fewer symbols, fewer rules.
3. Shift-key minimization. `[]` not `()`, `,` not `:`. Avoid `{}` `!` `@` `#` `$` `%` `^` `&` `*`.
4. One concept, one way. Every value is a struct. Every call uses `[]`. Every block uses indentation. No special cases.
5. Expressive but simple. Crow should feel like pseudocode that runs.
6. LLM-friendly. Consistent grammar, no ambiguity. LLMs generate correct Crow reliably.
7. Parallel by default. Concurrency is built in, not bolted on.
8. Dynamic but fast. Runtime type inference, transpiled to Rust for performance.
9. Zero effort verification. Automatic checking — no test files, no annotations.

## Implementation Rules

10. Builtin naming. Short, single-word, lowercase. No prefixes/suffixes. `int` not `to_int`, `len` not `length`. If it needs underscores, it's too long.
11. Interpreter–transpiler parity. Every feature must work identically in `crow run -i` and `crow build`.
12. No duplicates. Before adding syntax/operators/builtins, verify no existing feature does the same thing. Reject if it does.
13. Regression tests for bugs. Every bug fix must include a test that reproduces the bug. No silent regressions.
14. Prefer established crates. Use well-maintained Rust crates for non-trivial functionality. Don't hand-roll what a battle-tested library already solves.
15. No stubs, no partial delivery. A feature is not complete until every planned backend, variant, and code path is fully implemented and tested. Stub implementations that return "not yet implemented" errors are not acceptable for features marked as done. If a feature was designed with N backends, all N backends must work before it can be called complete.

## Documentation Rules

16. `docs/language-spec.md` is the source of truth. Read it before designing. Search it before claiming something is missing. Update it after implementing. Document rejected features as Design Notes in it.
17. Document every feature. Each crate and module gets its own doc file. Document what it does, its API, and usage examples. A feature without documentation is not done.
18. Track progress. `docs/roadmap.md` lists planned, in-progress, and rejected features. When a feature is built and documented, remove it from the roadmap. Rejected features stay with a reason.

## Workflow

Every feature follows this sequence. Do not skip steps.

1. **Architect.** Ask: "Is this the right module/layer/abstraction?" Check how mature languages solve the same problem. Propose placement. Challenge existing structure if wrong.
2. **Design.** Read `docs/language-spec.md` and `docs/roadmap.md`. Verify no duplicate exists. Define the Crow syntax, API, and Rust backend. Discuss with the user before writing code.
3. **Document first.** Update `docs/language-spec.md` with the new API. Update or create the module's doc file. Add the feature to `docs/roadmap.md` as in-progress.
4. **Implement.** Write the Rust code. Ensure interpreter–transpiler parity.
5. **Test.** Add `#[test]` unit tests (happy path, edge cases, errors). Add `examples/*.crow` integration test. Run `cargo test` — all must pass.
6. **Document after.** Update docs to reflect final implementation (if it diverged from step 3). Move feature from in-progress to done in `docs/roadmap.md`. Remove from roadmap if fully documented elsewhere.
7. **Build clean.** Run `cargo build` and `cargo test`. Fix all errors and warnings. No commits with broken builds or compiler warnings.
8. **Commit.** One clear commit message. Don't leave completed work uncommitted.
