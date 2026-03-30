//! Concurrent compute cache using `DashMap` for lock-free access
//!
//! This module provides a high-performance concurrent cache that uses `DashMap`
//! for lock-free read and write operations, eliminating `RwLock` contention.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;

use super::compute::{CacheEntry, CacheKey};
use super::config::CacheConfig;
use super::stats::{AtomicCacheStats, CacheStats, StatsProvider};
use crate::error::{MemoryError, MemoryResult};

/// A lock-free concurrent compute cache using `DashMap`
///
/// High-performance concurrent cache optimized for read-heavy workloads
/// with occasional writes (cache misses).
///
/// # Performance characteristics
///
/// - **Reads**: Lock-free, scales linearly with CPU cores
/// - **Writes**: Fine-grained locking per shard, minimal contention
/// - **Metrics**: Always-on atomic counters (`Relaxed` ordering — zero overhead on x86)
///
/// # Eviction
///
/// When the cache reaches `max_entries`, one **arbitrary** entry is removed to make
/// room. This is **not** LRU — no access metadata is tracked on reads, preserving
/// lock-free performance. For expression/template caches with high hit rates,
/// random eviction is sufficient.
///
/// # TTL
///
/// If [`CacheConfig::ttl`] is set, entries are lazily expired on read.
/// There is no background cleanup thread — expired entries linger until accessed.
///
/// # Thread safety
///
/// `compute_fn` in [`get_or_compute`](Self::get_or_compute) **may execute more
/// than once** for the same key under concurrent access. Only one result is stored.
/// Use only with idempotent compute functions.
///
/// # Honored `CacheConfig` fields
///
/// | Field | Used | Notes |
/// |-------|------|-------|
/// | `max_entries` | Yes | Hard capacity limit |
/// | `initial_capacity` | Yes | Pre-allocation hint for `DashMap` |
/// | `ttl` | Yes | Lazy expiry on reads |
/// | `policy` | No | Always random eviction |
/// | `track_metrics` | No | Metrics are always on |
/// | `load_factor` | No | `DashMap` manages its own load factor |
/// | `auto_cleanup` | No | No background cleanup |
/// | `cleanup_interval` | No | No background cleanup |
pub struct ConcurrentComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync,
{
    /// Lock-free concurrent map
    entries: Arc<DashMap<K, CacheEntry<V>>>,
    /// Cache configuration
    config: CacheConfig,
    /// Atomic statistics counters
    stats: Arc<AtomicCacheStats>,
}

impl<K, V> ConcurrentComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync,
{
    /// Create a new concurrent cache with the given maximum size
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self::with_config(CacheConfig::new(max_entries))
    }

    /// Create a new concurrent cache with the given configuration
    #[must_use]
    pub fn with_config(config: CacheConfig) -> Self {
        let initial_capacity = config.initial_capacity.unwrap_or(config.max_entries);

        Self {
            entries: Arc::new(DashMap::with_capacity(initial_capacity)),
            config,
            stats: Arc::new(AtomicCacheStats::new()),
        }
    }

    /// Get a value from cache without computing (fast lock-free read)
    ///
    /// Returns `None` if the key is absent or expired (when TTL is configured).
    /// Does **not** update access metadata — reads remain lock-free.
    pub fn get(&self, key: &K) -> Option<V> {
        let Some(entry_ref) = self.entries.get(key) else {
            self.stats.record_miss();
            return None;
        };

        // Lazy TTL expiry
        if self.is_expired(&entry_ref) {
            drop(entry_ref);
            self.entries.remove(key);
            self.stats.record_miss();
            return None;
        }

        self.stats.record_hit();
        Some(entry_ref.value().value.clone())
    }

    /// Get a value from cache, computing it if not present or expired
    ///
    /// On a cache miss (key absent or TTL-expired), `compute_fn` is called to
    /// produce the value. Under concurrent access, `compute_fn` **may be called
    /// more than once** for the same key — only one result is stored.
    ///
    /// # Errors
    ///
    /// Returns any error produced by `compute_fn`. Also returns
    /// [`MemoryError`] if eviction or insertion fails.
    pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> MemoryResult<V>
    where
        F: FnOnce() -> Result<V, MemoryError>,
    {
        // Fast path: check if already in cache (lock-free read)
        if let Some(entry) = self.entries.get(&key) {
            if !self.is_expired(&entry) {
                self.stats.record_hit();
                return Ok(entry.value().value.clone());
            }
            // Expired — drop ref and fall through to recompute
            drop(entry);
            self.entries.remove(&key);
        }

        // Cache miss
        self.stats.record_miss();

        // Evict if at capacity
        if self.entries.len() >= self.config.max_entries {
            self.evict_one();
        }

        // Compute the value
        let value = compute_fn()?;

        // Insert using entry API to handle concurrent insertion race
        let entry = self.entries.entry(key).or_insert_with(|| {
            self.stats.record_insertion(0);
            CacheEntry::new(value.clone())
        });

        Ok(entry.value().value.clone())
    }

    /// Insert a value directly without computation
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] if eviction fails when the cache is full.
    pub fn insert(&self, key: K, value: V) -> MemoryResult<()> {
        if self.entries.len() >= self.config.max_entries && !self.entries.contains_key(&key) {
            self.evict_one();
        }

        self.entries.insert(key, CacheEntry::new(value));
        self.stats.record_insertion(0);
        Ok(())
    }

    /// Check if a key exists in the cache (does not check TTL)
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Remove a specific key and return its value
    pub fn remove(&self, key: &K) -> Option<V> {
        self.entries.remove(key).map(|(_, entry)| {
            self.stats.record_deletion(0);
            entry.value
        })
    }

    /// Get the current number of entries in the cache
    ///
    /// Note: may include expired entries that haven't been lazily cleaned up yet.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries from the cache
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Get cache capacity (`max_entries`)
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.config.max_entries
    }

    /// Get a point-in-time snapshot of cache statistics
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.stats.snapshot()
    }

    /// Reset all statistics counters to zero
    pub fn reset_stats(&self) {
        self.stats.reset();
    }

    /// Get the configured TTL, if any
    #[must_use]
    pub fn ttl(&self) -> Option<Duration> {
        self.config.ttl
    }

    /// Check if an entry is expired based on configured TTL
    fn is_expired(&self, entry: &dashmap::mapref::one::Ref<'_, K, CacheEntry<V>>) -> bool {
        self.config
            .ttl
            .is_some_and(|ttl| entry.value().is_expired(ttl))
    }

    /// Remove one arbitrary entry to make room
    ///
    /// Uses an atomic flag with `retain` to remove exactly one entry,
    /// avoiding potential deadlocks from `iter()` + `remove()` on `DashMap`.
    fn evict_one(&self) {
        if self.entries.is_empty() {
            return;
        }

        let removed = std::sync::atomic::AtomicBool::new(false);
        self.entries.retain(|_, _| {
            if removed.load(std::sync::atomic::Ordering::Relaxed) {
                true // keep remaining entries
            } else {
                removed.store(true, std::sync::atomic::Ordering::Relaxed);
                self.stats.record_eviction();
                false // remove this one entry
            }
        });
    }
}

impl<K, V> StatsProvider for ConcurrentComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync,
{
    fn stats(&self) -> CacheStats {
        self.stats.snapshot()
    }

    fn reset_stats(&self) {
        self.stats.reset();
    }
}

// Implement Clone to share the cache across threads (entries + stats are Arc-shared)
impl<K, V> Clone for ConcurrentComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync,
{
    fn clone(&self) -> Self {
        Self {
            entries: Arc::clone(&self.entries),
            config: self.config.clone(),
            stats: Arc::clone(&self.stats),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn basic_get_or_compute() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        // First call should compute
        let result1 = cache.get_or_compute("key1".to_string(), || Ok(42));
        assert_eq!(result1.unwrap(), 42);

        // Second call should use cached value
        let result2 = cache.get_or_compute("key1".to_string(), || Ok(99));
        assert_eq!(result2.unwrap(), 42);
    }

    #[test]
    fn concurrent_access() {
        let cache = Arc::new(ConcurrentComputeCache::<String, i32>::new(100));
        let mut handles = vec![];

        for i in 0..10 {
            let cache_clone = Arc::clone(&cache);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    let key = format!("key_{}", j % 10);
                    let value = cache_clone.get_or_compute(key, || Ok(i * 100 + j));
                    assert!(value.is_ok());
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert!(!cache.is_empty());
        assert!(cache.len() <= 100);
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);
        assert_eq!(cache.get(&"key1".to_string()), None);

        cache.insert("key1".to_string(), 42).unwrap();
        assert_eq!(cache.get(&"key1".to_string()), Some(42));
    }

    #[test]
    fn eviction_respects_capacity() {
        let cache = ConcurrentComputeCache::<String, i32>::new(2);

        cache.get_or_compute("key1".to_string(), || Ok(1)).unwrap();
        cache.get_or_compute("key2".to_string(), || Ok(2)).unwrap();
        cache.get_or_compute("key3".to_string(), || Ok(3)).unwrap();

        assert!(cache.len() <= 2);
    }

    #[test]
    fn eviction_handles_many_insertions() {
        let cache = ConcurrentComputeCache::<String, i32>::new(5);

        for i in 0..20 {
            cache.get_or_compute(format!("key_{i}"), || Ok(i)).unwrap();
            assert!(cache.len() <= 5, "len={} after insert {i}", cache.len());
        }
    }

    #[test]
    fn metrics_basic() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        // Insert via get_or_compute (miss + insertion)
        cache.get_or_compute("a".to_string(), || Ok(1)).unwrap();
        // Hit via get_or_compute
        cache.get_or_compute("a".to_string(), || Ok(99)).unwrap();
        // Miss via get() for nonexistent key
        cache.get(&"nonexistent".to_string());
        // Hit via get() for existing key
        cache.get(&"a".to_string());

        let s = cache.stats();
        assert_eq!(s.hits, 2, "expected 2 hits, got {}", s.hits);
        assert_eq!(s.misses, 2, "expected 2 misses, got {}", s.misses);
        assert!(
            s.insertions >= 1,
            "expected >=1 insertion, got {}",
            s.insertions
        );
    }

    #[test]
    fn metrics_eviction_counted() {
        let cache = ConcurrentComputeCache::<String, i32>::new(2);

        cache.get_or_compute("a".to_string(), || Ok(1)).unwrap();
        cache.get_or_compute("b".to_string(), || Ok(2)).unwrap();
        cache.get_or_compute("c".to_string(), || Ok(3)).unwrap();

        let s = cache.stats();
        assert!(
            s.evictions >= 1,
            "expected >=1 eviction, got {}",
            s.evictions
        );
    }

    #[test]
    fn ttl_expiry_on_get() {
        let config = CacheConfig::new(10).with_ttl(Duration::from_millis(50));
        let cache = ConcurrentComputeCache::<String, i32>::with_config(config);

        cache.insert("k".to_string(), 42).unwrap();
        assert_eq!(cache.get(&"k".to_string()), Some(42));

        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            cache.get(&"k".to_string()),
            None,
            "entry should have expired"
        );

        let s = cache.stats();
        assert!(s.hits >= 1, "first get should be a hit");
        assert!(s.misses >= 1, "expired get should be a miss");
    }

    #[test]
    fn ttl_not_expired_returns_value() {
        let config = CacheConfig::new(10).with_ttl(Duration::from_secs(10));
        let cache = ConcurrentComputeCache::<String, i32>::with_config(config);

        cache.insert("k".to_string(), 42).unwrap();
        assert_eq!(cache.get(&"k".to_string()), Some(42));
    }

    #[test]
    fn get_or_compute_recomputes_after_ttl() {
        let config = CacheConfig::new(10).with_ttl(Duration::from_millis(50));
        let cache = ConcurrentComputeCache::<String, i32>::with_config(config);

        let v1 = cache.get_or_compute("k".to_string(), || Ok(1)).unwrap();
        assert_eq!(v1, 1);

        thread::sleep(Duration::from_millis(100));

        let v2 = cache.get_or_compute("k".to_string(), || Ok(2)).unwrap();
        assert_eq!(v2, 2, "should recompute after TTL expiry");
    }

    #[test]
    fn no_ttl_never_expires() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        cache.insert("k".to_string(), 42).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(cache.get(&"k".to_string()), Some(42));
    }

    #[test]
    fn concurrent_metrics_consistency() {
        let cache = Arc::new(ConcurrentComputeCache::<String, i32>::new(1000));
        let mut handles = vec![];

        for i in 0..10 {
            let c = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for j in 0..100 {
                    let key = format!("key_{}", j % 20);
                    c.get_or_compute(key, || Ok(i * 100 + j)).unwrap();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let s = cache.stats();
        assert_eq!(
            s.hits + s.misses,
            1000,
            "hits({}) + misses({}) should equal 1000",
            s.hits,
            s.misses
        );
    }

    #[test]
    fn stats_reset() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        cache.get_or_compute("a".to_string(), || Ok(1)).unwrap();
        cache.get_or_compute("a".to_string(), || Ok(1)).unwrap();

        assert!(cache.stats().hits > 0);

        cache.reset_stats();
        let s = cache.stats();
        assert_eq!(s.hits, 0);
        assert_eq!(s.misses, 0);
        assert_eq!(s.evictions, 0);
        assert_eq!(s.insertions, 0);
    }

    #[test]
    fn remove_tracks_deletion() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        cache.insert("k".to_string(), 42).unwrap();
        let removed = cache.remove(&"k".to_string());

        assert_eq!(removed, Some(42));
        assert_eq!(cache.stats().deletions, 1);
    }

    #[test]
    fn stats_provider_trait() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);
        cache.insert("k".to_string(), 42).unwrap();

        // Use trait method
        let s: CacheStats = StatsProvider::stats(&cache);
        assert!(s.insertions >= 1);

        StatsProvider::reset_stats(&cache);
        let s = StatsProvider::stats(&cache);
        assert_eq!(s.insertions, 0);
    }
}
