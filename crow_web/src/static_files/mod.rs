//! `web.static` module — Serve static files from a directory.
//!
//! ```text
//! use web
//! use web.static
//!
//! app = web.new[]
//! static[app, '/assets', './public']
//! ```

use axum::Router;
use tower_http::services::ServeDir;

/// Nest a static file directory under a URL path.
///
/// `serve_dir(router, "/assets", "./public")` serves files from `./public`
/// under the `/assets` URL prefix.
pub fn serve_dir(router: Router, url_path: &str, dir_path: &str) -> Router {
    router.nest_service(url_path, ServeDir::new(dir_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serve_dir_creates_router() {
        let router = Router::new();
        let _router = serve_dir(router, "/assets", "./nonexistent_but_valid");
        // Should not panic — directory existence is checked at request time
    }

    #[tokio::test]
    async fn test_serve_dir_integration() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        // Create a temp dir with a file
        let dir = std::env::temp_dir().join("crow_static_test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "hello static").unwrap();

        let router = serve_dir(Router::new(), "/files", dir.to_str().unwrap());

        let req = Request::builder()
            .uri("/files/hello.txt")
            .body(Body::empty())
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "hello static");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}

