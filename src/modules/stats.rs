//! `use math.stats`: every function is a row of the builtin table
//! (crate::table), written in Crow (src/modules/stats.co) over the math
//! builtins and the sys layer's special functions, which both modes call.
use crate::modules::CrowModule;

pub struct StatsModule;

/// The stats module's functions: its rows' names, derived from the table (a row added had to
/// be listed here too, and `fma` was not).
pub(crate) static STATS_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("math.stats"));

impl CrowModule for StatsModule {
    fn name(&self) -> &str { "math.stats" }

    fn functions(&self) -> &[&str] { &STATS_FUNCTIONS }
}
