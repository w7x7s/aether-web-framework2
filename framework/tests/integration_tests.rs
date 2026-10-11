//! # Aether Integration Test Suite
//!
//! Tests the full request lifecycle: Parse -> Route -> Validate -> Response

#[cfg(test)]
mod tests {
    use aether_http::limits::ServerLimits;
    use aether_http::method::Method;
    use aether_http::parser::RequestView;
    use aether_router::trie::{MatchResult, RadixRouter};
    use aether_validation::validator::Email;

    #[test]
    fn test_full_request_lifecycle() {
        let raw_http = b"POST /api/v1/users/42/activate HTTP/1.1\r\n\
                         Host: api.example.com\r\n\
                         Content-Length: 26\r\n\
                         Content-Type: application/json\r\n\
                         \r\n\
                         {\"email\":\"user@domain.com\"}";

        let limits = ServerLimits::default();
        let (req, consumed) = RequestView::parse(raw_http, &limits)
            .expect("Valid request")
            .expect("Complete request");

        assert_eq!(consumed, raw_http.len());
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.path, "/api/v1/users/42/activate");

        // Radix Routing
        let mut router = RadixRouter::new();
        router.add_route(Method::Post, "/api/v1/users/{id}/activate", 999);

        match router.find(req.method, req.path) {
            MatchResult::Found { handler_id, params } => {
                assert_eq!(handler_id, 999);
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].key, "id");
                assert_eq!(params[0].value, "42");
            }
            _ => panic!("Route matching failed"),
        }

        // Validation
        let email = Email::try_new("user@domain.com".to_string()).expect("Email valid");
        assert_eq!(email.as_str(), "user@domain.com");
    }

    #[test]
    fn test_pipelined_requests_framing() {
        let limits = ServerLimits::default();
        let req1 = b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let req2 = b"POST /second HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nworld";
        
        let mut combined = Vec::new();
        combined.extend_from_slice(req1);
        combined.extend_from_slice(req2);

        // Parse Request 1
        let (r1, consumed1) = RequestView::parse(&combined, &limits).unwrap().unwrap();
        assert_eq!(r1.path, "/first");
        assert_eq!(consumed1, req1.len());

        // Parse Request 2 from remaining slice
        let remaining = &combined[consumed1..];
        let (r2, consumed2) = RequestView::parse(remaining, &limits).unwrap().unwrap();
        assert_eq!(r2.path, "/second");
        assert_eq!(r2.body, b"world");
        assert_eq!(consumed2, req2.len());
    }

    #[test]
    fn test_fragmented_tcp_read_handling() {
        let limits = ServerLimits::default();
        let full = b"POST /submit HTTP/1.1\r\nHost: example.com\r\nContent-Length: 10\r\n\r\n0123456789";

        // Partial header
        let part1 = &full[..15];
        assert_eq!(RequestView::parse(part1, &limits).unwrap(), None);

        // Header complete, partial body
        let part2 = &full[..full.len() - 3];
        assert_eq!(RequestView::parse(part2, &limits).unwrap(), None);

        // Complete request
        let (req, consumed) = RequestView::parse(full, &limits).unwrap().unwrap();
        assert_eq!(consumed, full.len());
        assert_eq!(req.body, b"0123456789");
    }
}

