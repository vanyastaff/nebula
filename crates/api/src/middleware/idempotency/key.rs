//! Idempotency key validation, cache-key composition, and body fingerprinting.
//!
//! This module owns everything that identifies *which request* we are
//! deduplicating: the validated [`IdempotencyKey`] newtype, the per-principal
//! identity fingerprint derived from auth headers, the SHA-256 body fingerprint,
//! and the final cache-key string that combines all four dimensions
//! `(method, path, key, identity)`.

use axum::http::{HeaderMap, HeaderValue, Method, header};
use sha2::{Digest, Sha256};

use super::MAX_KEY_LEN;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned when parsing/validating an `Idempotency-Key` header.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdempotencyKeyError {
    /// The header was present but contained no characters.
    #[error("Idempotency-Key header is empty")]
    Empty,

    /// The header value exceeded [`MAX_KEY_LEN`] octets.
    #[error("Idempotency-Key header exceeds max length of {MAX_KEY_LEN} bytes")]
    TooLong,

    /// The header contained bytes outside the printable-ASCII range.
    ///
    /// Restricting to printable ASCII keeps the cache key safe to render in
    /// logs and metrics labels without a separate encoding step.
    #[error("Idempotency-Key must be printable ASCII")]
    InvalidCharacters,
}

// ── Idempotency key newtype ──────────────────────────────────────────────────

/// Validated `Idempotency-Key` header value.
///
/// Construct via [`IdempotencyKey::parse`] — direct construction is forbidden
/// so the validation invariants below are guaranteed by the type:
///
/// - non-empty
/// - ≤ [`MAX_KEY_LEN`] bytes
/// - printable ASCII (`0x21..=0x7e`)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Parse a raw header value into a validated [`IdempotencyKey`].
    ///
    /// # Errors
    ///
    /// Returns [`IdempotencyKeyError`] if the value is empty, exceeds
    /// [`MAX_KEY_LEN`], or contains non-printable / non-ASCII bytes.
    pub fn parse(raw: &str) -> Result<Self, IdempotencyKeyError> {
        if raw.is_empty() {
            return Err(IdempotencyKeyError::Empty);
        }
        if raw.len() > MAX_KEY_LEN {
            return Err(IdempotencyKeyError::TooLong);
        }
        if !raw.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(IdempotencyKeyError::InvalidCharacters);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Borrow the validated key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Fingerprinting & cache-key composition ───────────────────────────────────

/// Compute a SHA-256 fingerprint of the request body bytes.
///
/// Stored in [`super::store::CachedResponse::request_fingerprint`] and
/// compared on every cache hit to detect "same key, different body" reuse
/// (draft §2.5 → 422 per ADR-0048).
pub(super) fn fingerprint_request_body(body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hasher.finalize().into()
}

/// Derive a per-principal identity fingerprint from the request headers.
///
/// Mix in any material that distinguishes callers before `auth_middleware`
/// runs (`Authorization`, `X-API-Key`, raw `Cookie` for session flows).
/// Order is fixed so the hash is stable across requests. Missing headers
/// contribute an empty segment — the resulting hash still differs from "no
/// headers at all" because the segment separators stay in the input.
pub(super) fn identity_fingerprint(headers: &HeaderMap) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let auth = headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    let api_key = headers
        .get("x-api-key")
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    let cookie = headers
        .get(header::COOKIE)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    hasher.update(b"authorization=");
    hasher.update(auth);
    hasher.update(b"\nx-api-key=");
    hasher.update(api_key);
    hasher.update(b"\ncookie=");
    hasher.update(cookie);
    hasher.finalize().into()
}

/// Build the dedup cache key string from the four-dimensional scope.
///
/// Format: `{method}|{path}|{key}|{identity_hex}`
///
/// Hex-encodes the identity fingerprint inline rather than pull `hex` as a
/// direct dep — `format!` keeps the helper allocation-light enough for the
/// hot path (single `String`) and avoids an extra crate edge.
pub(super) fn build_cache_key(
    method: &Method,
    path: &str,
    key: &IdempotencyKey,
    identity: &[u8; 32],
) -> String {
    let mut identity_hex = String::with_capacity(identity.len() * 2);
    for byte in identity {
        identity_hex.push_str(&format!("{byte:02x}"));
    }
    format!("{method}|{path}|{key}|{identity_hex}")
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use axum::http::{HeaderValue, Method, header};

    use super::*;

    #[test]
    fn parse_rejects_empty_key() {
        assert_eq!(IdempotencyKey::parse(""), Err(IdempotencyKeyError::Empty));
    }

    #[test]
    fn parse_rejects_oversized_key() {
        let oversized = "a".repeat(MAX_KEY_LEN + 1);
        assert_eq!(
            IdempotencyKey::parse(&oversized),
            Err(IdempotencyKeyError::TooLong)
        );
    }

    #[test]
    fn parse_rejects_non_ascii() {
        assert_eq!(
            IdempotencyKey::parse("kéy"),
            Err(IdempotencyKeyError::InvalidCharacters)
        );
    }

    #[test]
    fn parse_rejects_whitespace_and_control() {
        assert_eq!(
            IdempotencyKey::parse("a b"),
            Err(IdempotencyKeyError::InvalidCharacters),
            "spaces are outside printable-ASCII range",
        );
        assert_eq!(
            IdempotencyKey::parse("a\tb"),
            Err(IdempotencyKeyError::InvalidCharacters),
        );
    }

    #[test]
    fn parse_accepts_typical_uuid() {
        let key = IdempotencyKey::parse("3a82d4c4-78c9-4e7f-9bcf-1e7d80e9f4b1")
            .expect("uuid string is a valid key");
        assert_eq!(key.as_str(), "3a82d4c4-78c9-4e7f-9bcf-1e7d80e9f4b1");
    }

    #[test]
    fn cache_key_includes_identity_so_callers_cannot_share() {
        let key = IdempotencyKey::parse("k1").unwrap();
        let mut h1 = HeaderMap::new();
        h1.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer A"));
        let mut h2 = HeaderMap::new();
        h2.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer B"));

        let id1 = identity_fingerprint(&h1);
        let id2 = identity_fingerprint(&h2);
        assert_ne!(
            id1, id2,
            "different bearer tokens MUST yield different scopes"
        );

        let ck1 = build_cache_key(&Method::POST, "/x", &key, &id1);
        let ck2 = build_cache_key(&Method::POST, "/x", &key, &id2);
        assert_ne!(ck1, ck2);
    }

    #[test]
    fn fingerprint_is_stable_and_distinguishes_payloads() {
        let a = fingerprint_request_body(b"payload-a");
        let a_again = fingerprint_request_body(b"payload-a");
        let b = fingerprint_request_body(b"payload-b");
        assert_eq!(a, a_again);
        assert_ne!(a, b);
    }
}
