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
pub(crate) fn consumed(existing: &CrowType, value: CrowType, in_body: bool) -> CrowType {
    match value {
        CrowType::Result(inner) if in_body && !matches!(existing, CrowType::Unknown | CrowType::Result(_)) => *inner,
        other => other,
    }
}

/// The type a variable settles on after a later assignment: what it held,
/// unified with what the value lands as.
pub(crate) fn settle(existing: &CrowType, value: CrowType, in_body: bool) -> CrowType {
    existing.unify(&consumed(existing, value, in_body))
}

/// Whether a variable holding `held` refuses a failed value whose payload
/// is `payload`: it holds a plain value the payload fits, so its first
/// value could not fail, and the failure is refused where it is assigned.
pub(crate) fn refuses_failed(held: &CrowType, payload: &CrowType) -> bool {
    !matches!(held, CrowType::Result(_)) && !format!("{:?}", held).contains("Unknown") && held.compatible(payload)
}
