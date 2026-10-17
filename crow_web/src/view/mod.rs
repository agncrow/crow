//! `web.view` module — WASM frontend components (stub).
//!
//! This module defines the component model for Crow's client-side view system.
//! The actual WASM compilation target is handled by the Crow compiler;
//! this module provides the type definitions and component API.
//!
//! ```text
//! use web.view
//!
//! component counter
//!   state = {count, 0}
//!
//!   render = fn[s]
//!     '<div>
//!       <span>{s.count}</span>
//!       <button data-event="inc">+</button>
//!     </div>'
//!
//!   on 'inc' = fn[s]
//!     s.count = s.count + 1
//! ```

use std::collections::HashMap;

/// A view component definition.
#[derive(Debug, Clone)]
pub struct CrowComponent {
    /// Component name.
    pub name: String,
    /// Initial state.
    pub initial_state: HashMap<String, serde_json::Value>,
    /// Event handlers (event_name → handler index).
    pub events: Vec<String>,
}

impl CrowComponent {
    /// Create a new component.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            initial_state: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Set an initial state value.
    pub fn state(mut self, key: &str, value: serde_json::Value) -> Self {
        self.initial_state.insert(key.to_string(), value);
        self
    }

    /// Register an event handler.
    pub fn on(mut self, event: &str) -> Self {
        self.events.push(event.to_string());
        self
    }
}

/// Virtual DOM node (simplified).
#[derive(Debug, Clone, serde::Serialize)]
pub enum CrowVNode {
    /// Text content.
    Text(String),
    /// Pre-built markup emitted verbatim (charts' SVG). Producers are
    /// trusted crate code, never user strings.
    Raw(String),
    /// Element with tag, attributes, and children.
    Element {
        tag: String,
        attrs: HashMap<String, String>,
        children: Vec<CrowVNode>,
    },
}

impl CrowVNode {
    /// Create a text node.
    pub fn text(s: &str) -> Self {
        CrowVNode::Text(s.to_string())
    }

    /// What a view shows in place of a value that failed: an id the
    /// server's log has the reason under (crate::core::response).
    pub fn failed(reason: &str) -> Self {
        CrowVNode::text(&crate::core::response::failed_in_view(reason))
    }

    /// Create an element node.
    pub fn el(tag: &str, attrs: HashMap<String, String>, children: Vec<CrowVNode>) -> Self {
        CrowVNode::Element {
            tag: tag.to_string(),
            attrs,
            children,
        }
    }

    /// Create an element with no attributes.
    pub fn tag(tag: &str, children: Vec<CrowVNode>) -> Self {
        Self::el(tag, HashMap::new(), children)
    }

    /// Add an attribute to this element. Returns self for chaining.
    /// No-op on text nodes.
    pub fn attr(mut self, key: &str, value: &str) -> Self {
        if let CrowVNode::Element { ref mut attrs, .. } = self {
            attrs.insert(key.to_string(), value.to_string());
        }
        self
    }

    /// A class computed as the page renders, joined to the element's own
    /// as the layout table joins them (crow_runtime::layout::render): each
    /// class once, single spaces, an empty one adding nothing.
    pub fn with_class(mut self, value: &str) -> Self {
        if let CrowVNode::Element { ref mut attrs, .. } = self {
            let mut classes: Vec<String> = attrs.get("class").map(|c| c.split_whitespace().map(str::to_string).collect()).unwrap_or_default();
            classes.extend(value.split_whitespace().map(str::to_string));
            if classes.is_empty() {
                attrs.remove("class");
            } else {
                attrs.insert("class".to_string(), classes.join(" "));
            }
        }
        self
    }

    /// `confirm = 'text'`: the click asks first. The element offers the
    /// question instead of its handler (crow_runtime::layout::confirm_wiring).
    pub fn confirm(self, text: &str) -> Self {
        match self {
            CrowVNode::Element { tag, attrs, children } => {
                let events: Vec<(String, String)> = attrs.iter().filter(|(k, _)| k.as_str() == "data-crow-click" || k.as_str() == "data-crow-args").map(|(k, v)| (k.clone(), v.clone())).collect();
                let mut attrs: HashMap<String, String> = attrs.into_iter().filter(|(k, _)| k.as_str() != "data-crow-click" && k.as_str() != "data-crow-args").collect();
                attrs.extend(crow_runtime::layout::confirm_wiring(events, Some(text)));
                CrowVNode::Element { tag, attrs, children }
            }
            other => other,
        }
    }

    /// Add a click event handler name. Sets `data-crow-click` attribute,
    /// and makes an element the browser does not press pressable from the
    /// keyboard (crow_runtime::layout::press_attrs).
    pub fn on_click(self, event_name: &str) -> Self {
        self.pressable().attr("data-crow-click", event_name)
    }

    /// A tab stop and a button's role for an element with a click that the
    /// browser does not press itself.
    fn pressable(self) -> Self {
        let extra = match &self {
            CrowVNode::Element { tag, attrs, .. } => {
                let set: Vec<(String, String)> = attrs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                crow_runtime::layout::press_attrs(tag, &set)
            }
            _ => Vec::new(),
        };
        extra.into_iter().fold(self, |node, (k, v)| node.attr(&k, &v))
    }

    /// A click handler with frozen arguments (F-11): the handler name plus
    /// a JSON array of values captured at render, delivered back with the
    /// event. `click = set_speed[10.0]` lowers to this.
    pub fn on_click_args(self, event_name: &str, args_json: &str) -> Self {
        self.pressable()
            .attr("data-crow-click", event_name)
            .attr("data-crow-args", args_json)
    }

    /// Add an input event handler name. Sets `data-crow-input` attribute.
    pub fn on_input(self, event_name: &str) -> Self {
        self.attr("data-crow-input", event_name)
    }

    /// Add a submit event handler name. Sets `data-crow-submit` attribute.
    pub fn on_submit(self, event_name: &str) -> Self {
        self.attr("data-crow-submit", event_name)
    }

    // --- Convenience element constructors ---

    pub fn div(children: Vec<CrowVNode>) -> Self { Self::tag("div", children) }
    pub fn span(children: Vec<CrowVNode>) -> Self { Self::tag("span", children) }
    pub fn p(children: Vec<CrowVNode>) -> Self { Self::tag("p", children) }
    pub fn h1(children: Vec<CrowVNode>) -> Self { Self::tag("h1", children) }
    pub fn h2(children: Vec<CrowVNode>) -> Self { Self::tag("h2", children) }
    pub fn h3(children: Vec<CrowVNode>) -> Self { Self::tag("h3", children) }
    pub fn h4(children: Vec<CrowVNode>) -> Self { Self::tag("h4", children) }
    pub fn h5(children: Vec<CrowVNode>) -> Self { Self::tag("h5", children) }
    pub fn h6(children: Vec<CrowVNode>) -> Self { Self::tag("h6", children) }
    pub fn button(children: Vec<CrowVNode>) -> Self { Self::tag("button", children) }
    pub fn ul(children: Vec<CrowVNode>) -> Self { Self::tag("ul", children) }
    pub fn ol(children: Vec<CrowVNode>) -> Self { Self::tag("ol", children) }
    pub fn li(children: Vec<CrowVNode>) -> Self { Self::tag("li", children) }
    pub fn form(children: Vec<CrowVNode>) -> Self { Self::tag("form", children) }
    pub fn input() -> Self { Self::el("input", HashMap::new(), vec![]) }
    pub fn img() -> Self { Self::el("img", HashMap::new(), vec![]) }
    pub fn a(children: Vec<CrowVNode>) -> Self { Self::tag("a", children) }
    pub fn nav(children: Vec<CrowVNode>) -> Self { Self::tag("nav", children) }
    pub fn header(children: Vec<CrowVNode>) -> Self { Self::tag("header", children) }
    pub fn footer(children: Vec<CrowVNode>) -> Self { Self::tag("footer", children) }
    pub fn main_el(children: Vec<CrowVNode>) -> Self { Self::tag("main", children) }
    pub fn section(children: Vec<CrowVNode>) -> Self { Self::tag("section", children) }
    pub fn table(children: Vec<CrowVNode>) -> Self { Self::tag("table", children) }
    pub fn tr(children: Vec<CrowVNode>) -> Self { Self::tag("tr", children) }
    pub fn td(children: Vec<CrowVNode>) -> Self { Self::tag("td", children) }
    pub fn th(children: Vec<CrowVNode>) -> Self { Self::tag("th", children) }
    pub fn thead(children: Vec<CrowVNode>) -> Self { Self::tag("thead", children) }
    pub fn tbody(children: Vec<CrowVNode>) -> Self { Self::tag("tbody", children) }
    pub fn select(children: Vec<CrowVNode>) -> Self { Self::tag("select", children) }
    pub fn option(children: Vec<CrowVNode>) -> Self { Self::tag("option", children) }
    pub fn textarea(children: Vec<CrowVNode>) -> Self { Self::tag("textarea", children) }
    pub fn label(children: Vec<CrowVNode>) -> Self { Self::tag("label", children) }
    pub fn pre(children: Vec<CrowVNode>) -> Self { Self::tag("pre", children) }
    pub fn code(children: Vec<CrowVNode>) -> Self { Self::tag("code", children) }
    pub fn details(children: Vec<CrowVNode>) -> Self { Self::tag("details", children) }
    pub fn summary(children: Vec<CrowVNode>) -> Self { Self::tag("summary", children) }
    pub fn hr() -> Self { Self::el("hr", HashMap::new(), vec![]) }
    /// A page's title and its metas: a served page carries them in its head
    /// (`split_head`).
    pub fn title(children: Vec<CrowVNode>) -> Self { Self::tag("title", children) }
    pub fn meta() -> Self { Self::el("meta", HashMap::new(), vec![]) }

    /// `open = expr` at render time: a real `open` attribute on details, a
    /// data attribute on the layers — the same rule the kernel applies to
    /// literals.
    pub fn open_attr(self, open: bool) -> Self {
        let is_details = matches!(&self, CrowVNode::Element { tag, .. } if tag == "details");
        if is_details {
            if open { self.attr("open", "") } else { self }
        } else {
            self.attr("data-open", if open { "true" } else { "false" })
        }
    }

    /// `nav[..., active = path]`: the link whose href is the active path
    /// gets aria-current="page".
    pub fn mark_active(mut self, active: &str) -> Self {
        if active.is_empty() { return self; }
        fn walk(node: &mut CrowVNode, active: &str) {
            if let CrowVNode::Element { tag, attrs, children } = node {
                if tag == "a" && attrs.get("href").map(|h| h == active).unwrap_or(false) {
                    attrs.insert("aria-current".to_string(), "page".to_string());
                }
                for c in children.iter_mut() { walk(c, active); }
            }
        }
        walk(&mut self, active);
        self
    }

    /// `icon[name, size, color]`, from the kernel's closed set.
    pub fn icon(name: &str, size: &str, color: &str) -> Self {
        match crow_runtime::layout::icon_html(name, size, color) {
            Ok(svg) => CrowVNode::Raw(svg),
            Err(e) => CrowVNode::text(&format!("error: {}", e)),
        }
    }

    /// `progress[value, max]`.
    pub fn progress(value: f64, max: f64) -> Self {
        CrowVNode::el("progress", HashMap::new(), vec![])
            .attr("class", "crow-progress")
            .attr("value", &fmt_num(value))
            .attr("max", &fmt_num(if max > 0.0 { max } else { 1.0 }))
    }

    /// `checkbox[label, var]` / `toggle[label, var]` — a bool bound to state.
    pub fn check_input(label: &str, key: &str, checked: bool, toggle: bool) -> Self {
        let mut input = CrowVNode::input()
            .attr("type", "checkbox")
            .attr("data-crow-input", &format!("__field:{}", key));
        if checked { input = input.attr("checked", ""); }
        CrowVNode::label(vec![input, CrowVNode::text(label)])
            .attr("class", if toggle { "crow-check crow-toggle" } else { "crow-check" })
    }

    /// `radio[options, var]` / `tabs[options, var]` — one of several
    /// strings bound to state; the selected option's label is marked.
    pub fn choice_input(options: &[String], key: &str, current: &str, tabs: bool) -> Self {
        let name = format!("__field:{}", key);
        let items: Vec<CrowVNode> = options.iter().map(|opt| {
            let selected = opt == current;
            let mut input = CrowVNode::input()
                .attr("type", "radio")
                .attr("name", &name)
                .attr("value", opt)
                .attr("data-crow-input", &name);
            if selected { input = input.attr("checked", ""); }
            let mut label = CrowVNode::label(vec![input, CrowVNode::text(opt)]);
            if tabs { label = label.attr("role", "tab").attr("aria-selected", if selected { "true" } else { "false" }); }
            label
        }).collect();
        let mut node = CrowVNode::div(items).attr("class", if tabs { "crow-tabs" } else { "crow-radio" });
        if tabs { node = node.attr("role", "tablist"); }
        node
    }

    /// `field[label, var, type, lines]` — a labelled input (or a textarea
    /// when `lines` > 0) bound to state.
    pub fn field_typed(label: &str, key: &str, current_value: &str, kind: &str, lines: i64) -> Self {
        let bound = format!("__field:{}", key);
        // a file input cannot be handed a value: it names the variable it
        // uploads into, and the name of the file picked is shown beside it
        if kind == "file" {
            let mut kids = vec![
                CrowVNode::label(vec![CrowVNode::text(label)]),
                CrowVNode::input().attr("type", "file").attr("data-crow-upload", &bound).attr("aria-label", label),
            ];
            if !current_value.is_empty() {
                kids.push(CrowVNode::tag("span", vec![CrowVNode::text(current_value)]).attr("class", "crow-file"));
            }
            return CrowVNode::div(kids).attr("class", "crow-field");
        }
        let control = if lines > 0 {
            CrowVNode::textarea(vec![CrowVNode::text(current_value)])
                .attr("rows", &lines.to_string())
                .attr("data-crow-input", &bound)
                .attr("aria-label", label)
        } else {
            CrowVNode::input()
                .attr("type", if kind.is_empty() { "text" } else { kind })
                .attr("value", current_value)
                .attr("data-crow-input", &bound)
                .attr("aria-label", label)
        };
        CrowVNode::div(vec![CrowVNode::label(vec![CrowVNode::text(label)]), control]).attr("class", "crow-field")
    }

    /// `widget[name, props = value, on = map[event = handler]]`: the element
    /// the program's module `widgets/name.js` draws. Its contents are the
    /// module's; the page reads the props and the events from it.
    pub fn widget(module: &str, props: &serde_json::Value, on: &[(&str, &str)]) -> Self {
        if module.is_empty() || !module.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return CrowVNode::text(&format!("widget: '{}' is not a module name — letters, digits, _ and -", module));
        }
        let events: serde_json::Map<String, serde_json::Value> = on.iter().map(|(e, h)| (e.to_string(), serde_json::json!(h))).collect();
        CrowVNode::div(vec![])
            .attr("class", "crow-widget")
            .attr("data-crow-widget", module)
            .attr("data-crow-props", &props.to_string())
            .attr("data-crow-on", &serde_json::Value::Object(events).to_string())
    }

    /// Render to HTML string.
    pub fn to_html(&self) -> String {
        match self {
            CrowVNode::Text(s) => escape_text(s),
            CrowVNode::Raw(s) => s.clone(),
            CrowVNode::Element {
                tag,
                attrs,
                children,
            } => {
                let mut attr_str = String::new();
                let mut sorted_attrs: Vec<_> = attrs.iter().collect();
                sorted_attrs.sort_by_key(|(k, _)| (*k).clone());
                for (k, v) in sorted_attrs {
                    attr_str.push_str(&format!(" {}=\"{}\"", k, html_escape(v)));
                }
                let lower = tag.to_ascii_lowercase();
                let children_html: String = match lower.as_str() {
                    // a script's or a style's text is read as it stands
                    "script" | "style" => children.iter().map(|c| match c {
                        CrowVNode::Text(t) => t.clone(),
                        other => other.to_html(),
                    }).collect(),
                    _ => children.iter().map(|c| c.to_html()).collect(),
                };
                if VOID.contains(&lower.as_str()) {
                    // no end tag: `</br>` would be read as a second break
                    return format!("<{}{}>{}", tag, attr_str, children_html);
                }
                // the browser drops the newline that opens these
                let lead = if DROPS_NEWLINE.contains(&lower.as_str()) && children_html.starts_with('\n') { "\n" } else { "" };
                format!("<{}{}>{}{}</{}>", tag, attr_str, lead, children_html, tag)
            }
        }
    }

    /// Build a `<table>` from headers and rows.
    ///
    /// ```text
    /// table[frame]  →  table_from_data(headers, rows)
    /// ```
    pub fn raw(s: &str) -> Self {
        CrowVNode::Raw(s.to_string())
    }

    /// chart[frame, ...] — extract named columns and hand geometry+SVG to
    /// the shared kernel, so both modes emit identical markup.
    pub fn chart_from_frame(
        f: &crow_runtime::frame::CrowFrame,
        x: &str,
        candle_cols: &[String],
        line_cols: &[String],
        bars_y: &str,
        marker: f64,
        tz: &str,
        height: f64,
        panes: &[Vec<String>],
        levels: &dyn crow_runtime::chart::LevelMap,
        points: Option<&crow_runtime::frame::CrowFrame>,
        seek: Option<&str>,
    ) -> Self {
        let col = |name: &str| -> Vec<f64> {
            f.col_native(name).map(|c| c.to_f64_vec()).unwrap_or_default()
        };
        let ts = col(x);
        let (o, h, l, c4);
        if candle_cols.len() == 4 {
            o = col(&candle_cols[0]);
            h = col(&candle_cols[1]);
            l = col(&candle_cols[2]);
            c4 = col(&candle_cols[3]);
        } else {
            o = vec![];
            h = vec![];
            l = vec![];
            c4 = vec![];
        }
        let lines_data: Vec<(String, Vec<f64>)> =
            line_cols.iter().map(|n| (n.clone(), col(n))).collect();
        let vol = if bars_y.is_empty() { vec![] } else { col(bars_y) };
        // Panes: each a list of column names; a pane draws the levels of
        // every column it holds
        let pane_data: Vec<Vec<(String, Vec<f64>)>> = panes
            .iter()
            .map(|cols| cols.iter().map(|n| (n.clone(), col(n))).collect())
            .collect();
        let pane_specs: Vec<crow_runtime::chart::Pane> = pane_data
            .iter()
            .map(|lines| crow_runtime::chart::Pane {
                lines: lines.iter().map(|(n, v)| (n.as_str(), v.as_slice())).collect(),
                levels: lines.iter().flat_map(|(n, _)| levels.levels_for(n)).collect(),
            })
            .collect();
        // Points: a frame with ts, y, label, kind columns (label and kind optional)
        let point_specs: Vec<crow_runtime::chart::Point> = points
            .map(|pf| {
                let pts = pf.col_native("ts").map(|c| c.to_f64_vec()).unwrap_or_default();
                let ys = pf.col_native("y").map(|c| c.to_f64_vec()).unwrap_or_default();
                let labels = pf.col_native("label");
                let kinds = pf.col_native("kind");
                (0..pts.len())
                    .map(|i| crow_runtime::chart::Point {
                        ts: pts[i],
                        y: ys.get(i).cloned().unwrap_or(f64::NAN),
                        label: labels.as_ref().map(|c| c.get_str(i)).unwrap_or_default(),
                        kind: kinds.as_ref().map(|c| c.get_str(i)).unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let spec = crow_runtime::chart::ChartSpec {
            ts: &ts,
            candles: (&o, &h, &l, &c4),
            lines: lines_data.iter().map(|(n, v)| (n.as_str(), v.as_slice())).collect(),
            volume: &vol,
            marker,
            tz,
            width: 720.0,
            height,
            panes: pane_specs,
            points: point_specs,
            seek,
        };
        match crow_runtime::chart::render_svg(&spec) {
            Ok(svg) => CrowVNode::Raw(svg),
            Err(e) => CrowVNode::text(&format!("chart error: {}", e)),
        }
    }

    pub fn table_from_data(headers: &[String], rows: &[Vec<String>]) -> Self {
        let header_cells: Vec<CrowVNode> = headers.iter()
            .map(|h| CrowVNode::th(vec![CrowVNode::text(h)]))
            .collect();
        let body_rows: Vec<CrowVNode> = rows.iter()
            .map(|row| {
                let cells: Vec<CrowVNode> = row.iter()
                    .map(|c| CrowVNode::td(vec![CrowVNode::text(c)]))
                    .collect();
                CrowVNode::tr(cells)
            })
            .collect();
        CrowVNode::table(vec![
            CrowVNode::thead(vec![CrowVNode::tr(header_cells)]),
            CrowVNode::tbody(body_rows),
        ])
    }

    /// Build a labeled `<input>` with two-way binding.
    ///
    /// ```text
    /// field['Min Vol', f.min_vol]  →  field(label, key, current_value)
    /// ```
    ///
    /// Renders: `<div class="crow-field"><label>Label</label><input data-crow-input="__field:key" value="..."></div>`
    pub fn field(label: &str, key: &str, current_value: &str) -> Self {
        CrowVNode::div(vec![
            CrowVNode::label(vec![CrowVNode::text(label)]),
            CrowVNode::input()
                .attr("type", "text")
                .attr("value", current_value)
                .attr("data-crow-input", &format!("__field:{}", key))
                .attr("aria-label", label),
        ]).attr("class", "crow-field")
    }

    /// slider[label, var, min, max, step] — a range input bound to state.
    pub fn slider(label: &str, key: &str, current_value: &str, min: f64, max: f64, step: f64) -> Self {
        CrowVNode::div(vec![
            CrowVNode::label(vec![CrowVNode::text(label)]),
            CrowVNode::input()
                .attr("type", "range")
                .attr("min", &format!("{}", min))
                .attr("max", &format!("{}", max))
                .attr("step", &format!("{}", step))
                .attr("value", current_value)
                .attr("data-crow-input", &format!("__field:{}", key))
                .attr("aria-label", label),
        ]).attr("class", "crow-field crow-slider")
    }

    /// Build a `<select>` dropdown with two-way binding.
    ///
    /// ```text
    /// select[options, pb_filter]  →  select_input(options, key, current_value)
    /// ```
    ///
    /// Renders: `<div class="crow-select"><select data-crow-change="__select:key"><option>...</option></select></div>`
    pub fn select_input(options: &[String], key: &str, current_value: &str) -> Self {
        let option_nodes: Vec<CrowVNode> = options.iter()
            .map(|opt| {
                let mut node = CrowVNode::option(vec![CrowVNode::text(opt)])
                    .attr("value", opt);
                if opt == current_value {
                    node = node.attr("selected", "selected");
                }
                node
            })
            .collect();
        CrowVNode::div(vec![
            CrowVNode::select(option_nodes)
                .attr("data-crow-change", &format!("__select:{}", key)),
        ]).attr("class", "crow-select")
    }
}

/// Elements a browser builds with no children: their end tag, if written,
/// is ignored.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param", "source", "track", "wbr"];

/// Elements whose content a browser reads as text up to their end tag: the
/// first ones as it stands, the last two with their entities read.
const RAW_TEXT: &[&str] = &["script", "style", "xmp", "iframe", "noembed", "noframes", "noscript", "textarea", "title"];

/// Elements whose first newline the browser drops (and so a writer doubles).
const DROPS_NEWLINE: &[&str] = &["pre", "listing", "textarea"];

/// Starting one of these closes an open `<p>`.
const CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "center", "dd", "details", "dialog", "dir", "div", "dl", "dt", "fieldset",
    "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "li", "listing",
    "main", "menu", "nav", "ol", "p", "pre", "search", "section", "summary", "table", "ul", "xmp",
];

/// The elements an end tag of an ordinary element does not reach past.
const SPECIAL: &[&str] = &[
    "address", "applet", "area", "article", "aside", "base", "basefont", "bgsound", "blockquote", "body", "br", "button",
    "caption", "center", "col", "colgroup", "dd", "details", "dir", "div", "dl", "dt", "embed", "fieldset", "figcaption",
    "figure", "footer", "form", "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hgroup", "hr",
    "html", "iframe", "img", "input", "keygen", "li", "link", "listing", "main", "marquee", "menu", "meta", "nav", "noembed",
    "noframes", "noscript", "object", "ol", "p", "param", "plaintext", "pre", "script", "search", "section", "select",
    "source", "style", "summary", "table", "tbody", "td", "template", "textarea", "tfoot", "th", "thead", "title", "tr",
    "track", "ul", "wbr", "xmp",
];

/// End tags that close their element only when it is in scope.
const SCOPED_END: &[&str] = &[
    "address", "article", "aside", "blockquote", "button", "center", "details", "dialog", "dir", "div", "dl", "fieldset",
    "figcaption", "figure", "footer", "header", "hgroup", "listing", "main", "menu", "nav", "ol", "pre", "search", "section",
    "summary", "ul", "form", "li", "dd", "dt", "h1", "h2", "h3", "h4", "h5", "h6", "select", "option", "optgroup",
    "table", "caption", "colgroup", "tbody", "thead", "tfoot", "tr", "td", "th",
];

/// Where a scope ends, for the scoped end tags and the auto-closes.
const SCOPE: &[&str] = &["applet", "caption", "html", "table", "td", "th", "marquee", "object", "template", "foreignobject", "desc"];

/// Tags that leave an SVG or MathML drawing for the page around it.
const BREAKS_OUT: &[&str] = &[
    "b", "big", "blockquote", "body", "br", "center", "code", "dd", "div", "dl", "dt", "em", "embed", "h1", "h2", "h3", "h4",
    "h5", "h6", "head", "hr", "i", "img", "li", "listing", "menu", "meta", "nobr", "ol", "p", "pre", "ruby", "s", "small",
    "span", "strong", "strike", "sub", "sup", "table", "tt", "u", "ul", "var",
];

fn unescape_markup(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let semi = rest[1..].find(';').map(|i| i + 1).filter(|i| *i <= 32);
        let decoded = semi.and_then(|i| {
            let name = &rest[1..i];
            let ch = if let Some(num) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(num, 16).ok().and_then(char::from_u32)
            } else if let Some(num) = name.strip_prefix('#') {
                num.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                named_entity(name)
            };
            ch.map(|c| (c, i + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The named characters a page is likely to write; the browser knows more,
/// and text keeps its entities as written, so only an attribute value can
/// meet one missing here.
fn named_entity(name: &str) -> Option<char> {
    let c = match name {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'', "nbsp" => '\u{a0}',
        "copy" => '©', "reg" => '®', "trade" => '™', "hellip" => '…', "mdash" => '—', "ndash" => '–',
        "lsquo" => '‘', "rsquo" => '’', "sbquo" => '‚', "ldquo" => '“', "rdquo" => '”', "bdquo" => '„',
        "laquo" => '«', "raquo" => '»', "lsaquo" => '‹', "rsaquo" => '›', "bull" => '•', "middot" => '·',
        "times" => '×', "divide" => '÷', "deg" => '°', "plusmn" => '±', "micro" => 'µ', "para" => '¶',
        "sect" => '§', "cent" => '¢', "pound" => '£', "euro" => '€', "yen" => '¥', "curren" => '¤',
        "iexcl" => '¡', "iquest" => '¿', "sup1" => '¹', "sup2" => '²', "sup3" => '³', "frac14" => '¼',
        "frac12" => '½', "frac34" => '¾', "not" => '¬', "shy" => '\u{ad}', "ordf" => 'ª', "ordm" => 'º',
        "larr" => '←', "rarr" => '→', "uarr" => '↑', "darr" => '↓', "harr" => '↔', "lArr" => '⇐',
        "rArr" => '⇒', "uArr" => '⇑', "dArr" => '⇓', "hArr" => '⇔', "le" => '≤', "ge" => '≥', "ne" => '≠',
        "minus" => '−', "asymp" => '≈', "equiv" => '≡', "infin" => '∞', "sum" => '∑', "prod" => '∏',
        "radic" => '√', "part" => '∂', "nabla" => '∇', "isin" => '∈', "notin" => '∉', "cap" => '∩',
        "cup" => '∪', "and" => '∧', "or" => '∨', "forall" => '∀', "exist" => '∃', "empty" => '∅',
        "prime" => '′', "Prime" => '″', "permil" => '‰', "dagger" => '†', "Dagger" => '‡', "loz" => '◊',
        "spades" => '♠', "clubs" => '♣', "hearts" => '♥', "diams" => '♦', "check" => '✓', "cross" => '✗',
        "star" => '☆', "starf" => '★', "thinsp" => '\u{2009}', "ensp" => '\u{2002}', "emsp" => '\u{2003}',
        "zwnj" => '\u{200c}', "zwj" => '\u{200d}', "lrm" => '\u{200e}', "rlm" => '\u{200f}',
        "alpha" => 'α', "beta" => 'β', "gamma" => 'γ', "delta" => 'δ', "epsilon" => 'ε', "zeta" => 'ζ',
        "eta" => 'η', "theta" => 'θ', "iota" => 'ι', "kappa" => 'κ', "lambda" => 'λ', "mu" => 'μ',
        "nu" => 'ν', "xi" => 'ξ', "omicron" => 'ο', "pi" => 'π', "rho" => 'ρ', "sigma" => 'σ', "tau" => 'τ',
        "upsilon" => 'υ', "phi" => 'φ', "chi" => 'χ', "psi" => 'ψ', "omega" => 'ω', "Delta" => 'Δ',
        "Sigma" => 'Σ', "Omega" => 'Ω', "Pi" => 'Π', "Theta" => 'Θ', "Lambda" => 'Λ', "Phi" => 'Φ',
        _ => return None,
    };
    Some(c)
}

/// Say once, per reason, that a page goes whole because its markup needs
/// the browser's repair — the author can fix the markup.
fn note_repaired(why: &str) {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if seen.lock().map(|mut s| s.insert(why.to_string())).unwrap_or(false) {
        eprintln!("crow live: this page is sent whole, not as changes: its markup has {} (the browser repairs it)", why);
    }
}

/// Elements whose end tag HTML lets a page leave out: the only ones a
/// browser closes for the page without repairing it.
const OMISSIBLE: &[&str] = &["p", "li", "dd", "dt", "option", "optgroup", "tr", "td", "th", "tbody", "thead", "tfoot", "caption", "colgroup", "rb", "rp", "rt", "rtc"];

/// Tags a browser reads its own way — ignored, renamed, or holding content
/// outside the page's children.
const READ_OWN_WAY: &[&str] = &["template", "image", "isindex", "plaintext", "frameset", "frame", "html", "head", "body", "math", "keygen"];

/// An element being built, and whether it is SVG.
struct Open {
    tag: String,
    attrs: HashMap<String, String>,
    children: Vec<CrowVNode>,
    foreign: bool,
}

/// Markup read into a tree the way a browser's parser builds it. Markup
/// written as it should be — every element closed by its own end tag, or
/// one HTML lets a page leave out — reads exactly as the browser reads it:
/// text runs whole, void elements childless, `<x/>` closing itself only in
/// a drawing, a script's text raw, a row given its `<tbody>`. Markup the
/// browser has to repair (misnested tags, a table's content outside its
/// cells, a stray end tag) is noted, with the first repair: a page that
/// needs one is sent whole, never as changes.
struct Builder {
    top: Vec<CrowVNode>,
    stack: Vec<Open>,
    repaired: Option<String>,
}

impl Builder {
    fn repair(&mut self, why: String) {
        if self.repaired.is_none() {
            self.repaired = Some(why);
        }
    }

    fn in_table_context(&self) -> bool {
        self.stack.last().map_or(false, |o| !o.foreign && matches!(o.tag.as_str(), "table" | "tbody" | "thead" | "tfoot" | "tr"))
    }

    /// Text lands on the node being built — beside the text before it, as
    /// one text; kept as written when it holds an entity, so the browser
    /// reads it.
    fn text(&mut self, source: &str) {
        if source.is_empty() {
            return;
        }
        if self.in_table_context() && !source.chars().all(|c| c.is_ascii_whitespace()) {
            self.repair(format!("text '{}' inside a table but outside its cells", source.trim()));
        }
        let list = self.current();
        match list.last_mut() {
            Some(last @ CrowVNode::Text(_)) => {
                if let CrowVNode::Text(prev) = last {
                    if source.contains('&') {
                        let merged = format!("{}{}", escape_text(prev), source);
                        *last = CrowVNode::Raw(merged);
                    } else {
                        prev.push_str(source);
                    }
                }
            }
            Some(CrowVNode::Raw(prev)) if !prev.starts_with("<!--") => prev.push_str(source),
            _ => list.push(if source.contains('&') { CrowVNode::Raw(source.to_string()) } else { CrowVNode::Text(source.to_string()) }),
        }
    }

    fn current(&mut self) -> &mut Vec<CrowVNode> {
        match self.stack.last_mut() {
            Some(o) => &mut o.children,
            None => &mut self.top,
        }
    }

    fn push(&mut self, node: CrowVNode) {
        self.current().push(node);
    }

    fn close(&mut self) {
        if let Some(o) = self.stack.pop() {
            self.push(CrowVNode::Element { tag: o.tag, attrs: o.attrs, children: o.children });
        }
    }

    /// Close elements until `len` remain, each without its end tag.
    fn close_implied(&mut self, len: usize) {
        while self.stack.len() > len {
            let open = self.stack.last().map(|o| (o.tag.clone(), o.foreign)).unwrap_or_default();
            if open.1 || !OMISSIBLE.contains(&open.0.as_str()) {
                self.repair(format!("<{}> is not closed by its own end tag", open.0));
            }
            self.close();
        }
    }

    /// Close the element at `at`, and those still open inside it.
    fn close_at(&mut self, at: usize) {
        self.close_implied(at + 1);
        self.close();
    }

    /// The nearest open `name` before a scope boundary (or `extra`).
    fn in_scope(&self, names: &[&str], extra: &[&str]) -> Option<usize> {
        for (i, o) in self.stack.iter().enumerate().rev() {
            if !o.foreign && names.contains(&o.tag.as_str()) {
                return Some(i);
            }
            let t = o.tag.to_ascii_lowercase();
            if SCOPE.contains(&t.as_str()) || extra.contains(&t.as_str()) {
                return None;
            }
        }
        None
    }

    fn in_foreign(&self) -> bool {
        self.stack.last().map_or(false, |o| o.foreign && !matches!(o.tag.to_ascii_lowercase().as_str(), "foreignobject" | "desc" | "title"))
    }

    fn open(&mut self, tag: String, attrs: HashMap<String, String>, foreign: bool) {
        self.stack.push(Open { tag, attrs, children: Vec::new(), foreign });
    }

    /// Open an element; returns the raw-text element it opened, if any, and
    /// whether the element drops a newline that opens it.
    fn start(&mut self, written: &str, attrs: HashMap<String, String>, self_closing: bool) -> (Option<String>, bool) {
        let lower = written.to_ascii_lowercase();
        if self.in_foreign() && BREAKS_OUT.contains(&lower.as_str()) {
            self.repair(format!("<{}> inside a drawing", lower));
            while self.in_foreign() {
                self.close();
            }
        }
        if self.in_foreign() || lower == "svg" {
            self.open(written.to_string(), attrs, true);
            if self_closing {
                self.close();
            }
            return (None, false);
        }
        let tag = lower;
        if READ_OWN_WAY.contains(&tag.as_str()) {
            self.repair(format!("<{}>, which the browser reads its own way", tag));
        }
        let table_part = matches!(tag.as_str(), "caption" | "colgroup" | "col" | "tbody" | "thead" | "tfoot" | "tr" | "td" | "th");
        if self.in_table_context() && !table_part && !matches!(tag.as_str(), "script" | "style") {
            self.repair(format!("<{}> inside a table but outside its cells", tag));
        }
        // a list's options hold text; the list holds options (browsers
        // differ on anything else)
        if self.stack.last().map_or(false, |o| o.tag == "option") && !matches!(tag.as_str(), "option" | "optgroup") {
            self.repair(format!("<{}> inside an option", tag));
        }
        if self.stack.last().map_or(false, |o| matches!(o.tag.as_str(), "select" | "optgroup")) && !matches!(tag.as_str(), "option" | "optgroup" | "hr") {
            self.repair(format!("<{}> inside a select", tag));
        }
        if CLOSES_P.contains(&tag.as_str()) {
            if let Some(at) = self.in_scope(&["p"], &["button"]) {
                self.close_at(at);
            }
        }
        match tag.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.stack.last().map_or(false, |o| matches!(o.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")) {
                    self.repair(format!("<{}> inside a heading", tag));
                    self.close();
                }
            }
            "li" | "dd" | "dt" => {
                let closes: &[&str] = if tag == "li" { &["li"] } else { &["dd", "dt"] };
                for i in (0..self.stack.len()).rev() {
                    let t = self.stack[i].tag.as_str();
                    if closes.contains(&t) {
                        self.close_at(i);
                        break;
                    }
                    if SPECIAL.contains(&t) && !matches!(t, "address" | "div" | "p") {
                        break;
                    }
                }
            }
            "option" | "optgroup" => {
                if self.stack.last().map_or(false, |o| o.tag == "option") {
                    self.close();
                }
                if tag == "optgroup" && self.stack.last().map_or(false, |o| o.tag == "optgroup") {
                    self.close();
                }
            }
            "a" | "button" | "nobr" | "form" => {
                if self.stack.iter().any(|o| o.tag == tag) {
                    self.repair(format!("<{}> inside another", tag));
                }
            }
            "tbody" | "thead" | "tfoot" | "caption" | "colgroup" => match self.stack.iter().rposition(|o| o.tag == "table") {
                Some(at) => self.close_implied(at + 1),
                None => self.repair(format!("<{}> outside a table", tag)),
            },
            "tr" => match self.stack.iter().rposition(|o| matches!(o.tag.as_str(), "table" | "tbody" | "thead" | "tfoot")) {
                Some(at) => {
                    self.close_implied(at + 1);
                    // a row written straight into a table: the browser gives it a body
                    if self.stack.last().map_or(false, |o| o.tag == "table") {
                        self.open("tbody".to_string(), HashMap::new(), false);
                    }
                }
                None => self.repair("<tr> outside a table".to_string()),
            },
            "td" | "th" => match self.stack.iter().rposition(|o| matches!(o.tag.as_str(), "table" | "tbody" | "thead" | "tfoot" | "tr")) {
                Some(at) if self.stack[at].tag == "tr" => self.close_implied(at + 1),
                Some(_) => self.repair(format!("<{}> outside a row", tag)),
                None => self.repair(format!("<{}> outside a table", tag)),
            },
            "col" => {
                if self.stack.last().map_or(true, |o| o.tag != "colgroup") {
                    self.repair("<col> outside a colgroup".to_string());
                }
            }
            _ => {}
        }
        if VOID.contains(&tag.as_str()) {
            self.open(tag, attrs, false);
            self.close();
            return (None, false);
        }
        let raw = RAW_TEXT.contains(&tag.as_str()).then(|| tag.clone());
        let drops = DROPS_NEWLINE.contains(&tag.as_str());
        self.open(tag, attrs, false);
        (raw, drops)
    }

    fn end(&mut self, written: &str) {
        let tag = written.to_ascii_lowercase();
        // in a drawing, the element of that name closes
        if self.stack.last().map_or(false, |o| o.foreign) {
            for i in (0..self.stack.len()).rev() {
                if !self.stack[i].foreign {
                    break;
                }
                if self.stack[i].tag.eq_ignore_ascii_case(&tag) {
                    self.close_at(i);
                    return;
                }
            }
        }
        // a void element's end tag is ignored, but a break's is a break
        if VOID.contains(&tag.as_str()) && tag != "br" {
            return;
        }
        let found = match tag.as_str() {
            "br" => None,
            "p" => self.in_scope(&["p"], &["button"]),
            "li" => self.in_scope(&["li"], &["ol", "ul"]),
            "table" | "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" | "caption" | "colgroup" => self
                .stack
                .iter()
                .rposition(|o| !o.foreign && (o.tag == tag || matches!(o.tag.as_str(), "table" | "html" | "template")))
                .filter(|at| self.stack[*at].tag == tag),
            t if SCOPED_END.contains(&t) => self.in_scope(&[t], &[]),
            _ => {
                let mut found = None;
                for i in (0..self.stack.len()).rev() {
                    if self.stack[i].tag.eq_ignore_ascii_case(&tag) {
                        found = Some(i);
                        break;
                    }
                    if SPECIAL.contains(&self.stack[i].tag.as_str()) {
                        break;
                    }
                }
                found
            }
        };
        match found {
            Some(at) => self.close_at(at),
            None => {
                self.repair(format!("</{}> with no <{}> open", tag, tag));
                // the browser makes a stray `</br>` a break and `</p>` an empty paragraph
                if tag == "br" || tag == "p" {
                    self.open(tag, HashMap::new(), false);
                    self.close();
                }
            }
        }
    }
}

/// Markup read into nodes the way a browser builds them for the page, so a
/// tree of these nodes has the page's shape, index for index. Text holding
/// an entity stays as written (a `Raw` the browser reads); comments too.
pub fn parse_markup(html: &str) -> Vec<CrowVNode> {
    read_markup(html).0
}

/// The nodes, and the first repair the browser makes to this markup, if it
/// makes one: then the nodes are this reader's best guess, not the page's
/// shape for certain.
pub fn read_markup(html: &str) -> (Vec<CrowVNode>, Option<String>) {
    let mut b = Builder { top: Vec::new(), stack: Vec::new(), repaired: None };
    let mut rest = html;
    // text runs to the next `<` that starts markup
    let starts_markup = |r: &str| {
        let mut cs = r.chars().skip(1);
        match cs.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '!' || c == '?' => true,
            Some('/') => cs.next().map_or(false, |c| c.is_ascii_alphabetic() || c == '>'),
            _ => false,
        }
    };
    while !rest.is_empty() {
        let mut at = 0;
        let mut found = None;
        while let Some(i) = rest[at..].find('<') {
            if starts_markup(&rest[at + i..]) {
                found = Some(at + i);
                break;
            }
            at += i + 1;
        }
        match found {
            Some(0) => {}
            Some(i) => {
                b.text(&rest[..i]);
                rest = &rest[i..];
            }
            None => {
                b.text(rest);
                break;
            }
        }
        if let Some(body) = rest.strip_prefix("<!--") {
            let end = body.find("-->").unwrap_or(body.len());
            let comment = &body[..end];
            if end == body.len() || comment.starts_with('>') || comment.starts_with("->") || comment.contains("--!") {
                b.repair("a comment the browser ends elsewhere".to_string());
            }
            b.push(CrowVNode::Raw(format!("<!--{}-->", comment)));
            rest = if end + 3 <= body.len() { &body[end + 3..] } else { "" };
            continue;
        }
        // the tag's end, skipping `>` inside quoted attribute values
        let mut quote: Option<char> = None;
        let mut end = rest.len();
        for (i, c) in rest.char_indices().skip(1) {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (None, '"') | (None, '\'') => quote = Some(c),
                (None, '>') => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        if end == rest.len() {
            b.repair("a tag that never ends".to_string());
        }
        let inside = &rest[1..end];
        rest = if end < rest.len() { &rest[end + 1..] } else { "" };
        if inside.starts_with('!') || inside.starts_with('?') {
            // a doctype is dropped; anything else of this form is read the
            // browser's own way
            if !inside.to_ascii_lowercase().starts_with("!doctype") {
                b.repair(format!("<{}>", inside));
                b.push(CrowVNode::Raw(format!("<!--{}-->", &inside[1..])));
            }
            continue;
        }
        if let Some(name) = inside.strip_prefix('/') {
            let name = name.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
            if !name.is_empty() {
                b.end(name);
            }
            continue;
        }
        let self_closing = inside.ends_with('/');
        let inside = inside.trim_end_matches('/');
        let name_end = inside.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inside.len());
        let written = &inside[..name_end];
        let foreign = b.in_foreign() || written.eq_ignore_ascii_case("svg");
        let mut attrs = HashMap::new();
        let mut a = &inside[name_end..];
        loop {
            a = a.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
            if a.is_empty() {
                break;
            }
            let key_end = a.find(|c: char| c == '=' || c.is_whitespace() || c == '/').unwrap_or(a.len()).max(1);
            let key = if foreign { a[..key_end].to_string() } else { a[..key_end].to_ascii_lowercase() };
            a = &a[key_end..];
            let mut value = String::new();
            let after = a.trim_start();
            if let Some(v) = after.strip_prefix('=') {
                let v = v.trim_start();
                let written_value = if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
                    let close_at = v[1..].find(q).map(|i| i + 1).unwrap_or(v.len());
                    a = if close_at < v.len() { &v[close_at + 1..] } else { "" };
                    &v[1..close_at]
                } else {
                    let stop = v.find(char::is_whitespace).unwrap_or(v.len());
                    a = &v[stop..];
                    &v[..stop]
                };
                value = unescape_markup(written_value);
                if let Some(why) = unread_reference(written_value) {
                    b.repair(why);
                }
            }
            // the first of a repeated attribute is the one the browser keeps
            attrs.entry(key).or_insert(value);
        }
        let (raw, drops_newline) = b.start(written, attrs, self_closing);
        if let Some(raw) = raw {
            // the content, as text, up to the element's end tag
            let lower = rest.to_ascii_lowercase();
            let close = format!("</{}", raw);
            let stop = lower.find(&close).unwrap_or(rest.len());
            let mut content = &rest[..stop];
            if drops_newline {
                content = content.strip_prefix('\n').unwrap_or(content);
            }
            if !content.is_empty() {
                if matches!(raw.as_str(), "textarea" | "title") {
                    b.text(content);
                } else {
                    b.push(CrowVNode::Text(content.to_string()));
                }
            }
            b.close();
            rest = &rest[stop..];
            rest = match rest.find('>') {
                Some(gt) => &rest[gt + 1..],
                None => "",
            };
        } else if drops_newline {
            rest = rest.strip_prefix('\n').unwrap_or(rest);
        }
    }
    while !b.stack.is_empty() {
        b.close();
    }
    (b.top, b.repaired)
}

/// A character reference in an attribute value that this reader does not
/// read as the browser would: one without its `;` that the browser still
/// reads, or one naming a character this reader does not know.
fn unread_reference(value: &str) -> Option<String> {
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        rest = &rest[at + 1..];
        let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '#').collect();
        if name.is_empty() {
            continue;
        }
        let terminated = rest[name.len()..].starts_with(';');
        let known = if let Some(num) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            u32::from_str_radix(num, 16).ok().and_then(char::from_u32).filter(|c| !matches!(*c as u32, 0 | 0x80..=0x9f)).is_some()
        } else if let Some(num) = name.strip_prefix('#') {
            num.parse::<u32>().ok().and_then(char::from_u32).filter(|c| !matches!(*c as u32, 0 | 0x80..=0x9f)).is_some()
        } else {
            named_entity(&name).is_some()
        };
        if name.starts_with('#') && (!terminated || !known) {
            return Some(format!("the reference &{} in an attribute", name));
        }
        if terminated && !known {
            return Some(format!("the reference &{}; in an attribute", name));
        }
        if !terminated && named_entity(&name).is_some() {
            return Some(format!("the reference &{} without its ';' in an attribute", name));
        }
    }
    None
}

impl CrowVNode {
    /// The page as a browser builds it from this node's markup: pre-built
    /// markup read into nodes, and every quirk of the reading applied — the
    /// tree a change is addressed against. Markup the browser would have to
    /// repair, or more than one node at the top, stays markup: that page is
    /// sent whole.
    pub fn as_built(self) -> CrowVNode {
        let html = self.to_html();
        let (mut nodes, repaired) = read_markup(&html);
        if let Some(why) = repaired {
            note_repaired(&why);
            return CrowVNode::Raw(html);
        }
        if nodes.len() == 1 {
            nodes.pop().unwrap_or(CrowVNode::Raw(html))
        } else {
            CrowVNode::Raw(html)
        }
    }

    /// What changed from `old` to `new`, as the page applies it: paths
    /// count children from the page's root element (`[0]`), and each change
    /// is a text, an element's attributes that changed, an element replaced,
    /// or a child added or taken away. A change names the tag of the element
    /// it touches (`tag`) or of the one holding the node it touches (`in`),
    /// so a page whose shape has drifted knows it.
    pub fn changes(old: &CrowVNode, new: &CrowVNode) -> Vec<serde_json::Value> {
        fn walk(old: &CrowVNode, new: &CrowVNode, path: &mut Vec<usize>, within: &str, out: &mut Vec<serde_json::Value>) {
            let place = |mut op: serde_json::Value| {
                if !within.is_empty() {
                    op["in"] = serde_json::json!(within);
                }
                op
            };
            match (old, new) {
                (CrowVNode::Text(a), CrowVNode::Text(b)) => {
                    if a != b {
                        out.push(place(serde_json::json!({ "op": "text", "path": path, "text": b })));
                    }
                }
                (CrowVNode::Element { tag: t1, attrs: a1, children: c1 }, CrowVNode::Element { tag: t2, attrs: a2, children: c2 }) if t1 == t2 => {
                    if a1 != a2 {
                        let set: serde_json::Map<String, serde_json::Value> = a2.iter()
                            .filter(|(k, v)| a1.get(*k) != Some(*v))
                            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                            .collect();
                        let remove: Vec<&String> = a1.keys().filter(|k| !a2.contains_key(*k)).collect();
                        out.push(serde_json::json!({ "op": "attrs", "path": path, "tag": t2, "set": set, "remove": remove }));
                    }
                    // children matched by id, the rest by position, as the
                    // page's morph matches them; each change's index is where
                    // the child is when the page applies it, in order
                    let key = |c: &CrowVNode| match c {
                        CrowVNode::Element { attrs, .. } => attrs.get("id").cloned(),
                        _ => None,
                    };
                    let old_at: HashMap<String, usize> = c1.iter().enumerate().filter_map(|(i, c)| key(c).map(|k| (k, i))).collect();
                    let new_at: HashMap<String, usize> = c2.iter().enumerate().filter_map(|(j, c)| key(c).map(|k| (k, j))).collect();
                    let (mut i, mut j, mut at) = (0, 0, 0);
                    let insert = |out: &mut Vec<serde_json::Value>, path: &Vec<usize>, at: usize, c: &CrowVNode| {
                        out.push(serde_json::json!({ "op": "insert", "path": path, "tag": t2, "index": at, "html": c.to_html() }));
                    };
                    let remove = |out: &mut Vec<serde_json::Value>, path: &Vec<usize>, at: usize| {
                        out.push(serde_json::json!({ "op": "remove", "path": path, "tag": t2, "index": at }));
                    };
                    while i < c1.len() && j < c2.len() {
                        let (ko, kn) = (key(&c1[i]), key(&c2[j]));
                        // still to come on the other side
                        let old_later = kn.as_ref().map_or(false, |k| old_at.get(k).map_or(false, |&x| x > i));
                        let new_later = ko.as_ref().map_or(false, |k| new_at.get(k).map_or(false, |&x| x > j));
                        if ko == kn {
                            path.push(at);
                            walk(&c1[i], &c2[j], path, t2, out);
                            path.pop();
                            i += 1;
                            j += 1;
                            at += 1;
                        } else if kn.is_some() && !old_later {
                            insert(out, path, at, &c2[j]);
                            j += 1;
                            at += 1;
                        } else if ko.is_some() && !new_later {
                            remove(out, path, at);
                            i += 1;
                        } else if kn.is_none() {
                            // an unkeyed child before a keyed one still wanted
                            insert(out, path, at, &c2[j]);
                            j += 1;
                            at += 1;
                        } else {
                            // the old child moves later (or is unkeyed): it goes,
                            // and a moved one comes back where it is wanted
                            remove(out, path, at);
                            i += 1;
                        }
                    }
                    for _ in i..c1.len() {
                        remove(out, path, at);
                    }
                    for c in &c2[j..] {
                        insert(out, path, at, c);
                        at += 1;
                    }
                }
                (CrowVNode::Raw(a), CrowVNode::Raw(b)) if a == b => {}
                _ => out.push(place(serde_json::json!({ "op": "replace", "path": path, "html": new.to_html() }))),
            }
        }
        let mut out = Vec::new();
        walk(old, new, &mut vec![0], "", &mut out);
        out
    }

    /// Apply changes to this page the way the browser does (`applyChange`
    /// in the live script) — how a test, or the runner that serves both
    /// modes, keeps the page a socket describes. An error is a change that
    /// does not fit the page: the browser asks for the whole page then.
    pub fn apply(&mut self, changes: &[serde_json::Value]) -> Result<(), String> {
        fn first(html: &str) -> Result<CrowVNode, String> {
            parse_markup(html).into_iter().next().ok_or_else(|| format!("no node in '{}'", html))
        }
        for c in changes {
            let path: Vec<usize> = c["path"].as_array().map(|p| p.iter().filter_map(|i| i.as_u64().map(|i| i as usize)).collect()).unwrap_or_default();
            if path.first() != Some(&0) {
                return Err(format!("a change outside the page: {}", c));
            }
            let mut node: &mut CrowVNode = self;
            let mut holder = String::new();
            for &i in &path[1..] {
                node = match node {
                    CrowVNode::Element { tag, children, .. } => {
                        holder = tag.clone();
                        children.get_mut(i).ok_or_else(|| format!("no child {} for {}", i, c))?
                    }
                    _ => return Err(format!("a path through text: {}", c)),
                };
            }
            if let Some(want) = c["in"].as_str() {
                if !holder.eq_ignore_ascii_case(want) {
                    return Err(format!("a node in <{}> where the change expects one in <{}>", holder, want));
                }
            }
            if let (Some(want), CrowVNode::Element { tag, .. }) = (c["tag"].as_str(), &*node) {
                if !tag.eq_ignore_ascii_case(want) {
                    return Err(format!("a <{}> where the change expects <{}>", tag, want));
                }
            }
            match (c["op"].as_str().unwrap_or(""), node) {
                ("text", CrowVNode::Text(t)) => *t = c["text"].as_str().unwrap_or("").to_string(),
                ("attrs", CrowVNode::Element { attrs, .. }) => {
                    for (k, v) in c["set"].as_object().into_iter().flatten() {
                        attrs.insert(k.clone(), v.as_str().unwrap_or("").to_string());
                    }
                    for k in c["remove"].as_array().into_iter().flatten() {
                        attrs.remove(k.as_str().unwrap_or(""));
                    }
                }
                ("replace", node) => *node = first(c["html"].as_str().unwrap_or(""))?,
                ("insert", CrowVNode::Element { children, .. }) => {
                    let at = (c["index"].as_u64().unwrap_or(0) as usize).min(children.len());
                    children.insert(at, first(c["html"].as_str().unwrap_or(""))?);
                }
                ("remove", CrowVNode::Element { children, .. }) => {
                    let at = c["index"].as_u64().unwrap_or(0) as usize;
                    if at >= children.len() {
                        return Err(format!("no child {} to take away: {}", at, c));
                    }
                    children.remove(at);
                }
                (op, _) => return Err(format!("'{}' does not fit the node at {:?}", op, path)),
            }
        }
        Ok(())
    }
}

/// A child that is a list contributes each item: rendered markup passes
/// through, anything else is text. The one rule for every list a view can
/// hold — strings, numbers, or the nodes a `for` over a view produced.
pub trait ChildList {
    fn into_nodes(&self) -> Vec<CrowVNode>;
}
impl ChildList for Vec<CrowVNode> {
    fn into_nodes(&self) -> Vec<CrowVNode> { self.clone() }
}
impl ChildList for Vec<String> {
    /// a list of strings is text, whatever the strings hold
    fn into_nodes(&self) -> Vec<CrowVNode> { self.iter().map(|s| CrowVNode::text(s)).collect() }
}
impl ChildList for Vec<i64> {
    fn into_nodes(&self) -> Vec<CrowVNode> { self.iter().map(|n| CrowVNode::text(&n.to_string())).collect() }
}
impl ChildList for Vec<f64> {
    fn into_nodes(&self) -> Vec<CrowVNode> { self.iter().map(|n| CrowVNode::text(&n.to_string())).collect() }
}
impl ChildList for Vec<bool> {
    fn into_nodes(&self) -> Vec<CrowVNode> { self.iter().map(|b| CrowVNode::text(&b.to_string())).collect() }
}
impl<T: ChildList + ?Sized> ChildList for &T {
    fn into_nodes(&self) -> Vec<CrowVNode> { (**self).into_nodes() }
}

/// The nodes a frame's column contributes as a child: each cell, as a
/// list's items do — its text as `log` writes it, a missing cell as a
/// failed value shows in a view (the interpreter had drawn it as 0, and
/// the compiled page the whole column as one text).
pub fn column_children(c: &crow_runtime::frame::CrowColumn) -> Vec<CrowVNode> {
    use crow_runtime::display::CrowDisplay;
    use crow_runtime::frame::Column;
    (0..c.len())
        .map(|i| {
            if !c.valid[i] {
                return CrowVNode::failed("missing");
            }
            CrowVNode::text(&match c.data.as_ref() {
                Column::Int(v) => v[i].crow_fmt(),
                Column::Float(v) => v[i].crow_fmt(),
                Column::Str(v) => v[i].to_string(),
                Column::Bool(v) => v[i].crow_fmt(),
                Column::DateTime(..) => c.data.fmt_cell(i, true),
            })
        })
        .collect()
}

/// The nodes a list child contributes.
pub fn children_of<T: ChildList + ?Sized>(list: &T) -> Vec<CrowVNode> {
    list.into_nodes()
}

/// A list of markup (views, elements, `doc`'s markup rendered to strings)
/// placed as markup — the one list whose strings are not text.
pub fn markup_children(list: &[String]) -> Vec<CrowVNode> {
    list.iter().map(|s| CrowVNode::Raw(s.clone())).collect()
}

impl CrowVNode {
    /// A boolean attribute (`disabled = starting`): present when true,
    /// absent when false — the browser reads presence, not the word.
    pub fn bool_attr(self, name: &str, on: bool) -> Self {
        if on { self.attr(name, "") } else { self }
    }
}

/// A number as an attribute value: whole numbers without a trailing `.0`.
fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{}", v) }
}

/// What a page's render places in its head (spec, LiveView / Pages): the
/// last `title` it places, and each `meta` with its attributes, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Head {
    pub title: Option<String>,
    pub metas: Vec<Vec<(String, String)>>,
}

impl Head {
    /// A page that places no title of its own is titled this.
    pub const UNTITLED: &'static str = "Crow App";

    /// The title a page shows.
    pub fn shown_title(&self) -> &str {
        self.title.as_deref().unwrap_or(Self::UNTITLED)
    }

    /// The head as a page's markup writes it: its title, and each meta
    /// marked as the render's (`data-crow-head`), which a later head
    /// replaces.
    pub fn to_html(&self) -> String {
        let mut out = format!("<title>{}</title>", html_escape(self.shown_title()));
        for attrs in &self.metas {
            out.push_str("\n<meta data-crow-head=\"\"");
            for (k, v) in attrs {
                out.push_str(&format!(" {}=\"{}\"", k, html_escape(v)));
            }
            out.push('>');
        }
        out
    }
}

impl CrowVNode {
    /// A render with its head taken out: the `title` and `meta` elements
    /// it places anywhere, pre-built markup included, which a served page
    /// carries in its head rather than its body.
    pub fn split_head(self) -> (CrowVNode, Head) {
        let mut head = Head::default();
        let body = strip_head(self, &mut head).unwrap_or_else(|| CrowVNode::text(""));
        (body, head)
    }
}

/// A node without the head elements under it, which land in `head`: `None`
/// for one that is itself a head element.
fn strip_head(node: CrowVNode, head: &mut Head) -> Option<CrowVNode> {
    match node {
        // a view the interpreter rendered is markup: read once, when it
        // holds a head element, and written again without it — unless the
        // browser would repair it, when the page is sent as it stands
        // (as_built) and the head elements, which the browser does not
        // draw, stay in it
        CrowVNode::Raw(html) if html.contains("<title") || html.contains("<meta") => {
            let (nodes, repaired) = read_markup(&html);
            let without: String = nodes.into_iter().filter_map(|n| strip_tree(n, head)).map(|n| n.to_html()).collect();
            Some(CrowVNode::Raw(if repaired.is_some() { html } else { without }))
        }
        other => strip_tree(other, head),
    }
}

/// `strip_head` over a tree; markup its reading kept whole stays as it is.
fn strip_tree(node: CrowVNode, head: &mut Head) -> Option<CrowVNode> {
    match node {
        CrowVNode::Element { tag, children, .. } if tag.eq_ignore_ascii_case("title") => {
            head.title = Some(text_of(&children));
            None
        }
        CrowVNode::Element { tag, attrs, .. } if tag.eq_ignore_ascii_case("meta") => {
            let mut attrs: Vec<(String, String)> = attrs.into_iter().collect();
            attrs.sort();
            head.metas.push(attrs);
            None
        }
        CrowVNode::Element { tag, attrs, children } => {
            Some(CrowVNode::Element { tag, attrs, children: children.into_iter().filter_map(|c| strip_head(c, head)).collect() })
        }
        other => Some(other),
    }
}

/// The text a node's children hold, their tags left out.
fn text_of(children: &[CrowVNode]) -> String {
    children
        .iter()
        .map(|c| match c {
            CrowVNode::Text(t) => t.clone(),
            CrowVNode::Element { children, .. } => text_of(children),
            // a title's text is read as it stands, as a browser reads it:
            // its entities decoded, no tags in it
            CrowVNode::Raw(html) => unescape_markup(html),
        })
        .collect()
}

/// A node shows as its markup, as the interpreter's view answers it: a
/// view called outside views — logged, handed to `doc.document` — is its
/// HTML (compiled, `log[card[…]]` had not built).
impl crow_runtime::display::CrowDisplay for CrowVNode {
    fn crow_repr(&self) -> String {
        self.to_html()
    }
}

/// Text placed in a page: `&`, `<` and `>` escaped, which is all text
/// needs — the one escape both modes write a view's text with (the
/// compiled page had escaped its quotes too, and the interpreter's had
/// not: crow check --self's views family).
pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An attribute's value placed in a page: text's escapes and its quotes.
pub fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Escape HTML special characters, as an attribute's value takes them.
fn html_escape(s: &str) -> String {
    escape_attr(s)
}

impl CrowComponent {
    /// Generate a self-contained JavaScript module that implements this component.
    /// This is the compilation target for `crow build --wasm` — the component's
    /// state machine and event handlers as a JS class.
    pub fn to_js_module(&self) -> String {
        let state_json = serde_json::to_string(&self.initial_state).unwrap_or_else(|_| "{}".to_string());
        let event_cases: String = self.events.iter().map(|ev| {
            format!("      case '{}': break; // handler injected by transpiler\n", ev)
        }).collect();

        format!(
            r#"// Auto-generated by Crow — do not edit
export class {} {{
  constructor() {{
    this.state = {};
    this._el = null;
  }}
  mount(el) {{
    this._el = typeof el === 'string' ? document.querySelector(el) : el;
    this.render();
    this._el.addEventListener('click', e => {{
      if (e.target.dataset.event) this.handle(e.target.dataset.event);
    }});
  }}
  handle(event) {{
    switch(event) {{
{}    }}
    this.render();
  }}
  render() {{
    if (this._el) this._el.innerHTML = this.view();
  }}
  view() {{
    return '';  // overridden by transpiler
  }}
}}"#,
            self.name, state_json, event_cases
        )
    }
}

#[cfg(test)]
mod change_tests {
    use super::*;

    /// A small deterministic source of choices for the generated pages.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }
    // markup that is loose on purpose: missing and stray end tags,
    // blocks in paragraphs, rows without bodies, drawings with page
    // markup inside, raw text, entities
    fn messy(r: &mut Rng, depth: u32) -> String {
        let bits = [
            "a", " ", "x &amp; y", "1 < 2", "&mdash;", "</span>", "</p>", "</br>", "<br>", "<hr>", "</div>", "<li>", "<p>",
            "<input value=\"v\">", "<!-- c -->", "<td>", "<tr>", "</tr>", "</td>", "<option>o", "<img src=\"i\">",
            "<script>if (a<b) go()</script>", "<pre>\n\nx</pre>", "<textarea>\nt &amp; u</textarea>", "<dd>d", "<dt>t",
            "<h1>h", "<h2>", "</h1>", "<a href=\"#\">l", "<button>b", "</li>", "<![CDATA[x]]>", "<?pi?>",
        ];
        let wraps = [
            "div", "p", "span", "ul", "ol", "li", "table", "tbody", "tr", "td", "b", "i", "svg", "g", "section",
            "select", "dl", "em", "h3", "button", "a", "label", "form", "pre", "nav", "thead", "th", "caption", "details",
        ];
        let mut out = String::new();
        for _ in 0..(1 + r.next(4)) {
            if depth > 0 && r.next(2) == 0 {
                let t = wraps[r.next(wraps.len() as u64) as usize];
                let inner = messy(r, depth - 1);
                let close = if r.next(4) == 0 { String::new() } else { format!("</{}>", t) };
                let self_close = t == "g" && r.next(3) == 0;
                if self_close {
                    out.push_str(&format!("<{}/>{}", t, inner));
                } else {
                    out.push_str(&format!("<{} class=\"c{}\">{}{}", t, r.next(3), inner, close));
                }
            } else {
                out.push_str(bits[r.next(bits.len() as u64) as usize]);
            }
        }
        out
    }
    // markup written as it should be, in each element's own content
    fn tidy(r: &mut Rng, depth: u32, ctx: &str) -> String {
        let text = ["a", " ", "x &amp; y", "1 < 2", "&mdash;&#39;", "q\"r", "tab\there"];
        let mut out = String::new();
        let pick = |r: &mut Rng, xs: &[&'static str]| xs[r.next(xs.len() as u64) as usize];
        let attr = |r: &mut Rng| match r.next(5) {
            0 => String::new(),
            1 => format!(" class=\"c{}\"", r.next(3)),
            4 => format!(" id=\"k{}\"", r.next(6)),
            2 => " data-crow-args=\"[&quot;A&amp;B&quot;]\" data-crow-click=\"go\"".to_string(),
            _ => " title=\"a &gt; b &#x41;\" hidden".to_string(),
        };
        for _ in 0..(1 + r.next(3)) {
            let piece = match ctx {
                "ul" | "ol" => format!("<li{}>{}</li>", attr(r), if depth > 0 { tidy(r, depth - 1, "flow") } else { "i".into() }),
                "table" => {
                    let rows = |r: &mut Rng, depth: u32| {
                        let mut rows = String::new();
                        for _ in 0..(1 + r.next(2)) {
                            rows.push_str("<tr>");
                            for _ in 0..(1 + r.next(3)) {
                                let cell = pick(r, &["td", "th"]);
                                rows.push_str(&format!("<{c}>{}</{c}>", if depth > 0 { tidy(r, depth - 1, "flow") } else { "1".into() }, c = cell));
                            }
                            rows.push_str("</tr>");
                        }
                        rows
                    };
                    match r.next(3) {
                        0 => format!("<thead>{}</thead><tbody>{}</tbody>", rows(r, 0), rows(r, depth)),
                        1 => rows(r, depth),
                        _ => format!("<tbody>{}</tbody> ", rows(r, depth)),
                    }
                }
                "svg" => match r.next(4) {
                    0 => format!("<rect x=\"{}\" width=\"2\"/>", r.next(9)),
                    1 => format!("<line class=\"head\" x1=\"{0}\" x2=\"{0}\"></line>", r.next(9)),
                    2 if depth > 0 => format!("<g transform=\"translate(1,2)\">{}</g>", tidy(r, depth - 1, "svg")),
                    _ => format!("<text x=\"1\">{}</text>", pick(r, &text)),
                },
                "select" => format!("<option value=\"{0}\"{1}>o{0}</option>", r.next(3), if r.next(2) == 0 { " selected" } else { "" }),
                "phrasing" => match r.next(8) {
                    0 | 1 => pick(r, &text).to_string(),
                    2 => "<br>".to_string(),
                    3 => format!("<input value=\"{}\" data-crow-input=\"f\">", pick(r, &["v", "a&amp;b"])),
                    4 => "<img src=\"i.png\" alt=\"\">".to_string(),
                    _ if depth > 0 => {
                        let t = pick(r, &["span", "b", "i", "em", "strong", "code", "label", "small"]);
                        format!("<{t}{}>{}</{t}>", attr(r), tidy(r, depth - 1, "phrasing"))
                    }
                    _ => pick(r, &text).to_string(),
                },
                _ => match r.next(16) {
                    0 | 1 => pick(r, &text).to_string(),
                    2 => "<!-- c -->".to_string(),
                    3 => "<hr>".to_string(),
                    4 => format!("<pre>{}x</pre>", pick(r, &["\n\n", "\n", ""])),
                    5 => "<textarea data-crow-input=\"t\">\nt &amp; u</textarea>".to_string(),
                    6 => format!("<select data-crow-change=\"s\">{}</select>", tidy(r, 0, "select")),
                    7 => format!("<p{}>{}</p>", attr(r), tidy(r, depth.saturating_sub(1), "phrasing")),
                    8 => format!("<button data-crow-click=\"b\">{}</button>", pick(r, &text)),
                    9 if depth > 0 => format!("<{t}>{}</{t}>", tidy(r, depth - 1, "ul"), t = pick(r, &["ul", "ol"])),
                    10 if depth > 0 => format!("<table{}>{}</table>", attr(r), tidy(r, depth - 1, "table")),
                    11 if depth > 0 => format!("<svg viewBox=\"0 0 9 9\" width=\"9\">{}</svg>", tidy(r, depth - 1, "svg")),
                    12 => format!("<h{0}>{1}</h{0}>", 1 + r.next(6), pick(r, &text)),
                    13 => "<script>if (a<b) go()</script>".to_string(),
                    _ if depth > 0 => {
                        let t = pick(r, &["div", "section", "nav", "main", "article", "aside", "header", "footer", "details", "span", "a"]);
                        let inner = if t == "span" || t == "a" { tidy(r, depth - 1, "phrasing") } else { tidy(r, depth - 1, "flow") };
                        format!("<{t}{}>{}</{t}>", attr(r), inner)
                    }
                    _ => pick(r, &text).to_string(),
                },
            };
            out.push_str(&piece);
        }
        out
    }

    fn shape(nodes: &[CrowVNode]) -> String {
        nodes.iter().map(|n| match n {
            CrowVNode::Text(t) => format!("'{}'", t),
            CrowVNode::Raw(r) => format!("raw'{}'", r),
            CrowVNode::Element { tag, children, .. } => format!("{}({})", tag, shape(children)),
        }).collect::<Vec<_>>().join(" ")
    }

    /// Markup reads the way a browser builds it: void elements take no
    /// children, `<x/>` closes itself in a drawing, a quoted `>` is part of
    /// a value, text keeps its place, and text holding an entity stays as
    /// written for the browser to read
    #[test]
    fn markup_reads_into_the_page_s_shape() {
        let nodes = parse_markup("<p title=\"a > b\">x &amp; y</p> <input value=\"1\"></input><svg><rect width=\"2\"/><line x1=\"3\"/></svg>");
        assert_eq!(shape(&nodes), "p(raw'x &amp; y') ' ' input() svg(rect() line())");
        match &nodes[0] {
            CrowVNode::Element { attrs, .. } => assert_eq!(attrs.get("title").map(|s| s.as_str()), Some("a > b")),
            other => panic!("{:?}", other),
        }
    }

    /// The quirks a browser applies to markup written as it should be are
    /// applied the same way — otherwise a change lands on the wrong node —
    /// and markup the browser has to repair is known to need it
    #[test]
    fn the_browser_s_quirks_are_the_tree_s() {
        let exact = [
            // a block closes the paragraph, which may leave out its end tag
            ("<p>a<div>b</div>", "p('a') div('b')"),
            // a row outside a body is given one
            ("<table><tr><td>1</td></tr></table>", "table(tbody(tr(td('1'))))"),
            // an item closes the one before it
            ("<ul><li>a<li>b</ul>", "ul(li('a') li('b'))"),
            // `<div/>` opens a div; in a drawing `<g/>` is closed
            ("<div/>x</div><svg><g/>y</svg>", "div('x') svg(g() 'y')"),
            // a script's text is read as it stands
            ("<script>if (a<b) go()</script>", "script('if (a<b) go()')"),
            // `<` that starts no tag is text
            ("a < b", "'a < b'"),
            // the newline that opens a pre is dropped
            ("<pre>\n\nx</pre>", "pre('\nx')"),
            // a comment is a node; a doctype is not
            ("<!doctype html><!-- c -->x", "raw'<!-- c -->' 'x'"),
            // a repeated attribute keeps its first value; a void end tag is nothing
            ("<i a=\"1\" a=\"2\"></i><input></input>", "i() input()"),
        ];
        for (html, want) in exact {
            let (nodes, repaired) = read_markup(html);
            assert_eq!(repaired, None, "reading {:?}", html);
            assert_eq!(shape(&nodes), want, "reading {:?}", html);
        }
        let repaired = [
            ("<p>a<div>b</div>c</p>", "</p> with no <p> open"),
            ("<br></br>", "</br> with no <br> open"),
            ("<table>x<tr><td>1</td></tr></table>", "text 'x' inside a table but outside its cells"),
            ("a</span>b", "</span> with no <span> open"),
            ("<span><div>a</span>b</div>", "</span> with no <span> open"),
            ("<p><b>x<div>y</div>", "<b> is not closed by its own end tag"),
            ("<div><span>x</div>", "<span> is not closed by its own end tag"),
            ("<svg><circle/><div>d</div></svg>", "<div> inside a drawing"),
            ("<a href=\"#\"><a>x</a></a>", "<a> inside another"),
            ("<td>x</td>", "<td> outside a table"),
            ("<select><div>x</div></select>", "<div> inside a select"),
            ("<i t=\"&amp\"></i>", "the reference &amp without its ';' in an attribute"),
        ];
        for (html, why) in repaired {
            assert_eq!(read_markup(html).1.as_deref(), Some(why), "reading {:?}", html);
        }
        match &parse_markup("<i t=\"&mdash;&#39;&#x41;&bogus;\"></i>")[0] {
            CrowVNode::Element { attrs, .. } => assert_eq!(attrs["t"], "—'A&bogus;"),
            other => panic!("{:?}", other),
        }
        // a page the browser repairs stays markup, sent whole
        assert!(matches!(CrowVNode::raw("<div><p><b>x<div>y</div></div>").as_built(), CrowVNode::Raw(_)));
    }

    /// The page written out and read back is the same page: the writer
    /// doubles a pre's opening newline, keeps a script's text raw, and
    /// writes no end tag for a void element
    #[test]
    fn a_page_reads_back_as_itself() {
        for html in [
            "<div><pre>\n\nx</pre><script>a<b</script><br><input value=\"q\"><p>x &amp; y</p></div>",
            "<div><table><tr><td>1</td></tr></table><svg viewBox=\"0 0 2 2\"><path d=\"M0 0\"/></svg></div>",
            "<div>a<!-- c -->b<ul><li>1<li>2</ul></div>",
        ] {
            let once = CrowVNode::raw(html).as_built();
            let twice = once.clone().as_built();
            assert_eq!(once.to_html(), twice.to_html(), "reading {:?}", html);
            assert_eq!(shape(&[once]), shape(&[twice]));
        }
        assert_eq!(CrowVNode::el("br", HashMap::new(), vec![]).to_html(), "<br>");
    }

    /// A table's pre-built rows become rows, and a moved mark on a chart is
    /// one change of its attributes, not the drawing again
    #[test]
    fn a_moved_mark_is_one_attribute_change() {
        let page = |x: &str| CrowVNode::div(vec![
            CrowVNode::p(vec![CrowVNode::text("09:30")]),
            CrowVNode::raw(&format!("<svg><rect x=\"0\" width=\"5\"/><line class=\"head\" x1=\"{x}\" x2=\"{x}\"/></svg>", x = x)),
        ]).as_built();
        let before = page("10");
        let after = page("11");
        let changes = CrowVNode::changes(&before, &after);
        assert_eq!(changes.len(), 1, "{:?}", changes);
        assert_eq!(changes[0]["op"], "attrs");
        assert_eq!(changes[0]["path"], serde_json::json!([0, 1, 1]));
        assert_eq!(changes[0]["tag"], "line");
        assert_eq!(changes[0]["set"]["x1"], "11");
        let table = CrowVNode::table(vec![CrowVNode::raw("<thead><tr><th>a</th></tr></thead><tbody><tr><td>1</td></tr></tbody>")]).as_built();
        match table {
            CrowVNode::Element { children, .. } => assert_eq!(children.len(), 2, "thead and tbody, as the browser has them"),
            other => panic!("{:?}", other),
        }
    }

    /// Rows added and taken away, and a text that changed
    #[test]
    fn rows_come_and_go_and_text_changes() {
        let list = |items: &[&str]| CrowVNode::div(items.iter().map(|t| CrowVNode::p(vec![CrowVNode::text(t)])).collect());
        let c = CrowVNode::changes(&list(&["a", "b"]), &list(&["a", "c", "d"]));
        let ops: Vec<&str> = c.iter().map(|x| x["op"].as_str().unwrap_or("")).collect();
        assert_eq!(ops, vec!["text", "insert"], "{:?}", c);
        let c = CrowVNode::changes(&list(&["a", "b", "c"]), &list(&["a"]));
        assert_eq!(c.iter().filter(|x| x["op"] == "remove").count(), 2, "{:?}", c);
        // rows with ids are matched by id: one added at the top, one taken
        // from the middle, one moved
        let rows = |ids: &[&str]| CrowVNode::tag("ul", ids.iter().map(|k| CrowVNode::tag("li", vec![CrowVNode::text(k)]).attr("id", k)).collect());
        let c = CrowVNode::changes(&rows(&["a", "b", "c", "d"]), &rows(&["z", "a", "b", "c", "d"]));
        assert_eq!(c, vec![serde_json::json!({ "op": "insert", "path": [0], "tag": "ul", "index": 0, "html": "<li id=\"z\">z</li>" })]);
        let c = CrowVNode::changes(&rows(&["a", "b", "c", "d"]), &rows(&["a", "c", "d"]));
        assert_eq!(c, vec![serde_json::json!({ "op": "remove", "path": [0], "tag": "ul", "index": 1 })]);
        for (from, to) in [(vec!["a", "b", "c"], vec!["c", "a", "b"]), (vec!["a", "b", "c"], vec!["b", "c", "a"]), (vec!["a", "b"], vec!["b", "x", "a"])] {
            let (old, new) = (rows(&from), rows(&to));
            let mut page = old.clone();
            page.apply(&CrowVNode::changes(&old, &new)).expect("the changes fit");
            assert_eq!(page.to_html(), new.to_html(), "{:?} to {:?}", from, to);
        }
    }

    /// The browser itself reads each piece of markup into the tree
    /// `parse_markup` builds — checked against Chrome, when there is one:
    /// `cargo test -p crow_web the_browser_agrees -- --ignored` (CHROME
    /// names another binary, CROW_BROWSER_CASES a longer run). Markup
    /// written as it should be must read without repair and as the browser
    /// reads it; loose markup must too, unless the reader says it is repaired.
    #[test]
    #[ignore]
    fn the_browser_agrees() {
        let chrome = std::env::var("CHROME").unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string());
        if !std::path::Path::new(&chrome).exists() {
            eprintln!("no browser at {}", chrome);
            return;
        }
        fn browser_shape(nodes: &[CrowVNode]) -> String {
            nodes.iter().map(|n| match n {
                CrowVNode::Text(t) => format!("'{}'", t),
                CrowVNode::Raw(r) if r.starts_with("<!--") => format!("#{}", &r[4..r.len() - 3]),
                CrowVNode::Raw(r) => format!("'{}'", unescape_markup(r)),
                CrowVNode::Element { tag, children, .. } => format!("{}({})", tag.to_ascii_lowercase(), browser_shape(children)),
            }).collect::<Vec<_>>().join(" ")
        }
        let mut r = Rng(11);
        let mut cases: Vec<serde_json::Value> = Vec::new();
        let mut sure_messy = 0;
        // CROW_BROWSER_CASES asks for a longer run
        let total: usize = std::env::var("CROW_BROWSER_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(6000);
        for i in 0..total {
            let (html, tidy_one) = if i % 2 == 0 { (messy(&mut r, 4), false) } else { (tidy(&mut r, 4, "flow"), true) };
            let (nodes, repaired) = read_markup(&html);
            if tidy_one {
                assert!(repaired.is_none(), "markup written as it should be needs no repair: {:?}\n{}", repaired, html);
            } else if repaired.is_none() {
                sure_messy += 1;
            } else {
                continue;
            }
            cases.push(serde_json::json!({ "html": html, "want": browser_shape(&nodes) }));
        }
        eprintln!("checking {} pieces of markup, {} of them loose", cases.len(), sure_messy);
        let count = cases.len();
        let data = serde_json::Value::Array(cases).to_string().replace('<', "\\u003c");
        let page = format!(r#"<!DOCTYPE html><html><body><pre id="out"></pre>
<script type="application/json" id="cases">{data}</script>
<script>
window.onerror = function(m) {{ document.getElementById('out').textContent = JSON.stringify({{error: String(m)}}); }};
function shape(nodes) {{
  return Array.prototype.map.call(nodes, function(n) {{
    if (n.nodeType === 3) return "'" + n.nodeValue + "'";
    if (n.nodeType === 8) return '#' + n.nodeValue;
    if (n.nodeType !== 1) return '?' + n.nodeType;
    return n.localName.toLowerCase() + '(' + shape(n.childNodes) + ')';
  }}).join(' ');
}}
var cases = JSON.parse(document.getElementById('cases').textContent), bad = [];
cases.forEach(function(c, i) {{
  var d = document.createElement('div');
  d.innerHTML = c.html;
  var got = shape(d.childNodes);
  if (got !== c.want) bad.push({{html: c.html, browser: got, crow: c.want}});
}});
document.getElementById('out').textContent = JSON.stringify({{checked: cases.length, bad: bad.slice(0, 8), badCount: bad.length}});
</script></body></html>"#, data = data);
        let dir = std::env::temp_dir().join(format!("crow_browser_check_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("check.html");
        std::fs::write(&file, page).expect("the check page is written");
        // the browser prints the page and stays up: read to the page's end
        let mut child = std::process::Command::new(&chrome)
            .args(["--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check"])
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .arg("--dump-dom")
            .arg(format!("file://{}", file.display()))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the browser runs");
        let mut stdout = child.stdout.take().expect("piped");
        let reader = std::thread::spawn(move || {
            use std::io::Read;
            let mut dom = Vec::new();
            let mut buf = [0u8; 65536];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                dom.extend_from_slice(&buf[..n]);
                if dom.ends_with(b"</html>\n") || dom.ends_with(b"</html>") {
                    break;
                }
            }
            dom
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !reader.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        let dom = String::from_utf8_lossy(&reader.join().unwrap_or_default()).to_string();
        let text = dom.split("<pre id=\"out\">").nth(1).and_then(|t| t.split("</pre>").next()).unwrap_or("");
        let report: serde_json::Value = serde_json::from_str(&unescape_markup(text)).unwrap_or_else(|e| panic!("no report ({}): {}", e, dom));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(report["checked"], count, "{}", report);
        assert_eq!(report["badCount"], 0, "the browser reads these differently:\n{}", serde_json::to_string_pretty(&report["bad"]).unwrap_or_default());
    }

    /// For any two pages, the changes from one applied to it give the other
    /// — checked over many pages that differ in text, attributes, children,
    /// tags and entities
    #[test]
    fn the_changes_turn_one_page_into_the_other() {
        let mut r = Rng(7);
        for round in 0..2000 {
            let a = CrowVNode::raw(&format!("<main>{}</main>", tidy(&mut r, 3, "flow"))).as_built();
            let b = CrowVNode::raw(&format!("<main>{}</main>", tidy(&mut r, 3, "flow"))).as_built();
            assert!(matches!((&a, &b), (CrowVNode::Element { .. }, CrowVNode::Element { .. })), "well-formed pages read as trees");
            let changes = CrowVNode::changes(&a, &b);
            let mut applied = a.clone();
            applied.apply(&changes).unwrap_or_else(|e| panic!("round {}: {}\n{}\n{}", round, e, a.to_html(), b.to_html()));
            assert_eq!(applied.to_html(), b.to_html(), "round {}", round);
            // and the page the browser reads from the result is the same
            assert_eq!(applied.clone().as_built().to_html(), b.to_html(), "round {}", round);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A render's head is taken out of its body (spec, LiveView / Pages):
    /// the last title it places, each meta in order, from a built tree and
    /// from the markup the interpreter renders; markup the browser would
    /// repair is sent as it stands, its head still read.
    #[test]
    fn a_render_s_head_is_taken_out_of_its_body() {
        let meta = |name: &str, content: &str| {
            CrowVNode::el("meta", [("name".to_string(), name.to_string()), ("content".to_string(), content.to_string())].into_iter().collect(), vec![])
        };
        let tree = CrowVNode::div(vec![
            CrowVNode::tag("title", vec![CrowVNode::text("Board")]),
            CrowVNode::p(vec![CrowVNode::text("hi")]),
            CrowVNode::div(vec![CrowVNode::tag("title", vec![CrowVNode::text("Card 7")])]),
            meta("description", "a card"),
        ]);
        let (body, head) = tree.split_head();
        assert_eq!(head.title.as_deref(), Some("Card 7"), "the last title placed");
        assert_eq!(head.metas, vec![vec![("content".to_string(), "a card".to_string()), ("name".to_string(), "description".to_string())]]);
        assert_eq!(body.to_html(), "<div><p>hi</p><div></div></div>");
        let raw = CrowVNode::raw("<div><title>A &amp; B</title><p>hi</p><meta name=\"description\" content=\"y\"></div>");
        let (body, head) = raw.split_head();
        assert_eq!(head.title.as_deref(), Some("A & B"));
        assert_eq!(head.metas.len(), 1);
        assert_eq!(body.to_html(), "<div><p>hi</p></div>");
        let repaired = CrowVNode::raw("<p><div>x</div><title>T</title></p>");
        let (body, head) = repaired.split_head();
        assert_eq!(head.title.as_deref(), Some("T"));
        assert!(body.to_html().contains("<title>T</title>"), "{}", body.to_html());
        let (body, head) = CrowVNode::p(vec![CrowVNode::text("x")]).split_head();
        assert_eq!(head, Head::default());
        assert_eq!(head.shown_title(), "Crow App");
        assert_eq!(body.to_html(), "<p>x</p>");
    }

    #[test]
    fn test_component_builder() {
        let comp = CrowComponent::new("counter")
            .state("count", serde_json::json!(0))
            .on("increment");
        assert_eq!(comp.name, "counter");
        assert_eq!(comp.initial_state["count"], 0);
        assert_eq!(comp.events, vec!["increment"]);
    }

    #[test]
    fn test_vnode_text() {
        let node = CrowVNode::text("hello");
        assert_eq!(node.to_html(), "hello");
    }

    #[test]
    fn test_vnode_element() {
        let node = CrowVNode::el(
            "div",
            HashMap::from([("class".to_string(), "main".to_string())]),
            vec![CrowVNode::text("hi")],
        );
        let html = node.to_html();
        assert!(html.contains("<div"));
        assert!(html.contains("class=\"main\""));
        assert!(html.contains("hi"));
        assert!(html.contains("</div>"));
    }

    #[test]
    fn test_vnode_nested() {
        let node = CrowVNode::el(
            "ul",
            HashMap::new(),
            vec![
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("a")]),
                CrowVNode::el("li", HashMap::new(), vec![CrowVNode::text("b")]),
            ],
        );
        let html = node.to_html();
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>a</li>"));
        assert!(html.contains("<li>b</li>"));
    }

    #[test]
    fn test_js_module_generation() {
        let comp = CrowComponent::new("counter")
            .state("count", serde_json::json!(0))
            .on("inc")
            .on("dec");
        let js = comp.to_js_module();
        assert!(js.contains("export class counter"));
        assert!(js.contains("case 'inc':"));
        assert!(js.contains("case 'dec':"));
        assert!(js.contains("this.state"));
    }

    #[test]
    fn test_html_escape() {
        let node = CrowVNode::text("<script>alert('xss')</script>");
        let html = node.to_html();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_convenience_div() {
        let node = CrowVNode::div(vec![CrowVNode::text("hi")]);
        let html = node.to_html();
        assert_eq!(html, "<div>hi</div>");
    }

    #[test]
    fn test_convenience_button_with_event() {
        let node = CrowVNode::button(vec![CrowVNode::text("+")]).on_click("inc");
        let html = node.to_html();
        assert!(html.contains("<button"));
        assert!(html.contains("data-crow-click=\"inc\""));
        assert!(html.contains("+"));
    }

    #[test]
    fn test_attr_chaining() {
        let node = CrowVNode::input()
            .attr("type", "text")
            .attr("name", "username")
            .on_input("typing");
        let html = node.to_html();
        assert!(html.contains("type=\"text\""));
        assert!(html.contains("name=\"username\""));
        assert!(html.contains("data-crow-input=\"typing\""));
    }

    #[test]
    fn test_tag_constructor() {
        let node = CrowVNode::tag("article", vec![CrowVNode::text("content")]);
        assert_eq!(node.to_html(), "<article>content</article>");
    }

    #[test]
    fn test_counter_vnode_tree() {
        let tree = CrowVNode::div(vec![
            CrowVNode::h1(vec![CrowVNode::text("Count: 0")]),
            CrowVNode::button(vec![CrowVNode::text("+")]).on_click("inc"),
            CrowVNode::button(vec![CrowVNode::text("-")]).on_click("dec"),
        ]);
        let html = tree.to_html();
        assert!(html.contains("<div>"));
        assert!(html.contains("<h1>Count: 0</h1>"));
        assert!(html.contains("data-crow-click=\"inc\""));
        assert!(html.contains("data-crow-click=\"dec\""));
    }

    #[test]
    fn test_table_from_data() {
        let headers = vec!["Name".to_string(), "Age".to_string()];
        let rows = vec![
            vec!["Alice".to_string(), "30".to_string()],
            vec!["Bob".to_string(), "25".to_string()],
        ];
        let table = CrowVNode::table_from_data(&headers, &rows);
        let html = table.to_html();
        assert!(html.contains("<table>"));
        assert!(html.contains("<thead>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("<th>Name</th>"));
        assert!(html.contains("<th>Age</th>"));
        assert!(html.contains("<td>Alice</td>"));
        assert!(html.contains("<td>30</td>"));
        assert!(html.contains("<td>Bob</td>"));
        assert!(html.contains("<td>25</td>"));
    }

    #[test]
    fn test_table_from_data_empty() {
        let headers = vec!["Col".to_string()];
        let rows: Vec<Vec<String>> = vec![];
        let table = CrowVNode::table_from_data(&headers, &rows);
        let html = table.to_html();
        assert!(html.contains("<th>Col</th>"));
        assert!(html.contains("<tbody></tbody>"));
    }

    #[test]
    fn test_field() {
        let field = CrowVNode::field("Min Vol", "min_vol", "100");
        let html = field.to_html();
        assert!(html.contains("class=\"crow-field\""));
        assert!(html.contains("<label>Min Vol</label>"));
        assert!(html.contains("data-crow-input=\"__field:min_vol\""));
        assert!(html.contains("value=\"100\""));
        assert!(html.contains("type=\"text\""));
    }

    #[test]
    fn test_select_input() {
        let options = vec!["all".to_string(), "long".to_string(), "short".to_string()];
        let sel = CrowVNode::select_input(&options, "pb_filter", "long");
        let html = sel.to_html();
        assert!(html.contains("class=\"crow-select\""));
        assert!(html.contains("data-crow-change=\"__select:pb_filter\""));
        assert!(html.contains("<option"));
        assert!(html.contains("selected=\"selected\""));
        // The selected option should be "long"
        assert!(html.contains(">long</option>"));
        assert!(html.contains(">all</option>"));
        assert!(html.contains(">short</option>"));
    }
}

