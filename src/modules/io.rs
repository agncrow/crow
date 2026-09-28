//! `use io`: `csv` and `lines`, rows of the builtin table (crate::table),
//! their runtime functions in `crow_runtime::io`, which both modes call.
//! (`read[]` and `write[]` are core builtins — no duplication, rule 17.)
use crate::modules::CrowModule;

pub struct IoModule;

pub(crate) const IO_FUNCTIONS: &[&str] = &["csv", "lines"];

impl CrowModule for IoModule {
    fn name(&self) -> &str { "io" }

    fn functions(&self) -> &[&str] { IO_FUNCTIONS }
}
