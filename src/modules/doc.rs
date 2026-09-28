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

pub(crate) const DOC_FUNCTIONS: &[&str] = &["markdown", "chapters", "menu", "pager", "plain", "document"];

impl CrowModule for DocModule {
    fn name(&self) -> &str { "doc" }

    fn functions(&self) -> &[&str] { DOC_FUNCTIONS }
}
