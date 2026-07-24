//! Idempotency dedup record DTO (hybrid replay cache).
use serde::{Deserialize, Serialize};

/// One cached idempotent-replay response.
///
/// Stored verbatim so a replayed request returns the original status +
/// headers + body. `fingerprint` binds the entry to the identity + body it
/// was first written for; `expires_at` drives the sweep. The owning
/// `cache_key` is tenant-namespaced by the decorator (`{scope}:{key}`), so a
/// cross-tenant probe can neither read nor poison another tenant's entry.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl std::fmt::Debug for CachedRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CachedRecord")
            .field("status", &self.status)
            .field("headers", &"[redacted]")
            .field(
                "body",
                &format_args!("[redacted; {} bytes]", self.body.len()),
            )
            .field("fingerprint", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::CachedRecord;

    #[test]
    fn cached_record_debug_redacts_headers_body_and_fingerprint() {
        const CANARY: &[u8] = b"IDEMPOTENCY_SECRET_CANARY-8c4d";
        let record = CachedRecord {
            status: 200,
            headers: CANARY.to_vec(),
            body: CANARY.to_vec(),
            fingerprint: CANARY.to_vec(),
            expires_at: "2026-07-21T00:00:00Z".to_owned(),
        };

        let debug = format!("{record:?}");
        assert!(debug.contains("CachedRecord"));
        assert!(!debug.contains("IDEMPOTENCY_SECRET_CANARY"));
    }
}
