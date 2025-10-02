//! Multi-level cache implementation
//!
//! This module provides a hierarchical cache with multiple levels,
//! similar to CPU caches (L1, L2, L3), where each level has different
//! performance characteristics.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, string::String, sync::Arc, vec::Vec},
    hashbrown::HashMap,
    spin::{Mutex, RwLock},
};

use super::compute::{CacheKey, CacheResult, ComputeCache};
use super::config::{CacheConfig, CacheMetrics};
use crate::core::error::{MemoryError, MemoryResult};

/// A level in a multi-level cache
pub trait CacheLevel<K, V>: Send + Sync {
    /// Try to get a value from this cache level
    fn get(&self, key: &K) -> Option<V>;

    /// Insert a value into this cache level
    fn insert(&self, key: K, value: V) -> MemoryResult<()>;

    /// Remove a value from this cache level
    fn remove(&self, key: &K) -> Option<V>;

    /// Clear all entries from this cache level
    fn clear(&self);

    /// Get the number of entries in this cache level
    fn len(&self) -> usize;

    /// Check if this cache level is empty
    fn is_empty(&self) -> bool;

    /// Get the name of this cache level
    fn name(&self) -> &str;

    /// Get the priority/level index (lower = higher priority)
    fn priority(&self) -> usize;

    /// Get the capacity of this cache level
    fn capacity(&self) -> usize;

    /// Get the current load factor
    fn load_factor(&self) -> f32;

    /// Check if the cache level is full
    fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Get the metrics for this cache level
    #[cfg(feature = "std")]
    fn metrics(&self) -> Option<CacheMetrics>;

    /// Reset metrics for this cache level
    #[cfg(feature = "std")]
    fn reset_metrics(&self);

    /// Warm up the cache with key-value pairs
    fn warm_up_entries(&self, entries: &[(K, V)]) -> MemoryResult<()>
    where
        K: Clone,
        V: Clone;

    /// Check if cache level supports TTL
    fn supports_ttl(&self) -> bool {
        false
    }

    /// Clean up expired entries (if supported)
    #[cfg(feature = "std")]
    fn cleanup_expired(&self) -> usize {
        0
    }
}

/// Extension trait for cache levels that provides additional warm-up functionality
pub trait CacheLevelExt<K, V>: CacheLevel<K, V> {
    /// Warm up the cache with an iterator of entries
    fn warm_up<I>(&self, entries: I) -> MemoryResult<()>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Clone,
        V: Clone,
    {
        let entries_vec: Vec<_> = entries.into_iter().collect();
        self.warm_up_entries(&entries_vec)
    }

    /// Warm up with keys using a compute function
    fn warm_up_with_compute<I, F>(&self, keys: I, compute_fn: F) -> MemoryResult<()>
    where
        I: IntoIterator<Item = K>,
        F: Fn(&K) -> MemoryResult<V>,
        K: Clone,
        V: Clone,
    {
        let entries: Result<Vec<_>, _> = keys
            .into_iter()
            .map(|key| compute_fn(&key).map(|value| (key, value)))
            .collect();

        match entries {
            Ok(entries_vec) => self.warm_up_entries(&entries_vec),
            Err(e) => Err(e),
        }
    }
}

/// Statistics for a multi-level cache
#[derive(Debug, Clone, Default)]
pub struct MultiLevelStats {
    /// Number of requests
    pub requests: usize,
    /// Number of hits at each level
    pub level_hits: Vec<usize>,
    /// Number of misses (not found in any level)
    pub misses: usize,
    /// Number of promotions between levels
    pub promotions: usize,
    /// Number of demotions between levels
    pub demotions: usize,
    /// Total compute time for cache misses
    #[cfg(feature = "std")]
    pub compute_time_ns: u64,
    /// Cache efficiency by level
    pub level_efficiency: Vec<f64>,
    /// Average response time by level
    #[cfg(feature = "std")]
    pub avg_response_time_ns: Vec<u64>,
}

impl MultiLevelStats {
    /// Create a new stats object with the given number of levels
    pub fn new(level_count: usize) -> Self {
        Self {
            requests: 0,
            level_hits: vec![0; level_count],
            misses: 0,
            promotions: 0,
            demotions: 0,
            #[cfg(feature = "std")]
            compute_time_ns: 0,
            level_efficiency: vec![0.0; level_count],
            #[cfg(feature = "std")]
            avg_response_time_ns: vec![0; level_count],
        }
    }

    /// Calculate the hit rate for a specific level
    pub fn level_hit_rate(&self, level: usize) -> f64 {
        if level >= self.level_hits.len() || self.requests == 0 {
            return 0.0;
        }

        self.level_hits[level] as f64 / self.requests as f64
    }

    /// Calculate the overall hit rate (across all levels)
    pub fn overall_hit_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }

        let total_hits: usize = self.level_hits.iter().sum();
        total_hits as f64 / self.requests as f64
    }

    /// Calculate miss rate
    pub fn miss_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }
        self.misses as f64 / self.requests as f64
    }

    /// Calculate promotion rate
    pub fn promotion_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }
        self.promotions as f64 / self.requests as f64
    }

    /// Get the most effective cache level
    pub fn most_effective_level(&self) -> Option<usize> {
        self.level_hits
            .iter()
            .enumerate()
            .max_by_key(|(_, hits)| *hits)
            .map(|(idx, _)| idx)
    }

    /// Calculate average access depth (lower is better)
    pub fn avg_access_depth(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }

        let weighted_depth: f64 = self.level_hits
            .iter()
            .enumerate()
            .map(|(level, &hits)| (level + 1) as f64 * hits as f64)
            .sum();

        let total_hits: usize = self.level_hits.iter().sum();
        if total_hits == 0 {
            return f64::INFINITY; // All misses
        }

        weighted_depth / total_hits as f64
    }

    /// Reset all stats to zero
    pub fn reset(&mut self) {
        self.requests = 0;
        for hits in &mut self.level_hits {
            *hits = 0;
        }
        self.misses = 0;
        self.promotions = 0;
        self.demotions = 0;
        #[cfg(feature = "std")]
        {
            self.compute_time_ns = 0;
        }
        for eff in &mut self.level_efficiency {
            *eff = 0.0;
        }
        #[cfg(feature = "std")]
        for time in &mut self.avg_response_time_ns {
            *time = 0;
        }
    }

    /// Update efficiency metrics
    pub fn update_efficiency(&mut self, level_metrics: &[Option<CacheMetrics>]) {
        for (idx, metrics_opt) in level_metrics.iter().enumerate() {
            if let Some(metrics) = metrics_opt {
                if idx < self.level_efficiency.len() {
                    self.level_efficiency[idx] = metrics.efficiency_score();
                }
            }
        }
    }
}

/// Configuration for promotion policy in multi-level cache
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PromotionPolicy {
    /// Always promote to higher levels on access
    Always,
    /// Promote after N accesses
    AfterNAccesses(usize),
    /// Promote based on frequency threshold (0.0 to 1.0)
    FrequencyBased(f64),
    /// Never promote automatically
    Never,
    /// Adaptive promotion based on cache performance
    Adaptive,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        PromotionPolicy::Adaptive
    }
}

/// Demotion policy for moving items to lower levels
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DemotionPolicy {
    /// Never demote
    Never,
    /// Demote least recently used items when cache is full
    LRU,
    /// Demote least frequently used items
    LFU,
    /// Demote based on age
    Age,
    /// Adaptive demotion
    Adaptive,
}

impl Default for DemotionPolicy {
    fn default() -> Self {
        DemotionPolicy::LRU
    }
}

/// Configuration for multi-level cache
#[derive(Debug, Clone)]
pub struct MultiLevelConfig {
    /// Promotion policy between levels
    pub promotion_policy: PromotionPolicy,
    /// Demotion policy between levels
    pub demotion_policy: DemotionPolicy,
    /// Whether to track statistics
    pub track_stats: bool,
    /// Enable background cleanup
    pub background_cleanup: bool,
    /// Cleanup interval for expired entries
    #[cfg(feature = "std")]
    pub cleanup_interval: Option<Duration>,
    /// Maximum number of items to promote per operation
    pub max_promotions_per_op: usize,
    /// Enable write-through to all levels
    pub write_through: bool,
    /// Enable read-ahead optimization
    pub read_ahead: bool,
}

impl Default for MultiLevelConfig {
    fn default() -> Self {
        Self {
            promotion_policy: PromotionPolicy::default(),
            demotion_policy: DemotionPolicy::default(),
            track_stats: false,
            background_cleanup: false,
            #[cfg(feature = "std")]
            cleanup_interval: None,
            max_promotions_per_op: 3,
            write_through: true,
            read_ahead: false,
        }
    }
}

impl MultiLevelConfig {
    /// Create a new configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set promotion policy
    pub fn with_promotion_policy(mut self, policy: PromotionPolicy) -> Self {
        self.promotion_policy = policy;
        self
    }

    /// Set demotion policy
    pub fn with_demotion_policy(mut self, policy: DemotionPolicy) -> Self {
        self.demotion_policy = policy;
        self
    }

    /// Enable statistics tracking
    pub fn with_stats(mut self) -> Self {
        self.track_stats = true;
        self
    }

    /// Enable background cleanup
    pub fn with_background_cleanup(mut self) -> Self {
        self.background_cleanup = true;
        self
    }

    /// Set cleanup interval
    #[cfg(feature = "std")]
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = Some(interval);
        self.background_cleanup = true;
        self
    }

    /// Configure for high performance
    pub fn for_high_performance() -> Self {
        Self::new()
            .with_promotion_policy(PromotionPolicy::Always)
            .with_demotion_policy(DemotionPolicy::LFU)
            .with_stats()
    }

    /// Configure for memory efficiency
    pub fn for_memory_efficiency() -> Self {
        Self::new()
            .with_promotion_policy(PromotionPolicy::AfterNAccesses(3))
            .with_demotion_policy(DemotionPolicy::LRU)
            .with_background_cleanup()
    }
}

/// A multi-level cache with hierarchical storage
pub struct MultiLevelCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// The cache levels, ordered from fastest/smallest to slowest/largest
    levels: Vec<Box<dyn CacheLevel<K, V>>>,
    /// Configuration
    config: MultiLevelConfig,
    /// Access counts for keys (used with frequency-based policies)
    access_counts: Arc<RwLock<HashMap<K, usize>>>,
    /// Cache statistics
    stats: Arc<RwLock<MultiLevelStats>>,
    /// Background cleanup handle
    #[cfg(feature = "std")]
    _cleanup_handle: Option<std::thread::JoinHandle<()>>,
}

impl<K, V> MultiLevelCache<K, V>
where
    K: CacheKey + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new multi-level cache with the given levels
    pub fn new(levels: Vec<Box<dyn CacheLevel<K, V>>>) -> Self {
        let level_count = levels.len();
        Self {
            levels,
            config: MultiLevelConfig::default(),
            access_counts: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(MultiLevelStats::new(level_count))),
            #[cfg(feature = "std")]
            _cleanup_handle: None,
        }
    }

    /// Create a new multi-level cache with configuration
    pub fn with_config(levels: Vec<Box<dyn CacheLevel<K, V>>>, config: MultiLevelConfig) -> Self {
        let level_count = levels.len();
        let mut cache = Self {
            levels,
            config,
            access_counts: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(MultiLevelStats::new(level_count))),
            #[cfg(feature = "std")]
            _cleanup_handle: None,
        };

        #[cfg(feature = "std")]
        if cache.config.background_cleanup {
            cache.start_background_cleanup();
        }

        cache
    }

    /// Start background cleanup thread
    #[cfg(feature = "std")]
    fn start_background_cleanup(&mut self) {
        if let Some(interval) = self.config.cleanup_interval {
            let levels_clone = self.levels.iter()
                .map(|level| level.name().to_string())
                .collect::<Vec<_>>();

            // TODO: Implement proper cleanup thread with shared references
            // This is a simplified version - in real implementation you'd need
            // to share references to the cache levels safely
        }
    }

    /// Get a value from the cache, trying each level in order
    pub fn get(&self, key: &K) -> Option<V> {
        #[cfg(feature = "std")]
        let start_time = Instant::now();

        if self.config.track_stats {
            let mut stats = self.stats.write().unwrap();
            stats.requests += 1;
        }

        // Try to get the value from each level, starting from the fastest
        for (level_idx, level) in self.levels.iter().enumerate() {
            if let Some(value) = level.get(key) {
                // Found in this level
                if self.config.track_stats {
                    let mut stats = self.stats.write().unwrap();
                    stats.level_hits[level_idx] += 1;

                    #[cfg(feature = "std")]
                    {
                        let response_time = start_time.elapsed().as_nanos() as u64;
                        if level_idx < stats.avg_response_time_ns.len() {
                            // Simple moving average
                            stats.avg_response_time_ns[level_idx] =
                                (stats.avg_response_time_ns[level_idx] + response_time) / 2;
                        }
                    }
                }

                // Update access count
                if matches!(self.config.promotion_policy,
                    PromotionPolicy::AfterNAccesses(_) | PromotionPolicy::FrequencyBased(_))
                {
                    let mut access_counts = self.access_counts.write().unwrap();
                    *access_counts.entry(key.clone()).or_insert(0) += 1;
                }

                // Promote to higher levels if needed
                self.maybe_promote(key, &value, level_idx);

                return Some(value);
            }
        }

        // Not found in any level
        if self.config.track_stats {
            let mut stats = self.stats.write().unwrap();
            stats.misses += 1;
        }

        None
    }

    /// Get a value from the cache, computing it if not present in any level
    pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> CacheResult<V>
    where F: FnOnce() -> Result<V, MemoryError> {
        // Try to get from cache first
        if let Some(value) = self.get(&key) {
            return Ok(value);
        }

        // Compute the value
        #[cfg(feature = "std")]
        let start_time = Instant::now();

        let value = compute_fn()?;

        #[cfg(feature = "std")]
        if self.config.track_stats {
            let compute_time = start_time.elapsed().as_nanos() as u64;
            let mut stats = self.stats.write().unwrap();
            stats.compute_time_ns += compute_time;
        }

        // Insert into appropriate levels
        self.insert(key, value.clone())?;

        Ok(value)
    }

    /// Get multiple values from cache
    pub fn get_batch<I>(&self, keys: I) -> Vec<(K, Option<V>)>
    where
        I: IntoIterator<Item = K>,
        K: Clone,
    {
        keys.into_iter()
            .map(|key| {
                let value = self.get(&key);
                (key, value)
            })
            .collect()
    }

    /// Get or compute multiple values
    pub fn get_or_compute_batch<I, F>(&self, keys: I, compute_fn: F) -> Vec<CacheResult<V>>
    where
        I: IntoIterator<Item = K>,
        F: Fn(&K) -> Result<V, MemoryError>,
        K: Clone,
    {
        keys.into_iter()
            .map(|key| {
                self.get_or_compute(key.clone(), || compute_fn(&key))
            })
            .collect()
    }

    /// Insert a value into appropriate cache levels
    pub fn insert(&self, key: K, value: V) -> MemoryResult<()> {
        if self.config.write_through {
            // Insert into all levels
            for level in &self.levels {
                level.insert(key.clone(), value.clone())?;
            }
        } else {
            // Insert only into first level (write-back)
            if !self.levels.is_empty() {
                self.levels[0].insert(key, value)?;
            }
        }
        Ok(())
    }

    /// Remove a value from all cache levels
    pub fn remove(&self, key: &K) -> Option<V> {
        let mut result = None;

        for level in &self.levels {
            if let Some(value) = level.remove(key) {
                result = Some(value);
            }
        }

        // Also remove from access counts
        if matches!(self.config.promotion_policy,
            PromotionPolicy::AfterNAccesses(_) | PromotionPolicy::FrequencyBased(_))
        {
            let mut access_counts = self.access_counts.write().unwrap();
            access_counts.remove(key);
        }

        result
    }

    /// Clear all cache levels
    pub fn clear(&self) {
        for level in &self.levels {
            level.clear();
        }

        // Also clear access counts
        let mut access_counts = self.access_counts.write().unwrap();
        access_counts.clear();

        // Reset stats
        if self.config.track_stats {
            let mut stats = self.stats.write().unwrap();
            stats.reset();
        }
    }

    /// Get the number of levels in the cache
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Get information about a specific level
    pub fn level_info(&self, level: usize) -> Option<LevelInfo> {
        self.levels.get(level).map(|l| LevelInfo {
            name: l.name().to_string(),
            priority: l.priority(),
            capacity: l.capacity(),
            current_size: l.len(),
            load_factor: l.load_factor(),
            is_full: l.is_full(),
            supports_ttl: l.supports_ttl(),
        })
    }

    /// Get information about all levels
    pub fn all_levels_info(&self) -> Vec<LevelInfo> {
        (0..self.level_count())
            .filter_map(|i| self.level_info(i))
            .collect()
    }

    /// Get the statistics for the multi-level cache
    pub fn stats(&self) -> MultiLevelStats {
        if self.config.track_stats {
            let mut stats = self.stats.read().unwrap().clone();

            // Update efficiency metrics
            let level_metrics: Vec<_> = self.levels.iter()
                .map(|level| {
                    #[cfg(feature = "std")]
                    { level.metrics() }
                    #[cfg(not(feature = "std"))]
                    { None }
                })
                .collect();

            stats.update_efficiency(&level_metrics);
            stats
        } else {
            MultiLevelStats::new(self.levels.len())
        }
    }

    /// Reset all statistics
    pub fn reset_stats(&self) {
        if self.config.track_stats {
            let mut stats = self.stats.write().unwrap();
            stats.reset();
        }

        // Reset level metrics
        #[cfg(feature = "std")]
        for level in &self.levels {
            level.reset_metrics();
        }
    }

    /// Warm up all cache levels with data
    pub fn warm_up(&self, entries: &[(K, V)]) -> MemoryResult<()>
    where
        K: Clone,
        V: Clone,
    {
        for level in &self.levels {
            level.warm_up_entries(entries)?;
        }
        Ok(())
    }

    /// Clean up expired entries in all levels
    #[cfg(feature = "std")]
    pub fn cleanup_expired(&self) -> usize {
        self.levels.iter()
            .map(|level| level.cleanup_expired())
            .sum()
    }

    /// Get cache efficiency report
    pub fn efficiency_report(&self) -> CacheEfficiencyReport {
        let stats = self.stats();
        let level_infos = self.all_levels_info();

        CacheEfficiencyReport {
            overall_hit_rate: stats.overall_hit_rate(),
            miss_rate: stats.miss_rate(),
            avg_access_depth: stats.avg_access_depth(),
            promotion_rate: stats.promotion_rate(),
            most_effective_level: stats.most_effective_level(),
            level_hit_rates: (0..self.level_count())
                .map(|i| stats.level_hit_rate(i))
                .collect(),
            level_efficiencies: stats.level_efficiency.clone(),
            level_infos,
            recommendations: self.generate_recommendations(&stats),
        }
    }

    /// Generate optimization recommendations
    fn generate_recommendations(&self, stats: &MultiLevelStats) -> Vec<String> {
        let mut recommendations = Vec::new();

        if stats.overall_hit_rate() < 0.7 {
            recommendations.push("Overall hit rate is low - consider increasing cache sizes".to_string());
        }

        if stats.avg_access_depth() > 2.0 {
            recommendations.push("High average access depth - optimize promotion policy".to_string());
        }

        if stats.promotion_rate() > 0.3 {
            recommendations.push("High promotion rate - consider more conservative promotion policy".to_string());
        }

        if let Some(most_effective) = stats.most_effective_level() {
            if most_effective > 0 {
                recommendations.push(format!(
                    "Level {} is most effective - consider increasing size of higher levels",
                    most_effective
                ));
            }
        }

        // Check individual level efficiency
        for (idx, &efficiency) in stats.level_efficiency.iter().enumerate() {
            if efficiency < 50.0 {
                recommendations.push(format!(
                    "Level {} has low efficiency ({:.1}%) - review configuration",
                    idx, efficiency
                ));
            }
        }

        if recommendations.is_empty() {
            recommendations.push("Cache hierarchy is performing well".to_string());
        }

        recommendations
    }

    /// Promote a value to higher levels based on the promotion policy
    fn maybe_promote(&self, key: &K, value: &V, found_level: usize) {
        // Don't promote if already at the highest level
        if found_level == 0 {
            return;
        }

        let should_promote = match self.config.promotion_policy {
            PromotionPolicy::Always => true,
            PromotionPolicy::AfterNAccesses(threshold) => {
                let access_counts = self.access_counts.read().unwrap();
                access_counts.get(key).map_or(false, |&count| count >= threshold)
            },
            PromotionPolicy::FrequencyBased(threshold) => {
                let access_counts = self.access_counts.read().unwrap();
                let count = access_counts.get(key).copied().unwrap_or(0);
                let total_requests = self.stats.read().unwrap().requests;
                if total_requests > 0 {
                    (count as f64 / total_requests as f64) >= threshold
                } else {
                    false
                }
            },
            PromotionPolicy::Never => false,
            PromotionPolicy::Adaptive => {
                // Simple adaptive logic: promote if cache hit rate in current level is high
                let stats = self.stats.read().unwrap();
                stats.level_hit_rate(found_level) > 0.8
            },
        };

        if should_promote {
            self.promote_to_higher_levels(key, value, found_level);
        }
    }

    /// Promote a value to higher levels (limited by max_promotions_per_op)
    fn promote_to_higher_levels(&self, key: &K, value: &V, found_level: usize) {
        let max_promotions = self.config.max_promotions_per_op.min(found_level);
        let mut promotions = 0;

        for level in (0..found_level).rev() {
            if promotions >= max_promotions {
                break;
            }

            if self.levels[level].insert(key.clone(), value.clone()).is_ok() {
                promotions += 1;

                if self.config.track_stats {
                    let mut stats = self.stats.write().unwrap();
                    stats.promotions += 1;
                }
            }
        }

        // Reset access count after promotion
        if matches!(self.config.promotion_policy, PromotionPolicy::AfterNAccesses(_)) {
            let mut access_counts = self.access_counts.write().unwrap();
            if let Some(count) = access_counts.get_mut(key) {
                *count = 0;
            }
        }
    }
}

/// Information about a cache level
#[derive(Debug, Clone)]
pub struct LevelInfo {
    pub name: String,
    pub priority: usize,
    pub capacity: usize,
    pub current_size: usize,
    pub load_factor: f32,
    pub is_full: bool,
    pub supports_ttl: bool,
}

/// Cache efficiency report
#[derive(Debug, Clone)]
pub struct CacheEfficiencyReport {
    pub overall_hit_rate: f64,
    pub miss_rate: f64,
    pub avg_access_depth: f64,
    pub promotion_rate: f64,
    pub most_effective_level: Option<usize>,
    pub level_hit_rates: Vec<f64>,
    pub level_efficiencies: Vec<f64>,
    pub level_infos: Vec<LevelInfo>,
    pub recommendations: Vec<String>,
}

/// Enhanced cache level implementation using ComputeCache
pub struct ComputeCacheLevel<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// The name of this level
    name: String,
    /// Priority/level index
    priority: usize,
    /// The underlying compute cache
    cache: Arc<Mutex<ComputeCache<K, V>>>,
}

impl<K, V> ComputeCacheLevel<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// Create a new compute cache level
    pub fn new(name: &str, priority: usize, config: CacheConfig) -> Self {
        Self {
            name: name.to_string(),
            priority,
            cache: Arc::new(Mutex::new(ComputeCache::with_config(config))),
        }
    }

    /// Create with capacity
    pub fn with_capacity(name: &str, priority: usize, capacity: usize) -> Self {
        Self::new(name, priority, CacheConfig::new(capacity))
    }
}

impl<K, V> CacheLevel<K, V> for ComputeCacheLevel<K, V>
where
    K: CacheKey + Send + Sync,
    V: Clone + Send + Sync,
{
    fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.lock().unwrap();
        cache.get(key)
    }

    fn insert(&self, key: K, value: V) -> MemoryResult<()> {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(key, value)
    }

    fn remove(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.lock().unwrap();
        cache.remove(key)
    }

    fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }

    fn len(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.len()
    }

    fn is_empty(&self) -> bool {
        let cache = self.cache.lock().unwrap();
        cache.is_empty()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> usize {
        self.priority
    }

    fn capacity(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.capacity()
    }

    fn load_factor(&self) -> f32 {
        let cache = self.cache.lock().unwrap();
        cache.load_factor()
    }

    #[cfg(feature = "std")]
    fn metrics(&self) -> Option<CacheMetrics> {
        let cache = self.cache.lock().unwrap();
        Some(cache.metrics())
    }

    #[cfg(feature = "std")]
    fn reset_metrics(&self) {
        let cache = self.cache.lock().unwrap();
        cache.reset_metrics();
    }

    fn warm_up_entries(&self, entries: &[(K, V)]) -> MemoryResult<()>
    where
        K: Clone,
        V: Clone,
    {
        let mut cache = self.cache.lock().unwrap();
        for (key, value) in entries {
            cache.insert(key.clone(), value.clone())?;
        }
        Ok(())
    }

    fn supports_ttl(&self) -> bool {
        true // ComputeCache supports TTL
    }

    #[cfg(feature = "std")]
    fn cleanup_expired(&self) -> usize {
        let mut cache = self.cache.lock().unwrap();
        cache.cleanup_expired()
    }
}

// Implement the extension trait for ComputeCacheLevel
impl<K, V> CacheLevelExt<K, V> for ComputeCacheLevel<K, V>
where
    K: CacheKey + Send + Sync,
    V: Clone + Send + Sync,
{}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_cache() -> MultiLevelCache<String, usize> {
        let l1 = ComputeCacheLevel::with_capacity("L1", 0, 10);
        let l2 = ComputeCacheLevel::with_capacity("L2", 1, 100);
        let l3 = ComputeCacheLevel::with_capacity("L3", 2, 1000);

        let levels: Vec<Box<dyn CacheLevel<String, usize>>> = vec![
            Box::new(l1),
            Box::new(l2),
            Box::new(l3),
        ];

        MultiLevelCache::with_config(
            levels,
            MultiLevelConfig::new()
                .with_promotion_policy(PromotionPolicy::Always)
                .with_stats()
        )
    }

    #[test]
    fn test_multi_level_cache_basic() {
        let cache = create_test_cache();

        // Test compute and cache
        let result = cache.get_or_compute("key1".to_string(), || Ok(42));
        assert_eq!(result.unwrap(), 42);

        // Should be in all levels due to write-through
        assert_eq!(cache.level_info(0).unwrap().current_size, 1);
        assert_eq!(cache.level_info(1).unwrap().current_size, 1);
        assert_eq!(cache.level_info(2).unwrap().current_size, 1);

        // Get from cache (should hit L1)
        let value = cache.get(&"key1".to_string()).unwrap();
        assert_eq!(value, 42);

        let stats = cache.stats();
        assert_eq!(stats.requests, 1);
        assert_eq!(stats.level_hits[0], 1);
        assert_eq!(stats.misses, 0);
    }

    #[test]
    fn test_promotion_policies() {
        let l1 = ComputeCacheLevel::with_capacity("L1", 0, 10);
        let l2 = ComputeCacheLevel::with_capacity("L2", 1, 100);

        let levels: Vec<Box<dyn CacheLevel<String, usize>>> = vec![
            Box::new(l1),
            Box::new(l2),
        ];

        let cache = MultiLevelCache::with_config(
            levels,
            MultiLevelConfig::new()
                .with_promotion_policy(PromotionPolicy::AfterNAccesses(2))
                .with_stats()
        );

        // Insert only in L2
        cache.levels[1].insert("key1".to_string(), 42).unwrap();

        // First access - should not promote
        let value = cache.get(&"key1".to_string()).unwrap();
        assert_eq!(value, 42);
        assert_eq!(cache.level_info(0).unwrap().current_size, 0);

        // Second access - should promote
        let value = cache.get(&"key1".to_string()).unwrap();
        assert_eq!(value, 42);
        assert_eq!(cache.level_info(0).unwrap().current_size, 1);
    }

    #[test]
    fn test_frequency_based_promotion() {
        let l1 = ComputeCacheLevel::with_capacity("L1", 0, 10);
        let l2 = ComputeCacheLevel::with_capacity("L2", 1, 100);

        let levels: Vec<Box<dyn CacheLevel<String, usize>>> = vec![
            Box::new(l1),
            Box::new(l2),
        ];

        let cache = MultiLevelCache::with_config(
            levels,
            MultiLevelConfig::new()
                .with_promotion_policy(PromotionPolicy::FrequencyBased(0.5))
                .with_stats()
        );

        // Insert only in L2
        cache.levels[1].insert("key1".to_string(), 42).unwrap();

        // Access the key multiple times to reach frequency threshold
        for _ in 0..10 {
            let _ = cache.get(&"key1".to_string());
        }

        // Should be promoted to L1 due to high frequency
        assert_eq!(cache.level_info(0).unwrap().current_size, 1);
    }

    #[test]
    fn test_batch_operations() {
        let cache = create_test_cache();

        let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
        let results = cache.get_or_compute_batch(keys, |k| Ok(k.len()));

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));

        // Test batch get
        let batch_keys = vec!["key1".to_string(), "key4".to_string()];
        let batch_results = cache.get_batch(batch_keys);

        assert_eq!(batch_results.len(), 2);
        assert!(batch_results[0].1.is_some()); // key1 exists
        assert!(batch_results[1].1.is_none());  // key4 doesn't exist
    }

    #[test]
    fn test_efficiency_report() {
        let cache = create_test_cache();

        // Add some data and access patterns
        for i in 0..10 {
            let key = format!("key{}", i);
            cache.get_or_compute(key.clone(), || Ok(i)).unwrap();

            // Access some keys multiple times
            if i < 5 {
                cache.get(&key);
                cache.get(&key);
            }
        }

        let report = cache.efficiency_report();
        assert!(report.overall_hit_rate > 0.0);
        assert!(report.level_infos.len() == 3);
        assert!(!report.recommendations.is_empty());
    }

    #[test]
    fn test_cache_clear_and_stats() {
        let cache = create_test_cache();

        // Add some data
        cache.get_or_compute("key1".to_string(), || Ok(42)).unwrap();
        cache.get_or_compute("key2".to_string(), || Ok(84)).unwrap();

        assert!(cache.level_info(0).unwrap().current_size > 0);

        // Clear and verify
        cache.clear();
        assert_eq!(cache.level_info(0).unwrap().current_size, 0);
        assert_eq!(cache.level_info(1).unwrap().current_size, 0);
        assert_eq!(cache.level_info(2).unwrap().current_size, 0);

        // Stats should be reset
        let stats = cache.stats();
        assert_eq!(stats.requests, 0);
        assert_eq!(stats.misses, 0);
    }
}