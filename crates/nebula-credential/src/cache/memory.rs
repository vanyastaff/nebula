//! In-memory token cache implementation

use crate::core::AccessToken;
use crate::core::CredentialError;
use crate::traits::TokenCache;
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// In-memory token cache
pub struct MemoryTokenCache {
    cache: Arc<DashMap<String, CacheEntry>>,
    config: MemoryCacheConfig,
    stats: Arc<CacheStats>,
}

/// Configuration for memory cache
#[derive(Debug, Clone)]
pub struct MemoryCacheConfig {
    /// Maximum number of entries
    pub max_size: usize,
    /// Default TTL
    pub default_ttl: Duration,
}

impl Default for MemoryCacheConfig {
    fn default() -> Self {
        Self {
            max_size: 10_000,
            default_ttl: Duration::from_secs(300),
        }
    }
}

struct CacheEntry {
    token: AccessToken,
    expires_at: Instant,
}

/// Cache statistics
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Cache hits
    pub hits: AtomicU64,
    /// Cache misses
    pub misses: AtomicU64,
    /// Evictions
    pub evictions: AtomicU64,
}

impl CacheStats {
    /// Calculate hit rate
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let total = hits + self.misses.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }
}

impl MemoryTokenCache {
    /// Create new memory cache
    pub fn new(config: MemoryCacheConfig) -> Arc<Self> {
        Arc::new(Self {
            cache: Arc::new(DashMap::new()),
            config,
            stats: Arc::new(CacheStats::default()),
        })
    }

    /// Create with default config
    pub fn default_config() -> Arc<Self> {
        Self::new(MemoryCacheConfig::default())
    }

    /// Get statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get size
    pub fn size(&self) -> usize {
        self.cache.len()
    }

    fn evict_if_needed(&self) {
        if self.cache.len() >= self.config.max_size {
            let to_remove = self.config.max_size / 10;
            let mut removed = 0;
            for entry in self.cache.iter() {
                if removed >= to_remove {
                    break;
                }
                let key = entry.key().clone();
                self.cache.remove(&key);
                removed += 1;
            }
            self.stats
                .evictions
                .fetch_add(removed as u64, Ordering::Relaxed);
        }
    }
}

#[async_trait::async_trait]
impl TokenCache for MemoryTokenCache {
    async fn get(&self, key: &str) -> Result<Option<AccessToken>, CredentialError> {
        match self.cache.get(key) {
            Some(entry) => {
                if entry.expires_at > Instant::now() {
                    self.stats.hits.fetch_add(1, Ordering::Relaxed);
                    Ok(Some(entry.token.clone()))
                } else {
                    drop(entry);
                    self.cache.remove(key);
                    self.stats.misses.fetch_add(1, Ordering::Relaxed);
                    self.stats.evictions.fetch_add(1, Ordering::Relaxed);
                    Ok(None)
                }
            }
            None => {
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }
        }
    }

    async fn put(
        &self,
        key: &str,
        token: &AccessToken,
        ttl: Duration,
    ) -> Result<(), CredentialError> {
        self.evict_if_needed();

        let entry = CacheEntry {
            token: token.clone(),
            expires_at: Instant::now() + ttl,
        };

        self.cache.insert(key.to_string(), entry);
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<(), CredentialError> {
        self.cache.remove(key);
        Ok(())
    }

    async fn clear(&self) -> Result<(), CredentialError> {
        self.cache.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AccessToken;

    #[tokio::test]
    async fn test_memory_cache_basic() {
        let cache = MemoryTokenCache::default_config();

        let result = cache.get("test").await.unwrap();
        assert!(result.is_none());
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 1);

        let token = AccessToken::bearer("test_token".to_string());
        cache
            .put("test", &token, Duration::from_secs(60))
            .await
            .unwrap();

        let retrieved = cache.get("test").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 1);

        cache.del("test").await.unwrap();
        assert!(cache.get("test").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_memory_cache_expiration() {
        let cache = MemoryTokenCache::default_config();
        let token = AccessToken::bearer("test_token".to_string());

        cache
            .put("test", &token, Duration::from_millis(50))
            .await
            .unwrap();
        assert!(cache.get("test").await.unwrap().is_some());

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(cache.get("test").await.unwrap().is_none());
        assert!(cache.stats().evictions.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn test_cache_stats() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);

        stats.hits.store(7, Ordering::Relaxed);
        stats.misses.store(3, Ordering::Relaxed);
        assert_eq!(stats.hit_rate(), 0.7);
    }
}
