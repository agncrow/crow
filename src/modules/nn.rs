//! `use math.nn`: every function is a row of the builtin table
//! (crate::table), its runtime function in `crow_runtime::nn`, which both
//! modes call.
use crate::modules::CrowModule;

pub struct NnModule;

pub(crate) const NN_FUNCTIONS: &[&str] = &[
    "relu", "sigmoid", "softmax", "gelu",
    "mse", "crossentropy", "bce",
    "linear",
];

impl CrowModule for NnModule {
    fn name(&self) -> &str { "math.nn" }

    fn functions(&self) -> &[&str] { NN_FUNCTIONS }
}
