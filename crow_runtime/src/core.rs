//! The collection builtins, once: `range`, `push`, `pop`, `reverse`,
//! `concat`, `slice`, `sort`, `set`, `del`. The interpreter unwraps its
//! dynamic value and calls these; the compiled program calls them
//! directly. Each takes the collection by value, so a caller that has
//! moved a uniquely held collection in gets its work done in place, and
//! a caller that cloned first paid exactly what it would have anyway.
//! The edge cases (clamped slices, empty pops, downward ranges) live
//! here and nowhere else — `core-runtime-builtins.md`.

use std::collections::BTreeMap;

use rand::RngExt;

/// `range[stop]`, `range[start, stop]`, `range[start, stop, step]`: the
/// ints from `start` up to (or, with a negative step, down to) `stop`,
/// exclusive. A zero step is refused.
pub fn range(start: i64, stop: i64, step: i64) -> Result<Vec<i64>, String> {
    if step == 0 {
        return Err("range[] step cannot be zero".to_string());
    }
    let mut out = Vec::new();
    let mut i = start;
    // a step past the last int ends the range: its next value would be
    // beyond `stop` too
    if step > 0 {
        while i < stop {
            out.push(i);
            let Some(next) = i.checked_add(step) else { break };
            i = next;
        }
    } else {
        while i > stop {
            out.push(i);
            let Some(next) = i.checked_add(step) else { break };
            i = next;
        }
    }
    Ok(out)
}

/// `m.key` / `m[key]`: the value, or the words both modes stop with.
pub fn map_key<T: Clone>(m: &std::collections::BTreeMap<String, T>, key: &str) -> Result<T, String> {
    crate::stops::key(m, key).cloned()
}

/// `push[arr, x]`: the array with `x` at the end.
pub fn push<T>(mut v: Vec<T>, x: T) -> Vec<T> {
    v.push(x);
    v
}

/// `pop[arr]`: the array without its last element; an empty array stays
/// empty (a total function, like `slice`).
pub fn pop<T>(mut v: Vec<T>) -> Vec<T> {
    v.pop();
    v
}

/// `reverse[arr]`.
pub fn reverse<T>(mut v: Vec<T>) -> Vec<T> {
    v.reverse();
    v
}

/// `concat[a, b]` for arrays: `a` followed by `b`.
pub fn concat<T: Clone>(mut a: Vec<T>, b: &[T]) -> Vec<T> {
    a.extend_from_slice(b);
    a
}

/// `slice[arr, start, end]`: the elements from `start` to `end`
/// exclusive, both clamped to the array, empty when `start` is not
/// before `end`. Never out of range.
pub fn slice<T: Clone>(v: &[T], start: i64, end: i64) -> Vec<T> {
    let s = start.max(0) as usize;
    let e = (end.max(0) as usize).min(v.len());
    if s >= e { Vec::new() } else { v[s..e].to_vec() }
}

/// `slice[tensor, start, end]` along the first axis, clamped like the
/// array form; an empty slice is the empty 1-D tensor.
pub fn slice_tensor(t: &crate::tensor::CrowTensor, start: i64, end: i64) -> crate::tensor::CrowTensor {
    let s = start.max(0) as usize;
    let e = (end.max(0) as usize).min(t.len());
    if s >= e { crate::tensor::CrowTensor::from_vec(vec![], vec![0]) } else { t.slice(0, s, e) }
}

/// `slice[text, start, end]` counts characters, so a slice never lands
/// inside a glyph and both modes agree on `'héllo'`.
pub fn slice_str(s: &str, start: i64, end: i64) -> String {
    let st = start.max(0) as usize;
    let en = end.max(0) as usize;
    if st >= en { String::new() } else { s.chars().skip(st).take(en - st).collect() }
}

/// `sort[arr]` with the caller's ordering (the interpreter orders its
/// dynamic values; the typed sorts below are for the compiled program).
pub fn sort_by<T, F: FnMut(&T, &T) -> std::cmp::Ordering>(mut v: Vec<T>, cmp: F) -> Vec<T> {
    v.sort_by(cmp);
    v
}

pub fn sort_ints(mut v: Vec<i64>) -> Vec<i64> {
    v.sort_unstable();
    v
}

/// The order `sort` puts numbers in, in both modes: ascending, NaN last.
/// (NaN compared equal to everything, so a list holding one sorted to an
/// order that depended on where it stood.)
pub fn order_floats(a: &f64, b: &f64) -> std::cmp::Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
    }
}

pub fn sort_floats(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(order_floats);
    v
}

/// `sort[t]` of a tensor: its numbers in order, as one axis. (Compiled,
/// a NaN among them panicked.)
pub fn sort_tensor(t: &crate::tensor::CrowTensor) -> crate::tensor::CrowTensor {
    let v = sort_floats(t.to_vec());
    let n = v.len();
    crate::tensor::CrowTensor::from_vec(v, vec![n])
}

pub fn sort_strs(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// `set[map, key, value]`: the map with the entry added or replaced.
pub fn map_set<T>(mut m: BTreeMap<String, T>, key: String, value: T) -> BTreeMap<String, T> {
    m.insert(key, value);
    m
}

/// `del[map, key]`: the map without the entry, unchanged if absent.
pub fn map_del<T>(mut m: BTreeMap<String, T>, key: &str) -> BTreeMap<String, T> {
    m.remove(key);
    m
}

// ── the collections a program calls ─────────────────────────────────
// One function each, generic over the items: the compiled program calls
// it with its typed items, the interpreter with its own values, so a list
// means one thing to both (docs/bug-classes.md §4).

/// `set[map, key, value]`.
pub fn set<T>(m: BTreeMap<String, T>, key: &str, value: T) -> BTreeMap<String, T> {
    map_set(m, key.to_string(), value)
}

/// `len[list]`.
pub fn len<T>(v: &[T]) -> i64 {
    v.len() as i64
}

/// `len[text]`, in characters.
pub fn len_text(s: &str) -> i64 {
    s.chars().count() as i64
}

/// `len[map]`: its entries.
pub fn len_map<T>(m: &BTreeMap<String, T>) -> i64 {
    m.len() as i64
}

/// `len[tensor]`: the items a loop walks and an index reads — a vector's
/// numbers, a matrix's rows (its first dimension). (It had counted every
/// number, so `for i in range[len[m]]` read past a matrix's last row.)
pub fn len_tensor(t: &crate::tensor::CrowTensor) -> i64 {
    match t.shape() {
        [rows, _, ..] => *rows as i64,
        _ => t.len() as i64,
    }
}

/// What a loop over a matrix walks: its rows. A tensor's rank is in its
/// type (spec, Arrays), and the lowering decides from it whether a loop
/// walks rows or numbers and an index reads a row or a number — refusing
/// the decision where the rank is not known — so both backends read a
/// value through these four, and a value whose rank disagrees with its
/// type stops them with the same words: a bug in Crow's shape rules, never
/// a silent difference between the two.
pub fn walk_rows(t: &crate::tensor::CrowTensor) -> Result<Vec<crate::tensor::CrowTensor>, String> {
    if t.ndim() < 2 {
        return Err(rank_disagrees("rows", "a matrix", t));
    }
    Ok(t.rows())
}

/// What a loop over a vector walks: its numbers.
pub fn walk_numbers(t: &crate::tensor::CrowTensor) -> Result<Vec<f64>, String> {
    if t.ndim() >= 2 {
        return Err(rank_disagrees("numbers", "a vector", t));
    }
    Ok(t.to_vec())
}

/// `m[i]` of a matrix: its row, or the reason the index is outside.
pub fn row_at(t: &crate::tensor::CrowTensor, i: i64) -> Result<crate::tensor::CrowTensor, String> {
    if t.ndim() < 2 {
        return Err(rank_disagrees("a row", "a matrix", t));
    }
    Ok(t.index0(crate::stops::row(i, t.shape()[0])?))
}

/// `v[i]` of a vector: its number, or the reason the index is outside.
pub fn number_at(t: &crate::tensor::CrowTensor, i: i64) -> Result<f64, String> {
    if t.ndim() >= 2 {
        return Err(rank_disagrees("a number", "a vector", t));
    }
    Ok(t.get_flat(crate::stops::item(i, t.len())?))
}

fn rank_disagrees(read: &str, typed: &str, t: &crate::tensor::CrowTensor) -> String {
    format!(
        "the program reads {} of a tensor its type says is {}, but the value has shape {:?} — a bug in Crow's shape rules, not in this program",
        read,
        typed,
        t.shape()
    )
}

/// `len[frame]`: its rows.
pub fn len_frame(f: &crate::frame::CrowFrame) -> i64 {
    f.nrows() as i64
}

/// `len[col]`: a frame's column's cells, a missing one too.
pub fn len_column(c: &crate::frame::CrowColumn) -> i64 {
    c.len() as i64
}

/// `len[j]` of a parsed JSON value: a list's items, an object's entries,
/// text's characters. A number or a bool has no length: the refusal the
/// interpreter gives the value it parsed to (the table's words for `len`).
pub fn len_json(j: &serde_json::Value) -> Result<i64, String> {
    let got = match j {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) | serde_json::Value::String(_) => return Ok(crate::json::len(j)),
        serde_json::Value::Number(n) if n.is_i64() => "an int",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::Bool(_) => "a bool",
        serde_json::Value::Null => "a value",
    };
    Err(format!("len[] takes a list, text, a map, numbers, a frame or bytes, got {}", got))
}

/// `contains[list, x]`: whether an item is `x`, compared as `eq` compares
/// (a record field by field, a map entry by entry).
pub fn contains<T: PartialEq>(v: &[T], x: &T) -> bool {
    v.contains(x)
}

/// `contains[text, part]`.
pub fn contains_text(s: &str, part: &str) -> bool {
    s.contains(part)
}

/// `contains[tensor, x]`.
pub fn contains_number(t: &crate::tensor::CrowTensor, x: f64) -> bool {
    t.to_vec().contains(&x)
}

/// `keys[map]`, in order.
pub fn keys<T>(m: &BTreeMap<String, T>) -> Vec<String> {
    m.keys().cloned().collect()
}

/// `values[map]`, in their keys' order.
pub fn values<T: Clone>(m: &BTreeMap<String, T>) -> Vec<T> {
    m.values().cloned().collect()
}

/// `has[map, key]`.
pub fn has<T>(m: &BTreeMap<String, T>, key: &str) -> bool {
    m.contains_key(key)
}

/// `reverse[text]`, by character.
pub fn reverse_text(s: &str) -> String {
    s.chars().rev().collect()
}

/// `reverse[tensor]`: its numbers backwards, as one axis.
pub fn reverse_tensor(t: &crate::tensor::CrowTensor) -> crate::tensor::CrowTensor {
    let mut v = t.to_vec();
    v.reverse();
    let n = v.len();
    crate::tensor::CrowTensor::from_vec(v, vec![n])
}

/// `pop[tensor]`: its numbers but the last, as one axis.
pub fn pop_tensor(t: &crate::tensor::CrowTensor) -> crate::tensor::CrowTensor {
    let mut v = t.to_vec();
    v.pop();
    let n = v.len();
    crate::tensor::CrowTensor::from_vec(v, vec![n])
}

/// `push[tensor, x]`: a number onto a vector.
pub fn push_number(t: &crate::tensor::CrowTensor, x: f64) -> Result<crate::tensor::CrowTensor, String> {
    t.push_scalar(x)
}

/// `push[matrix, row]`: a row onto a matrix.
pub fn push_row(t: &crate::tensor::CrowTensor, row: &crate::tensor::CrowTensor) -> Result<crate::tensor::CrowTensor, String> {
    t.push_row(row)
}

/// `concat[a, b]` of text.
pub fn concat_text(a: &str, b: &str) -> String {
    format!("{}{}", a, b)
}

/// `concat[a, b]` of tensors, along the first axis.
pub fn concat_tensors(a: &crate::tensor::CrowTensor, b: &crate::tensor::CrowTensor) -> Result<crate::tensor::CrowTensor, String> {
    a.try_concat(b, 0)
}

/// `concat[a, b, axis]` of tensors.
pub fn concat_axis(a: &crate::tensor::CrowTensor, b: &crate::tensor::CrowTensor, axis: i64) -> Result<crate::tensor::CrowTensor, String> {
    a.try_concat(b, axis)
}

/// `range[n]`: 0 up to `n`.
pub fn range_to(n: i64) -> Result<Vec<i64>, String> {
    range(0, n, 1)
}

/// `range[start, stop]`.
pub fn range_from(start: i64, stop: i64) -> Result<Vec<i64>, String> {
    range(start, stop, 1)
}

/// `map[]`: an empty map, its values' type what the program puts in it.
pub fn map_new<T>() -> BTreeMap<String, T> {
    BTreeMap::new()
}

// ── a plain number's math ───────────────────────────────────────────
// (on the dual numbers a function `grad` differentiates, each is the
// derivative's own form, written with the program)

/// `abs[n]` of an int; past 64 bits, the reason.
pub fn abs_int(n: i64) -> Result<i64, String> {
    crate::stops::abs(n)
}

/// `abs[x]` of a float.
pub fn abs_float(x: f64) -> f64 {
    x.abs()
}

/// `sqrt[x]`: NaN of a negative number, as the float's is.
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// `floor`, `ceil` and `round` of an int: the int.
pub fn whole(n: i64) -> i64 {
    n
}

/// `floor[x]`: the int at or below, cut to 64 bits (NaN is 0).
pub fn floor(x: f64) -> i64 {
    x.floor() as i64
}

/// `ceil[x]`: the int at or above, cut to 64 bits (NaN is 0).
pub fn ceil(x: f64) -> i64 {
    x.ceil() as i64
}

/// `round[x]`: the nearest int, a half away from zero, cut to 64 bits.
pub fn round(x: f64) -> i64 {
    x.round() as i64
}

// ── the program and its system ──────────────────────────────────────

/// `args[]`: the arguments the program was given — after the source file
/// when `crow` runs it, after the program when it runs compiled.
pub fn args() -> Vec<String> {
    let all: Vec<String> = std::env::args().collect();
    let by_crow = all.first().and_then(|a| std::path::Path::new(a).file_stem().map(|s| s == "crow")).unwrap_or(false);
    if by_crow {
        let source = all.iter().position(|a| [".co", ".coc", ".crow", ".crowc"].iter().any(|e| a.ends_with(e)));
        return source.map(|i| all[i + 1..].to_vec()).unwrap_or_default();
    }
    all.into_iter().skip(1).collect()
}

/// `env[name]`: the variable's text, or its absence as the reason.
pub fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("environment variable '{}' not set", name))
}

/// `env[name, default]`: the variable's text, or the default as text — a
/// variable is text whichever answers (the interpreter had answered a
/// default of another type as that type).
pub fn env_or(name: &str, default: &crate::cell::Cell) -> Result<String, String> {
    Ok(std::env::var(name).unwrap_or_else(|_| default.text()))
}

/// `input[]`: a line from stdin.
pub fn input() -> Result<String, String> {
    read_line(None)
}

/// `input[prompt]`: the prompt, then a line from stdin.
pub fn input_prompt(prompt: &str) -> Result<String, String> {
    read_line(Some(prompt))
}

/// `exit[]`: the program ends, what it printed written out.
pub fn exit() {
    exit_code(0)
}

/// `exit[code]`: the program ends with the code.
pub fn exit_code(code: i64) {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(code as i32)
}

/// `sleep[ms]` of an int: milliseconds; one that is negative is refused.
pub fn sleep_ms(ms: i64) -> Result<(), String> {
    if ms < 0 {
        return Err(format!("sleep[] takes a time that is not negative (got {})", ms));
    }
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    Ok(())
}

/// `halt[reason]`: the program stops with the reason, on the line of the
/// call, as a builtin given what it cannot take does — a program's own
/// precondition, and a standard module's written in Crow (spec, When a
/// Program Stops). (Named `halt`, as `assert`'s own words have it: `stop`
/// is a trading program's stop-loss, a bus's stop.)
pub fn halt(reason: &str) -> Result<(), String> {
    Err(reason.to_string())
}

/// `sleep[s]` of a float: seconds; one that is negative is refused.
pub fn sleep_secs(s: f64) -> Result<(), String> {
    if s.is_nan() || s < 0.0 || s.is_infinite() {
        return Err(format!("sleep[] takes a time that is not negative (got {})", s));
    }
    std::thread::sleep(std::time::Duration::from_secs_f64(s));
    Ok(())
}


/// `rand[]`: a float in [0, 1) from the program's RNG, the one `seed[n]`
/// seeds, so a seeded program draws the same numbers in both modes.
pub fn rand_float() -> f64 {
    let mut out = 0.0;
    crate::rng::with_rng(|r| out = r.random::<f64>());
    out
}

/// `rand[n]`: an int in 0..n; `n` must be positive.
pub fn rand_int(n: i64) -> Result<i64, String> {
    if n <= 0 {
        return Err(format!("rand[n] needs a positive n, got {}", n));
    }
    let mut out = 0;
    crate::rng::with_rng(|r| out = r.random_range(0..n));
    Ok(out)
}

/// `rand[min, max]`: an int in min..=max; `max` must not be below `min`.
pub fn rand_between(min: i64, max: i64) -> Result<i64, String> {
    if max < min {
        return Err(format!("rand[min, max] needs min <= max, got {} and {}", min, max));
    }
    let mut out = 0;
    crate::rng::with_rng(|r| out = r.random_range(min..=max));
    Ok(out)
}

/// `join[tensor, sep]`: a whole number prints without a decimal point,
/// as it does in a list, everything else as the number is.
/// `min[a, b]` of two ints.
pub fn min_ints(a: i64, b: i64) -> i64 {
    a.min(b)
}

/// `max[a, b]` of two ints.
pub fn max_ints(a: i64, b: i64) -> i64 {
    a.max(b)
}

/// `min[a, b]` of numbers (an int beside a float is a float): the other
/// when one is NaN.
pub fn min_floats(a: f64, b: f64) -> f64 {
    a.min(b)
}

/// `max[a, b]` of numbers.
pub fn max_floats(a: f64, b: f64) -> f64 {
    a.max(b)
}

/// `join[items, sep]`: the items' words (`crate::cell::Words`) between
/// separators.
pub fn join_words(items: &[String], sep: &str) -> String {
    items.join(sep)
}

pub fn join_floats(v: &[f64], sep: &str) -> String {
    v.iter()
        .map(|&x| if x == (x as i64) as f64 && x.is_finite() { (x as i64).to_string() } else { x.to_string() })
        .collect::<Vec<String>>()
        .join(sep)
}

/// What `shell[cmd]` answers: the exit status and what the command wrote,
/// trailing whitespace trimmed from both streams. It is the compiled
/// program's `CrowShell`, shown as the interpreter shows its record.
#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    pub status: i64,
    pub stdout: String,
    pub stderr: String,
}

impl Shell {
    /// The record a failed call holds — a command that never ran — as a
    /// compiled program's own structs have theirs (`crow_default`).
    pub fn crow_default() -> Self {
        Shell { status: -1, stdout: String::new(), stderr: String::new() }
    }
}

impl crate::display::CrowDisplay for Shell {
    fn crow_repr(&self) -> String {
        format!("[status = {}, stderr = {}, stdout = {}]", self.status.crow_repr(), self.stderr.crow_repr(), self.stdout.crow_repr())
    }
}

/// A command's answer as JSON: the object the interpreter's record encodes
/// as.
impl crate::json::ToJson for Shell {
    fn to_json(&self) -> serde_json::Value {
        crate::json::object(vec![("status", self.status.to_json()), ("stderr", self.stderr.to_json()), ("stdout", self.stdout.to_json())])
    }
}

/// `shell[cmd]` through `sh -c`. Only a command that cannot start is an
/// error; a non-zero exit is an answer, read from `.status`.
pub fn shell(cmd: &str) -> Result<Shell, String> {
    match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(out) => Ok(Shell {
            status: out.status.code().unwrap_or(-1) as i64,
            stdout: String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim_end().to_string(),
        }),
        Err(e) => Err(format!("shell failed: {}", e)),
    }
}

/// `input[]` / `input[prompt]`: one line from stdin without its line
/// ending and nothing else removed; the prompt is written first.
pub fn read_line(prompt: Option<&str>) -> Result<String, String> {
    use std::io::Write;
    if let Some(p) = prompt {
        print!("{}", p);
        let _ = std::io::stdout().flush();
    }
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(_) => Ok(line.trim_end_matches('\n').trim_end_matches('\r').to_string()),
        Err(e) => Err(format!("input error: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_walks_up_and_down_and_refuses_zero() {
        assert_eq!(range(0, 5, 1).unwrap(), vec![0, 1, 2, 3, 4]);
        assert_eq!(range(2, 8, 3).unwrap(), vec![2, 5]);
        assert_eq!(range(5, 0, -2).unwrap(), vec![5, 3, 1]);
        assert_eq!(range(3, 3, 1).unwrap(), Vec::<i64>::new());
        assert!(range(0, 1, 0).is_err());
        // near the last int the range ends instead of wrapping round forever
        assert_eq!(range(i64::MAX - 6, i64::MAX, 5).unwrap(), vec![i64::MAX - 6, i64::MAX - 1]);
        assert_eq!(range(i64::MIN + 6, i64::MIN, -5).unwrap(), vec![i64::MIN + 6, i64::MIN + 1]);
    }

    #[test]
    fn slice_clamps_and_never_panics() {
        let v = vec![10, 20, 30, 40, 50];
        assert_eq!(slice(&v, 1, 3), vec![20, 30]);
        assert_eq!(slice(&v, 3, 99), vec![40, 50]);
        assert_eq!(slice(&v, -2, 2), vec![10, 20]);
        assert_eq!(slice(&v, 4, 1), Vec::<i64>::new());
        assert_eq!(slice_str("héllo wörld", 1, 4), "éll");
        assert_eq!(slice_str("abc", 2, 99), "c");
        assert_eq!(slice_str("abc", 3, 1), "");
        let t = crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        assert_eq!(slice_tensor(&t, 1, 99).to_vec(), vec![2.0, 3.0]);
        assert_eq!(slice_tensor(&t, 2, 1).len(), 0);
    }

    #[test]
    fn push_pop_reverse_concat_are_total() {
        assert_eq!(push(vec![1, 2], 3), vec![1, 2, 3]);
        assert_eq!(pop(vec![1, 2, 3]), vec![1, 2]);
        assert_eq!(pop(Vec::<i64>::new()), Vec::<i64>::new());
        assert_eq!(reverse(vec![1, 2, 3]), vec![3, 2, 1]);
        assert_eq!(concat(vec![1, 2], &[3]), vec![1, 2, 3]);
    }

    #[test]
    fn sorts_order_ints_floats_and_strings() {
        assert_eq!(sort_ints(vec![3, 1, 2]), vec![1, 2, 3]);
        assert_eq!(sort_floats(vec![2.5, -1.0, 0.5]), vec![-1.0, 0.5, 2.5]);
        assert_eq!(sort_strs(vec!["pear".into(), "apple".into(), "fig".into()]), vec!["apple", "fig", "pear"]);
        assert_eq!(sort_by(vec![3, 1, 2], |a, b| b.cmp(a)), vec![3, 2, 1]);
    }

    #[test]
    fn rand_follows_the_seed() {
        // the tensor tests share the process-global RNG; seed and draw
        // under the same guard the runtime's own tests use
        let _guard = crate::tensor::tests::rng_lock();
        crate::rng::set_seed(7);
        let a = (rand_float(), rand_int(10).unwrap(), rand_between(1, 6).unwrap());
        crate::rng::set_seed(7);
        let b = (rand_float(), rand_int(10).unwrap(), rand_between(1, 6).unwrap());
        assert_eq!(a, b);
        assert!((0.0..1.0).contains(&a.0));
        assert!((0..10).contains(&a.1));
        assert!((1..=6).contains(&a.2));
        assert!(rand_int(0).is_err());
        assert!(rand_between(3, 2).is_err());
        crate::rng::clear_seed();
    }

    #[test]
    fn join_floats_prints_whole_numbers_bare() {
        assert_eq!(join_floats(&[1.5, 2.0, 3.25], "|"), "1.5|2|3.25");
        assert_eq!(join_floats(&[], ","), "");
        assert_eq!(join_floats(&[-0.0, 1e21], " "), "0 1000000000000000000000");
    }

    #[test]
    fn shell_reports_status_and_trims_the_streams() {
        let sh = shell("printf 'out\n\n'; printf 'err \n' >&2; exit 3").unwrap();
        assert_eq!(sh.status, 3);
        assert_eq!(sh.stdout, "out");
        assert_eq!(sh.stderr, "err");
        assert_eq!(shell("exit 0").unwrap().status, 0);
    }

    #[test]
    fn map_set_and_del() {
        let m = map_set(BTreeMap::new(), "a".to_string(), 1);
        let m = map_set(m, "b".to_string(), 2);
        let m = map_set(m, "a".to_string(), 3);
        assert_eq!(m.get("a"), Some(&3));
        assert_eq!(m.len(), 2);
        let m = map_del(m, "b");
        let m = map_del(m, "zz");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn a_moved_vector_grows_in_place() {
        // The whole point: pushing into a vector we own reuses its buffer.
        let mut v: Vec<i64> = Vec::with_capacity(16);
        let before = v.as_ptr();
        for i in 0..10 {
            v = push(v, i);
        }
        assert_eq!(v.as_ptr(), before);
        assert_eq!(v.len(), 10);
    }

    /// A loop walks, and an index reads, what the type decided; a value of
    /// another rank is refused with the reason, never read the other way.
    #[test]
    fn a_walk_and_a_read_follow_the_rank_the_type_decided() {
        let m = crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let v = crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
        assert_eq!(walk_rows(&m).unwrap().len(), 2);
        assert_eq!(walk_numbers(&v).unwrap(), vec![1.0, 2.0]);
        assert_eq!(row_at(&m, 1).unwrap().to_vec(), vec![3.0, 4.0]);
        assert_eq!(number_at(&v, -1).unwrap(), 2.0);
        assert!(walk_rows(&v).unwrap_err().contains("reads rows of a tensor its type says is a matrix, but the value has shape [2]"));
        assert!(walk_numbers(&m).unwrap_err().contains("a bug in Crow's shape rules"));
        assert!(row_at(&v, 0).unwrap_err().contains("reads a row"));
        assert!(number_at(&m, 0).unwrap_err().contains("reads a number"));
        assert!(row_at(&m, 2).unwrap_err().contains("out of range"));
    }

}
