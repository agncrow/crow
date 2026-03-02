//! Global seeded RNG for reproducible random number generation.
//!
//! When `set_seed(n)` is called, all subsequent random tensor operations
//! use a deterministic RNG seeded with `n`. When no seed is set (or after
//! `clear_seed()`), random operations use the thread-local RNG.

use std::sync::Mutex;

use rand::rngs::StdRng;
use rand::SeedableRng;

static GLOBAL_RNG: Mutex<Option<StdRng>> = Mutex::new(None);

/// Set the global seed for reproducibility. All subsequent random tensor
/// operations will use a deterministic RNG seeded with this value.
pub fn set_seed(seed: u64) {
    let mut rng = GLOBAL_RNG.lock().unwrap();
    *rng = Some(StdRng::seed_from_u64(seed));
}

/// Clear the global seed. Random operations revert to thread-local RNG.
pub fn clear_seed() {
    let mut rng = GLOBAL_RNG.lock().unwrap();
    *rng = None;
}

/// Generate random data using the global seeded RNG if available,
/// otherwise using a thread-local RNG. Takes a size and a function
/// that generates one sample from an RNG.
pub fn gen_random_vec(n: usize, mut sample_fn: impl FnMut(&mut StdRng) -> f64) -> Vec<f64> {
    let mut guard = GLOBAL_RNG.lock().unwrap();
    if let Some(ref mut rng) = *guard {
        (0..n).map(|_| sample_fn(rng)).collect()
    } else {
        drop(guard);
        let mut rng = StdRng::seed_from_u64(rand::random());
        (0..n).map(|_| sample_fn(&mut rng)).collect()
    }
}

/// Run a closure that needs mutable access to the RNG (e.g., for shuffle).
pub fn with_rng(f: impl FnOnce(&mut StdRng)) {
    let mut guard = GLOBAL_RNG.lock().unwrap();
    if let Some(ref mut rng) = *guard {
        f(rng);
    } else {
        drop(guard);
        let mut rng = StdRng::seed_from_u64(rand::random());
        f(&mut rng);
    }
}

