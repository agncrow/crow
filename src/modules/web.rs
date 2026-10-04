//! `use web` and its modules, as the registry answers them: the names
//! written in Crow — `msgpack` (src/modules/web.co), `web.auth`'s `sign`
//! and `verify` (src/modules/auth.co) — each a row of the builtin table
//! (crate::table). The server, the pages and the other responses stay the
//! compiler's own builtins (crate::lower_ir::checks, WEB_RESERVED).
use crate::modules::CrowModule;

pub struct WebModule;

pub(crate) const WEB_FUNCTIONS: &[&str] = &["msgpack"];

impl CrowModule for WebModule {
    fn name(&self) -> &str {
        "web"
    }

    fn functions(&self) -> &[&str] {
        WEB_FUNCTIONS
    }
}

pub struct AuthModule;

pub(crate) const AUTH_FUNCTIONS: &[&str] = &["sign", "verify"];

impl CrowModule for AuthModule {
    fn name(&self) -> &str {
        "web.auth"
    }

    fn functions(&self) -> &[&str] {
        AUTH_FUNCTIONS
    }
}
