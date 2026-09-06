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

/// A patch operation produced by diffing two VNode trees.
#[derive(Debug, Clone, serde::Serialize)]
pub enum VPatch {
    /// Replace the entire node at the given path.
    Replace { path: Vec<usize>, node: CrowVNode },
    /// Update attributes on the node at the given path.
    SetAttrs { path: Vec<usize>, attrs: HashMap<String, String> },
    /// Insert a child at index in the node at the given path.
    InsertChild { path: Vec<usize>, index: usize, node: CrowVNode },
    /// Remove the child at index in the node at the given path.
    RemoveChild { path: Vec<usize>, index: usize },
    /// Update text content at the given path.
    SetText { path: Vec<usize>, text: String },
}

impl CrowVNode {
    /// Create a text node.
    pub fn text(s: &str) -> Self {
        CrowVNode::Text(s.to_string())
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

    /// Add a click event handler name. Sets `data-crow-click` attribute.
    pub fn on_click(self, event_name: &str) -> Self {
        self.attr("data-crow-click", event_name)
    }

    /// A click handler with frozen arguments (F-11): the handler name plus
    /// a JSON array of values captured at render, delivered back with the
    /// event. `click = set_speed[10.0]` lowers to this.
    pub fn on_click_args(self, event_name: &str, args_json: &str) -> Self {
        self.attr("data-crow-click", event_name)
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
        let control = if lines > 0 {
            CrowVNode::textarea(vec![CrowVNode::text(current_value)])
                .attr("rows", &lines.to_string())
                .attr("data-crow-input", &bound)
        } else {
            CrowVNode::input()
                .attr("type", if kind.is_empty() { "text" } else { kind })
                .attr("value", current_value)
                .attr("data-crow-input", &bound)
        };
        CrowVNode::div(vec![CrowVNode::label(vec![CrowVNode::text(label)]), control]).attr("class", "crow-field")
    }

    /// Render to HTML string.
    pub fn to_html(&self) -> String {
        match self {
            CrowVNode::Text(s) => html_escape(s),
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
                let children_html: String = children.iter().map(|c| c.to_html()).collect();
                format!("<{}{}>{}</{}>", tag, attr_str, children_html, tag)
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
                .attr("data-crow-input", &format!("__field:{}", key)),
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
                .attr("data-crow-input", &format!("__field:{}", key)),
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

    /// Diff two VNode trees. Returns a list of patches to transform `old` into `new`.
    pub fn diff(old: &CrowVNode, new: &CrowVNode) -> Vec<VPatch> {
        let mut patches = Vec::new();
        Self::diff_recursive(old, new, &mut vec![], &mut patches);
        patches
    }

    fn diff_recursive(old: &CrowVNode, new: &CrowVNode, path: &mut Vec<usize>, patches: &mut Vec<VPatch>) {
        match (old, new) {
            (CrowVNode::Text(a), CrowVNode::Text(b)) => {
                if a != b {
                    patches.push(VPatch::SetText { path: path.clone(), text: b.clone() });
                }
            }
            (CrowVNode::Raw(a), CrowVNode::Raw(b)) => {
                if a != b {
                    patches.push(VPatch::Replace { path: path.clone(), node: new.clone() });
                }
            }
            (CrowVNode::Element { tag: t1, attrs: a1, children: c1 },
             CrowVNode::Element { tag: t2, attrs: a2, children: c2 }) => {
                if t1 != t2 {
                    patches.push(VPatch::Replace { path: path.clone(), node: new.clone() });
                    return;
                }
                if a1 != a2 {
                    patches.push(VPatch::SetAttrs { path: path.clone(), attrs: a2.clone() });
                }
                let min_len = c1.len().min(c2.len());
                for i in 0..min_len {
                    path.push(i);
                    Self::diff_recursive(&c1[i], &c2[i], path, patches);
                    path.pop();
                }
                // Extra children in new → insert
                for i in min_len..c2.len() {
                    patches.push(VPatch::InsertChild { path: path.clone(), index: i, node: c2[i].clone() });
                }
                // Extra children in old → remove (in reverse to keep indices stable)
                for i in (min_len..c1.len()).rev() {
                    patches.push(VPatch::RemoveChild { path: path.clone(), index: i });
                }
            }
            _ => {
                // Type mismatch (Text vs Element) → full replace
                patches.push(VPatch::Replace { path: path.clone(), node: new.clone() });
            }
        }
    }
}

/// A number as an attribute value: whole numbers without a trailing `.0`.
fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{}", v) }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
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
mod tests {
    use super::*;

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
    fn test_diff_identical_trees() {
        let a = CrowVNode::el("div", HashMap::new(), vec![CrowVNode::text("hi")]);
        let patches = CrowVNode::diff(&a, &a);
        assert!(patches.is_empty());
    }

    #[test]
    fn test_diff_text_change() {
        let a = CrowVNode::text("old");
        let b = CrowVNode::text("new");
        let patches = CrowVNode::diff(&a, &b);
        assert_eq!(patches.len(), 1);
        match &patches[0] {
            VPatch::SetText { text, .. } => assert_eq!(text, "new"),
            _ => panic!("expected SetText"),
        }
    }

    #[test]
    fn test_diff_tag_change() {
        let a = CrowVNode::el("div", HashMap::new(), vec![]);
        let b = CrowVNode::el("span", HashMap::new(), vec![]);
        let patches = CrowVNode::diff(&a, &b);
        assert_eq!(patches.len(), 1);
        assert!(matches!(&patches[0], VPatch::Replace { .. }));
    }

    #[test]
    fn test_diff_attr_change() {
        let a = CrowVNode::el("div", HashMap::from([("id".into(), "a".into())]), vec![]);
        let b = CrowVNode::el("div", HashMap::from([("id".into(), "b".into())]), vec![]);
        let patches = CrowVNode::diff(&a, &b);
        assert_eq!(patches.len(), 1);
        match &patches[0] {
            VPatch::SetAttrs { attrs, .. } => assert_eq!(attrs["id"], "b"),
            _ => panic!("expected SetAttrs"),
        }
    }

    #[test]
    fn test_diff_child_added() {
        let a = CrowVNode::el("ul", HashMap::new(), vec![CrowVNode::text("a")]);
        let b = CrowVNode::el("ul", HashMap::new(), vec![CrowVNode::text("a"), CrowVNode::text("b")]);
        let patches = CrowVNode::diff(&a, &b);
        assert!(patches.iter().any(|p| matches!(p, VPatch::InsertChild { index: 1, .. })));
    }

    #[test]
    fn test_diff_child_removed() {
        let a = CrowVNode::el("ul", HashMap::new(), vec![CrowVNode::text("a"), CrowVNode::text("b")]);
        let b = CrowVNode::el("ul", HashMap::new(), vec![CrowVNode::text("a")]);
        let patches = CrowVNode::diff(&a, &b);
        assert!(patches.iter().any(|p| matches!(p, VPatch::RemoveChild { index: 1, .. })));
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

