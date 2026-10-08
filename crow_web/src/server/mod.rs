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
mod limit;
mod uploads;

pub use app::{route_stopped, CrowApp, CrowHandler};
pub use app::base_url;
pub use convert::{request_from_axum, response_to_axum};
pub use uploads::configure_uploads;

