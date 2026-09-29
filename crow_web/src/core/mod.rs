//! Core types shared by all crow_web modules.
//!
//! No I/O, no networking — pure data structures.
//! Every module depends on `core`. No module depends on any other module.

pub mod headers;
pub mod path;
pub mod request;
pub mod response;

pub use headers::*;
pub use request::CrowRequest;
pub use response::CrowResponse;

