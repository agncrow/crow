//! Process memory (spec, Process memory): records a program holds for the
//! process under names, shared by its threads — what a module's functions
//! keep between calls (Magpie's sequencer: the rows put since the last
//! flush, the key index, the newest row per key). A name holds one kind of
//! record, the first it was given; a read, a revision or a write of another
//! kind is refused, naming both, in both modes. A revision runs under the
//! name's lock, so two threads revising one name take turns; the function
//! a revision runs reaches only the record it is handed — a call from it
//! to its own name again is refused, where it would wait on itself.
//!
//! The compiled program holds each record as its own type (`Record`, which
//! the emitter implements for every record kind a program declares); the
//! interpreter holds its values, under the same names and the same rules.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// A record kind as the compiled program declares it: the name its IR
/// gives the kind (one per program, a module's own prefixed), and the
/// name a message shows.
pub trait Record: Any + Send + Sync + Clone {
    const KIND: &'static str;
    const SHOWN: &'static str;
}

/// What a name holds: its record's kind, how a message shows it, and the
/// record.
pub struct Held {
    pub kind: String,
    pub shown: String,
    pub value: Box<dyn Any + Send + Sync>,
}

/// A name's place: what it holds, behind the lock a revision takes.
pub type Slot = Arc<Mutex<Option<Held>>>;

fn registry() -> &'static Mutex<HashMap<String, Slot>> {
    static R: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The place of a name, made empty at its first use.
pub fn slot(name: &str) -> Slot {
    let mut r = crate::lock(registry());
    r.entry(name.to_string()).or_insert_with(|| Arc::new(Mutex::new(None))).clone()
}

thread_local! {
    /// The names this thread is revising.
    static REVISING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Refused where this thread is revising the name: the call would wait on
/// the lock its own revision holds.
pub fn not_revising(name: &str) -> Result<(), String> {
    if REVISING.with(|r| r.borrow().contains(name)) {
        return Err(format!("memory: '{}' is being revised — a revision's function reaches only the record it is handed, and answers the one to hold", name));
    }
    Ok(())
}

/// The refusal of a record of another kind than the one a name holds.
pub fn other_kind(name: &str, held: &str, given: &str) -> String {
    format!("memory: '{}' holds {}, not {} — a name holds one kind of record", name, held, given)
}

/// Whether what a name holds is of the kind given: nothing held is.
pub fn check(name: &str, held: &Option<Held>, kind: &str, shown: &str) -> Result<(), String> {
    match held {
        Some(h) if h.kind != kind => Err(other_kind(name, &h.shown, shown)),
        _ => Ok(()),
    }
}

/// The record a name holds, of the kind given: none where nothing is held.
pub fn held_value<V: Any + Clone>(name: &str, held: &Option<Held>, kind: &str, shown: &str) -> Result<Option<V>, String> {
    check(name, held, kind, shown)?;
    match held {
        None => Ok(None),
        Some(h) => h.value.downcast_ref::<V>().cloned().map(Some).ok_or_else(|| other_kind(name, &h.shown, shown)),
    }
}

/// The record held for the process under the name, replacing what was.
pub fn remember_as<V: Any + Send + Sync>(name: &str, kind: &str, shown: &str, value: V) -> Result<(), String> {
    not_revising(name)?;
    let s = slot(name);
    let mut held = crate::lock(&s);
    check(name, &held, kind, shown)?;
    *held = Some(Held { kind: kind.to_string(), shown: shown.to_string(), value: Box::new(value) });
    Ok(())
}

/// The record the name holds, or the default where it holds none.
pub fn recall_as<V: Any + Clone>(name: &str, kind: &str, shown: &str, default: V) -> Result<V, String> {
    not_revising(name)?;
    let s = slot(name);
    let held = crate::lock(&s);
    Ok(held_value(name, &held, kind, shown)?.unwrap_or(default))
}

/// Nothing held under the name, nor under any name beneath it (the name
/// and a slash: `log/index/k` is beneath `log/index`): a later read answers
/// its default, and the name may take a record of another kind. Names are
/// a tree, so what a module keeps per key under a name it forgets at once.
pub fn forget(name: &str) -> Result<(), String> {
    not_revising(name)?;
    let beneath = format!("{}/", name.trim_end_matches('/'));
    let names: Vec<String> = crate::lock(registry()).keys().filter(|n| n.as_str() == name || n.starts_with(&beneath)).cloned().collect();
    for n in &names {
        not_revising(n)?;
    }
    for n in names {
        let s = slot(&n);
        *crate::lock(&s) = None;
    }
    Ok(())
}

/// Every name beginning with the prefix forgotten.
pub fn forget_under(prefix: &str) -> Result<(), String> {
    let names: Vec<String> = crate::lock(registry()).keys().filter(|n| n.starts_with(prefix)).cloned().collect();
    for n in names {
        let s = slot(&n);
        *crate::lock(&s) = None;
    }
    Ok(())
}

/// A path as a name for what is kept of it: absolute, its links resolved —
/// the path as given where it cannot be resolved (it does not exist yet).
pub fn canonical(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| path.to_string())
}

/// Every name that is the path, or a path under it, forgotten: what a
/// program keeps for a path goes with the path (`rm`). A name a program
/// gives a path's record is its canonical path (`canonical`), as given
/// and as resolved both, since the path is gone by the time a reader asks.
pub fn forget_path(path: &str) {
    if path.is_empty() {
        return;
    }
    for root in [path.to_string(), canonical(path)] {
        // (a name being revised by this thread is left: its revision ends
        // with what it answers)
        let _ = forget(&root);
    }
}

/// A revision: `f` run under the name's lock, handed the record held (the
/// default where none is) — handed over, not copied, so a revision of a
/// large record costs what `f` does — and the record it keeps (`Some`)
/// held afterwards. Where `f`'s answer can fail (`can_fail`), the record
/// is copied first, and a revision that keeps nothing leaves it as it was;
/// where it cannot, `f` always keeps one. Answers what `f` answers.
pub fn revise_as<V: Any + Send + Sync + Clone, R>(name: &str, kind: &str, shown: &str, default: V, can_fail: bool, f: impl FnOnce(V) -> (Option<V>, R)) -> Result<R, String> {
    not_revising(name)?;
    let s = slot(name);
    let mut held = crate::lock(&s);
    check(name, &held, kind, shown)?;
    let current: V = match held.take() {
        None => default,
        Some(h) => match h.value.downcast::<V>() {
            Ok(v) => *v,
            Err(value) => {
                let shown_held = h.shown.clone();
                *held = Some(Held { kind: h.kind, shown: h.shown, value });
                return Err(other_kind(name, &shown_held, shown));
            }
        },
    };
    let backup = if can_fail { Some(current.clone()) } else { None };
    REVISING.with(|r| r.borrow_mut().insert(name.to_string()));
    // (the name let go however `f` ends: a stop inside it unwinds through
    // here in a served program, and the next request may revise it)
    struct Release(String);
    impl Drop for Release {
        fn drop(&mut self) {
            REVISING.with(|r| r.borrow_mut().remove(&self.0));
        }
    }
    let release = Release(name.to_string());
    let (kept, answer) = f(current);
    drop(release);
    if let Some(v) = kept.or(backup) {
        *held = Some(Held { kind: kind.to_string(), shown: shown.to_string(), value: Box::new(v) });
    }
    Ok(answer)
}

/// The compiled program's: a record of its own kind held.
pub fn remember<T: Record>(name: &str, value: T) -> Result<(), String> {
    remember_as(name, T::KIND, T::SHOWN, value)
}

/// The compiled program's: the record held, or the default.
pub fn recall<T: Record>(name: &str, default: T) -> Result<T, String> {
    recall_as(name, T::KIND, T::SHOWN, default)
}

/// The compiled program's: a revision of a record of its own kind.
pub fn revise<T: Record, R>(name: &str, default: T, can_fail: bool, f: impl FnOnce(T) -> (Option<T>, R)) -> Result<R, String> {
    revise_as(name, T::KIND, T::SHOWN, default, can_fail, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Count(i64);
    impl Record for Count {
        const KIND: &'static str = "Count";
        const SHOWN: &'static str = "Count";
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Other(String);
    impl Record for Other {
        const KIND: &'static str = "Other";
        const SHOWN: &'static str = "Other";
    }

    /// A name answers its default until it holds a record, holds one kind,
    /// and takes another once forgotten; a revision keeps what its
    /// function keeps, and threads revising one name take turns.
    #[test]
    fn a_name_holds_one_kind_and_revisions_take_turns() {
        let name = "memory-test-count";
        assert_eq!(recall(name, Count(7)), Ok(Count(7)));
        remember(name, Count(1)).expect("held");
        assert_eq!(recall(name, Count(0)), Ok(Count(1)));
        assert!(recall(name, Other(String::new())).expect_err("another kind").contains("holds Count, not Other"));
        assert!(remember(name, Other("x".into())).is_err());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        revise(name, Count(0), false, |c| (Some(Count(c.0 + 1)), ())).expect("revised");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("joined");
        }
        assert_eq!(recall(name, Count(0)), Ok(Count(801)));
        // a revision that keeps nothing changes nothing
        assert_eq!(revise(name, Count(0), true, |c| (None, c.0)), Ok(801));
        assert_eq!(recall(name, Count(0)), Ok(Count(801)));
        // a revision's function reaching its own name is refused
        let inner = revise(name, Count(0), false, |c| {
            let again = recall(name, Count(0));
            (Some(c), again)
        })
        .expect("revised");
        assert!(inner.expect_err("waits on itself").contains("is being revised"));
        // a name's forgetting takes the names beneath it
        remember("memory-test-count/k1", Count(5)).expect("held");
        forget(name).expect("forgotten");
        assert_eq!(recall("memory-test-count/k1", Count(0)), Ok(Count(0)));
        assert_eq!(recall(name, Other("d".into())), Ok(Other("d".into())));
        remember(name, Other("o".into())).expect("held");
        forget_under("memory-test-").expect("forgotten");
        assert_eq!(recall(name, Count(3)), Ok(Count(3)));
    }
}
