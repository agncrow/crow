//! `use llm`: a language model as a connection — every function a row of
//! the builtin table (crate::table), written in Crow (src/modules/llm.co)
//! over `request` and the sys layer, which both modes call. The design is
//! docs/llm-design.md.
use crate::modules::CrowModule;

pub struct LlmModule;

/// The module's functions: its rows' names, derived from the table.
pub(crate) static LLM_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("llm"));

impl CrowModule for LlmModule {
    fn name(&self) -> &str {
        "llm"
    }

    fn functions(&self) -> &[&str] {
        &LLM_FUNCTIONS
    }
}
