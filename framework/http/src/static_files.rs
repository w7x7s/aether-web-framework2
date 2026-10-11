//! # Aether Static Files Engine
//!
//! Zero-Copy static asset resolution with automatic MIME type detection,
//! strong ETag hashing, Cache-Control headers, conditional HTTP 304 responses,
//! and hardened path traversal protection.

use crate::response::Response;
use bytes::Bytes;
use std::path::{Component, Path, PathBuf};

/// Determines MIME type from file extension without external runtime dependencies.
pub fn mime_from_extension(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// Computes a fast lightweight ETag for cache validation.
pub fn generate_etag(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("\"W/{:x}-{:x}\"", hash, data.len())
}

/// Securely resolves a relative file path against a root directory, strictly preventing
/// path traversal attacks (`../`, `%2e%2e/`, Windows backslash escapes, null bytes, symlink escapes).
pub fn safe_resolve_path(root_dir: &Path, request_path: &str) -> Result<PathBuf, ()> {
    // Reject null bytes immediately
    if request_path.contains('\0') {
        return Err(());
    }

    let normalized = request_path.trim_start_matches('/');
    let target = Path::new(normalized);

    // Ensure no ParentDir (`..`) or Prefix/RootDir components exist
    for comp in target.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(()),
        }
    }

    let full_path = root_dir.join(target);

    // Canonicalize if file exists to verify it stays strictly inside root_dir
    if let (Ok(canonical_root), Ok(canonical_target)) = (root_dir.canonicalize(), full_path.canonicalize()) {
        if !canonical_target.starts_with(&canonical_root) {
            return Err(());
        }
    }

    Ok(full_path)
}

/// Builds an optimized Response for static files with caching.
pub fn serve_static(
    filename: &str,
    content: Bytes,
    if_none_match: Option<&str>,
    max_age_secs: u32,
) -> Response {
    let ext = filename.rsplit('.').next().unwrap_or("");
    let mime = mime_from_extension(ext);
    let etag = generate_etag(&content);

    // Conditional GET: 304 Not Modified check
    if let Some(req_etag) = if_none_match {
        if req_etag.trim() == etag {
            let mut not_modified = Response::status(304, "Not Modified");
            not_modified.headers.push(("ETag", etag));
            not_modified.headers.push(("Cache-Control", format!("public, max-age={}", max_age_secs)));
            return not_modified;
        }
    }

    let mut res = Response::ok();
    res.headers.push(("Content-Type", mime.to_string()));
    res.headers.push(("Content-Length", content.len().to_string()));
    res.headers.push(("ETag", etag));
    res.headers.push(("Cache-Control", format!("public, max-age={}", max_age_secs)));
    res.body = content;
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_traversal_prevention() {
        let root = Path::new("public");
        assert!(safe_resolve_path(root, "app.js").is_ok());
        assert!(safe_resolve_path(root, "css/style.css").is_ok());

        // Attacks
        assert!(safe_resolve_path(root, "../etc/passwd").is_err());
        assert!(safe_resolve_path(root, "sub/../../secret").is_err());
        assert!(safe_resolve_path(root, "/etc/shadow").is_err());
        assert!(safe_resolve_path(root, "foo\0bar.txt").is_err());
    }
}
