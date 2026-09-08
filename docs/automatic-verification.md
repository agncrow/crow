# Automatic Verification in Crow

**Design goal:** The developer writes code. Nothing else. No tests, no contracts, no annotations. Crow verifies correctness automatically.

**Philosophy:** The code already tells the compiler everything — every branch, every type, every data flow, every access pattern. The compiler should use that knowledge to find bugs, prove safety, and generate tests without any developer effort.

## Overview

One command. Zero effort. Interactive.

```
crow check

  12 functions checked, 2 need review

  [1/2] src/parser.co > get_middle crashes on empty list (line 3)
        items[mid] — but items is empty
        try: get_middle[[]]

        enter = recheck, s = skip, t = try input >
```

When everything is fine:

```
crow check

  ✓ 12 functions checked, no issues found
```

For CI or batch mode, `crow check -v` dumps the full report with no interaction:

```
crow check -v

  static analysis
    functions: 12 (8 pure, 4 impure)
    unused variables: none
    unreachable code: none
    parallel safety: no races
    for loops: all terminate

  dynamic analysis (pure functions only)
    8 functions tested
    47 edge cases generated, all passed
    snapshots: 6 unchanged

  ✓ no issues found
```

### Interactive flow

```
developer runs crow check
  ↓
static analysis (instant)
  ↓
generate edge cases for pure functions (simple types only)
  ↓
run edge cases in interpreter (fast)
  ↓
show summary: "12 functions checked, 2 need review"
  ↓
present first issue
  ↓
developer fixes code ←──────┐
  ↓                          │
re-run just that function    │
  ↓                          │
still broken? show again ────┘
  ↓
fixed? show next issue
  ↓
all done? "✓ no issues found"
```

### Output principles

| Principle | Rule |
|---|---|
| Lead with the answer | "2 need review" or "no issues found" — not stats |
| Plain English | "crashes on empty list" — not "index out of bounds at line 3" |
| Give a repro | `try: get_middle[[]]` — they can paste it into `crow run` |
| Hide stats by default | Coverage, branch counts, etc. only shown with `-v` |
| No jargon | Edge cases are generated but aren't counted unless `-v` is used. |
| One line when green | `✓ 12 functions checked, no issues found` |
| Module context | `src/parser.co > get_middle` — not just `get_middle` |

---

## Feature 1: Purity Detection

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Which functions are deterministic and side-effect-free

A pure function has no I/O, no mutation of external state, no randomness. Same input → same output, always. The compiler can detect this automatically.

```
fn double[x]              --- pure: just arithmetic
  return x * 2

fn fetch_user[id]         --- impure: calls read[]
  data = read['users/`id`.json']
  return json[data]

fn process[items]         --- pure: only calls pure functions
  return for x in items
    double[x]
```

### Why purity matters

1. **Pure functions are fully auto-testable.** The compiler can generate random inputs and verify outputs.
2. **Pure functions are safely parallelizable.** No side effects → no data races, guaranteed.
3. **Pure functions are cacheable.** `double[5]` always returns `10` — the compiler can memoize.
4. **Impure functions need attention.** The compiler flags them so the developer knows where the risk is.

```
crow check -v

  purity:
    pure:   double, binary_search, fibonacci, sort, filter
    impure: fetch_user (read[]), save_report (write[]), send_email (fetch[])
```

Purity classification is silent in normal mode — it only matters internally (determines which functions get edge case testing). Shown with `-v`.

### What makes a function impure

| Side effect | Builtins |
|---|---|
| File I/O | `read[]`, `write[]` |
| Network | `fetch[]` |
| Randomness | `rand[]` |
| Time | `time[]` |
| Process control | `exit[]`, `sleep[]` |
| Console output | `log[]`, `input[]` |
| Environment | `env[]` |

A function is pure if it (and everything it calls) uses none of these.

---

## Feature 2: Guaranteed Static Checks

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Unused variables, unreachable code, infinite loops, parallel safety violations

These checks are always correct — zero false positives. The compiler is certain.

### What's guaranteed

| Check | How |
|---|---|
| **Unused variables** | Variable assigned, never read. Syntactic check. |
| **Unreachable code** | Code after `return`. Dead `else` after `if true`. |
| **For-loop termination** | `for` iterates a finite collection. Always terminates. |
| **While-loop termination** | Ranking function analysis — does the loop variable converge? |
| **Parallel safety** | RAW/WAW/WAR hazard detection on parallel waves. Already implemented in `src/parallel.rs`. |

### Example

```
fn process[items]
    x = 10
    y = 20
    result = x + y
    return result

fn categorize[score]
    if score gt 90
        return 'A'
    if score gt 80
        return 'B'
    return 'C'
    return 'D'
```

```
crow check

  2 issues found

  [1/2] src/utils.co > process has unused variable (line 3)
        y = 20 — assigned but never read

  [2/2] src/utils.co > categorize has unreachable code (line 12)
        return 'D' — after return 'C', can never execute

  enter = recheck, s = skip >
```

### Termination analysis

`for` loops always terminate (finite collection). `while` loops need analysis:

```
while low lte high
  low = mid + 1       --- low increases or high decreases
  high = mid - 1      --- gap always shrinks → terminates ✓
```

When the compiler can't prove termination, it warns but doesn't block:

```
  [1/1] src/math.co > collatz — while loop may not terminate (line 3)
        loop variable 'n' has no guaranteed convergence
```

### Parallel safety

```
crow check -v

  parallel waves:
    wave 1: [a, c]     --- parallel ✓
    wave 2: [b]        --- needs a
    wave 3: [d]        --- needs a, b, c
  no data races ✓
```

---

## Feature 3: Edge Case Generation

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Boundary bugs, off-by-one errors, empty input crashes

For pure functions with simple parameter types, the compiler generates boundary inputs and runs them automatically. Functions with complex types (maps, structs, nested structures) are skipped.

### Supported parameter types

| Type | Generated inputs |
|---|---|
| `int` | `0`, `1`, `-1`, `max_int`, `min_int` |
| `float` | `0.0`, `1.0`, `-1.0`, `0.5`, `-0.5` |
| `str` | `''`, `'a'`, `'hello'` |
| `bool` | `true`, `false` |
| `list of int` | `[]`, `[0]`, `[1, 2, 3]`, `[1]`, `[-1, 0, 1]` |
| `list of str` | `[]`, `['']`, `['a', 'b']` |

Functions with `map`, `struct`, or nested list parameters are **not tested** — they're silently skipped. This is a known limitation. The system tests what it can and is honest about what it can't.

### Example — all passing

```
fn binary_search[items, key]
  low = 0
  high = len[items] - 1
  while low lte high
    mid = floor[[low + high] / 2]
    if items[mid] eq key
      return mid
    if items[mid] lt key
      low = mid + 1
    else
      high = mid - 1
  return -1
```

```
crow check

  ✓ 12 functions checked, no issues found
```

With `-v`:

```
crow check -v

  fn binary_search — 10 edge cases, all passed ✓
    binary_search[[], 0]           → -1
    binary_search[[1], 1]          → 0
    binary_search[[1], 2]          → -1
    binary_search[[1, 2, 3], 2]    → 1
    binary_search[[1, 2, 3], 4]    → -1
    ... 5 more

  fn process_config — skipped (map parameter, not supported yet)
```

### Example — bug found

```
fn get_middle[items]
  mid = len[items] / 2
  return items[mid]
```

```
crow check

  1 issue found

  [1/1] src/utils.co > get_middle crashes on empty list (line 3)
        items[mid] — but items is empty
        try: get_middle[[]]

        enter = recheck, s = skip, t = try input >
```

### How it generates inputs

The system uses the transpiler's type inference — by the time Crow code is transpiled to Rust, parameter types are known. It then generates boundary values for each type using the table above. All combinations of boundary values across parameters are tested (capped at ~50 combinations per function to stay fast).

---

## Feature 4: Regression Snapshots

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Accidental behavior changes during refactoring or parameter tuning

### First run — snapshot created

```
crow check

  ✓ 12 functions checked, no issues found
  snapshots saved for 8 pure functions
```

Behind the scenes, `.crow/snapshots/` records every input-output pair from edge case generation:

```
--- .crow/snapshots/binary_search.snap
binary_search[[], 5] = -1
binary_search[[5], 5] = 0
binary_search[[5], 3] = -1
binary_search[[1, 2, 3], 1] = 0
binary_search[[1, 2, 3], 3] = 2
...
```

### Later — behavior change detected

You edit the function. Run `crow check` again:

```
crow check

  1 issue found

  [1/1] src/search.co > binary_search behavior changed (line 8)
        binary_search[[1, 1, 1], 1] was 1, now 0
        try: binary_search[[1, 1, 1], 1]

        enter = recheck, a = accept new behavior, s = skip >\
```

If you press `a`, the snapshot updates. If you fix the code and press enter, it rechecks.

### Why this is powerful

- **No test file to maintain.** The snapshot IS the test.
- **Catches subtle changes.** Even if the function still "works", any behavior difference is flagged.
- **Works across refactors.** Rename variables, restructure code — if the outputs match, it passes.
- **Version controlled.** `.crow/snapshots/` goes in git. Team members see regressions too.
- **Parameter tuning.** Change a threshold in a config, instantly see which functions produce different outputs.

---

## Guaranteed vs. Best-Effort

Not all checks carry the same confidence:

| Category | Features | Confidence |
|---|---|---|
| **Guaranteed** | Purity detection, unused variables, unreachable code, parallel safety | 100% correct. Zero false positives. Provable from the AST. |
| **Best-effort static** | Termination proof | Usually correct. Some cases are undecidable (halting problem). Warns, never blocks. |
| **Dynamic** | Edge case generation, regression snapshots | Finds real bugs, but can't prove absence of bugs. Limited to simple parameter types. |

The developer should know: green on guaranteed checks means **proven safe**. Green on dynamic checks means **no bugs found in the inputs we tested**.

---

## Limitations

Honest about what `crow check` can't do:

| Limitation | Why |
|---|---|
| **Complex parameter types** | Edge case generation only works for `int`, `float`, `str`, `bool`, and lists of those. Functions taking maps, structs, or nested structures are skipped. |
| **No shrinking** | When a crash is found, the system reports the first failing input. It doesn't try to find a simpler one. The boundary-value approach usually produces simple inputs anyway. |
| **No coverage tracking** | The system doesn't know which branches were hit. It generates inputs from the type signature, not from branch analysis. |
| **No concolic execution** | No symbolic execution or constraint solving. Just boundary values. |
| **Impure functions** | Not tested at all. Only pure functions get edge cases. |

These are all candidates for future improvement, but the V1 scope is deliberately small and honest.

---

## Summary

| # | Feature | Phase | Confidence | Catches |
|---|---|---|---|---|
| 1 | Purity detection | Static | Guaranteed | Side effects, testability classification |
| 2 | Static checks | Static | Guaranteed | Unused vars, unreachable code, parallel safety |
| 3 | Edge case generation | Dynamic | Best-effort | Boundary bugs, crashes (simple types only) |
| 4 | Regression snapshots | Dynamic | Best-effort | Accidental behavior changes |

All run under one command: `crow check`.

### Implementation priority

| Priority | Feature | Complexity | Value |
|---|---|---|---|
| 1 | Purity detection | Low | Foundation for everything else |
| 2 | Static checks | Low | Guaranteed correctness, easy wins |
| 3 | Edge case generation | Medium | The actual differentiator |
| 4 | Regression snapshots | Low | Huge practical value for refactoring |

### Design principles

1. **Zero effort.** No annotations, no contracts, no test files. The code is the spec.
2. **One command.** `crow check`. That's it.
3. **Interactive.** One issue at a time. Fix and recheck. Not a wall of text.
4. **Helpful, not blocking.** Warnings, not errors. The developer decides what to fix.
5. **Plain English.** "crashes on empty list" — not "IndexOutOfBoundsException".
6. **Fast.** `crow check` should complete in under a second for typical projects.
7. **Honest.** Only test what we can. Skip what we can't. No false confidence.

### What no other language does

- **Rust** requires type annotations and lifetime management. Crow doesn't.
- **Haskell** requires monads to separate pure from impure. Crow detects it automatically.
- **QuickCheck/Hypothesis** require you to write properties or test functions. Crow generates tests from the code itself.
- **Snapshot tools** (Jest snapshots, etc.) require existing test files. Crow generates its own.

Crow is the first language where `crow check` with zero test files gives you edge case generation, regression snapshots, and interactive fixing — for pure functions with simple types, fully automatically.

---

## Design Note: No Mutation Testing

**Considered:** Automatically mutating code (changing `lt` to `lte`, `+1` to `-1`) and re-running edge cases to find weak spots in test coverage.

**Rejected:** Mutation testing is computationally expensive, produces noisy results, and creates a false sense of completeness. It also adds significant complexity to the toolchain.

**What to use instead:** If `crow check` finds no crashes and snapshots are stable, the code is in good shape. If you want more confidence, write explicit tests — Crow doesn't prevent manual testing, it just doesn't require it.

**Revisit when:** The language is mature, the edge case generator is battle-tested, and there's evidence that mutation testing catches bugs the other features miss.

---

## Design Note: No Auto-Fix

**Considered:** Having `crow check` suggest and automatically apply fixes (e.g., adding `if len[items] eq 0` guards).

**Rejected:** The compiler doesn't know the developer's intent. Adding a guard might be wrong — maybe the function should return a default value, or maybe the caller should never pass an empty list. Auto-fix trains developers to accept suggestions without understanding the underlying problem.

**What to use instead:** `crow check` shows the problem clearly ("crashes on empty list") and provides a reproducible input (`try: get_middle[[]]`). The developer decides the right fix.

---

## Design Note: No Shrinking or Coverage Tracking (Initially)

**Considered:** Shrinking (reducing failing inputs to minimal reproductions) and coverage tracking (instrumenting the interpreter to find untested branches, then auto-generating inputs to fill gaps).

**Rejected for V1:** Shrinking requires a search algorithm that understands how to simplify each type — doable but adds complexity. Coverage-guided input generation is essentially constraint solving — a research-grade problem. Both are valuable but not necessary for V1.

**What to use instead:** The boundary-value approach already produces small, readable inputs (`[]`, `[1]`, `0`, `-1`). Most crashes will be caught by these simple cases. Coverage tracking can be added later as a `-v` diagnostic without changing the user-facing behavior.

**Revisit when:** V1 is shipped and real-world usage shows that boundary values miss important bugs that shrinking or coverage-guided generation would catch.
