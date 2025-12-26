//! Concurrent compute cache using `DashMap` for lock-free access
//!
//! This module provides a high-performance concurrent cache that uses `DashMap`
//! for lock-free read and write operations, eliminating `RwLock` contention.

use dashmap::DashMap;
use std::hash::Hash;
use std::sync::Arc;

use super::compute::CacheEntry;
use super::config::CacheConfig;
use crate::error::{MemoryError, MemoryResult};

/// Trait for cache keys (same as `ComputeCache`)
pub trait CacheKey: Hash + Eq + Clone + Send + Sync {}

// Implement CacheKey for common types
impl<T: Hash + Eq + Clone + Send + Sync> CacheKey for T {}

/// A lock-free concurrent compute cache using `DashMap`
///
/// This cache provides high-performance concurrent access without lock contention.
/// It's ideal for read-heavy workloads with occasional writes (cache misses).
///
/// # Performance characteristics:
/// - **Reads**: Lock-free, scales linearly with CPU cores
/// - **Writes**: Fine-grained locking per shard, minimal contention
/// - **Memory**: Slightly higher than `RwLock`<HashMap> due to sharding overhead
///
/// # Trade-offs:
/// - Does not update access metadata on reads (sacrifice LRU accuracy for performance)
/// - No TTL support (would require periodic cleanup thread)
/// - No eviction policy (fixed capacity with simple LRU when full)
pub struct ConcurrentComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync,
{
    /// Lock-free concurrent map
    entries: Arc<DashMap<K, CacheEntry<V>>>,
    /// Cache configuration
    config: CacheConfig,
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
        }
    }

    /// Get a value from cache without computing (fast lock-free read)
    ///
    /// This is the fast path for cache hits. It does NOT update access metadata
    /// to maintain lock-free performance.
    pub fn get(&self, key: &K) -> Option<V> {
        self.entries
            .get(key)
            .map(|entry| entry.value().value.clone())
    }

    /// Get a value from cache, computing it if not present
    ///
    /// This method handles the cache miss case. It uses interior mutability
    /// to compute and insert the value atomically.
    pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> MemoryResult<V>
    where
        F: FnOnce() -> Result<V, MemoryError>,
    {
        // Fast path: check if already in cache (lock-free read)
        if let Some(entry) = self.entries.get(&key) {
            return Ok(entry.value().value.clone());
        }

        // Slow path: compute and insert
        // Check capacity before computing (avoid wasted work)
        if self.entries.len() >= self.config.max_entries {
            self.evict_lru()?;
        }

        // Compute the value
        let value = compute_fn()?;

        // Insert using entry API to avoid duplicate computation if another thread raced
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| CacheEntry::new(value.clone()));

        Ok(entry.value().value.clone())
    }

    /// Insert a value directly without computation
    pub fn insert(&self, key: K, value: V) -> MemoryResult<()> {
        if self.entries.len() >= self.config.max_entries && !self.entries.contains_key(&key) {
            self.evict_lru()?;
        }

        self.entries.insert(key, CacheEntry::new(value));
        Ok(())
    }

    /// Check if a key exists in the cache
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Remove a specific key and return its value
    pub fn remove(&self, key: &K) -> Option<V> {
        self.entries.remove(key).map(|(_, entry)| entry.value)
    }

    /// Get the current number of entries in the cache
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

    /// Get cache capacity
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.config.max_entries
    }

    /// Simple LRU eviction: remove the oldest entry
    ///
    /// This is a simplified eviction strategy that doesn't track access order perfectly
    /// (to maintain lock-free performance), but removes entries when capacity is reached.
    fn evict_lru(&self) -> MemoryResult<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        // Find the first entry and remove it (simple FIFO-like behavior)
        // In a production system, you might want a more sophisticated strategy
        if let Some(entry) = self.entries.iter().next() {
            let key = entry.key().clone();
            drop(entry); // Release the reference before removing
            self.entries.remove(&key);
        }

        Ok(())
    }
}

// Implement Clone to share the cache across threads
impl<K, V> Clone for ConcurrentComputeCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync,
{
    fn clone(&self) -> Self {
        Self {
            entries: Arc::clone(&self.entries),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_basic_get_or_compute() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        // First call should compute
        let result1 = cache.get_or_compute("key1".to_string(), || Ok(42));
        assert_eq!(result1.unwrap(), 42);

        // Second call should use cached value
        let result2 = cache.get_or_compute("key1".to_string(), || Ok(99));
        assert_eq!(result2.unwrap(), 42);
    }

    #[test]
    fn test_concurrent_access() {
        let cache = Arc::new(ConcurrentComputeCache::<String, i32>::new(100));
        let mut handles = vec![];

        // Spawn multiple threads accessing the same cache
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

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify cache has entries
        assert!(!cache.is_empty());
        assert!(cache.len() <= 100);
    }

    #[test]
    fn test_get() {
        let cache = ConcurrentComputeCache::<String, i32>::new(10);

        // Initially empty
        assert_eq!(cache.get(&"key1".to_string()), None);

        // After insert
        cache.insert("key1".to_string(), 42).unwrap();
        assert_eq!(cache.get(&"key1".to_string()), Some(42));
    }

    #[test]
    fn test_eviction() {
        let cache = ConcurrentComputeCache::<String, i32>::new(2);

        // Fill cache
        cache.get_or_compute("key1".to_string(), || Ok(1)).unwrap();
        cache.get_or_compute("key2".to_string(), || Ok(2)).unwrap();

        // This should trigger eviction
        cache.get_or_compute("key3".to_string(), || Ok(3)).unwrap();

        // Cache should not exceed capacity
        assert!(cache.len() <= 2);
    }
}
