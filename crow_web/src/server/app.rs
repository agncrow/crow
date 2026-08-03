//! CrowApp — the main web server abstraction.
//!
//! Wraps an axum `Router` and provides Crow-idiomatic methods:
//!
//! ```text
//! app = web.new[]
//! route[app, '/', fn[req] html['hello'] ]
//! serve[app, 3000]
//! ```

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Request},
    response::Response,
    routing::{delete, get, head, options, patch, post, put},
    Router,
};

use crate::core::{CrowRequest, CrowResponse};
use super::convert::{request_from_axum, response_to_axum};

/// Handler function type: takes a CrowRequest, returns a CrowResponse.
pub type CrowHandler = Arc<dyn Fn(CrowRequest) -> CrowResponse + Send + Sync>;

/// The Crow web application — wraps axum's Router.
pub struct CrowApp {
    router: Router,
}

impl CrowApp {
    /// Create a new app (`web.new[]` in Crow).
    pub fn new() -> Self {
        Self {
            router: Router::new(),
        }
    }

    /// Add a GET route (`route[app, path, handler]` in Crow).
    pub fn route(mut self, path: &str, handler: CrowHandler) -> Self {
        let axum_path = crow_path_to_axum(path);
        self.router = self.router.route(&axum_path, get(make_axum_handler(handler)));
        self
    }

    /// Add a route with a specific HTTP method (`route[app, path, method, handler]` in Crow).
    pub fn route_method(mut self, path: &str, method: &str, handler: CrowHandler) -> Self {
        let axum_path = crow_path_to_axum(path);
        let method_router = match method.to_uppercase().as_str() {
            "GET" => get(make_axum_handler(handler)),
            "POST" => post(make_axum_handler(handler)),
            "PUT" => put(make_axum_handler(handler)),
            "DELETE" => delete(make_axum_handler(handler)),
            "PATCH" => patch(make_axum_handler(handler)),
            "HEAD" => head(make_axum_handler(handler)),
            "OPTIONS" => options(make_axum_handler(handler)),
            _ => get(make_axum_handler(handler)), // fallback to GET
        };
        self.router = self.router.route(&axum_path, method_router);
        self
    }

    /// Start the server (`serve[app, port]` in Crow).
    pub async fn serve(self, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let addr = format!("0.0.0.0:{}", port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!(
                    "error: port {} is already in use — another instance of this app? \
                     stop it with: crow stop  (or: lsof -ti :{} | xargs kill)",
                    port, port
                );
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("error: cannot start server on port {}: {}", port, e);
                std::process::exit(1);
            }
        };
        // Add a default welcome fallback so new projects don't show a blank page
        let router = self.router.fallback(default_welcome_handler);
        println!("crow server listening on http://{}", addr);
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Add a WebSocket route (`ws[app, path, handler]` in Crow).
    ///
    /// Available only when both `http` and `ws` features are enabled.
    #[cfg(feature = "ws")]
    pub fn ws_route(mut self, path: &str, handler: crate::ws::CrowWsHandler) -> Self {
        let axum_path = crow_path_to_axum(path);
        self.router = self.router.route(
            &axum_path,
            get(crate::ws::make_ws_axum_handler(handler)),
        );
        self
    }

    /// Add an SSE route (`sse[app, path, handler]` in Crow).
    ///
    /// Available only when the `sse` feature is enabled.
    /// The handler receives a `CrowSseSender` and pushes events.
    #[cfg(feature = "sse")]
    pub fn sse_route(mut self, path: &str, handler: crate::sse::CrowSseHandler) -> Self {
        let axum_path = crow_path_to_axum(path);
        self.router = self.router.route(
            &axum_path,
            get(crate::sse::make_sse_axum_handler(handler)),
        );
        self
    }

    // ── Convenience methods for feature-gated modules ──────────────

    /// Apply CORS middleware (`cors[app, {origins, '*'}]` in Crow).
    ///
    /// Available only when the `cors` feature is enabled.
    #[cfg(feature = "cors")]
    pub fn cors(self, config: crate::cors::CrowCors) -> Self {
        Self {
            router: config.apply(self.router),
        }
    }

    /// Apply permissive CORS (allow all origins/methods/headers).
    ///
    /// Shorthand for `app.cors(CrowCors::permissive())`.
    #[cfg(feature = "cors")]
    pub fn cors_permissive(self) -> Self {
        self.cors(crate::cors::CrowCors::permissive())
    }

    /// Serve a static file directory (`static[app, '/assets', './public']` in Crow).
    ///
    /// Available only when the `static_files` feature is enabled.
    #[cfg(feature = "static_files")]
    pub fn static_dir(self, url_path: &str, dir_path: &str) -> Self {
        Self {
            router: crate::static_files::serve_dir(self.router, url_path, dir_path),
        }
    }

    /// Attach a rate limiter (`limit[app, 100, 60]` in Crow).
    ///
    /// Returns the `CrowRateLimiter` so the user can call `limiter.check(key)`
    /// in route handlers.
    ///
    /// Available only when the `rate` feature is enabled.
    #[cfg(feature = "rate")]
    pub fn rate_limiter(max_requests: u64, window_secs: u64) -> crate::rate::CrowRateLimiter {
        crate::rate::CrowRateLimiter::new(max_requests, window_secs)
    }

    /// Create a session store (`sessions[]` in Crow).
    ///
    /// Returns a `CrowSessionStore` to be shared across handlers.
    ///
    /// Available only when the `session` feature is enabled.
    #[cfg(feature = "session")]
    pub fn session_store() -> crate::session::CrowSessionStore {
        crate::session::CrowSessionStore::new()
    }

    /// Load templates from a glob pattern (`templates['./templates/**/*']` in Crow).
    ///
    /// Returns a `CrowTemplates` engine for rendering.
    ///
    /// Available only when the `template` feature is enabled.
    #[cfg(feature = "template")]
    pub fn templates(glob: &str) -> Result<crate::template::CrowTemplates, String> {
        crate::template::CrowTemplates::from_glob(glob)
    }

    /// Merge an external axum Router into this app.
    /// Used by `live[]` to combine LiveView routers with the app.
    pub fn merge(self, other: Router) -> Self {
        Self {
            router: self.router.merge(other),
        }
    }

    /// Get the inner router (for testing or advanced composition).
    pub fn into_router(self) -> Router {
        self.router
    }
}

impl Default for CrowApp {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert Crow path syntax (`:id`) to axum 0.8 path syntax (`{id}`).
///
/// Axum 0.8 uses `{param}` for capture groups instead of `:param`.
/// Crow uses the more common `:param` syntax, so we convert here.
fn crow_path_to_axum(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == ':' {
            // Collect the parameter name
            let mut param = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    param.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            if param.is_empty() {
                result.push(':');
            } else {
                result.push('{');
                result.push_str(&param);
                result.push('}');
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Create an axum handler from a CrowHandler.
///
/// Extracts route params, converts axum Request → CrowRequest,
/// calls the user's handler, converts CrowResponse → axum Response.
fn make_axum_handler(
    handler: CrowHandler,
) -> impl Fn(
    Option<Path<std::collections::HashMap<String, String>>>,
    Request<Body>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
       + Clone
       + Send
       + Sync
       + 'static {
    move |path_params: Option<Path<std::collections::HashMap<String, String>>>,
          req: Request<Body>| {
        let handler = handler.clone();
        Box::pin(async move {
            let mut crow_req = request_from_axum(req).await;

            // Inject route params from axum's Path extractor
            if let Some(Path(params)) = path_params {
                crow_req.params = params;
            }

            let crow_resp = handler(crow_req);
            response_to_axum(crow_resp)
        })
    }
}

/// Default welcome page shown when no routes match.
/// Gives new projects a friendly landing page instead of a blank screen.
async fn default_welcome_handler() -> axum::response::Html<&'static str> {
    axum::response::Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Welcome to Crow</title>
<style>
  body { font-family: system-ui, sans-serif; display: flex; justify-content: center; align-items: center; min-height: 100vh; margin: 0; background: #0a0a0a; color: #e0e0e0; }
  .card { text-align: center; max-width: 480px; padding: 2rem; }
  h1 { font-size: 2.5rem; margin-bottom: 0.5rem; }
  .crow { font-size: 4rem; }
  p { color: #888; line-height: 1.6; }
  code { background: #1a1a2e; padding: 2px 8px; border-radius: 4px; font-size: 0.9em; }
</style>
</head>
<body>
<div class="card">
  <div class="crow">&#x1F426;</div>
  <h1>Welcome to Crow</h1>
  <p>Your server is running. Add routes with <code>live[app, '/', render, handle]</code> or <code>route[app, '/', handler]</code>.</p>
</div>
</body>
</html>"#)
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt; // for `oneshot`

    fn hello_handler(_req: CrowRequest) -> CrowResponse {
        CrowResponse::json(serde_json::json!({"message": "hello"}))
    }

    fn echo_param_handler(req: CrowRequest) -> CrowResponse {
        let id = req.param("id").unwrap_or("none").to_string();
        CrowResponse::json(serde_json::json!({"id": id}))
    }

    #[tokio::test]
    async fn test_crow_app_get_route() {
        let app = CrowApp::new()
            .route("/hello", Arc::new(hello_handler));

        let req = HttpRequest::builder()
            .uri("/hello")
            .body(Body::empty())
            .unwrap();

        let resp = app.into_router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "hello");
    }

    #[tokio::test]
    async fn test_crow_app_post_route() {
        let app = CrowApp::new()
            .route_method("/data", "POST", Arc::new(hello_handler));

        let req = HttpRequest::builder()
            .method("POST")
            .uri("/data")
            .body(Body::empty())
            .unwrap();

        let resp = app.into_router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn test_crow_app_404_on_unknown_route() {
        let app = CrowApp::new()
            .route("/hello", Arc::new(hello_handler));

        let req = HttpRequest::builder()
            .uri("/unknown")
            .body(Body::empty())
            .unwrap();

        let resp = app.into_router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn test_crow_app_route_params() {
        let app = CrowApp::new()
            .route("/users/:id", Arc::new(echo_param_handler));

        let req = HttpRequest::builder()
            .uri("/users/42")
            .body(Body::empty())
            .unwrap();

        let resp = app.into_router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], "42");
    }

    #[test]
    fn test_crow_path_to_axum() {
        assert_eq!(crow_path_to_axum("/users/:id"), "/users/{id}");
        assert_eq!(crow_path_to_axum("/"), "/");
        assert_eq!(crow_path_to_axum("/a/:b/:c"), "/a/{b}/{c}");
        assert_eq!(crow_path_to_axum("/static/file.txt"), "/static/file.txt");
    }

    /// Integration test: start a real server with a WebSocket echo route,
    /// connect with `CrowWebSocket`, send a message, and verify the echo.
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_ws_route_echo_integration() {
        use crate::ws;

        // Build app with a WebSocket echo handler
        let app = CrowApp::new().ws_route(
            "/echo",
            Arc::new(|sender: ws::CrowWsSender, msg: String| {
                sender.send(&format!("echo: {msg}"));
            }),
        );

        // Bind to a random available port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn the server in the background
        tokio::spawn(async move {
            axum::serve(listener, app.into_router()).await.unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Connect using the blocking client (run in spawn_blocking to avoid
        // blocking the tokio runtime)
        let port = addr.port();
        let result = tokio::task::spawn_blocking(move || {
            let sock = ws::connect(&format!("ws://127.0.0.1:{port}/echo"));
            assert!(sock.ok(), "WebSocket connection should succeed");

            sock.send("hello");
            let reply = sock.recv();
            sock.close();
            reply
        })
        .await
        .unwrap();

        assert_eq!(result, Some("echo: hello".to_string()));
    }
}

