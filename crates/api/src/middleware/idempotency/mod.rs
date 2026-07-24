//! Idempotency-Key Middleware (M3.4).
//!
//! Generic completed-response replay per the IETF draft
//! [draft-ietf-httpapi-idempotency-key]. A client supplies an `Idempotency-Key`
//! header on an explicitly allow-listed request; after a response is
//! successfully cached, the middleware replays it byte-for-byte for subsequent
//! requests carrying the same key (within the configured TTL).
//!
//! ## Cached replay, not at-most-once execution
//!
//! After a response is cached, subsequent requests with the same `(method,
//! path, key, identity, body)` tuple replay it without invoking the handler.
//! This layer does **not** provide an in-flight claim or an at-most-once
//! execution guarantee. Concurrent cache misses may both run the handler, and
//! a completed handler whose response cannot be stored may run again on retry.
//! Product mutations therefore remain outside the first-party allow-list until
//! a durable atomic pending/terminal operation ledger owns their retry
//! protocol.
//!
//! [draft-ietf-httpapi-idempotency-key]: https://datatracker.ietf.org/doc/draft-ietf-httpapi-idempotency-key/
//!
//! ## Cache key composition
//!
//! The lookup key combines:
//!
//! - HTTP method (uppercase ASCII)
//! - Request URI path (without query string)
//! - The validated `Idempotency-Key` header value
//! - A fingerprint of identity-bearing material (`Authorization`, `X-API-Key`, and the raw `Cookie`
//!   header when present so session-cookie auth does not collide across principals)
//!
//! Including the identity fingerprint is **mandatory** because this layer sits
//! in the outer middleware stack — it runs *before* `auth_middleware` resolves
//! the [`Principal`](nebula_core::scope::Principal). Two callers MUST never
//! share a cached response even if they happen to choose the same key.
//!
//! ## Body fingerprint
//!
//! Every cached entry also stores a SHA-256 fingerprint of the request body.
//! Reusing the same key with a different body yields **422 Unprocessable
//! Entity** per draft §2.5 — protects against client bugs that re-use keys
//! across logically distinct operations.
//!
//! ## What gets cached
//!
//! Only explicitly allow-listed `POST` route templates participate. The
//! default route set is empty. First-party composition currently opts in only
//! internal `_test` fixtures; no product mutation relies on this cache for
//! retry safety. Responses with `Set-Cookie` or `Cache-Control: no-store` are
//! never buffered or stored even on an allow-listed route.
//!
//! Within that route set, only `2xx` and `4xx` responses are cached. `5xx`
//! responses are passed through uncached so transient backend failures do not
//! pin a permanent error for the TTL window. Responses larger than
//! [`IdempotencyConfig::max_response_body_bytes`] are passed through
//! uncached as well.
//!
//! ## Position in the middleware stack
//!
//! [`crate::app::build_app`] mounts this layer on the API router **before**
//! merging the webhook transport — webhook ingress has its own dedup
//! contract (provider signature + replay-window timestamp) and never
//! carries `Idempotency-Key`. The mount sits **inside** `request_id` and
//! `security_headers` so cached replays still acquire fresh
//! `X-Request-ID` and security headers when they leave the server:
//!
//! ```text
//! outermost                                                            innermost
//!  rate_limit -> request_id -> security_headers -> trace_w3c -> trace -> trace_response_inject -> compression -> cors -> idempotency -> routes
//!                                                                       (api only)
//! ```
//!
//! ## Backend selection
//!
//! [`memory::InMemoryIdempotencyStore`] is the dev / single-process default;
//! [`store::StorageBackedIdempotencyStore`] adapts a layer-1
//! `nebula_storage::repos::IdempotencyStoreRepo` (PG-backed in
//! production deployments) onto this trait. Selection is driven by
//! `ApiConfig.idempotency.backend` per **idempotency backend**: the in-memory
//! backend loses dedup state across restart and across runners, so
//! production deployments running more than one API replica must select
//! `Postgres` to satisfy the §M3 1.0 closure criterion.
//!
//! ## Module layout
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | [`key`] | [`IdempotencyKey`] newtype, validation, cache-key composition, body fingerprint |
//! | [`store`] | [`IdempotencyStore`] trait, [`IdempotencyStoreError`], [`StorageBackedIdempotencyStore`] |
//! | [`memory`] | [`InMemoryIdempotencyStore`] (moka cache) |
//! | [`layer`] | [`IdempotencyLayer`] + tower `Service` + metrics + buffering + replay-header logic |

pub mod key;
pub mod layer;
pub mod memory;
pub mod store;

// ── Header constants ─────────────────────────────────────────────────────────

use axum::http::HeaderName;

/// Header name carrying the client-supplied idempotency key.
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Header set on replayed responses so callers can distinguish a cache hit
/// from a fresh handler invocation.
pub const IDEMPOTENT_REPLAY_HEADER: HeaderName = HeaderName::from_static("idempotent-replay");

// ── Shared constants ─────────────────────────────────────────────────────────

/// Default cache lifetime — 24h matches the draft RFC's recommendation.
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;

/// Default cap on the number of cached responses.
pub const DEFAULT_MAX_ENTRIES: u64 = 10_000;

/// Default cap on the request/response body size eligible for caching (1 MiB).
pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Maximum acceptable length of an `Idempotency-Key` header value.
///
/// Picked to align with common gateway header limits and the draft RFC's
/// "opaque token, ≤ 255 octets" guidance.
pub const MAX_KEY_LEN: usize = 255;

/// Matched POST route templates whose completed responses may be cached by the
/// first-party API composition.
///
/// Only internal test fixtures participate. Product mutations pass through
/// normally even when a client supplies `Idempotency-Key`: this cache has no
/// atomic in-flight claim and must not be presented as at-most-once execution.
/// A product route may join only after a durable pending/terminal operation
/// ledger defines its retry protocol.
pub(crate) const REPLAY_SAFE_POST_ROUTES: &[&str] = &["/api/v1/_test/echo", "/api/v1/_test/fail"];

// ── Configuration ────────────────────────────────────────────────────────────

/// Tunables for the idempotency middleware.
#[derive(Debug, Clone)]
pub struct IdempotencyConfig {
    /// Maximum buffered request body size (in bytes) eligible for caching.
    /// Requests larger than this pass through without idempotency tracking.
    pub max_request_body_bytes: usize,
    /// Maximum buffered response body size (in bytes) eligible for caching.
    /// Larger responses are returned to the caller but never cached.
    pub max_response_body_bytes: usize,
}

impl Default for IdempotencyConfig {
    fn default() -> Self {
        Self {
            max_request_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_response_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }
}

// ── Re-exports (public-surface preservation) ─────────────────────────────────

pub use key::{IdempotencyKey, IdempotencyKeyError};
pub use layer::IdempotencyLayer;
pub use memory::InMemoryIdempotencyStore;
pub use store::{
    CachedResponse, IdempotencyStore, IdempotencyStoreError, StorageBackedIdempotencyStore,
};

#[cfg(test)]
mod tests {
    use super::REPLAY_SAFE_POST_ROUTES;

    #[test]
    fn product_mutations_are_not_replay_allowlisted() {
        assert_eq!(
            REPLAY_SAFE_POST_ROUTES,
            ["/api/v1/_test/echo", "/api/v1/_test/fail"]
        );
        assert!(
            !REPLAY_SAFE_POST_ROUTES
                .contains(&"/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh")
        );
        assert!(
            !REPLAY_SAFE_POST_ROUTES
                .contains(&"/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke")
        );
    }
}
