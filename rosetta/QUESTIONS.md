# Questions for the language

Decisions the Rosetta Code run met that are the language's to make, not a
fix to apply: in each, both modes agree, and the program that met it says
what it chose meanwhile. The findings the run fixed or left open are in
`findings/README.md`; the tasks not done in full, in `REVIEW.md`. Vijay
decided every one on 2026-10-29: the decision follows each question, with
the test that holds it.

- **Where `fmt`'s fill goes beside a sign.** `fmt[-42, width = 5, fill = '0']`
  is `00-42` — the fill before the sign — where printf's `%05d` gives
  `-0042`. The spec calls `fill` a pad character and says no more.
  **Decided:** after the sign, `-0042`, as printf and Python pad; any other
  fill before (parity 450).
- **How `fmt` rounds a half.** `fmt` of a float rounds a half to even
  (`fmt[2.5, decimals = 0]` is `2`, `fmt[0.125, decimals = 2]` is `0.12`),
  while `fmt` of a fraction and `round` round a half away from zero
  (`fmt[fraction[1, 8], decimals = 2]` is `0.13`, `round[2.5]` is `3`). The
  spec does not say which `fmt` does.
  **Decided:** kept, and said in the spec — a float's text rounds its exact
  binary value with ties to even, as every other language prints floats;
  exact numbers and `round` round half away from zero (parity 450).
- **Bytes past 127 in a program.** `read` takes UTF-8 text and `use sys`
  is the standard library's own, so a program cannot read or write the
  bytes 128 to 255 (Fixed length records, Musical scale). Binary file
  input and output for programs is a feature to decide.
  **Decided:** deferred — binary file input and output is a feature to
  design when a program needs it beyond these two tasks.
- **math's `sum` of ints.** math's `sum` answers a float, so `sum[xs] % n`
  of a list of ints is refused under `use math` (ISBN13 check digit), where
  a sum of ints is an int in the core. Is a float what `sum` of ints should
  answer?
  **Decided:** an int, every digit (parity 451). (There is no core `sum`:
  `sum` is math's.)
- **A top-level `seed` and a page's declarations.** In a `use ui` program
  (or `use web` with a derivation) every declaration is computed before
  the top level's other statements, so `seed[2026]` then `x = rand[0,
  9999]` draws `x` unseeded — another number on every run, in both modes
  (Create an HTML table). Should a top-level `seed` run among the
  declarations, in the order written, as a connection a declaration uses
  does?
  **Decided:** a page's top-level `seed[n]` runs before its declarations
  are set, wherever it is written, so the facts' draws are seeded (parity
  452).
- **`sort` of fractions and decimals.** Both compare by value (`lt`,
  `min`, `max`), and the spec's `sort` orders "numbers ascending", but no
  row of `sort` takes a list of either (Rational calculator); rows for
  them need a list kind of each in the table. The refusal now names the
  kind (`got a list of fractions`).
  **Decided:** sorted by value, as `lt` compares them (parity 456).
- **A parsed JSON whole number past 2^64.** serde_json, without its
  `arbitrary_precision` feature, holds such a number as a float at the
  parse, so `int[…]` and `encode` of it are the float's (JSON). Numbers up
  to 2^64 − 1 are now read exactly. Keeping every digit means turning the
  feature on for every crate — a JSON number held as its text — a cost to
  all JSON reading.
  **Decided:** left as it is: keeping every digit would cost all JSON
  reading; such numbers can travel as text.
- **Lock order inside the library.** Two names of process memory revised
  in opposite orders on two threads deadlock the compiled program (Mutex);
  the spec refuses only a revision that reaches its own name. Since
  2026-10-28 programs cannot import `memory`, so this is the standard
  library's own question.
  **Decided:** left to the library while `memory` is its own; made public
  again, two locks would be taken in name order.
- **A parsed value pushed onto an empty list.** `xs = []`, then `xs =
  push[xs, j['a']]`, reads the parsed value as a number — push's rows take
  one there, so that a later `max[hi]` reads numbers (packages/market's
  bars) — and the program stops on text (Associative array/Merging). A map
  held in a variable now holds parsed values as they are (parity 431), as a
  list made in the call does (`push[[], j]`). Reading the list by its later
  uses would need the settle pass to see reads, not only writes. Should an
  empty list held in a variable hold parsed values as they are?
  **Decided:** the rule stays (packages/market, kg and features lean on
  it), and the stop says why and how to keep parsed values, `push[[], …]`
  (parity 457).
- **An int beside a record, handed to one function.** `first[5]` and
  `first[Pick[1, 2]]`: the spec makes the parameter a record or none, and 5
  stops the program (`a record or 0 (none) is needed here, and this value
  is 5`), though a literal int other than 0 can never be the none (Monads/
  List monad). Should such a call get a copy of its own? A variable first
  given 0 that later holds a record must keep joining: its first round's
  call hands an int.
  **Decided:** the spec stays — the parameter holds a record or none, and
  another int stops; arithmetic on its answer is refused naming a record or
  none, with the way out (errors k32).
- **A local named like an import's name.** The spec refuses it (Names Are
  Never Ambiguous, rule 1, with `e = 100` as its example), and the top
  level does; inside a function, `e`, `mean`, `sign` or `stack` under `use
  math` (or `code` under `use bits`) shadows the module's name silently
  (Pathological floating point problems). Enforced there, 29 of the 696
  programs here would be refused — locals named `e`, `sign`, `sum`,
  `zeros`, `stack`, `code`, `where`, `ones`, `choice`, `full` and `gamma`.
  Enforce the rule in functions, or let a function's locals shadow an
  import's names?
  **Decided:** refused in functions too, as the spec says (errors k30,
  k31); the 29 programs here, four packages' files, an example and finch
  were renamed.
- **A computed int in a list written down.** A literal list of whole
  numbers is a vector unless the program grows it or writes it by
  position, and a vector holds its numbers as floats. Numerals past 2^53
  make a list of ints, but a computed item does not: `a = 2 ^ 60 + 1`,
  `xs = [a, 3]` holds 2^60, and `xs[0] eq a` is false — silently, in both
  modes; past 64 bits the literal stops where it is made (Abundant odd
  numbers). The width pass knows `a`'s range, but only after the literal's
  type is fixed. Should a literal holding a computed int be a list of
  ints, or stop where a float cannot hold the int exactly?
  **Decided:** it stops where the list is made, naming `push` (parity 454).
- **`concat` of three texts.** `concat[a, b]` joins two texts, and
  `concat[[a, b, c]]` joins a literal list of them; `concat[a, b, c]` is
  refused (Bioinformatics/Sequence mutation), since no row takes three
  texts — its rows of three join tensors along an axis. One way to do
  things says the list form is that way; should the refusal name it?
  **Decided:** allowed — `concat[a, b, c, …]` joins its pieces (parity
  453); the axis form takes the axis written down or named.
- **A list of 0s that later holds records.** The spec makes a variable, a
  field, a parameter or an answer given 0 and a record *a record or none*,
  but not a list's items: empty buckets made as `for i in range[n]` / `0`,
  then `b[h] = Entry[k, b[h]]`, are a list of ints, and reading `e.next` of
  an item is refused as an int's field (findings/monads_list_monad-4).
  Should a list's items follow the rule — the list a list of records or
  none from its declaration, each 0 none?
  **Decided:** its items follow the rule, a list of records or none, each 0
  none (parity 455).
