//! A handler's database transaction (core-design.md §1.2): the first write a
//! handler makes through a connection begins a transaction on it, every
//! later read and write through that connection is inside it, and it
//! commits when the handler does and rolls back when it stops. The live
//! runtime and the interpreter open the scope around each handler; the
//! database layer, which registers itself here, does the rest.

use std::sync::OnceLock;

/// What the database layer does when a handler's scope opens and closes.
pub struct Hooks {
    pub begin: fn(),
    pub finish: fn(bool) -> Result<(), String>,
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// The database layer registers once.
pub fn register(hooks: Hooks) {
    let _ = HOOKS.set(hooks);
}

/// A handler starts on this thread: its writes are one transaction per
/// connection until `finish`.
pub fn begin() {
    if let Some(h) = HOOKS.get() {
        (h.begin)();
    }
}

/// The handler ended: commit every transaction it opened, in the order it
/// opened them — or roll every one back when it stopped.
pub fn finish(commit: bool) -> Result<(), String> {
    match HOOKS.get() {
        Some(h) => (h.finish)(commit),
        None => Ok(()),
    }
}
