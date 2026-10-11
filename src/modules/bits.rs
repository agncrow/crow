//! `use bits` — an int's bits and a character's code: `bitand`, `bitor`,
//! `bitxor`, `bitcount`, `shift`, `code` and `char`, each a row of the
//! builtin table (crate::table) whose function is the runtime's
//! (`crow_runtime::bits`), so both modes answer alike — for ints of any
//! size, read as two's complement.
use crate::modules::CrowModule;

pub struct BitsModule;

/// The module's functions: its rows' names, derived from the table.
pub(crate) static BITS_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("bits"));

impl CrowModule for BitsModule {
    fn name(&self) -> &str { "bits" }

    fn functions(&self) -> &[&str] { &BITS_FUNCTIONS }
}
