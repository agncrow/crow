//! `use sys`: the standard library's own layer — bytes, and a connection
//! to a host read and written as bytes — rows of the builtin table
//! (crate::table), their runtime functions in `crow_runtime::sys`, which
//! both modes call. A program may not import it: only a module of the
//! standard library (the spec, "The standard library's layer").
use crate::modules::CrowModule;

pub struct SysModule;

/// The sys rows a standard module may call: every sys row's name, derived
/// from the table (a row left out of the list had been "undefined
/// function" in both modes).
pub(crate) static SYS_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("sys"));

impl CrowModule for SysModule {
    fn name(&self) -> &str { "sys" }

    fn functions(&self) -> &[&str] { &SYS_FUNCTIONS }
}
