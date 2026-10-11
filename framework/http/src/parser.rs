//! # Ultra-fast Zero-Copy HTTP Request Parser
//!
//! Borrows all slices (&'req str, &'req [u8]) directly from the socket buffer.
//! Zero string copies, zero heap allocations for standard requests.
//! Hardened with RFC 9112 HTTP/1.1 Request Smuggling (CL.TE, TE.CL, Dual-CL) protection.

use crate::header::{HeaderMap, HeaderView};
use crate::limits::ServerLimits;
use crate::method::Method;
use aether_core::error::AetherError;

/// تمثيل مباشر وسريع لطلب HTTP يستعير شرائح الذاكرة (`&'req str`, `&'req [u8]`) مباشرة من بافر المقبس بدون أي نسخ (Zero-Copy).
#[derive(Debug)]
pub struct RequestView<'req> {
    pub method: Method,
    pub raw_uri: &'req str,
    pub path: &'req str,
    pub query_str: Option<&'req str>,
    pub version: u8,
    pub headers: HeaderMap<'req>,
    pub body: &'req [u8],
}

impl<'req> RequestView<'req> {
    /// تحليل بايتات المقبس والتحقق الصارم من التوافق مع RFC 9112 والحماية من Request Smuggling.
    /// 
    /// يعيد `Ok(Some((RequestView, consumed_bytes)))` إذا كان الطلب مكتملاً،
    /// أو `Ok(None)` إذا كانت البيانات جزئية ويجب انتظار باقي الحزم من TCP.
    pub fn parse(buf: &'req [u8], limits: &ServerLimits) -> Result<Option<(Self, usize)>, AetherError> {
        let mut raw_headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut raw_headers);

        let status = req.parse(buf).map_err(|_| AetherError::MalformedRequest)?;

        match status {
            httparse::Status::Complete(header_len) => {
                if header_len > limits.max_header_bytes {
                    return Err(AetherError::HeaderTooLarge);
                }

                let method_str = req.method.ok_or(AetherError::MalformedRequest)?;
                let method = Method::from_bytes(method_str.as_bytes())
                    .ok_or(AetherError::MalformedRequest)?;

                let raw_uri = req.path.ok_or(AetherError::MalformedRequest)?;
                let version = req.version.unwrap_or(1);

                // فصل المسار عن معلمات الاستعلام بدون تخصيص: /users?page=1 -> path="/users", query="page=1"
                let (path, query_str) = if let Some(idx) = raw_uri.find('?') {
                    (&raw_uri[..idx], Some(&raw_uri[idx + 1..]))
                } else {
                    (raw_uri, None)
                };

                let mut headers = HeaderMap::new();
                let mut content_length: Option<usize> = None;
                let mut has_transfer_encoding = false;

                for h in req.headers.iter() {
                    headers.push(HeaderView {
                        name: h.name,
                        value: h.value,
                    });

                    if h.name.eq_ignore_ascii_case("content-length") {
                        if content_length.is_some() {
                            // الحماية من هجمات Request Smuggling: تم رصد تكرار Content-Length
                            return Err(AetherError::MalformedRequest);
                        }

                        let val_str = std::str::from_utf8(h.value)
                            .map_err(|_| AetherError::MalformedRequest)?;
                        let parsed_len = val_str.trim().parse::<usize>()
                            .map_err(|_| AetherError::MalformedRequest)?;
                        content_length = Some(parsed_len);
                    } else if h.name.eq_ignore_ascii_case("transfer-encoding") {
                        has_transfer_encoding = true;
                    }
                }

                // RFC 9112 بند 6.3: حظر الجمع بين Transfer-Encoding و Content-Length (منع ثغرات CL.TE و TE.CL)
                if content_length.is_some() && has_transfer_encoding {
                    return Err(AetherError::MalformedRequest);
                }

                let cl = content_length.unwrap_or(0);
                if cl > limits.max_body_bytes {
                    return Err(AetherError::BodyLimitExceeded);
                }

                let total_needed = header_len + cl;
                if buf.len() < total_needed {
                    // Need more body bytes
                    return Ok(None);
                }

                let body = &buf[header_len..total_needed];

                Ok(Some((
                    RequestView {
                        method,
                        raw_uri,
                        path,
                        query_str,
                        version,
                        headers,
                        body,
                    },
                    total_needed,
                )))
            }
            httparse::Status::Partial => {
                if buf.len() > limits.max_header_bytes {
                    return Err(AetherError::HeaderTooLarge);
                }
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smuggling_dual_content_length_rejected() {
        let limits = ServerLimits::default();
        let raw = b"POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 10\r\n\r\nhello";
        let result = RequestView::parse(raw, &limits);
        assert!(matches!(result, Err(AetherError::MalformedRequest)));
    }

    #[test]
    fn test_smuggling_cl_te_conflict_rejected() {
        let limits = ServerLimits::default();
        let raw = b"POST / HTTP/1.1\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        let result = RequestView::parse(raw, &limits);
        assert!(matches!(result, Err(AetherError::MalformedRequest)));
    }
}

