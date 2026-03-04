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

    /// Render to HTML string.
    pub fn to_html(&self) -> String {
        match self {
            CrowVNode::Text(s) => s.clone(),
            CrowVNode::Element {
                tag,
                attrs,
                children,
            } => {
                let attr_str: String = attrs
                    .iter()
                    .map(|(k, v)| format!(" {}=\"{}\"", k, v))
                    .collect();
                let children_html: String = children.iter().map(|c| c.to_html()).collect();
                format!("<{}{}>{}</{}>", tag, attr_str, children_html, tag)
            }
        }
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
}

