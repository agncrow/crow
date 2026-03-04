//! `web.live` module — LiveView: server-rendered HTML with real-time updates.
//!
//! ```text
//! use web
//! use web.live
//!
//! app = web.new[]
//! live[app, '/counter', fn[state]
//!   state.count = state.count + 1
//!   '<div>Count: {state.count}</div>'
//! ]
//! ```

use std::collections::HashMap;

/// Server-side component state.
#[derive(Debug, Clone)]
pub struct CrowLiveState {
    pub data: HashMap<String, serde_json::Value>,
}

impl CrowLiveState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    /// Get a value from state.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }

    /// Set a value in state.
    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.data.insert(key.to_string(), value);
    }
}

impl Default for CrowLiveState {
    fn default() -> Self {
        Self::new()
    }
}

/// A diff representing changes to HTML content.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrowDiff {
    /// Target element selector.
    pub target: String,
    /// New HTML content.
    pub html: String,
    /// Type: "replace", "append", "prepend"
    pub action: String,
}

/// Simple HTML differ — compares old and new HTML and produces diffs.
pub fn diff_html(old: &str, new: &str) -> Vec<CrowDiff> {
    if old == new {
        return vec![];
    }
    // Simple full-replace strategy for now.
    // A real implementation would do DOM diffing.
    vec![CrowDiff {
        target: "body".to_string(),
        html: new.to_string(),
        action: "replace".to_string(),
    }]
}

/// Generate the LiveView client-side JavaScript.
pub fn client_js(ws_path: &str) -> String {
    format!(
        r#"<script>
(function() {{
    const ws = new WebSocket(location.origin.replace('http','ws') + '{}');
    ws.onmessage = function(e) {{
        const diffs = JSON.parse(e.data);
        diffs.forEach(function(d) {{
            const el = document.querySelector(d.target);
            if (el) {{
                if (d.action === 'replace') el.innerHTML = d.html;
                else if (d.action === 'append') el.innerHTML += d.html;
                else if (d.action === 'prepend') el.innerHTML = d.html + el.innerHTML;
            }}
        }});
    }};
    document.addEventListener('click', function(e) {{
        if (e.target.dataset.event) {{
            ws.send(JSON.stringify({{event: e.target.dataset.event}}));
        }}
    }});
}})();
</script>"#,
        ws_path
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_live_state() {
        let mut state = CrowLiveState::new();
        state.set("count", serde_json::json!(0));
        assert_eq!(state.get("count"), Some(&serde_json::json!(0)));
    }

    #[test]
    fn test_diff_same_html() {
        let diffs = diff_html("<div>hello</div>", "<div>hello</div>");
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_changed_html() {
        let diffs = diff_html("<div>old</div>", "<div>new</div>");
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].action, "replace");
        assert_eq!(diffs[0].html, "<div>new</div>");
    }

    #[test]
    fn test_client_js() {
        let js = client_js("/ws/live");
        assert!(js.contains("WebSocket"));
        assert!(js.contains("/ws/live"));
    }

    #[test]
    fn test_live_state_default() {
        let state = CrowLiveState::default();
        assert!(state.data.is_empty());
    }
}

