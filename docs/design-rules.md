# Crow design rules

The principles every feature is measured against, the rules for building
one, and the workflow. The spec and the code cite these by number
("principle #3", "design rule 11"), so the numbering is stable: a rule is
reworded or retired in place, never renumbered. `CLAUDE.md` at the
repository root points assistants here.

## Core principles

1. **One way to do things.** One obvious way per task. No aliases, no
   alternative syntax. Before adding anything, ask whether it duplicates
   something that exists; if it does, don't add it. Every rejected feature
   gets a Design Note in the spec saying why.
2. **Minimal syntax.** The simplest unambiguous form. Fewer keywords, fewer
   symbols, fewer rules.
3. **Shift-key minimization.** `[]` not `()`, `,` not `:`. Avoid `{}` `!`
   `@` `#` `$` `%` `^` `&` `*`. A shift-key exception (`^` for power) is a
   recorded tradeoff, not a habit.
4. **One concept, one way.** Every call uses `[]`. Every block is
   indentation. Every value is a struct. No special cases.
5. **Expressive but simple.** Crow reads like pseudocode that runs.
6. **LLM-friendly.** A consistent grammar with no ambiguity, so a model
   generates correct Crow reliably. Errors teach the one right form.
7. **Parallel by default.** Concurrency is built in, not bolted on: `for`
   runs its iterations concurrently, `later[]` and the live loop are the
   only background machinery.
8. **Dynamic but fast.** Types are inferred; programs transpile to Rust for
   performance.
9. **Verification is part of the function.** `test` blocks sit next to the
   code they check; the parity corpus and the book are executable; `crow
   check` finds hazards without running anything. No separate test
   language, no annotations.

## Implementation rules

10. **Builtin naming.** Short, single-word, lowercase, no prefixes or
    suffixes: `int` not `to_int`, `len` not `length`. If it needs an
    underscore, it's too long.
11. **Interpreter–compiler parity.** Every feature works identically in
    `crow run -i` and `crow run`. New behavior gets a program in
    `tests/parity/`; `crow test --parity` diffs the two modes.
12. **One kernel, both modes.** Logic both modes need (chart geometry,
    layout, markdown, indicators, formatting) lives in `crow_runtime`, and
    both the interpreter and generated code call it. The modes agree by
    construction, not by discipline.
13. **Regression tests for bugs.** Every bug fix includes a test that
    reproduces the bug.
14. **Prefer established crates.** Use well-maintained Rust crates for
    non-trivial functionality; don't hand-roll what a battle-tested library
    solves.
15. **No stubs, no partial delivery.** A feature is not done until every
    planned backend, variant, and code path works and is tested. A "not yet
    implemented" path in a feature marked done is a bug.
16. **No syntax split between frontend and backend.** A file that uses
    `use ui` is still plain Crow on every line: no DSL, no Rust names
    leaking through. The compiler maps Crow calls to the backend
    (`state.get['key', 0]`, never `get_i64`).

## Documentation rules

17. **`docs/language-spec.md` is the source of truth.** Read it before
    designing, search it before claiming something is missing, update it
    after implementing. Rejected features are recorded there as Design
    Notes.
18. **Document every feature.** The spec section, the book, and the
    generated standard-library pages; `crow test --docs` checks that the
    documentation covers every compiler name and nothing more. A feature
    without documentation is not done.
19. **Track progress.** `docs/roadmap.md` lists planned, in-progress, and
    rejected features. Built and documented features leave the roadmap;
    rejected ones stay, with the reason.

## Workflow

Every feature follows this sequence.

1. **Architect.** Is this the right module, layer, abstraction? How do
   mature languages solve it? Propose the placement; challenge the existing
   structure if it's wrong.
2. **Design.** Read the spec and the roadmap. Verify no duplicate exists.
   Define the Crow syntax, the API, and the Rust backend. Discuss before
   writing code when the surface is new.
3. **Document first.** Put the API in the spec. Add the feature to the
   roadmap as in progress.
4. **Implement.** Rust in the right crate; parity in both modes.
5. **Test.** Unit tests (happy path, edge cases, errors); a `tests/parity/`
   program when both modes must agree on new behavior; a book block when it
   is user-facing; an `app[]` harness test for anything a page renders.
6. **Verify.** `cargo test --workspace`, `crow test --parity`, `crow test
   --packages`, `crow test --fuzz`, `crow test --docs`, `crow test --book`,
   and the layout suites on the release binary.
   CI runs the same on every push; it must be green.
7. **Document after.** Reconcile the spec with what shipped; move the
   feature off the roadmap.
