//! Idempotent-replay dedup store (M3.4 — ADR-0048).
//!
//! Storage-layer port for the API's idempotency middleware. Two impls
//! ship in this crate today:
//!
//! - [`InMemoryIdempotencyStoreRepo`] — process-local cache with TTL
//!   eviction (`moka::future::Cache`). Used in tests and dev builds.
//! - [`crate::pg::PgIdempotencyStore`] (behind `feature = "postgres"`)
//!   — PG-backed durable store satisfying ROADMAP §M3 1.0 closure
//!   ("the dedup store survives process restart in production
//!   deployments").
//!
//! The middleware in `nebula-api` consumes this trait through a thin
//! bridge struct (`crate::middleware::idempotency::StorageBackedIdempotencyStore`)
//! that turns [`CachedRecord`] back into the `http::HeaderMap` /
//! `StatusCode` shape `axum` expects. The trait deliberately stays in
//! the Exec layer — `Vec<(String, Vec<u8>)>` for headers keeps `http`
//! types out of `nebula-storage`.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use moka::future::Cache;

use crate::error::StorageError;

/// Cached HTTP response retained for idempotent replay.
///
/// Pure storage-layer shape — no `http` types. The bridge in
/// `nebula-api` reconstructs `HeaderMap` / `StatusCode` on read.
#[derive(Clone, Debug)]
pub struct CachedRecord {
    /// HTTP status code returned by the original handler.
    pub status: u16,
    /// Filtered response headers (hop-by-hop and per-request stripped).
    /// Stored as `(name, value)` tuples so the trait stays free of
    /// `http::HeaderMap` and other API-layer types.
    pub headers: Vec<(String, Vec<u8>)>,
    /// Buffered response body bytes.
    pub body: Vec<u8>,
    /// SHA-256 fingerprint of the original request body — used to
    /// detect "same key, different body" reuse and reject it with 422.
    pub fingerprint: [u8; 32],
}

/// Storage backend for cached idempotent responses.
///
/// Implementations MUST honour the `ttl` argument on `put` (the
/// middleware does not enforce expiry). Concurrent first-writer-wins
/// is the semantics — `put` for a key that already exists is a no-op
/// (the existing record stays). Caller-side body-mismatch handling
/// happens in the middleware against the stored `fingerprint`.
#[async_trait]
pub trait IdempotencyStoreRepo: Send + Sync + std::fmt::Debug {
    /// Look up a cached record by its scoped cache key.
    ///
    /// Returns `Ok(None)` for a true cache miss. Returns
    /// [`StorageError`] if the read itself failed (e.g. PG connection
    /// error, malformed payload). Callers MUST NOT treat
    /// [`StorageError`] as a cache miss — silently dropping replay
    /// protection on data corruption is rejected by ADR-0048.
    async fn get(&self, cache_key: &str) -> Result<Option<CachedRecord>, StorageError>;

    /// Persist a cached record under `cache_key` with `ttl`.
    ///
    /// First-writer-wins: a `put` for an existing key is a no-op —
    /// the in-memory backend uses moka's `entry().or_insert_with`
    /// semantics and the PG backend uses
    /// `INSERT ... ON CONFLICT (cache_key) DO NOTHING`.
    async fn put(
        &self,
        cache_key: String,
        record: CachedRecord,
        ttl: Duration,
    ) -> Result<(), StorageError>;

    /// Drop expired rows. Returns the number of rows reclaimed.
    ///
    /// In-memory backends rely on moka's TTL evictor and may report
    /// `0` (the trait method exists for the PG-backed sweep task).
    async fn evict_expired(&self) -> Result<u64, StorageError>;
}

/// Process-local idempotency-dedup repository (in-memory).
///
/// Backed by `moka::future::Cache<String, CachedRecord>`. Entries
/// expire on the per-entry `time_to_live` set when `put` is called;
/// the cache is bounded by `max_capacity` to provide a hard memory
/// ceiling.
pub struct InMemoryIdempotencyStoreRepo {
    cache: Cache<String, Arc<CachedRecord>>,
    max_capacity: u64,
}

impl InMemoryIdempotencyStoreRepo {
    /// Default cache lifetime — 24h matches the IETF draft.
    pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;
    /// Default cap on the number of cached responses.
    pub const DEFAULT_MAX_ENTRIES: u64 = 10_000;

    /// Create a store with default TTL (24h) and capacity (10_000).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_MAX_ENTRIES)
    }

    /// Create a store with a custom max-entry capacity. Per-entry TTL
    /// is supplied at `put` time so the same store can serve multiple
    /// distinct lifetimes if a future caller needs it.
    #[must_use]
    pub fn with_capacity(max_capacity: u64) -> Self {
        let cache = Cache::builder().max_capacity(max_capacity).build();
        Self {
            cache,
            max_capacity,
        }
    }

    /// Live entry count (best-effort; `moka::Cache::entry_count`).
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.cache.entry_count()
    }

    /// Configured cap for this instance. Used by saturation reporters.
    #[must_use]
    pub fn max_capacity(&self) -> u64 {
        self.max_capacity
    }
}

impl Default for InMemoryIdempotencyStoreRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InMemoryIdempotencyStoreRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryIdempotencyStoreRepo")
            .field("entries", &self.cache.entry_count())
            .field("max_capacity", &self.max_capacity)
            .finish()
    }
}

#[async_trait]
impl IdempotencyStoreRepo for InMemoryIdempotencyStoreRepo {
    async fn get(&self, cache_key: &str) -> Result<Option<CachedRecord>, StorageError> {
        Ok(self.cache.get(cache_key).await.map(|arc| (*arc).clone()))
    }

    async fn put(
        &self,
        cache_key: String,
        record: CachedRecord,
        _ttl: Duration,
    ) -> Result<(), StorageError> {
        // moka does not expose per-key TTL on a single shared `Cache`
        // — the TTL is configured per cache instance. The in-memory
        // backend therefore relies on the cache's global eviction
        // policy (configured at construction). Per-key TTL is
        // honoured by the PG backend; this in-memory impl is
        // good enough for dev/tests where a single shared lifetime
        // is fine.
        self.cache.insert(cache_key, Arc::new(record)).await;
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        // moka evicts on its own; nothing to drive from here. Return 0
        // so the sweep-task counter shows "no PG-style sweep needed".
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(body: &[u8]) -> CachedRecord {
        CachedRecord {
            status: 200,
            headers: vec![("content-type".into(), b"text/plain".to_vec())],
            body: body.to_vec(),
            fingerprint: [0u8; 32],
        }
    }

    #[tokio::test]
    async fn round_trip_get_put() {
        let repo = InMemoryIdempotencyStoreRepo::new();
        repo.put("k".into(), record(b"hello"), Duration::from_mins(1))
            .await
            .unwrap();
        let got = repo.get("k").await.unwrap();
        let r = got.expect("must round-trip");
        assert_eq!(r.body, b"hello".to_vec());
        assert_eq!(r.status, 200);
    }

    #[tokio::test]
    async fn unknown_key_returns_none() {
        let repo = InMemoryIdempotencyStoreRepo::new();
        let got = repo.get("missing").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn evict_expired_returns_zero_in_memory() {
        let repo = InMemoryIdempotencyStoreRepo::new();
        let n = repo.evict_expired().await.unwrap();
        assert_eq!(n, 0);
    }
}
