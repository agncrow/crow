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
    `tests/parity/`; `crow test --parity` diffs the two modes. A parity
    program logs what it checks: one that prints nothing in either mode
    fails, since the same silence compares nothing.
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
20. **Formulas in Crow, primitives in Rust, laws in Crow for both.** A
    standard module is written in Crow wherever it is a formula over other
    builtins (`math.nn`, `math.stats`, the indicators); native only when
    it must be — it touches the operating system, it is the compiler's own
    mechanism (the autograd tape), it is a validated numeric kernel
    (`faer`, `rustfft`), or it is a scalar loop the interpreter cannot
    afford. The native primitives the Crow-written modules stand on live
    under `use sys`, which only the compiler's own modules may import: a
    program sees one way (`serve`, `request`, `connect`, `read`), the
    library the smallest layer that lets the rest be Crow. Every kernel
    that stays native gets laws in Crow (known answers, algebraic
    identities), run by the gate in both modes. A standard module written
    in Crow keeps its contract in the builtin table (`crow!` rows: what
    each function takes, answers and how it fails) and its body in
    `src/modules/<name>.co`, inlined into the program that imports it and
    tested on its own (`crow test src/modules/nn.co`); a stop inside it
    names the line of the call in the program. The spec's Design Note
    "what stays native, and why" (section 13) accounts for every crate
    the manifests name, by one of the four reasons; a crate added gets
    its line there.

## Verification rules

21. **One vocabulary, total.** A value has one type, `crate::ir::Ty`,
    which inference, the lowering, the verifier and both backends read
    and write; the interpreter holds a value as `Value`; a builtin row
    states its contract as `Kind`, and `Arg` is a type's class for
    choosing a row (`Arg::of_type`). No other enum names a type. A `match`
    over `Ty`, `Value`, `Kind` or `Arg` names every variant; a wildcard
    arm is for a pair of two kinds and nothing else, since a wildcard
    hides the variant nobody thought about (a list of moments compared
    whole fell to `_ => false`). `src/totality.rs` names every variant
    and holds every function that maps one: a variant added fails there
    until it is placed, and each test says what each variant answers.
    The one row choice for a bare name is `crate::table::module_of_bare`,
    which inference and the lowering both call.
22. **Covered.** The gate measures line coverage of the compiler
    (`scripts/coverage.sh`: the unit tests, the docs check, the parity
    corpus and the packages under `cargo llvm-cov`) and fails when a
    crate falls under its floor, recorded in the script; the floor only
    rises. A change that adds code adds the test that reaches it — a unit
    test for a function, a parity program for both modes, a book block
    for a user-facing name, a harness for a page — and a line the gate
    cannot reach is named in the commit message with why.
    `scripts/coverage.sh --arms` writes the lines no suite reaches, by file,
    with the match arms among them named: a backend's arm for what the
    lowering must never produce is where the next slip hides.
23. **Held to itself.** Line coverage says a line ran, not that it ran
    with every kind it meets: a feature tested where its author expected it
    passed every suite while it broke beside an older one. `crow check
    --self` (`src/selfcheck.rs`) writes its cases from the compiler's own
    tables — every row with every kind it takes, every collection at every
    index, every operator over every pair of operands, the shapes a value
    meets — and runs each in every configuration that must agree
    (interpreted, walked, unfused, unmoved, compiled). A new row, kind or
    operator is covered by the tables the check reads: a `Kind` names the
    values it is tried with (the match has no wildcard). The findings known
    are listed in `tests/self/known.txt`, which only shrinks.
24. **The compiler holds the language.** The compiler — the parser, the
    lowering, inference, the IR and its passes, both backends, and the
    builtin registry's logic — knows syntax, the types the spec defines
    and analyses over the program, and names no module and no builtin.
    What a builtin is (its kinds, effects, options, failure, its in-place
    and growing forms, its crate) is declared once, in its row; what a
    module is (its path, source, crate, parent, docs) once, in its
    manifest; a module written in Crow carries no declaration at all —
    its functions are its contract. Modules are built with the language,
    never inside it (Vijay, 2026-10-18), and a Crow file never names a
    Rust type, function or attribute. The compiler is frozen: a change to
    it is a language feature, an optimization or a bug fix, named as such
    in the commit and asked for before it is made, with a strong reason;
    a feature a module needs is built in the module or, where the
    language cannot express it, proposed as a general capability any
    module could use. `src/freeze.rs` counts the module and builtin names
    the compiler's sources hold against `tests/freeze/names.txt`, which
    only shrinks. The removal and its phases: docs/extending-crow.md.

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
