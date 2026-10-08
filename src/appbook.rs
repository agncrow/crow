//! The app's book (app-baseline-design.md §8): `crow book` inside an app
//! derives what the app does and writes a book — the app, its pages with
//! composition trees and wireframes, a chapter per part, the data, the
//! changes since the accepted baseline — as Markdown rendered by Crow's
//! own `doc` module with the Crow Book's page shell, then serves it.

use crate::baseline::{Artifacts, Change, Rule, Summary};
use crate::wireframe::Wire;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub struct Input<'a> {
    pub entry: &'a str,
    pub name: String,
    pub artifacts: &'a Artifacts,
    pub rules: &'a [Rule],
    pub changes: &'a [Change],
    pub baseline: Option<&'a Summary>,
    /// part file → its leading comment
    pub descriptions: BTreeMap<String, String>,
    /// (part file, test name, steps)
    pub tests: Vec<(String, String, Vec<String>)>,
    pub wide: bool,
}

const BOOK_DIR: &str = ".crow/book";

fn slug(file: &str) -> String {
    file.trim_end_matches(".co").chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string()
}

fn decl(key: &str) -> &str { key.split(' ').nth(1).unwrap_or(key) }

fn link_decl(name: &str, kind: &str, files: &BTreeMap<String, String>) -> String {
    match files.get(&format!("{} {}", kind, name)) {
        Some(f) => format!("[{}](parts/{}.md#{})", name, slug(f), name.replace('_', "-")),
        None => name.to_string(),
    }
}

/// The lines of an artifact after its header lines (the paths of a
/// handler, the tree of a view).
fn body_of(text: &str, skip_prefixes: &[&str]) -> String {
    text.lines().skip(1).filter(|l| !skip_prefixes.iter().any(|p| l.trim_start().starts_with(p))).collect::<Vec<_>>().join("\n")
}

fn header_field(text: &str, field: &str) -> String {
    text.lines().find_map(|l| l.trim_start().strip_prefix(field).map(|v| v.trim().to_string())).unwrap_or_default()
}

/// Write the Markdown chapters, the summary and the site program; render
/// with the interpreter; return the site directory.
pub fn build(input: &Input) -> Result<PathBuf, String> {
    let art = input.artifacts;
    let s = &art.structure;
    let src = PathBuf::from(BOOK_DIR).join("src");
    let site = PathBuf::from(BOOK_DIR).join("site");
    std::fs::create_dir_all(src.join("parts")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&site).map_err(|e| e.to_string())?;
    let mut wire = Wire::new(art);
    let widths: [(f64, &str); 2] = [(1280.0, "1280"), (390.0, "390")];

    // ---- parts: which declarations live where
    let mut parts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, file) in &art.files { parts.entry(file.clone()).or_default().push(key.clone()); }
    let count = |keys: &[String], kind: &str| keys.iter().filter(|k| k.starts_with(&format!("{} ", kind))).count();

    // ---- index
    let mut index = format!("# {}\n\n", input.name);
    if let Some(d) = input.descriptions.get(input.entry) { index.push_str(&format!("{}\n\n", d)); }
    index.push_str(&format!(
        "{} part{} · {} facts · {} derivations · {} handlers · {} views · {} page{}\n\n",
        parts.len(), if parts.len() == 1 { "" } else { "s" },
        art.entries.keys().filter(|k| k.starts_with("fact ")).count(),
        art.entries.keys().filter(|k| k.starts_with("derive ")).count(),
        art.entries.keys().filter(|k| k.starts_with("handler ")).count(),
        art.entries.keys().filter(|k| k.starts_with("view ")).count(),
        s.pages.len(), if s.pages.len() == 1 { "" } else { "s" }
    ));
    index.push_str("Every page of this book is derived from the source on each start of `crow book`: nothing here is written by hand except the comment each part opens with.\n\n");
    index.push_str("## Pages\n\n");
    for (path, v) in &s.pages { index.push_str(&format!("- `{}` → {} — see [Pages](pages.md)\n", path, link_decl(v, "view", &art.files))); }
    index.push_str("\n## Parts\n\n| Part | Holds | What it is |\n|---|---|---|\n");
    for (file, keys) in &parts {
        let holds = format!("{} facts, {} derivations, {} handlers, {} views", count(keys, "fact"), count(keys, "derive"), count(keys, "handler"), count(keys, "view"));
        let desc = input.descriptions.get(file).cloned().unwrap_or_default();
        index.push_str(&format!("| [{}](parts/{}.md) | {} | {} |\n", file, slug(file), holds, desc));
    }
    index.push_str("\n## How to read it\n\n1. **Pages** shows how the UI is composed: each page, the views under the conditions that show them, the inputs they bind, the controls and what they fire, and a wireframe per state at desktop and phone widths.\n2. **A part's chapter** shows its facts, derivations, handlers (every path with what it writes and does), views (tree and wireframes) and tests.\n3. **Data** is the wiring across the app: who writes and reads each fact, what each derivation reads, which handler writes which table.\n4. **Changes** is what moved since the accepted baseline, as `crow check` reports it.\n");
    std::fs::write(src.join("index.md"), index).map_err(|e| e.to_string())?;

    // ---- pages
    let mut pages = String::from("# Pages\n\n");
    for (path, v) in &s.pages {
        pages.push_str(&format!("## page {}\n\nRendered by {}.\n\n### Composition\n\n```text\n{}```\n\n", path, link_decl(v, "view", &art.files), page_tree(art, v)));
        pages.push_str("### Wireframes\n\nOne per state of the page, laid out by the kernel's layout engine from the page's tree with placeholders — ‹name› stands for a value, a table shows its columns and three rows, a chart is its box.\n\n");
        if let Some(paths) = s.wires.get(v) {
            for (guard, node) in paths {
                let state = if guard.is_empty() { "always".to_string() } else { format!("when {}", guard) };
                pages.push_str(&format!("#### {}\n\n", state));
                for (w, label) in widths {
                    match wire.svg(node, guard, w, input.wide && w > 600.0, false) {
                        Ok(svg) => pages.push_str(&format!("**{} px**\n\n{}\n\n", label, svg)),
                        Err(e) => pages.push_str(&format!("**{} px** — could not lay out: {}\n\n", label, e)),
                    }
                }
            }
        }
    }
    std::fs::write(src.join("pages.md"), pages).map_err(|e| e.to_string())?;

    // ---- one chapter per part
    let mut summary = format!("# {}\n\n- [The app](index.md)\n- [Pages](pages.md)\n\n# Parts\n\n", input.name);
    for (file, keys) in &parts {
        let mut md = format!("# {}\n\n", file);
        if let Some(d) = input.descriptions.get(file) { md.push_str(&format!("{}\n\n", d)); }
        let facts: Vec<&String> = keys.iter().filter(|k| k.starts_with("fact ")).collect();
        if !facts.is_empty() {
            md.push_str("## Facts\n\n| Fact | Type | Bound by | Written by | Read by |\n|---|---|---|---|---|\n");
            for k in facts {
                let name = decl(k);
                let text = &art.entries[k.as_str()];
                let (b, w, r) = s.facts.get(name).cloned().unwrap_or_default();
                md.push_str(&format!("| <a id=\"{}\"></a>`{}` | {} | {} | {} | {} |\n", name.replace('_', "-"), name, header_field(text, "type:"),
                    b.iter().map(|x| link_decl(x, "view", &art.files)).collect::<Vec<_>>().join(", "),
                    w.iter().map(|x| link_decl(x, "handler", &art.files)).collect::<Vec<_>>().join(", "),
                    if r.is_empty() { "nobody".to_string() } else { r.iter().cloned().collect::<Vec<_>>().join(", ") }));
            }
            md.push('\n');
        }
        let derives: Vec<&String> = keys.iter().filter(|k| k.starts_with("derive ")).collect();
        if !derives.is_empty() {
            md.push_str("## Derivations\n\n");
            for k in derives {
                let name = decl(k);
                let text = &art.entries[k.as_str()];
                md.push_str(&format!("<a id=\"{}\"></a>**{}** — {}", name.replace('_', "-"), name, header_field(text, "reads:")));
                let tables = header_field(text, "tables:");
                if !tables.is_empty() { md.push_str(&format!(" · tables {}", tables)); }
                md.push_str(&format!(" · read by {}\n\n", header_field(text, "read by:")));
            }
        }
        let handlers: Vec<&String> = keys.iter().filter(|k| k.starts_with("handler ")).collect();
        if !handlers.is_empty() {
            md.push_str("## Handlers\n\nEach handler as every path through it: the condition, what it writes and from what, the effects it performs. `vN` names a value read on the way; `$N` a parameter; `may` what a loop may do.\n\n");
            for k in handlers {
                let name = decl(k);
                let text = &art.entries[k.as_str()];
                let wired = header_field(text, "wired:");
                let called = header_field(text, "called by:");
                md.push_str(&format!("### <a id=\"{}\"></a>{}\n\n", name.replace('_', "-"), name));
                if !wired.is_empty() { md.push_str(&format!("Wired: {}\n\n", wired)); }
                if !called.is_empty() { md.push_str(&format!("Called by: {}\n\n", called)); }
                if let Some(reach) = art.reach.get(k.as_str()) { if !reach.is_empty() { md.push_str(&format!("Reaches: {}\n\n", reach.join(", "))); } }
                md.push_str(&format!("```text\n{}\n```\n\n", body_of(text, &["wired:", "called by:", "inputs:"])));
            }
        }
        let views: Vec<&String> = keys.iter().filter(|k| k.starts_with("view ")).collect();
        if !views.is_empty() {
            md.push_str("## Views\n\n");
            for k in views {
                let name = decl(k);
                let text = &art.entries[k.as_str()];
                md.push_str(&format!("### <a id=\"{}\"></a>{}\n\nComposed on: {}\n\n```text\n{}\n```\n\n", name.replace('_', "-"), name, header_field(text, "composed on:"), body_of(text, &["composed on:", "reads:"])));
                if let Some(paths) = s.wires.get(name) {
                    for (guard, node) in paths {
                        let state = if guard.is_empty() { "always".to_string() } else { format!("when {}", guard) };
                        md.push_str(&format!("**{}**\n\n", state));
                        for (w, label) in widths {
                            match wire.svg(node, guard, w, input.wide && w > 600.0, true) {
                                Ok(svg) => md.push_str(&format!("{} px\n\n{}\n\n", label, svg)),
                                Err(e) => md.push_str(&format!("{} px — could not lay out: {}\n\n", label, e)),
                            }
                        }
                    }
                }
            }
        }
        let tests: Vec<&(String, String, Vec<String>)> = input.tests.iter().filter(|(f, _, _)| f == file).collect();
        if !tests.is_empty() {
            md.push_str("## Tests\n\nEach named test is a scenario: what it opens, what it clicks and changes, what it expects.\n\n");
            for (_, name, steps) in tests {
                md.push_str(&format!("### {}\n\n", name));
                for (i, st) in steps.iter().enumerate() { md.push_str(&format!("{}. {}\n", i + 1, st)); }
                md.push('\n');
            }
        }
        std::fs::write(src.join("parts").join(format!("{}.md", slug(file))), md).map_err(|e| e.to_string())?;
        summary.push_str(&format!("- [{}](parts/{}.md)\n", file, slug(file)));
    }

    // ---- data
    let mut data = String::from("# Data\n\nThe wiring across the app: every fact with who writes and reads it, every derivation with what it reads, every effect a handler performs.\n\n## Facts\n\n| Fact | Part | Bound by | Written by | Read by |\n|---|---|---|---|---|\n");
    for (f, (b, w, r)) in &s.facts {
        let file = art.files.get(&format!("fact {}", f)).cloned().unwrap_or_default();
        data.push_str(&format!("| `{}` | [{}](parts/{}.md) | {} | {} | {} |\n", f, file, slug(&file), b.iter().cloned().collect::<Vec<_>>().join(", "), w.iter().cloned().collect::<Vec<_>>().join(", "), if r.is_empty() { "nobody".to_string() } else { format!("{}", r.len()) }));
    }
    data.push_str("\n## Derivations\n\n| Derivation | Part | Reads | Tables |\n|---|---|---|---|\n");
    for d in &s.derives {
        if let Some(text) = art.entries.get(&format!("derive {}", d)) {
            let file = art.files.get(&format!("derive {}", d)).cloned().unwrap_or_default();
            data.push_str(&format!("| `{}` | [{}](parts/{}.md) | {} | {} |\n", d, file, slug(&file), header_field(text, "reads:"), header_field(text, "tables:")));
        }
    }
    data.push_str("\n## Effects\n\nWhat each handler does beyond writing facts.\n\n| Handler | Effects |\n|---|---|\n");
    for (h, es) in &s.effects {
        if es.is_empty() { continue; }
        data.push_str(&format!("| {} | {} |\n", link_decl(h, "handler", &art.files), es.iter().cloned().collect::<Vec<_>>().join(" · ")));
    }
    data.push_str("\n## Queries\n\nEvery query a handler reads, by handle.\n\n| Handler | Query |\n|---|---|\n");
    for (k, text) in &art.entries {
        if !k.starts_with("handler ") { continue; }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for l in text.lines() {
            let t = l.trim_start();
            if let Some(rest) = t.strip_prefix("reads ") {
                if rest.contains("= query[") {
                    let q: String = rest.chars().take(140).collect();
                    if seen.insert(q.clone()) { data.push_str(&format!("| {} | `{}` |\n", decl(k), q.replace('|', "\\|"))); }
                }
            }
        }
    }
    std::fs::write(src.join("data.md"), data).map_err(|e| e.to_string())?;

    // ---- changes
    let mut changes = String::from("# Changes\n\n");
    match input.baseline {
        None => changes.push_str("No baseline is recorded yet: run `crow check` once and it records what the app does; every check after diffs against it.\n"),
        Some(b) if b.first => changes.push_str(&format!("The baseline was just recorded: {} declarations. Every check from now on diffs against it.\n", b.recorded)),
        Some(b) => {
            if input.changes.is_empty() {
                changes.push_str(&format!("The baseline is current: {} declarations unchanged.\n", b.unchanged));
            } else {
                changes.push_str(&format!("{} changed since the accepted baseline. Accept with `a` in `crow check`, or `crow check --accept`.\n\n", input.changes.len()));
                for c in input.changes {
                    changes.push_str(&format!("## {}\n\nIn {}{}\n\n```text\n{}\n```\n\n", c.key, if c.file.is_empty() { "(removed)".to_string() } else { c.file.clone() },
                        if c.reach.is_empty() { String::new() } else { format!(" — reaches {}", c.reach.join(", ")) },
                        crate::baseline::compact_diff(&c.was, &c.now).join("\n")));
                }
            }
        }
    }
    if !input.rules.is_empty() {
        changes.push_str("\n## Findings\n\n");
        for r in input.rules { changes.push_str(&format!("- **{}** ({}): {} — {}\n", r.subject, r.file, r.detail, r.fix)); }
    }
    std::fs::write(src.join("changes.md"), changes).map_err(|e| e.to_string())?;

    summary.push_str("\n# Across the app\n\n- [Data](data.md)\n- [Changes](changes.md)\n");
    std::fs::write(src.join("SUMMARY.md"), summary).map_err(|e| e.to_string())?;

    // ---- the site program: the Crow Book's own page shell
    let site_crow = format!(r#"--- The app's book, rendered by Crow's doc module: written by `crow book`.
use web, ui, doc

src = '{src}/'
out = '{out}/'

view chapter[title, body, nav, pages, up]
    return stack[
        row[
            button['☰', class = 'nav-toggle', title = 'menu'],
            a['{name}', href = '`up`index.html', class = 'title'],
            row[
                input[class = 'doc-search', type = 'search', placeholder = 'Search  /', hidden_below = 'phone'],
                gap = 'm', align = 'center', class = 'links', hidden_below = 'phone'
            ],
            gap = 'm', align = 'center', justify = 'between', class = 'doc-top'
        ],
        div[class = 'doc-results', hidden = 'hidden'],
        row[
            stack[nav, width = '272px', hidden_below = 'tablet', class = 'doc-side'],
            stack[body, pages, fill = 'true', class = 'doc', pad = 'xl'],
            align = 'start'
        ]
    ]

toc = doc.chapters[read['`src`SUMMARY.md']]
for i in range[len[toc]]
    path = toc.path[i]
    title = toc.title[i]
    up = ''
    if contains[path, '/']
        up = '../'
        parts = split[path, '/']
        folder = parts[0]
        mkdir['`out``folder`']
    doc_page = doc.document[title, chapter[title, doc.markdown[read['`src``path`']], doc.menu[toc, path], doc.pager[toc, path], up]]
    html_path = replace[path, '.md', '.html']
    write['`out``html_path`', doc_page]
first = toc.path[0]
write['`out`index.html', doc.document[toc.title[0], chapter[toc.title[0], doc.markdown[read['`src``first`']], doc.menu[toc, first], doc.pager[toc, first], '']]]
entries = for i in range[len[toc]]
    path = toc.path[i]
    map[t = toc.title[i], p = replace[path, '.md', '.html'], x = doc.plain[read['`src``path`']]]
write['`out`search.json', encode[entries]]
"#, src = src.display(), out = site.display(), name = input.name.replace('\'', "\\'"));
    let site_path = PathBuf::from(BOOK_DIR).join("site.co");
    std::fs::write(&site_path, site_crow).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = std::process::Command::new(&exe).args(["run", "-i", site_path.to_str().unwrap_or("")]).status().map_err(|e| e.to_string())?;
    if !status.success() || !site.join("index.html").exists() {
        return Err("rendering the app's book failed".into());
    }
    Ok(site)
}

/// The composition tree under one page's view, as `crow check --show`
/// prints it, without the header line.
fn page_tree(art: &Artifacts, view: &str) -> String {
    let full = crate::baseline::render_structure(art, "x");
    let mut out = String::new();
    let mut in_page = false;
    for l in full.lines() {
        if l.starts_with("page ") { in_page = l.contains(&format!("@{} ", view)); if in_page { out.push_str(l); out.push('\n'); } continue; }
        if in_page {
            if l.is_empty() || l.starts_with("handlers") || l.starts_with("facts") { break; }
            out.push_str(l); out.push('\n');
        }
    }
    out
}

/// The steps a named test takes, as a reader would say them.
pub fn test_steps(body: &[crate::ast::Stmt]) -> Vec<String> {
    use crate::ast::{Expr, Stmt};
    fn text(e: &Expr) -> String {
        match e {
            Expr::Str(s) => format!("'{}'", s),
            Expr::Int(n) => n.to_string(),
            Expr::Float(f) => format!("{}", f),
            Expr::Bool(b) => b.to_string(),
            Expr::Ident(n) => n.clone(),
            Expr::Call { callee, args } => format!("{}[{}]", text(callee), args.iter().map(text).collect::<Vec<_>>().join(", ")),
            Expr::FieldAccess { object, field } => format!("{}.{}", text(object), field),
            Expr::BinOp { left, op, right } => format!("{} {} {}", text(left), format!("{:?}", op).to_lowercase(), text(right)),
            Expr::UnaryOp { expr, .. } => format!("not {}", text(expr)),
            Expr::Array(items) => format!("[{}]", items.iter().map(text).collect::<Vec<_>>().join(", ")),
            other => format!("{:?}", other),
        }
    }
    let mut steps = Vec::new();
    for st in body {
        let e = match st { Stmt::Assign { value, .. } => value, Stmt::ExprStmt(e) => e, _ => continue };
        if let Expr::Call { callee, args } = e {
            if let Expr::Ident(n) = callee.as_ref() {
                match (n.as_str(), args.as_slice()) {
                    ("app", [v, ..]) => steps.push(format!("open the view `{}`", text(v))),
                    ("click", [_, h]) => steps.push(format!("click `{}`", text(h))),
                    ("change", [_, k, v]) => steps.push(format!("set {} to {}", text(k).trim_matches('\''), text(v))),
                    ("assert", [c, ..]) => steps.push(format!("expect `{}`", text(c))),
                    _ => {}
                }
            }
        }
    }
    steps
}
