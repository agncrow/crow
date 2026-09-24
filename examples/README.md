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
| `layout_dashboard.co` | The layout vocabulary: `stack` / `row` / `grid` / `overlay` / `scroll` with literal attributes instead of CSS, a phone rule, and `test` blocks that check positions, sizes, colours and overflow with `box[]`, `style[]`, `fits[]` — no browser. `crow test` it, then `crow run` it. |
| `db_demo.co` | `use db` end to end on in-memory SQLite: create, insert, query, transactions. |
| `ml_demo.co` | A small neural network with `grad`, `use math.nn`, and tensors. |
| `web_test.co` | JSON routes with `use web` (serves; not run by the checks). |

## Language features

| File | Shows |
|---|---|
| `binary_search.co` | `while`, indexing, a classic algorithm. |
| `error_value_test.co`, `auto_propagation.co`, `error_line.co` | Errors as values: `.ok`/`.reason`/`.kind`, propagation through calls and operators, line reporting. |
| `first_class_test.co` | Functions as values, passed by name. |
| `fmt_test.co` | `fmt` with `decimals`, `commas`, `width`, `align`, `percent`. |
| `json_test.co` | `json[]` / `encode[]`, open-schema data. |
| `module_test.co`, `module_test2.co`, `mymath.co`, `multifile/`, `myapp/` | User modules, qualified access, subdirectory imports. (`mymath.co` is a module, not a program.) |
| `mutation_test.co`, `simple_mutation.co` | Immutable-by-rebuild collections: `push`, `set`, reassignment. |
| `negative_index.co`, `string_index.co`, `range_test.co` | Indexing, negative indices, `range`. |
| `operators_test.co` | Word operators, `^`, `%`, precedence. |
| `then_chaining.co` | `then` pipelines. |
| `type_builtin.co` | `type[]`. |
| `p3_test.co` | Regex, shell, hashing, file system builtins. |
| `feature_test.co`, `test.co`, `stdlib_test.co`, `transpiler_test.co` | Broad regression sweeps across builtins. |

## Parallelism

| File | Shows |
|---|---|
| `parallel_for_test.co`, `parallel_test.co`, `auto_parallel_test.co` | `for` as a parallel expression, the sequential fallback on outer mutation, statement-level waves. |
| `sum_test.co` | Reductions with `use math`. |
| `gpu_test.co` | `gpu[]` placement (falls back to CPU without a GPU). |

## Data: tensors, frames, files

| File | Shows |
|---|---|
| `numarray_test.co`, `math_test.co`, `nn_test.co`, `grad_test.co` | Tensors, `use math`, `use math.nn`, autograd. |
| `frame_test.co` | `use frame`: load, filter with masks, sort, group, join. |
| `io_test.co`, `load_test.co` | `use io` (`csv`, `lines`) and `math.load` for numeric CSV. |

## Benchmarks

`bench_*.co` are the benchmarks-game programs (binary trees, fibonacci,
n-body, n-sieve, spectral norm, merkle trees) plus `bench_par`/`bench_seq`.
The `_15`/`_500k`/`_100k`/`_500` variants are the large-input versions —
run them compiled, not interpreted.
