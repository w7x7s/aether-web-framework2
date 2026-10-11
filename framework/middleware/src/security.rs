use aether_http::response::Response;

/// Production security headers middleware.
#[derive(Debug, Clone)]
pub struct SecurityHeadersMiddleware {
    pub hsts: bool,
    pub frame_options: &'static str,
    pub content_type_options: &'static str,
    pub xss_protection: &'static str,
}

impl Default for SecurityHeadersMiddleware {
    fn default() -> Self {
        Self {
            hsts: true,
            frame_options: "DENY",
            content_type_options: "nosniff",
            xss_protection: "1; mode=block",
        }
    }
}

impl SecurityHeadersMiddleware {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply security headers to the outgoing response
    pub fn apply(&self, response: &mut Response) {
        response.add_header("X-Content-Type-Options", self.content_type_options);
        response.add_header("X-Frame-Options", self.frame_options);
        response.add_header("X-XSS-Protection", self.xss_protection);
        if self.hsts {
            response.add_header("Strict-Transport-Security", "max-age=31536000; includeSubDomains");
        }
    }
}
