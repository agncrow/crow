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
    let mut rng = crate::lock(&GLOBAL_RNG);
    *rng = Some(StdRng::seed_from_u64(seed));
}

/// A replay of recordings (`crow test --fixtures`) seeds every program it
/// runs as the recording was: `CROW_SEED` is the seed, taken before the
/// first draw.
fn seed_from_env() {
    static FROM_ENV: std::sync::Once = std::sync::Once::new();
    FROM_ENV.call_once(|| {
        if let Some(seed) = std::env::var("CROW_SEED").ok().and_then(|s| s.trim().parse::<u64>().ok()) {
            set_seed(seed);
        }
    });
}

/// Clear the global seed. Random operations revert to thread-local RNG.
pub fn clear_seed() {
    let mut rng = crate::lock(&GLOBAL_RNG);
    *rng = None;
}

/// Generate random data using the global seeded RNG if available,
/// otherwise using a thread-local RNG. Takes a size and a function
/// that generates one sample from an RNG.
pub fn gen_random_vec(n: usize, mut sample_fn: impl FnMut(&mut StdRng) -> f64) -> Vec<f64> {
    seed_from_env();
    let mut guard = crate::lock(&GLOBAL_RNG);
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
    seed_from_env();
    let mut guard = crate::lock(&GLOBAL_RNG);
    if let Some(ref mut rng) = *guard {
        f(rng);
    } else {
        drop(guard);
        let mut rng = StdRng::seed_from_u64(rand::random());
        f(&mut rng);
    }
}

