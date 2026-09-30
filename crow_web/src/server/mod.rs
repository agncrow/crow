//! `web` server module — HTTP server powered by axum.
//!
//! Enabled by the `http` feature flag.
//!
//! ```text
//! use web
//!
//! app = web.new[]
//! route[app, '/', fn[req] html['hello'] ]
//! serve[app, 3000]
//! ```

mod app;
mod convert;

pub use app::{route_stopped, CrowApp, CrowHandler};
pub use convert::{request_from_axum, response_to_axum};

