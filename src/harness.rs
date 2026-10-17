//! The harness's own pieces, shared by the walker and the tools: the
//! HTML it renders parsed back into nodes, selectors over them, the
//! escapes, the layout engine's source and the box it produces.

use crate::value::*;

/// (the one escape both modes write a page with: crow_web's)
pub(crate) fn html_escape_text(s: &str) -> String {
    crow_web::view::escape_text(s)
}

pub(crate) fn html_escape_attr(s: &str) -> String {
    crow_web::view::escape_attr(s)
}

/// A node of the harness's own rendered HTML (well-formed by construction).
pub(crate) struct HNode {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<HNode>,
    text: String, // for text nodes (tag is empty)
}

fn html_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// Parse the harness's rendered markup into a tree. Handles open/close tags,
/// self-closing tags (charts' SVG), quoted attributes, and text.
pub(crate) fn harness_parse_html(html: &str) -> HNode {
    let mut root = HNode { tag: String::new(), attrs: vec![], children: vec![], text: String::new() };
    let mut stack: Vec<HNode> = vec![];
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;
    let push_child = |stack: &mut Vec<HNode>, root: &mut HNode, node: HNode| {
        if let Some(parent) = stack.last_mut() { parent.children.push(node); } else { root.children.push(node); }
    };
    while i < chars.len() {
        if chars[i] == '<' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '>' { j += 1; }
            let inside: String = chars[i + 1..j.min(chars.len())].iter().collect();
            i = j + 1;
            let inside = inside.trim();
            if inside.starts_with('/') {
                let name = inside[1..].trim();
                // close the nearest open element with this name
                if let Some(pos) = stack.iter().rposition(|n| n.tag == name) {
                    while stack.len() > pos + 1 { let n = stack.pop().unwrap(); push_child(&mut stack, &mut root, n); }
                    let n = stack.pop().unwrap();
                    push_child(&mut stack, &mut root, n);
                }
                continue;
            }
            if inside.starts_with('!') { continue; }
            let self_closing = inside.ends_with('/');
            let inside = inside.trim_end_matches('/').trim();
            let mut parts = inside.splitn(2, char::is_whitespace);
            let tag = parts.next().unwrap_or("").to_string();
            let rest = parts.next().unwrap_or("").to_string();
            let mut attrs = vec![];
            let rc: Vec<char> = rest.chars().collect();
            let mut k = 0;
            while k < rc.len() {
                while k < rc.len() && rc[k].is_whitespace() { k += 1; }
                let ks = k;
                while k < rc.len() && rc[k] != '=' && !rc[k].is_whitespace() { k += 1; }
                let key: String = rc[ks..k].iter().collect();
                if key.is_empty() { break; }
                let mut val = String::new();
                if k < rc.len() && rc[k] == '=' {
                    k += 1;
                    if k < rc.len() && (rc[k] == '"' || rc[k] == '\'') {
                        let q = rc[k]; k += 1; let vs = k;
                        while k < rc.len() && rc[k] != q { k += 1; }
                        val = rc[vs..k.min(rc.len())].iter().collect(); k += 1;
                    } else {
                        let vs = k; while k < rc.len() && !rc[k].is_whitespace() { k += 1; }
                        val = rc[vs..k].iter().collect();
                    }
                }
                attrs.push((key, html_unescape(&val)));
            }
            let node = HNode { tag, attrs, children: vec![], text: String::new() };
            if self_closing { push_child(&mut stack, &mut root, node); } else { stack.push(node); }
        } else {
            let mut j = i;
            while j < chars.len() && chars[j] != '<' { j += 1; }
            let text: String = chars[i..j].iter().collect();
            i = j;
            if !text.trim().is_empty() || !text.is_empty() {
                push_child(&mut stack, &mut root, HNode { tag: String::new(), attrs: vec![], children: vec![], text: html_unescape(&text) });
            }
        }
    }
    while let Some(n) = stack.pop() { push_child(&mut stack, &mut root, n); }
    root
}

/// A parsed page written back out in one canonical form — attributes
/// sorted, text trimmed, the marks that only say when it was drawn
/// (`data-crow-stale`, `data-crow-busy`, `aria-busy`) left out — so two
/// renderers' pages compare by what they show and wire.
pub(crate) fn harness_canonical(n: &HNode) -> String {
    fn walk(n: &HNode, out: &mut String) {
        if n.tag.is_empty() {
            let t = n.text.split_whitespace().collect::<Vec<_>>().join(" ");
            out.push_str(&html_escape_text(&t));
            return;
        }
        let mut attrs: Vec<&(String, String)> = n.attrs.iter()
            .filter(|(k, _)| !matches!(k.as_str(), "data-crow-stale" | "data-crow-busy" | "aria-busy"))
            .collect();
        attrs.sort();
        out.push('<');
        out.push_str(&n.tag);
        for (k, v) in attrs {
            out.push_str(&format!(" {}=\"{}\"", k, html_escape_attr(v)));
        }
        out.push('>');
        for c in &n.children {
            walk(c, out);
        }
        out.push_str(&format!("</{}>", n.tag));
    }
    let mut out = String::new();
    if n.tag.is_empty() {
        for c in &n.children {
            walk(c, &mut out);
        }
    } else {
        walk(n, &mut out);
    }
    out
}

fn hnode_text(n: &HNode) -> String {
    if n.tag.is_empty() { return n.text.clone(); }
    n.children.iter().map(hnode_text).collect::<Vec<_>>().join("")
}

fn hnode_matches(n: &HNode, tag: &str, class: &str) -> bool {
    if n.tag.is_empty() { return false; }
    if !tag.is_empty() && n.tag != tag { return false; }
    if !class.is_empty() {
        let cls = n.attrs.iter().find(|(k, _)| k == "class").map(|(_, v)| v.clone()).unwrap_or_default();
        if !cls.split_whitespace().any(|c| c == class) { return false; }
    }
    true
}

/// Descendant selection: `steps[i]` must match an ancestor-or-self chain in
/// document order; the last step's matches are collected as records.
pub(crate) fn harness_select(n: &HNode, steps: &[(String, String)], depth: usize, out: &mut Vec<CrowValue>) {
    let normalized = harness_normalize_steps(steps);
    let steps = &normalized;
    for child in &n.children {
        if child.tag.is_empty() { continue; }
        let (tag, class) = &steps[depth];
        if hnode_matches(child, tag, class) {
            if depth + 1 == steps.len() {
                let mut m = std::collections::BTreeMap::new();
                // every attribute is a key (href, name, src, …); tag, text and
                // class are always present so records read uniformly
                for (k, v) in &child.attrs {
                    m.insert(k.clone(), CrowValue::ok(Value::Str(v.clone())));
                }
                let cls = child.attrs.iter().find(|(k, _)| k == "class").map(|(_, v)| v.clone()).unwrap_or_default();
                let text = hnode_text(child).split_whitespace().collect::<Vec<_>>().join(" ");
                m.insert("tag".to_string(), CrowValue::ok(Value::Str(child.tag.clone())));
                m.insert("text".to_string(), CrowValue::ok(Value::Str(text)));
                m.insert("class".to_string(), CrowValue::ok(Value::Str(cls)));
                out.push(CrowValue::ok(Value::map(m)));
            } else {
                harness_select(child, steps, depth + 1, out);
            }
        }
        // a later step may still start deeper down
        harness_select(child, steps, depth, out);
    }
}

/// The layout engine, written in Crow and shipped inside the compiler.
pub(crate) const LAYOUT_ENGINE_SRC: &str = include_str!("modules/layout.co");

/// One box the layout engine produced: where an element sits at a width.
/// The tag and class travel for callers that draw by element kind; the
/// wireframes pair boxes by path and read the geometry.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LayoutBox {
    pub tag: String,
    pub class: String,
    pub path: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub overflow: bool,
}


/// A rendered node as the engine's LNode struct value.
pub(crate) fn harness_to_lnode(n: &HNode) -> CrowValue {
    let mut attrs = std::collections::BTreeMap::new();
    let mut class = String::new();
    for (k, v) in &n.attrs {
        if k == "class" { class = v.clone(); }
        // the kernel table's data-* carry the responsive rules
        let key = k.trim_start_matches("data-").replace('-', "_");
        attrs.insert(key, CrowValue::ok(Value::Str(v.clone())));
    }
    // inline style declarations become attrs the engine understands
    if let Some((_, style)) = n.attrs.iter().find(|(k, _)| k == "style") {
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                let (k, v) = (k.trim(), v.trim());
                let mapped: Option<(&str, String)> = match k {
                    "gap" => Some(("gap", crow_space_name(v))),
                    "padding" => {
                        let parts: Vec<String> = v.split_whitespace().map(crow_space_name).collect();
                        Some(("pad", parts.join(" ")))
                    }
                    "margin" => Some(("margin", crow_space_name(v.split_whitespace().next().unwrap_or("0px")))),
                    "width" => Some(("width", v.to_string())),
                    "height" => Some(("height", v.to_string())),
                    "flex" => Some(("fill", "true".to_string())),
                    "flex-wrap" if v == "wrap" => Some(("wrap", "true".to_string())),
                    "font-size" => Some(("size", match v { "12px" => "xs", "14px" => "s", "16px" => "m", "20px" => "l", "24px" => "xl", "32px" => "xxl", _ => "m" }.to_string())),
                    "font-weight" => Some(("weight", match v { "700" => "bold", "500" => "medium", _ => "regular" }.to_string())),
                    "font-family" if v.contains("monospace") => Some(("font", "mono".to_string())),
                    "justify-content" => Some(("justify", match v { "space-between" => "between", "center" => "center", "flex-end" => "end", "space-around" => "around", _ => "start" }.to_string())),
                    "align-items" => Some(("align", match v { "flex-start" => "start", "center" => "center", "flex-end" => "end", "stretch" => "stretch", _ => "center" }.to_string())),
                    "grid-template-columns" => {
                        // repeat(N, minmax(0, 1fr)) → N
                        v.strip_prefix("repeat(").and_then(|r| r.split(',').next()).map(|n| ("columns", n.trim().to_string()))
                    }
                    _ => None,
                };
                if let Some((k, v)) = mapped { attrs.insert(k.to_string(), CrowValue::ok(Value::Str(v))); }
            }
        }
    }
    let kids: Vec<CrowValue> = n.children.iter().filter(|c| !c.tag.is_empty()).map(harness_to_lnode).collect();
    let text = hnode_text(n).split_whitespace().collect::<Vec<_>>().join(" ");
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("tag".to_string(), CrowValue::ok(Value::Str(n.tag.clone())));
    fields.insert("class".to_string(), CrowValue::ok(Value::Str(class)));
    fields.insert("text".to_string(), CrowValue::ok(Value::Str(text)));
    fields.insert("attrs".to_string(), CrowValue::ok(Value::map(attrs)));
    fields.insert("kids".to_string(), CrowValue::ok(Value::Array(std::sync::Arc::new(kids))));
    CrowValue::ok(Value::StructInstance(std::sync::Arc::new(StructInstance::new("LNode".to_string(), "LNode".to_string(), fields))))
}

fn crow_space_name(px: &str) -> String {
    match px.trim() { "4px" => "xs", "8px" => "s", "16px" => "m", "24px" => "l", "32px" => "xl", _ => "" }.to_string()
}

/// `path` is "/tag.class/tag.class/…" from the engine; a selector chain
/// matches when its steps appear in order as ancestors-or-self.
/// A selector step naming a layout container (`grid`, `row`, …) means its
/// class: containers render as divs carrying the primitive's name.
fn harness_normalize_steps(steps: &[(String, String)]) -> Vec<(String, String)> {
    steps.iter().map(|(t, c)| {
        if crow_runtime::layout::CONTAINERS.contains(&t.as_str()) && c.is_empty() { (String::new(), t.clone()) } else { (t.clone(), c.clone()) }
    }).collect()
}

pub(crate) fn harness_path_matches(path: &str, steps: &[(String, String)]) -> bool {
    let steps = &harness_normalize_steps(steps);
    let nodes: Vec<(&str, &str)> = path.split('/').filter(|s| !s.is_empty()).map(|seg| {
        match seg.find('.') { Some(i) => (&seg[..i], &seg[i + 1..]), None => (seg, "") }
    }).collect();
    if nodes.is_empty() || steps.is_empty() { return false; }
    // the last step must match the node itself
    let matches = |(tag, class): (&str, &str), (st, sc): &(String, String)| -> bool {
        (st.is_empty() || tag == st) && (sc.is_empty() || class.split_whitespace().any(|c| c == sc))
    };
    let last = nodes[nodes.len() - 1];
    if !matches(last, &steps[steps.len() - 1]) { return false; }
    let mut si = steps.len() - 1;
    let mut ni = nodes.len() - 1;
    while si > 0 {
        si -= 1;
        loop {
            if ni == 0 { return false; }
            ni -= 1;
            if matches(nodes[ni], &steps[si]) { break; }
        }
    }
    true
}
