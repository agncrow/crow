# One runtime for the builtins (stage 3 tracker)

*Stage 3 of `core-rewrite-notes.md` §9: every builtin's meaning lives
once, in `crow_runtime`, and the interpreter and the compiled program
both call it. This file is the audit and the worklist. Started
2026-09-07 from reading both implementations side by side.*

## How the duplication looks

The interpreter dispatched on the builtin's name and the dynamic
`Value` (`interpreter.rs`, 111 arms; since the IR interpreter became
`crow run -i` the arms live in `builtins.rs` and `ops.rs`, and
`interpreter.rs` is gone). The transpiler dispatches on the
name and the inferred type and emits Rust text inline
(`transpiler.rs`, 121 sites). Where the runtime already owns the
operation (tensors, frames, moments, display, later) both sides call
it and agree by construction. Where it does not, each side carries its
own edge cases, and those are where the modes drift.

## The shape of the fix

A module `crow_runtime::core` of typed functions, generic where the
element type varies, taking ownership where a value can be reused:

```rust
pub fn push<T>(mut v: Vec<T>, x: T) -> Vec<T>          // in place when moved in
pub fn slice<T: Clone>(v: &[T], start: i64, end: i64) -> Vec<T>   // clamped, empty when start >= end
pub fn slice_str(s: &str, start: i64, end: i64) -> String          // by character
pub fn pop<T>(v: Vec<T>) -> Result<Vec<T>, String>                // empty is an error, both modes
```

The interpreter unwraps `Value`, calls the function, wraps the result.
The transpiler emits the call, and for `x = push[x, v]` emits
`x = crow_runtime::core::push(x, v)`, moving `x`, so the list grows in
place: that is the spec's promised optimisation, which the emitted
`{ let mut tmp = x.clone(); tmp.push(v); tmp }` does not deliver today.
The interpreter gets the same by taking the variable out of its
environment for the duration of the call when the statement has that
shape (its first form of uniqueness).

A builtin moves when: the runtime function exists with a unit test,
both sides call it, a parity program pins the edge cases, and the
benchmark table still reads the same or better.

## The audit

Status: `runtime` = both call the runtime already; `move` = to move;
`keep` = trivially the same in both and not worth a call; `n/a` = one
side only.

| Builtin | Interpreter | Compiled | Divergence seen | Status |
|---|---|---|---|---|
| `range` | `core::range` | `core::range` | compiled `step_by` could not count down (fixed) | **runtime** |
| `abs sqrt floor ceil round` | own, with dual numbers for `grad` | Rust methods | `floor`/`ceil`/`round` return int in both | keep |
| `min max` | own, int/float mixing | Rust methods | mixing rules must agree | move |
| `split trim replace upper lower starts ends repeat` | Rust string methods | Rust string methods | none | keep |
| `join` | items by Display; tensor via `core::join_floats` | same | compiled join of an empty literal did not compile (fixed) | **runtime** |
| `fmt` | `display::crow_fmt_*`, `moments::fmt_time` | same | none | runtime |
| `int float str` | parse rules, error kinds; `str` is the text `log` prints | same; `str` of a list or map now goes through the display kernel | compiled `str` of a list or map did not compile (fixed) | keep |
| `len` | array, tensor, chars, map, frame | typed; JSON via `json::len` | fixed 2026-09-07 (parity 40) | runtime |
| `contains` | array by variant equality, tensor, substring | typed | none seen | keep |
| `slice` | `core::slice`, `slice_str`, `slice_tensor` | same | compiled arrays panicked out of range, compiled tensors errored (fixed: clamped everywhere) | **runtime** |
| `keys values has` | BTreeMap | BTreeMap | none | keep |
| `push` | `core::push`, moved in for `x = push[x, v]` | same, moved in | in place in both modes now | **runtime** |
| `pop` | `core::pop` | same | decided: an empty array stays empty (spec row) | **runtime** |
| `sort` | `core::sort_by` with the value ordering | `core::sort_ints/floats/strs` by element type | the interpreter's slowness was the loop's environment copy, not the sort (see log) | **runtime** |
| `reverse` | `core::reverse` for lists; tensor, string as before | same | none | **runtime** |
| `concat` | `core::concat` for lists; string, tensor as before | same | none | **runtime** |
| `set del` | `core::map_set`, `map_del`, moved in for `m = set[m, k, v]` | same, moved in | in place in both modes now | **runtime** |
| `args` | skips `run -i file` | all args | equivalent | keep |
| `input` | `core::read_line` | same, stamps `captured` | compiled trimmed all trailing whitespace where the interpreter trims the line ending only (fixed: line ending only) | **runtime** |
| `rand` | `core::rand_float/int/between` on the seeded RNG | same | `seed[n]` now governs every draw (parity 42) | **runtime** |
| `time sleep env exit` | std | std | none | keep |
| `read write exists mkdir ls rm mv cp` | `crow_runtime::io::fs_*` | same | compiled reasons dropped the path in six of eight (fixed: the path is named); compiled `ls` skipped unreadable entries silently (fixed: an error); compiled `rm` moved its path argument so the variable could not be used again (fixed) | **runtime** |
| `json encode` | `crow_to_json` on values, floats and tensors via `json::float_to_json` / `tensor_to_json` | `json::ToJson` for every native type | compiled `encode` wrote a tensor inside a map or list in its internal `{shape, data}` form (fixed: a list everywhere); a NaN encoded as `null` compiled but was an error interpreted (decided: `null`); a matrix now encodes as rows in both | **runtime** |
| `shell` | `core::shell` → map of status/stdout/stderr | `core::shell` → `CrowShell` record | none now | **runtime** |
| `hash` | sha2/md5 | to check | none | keep |
| `find matches sub` | `crow_runtime::text` | same | `find[text, pattern]` answered a record with an `ok` key the interpreter could never read (`.ok` is every value's status, so a miss still read as ok): decided, the record keeps `groups` and a miss is an error value, so `m.ok` is the status in both modes; an invalid pattern halts the compiled `matches`/`sub` with the reason where the interpreter answers an error value (residual, needs `Result` typing) | **runtime** |
| `now moment calendar parse_time` | `moments` | `moments` | none | runtime |
| `error ok reason kind trace` | value struct | `CrowResult` | none | keep |
| `log` | `display` | `display` | none | runtime |
| `any all find` (predicates) | interpreter calls the fn value | closures | none | keep |
| `later running` | inline schedule | `later` runtime / state | by design | runtime |
| ui, web, db builtins | build runtime objects | build runtime objects | by design | runtime |
| math module | tensor runtime | tensor runtime | none | runtime |

## Order

1. Collections, because the benchmarks show them: `push`, `pop`, `set`,
   `del`, `slice`, `sort`, `range`, with the in-place move in both
   modes. Parity program for clamping and empty pops.
2. Text and conversion: `join`, `str`, `input`, `shell`.
3. Files and JSON: one `io` and one encoder.
4. `rand` on the seeded runtime RNG, so `seed[]` means what it says.

## Log

- 2026-09-07: audit written; `len` of JSON and `*` of vectors fixed on
  the way (parity 40, `try_matmul`).
- 2026-09-07, batch 1 (collections): `crow_runtime::core` with
  `range`, `push`, `pop`, `reverse`, `concat`, `slice`, `slice_str`,
  `slice_tensor`, `sort_by`, `sort_ints/floats/strs`, `map_set`,
  `map_del`; both modes call it; `x = push[x, v]` and the other
  in-place shapes (`ast::IN_PLACE_BUILTINS`, `ast::in_place_call_on`)
  move the collection into the call in both modes (the interpreter
  takes it out of its variable for the call; the transpiler emits the
  move); parity 41 pins the edges. Decisions: `pop` of an empty array
  is the empty array; `slice` clamps everywhere; `range` counts down
  with a negative step. On the way, the interpreter's real slowness:
  a parallel `for` cloned the whole interpreter, environment included,
  once per item; scopes are now shared by reference count and cloned
  once per chunk of items, which took the benchmark's collect program
  from 111 s to under a second in the interpreter.
- 2026-09-07, `rand`: `core::rand_float`, `rand_int`, `rand_between`
  draw from the runtime's seeded RNG (`rng.rs`), so `seed[n]` governs
  the core `rand[]` in both modes; the old time-hash was unseeded in
  both. Parity 42 pins it, and found that three draws in three
  independent statements are one wave and run in parallel, so their
  order is not fixed: a seeded program is deterministic only where its
  statements depend on each other. Evidence for dropping statement
  waves in stage 4 (`core-rewrite-notes.md` §4, §8).
- 2026-09-08: statement waves dropped in both modes (spec §14, Design
  Note "Statements Run in Order"); parity 43.
- 2026-09-08, batch 2 (text): `core::join_floats`, `core::shell`,
  `core::read_line`; compiled `str[]` of a list or map goes through
  the display kernel as `log` does (it did not compile before), a
  compiled `join` of an empty literal joins to nothing (it did not
  compile), and compiled `input` keeps trailing spaces as the
  interpreter does. Parity 44.
- 2026-09-08, batch 3 (files): `crow_runtime::io::fs_read`, `fs_write`,
  `fs_exists`, `fs_mkdir`, `fs_ls`, `fs_rm`, `fs_mv`, `fs_cp`, with the
  reason texts that name the path; both modes call them. Found on the
  way: six compiled reasons omitted the path, compiled `ls` skipped
  unreadable entries, and compiled `rm` moved its argument. Parity 45.
- 2026-09-08, batch 4 (JSON): `json::float_to_json`, `tensor_to_json`
  and the `ToJson` trait; compiled `encode` of a map or list holding
  tensors wrote their internal form; a NaN or infinite float encodes as
  `null` in both modes; a matrix as its rows. Parity 46.
- 2026-09-08, batch 5 (regex): `crow_runtime::text::{matches, find,
  sub}`; `find[text, pattern]` answers a record with `groups` or an
  error value (its `ok` key was unreadable in the interpreter, where
  `.ok` is every value's status, so a miss read as ok); compiled
  `matches` and `sub` report an invalid pattern with the runtime's
  reason instead of a bare unwrap. Parity 47. To decide with the IR's
  typed columns: `.ok` on a list is per element in the interpreter (a
  string column's validity) and in the compiled program only for a
  `Vec<String>`, `true` for any other list, and the status for a
  `Result` holding a list.
- 2026-09-22, decided by the IR's typed columns: `.ok` is a value's
  status, a list's included (the spec's "Non-fallible values always have
  `ok` of `true`"); a frame column's `.ok` is its rows' validity. The
  lowering picks by static type (`Meta::Valid` for a `Ty::Column`, else
  `Meta::Ok`), the interpreter's `ops::field("ok")` stopped mapping over
  lists (`ops::valid` does it for columns), and the compiled program
  dropped its `Vec<String>` branch (`col_ok` is gone). finch's migration
  job had refused a directory it had read (`if not listing.ok` on
  `[true]`). Parity 50.
- 2026-09-08, the interpreter's frame paths, read and left for the IR:
  a column read (`df.price`) builds one full value per row, so a
  million-row column costs about 120 MB of values before any work is
  done, and a comparison against it builds a second list of full
  values as the mask, which `filter` then converts back to booleans.
  That is the whole of the 5× on `filter` and the 20× on a column
  `sum`. Reading numeric columns as tensors would fix the cost but
  change the type of every int cell to float, which flows into SQL
  parameters (a Postgres int column refuses a float); a compact
  boolean list needs a value the interpreter does not have. Both are
  what typed columns in the IR give by construction (stage 4), so the
  interpreter keeps its representation until then.
