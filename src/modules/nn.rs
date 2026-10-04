//! `use math.nn`: written in Crow (nn.co, beside this file) over the math
//! builtins, design rule 20. Every function is a row of the builtin table
//! (crate::table, `crow!`) that states its contract; the body is the
//! module's source, inlined into every program that imports it
//! (crate::modules::inline_crow_modules) and called where the row is
//! called, in both modes.
use crate::modules::CrowModule;

pub struct NnModule;

/// The module's source: one copy, in the binary.
pub(crate) const NN_SOURCE: &str = include_str!("nn.co");

pub(crate) const NN_FUNCTIONS: &[&str] = &[
    "relu", "sigmoid", "softmax", "gelu",
    "mse", "crossentropy", "bce",
    "linear",
];

impl CrowModule for NnModule {
    fn name(&self) -> &str { "math.nn" }

    fn functions(&self) -> &[&str] { NN_FUNCTIONS }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module's functions are its source's, and each has its row.
    #[test]
    fn the_sources_functions_are_the_modules_rows() {
        let declared: Vec<&str> = NN_SOURCE.lines().filter_map(|l| l.strip_prefix("fn ")).map(|l| l.split('[').next().unwrap_or("")).filter(|f| !f.is_empty()).collect();
        for f in NN_FUNCTIONS {
            assert!(declared.contains(f), "nn.co declares {}", f);
            let row = crate::table::ROWS.iter().find(|r| r.module == "math.nn" && r.name == *f).unwrap_or_else(|| panic!("a row for {}", f));
            assert!(row.is_crow(), "the row of {} is written in Crow", f);
            assert_eq!(crate::table::crow_fn_name(row), format!("_math_nn_{}", f));
        }
        // the source's other functions are its own helpers, named apart
        for f in &declared {
            assert!(NN_FUNCTIONS.contains(f) || !crate::table::ROWS.iter().any(|r| r.name == *f && r.module == "math.nn"), "{} is a helper", f);
        }
    }
}
