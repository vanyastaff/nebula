//! Idempotency-Key Middleware (M3.4).
//!
//! Generic POST-replay protection per the IETF draft
//! [draft-ietf-httpapi-idempotency-key]. A client supplies an `Idempotency-Key`
//! header with any state-changing request; the middleware caches the first
//! response and replays it byte-for-byte for subsequent requests carrying the
//! same key (within the configured TTL).
//!
//! ## At-most-once semantics
//!
//! After the first response is cached, subsequent requests with the same
//! `(method, path, key, identity, body)` tuple replay the cached
//! response without invoking the handler. **Concurrent in-flight
//! requests are not deduplicated**: two requests that race past the
//! `get` lookup before the first `put` lands both run the handler;
//! `put` is first-writer-wins, so the second request's response is
//! discarded but the side effects of its handler invocation are not
//! rolled back. True at-most-once under contention requires a "pending
//! claim" record (followers wait or replay instead of running the
//! handler again) — tracked as a follow-up under idempotency backend
//! "Open Questions / Follow-ups".
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
//! Only `2xx` and `4xx` responses. `5xx` responses are passed through
//! uncached so transient backend failures do not pin a permanent error for
//! the TTL window. Responses larger than
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
