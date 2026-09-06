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
pub const CONTAINERS: &[&str] = &["stack", "row", "grid", "overlay", "scroll"];

/// Attributes the vocabulary owns. Anything else on an element passes
/// through as an HTML attribute (class, href, name, …).
pub const LAYOUT_ATTRS: &[&str] = &[
    "gap", "pad", "margin", "align", "justify", "wrap", "columns", "columns_then",
    "width", "height", "min_width", "max_width", "aspect", "fill",
    "surface", "radius", "border", "shadow", "color", "background",
    "size", "weight", "lines", "anchor", "below", "becomes", "hidden_below",
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
    let mut css: Vec<String> = Vec::new();
    let mut classes: Vec<String> = Vec::new();
    let mut data = Vec::new();
    let mut passthrough = Vec::new();
    let mut errors = Vec::new();
    if is_container { classes.push(tag.to_string()); }
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
        tag: if is_container { "div".to_string() } else { tag.to_string() },
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
/// whose truthy cells put `marked` on their row.
pub fn frame_table_html(f: &crate::frame::CrowFrame, click: Option<&str>, key: Option<&str>, mark: Option<&str>) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let mut out = String::from("<thead><tr>");
    for c in &f.columns { out.push_str(&format!("<th>{}</th>", esc(c))); }
    out.push_str("</tr></thead><tbody>");
    let key_i = key.and_then(|k| f.columns.iter().position(|c| c == k)).unwrap_or(0);
    let mark_i = mark.and_then(|m| f.columns.iter().position(|c| c == m));
    for r in 0..f.nrows() {
        let mut tr = String::from("<tr");
        if let Some(i) = mark_i {
            if f.data[i].truthy(r) && f.validity[i][r] { tr.push_str(" class=\"marked\""); }
        }
        if let Some(h) = click {
            if f.ncols() > 0 {
                let cell = f.data[key_i].fmt_cell(r, f.validity[key_i][r]);
                tr.push_str(&format!(" data-crow-click=\"{}\" data-crow-args=\"[&quot;{}&quot;]\"", esc(h), esc(&cell.replace('"', "\\\""))));
            }
        }
        tr.push('>');
        for c in 0..f.ncols() { tr.push_str(&format!("<td>{}</td>", esc(&f.data[c].fmt_cell(r, f.validity[c][r])))); }
        tr.push_str("</tr>");
        out.push_str(&tr);
    }
    out.push_str("</tbody>");
    out
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
.controls{display:flex;flex-wrap:wrap;gap:0.5rem;align-items:center;margin:0.9rem 0}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:0.6rem;margin:1.1rem 0}
.card{background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:4px;padding:0.75rem 0.9rem}
.card h3{margin:0 0 0.3rem;font-size:0.68rem;font-weight:600;text-transform:uppercase;letter-spacing:0.09em;color:var(--crow-muted)}
.card p{margin:0;font-family:ui-monospace,"SF Mono",Menlo,monospace;font-size:1.25rem;font-variant-numeric:tabular-nums;letter-spacing:-0.01em}
button.primary{background:var(--crow-accent);border-color:var(--crow-accent);color:var(--crow-bg);font-weight:600}
button.primary:hover{filter:brightness(1.08)}
.up{color:var(--crow-up)}
.down{color:var(--crow-down)}
.stale{color:var(--crow-muted)}
tr[data-crow-click]{cursor:pointer}
tr[data-crow-click]:hover td{background:color-mix(in srgb,var(--crow-accent) 10%,transparent)}
tr.marked td{background:color-mix(in srgb,var(--crow-up) 14%,transparent)}
.crow-slider input{width:16rem;padding:0;border:0;background:transparent}"#;

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
