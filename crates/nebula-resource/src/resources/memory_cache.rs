//! In-memory cache resource implementation with LRU eviction

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    traits::{HealthCheckable, HealthStatus},
};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// In-memory cache configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MemoryCacheConfig {
    /// Maximum number of entries in cache
    pub max_entries: usize,
    /// Default TTL for entries (None = no expiration)
    pub default_ttl: Option<Duration>,
    /// Enable statistics tracking
    pub enable_stats: bool,
}

impl Default for MemoryCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            default_ttl: None,
            enable_stats: true,
        }
    }
}

impl ResourceConfig for MemoryCacheConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.max_entries == 0 {
            return Err(ResourceError::configuration(
                "Max entries must be greater than 0",
            ));
        }

        if self.max_entries > 10_000_000 {
            return Err(ResourceError::configuration(
                "Max entries too large (max: 10M)",
            ));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if other.max_entries > 0 {
            self.max_entries = other.max_entries;
        }
        if other.default_ttl.is_some() {
            self.default_ttl = other.default_ttl;
        }
        self.enable_stats = other.enable_stats;
    }
}

/// In-memory cache resource
pub struct MemoryCacheResource;

#[async_trait::async_trait]
impl Resource for MemoryCacheResource {
    type Config = MemoryCacheConfig;
    type Instance = MemoryCacheInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("memory-cache", "1.0"),
            "In-memory LRU cache for fast local caching".to_string(),
        )
        .poolable()
        .health_checkable()
        .with_tag("type", "cache")
        .with_tag("backend", "memory")
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        let cache = LruCache::new(config.max_entries);
        let stats = if config.enable_stats {
            Some(CacheStats::new())
        } else {
            None
        };

        Ok(MemoryCacheInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            cache: Arc::new(parking_lot::RwLock::new(cache)),
            default_ttl: config.default_ttl,
            stats: Arc::new(parking_lot::Mutex::new(stats)),
        })
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // Memory automatically freed on drop
        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        // Always valid for in-memory cache
        let _ = instance;
        Ok(true)
    }
}

/// In-memory cache instance
pub struct MemoryCacheInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    cache: Arc<parking_lot::RwLock<LruCache<String, CacheEntry>>>,
    default_ttl: Option<Duration>,
    stats: Arc<parking_lot::Mutex<Option<CacheStats>>>,
}

impl ResourceInstance for MemoryCacheInstance {
    fn instance_id(&self) -> uuid::Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> crate::core::lifecycle::LifecycleState {
        *self.state.read()
    }

    fn context(&self) -> &ResourceContext {
        &self.context
    }

    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_accessed.lock()
    }

    fn touch(&self) {
        *self.last_accessed.lock() = Some(chrono::Utc::now());
    }
}

impl MemoryCacheInstance {
    /// Get a value from cache
    pub fn get(&self, key: &str) -> Option<String> {
        self.touch();

        let mut cache = self.cache.write();
        let key_string = key.to_string();
        if let Some(entry) = cache.get(&key_string) {
            // Check if expired
            if let Some(expires_at) = entry.expires_at
                && Instant::now() > expires_at {
                    // Expired, remove it
                    cache.remove(&key_string);
                    self.record_miss();
                    return None;
                }

            self.record_hit();
            Some(entry.value.clone())
        } else {
            self.record_miss();
            None
        }
    }

    /// Set a value in cache with default TTL
    pub fn set(&self, key: impl Into<String>, value: impl Into<String>) {
        self.touch();
        let ttl = self.default_ttl;
        self.set_with_ttl(key, value, ttl);
    }

    /// Set a value with custom TTL
    pub fn set_with_ttl(
        &self,
        key: impl Into<String>,
        value: impl Into<String>,
        ttl: Option<Duration>,
    ) {
        self.touch();

        let expires_at = ttl.map(|d| Instant::now() + d);
        let entry = CacheEntry {
            value: value.into(),
            expires_at,
            created_at: Instant::now(),
        };

        let mut cache = self.cache.write();
        cache.insert(key.into(), entry);
        self.record_set();
    }

    /// Remove a key from cache
    pub fn remove(&self, key: &str) -> bool {
        self.touch();

        let mut cache = self.cache.write();
        cache.remove(&key.to_string()).is_some()
    }

    /// Check if key exists (and not expired)
    pub fn exists(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Clear all entries
    pub fn clear(&self) {
        self.touch();

        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get number of entries in cache
    pub fn len(&self) -> usize {
        let cache = self.cache.read();
        cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get cache statistics
    pub fn stats(&self) -> Option<CacheStatistics> {
        let stats = self.stats.lock();
        stats.as_ref().map(|s| s.snapshot())
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        if let Some(stats) = self.stats.lock().as_mut() {
            stats.reset();
        }
    }

    // Internal stats recording
    fn record_hit(&self) {
        if let Some(stats) = self.stats.lock().as_mut() {
            stats.hits += 1;
        }
    }

    fn record_miss(&self) {
        if let Some(stats) = self.stats.lock().as_mut() {
            stats.misses += 1;
        }
    }

    fn record_set(&self) {
        if let Some(stats) = self.stats.lock().as_mut() {
            stats.sets += 1;
        }
    }
}

#[async_trait::async_trait]
impl HealthCheckable for MemoryCacheInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        // In-memory cache is always healthy if accessible
        Ok(HealthStatus::healthy())
    }

    async fn detailed_health_check(
        &self,
        _context: &ResourceContext,
    ) -> ResourceResult<HealthStatus> {
        let cache = self.cache.read();
        let size = cache.len();
        let capacity = cache.capacity();

        let mut status = HealthStatus::healthy()
            .with_metadata("size", size.to_string())
            .with_metadata("capacity", capacity.to_string())
            .with_metadata(
                "utilization",
                format!("{:.1}%", (size as f64 / capacity as f64) * 100.0),
            );

        if let Some(stats) = self.stats() {
            status = status
                .with_metadata("hits", stats.hits.to_string())
                .with_metadata("misses", stats.misses.to_string())
                .with_metadata("hit_rate", format!("{:.2}%", stats.hit_rate() * 100.0));
        }

        Ok(status)
    }
}

/// Cache entry with expiration
#[derive(Debug, Clone)]
struct CacheEntry {
    value: String,
    expires_at: Option<Instant>,
    created_at: Instant,
}

/// LRU Cache implementation
struct LruCache<K: Hash + Eq, V> {
    map: HashMap<K, (V, usize)>, // (value, access_order)
    capacity: usize,
    access_counter: usize,
}

impl<K: Hash + Eq + Clone, V> LruCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            capacity,
            access_counter: 0,
        }
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        if let Some((value, order)) = self.map.get_mut(key) {
            self.access_counter += 1;
            *order = self.access_counter;
            Some(value)
        } else {
            None
        }
    }

    fn insert(&mut self, key: K, value: V) {
        self.access_counter += 1;

        // If at capacity, evict LRU
        if self.map.len() >= self.capacity && !self.map.contains_key(&key) {
            self.evict_lru();
        }

        self.map.insert(key, (value, self.access_counter));
    }

    fn remove(&mut self, key: &K) -> Option<V> {
        self.map.remove(key).map(|(v, _)| v)
    }

    fn clear(&mut self) {
        self.map.clear();
        self.access_counter = 0;
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn capacity(&self) -> usize {
        self.capacity
    }

    fn evict_lru(&mut self) {
        // Find entry with smallest access_order
        if let Some((lru_key, _)) = self.map.iter().min_by_key(|(_, (_, order))| order) {
            let lru_key = lru_key.clone();
            self.map.remove(&lru_key);
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
struct CacheStats {
    hits: u64,
    misses: u64,
    sets: u64,
}

impl CacheStats {
    fn new() -> Self {
        Self::default()
    }

    fn snapshot(&self) -> CacheStatistics {
        CacheStatistics {
            hits: self.hits,
            misses: self.misses,
            sets: self.sets,
        }
    }

    fn reset(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.sets = 0;
    }
}

/// Public cache statistics
#[derive(Debug, Clone)]
pub struct CacheStatistics {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of cache sets
    pub sets: u64,
}

impl CacheStatistics {
    /// Calculate hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Calculate total requests
    pub fn total_requests(&self) -> u64 {
        self.hits + self.misses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_cache_basic_operations() {
        let config = MemoryCacheConfig {
            max_entries: 10,
            default_ttl: None,
            enable_stats: true,
        };

        let resource = MemoryCacheResource;
        let context = ResourceContext::new(
            "test-wf".to_string(),
            "test-exec".to_string(),
            "dev".to_string(),
            "test-tenant".to_string(),
        );

        let cache = resource.create(&config, &context).await.unwrap();

        // Test set and get
        cache.set("key1", "value1");
        assert_eq!(cache.get("key1"), Some("value1".to_string()));

        // Test missing key
        assert_eq!(cache.get("key2"), None);

        // Test remove
        cache.set("key2", "value2");
        assert!(cache.remove("key2"));
        assert_eq!(cache.get("key2"), None);

        // Test stats
        let stats = cache.stats().unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.sets, 2);
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let config = MemoryCacheConfig {
            max_entries: 3,
            default_ttl: None,
            enable_stats: false,
        };

        let resource = MemoryCacheResource;
        let context = ResourceContext::new(
            "test-wf".to_string(),
            "test-exec".to_string(),
            "dev".to_string(),
            "test-tenant".to_string(),
        );

        let cache = resource.create(&config, &context).await.unwrap();

        // Fill cache
        cache.set("k1", "v1");
        cache.set("k2", "v2");
        cache.set("k3", "v3");

        assert_eq!(cache.len(), 3);

        // Access k1 to make it more recent
        cache.get("k1");

        // Add k4, should evict k2 (least recently used)
        cache.set("k4", "v4");

        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get("k1"), Some("v1".to_string())); // Still there
        assert_eq!(cache.get("k2"), None); // Evicted
        assert_eq!(cache.get("k4"), Some("v4".to_string())); // New
    }

    #[tokio::test]
    async fn test_ttl_expiration() {
        let config = MemoryCacheConfig {
            max_entries: 10,
            default_ttl: Some(Duration::from_millis(100)),
            enable_stats: false,
        };

        let resource = MemoryCacheResource;
        let context = ResourceContext::new(
            "test-wf".to_string(),
            "test-exec".to_string(),
            "dev".to_string(),
            "test-tenant".to_string(),
        );

        let cache = resource.create(&config, &context).await.unwrap();

        cache.set("expire", "soon");
        assert_eq!(cache.get("expire"), Some("soon".to_string()));

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(cache.get("expire"), None); // Expired
    }
}
