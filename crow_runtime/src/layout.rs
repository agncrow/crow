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
    "tip", "open", "active", "side", "font",
];

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
            "class" => classes.push(v.to_string()),
            "gap" => match space_px(v) { Some(px) => css.push(format!("gap:{}px", px)), None => bad("spacing step (xs s m l xl)", &mut errors) },
            "pad" => {
                // 'm' or 'm l' (vertical horizontal)
                let parts: Vec<&str> = v.split_whitespace().collect();
                let pxs: Vec<Option<u32>> = parts.iter().map(|p| space_px(p)).collect();
                if pxs.is_empty() || pxs.iter().any(|p| p.is_none()) { bad("spacing step (xs s m l xl)", &mut errors); }
                else if pxs.len() == 1 { css.push(format!("padding:{}px", pxs[0].unwrap())); }
                else { css.push(format!("padding:{}px {}px", pxs[0].unwrap(), pxs[1].unwrap())); }
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

/// The most rows a table draws without `page` — as a chart has a point
/// budget, never a silent cut.
pub const TABLE_BUDGET: usize = 1000;

pub fn frame_table_html(f: &crate::frame::CrowFrame, click: Option<&str>, key: Option<&str>, mark: Option<&str>, delta: &[String], actions: &[(String, String)], hide: &[String], confirm: &[(String, String)], page: Option<Paging>) -> String {
    use crate::frame::Column;
    let total = f.nrows();
    if page.is_none() && total > TABLE_BUDGET {
        return format!(
            "<tbody><tr><td class=\"crow-refused\">table: {} rows — a table shows at most {} at a time: table[frame, page = 50]</td></tr></tbody>",
            total, TABLE_BUDGET
        );
    }
    // the rows this page shows
    let (first, last) = match page {
        Some(p) => {
            let pages = total.div_ceil(p.size.max(1)).max(1);
            let at = p.at.min(pages - 1);
            (at * p.size, ((at + 1) * p.size).min(total))
        }
        None => (0, total),
    };
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let numeric: Vec<bool> = f.data.iter().map(|c| matches!(c.as_ref(), Column::Int(_) | Column::Float(_))).collect();
    let is_delta: Vec<bool> = f.columns.iter().map(|c| delta.iter().any(|d| d == c)).collect();
    let shown: Vec<usize> = (0..f.ncols()).filter(|i| !hide.iter().any(|h| h == &f.columns[*i])).collect();
    let mut out = String::from("<thead><tr>");
    for &i in &shown {
        let c = &f.columns[i];
        if numeric[i] { out.push_str(&format!("<th class=\"num\">{}</th>", esc(c))); } else { out.push_str(&format!("<th>{}</th>", esc(c))); }
    }
    if !actions.is_empty() { out.push_str("<th></th>"); }
    out.push_str("</tr></thead><tbody>");
    let key_i = key.and_then(|k| f.columns.iter().position(|c| c == k)).unwrap_or(0);
    let mark_i = mark.and_then(|m| f.columns.iter().position(|c| c == m));
    let key_json = |r: usize| -> String {
        let cell = f.data[key_i].fmt_cell(r, f.validity[key_i][r]);
        if numeric[key_i] && f.validity[key_i][r] { format!("[{}]", cell.trim()) } else { format!("[&quot;{}&quot;]", esc(&cell.replace('"', "\\\""))) }
    };
    // the same arguments as the page sends them back, before escaping
    let raw_key_json = |r: usize| -> String {
        let cell = f.data[key_i].fmt_cell(r, f.validity[key_i][r]);
        if numeric[key_i] && f.validity[key_i][r] { format!("[{}]", cell.trim()) } else { serde_json::json!([cell]).to_string() }
    };
    for r in first..last {
        let mut tr = String::from("<tr");
        if let Some(i) = mark_i {
            if f.data[i].truthy(r) && f.validity[i][r] { tr.push_str(" class=\"marked\""); }
        }
        if let Some(h) = click {
            if f.ncols() > 0 {
                tr.push_str(&format!(" data-crow-click=\"{}\" data-crow-args=\"{}\"", esc(h), key_json(r)));
            }
        }
        tr.push('>');
        for &c in &shown {
            let text = f.data[c].fmt_cell(r, f.validity[c][r]);
            let mut classes: Vec<&str> = Vec::new();
            if numeric[c] { classes.push("num"); }
            if is_delta[c] && f.validity[c][r] {
                let sign = match f.data[c].as_ref() {
                    Column::Int(v) => v[r].signum() as f64,
                    Column::Float(v) => if v[r].is_finite() { v[r].signum() } else { 0.0 },
                    _ => text.trim().parse::<f64>().map(|x| x.signum()).unwrap_or(0.0),
                };
                if sign > 0.0 { classes.push("up"); } else if sign < 0.0 { classes.push("down"); }
            }
            if classes.is_empty() { tr.push_str(&format!("<td>{}</td>", esc(&text))); }
            else { tr.push_str(&format!("<td class=\"{}\">{}</td>", classes.join(" "), esc(&text))); }
        }
        if !actions.is_empty() && f.ncols() > 0 {
            tr.push_str("<td class=\"actions\">");
            for (label, h) in actions {
                match confirm.iter().find(|(l, _)| l == label) {
                    // an action that asks first offers the question
                    Some((_, text)) => tr.push_str(&format!(
                        "<button data-crow-click=\"__ask\" data-crow-args=\"{}\">{}</button>",
                        esc(&ask_args(h, Some(&raw_key_json(r)), text)),
                        esc(label)
                    )),
                    None => tr.push_str(&format!("<button data-crow-click=\"{}\" data-crow-args=\"{}\">{}</button>", esc(h), key_json(r), esc(label))),
                }
            }
            tr.push_str("</td>");
        }
        tr.push_str("</tr>");
        out.push_str(&tr);
    }
    out.push_str("</tbody>");
    // the pager: which rows, and the pages either side
    if let Some(p) = page {
        let at = first / p.size.max(1);
        let span = shown.len() + usize::from(!actions.is_empty());
        let step = |label: &str, to: Option<usize>| match to {
            Some(n) => format!("<button data-crow-args=\"[{},{}]\" data-crow-click=\"__page\">{}</button>", p.index, n, label),
            None => format!("<button disabled=\"\">{}</button>", label),
        };
        let shown_rows = if total == 0 { "0 of 0".to_string() } else { format!("{}–{} of {}", first + 1, last, total) };
        out.push_str(&format!(
            "<tfoot><tr><td class=\"crow-pager\" colspan=\"{}\">{}<span>{}</span>{}</td></tr></tfoot>",
            span.max(1),
            step("Previous", at.checked_sub(1)),
            shown_rows,
            step("Next", (last < total).then_some(at + 1))
        ));
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
    let Some((_, d)) = ICONS.iter().find(|(n, _)| *n == name) else {
        let names: Vec<&str> = ICONS.iter().map(|(n, _)| *n).collect();
        return Err(format!("icon '{}' is not in the set: {}", name, names.join(" ")));
    };
    let px = icon_px(if size.is_empty() { "m" } else { size }).ok_or_else(|| format!("size = '{}' is not a type step (xs s m l xl)", size))?;
    let color_css = if color.is_empty() { String::new() } else {
        format!(" style=\"color:{}\"", color_var(color).ok_or_else(|| format!("color = '{}' is not a colour token (text muted accent up down)", color))?)
    };
    let fill = if name == "dot" { "currentColor" } else { "none" };
    Ok(format!(
        "<svg class=\"icon icon-{n}\" width=\"{px}\" height=\"{px}\" viewBox=\"0 0 24 24\" fill=\"{fill}\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"{color}><path d=\"{d}\"></path></svg>",
        n = name, px = px, fill = fill, color = color_css, d = d
    ))
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
pub const DEFAULT_CSS: &str = r#":root{--crow-bg:#f6f7f9;--crow-surface:#ffffff;--crow-text:#1b222b;--crow-muted:#5d6773;--crow-border:#dce1e8;--crow-accent:#a86e1c;--crow-up:#177a4e;--crow-down:#bd3a3a;--crow-series-2:#2f6fb5;--crow-series-3:#7c4dbd;--crow-series-4:#178a8a;--crow-series-5:#c23d7a;--crow-series-6:#d2701a}
@media (prefers-color-scheme:dark){:root{--crow-bg:#0f1317;--crow-surface:#171d24;--crow-text:#e6e9ec;--crow-muted:#8b95a1;--crow-border:#28313c;--crow-accent:#e8a940;--crow-up:#3dbd85;--crow-down:#e06060;--crow-series-2:#5b9cf0;--crow-series-3:#a884ee;--crow-series-4:#3cc0c0;--crow-series-5:#ee6fa8;--crow-series-6:#f09a4a}}
:root[data-theme="dark"]{--crow-bg:#0f1317;--crow-surface:#171d24;--crow-text:#e6e9ec;--crow-muted:#8b95a1;--crow-border:#28313c;--crow-accent:#e8a940;--crow-up:#3dbd85;--crow-down:#e06060;--crow-series-2:#5b9cf0;--crow-series-3:#a884ee;--crow-series-4:#3cc0c0;--crow-series-5:#ee6fa8;--crow-series-6:#f09a4a}
:root[data-theme="light"]{--crow-bg:#f6f7f9;--crow-surface:#ffffff;--crow-text:#1b222b;--crow-muted:#5d6773;--crow-border:#dce1e8;--crow-accent:#a86e1c;--crow-up:#177a4e;--crow-down:#bd3a3a;--crow-series-2:#2f6fb5;--crow-series-3:#7c4dbd;--crow-series-4:#178a8a;--crow-series-5:#c23d7a;--crow-series-6:#d2701a}
*{box-sizing:border-box}
body{margin:0 auto;max-width:860px;padding:2.2rem 1.25rem 4rem;background:var(--crow-bg);color:var(--crow-text);font-family:system-ui,-apple-system,"Segoe UI",sans-serif;font-size:15px;line-height:1.55}
h1{font-size:1.45rem;font-weight:650;letter-spacing:-0.015em;margin:0 0 0.25rem}
h2{font-size:0.8rem;font-weight:600;text-transform:uppercase;letter-spacing:0.09em;color:var(--crow-muted);margin:2.2rem 0 0.7rem}
h3{font-size:0.95rem;margin:1.2rem 0 0.4rem}
p{margin:0.4rem 0}
a{color:var(--crow-accent)}
button{font:inherit;font-size:0.92rem;color:var(--crow-text);background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:0.42rem 0.95rem;cursor:pointer;transition:border-color 120ms ease,background 120ms ease}
button:hover{border-color:var(--crow-accent)}
button:focus-visible,input:focus-visible,select:focus-visible,textarea:focus-visible{outline:2px solid var(--crow-accent);outline-offset:1px}
@media (prefers-reduced-motion:reduce){button{transition:none}}
label{color:var(--crow-muted);font-size:0.88rem;display:inline-flex;align-items:center;gap:0.5rem}
input,select,textarea{font:inherit;font-size:0.92rem;color:var(--crow-text);background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:0.4rem 0.6rem}
input{width:9.5rem}
pre,code{font-family:ui-monospace,"SF Mono",Menlo,monospace;font-size:0.9em}
table{border-collapse:collapse;width:100%;font-size:0.92rem}
th{text-align:left;font-weight:600;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.08em;color:var(--crow-muted);border-bottom:1px solid var(--crow-border);padding:0.45rem 0.9rem 0.45rem 0}
td{border-bottom:1px solid var(--crow-border);padding:0.5rem 0.9rem 0.5rem 0;font-variant-numeric:tabular-nums}
td.crow-pager{border-bottom:0;text-align:right;color:var(--crow-muted)}
td.crow-pager span{margin:0 0.75rem}
td.crow-refused{color:var(--crow-down)}
.controls{display:flex;flex-wrap:wrap;gap:0.5rem;align-items:center;margin:0.9rem 0}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:0.6rem;margin:1.1rem 0}
.card{background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:0.75rem 0.9rem}
.card h3{margin:0 0 0.3rem;font-size:0.68rem;font-weight:600;text-transform:uppercase;letter-spacing:0.09em;color:var(--crow-muted)}
.card p{margin:0}
header{display:flex;align-items:center;padding:0.5rem 0.9rem;background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px}
td.actions{white-space:nowrap}td.actions button{padding:0.15rem 0.55rem;font-size:0.8rem;margin-right:0.35rem}
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
summary{cursor:pointer;font-weight:600;list-style:none;display:flex;align-items:center;gap:0.5rem}
summary::-webkit-details-marker{display:none}
summary::before{content:'';width:0.4em;height:0.4em;border-right:2px solid var(--crow-muted);border-bottom:2px solid var(--crow-muted);transform:rotate(-45deg);transition:transform 120ms ease;flex:none}
details[open] > summary::before{transform:rotate(45deg)}
details > :not(summary){margin-top:0.5rem}
.dropdown{position:relative;display:inline-block;border:0;padding:0;background:transparent}
.dropdown > summary{border:1px solid var(--crow-border);border-radius:4px;padding:0.42rem 0.95rem;background:var(--crow-surface);font-weight:400;font-size:0.92rem}
.dropdown > summary::before{order:2;margin-left:0.25rem}
.dropdown > :not(summary){position:absolute;top:calc(100% + 4px);left:0;z-index:20;min-width:12rem;margin:0;padding:0.35rem;background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;box-shadow:0 6px 18px rgba(0,0,0,.18)}
.dropdown[data-side="right"] > :not(summary){left:auto;right:0}
.dropdown > :not(summary) > button,.dropdown > :not(summary) > a{display:block;width:100%;text-align:left;border:0;background:transparent;border-radius:3px;padding:0.4rem 0.6rem;color:var(--crow-text);text-decoration:none}
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
.crow-check{display:inline-flex;align-items:center;gap:0.5rem;color:var(--crow-text);font-size:0.92rem;cursor:pointer}
.crow-check input{width:auto;margin:0;accent-color:var(--crow-accent)}
.crow-toggle input{appearance:none;-webkit-appearance:none;width:2.2rem;height:1.25rem;border-radius:999px;background:var(--crow-border);position:relative;padding:0;border:0;transition:background 120ms ease;cursor:pointer}
.crow-toggle input::after{content:'';position:absolute;top:2px;left:2px;width:calc(1.25rem - 4px);height:calc(1.25rem - 4px);border-radius:50%;background:var(--crow-surface);box-shadow:0 1px 2px rgba(0,0,0,.3);transition:transform 120ms ease}
.crow-toggle input:checked{background:var(--crow-accent)}
.crow-toggle input:checked::after{transform:translateX(0.95rem)}
.crow-radio{display:inline-flex;gap:1rem;flex-wrap:wrap;align-items:center}
.crow-radio label{color:var(--crow-text);cursor:pointer}
.crow-radio input{width:auto;margin:0;accent-color:var(--crow-accent)}
.crow-tabs{display:flex;gap:0;border-bottom:1px solid var(--crow-border);align-items:stretch}
.crow-tabs label{cursor:pointer;padding:0.5rem 0.9rem;margin-bottom:-1px;border-bottom:2px solid transparent;color:var(--crow-muted);font-size:0.92rem;font-weight:500;white-space:nowrap;position:relative}
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
nav:not(.doc-nav):not(.doc-pager) a{color:var(--crow-muted);text-decoration:none;padding:0.35rem 0.6rem;border-radius:4px}
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

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(n: usize) -> crate::frame::CrowFrame {
        crate::frame::CrowFrame::new(vec!["n".to_string()], vec![crate::frame::Column::Int((0..n as i64).collect())], vec![vec![true; n]])
    }

    /// A paged table draws its page and says which; past its last page it
    /// shows the last; a table over the budget without a page is refused
    #[test]
    fn a_big_table_pages_and_a_bigger_one_is_refused() {
        let page = |at| Some(Paging { size: 50, index: 0, at });
        let html = frame_table_html(&numbers(120), None, None, None, &[], &[], &[], &[], page(1));
        assert!(html.contains("<td class=\"num\">50</td>") && !html.contains("<td class=\"num\">49</td>"), "{}", html);
        assert!(html.contains("<span>51–100 of 120</span>"), "{}", html);
        assert!(html.contains("data-crow-args=\"[0,0]\"") && html.contains("data-crow-args=\"[0,2]\""), "{}", html);
        let last = frame_table_html(&numbers(120), None, None, None, &[], &[], &[], &[], page(9));
        assert!(last.contains("<span>101–120 of 120</span>") && last.contains("<button disabled=\"\">Next</button>"), "{}", last);
        let refused = frame_table_html(&numbers(1001), None, None, None, &[], &[], &[], &[], None);
        assert!(refused.contains("table: 1001 rows — a table shows at most 1000 at a time"), "{}", refused);
        assert!(!frame_table_html(&numbers(1000), None, None, None, &[], &[], &[], &[], None).contains("crow-refused"));
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
}
