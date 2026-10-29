//! `later[fn]` — background work for scripts and servers.
//!
//! Semantics: the fn runs off the caller's path on its own thread;
//! schedules of the same fn coalesce, never stack — one asked while the fn
//! is in flight runs once more when it ends (the latest schedule's work,
//! however many arrived meanwhile), so its last run reads every write made
//! before it (spec §15); `running[fn]` reports in-flight status; the
//! process waits for all pending work before exiting (`wait_all`, emitted
//! at the end of main). In the interpreter the work runs synchronously
//! inline — a legal schedule — which keeps tests deterministic.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

/// Work a schedule hands over.
type Work = Box<dyn FnOnce() + Send>;

/// The names in flight, each with the work asked again while it ran.
fn registry() -> &'static Mutex<HashMap<String, Option<Work>>> {
    static R: OnceLock<Mutex<HashMap<String, Option<Work>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn handles() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static H: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Vec::new()))
}

/// Frees the name where the work panics (a work that ends frees it under
/// the registry's lock, deciding there that nothing was asked again).
struct InFlight(String);
impl Drop for InFlight {
    fn drop(&mut self) {
        if std::thread::panicking() {
            crate::lock(registry()).remove(&self.0);
        }
    }
}

/// Schedule `work` under `name`. Returns true where it starts a run, false
/// where a run of the name is in flight: then it runs once more when that
/// run ends (the latest schedule's work).
pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    {
        let mut reg = crate::lock(registry());
        if let Some(again) = reg.get_mut(name) {
            *again = Some(Box::new(work));
            return false;
        }
        reg.insert(name.to_string(), None);
    }
    let guard_name = name.to_string();
    let handle = std::thread::spawn(move || {
        let _guard = InFlight(guard_name.clone());
        let mut next: Work = Box::new(work);
        loop {
            next();
            // asked again meanwhile: run once more; else the name is free,
            // decided under the lock a schedule takes
            let mut reg = crate::lock(registry());
            match reg.get_mut(&guard_name).and_then(Option::take) {
                Some(w) => next = w,
                None => {
                    reg.remove(&guard_name);
                    break;
                }
            }
        }
    });
    crate::lock(handles()).push(handle);
    true
}

/// Is a schedule with this name still in flight?
pub fn running(name: &str) -> bool {
    crate::lock(registry()).contains_key(name)
}

/// Block until every scheduled work item has finished. Called at the end
/// of main so background work can't silently vanish at exit.
pub fn wait_all() {
    loop {
        let handle = crate::lock(handles()).pop();
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

    /// The registry and the handles are process-global: a test that joins
    /// while another spawns can join the other's job and return before its
    /// own ran. The tests take turns.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn spawn_runs_and_wait_all_joins() {
        let _g = serial();
        static RAN: AtomicUsize = AtomicUsize::new(0);
        assert!(spawn("t1", || {
            RAN.fetch_add(1, Ordering::SeqCst);
        }));
        wait_all();
        assert_eq!(RAN.load(Ordering::SeqCst), 1);
        assert!(!running("t1"));
    }

    #[test]
    fn overlapping_schedules_coalesce() {
        let _g = serial();
        static RAN: AtomicUsize = AtomicUsize::new(0);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert!(spawn("t2", move || {
            rx.recv().unwrap();
            RAN.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(running("t2"));
        // Schedules while the first is blocked: the latest runs once more
        // when it ends, the earlier ones none.
        assert!(!spawn("t2", || {
            RAN.fetch_add(100, Ordering::SeqCst);
        }));
        assert!(!spawn("t2", || {
            RAN.fetch_add(1000, Ordering::SeqCst);
        }));
        tx.send(()).unwrap();
        wait_all();
        assert_eq!(RAN.load(Ordering::SeqCst), 1001);
        assert!(!running("t2"));
        // After completion the name is free again.
        assert!(spawn("t2", || {
            RAN.fetch_add(10, Ordering::SeqCst);
        }));
        wait_all();
        assert_eq!(RAN.load(Ordering::SeqCst), 1011);
    }

    #[test]
    fn panicking_work_frees_the_name() {
        let _g = serial();
        assert!(spawn("t3", || panic!("boom")));
        wait_all();
        assert!(!running("t3"), "panic must not leak the in-flight entry");
    }
}
