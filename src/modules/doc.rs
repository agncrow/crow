//! `use doc` — documentation from Markdown, rendered by the kernel
//! (`crow_runtime::markdown`) so the interpreter and compiled programs
//! produce the same pages. `markdown[text]` (HTML for a chapter),
//! `chapters[summary]` (the table of contents as a frame),
//! `menu[chapters, current]` (navigation markup), `pager[chapters, current]`
//! (previous/next links), `plain[text]` (the words, for a search index), and
//! `document[title, body]` (a complete page with the kernel stylesheet):
//! each a row of the builtin table (crate::table).
use crate::modules::CrowModule;

pub struct DocModule;

/// The doc module's functions: its rows' names, derived from the table (a row added had to
/// be listed here too, and `fma` was not).
pub(crate) static DOC_FUNCTIONS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| crate::table::module_names("doc"));

impl CrowModule for DocModule {
    fn name(&self) -> &str { "doc" }

    fn functions(&self) -> &[&str] { &DOC_FUNCTIONS }
}
