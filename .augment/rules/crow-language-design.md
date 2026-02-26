# Crow Language Design Rules

When designing or implementing the Crow language:

1. **Minimize keywords and symbols.** Always prefer the simplest, shortest syntax that is still unambiguous.
2. **Expressive but simple.** Easy to read, easy to write.
3. **LLM-friendly.** Consistent grammar, one obvious way to do things.
4. **Parallel by default.** Concurrency is built into the language, not bolted on.
5. **Dynamic but fast.** Types are inferred at runtime and specialized for performance.
6. **Refer to `docs/language-spec.md`** for the current language specification before making any design or implementation decisions.
7. **Minimize shift-key usage.** Prefer symbols that don't require the shift key (e.g. prefer `,` over `:`, avoid `{}` `()` `!` `@` `#` `$` `%` `^` `&` `*` `+` where possible). The language should be easy to type.
8. **One way to do things.** There should be exactly one obvious way to accomplish any task. No aliases, no alternative syntax, no shortcuts that duplicate existing functionality.
9. **One concept, one way.** Every value is a struct. Every call uses `[]`. Every block uses indentation. Consistency across the entire language — no special cases.
10. **Builtin naming convention.** Builtin names must be short, single-word, lowercase, with no prefixes (`to_`, `get_`, `is_`) and no suffixes (`_with`, `_by`, `_from`). Use the shortest unambiguous name: `int` not `to_int`, `upper` not `to_upper`, `starts` not `starts_with`, `len` not `length`. If a name needs underscores, it's probably too long.
11. **After implementing a feature:** Always (a) update `docs/language-spec.md` to reflect the new or changed behavior, (b) write a `.crow` test program in `examples/` that exercises the feature end-to-end, and (c) add Rust `#[test]` unit tests in the relevant source file to verify transpiler/interpreter output. A feature is not done until the spec, the test program, and the unit tests are all in place and passing.
12. **After deciding NOT to implement a feature:** Add a "Design Note" section to `docs/language-spec.md` explaining what was considered, why it was rejected, and what to use instead. See §6 ("No Pattern Matching or Sum Types") as the template. This preserves the reasoning for future contributors and prevents the same debate from recurring.
13. **Commit after implementation is done.** Once a feature (or removal) is fully implemented — code changes, spec updates, tests passing — immediately commit the changes with a clear, descriptive commit message. Don't leave completed work uncommitted.
14. **Interpreter–transpiler parity.** Every language feature must work identically in both the interpreter (`crow run -i`) and the transpiler (`crow build`). Always implement and test in both. A feature is not complete until both paths produce the same output for the same input.

