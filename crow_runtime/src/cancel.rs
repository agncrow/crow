//! Stopping work nobody waits for (session-layer-design.md §3.4): a
//! background recompute whose session is gone or whose inputs moved, and
//! everything in flight when the program is told to stop. Work runs under
//! a flag; a driver that can stop a running query on its server (ClickHouse,
//! Postgres) watches the flag and stops it, so a superseded query stops
//! costing the database and a restarted app does not inherit its
//! predecessor's queries.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Set once the program is told to stop.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// How much work runs under a flag right now: a stop waits for it to
/// notice (a driver tells its server to stop the query it was waiting
/// for) before the process ends.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// How long a stop waits for the work in flight.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// What the drivers do when the program stops: stop what they started.
static HOOKS: Mutex<Vec<Box<dyn Fn() + Send + Sync>>> = Mutex::new(Vec::new());

/// Run `f` with `flag` as the current work's flag, on this thread.
pub fn with_flag<T>(flag: Arc<AtomicBool>, f: impl FnOnce() -> T) -> T {
    let saved = CURRENT.with(|c| c.replace(Some(flag)));
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    struct Restore(Option<Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let saved = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = saved);
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _restore = Restore(saved);
    f()
}

/// How much work runs under a flag right now.
pub fn in_flight() -> usize {
    IN_FLIGHT.load(Ordering::SeqCst)
}

/// Work a stop waits for while this is held: a driver's statement, whose
/// server is told to stop it before the program is gone.
pub struct FlightGuard(());

impl Drop for FlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn in_flight_guard() -> FlightGuard {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    FlightGuard(())
}

/// The flag of the work running on this thread, if it runs under one.
pub fn current() -> Option<Arc<AtomicBool>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Whether the work under `flag` — or everything — should stop.
pub fn stopped(flag: &Option<Arc<AtomicBool>>) -> bool {
    SHUTDOWN.load(Ordering::SeqCst) || flag.as_ref().map(|f| f.load(Ordering::SeqCst)).unwrap_or(false)
}

/// Whether the work running on this thread should stop.
pub fn current_stopped() -> bool {
    stopped(&current())
}

/// Whether the program has been told to stop.
pub fn shutting_down() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// Register what to do when the program stops.
pub fn on_shutdown(hook: Box<dyn Fn() + Send + Sync>) {
    HOOKS.lock().unwrap_or_else(|p| p.into_inner()).push(hook);
}

/// The program is stopping: every flag reads as set, and every driver
/// stops what it started. Runs its hooks once, then waits — three seconds
/// at most — for the work in flight to notice and tell its server (a
/// query a derivation was waiting for is cancelled there, not left
/// running after the program is gone).
pub fn shutdown() {
    if SHUTDOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let hooks = std::mem::take(&mut *HOOKS.lock().unwrap_or_else(|p| p.into_inner()));
    for hook in hooks {
        hook();
    }
    let by = std::time::Instant::now() + STOP_WAIT;
    while in_flight() > 0 && std::time::Instant::now() < by {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_runs_under_its_flag_and_the_flag_is_restored() {
        assert!(current().is_none());
        let flag = Arc::new(AtomicBool::new(false));
        with_flag(flag.clone(), || {
            assert!(!current_stopped());
            flag.store(true, Ordering::SeqCst);
            assert!(current_stopped(), "a set flag stops the work under it");
        });
        assert!(current().is_none(), "the thread's flag is restored after the work");
    }
}
