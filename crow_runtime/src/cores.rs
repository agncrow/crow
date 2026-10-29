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

/// The shared pool's threads with the program's own stack
/// ([`crate::stops::STACK`]), made before anything runs on it: a recursion
/// inside a parallel loop reaches the depth the program's thread does, and
/// stops past it with the same words — on rayon's two megabytes it had
/// aborted the process, 643 calls deep interpreted (Rosetta Code, Find
/// limit of recursion). Asked again, or once a pool exists, it does nothing.
pub fn deep_stacks() {
    let _ = rayon::ThreadPoolBuilder::new().stack_size(crate::stops::STACK).build_global();
}

/// The pool of `n` threads, made the first time it is asked for (none if
/// the system refuses the threads: the loop then runs where it is), its
/// threads with the program's stack as the shared pool's are.
fn pool(n: usize) -> Option<&'static rayon::ThreadPool> {
    let mut pools = POOLS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = pools.iter().find(|(k, _)| *k == n) {
        return Some(*p);
    }
    let p: &'static rayon::ThreadPool = Box::leak(Box::new(rayon::ThreadPoolBuilder::new().num_threads(n).stack_size(crate::stops::STACK).build().ok()?));
    pools.push((n, p));
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The helpers answer the same worked where they are as across the
    /// cores: below the share and above it, in order, sorts stable where
    /// they say so
    #[test]
    fn a_piece_worked_here_or_across_the_cores_is_the_same() {
        for n in [10, SHARED_MIN + 5] {
            let squares = map_range(n, n, |i| i * i);
            assert_eq!(squares, (0..n).map(|i| i * i).collect::<Vec<_>>());
            let xs: Vec<u64> = (0..n as u64).rev().collect();
            assert_eq!(map_items(&xs, n, |x| x + 1), xs.iter().map(|x| x + 1).collect::<Vec<_>>());
            let mut a = xs.clone();
            sort_unstable(&mut a);
            assert!(a.windows(2).all(|w| w[0] <= w[1]));
            // (stable: equal keys keep their order)
            let mut pairs: Vec<(u64, usize)> = (0..n).map(|i| ((i % 7) as u64, i)).collect();
            sort_by(&mut pairs, |x, y| x.0.cmp(&y.0));
            assert!(pairs.windows(2).all(|w| w[0].0 < w[1].0 || (w[0].0 == w[1].0 && w[0].1 < w[1].1)));
        }
        assert!(!shared(SHARED_MIN - 1) && shared(SHARED_MIN));
        assert!(blocking_threads() >= 64);
    }

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

/// How many items of work are worth sharing across the cores: handing work
/// to the pool and waiting for it costs about what a few thousand items do,
/// so a smaller piece is worked where it is — a web handler's frame of a
/// dozen rows had waited on the pool at every sort, and the handlers' threads
/// on each other.
pub const SHARED_MIN: usize = 8192;

/// Whether `n` items of work are worth sharing across the cores.
pub fn shared(n: usize) -> bool {
    n >= SHARED_MIN
}

/// `f` of each of `0..n`, on the cores when `work` items are worth sharing.
pub fn map_range<R: Send>(n: usize, work: usize, f: impl Fn(usize) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    if shared(work) { (0..n).into_par_iter().map(f).collect() } else { (0..n).map(f).collect() }
}

/// `f` of each item, on the cores when `work` items are worth sharing.
pub fn map_items<T: Sync, R: Send>(xs: &[T], work: usize, f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    if shared(work) { xs.par_iter().map(f).collect() } else { xs.iter().map(f).collect() }
}

/// `sort_unstable`, on the cores for a long enough slice.
pub fn sort_unstable<T: Ord + Send>(v: &mut [T]) {
    use rayon::prelude::*;
    if shared(v.len()) { v.par_sort_unstable() } else { v.sort_unstable() }
}

/// `sort_by` (stable), on the cores for a long enough slice.
pub fn sort_by<T: Send>(v: &mut [T], cmp: impl Fn(&T, &T) -> std::cmp::Ordering + Sync) {
    use rayon::prelude::*;
    if shared(v.len()) { v.par_sort_by(cmp) } else { v.sort_by(cmp) }
}

/// How many threads a server runs its waiting handlers on at once — a
/// route's that queries, a page's — four for each core, at least 64; more
/// requests wait their turn. The async runtime's own default (512) let
/// hundreds of handlers wait on one pool of connections, and handing each
/// request to a thread among them cost more than the query.
pub fn blocking_threads() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores * 4).max(64)
}
