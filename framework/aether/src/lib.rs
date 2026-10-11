//! # Aether Web Framework
//!
//! High-performance, ergonomic Rust web framework with thread-per-core architecture,
//! zero-copy HTTP parsing, O(1) request-scoped arena allocation, and rich developer diagnostics.

pub mod app;
pub mod config;

pub use aether_core::diagnostic::{Diagnostic, ErrorCode, ServerBanner};
pub use aether_core::error::AetherError;
pub use aether_http::method::Method;
pub use aether_http::response::Response;
pub use aether_http::static_files::{generate_etag, mime_from_extension, serve_static};
pub use aether_middleware::{CorsConfig, CorsMiddleware, LoggerMiddleware, SecurityHeadersMiddleware};
pub use app::{AetherApp, AppStateContainer, IntoResponse, RequestContext, RouteGroup};
pub use config::AetherConfig;

pub mod prelude {
    pub use crate::app::{AetherApp, AppStateContainer, IntoResponse, RequestContext, RouteGroup};
    pub use crate::config::AetherConfig;
    pub use aether_core::diagnostic::{Diagnostic, ErrorCode, ServerBanner};
    pub use aether_core::error::AetherError;
    pub use aether_http::method::Method;
    pub use aether_http::response::Response;
    pub use aether_http::static_files::{generate_etag, mime_from_extension, serve_static};
    pub use aether_middleware::{CorsConfig, CorsMiddleware, LoggerMiddleware, SecurityHeadersMiddleware};
}
