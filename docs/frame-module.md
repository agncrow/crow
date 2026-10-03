# Frame Module Design

## Benchmark Context

1GB mixed CSV (12.7M rows × 10 columns, 4 string + 6 numeric):

| Tool | Time | Memory |
|------|------|--------|
| Polars | 0.18s | ~1GB |
| Pandas | 4.1s | ~3GB |
| Crow csv[] (transpiled) | 31.6s | 36GB |
| Crow csv[] (interpreter) | 144.5s | 32GB |

Current `csv[]` creates one BTreeMap per row — 12.7M maps × 10 entries = ~254M string allocations. Column-oriented storage is essential.

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
- Optimized internal storage (typed vectors: Vec<i64>, Vec<f64>, Vec<String>, Vec<bool>, plus Vec<i64> + Vec<i16> for DateTime)
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
| sort | Sort by column | `s = sort[df, 'age']` |
| group | Group and aggregate, by one key or several | `g = group[df, ['dept', 'city'], ['salary', 'mean'], ['age', 'max']]` |
| pivot | The long frame made wide: rows, columns, the values between | `wide = pivot[df, 'symbol', 'day', 'close']` |
| window | A column computed over each row's partition | `with[df, 'avg3', window[df, 'close', 'mean', n = 3, by = 'symbol']]` |
| join | Combine frames on key | `m = join[df1, df2, 'id']` |
| stack | Stack frames vertically | `all = stack[df1, df2]` |
| head | First N rows | `top = head[df, 5]` |
| tail | Last N rows | `bottom = tail[df, 5]` |
| fill | Fill missing values | `clean = fill[df, 'age', 0]` |
| unique | Unique values in column | `vals = unique[df.age]` |
| rename | Rename column | `df2 = rename[df, 'salary', 'pay']` |
| stats | Summary statistics | `stats[df]` |
| datetime | Construct datetime | `d = datetime['2024-01-15']` |
| tz | Convert timezone | `utc = tz[d, '+09:00']` |

- `sort` defaults ascending. `sort[df, 'age', desc = true]` for descending.
- `group` takes pairs of `['column', 'agg']` — single scan, multiple aggregations; a list of keys groups by their combinations, each key column kept as it was.
- `pivot` reshapes only: a pair two rows share stops the program, naming the `group` that aggregates it first (spec, Design Note "pivot reshapes, group aggregates").
- `window` is `sum`, `mean`, `min`, `max`, `count`, `lag`, `lead` or `rank` over the trailing `n` rows (0: every row so far) within the partitions `by` names (2026-10-02).
- `join` defaults inner. `join[df1, df2, 'id', 'left']` for left/right/outer.

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

## Single DateTime Type

Internal storage: UTC epoch milliseconds (`i64`) + timezone offset in minutes (`i16`).

- All arithmetic in UTC — no daylight saving bugs
- Date-only → midnight: `2024-01-15` → `2024-01-15T00:00:00+00:00`
- No separate Date or Time types — one type, one way
- v1: UTC offset format (`+05:30`, `-08:00`). No IANA names — can add later.

```crow
--- auto-detect ISO 8601 in CSV
--- 2024-01-15, 2024-01-15T10:30:00, 2024-01-15T10:30:00-05:00, 2024-01-15T10:30:00Z

--- construction
d = datetime['2024-01-15T10:30:00-05:00']

--- properties
d.year       --- 2024
d.month      --- 1
d.day        --- 15
d.hour       --- 10
d.minute     --- 30
d.second     --- 0
d.offset     --- -300 (minutes from UTC)

--- timezone conversion
utc = tz[d, 'UTC']
tokyo = tz[d, '+09:00']
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