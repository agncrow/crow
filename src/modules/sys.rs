//! `use sys`: the standard library's own layer — bytes, and a connection
//! to a host read and written as bytes — rows of the builtin table
//! (crate::table), their runtime functions in `crow_runtime::sys`, which
//! both modes call. A program may not import it: only a module of the
//! standard library (the spec, "The standard library's layer").
use crate::modules::CrowModule;

pub struct SysModule;

pub(crate) const SYS_FUNCTIONS: &[&str] = &["open", "send", "recv", "close", "bytes", "text", "slice", "find", "concat", "byte"];

impl CrowModule for SysModule {
    fn name(&self) -> &str { "sys" }

    fn functions(&self) -> &[&str] { SYS_FUNCTIONS }
}
