//! `web.cors` module — Cross-Origin Resource Sharing middleware.
//!
//! ```text
//! use web
//! use web.cors
//!
//! app = web.new[]
//! cors[app, {origins, '*'}]
//! ```

use axum::Router;
use tower_http::cors::{Any, CorsLayer};

/// CORS configuration builder.
#[derive(Clone)]
pub struct CrowCors {
    layer: CorsLayer,
}

impl CrowCors {
    /// Allow all origins, methods, and headers (`cors[app, {origins, '*'}]`).
    pub fn permissive() -> Self {
        Self {
            layer: CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        }
    }

    /// Allow specific origins.
    pub fn origins(origins: Vec<String>) -> Self {
        let parsed: Vec<_> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        Self {
            layer: CorsLayer::new()
                .allow_origin(parsed)
                .allow_methods(Any)
                .allow_headers(Any),
        }
    }

    /// Apply this CORS config to a router.
    pub fn apply(self, router: Router) -> Router {
        router.layer(self.layer)
    }

    /// Get the inner CorsLayer for manual composition.
    pub fn into_layer(self) -> CorsLayer {
        self.layer
    }
}

/// Convenience: create a permissive CORS layer.
pub fn permissive() -> CrowCors {
    CrowCors::permissive()
}

/// Convenience: create a CORS layer allowing specific origins.
pub fn origins(origins: Vec<String>) -> CrowCors {
    CrowCors::origins(origins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cors_permissive() {
        let cors = CrowCors::permissive();
        // Should not panic — layer is valid
        let _layer = cors.into_layer();
    }

    #[test]
    fn test_cors_origins() {
        let cors = CrowCors::origins(vec![
            "http://localhost:3000".to_string(),
            "https://example.com".to_string(),
        ]);
        let _layer = cors.into_layer();
    }

    #[tokio::test]
    async fn test_cors_apply_to_router() {
        let router = Router::new();
        let cors = CrowCors::permissive();
        let _router = cors.apply(router);
        // Router with CORS layer should be valid
    }
}

