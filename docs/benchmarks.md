# Benchmarks

*Measured by `crow test --bench --write` on 2026-09-08, Apple M4 Max (aarch64), commit 215534f. The interpreter is `crow run -i`; compiled is the release build `crow build` makes of the same program, timed from inside the program so a build is never counted. Sizes: full, 1000000 csv rows. The programs are in `tests/bench/`; `crow test --bench --quick` runs them at small sizes in CI so they cannot rot.*

| Program | Section | Interpreter | Compiled | Compiled is |
|---|---|---|---|---|
| collect | push_loop | 6.0ms | 17µs | ×356.4 faster |
| collect | for_collect | 127.7ms | 1.8ms | ×70.9 faster |
| collect | map_set_loop | 13.6ms | 1.3ms | ×10.3 faster |
| collect | join_split | 66.9ms | 9.8ms | ×6.8 faster |
| collect | sort | 3.4ms | 513µs | ×6.6 faster |
| collect | json_roundtrip | 20.9ms | 1.6ms | ×13.3 faster |
| collect | whole program | 269.1ms | 21.1ms | ×12.8 faster |
| frames | csv_load | 64.8ms | 61.4ms | ×1.1 faster |
| frames | filter | 76.3ms | 16.2ms | ×4.7 faster |
| frames | group | 10.9ms | 9.4ms | ×1.2 faster |
| frames | sort | 67.6ms | 65.4ms | ×1.0 faster |
| frames | join | 33.2ms | 29.4ms | ×1.1 faster |
| frames | select | 6µs | 2µs | ×3.1 faster |
| frames | column_sum | 14.5ms | 1.5ms | ×9.6 faster |
| frames | csv_save | 36.4ms | 33.8ms | ×1.1 faster |
| frames | whole program | 542.7ms | 421.8ms | ×1.3 faster |
| tensors | matmul | 8.6ms | 7.9ms | ×1.1 faster |
| tensors | elementwise_chain | 42.0ms | 40.2ms | ×1.0 faster |
| tensors | dot | 7.3ms | 7.4ms | ×1.0 slower |
| tensors | sum | 1.3ms | 1.3ms | ×1.0 faster |
| tensors | argsort | 1.14s | 1.14s | ×1.0 faster |
| tensors | gram | 9.6ms | 8.7ms | ×1.1 faster |
| tensors | inverse_300 | 3.3ms | 3.6ms | ×1.1 slower |
| tensors | whole program | 1.29s | 1.29s | ×1.0 faster |
| train | linreg_gd | 156.4ms | 148.2ms | ×1.1 faster |
| train | mlp_forward | 8.6ms | 8.8ms | ×1.0 slower |
| train | grad_calls | 48.7ms | 38µs | ×1276.5 faster |
| train | whole program | 223.7ms | 162.5ms | ×1.4 faster |

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
| whole program | 111 s | 269 ms | |

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
- **A freshly built binary's first run pays the loader**, about
  270 ms on this machine for any compiled program, hello world
  included, and nothing after that: the first tables charged it to the
  training and tensor programs' whole-process time. The runner now
  warms each binary once at the quick sizes before timing it.
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
