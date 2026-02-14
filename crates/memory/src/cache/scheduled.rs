//! Simple scheduled cache with TTL cleanup
//!
//! This module provides a lightweight wrapper around `ComputeCache` with
//! periodic TTL-based cleanup. For the 80% use case in workflow automation.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use super::compute::{CacheKey, CacheResult, ComputeCache};
use super::config::CacheConfig;
use crate::error::MemoryError;

/// A cache with automatic TTL-based cleanup
///
/// This is a simple wrapper around `ComputeCache` that spawns a background
/// thread to periodically remove expired entries.
///
/// # Example
///
/// ```rust
/// use nebula_memory::cache::ScheduledCache;
/// use std::time::Duration;
///
/// let cache = ScheduledCache::new(100, Duration::from_secs(60));
///
/// // Insert with TTL
/// cache.insert_with_ttl("key", "value", Duration::from_secs(5));
///
/// // Will be automatically cleaned up after 5 seconds
/// ```
pub struct ScheduledCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    cache: Arc<Mutex<ComputeCache<K, V>>>,
    ttls: Arc<Mutex<HashMap<K, Instant>>>,
    cleanup_interval: Duration,
    shutdown: Arc<AtomicBool>,
    cleanup_thread: Option<JoinHandle<()>>,
}

impl<K, V> ScheduledCache<K, V>
where
    K: CacheKey + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new scheduled cache
    ///
    /// # Arguments
    ///
    /// * `max_entries` - Maximum number of entries before eviction
    /// * `cleanup_interval` - How often to check for expired entries
    #[must_use]
    pub fn new(max_entries: usize, cleanup_interval: Duration) -> Self {
        let cache = Arc::new(Mutex::new(ComputeCache::new(max_entries)));
        let ttls: Arc<Mutex<HashMap<K, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let cache_clone = cache.clone();
        let ttls_clone = ttls.clone();
        let shutdown_clone = shutdown.clone();

        let cleanup_thread = thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                thread::sleep(cleanup_interval);

                // Clean up expired entries
                let now = Instant::now();
                let mut ttls_guard = ttls_clone.lock();
                let mut cache_guard = cache_clone.lock();

                // Collect expired keys
                let expired_keys: Vec<K> = ttls_guard
                    .iter()
                    .filter(|(_, expiry)| **expiry <= now)
                    .map(|(k, _)| k.clone())
                    .collect();

                // Remove expired entries
                for key in expired_keys {
                    ttls_guard.remove(&key);
                    cache_guard.remove(&key);
                }
            }
        });

        Self {
            cache,
            ttls,
            cleanup_interval,
            shutdown,
            cleanup_thread: Some(cleanup_thread),
        }
    }

    /// Create with custom configuration
    #[must_use]
    pub fn with_config(config: CacheConfig, cleanup_interval: Duration) -> Self {
        let cache = Arc::new(Mutex::new(ComputeCache::with_config(config)));
        let ttls: Arc<Mutex<HashMap<K, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let cache_clone = cache.clone();
        let ttls_clone = ttls.clone();
        let shutdown_clone = shutdown.clone();

        let cleanup_thread = thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                thread::sleep(cleanup_interval);

                let now = Instant::now();
                let mut ttls_guard = ttls_clone.lock();
                let mut cache_guard = cache_clone.lock();

                let expired_keys: Vec<K> = ttls_guard
                    .iter()
                    .filter(|(_, expiry)| **expiry <= now)
                    .map(|(k, _)| k.clone())
                    .collect();

                for key in expired_keys {
                    ttls_guard.remove(&key);
                    cache_guard.remove(&key);
                }
            }
        });

        Self {
            cache,
            ttls,
            cleanup_interval,
            shutdown,
            cleanup_thread: Some(cleanup_thread),
        }
    }

    /// Insert a value with TTL
    #[inline]
    pub fn insert_with_ttl(&self, key: K, value: V, ttl: Duration) {
        let mut cache = self.cache.lock();
        let mut ttls = self.ttls.lock();

        let _ = cache.insert(key.clone(), value);
        ttls.insert(key, Instant::now() + ttl);
    }

    /// Get a value from the cache
    #[inline]
    pub fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.lock();
        cache.get(key)
    }

    /// Get or compute a value
    pub fn get_or_compute<F>(&self, key: K, f: F) -> CacheResult<V>
    where
        F: FnOnce() -> Result<V, MemoryError>,
    {
        let mut cache = self.cache.lock();
        cache.get_or_compute(key, f)
    }

    /// Get or compute with TTL
    pub fn get_or_compute_with_ttl<F>(&self, key: K, ttl: Duration, f: F) -> CacheResult<V>
    where
        F: FnOnce() -> Result<V, MemoryError>,
    {
        // Check if value exists
        {
            let mut cache = self.cache.lock();
            if let Some(value) = cache.get(&key) {
                return Ok(value);
            }
        }

        // Compute and insert with TTL
        let value = f()?;
        self.insert_with_ttl(key.clone(), value.clone(), ttl);
        Ok(value)
    }

    /// Remove a value from the cache
    #[inline]
    pub fn remove(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.lock();
        let mut ttls = self.ttls.lock();

        ttls.remove(key);
        cache.remove(key)
    }

    /// Clear all entries
    #[inline]
    pub fn clear(&self) {
        let mut cache = self.cache.lock();
        let mut ttls = self.ttls.lock();

        cache.clear();
        ttls.clear();
    }

    /// Get current size
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        let cache = self.cache.lock();
        cache.len()
    }

    /// Check if empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Manually trigger cleanup of expired entries
    pub fn cleanup_expired(&self) {
        let now = Instant::now();
        let mut ttls = self.ttls.lock();
        let mut cache = self.cache.lock();

        let expired_keys: Vec<K> = ttls
            .iter()
            .filter(|(_, expiry)| **expiry <= now)
            .map(|(k, _)| k.clone())
            .collect();

        for key in expired_keys {
            ttls.remove(&key);
            cache.remove(&key);
        }
    }

    /// Get the cleanup interval
    #[inline]
    #[must_use]
    pub fn cleanup_interval(&self) -> Duration {
        self.cleanup_interval
    }
}

impl<K, V> Drop for ScheduledCache<K, V>
where
    K: CacheKey,
    V: Clone + Send + Sync + 'static,
{
    fn drop(&mut self) {
        // Signal shutdown
        self.shutdown.store(true, Ordering::Relaxed);

        // Wait for cleanup thread to finish
        if let Some(handle) = self.cleanup_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let cache = ScheduledCache::new(10, Duration::from_secs(60));

        cache.insert_with_ttl("key1", "value1", Duration::from_secs(10));
        assert_eq!(cache.get(&"key1"), Some("value1"));
    }

    #[test]
    fn test_ttl_expiration() {
        let cache = ScheduledCache::new(10, Duration::from_millis(50));

        cache.insert_with_ttl("key1", "value1", Duration::from_millis(100));
        assert_eq!(cache.get(&"key1"), Some("value1"));

        // Wait for expiration + cleanup cycle
        thread::sleep(Duration::from_millis(200));

        // Should be cleaned up
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_manual_cleanup() {
        let cache = ScheduledCache::new(10, Duration::from_secs(60));

        // Insert with very short TTL
        cache.insert_with_ttl("key1", "value1", Duration::from_millis(1));

        // Wait for expiration
        thread::sleep(Duration::from_millis(10));

        // Value still in cache (cleanup hasn't run)
        assert_eq!(cache.get(&"key1"), Some("value1"));

        // Trigger manual cleanup
        cache.cleanup_expired();

        // Now it should be gone
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_remove() {
        let cache = ScheduledCache::new(10, Duration::from_secs(60));

        cache.insert_with_ttl("key1", "value1", Duration::from_secs(10));
        assert_eq!(cache.len(), 1);

        let removed = cache.remove(&"key1");
        assert_eq!(removed, Some("value1"));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_clear() {
        let cache = ScheduledCache::new(10, Duration::from_secs(60));

        cache.insert_with_ttl("key1", "value1", Duration::from_secs(10));
        cache.insert_with_ttl("key2", "value2", Duration::from_secs(10));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&"key1"), None);
    }

    #[test]
    fn test_get_or_compute_with_ttl() {
        let cache = ScheduledCache::new(10, Duration::from_secs(60));
        let counter = Arc::new(Mutex::new(0));

        // First call: compute
        let c = counter.clone();
        let result = cache
            .get_or_compute_with_ttl("key", Duration::from_secs(10), move || {
                *c.lock() += 1;
                Ok::<_, MemoryError>(42)
            })
            .unwrap();

        assert_eq!(result, 42);
        assert_eq!(*counter.lock(), 1);

        // Second call: cached
        let c = counter.clone();
        let result = cache
            .get_or_compute_with_ttl("key", Duration::from_secs(10), move || {
                *c.lock() += 1;
                Ok::<_, MemoryError>(42)
            })
            .unwrap();

        assert_eq!(result, 42);
        assert_eq!(*counter.lock(), 1); // Not incremented
    }

    #[test]
    fn test_cleanup_thread_shutdown() {
        let cache = ScheduledCache::new(10, Duration::from_millis(10));

        cache.insert_with_ttl("key1", "value1", Duration::from_secs(10));
        assert_eq!(cache.len(), 1);

        // Drop should cleanly shutdown the thread
        drop(cache);
    }

    #[test]
    fn test_multiple_ttls() {
        let cache = ScheduledCache::new(10, Duration::from_millis(50));

        cache.insert_with_ttl("short", "value1", Duration::from_millis(50));
        cache.insert_with_ttl("long", "value2", Duration::from_secs(10));

        // Both should exist initially
        assert_eq!(cache.get(&"short"), Some("value1"));
        assert_eq!(cache.get(&"long"), Some("value2"));

        // Wait for short TTL to expire + cleanup
        thread::sleep(Duration::from_millis(150));

        // Short should be gone, long should remain
        assert_eq!(cache.get(&"short"), None);
        assert_eq!(cache.get(&"long"), Some("value2"));
    }

    #[test]
    fn test_is_empty() {
        let cache = ScheduledCache::new(10, Duration::from_secs(60));
        assert!(cache.is_empty());

        cache.insert_with_ttl("key1", "value1", Duration::from_secs(10));
        assert!(!cache.is_empty());
    }
}
