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
    pub fn button(children: Vec<CrowVNode>) -> Self { Self::tag("button", children) }
    pub fn ul(children: Vec<CrowVNode>) -> Self { Self::tag("ul", children) }
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

    /// Render to HTML string.
    pub fn to_html(&self) -> String {
        match self {
            CrowVNode::Text(s) => html_escape(s),
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
}

