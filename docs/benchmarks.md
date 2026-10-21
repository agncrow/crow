# Benchmarks

*Measured by `crow test --bench --write` on 2026-10-06, Apple M4 Max (aarch64), commit 8cb850b. The interpreter is `crow run -i`; compiled is the release build `crow build` makes of the same program, timed from inside the program so a build is never counted. Sizes: full, 1000000 csv rows. The programs are in `tests/bench/`; `crow test --bench --quick` runs them at small sizes in CI so they cannot rot.*

| Program | Section | Interpreter | Compiled | Compiled is |
|---|---|---|---|---|
| collect | push_loop | 3.6ms | 32µs | ×112.9 faster |
| collect | for_collect | 43.8ms | 2.0ms | ×21.5 faster |
| collect | for_fn | 174.6ms | 2.3ms | ×77.4 faster |
| collect | map_set_loop | 5.8ms | 2.4ms | ×2.4 faster |
| collect | join_split | 21.6ms | 12.7ms | ×1.7 faster |
| collect | sort | 1.4ms | 527µs | ×2.7 faster |
| collect | json_roundtrip | 6.2ms | 2.8ms | ×2.2 faster |
| collect | whole program | 271.7ms | 32.9ms | ×8.3 faster |
| frames | csv_load | 46.8ms | 37.7ms | ×1.2 faster |
| frames | filter | 6.4ms | 6.1ms | ×1.1 faster |
| frames | group | 3.2ms | 3.6ms | ×1.1 slower |
| frames | sort | 17.8ms | 18.1ms | ×1.0 slower |
| frames | join | 9.8ms | 10.6ms | ×1.1 slower |
| frames | select | 11µs | 3µs | ×3.8 faster |
| frames | column_sum | 5.9ms | 1.4ms | ×4.1 faster |
| frames | csv_save | 37.9ms | 35.4ms | ×1.1 faster |
| frames | whole program | 193.8ms | 130.4ms | ×1.5 faster |
| tensors | matmul | 9.6ms | 7.1ms | ×1.4 faster |
| tensors | elementwise_chain | 3.2ms | 3.3ms | ×1.0 slower |
| tensors | dot | 7.1ms | 7.1ms | ×1.0 slower |
| tensors | sum | 1.4ms | 1.4ms | ×1.0 faster |
| tensors | argsort | 1.21s | 1.19s | ×1.0 faster |
| tensors | gram | 9.2ms | 7.9ms | ×1.2 faster |
| tensors | inverse_300 | 6.4ms | 3.3ms | ×1.9 faster |
| tensors | whole program | 1.51s | 1.49s | ×1.0 faster |
| train | linreg_gd | 168.7ms | 175.3ms | ×1.0 slower |
| train | mlp_forward | 8.1ms | 8.5ms | ×1.0 slower |
| train | grad_calls | 34.8ms | 33µs | ×1057.1 faster |
| train | whole program | 228.8ms | 198.5ms | ×1.2 faster |

## Notes

- **2026-10-21, the interpreter's speed, lost for three days and
  kept.** Phase 1 of the compiler's freeze (e967633) had turned the
  lists the interpreter reads after every builtin call — whether the
  answer is stamped, whether a list grows in place, whether a rehearsal
  records the call — into scans of the whole table by name. Re-measuring
  the benchmarks game programs against Rust, Go and CPython found n-body
  interpreted at 8.0 s for a million steps where 0.7.0 took 2.0 s, and
  k-nucleotide and reverse complement at twice their time, with every
  suite passing. A name is looked up in an index built once now
  (d3ee350): n-body 2.13 s, k-nucleotide 0.26 s, reverse complement
  0.55 s, spectral norm 6.57 s (medians of three runs, Apple M4 Max);
  the compiled programs were unchanged. A unit test measures the index
  against a scan in the same run. The table above is still the
  2026-10-06 recording.
- **2026-10-01, element-wise chains in one pass.** `tensors`'
  `elementwise_chain` (`x * 2.0 + x .* x - 1.0` over ten million
  elements) is one call of the fused kernel (second-half-design.md §3):
  6.1 ms interpreted and 5.9 ms compiled, where the operators, each
  making a tensor of its own, took 37.6 ms and 34.2 ms
  (`CROW_NO_FUSE=1`, the same build, back to back). The table above is
  still the 2026-09-23 recording.

- **2026-09-23, one rule for parallel loops.** A loop runs in parallel
  when nothing its body does can show the order (spec §14, For Loops),
  decided once in the lowering. Before, the interpreter kept any loop
  that called a function the program defined in order. The new
  `for_fn` section of `collect` (a million items through a pure
  function), both binaries back to back on the full sizes: interpreter
  892.6 ms before, 259.4 ms after (×3.4); compiled 1.6 ms and 1.5 ms,
  unchanged, since it ran such loops in parallel already. Every other
  section moved within the run-to-run noise of a recording. The table
  above is the recording at b5d5008, the first with the section.
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
