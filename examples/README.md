# Examples

Runnable Crow programs. Every file here builds in compiled mode and runs
in the interpreter (`crow run file.crow` / `crow run -i file.crow`), except
where noted. The book (`docs/book`) teaches the language in order; these
are the programs to read once you know the basics.

## Applications

| File | Shows |
|---|---|
| `dashboard.crow` + `dashboard_seed.crow` | The book's final project — a live SQLite-backed trading dashboard: views, reactive state, frames, `fmt`. Seed first, then run. |
| `counter.crow` | The smallest LiveView app: one reactive variable, two handlers, one view. |
| `layout_dashboard.crow` | The layout vocabulary: `stack` / `row` / `grid` / `overlay` / `scroll` with literal attributes instead of CSS, a phone rule, and `test` blocks that check positions, sizes, colours and overflow with `box[]`, `style[]`, `fits[]` — no browser. `crow test` it, then `crow run` it. |
| `db_demo.crow` | `use db` end to end on in-memory SQLite: create, insert, query, transactions. |
| `ml_demo.crow` | A small neural network with `grad`, `use math.nn`, and tensors. |
| `web_test.crow` | JSON routes with `use web` (serves; not run by the checks). |

## Language features

| File | Shows |
|---|---|
| `binary_search.crow` | `while`, indexing, a classic algorithm. |
| `error_value_test.crow`, `auto_propagation.crow`, `error_line.crow` | Errors as values: `.ok`/`.reason`/`.kind`, propagation through calls and operators, line reporting. |
| `first_class_test.crow` | Functions as values, passed by name. |
| `fmt_test.crow` | `fmt` with `decimals`, `commas`, `width`, `align`, `percent`. |
| `json_test.crow` | `json[]` / `encode[]`, open-schema data. |
| `module_test.crow`, `module_test2.crow`, `mymath.crow`, `multifile/`, `myapp/` | User modules, qualified access, subdirectory imports. (`mymath.crow` is a module, not a program.) |
| `mutation_test.crow`, `simple_mutation.crow` | Immutable-by-rebuild collections: `push`, `set`, reassignment. |
| `negative_index.crow`, `string_index.crow`, `range_test.crow` | Indexing, negative indices, `range`. |
| `operators_test.crow` | Word operators, `^`, `%`, precedence. |
| `then_chaining.crow` | `then` pipelines. |
| `type_builtin.crow` | `type[]`. |
| `p3_test.crow` | Regex, shell, hashing, file system builtins. |
| `feature_test.crow`, `test.crow`, `stdlib_test.crow`, `transpiler_test.crow` | Broad regression sweeps across builtins. |

## Parallelism

| File | Shows |
|---|---|
| `parallel_for_test.crow`, `parallel_test.crow`, `auto_parallel_test.crow` | `for` as a parallel expression, the sequential fallback on outer mutation, statement-level waves. |
| `sum_test.crow` | Reductions with `use math`. |
| `gpu_test.crow` | `gpu[]` placement (falls back to CPU without a GPU). |

## Data: tensors, frames, files

| File | Shows |
|---|---|
| `numarray_test.crow`, `math_test.crow`, `nn_test.crow`, `grad_test.crow` | Tensors, `use math`, `use math.nn`, autograd. |
| `frame_test.crow` | `use frame`: load, filter with masks, sort, group, join. |
| `io_test.crow`, `load_test.crow` | `use io` (`csv`, `lines`) and `math.load` for numeric CSV. |

## Benchmarks

`bench_*.crow` are the benchmarks-game programs (binary trees, fibonacci,
n-body, n-sieve, spectral norm, merkle trees) plus `bench_par`/`bench_seq`.
The `_15`/`_500k`/`_100k`/`_500` variants are the large-input versions —
run them compiled, not interpreted.
