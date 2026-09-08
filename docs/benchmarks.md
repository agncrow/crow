# Benchmarks

*Measured by `crow test --bench --write` on 2026-09-08, Apple M4 Max (aarch64), commit 372080f. The interpreter is `crow run -i`; compiled is the release build `crow build` makes of the same program, timed from inside the program so a build is never counted. Sizes: full, 1000000 csv rows. The programs are in `tests/bench/`; `crow test --bench --quick` runs them at small sizes in CI so they cannot rot.*

| Program | Section | Interpreter | Compiled | Compiled is |
|---|---|---|---|---|
| collect | push_loop | 6.0ms | 18µs | ×333.1 faster |
| collect | for_collect | 121.7ms | 2.1ms | ×57.4 faster |
| collect | map_set_loop | 9.4ms | 1.3ms | ×7.2 faster |
| collect | join_split | 37.1ms | 10.1ms | ×3.7 faster |
| collect | sort | 3.6ms | 461µs | ×7.9 faster |
| collect | json_roundtrip | 21.0ms | 2.4ms | ×8.7 faster |
| collect | whole program | 216.3ms | 23.9ms | ×9.1 faster |
| frames | csv_load | 69.3ms | 59.4ms | ×1.2 faster |
| frames | filter | 58.6ms | 15.8ms | ×3.7 faster |
| frames | group | 13.9ms | 8.9ms | ×1.6 faster |
| frames | sort | 65.4ms | 65.3ms | ×1.0 faster |
| frames | join | 27.2ms | 31.6ms | ×1.2 slower |
| frames | select | 6µs | 4µs | ×1.5 faster |
| frames | column_sum | 26.8ms | 1.7ms | ×15.8 faster |
| frames | csv_save | 39.8ms | 33.8ms | ×1.2 faster |
| frames | whole program | 501.0ms | 399.0ms | ×1.3 faster |
| tensors | matmul | 9.8ms | 8.6ms | ×1.1 faster |
| tensors | elementwise_chain | 33.5ms | 40.1ms | ×1.2 slower |
| tensors | dot | 7.9ms | 9.8ms | ×1.2 slower |
| tensors | sum | 1.5ms | 2.4ms | ×1.6 slower |
| tensors | argsort | 1.39s | 1.91s | ×1.4 slower |
| tensors | gram | 8.4ms | 8.1ms | ×1.0 faster |
| tensors | inverse_300 | 4.4ms | 3.2ms | ×1.4 faster |
| tensors | whole program | 1.54s | 2.07s | ×1.3 slower |
| train | linreg_gd | 163.0ms | 162.5ms | ×1.0 faster |
| train | mlp_forward | 8.8ms | 9.0ms | ×1.0 slower |
| train | grad_calls | 74.4ms | 39µs | ×1904.0 faster |
| train | whole program | 255.9ms | 179.3ms | ×1.4 faster |

## Notes

- **2026-09-08, the IR interpreter is the interpreter.** The
  interpreter column is the IR walker (`ir_interp.rs`) from this
  recording on; the AST interpreter is deleted. Against the last AST
  binary on the same machine every whole program is level or faster;
  by section the push loop, the map-set loop and join/split are faster
  (the in-place builtins, and a string-building loop that now runs in
  parallel: an interpolation reaches only what its spans reach), the
  plain collecting loop and the small sort are slower by an eighth and
  a fifth at their sizes. The column sum is level in isolation; a run
  right after the book's six workers measured it at twice, which is
  the machine, not the walker. This recording's machine was slower
  than the last one's in both columns (the tensor argsort by a third
  compiled and interpreted alike); the columns compare within a
  recording, not across them.

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
