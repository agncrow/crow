//! `use frame`: every function is a row of the builtin table
//! (crate::table), its runtime function in `crow_runtime::frames`, which
//! both modes call — `load` and `save` too, whose calls are effects the
//! session layer performs.
use crate::modules::CrowModule;

pub struct FrameModule;

/// The frame module's functions: its rows' names, derived from the table.
pub(crate) static FRAME_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("frame"));

impl CrowModule for FrameModule {
    fn name(&self) -> &str { "frame" }

    fn functions(&self) -> &[&str] { &FRAME_FUNCTIONS }
}
