//! # Aether Response Builder
//!
//! Pre-formats HTTP/1.1 response status lines and headers directly into
//! output bytes slices to eliminate temporary allocations.

use bytes::{Bytes, BytesMut};

pub struct Response {
    pub status: u16,
    pub status_text: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Bytes,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            status: 200,
            status_text: "OK",
            headers: Vec::with_capacity(6),
            body: Bytes::new(),
        }
    }

    pub fn json(body: Bytes) -> Self {
        let mut r = Self::ok();
        r.headers.push(("Content-Type", "application/json; charset=utf-8".to_string()));
        r.headers.push(("Content-Length", body.len().to_string()));
        r.body = body;
        r
    }

    /// Creates a JSON response with a custom HTTP status code and status text
    pub fn json_status(status: u16, status_text: &'static str, body: impl Into<Bytes>) -> Self {
        let b = body.into();
        let mut r = Self::status(status, status_text);
        r.headers.push(("Content-Type", "application/json; charset=utf-8".to_string()));
        r.headers.push(("Content-Length", b.len().to_string()));
        r.body = b;
        r
    }

    pub fn text(body: &'static str) -> Self {
        let mut r = Self::ok();
        r.headers.push(("Content-Type", "text/plain; charset=utf-8".to_string()));
        r.headers.push(("Content-Length", body.len().to_string()));
        r.body = Bytes::from_static(body.as_bytes());
        r
    }

    pub fn html(body: &'static str) -> Self {
        let mut r = Self::ok();
        r.headers.push(("Content-Type", "text/html; charset=utf-8".to_string()));
        r.headers.push(("Content-Length", body.len().to_string()));
        r.body = Bytes::from_static(body.as_bytes());
        r
    }

    /// Creates an HTML response from a dynamic owned String
    pub fn html_dynamic(body: impl Into<String>) -> Self {
        let b = body.into();
        let bytes = Bytes::from(b.into_bytes());
        Self::bytes(bytes, "text/html; charset=utf-8")
    }

    /// Creates a Plain Text response from a dynamic owned String
    pub fn text_dynamic(body: impl Into<String>) -> Self {
        let b = body.into();
        let bytes = Bytes::from(b.into_bytes());
        Self::bytes(bytes, "text/plain; charset=utf-8")
    }

    /// Reads and serves a file directly from the filesystem with automatic MIME type detection
    pub fn file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read(path)?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let mime = crate::static_files::mime_from_extension(ext);
        Ok(Self::bytes(Bytes::from(content), mime))
    }

    pub fn bytes(body: Bytes, content_type: &'static str) -> Self {
        let mut r = Self::ok();
        r.headers.push(("Content-Type", content_type.to_string()));
        r.headers.push(("Content-Length", body.len().to_string()));
        r.body = body;
        r
    }

    pub fn status(status: u16, msg: &'static str) -> Self {
        Self {
            status,
            status_text: msg,
            headers: vec![("Content-Length", "0".to_string())],
            body: Bytes::new(),
        }
    }

    /// Creates an HTTP redirect response (301 Moved Permanently, 302 Found, 307 Temporary Redirect, or 308 Permanent Redirect)
    pub fn redirect(location: impl Into<String>, status: u16) -> Self {
        let text = match status {
            301 => "Moved Permanently",
            302 => "Found",
            307 => "Temporary Redirect",
            308 => "Permanent Redirect",
            _ => "See Other",
        };
        let mut r = Self::status(status, text);
        r.headers.push(("Location", location.into()));
        r
    }

    /// Attaches a secure Cookie to the response (`Set-Cookie` header)
    pub fn set_cookie(
        mut self,
        name: &str,
        value: &str,
        http_only: bool,
        secure: bool,
        same_site: &str,
        max_age_secs: Option<u64>,
    ) -> Self {
        let mut cookie_str = format!("{}={}; SameSite={}; Path=/", name, value, same_site);
        if http_only {
            cookie_str.push_str("; HttpOnly");
        }
        if secure {
            cookie_str.push_str("; Secure");
        }
        if let Some(age) = max_age_secs {
            cookie_str.push_str(&format!("; Max-Age={}", age));
        }
        self.headers.push(("Set-Cookie", cookie_str));
        self
    }

    /// Adds a custom header (builder pattern)
    pub fn header(mut self, key: &'static str, val: impl Into<String>) -> Self {
        self.headers.push((key, val.into()));
        self
    }

    /// Adds a custom header in-place
    pub fn add_header(&mut self, key: &'static str, val: impl Into<String>) {
        self.headers.push((key, val.into()));
    }

    /// Formats the complete HTTP/1.1 payload into a reusable buffer with zero intermediate String allocations.
    pub fn write_http1_to(&self, out: &mut BytesMut) {
        use std::io::Write;
        {
            let mut writer = bytes::BufMut::writer(&mut *out);
            let _ = write!(writer, "HTTP/1.1 {} {}\r\n", self.status, self.status_text);

            for (k, v) in &self.headers {
                let _ = write!(writer, "{}: {}\r\n", k, v);
            }
            let _ = write!(writer, "\r\n");
        }
        out.extend_from_slice(&self.body);
    }
}
