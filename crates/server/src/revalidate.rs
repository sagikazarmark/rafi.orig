//! Responses a browser keeps and revalidates on every use: `no-cache` with an
//! entity tag from the content's hash, and a `304 Not Modified` for a request
//! that already has it.

use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

/// The entity tag of `content`: part of its SHA-256, so it changes with it.
pub(crate) fn etag(content: &[u8]) -> String {
    let hash = Sha256::digest(content);
    format!("\"{}\"", hex::encode(&hash[..8]))
}

/// `body` as a response a browser revalidates on every use (`no-cache`, with
/// `etag`), or a 304 for a request that already has that version. Anything
/// that holds for both, the response's extensions among it, the caller adds
/// to what this returns.
pub(crate) fn no_cache(headers: &HeaderMap, etag: &str, body: impl IntoResponse) -> Response {
    let cache = [
        (header::ETAG, etag.to_owned()),
        (header::CACHE_CONTROL, "no-cache".to_owned()),
    ];
    if not_modified(headers, etag) {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (cache, body).into_response()
}

/// Whether `If-None-Match` names `etag` (a weak comparison) or is `*`.
fn not_modified(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}
