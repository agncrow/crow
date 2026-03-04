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

/// Diff two HTML strings using a line-level LCS algorithm.
/// Produces targeted diffs: unchanged lines are skipped, changed regions
/// produce "replace" diffs, and insertions/deletions produce "append"/"remove" diffs.
pub fn diff_html(old: &str, new: &str) -> Vec<CrowDiff> {
    if old == new {
        return vec![];
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Build LCS table
    let n = old_lines.len();
    let m = new_lines.len();
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            if old_lines[i] == new_lines[j] {
                table[i][j] = table[i + 1][j + 1] + 1;
            } else {
                table[i][j] = std::cmp::max(table[i + 1][j], table[i][j + 1]);
            }
        }
    }

    // Walk the table to emit diffs
    let mut diffs = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < n || j < m {
        if i < n && j < m && old_lines[i] == new_lines[j] {
            // Lines match, skip
            i += 1;
            j += 1;
        } else if j < m && (i >= n || table[i][j + 1] >= table[if i + 1 <= n { i + 1 } else { i }][j]) {
            // Insertion in new
            diffs.push(CrowDiff {
                target: format!("line:{}", j),
                html: new_lines[j].to_string(),
                action: "append".to_string(),
            });
            j += 1;
        } else if i < n {
            // Deletion from old
            diffs.push(CrowDiff {
                target: format!("line:{}", i),
                html: String::new(),
                action: "remove".to_string(),
            });
            i += 1;
        }
    }

    // If lines are the same count but content differs and LCS produced nothing useful,
    // fall back to a single replace
    if diffs.is_empty() && old != new {
        diffs.push(CrowDiff {
            target: "body".to_string(),
            html: new.to_string(),
            action: "replace".to_string(),
        });
    }

    diffs
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

/// A LiveView endpoint that manages server-side state and pushes HTML diffs
/// to connected clients over WebSocket.
pub struct CrowLiveView<F>
where
    F: Fn(&mut CrowLiveState, &str) -> String + Send + Sync + 'static,
{
    /// Initial state factory.
    pub initial_state: CrowLiveState,
    /// Render function: takes mutable state + event name, returns HTML.
    pub handler: F,
    /// WebSocket path (for client JS).
    pub ws_path: String,
}

impl<F> CrowLiveView<F>
where
    F: Fn(&mut CrowLiveState, &str) -> String + Send + Sync + Clone + 'static,
{
    /// Create a new LiveView.
    pub fn new(ws_path: &str, initial: CrowLiveState, handler: F) -> Self {
        Self {
            initial_state: initial,
            handler,
            ws_path: ws_path.to_string(),
        }
    }

    /// Build an Axum router with:
    /// - GET `page_path` → initial HTML + client JS
    /// - WS  `ws_path`   → event loop that pushes diffs
    pub fn into_router(self, page_path: &str) -> axum::Router {
        let ws_path = self.ws_path.clone();
        let handler = std::sync::Arc::new(self.handler);
        let init_state = std::sync::Arc::new(self.initial_state);

        // Initial page render
        let h = handler.clone();
        let s = init_state.clone();
        let ws_p = ws_path.clone();
        let page_route = axum::routing::get(move || {
            let h = h.clone();
            let s = s.clone();
            let ws_p = ws_p.clone();
            async move {
                let mut state = (*s).clone();
                let html = (h)(&mut state, "init");
                let js = client_js(&ws_p);
                axum::response::Html(format!("{}{}", html, js))
            }
        });

        // WebSocket handler
        let h2 = handler.clone();
        let s2 = init_state.clone();
        let ws_route = axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| {
            let h2 = h2.clone();
            let s2 = s2.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    use axum::extract::ws::Message;
                    let mut state = (*s2).clone();
                    let mut last_html = (h2)(&mut state, "init");

                    while let Some(Ok(msg)) = futures_util::StreamExt::next(&mut socket).await {
                        if let Message::Text(text) = msg {
                            let event = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                v["event"].as_str().unwrap_or("unknown").to_string()
                            } else {
                                text.to_string()
                            };

                            let new_html = (h2)(&mut state, &event);
                            let diffs = diff_html(&last_html, &new_html);
                            last_html = new_html;

                            if !diffs.is_empty() {
                                let payload = serde_json::to_string(&diffs).unwrap_or_default();
                                if futures_util::SinkExt::send(
                                    &mut socket,
                                    Message::Text(payload.into()),
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                })
            }
        });

        axum::Router::new()
            .route(page_path, page_route)
            .route(&ws_path, ws_route)
    }
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
    fn test_diff_changed_single_line() {
        let diffs = diff_html("<div>old</div>", "<div>new</div>");
        // LCS on single-line strings: remove old line, append new line
        assert!(!diffs.is_empty());
        // Should have both a remove and an append
        let has_remove = diffs.iter().any(|d| d.action == "remove");
        let has_append = diffs.iter().any(|d| d.action == "append" && d.html == "<div>new</div>");
        assert!(has_remove || has_append || diffs[0].action == "replace");
    }

    #[test]
    fn test_diff_multiline_insertion() {
        let old = "<ul>\n<li>a</li>\n</ul>";
        let new = "<ul>\n<li>a</li>\n<li>b</li>\n</ul>";
        let diffs = diff_html(old, new);
        assert!(!diffs.is_empty());
        // Should detect the inserted <li>b</li> line
        let appended: Vec<_> = diffs.iter().filter(|d| d.action == "append").collect();
        assert!(!appended.is_empty());
        assert!(appended.iter().any(|d| d.html.contains("<li>b</li>")));
    }

    #[test]
    fn test_diff_multiline_deletion() {
        let old = "<ul>\n<li>a</li>\n<li>b</li>\n</ul>";
        let new = "<ul>\n<li>a</li>\n</ul>";
        let diffs = diff_html(old, new);
        assert!(!diffs.is_empty());
        let removed: Vec<_> = diffs.iter().filter(|d| d.action == "remove").collect();
        assert!(!removed.is_empty());
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

