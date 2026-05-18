//! Process-local idempotency store backed by [`moka::future::Cache`].
//!
//! [`InMemoryIdempotencyStore`] is the dev / single-process default backend.
//! Entries expire after a configurable TTL and the cache is bounded by a
//! maximum entry count to provide a hard memory ceiling. For production
//! deployments running more than one API replica, use
//! [`super::store::StorageBackedIdempotencyStore`] (PG-backed) instead —
//! the in-memory store loses dedup state across restarts and across runners
//! (idempotency backend).

use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use moka::future::Cache;

use super::{
    DEFAULT_MAX_ENTRIES, DEFAULT_TTL_SECS,
    store::{CachedResponse, IdempotencyStore, IdempotencyStoreError},
};

/// Process-local idempotency store backed by [`moka::future::Cache`].
///
/// Entries expire after a configurable TTL and the cache is bounded by a
/// maximum entry count to provide a hard memory ceiling.
pub struct InMemoryIdempotencyStore {
    pub(super) cache: Cache<String, Arc<CachedResponse>>,
    max_entries: u64,
}

impl InMemoryIdempotencyStore {
    /// Create a store with the default TTL ([`DEFAULT_TTL_SECS`]) and capacity
    /// ([`DEFAULT_MAX_ENTRIES`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_ttl_and_capacity(Duration::from_secs(DEFAULT_TTL_SECS), DEFAULT_MAX_ENTRIES)
    }

    /// Create a store with a custom TTL and capacity.
    #[must_use]
    pub fn with_ttl_and_capacity(ttl: Duration, max_entries: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(ttl)
            .max_capacity(max_entries)
            .build();
        Self { cache, max_entries }
    }
}

impl Default for InMemoryIdempotencyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryIdempotencyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryIdempotencyStore")
            .field("entries", &self.cache.entry_count())
            .finish()
    }
}

#[async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn get(&self, key: &str) -> Result<Option<Arc<CachedResponse>>, IdempotencyStoreError> {
        Ok(self.cache.get(key).await)
    }

    async fn put(
        &self,
        key: String,
        response: Arc<CachedResponse>,
    ) -> Result<(), IdempotencyStoreError> {
        self.cache.insert(key, response).await;
        Ok(())
    }

    fn store_kind(&self) -> &'static str {
        "in-memory"
    }

    fn saturation_ppm(&self) -> Option<u64> {
        if self.max_entries == 0 {
            return None;
        }
        // `entry_count` is `u64`; multiply first, then divide, capping at
        // 1_000_000 to keep the gauge inside the `0..=1_000_000` ppm range
        // even if `moka` over-counts under contention (it is documented as
        // a best-effort approximation).
        let ppm = self
            .cache
            .entry_count()
            .saturating_mul(1_000_000)
            .saturating_div(self.max_entries);
        Some(ppm.min(1_000_000))
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::{HeaderMap, StatusCode};

    use super::*;

    #[tokio::test]
    async fn in_memory_store_round_trip() {
        let store = InMemoryIdempotencyStore::new();
        let resp = Arc::new(CachedResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: b"hello".to_vec(),
            request_fingerprint: [0u8; 32],
        });
        store
            .put("k1".to_owned(), Arc::clone(&resp))
            .await
            .expect("in-memory put never errors");
        let fetched = store
            .get("k1")
            .await
            .expect("get must not error")
            .expect("entry must be present");
        assert_eq!(fetched.body, b"hello");
        assert!(
            store
                .get("missing")
                .await
                .expect("get must not error")
                .is_none()
        );
    }

    #[tokio::test]
    async fn in_memory_store_honours_ttl() {
        let store = InMemoryIdempotencyStore::with_ttl_and_capacity(Duration::from_millis(50), 16);
        let resp = Arc::new(CachedResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Vec::new(),
            request_fingerprint: [0u8; 32],
        });
        store.put("expiring".to_owned(), resp).await.expect("put");
        assert!(store.get("expiring").await.expect("get").is_some());
        tokio::time::sleep(Duration::from_millis(120)).await;
        // moka may need a tick to evict; force a sync.
        store.cache.run_pending_tasks().await;
        assert!(store.get("expiring").await.expect("get").is_none());
    }
}
