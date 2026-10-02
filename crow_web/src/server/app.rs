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

/// A route's handler: its name, which the answer to a stop and the log
/// give, and what answers the request.
#[derive(Clone)]
pub struct CrowHandler {
    name: String,
    answer: Arc<dyn Fn(CrowRequest) -> CrowResponse + Send + Sync>,
}

impl CrowHandler {
    /// The handler named `name` (the program's function), answering with
    /// `answer`.
    pub fn new(name: &str, answer: impl Fn(CrowRequest) -> CrowResponse + Send + Sync + 'static) -> Self {
        Self { name: name.to_string(), answer: Arc::new(answer) }
    }
}

/// What a route answers when its handler fails or stops: 500 and an id,
/// which the server's log names beside the handler and the reason — the
/// same in both modes. (The answer had carried the reason itself, telling
/// any client what the program reaches; Vijay, 2026-10-01.)
pub fn route_stopped(handler: &str, reason: &str) -> CrowResponse {
    let id = crate::core::response::failure_id();
    eprintln!("crow: {} stopped [{}]: {}", handler, id, reason);
    CrowResponse::stopped(&id)
}

/// The Crow web application — wraps axum's Router.
pub struct CrowApp {
    router: Router,
    /// whether a route or a page was registered: an app with none shows the
    /// welcome page at every address, an app with some a 404 at an address
    /// none answers
    routed: bool,
    /// `serve[…, cors = […]]`: the origins a page on another site may call
    /// this app from (`'*'` any)
    cors: Vec<String>,
    /// `serve[…, files = 'public']`: the directory whose files an address
    /// no route or page answers is
    files: Option<String>,
}

impl CrowApp {
    /// Create a new app (`web.new[]` in Crow).
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            routed: false,
            cors: Vec::new(),
            files: None,
        }
    }

    /// Add a GET route (`route[path, handler]` in Crow).
    pub fn route(self, path: &str, handler: CrowHandler) -> Self {
        self.route_limited(path, "GET", handler, 0)
    }

    /// Add a route with a specific HTTP method (`route[path, method, handler]` in Crow).
    pub fn route_method(self, path: &str, method: &str, handler: CrowHandler) -> Self {
        self.route_limited(path, method, handler, 0)
    }

    /// Add a route that answers each client at most `per_minute` times a
    /// minute (`route[path, handler, limit = 60]`; 0 is no limit): a request
    /// over it is answered `429 Too Many Requests`, with the seconds to wait
    /// in `Retry-After`.
    pub fn route_limited(mut self, path: &str, method: &str, handler: CrowHandler, per_minute: u64) -> Self {
        let axum_path = crow_path_to_axum(path);
        let limit = (per_minute > 0).then(|| super::limit::Limit::new(per_minute));
        let h = make_axum_handler(handler, limit);
        let method_router = match method.to_uppercase().as_str() {
            "POST" => post(h),
            "PUT" => put(h),
            "DELETE" => delete(h),
            "PATCH" => patch(h),
            "HEAD" => head(h),
            "OPTIONS" => options(h),
            _ => get(h),
        };
        self.router = self.router.route(&axum_path, method_router);
        self.routed = true;
        self
    }

    /// `serve[…, cors = ['https://app.example.com']]`: the origins a page on
    /// another site may call this app from, `'*'` for any; a preflight is
    /// answered for them, and none other is told the app answers.
    pub fn cors_origins(mut self, origins: &[String]) -> Self {
        self.cors = origins.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        self
    }

    /// `serve[…, files = 'public']`: an address no route or page answers is
    /// the directory's file at that path (`/logo.png` is
    /// `public/logo.png`, `/` its `index.html`), and a 404 when there is
    /// none.
    pub fn files_from(mut self, dir: &str) -> Self {
        if !dir.trim().is_empty() {
            self.files = Some(dir.trim().to_string());
            self.routed = true;
        }
        self
    }

    /// Start the server (`serve[app, port]` in Crow). It listens on this
    /// machine only, unless `CROW_HOST` names another interface.
    pub async fn serve(self, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        // `crow test --compiled` starts the program on a port of its own
        let port = std::env::var("CROW_PORT").ok().and_then(|p| p.trim().parse::<u16>().ok()).unwrap_or(port);
        let host = listen_host(std::env::var("CROW_HOST").ok());
        let addr = if host.contains(':') && !host.starts_with('[') {
            format!("[{}]:{}", host, port)
        } else {
            format!("{}:{}", host, port)
        };
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
        let router = self.served();
        // under crow dev: its routes, and an end when crow dev ends
        #[cfg(feature = "live")]
        let router = match crate::live::dev_router() {
            Some(dev) => {
                crate::live::end_with_parent();
                router.merge(dev)
            }
            None => router,
        };
        println!("crow server listening on http://{}", addr);
        tokio::spawn(stop_on_signal());
        axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
        Ok(())
    }

    /// Merge an external axum Router into this app.
    /// Used by `live[]` to combine LiveView routers with the app.
    pub fn merge(self, other: Router) -> Self {
        Self {
            router: self.router.merge(other),
            routed: true,
            ..self
        }
    }

    /// Get the inner router (for testing or advanced composition).
    pub fn into_router(self) -> Router {
        self.router
    }

    /// The router as `serve` runs it: its fallback, its files and its CORS.
    pub fn served_router(self) -> Router {
        self.served()
    }

    /// The router `serve` runs: an app with no routes shows the welcome page
    /// at every address, so a new project never shows a blank one; an app
    /// with some answers an address none of them does with a 404.
    fn served(self) -> Router {
        let router = match (&self.files, self.routed) {
            (Some(dir), _) => {
                let files = tower_http::services::ServeDir::new(dir).not_found_service(get(not_found_handler));
                self.router.fallback(move |req: axum::extract::Request| {
                    let files = files.clone();
                    async move {
                        use axum::response::IntoResponse;
                        if hidden_path(req.uri().path()) {
                            return not_found_handler(req.uri().clone()).await.into_response();
                        }
                        match tower::ServiceExt::oneshot(files, req).await {
                            Ok(found) => found.into_response(),
                            Err(never) => match never {},
                        }
                    }
                })
            }
            (None, true) => self.router.fallback(not_found_handler),
            (None, false) => self.router.fallback(default_welcome_handler),
        };
        if self.cors.is_empty() {
            return router;
        }
        use tower_http::cors::{AllowOrigin, Any, CorsLayer};
        let origin = if self.cors.iter().any(|o| o == "*") {
            AllowOrigin::any()
        } else {
            AllowOrigin::list(self.cors.iter().filter_map(|o| o.parse().ok()))
        };
        router.layer(CorsLayer::new().allow_origin(origin).allow_methods(Any).allow_headers(Any))
    }
}

impl Default for CrowApp {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a served app listens: this machine, unless `CROW_HOST` names
/// another interface — a container or a server behind a proxy sets
/// `CROW_HOST=0.0.0.0`. An app is never reachable from the network by
/// accident: its handlers act on its data.
pub fn listen_host(crow_host: Option<String>) -> String {
    crow_host
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Convert Crow path syntax (`:id`) to axum 0.8 path syntax (`{id}`),
/// as a page's path is (crate::core::path).
fn crow_path_to_axum(path: &str) -> String {
    crate::core::path::to_axum(path)
}

/// Create an axum handler from a CrowHandler.
///
/// Extracts route params, converts axum Request → CrowRequest, calls the
/// user's handler, converts CrowResponse → axum Response. The handler runs
/// off the server's threads, as a page's does, so a slow one delays only
/// its own request; one that stops answers 500 with the reason, as the
/// interpreter's route does (the compiled program had dropped the
/// connection, saying nothing).
fn make_axum_handler(
    handler: CrowHandler,
    limit: Option<super::limit::Limit>,
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
        let limit = limit.clone();
        Box::pin(async move {
            if let Some(limit) = &limit {
                let peer = req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
                let forwarded = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok());
                if let Err(wait) = limit.check(&super::limit::client_of(peer, forwarded)) {
                    return axum::response::IntoResponse::into_response((
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        [(axum::http::header::RETRY_AFTER, wait.to_string())],
                        format!("too many requests: try again in {} s", wait),
                    ));
                }
            }
            let mut crow_req = match request_from_axum(req).await {
                Ok(r) => r,
                Err(refused) => return refused,
            };

            // Inject route params from axum's Path extractor
            if let Some(Path(params)) = path_params {
                crow_req.params = params.into_iter().collect();
            }

            let files = crow_req.files.clone();
            let CrowHandler { name, answer } = handler;
            let crow_resp = match tokio::task::spawn_blocking(move || crow_runtime::stops::guarded(|| answer(crow_req))).await {
                Ok(Ok(resp)) => resp,
                Ok(Err(why)) => route_stopped(&name, &why),
                Err(e) => route_stopped(&name, &e.to_string()),
            };
            // the request's uploads are its handler's while it runs
            super::uploads::release(&files);
            response_to_axum(crow_resp)
        })
    }
}

/// An address no route or page answers, in an app that has some.
/// Whether a path goes through a file or directory whose name starts with a
/// dot — `.env`, `.git/`, `.crow/keys/` — which no app serves from its files
/// (`files = '.'` had handed out the key the sign-in cookies are sealed
/// with). `.well-known`, where a certificate authority looks, is served.
/// Decoded first: `%2e` is a dot.
fn hidden_path(path: &str) -> bool {
    crate::core::request::decode_component(path).split(['/', '\\']).any(|segment| segment.starts_with('.') && segment != ".well-known")
}

async fn not_found_handler(uri: axum::http::Uri) -> (axum::http::StatusCode, axum::response::Html<String>) {
    let path = uri.path().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    (axum::http::StatusCode::NOT_FOUND, axum::response::Html(format!("<!DOCTYPE html><title>Not found</title><p>no page at {}</p>", path)))
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
  .co { font-size: 4rem; }
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


/// Told to stop (`crow stop`, a rebuild, Ctrl-C): what the program started
/// on its databases stops with it — a query nobody will read is not left
/// running on the server — and then the process ends.
async fn stop_on_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(t) => t,
            Err(_) => {
                let _ = ctrl_c.await;
                return stop_now().await;
            }
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
    stop_now().await;
}

async fn stop_now() {
    let _ = tokio::task::spawn_blocking(crow_runtime::cancel::shutdown).await;
    std::process::exit(0);
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

    /// A served app listens on this machine unless told otherwise: it used
    /// to listen on every interface, so anyone on the network reached its
    /// handlers
    #[test]
    fn an_app_listens_on_this_machine_unless_told() {
        assert_eq!(listen_host(None), "127.0.0.1");
        assert_eq!(listen_host(Some("  ".into())), "127.0.0.1");
        assert_eq!(listen_host(Some("0.0.0.0".into())), "0.0.0.0");
    }

    fn echo_param_handler(req: CrowRequest) -> CrowResponse {
        let id = req.param("id").unwrap_or("none").to_string();
        CrowResponse::json(serde_json::json!({"id": id}))
    }

    #[tokio::test]
    async fn test_crow_app_get_route() {
        let app = CrowApp::new()
            .route("/hello", CrowHandler::new("hello", hello_handler));

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
            .route_method("/data", "POST", CrowHandler::new("hello", hello_handler));

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
            .route("/hello", CrowHandler::new("hello", hello_handler));

        let req = HttpRequest::builder()
            .uri("/unknown")
            .body(Body::empty())
            .unwrap();

        let resp = app.into_router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    /// An app with no routes welcomes at every address; an app with routes
    /// or pages answers an address none of them does with a 404 (it had
    /// shown the welcome page there too).
    #[tokio::test]
    async fn an_address_no_route_answers_is_not_found() {
        let get = |uri: &str| HttpRequest::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = CrowApp::new().served().oneshot(get("/anything")).await.unwrap();
        assert_eq!(resp.status(), 200, "a new project is welcomed");
        let app = CrowApp::new().route("/hello", CrowHandler::new("hello", hello_handler));
        let resp = app.served().oneshot(get("/nothere")).await.unwrap();
        assert_eq!(resp.status(), 404);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 16).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("no page at /nothere"));
        let pages = CrowApp::new().merge(Router::new().route("/card/{card}", get_handler()));
        assert_eq!(pages.served().oneshot(get("/cards")).await.unwrap().status(), 404, "a page counts as a route");
    }

    /// A handler that stops answers 500 and an id, as the interpreter's
    /// route does (the compiled program had dropped the connection) — the
    /// reason is the server's log's, beside the id, not the client's — and
    /// the server answers the next request
    #[tokio::test]
    async fn a_handler_that_stops_answers_500() {
        fn boom(_req: CrowRequest) -> CrowResponse {
            crow_runtime::stops::halt(5, "index 9 is out of range for an array of 2 items")
        }
        let get = |uri: &str| HttpRequest::builder().uri(uri).body(Body::empty()).unwrap();
        let router = CrowApp::new().route("/boom", CrowHandler::new("boom", boom)).route("/hello", CrowHandler::new("hello", hello_handler)).into_router();
        let resp = router.clone().oneshot(get("/boom")).await.unwrap();
        assert_eq!(resp.status(), 500);
        let body = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).to_string();
        let id = body.strip_prefix("the request could not be answered — the server's log has why, as ").unwrap_or_else(|| panic!("{}", body));
        assert!(id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()), "{}", body);
        assert!(!body.contains("index 9"), "the reason is not the client's: {}", body);
        assert_eq!(router.oneshot(get("/hello")).await.unwrap().status(), 200);
    }

    fn get_handler() -> axum::routing::MethodRouter {
        axum::routing::get(|| async { "a page" })
    }

    #[tokio::test]
    async fn test_crow_app_route_params() {
        let app = CrowApp::new()
            .route("/users/:id", CrowHandler::new("echo", echo_param_handler));

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
}

