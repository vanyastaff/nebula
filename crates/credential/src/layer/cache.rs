//! Caching layer for credential storage.
//!
//! Wraps any [`CredentialStore`] with a moka LRU + TTL cache.
//! Caches [`StoredCredential`] including ciphertext data — the cache
//! sits below `EncryptionLayer` in the layer stack, so it never holds
//! plaintext secrets.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use moka::future::Cache;

use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Configuration for the credential cache.
///
/// # Examples
///
/// ```
/// use nebula_credential::CacheConfig as StoreCacheConfig;
///
/// let config = StoreCacheConfig {
///     max_entries: 5_000,
///     ttl: std::time::Duration::from_secs(600),
///     tti: std::time::Duration::from_secs(300),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of cached entries. Default: 10,000.
    pub max_entries: u64,
    /// Time-to-live for cached entries. Default: 5 minutes.
    pub ttl: Duration,
    /// Time-to-idle — evict after this duration of inactivity. Default: 2 minutes.
    pub tti: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            ttl: Duration::from_secs(300),
            tti: Duration::from_secs(120),
        }
    }
}

/// Cache performance statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
}

impl CacheStats {
    /// Calculate cache hit rate as a fraction (0.0 – 1.0).
    ///
    /// Returns 0.0 if no requests have been recorded.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::layer::cache::CacheStats;
    ///
    /// let stats = CacheStats {
    ///     hits: 80,
    ///     misses: 20,
    /// };
    /// assert!((stats.hit_rate() - 0.8).abs() < f64::EPSILON);
    /// ```
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Caching layer wrapping a [`CredentialStore`].
///
/// Sits below [`EncryptionLayer`](crate::layer::EncryptionLayer) in the
/// layer stack — cached values are **ciphertext**, never plaintext secrets.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::{InMemoryStore, layer::cache::{CacheLayer, CacheConfig}};
///
/// let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
/// ```
pub struct CacheLayer<S> {
    /// The wrapped inner store.
    inner: S,
    /// Moka cache instance.
    cache: Cache<String, StoredCredential>,
    /// Cache hit counter.
    hits: AtomicU64,
    /// Cache miss counter.
    misses: AtomicU64,
}

impl<S> CacheLayer<S> {
    /// Create a new caching layer wrapping the given store.
    pub fn new(inner: S, config: CacheConfig) -> Self {
        let cache = Cache::builder()
            .max_capacity(config.max_entries)
            .time_to_live(config.ttl)
            .time_to_idle(config.tti)
            .build();

        Self {
            inner,
            cache,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Returns cache hit/miss statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    /// Invalidates a specific cache entry.
    pub async fn invalidate(&self, id: &str) {
        self.cache.invalidate(id).await;
    }

    /// Invalidates all cache entries.
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }
}

impl<S: CredentialStore> CredentialStore for CacheLayer<S> {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        if let Some(cached) = self.cache.get(id).await {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached);
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        let credential = self.inner.get(id).await?;
        self.cache.insert(id.to_string(), credential.clone()).await;
        Ok(credential)
    }

    async fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        // Invalidate before write to prevent stale reads during the write.
        self.cache.invalidate(&credential.id).await;
        let stored = self.inner.put(credential, mode).await?;
        self.cache.insert(stored.id.clone(), stored.clone()).await;
        Ok(stored)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.cache.invalidate(id).await;
        self.inner.delete(id).await
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        // Pass through — list results are too dynamic to cache.
        self.inner.list(state_kind).await
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        if self.cache.get(id).await.is_some() {
            return Ok(true);
        }
        self.inner.exists(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::{PutMode, test_helpers::make_credential},
        store_memory::InMemoryStore,
    };

    #[tokio::test]
    async fn cache_hit_returns_cached() {
        let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
        let cred = make_credential("c1", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // First get — cache was populated by put, so this is a hit.
        let first = store.get("c1").await.unwrap();
        assert_eq!(first.data, b"data");

        // Second get — definitely a cache hit.
        let second = store.get("c1").await.unwrap();
        assert_eq!(second.data, b"data");

        assert!(store.stats().hits >= 1);
    }

    #[tokio::test]
    async fn put_invalidates_and_caches_new_value() {
        let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
        let cred = make_credential("c1", b"v1");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Read to populate cache.
        let _ = store.get("c1").await.unwrap();

        // Overwrite with new data.
        let updated = make_credential("c1", b"v2");
        store.put(updated, PutMode::Overwrite).await.unwrap();

        // Should see the new data (not stale cache).
        let fetched = store.get("c1").await.unwrap();
        assert_eq!(fetched.data, b"v2");
    }

    #[tokio::test]
    async fn delete_invalidates_cache() {
        let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
        let cred = make_credential("c1", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Populate cache.
        let _ = store.get("c1").await.unwrap();

        // Delete.
        store.delete("c1").await.unwrap();

        // Should be gone.
        let err = store.get("c1").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn stats_track_hits_and_misses() {
        let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
        let cred = make_credential("c1", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Miss — not yet read via get.
        store.invalidate("c1").await;
        let _ = store.get("c1").await.unwrap();
        assert_eq!(store.stats().misses, 1);

        // Hit — now cached.
        let _ = store.get("c1").await.unwrap();
        assert_eq!(store.stats().hits, 1);
    }

    #[tokio::test]
    async fn exists_uses_cache() {
        let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
        let cred = make_credential("c1", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Populate cache via get.
        let _ = store.get("c1").await.unwrap();

        // exists should return true from cache.
        assert!(store.exists("c1").await.unwrap());

        // Non-existent should fall through to inner.
        assert!(!store.exists("missing").await.unwrap());
    }

    #[tokio::test]
    async fn list_passes_through() {
        let store = CacheLayer::new(InMemoryStore::new(), CacheConfig::default());
        let cred = make_credential("c1", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let ids = store.list(None).await.unwrap();
        assert_eq!(ids, vec!["c1"]);

        let filtered = store.list(Some("test")).await.unwrap();
        assert_eq!(filtered, vec!["c1"]);

        let empty = store.list(Some("nonexistent")).await.unwrap();
        assert!(empty.is_empty());
    }
}
