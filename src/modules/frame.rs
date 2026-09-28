//! `use frame`: every function is a row of the builtin table
//! (crate::table), its runtime function in `crow_runtime::frames`, which
//! both modes call — `load` and `save` too, whose calls are effects the
//! session layer performs.
use crate::modules::CrowModule;

pub struct FrameModule;

pub(crate) const FRAME_FUNCTIONS: &[&str] = &[
    "load", "save", "select", "filter", "sort", "group", "join", "stack",
    "head", "tail", "fill", "unique", "rename", "stats", "datetime", "tz",
    "new", "with",
];

impl CrowModule for FrameModule {
    fn name(&self) -> &str { "frame" }

    fn functions(&self) -> &[&str] { FRAME_FUNCTIONS }
}
