//! `use doc` — documentation from Markdown, rendered by the kernel
//! (`crow_runtime::markdown`) so the interpreter and compiled programs
//! produce the same pages. Four names: `markdown[text]` (HTML for a
//! chapter), `chapters[summary]` (the table of contents as a frame),
//! `menu[chapters, current]` (navigation markup), `pager[chapters, current]`
//! (previous/next links), `plain[text]` (the words, for a search index), and
//! `document[title, body]` (a complete page with the kernel stylesheet).
use crate::modules::CrowModule;
use crate::type_infer::CrowType;
use crate::value::{CrowValue, Value};

pub struct DocModule;

pub(crate) const DOC_FUNCTIONS: &[&str] = &["markdown", "chapters", "menu", "pager", "plain", "document"];

impl CrowModule for DocModule {
    fn name(&self) -> &str { "doc" }

    fn functions(&self) -> &[&str] { DOC_FUNCTIONS }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        for a in args {
            if !a.ok {
                return Ok(a.clone());
            }
        }
        call_doc(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        let n = arg_strs.len();
        let out = match (name, n) {
            ("markdown", 1) => format!("crow_runtime::markdown::to_html(&{})", arg_strs[0]),
            ("chapters", 1) => format!("crow_runtime::markdown::chapters_frame(&{})", arg_strs[0]),
            ("menu", 2) => format!("crow_runtime::markdown::menu_from_frame(&{}, &{})", arg_strs[0], arg_strs[1]),
            ("pager", 2) => format!("crow_runtime::markdown::pager_from_frame(&{}, &{})", arg_strs[0], arg_strs[1]),
            ("plain", 1) => format!("crow_runtime::markdown::plain_text(&{})", arg_strs[0]),
            ("document", 2) => format!("crow_runtime::markdown::document(&{}, &{})", arg_strs[0], arg_strs[1]),
            _ => return Err(format!("{}: unknown doc function or wrong argument count ({})", name, n)),
        };
        Ok(out)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        match name {
            "chapters" => CrowType::Frame,
            // markup a view places as markup; the words and a whole page are text
            "markdown" | "menu" | "pager" => CrowType::Markup,
            "plain" | "document" => CrowType::Str,
            _ => CrowType::Unknown,
        }
    }
}

fn to_str<'a>(v: &'a CrowValue, fname: &str, what: &str) -> Result<&'a str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        other => Err(format!("{}: {} must be a string, got {}", fname, what, other.type_name())),
    }
}

fn call_doc(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "markdown" => {
            if args.len() != 1 { return Err("markdown: takes 1 argument — markdown[text]".into()); }
            let md = to_str(&args[0], name, "the text")?;
            Ok(CrowValue::markup(crow_runtime::markdown::to_html(md)))
        }
        "chapters" => {
            if args.len() != 1 { return Err("chapters: takes 1 argument — chapters[summary_text]".into()); }
            let s = to_str(&args[0], name, "the summary")?;
            Ok(CrowValue::ok(Value::Frame(crow_runtime::markdown::chapters_frame(s))))
        }
        "menu" => {
            if args.len() != 2 { return Err("menu: takes 2 arguments — menu[chapters, current_path]".into()); }
            let f = match &args[0].value {
                Value::Frame(f) => f,
                other => return Err(format!("menu: the first argument is the chapters frame, got {}", other.type_name())),
            };
            let current = to_str(&args[1], name, "the current path")?;
            Ok(CrowValue::markup(crow_runtime::markdown::menu_from_frame(f, current)))
        }
        "pager" => {
            if args.len() != 2 { return Err("pager: takes 2 arguments — pager[chapters, current_path]".into()); }
            let f = match &args[0].value {
                Value::Frame(f) => f,
                other => return Err(format!("pager: the first argument is the chapters frame, got {}", other.type_name())),
            };
            let current = to_str(&args[1], name, "the current path")?;
            Ok(CrowValue::markup(crow_runtime::markdown::pager_from_frame(f, current)))
        }
        "plain" => {
            if args.len() != 1 { return Err("plain: takes 1 argument — plain[markdown_text]".into()); }
            let md = to_str(&args[0], name, "the text")?;
            Ok(CrowValue::ok(Value::Str(crow_runtime::markdown::plain_text(md))))
        }
        "document" => {
            if args.len() != 2 { return Err("document: takes 2 arguments — document[title, body]".into()); }
            let title = to_str(&args[0], name, "the title")?;
            let body = to_str(&args[1], name, "the body")?;
            Ok(CrowValue::ok(Value::Str(crow_runtime::markdown::document(title, body))))
        }
        _ => Err(format!("{}: unknown doc function", name)),
    }
}
