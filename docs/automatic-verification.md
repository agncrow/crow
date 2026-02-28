# Automatic Verification in Crow

**Design goal:** The developer writes code. Nothing else. No tests, no contracts, no annotations. Crow verifies correctness automatically.

**Philosophy:** The code already tells the compiler everything — every branch, every type, every data flow, every access pattern. The compiler should use that knowledge to find bugs, prove safety, and generate tests without any developer effort.

## Overview

One command, zero annotations:

```
crow check

  --- static analysis ---
  functions analyzed: 12
  pure: 8/12  impure: 4/12
  crash-free: 11/12
    WARNING: parse_csv may crash on empty input (line 34)
  termination: 12/12 proven
  parallel safety: no races detected

  --- dynamic analysis ---
  47 functions tested, 312 edge cases generated
  all passed
  mutation score: 94% (289/307 mutants killed)
  3 weak spots:
    - binary_search: test with duplicates needed
    - parse_csv: empty string not tested
    - sort: single-element list not tested
  snapshots saved
```

Static analysis runs first (instant, no execution). Then dynamic analysis generates edge cases and runs them. One command does everything.

---

## Feature 1: Crash-Proof Analysis

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Index out of bounds, division by zero, type mismatches

The compiler traces data flow through every function and proves that operations are safe — or reports exactly where they aren't.

### Example

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

The compiler traces:
- `low` starts at 0, `high` starts at `len[items] - 1`
- `mid = floor[[low + high] / 2]` — always between `low` and `high`
- Loop condition `low lte high` ensures `mid` is a valid index
- But what if `items` is empty? `high = -1`, loop doesn't execute → safe

```
crow check

  binary_search: crash-free ✓
    items[mid] — proven safe (mid always in bounds when loop runs)
    floor division — no division by zero
```

### When it can't prove safety

```
fn get_middle[items]
  mid = len[items] / 2
  return items[mid]
```

```
crow check

  get_middle: items[mid] may be out of bounds
    when: items = []
    fix:  add 'if len[items] eq 0' guard
```

### Techniques

- **Abstract interpretation** — track value ranges through branches and loops
- **Interval arithmetic** — `low` is in `[0, len-1]`, `high` is in `[0, len-1]`, so `mid` is in `[0, len-1]`
- **Path-sensitive analysis** — inside `if len[items] gt 0`, the compiler knows the list is non-empty

### Prior art

| Tool | Language | Approach |
|---|---|---|
| SPARK | Ada | Formal verification with contracts |
| Zig | Zig | Runtime bounds checks + safety analysis |
| Infer | Java/C | Facebook's static analyzer for null/bounds |
| Prusti | Rust | Verification for Rust via Viper |

**Crow's difference:** No contracts, no annotations. The analysis is fully automatic.

---

## Feature 2: Purity Detection

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Which functions are deterministic and side-effect-free

A pure function has no I/O, no mutation of external state, no randomness. Same input → same output, always. The compiler can detect this automatically.

```
fn double[x]              --- pure: just arithmetic
  return x * 2

fn fetch_user[id]         --- impure: calls read[]
  data = read['users/{id}.json']
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
crow check

  pure:   double, binary_search, fibonacci, sort, filter  (auto-verified ✓)
  impure: fetch_user, save_report, send_email             (need review)
    fetch_user: calls read[] (I/O)
    save_report: calls write[] (I/O)
    send_email: calls fetch[] (network)
```

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

## Feature 3: Termination Proof

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Infinite loops

### For loops — always terminate

`for` loops iterate over a finite collection. They always terminate. Nothing to check.

```
for x in items        --- finite collection → terminates ✓
  log[x]

for i in range[100]   --- range is finite → terminates ✓
  log[i]
```

### While loops — need analysis

The compiler checks: does the loop condition's variable converge toward termination?

```
while low lte high
  ...
  low = mid + 1       --- low increases
  high = mid - 1      --- or high decreases
                      --- gap always shrinks → terminates ✓
```

```
while len[queue] gt 0
  item = pop[queue]   --- queue shrinks each iteration → terminates ✓
  process[item]
```

### When it can't prove termination

```
fn collatz[n]
  while n neq 1
    if n % 2 eq 0
      n = n / 2
    else
      n = n * 3 + 1
  return n
```

```
crow check

  WARNING: collatz — while loop may not terminate
    loop variable 'n' has no guaranteed convergence
    (this is an open problem in mathematics)
```

The compiler doesn't block compilation — it just warns. Some loops are genuinely undecidable, and that's fine.

### Techniques

- **Ranking functions** — find a value that strictly decreases each iteration and is bounded below
- **Loop variant analysis** — `len[queue]` decreases by 1 each iteration, bounded by 0
- **Simple patterns** — counter moving toward a bound (`i = i + 1` with `while i lt n`)

---

## Feature 4: Automatic Edge Case Generation

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Boundary bugs, off-by-one errors, empty input crashes, type edge cases

The compiler reads the function signature and body, then generates inputs that exercise every branch and boundary.

### Example

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
crow test

  binary_search:
    binary_search[[], 5]              = -1     --- empty list
    binary_search[[5], 5]             = 0      --- single element, found
    binary_search[[5], 3]             = -1     --- single element, not found
    binary_search[[1, 2, 3], 1]       = 0      --- first element
    binary_search[[1, 2, 3], 3]       = 2      --- last element
    binary_search[[1, 2, 3], 2]       = 1      --- middle element
    binary_search[[1, 2, 3], 4]       = -1     --- above range
    binary_search[[1, 2, 3], 0]       = -1     --- below range
    binary_search[[1, 1, 1], 1]       = 1      --- duplicates
    binary_search[[1..1000], 500]     = 499    --- large list

    10/10 passed, 0 crashes, 100% path coverage ✓
```

### How it generates inputs

| Source | Strategy |
|---|---|
| **Parameter types** | `items` is a list of ints → generate `[]`, `[x]`, `[x,y]`, `[x,y,z]`, large lists |
| **Branch conditions** | `items[mid] eq key` → generate inputs where key is present and absent |
| **Boundary values** | `len[items] - 1` → test length 0, 1, 2 |
| **Comparisons in code** | `lt`, `lte`, `eq` → test values at, above, and below the boundary |
| **Loop structure** | `while low lte high` → test 0 iterations, 1 iteration, many iterations |
| **Concolic execution** | Run symbolically, solve constraints to find inputs that hit uncovered branches |

### What the developer does

Nothing. They look at the output for 5 seconds. If `binary_search[[1,2,3], 2]` returns `1`, it's correct. If it returns `0`, something's wrong. The system finds the edge cases, the human judges correctness.

---

## Feature 5: Mutation Testing

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Weak test coverage — places where a bug could hide undetected

After generating and running edge cases, Crow automatically **mutates** the code and re-runs:

### How it works

1. Take the original function
2. Create "mutants" — small changes that might introduce bugs
3. Run all generated test cases against each mutant
4. If a mutant passes all tests, the tests are weak at that point

### Mutation operators

| Original | Mutant | What it tests |
|---|---|---|
| `lt` | `lte` | Off-by-one in comparisons |
| `lte` | `lt` | Off-by-one in comparisons |
| `eq` | `neq` | Inverted conditions |
| `+ 1` | `- 1` | Arithmetic direction |
| `+ 1` | `+ 0` | Boundary adjustment |
| `return mid` | `return mid + 1` | Off-by-one in results |
| `return -1` | `return 0` | Wrong default value |

### Example output

```
crow test

  binary_search: 10/10 edge cases passed ✓
  mutation testing:
    mutant 1: changed 'lt' to 'lte' on line 7 — KILLED by test case 4 ✓
    mutant 2: changed 'lte' to 'lt' on line 4 — KILLED by test case 2 ✓
    mutant 3: changed '+ 1' to '- 1' on line 9 — KILLED by test case 6 ✓
    mutant 4: changed 'mid' to 'mid + 1' on line 6 — KILLED by test case 1 ✓
    mutant 5: changed '-1' to '0' on line 12 — KILLED by test case 3 ✓

  mutation score: 5/5 mutants killed ✓ (100%)
```

If a mutant survives:

```
  mutant 6: changed 'lte' to 'lt' on line 4 — SURVIVED
    → no test case distinguishes original from mutant
    → try: binary_search[[1], 1] — original returns 0, mutant returns -1
    → suggested test case added to next run
```

The system tells you **exactly** what's undertested and suggests the missing test case.

---

## Feature 6: Regression Snapshots

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Accidental behavior changes

### First run — snapshot created

```
crow test

  binary_search: 10 cases generated, all passed ✓
  snapshot saved → .crow/snapshots/binary_search.snap
```

The snapshot records every input-output pair:

```
--- .crow/snapshots/binary_search.snap ---
binary_search[[], 5] = -1
binary_search[[5], 5] = 0
binary_search[[5], 3] = -1
binary_search[[1, 2, 3], 1] = 0
binary_search[[1, 2, 3], 3] = 2
...
```

### Later — behavior change detected

You edit the function. Run `crow test` again:

```
crow test

  binary_search: REGRESSION
    binary_search[[1, 1, 1], 1] was 1, now 0
    changed on line 8

  accept new behavior? [y/n]
```

If you type `y`, the snapshot updates. If you type `n`, you know you introduced a bug.

### Why this is powerful

- **No test file to maintain.** The snapshot IS the test.
- **Catches subtle changes.** Even if the function still "works", any behavior difference is flagged.
- **Works across refactors.** Rename variables, restructure code — if the outputs match, it passes.
- **Version controlled.** `.crow/snapshots/` goes in git. Team members see regressions too.

---

## Feature 7: Parallel Safety Verification

**Command:** `crow check`
**Effort:** Zero
**What it catches:** Data races in automatically parallelized code

Crow parallelizes `for` loops and independent statements automatically. The compiler must verify this is safe.

### Already implemented

Crow already detects outer-scope mutation in `for` loops and falls back to sequential:

```
total = 0
for x in items
  total = total + x    --- mutates outer 'total' → sequential ✓
```

### Extended analysis

```
crow check

  main:
    a = compute[x]     --- independent ┐
    c = compute[y]     --- independent ┘ parallel ✓
    b = compute[a]     --- depends on a → sequential after a ✓
    d = a + b + c      --- depends on a, b, c → sequential after all ✓

  parallel waves:
    wave 1: [a, c]     --- parallel
    wave 2: [b]        --- sequential (needs a)
    wave 3: [d]        --- sequential (needs a, b, c)

  no data races detected ✓
```

### What it checks

| Hazard | Description | Example |
|---|---|---|
| RAW | Read-after-write — reading a value that's being computed in parallel | `a = 1; b = a + 1` (b reads a) |
| WAW | Write-after-write — two parallel statements writing the same variable | `a = 1; a = 2` |
| WAR | Write-after-read — writing a variable that's being read in parallel | `b = a; a = 5` |

All three are already detected by `src/parallel.rs`. This feature just surfaces the analysis to the developer.

---

## Summary

| Feature | Phase | Effort | Catches |
|---|---|---|---|
| Crash-proof analysis | Static | Zero | Index OOB, div by zero, type errors |
| Purity detection | Static | Zero | Side effects, testability classification |
| Termination proof | Static | Zero | Infinite loops |
| Parallel safety | Static | Zero | Data races |
| Edge case generation | Dynamic | Zero | Boundary bugs, off-by-one, empty inputs |
| Mutation testing | Dynamic | Zero | Weak test coverage |
| Regression snapshots | Dynamic | Zero | Accidental behavior changes |

All run under one command: `crow check`.

### Implementation priority

| Priority | Feature | Complexity | Value |
|---|---|---|---|
| 1 | Purity detection | Low | Foundation for everything else |
| 2 | Edge case generation | Medium | Highest developer-visible value |
| 3 | Regression snapshots | Low | Easy win, huge practical value |
| 4 | Crash-proof analysis | High | Strongest safety guarantee |
| 5 | Mutation testing | Medium | Makes edge case generation trustworthy |
| 6 | Termination proof | Medium | Nice to have, hard edge cases |
| 7 | Parallel safety (extended) | Low | Already mostly implemented |

### Design principles

1. **Zero effort.** No annotations, no contracts, no test files. The code is the spec.
2. **One command.** `crow check`. That's it.
3. **Helpful, not blocking.** Warnings, not errors. The developer decides what to fix.
4. **Incremental.** Each feature works independently. Ship them one at a time.
5. **Fast.** `crow check` should run in under a second for typical projects. `crow test` in under 10 seconds.

### What no other language does

- **Dafny/SPARK** require contracts (pre/post conditions). Crow doesn't.
- **Rust** requires type annotations and lifetime management. Crow doesn't.
- **Haskell** requires monads to separate pure from impure. Crow detects it automatically.
- **QuickCheck** requires you to write properties. Crow generates tests from the code itself.
- **Mutation testing tools** (Stryker, mutmut) require an existing test suite. Crow generates its own.

Crow would be the first language where `crow test` with zero test files gives you edge case coverage, mutation testing, and regression snapshots — all from the code alone.
