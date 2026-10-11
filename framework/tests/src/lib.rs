#[cfg(test)]
mod tests {
    use aether::prelude::*;
    use aether_core::arena::RequestArena;
    use aether_core::bitset::Bitset64;
    use aether_core::diagnostic::{Diagnostic, ErrorCode, ServerBanner};
    use aether_http::limits::ServerLimits;
    use aether_http::parser::RequestView;
    use aether_http::static_files::{generate_etag, mime_from_extension};
    use bytes::BytesMut;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn test_error_codes_and_diagnostics() {
        let diag = Diagnostic::new(ErrorCode::E1001ServerStartupFailed, "Address already in use")
            .with_context("Address", "0.0.0.0:3000")
            .with_hint("Port is occupied.")
            .with_suggestion("Change port via AETHER_PORT=3001");

        let rendered = diag.format_terminal(false);
        assert!(rendered.contains("ERROR [E1001]"));
        assert!(rendered.contains("Address: 0.0.0.0:3000"));
        assert!(rendered.contains("Change port via AETHER_PORT=3001"));
    }

    #[test]
    fn test_server_banner() {
        let banner = ServerBanner {
            env: "development",
            address: "http://127.0.0.1:3000".to_string(),
            routes_count: 12,
            workers: 8,
        };
        let text = banner.render();
        assert!(text.contains("AETHER WEB FRAMEWORK"));
        assert!(text.contains("http://127.0.0.1:3000"));
    }

    #[test]
    fn test_static_mime_and_etag() {
        assert_eq!(mime_from_extension("html"), "text/html; charset=utf-8");
        assert_eq!(mime_from_extension("css"), "text/css; charset=utf-8");
        assert_eq!(mime_from_extension("js"), "application/javascript; charset=utf-8");
        assert_eq!(mime_from_extension("png"), "image/png");

        let data = b"console.log('hello aether');";
        let etag1 = generate_etag(data);
        let etag2 = generate_etag(data);
        assert_eq!(etag1, etag2);
        assert!(etag1.starts_with("\"W/"));
    }

    #[test]
    fn test_request_arena_zero_copy() {
        let mut arena = RequestArena::with_capacity(4096);
        let s = arena.alloc_slice(b"Zero-Copy Slice").unwrap();
        assert_eq!(s, b"Zero-Copy Slice");
        arena.reset();
        assert_eq!(arena.allocated_bytes(), 0);
    }

    #[test]
    fn test_64bit_bitset() {
        let mut bitset = Bitset64::new(0);
        bitset = bitset.set(0); // GET
        bitset = bitset.set(1); // POST
        assert!(bitset.contains(0));
        assert!(bitset.contains(1));
        assert!(!bitset.contains(2)); // PUT
    }

    #[test]
    fn test_app_state_container() {
        struct DbConfig {
            pool_size: u32,
        }

        let mut container = AppStateContainer::new();
        container.insert(DbConfig { pool_size: 50 });

        let retrieved = container.get::<DbConfig>().expect("DbConfig must exist");
        assert_eq!(retrieved.pool_size, 50);
    }

    #[test]
    fn test_http_request_parsing_and_query_split() {
        let raw_req = b"GET /api/v1/search?q=rust&limit=10 HTTP/1.1\r\nHost: localhost:3000\r\nCookie: session_id=xyz987\r\n\r\n";
        let limits = ServerLimits::default();
        let parsed = RequestView::parse(raw_req, &limits).unwrap();
        assert!(parsed.is_some());

        let (req, consumed) = parsed.unwrap();
        assert_eq!(consumed, raw_req.len());
        assert_eq!(req.path, "/api/v1/search");
        assert_eq!(req.query_str, Some("q=rust&limit=10"));
        assert_eq!(req.method, Method::Get);
    }

    #[test]
    fn test_response_headers_and_cookies() {
        let res = Response::ok()
            .set_cookie("auth_token", "secret123", true, true, "Strict", Some(3600))
            .header("X-Custom", "AetherEngine");

        let mut out = BytesMut::new();
        res.write_http1_to(&mut out);
        let out_str = std::str::from_utf8(&out).unwrap();

        assert!(out_str.contains("HTTP/1.1 200 OK"));
        assert!(out_str.contains("Set-Cookie: auth_token=secret123; SameSite=Strict; Path=/; HttpOnly; Secure; Max-Age=3600"));
        assert!(out_str.contains("X-Custom: AetherEngine"));
    }
}
