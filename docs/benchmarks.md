# Benchmarks

*Measured by `crow test --bench --write` on 2026-09-07, Apple M4 Max (aarch64), commit c60af9a. The interpreter is `crow run -i`; compiled is the release build `crow build` makes of the same program, timed from inside the program so a build is never counted. Sizes: full, 1000000 csv rows. The programs are in `tests/bench/`; `crow test --bench --quick` runs them at small sizes in CI so they cannot rot.*

| Program | Section | Interpreter | Compiled | Compiled is |
|---|---|---|---|---|
| collect | push_loop | 2.58s | 7.1ms | ×364.8 faster |
| collect | for_collect | 13.66s | 1.5ms | ×9195.0 faster |
| collect | map_set_loop | 6.09s | 1.19s | ×5.1 faster |
| collect | join_split | 48.42s | 10.0ms | ×4863.1 faster |
| collect | sort | 32.37s | 410µs | ×78926.8 faster |
| collect | json_roundtrip | 7.81s | 1.5ms | ×5066.5 faster |
| collect | whole program | 111.04s | 1.47s | ×75.6 faster |
| frames | csv_load | 73.4ms | 78.2ms | ×1.1 slower |
| frames | filter | 87.8ms | 17.2ms | ×5.1 faster |
| frames | group | 11.9ms | 9.8ms | ×1.2 faster |
| frames | sort | 126.2ms | 100.8ms | ×1.3 faster |
| frames | join | 30.6ms | 31.2ms | ×1.0 slower |
| frames | select | 38µs | 14µs | ×2.7 faster |
| frames | column_sum | 14.9ms | 1.5ms | ×9.6 faster |
| frames | csv_save | 56.3ms | 41.2ms | ×1.4 faster |
| frames | whole program | 637.3ms | 782.3ms | ×1.2 slower |
| tensors | matmul | 20.7ms | 22.5ms | ×1.1 slower |
| tensors | elementwise_chain | 108.1ms | 106.4ms | ×1.0 faster |
| tensors | dot | 13.0ms | 14.6ms | ×1.1 slower |
| tensors | sum | 1.4ms | 3.3ms | ×2.3 slower |
| tensors | argsort | 2.29s | 2.21s | ×1.0 faster |
| tensors | gram | 11.4ms | 9.4ms | ×1.2 faster |
| tensors | inverse_300 | 5.0ms | 4.9ms | ×1.0 faster |
| tensors | whole program | 2.47s | 2.70s | ×1.1 slower |
| train | linreg_gd | 168.8ms | 159.4ms | ×1.1 faster |
| train | mlp_forward | 9.2ms | 9.8ms | ×1.1 slower |
| train | grad_calls | 50.6ms | 42µs | ×1198.5 faster |
| train | whole program | 242.9ms | 482.6ms | ×2.0 slower |

## Notes

Findings from the first full run, 2026-09-07. Each is a lead for the
core programme (`core-rewrite-notes.md` §9); none is a threshold the
suite enforces.

- **The interpreter copies on every write to a collection.** `push` and
  `set` clone the whole list or map (`interpreter.rs`, the `"push"` and
  `"set"` arms), so a loop that collects is quadratic: ten thousand
  pushes take 2.6 s, ten thousand map sets 6.1 s. The compiled program
  clones too (the spec's promised in-place rewrite of `x = push[x, v]`
  is not emitted) but a `Vec<i64>` memcpy is cheap enough to hide it at
  this size. One runtime function taking the collection by value, moved
  in when the statement reassigns the same variable, gives both modes
  the real thing (stage 3, `core-runtime-builtins.md`).
- **The interpreter's `sort` is not a sort.** Twenty thousand ints take
  32 s, five thousand take 2.7 s: linear in n at about 1.6 ms per
  element, which no comparison sort explains. The suspects are the
  value representation (a `CrowValue` carries its reason, kind, trace
  and capture moment beside its payload) and the comparator; to be
  settled when `sort` moves into the runtime.
- **Collecting a million items in the interpreter takes 13.7 s**, and
  joining and splitting two hundred thousand strings 48 s. Every item is
  a full `CrowValue`; growing a vector of them reallocates and copies
  the lot. The compiled program does the same work in 1.5 ms and 10 ms.
- **JSON in the interpreter is slow per record**: five thousand small
  records encoded and parsed back take 7.8 s, each passing through the
  dynamic value twice. The compiled program keeps JSON as a
  `serde_json::Value` and takes 1.5 ms.
- **Frames and tensors are equal in both modes** because both call the
  same runtime (columns, faer, rayon); the interpreter adds nothing
  measurable to a whole-column operation, and the `argsort` of ten
  million floats costs 2.2 s in either. Under `--quick` the compiled
  side is an unoptimised build, so its tensor numbers there mean
  nothing; the release table above is the one to read.
- **A compiled program starts slower than it runs**: the training
  program's whole-process time is 480 ms against 240 ms interpreted,
  while every section inside it is even or faster. The GPU device is
  probed at start; a program that never asks for it should not pay.
- **Two bugs the programs found on the first run**, both fixed with
  tests: `*` between two 1-D tensors panicked inside the runtime
  instead of stopping with a reason (`try_matmul`), and `len[]` of a
  parsed JSON value did not compile (`json::len`, parity program 40).
- **Not measured yet:** a live app under concurrent sessions and a
  derivation over a growing table. Both wait for the harness to run
  handlers as transactions (core-design.md §1), which is when their
  cost changes.
