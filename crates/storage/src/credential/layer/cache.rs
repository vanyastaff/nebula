//! Caching layer for credential storage.
//!
//! Wraps any [`CredentialPersistence`] with a moka LRU + TTL cache.
//! First-party deployment composition deliberately does not install this
//! process-local layer: without a coherent durable invalidation protocol it
//! cannot observe revocation performed by another replica. It is suitable
//! only for explicitly single-process compositions, and must sit below
//! `EncryptionLayer` so cached data is ciphertext.

use std::{
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{Hash, Hasher},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use async_trait::async_trait;
use moka::future::Cache;
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialCommit, CredentialCreate, CredentialOwner, CredentialPersistence,
    CredentialPersistenceError, CredentialRecordState, CredentialReplacement, CredentialSelector,
    CredentialTombstone, StoredCredential, StoredCredentialHead,
};
use tokio::sync::Mutex;

/// Fixed-size lock striping keeps same-selector cache fills and mutations
/// linearizable without retaining one lock per attacker-controlled key.
const LOCK_SHARD_COUNT: usize = 64;

/// Configuration for the credential cache.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_storage::credential::CacheConfig;
///
/// let config = CacheConfig {
///     max_entries: 5_000,
///     ttl: Duration::from_secs(600),
///     tti: Duration::from_secs(300),
/// };
/// assert_eq!(config.max_entries, 5_000);
/// assert_eq!(config.ttl, Duration::from_secs(600));
/// assert_eq!(config.tti, Duration::from_secs(300));
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
            ttl: Duration::from_mins(5),
            tti: Duration::from_mins(2),
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
    /// ```rust
    /// use nebula_storage::credential::CacheStats;
    ///
    /// let stats = CacheStats {
    ///     hits: 80,
    ///     misses: 20,
    /// };
    /// assert!((stats.hit_rate() - 0.8).abs() < f64::EPSILON);
    ///
    /// // No requests recorded -> 0.0, not a division-by-zero.
    /// let empty = CacheStats { hits: 0, misses: 0 };
    /// assert_eq!(empty.hit_rate(), 0.0);
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

/// Caching layer wrapping a [`CredentialPersistence`].
///
/// Sits below [`EncryptionLayer`](super::EncryptionLayer) in the
/// layer stack — cached values are **ciphertext**, never plaintext secrets.
/// Do not use this layer in a multi-replica composition without a durable,
/// source-of-truth coherence protocol.
///
/// # Examples
///
/// Wiring the cache over a live SQLite backend (requires the `sqlite`
/// feature; the async `connect` makes this `no_run`):
///
/// ```rust,no_run
/// # #[cfg(feature = "sqlite")]
/// # async fn doc() -> Result<(), Box<dyn std::error::Error>> {
/// use nebula_storage::credential::{CacheConfig, CacheLayer, SqliteCredentialPersistence};
///
/// let backend = SqliteCredentialPersistence::connect("sqlite://creds.db").await?;
/// let store = CacheLayer::new(backend, CacheConfig::default());
/// # let _ = store;
/// # Ok(())
/// # }
/// ```
pub struct CacheLayer<S> {
    /// The wrapped inner store.
    inner: S,
    /// Moka cache instance.
    cache: Cache<CredentialSelector, StoredCredential>,
    /// Cache hit counter.
    hits: AtomicU64,
    /// Cache miss counter.
    misses: AtomicU64,
    /// Bounded same-selector serialization for fill/write/delete races.
    locks: Box<[Mutex<()>]>,
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
            locks: (0..LOCK_SHARD_COUNT)
                .map(|_| Mutex::new(()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
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
    pub async fn invalidate(&self, selector: &CredentialSelector) {
        let _guard = self.lock(selector).lock().await;
        self.cache.invalidate(selector).await;
    }

    fn lock(&self, selector: &CredentialSelector) -> &Mutex<()> {
        let mut hasher = DefaultHasher::new();
        selector.hash(&mut hasher);
        let shard = usize::from(hasher.finish().to_ne_bytes()[0]) % self.locks.len();
        &self.locks[shard]
    }
}

impl<S> fmt::Debug for CacheLayer<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("CacheLayer").finish_non_exhaustive()
    }
}

#[async_trait]
impl<S: CredentialPersistence> CredentialPersistence for CacheLayer<S> {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        // Keep a fill serialized with writes/deletes for this selector. Without
        // this guard, a miss can fetch v1, a concurrent write can publish v2,
        // and the delayed miss can then reinsert stale v1 after invalidation.
        let _guard = self.lock(selector).lock().await;
        if let Some(cached) = self.cache.get(selector).await {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached);
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        let credential = self.inner.get(selector).await?;
        self.cache
            .insert(selector.clone(), credential.clone())
            .await;
        Ok(credential)
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let _guard = self.lock(selector).lock().await;
        self.inner.get_head(selector).await
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let _guard = self.lock(selector).lock().await;
        // Clear before I/O. If the caller is cancelled after the durable
        // operation, the cache remains empty rather than known-stale. The
        // secret-free commit cannot populate a physical-record cache without a
        // forbidden post-write read.
        self.cache.invalidate(selector).await;
        self.inner.create(selector, create).await
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let _guard = self.lock(selector).lock().await;
        self.cache.invalidate(selector).await;
        self.inner.replace(selector, replacement).await
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let _guard = self.lock(selector).lock().await;
        self.cache.invalidate(selector).await;
        self.inner.tombstone(selector, tombstone).await
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        // Pass through — list results are too dynamic to cache.
        self.inner.list(owner, state_kind).await
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        self.inner.list_heads(owner, state_kind).await
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        let _guard = self.lock(selector).lock().await;
        if let Some(cached) = self.cache.get(selector).await {
            return Ok(cached.state() == CredentialRecordState::Live);
        }
        self.inner.exists(selector).await
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::sync::{Arc, atomic::AtomicBool};

    use async_trait::async_trait;
    use nebula_core::CredentialId;
    use nebula_storage_port::{
        CredentialCommit, CredentialCreate, CredentialOwner, CredentialPersistence,
        CredentialPersistenceError, CredentialReplacement, CredentialSelector, CredentialTombstone,
        CredentialVersion, StoredCredential, StoredCredentialHead, StoredLiveCredential,
    };
    use tokio::sync::Notify;

    use crate::credential::test_support::{make_credential, make_replacement};

    use super::{super::super::sqlite::SqliteCredentialPersistence, *};

    fn owner() -> CredentialOwner {
        CredentialOwner::from_canonical("test-owner")
    }

    fn selector(id: CredentialId) -> CredentialSelector {
        CredentialSelector::new(owner(), id)
    }

    fn selector_for(owner: &str, id: CredentialId) -> CredentialSelector {
        CredentialSelector::new(CredentialOwner::from_canonical(owner), id)
    }

    fn version(value: i64) -> CredentialVersion {
        CredentialVersion::try_from(value).expect("test version must be valid")
    }

    fn into_live(record: StoredCredential) -> StoredLiveCredential {
        let StoredCredential::Live(record) = record else {
            panic!("test fixture must remain live");
        };
        record
    }

    #[derive(Debug, Default)]
    struct ReadGate {
        delay_next_get: AtomicBool,
        snapshot_captured: Notify,
        release_snapshot: Notify,
    }

    #[derive(Debug)]
    struct DelayedReadPersistence<S> {
        inner: S,
        gate: Arc<ReadGate>,
    }

    #[async_trait]
    impl<S: CredentialPersistence> CredentialPersistence for DelayedReadPersistence<S> {
        async fn get(
            &self,
            selector: &CredentialSelector,
        ) -> Result<StoredCredential, CredentialPersistenceError> {
            let snapshot = self.inner.get(selector).await?;
            if self.gate.delay_next_get.swap(false, Ordering::SeqCst) {
                self.gate.snapshot_captured.notify_one();
                self.gate.release_snapshot.notified().await;
            }
            Ok(snapshot)
        }

        async fn get_head(
            &self,
            selector: &CredentialSelector,
        ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
            self.inner.get_head(selector).await
        }

        async fn create(
            &self,
            selector: &CredentialSelector,
            create: CredentialCreate,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            self.inner.create(selector, create).await
        }

        async fn replace(
            &self,
            selector: &CredentialSelector,
            replacement: CredentialReplacement,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            self.inner.replace(selector, replacement).await
        }

        async fn tombstone(
            &self,
            selector: &CredentialSelector,
            tombstone: CredentialTombstone,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            self.inner.tombstone(selector, tombstone).await
        }

        async fn list(
            &self,
            owner: &CredentialOwner,
            state_kind: Option<&str>,
        ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
            self.inner.list(owner, state_kind).await
        }

        async fn list_heads(
            &self,
            owner: &CredentialOwner,
            state_kind: Option<&str>,
        ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
            self.inner.list_heads(owner, state_kind).await
        }

        async fn exists(
            &self,
            selector: &CredentialSelector,
        ) -> Result<bool, CredentialPersistenceError> {
            self.inner.exists(selector).await
        }
    }

    #[tokio::test]
    async fn cache_hit_returns_cached() -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let selector = selector(CredentialId::new());
        store.create(&selector, make_credential(b"data")).await?;

        // Secret-free mutation commits leave the physical-record cache empty.
        let first = into_live(store.get(&selector).await?);
        assert_eq!(first.data().as_ref(), b"data");

        let second = into_live(store.get(&selector).await?);
        assert_eq!(second.data().as_ref(), b"data");

        assert_eq!(store.stats(), CacheStats { hits: 1, misses: 1 });
        Ok(())
    }

    #[tokio::test]
    async fn replace_invalidates_cached_value() -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let selector = selector(CredentialId::new());
        let created = store.create(&selector, make_credential(b"v1")).await?;

        // Read to populate cache.
        let _ = store.get(&selector).await?;

        // Replace with new data.
        store
            .replace(&selector, make_replacement(created.version(), b"v2"))
            .await?;

        // Should see the new data (not stale cache).
        let fetched = into_live(store.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"v2");
        Ok(())
    }

    #[tokio::test]
    async fn tombstone_invalidates_cached_live_record() -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let selector = selector(CredentialId::new());
        let created = store.create(&selector, make_credential(b"data")).await?;

        // Populate cache.
        let _ = store.get(&selector).await?;

        store
            .tombstone(&selector, CredentialTombstone::new(created.version()))
            .await?;

        assert!(matches!(
            store.get(&selector).await?,
            StoredCredential::Tombstoned(_)
        ));
        assert!(!store.exists(&selector).await?);
        Ok(())
    }

    #[tokio::test]
    async fn stats_track_hits_and_misses() -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let selector = selector(CredentialId::new());
        store.create(&selector, make_credential(b"data")).await?;

        // Miss — not yet read via get.
        store.invalidate(&selector).await;
        let _ = store.get(&selector).await?;
        assert_eq!(store.stats().misses, 1);

        // Hit — now cached.
        let _ = store.get(&selector).await?;
        assert_eq!(store.stats().hits, 1);
        Ok(())
    }

    #[tokio::test]
    async fn exists_uses_cache() -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let primary_selector = selector(CredentialId::new());
        store
            .create(&primary_selector, make_credential(b"data"))
            .await?;

        // Populate cache via get.
        let _ = store.get(&primary_selector).await?;

        // exists should return true from cache.
        assert!(store.exists(&primary_selector).await?);

        // Non-existent should fall through to inner.
        assert!(!store.exists(&selector(CredentialId::new())).await?);
        Ok(())
    }

    #[tokio::test]
    async fn list_passes_through() -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let credential_id = CredentialId::new();
        store
            .create(&selector(credential_id), make_credential(b"data"))
            .await?;

        let ids = store.list(&owner(), None).await?;
        assert_eq!(ids, vec![credential_id]);

        let filtered = store.list(&owner(), Some("test")).await?;
        assert_eq!(filtered, vec![credential_id]);

        let empty = store.list(&owner(), Some("nonexistent")).await?;
        assert!(empty.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn cached_row_is_isolated_from_same_id_under_another_owner()
    -> Result<(), CredentialPersistenceError> {
        let store = CacheLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            CacheConfig::default(),
        );
        let credential_id = CredentialId::new();
        let owner_a = selector_for("owner-a", credential_id);
        let owner_b = selector_for("owner-b", credential_id);

        store
            .create(&owner_a, make_credential(b"owner-a-secret"))
            .await?;
        let cached = into_live(store.get(&owner_a).await?);
        assert_eq!(cached.data().as_ref(), b"owner-a-secret");
        let stats_before_foreign_read = store.stats();

        let foreign_read = store.get(&owner_b).await;
        assert_eq!(foreign_read, Err(CredentialPersistenceError::NotFound));
        assert_eq!(
            store.stats().misses,
            stats_before_foreign_read.misses + 1,
            "owner B must not hit owner A's same-id cache entry"
        );
        assert!(!store.exists(&owner_b).await?);

        let foreign_tombstone = store
            .tombstone(&owner_b, CredentialTombstone::new(version(1)))
            .await;
        assert_eq!(foreign_tombstone, Err(CredentialPersistenceError::NotFound));
        let foreign_replace = store
            .replace(&owner_b, make_replacement(version(1), b"owner-b-write"))
            .await;
        assert_eq!(foreign_replace, Err(CredentialPersistenceError::NotFound));

        let survivor = into_live(store.get(&owner_a).await?);
        assert_eq!(survivor.data().as_ref(), b"owner-a-secret");
        assert_eq!(survivor.version(), version(1));
        Ok(())
    }

    #[tokio::test]
    async fn delayed_cache_fill_cannot_overwrite_a_concurrent_write()
    -> Result<(), CredentialPersistenceError> {
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        let race_selector = selector(CredentialId::new());
        inner.create(&race_selector, make_credential(b"v1")).await?;

        let gate = Arc::new(ReadGate::default());
        gate.delay_next_get.store(true, Ordering::SeqCst);
        let store = Arc::new(CacheLayer::new(
            DelayedReadPersistence {
                inner,
                gate: Arc::clone(&gate),
            },
            CacheConfig::default(),
        ));

        let reader_store = Arc::clone(&store);
        let reader_selector = race_selector.clone();
        let reader = tokio::spawn(async move { reader_store.get(&reader_selector).await });
        gate.snapshot_captured.notified().await;

        assert!(
            store.lock(&race_selector).try_lock().is_err(),
            "the selector shard must remain locked while a miss is being filled"
        );

        let writer_started = Arc::new(Notify::new());
        let writer_store = Arc::clone(&store);
        let writer_selector = race_selector.clone();
        let writer_started_signal = Arc::clone(&writer_started);
        let writer = tokio::spawn(async move {
            writer_started_signal.notify_one();
            writer_store
                .replace(&writer_selector, make_replacement(version(1), b"v2"))
                .await
        });
        writer_started.notified().await;
        tokio::task::yield_now().await;
        assert!(
            !writer.is_finished(),
            "the write must wait until the in-flight cache fill releases its shard"
        );

        gate.release_snapshot.notify_one();
        let delayed_read = into_live(reader.await.expect("reader task must not panic")?);
        assert_eq!(delayed_read.data().as_ref(), b"v1");
        let written = writer.await.expect("writer task must not panic")?;
        assert_eq!(written.version(), version(2));

        let final_read = into_live(store.get(&race_selector).await?);
        assert_eq!(final_read.data().as_ref(), b"v2");
        assert_eq!(final_read.version(), version(2));
        Ok(())
    }
}
