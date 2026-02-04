//! Cache layer for credential storage with hit/miss tracking.

use crate::core::{CredentialId, CredentialMetadata};
use crate::manager::config::CacheConfig;
use crate::utils::EncryptedData;
use moka::future::Cache;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cached credential entry (encrypted data + metadata)
pub type CachedCredential = (EncryptedData, CredentialMetadata);

/// Cache layer wrapping moka with hit/miss tracking
pub struct CacheLayer {
    /// Underlying moka cache instance
    cache: Cache<CredentialId, CachedCredential>,

    /// Cache hit counter
    hits: AtomicU64,

    /// Cache miss counter
    misses: AtomicU64,

    /// Cache configuration
    config: CacheConfig,
}

impl CacheLayer {
    /// Create new cache from configuration
    ///
    /// # Arguments
    ///
    /// * `config` - Cache configuration with TTL, capacity, eviction settings
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::manager::{CacheLayer, CacheConfig};
    /// use std::time::Duration;
    ///
    /// let config = CacheConfig {
    ///     enabled: true,
    ///     ttl: Some(Duration::from_secs(300)),
    ///     idle_timeout: None,
    ///     max_capacity: 1000,
    ///     eviction_strategy: EvictionStrategy::Lru,
    /// };
    ///
    /// let cache = CacheLayer::new(&config);
    /// ```
    pub fn new(config: &CacheConfig) -> Self {
        let mut builder = Cache::builder().max_capacity(config.max_capacity as u64);

        if let Some(ttl) = config.ttl {
            builder = builder.time_to_live(ttl);
        }

        if let Some(idle) = config.idle_timeout {
            builder = builder.time_to_idle(idle);
        }

        Self {
            cache: builder.build(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            config: config.clone(),
        }
    }

    /// Get credential from cache (increments hit/miss counters)
    ///
    /// # Arguments
    ///
    /// * `id` - Credential identifier to look up
    ///
    /// # Returns
    ///
    /// - `Some((EncryptedData, CredentialMetadata))` if cache hit (increments hit counter)
    /// - `None` if cache miss (increments miss counter)
    pub async fn get(&self, id: &CredentialId) -> Option<CachedCredential> {
        match self.cache.get(id).await {
            Some(credential) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(credential)
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Insert credential into cache
    ///
    /// # Arguments
    ///
    /// * `id` - Credential identifier
    /// * `data` - Encrypted credential data
    /// * `metadata` - Credential metadata
    pub async fn insert(
        &self,
        id: CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
    ) {
        self.cache.insert(id, (data, metadata)).await;
    }

    /// Invalidate single cache entry
    ///
    /// # Arguments
    ///
    /// * `id` - Credential identifier to invalidate
    pub async fn invalidate(&self, id: &CredentialId) {
        self.cache.invalidate(id).await;
    }

    /// Invalidate all cache entries
    pub async fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }

    /// Get cache performance statistics
    ///
    /// # Returns
    ///
    /// `CacheStats` with current hit/miss counts, size, and capacity
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            size: self.cache.entry_count(),
            max_capacity: self.config.max_capacity,
        }
    }
}

/// Cache performance statistics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Total cache hits
    pub hits: u64,

    /// Total cache misses
    pub misses: u64,

    /// Current number of cached entries
    pub size: u64,

    /// Maximum cache capacity
    pub max_capacity: usize,
}

impl CacheStats {
    /// Calculate cache hit rate as percentage (0.0 - 1.0)
    ///
    /// # Returns
    ///
    /// - Hit rate as float (0.0 = 0%, 1.0 = 100%)
    /// - Returns 0.0 if no requests yet (avoid division by zero)
    ///
    /// # Examples
    ///
    /// ```
    /// let stats = CacheStats { hits: 80, misses: 20, size: 50, max_capacity: 100 };
    /// assert_eq!(stats.hit_rate(), 0.8); // 80% hit rate
    /// ```
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Check if cache is at maximum capacity
    ///
    /// # Returns
    ///
    /// `true` if current size equals max capacity
    pub fn is_full(&self) -> bool {
        self.size >= self.max_capacity as u64
    }

    /// Calculate cache utilization as percentage (0.0 - 1.0)
    ///
    /// # Returns
    ///
    /// - Utilization as float (0.0 = empty, 1.0 = full)
    ///
    /// # Examples
    ///
    /// ```
    /// let stats = CacheStats { hits: 80, misses: 20, size: 50, max_capacity: 100 };
    /// assert_eq!(stats.utilization(), 0.5); // 50% utilized
    /// ```
    pub fn utilization(&self) -> f64 {
        if self.max_capacity == 0 {
            0.0
        } else {
            self.size as f64 / self.max_capacity as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_stats_hit_rate() {
        let stats = CacheStats {
            hits: 80,
            misses: 20,
            size: 50,
            max_capacity: 100,
        };

        assert_eq!(stats.hit_rate(), 0.8);
    }

    #[test]
    fn test_cache_stats_hit_rate_zero_requests() {
        let stats = CacheStats {
            hits: 0,
            misses: 0,
            size: 0,
            max_capacity: 100,
        };

        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_cache_stats_is_full() {
        let full = CacheStats {
            hits: 100,
            misses: 50,
            size: 100,
            max_capacity: 100,
        };

        let not_full = CacheStats {
            hits: 100,
            misses: 50,
            size: 50,
            max_capacity: 100,
        };

        assert!(full.is_full());
        assert!(!not_full.is_full());
    }

    #[test]
    fn test_cache_stats_utilization() {
        let stats = CacheStats {
            hits: 80,
            misses: 20,
            size: 50,
            max_capacity: 100,
        };

        assert_eq!(stats.utilization(), 0.5);
    }

    #[test]
    fn test_cache_stats_utilization_zero_capacity() {
        let stats = CacheStats {
            hits: 0,
            misses: 0,
            size: 0,
            max_capacity: 0,
        };

        assert_eq!(stats.utilization(), 0.0);
    }
}
