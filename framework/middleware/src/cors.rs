//! # Zero-Allocation CORS Middleware

use aether_http::response::Response;

#[derive(Clone, Debug)]
pub struct CorsConfig {
    pub allow_origin: &'static str,
    pub allow_methods: &'static str,
    pub allow_headers: &'static str,
    pub max_age_secs: u32,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allow_origin: "*",
            allow_methods: "GET, POST, PUT, DELETE, PATCH, OPTIONS",
            allow_headers: "Authorization, Content-Type, Accept",
            max_age_secs: 86400,
        }
    }
}

pub struct CorsMiddleware {
    config: CorsConfig,
}

impl CorsMiddleware {
    pub fn new(config: CorsConfig) -> Self {
        Self { config }
    }

    pub fn apply(&self, response: &mut Response) {
        response.add_header("Access-Control-Allow-Origin", self.config.allow_origin);
        response.add_header("Access-Control-Allow-Methods", self.config.allow_methods);
        response.add_header("Access-Control-Allow-Headers", self.config.allow_headers);
    }
}
