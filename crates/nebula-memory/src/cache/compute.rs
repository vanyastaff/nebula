//! Compute cache implementation
//!
//! This module provides a generic compute cache that stores computed values
//! and avoids recomputation when the same key is requested again.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, sync::Arc, vec::Vec},
    core::{hash::Hash, time::Duration},
    hashbrown::HashMap,
    spin::Mutex,
};

use super::config::{CacheConfig, CacheMetrics, EvictionPolicy};
use crate::core::error::{MemoryError, MemoryResult};

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
    #[cfg(feature = "std")]
    pub created_at: Instant,
    /// When the entry was last accessed
    #[cfg(feature = "std")]
    pub last_accessed: Instant,
    /// Number of times the entry has been accessed
    pub access_count: usize,
    /// Access order for no-std LRU implementation
    #[cfg(not(feature = "std"))]
    pub access_order: u64,
}

impl<V: Clone> CacheEntry<V> {
    /// Create a new cache entry
    #[cfg(feature = "std")]
    pub fn new(value: V) -> Self {
        let now = Instant::now();
        Self { value, created_at: now, last_accessed: now, access_count: 1 }
    }

    /// Create a new cache entry (no-std version)
    #[cfg(not(feature = "std"))]
    pub fn new(value: V) -> Self {
        Self { value, access_count: 1, access_order: 0 }
    }

    /// Update the access time and count
    #[cfg(feature = "std")]
    pub fn mark_accessed(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }

    /// Update the access count (no-std version)
    #[cfg(not(feature = "std"))]
    pub fn mark_accessed(&mut self, current_order: u64) {
        self.access_count += 1;
        self.access_order = current_order;
    }

    /// Check if the entry has expired
    #[cfg(feature = "std")]
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() > ttl
    }

    /// No-op for no-std (always returns false)
    #[cfg(not(feature = "std"))]
    pub fn is_expired(&self, _ttl: Duration) -> bool {
        false
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
    #[cfg(feature = "std")]
    metrics: Arc<Mutex<CacheMetrics>>,
    /// Access order counter for no-std LRU
    #[cfg(not(feature = "std"))]
    access_counter: u64,
}

impl<K, V> ComputeCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// Create a new compute cache with the given maximum size
    pub fn new(max_entries: usize) -> Self {
        Self::with_config(CacheConfig::new(max_entries))
    }

    /// Create a new compute cache with the given configuration
    pub fn with_config(config: CacheConfig) -> Self {
        let initial_capacity = config.initial_capacity.unwrap_or_else(|| config.max_entries);

        #[cfg(feature = "std")]
        let metrics = Arc::new(Mutex::new(CacheMetrics::new()));

        Self {
            entries: HashMap::with_capacity(initial_capacity),
            config,
            #[cfg(feature = "std")]
            metrics,
            #[cfg(not(feature = "std"))]
            access_counter: 0,
        }
    }

    /// Get a value from the cache, computing it if not present
    pub fn get_or_compute<F>(&mut self, key: K, compute_fn: F) -> CacheResult<V>
    where F: FnOnce() -> Result<V, MemoryError> {
        // Check if the key exists in the cache
        if let Some(entry) = self.entries.get_mut(&key) {
            // Check if the entry has expired
            #[cfg(feature = "std")]
            if let Some(ttl) = self.config.ttl {
                if entry.is_expired(ttl) {
                    // Entry has expired, remove it
                    self.entries.remove(&key);

                    if self.config.track_metrics {
                        let mut metrics = self.metrics.lock().unwrap();
                        metrics.evictions += 1;
                    }
                } else {
                    // Entry is valid, update access time and return
                    entry.mark_accessed();

                    if self.config.track_metrics {
                        let mut metrics = self.metrics.lock().unwrap();
                        metrics.hits += 1;
                    }

                    return Ok(entry.value.clone());
                }
            } else {
                // No TTL, just update access time and return
                #[cfg(feature = "std")]
                entry.mark_accessed();

                #[cfg(not(feature = "std"))]
                {
                    self.access_counter += 1;
                    entry.mark_accessed(self.access_counter);
                }

                #[cfg(feature = "std")]
                if self.config.track_metrics {
                    let mut metrics = self.metrics.lock().unwrap();
                    metrics.hits += 1;
                }

                return Ok(entry.value.clone());
            }
        }

        // Key not in cache or entry expired, compute the value
        #[cfg(feature = "std")]
        if self.config.track_metrics {
            let mut metrics = self.metrics.lock().unwrap();
            metrics.misses += 1;
        }

        // Check if we need to evict an entry
        if self.entries.len() >= self.config.max_entries {
            self.evict_entry()?;
        }

        // Compute the value
        #[cfg(feature = "std")]
        let start_time = Instant::now();

        let value = compute_fn()?;

        #[cfg(feature = "std")]
        if self.config.track_metrics {
            let compute_time = start_time.elapsed().as_nanos() as u64;
            let mut metrics = self.metrics.lock().unwrap();
            metrics.compute_time_ns += compute_time;
            metrics.insertions += 1;
        }

        // Insert the new entry
        let mut entry = CacheEntry::new(value.clone());

        #[cfg(not(feature = "std"))]
        {
            self.access_counter += 1;
            entry.access_order = self.access_counter;
        }

        self.entries.insert(key, entry);

        Ok(value)
    }

    /// Get a value from cache without computing (read-only operation)
    pub fn get(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.entries.get_mut(key) {
            // Check if expired
            #[cfg(feature = "std")]
            if let Some(ttl) = self.config.ttl {
                if entry.is_expired(ttl) {
                    self.entries.remove(key);
                    return None;
                }
            }

            // Update access metadata
            #[cfg(feature = "std")]
            entry.mark_accessed();

            #[cfg(not(feature = "std"))]
            {
                self.access_counter += 1;
                entry.mark_accessed(self.access_counter);
            }

            #[cfg(feature = "std")]
            if self.config.track_metrics {
                let mut metrics = self.metrics.lock().unwrap();
                metrics.hits += 1;
            }

            Some(entry.value.clone())
        } else {
            #[cfg(feature = "std")]
            if self.config.track_metrics {
                let mut metrics = self.metrics.lock().unwrap();
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

        let mut entry = CacheEntry::new(value);

        #[cfg(not(feature = "std"))]
        {
            self.access_counter += 1;
            entry.access_order = self.access_counter;
        }

        self.entries.insert(key, entry);

        #[cfg(feature = "std")]
        if self.config.track_metrics {
            let mut metrics = self.metrics.lock().unwrap();
            metrics.insertions += 1;
        }

        Ok(())
    }

    /// Check if a key exists without updating access metadata
    pub fn contains_key(&self, key: &K) -> bool {
        if let Some(entry) = self.entries.get(key) {
            #[cfg(feature = "std")]
            if let Some(ttl) = self.config.ttl {
                !entry.is_expired(ttl)
            } else {
                true
            }

            #[cfg(not(feature = "std"))]
            true
        } else {
            false
        }
    }

    /// Remove a specific key and return its value
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.entries.remove(key).map(|entry| entry.value)
    }

    /// Get all keys currently in the cache
    pub fn keys(&self) -> Vec<K> {
        self.entries.keys().cloned().collect()
    }

    /// Get cache capacity
    pub fn capacity(&self) -> usize {
        self.config.max_entries
    }

    /// Get current load factor
    pub fn load_factor(&self) -> f32 {
        self.entries.len() as f32 / self.config.max_entries as f32
    }

    /// Batch get or compute multiple values
    pub fn get_or_compute_batch<F>(&mut self, keys: Vec<K>, compute_fn: F) -> Vec<CacheResult<V>>
    where F: Fn(&K) -> Result<V, MemoryError> {
        keys.into_iter()
            .map(|key| {
                let key_ref = &key;
                self.get_or_compute(key, || compute_fn(key_ref))
            })
            .collect()
    }

    /// Warm up cache with values for given keys
    pub fn warm_up<F>(&mut self, keys: Vec<K>, compute_fn: F) -> CacheResult<()>
    where F: Fn(&K) -> Result<V, MemoryError> {
        for key in keys {
            if !self.contains_key(&key) {
                let key_ref = &key;
                self.get_or_compute(key, || compute_fn(key_ref))?;
            }
        }
        Ok(())
    }

    /// Clean up expired entries (std only)
    #[cfg(feature = "std")]
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
                let mut metrics = self.metrics.lock().unwrap();
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
    #[cfg(feature = "std")]
    fn evict_lru(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self.entries.iter().min_by_key(|(_, entry)| entry.last_accessed) {
            let key = key.clone();
            self.entries.remove(&key);

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock().unwrap();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// LRU for no-std using access_order
    #[cfg(not(feature = "std"))]
    fn evict_lru(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self.entries.iter().min_by_key(|(_, entry)| entry.access_order) {
            let key = key.clone();
            self.entries.remove(&key);
        }

        Ok(())
    }

    /// Evict the least frequently used entry
    fn evict_lfu(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self.entries.iter().min_by_key(|(_, entry)| entry.access_count) {
            let key = key.clone();
            self.entries.remove(&key);

            #[cfg(feature = "std")]
            if self.config.track_metrics {
                let mut metrics = self.metrics.lock().unwrap();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// Evict the oldest entry (FIFO)
    #[cfg(feature = "std")]
    fn evict_fifo(&mut self) -> MemoryResult<()> {
        if let Some((key, _)) = self.entries.iter().min_by_key(|(_, entry)| entry.created_at) {
            let key = key.clone();
            self.entries.remove(&key);

            if self.config.track_metrics {
                let mut metrics = self.metrics.lock().unwrap();
                metrics.evictions += 1;
            }
        }

        Ok(())
    }

    /// FIFO for no-std - remove entry with lowest access_order that hasn't been updated
    #[cfg(not(feature = "std"))]
    fn evict_fifo(&mut self) -> MemoryResult<()> {
        // In no-std, we use access_order as creation order approximation
        if let Some((key, _)) = self.entries.iter()
            .min_by_key(|(_, entry)| (entry.access_order, entry.access_count))
        {
            let key = key.clone();
            self.entries.remove(&key);
        }

        Ok(())
    }

    /// Evict a random entry
    fn evict_random(&mut self) -> MemoryResult<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        #[cfg(feature = "std")]
        {
            // True random using thread_rng
            use rand::seq::IteratorRandom;
            let keys: Vec<_> = self.entries.keys().cloned().collect();
            if let Some(key) = keys.choose(&mut rand::thread_rng()) {
                self.entries.remove(key);

                if self.config.track_metrics {
                    let mut metrics = self.metrics.lock().unwrap();
                    metrics.evictions += 1;
                }
            }
        }

        #[cfg(not(feature = "std"))]
        {
            // Pseudo-random for no-std
            let len = self.entries.len();
            let index = (len.wrapping_mul(1103515245).wrapping_add(12345)) % len;
            if let Some(key) = self.entries.keys().nth(index).cloned() {
                self.entries.remove(&key);
            }
        }

        Ok(())
    }

    /// Evict expired entries
    #[cfg(feature = "std")]
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
                let mut metrics = self.metrics.lock().unwrap();
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

    /// Simplified expired eviction for no-std (falls back to LRU)
    #[cfg(not(feature = "std"))]
    fn evict_expired(&mut self) -> MemoryResult<()> {
        self.evict_lru()
    }

    /// Adaptive eviction based on access patterns
    fn evict_adaptive(&mut self) -> MemoryResult<()> {
        // Simple adaptive logic: use LFU if average access count is high, otherwise LRU
        let total_accesses: usize = self.entries.values().map(|e| e.access_count).sum();
        let avg_accesses = if !self.entries.is_empty() {
            total_accesses / self.entries.len()
        } else {
            0
        };

        if avg_accesses > 3 {
            self.evict_lfu()
        } else {
            self.evict_lru()
        }
    }

    /// Get the current number of entries in the cache
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries from the cache
    pub fn clear(&mut self) {
        self.entries.clear();

        #[cfg(not(feature = "std"))]
        {
            self.access_counter = 0;
        }
    }

    /// Get the cache metrics
    #[cfg(feature = "std")]
    pub fn metrics(&self) -> CacheMetrics {
        if self.config.track_metrics {
            self.metrics.lock().unwrap().clone()
        } else {
            CacheMetrics::default()
        }
    }

    /// Reset the cache metrics
    #[cfg(feature = "std")]
    pub fn reset_metrics(&self) {
        if self.config.track_metrics {
            self.metrics.lock().unwrap().reset();
        }
    }
}

// Thread-safe wrapper
#[cfg(feature = "std")]
pub struct ThreadSafeComputeCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    inner: Arc<Mutex<ComputeCache<K, V>>>,
}

#[cfg(feature = "std")]
impl<K, V> ThreadSafeComputeCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ComputeCache::new(max_entries))),
        }
    }

    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ComputeCache::with_config(config))),
        }
    }

    pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> CacheResult<V>
    where F: FnOnce() -> Result<V, MemoryError> {
        let mut cache = self.inner.lock().unwrap();
        cache.get_or_compute(key, compute_fn)
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.inner.lock().unwrap();
        cache.get(key)
    }

    pub fn insert(&self, key: K, value: V) -> CacheResult<()> {
        let mut cache = self.inner.lock().unwrap();
        cache.insert(key, value)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        let cache = self.inner.lock().unwrap();
        cache.contains_key(key)
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        let mut cache = self.inner.lock().unwrap();
        cache.remove(key)
    }

    pub fn len(&self) -> usize {
        let cache = self.inner.lock().unwrap();
        cache.len()
    }

    pub fn clear(&self) {
        let mut cache = self.inner.lock().unwrap();
        cache.clear()
    }

    pub fn metrics(&self) -> CacheMetrics {
        let cache = self.inner.lock().unwrap();
        cache.metrics()
    }
}

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
        let results = cache.get_or_compute_batch(keys, |k| {
            Ok(k.len())
        });

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn test_error_handling() {
        let mut cache = ComputeCache::<String, usize>::new(10);

        // Error should be propagated
        let result =
            cache.get_or_compute("error".to_string(), || Err(MemoryError::allocation_failed()));

        assert!(result.is_err());

        // After error, key should not be cached
        let result = cache.get_or_compute("error".to_string(), || Ok(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_thread_safe_cache() {
        let cache = ThreadSafeComputeCache::<String, usize>::new(10);

        let result = cache.get_or_compute("key1".to_string(), || Ok(42));
        assert_eq!(result.unwrap(), 42);

        assert!(cache.contains_key(&"key1".to_string()));
        assert_eq!(cache.get(&"key1".to_string()), Some(42));
    }
}