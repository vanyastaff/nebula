//! Simple async cache implementation
//!
//! This module provides a lightweight async cache focused on the 80% use case:
//! - Simple get-or-compute semantics
//! - Async/await support
//! - Minimal overhead
//! - Clean, testable code
//!
//! For advanced features like deduplication, circuit breaking, or rate limiting,
//! use the decorator types in the parent module.

use std::{
    collections::HashMap,
    hash::Hash,
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(feature = "async")]
use tokio::sync::RwLock;

/// A simple async cache for get-or-compute operations
///
/// # Examples
///
/// ```rust
/// use nebula_memory::cache::AsyncCache;
///
/// #[tokio::main]
/// async fn main() {
///     let cache = AsyncCache::new(100);
///
///     // Get or compute a value
///     let value = cache.get_or_compute("key", || async {
///         expensive_computation().await
///     }).await.unwrap();
/// }
///
/// async fn expensive_computation() -> Result<i32, String> {
///     Ok(42)
/// }
/// ```
#[derive(Clone)]
pub struct AsyncCache<K, V> {
    inner: Arc<RwLock<CacheInner<K, V>>>,
    max_entries: usize,
}

struct CacheInner<K, V> {
    entries: HashMap<K, CacheEntry<V>>,
}

struct CacheEntry<V> {
    value: V,
    created_at: Instant,
    last_accessed: Instant,
    access_count: u64,
}

impl<V> CacheEntry<V> {
    fn new(value: V) -> Self {
        let now = Instant::now();
        Self {
            value,
            created_at: now,
            last_accessed: now,
            access_count: 1,
        }
    }

    fn update_access(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count = self.access_count.saturating_add(1);
    }
}

impl<K, V> AsyncCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new async cache with the specified maximum number of entries
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// let cache = AsyncCache::<String, i32>::new(100);
    /// ```
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheInner {
                entries: HashMap::with_capacity(max_entries.min(1024)),
            })),
            max_entries,
        }
    }

    /// Get a value from the cache, or compute it if not present
    ///
    /// This is the primary API for the cache. If the key exists, the cached value
    /// is returned immediately. Otherwise, the computation function is called,
    /// the result is cached, and returned.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     let value = cache.get_or_compute("key", || async {
    ///         Ok::<_, std::io::Error>(expensive_computation().await)
    ///     }).await.unwrap();
    ///
    ///     // Second call returns cached value immediately
    ///     let cached = cache.get_or_compute("key", || async {
    ///         panic!("Should not compute!");
    ///     }).await.unwrap();
    ///
    ///     assert_eq!(value, cached);
    /// }
    ///
    /// async fn expensive_computation() -> i32 { 42 }
    /// ```
    pub async fn get_or_compute<F, Fut, E>(&self, key: K, f: F) -> Result<V, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<V, E>>,
    {
        // Fast path: read lock
        {
            let cache = self.inner.read().await;
            if let Some(entry) = cache.entries.get(&key) {
                return Ok(entry.value.clone());
            }
        }

        // Slow path: compute and insert
        let value = f().await?;

        {
            let mut cache = self.inner.write().await;

            // Evict if at capacity (simple LRU)
            if cache.entries.len() >= self.max_entries
                && let Some(evict_key) = cache
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_accessed)
                    .map(|(k, _)| k.clone())
                {
                    cache.entries.remove(&evict_key);
                }

            cache.entries.insert(key, CacheEntry::new(value.clone()));
        }

        Ok(value)
    }

    /// Get a value from the cache if it exists
    ///
    /// Returns `None` if the key is not in the cache.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     assert!(cache.get(&"key").await.is_none());
    ///
    ///     cache.insert("key", 42).await;
    ///
    ///     assert_eq!(cache.get(&"key").await, Some(42));
    /// }
    /// ```
    pub async fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.inner.write().await;
        if let Some(entry) = cache.entries.get_mut(key) {
            entry.update_access();
            Some(entry.value.clone())
        } else {
            None
        }
    }

    /// Insert a value into the cache
    ///
    /// Returns the previous value if the key already existed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     assert_eq!(cache.insert("key", 42).await, None);
    ///     assert_eq!(cache.insert("key", 43).await, Some(42));
    /// }
    /// ```
    pub async fn insert(&self, key: K, value: V) -> Option<V> {
        let mut cache = self.inner.write().await;

        // Evict if at capacity
        if cache.entries.len() >= self.max_entries && !cache.entries.contains_key(&key)
            && let Some(evict_key) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(k, _)| k.clone())
            {
                cache.entries.remove(&evict_key);
            }

        cache
            .entries
            .insert(key, CacheEntry::new(value))
            .map(|e| e.value)
    }

    /// Remove a value from the cache
    ///
    /// Returns the removed value if it existed.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     cache.insert("key", 42).await;
    ///     assert_eq!(cache.remove(&"key").await, Some(42));
    ///     assert_eq!(cache.remove(&"key").await, None);
    /// }
    /// ```
    pub async fn remove(&self, key: &K) -> Option<V> {
        let mut cache = self.inner.write().await;
        cache.entries.remove(key).map(|e| e.value)
    }

    /// Check if the cache contains a key
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     assert!(!cache.contains_key(&"key").await);
    ///     cache.insert("key", 42).await;
    ///     assert!(cache.contains_key(&"key").await);
    /// }
    /// ```
    pub async fn contains_key(&self, key: &K) -> bool {
        let cache = self.inner.read().await;
        cache.entries.contains_key(key)
    }

    /// Clear all entries from the cache
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     cache.insert("key1", 42).await;
    ///     cache.insert("key2", 43).await;
    ///
    ///     cache.clear().await;
    ///
    ///     assert_eq!(cache.len().await, 0);
    /// }
    /// ```
    pub async fn clear(&self) {
        let mut cache = self.inner.write().await;
        cache.entries.clear();
    }

    /// Get the number of entries in the cache
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     assert_eq!(cache.len().await, 0);
    ///     cache.insert("key", 42).await;
    ///     assert_eq!(cache.len().await, 1);
    /// }
    /// ```
    pub async fn len(&self) -> usize {
        let cache = self.inner.read().await;
        cache.entries.len()
    }

    /// Check if the cache is empty
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_memory::cache::AsyncCache;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let cache = AsyncCache::new(100);
    ///
    ///     assert!(cache.is_empty().await);
    ///     cache.insert("key", 42).await;
    ///     assert!(!cache.is_empty().await);
    /// }
    /// ```
    pub async fn is_empty(&self) -> bool {
        let cache = self.inner.read().await;
        cache.entries.is_empty()
    }

    /// Get cache statistics
    ///
    /// Returns information about cache size, oldest entry, etc.
    pub async fn stats(&self) -> CacheStats {
        let cache = self.inner.read().await;

        let oldest_entry = cache
            .entries
            .values()
            .min_by_key(|e| e.created_at)
            .map(|e| e.created_at.elapsed());

        let total_accesses = cache.entries.values().map(|e| e.access_count).sum();

        CacheStats {
            size: cache.entries.len(),
            capacity: self.max_entries,
            oldest_entry_age: oldest_entry,
            total_accesses,
        }
    }
}

/// Statistics about the cache
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Current number of entries in the cache
    pub size: usize,
    /// Maximum number of entries
    pub capacity: usize,
    /// Age of the oldest entry
    pub oldest_entry_age: Option<Duration>,
    /// Total number of accesses across all entries
    pub total_accesses: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_simple_caching() {
        let cache = AsyncCache::new(10);
        let counter = Arc::new(AtomicUsize::new(0));

        // First call: compute
        let c = counter.clone();
        let v1 = cache
            .get_or_compute("key", || async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok::<_, std::io::Error>(42)
            })
            .await
            .unwrap();

        assert_eq!(v1, 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second call: cached
        let v2 = cache
            .get_or_compute("key", || async {
                panic!("Should not compute!");
            })
            .await
            .unwrap();

        assert_eq!(v2, 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Not incremented
    }

    #[tokio::test]
    async fn test_eviction() {
        let cache = AsyncCache::new(2);

        cache.insert("key1", 1).await;
        cache.insert("key2", 2).await;

        // Access key1 to make it more recent
        cache.get(&"key1").await;

        // Insert key3, should evict key2 (LRU)
        cache.insert("key3", 3).await;

        assert!(cache.contains_key(&"key1").await);
        assert!(!cache.contains_key(&"key2").await); // Evicted
        assert!(cache.contains_key(&"key3").await);
    }

    #[tokio::test]
    async fn test_basic_operations() {
        let cache = AsyncCache::new(10);

        // Insert
        assert_eq!(cache.insert("key", 42).await, None);
        assert_eq!(cache.insert("key", 43).await, Some(42));

        // Get
        assert_eq!(cache.get(&"key").await, Some(43));
        assert_eq!(cache.get(&"missing").await, None);

        // Contains
        assert!(cache.contains_key(&"key").await);
        assert!(!cache.contains_key(&"missing").await);

        // Remove
        assert_eq!(cache.remove(&"key").await, Some(43));
        assert_eq!(cache.remove(&"key").await, None);

        // Len
        cache.insert("k1", 1).await;
        cache.insert("k2", 2).await;
        assert_eq!(cache.len().await, 2);
        assert!(!cache.is_empty().await);

        // Clear
        cache.clear().await;
        assert_eq!(cache.len().await, 0);
        assert!(cache.is_empty().await);
    }

    #[tokio::test]
    async fn test_stats() {
        let cache = AsyncCache::new(10);

        cache.insert("key1", 1).await;
        cache.insert("key2", 2).await;

        // Access key1 multiple times
        for _ in 0..5 {
            cache.get(&"key1").await;
        }

        let stats = cache.stats().await;
        assert_eq!(stats.size, 2);
        assert_eq!(stats.capacity, 10);
        assert!(stats.oldest_entry_age.is_some());
        assert!(stats.total_accesses > 0);
    }
}
