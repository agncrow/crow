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
    let value = consumed(existing, value, in_body);
    // whole numbers held as a tensor (a literal, `q = [1]`) given a list of
    // ints the body builds (`q = big_mul[q, k]`, its limbs pushed one by
    // one) hold a list of ints from the declaration on: the tensor's form
    // had converted the numbers to floats and back at every assignment and
    // every call, and lost an int past 2^53
    if is_ints(existing) && matches!(&value, Ty::List(item) if matches!(item.as_ref(), Ty::Int)) {
        return Ty::List(Box::new(Ty::Int));
    }
    let joined = existing.unify(&value);
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
        (Ty::Map(kh, held), Ty::Map(kj, joined)) => Ty::Map((*kh).unify(kj).unwrap_or(*kh), Box::new(settle(held, *joined, false))),
        (_, joined) => joined,
    }
}

/// A tensor of whole numbers, one axis (a literal list of ints).
pub(crate) fn is_ints(t: &Ty) -> bool {
    matches!(t, Ty::Tensor(e, dims) if matches!(e.as_ref(), Ty::Int) && dims.as_ref().is_none_or(|d| d.len() <= 1))
}

/// Whether a variable holding `held` refuses a failed value whose payload
/// is `payload`: it holds a plain value the payload fits, so its first
/// value could not fail, and the failure is refused where it is assigned.
pub(crate) fn refuses_failed(held: &Ty, payload: &Ty) -> bool {
    // (a list or a map whose items nothing has typed yet is still no
    // value that can fail: `out = []`, then `out = push[out, x]` with an x
    // that can fail, is a consumption point — left undecided, the push's
    // failure had been assigned to the list in a function no call typed,
    // and the lowered program disagreed with itself; found writing
    // Magpie's read verbs in Crow)
    let open_collection = matches!(held, Ty::List(_) | Ty::Map(..));
    !matches!(held, Ty::Result(_)) && (!held.has_unknown() || open_collection) && held.compatible(payload)
}
