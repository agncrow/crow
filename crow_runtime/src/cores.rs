//! How many cores a program's parallel loops use (`cpu[n]`, spec: Resource
//! Control), read in both modes where a parallel loop starts.
//!
//! `cpu[n]` holds from the call on, in the thread that runs it; written in a
//! function, until the function returns (`Restore`). The crowfile's `cpu`
//! is the default every thread starts from. With no count, a loop runs on
//! rayon's global pool — every core; with one, on a pool of that many
//! threads, which a loop nested inside it runs on too. (The compiled
//! program had applied `cpu[1]` while it printed the program, so a loop in a
//! function printed before the call ran on every core, and `cpu[4]` bounded
//! nothing; the interpreter had sized rayon's global pool once, for the
//! whole process, whatever came later.)

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    /// This thread's count: 0 where `cpu[…]` has set none.
    static CORES: Cell<usize> = const { Cell::new(0) };
}

/// The crowfile's count, every thread's default: 0 for every core.
static DEFAULT: AtomicUsize = AtomicUsize::new(0);

/// The pools made, one per count asked for (a program asks for a few).
static POOLS: Mutex<Vec<(usize, &'static rayon::ThreadPool)>> = Mutex::new(Vec::new());

/// The program's default count, from its crowfile.
pub fn set_default(n: usize) {
    DEFAULT.store(n, Ordering::Relaxed);
}

/// `cpu[n]`: the count from here on in this thread, or the reason it is
/// none; answers the count it replaces.
pub fn set(n: i64) -> Result<usize, String> {
    if n < 1 {
        return Err("cpu[] requires a positive integer".to_string());
    }
    Ok(CORES.with(|c| c.replace(n as usize)))
}

/// The count a loop starting here runs on: 0 for every core.
pub fn get() -> usize {
    match CORES.with(|c| c.get()) {
        0 => DEFAULT.load(Ordering::Relaxed),
        n => n,
    }
}

/// A function's count put back as it returns: made where a function that
/// calls `cpu[…]` starts.
pub struct Restore(usize);

impl Restore {
    pub fn new() -> Restore {
        Restore(CORES.with(|c| c.get()))
    }
}

impl Default for Restore {
    fn default() -> Restore {
        Restore::new()
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        CORES.with(|c| c.set(self.0));
    }
}

/// A parallel loop run on the cores the count allows: on a pool of that
/// many threads, or where it is when no count is set — rayon's global pool,
/// or the pool an enclosing loop runs on.
pub fn run<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    match get() {
        0 => f(),
        n => match pool(n) {
            Some(p) => p.install(f),
            None => f(),
        },
    }
}

/// The pool of `n` threads, made the first time it is asked for (none if
/// the system refuses the threads: the loop then runs where it is).
fn pool(n: usize) -> Option<&'static rayon::ThreadPool> {
    let mut pools = POOLS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = pools.iter().find(|(k, _)| *k == n) {
        return Some(*p);
    }
    let p: &'static rayon::ThreadPool = Box::leak(Box::new(rayon::ThreadPoolBuilder::new().num_threads(n).build().ok()?));
    pools.push((n, p));
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A count bounds the loops after it, a function's is put back as it
    /// returns, and a loop nested in a bounded one runs on the same pool.
    #[test]
    fn a_count_bounds_the_loops_after_it() {
        use rayon::prelude::*;
        let all = rayon::current_num_threads();
        assert_eq!(run(rayon::current_num_threads), all);
        assert_eq!(set(0), Err("cpu[] requires a positive integer".to_string()));
        let before = set(2).expect("a count");
        assert_eq!(run(rayon::current_num_threads), 2);
        let nested: Vec<usize> = run(|| (0..4).into_par_iter().map(|_| run(rayon::current_num_threads)).collect());
        assert!(nested.iter().all(|&n| n == 2), "{:?}", nested);
        {
            let _back = Restore::new();
            set(1).expect("a count");
            assert_eq!(run(rayon::current_num_threads), 1);
        }
        assert_eq!(get(), 2, "the function's count is put back as it returns");
        CORES.with(|c| c.set(before));
    }
}
