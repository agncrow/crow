//! crow_web — Crow's modular web framework.
//!
//! One crate with Cargo feature flags per module.
//! `use web.X` in Crow source makes the Rust backend enable only
//! the `X` feature — minimal binary, minimal compile time.
//!
//! ## Architecture
//!
//! - `core` — shared types (Request, Response, Headers, Status). Always available.
//! - Feature-gated modules are added as they are implemented.
//!
//! ## Module dependency rules
//!
//! - Every module depends on `core`.
//! - No module depends on any other module.

// A runtime function answers its reason, or stops with it: it never
// panics (docs/bug-classes.md §3). `cargo clippy`, which the gate and CI
// run, refuses a panic, an unwrap or an expect that is not allowed with the
// invariant that makes it safe.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable, clippy::todo, clippy::unimplemented))]

pub mod core;

#[cfg(feature = "http")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "ws")]
pub mod ws;

#[cfg(feature = "auth")]
pub mod auth;

#[cfg(feature = "mail")]
pub mod mail;

#[cfg(feature = "live")]
pub mod live;

pub mod view;
