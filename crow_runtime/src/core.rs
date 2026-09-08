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
    if step > 0 {
        while i < stop {
            out.push(i);
            i += step;
        }
    } else {
        while i > stop {
            out.push(i);
            i += step;
        }
    }
    Ok(out)
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

/// Floats order by value; a NaN sorts as equal to everything, which is
/// what the interpreter has always done.
pub fn sort_floats(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
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
}
