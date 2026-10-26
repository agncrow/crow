# Examples

Runnable Crow programs. Every file here builds in compiled mode and runs
in the interpreter (`crow run file.co` / `crow run -i file.co`), except
where noted. The book (`docs/book`) teaches the language in order; these
are the programs to read once you know the basics.

## Applications

| File | Shows |
|---|---|
| `dashboard.co` + `dashboard_seed.co` | The book's final project — a live SQLite-backed trading dashboard: views, reactive state, frames, `fmt`. Seed first, then run. |
| `counter.co` | The smallest LiveView app: one reactive variable, two handlers, one view. |
| `taskboard/` | The second reference app, a team's task board, built as parts from the element vocabulary and one widget. People sign in against stored password hashes (`reader`, `sign_in`, `hash`); an admin makes boards; everyone adds, moves, rates and discusses cards, and sees the others' changes as they land; files go in through a file field and out with `download`; a paged log records who did what. From its folder, `crow run main.co` serves it on port 3300 (sign in as `ada`/`lovelace` or `bob`/`builder`), `crow test board/cards.co` (and `door.co`, `page.co`) runs its harness tests, and `crow test --served` from the repository root drives it in both modes. |
| `rag/` | The third reference app, a RAG chat over the notes under its `docs/`: the chunks embedded once through a model and indexed (`packages/vectors`), a question's chunks found by the vectors' nearest and BM25's best fused by reciprocal rank (`packages/retrieval`), and the model's answer with its citations (`use llm`). From its folder, `crow run main.co` serves it on port 3400 against Ollama (or the model `RAG_MODEL` and `RAG_EMBEDDER` name); `crow test --fixtures chat/page.co` runs the page's tests from the recorded fixtures, with no model. |
| `agent/` | The fourth reference app, a support agent over a shop's orders in SQLite (`use llm`'s `act`): three tools, the refund behind the program's gate, every run written to the database as a trace — a row per run, a row per step — and a page over the traces with their tokens and latency charted. From its folder, `crow run main.co` serves it on port 3530 against a local mlx-lm server (or the model `AGENT_MODEL` names); `crow test --fixtures agent/support.co` runs its evals from the recorded fixtures with no model — exact checks on the steps frame and a model-as-judge — and `--fixtures agent/chat.co` / `agent/trace.co` the pages', `--compiled` too. |
| `lake/` | The fifth reference app, a page over a log (`use lake`): orders land as batches, the page reads the table the log holds, a search over the notes and the log's figures, each a derivation that follows the writes. From its folder, `crow run main.co` serves it on port 3600; `crow test --served lake` from the repository root drives it in both modes. |
| `layout_dashboard.co` | The layout vocabulary: `stack` / `row` / `grid` / `overlay` / `scroll` with literal attributes instead of CSS, a phone rule, and `test` blocks that check positions, sizes, colours and overflow with `box[]`, `style[]`, `fits[]` — no browser. `crow test` it, then `crow run` it. |
| `db_demo.co` | `use db` end to end on in-memory SQLite: create, insert, query, transactions. |
| `ml_demo.co` | A line fitted by `grad`, then a small network learning XOR with `use math.nn`, its backpropagation written out over tensors. |
| `web_test.co` | Routes answering `text`, `html`, `redirect` and `respond` with `use web` (serves on port 3000; not run by the checks). |

## Language features

| File | Shows |
|---|---|
| `binary_search.co` | `while`, indexing, a classic algorithm. |
| `error_value_test.co`, `auto_propagation.co`, `error_line.co` | Errors as values: `.ok`/`.reason`/`.kind`, propagation through calls and operators, line reporting. |
| `first_class_test.co` | Functions as values, passed by name. |
| `fmt_test.co` | A program `crow fmt` leaves as it is: the formatter's own sample. |
| `json_test.co` | `json[]` / `encode[]`, open-schema data. |
| `module_test.co`, `module_test2.co`, `mymath.co`, `multifile/`, `myapp/` | User modules, qualified access, subdirectory imports. (`mymath.co` is a module, not a program.) |
| `mutation_test.co`, `simple_mutation.co` | Writes in place: a record's field (`p.x = 99`), a list's item and a map's key (`nums[0] = 99`, `m['a'] = 99`), and `set` assigned back. |
| `negative_index.co`, `string_index.co`, `range_test.co` | Indexing, negative indices, `range`. |
| `operators_test.co` | `^`, grouped from the right (`2 ^ 3 ^ 2` is 512), the element-wise `.*`, `./` and `.^`, and `*` of two matrices as their product. |
| `then_chaining.co` | `then` pipelines. |
| `type_builtin.co` | `type[]`. |
| `p3_test.co` | Shell, hashing, regex and moments. |
| `feature_test.co`, `test.co`, `stdlib_test.co`, `transpiler_test.co` | Broad regression sweeps across builtins. |

## Parallelism

| File | Shows |
|---|---|
| `parallel_for_test.co`, `parallel_test.co`, `auto_parallel_test.co` | `for` loops: a collecting loop that only computes runs on every core, and one whose body writes an outer variable or logs runs in order; `cpu[n]`; and independent statements, which run in the order written (spec section 14). |
| `sum_test.co` | Reductions with `use math`. |
| `gpu_test.co` | `gpu[]` placement (falls back to CPU without a GPU). |

## Data: tensors, frames, files

| File | Shows |
|---|---|
| `numarray_test.co`, `math_test.co`, `nn_test.co`, `grad_test.co` | Tensors, `use math`, `use math.nn`, autograd. |
| `frame_test.co` | `use frame`: `load`, `select`, `head`, `sort`, `rename`, `stats`, `fill` and a column read, over `test_data.csv` — it names the file as `examples/test_data.csv`, so run it from the repository root. |
| `io_test.co`, `load_test.co` | `use io` (`csv`, `lines`) and `math.load` for numeric CSV. |

## Benchmarks

`bench_*.co` are the benchmarks-game programs (binary trees, n-body,
n-sieve, spectral norm, merkle trees), plus `bench_fibonacci` (recursion),
`bench_helloworld` (a program's start), `bench_math` (the math module's
kernels, beside `bench_math.py`, the NumPy program it is compared with)
and `bench_par`/`bench_seq` (the same eight prime counts on `cpu[8]` and
on `cpu[1]`). The `_15`/`_500k`/`_100k`/`_500` variants are the
large-input versions — run them compiled, not interpreted.
