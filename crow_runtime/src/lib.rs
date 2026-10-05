// A runtime function answers its reason, or stops with it: it never
// panics (docs/bug-classes.md §3). `cargo clippy`, which the gate and CI
// run, refuses a panic, an unwrap or an expect that is not allowed with the
// invariant that makes it safe.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable, clippy::todo, clippy::unimplemented))]

pub mod display;
pub mod distributed;
pub mod frame;
pub mod frames;
pub mod fused;
pub mod hash;
pub mod indicators;
pub mod gpu;
pub mod io;
pub mod json;
pub mod later;
pub mod layout;
pub mod cancel;
pub mod autograd;
pub mod sys;
pub mod rowbinary;
pub mod postgres_rows;
pub mod cell;
pub mod chart;
pub mod core;
pub mod moments;
pub mod linalg;
pub mod markdown;
pub mod mask;
pub mod math;
pub mod pool;
pub mod rng;
pub mod signal;
pub mod signin;
pub mod sources;
pub mod sparse;
pub mod special;
pub mod sql;
pub mod stats;
pub mod stops;
pub mod ta;
pub mod txn;
pub mod tensor;
pub mod text;

/// A lock that a panic on another thread cannot poison for everyone after
/// it: the data is still there, and the program goes on with it. A stop,
/// never a crash (docs/bug-classes.md §3).
pub fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
