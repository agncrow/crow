# Frame Module Design

## Performance and memory

Measured 2026-10-06 on a 16-core Apple M4 Max: the same ten operations
over a 10-million-row, 659 MiB CSV (ten columns, four of text), in Crow
compiled (best of three), pandas 3.0 (single-threaded) and polars 2.0
(every core); wall seconds, and the peak memory of the whole run.

| Operation | Crow | pandas | polars |
|---|---|---|---|
| load CSV | 0.41 | 4.67 | 1.04 |
| filter | 0.07 | 0.24 | 0.05 |
| group, 3 aggregates | 0.04 | 0.34 | 0.03 |
| sort by a float | 0.35 | 2.04 | 0.53 |
| join, 8-row table | 0.13 | 0.49 | 0.22 |
| window, running sum | 0.07 | 0.31 | 0.16 |
| unique | 0.01 | 0.14 | 0.05 |
| save CSV, 3 columns | 0.40 | 4.96 | 0.12 |
| peak memory | 4.7 GB | 3.5 GB | 5.2 GB |

The table itself takes about 90 bytes a row here (polars: 74; pandas: 320
with its Python-object strings). Before the storage work of the same day
it took 220 bytes a row and the run peaked at 8.0 GB; load took 1.11 s,
filter 0.69, sort 1.56, join 0.81. What changed, in order of payoff:

1. Text is held as Arrow holds it, every cell's bytes in one buffer with
   an offset per cell, or — for a column of few distinct values, which a
   file's sample shows — as a code per cell into a dictionary. A group, a
   join, a sort, a filter for equality and `unique` work on the codes.
2. The compiled filter's one-pass predicate compares a block at a time,
   one loop per kind and operator (it had read every cell into a generic
   value: 3.6 times the interpreter's mask).
3. The CSV reader splits each line once (it had scanned every line once
   per column), parses pieces of the file across the cores straight into
   typed parts, decides each column's kind from a sample and reads a
   column whole again only where the sample misled, and reads the file's
   bytes in place rather than through a copy of the text.
4. Which cells are there is a bit per cell (`validity.rs`), as this
   document always said; it had been a byte.
5. A sort packs each cell into a key and sorts (key, row) pairs across
   the cores, which keeps equal cells in their order; a group by several
   keys packs their codes instead of allocating a vector per row.
6. A join looks a dictionary key up once per distinct value, and shares
   the left columns by reference where every left row matched once, in
   order.

A frame a database answers takes the same storage: the decoders of
ClickHouse's RowBinary and of Postgres's rows build text straight into it,
coded while a column's values stay few (decided at its first 256 cells),
where they had collected a string per cell and copied the column once
more. A read-only selection of 2.5 million bars over 510 symbols from
the local ClickHouse, decoded and worked on in the interpreter, same rows
each run, warm: the query 0.75 → 0.64 s with the transfer included,
group by symbol 0.051 → 0.020, sort by symbol 0.023 → 0.016, filter on a
symbol 0.005 → 0.003, unique 0.035 → 0.003, peak memory 725 → 605 MB.

Still slower than polars: saving CSV (3×), and filter and group by a
third. `tests/bench/frames.co` holds the repository's own numbers at a
million rows in both modes (docs/benchmarks.md).

---

## New `Value::Frame` Type

New `Value::Frame` variant with column-oriented storage.

```crow
use frame
df = load['people.csv']
type[df]    --- 'frame'
df.age      --- [30, 25, 42, ...]
df.shape    --- [1000, 5]
df.columns  --- ['name', 'age', 'city', 'salary', 'score']
```

- Clean separation — frame is its own thing
- Properties (.shape, .columns, .dtypes) without conflicting with column names
- Column order preserved (not alphabetically sorted like BTreeMap)
- Optimized internal storage: typed vectors (Vec<i64>, Vec<f64>, Vec<bool>, Vec<i64> + Vec<i16> for DateTime); text in one buffer with an offset per cell, or a code per cell into a dictionary (`textcol.rs`); a bit per cell for which are there (`validity.rs`)
- Display shows a nice table format

---

## Vectorized Column Expressions for Filtering

`filter[df, df.age gt 50]` where `df.age gt 50` returns a boolean array. Vectorized — operates on whole columns at once, no per-row allocation.

```crow
use frame
df = load['people.csv']

--- filter rows where age > 50
old = filter[df, df.age gt 50]

--- string equality
alices = filter[df, df.name eq 'alice']

--- compound conditions
rich_old = filter[df, df.age gt 50 and df.salary gt 100000]

--- negation
others = filter[df, not (df.name eq 'alice')]
```

Mask semantics (implemented in both interpreter and transpiler):
- `eq ne gt lt gte lte` on a column vs a scalar → boolean array, element-wise
- `and` / `or` on two boolean arrays → boolean array, element-wise
- `not` on a boolean array → boolean array, element-wise
- Missing values compare as `false` (excluded by the filter)

---

## Frame ↔ Tensor Conversion

Two forms of `tensor[]`:

```crow
use frame
use math

df = load['scores.csv']
--- df has columns: age(float), salary(float), score(float)

--- all-numeric frame → tensor
t = tensor[df]                        --- shape: [N, 3]

--- pick specific columns → tensor
t = tensor[df, [age, salary]]         --- shape: [N, 2]
```

Not duplicates — different use cases. `tensor[df]` errors on string columns to prevent silent data loss.

---

## Function List

Replaces `math.load` and `io.csv[]` — `load` is the single CSV entry point.

**Column references: strings for names, expressions for computation.**
Naming a column is a string (`sort[df, 'age']`). Computing on a column is a
normal Crow expression over `df.col` (`filter[df, df.age gt 50]`). Every line
is plain Crow — no special evaluation rules inside frame calls.

```crow
sort[df, 'age']                    --- name a column: string
sort[df, 'first name']             --- spaces, hyphens — no escape hatch needed
select[df, ['name', 'age']]        --- list of names: strings
filter[df, df.age gt 50]           --- compute on a column: expression

col = 'age'
sorted = sort[df, col]             --- dynamic column names just work
```

> **Design Note (July 2026):** Bare-name column references (`sort[df, age]`)
> were removed. The rule "an identifier that isn't a defined variable becomes a
> string" made meaning scope-dependent — adding a local variable named `age`
> silently changed `sort[df, age]` from a column reference to a value — and the
> interpreter and transpiler resolved the ambiguity differently, breaking
> parity. Strings are one mechanism with no sugar, per design rule 19 (no DSL:
> every line is valid Crow).

| Function | Description | Example |
|----------|-------------|---------|
| load | Load CSV → frame | `df = load['data.csv']` |
| save | Write frame → CSV | `save[df, 'out.csv']` |
| select | Pick columns | `sub = select[df, ['age', 'salary']]` |
| filter | Filter rows | `old = filter[df, df.age gt 50]` |
| sort | Sort by a column, or by several with a direction each | `s = sort[df, ['symbol', 'qty'], desc = [false, true]]` |
| group | Group and aggregate, by one key or several | `g = group[df, ['dept', 'city'], ['salary', 'mean'], ['age', 'max']]` |
| pivot | The long frame made wide: rows, columns, the values between | `wide = pivot[df, 'symbol', 'day', 'close']` |
| window | A column computed over each row's partition, over rows or a span of time | `with[df, 'avg3', window[df, 'close', 'mean', n = 3, by = 'symbol']]` |
| join | Combine frames on a key or several: inner, left, right, outer, semi, anti, as-of | `m = join[bars, signals, ['symbol', 'at'], 'asof']` |
| unpivot | The wide frame made long: keys, `variable`, `value` | `long = unpivot[df, 'id', ['q1', 'q2']]` |
| complete | The rows with every cell present, or every cell of some columns | `clean = complete[df, ['qty', 'price']]` |
| sample | n rows drawn by the seeded generator, in the frame's order | `some = sample[df, 100]` |
| where | A column picked by a mask from two sides | `with[df, 'px', where[df.filled, df.price, 0.0]]` |
| bucket | Each moment's bucket start, a column of moments | `with[df, 'hour', bucket[df.at, 3600]]` |
| stack | Stack frames vertically | `all = stack[df1, df2]` |
| head | First N rows | `top = head[df, 5]` |
| tail | Last N rows | `bottom = tail[df, 5]` |
| fill | Fill missing values, with a value or by direction | `clean = fill[df, 'age', how = 'forward']` |
| unique | The column's distinct present cells, or the first row of each distinct combination | `rows = unique[df, ['symbol', 'side']]` |
| rename | Rename column | `df2 = rename[df, 'salary', 'pay']` |
| stats | Summary statistics | `stats[df]` |

- `sort` defaults ascending. `sort[df, 'age', desc = true]` for descending. Stable, missing cells last either way (2026-10-06, held to pandas).
- `group` takes pairs of `['column', 'agg']` — single scan, multiple aggregations; a list of keys groups by their combinations, each key column kept as it was. Each aggregate is in the kind it has: a count or a sum over ints an int, a mean a float, a min or max a cell of the column's kind, missing where a group has none present (2026-10-06).
- `pivot` reshapes only: a pair two rows share stops the program, naming the `group` that aggregates it first (spec, Design Note "pivot reshapes, group aggregates").
- `group` and `window` take `sum`, `mean`, `min`, `max`, `count`, `median`, `std`, `var`, `first`, `last`, `distinct` and a percentile `p1`…`p99`; `window` also `lag`, `lead` and `rank`, over the trailing `n` rows (0: every row so far) or the rows within `span` seconds before each row's own on the column `on` names, within the partitions `by` names (2026-10-02; the rest 2026-10-07, held to pandas 3.0 to the last digit — its rolling kernels compile to fused multiply-adds, which the runtime's `mul_add` reproduces).
- `join` defaults inner. `join[df1, df2, 'id', 'left']` for left/right/outer; `semi` and `anti` keep the left's rows with and without a match; `asof` matches the last key by time, the others exactly.
- Text functions (`lower`, `upper`, `trim`, `replace`, `sub`, `contains`, `starts`, `ends`), casts (`int`, `float`, `str`, `moment`) and a moment's fields (`df.at.hour`) work over a column cell by cell; a cell that cannot convert is missing.

Plus properties: `.shape`, `.columns`, `.dtypes`

Plus conversion: `tensor[df]` or `tensor[df, ['age', 'salary']]`

---

## Missing Values — Validity Bitmap + `.ok`

Each column stores a **validity bitmap** (1 bit per row). No sentinels, no `none`/`null` in the language.

- **Column ops** (vectorized): missing propagates in arithmetic, excluded in comparisons. Zero error values allocated.
- **Individual cell access**: missing surfaces as an **error value** (Crow's existing system).
- **Numeric storage**: missing cells store `nan` in `Vec<f64>` (bitmap is source of truth). `tensor[df]` preserves `nan` — standard for ML.
- **String/DateTime**: bitmap says "missing", individual access returns error value.

```crow
df.name[1]       --- error: missing
df.name[1].ok    --- false

--- .ok returns boolean array — same for every column type
df.name.ok       --- [true, true, false, true]
df.age.ok        --- [true, false, true, true]

--- count present/missing
count[df.age.ok]         --- 3 (present)
count[not df.age.ok]     --- 1 (missing)
```

---

## Moments in a frame

A `datetime` column stores UTC epoch milliseconds (`i64`) and the offset
each cell was written in, in minutes (`i16`) — the same two numbers a
moment carries (`crow_runtime::moments::Moment`), so a cell read one at a
time is a moment, a list of moments handed to `with` is a column, and a
mask compares the column with a moment by instant. One kind in both modes
and from every source, since 2026-10-06 (spec, Design Note "a moment is one
kind"); the module's own `datetime` and `tz` went with it, their jobs done
by `moment[text]` and `moment[t, tz = zone]`.

```crow
--- auto-detect ISO 8601 in CSV
--- 2024-01-15, 2024-01-15T10:30:00, 2024-01-15T10:30:00-05:00, 2024-01-15T10:30:00Z

d = moment['2024-01-15T10:30:00-05:00']
d.year           --- 2024
d.offset         --- -300 (minutes from UTC)
moment[d, tz = 'UTC']
moment[d, tz = '+09:00']
```

---

## Resolved Open Questions

- **`head[df, 5]`** returns a frame (composable: `filter[head[df, 5], ...]`)
- **`log[df]`** displays a table summary: column names, dtypes, row count, first few rows
- **`df.age * 2`** (column arithmetic) included from day one — essential for data science
- **Missing values** handled via validity bitmap + `.ok`

---

## Complete Example

```crow
use frame
use math

--- CSV: name,age,salary,start_date
---      alice,30,50000,2024-01-15
---      bob,,75000,
---      ,25,,2024-03-20
---      carol,42,90000,2024-06-01

--- load
df = load['people.csv']
log[df]

--- properties
log[df.shape]      --- [4, 4]
log[df.columns]    --- ['name', 'age', 'salary', 'start_date']
log[df.dtypes]     --- ['str', 'int', 'float', 'datetime']

--- column access
log[df.age]        --- [30, nan, 25, 42]
log[df.name]       --- ['alice', 'bob', error, 'carol']

--- missing values
log[df.age.ok]             --- [true, false, true, true]
log[count[not df.age.ok]]  --- 1
clean = fill[df, 'age', 0]

--- column arithmetic
log[df.salary * 1.1]       --- [55000, 82500, nan, 99000]

--- filter
rich = filter[df, df.salary gt 60000]

--- select
sub = select[df, ['age', 'salary']]

--- sort
sorted = sort[df, 'age']
sorted_desc = sort[df, 'age', desc = true]

--- group (single scan, multi-agg)
g = group[df, 'name', ['salary', 'mean'], ['age', 'max']]

--- join
deps = load['departments.csv']
merged = join[df, deps, 'name']
merged2 = join[df, deps, 'name', 'left']

--- stack
df3 = load['more_people.csv']
all = stack[df, df3]

--- unique
log[unique[df.name]]   --- ['alice', 'bob', 'carol']

--- rename
df2 = rename[df, 'salary', 'pay']

--- stats
stats[df]

--- head/tail
top = head[df, 5]

--- save
save[df, 'out.csv']

--- convert to tensor
t = tensor[df, ['age', 'salary']]
```