//! The statement rules: how a variable's type settles across its
//! assignments, and when a variable refuses a value that failed. Shared by
//! inference and the lowering, as `rules.rs` shares the expression rules
//! (core-rewrite-notes §10, F5 step S3).

use super::*;

/// What a later value lands as in a body's variable. A variable whose first
/// value could not fail holds no error (spec, Reassignment), so a value that
/// can fail lands as its payload, and the lowering propagates the failure
/// where it is assigned (`ms = push[ms, m]` inside `if m.ok` kept `ms` a
/// list of numbers there, and a record built from it typed its field as a
/// list that could fail). Only in a body, where the first assignment binds
/// afresh: at the top level the fixpoint meets a declaration again, its own
/// last type in hand.
pub(crate) fn consumed(existing: &Ty, value: Ty, in_body: bool) -> Ty {
    match value {
        Ty::Result(inner) if in_body && !matches!(existing, Ty::Unknown | Ty::Result(_)) => *inner,
        other => other,
    }
}

/// The type a variable settles on after a later assignment: what it held,
/// unified with what the value lands as — in the form it was declared in.
/// Numbers held as a list and numbers held as a tensor join either way
/// (`unify`), but a variable's form is its declaration's, as the compiled
/// program declares it (a fact's as the state keeps it): numbers assigned
/// as a list to a variable holding a tensor land as the tensor, and the
/// other way, converted where they are assigned. The join alone had turned
/// a variable into a later value's form while its declaration kept the
/// first (finch's `ep_start = [0.0]`, given a record's list in a handler,
/// did not build; nor did `t = map[a = [1.5]]` given a list).
pub(crate) fn settle(existing: &Ty, value: Ty, in_body: bool) -> Ty {
    let joined = existing.unify(&consumed(existing, value, in_body));
    match (existing, joined) {
        (Ty::Tensor(_, dims), Ty::List(item)) => {
            let elem = match *item {
                Ty::Tensor(e, _) => *e,
                e => e,
            };
            Ty::Tensor(Box::new(elem), dims.clone())
        }
        (Ty::List(_), Ty::Tensor(elem, dims)) => {
            let item = match dims {
                Some(d) if d.len() >= 2 => Ty::Tensor(elem, Some(d[1..].to_vec())),
                _ => *elem,
            };
            Ty::List(Box::new(item))
        }
        // (a map's values, held either way, keep the map's form too)
        (Ty::Map(held), Ty::Map(joined)) => Ty::Map(Box::new(settle(held, *joined, false))),
        (_, joined) => joined,
    }
}

/// Whether a variable holding `held` refuses a failed value whose payload
/// is `payload`: it holds a plain value the payload fits, so its first
/// value could not fail, and the failure is refused where it is assigned.
pub(crate) fn refuses_failed(held: &Ty, payload: &Ty) -> bool {
    !matches!(held, Ty::Result(_)) && !held.has_unknown() && held.compatible(payload)
}
