//! Compute cache implementation
//!
//! This module provides a generic compute cache that stores computed values
//! and avoids recomputation when the same key is requested again.

#![allow(clippy::excessive_nesting)]

use std::{
    collections::HashMap,
    hash::Hash,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use super::config::{CacheConfig, CacheMetrics, EvictionPolicy};
use crate::error::{MemoryError, MemoryResult};

/// Trait for cache keys
pub trait CacheKey: Hash + Eq + Clone {}

// Implement CacheKey for common types
impl<T: Hash + Eq + Clone> CacheKey for T {}

/// Result type for cache operations
pub type CacheResult<T> = Result<T, MemoryError>;

/// Cache entry with metadata
#[derive(Debug, Clone)]
pub struct CacheEntry<V> {
    /// The cached value
    pub value: V,
    /// When the entry was created
    pub created_at: Instant,
    /// When the entry was last accessed
    pub last_accessed: Instant,
    /// Number of times the entry has been accessed
    pub access_count: usize,
}

impl<V: Clone> CacheEntry<V> {
    /// Create a new cache entry
    pub fn new(value: V) -> Self {
        let now = Instant::now();
        Self {
            value,
            created_at: now,
            last_accessed: now,
            access_count: 1,
        }
    }

    /// Update the access time and count
    pub fn mark_accessed(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }

    /// Check if the entry has expired
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() > ttl
    }
}

/// A generic compute cache that stores computed values
pub struct ComputeCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// The cache storage
    entries: HashMap<K, CacheEntry<V>>,
    /// Cache configuration
    config: CacheConfig,
    /// Cache metrics
    metrics: Arc<Mutex<CacheMetrics>>,
}

impl<K, V> ComputeCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// Create a new compute cache with the given maximum size
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self::with_config(CacheConfig::new(max_entries))
    }

    /// Create a new compute cache with the given configuration
    #[must_use]
    pub fn with_config(config: CacheConfig) -> Self {
        let initial_capacity = config.initial_capacity.unwrap_or(config.max_entries);

        let metrics = Arc::new(Mutex::new(CacheMetrics::new()));

        Self {
            entries: HashMap::with_capacity(initial_capacity),
            config,
            metrics,
        }
    }

    /// Get a value from the cache, computing it if not present
    pub fn get_or_compute<F>(&mut self, key: K, compute_fn: F) -> CacheResult<V>
    where
        F: FnOnce() -> Result<V, MemoryError>,
    {
        // Check if the key exists in the cache
        if let Some(entry) = self.entries.get_mut(&key) {
            // Check if the entry has expired
            if let Some(ttl) = self.config.ttl {
                if entry.is_expired(ttl) {
                    // Entry has expired, remove it
                    self.entries.remove(&key);

                    if self.config.track_metrics {
                        let mut metrics = self.metrics.lock();
                        metrics.evictions += 1;
                    }
                } else {
                    // Entry is valid, update access time and return
                    entry.mark_accessed();

                    if self.config.track_metrics {
                        let mut metrics = self.metrics.lock();
                        metrics.hits += 1;
                    }

                    return Ok(entry.value.clone());
                }
            } else {
                // No TTL, just update access time and return
                entry.mark_accessed();

                if self.config.track_metrics {
                    let mut metrics = self.metrics.lock();
                    metrics.hits += 1;
                }

                return Ok(entry.value.clone());
            }
        }

        // Key not in cache or entry expired, compute the value
        if self.config.track_metrics {
            let mut metrics = self.metrics.lock();
            metrics.misses += 1;
        }

        // Check if we need to evict an entry
        if self.entries.len() >= self.config.max_entries {
            self.evict_entry()?;
        }

        // Compute the value
        let start_time = Instant::now();

        let value = compute_fn()?;

        if self.config.track_metrics {
            let compute_time = start_time.elapsed().as_nanos() as u64;
            let mut metrics = self.metrics.lock();
            metrics.compute_time_ns += compute_time;
            metrics.insertions += 1;
        }

        // Insert the new entry
        let entry = CacheEntry::new(value.clone());
        self.entries.insert(key, entry);

        Ok(value)
    }

    /// Get a value from cache without computing (read-only operation)
    pub fn get(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.entries.get_mut(key) {
            // Check if expired
            if let Some(ttl) = self.config.ttl
                && entry.is_expired(ttl)
            {
                self.entries.remove(key);
                return None;
            }

            // Update access metadata
            entry.mark_accessed();

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.hits += 1;
            }

            Some(entry.value.clone())
        } else {
            if self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.misses += 1;
            }
            None
        }
    }

    /// Insert a value directly without computation
    pub fn insert(&mut self, key: K, value: V) -> CacheResult<()> {
        if self.entries.len() >= self.config.max_entries && !self.entries.contains_key(&key) {
            self.evict_entry()?;
        }

        let entry = CacheEntry::new(value);
        self.entries.insert(key, entry);

        if self.config.track_metrics {
            let mut metrics = self.metrics.lock();
            metrics.insertions += 1;
        }

        Ok(())
    }

    /// Check if a key exists without updating access metadata
    pub fn contains_key(&self, key: &K) -> bool {
        if let Some(entry) = self.entries.get(key) {
            if let Some(ttl) = self.config.ttl {
                !entry.is_expired(ttl)
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Remove a specific key and return its value
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.entries.remove(key).map(|entry| entry.value)
    }

    /// Get all keys currently in the cache
    #[must_use]
    pub fn keys(&self) -> Vec<K> {
        self.entries.keys().cloned().collect()
    }

    /// Get cache capacity
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.config.max_entries
    }

    /// Get current load factor
    #[must_use]
    pub fn load_factor(&self) -> f32 {
        self.entries.len() as f32 / self.config.max_entries as f32
    }

    /// Batch get or compute multiple values
    pub fn get_or_compute_batch<F>(&mut self, keys: Vec<K>, compute_fn: F) -> Vec<CacheResult<V>>
    where
        F: Fn(&K) -> Result<V, MemoryError>,
    {
        keys.into_iter()
            .map(|key| self.get_or_compute(key.clone(), || compute_fn(&key)))
            .collect()
    }

    /// Warm up cache with values for given keys
    pub fn warm_up<F>(&mut self, keys: Vec<K>, compute_fn: F) -> CacheResult<()>
    where
        F: Fn(&K) -> Result<V, MemoryError>,
    {
        for key in keys {
            if !self.contains_key(&key) {
                self.get_or_compute(key.clone(), || compute_fn(&key))?;
            }
        }
        Ok(())
    }

    /// Clean up expired entries
    pub fn cleanup_expired(&mut self) -> usize {
        if let Some(ttl) = self.config.ttl {
            let expired_keys: Vec<K> = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.is_expired(ttl))
                .map(|(key, _)| key.clone())
                .collect();

            let count = expired_keys.len();
            for key in expired_keys {
                self.entries.remove(&key);
            }

            if count > 0 && self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.evictions += count;
            }

            count
        } else {
            0
        }
    }

    /// Evict an entry according to the configured policy
    fn evict_entry(&mut self) -> MemoryResult<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        match self.config.policy {
            EvictionPolicy::LRU => self.evict_lru(),
            EvictionPolicy::LFU => self.evict_lfu(),
            EvictionPolicy::FIFO => self.evict_fifo(),
            EvictionPolicy::Random => self.evict_random(),
            EvictionPolicy::TTL => self.evict_expired(),
            EvictionPolicy::Adaptive => self.evict_adaptive(),
        }
    }

    /// Evict the least recently used entry
    fn evict_lru(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
        {
            let key = key.clone();
            self.entries.remove(&key);

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// Evict the least frequently used entry
    fn evict_lfu(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.access_count)
        {
            let key = key.clone();
            self.entries.remove(&key);

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// Evict the oldest entry (FIFO)
    fn evict_fifo(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.created_at)
        {
            let key = key.clone();
            self.entries.remove(&key);

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// Evict a random entry
    fn evict_random(&mut self) -> MemoryResult<()> {
        use rand::prelude::IndexedRandom;

        if self.entries.is_empty() {
            return Ok(());
        }

        // True random using rng
        let keys: Vec<_> = self.entries.keys().cloned().collect();
        if let Some(key) = keys.choose(&mut rand::rng()) {
            self.entries.remove(key);

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// Evict expired entries
    fn evict_expired(&mut self) -> MemoryResult<()> {
        if let Some(ttl) = self.config.ttl {
            let expired_keys: Vec<K> = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.is_expired(ttl))
                .map(|(key, _)| key.clone())
                .collect();

            let count = expired_keys.len();

            for key in expired_keys {
                self.entries.remove(&key);
            }

            if count > 0 && self.config.track_metrics {
                let mut metrics = self.metrics.lock();
                metrics.evictions += count;
            }

            // If no expired entries, fall back to LRU
            if count == 0 {
                return self.evict_lru();
            }
        } else {
            // No TTL configured, fall back to LRU
            return self.evict_lru();
        }

        Ok(())
    }

    /// Adaptive eviction based on access patterns
    fn evict_adaptive(&mut self) -> MemoryResult<()> {
        // Simple adaptive logic: use LFU if average access count is high, otherwise LRU
        let total_accesses: usize = self.entries.values().map(|e| e.access_count).sum();
        let avg_accesses = if self.entries.is_empty() {
            0
        } else {
            total_accesses / self.entries.len()
        };

        if avg_accesses > 3 {
            self.evict_lfu()
        } else {
            self.evict_lru()
        }
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
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get the cache metrics
    #[must_use]
    pub fn metrics(&self) -> CacheMetrics {
        if self.config.track_metrics {
            self.metrics.lock().clone()
        } else {
            CacheMetrics::default()
        }
    }

    /// Reset the cache metrics
    pub fn reset_metrics(&self) {
        if self.config.track_metrics {
            self.metrics.lock().reset();
        }
    }
}

// Note: For thread-safe compute caching, use the `concurrent` module with DashMap-based caches
// which provide lock-free concurrent access. The Arc<Mutex<ComputeCache>> pattern has been
// removed as it's not idiomatic Rust and creates unnecessary contention.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_caching() {
        let mut cache = ComputeCache::<String, usize>::new(10);

        // First call should compute
        let result1 = cache.get_or_compute("key1".to_string(), || Ok(42));
        assert_eq!(result1.unwrap(), 42);

        // Second call should use cached value
        let result2 = cache.get_or_compute("key1".to_string(), || Ok(99));
        assert_eq!(result2.unwrap(), 42);

        // Different key should compute new value
        let result3 = cache.get_or_compute("key2".to_string(), || Ok(99));
        assert_eq!(result3.unwrap(), 99);
    }

    #[test]
    fn test_direct_operations() {
        let mut cache = ComputeCache::<String, usize>::new(10);

        // Test insert
        cache.insert("key1".to_string(), 42).unwrap();
        assert_eq!(cache.get(&"key1".to_string()), Some(42));

        // Test contains_key
        assert!(cache.contains_key(&"key1".to_string()));
        assert!(!cache.contains_key(&"nonexistent".to_string()));

        // Test remove
        assert_eq!(cache.remove(&"key1".to_string()), Some(42));
        assert_eq!(cache.get(&"key1".to_string()), None);
    }

    #[test]
    fn test_eviction() {
        let mut cache = ComputeCache::<String, usize>::new(2);

        // Fill the cache
        let _ = cache.get_or_compute("key1".to_string(), || Ok(1));
        let _ = cache.get_or_compute("key2".to_string(), || Ok(2));

        // This should evict one entry
        let _ = cache.get_or_compute("key3".to_string(), || Ok(3));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_batch_operations() {
        let mut cache = ComputeCache::<String, usize>::new(10);

        let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
        let results = cache.get_or_compute_batch(keys, |k| Ok(k.len()));

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn test_error_handling() {
        let mut cache = ComputeCache::<String, usize>::new(10);

        // Error should be propagated
        let result = cache.get_or_compute("error".to_string(), || {
            Err(MemoryError::allocation_failed(0, 1))
        });

        assert!(result.is_err());

        // After error, key should not be cached
        let result = cache.get_or_compute("error".to_string(), || Ok(42));
        assert_eq!(result.unwrap(), 42);
    }
}
