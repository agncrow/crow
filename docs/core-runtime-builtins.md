# One runtime for the builtins (stage 3 tracker)

*Stage 3 of `core-rewrite-notes.md` §9: every builtin's meaning lives
once, in `crow_runtime`, and the interpreter and the compiled program
both call it. This file is the audit and the worklist. Started
2026-09-07 from reading both implementations side by side.*

## How the duplication looks

The interpreter dispatches on the builtin's name and the dynamic
`Value` (`interpreter.rs`, 111 arms). The transpiler dispatches on the
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
| `range` | own loop | own loop | none | move (one generator) |
| `abs sqrt floor ceil round` | own, with dual numbers for `grad` | Rust methods | `floor`/`ceil`/`round` return int in both | keep |
| `min max` | own, int/float mixing | Rust methods | mixing rules must agree | move |
| `split trim replace upper lower starts ends repeat` | Rust string methods | Rust string methods | none | keep |
| `join` | formats int/float/bool items, tensor items | to check | item formatting is a display rule | move |
| `fmt` | `display::crow_fmt_*`, `moments::fmt_time` | same | none | runtime |
| `int float str` | parse rules, error kinds | same rules, inline | `str` of a float goes through two Display impls | move (`str`) |
| `len` | array, tensor, chars, map, frame | typed; JSON via `json::len` | fixed 2026-09-07 (parity 40) | runtime |
| `contains` | array by variant equality, tensor, substring | typed | none seen | keep |
| `slice` | clamps start and end, empty when start ≥ end; chars for strings | indexes directly for arrays: **panics out of range** | **yes** | move |
| `keys values has` | BTreeMap | BTreeMap | none | keep |
| `push` | **clones the list**; tensor via runtime | **clones the list**; tensor via runtime | none in meaning, both quadratic | move (in place) |
| `pop` | empty list is an error value | **empty list is a silent no-op** | **yes** | move |
| `sort` | mixed comparator, tensor via to_vec | `Vec::sort` (needs `Ord`), tensor via to_vec | float arrays that are not tensors would not compile; interpreter far too slow (benchmarks) | move |
| `reverse` | list, tensor, string | same | none | keep |
| `concat` | string, list, tensor with axis | same | none | keep |
| `set del` | clones the map; homogeneity check | clones the map | none in meaning, both quadratic | move (in place) |
| `args` | skips `run -i file` | all args | equivalent | keep |
| `input` | reads a line | reads a line, stamps `captured` | interpreter stamp to check | move |
| `rand` | time hash, ignores `seed` | same text | both ignore `seed`; `rng.rs` exists | move (one RNG) |
| `time sleep env exit` | std | std | none | keep |
| `read write exists mkdir ls rm mv cp` | std, error kinds `io` | to check | error texts | move (one `io`) |
| `json encode` | `json_to_crow` / `crow_to_json` in the interpreter | `serde_json::Value` kept dynamic | number formatting in `encode` | move (one encoder) |
| `shell` | `sh -c`, map of status/stdout/stderr | to check | trailing newline trimming | move |
| `hash` | sha2/md5 | to check | none | keep |
| `find matches sub` | regex, groups map | to check | `find` with no match is an error value | move |
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
