//! Caching system for validation results
//! 
//! This module provides a flexible caching system for validation results,
//! improving performance by avoiding redundant validation operations.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde_json::Value;
use tracing::{debug, info, warn, trace};
use crate::types::{ValidationResult, ValidationError, ValidationConfig};

// ==================== Cache Entry ====================

/// A cached validation result
#[derive(Debug, Clone)]
struct CacheEntry {
    /// The validation result
    result: ValidationResult<()>,
    /// When this entry was created
    created_at: Instant,
    /// When this entry expires
    expires_at: Instant,
    /// Number of times this entry has been accessed
    access_count: u64,
    /// Last access time
    last_accessed: Instant,
}

impl CacheEntry {
    /// Create a new cache entry
    fn new(result: ValidationResult<()>, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            result,
            created_at: now,
            expires_at: now + ttl,
            access_count: 0,
            last_accessed: now,
        }
    }
    
    /// Check if the entry has expired
    fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }
    
    /// Mark the entry as accessed
    fn mark_accessed(&mut self) {
        self.access_count += 1;
        self.last_accessed = Instant::now();
    }
    
    /// Get the age of the entry
    fn age(&self) -> Duration {
        Instant::now().duration_since(self.created_at)
    }
    
    /// Get the time until expiration
    fn time_until_expiry(&self) -> Duration {
        if self.is_expired() {
            Duration::ZERO
        } else {
            self.expires_at.duration_since(Instant::now())
        }
    }
}

// ==================== Cache Key ====================

/// A cache key for validation results
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// The value being validated (serialized to string)
    value_hash: String,
    /// The validator ID
    validator_id: String,
    /// Additional context information
    context_hash: Option<String>,
}

impl CacheKey {
    /// Create a new cache key
    pub fn new(value: &Value, validator_id: &str, context: Option<&str>) -> Self {
        let value_hash = Self::hash_value(value);
        let context_hash = context.map(|c| Self::hash_string(c));
        
        Self {
            value_hash,
            validator_id: validator_id.to_string(),
            context_hash,
        }
    }
    
    /// Create a simple cache key without context
    pub fn simple(value: &Value, validator_id: &str) -> Self {
        Self::new(value, validator_id, None)
    }
    
    /// Hash a JSON value
    fn hash_value(value: &Value) -> String {
        // Simple hash for now - in production you might want a more sophisticated approach
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        if let Ok(bytes) = serde_json::to_vec(value) {
            bytes.hash(&mut hasher);
        }
        format!("{:x}", hasher.finish())
    }
    
    /// Hash a string
    fn hash_string(s: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

// ==================== Validation Cache ====================

/// Cache for validation results
/// 
/// This cache stores validation results to avoid redundant validation operations.
/// It supports configurable TTL, size limits, and eviction policies.
#[derive(Debug)]
pub struct ValidationCache {
    /// Cached validation results
    entries: Arc<RwLock<HashMap<CacheKey, CacheEntry>>>,
    /// Configuration for the cache
    config: CacheConfig,
    /// Cache statistics
    stats: Arc<RwLock<CacheStats>>,
}

impl ValidationCache {
    /// Create a new validation cache
    pub fn new(config: CacheConfig) -> Self {
        info!("Creating validation cache with config: {:?}", config);
        
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            config,
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }
    
    /// Create a new cache with default configuration
    pub fn new_default() -> Self {
        Self::new(CacheConfig::default())
    }
    
    /// Get a cached validation result
    /// 
    /// # Arguments
    /// * `key` - The cache key
    /// 
    /// # Returns
    /// * `Option<ValidationResult<()>>` - The cached result if found and valid
    pub async fn get(&self, key: &CacheKey) -> Option<ValidationResult<()>> {
        let mut entries = self.entries.write().await;
        
        if let Some(entry) = entries.get_mut(key) {
            // Check if entry has expired
            if entry.is_expired() {
                trace!("Cache entry expired for key: {:?}", key);
                entries.remove(key);
                self.update_stats_eviction().await;
                return None;
            }
            
            // Mark as accessed
            entry.mark_accessed();
            self.update_stats_hit().await;
            
            trace!("Cache hit for key: {:?}", key);
            Some(entry.result.clone())
        } else {
            self.update_stats_miss().await;
            trace!("Cache miss for key: {:?}", key);
            None
        }
    }
    
    /// Store a validation result in the cache
    /// 
    /// # Arguments
    /// * `key` - The cache key
    /// * `result` - The validation result to cache
    /// * `ttl` - Time to live for the cache entry
    /// 
    /// # Returns
    /// * `Result<(), CacheError>` - Success or failure
    pub async fn set(
        &self,
        key: CacheKey,
        result: ValidationResult<()>,
        ttl: Duration,
    ) -> Result<(), CacheError> {
        // Check if we need to evict entries
        self.ensure_capacity().await?;
        
        let entry = CacheEntry::new(result, ttl);
        
        {
            let mut entries = self.entries.write().await;
            entries.insert(key.clone(), entry);
        }
        
        self.update_stats_set().await;
        trace!("Cached validation result for key: {:?}", key);
        
        Ok(())
    }
    
    /// Store a validation result with default TTL
    pub async fn set_default(&self, key: CacheKey, result: ValidationResult<()>) -> Result<(), CacheError> {
        self.set(key, result, self.config.default_ttl).await
    }
    
    /// Remove a specific entry from the cache
    pub async fn remove(&self, key: &CacheKey) -> bool {
        let mut entries = self.entries.write().await;
        let removed = entries.remove(key).is_some();
        
        if removed {
            self.update_stats_removal().await;
        }
        
        removed
    }
    
    /// Clear all entries from the cache
    pub async fn clear(&self) {
        info!("Clearing validation cache");
        
        {
            let mut entries = self.entries.write().await;
            entries.clear();
        }
        
        {
            let mut stats = self.stats.write().await;
            stats.reset();
        }
        
        info!("Validation cache cleared");
    }
    
    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let stats = self.stats.read().await;
        stats.clone()
    }
    
    /// Get cache size
    pub async fn size(&self) -> usize {
        let entries = self.entries.read().await;
        entries.len()
    }
    
    /// Check if cache is empty
    pub async fn is_empty(&self) -> bool {
        self.size().await == 0
    }
    
    /// Clean up expired entries
    pub async fn cleanup(&self) -> usize {
        let mut entries = self.entries.write().await;
        let initial_size = entries.len();
        
        entries.retain(|_, entry| !entry.is_expired());
        
        let removed = initial_size - entries.len();
        if removed > 0 {
            info!("Cleaned up {} expired cache entries", removed);
        }
        
        removed
    }
    
    /// Ensure the cache doesn't exceed capacity
    async fn ensure_capacity(&self) -> Result<(), CacheError> {
        let current_size = self.size().await;
        
        if current_size >= self.config.max_entries {
            match self.config.eviction_policy {
                EvictionPolicy::LRU => self.evict_lru().await,
                EvictionPolicy::LFU => self.evict_lfu().await,
                EvictionPolicy::FIFO => self.evict_fifo().await,
                EvictionPolicy::Random => self.evict_random().await,
            }
        }
        
        Ok(())
    }
    
    /// Evict least recently used entries
    async fn evict_lru(&self) {
        let mut entries = self.entries.write().await;
        let mut entries_vec: Vec<_> = entries.drain().collect();
        
        // Sort by last accessed time (oldest first)
        entries_vec.sort_by_key(|(_, entry)| entry.last_accessed);
        
        // Keep only the most recent entries
        let to_keep = self.config.max_entries.saturating_sub(self.config.eviction_batch_size);
        let to_remove = entries_vec.len().saturating_sub(to_keep);
        
        if to_remove > 0 {
            info!("Evicting {} LRU cache entries", to_remove);
            
            // Remove oldest entries
            entries_vec.truncate(to_keep);
            
            // Re-insert remaining entries
            for (key, entry) in entries_vec {
                entries.insert(key, entry);
            }
        }
    }
    
    /// Evict least frequently used entries
    async fn evict_lfu(&self) {
        let mut entries = self.entries.write().await;
        let mut entries_vec: Vec<_> = entries.drain().collect();
        
        // Sort by access count (lowest first)
        entries_vec.sort_by_key(|(_, entry)| entry.access_count);
        
        // Keep only the most frequently used entries
        let to_keep = self.config.max_entries.saturating_sub(self.config.eviction_batch_size);
        let to_remove = entries_vec.len().saturating_sub(to_keep);
        
        if to_remove > 0 {
            info!("Evicting {} LFU cache entries", to_remove);
            
            // Remove least frequently used entries
            entries_vec.truncate(to_keep);
            
            // Re-insert remaining entries
            for (key, entry) in entries_vec {
                entries.insert(key, entry);
            }
        }
    }
    
    /// Evict entries in FIFO order
    async fn evict_fifo(&self) {
        let mut entries = self.entries.write().await;
        let mut entries_vec: Vec<_> = entries.drain().collect();
        
        // Sort by creation time (oldest first)
        entries_vec.sort_by_key(|(_, entry)| entry.created_at);
        
        // Keep only the most recent entries
        let to_keep = self.config.max_entries.saturating_sub(self.config.eviction_batch_size);
        let to_remove = entries_vec.len().saturating_sub(to_keep);
        
        if to_remove > 0 {
            info!("Evicting {} FIFO cache entries", to_remove);
            
            // Remove oldest entries
            entries_vec.truncate(to_keep);
            
            // Re-insert remaining entries
            for (key, entry) in entries_vec {
                entries.insert(key, entry);
            }
        }
    }
    
    /// Evict random entries
    async fn evict_random(&self) {
        let mut entries = self.entries.write().await;
        let mut entries_vec: Vec<_> = entries.drain().collect();
        
        // Shuffle entries randomly
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        entries_vec.shuffle(&mut rng);
        
        // Keep only the first entries
        let to_keep = self.config.max_entries.saturating_sub(self.config.eviction_batch_size);
        let to_remove = entries_vec.len().saturating_sub(to_keep);
        
        if to_remove > 0 {
            info!("Evicting {} random cache entries", to_remove);
            
            // Remove random entries
            entries_vec.truncate(to_keep);
            
            // Re-insert remaining entries
            for (key, entry) in entries_vec {
                entries.insert(key, entry);
            }
        }
    }
    
    /// Update statistics for cache hit
    async fn update_stats_hit(&self) {
        let mut stats = self.stats.write().await;
        stats.hits += 1;
        stats.total_requests += 1;
    }
    
    /// Update statistics for cache miss
    async fn update_stats_miss(&self) {
        let mut stats = self.stats.write().await;
        stats.misses += 1;
        stats.total_requests += 1;
    }
    
    /// Update statistics for cache set
    async fn update_stats_set(&self) {
        let mut stats = self.stats.write().await;
        stats.sets += 1;
    }
    
    /// Update statistics for cache eviction
    async fn update_stats_eviction(&self) {
        let mut stats = self.stats.write().await;
        stats.evictions += 1;
    }
    
    /// Update statistics for cache removal
    async fn update_stats_removal(&self) {
        let mut stats = self.stats.write().await;
        stats.removals += 1;
    }
}

// ==================== Cache Configuration ====================

/// Configuration for the validation cache
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries in the cache
    pub max_entries: usize,
    /// Default time to live for cache entries
    pub default_ttl: Duration,
    /// Eviction policy when cache is full
    pub eviction_policy: EvictionPolicy,
    /// Number of entries to evict in each batch
    pub eviction_batch_size: usize,
    /// Whether to enable cache statistics
    pub enable_stats: bool,
    /// Whether to enable cache cleanup
    pub enable_cleanup: bool,
    /// Cleanup interval
    pub cleanup_interval: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            default_ttl: Duration::from_secs(300), // 5 minutes
            eviction_policy: EvictionPolicy::LRU,
            eviction_batch_size: 100,
            enable_stats: true,
            enable_cleanup: true,
            cleanup_interval: Duration::from_secs(60), // 1 minute
        }
    }
}

impl CacheConfig {
    /// Create new cache configuration
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set maximum entries
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries;
        self
    }
    
    /// Set default TTL
    pub fn with_default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl;
        self
    }
    
    /// Set eviction policy
    pub fn with_eviction_policy(mut self, policy: EvictionPolicy) -> Self {
        self.eviction_policy = policy;
        self
    }
    
    /// Set eviction batch size
    pub fn with_eviction_batch_size(mut self, size: usize) -> Self {
        self.eviction_batch_size = size;
        self
    }
    
    /// Disable statistics
    pub fn without_stats(mut self) -> Self {
        self.enable_stats = false;
        self
    }
    
    /// Disable cleanup
    pub fn without_cleanup(mut self) -> Self {
        self.enable_cleanup = false;
        self
    }
    
    /// Set cleanup interval
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = interval;
        self
    }
}

// ==================== Eviction Policy ====================

/// Cache eviction policy
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In, First Out
    FIFO,
    /// Random eviction
    Random,
}

// ==================== Cache Statistics ====================

/// Statistics about cache performance
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total number of cache hits
    pub hits: u64,
    /// Total number of cache misses
    pub misses: u64,
    /// Total number of cache sets
    pub sets: u64,
    /// Total number of cache evictions
    pub evictions: u64,
    /// Total number of cache removals
    pub removals: u64,
    /// Total number of requests
    pub total_requests: u64,
}

impl CacheStats {
    /// Get hit rate as a percentage
    pub fn hit_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            (self.hits as f64 / self.total_requests as f64) * 100.0
        }
    }
    
    /// Get miss rate as a percentage
    pub fn miss_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            (self.misses as f64 / self.total_requests as f64) * 100.0
        }
    }
    
    /// Reset all statistics
    pub fn reset(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.sets = 0;
        self.evictions = 0;
        self.removals = 0;
        self.total_requests = 0;
    }
}

// ==================== Cache Errors ====================

/// Errors that can occur during cache operations
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// Cache is full and eviction failed
    #[error("Cache is full and eviction failed")]
    CacheFull,
    
    /// Invalid cache configuration
    #[error("Invalid cache configuration: {}", .0)]
    InvalidConfig(String),
    
    /// Cache operation failed
    #[error("Cache operation failed: {}", .0)]
    OperationFailed(String),
}

// ==================== Cache Builder ====================

/// Builder for creating validation caches with custom configuration
#[derive(Debug)]
pub struct CacheBuilder {
    config: CacheConfig,
}

impl CacheBuilder {
    /// Create a new cache builder
    pub fn new() -> Self {
        Self {
            config: CacheConfig::default(),
        }
    }
    
    /// Set maximum entries
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.config.max_entries = max_entries;
        self
    }
    
    /// Set default TTL
    pub fn with_default_ttl(mut self, ttl: Duration) -> Self {
        self.config.default_ttl = ttl;
        self
    }
    
    /// Set eviction policy
    pub fn with_eviction_policy(mut self, policy: EvictionPolicy) -> Self {
        self.config.eviction_policy = policy;
        self
    }
    
    /// Set eviction batch size
    pub fn with_eviction_batch_size(mut self, size: usize) -> Self {
        self.config.eviction_batch_size = size;
        self
    }
    
    /// Disable statistics
    pub fn without_stats(mut self) -> Self {
        self.config.enable_stats = false;
        self
    }
    
    /// Disable cleanup
    pub fn without_cleanup(mut self) -> Self {
        self.config.enable_cleanup = false;
        self
    }
    
    /// Set cleanup interval
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.config.cleanup_interval = interval;
        self
    }
    
    /// Build the cache
    pub fn build(self) -> ValidationCache {
        ValidationCache::new(self.config)
    }
}

impl Default for CacheBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Re-exports ====================

pub use ValidationCache as Cache;
pub use CacheBuilder as Builder;
pub use CacheConfig as Config;
pub use CacheStats as Stats;
pub use CacheError as Error;
pub use CacheKey as Key;
pub use EvictionPolicy as Policy;
