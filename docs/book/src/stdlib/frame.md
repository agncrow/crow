# frame

Column-oriented data frames: load, select, filter, sort, group, join, and time handling.


Import with `use frame`. Column-oriented DataFrames for tabular data. See `docs/frame-module.md` for design details.

**Column references: strings for names, expressions for computation.** Naming a column is a string; computing on a column is a normal Crow expression over `df.col`. Every line is plain Crow — there is no special evaluation inside frame calls.

| Function | Description | Example |
|---|---|---|
| `new[]` | The empty frame (`frame.new[]`, since `new` reads best qualified); add columns with `with` | [Frames: `use frame`](../by-example/frames.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `with[frame, col, values]` | A frame with one more (or replaced) column: `values` is an array, a tensor, or a scalar repeated for every row; the length must match the rows (error-value otherwise) | [Frames: `use frame`](../by-example/frames.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `load[path]` | Load a table → frame, in the format its extension names — CSV, JSON records (columns in name order), Parquet — from a file, a URL or an S3 object (error-value on failure; Files, URLs and S3) | [Databases: `use db`](../by-example/db.md) · [Frames: `use frame`](../by-example/frames.md) · [Web: responses and submodules](../by-example/web-more.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `save[frame, path]` | Write frame → a file or an S3 object, as CSV, JSON records or Parquet by its extension (CSV otherwise); a failure is an error value of kind `'frame'`, as `load`'s is (compiled, it had stopped the program) | [Frames: `use frame`](../by-example/frames.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `select[frame, cols]` | Pick columns: `select[df, ['name', 'age']]` | [Frames: `use frame`](../by-example/frames.md) |
| `filter[frame, mask]` | Keep rows where mask is true: `filter[df, df.age gt 50]` | [Frames: `use frame`](../by-example/frames.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Views and Live Updates](../ch13-03-views.md) |
| `sort[frame, col]` | Sort ascending: `sort[df, 'age']`. Descending: `sort[df, 'age', desc = true]` | [Arrays](../by-example/arrays.md) · [Frames: `use frame`](../by-example/frames.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [Views and Live Updates](../ch13-03-views.md) |
| `group[frame, col, aggs...]` | Group + aggregate: `group[df, 'dept', ['salary', 'mean'], ['age', 'max']]` | [Frames: `use frame`](../by-example/frames.md) · [Project: Analyzing Data](../ch10-00-analyzing-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `join[f1, f2, key]` | Join on key column: `join[df1, df2, 'id']`. Type: `join[df1, df2, 'id', 'left']` | [Frames: `use frame`](../by-example/frames.md) · [Strings](../by-example/strings.md) · [Strings and Interpolation](../ch03-03-strings.md) |
| `stack[f1, f2]` | Stack frames vertically | [Frames: `use frame`](../by-example/frames.md) · [Math: tensors](../by-example/math.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `head[frame, n]` / `tail[frame, n]` | First/last n rows (returns a frame) | [Frames: `use frame`](../by-example/frames.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) |
| `fill[frame, col, value]` | Fill missing values in a column | [Frames: `use frame`](../by-example/frames.md) |
| `unique[frame, col]` | Unique values of a column: `unique[df, 'symbol']` | [Frames: `use frame`](../by-example/frames.md) · [Math: reductions and linear algebra](../by-example/math-linalg.md) |
| `rename[frame, old, new]` | Rename a column: `rename[df, 'salary', 'pay']` | [Frames: `use frame`](../by-example/frames.md) |
| `stats[frame]` | Summary statistics | [Frames: `use frame`](../by-example/frames.md) |
| `datetime[s]` | Parse ISO 8601 datetime; text that is no time is an error value of kind `'time'` | [Frames: `use frame`](../by-example/frames.md) |
| `tz[d, offset]` | Convert timezone: `tz[d, '+09:00']`; the offset is `'UTC'` or `'+HH:MM'` / `'-HH:MM'`, and text or an offset that is no time is an error value of kind `'time'` | [Frames: `use frame`](../by-example/frames.md) |

Properties: `.shape`, `.columns`, `.dtypes`. Column access: `df.age`. `len[df]` returns the row count. Aggregation names for `group`: `'sum'`, `'mean'`, `'min'`, `'max'`, `'count'`. Join types: `'inner'` (the default), `'left'`, `'right'`, `'outer'`.

A column the frame lacks, an aggregation or a join it does not know, a mask of another length than the rows, a negative count for `head` or `tail`, a `fill` value of another kind than the column's (an int fills a float column), and a count of arguments the function does not take stop the program with the reason (see When a Program Stops), in both modes, as `df.col` of a missing column does. A column copied with `with` keeps its missing cells.

**A frame's type is its columns.** A frame built in the program (`frame.new[]`, `with` given a column's name), a query a database described (see db, Typed cells), and a frame derived from either through `select`, `filter`, `sort`, `head`, `tail`, `fill`, `stack`, `join`, `rename` or `group` carries its columns in its type, each with its cells' type (a column `with` copies keeps its cells' type). A column named by a variable (`with[df, col, xs]`) leaves the columns the type knows and makes the frame *open*: a column it cannot name may be there. A reassignment binds the frame anew: after `df = with[df, 'x', xs]`, `df` has `x` from that line on, and before it the frame it was. Where branches, or a loop's rounds, meet, a frame has the columns they all give it; a column only some add may be there, and is read as the program runs. A function handed frames of different columns is a copy per set of columns, so it reads the columns each call hands it. Where a frame's columns are all known, a column it lacks — read as `df.x`, named where `select`, `sort`, `fill`, `rename`, `group` or `join` takes a column, or named by a table (`key`, `mark`, `delta`, `hide`) or a chart (`candles`, `lines`, `bars_y`) — is refused before the program runs, in both modes, with the columns the frame has (`frame has no property or column 'cost' — its columns are 'price'`, and, read in a function, the parameter the frame was handed as); the stop above is for a frame whose columns show only as it runs (a file `load` reads then, a query no database described, a column a variable names).

**Design Note: a frame's reassignment is a new binding.** Considered: one type per variable, as every other variable has, the frame's columns joined over its assignments — which loses exactly the columns a program adds (`df = with[df, 'x', xs]`, then `df.x`); and a name per shape (`df2 = with[df, …]`), which asks the program to rename what it changes, and makes a loop that adds columns impossible to write. Decided (Vijay, 2026-09-29): a frame variable's type is its last assignment's — a new binding where its columns change, as a Rust `let` shadows one — and where ways through meet it has the columns they share.

**Design Note: a frame operation's misuse stops the program.** Considered: error values of kind `'frame'`, which the table of kinds listed for `filter`, `sort`, `group` and `join`, and which the interpreter answered. Rejected: the operations are typed as giving a frame, which cannot fail, and the compiled program stopped — so a program that checked `.ok` worked in one mode only — and the same column the frame lacks, read as `df.col`, stops the program. A misuse is the program's mistake, not something it reads: an error value would carry it to wherever a decision first met it. Reading a file, `load`, and values from elsewhere, `with`, stay error values.

### Boolean Masks

Filtering uses ordinary Crow expressions that evaluate to boolean arrays. These operations are vectorized (element-wise):

- `eq ne gt lt gte lte` — column vs scalar → boolean array (the scalar may come first: `50 lt df.age`)
- `and` / `or` — two boolean arrays → boolean array
- `not` — boolean array → boolean array

```
use frame
df = load['people.csv']

old = filter[df, df.age gt 50]
alices = filter[df, df.name eq 'alice']
rich_old = filter[df, df.age gt 50 and df.salary gt 100000]
others = filter[df, not (df.name eq 'alice')]
```

Each cell is compared with the value as two values compare (see Comparison): numbers by value, exactly — NaN holds only `ne` — text by its characters and ordered by character code, bools equal or not, a value of another kind unequal. An order between kinds with none — `df.name gt 2`, `gt` on a column of bools — is refused where the column's type is known, and stops the program where it shows only as it runs (a file `load` reads). Missing values compare as `false` for every operator, `ne` included — they are excluded by any filter, and `not (df.age gt 50)` is what keeps them. A list compared with one value is a mask the same way. (Compiled, a missing or NaN cell had been `false` for `ne`, `eq` had been nearness within 2⁻⁵², a column of text had been parsed as numbers and a column of bools had compared as nothing; the interpreter had counted a missing cell as `ne` and stopped on an order of text; a list of values that can fail, compared with one value, had not built.)

**A filter's predicate is computed in one pass.** `filter[df, df.age gt 50 and df.name ne 'bob']` — a predicate that compares the frame's own columns with values that only read, asks a column's `.ok`, and combines those with `and`, `or` and `not` — reads no column into a list and makes no mask per comparison: the rows are decided block by block, across the cores for a large frame, each cell as its comparison decides it. The rows kept are the mask's, and a column the frame lacks or an order between kinds with none stops with the operator's words at the first comparison that meets it. The same in both modes.

A column's `.ok` is its rows' validity — `true` where the cell is present, `false` where it is missing — so `filter[df, df.age.ok]` keeps the complete rows. (Every other value's `.ok` is its own status: a list that did not fail is ok.) A column shows a missing cell as `nan`. Read one at a time (`df.age[3]`), a cell of a column that can have missing cells is fallible: a missing one is an error value of reason `missing`, which computes and cannot be decided on (§10), so `.ok` guards it and a guard narrows it to its value. The columns that can are a database column that can hold NULL (see db, Typed cells) and the side of a `join` a row can have no match on — a left join's right columns, a right join's left ones, all of an outer join's. `fill[df, 'age', 0]` gives a column a value where it had none, and its cells are plain again, as are a column's that cannot have missing cells. A mask decides every row, so a list of values that can fail — built from such cells — is refused as one, in both modes: fill the column first, or guard each value with `.ok`. Handed whole to a function that takes numbers — `mean[df.age]`, `median`, `sma[bars.c, 3]` — or used in arithmetic — `df.price * 2`, `df.a + df.b` — a column is its numbers, as a list of numbers is, and the answer is a tensor; a missing cell stops the program (`row 3 of the column is missing — fill the column first: fill[frame, 'column', value]`), and so does a column of text or of bools, in both modes. (Compiled, a missing cell had been read as `0` and a column of text parsed, where the interpreter stopped — or, for `df.a + df.b`, refused the sum.) A comparison is a mask, as above. A column copied with `with` keeps its kind and its missing cells, even when every cell is missing.

**Design Note: a missing cell is an error, not a placeholder.** Considered: reading a missing cell as its column's placeholder in both modes — `NaN` in a number column, `0` in an int one, `false` in a bool one, `nan` in text — as a filter reads it, which both modes did. Honest for a number, since NaN compares as false; for an int or a bool it is a guess the program cannot tell from a value, of the kind §10 refuses elsewhere (an unmatched row's count read as `0`). Decided (Vijay, 2026-09-25, typed frames decision 3): where the schema says a column can have missing cells its cells are fallible, and where it says it cannot they are plain, so no cell is a guess.

Dynamic column names need no special syntax — a string variable works everywhere a column name does:

```
col = 'age'
sorted = sort[df, col]
```

### Design Note: No Bare-Name Column References

**What was considered (and originally shipped):** Bare identifiers as column names — `sort[df, age]`, `group[df, dept, [salary, mean]]` — where an identifier that isn't a defined variable is treated as a column-name string.

**Why it was removed:**

1. **Scope-dependent meaning.** Adding a local variable named `age` silently changed `sort[df, age]` from a column reference into a value argument. Code broke at a distance with no error.
2. **Interpreter–transpiler divergence.** The interpreter preferred the variable when one existed; the transpiler force-converted certain argument positions to strings regardless. The same program meant different things in the two modes, violating the parity rule.
3. **Violates rule 19 (no DSL).** A second evaluation rule inside frame calls means a line of Crow can't be read without knowing which builtin it appears in.

**What to use instead:** Strings for column names, expressions for computation — as specified above. One mechanism, no sugar, dynamic columns free.



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
