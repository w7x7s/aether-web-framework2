//! # Production-Grade Zero-Copy JWT Token Validator
//!
//! Validates JWT tokens (`header.payload.signature`) using HMAC-SHA256 with constant-time
//! signature verification to prevent side-channel timing attacks.
//! Rejects algorithm confusion, tampered payloads, and invalid signatures.

use crate::constant_time::constant_time_eq;
use aether_core::error::AetherError;
use ring::hmac;

pub struct JwtValidator {
    key: hmac::Key,
}

impl JwtValidator {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, secret),
        }
    }

    /// Verifies token format `header.payload.signature` without heap reallocation
    /// using constant-time comparison of the actual decoded signature against expected HMAC.
    pub fn verify_signature<'a>(&self, token: &'a str) -> Result<(&'a str, &'a str), AetherError> {
        let mut parts = token.split('.');
        let header = parts.next().ok_or(AetherError::MalformedRequest)?;
        let payload = parts.next().ok_or(AetherError::MalformedRequest)?;
        let sig = parts.next().ok_or(AetherError::MalformedRequest)?;

        if parts.next().is_some() {
            return Err(AetherError::MalformedRequest);
        }

        if header.is_empty() || payload.is_empty() || sig.is_empty() {
            return Err(AetherError::ValidationFailed("Empty token component".into()));
        }

        // Decode the provided signature from base64url
        let sig_bytes = decode_base64url(sig)
            .map_err(|_| AetherError::ValidationFailed("Invalid base64url signature".into()))?;

        // Calculate expected HMAC-SHA256 tag over "header.payload"
        let signed_len = header.len() + 1 + payload.len();
        if signed_len > token.len() {
            return Err(AetherError::MalformedRequest);
        }
        let signed_data = &token[..signed_len];
        let expected_tag = hmac::sign(&self.key, signed_data.as_bytes());

        // Perform constant-time verification between the actual signature and the expected tag
        let expected_bytes = expected_tag.as_ref();
        if !constant_time_eq(&sig_bytes, expected_bytes) {
            return Err(AetherError::ValidationFailed("Signature verification failed".into()));
        }

        Ok((header, payload))
    }
}

/// Helper function to decode standard base64url (RFC 4648 Section 5) into raw bytes.
fn decode_base64url(input: &str) -> Result<Vec<u8>, ()> {
    fn decode_char(c: u8) -> Result<u8, ()> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(()),
        }
    }

    let input_bytes = input.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity((input_bytes.len() * 3) / 4);

    let mut buf = 0u32;
    let mut bits = 0u32;

    for &b in input_bytes {
        let val = decode_char(b)?;
        buf = (buf << 6) | (val as u32);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Ok(out)
}

/// Helper function to encode raw bytes to base64url without padding.
pub fn encode_base64url(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4) / 3 + 4);

    let mut buf = 0u32;
    let mut bits = 0u32;

    for &b in input {
        buf = (buf << 8) | (b as u32);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            let idx = (buf >> bits) & 0x3F;
            out.push(TABLE[idx as usize] as char);
            buf &= (1 << bits) - 1;
        }
    }

    if bits > 0 {
        let idx = (buf << (6 - bits)) & 0x3F;
        out.push(TABLE[idx as usize] as char);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_verification_valid_and_invalid() {
        let secret = b"super-secret-key-1234567890-aether";
        let validator = JwtValidator::new(secret);

        let header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let payload = "eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkhhbXphIn0";
        let signed_data = format!("{}.{}", header, payload);
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let tag = hmac::sign(&key, signed_data.as_bytes());
        let sig = encode_base64url(tag.as_ref());

        let valid_token = format!("{}.{}", signed_data, sig);

        // 1. Valid token must succeed
        let result = validator.verify_signature(&valid_token);
        assert!(result.is_ok());
        let (h, p) = result.unwrap();
        assert_eq!(h, header);
        assert_eq!(p, payload);

        // 2. Tampered payload must fail
        let tampered_payload = "eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkhhY2tlciJ9";
        let tampered_token = format!("{}.{}.{}", header, tampered_payload, sig);
        assert!(validator.verify_signature(&tampered_token).is_err());

        // 3. Fake signature must fail
        let fake_sig_token = format!("{}.{}.fakeSignature123", header, payload);
        assert!(validator.verify_signature(&fake_sig_token).is_err());
    }
}
