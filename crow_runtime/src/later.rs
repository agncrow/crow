//! `later[fn]` — background work for scripts and servers.
//!
//! Semantics: the fn runs off the caller's path on its own thread;
//! overlapping schedules of the same fn are skipped (skip-if-running);
//! `running[fn]` reports in-flight status; the process waits for all
//! pending work before exiting (`wait_all`, emitted at the end of main).
//! In the interpreter the work runs synchronously inline — a legal
//! schedule — which keeps tests deterministic.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

fn registry() -> &'static Mutex<HashSet<String>> {
    static R: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashSet::new()))
}

fn handles() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static H: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Vec::new()))
}

/// Removes the registry entry even if the work panics.
struct InFlight(String);
impl Drop for InFlight {
    fn drop(&mut self) {
        registry().lock().unwrap().remove(&self.0);
    }
}

/// Schedule `work` under `name`. Returns false (and does nothing) if a
/// schedule with the same name is still in flight.
pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    {
        let mut reg = registry().lock().unwrap();
        if !reg.insert(name.to_string()) {
            return false;
        }
    }
    let guard_name = name.to_string();
    let handle = std::thread::spawn(move || {
        let _guard = InFlight(guard_name);
        work();
    });
    handles().lock().unwrap().push(handle);
    true
}

/// Is a schedule with this name still in flight?
pub fn running(name: &str) -> bool {
    registry().lock().unwrap().contains(name)
}

/// Block until every scheduled work item has finished. Called at the end
/// of main so background work can't silently vanish at exit.
pub fn wait_all() {
    loop {
        let handle = handles().lock().unwrap().pop();
        match handle {
            Some(h) => {
                let _ = h.join();
            }
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn spawn_runs_and_wait_all_joins() {
        static RAN: AtomicUsize = AtomicUsize::new(0);
        assert!(spawn("t1", || {
            RAN.fetch_add(1, Ordering::SeqCst);
        }));
        wait_all();
        assert_eq!(RAN.load(Ordering::SeqCst), 1);
        assert!(!running("t1"));
    }

    #[test]
    fn overlapping_schedules_skip() {
        static RAN: AtomicUsize = AtomicUsize::new(0);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert!(spawn("t2", move || {
            rx.recv().unwrap();
            RAN.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(running("t2"));
        // Second schedule while the first is blocked: skipped.
        assert!(!spawn("t2", || {
            RAN.fetch_add(100, Ordering::SeqCst);
        }));
        tx.send(()).unwrap();
        wait_all();
        assert_eq!(RAN.load(Ordering::SeqCst), 1);
        // After completion the name is free again.
        assert!(spawn("t2", || {
            RAN.fetch_add(10, Ordering::SeqCst);
        }));
        wait_all();
        assert_eq!(RAN.load(Ordering::SeqCst), 11);
    }

    #[test]
    fn panicking_work_frees_the_name() {
        assert!(spawn("t3", || panic!("boom")));
        wait_all();
        assert!(!running("t3"), "panic must not leak the in-flight entry");
    }
}
