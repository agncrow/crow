//! `use math.stats`: every function is a row of the builtin table
//! (crate::table), written in Crow (src/modules/stats.co) over the math
//! builtins and the sys layer's special functions, which both modes call.
use crate::modules::CrowModule;

pub struct StatsModule;

pub(crate) const STATS_FUNCTIONS: &[&str] = &[
    "median", "std", "var", "percentile", "zscore", "corr", "cov", "ttest", "linreg",
    "chisq", "anova", "kstest", "fit",
];

impl CrowModule for StatsModule {
    fn name(&self) -> &str { "math.stats" }

    fn functions(&self) -> &[&str] { STATS_FUNCTIONS }
}
