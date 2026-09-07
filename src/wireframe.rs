//! Wireframes for the app's book (app-baseline-design.md §8): a view's
//! tree, laid out by the kernel's own layout engine with placeholders
//! where the tree has values, drawn as SVG. Nothing is rendered from
//! state and nothing runs against a database: every path of every view is
//! drawn from its conditional tree alone.

use crate::baseline::{Artifacts, WNode};
use crate::interpreter::{Interpreter, LayoutBox};
use std::collections::BTreeMap;

/// One element as emitted, in document order, so a box can carry its text.
struct Emitted {
    path: String,
    text: String,
    kind: &'static str,
}

pub struct Wire<'a> {
    art: &'a Artifacts,
    interp: Interpreter,
}

const CONTAINERS: &[&str] = &["stack", "row", "grid", "overlay", "scroll", "modal", "drawer", "toast", "header", "nav", "footer", "main", "section", "div", "card", "details"];
const LAYERS: &[&str] = &["modal", "drawer", "toast"];
const TEXTY: &[&str] = &["p", "span", "h1", "h2", "h3", "h4", "a", "small", "strong", "em", "label", "pre", "code", "summary", "li", "td", "th"];

/// What the kernel renders for an element: its tag, class, style and the
/// data attributes carrying responsive rules.
fn rendered(tag: &str, attrs: &[(String, String)]) -> (String, String, String, String) {
    let r = crow_runtime::layout::render(tag, attrs);
    let data: String = r.data.iter().map(|(k, v)| format!("{}=\"{}\"", k, esc(v))).collect::<Vec<_>>().join(" ");
    (r.tag, r.class, r.style, data)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A literal attribute value: `'m'` → `m`; anything else is dynamic and
/// unknown to a wireframe.
fn literal(v: &str) -> Option<String> {
    let t = v.trim();
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') { return Some(t[1..t.len() - 1].to_string()); }
    if t.chars().all(|c| c.is_ascii_digit() || c == '.') && !t.is_empty() { return Some(t.to_string()); }
    if t == "true" || t == "false" { return Some(t.to_string()); }
    None
}

/// The items of a literal list `['a', 'b']`, or placeholders for a name.
fn options_of(v: &str, n: usize) -> Vec<String> {
    let t = v.trim();
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        let items: Vec<String> = inner.split(',').filter_map(|i| literal(i.trim())).collect();
        if !items.is_empty() { return items; }
    }
    let name = t.rsplit('_').next().unwrap_or("option").trim_matches(|c: char| !c.is_alphanumeric());
    (1..=n).map(|i| format!("{} {}", name, i)).collect()
}

impl<'a> Wire<'a> {
    pub fn new(art: &'a Artifacts) -> Self {
        Wire { art, interp: Interpreter::new() }
    }

    /// The tree a sub-view shows under a parent's guard: among its paths
    /// consistent with the parent's guard, the one in its best state —
    /// data present, nothing failed — so a composed page draws the app as
    /// it looks when it works, not its error messages.
    fn view_node(&self, view: &str, parent_guard: &str) -> Option<WNode> {
        let paths = self.art.structure.wires.get(view)?;
        let consistent = |g: &str| g.split(" and ").all(|c| {
            let neg = if let Some(inner) = c.strip_prefix("(not ").and_then(|r| r.strip_suffix(')')) { inner.to_string() } else { format!("(not {})", c) };
            !parent_guard.split(" and ").any(|pc| pc == neg)
        });
        let score = |g: &str| -> i64 {
            let mut s = 0;
            for c in g.split(" and ") {
                let negated = c.starts_with("(not ");
                if c.contains(".ok") { s += if negated { -2 } else { 2 }; }
                if c.contains(" gt 0") || c.contains(" neq ''") { s += if negated { -1 } else { 1 }; }
                if c.contains(" eq 0") || c.contains(" eq ''") { s += if negated { 1 } else { -1 }; }
            }
            s
        };
        let mut best: Option<(i64, &WNode)> = None;
        for (g, node) in paths {
            if !g.is_empty() && !consistent(g) { continue; }
            let sc = score(g);
            if best.map_or(true, |(b, _)| sc > b) { best = Some((sc, node)); }
        }
        best.map(|(_, n)| n.clone()).or_else(|| paths.first().map(|(_, n)| n.clone()))
    }

    /// Emit HTML for a node, mirroring what the kernel renders, with the
    /// engine's path per element so boxes can be labelled.
    fn html(&self, node: &WNode, parent: &str, guard: &str, layers_open: bool, depth: usize, seen: &mut Vec<String>, out: &mut String, emitted: &mut Vec<Emitted>) {
        if depth > 24 { return; }
        match node.tag.as_str() {
            "@" => {
                if seen.contains(&node.view) { return; }
                if let Some(sub) = self.view_node(&node.view, guard) {
                    seen.push(node.view.clone());
                    self.html(&sub, parent, guard, layers_open, depth + 1, seen, out, emitted);
                    seen.pop();
                }
            }
            "list" => { for k in &node.kids { self.html(k, parent, guard, layers_open, depth + 1, seen, out, emitted); } }
            "for" => {
                for _ in 0..3 { for k in &node.kids { self.html(k, parent, guard, layers_open, depth + 1, seen, out, emitted); } }
            }
            "text" => {
                let path = format!("{}/span.", parent);
                out.push_str(&format!("<span>{}</span>", esc(&node.text)));
                emitted.push(Emitted { path, text: node.text.clone(), kind: "text" });
            }
            tag => {
                // literal attributes reach the kernel's renderer; the rest are the wireframe's own
                let own = ["binds", "options", "click", "input", "submit", "change", "seek", "action", "type", "lines", "min", "max", "step", "key", "hide", "delta", "mark", "actions"];
                let mut attrs: Vec<(String, String)> = Vec::new();
                for (k, v) in &node.attrs {
                    if own.contains(&k.as_str()) || k.starts_with("action ") { continue; }
                    if k == "open" { attrs.push((k.clone(), if layers_open { "true".into() } else { "false".into() })); continue; }
                    if let Some(l) = literal(v) { attrs.push((k.clone(), l)); }
                }
                if LAYERS.contains(&tag) && !layers_open { return; }
                let r = rendered(tag, &attrs);
                let (rtag, rclass, rstyle) = (r.0, r.1, r.2);
                let path = format!("{}/{}.{}", parent, rtag, rclass);
                let attr_of = |k: &str| node.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
                let open_tag = |cls: &str, style: &str, extra: &str| {
                    let mut s = format!("<{}", rtag);
                    if !cls.is_empty() { s.push_str(&format!(" class=\"{}\"", cls)); }
                    if !style.is_empty() { s.push_str(&format!(" style=\"{}\"", style)); }
                    if !extra.is_empty() { s.push(' '); s.push_str(extra); }
                    s.push('>');
                    s
                };
                match tag {
                    "button" => {
                        out.push_str(&open_tag(&rclass, &rstyle, ""));
                        out.push_str(&esc(&node.text));
                        out.push_str("</button>");
                        emitted.push(Emitted { path, text: node.text.clone(), kind: "button" });
                    }
                    "field" | "slider" => {
                        let kind = attr_of("type").and_then(|v| literal(&v)).unwrap_or_else(|| if tag == "slider" { "range".into() } else { "text".into() });
                        let fact = attr_of("binds").unwrap_or_default();
                        let value = format!("‹{}›", fact.rsplit('_').next().unwrap_or("value"));
                        out.push_str("<div class=\"crow-field\">");
                        let p_label = format!("{}/label.", format!("{}/div.crow-field", parent));
                        let p_input = format!("{}/input.", format!("{}/div.crow-field", parent));
                        emitted.push(Emitted { path: format!("{}/div.crow-field", parent), text: String::new(), kind: "container" });
                        out.push_str(&format!("<label>{}</label>", esc(&node.text)));
                        emitted.push(Emitted { path: p_label, text: node.text.clone(), kind: "text" });
                        out.push_str(&format!("<input type=\"{}\" value=\"{}\">", kind, esc(&value)));
                        emitted.push(Emitted { path: p_input, text: if kind == "range" { String::new() } else { value }, kind: "input" });
                        out.push_str("</div>");
                    }
                    "select" => {
                        let opts = attr_of("options").map(|v| options_of(&v, 3)).unwrap_or_else(|| vec!["option 1".into(), "option 2".into()]);
                        out.push_str("<select>");
                        for o in &opts { out.push_str(&format!("<option>{}</option>", esc(o))); }
                        out.push_str("</select>");
                        emitted.push(Emitted { path: format!("{}/select.", parent), text: opts.first().cloned().unwrap_or_default(), kind: "input" });
                    }
                    "radio" | "tabs" => {
                        let opts = attr_of("options").map(|v| options_of(&v, 3)).unwrap_or_else(|| vec!["one".into(), "two".into()]);
                        let cls = if tag == "tabs" { "crow-tabs" } else { "crow-radio" };
                        out.push_str(&format!("<div class=\"{}\">", cls));
                        let p_div = format!("{}/div.{}", parent, cls);
                        emitted.push(Emitted { path: p_div.clone(), text: String::new(), kind: "container" });
                        for o in &opts {
                            out.push_str(&format!("<label><input type=\"radio\">{}</label>", esc(o)));
                            emitted.push(Emitted { path: format!("{}/label.", p_div), text: o.clone(), kind: if tag == "tabs" { "tab" } else { "text" } });
                            emitted.push(Emitted { path: format!("{}/label./input.", p_div), text: String::new(), kind: "hidden" });
                        }
                        out.push_str("</div>");
                    }
                    "checkbox" | "toggle" => {
                        out.push_str(&format!("<label class=\"crow-check\"><input type=\"checkbox\">{}</label>", esc(&node.text)));
                        emitted.push(Emitted { path: format!("{}/label.crow-check", parent), text: node.text.clone(), kind: "check" });
                        emitted.push(Emitted { path: format!("{}/label.crow-check/input.", parent), text: String::new(), kind: "hidden" });
                    }
                    "table" => {
                        let mut cols: Vec<String> = Vec::new();
                        if let Some(k) = attr_of("key").and_then(|v| literal(&v)) { cols.push(k); }
                        if let Some(d) = attr_of("delta") { for c in options_of(&d, 0) { if !cols.contains(&c) { cols.push(c); } } }
                        if let Some(h) = attr_of("hide") { for c in options_of(&h, 0) { cols.retain(|x| x != &c); } }
                        while cols.len() < 4 { cols.push(format!("column {}", cols.len() + 1)); }
                        let has_actions = node.attrs.iter().any(|(k, _)| k.starts_with("action "));
                        out.push_str("<table>");
                        let p_table = format!("{}/table.", parent);
                        emitted.push(Emitted { path: p_table.clone(), text: cols.join(" · "), kind: "table" });
                        out.push_str("<thead><tr>");
                        for c in &cols { out.push_str(&format!("<th>{}</th>", esc(c))); }
                        if has_actions { out.push_str("<th></th>"); }
                        out.push_str("</tr></thead><tbody>");
                        for _ in 0..3 {
                            out.push_str("<tr>");
                            for _ in &cols { out.push_str("<td>···</td>"); }
                            if has_actions { out.push_str("<td class=\"actions\"><button>action</button></td>"); }
                            out.push_str("</tr>");
                        }
                        out.push_str("</tbody></table>");
                    }
                    "chart" => {
                        let h = attr_of("height").and_then(|v| literal(&v)).unwrap_or_else(|| "320".into());
                        out.push_str(&format!("<svg width=\"100%\" height=\"{}\"></svg>", h));
                        emitted.push(Emitted { path: format!("{}/svg.", parent), text: "chart".into(), kind: "chart" });
                    }
                    "img" => { out.push_str("<img>"); emitted.push(Emitted { path: format!("{}/img.", parent), text: "image".into(), kind: "chart" }); }
                    "icon" => { out.push_str("<svg width=\"20\" height=\"20\"></svg>"); emitted.push(Emitted { path: format!("{}/svg.", parent), text: String::new(), kind: "icon" }); }
                    "divider" => { out.push_str("<hr class=\"divider\">"); emitted.push(Emitted { path: format!("{}/hr.divider", parent), text: String::new(), kind: "line" }); }
                    "progress" => { out.push_str("<progress></progress>"); emitted.push(Emitted { path: format!("{}/progress.", parent), text: String::new(), kind: "input" }); }
                    _ if CONTAINERS.contains(&tag) => {
                        out.push_str(&open_tag(&rclass, &rstyle, &r.3));
                        emitted.push(Emitted { path: path.clone(), text: String::new(), kind: if LAYERS.contains(&tag) { "layer" } else { "container" } });
                        if !node.text.is_empty() {
                            out.push_str(&format!("<span>{}</span>", esc(&node.text)));
                            emitted.push(Emitted { path: format!("{}/span.", path), text: node.text.clone(), kind: "text" });
                        }
                        for k in &node.kids { self.html(k, &path, guard, layers_open, depth + 1, seen, out, emitted); }
                        out.push_str(&format!("</{}>", rtag));
                    }
                    _ => {
                        // text-bearing elements and anything the kernel knows
                        let kind = if tag.starts_with('h') && tag.len() == 2 { "heading" } else if TEXTY.contains(&tag) { "text" } else { "text" };
                        out.push_str(&open_tag(&rclass, &rstyle, &r.3));
                        emitted.push(Emitted { path: path.clone(), text: node.text.clone(), kind });
                        out.push_str(&esc(&node.text));
                        for k in &node.kids { self.html(k, &path, guard, layers_open, depth + 1, seen, out, emitted); }
                        out.push_str(&format!("</{}>", rtag));
                    }
                }
            }
        }
    }

    /// The wireframe of a tree at a viewport width.
    pub fn svg(&mut self, node: &WNode, guard: &str, width: f64, wide: bool, layers_open: bool) -> Result<String, String> {
        let mut html = String::new();
        let mut emitted = Vec::new();
        let mut seen = Vec::new();
        self.html(node, "/.", guard, layers_open, 0, &mut seen, &mut html, &mut emitted);
        let boxes = match self.interp.layout_html(&html, width, wide) {
            Ok(b) => b,
            Err(e) => {
                // an error inside the engine leaves the interpreter's scopes
                // unwound: the next drawing gets a fresh one
                self.interp = Interpreter::new();
                return Err(e);
            }
        };
        Ok(draw(&boxes, &emitted, width))
    }
}

/// Draw the boxes: containers as faint outlines, controls filled, text as
/// text. Labels are paired to boxes by the engine's path and reading order.
fn draw(boxes: &[LayoutBox], emitted: &[Emitted], width: f64) -> String {
    // pair: for each path, boxes in reading order ↔ elements in document order
    let mut by_path: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, b) in boxes.iter().enumerate() { by_path.entry(b.path.as_str()).or_default().push(i); }
    for list in by_path.values_mut() {
        list.sort_by(|a, b| {
            let (ba, bb) = (&boxes[*a], &boxes[*b]);
            (ba.y as i64, ba.x as i64).cmp(&(bb.y as i64, bb.x as i64))
        });
    }
    let mut label: BTreeMap<usize, (&str, &str)> = BTreeMap::new();
    let mut taken: BTreeMap<&str, usize> = BTreeMap::new();
    for e in emitted {
        let n = taken.entry(e.path.as_str()).or_insert(0);
        if let Some(list) = by_path.get(e.path.as_str()) {
            if let Some(&i) = list.get(*n) { label.insert(i, (e.text.as_str(), e.kind)); }
        }
        *n += 1;
    }
    let mut height: f64 = 0.0;
    for b in boxes { height = height.max(b.y + b.height); }
    let height = (height + 12.0).max(40.0);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"wireframe\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" style=\"max-width:100%;height:auto;background:#fff;border:1px solid #ddd;border-radius:4px\">",
        w = width as i64, h = height as i64
    );
    svg.push_str("<style>text{font:11px -apple-system,system-ui,sans-serif;fill:#222}.h text{font-weight:700}.m text{fill:#888}</style>");
    // containers first, so controls draw over them
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|a, b| (boxes[*b].width * boxes[*b].height).partial_cmp(&(boxes[*a].width * boxes[*a].height)).unwrap_or(std::cmp::Ordering::Equal));
    for i in order {
        let b = &boxes[i];
        let (text, kind) = label.get(&i).copied().unwrap_or(("", "container"));
        if kind == "hidden" { continue; }
        let (x, y, w, h) = (b.x, b.y, b.width.max(1.0), b.height.max(1.0));
        let (fill, stroke, dash) = match kind {
            "button" => ("#e3ecf9", "#5b7fb5", ""),
            "tab" => ("#eef2f7", "#8aa", ""),
            "input" => ("#fff", "#999", ""),
            "check" => ("#fff", "#aaa", ""),
            "table" => ("#fafafa", "#999", ""),
            "chart" => ("#f3f3f3", "#bbb", ""),
            "layer" => ("#fffbe6", "#d9b300", ""),
            "text" | "heading" => ("none", "none", ""),
            "icon" | "line" => ("none", "#bbb", ""),
            _ => ("none", "#cfcfcf", "4 3"),
        };
        if fill != "none" || stroke != "none" {
            svg.push_str(&format!("<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"3\" fill=\"{}\" stroke=\"{}\"{}/>", x, y, w, h, fill, stroke, if dash.is_empty() { String::new() } else { format!(" stroke-dasharray=\"{}\"", dash) }));
        }
        if kind == "chart" {
            svg.push_str(&format!("<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#ccc\"/>", x, y + h, x + w, y));
        }
        if kind == "table" {
            let rows = 4.0;
            let rh = h / rows;
            for r in 1..rows as i64 { let yy = y + rh * r as f64; svg.push_str(&format!("<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#ddd\"/>", x, yy, x + w, yy)); }
        }
        if !text.is_empty() && w > 12.0 {
            let max_chars = ((w - 4.0) / 5.6).max(1.0) as usize;
            let shown: String = if text.chars().count() > max_chars { format!("{}…", text.chars().take(max_chars.saturating_sub(1)).collect::<String>()) } else { text.to_string() };
            let cls = if kind == "heading" { " class=\"h\"" } else if text.starts_with('‹') { " class=\"m\"" } else { "" };
            // a table's columns sit in its header row; other text at its box's middle or top
            let ty = if kind == "table" { y + (h / 4.0 / 2.0 + 4.0).min(22.0) } else { y + (h / 2.0 + 4.0).min(14.0).max(10.0) };
            svg.push_str(&format!("<g{}><text x=\"{:.1}\" y=\"{:.1}\">{}</text></g>", cls, x + 4.0, ty, esc(&shown)));
        }
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree with a row of buttons lays out into boxes and draws as SVG
    /// with each button's label.
    #[test]
    fn a_tree_draws_as_a_wireframe_with_labels() {
        // the layout engine runs in the interpreter; the debug build needs a deeper stack
        std::thread::Builder::new().stack_size(256 << 20).spawn(a_tree_draws).unwrap().join().unwrap();
    }

    fn a_tree_draws() {
        let src = "use web, ui\nstatus = ''\nfn go[]\n    status = 'went'\nview v[]\n    return stack[row[button['Start', click = go], button['Stop', click = go], gap = 's'], p['`status`'], field['Name', status], gap = 'm']\npage['/', v]\n";
        let stmts = crate::build(src).unwrap();
        let lowered = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = crate::type_infer::TypeContext::new();
        ctx.infer_program(&lowered);
        let (art, _) = crate::baseline::derive_artifacts(&lowered, &ctx, &BTreeMap::new(), "main.crow");
        let (guard, node) = art.structure.wires["v"][0].clone();
        let mut w = Wire::new(&art);
        let svg = w.svg(&node, &guard, 600.0, false, true).expect("laid out");
        assert!(svg.contains(">Start<") && svg.contains(">Stop<"), "{}", svg);
        assert!(svg.contains("‹status›"), "dynamic text is a placeholder\n{}", svg);
        assert!(svg.contains(">Name<"), "a field shows its label\n{}", svg);
        assert!(svg.matches("<rect").count() >= 4, "{}", svg);
    }
}
