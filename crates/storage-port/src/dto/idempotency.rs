//! Idempotency dedup record DTO (hybrid replay cache).
use serde::{Deserialize, Serialize};

/// One cached idempotent-replay response.
///
/// Stored verbatim so a replayed request returns the original status +
/// headers + body. `fingerprint` binds the entry to the identity + body it
/// was first written for; `expires_at` drives the sweep. The owning
/// `cache_key` is tenant-namespaced by the decorator (`{scope}:{key}`), so a
/// cross-tenant probe can neither read nor poison another tenant's entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedRecord {
    /// HTTP status code of the cached response.
    pub status: u16,
    /// Serialized response headers.
    pub headers: Vec<u8>,
    /// Serialized response body.
    pub body: Vec<u8>,
    /// Identity + body fingerprint the entry was first written for.
    pub fingerprint: Vec<u8>,
    /// Expiry timestamp (RFC 3339); drives the eviction sweep.
    pub expires_at: String,
}
