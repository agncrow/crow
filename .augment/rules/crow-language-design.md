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

