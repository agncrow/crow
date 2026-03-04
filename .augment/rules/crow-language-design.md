# Crow Language Design Rules

## Core Principles

Crow is a minimalist programming language for data science with automatic verification. Every design and implementation decision must uphold these principles:

1. **One way to do things.** There must be exactly one obvious way to accomplish any task. No aliases, no alternative syntax, no shortcuts that duplicate existing functionality. Before adding any feature, ask: "Does this create a second way to do something that's already possible?" If yes, don't add it. Examples: no string `+` (use interpolation), no `+=` (use `x = x + 1`), no double-quote strings (use single quotes), no ternary (use `if`/`else`).
2. **Minimal syntax.** Prefer the simplest, shortest syntax that is still unambiguous. Fewer keywords, fewer symbols, fewer rules to learn.
3. **Shift-key minimization.** Prefer symbols that don't require the shift key. `[]` for calls not `()`. `,` for separators not `:`. Avoid `{}` `!` `@` `#` `$` `%` `^` `&` `*` where possible. The language should be easy to type.
4. **One concept, one way.** Every value is a struct. Every call uses `[]`. Every block uses indentation. Consistency across the entire language — no special cases.
5. **Expressive but simple.** Easy to read, easy to write. Crow should feel like pseudocode that runs.
6. **LLM-friendly.** Consistent grammar with no ambiguity. One obvious way to do things means LLMs generate correct Crow code reliably.
7. **Parallel by default.** Concurrency is built into the language, not bolted on.
8. **Dynamic but fast.** Types are inferred at runtime and specialized for performance via transpilation to Rust.
9. **Zero effort verification.** Automatic checking with no test files, no annotations, no extra work from the programmer.

## Implementation Rules

10. **Refer to `docs/language-spec.md`** for the current language specification before making any design or implementation decisions.
11. **Builtin naming convention.** Builtin names must be short, single-word, lowercase, with no prefixes (`to_`, `get_`, `is_`) and no suffixes (`_with`, `_by`, `_from`). Use the shortest unambiguous name: `int` not `to_int`, `upper` not `to_upper`, `starts` not `starts_with`, `len` not `length`. If a name needs underscores, it's probably too long.
12. **After implementing a feature:** Always (a) update `docs/language-spec.md` to reflect the new or changed behavior, (b) write a `.crow` test program in `examples/` that exercises the feature end-to-end, and (c) add Rust `#[test]` unit tests in the relevant source file to verify transpiler/interpreter output. A feature is not done until the spec, the test program, and the unit tests are all in place and passing.
13. **After deciding NOT to implement a feature:** Add a "Design Note" section to `docs/language-spec.md` explaining what was considered, why it was rejected, and what to use instead. This preserves the reasoning for future contributors and prevents the same debate from recurring.
14. **Commit after implementation is done.** Once a feature (or removal) is fully implemented — code changes, spec updates, tests passing — immediately commit the changes with a clear, descriptive commit message. Don't leave completed work uncommitted.
15. **Interpreter–transpiler parity.** Every language feature must work identically in both the interpreter (`crow run -i`) and the transpiler (`crow build`). Always implement and test in both. A feature is not complete until both paths produce the same output for the same input.
16. **Unit tests for every feature.** Every language feature, builtin, CLI command, and checker rule must have Rust `#[test]` unit tests in the relevant source file. Tests should cover: (a) the happy path, (b) edge cases and boundary values, (c) error cases and invalid input. If a bug is fixed, add a regression test that reproduces the bug. Run `cargo test` and confirm all tests pass before considering any change complete.
17. **Always enforce "one way."** When proposing or implementing any new syntax, operator, keyword, or builtin, verify it does not duplicate existing functionality. If it does, reject it and document why in a Design Note. This rule overrides convenience — no feature is worth having two ways to do the same thing.
18. **Verify before claiming missing.** Before claiming any feature is missing, unimplemented, or needs to be added, search `docs/language-spec.md` thoroughly. Check operators (e.g., `*` is matmul, `^` is power, `.^` is element-wise power), core builtins (e.g., `sort`, `slice`), module tables (e.g., `math.stats` has `median`, `std`, `var`), and Design Notes. If a feature is already handled by an operator, builtin, or sub-module, it is NOT missing. Do not propose adding a function that duplicates an existing operator.

