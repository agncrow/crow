# Questions for the language

Decisions the Rosetta Code run met that are the language's to make, not a
fix to apply: in each, both modes agree, and the program that met it says
what it chose meanwhile. The findings the run fixed or left open are in
`findings/README.md`; the tasks not done in full, in `REVIEW.md`.

- **Where `fmt`'s fill goes beside a sign.** `fmt[-42, width = 5, fill = '0']`
  is `00-42` — the fill before the sign — where printf's `%05d` gives
  `-0042`. The spec calls `fill` a pad character and says no more.
- **How `fmt` rounds a half.** `fmt` of a float rounds a half to even
  (`fmt[2.5, decimals = 0]` is `2`, `fmt[0.125, decimals = 2]` is `0.12`),
  while `fmt` of a fraction and `round` round a half away from zero
  (`fmt[fraction[1, 8], decimals = 2]` is `0.13`, `round[2.5]` is `3`). The
  spec does not say which `fmt` does.
- **Bytes past 127 in a program.** `read` takes UTF-8 text and `use sys`
  is the standard library's own, so a program cannot read or write the
  bytes 128 to 255 (Fixed length records, Musical scale). Binary file
  input and output for programs is a feature to decide.
- **math's `sum` of ints.** math's `sum` answers a float, so `sum[xs] % n`
  of a list of ints is refused under `use math` (ISBN13 check digit), where
  a sum of ints is an int in the core. Is a float what `sum` of ints should
  answer?
- **A top-level `seed` and a page's declarations.** In a `use ui` program
  (or `use web` with a derivation) every declaration is computed before
  the top level's other statements, so `seed[2026]` then `x = rand[0,
  9999]` draws `x` unseeded — another number on every run, in both modes
  (Create an HTML table). Should a top-level `seed` run among the
  declarations, in the order written, as a connection a declaration uses
  does?
- **`sort` of fractions and decimals.** Both compare by value (`lt`,
  `min`, `max`), and the spec's `sort` orders "numbers ascending", but no
  row of `sort` takes a list of either (Rational calculator); rows for
  them need a list kind of each in the table. The refusal now names the
  kind (`got a list of fractions`).
- **A parsed JSON whole number past 2^64.** serde_json, without its
  `arbitrary_precision` feature, holds such a number as a float at the
  parse, so `int[…]` and `encode` of it are the float's (JSON). Numbers up
  to 2^64 − 1 are now read exactly. Keeping every digit means turning the
  feature on for every crate — a JSON number held as its text — a cost to
  all JSON reading.
- **Lock order inside the library.** Two names of process memory revised
  in opposite orders on two threads deadlock the compiled program (Mutex);
  the spec refuses only a revision that reaches its own name. Since
  2026-10-28 programs cannot import `memory`, so this is the standard
  library's own question.
