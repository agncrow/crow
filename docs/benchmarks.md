# Benchmarks

*Measured by `crow test --bench --write` on 2026-09-07, Apple M4 Max (aarch64), commit 38218e7. The interpreter is `crow run -i`; compiled is the release build `crow build` makes of the same program, timed from inside the program so a build is never counted. Sizes: full, 1000000 csv rows. The programs are in `tests/bench/`; `crow test --bench --quick` runs them at small sizes in CI so they cannot rot.*

| Program | Section | Interpreter | Compiled | Compiled is |
|---|---|---|---|---|
| collect | push_loop | 5.7ms | 46µs | ×124.5 faster |
| collect | for_collect | 147.9ms | 1.9ms | ×76.0 faster |
| collect | map_set_loop | 14.3ms | 1.3ms | ×11.3 faster |
| collect | join_split | 95.8ms | 9.4ms | ×10.2 faster |
| collect | sort | 60.8ms | 511µs | ×119.0 faster |
| collect | json_roundtrip | 32.8ms | 1.5ms | ×21.5 faster |
| collect | whole program | 387.8ms | 158.9ms | ×2.4 faster |
| frames | csv_load | 66.4ms | 73.6ms | ×1.1 slower |
| frames | filter | 94.2ms | 17.2ms | ×5.5 faster |
| frames | group | 11.7ms | 9.5ms | ×1.2 faster |
| frames | sort | 142.3ms | 67.4ms | ×2.1 faster |
| frames | join | 41.5ms | 29.7ms | ×1.4 faster |
| frames | select | 53µs | 11µs | ×4.8 faster |
| frames | column_sum | 41.3ms | 1.9ms | ×21.4 faster |
| frames | csv_save | 42.9ms | 37.2ms | ×1.2 faster |
| frames | whole program | 708.6ms | 666.2ms | ×1.1 faster |
| tensors | matmul | 24.9ms | 24.6ms | ×1.0 faster |
| tensors | elementwise_chain | 124.2ms | 139.4ms | ×1.1 slower |
| tensors | dot | 15.8ms | 11.5ms | ×1.4 faster |
| tensors | sum | 3.9ms | 3.8ms | ×1.0 faster |
| tensors | argsort | 2.26s | 2.34s | ×1.0 slower |
| tensors | gram | 10.1ms | 11.2ms | ×1.1 slower |
| tensors | inverse_300 | 4.9ms | 4.5ms | ×1.1 faster |
| tensors | whole program | 2.47s | 2.80s | ×1.1 slower |
| train | linreg_gd | 160.0ms | 155.7ms | ×1.0 faster |
| train | mlp_forward | 10.0ms | 9.4ms | ×1.1 faster |
| train | grad_calls | 49.0ms | 33µs | ×1477.4 faster |
| train | whole program | 232.7ms | 419.9ms | ×1.8 slower |

## Notes

Findings from the first day of measuring, 2026-09-07. Each is a lead
for the core programme (`core-rewrite-notes.md` §9); none is a
threshold the suite enforces.

**What the first table said, and what one day changed.** The first
full run put the interpreter's collect program at 111 s. Three copies
were behind it, all found by the table, all gone by the third run:

| Section (interpreter) | First run | After | What was copied |
|---|---|---|---|
| push_loop (10k) | 2.58 s | 5.7 ms | the list on every push, and the value once more when stored |
| map_set_loop (10k) | 6.09 s | 14 ms | the map on every set, likewise |
| for_collect (1M) | 13.66 s | 148 ms | the whole interpreter, per item, in every parallel loop |
| join_split (200k) | 48.4 s | 96 ms | the same: a loop after a large array copied that array per item |
| sort (20k) | 32.4 s | 61 ms | the same; the sort itself takes 80 µs |
| json_roundtrip (5k) | 7.81 s | 33 ms | the same |
| whole program | 111 s | 388 ms | |

The compiled program's `push` and `set` cloned too (7.1 ms and 1.19 s
for the two loops); they take 46 µs and 1.3 ms now. Both modes call
one runtime function that takes the collection by value, and
`x = push[x, v]` moves it in (`core-runtime-builtins.md`).

- **The interpreter's parallel `for` cloned the whole interpreter per
  item.** Every value in scope was copied once per iteration, so any
  loop after a large array existed was quadratic in that array; the
  "slow sort" and "slow JSON" of the first table were this. Scopes are
  reference-counted and copied on write now, and the clone is per
  chunk of items.
- **What is left in the interpreter is per-statement cost**: about
  150 ns per iteration of a collecting loop, 1.5 µs per push. Frames
  and tensors are equal in both modes because both call the same
  runtime; `argsort` of ten million floats costs 2.3 s in either, and
  is the runtime's to improve. Two frame sections stand out where the
  interpreter is slower for no runtime reason: `filter` (5×) and a
  column `sum` (21×) go through per-value paths the compiled program
  does not take.
- **A compiled program starts slower than it runs**: the training
  program's whole-process time is 420 ms against 233 ms interpreted,
  while every section inside it is even. The GPU device is probed at
  start; a program that never asks for it should not pay.
- **Bugs the programs found**, each fixed with a test: `*` between two
  1-D tensors panicked (`try_matmul`); `len[]` of a parsed JSON value
  did not compile (`json::len`, parity 40); the compiled `slice`
  indexed without clamping and panicked out of range; the compiled
  `pop` of an empty list was silent where the interpreter errored; the
  compiled `range` could not count down (all three in `core`, parity
  41).
- **Not measured yet:** a live app under concurrent sessions and a
  derivation over a growing table. Both wait for the harness to run
  handlers as transactions (core-design.md §1), which is when their
  cost changes.
