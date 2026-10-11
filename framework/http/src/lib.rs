//! # Aether HTTP Core Crate

pub mod header;
pub mod limits;
pub mod method;
pub mod parser;
pub mod response;
pub mod static_files;

pub use header::{HeaderMap, HeaderView, standard_headers};
pub use limits::ServerLimits;
pub use method::Method;
pub use parser::RequestView;
pub use response::Response;
pub use static_files::{mime_from_extension, generate_etag, serve_static};
