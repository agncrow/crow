//! The layout vocabulary's serialization: the one table that turns a
//! primitive (`stack`, `row`, `grid`, `overlay`, `scroll`) and its layout
//! attributes into the CSS a browser lays out with. Both execution modes
//! render through this table, and the layout engine (written in Crow,
//! shipped inside the compiler) implements the same rules for tests and
//! the desktop shell — see docs/ui-engine-design.md.
//!
//! Everything here is deliberately boring CSS: block flow, flex without
//! intrinsic-size tricks, grid with fixed and fractional tracks, so that
//! every browser agrees with the engine.

/// Containers the vocabulary adds. They render as `<div class="stack">` etc.
/// `modal`, `drawer` and `toast` are layers: fixed-position containers shown
/// while `open = true`, laid out by the stylesheet.
pub const CONTAINERS: &[&str] = &["stack", "row", "grid", "overlay", "scroll", "modal", "drawer", "toast"];

/// Elements the vocabulary adds that render as a different HTML tag with a
/// class: (name, tag).
pub const ALIASED_ELEMENTS: &[(&str, &str)] = &[("divider", "hr"), ("dropdown", "details")];

/// Elements a browser builds with no children: their end tag, if written,
/// is ignored, so both renderers write none (crow_web's `CrowVNode::to_html`
/// and the interpreter's `render_element`, which had written `<img …></img>`).
pub const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param", "source", "track", "wbr"];

/// The HTML tag an element renders as.
pub fn html_tag(name: &str) -> &str {
    if CONTAINERS.contains(&name) { return "div"; }
    ALIASED_ELEMENTS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t).unwrap_or(name)
}

/// HTML's boolean attributes: present or absent, never "false". A view
/// writes them from state (`disabled = starting`) and the renderers keep
/// the browser's rule.
pub const BOOL_ATTRS: &[&str] = &["disabled", "hidden", "readonly", "required", "checked", "selected", "multiple", "autofocus"];

/// Attributes the vocabulary owns. Anything else on an element passes
/// through as an HTML attribute (class, href, name, …).
pub const LAYOUT_ATTRS: &[&str] = &[
    "gap", "pad", "margin", "align", "justify", "wrap", "columns", "columns_then",
    "width", "height", "min_width", "max_width", "aspect", "fill", "fit",
    "surface", "radius", "border", "shadow", "color", "background",
    "size", "weight", "lines", "anchor", "below", "becomes", "hidden_below",
    "tip", "open", "active", "side", "font", "shortcut",
];

/// A shortcut as the browser names it (`aria-keyshortcuts`): `'ctrl+s'` is
/// `Control+S`, `'escape'` is `Escape`, `'?'` is `?`; alternatives are
/// separated by spaces (`'ctrl+s meta+s'`). None when one is not a key: a
/// modifier other than ctrl, alt, shift and meta, a name no key has, or
/// shift with a character it changes (`'shift+/'` is written `'?'`).
pub fn shortcut_aria(v: &str) -> Option<String> {
    const NAMED: &[(&str, &str)] = &[
        ("enter", "Enter"), ("escape", "Escape"), ("tab", "Tab"), ("space", "Space"),
        ("backspace", "Backspace"), ("delete", "Delete"), ("up", "ArrowUp"), ("down", "ArrowDown"),
        ("left", "ArrowLeft"), ("right", "ArrowRight"), ("home", "Home"), ("end", "End"),
        ("pageup", "PageUp"), ("pagedown", "PageDown"),
    ];
    let mut alternatives = Vec::new();
    for alt in v.split_whitespace() {
        // `ctrl++` presses ctrl and `+`
        let (mods, key) = match alt.strip_suffix("++") {
            Some(rest) => (rest, "+"),
            None if alt == "+" => ("", "+"),
            None => alt.rsplit_once('+').unwrap_or(("", alt)),
        };
        let mut held = [false; 4];
        if !mods.is_empty() {
            for m in mods.split('+') {
                let i = ["ctrl", "alt", "shift", "meta"].iter().position(|x| x.eq_ignore_ascii_case(m))?;
                if held[i] {
                    return None;
                }
                held[i] = true;
            }
        }
        let lower = key.to_ascii_lowercase();
        let named = NAMED.iter().find(|(n, _)| *n == lower).map(|(_, a)| a.to_string());
        let function = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)).map(|n| format!("F{}", n));
        let mut chars = key.chars();
        let key_aria = match (named.or(function), chars.next(), chars.next()) {
            (Some(a), _, _) => a,
            (None, Some(c), None) if c.is_alphabetic() => c.to_uppercase().to_string(),
            // a character shift makes is written as that character
            (None, Some(c), None) if !c.is_whitespace() && !held[2] => c.to_string(),
            _ => return None,
        };
        let names = ["Control", "Alt", "Shift", "Meta"];
        let mut parts: Vec<String> = (0..4).filter(|i| held[*i]).map(|i| names[i].to_string()).collect();
        parts.push(key_aria);
        alternatives.push(parts.join("+"));
    }
    if alternatives.is_empty() {
        None
    } else {
        Some(alternatives.join(" "))
    }
}

/// The spacing scale, in px.
pub fn space_px(name: &str) -> Option<u32> {
    match name { "none" => Some(0), "xs" => Some(4), "s" => Some(8), "m" => Some(16), "l" => Some(24), "xl" => Some(32), _ => None }
}
/// The type scale: (font-size px, line-height px).
pub fn type_px(name: &str) -> Option<(u32, u32)> {
    match name { "xs" => Some((12, 18)), "s" => Some((14, 20)), "m" => Some((16, 24)), "l" => Some((20, 28)), "xl" => Some((24, 32)), "xxl" => Some((32, 40)), _ => None }
}
/// Named viewport widths: a rule "below = 'tablet'" applies under this many px.
pub fn width_px(name: &str) -> Option<u32> {
    match name { "phone" => Some(600), "tablet" => Some(1024), "desktop" => Some(1600), _ => None }
}
pub fn radius_px(name: &str) -> Option<&'static str> {
    match name { "none" => Some("0"), "s" => Some("2px"), "m" => Some("4px"), "l" => Some("8px"), "round" => Some("999px"), _ => None }
}
pub fn shadow_css(name: &str) -> Option<&'static str> {
    match name {
        "s" => Some("0 1px 2px rgba(0,0,0,.12)"),
        "m" => Some("0 2px 6px rgba(0,0,0,.14)"),
        "l" => Some("0 6px 18px rgba(0,0,0,.18)"),
        _ => None,
    }
}
/// A colour is only ever a token name.
pub fn color_var(name: &str) -> Option<String> {
    match name {
        "text" | "muted" | "accent" | "up" | "down" | "border" | "surface" | "bg" => Some(format!("var(--crow-{})", name)),
        _ => None,
    }
}

/// A size value: '240px', '50%', or 'fill'.
fn size_css(v: &str) -> Option<String> {
    if v == "fill" { return None; }
    if v.ends_with("px") || v.ends_with('%') { return Some(v.to_string()); }
    v.parse::<u32>().ok().map(|n| format!("{}px", n))
}

/// The rendered form of an element with layout attributes.
pub struct Rendered {
    /// HTML tag to emit (`div` for containers, the element's own tag otherwise)
    pub tag: String,
    /// class attribute value ("" if none)
    pub class: String,
    /// inline style ("" if none)
    pub style: String,
    /// data-* attributes carrying responsive rules (the stylesheet has the media queries)
    pub data: Vec<(String, String)>,
    /// attributes that are not layout — passed through
    pub passthrough: Vec<(String, String)>,
    /// attribute errors (unknown scale names etc.)
    pub errors: Vec<String>,
}

/// Turn a tag and its literal attributes into markup pieces.
pub fn render(tag: &str, attrs: &[(String, String)]) -> Rendered {
    let is_container = CONTAINERS.contains(&tag);
    let is_aliased = ALIASED_ELEMENTS.iter().any(|(n, _)| *n == tag);
    let mut css: Vec<String> = Vec::new();
    let mut classes: Vec<String> = Vec::new();
    let mut data = Vec::new();
    let mut passthrough = Vec::new();
    let mut errors = Vec::new();
    if is_container || is_aliased { classes.push(tag.to_string()); }
    // the layers say what they are: a dialog over the page, a side panel,
    // a notice a screen reader announces
    if !attrs.iter().any(|(k, _)| k == "role") {
        match tag {
            "modal" => passthrough.extend([("role".to_string(), "dialog".to_string()), ("aria-modal".to_string(), "true".to_string())]),
            "drawer" => passthrough.push(("role".to_string(), "dialog".to_string())),
            "toast" => passthrough.push(("role".to_string(), "status".to_string())),
            _ => {}
        }
    }
    match tag {
        "stack" => css.push("display:flex;flex-direction:column".into()),
        "row" => {
            css.push("display:flex;flex-direction:row;flex-wrap:nowrap".into());
            // the default cross-axis alignment, unless the view says otherwise
            if !attrs.iter().any(|(k, _)| k == "align") { css.push("align-items:center".into()); }
        }
        "grid" => css.push("display:grid".into()),
        "overlay" => css.push("position:relative".into()),
        "scroll" => css.push("overflow:auto".into()),
        _ => {}
    }
    let mut columns: Option<String> = None;
    // Deterministic output whichever renderer calls: the interpreter hands
    // attributes in map order, compiled views in source order — emit in
    // name order so the two write identical markup.
    let mut attrs: Vec<&(String, String)> = attrs.iter().collect();
    attrs.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in attrs {
        let k = k.as_str();
        let v = v.as_str();
        let bad = |what: &str, errors: &mut Vec<String>| errors.push(format!("{} = '{}' is not a {} (see the layout vocabulary in the spec)", k, v, what));
        match k {
            // (an empty class adds none: `class = ''` had written a class
            // attribute of nothing interpreted not at all, and `row`'s own
            // with a space after it)
            "class" => classes.extend(v.split_whitespace().map(str::to_string)),
            "gap" => match space_px(v) { Some(px) => css.push(format!("gap:{}px", px)), None => bad("spacing step (xs s m l xl)", &mut errors) },
            "pad" => {
                // 'm' or 'm l' (vertical horizontal)
                let parts: Vec<&str> = v.split_whitespace().collect();
                let pxs: Vec<Option<u32>> = parts.iter().map(|p| space_px(p)).collect();
                match pxs.as_slice() {
                    [Some(all)] => css.push(format!("padding:{}px", all)),
                    [Some(v), Some(h)] => css.push(format!("padding:{}px {}px", v, h)),
                    _ => bad("spacing step (xs s m l xl)", &mut errors),
                }
            }
            "margin" => match space_px(v) { Some(px) => css.push(format!("margin:{}px 0", px)), None => bad("spacing step (xs s m l xl)", &mut errors) },
            "align" => match v { "start" => css.push("align-items:flex-start".into()), "center" => css.push("align-items:center".into()), "end" => css.push("align-items:flex-end".into()), "stretch" => css.push("align-items:stretch".into()), _ => bad("alignment (start center end stretch)", &mut errors) },
            "justify" => match v { "start" => css.push("justify-content:flex-start".into()), "center" => css.push("justify-content:center".into()), "end" => css.push("justify-content:flex-end".into()), "between" => css.push("justify-content:space-between".into()), "around" => css.push("justify-content:space-around".into()), _ => bad("distribution (start center end between around)", &mut errors) },
            "wrap" => if v == "true" { css.push("flex-wrap:wrap".into()) },
            "columns" => {
                if let Ok(n) = v.parse::<u32>() { columns = Some(format!("repeat({}, minmax(0, 1fr))", n)); }
                else {
                    // '240px fill' — named tracks
                    let tracks: Vec<String> = v.split_whitespace().map(|t| if t == "fill" { "minmax(0, 1fr)".to_string() } else { size_css(t).unwrap_or_else(|| t.to_string()) }).collect();
                    columns = Some(tracks.join(" "));
                }
            }
            "columns_then" => data.push(("data-columns-then".to_string(), v.to_string())),
            "width" => if v == "fill" { css.push("flex:1 1 0;min-width:0".into()) } else { match size_css(v) { Some(s) => css.push(format!("width:{}", s)), None => bad("size ('240px', '50%', 'fill')", &mut errors) } },
            "fill" => if v == "true" { css.push("flex:1 1 0;min-width:0".into()) },
            "height" => match size_css(v) { Some(s) => css.push(format!("height:{}", s)), None => bad("size ('240px')", &mut errors) },
            "min_width" => match size_css(v) { Some(s) => css.push(format!("min-width:{}", s)), None => bad("size", &mut errors) },
            "max_width" => match size_css(v) { Some(s) => css.push(format!("max-width:{}", s)), None => bad("size", &mut errors) },
            "aspect" => css.push(format!("aspect-ratio:{}", v)),
            "fit" => match v { "cover" | "contain" => css.push(format!("object-fit:{};width:100%", v)), _ => bad("fit (cover contain)", &mut errors) },
            // a tooltip is the browser's own: the title attribute
            "tip" => passthrough.push(("title".to_string(), v.to_string())),
            // the keys that press the element, by the browser's names for
            // them, which the page's script reads too
            "shortcut" => match shortcut_aria(v) {
                Some(keys) => passthrough.push(("aria-keyshortcuts".to_string(), keys)),
                None => bad("shortcut ('ctrl+s', 'escape', '?', 'ctrl+s meta+s')", &mut errors),
            },
            // figures and code read in the monospace face with tabular digits
            "font" => match v { "mono" => css.push("font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-variant-numeric:tabular-nums;letter-spacing:-0.01em".into()), _ => bad("font (mono)", &mut errors) },
            // open: a real attribute on details (the browser needs it), a
            // data attribute on the layers (the stylesheet shows and hides)
            "open" => match v {
                "true" | "false" => {
                    if tag == "details" || tag == "dropdown" { if v == "true" { passthrough.push(("open".to_string(), String::new())); } }
                    else { data.push(("data-open".to_string(), v.to_string())); }
                }
                _ => bad("truth (true false)", &mut errors),
            },
            "active" => data.push(("data-active".to_string(), v.to_string())),
            "side" => match v { "left" | "right" => data.push(("data-side".to_string(), v.to_string())), _ => bad("side (left right)", &mut errors) },
            "surface" => match v { "card" => classes.push("card".into()), _ => bad("surface (card)", &mut errors) },
            "radius" => match radius_px(v) { Some(r) => css.push(format!("border-radius:{}", r)), None => bad("radius (none s m l round)", &mut errors) },
            "border" => if v == "true" { css.push("border:1px solid var(--crow-border)".into()) },
            "shadow" => match shadow_css(v) { Some(s) => css.push(format!("box-shadow:{}", s)), None => bad("shadow (s m l)", &mut errors) },
            "color" => match color_var(v) { Some(c) => css.push(format!("color:{}", c)), None => bad("colour token (text muted accent up down)", &mut errors) },
            "background" => match color_var(v) { Some(c) => css.push(format!("background:{}", c)), None => bad("colour token (surface bg accent up down)", &mut errors) },
            "size" => match type_px(v) { Some((fs, lh)) => css.push(format!("font-size:{}px;line-height:{}px", fs, lh)), None => bad("type step (xs s m l xl xxl)", &mut errors) },
            "weight" => match v { "regular" => css.push("font-weight:400".into()), "medium" => css.push("font-weight:500".into()), "bold" => css.push("font-weight:700".into()), _ => bad("weight (regular medium bold)", &mut errors) },
            "lines" => css.push(format!("display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:{};overflow:hidden", v)),
            "anchor" => data.push(("data-anchor".to_string(), v.to_string())),
            "below" => match width_px(v) { Some(_) => data.push(("data-below".to_string(), v.to_string())), None => bad("named width (phone tablet desktop)", &mut errors) },
            "becomes" => data.push(("data-becomes".to_string(), v.to_string())),
            "hidden_below" => match width_px(v) { Some(_) => data.push(("data-hidden-below".to_string(), v.to_string())), None => bad("named width (phone tablet desktop)", &mut errors) },
            _ if BOOL_ATTRS.contains(&k) => match v {
                "true" => passthrough.push((k.to_string(), String::new())),
                "false" => {}
                _ => passthrough.push((k.to_string(), v.to_string())),
            },
            _ => passthrough.push((k.to_string(), v.to_string())),
        }
    }
    if tag == "grid" {
        css.push(format!("grid-template-columns:{}", columns.clone().unwrap_or_else(|| "repeat(2, minmax(0, 1fr))".into())));
    }
    if tag == "overlay" {
        // children after the first are positioned by their anchor (stylesheet rules)
    }
    Rendered {
        tag: html_tag(tag).to_string(),
        class: classes.join(" "),
        style: css.join(";"),
        data,
        passthrough,
        errors,
    }
}

/// The responsive and overlay rules the data-* attributes rely on. Appended
/// to the default stylesheet. Named widths: phone < 600, tablet < 1024.
/// A frame as table rows — the markup inside `<table>` — for both renderers.
/// `click` names a handler each row sends with the `key` column's cell as its
/// argument (the first column when no key is given); `mark` names a column
/// whose truthy cells put `marked` on their row; `delta` names columns whose
/// cells are colored by sign (`up` / `down`). Numeric columns are typed:
/// their cells carry `num` and the stylesheet right-aligns them.
/// The rows of a frame. `click` sends the `key` column's cell of the
/// clicked row to a handler; `mark` tints rows whose cell is true; `delta`
/// colours cells by sign; `actions` adds a trailing cell of buttons, each
/// sending the row's key to its handler; `hide` keeps columns out of the
/// table while they stay in the frame (a key column, a helper). A numeric
/// key travels as a number, a text key as a string.
/// A click that asks first (`confirm = 'text'`): the element offers the
/// question, not the handler — `__ask` carries the handler, the arguments
/// its click froze and the text — and the runtime's dialog offers the
/// handler only while it is open, so the answer cannot be skipped. Both
/// modes wire an element through here.
pub fn confirm_wiring(events: Vec<(String, String)>, confirm: Option<&str>) -> Vec<(String, String)> {
    let Some(text) = confirm else { return events };
    let Some(handler) = events.iter().find(|(k, _)| k == "data-crow-click").map(|(_, v)| v.clone()) else { return events };
    let args = events.iter().find(|(k, _)| k == "data-crow-args").map(|(_, v)| v.clone());
    let mut out: Vec<(String, String)> = events.into_iter().filter(|(k, _)| k != "data-crow-click" && k != "data-crow-args").collect();
    out.push(("data-crow-click".to_string(), "__ask".to_string()));
    out.push(("data-crow-args".to_string(), ask_args(&handler, args.as_deref(), text)));
    out
}

/// Elements the browser presses itself: focusable, and pressed by Enter
/// (and Space).
const NATIVE_PRESS: &[&str] = &["a", "button", "input", "select", "textarea", "summary", "label", "option"];

/// What an element with a click needs to be pressed from the keyboard: a
/// tab stop, and the role of a button, for an element the browser does not
/// press itself (a table row keeps its role; the page's script presses a
/// focused one on Enter and Space). None where the program set either.
pub fn press_attrs(tag: &str, attrs: &[(String, String)]) -> Vec<(String, String)> {
    if NATIVE_PRESS.contains(&tag) || attrs.iter().any(|(k, _)| k == "role" || k == "tabindex") {
        return Vec::new();
    }
    let mut out = vec![("tabindex".to_string(), "0".to_string())];
    if tag != "tr" {
        out.push(("role".to_string(), "button".to_string()));
    }
    out
}

/// The question a confirmed click sends: the handler, its frozen arguments
/// (as the text its click would have sent) and what to ask.
pub fn ask_args(handler: &str, args: Option<&str>, text: &str) -> String {
    serde_json::json!([handler, args, text]).to_string()
}

/// `table[frame, page = n]`: the page size, which paged table of the render
/// this is, and the page its session is on (clamped to the frame's pages).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paging {
    pub size: usize,
    pub index: usize,
    pub at: usize,
}

/// A table the reader sorts (`sort = true`): which of the render's sorted
/// tables it is, and the column the session sorts it by, with whether it
/// is descending.
#[derive(Clone, Debug, Default)]
pub struct Sorting {
    pub index: usize,
    pub by: Option<(String, bool)>,
}

/// The most rows a table draws without `page` — as a chart has a point
/// budget, never a silent cut.
pub const TABLE_BUDGET: usize = 1000;

/// The rows of a frame a table shows, in the order it shows them: the
/// session's page of a paged table, its order if sorted (the one reading
/// of both, which the page's markup and a drawn table's data share). None
/// for more rows than a table shows unpaged (`TABLE_BUDGET`).
pub fn table_rows(f: &crate::frame::CrowFrame, page: Option<Paging>, sort: Option<&Sorting>) -> Option<Vec<usize>> {
    let total = f.nrows();
    if page.is_none() && total > TABLE_BUDGET {
        return None;
    }
    let (first, last) = page_bounds(total, page);
    Some(row_order(f, sort)[first..last].to_vec())
}

/// The first row a page shows and the one after its last.
fn page_bounds(total: usize, page: Option<Paging>) -> (usize, usize) {
    // the rows this page shows
    match page {
        Some(p) => {
            let pages = total.div_ceil(p.size.max(1)).max(1);
            let at = p.at.min(pages - 1);
            (at * p.size, ((at + 1) * p.size).min(total))
        }
        None => (0, total),
    }
}

/// The order a table's rows show in.
fn row_order(f: &crate::frame::CrowFrame, sort: Option<&Sorting>) -> Vec<usize> {
    use crate::frame::Column;
    let total = f.nrows();
    // the order the rows show in: the frame's, or the column the reader
    // sorted by (numbers as numbers, moments by when, text alphabetically)
    // — missing cells last either way, equal cells in the frame's order
    let mut order: Vec<usize> = (0..total).collect();
    let sorted_by = sort.and_then(|s| s.by.as_ref()).and_then(|(col, desc)| Some((f.columns.iter().position(|x| x == col)?, *desc)));
    if let Some((c, desc)) = sorted_by {
        order.sort_by(|&a, &b| {
            use std::cmp::Ordering;
            match (f.validity[c][a], f.validity[c][b]) {
                (false, false) => Ordering::Equal,
                (false, true) => Ordering::Greater,
                (true, false) => Ordering::Less,
                (true, true) => {
                    let o = match f.data[c].as_ref() {
                        Column::Int(v) => v[a].cmp(&v[b]),
                        Column::Float(v) => v[a].partial_cmp(&v[b]).unwrap_or(Ordering::Equal),
                        Column::Bool(v) => v[a].cmp(&v[b]),
                        // a moment by when it is, whatever its offset
                        Column::DateTime(ms, _) => ms[a].cmp(&ms[b]),
                        other => {
                            let (x, y) = (other.fmt_cell(a, true), other.fmt_cell(b, true));
                            x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(&y))
                        }
                    };
                    if desc { o.reverse() } else { o }
                }
            }
        });
    }
    order
}

/// One element of the markup the kernel builds (a table's parts, an icon):
/// its tag, its attributes in the order the markup writes them, each value
/// as the page reads it back (unescaped), and its children. The markup is
/// these parts written out (`write`); a renderer that holds nodes makes its
/// nodes from them, so the two never differ.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub tag: &'static str,
    pub attrs: Vec<(&'static str, String)>,
    pub kids: Vec<Kid>,
}

/// A part's child: another part, or text as it shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Kid {
    Part(Part),
    Text(String),
}

/// What the kernel's markup escapes, in text and in attribute values
/// alike: `&`, `<`, `>` and the quote.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Part {
    fn new(tag: &'static str) -> Self {
        Part { tag, attrs: Vec::new(), kids: Vec::new() }
    }

    fn attr(mut self, k: &'static str, v: impl Into<String>) -> Self {
        self.attrs.push((k, v.into()));
        self
    }

    fn kid(mut self, p: Part) -> Self {
        self.kids.push(Kid::Part(p));
        self
    }

    fn text(mut self, t: impl Into<String>) -> Self {
        self.kids.push(Kid::Text(t.into()));
        self
    }

    /// The part as markup: attributes in their order, every value and text
    /// escaped (`escape`), an end tag for each.
    pub fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.tag);
        for (k, v) in &self.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape(v));
            out.push('"');
        }
        out.push('>');
        for k in &self.kids {
            match k {
                Kid::Part(p) => p.write(out),
                Kid::Text(t) => out.push_str(&escape(t)),
            }
        }
        if !VOID.contains(&self.tag) {
            out.push_str("</");
            out.push_str(self.tag);
            out.push('>');
        }
    }
}

/// What a table was told of its frame's rows (`table[frame, click = f,
/// key = 'sym', …]`), as both renderers hand it to the kernel.
#[derive(Clone, Copy, Debug)]
pub struct TableOf<'a> {
    pub frame: &'a crate::frame::CrowFrame,
    pub click: Option<&'a str>,
    pub key: Option<&'a str>,
    pub mark: Option<&'a str>,
    pub delta: &'a [String],
    pub actions: &'a [(String, String)],
    pub hide: &'a [String],
    pub confirm: &'a [(String, String)],
    pub page: Option<Paging>,
    pub sort: Option<&'a Sorting>,
    /// `formats = map[pnl = 'money']`: the words a column's cells show in
    /// (`cell_formatted`); the values underneath sort, colour and align.
    pub formats: &'a [(String, String)],
}

/// The formats a table's cell can be shown in (`number:N` besides).
pub const CELL_FORMATS: &[&str] = &["money", "whole", "percent", "date", "time", "datetime"];

/// Whether a table knows the format: one of `CELL_FORMATS`, or `number:N`
/// with N from 0 to 12.
pub fn cell_format_known(format: &str) -> bool {
    CELL_FORMATS.contains(&format) || format.strip_prefix("number:").and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n <= 12)
}

/// A number with `places` decimals and its thousands grouped by commas
/// (`-1,234.50`), the minus kept.
fn grouped(x: f64, places: usize) -> String {
    // (half away from zero, as a ledger rounds: 1,234.5 is 1,235)
    let scale = 10f64.powi(places as i32);
    let text = format!("{:.*}", places, (x.abs() * scale).round() / scale);
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w.to_string(), Some(f.to_string())),
        None => (text.clone(), None),
    };
    let mut out = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(&f);
    }
    // (a value that rounds to zero shows no minus)
    let zero = text.chars().all(|c| c == '0' || c == '.');
    if x < 0.0 && !zero { format!("-{}", out) } else { out }
}

/// A cell as the format shows it. A missing cell, a number that is no
/// number (nan, inf) and a value the format cannot read show as they
/// would unformatted. A moment is read from a moment column (in its own
/// zone), an int or float as epoch milliseconds (UTC), or ISO text.
pub fn cell_formatted(col: &crate::frame::Column, r: usize, valid: bool, format: &str) -> String {
    use crate::frame::Column;
    let plain = || col.fmt_cell(r, valid);
    if !valid {
        return plain();
    }
    let number = match col {
        Column::Int(v) => Some(v[r] as f64),
        Column::Float(v) => Some(v[r]),
        Column::Str(v) => v[r].trim().parse::<f64>().ok(),
        _ => None,
    };
    let places = match format {
        "money" => Some(2),
        "whole" => Some(0),
        "percent" => Some(2),
        f => f.strip_prefix("number:").and_then(|n| n.parse::<usize>().ok()),
    };
    if let Some(places) = places {
        return match number {
            Some(x) if x.is_finite() => {
                let shown = if format == "percent" { format!("{:.2}", (x * 100.0).round() / 100.0) } else { grouped(x, places) };
                if format == "percent" { format!("{}%", shown) } else { shown }
            }
            _ => plain(),
        };
    }
    let moment = match col {
        Column::DateTime(ms, off) => Some((ms[r], off[r])),
        Column::Int(v) => Some((v[r], 0)),
        Column::Float(v) if v[r].is_finite() => Some((v[r] as i64, 0)),
        Column::Str(v) => crate::frame::parse_datetime(&v[r]),
        _ => None,
    };
    let Some((ms, off)) = moment else { return plain() };
    let iso = crate::frame::format_datetime(ms, off);
    let (day, rest) = iso.split_once('T').unwrap_or((iso.as_str(), "00:00:00"));
    let clock = rest.get(..8).unwrap_or("00:00:00");
    match format {
        "date" => day.to_string(),
        "time" => clock.to_string(),
        "datetime" => format!("{} {}", day, clock),
        _ => plain(),
    }
}

impl TableOf<'_> {
    /// The words a cell shows: in its column's format, else as it is.
    fn cell_text(&self, c: usize, r: usize) -> String {
        let f = self.frame;
        match self.formats.iter().find(|(col, _)| col == &f.columns[c]) {
            Some((_, format)) => cell_formatted(&f.data[c], r, f.validity[c][r], format),
            None => f.data[c].fmt_cell(r, f.validity[c][r]),
        }
    }

    fn numeric(&self, c: usize) -> bool {
        matches!(self.frame.data[c].as_ref(), crate::frame::Column::Int(_) | crate::frame::Column::Float(_))
    }

    fn key_index(&self) -> usize {
        self.key.and_then(|k| self.frame.columns.iter().position(|c| c == k)).unwrap_or(0)
    }

    /// A row's key cell as its click sends it back: a number as itself, a
    /// text in quotes, its own quotes marked.
    fn key_args(&self, r: usize) -> String {
        let k = self.key_index();
        let cell = self.frame.data[k].fmt_cell(r, self.frame.validity[k][r]);
        if self.numeric(k) && self.frame.validity[k][r] { format!("[{}]", cell.trim()) } else { format!("[\"{}\"]", cell.replace('"', "\\\"")) }
    }

    /// The same cell as JSON, as an action that asks first carries it.
    fn key_json(&self, r: usize) -> String {
        let k = self.key_index();
        let cell = self.frame.data[k].fmt_cell(r, self.frame.validity[k][r]);
        if self.numeric(k) && self.frame.validity[k][r] { format!("[{}]", cell.trim()) } else { serde_json::json!([cell]).to_string() }
    }

    fn shown_columns(&self) -> Vec<usize> {
        (0..self.frame.ncols()).filter(|i| !self.hide.iter().any(|h| h == &self.frame.columns[*i])).collect()
    }

    /// An action's click: its handler and arguments, or the question it
    /// asks first.
    fn action(&self, label: &str, h: &str, r: usize) -> (String, String) {
        match self.confirm.iter().find(|(l, _)| l == label) {
            Some((_, text)) => ("__ask".to_string(), ask_args(h, Some(&self.key_json(r)), text)),
            None => (h.to_string(), self.key_args(r)),
        }
    }

    /// What a drawn renderer sends from the table, as its parts carry it
    /// (one-view-design.md 5a; none where the table refuses its rows): for
    /// each row it shows, in the order it shows them, the row's click with
    /// its key cell when the table has one, then each action's (an action
    /// that asks first sends the question), with the text each of the
    /// frame's cells shows; each sorted header's `__sort` value; and the
    /// pager's two steps' `__page` values (none at an end).
    pub fn sends(&self) -> Option<TableSends> {
        let f = self.frame;
        let total = f.nrows();
        if self.page.is_none() && total > TABLE_BUDGET {
            return None;
        }
        let (first, last) = page_bounds(total, self.page);
        let order = row_order(f, self.sort);
        let mut rows = Vec::new();
        for &r in &order[first..last] {
            let mut sends = Vec::new();
            if f.ncols() > 0 {
                if let Some(h) = self.click {
                    sends.push((h.to_string(), self.key_args(r)));
                }
                for (label, h) in self.actions {
                    sends.push(self.action(label, h, r));
                }
            }
            let shown: Vec<String> = (0..f.ncols()).map(|c| self.cell_text(c, r)).collect();
            rows.push((sends, shown));
        }
        let sorts = match self.sort {
            Some(s) => self.shown_columns().into_iter().map(|i| (f.columns[i].clone(), sort_args(s.index, &f.columns[i]))).collect(),
            None => Vec::new(),
        };
        let steps = match self.page {
            Some(p) => self.steps(p, first, last).map(|n| n.map(|n| page_args(p.index, n))),
            None => [None, None],
        };
        Some(TableSends { rows, sorts, steps })
    }

    /// The pager's two steps: the page each goes to, none at an end.
    fn steps(&self, p: Paging, first: usize, last: usize) -> [Option<usize>; 2] {
        let at = first / p.size.max(1);
        [at.checked_sub(1), (last < self.frame.nrows()).then_some(at + 1)]
    }
}

/// What a drawn renderer sends from a table (`TableOf::sends`): each shown
/// row's sends (event, value) and its cells' texts, each sorted header's
/// column and value, the pager's previous and next values.
#[derive(Debug, Clone, PartialEq)]
pub struct TableSends {
    #[allow(clippy::type_complexity)]
    pub rows: Vec<(Vec<(String, String)>, Vec<String>)>,
    pub sorts: Vec<(String, String)>,
    pub steps: [Option<String>; 2],
}

/// A sorted header's click: which sorted table, and its column.
fn sort_args(index: usize, column: &str) -> String {
    format!("[{},{}]", index, serde_json::json!(column))
}

/// A pager's click: which paged table, and the page.
fn page_args(index: usize, at: usize) -> String {
    format!("[{},{}]", index, at)
}

/// A frame's rows as the parts inside a `<table>` (its head, its body and,
/// paged, its pager), for both renderers. `click` names a handler each row
/// sends with the `key` column's cell as its argument (the first column when
/// no key is given); `mark` names a column whose truthy cells put `marked`
/// on their row; `delta` names columns whose cells are coloured by sign
/// (`up` / `down`); `actions` adds a trailing cell of buttons, each sending
/// the row's key to its handler (asking first where `confirm` says); `hide`
/// keeps columns out of the table while they stay in the frame. Numeric
/// columns are typed: their cells carry `num` and the stylesheet
/// right-aligns them. A numeric key travels as a number, a text key as a
/// string.
pub fn frame_table_parts(t: &TableOf) -> Vec<Part> {
    use crate::frame::Column;
    let f = t.frame;
    let total = f.nrows();
    if t.page.is_none() && total > TABLE_BUDGET {
        let say = format!("table: {} rows — a table shows at most {} at a time: table[frame, page = 50]", total, TABLE_BUDGET);
        return vec![Part::new("tbody").kid(Part::new("tr").kid(Part::new("td").attr("class", "crow-refused").text(say)))];
    }
    let (first, last) = page_bounds(total, t.page);
    let is_delta: Vec<bool> = f.columns.iter().map(|c| t.delta.iter().any(|d| d == c)).collect();
    let shown = t.shown_columns();
    let order = row_order(f, t.sort);
    let mut head = Part::new("tr");
    for &i in &shown {
        let c = &f.columns[i];
        let mut th = Part::new("th");
        if t.numeric(i) {
            th = th.attr("class", "num");
        }
        th = match t.sort {
            // a header the reader presses to sort by its column
            Some(s) => {
                match &s.by {
                    Some((col, desc)) if col == c => th = th.attr("aria-sort", if *desc { "descending" } else { "ascending" }),
                    _ => {}
                }
                th.kid(Part::new("button").attr("data-crow-args", sort_args(s.index, c)).attr("data-crow-click", "__sort").text(c.as_str()))
            }
            None => th.text(c.as_str()),
        };
        head = head.kid(th);
    }
    if !t.actions.is_empty() {
        head = head.kid(Part::new("th"));
    }
    let mut body = Part::new("tbody");
    let mark_i = t.mark.and_then(|m| f.columns.iter().position(|c| c == m));
    for &r in &order[first..last] {
        let mut tr = Part::new("tr");
        if let Some(i) = mark_i
            && f.data[i].truthy(r)
            && f.validity[i][r]
        {
            tr = tr.attr("class", "marked");
        }
        if let Some(h) = t.click
            && f.ncols() > 0
        {
            tr = tr.attr("data-crow-click", h).attr("data-crow-args", t.key_args(r)).attr("tabindex", "0");
        }
        for &c in &shown {
            let text = t.cell_text(c, r);
            let mut classes: Vec<&str> = Vec::new();
            if t.numeric(c) {
                classes.push("num");
            }
            if is_delta[c] && f.validity[c][r] {
                let sign = match f.data[c].as_ref() {
                    Column::Int(v) => v[r].signum() as f64,
                    Column::Float(v) => if v[r].is_finite() { v[r].signum() } else { 0.0 },
                    _ => text.trim().parse::<f64>().map(|x| x.signum()).unwrap_or(0.0),
                };
                if sign > 0.0 {
                    classes.push("up");
                } else if sign < 0.0 {
                    classes.push("down");
                }
            }
            let mut td = Part::new("td");
            if !classes.is_empty() {
                td = td.attr("class", classes.join(" "));
            }
            tr = tr.kid(td.text(text));
        }
        if !t.actions.is_empty() && f.ncols() > 0 {
            let mut td = Part::new("td").attr("class", "actions");
            for (label, h) in t.actions {
                // an action that asks first offers the question
                let (handler, args) = t.action(label, h, r);
                td = td.kid(Part::new("button").attr("data-crow-click", handler).attr("data-crow-args", args).text(label.as_str()));
            }
            tr = tr.kid(td);
        }
        body = body.kid(tr);
    }
    let mut parts = vec![Part::new("thead").kid(head), body];
    // the pager: which rows, and the pages either side
    if let Some(p) = t.page {
        let span = shown.len() + usize::from(!t.actions.is_empty());
        let step = |label: &str, to: Option<usize>| match to {
            Some(n) => Part::new("button").attr("data-crow-args", page_args(p.index, n)).attr("data-crow-click", "__page").text(label),
            None => Part::new("button").attr("disabled", "").text(label),
        };
        let [before, after] = t.steps(p, first, last);
        let shown_rows = if total == 0 { "0 of 0".to_string() } else { format!("{}–{} of {}", first + 1, last, total) };
        let pager = Part::new("td").attr("class", "crow-pager").attr("colspan", span.max(1).to_string()).kid(step("Previous", before)).kid(Part::new("span").text(shown_rows)).kid(step("Next", after));
        parts.push(Part::new("tfoot").kid(Part::new("tr").kid(pager)));
    }
    parts
}

/// The clicks a table's rows offer, as the page sends them back (handler,
/// arguments): each row's click and actions, each sorted header, the
/// pager's steps — what its parts carry, without making them.
pub fn frame_table_clicks(t: &TableOf) -> Vec<(String, String)> {
    let f = t.frame;
    let total = f.nrows();
    let mut out = Vec::new();
    if t.page.is_none() && total > TABLE_BUDGET {
        return out;
    }
    if let Some(s) = t.sort {
        for &i in &t.shown_columns() {
            out.push(("__sort".to_string(), sort_args(s.index, &f.columns[i])));
        }
    }
    let (first, last) = page_bounds(total, t.page);
    if f.ncols() > 0 && (t.click.is_some() || !t.actions.is_empty()) {
        for &r in &row_order(f, t.sort)[first..last] {
            if let Some(h) = t.click {
                out.push((h.to_string(), t.key_args(r)));
            }
            for (label, h) in t.actions {
                out.push(t.action(label, h, r));
            }
        }
    }
    if let Some(p) = t.page {
        for n in t.steps(p, first, last).into_iter().flatten() {
            out.push(("__page".to_string(), page_args(p.index, n)));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn frame_table_html(f: &crate::frame::CrowFrame, click: Option<&str>, key: Option<&str>, mark: Option<&str>, delta: &[String], actions: &[(String, String)], hide: &[String], confirm: &[(String, String)], page: Option<Paging>, sort: Option<Sorting>) -> String {
    let t = TableOf { frame: f, click, key, mark, delta, actions, hide, confirm, page, sort: sort.as_ref(), formats: &[] };
    let mut out = String::new();
    for p in frame_table_parts(&t) {
        p.write(&mut out);
    }
    out
}

/// `nav[..., active = '/path']`: the link whose href is the active path
/// gets `aria-current="page"`. String form, for the renderer that builds
/// markup directly; the vnode renderer marks its tree the same way.
pub fn nav_active_html(inner: &str, active: &str) -> String {
    if active.is_empty() { return inner.to_string(); }
    let needle_dq = format!("href=\"{}\"", active);
    let mut out = String::with_capacity(inner.len() + 32);
    let mut rest = inner;
    while let Some(pos) = rest.find("<a ") {
        let (before, after) = rest.split_at(pos);
        out.push_str(before);
        let end = after.find('>').unwrap_or(after.len());
        let open_tag = &after[..end];
        if open_tag.contains(&needle_dq) && !open_tag.contains("aria-current") {
            out.push_str("<a aria-current=\"page\"");
            out.push_str(&open_tag[2..]);
        } else {
            out.push_str(open_tag);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// The icon set: a closed list of names, each a 24×24 stroke path. Both
/// renderers emit the same inline SVG, so an icon is theme-colored text.
pub const ICONS: &[(&str, &str)] = &[
    ("check", "M5 12l5 5L20 7"),
    ("x", "M6 6l12 12M18 6L6 18"),
    ("plus", "M12 5v14M5 12h14"),
    ("minus", "M5 12h14"),
    ("chevron_down", "M6 9l6 6 6-6"),
    ("chevron_up", "M6 15l6-6 6 6"),
    ("chevron_left", "M15 6l-6 6 6 6"),
    ("chevron_right", "M9 6l6 6-6 6"),
    ("arrow_up", "M12 19V5M5 12l7-7 7 7"),
    ("arrow_down", "M12 5v14M5 12l7 7 7-7"),
    ("arrow_left", "M19 12H5M12 5l-7 7 7 7"),
    ("arrow_right", "M5 12h14M12 5l7 7-7 7"),
    ("search", "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4"),
    ("menu", "M4 6h16M4 12h16M4 18h16"),
    ("more", "M4 12a1 1 0 1 0 2 0 1 1 0 1 0-2 0M11 12a1 1 0 1 0 2 0 1 1 0 1 0-2 0M18 12a1 1 0 1 0 2 0 1 1 0 1 0-2 0"),
    ("settings", "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"),
    ("user", "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0"),
    ("home", "M3 11l9-8 9 8M5 10v10h5v-6h4v6h5V10"),
    ("calendar", "M4 5h16v15H4zM4 10h16M8 3v4M16 3v4"),
    ("clock", "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2"),
    ("refresh", "M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5"),
    ("play", "M7 5v14l12-7z"),
    ("pause", "M7 5v14M17 5v14"),
    ("stop", "M6 6h12v12H6z"),
    ("upload", "M12 16V4M6 10l6-6 6 6M4 20h16"),
    ("download", "M12 4v12M6 10l6 6 6-6M4 20h16"),
    ("trash", "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6"),
    ("edit", "M4 20h4l11-11-4-4L4 16v4zM13 7l4 4"),
    ("copy", "M9 9h11v11H9zM5 15H4V4h11v1"),
    ("link", "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1.5 1.5M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1.5-1.5"),
    ("external", "M14 4h6v6M20 4l-9 9M19 14v6H4V5h6"),
    ("info", "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v5M12 8h.01"),
    ("warning", "M12 3l10 18H2zM12 10v4M12 17h.01"),
    ("error", "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM9 9l6 6M15 9l-6 6"),
    ("success", "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM8 12l3 3 5-6"),
    ("star", "M12 3l2.8 6 6.2.8-4.5 4.4 1.1 6.3L12 17.5 6.4 20.5l1.1-6.3L3 9.8 9.2 9z"),
    ("filter", "M3 5h18l-7 8v6l-4 2v-8z"),
    ("sort", "M7 4v16M7 20l-3-3M7 20l3-3M17 20V4M17 4l-3 3M17 4l3 3"),
    ("chart", "M4 20V10M10 20V4M16 20v-7M22 20H2"),
    ("table", "M3 5h18v14H3zM3 10h18M3 15h18M9 5v14M15 5v14"),
    ("bell", "M6 16V11a6 6 0 0 1 12 0v5l2 2H4zM10 21h4"),
    ("mail", "M3 6h18v12H3zM3 7l9 6 9-6"),
    ("lock", "M6 11h12v10H6zM8 11V7a4 4 0 0 1 8 0v4"),
    ("eye", "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z"),
    ("eye_off", "M3 3l18 18M10.5 5.2A10 10 0 0 1 22 12s-1.3 2.4-3.6 4.3M6.6 6.6C3.7 8.4 2 12 2 12s4 7 10 7c1.5 0 2.8-.3 4-.8M9.9 9.9a3 3 0 0 0 4.2 4.2"),
    ("dollar", "M12 2v20M17 6.5C16 5 14 4.5 12 4.5S7.5 5.5 7.5 8s2 3.5 4.5 3.5 4.5 1 4.5 3.5-2 3.5-4.5 3.5S7 17.5 6.5 16"),
    ("bookmark", "M6 3h12v18l-6-4-6 4z"),
    ("flag", "M5 21V4h12l-2 4 2 4H5"),
    ("tag", "M3 3h9l9 9-9 9-9-9zM8 8h.01"),
    ("circle", "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z"),
    ("dot", "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"),
    ("loader", "M12 2v4M12 18v4M4.9 4.9l2.8 2.8M16.3 16.3l2.8 2.8M2 12h4M18 12h4M4.9 19.1l2.8-2.8M16.3 7.7l2.8-2.8"),
];

/// Icon sizes follow the type scale: xs 12 · s 16 · m 20 · l 24 · xl 32.
pub fn icon_px(size: &str) -> Option<u32> {
    match size { "xs" => Some(12), "s" => Some(16), "m" => Some(20), "l" => Some(24), "xl" => Some(32), _ => None }
}

/// `icon[name, size = 'm', color = 'text']` as inline SVG, or the error
/// that names the set.
pub fn icon_html(name: &str, size: &str, color: &str) -> Result<String, String> {
    let mut out = String::new();
    icon_part(name, size, color)?.write(&mut out);
    Ok(out)
}

/// The icon as its part: an `svg` of one stroke path, coloured by a token
/// or by the text around it.
pub fn icon_part(name: &str, size: &str, color: &str) -> Result<Part, String> {
    let Some((_, d)) = ICONS.iter().find(|(n, _)| *n == name) else {
        let names: Vec<&str> = ICONS.iter().map(|(n, _)| *n).collect();
        return Err(format!("icon '{}' is not in the set: {}", name, names.join(" ")));
    };
    let px = icon_px(if size.is_empty() { "m" } else { size }).ok_or_else(|| format!("size = '{}' is not a type step (xs s m l xl)", size))?;
    let fill = if name == "dot" { "currentColor" } else { "none" };
    let mut svg = Part::new("svg")
        .attr("class", format!("icon icon-{}", name))
        .attr("width", px.to_string())
        .attr("height", px.to_string())
        .attr("viewBox", "0 0 24 24")
        .attr("fill", fill)
        .attr("stroke", "currentColor")
        .attr("stroke-width", "2")
        .attr("stroke-linecap", "round")
        .attr("stroke-linejoin", "round")
        .attr("aria-hidden", "true");
    if !color.is_empty() {
        let c = color_var(color).ok_or_else(|| format!("color = '{}' is not a colour token (text muted accent up down)", color))?;
        svg = svg.attr("style", format!("color:{}", c));
    }
    Ok(svg.kid(Part::new("path").attr("d", *d)))
}

/// The look an app asks for with `serve[..., theme, density, accent]`:
/// attributes for the `<html>` element and an inline accent override.
pub fn look_attrs(theme: &str, density: &str, accent: &str) -> Result<String, String> {
    let mut out = String::new();
    match theme {
        "" | "system" => {}
        "dark" | "light" => out.push_str(&format!(" data-theme=\"{}\"", theme)),
        _ => return Err(format!("theme = '{}' is not a theme (light dark system)", theme)),
    }
    match density {
        "" | "comfortable" => {}
        "compact" => out.push_str(" data-density=\"compact\""),
        _ => return Err(format!("density = '{}' is not a density (comfortable compact)", density)),
    }
    if !accent.is_empty() {
        let ok = accent.starts_with('#') && (accent.len() == 4 || accent.len() == 7) && accent[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok { return Err(format!("accent = '{}' is not a hex colour like '#c60' or '#cc6600'", accent)); }
        out.push_str(&format!(" style=\"--crow-accent:{}\"", accent));
    }
    Ok(out)
}

/// The kernel stylesheet: theme tokens, element defaults, and the eight
/// classes — every served page and every exported document starts here.
pub const DEFAULT_CSS: &str = r#":root{--crow-bg:#f6f7f9;--crow-surface:#ffffff;--crow-text:#1b222b;--crow-muted:#5d6773;--crow-border:#dce1e8;--crow-accent:#9a6519;--crow-up:#177a4e;--crow-down:#bd3a3a;--crow-series-2:#2f6fb5;--crow-series-3:#7c4dbd;--crow-series-4:#178a8a;--crow-series-5:#c23d7a;--crow-series-6:#d2701a}
@media (prefers-color-scheme:dark){:root{--crow-bg:#0f1317;--crow-surface:#171d24;--crow-text:#e6e9ec;--crow-muted:#8b95a1;--crow-border:#28313c;--crow-accent:#e8a940;--crow-up:#3dbd85;--crow-down:#e06060;--crow-series-2:#5b9cf0;--crow-series-3:#a884ee;--crow-series-4:#3cc0c0;--crow-series-5:#ee6fa8;--crow-series-6:#f09a4a}}
:root[data-theme="dark"]{--crow-bg:#0f1317;--crow-surface:#171d24;--crow-text:#e6e9ec;--crow-muted:#8b95a1;--crow-border:#28313c;--crow-accent:#e8a940;--crow-up:#3dbd85;--crow-down:#e06060;--crow-series-2:#5b9cf0;--crow-series-3:#a884ee;--crow-series-4:#3cc0c0;--crow-series-5:#ee6fa8;--crow-series-6:#f09a4a}
:root[data-theme="light"]{--crow-bg:#f6f7f9;--crow-surface:#ffffff;--crow-text:#1b222b;--crow-muted:#5d6773;--crow-border:#dce1e8;--crow-accent:#9a6519;--crow-up:#177a4e;--crow-down:#bd3a3a;--crow-series-2:#2f6fb5;--crow-series-3:#7c4dbd;--crow-series-4:#178a8a;--crow-series-5:#c23d7a;--crow-series-6:#d2701a}
:root{--crow-font-size:15px;--crow-line-height:1.55;--crow-page-max:860px;--crow-page-pad:2.2rem 1.25rem 4rem;--crow-heading-1:1.45rem;--crow-heading-2:0.8rem;--crow-heading-3:0.95rem;--crow-control-font:0.92rem;--crow-control-pad:0.42rem 0.95rem;--crow-field-pad:0.4rem 0.6rem;--crow-label-font:0.88rem;--crow-table-font:0.92rem;--crow-column-head:0.72rem;--crow-head-pad:0.45rem 0.9rem 0.45rem 0;--crow-cell-pad:0.5rem 0.9rem 0.5rem 0;--crow-card-label:0.68rem;--crow-action-font:0.8rem;--crow-action-pad:0.15rem 0.55rem;--crow-tab-pad:0.5rem 0.9rem;--crow-link-pad:0.35rem 0.6rem;--crow-target:auto}
*{box-sizing:border-box}
body{margin:0 auto;max-width:var(--crow-page-max);padding:var(--crow-page-pad);background:var(--crow-bg);color:var(--crow-text);font-family:system-ui,-apple-system,"Segoe UI",sans-serif;font-size:var(--crow-font-size);line-height:var(--crow-line-height)}
h1{font-size:var(--crow-heading-1);font-weight:650;letter-spacing:-0.015em;margin:0 0 0.25rem}
h2{font-size:var(--crow-heading-2);font-weight:600;text-transform:uppercase;letter-spacing:0.09em;color:var(--crow-muted);margin:2.2rem 0 0.7rem}
h3{font-size:var(--crow-heading-3);margin:1.2rem 0 0.4rem}
p{margin:0.4rem 0}
a{color:var(--crow-accent)}
button{font:inherit;font-size:var(--crow-control-font);color:var(--crow-text);background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:var(--crow-control-pad);min-height:var(--crow-target);min-width:var(--crow-target);cursor:pointer;transition:border-color 120ms ease,background 120ms ease}
button:hover{border-color:var(--crow-accent)}
button:disabled{opacity:.45;cursor:not-allowed}
button:disabled:hover{border-color:var(--crow-border)}
button:focus-visible,input:focus-visible,select:focus-visible,textarea:focus-visible{outline:2px solid var(--crow-accent);outline-offset:1px}
@media (prefers-reduced-motion:reduce){button{transition:none}}
label{color:var(--crow-muted);font-size:var(--crow-label-font);display:inline-flex;align-items:center;gap:0.5rem}
input,select,textarea{font:inherit;font-size:var(--crow-control-font);color:var(--crow-text);background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:var(--crow-field-pad);min-height:var(--crow-target)}
input{width:9.5rem}
pre,code{font-family:ui-monospace,"SF Mono",Menlo,monospace;font-size:0.9em}
table{border-collapse:collapse;width:100%;font-size:var(--crow-table-font)}
th{text-align:left;font-weight:600;font-size:var(--crow-column-head);text-transform:uppercase;letter-spacing:0.08em;color:var(--crow-muted);border-bottom:1px solid var(--crow-border);padding:var(--crow-head-pad)}
td{border-bottom:1px solid var(--crow-border);padding:var(--crow-cell-pad);font-variant-numeric:tabular-nums}
th button{font:inherit;text-transform:inherit;letter-spacing:inherit;color:inherit;background:none;border:0;border-radius:0;padding:0;cursor:pointer}
th[aria-sort="ascending"] button::after{content:" ▲"}
th[aria-sort="descending"] button::after{content:" ▼"}
td.crow-pager{border-bottom:0;text-align:right;color:var(--crow-muted)}
[data-crow-click][tabindex]:focus-visible{outline:2px solid var(--crow-accent);outline-offset:-2px}
@media (prefers-reduced-motion:reduce){.drawer{transition:none}}
td.crow-pager span{margin:0 0.75rem}
td.crow-refused{color:var(--crow-down)}
.controls{display:flex;flex-wrap:wrap;gap:0.5rem;align-items:center;margin:0.9rem 0}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:0.6rem;margin:1.1rem 0}
.card{background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:0.75rem 0.9rem}
.card h3{margin:0 0 0.3rem;font-size:var(--crow-card-label);font-weight:600;text-transform:uppercase;letter-spacing:0.09em;color:var(--crow-muted)}
.card p{margin:0}
header{display:flex;align-items:center;padding:0.5rem 0.9rem;background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px}
td.actions{white-space:nowrap}td.actions button{padding:var(--crow-action-pad);font-size:var(--crow-action-font);margin-right:0.35rem}
button.primary{background:var(--crow-accent);border-color:var(--crow-accent);color:var(--crow-bg);font-weight:600}
button.primary:hover{filter:brightness(1.08)}
.up{color:var(--crow-up)}
.down{color:var(--crow-down)}
.stale{color:var(--crow-muted)}
tr[data-crow-click]{cursor:pointer}
tr[data-crow-click]:hover td{background:color-mix(in srgb,var(--crow-accent) 10%,transparent)}
tr.marked td{background:color-mix(in srgb,var(--crow-up) 14%,transparent)}
.crow-slider input{width:16rem;padding:0;border:0;background:transparent}
.crow-field{display:inline-flex;align-items:center;gap:0.4rem}
.crow-field:has(textarea){align-items:flex-start}
hr.divider{border:0;border-top:1px solid var(--crow-border);margin:0;width:100%;height:0}
.row > hr.divider{width:0;height:auto;align-self:stretch;border-top:0;border-left:1px solid var(--crow-border);min-height:1.5em}
details{border:1px solid var(--crow-border);border-radius:4px;background:var(--crow-surface);padding:0.5rem 0.75rem}
summary{min-height:var(--crow-target);cursor:pointer;font-weight:600;list-style:none;display:flex;align-items:center;gap:0.5rem}
summary::-webkit-details-marker{display:none}
summary::before{content:'';width:0.4em;height:0.4em;border-right:2px solid var(--crow-muted);border-bottom:2px solid var(--crow-muted);transform:rotate(-45deg);transition:transform 120ms ease;flex:none}
details[open] > summary::before{transform:rotate(45deg)}
details > :not(summary){margin-top:0.5rem}
.dropdown{position:relative;display:inline-block;border:0;padding:0;background:transparent}
.dropdown > summary{border:1px solid var(--crow-border);border-radius:4px;padding:var(--crow-control-pad);background:var(--crow-surface);font-weight:400;font-size:var(--crow-control-font)}
.dropdown > summary::before{order:2;margin-left:0.25rem}
.dropdown > :not(summary){position:absolute;top:calc(100% + 4px);left:0;z-index:20;min-width:12rem;margin:0;padding:0.35rem;background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;box-shadow:0 6px 18px rgba(0,0,0,.18)}
.dropdown[data-side="right"] > :not(summary){left:auto;right:0}
.dropdown > :not(summary) > button,.dropdown > :not(summary) > a{display:block;width:100%;text-align:left;border:0;background:transparent;border-radius:3px;padding:0.4rem 0.6rem;min-height:var(--crow-target);color:var(--crow-text);text-decoration:none}
.dropdown > :not(summary) > button:hover,.dropdown > :not(summary) > a:hover{background:color-mix(in srgb,var(--crow-accent) 12%,transparent)}
.modal{display:none;position:fixed;inset:0;z-index:100;background:rgba(0,0,0,.45);align-items:center;justify-content:center;padding:1rem}
.modal[data-open="true"]{display:flex}
.modal > *{background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:6px;padding:1.25rem;max-width:min(560px,100%);max-height:90vh;overflow:auto;box-shadow:0 6px 18px rgba(0,0,0,.25)}
.drawer{position:fixed;top:0;bottom:0;right:0;z-index:100;width:min(420px,92vw);background:var(--crow-surface);border-left:1px solid var(--crow-border);box-shadow:-6px 0 18px rgba(0,0,0,.18);padding:1.25rem;overflow:auto;display:flex;flex-direction:column;gap:0.75rem;transform:translateX(105%);visibility:hidden;transition:transform 160ms ease,visibility 160ms}
.drawer[data-open="true"]{transform:none;visibility:visible}
.drawer[data-side="left"]{left:0;right:auto;border-left:0;border-right:1px solid var(--crow-border);box-shadow:6px 0 18px rgba(0,0,0,.18);transform:translateX(-105%)}
.drawer[data-side="left"][data-open="true"]{transform:none}
.toast{display:none;position:fixed;right:24px;bottom:24px;z-index:110;max-width:min(420px,92vw);background:var(--crow-surface);color:var(--crow-text);border:1px solid var(--crow-border);border-left:4px solid var(--crow-accent);border-radius:4px;padding:0.7rem 1rem;box-shadow:0 6px 18px rgba(0,0,0,.18)}
.toast[style*="color:"]{border-left-color:currentColor}
.toast[data-open="true"]{display:block;animation:crow-toast 4s ease forwards}
@keyframes crow-toast{0%{opacity:0;transform:translateY(8px)}8%{opacity:1;transform:none}85%{opacity:1}100%{opacity:0;visibility:hidden}}
.crow-tip{position:fixed;z-index:120;max-width:min(320px,90vw);background:var(--crow-text);color:var(--crow-bg);border-radius:4px;padding:0.35rem 0.6rem;font-size:0.85rem;line-height:1.3;pointer-events:none;box-shadow:0 4px 12px rgba(0,0,0,.2)}
.crow-check{display:inline-flex;align-items:center;gap:0.5rem;color:var(--crow-text);font-size:var(--crow-control-font);min-height:var(--crow-target);cursor:pointer}
.crow-check input{width:auto;margin:0;accent-color:var(--crow-accent)}
.crow-toggle input{appearance:none;-webkit-appearance:none;width:2.2rem;height:1.25rem;border-radius:999px;background:var(--crow-border);position:relative;padding:0;border:0;transition:background 120ms ease;cursor:pointer}
.crow-toggle input::after{content:'';position:absolute;top:2px;left:2px;width:calc(1.25rem - 4px);height:calc(1.25rem - 4px);border-radius:50%;background:var(--crow-surface);box-shadow:0 1px 2px rgba(0,0,0,.3);transition:transform 120ms ease}
.crow-toggle input:checked{background:var(--crow-accent)}
.crow-toggle input:checked::after{transform:translateX(0.95rem)}
.crow-radio{display:inline-flex;gap:1rem;flex-wrap:wrap;align-items:center}
.crow-radio label{color:var(--crow-text);min-height:var(--crow-target);cursor:pointer}
.crow-radio input{width:auto;margin:0;accent-color:var(--crow-accent)}
.crow-tabs{display:flex;gap:0;border-bottom:1px solid var(--crow-border);align-items:stretch}
.crow-tabs label{cursor:pointer;padding:var(--crow-tab-pad);min-height:var(--crow-target);margin-bottom:-1px;border-bottom:2px solid transparent;color:var(--crow-muted);font-size:var(--crow-control-font);font-weight:500;white-space:nowrap;position:relative}
.crow-tabs label[aria-selected="true"]{color:var(--crow-text);border-bottom-color:var(--crow-accent)}
.crow-tabs input{position:absolute;opacity:0;width:0;height:0;margin:0;padding:0;border:0}
progress.crow-progress{display:block;width:100%;height:0.5rem;appearance:none;-webkit-appearance:none;border:0;border-radius:999px;overflow:hidden;background:var(--crow-border);accent-color:var(--crow-accent)}
progress.crow-progress::-webkit-progress-bar{background:var(--crow-border)}
progress.crow-progress::-webkit-progress-value{background:var(--crow-accent)}
progress.crow-progress::-moz-progress-bar{background:var(--crow-accent)}
svg.icon{display:inline-block;vertical-align:-0.2em;flex:none}
.icon-loader{animation:crow-spin 1s linear infinite}
@keyframes crow-spin{to{transform:rotate(360deg)}}
nav:not(.doc-nav):not(.doc-pager){display:flex;gap:0.25rem;align-items:center;flex-wrap:wrap}
nav:not(.doc-nav):not(.doc-pager) a{color:var(--crow-muted);text-decoration:none;padding:var(--crow-link-pad);min-height:var(--crow-target);border-radius:4px}
nav:not(.doc-nav):not(.doc-pager) a:hover{color:var(--crow-text)}
nav:not(.doc-nav):not(.doc-pager) a[aria-current="page"]{color:var(--crow-text);background:color-mix(in srgb,var(--crow-accent) 14%,transparent)}
td.num,th.num{text-align:right}
td.up{color:var(--crow-up)}
td.down{color:var(--crow-down)}
img[style*="object-fit"]{display:block}
:root[data-wide="true"] body{max-width:none;padding:1.25rem 1.25rem 3rem}
:root[data-density="compact"] body{font-size:13.5px;line-height:1.45;padding-top:1.25rem}
[data-density="compact"] h1{font-size:1.25rem}
[data-density="compact"] h2{margin:1.4rem 0 0.5rem}
[data-density="compact"] button{padding:0.3rem 0.7rem;font-size:0.88rem}
[data-density="compact"] input,[data-density="compact"] select,[data-density="compact"] textarea{padding:0.28rem 0.5rem;font-size:0.88rem}
[data-density="compact"] th{padding:0.3rem 0.7rem 0.3rem 0}
[data-density="compact"] td{padding:0.32rem 0.7rem 0.32rem 0}
[data-density="compact"] .card{padding:0.55rem 0.7rem}
[data-density="compact"] 
[data-density="compact"] .crow-tabs label{padding:0.4rem 0.7rem}"#;

// Responsive rules override the element's own inline rule (the base
// display/direction the kernel emits), so they carry !important.
pub const RESPONSIVE_CSS: &str = "\
.overlay > :not(:first-child){position:absolute}\
.overlay > [data-anchor=\"center\"]{left:50%;top:50%;transform:translate(-50%,-50%)}\
.overlay > [data-anchor=\"top_right\"]{right:0;top:0}\
.overlay > [data-anchor=\"top_left\"]{left:0;top:0}\
.overlay > [data-anchor=\"bottom_right\"]{right:0;bottom:0}\
.overlay > [data-anchor=\"bottom_left\"]{left:0;bottom:0}\
@media (max-width:1023px){\
[data-below=\"tablet\"][data-becomes=\"stack\"]{flex-direction:column!important;align-items:stretch!important}\
[data-below=\"tablet\"][data-columns-then=\"1\"]{grid-template-columns:repeat(1,minmax(0,1fr))!important}\
[data-below=\"tablet\"][data-columns-then=\"2\"]{grid-template-columns:repeat(2,minmax(0,1fr))!important}\
[data-hidden-below=\"tablet\"]{display:none!important}}\
@media (max-width:599px){\
[data-below=\"phone\"][data-becomes=\"stack\"]{flex-direction:column!important;align-items:stretch!important}\
[data-below=\"phone\"][data-columns-then=\"1\"]{grid-template-columns:repeat(1,minmax(0,1fr))!important}\
[data-below=\"phone\"][data-columns-then=\"2\"]{grid-template-columns:repeat(2,minmax(0,1fr))!important}\
[data-hidden-below=\"phone\"]{display:none!important}}";

/// Where the mobile theme applies: a phone, held either way up — a coarse
/// pointer with a side under the phone width. A narrow window on a desktop
/// keeps the desktop look (the layout engine and its conformance fixture
/// at 390px model that look); `<html data-surface="mobile">` asks for the
/// mobile theme anywhere, and `data-surface="desktop"` keeps a phone on
/// the desktop's.
pub const MOBILE_WHEN: &str = "(pointer:coarse) and (max-width:599px),(pointer:coarse) and (max-height:599px)";

/// The mobile theme (one-view-design.md, 3.3): the kernel's size tokens
/// given a phone's values, then the few structural rules a phone needs —
/// rows that wrap, grids that collapse, wide things that scroll in their
/// own box, the layers and the runtime's own boxes inside the safe area.
/// One rule a line; `mobile_css()` scopes each to `MOBILE_WHEN` and to
/// `data-surface="mobile"` with no specificity added, so an app's own
/// stylesheet, which comes after, still wins where it speaks. (A rule
/// about the root element starts with `:root` or `html`.)
///
/// The tokens (desktop's value in the kernel stylesheet -> the phone's):
/// --crow-font-size 15px -> 16px (body copy; 16px also keeps iOS from
/// zooming into a field), --crow-line-height 1.55 -> 1.5, --crow-page-max
/// 860px -> none, --crow-page-pad -> 12px 16px 32px beside the safe area,
/// --crow-heading-1/-2/-3 1.45rem/0.8rem/0.95rem -> 1.75rem/13px/1.125rem,
/// --crow-control-font 0.92rem -> 16px (buttons, fields, selects, tabs,
/// checks, a dropdown's summary), --crow-control-pad, --crow-field-pad,
/// --crow-label-font 0.88rem -> 16px, --crow-table-font 0.92rem -> 16px,
/// --crow-column-head 0.72rem -> 12.5px (a caption), --crow-head-pad,
/// --crow-cell-pad (a row 44px tall), --crow-card-label 0.68rem -> 12.5px
/// (a caption), --crow-action-font 0.8rem -> 16px, --crow-action-pad,
/// --crow-tab-pad, --crow-link-pad, --crow-target auto -> 44px (the least
/// height, and a button's least width, of every control), and the
/// phone's own: --crow-grid-min 280px (a grid's least column), the type
/// steps --crow-type-xs/-s/-l/-xl 13/16/22/26px for `size =` 12/14/20/24,
/// and --crow-surface-name `mobile` (what a check reads to know the theme
/// is on).
pub const MOBILE_RULES: &str = r#":root{--crow-surface-name:mobile;--crow-font-size:16px;--crow-line-height:1.5;--crow-page-max:none;--crow-page-pad:max(12px,env(safe-area-inset-top)) max(16px,env(safe-area-inset-right)) calc(32px + env(safe-area-inset-bottom)) max(16px,env(safe-area-inset-left));--crow-heading-1:1.75rem;--crow-heading-2:13px;--crow-heading-3:1.125rem;--crow-control-font:16px;--crow-control-pad:0.55rem 1rem;--crow-field-pad:0.55rem 0.75rem;--crow-label-font:16px;--crow-table-font:16px;--crow-column-head:12.5px;--crow-head-pad:10px 16px 10px 0;--crow-cell-pad:10px 16px 10px 0;--crow-card-label:12.5px;--crow-action-font:16px;--crow-action-pad:0.5rem 0.75rem;--crow-tab-pad:0.5rem 1rem;--crow-link-pad:0.5rem 0.75rem;--crow-target:44px;--crow-grid-min:280px;--crow-type-xs:13px;--crow-type-s:16px;--crow-type-l:22px;--crow-type-xl:26px}
body{-webkit-text-size-adjust:100%;text-size-adjust:100%;overflow-wrap:break-word}
:root[data-wide="true"] body,:root[data-density="compact"] body{max-width:none;padding:var(--crow-page-pad);font-size:var(--crow-font-size);line-height:var(--crow-line-height)}
:root[data-density="compact"] h1{font-size:var(--crow-heading-1)}
:root[data-density="compact"] button{padding:var(--crow-control-pad);font-size:var(--crow-control-font)}
:root[data-density="compact"] input,:root[data-density="compact"] select,:root[data-density="compact"] textarea{padding:var(--crow-field-pad);font-size:var(--crow-control-font)}
:root[data-density="compact"] th{padding:var(--crow-head-pad)}
:root[data-density="compact"] td{padding:var(--crow-cell-pad)}
[style*="font-size:12px;line-height:18px"]{font-size:var(--crow-type-xs)!important}
[style*="font-size:14px;line-height:20px"]{font-size:var(--crow-type-s)!important;line-height:24px!important}
[style*="font-size:20px;line-height:28px"]{font-size:var(--crow-type-l)!important}
[style*="font-size:24px;line-height:32px"]{font-size:var(--crow-type-xl)!important}
.row{flex-wrap:wrap!important}
header,.crow-field{flex-wrap:wrap}
#crow-root > *,.stack > *,.row > *,.grid > *,.card > *,form > *,.crow-field > *{max-width:100%}
.grid{grid-template-columns:repeat(auto-fill,minmax(min(100%,var(--crow-grid-min)),1fr))!important}
table{display:block;overflow-x:auto;max-width:100%}
th:first-child,td:first-child{position:sticky;left:0;z-index:1;background:var(--crow-bg)}
.card th:first-child,.card td:first-child,.modal th:first-child,.modal td:first-child,.drawer th:first-child,.drawer td:first-child{background:var(--crow-surface)}
td.crow-pager{text-align:left}
pre{overflow-x:auto;max-width:100%}
img,video,canvas,iframe,svg:not(.icon){max-width:100%}
img{height:auto}
input,select,textarea{max-width:100%}
.crow-tabs{overflow-x:auto;scrollbar-width:none;max-width:100%}
nav:not(.doc-nav):not(.doc-pager) a{display:inline-flex;align-items:center}
.dropdown > :not(summary) > a{display:flex;align-items:center}
:where(.stack,.row) > a[href]{display:flex;align-items:center;gap:0.4em;min-height:var(--crow-target)}
select{min-width:var(--crow-target)}
[data-crow-click]:not(tr):not(td){min-height:var(--crow-target)}
.crow-check:not(.crow-toggle) input,.crow-radio input{width:20px;height:20px}
.dropdown > :not(summary){max-width:calc(100vw - 32px)}
.modal{padding:max(12px,env(safe-area-inset-top)) max(12px,env(safe-area-inset-right)) max(12px,env(safe-area-inset-bottom)) max(12px,env(safe-area-inset-left))}
.modal > *{max-width:100%;max-height:100%}
.drawer{max-width:100vw;padding-top:calc(1.25rem + env(safe-area-inset-top));padding-bottom:calc(1.25rem + env(safe-area-inset-bottom))}
.toast,.crow-notice{left:max(12px,env(safe-area-inset-left));right:max(12px,env(safe-area-inset-right));bottom:calc(12px + env(safe-area-inset-bottom));max-width:none}
.crow-notice{font-size:var(--crow-font-size)}
.crow-confirm{padding:max(12px,env(safe-area-inset-top)) max(12px,env(safe-area-inset-right)) max(12px,env(safe-area-inset-bottom)) max(12px,env(safe-area-inset-left))}
.crow-confirm-box{max-width:100%;font-size:var(--crow-font-size)}
.crow-confirm-actions button{flex:1 1 0}
html[data-crow-offline]::after{top:calc(8px + env(safe-area-inset-top));right:max(12px,env(safe-area-inset-right));font-size:14px}"#;

/// The mobile theme as the page carries it, after the kernel and the
/// runtime's own rules: every rule of `MOBILE_RULES` under `MOBILE_WHEN`
/// (unless the page says `data-surface="desktop"`) and again for
/// `data-surface="mobile"`, scoped by `:where`, which adds no specificity.
pub fn mobile_css() -> &'static str {
    static CSS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CSS.get_or_init(|| {
        let scoped = |when: &str| -> String {
            let mut out = String::new();
            for line in MOBILE_RULES.lines().filter(|l| !l.trim().is_empty()) {
                let (selectors, body) = line.split_once('{').unwrap_or((line, ""));
                let mut parts = Vec::new();
                let (mut depth, mut start) = (0i32, 0usize);
                for (i, c) in selectors.char_indices() {
                    match c {
                        '(' | '[' => depth += 1,
                        ')' | ']' => depth -= 1,
                        ',' if depth == 0 => {
                            parts.push(&selectors[start..i]);
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                parts.push(&selectors[start..]);
                let scoped: Vec<String> = parts
                    .iter()
                    .map(|s| {
                        let s = s.trim();
                        // the root itself: the scope joins its own compound
                        for root in [":root", "html"] {
                            if let Some(rest) = s.strip_prefix(root) {
                                return format!("{}:where({}){}", root, when, rest);
                            }
                        }
                        format!(":where(:root{}) {}", when, s)
                    })
                    .collect();
                out.push_str(&scoped.join(","));
                out.push('{');
                out.push_str(body);
            }
            out
        };
        format!("@media {}{{{}}}{}", MOBILE_WHEN, scoped(":not([data-surface=\"desktop\"])"), scoped("[data-surface=\"mobile\"]"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each format's words: grouped thousands and the minus kept, a
    /// percent of the number as it is, a moment from its column, epoch
    /// milliseconds or ISO text; what a format cannot read shows plain.
    #[test]
    fn a_cell_shows_in_its_format() {
        use crate::frame::Column;
        let f = |c: &Column, r: usize, fmt: &str| cell_formatted(c, r, true, fmt);
        let x = Column::Float(vec![1234.5, -1234567.891, -0.001, f64::NAN, 0.0]);
        assert_eq!(f(&x, 0, "money"), "1,234.50");
        assert_eq!(f(&x, 1, "money"), "-1,234,567.89");
        assert_eq!(f(&x, 2, "money"), "0.00");
        assert_eq!(f(&x, 3, "money"), "nan");
        assert_eq!(f(&x, 1, "whole"), "-1,234,568");
        assert_eq!(f(&x, 0, "number:3"), "1,234.500");
        assert_eq!(f(&x, 0, "number:0"), "1,235");
        assert_eq!(f(&x, 0, "percent"), "1234.50%");
        assert_eq!(f(&Column::Int(vec![1000]), 0, "whole"), "1,000");
        let at = Column::Int(vec![1762421400000]);
        assert_eq!(f(&at, 0, "date"), "2025-11-06");
        assert_eq!(f(&at, 0, "time"), "09:30:00");
        assert_eq!(f(&at, 0, "datetime"), "2025-11-06 09:30:00");
        let iso = Column::strs(vec!["2026-11-06T09:30:05Z".into(), "soon".into()]);
        assert_eq!(f(&iso, 0, "datetime"), "2026-11-06 09:30:05");
        assert_eq!(f(&iso, 1, "date"), "soon");
        assert_eq!(f(&Column::DateTime(vec![1762421400000], vec![-300]), 0, "time"), "04:30:00");
        assert_eq!(cell_formatted(&x, 0, false, "money"), x.fmt_cell(0, false));
        assert!(cell_format_known("number:2") && !cell_format_known("number:x") && !cell_format_known("dollars"));
    }

    fn numbers(n: usize) -> crate::frame::CrowFrame {
        crate::frame::CrowFrame::new(vec!["n".to_string()], vec![crate::frame::Column::Int((0..n as i64).collect())], vec![vec![true; n]])
    }

    /// The layers say what they are; an element with a click the browser
    /// does not press is a tab stop with a button's role (a row keeps its
    /// own), and one the program gave a role or a tab stop is left alone
    #[test]
    fn what_the_page_tells_a_screen_reader() {
        let roles = |tag: &str| render(tag, &[]).passthrough;
        assert_eq!(roles("modal"), vec![("role".to_string(), "dialog".to_string()), ("aria-modal".to_string(), "true".to_string())]);
        assert_eq!(roles("drawer"), vec![("role".to_string(), "dialog".to_string())]);
        assert_eq!(roles("toast"), vec![("role".to_string(), "status".to_string())]);
        assert!(roles("stack").is_empty());
        assert_eq!(render("modal", &[("role".into(), "alertdialog".into())]).passthrough, vec![("role".to_string(), "alertdialog".to_string())]);
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(press_attrs("div", &[]), vec![pair("tabindex", "0"), pair("role", "button")]);
        assert_eq!(press_attrs("tr", &[]), vec![pair("tabindex", "0")]);
        assert!(press_attrs("button", &[]).is_empty() && press_attrs("a", &[]).is_empty());
        assert!(press_attrs("div", &[pair("role", "tab")]).is_empty());
        let f = crate::frame::CrowFrame::new(vec!["sym".to_string()], vec![crate::frame::Column::strs(vec!["IMRN".into()])], vec![vec![true]]);
        let rows = frame_table_html(&f, Some("focus"), None, None, &[], &[], &[], &[], None, None);
        assert!(rows.contains("data-crow-click=\"focus\" data-crow-args=\"[&quot;IMRN&quot;]\" tabindex=\"0\""), "{}", rows);
    }

    /// The default stylesheet's text colours meet WCAG AA (4.5:1) on both
    /// grounds, in both themes (the light accent, which links are drawn
    /// in, had been 3.99:1)
    #[test]
    fn the_default_text_colours_meet_aa_contrast() {
        fn lum(hex: &str) -> f64 {
            let c = |i: usize| {
                let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
        }
        let ratio = |a: &str, b: &str| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        let mut themes = 0;
        for block in DEFAULT_CSS.split('}') {
            if !block.contains("--crow-bg:") {
                continue;
            }
            themes += 1;
            let token = |name: &str| block.split(&format!("--crow-{}:", name)).nth(1).map(|v| v[..7].to_string()).unwrap_or_default();
            for fg in ["text", "muted", "accent", "up", "down"] {
                for bg in ["bg", "surface"] {
                    let r = ratio(&token(fg), &token(bg));
                    assert!(r >= 4.5, "{} {} on {} {}: {:.2} in {}", fg, token(fg), bg, token(bg), r, block.split('{').next().unwrap_or(""));
                }
            }
        }
        assert!(themes >= 2, "both themes were read");
    }

    /// A sorted table's headers sort its rows: numbers as numbers, text
    /// alphabetically, missing cells last either way, equal cells in the
    /// frame's order; the sorted header says so, and a page shows the
    /// sorted rows
    #[test]
    fn a_sorted_table_shows_the_rows_in_its_columns_order() {
        use crate::frame::{Column, CrowFrame};
        let f = CrowFrame::new(
            vec!["sym".to_string(), "px".to_string()],
            vec![Column::strs(vec!["b".into(), "A".into(), "c".into(), "a".into()]), Column::Float(vec![2.0, 10.0, 0.0, 2.0])],
            vec![vec![true; 4], vec![true, true, false, true]],
        );
        // the first cell of each body row
        let rows = |html: &str| -> Vec<String> {
            let body = html.split("<tbody>").nth(1).and_then(|b| b.split("</tbody>").next()).unwrap_or("");
            body.split("<tr>").skip(1).map(|r| r.split("</td>").next().unwrap_or("").trim_start_matches("<td>").to_string()).collect()
        };
        let sorted = |by: Option<(&str, bool)>| Some(Sorting { index: 3, by: by.map(|(c, d)| (c.to_string(), d)) });
        let unsorted = frame_table_html(&f, None, None, None, &[], &[], &[], &[], None, sorted(None));
        assert!(unsorted.contains("<th><button data-crow-args=\"[3,&quot;sym&quot;]\" data-crow-click=\"__sort\">sym</button></th>"), "{}", unsorted);
        assert!(unsorted.contains("<th class=\"num\"><button data-crow-args=\"[3,&quot;px&quot;]\" data-crow-click=\"__sort\">px</button></th>"), "{}", unsorted);
        assert!(!unsorted.contains("aria-sort"), "{}", unsorted);
        assert_eq!(rows(&unsorted), ["b", "A", "c", "a"], "the frame's own order");
        let by_px = frame_table_html(&f, None, None, None, &[], &[], &[], &[], None, sorted(Some(("px", false))));
        assert!(by_px.contains("<th class=\"num\" aria-sort=\"ascending\">"), "{}", by_px);
        assert_eq!(rows(&by_px), ["b", "a", "A", "c"], "2.0, 2.0 in the frame's order, 10.0, then the missing cell");
        let down = frame_table_html(&f, None, None, None, &[], &[], &[], &[], None, sorted(Some(("px", true))));
        assert!(down.contains("aria-sort=\"descending\""), "{}", down);
        assert_eq!(rows(&down), ["A", "b", "a", "c"], "descending, the missing cell still last");
        let by_sym = frame_table_html(&f, None, None, None, &[], &[], &[], &[], None, sorted(Some(("sym", false))));
        assert_eq!(rows(&by_sym), ["A", "a", "b", "c"], "alphabetically");
        let paged = frame_table_html(&f, None, None, None, &[], &[], &[], &[], Some(Paging { size: 2, index: 0, at: 1 }), sorted(Some(("sym", false))));
        assert_eq!(rows(&paged), ["b", "c"], "the second page of the sorted rows");
        assert!(!frame_table_html(&f, None, None, None, &[], &[], &[], &[], None, None).contains("__sort"), "a table not sorted draws plain headers");
    }

    /// A paged table draws its page and says which; past its last page it
    /// shows the last; a table over the budget without a page is refused
    #[test]
    fn a_big_table_pages_and_a_bigger_one_is_refused() {
        let page = |at| Some(Paging { size: 50, index: 0, at });
        let html = frame_table_html(&numbers(120), None, None, None, &[], &[], &[], &[], page(1), None);
        assert!(html.contains("<td class=\"num\">50</td>") && !html.contains("<td class=\"num\">49</td>"), "{}", html);
        assert!(html.contains("<span>51–100 of 120</span>"), "{}", html);
        assert!(html.contains("data-crow-args=\"[0,0]\"") && html.contains("data-crow-args=\"[0,2]\""), "{}", html);
        let last = frame_table_html(&numbers(120), None, None, None, &[], &[], &[], &[], page(9), None);
        assert!(last.contains("<span>101–120 of 120</span>") && last.contains("<button disabled=\"\">Next</button>"), "{}", last);
        let refused = frame_table_html(&numbers(1001), None, None, None, &[], &[], &[], &[], None, None);
        assert!(refused.contains("table: 1001 rows — a table shows at most 1000 at a time"), "{}", refused);
        assert!(!frame_table_html(&numbers(1000), None, None, None, &[], &[], &[], &[], None, None).contains("crow-refused"));
    }
    /// A shortcut is written as a person reads it and rendered as the
    /// browser names it; a key no keyboard has, or a shift the character
    /// already says, is not a shortcut.
    #[test]
    fn a_shortcut_is_rendered_as_the_browser_names_its_keys() {
        for (written, named) in [
            ("ctrl+s", "Control+S"),
            ("Ctrl+S", "Control+S"),
            ("escape", "Escape"),
            ("?", "?"),
            ("/", "/"),
            ("shift+a", "Shift+A"),
            ("meta+shift+z", "Shift+Meta+Z"),
            ("alt+up", "Alt+ArrowUp"),
            ("f5", "F5"),
            ("ctrl++", "Control++"),
            ("ctrl+s meta+s", "Control+S Meta+S"),
        ] {
            assert_eq!(shortcut_aria(written).as_deref(), Some(named), "{}", written);
        }
        for wrong in ["", "hyper+s", "ctrl+", "ctrl+ctrl+s", "enterr", "shift+/", "f13", "ctrl+ab"] {
            assert_eq!(shortcut_aria(wrong), None, "{:?}", wrong);
        }
        let r = render("button", &[("shortcut".into(), "ctrl+s".into())]);
        assert_eq!(r.passthrough, vec![("aria-keyshortcuts".to_string(), "Control+S".to_string())]);
        let r = render("button", &[("shortcut".into(), "hyper+s".into())]);
        assert_eq!(r.errors, vec!["shortcut = 'hyper+s' is not a shortcut ('ctrl+s', 'escape', '?', 'ctrl+s meta+s') (see the layout vocabulary in the spec)".to_string()]);
    }
    #[test]
    fn containers_render_as_divs_with_their_rule() {
        let r = render("stack", &[("gap".into(), "m".into()), ("pad".into(), "l".into())]);
        assert_eq!(r.tag, "div");
        assert_eq!(r.class, "stack");
        assert_eq!(r.style, "display:flex;flex-direction:column;gap:16px;padding:24px");
        assert!(r.errors.is_empty());
    }
    #[test]
    fn responsive_rules_beat_the_inline_base_rule() {
        // The base rule (flex-direction:row, grid-template-columns) is emitted
        // inline on the element; a media rule only wins over it with !important.
        // Found by the first browser conformance check on the finch app.
        for rule in RESPONSIVE_CSS.split('}') {
            if rule.contains("[data-below=") || rule.contains("[data-hidden-below=") {
                assert!(rule.contains("!important"), "responsive rule without !important: {}", rule);
            }
        }
    }
    /// The mobile theme is the desktop's tokens with other values: every
    /// size token it sets is one the kernel stylesheet defines and uses
    /// (bar the few a phone alone needs), and every token the kernel uses
    /// it defines.
    #[test]
    fn the_mobile_theme_sets_the_kernels_own_tokens() {
        let defined = |css: &str, root: &str| -> Vec<String> {
            let block = css.split(&format!("\n{}{{", root)).nth(1).or_else(|| css.strip_prefix(&format!("{}{{", root))).unwrap_or("");
            let block = block.split('}').next().unwrap_or("");
            block.split(';').filter_map(|d| d.trim().strip_prefix("--crow-")).filter_map(|d| d.split(':').next()).map(str::to_string).collect()
        };
        let sizes = DEFAULT_CSS.lines().find(|l| l.starts_with(":root{--crow-font-size:")).expect("the kernel's size tokens");
        let kernel: Vec<String> = defined(sizes, ":root");
        assert!(kernel.len() >= 20, "{:?}", kernel);
        for t in &kernel {
            assert!(DEFAULT_CSS.contains(&format!("var(--crow-{})", t)), "the kernel defines --crow-{} and never uses it", t);
        }
        let mobile = defined(MOBILE_RULES, ":root");
        const PHONE_ONLY: &[&str] = &["surface-name", "grid-min", "type-xs", "type-s", "type-l", "type-xl"];
        for t in &mobile {
            assert!(kernel.contains(t) || PHONE_ONLY.contains(&t.as_str()), "the mobile theme sets --crow-{}, which the kernel has not", t);
        }
        for t in &kernel {
            assert!(mobile.contains(t), "the mobile theme leaves --crow-{} at the desktop's value", t);
        }
        for t in PHONE_ONLY {
            assert!(*t == "surface-name" || MOBILE_RULES.contains(&format!("var(--crow-{})", t)), "--crow-{} is set and never used", t);
        }
    }
    /// Every rule of the mobile theme is scoped to the phone (or to a page
    /// that asks for it) and adds no specificity, so the desktop is left as
    /// it was and an app's stylesheet still wins where it speaks.
    #[test]
    fn the_mobile_theme_is_scoped_and_adds_no_specificity() {
        let css = mobile_css();
        let (media, forced) = css.split_at(css.find("}}").expect("the media block closes") + 2);
        assert!(media.starts_with(&format!("@media {}{{", MOBILE_WHEN)), "{}", &media[..80]);
        let rules = MOBILE_RULES.lines().filter(|l| !l.trim().is_empty()).count();
        for (block, when) in [(&media[MOBILE_WHEN.len() + 8..media.len() - 1], ":not([data-surface=\"desktop\"])"), (forced, "[data-surface=\"mobile\"]")] {
            let mut seen = 0;
            for rule in block.split('}').filter(|r| !r.is_empty()) {
                seen += 1;
                let selectors = rule.split('{').next().unwrap_or("");
                let mut parts = Vec::new();
                let (mut depth, mut start) = (0i32, 0usize);
                for (i, c) in selectors.char_indices() {
                    match c {
                        '(' | '[' => depth += 1,
                        ')' | ']' => depth -= 1,
                        ',' if depth == 0 => {
                            parts.push(&selectors[start..i]);
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                parts.push(&selectors[start..]);
                for s in parts {
                    let scoped = s.starts_with(&format!(":where(:root{}) ", when)) || s.starts_with(&format!(":root:where({})", when)) || s.starts_with(&format!("html:where({})", when));
                    assert!(scoped, "an unscoped selector: {} in {}", s, rule);
                }
            }
            assert_eq!(seen, rules, "{}", block);
        }
    }
    #[test]
    fn grid_columns_and_responsive_rules() {
        let r = render("grid", &[("columns".into(), "2".into()), ("below".into(), "phone".into()), ("columns_then".into(), "1".into())]);
        assert!(r.style.contains("grid-template-columns:repeat(2, minmax(0, 1fr))"));
        assert_eq!(r.data, vec![("data-below".to_string(), "phone".to_string()), ("data-columns-then".to_string(), "1".to_string())]);
    }
    #[test]
    fn unknown_scale_is_an_error_and_html_attrs_pass_through() {
        let r = render("p", &[("size".into(), "huge".into()), ("href".into(), "/x".into())]);
        assert_eq!(r.tag, "p");
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.passthrough, vec![("href".to_string(), "/x".to_string())]);
    }
    /// The clicks a table offers are the ones its parts carry, for every
    /// table of thousands made at random: clicks, keys of every kind,
    /// actions that ask first, sorted headers, pagers, a refused table.
    #[test]
    fn a_tables_clicks_are_the_ones_its_parts_carry() {
        use crate::frame::{Column, CrowFrame};
        fn carried(p: &Part, out: &mut Vec<(String, String)>) {
            if let Some((_, h)) = p.attrs.iter().find(|(k, _)| *k == "data-crow-click") {
                let args = p.attrs.iter().find(|(k, _)| *k == "data-crow-args").map(|(_, v)| v.clone()).unwrap_or_default();
                out.push((h.clone(), args));
            }
            for k in &p.kids {
                if let Kid::Part(q) = k {
                    carried(q, out);
                }
            }
        }
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |n: usize| -> usize {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        const CELLS: &[&str] = &["IMRN", "a\"b", "P&L", "<x>", "", " ", "a\\b", "née", "\n"];
        for round in 0..2000 {
            let n = next(7) + if round % 50 == 0 { 1001 } else { 0 };
            let s: Vec<String> = (0..n).map(|_| CELLS[next(CELLS.len())].to_string()).collect();
            let x: Vec<f64> = (0..n).map(|_| [1.5, -2.0, f64::NAN][next(3)]).collect();
            let valid: Vec<bool> = (0..n).map(|_| next(5) != 0).collect();
            let f = CrowFrame::new(vec!["sym".into(), "px".into()], vec![Column::strs(s), Column::Float(x)], vec![vec![true; n], valid]);
            let actions: Vec<(String, String)> = (0..next(3)).map(|i| (["Buy", "Sell & go"][i % 2].to_string(), ["buy", "sell"][i % 2].to_string())).collect();
            let confirm: Vec<(String, String)> = if next(2) == 0 { vec![("Sell & go".into(), "Sure \"now\"?".into())] } else { vec![] };
            let sort = (next(2) == 0).then(|| Sorting { index: next(3), by: (next(2) == 0).then(|| ("px".to_string(), next(2) == 0)) });
            let hide: Vec<String> = if next(3) == 0 { vec!["sym".to_string()] } else { vec![] };
            let t = TableOf {
                frame: &f,
                click: (next(2) == 0).then_some("focus"),
                key: [None, Some("px"), Some("sym"), Some("gone")][next(4)],
                mark: None,
                delta: &[],
                actions: &actions,
                hide: &hide,
                confirm: &confirm,
                page: (next(2) == 0).then(|| Paging { size: next(4) + 1, index: next(3), at: next(5) }),
                sort: sort.as_ref(),
                formats: &[],
            };
            let mut from_parts = Vec::new();
            for p in frame_table_parts(&t) {
                carried(&p, &mut from_parts);
            }
            let mut clicks = frame_table_clicks(&t);
            from_parts.sort();
            clicks.sort();
            assert_eq!(clicks, from_parts);
        }
    }
}
