//! `use io`: `csv` and `lines`, rows of the builtin table (crate::table),
//! their runtime functions in `crow_runtime::io`, which both modes call.
//! (`read[]` and `write[]` are core builtins — no duplication, rule 17.)
use crate::modules::CrowModule;

pub struct IoModule;

/// The io module's functions: its rows' names, derived from the table (a row added had to
/// be listed here too, and `fma` was not).
pub(crate) static IO_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("io"));

impl CrowModule for IoModule {
    fn name(&self) -> &str { "io" }

    fn functions(&self) -> &[&str] { &IO_FUNCTIONS }
}
