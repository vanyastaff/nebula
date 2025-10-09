//! Partitioned cache implementation
//!
//! This module provides a partitioned cache that divides the cache into
//! multiple segments to reduce lock contention and improve concurrency.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
    thread,
    time::Instant,
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, string::String, sync::Arc, vec::Vec},
    core::hash::{Hash, Hasher},
    hashbrown::HashMap,
    spin::RwLock,
};

use super::compute::{CacheKey, CacheResult, ComputeCache};
use super::config::{CacheConfig, CacheMetrics};
use crate::error::{MemoryError, MemoryResult};

/// Hash function strategy for partitioning
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HashStrategy {
    /// Use default hasher
    Default,
    /// Use FNV hash (faster for small keys)
    Fnv,
    /// Use consistent hashing (better distribution)
    Consistent,
    /// Use modulo of key hash
    Modulo,
}

impl Default for HashStrategy {
    fn default() -> Self {
        HashStrategy::Default
    }
}

/// Configuration for partitioned cache
#[derive(Debug, Clone)]
pub struct PartitionedConfig {
    /// Base cache configuration
    pub cache_config: CacheConfig,
    /// Number of partitions
    pub partition_count: usize,
    /// Hash strategy for partitioning
    pub hash_strategy: HashStrategy,
    /// Enable partition-level metrics
    pub partition_metrics: bool,
    /// Enable automatic rebalancing
    pub auto_rebalance: bool,
    /// Rebalancing threshold (load factor difference)
    pub rebalance_threshold: f32,
    /// Concurrency level (affects locking strategy)
    pub concurrency_level: usize,
}

impl Default for PartitionedConfig {
    fn default() -> Self {
        Self {
            cache_config: CacheConfig::default(),
            partition_count: num_cpus(),
            hash_strategy: HashStrategy::default(),
            partition_metrics: false,
            auto_rebalance: false,
            rebalance_threshold: 0.3,
            concurrency_level: num_cpus(),
        }
    }
}

impl PartitionedConfig {
    /// Create new partitioned config
    pub fn new(max_entries: usize, partition_count: usize) -> Self {
        Self {
            cache_config: CacheConfig::new(max_entries),
            partition_count: partition_count.max(1),
            ..Default::default()
        }
    }

    /// Set hash strategy
    pub fn with_hash_strategy(mut self, strategy: HashStrategy) -> Self {
        self.hash_strategy = strategy;
        self
    }

    /// Enable partition metrics
    pub fn with_partition_metrics(mut self) -> Self {
        self.partition_metrics = true;
        self
    }

    /// Enable auto rebalancing
    pub fn with_auto_rebalance(mut self, threshold: f32) -> Self {
        self.auto_rebalance = true;
        self.rebalance_threshold = threshold;
        self
    }

    /// Set concurrency level
    pub fn with_concurrency_level(mut self, level: usize) -> Self {
        self.concurrency_level = level.max(1);
        self
    }

    /// Configure for high concurrency
    pub fn for_high_concurrency(max_entries: usize) -> Self {
        let partition_count = (num_cpus() * 2).max(8);
        Self::new(max_entries, partition_count)
            .with_hash_strategy(HashStrategy::Consistent)
            .with_partition_metrics()
            .with_concurrency_level(partition_count)
    }

    /// Configure for memory efficiency
    pub fn for_memory_efficiency(max_entries: usize) -> Self {
        let partition_count = num_cpus().max(2).min(4);
        Self::new(max_entries, partition_count).with_hash_strategy(HashStrategy::Modulo)
    }

    /// Configure for balanced performance
    pub fn for_balanced_performance(max_entries: usize) -> Self {
        let partition_count = num_cpus();
        Self::new(max_entries, partition_count)
            .with_hash_strategy(HashStrategy::Default)
            .with_partition_metrics()
            .with_auto_rebalance(0.2)
    }

    /// Validate configuration
    pub fn validate(&self) -> MemoryResult<()> {
        self.cache_config.validate()?;

        if self.partition_count == 0 {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        if !(0.1..=1.0).contains(&self.rebalance_threshold) {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        Ok(())
    }
}

/// Get number of CPUs, with fallback for no-std
fn num_cpus() -> usize {
    #[cfg(feature = "std")]
    {
        thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
    }
    #[cfg(not(feature = "std"))]
    {
        4 // Default fallback for no-std
    }
}

/// Partition information for monitoring
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub index: usize,
    pub size: usize,
    pub capacity: usize,
    pub load_factor: f32,
    pub hit_rate: f64,
    #[cfg(feature = "std")]
    pub metrics: Option<CacheMetrics>,
}

/// Statistics for the partitioned cache
#[derive(Debug, Clone, Default)]
pub struct PartitionedStats {
    /// Total number of requests
    pub total_requests: usize,
    /// Total hits across all partitions
    pub total_hits: usize,
    /// Total misses across all partitions
    pub total_misses: usize,
    /// Lock contention count
    pub lock_contentions: usize,
    /// Rebalancing operations
    pub rebalance_operations: usize,
    /// Per-partition statistics
    pub partition_stats: Vec<PartitionInfo>,
}

impl PartitionedStats {
    /// Calculate overall hit rate
    pub fn hit_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_hits as f64 / self.total_requests as f64
        }
    }

    /// Calculate load balance score (0.0 = perfect, 1.0 = completely unbalanced)
    pub fn load_balance_score(&self) -> f64 {
        if self.partition_stats.is_empty() {
            return 0.0;
        }

        let load_factors: Vec<f32> = self.partition_stats.iter().map(|p| p.load_factor).collect();

        let avg_load = load_factors.iter().sum::<f32>() / load_factors.len() as f32;
        let variance: f32 = load_factors
            .iter()
            .map(|&load| (load - avg_load).powi(2))
            .sum::<f32>()
            / load_factors.len() as f32;

        variance.sqrt() as f64
    }

    /// Get most loaded partition
    pub fn most_loaded_partition(&self) -> Option<usize> {
        self.partition_stats
            .iter()
            .max_by(|a, b| a.load_factor.partial_cmp(&b.load_factor).unwrap())
            .map(|p| p.index)
    }

    /// Get least loaded partition
    pub fn least_loaded_partition(&self) -> Option<usize> {
        self.partition_stats
            .iter()
            .min_by(|a, b| a.load_factor.partial_cmp(&b.load_factor).unwrap())
            .map(|p| p.index)
    }
}

/// A partitioned cache that divides the cache into multiple segments
pub struct PartitionedCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// The cache partitions using ComputeCache
    partitions: Vec<Arc<RwLock<ComputeCache<K, V>>>>,
    /// Configuration
    config: PartitionedConfig,
    /// Global statistics
    #[cfg(feature = "std")]
    stats: Arc<RwLock<PartitionedStats>>,
    /// Hash ring for consistent hashing
    hash_ring: Vec<u64>,
}

impl<K, V> PartitionedCache<K, V>
where
    K: CacheKey,
    V: Clone,
{
    /// Create a new partitioned cache
    pub fn new(max_entries: usize, partition_count: usize) -> Self {
        Self::with_config(PartitionedConfig::new(max_entries, partition_count))
    }

    /// Create a new partitioned cache with configuration
    pub fn with_config(config: PartitionedConfig) -> Self {
        config
            .validate()
            .expect("Invalid partitioned cache configuration");

        let partition_count = config.partition_count;
        let entries_per_partition =
            (config.cache_config.max_entries + partition_count - 1) / partition_count;

        // Create partition configurations
        let partition_config = CacheConfig {
            max_entries: entries_per_partition,
            ..config.cache_config.clone()
        };

        // Create partitions
        let mut partitions = Vec::with_capacity(partition_count);
        for _ in 0..partition_count {
            let cache = ComputeCache::with_config(partition_config.clone());
            partitions.push(Arc::new(RwLock::new(cache)));
        }

        // Create hash ring for consistent hashing
        let hash_ring = Self::create_hash_ring(partition_count);

        #[cfg(feature = "std")]
        let stats = Arc::new(RwLock::new(PartitionedStats::default()));

        Self {
            partitions,
            config,
            #[cfg(feature = "std")]
            stats,
            hash_ring,
        }
    }

    /// Create hash ring for consistent hashing
    fn create_hash_ring(partition_count: usize) -> Vec<u64> {
        let mut ring = Vec::with_capacity(partition_count * 100); // 100 virtual nodes per partition

        for partition in 0..partition_count {
            for virtual_node in 0..100 {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                hasher.write_usize(partition);
                hasher.write_usize(virtual_node);
                ring.push(hasher.finish());
            }
        }

        ring.sort_unstable();
        ring
    }

    /// Get the partition index for a key
    fn get_partition_index(&self, key: &K) -> usize {
        match self.config.hash_strategy {
            HashStrategy::Default => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                key.hash(&mut hasher);
                (hasher.finish() as usize) % self.config.partition_count
            }
            HashStrategy::Fnv => {
                // Simple FNV-like hash
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                key.hash(&mut hasher);
                let hash = hasher.finish();
                // Simple FNV-like transformation
                let fnv_hash = hash.wrapping_mul(1099511628211u64);
                (fnv_hash as usize) % self.config.partition_count
            }
            HashStrategy::Consistent => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                key.hash(&mut hasher);
                let key_hash = hasher.finish();

                // Find position in hash ring
                match self.hash_ring.binary_search(&key_hash) {
                    Ok(pos) | Err(pos) => {
                        let ring_pos = if pos >= self.hash_ring.len() { 0 } else { pos };
                        let virtual_node = self.hash_ring[ring_pos];

                        // Extract partition from virtual node
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        hasher.write_u64(virtual_node);
                        (hasher.finish() as usize) % self.config.partition_count
                    }
                }
            }
            HashStrategy::Modulo => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                key.hash(&mut hasher);
                (hasher.finish() as usize) % self.config.partition_count
            }
        }
    }

    /// Get a value from the cache, computing it if not present
    pub fn get_or_compute<F>(&self, key: K, compute_fn: F) -> CacheResult<V>
    where
        F: FnOnce() -> Result<V, MemoryError>,
    {
        let partition_idx = self.get_partition_index(&key);
        let partition = &self.partitions[partition_idx];

        #[cfg(feature = "std")]
        let start_time = Instant::now();

        // Try read lock first for cache hit
        {
            let mut cache = partition.write().unwrap();
            if let Some(value) = cache.get(&key) {
                #[cfg(feature = "std")]
                if self.config.partition_metrics {
                    let mut stats = self.stats.write().unwrap();
                    stats.total_requests += 1;
                    stats.total_hits += 1;
                }

                return Ok(value);
            }
        }

        // Cache miss - get write lock and compute
        let mut cache = partition.write().unwrap();

        // Double-check pattern
        if let Some(value) = cache.get(&key) {
            #[cfg(feature = "std")]
            if self.config.partition_metrics {
                let mut stats = self.stats.write().unwrap();
                stats.total_requests += 1;
                stats.total_hits += 1;
            }

            return Ok(value);
        }

        // Actually compute the value
        let result = cache.get_or_compute(key, compute_fn);

        #[cfg(feature = "std")]
        if self.config.partition_metrics {
            let mut stats = self.stats.write().unwrap();
            stats.total_requests += 1;
            if result.is_ok() {
                stats.total_misses += 1;
            }
        }

        result
    }

    /// Get a value from cache without computing
    pub fn get(&self, key: &K) -> Option<V> {
        let partition_idx = self.get_partition_index(key);
        let partition = &self.partitions[partition_idx];

        let mut cache = partition.write().unwrap();
        let result = cache.get(key);

        #[cfg(feature = "std")]
        if self.config.partition_metrics {
            let mut stats = self.stats.write().unwrap();
            stats.total_requests += 1;
            if result.is_some() {
                stats.total_hits += 1;
            } else {
                stats.total_misses += 1;
            }
        }

        result
    }

    /// Insert a value directly
    pub fn insert(&self, key: K, value: V) -> CacheResult<()> {
        let partition_idx = self.get_partition_index(&key);
        let partition = &self.partitions[partition_idx];

        let mut cache = partition.write().unwrap();
        cache.insert(key, value)
    }

    /// Remove a value
    pub fn remove(&self, key: &K) -> Option<V> {
        let partition_idx = self.get_partition_index(key);
        let partition = &self.partitions[partition_idx];

        let mut cache = partition.write().unwrap();
        cache.remove(key)
    }

    /// Check if key exists
    pub fn contains_key(&self, key: &K) -> bool {
        let partition_idx = self.get_partition_index(key);
        let partition = &self.partitions[partition_idx];

        let cache = partition.read().unwrap();
        cache.contains_key(key)
    }

    /// Get batch of values
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

    /// Get or compute batch of values
    pub fn get_or_compute_batch<I, F>(&self, keys: I, compute_fn: F) -> Vec<CacheResult<V>>
    where
        I: IntoIterator<Item = K>,
        F: Fn(&K) -> Result<V, MemoryError>,
        K: Clone,
    {
        keys.into_iter()
            .map(|key| self.get_or_compute(key.clone(), || compute_fn(&key)))
            .collect()
    }

    /// Get the current number of entries in the cache
    pub fn len(&self) -> usize {
        self.partitions
            .iter()
            .map(|partition| {
                let cache = partition.read().unwrap();
                cache.len()
            })
            .sum()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.partitions.iter().all(|partition| {
            let cache = partition.read().unwrap();
            cache.is_empty()
        })
    }

    /// Clear all entries from the cache
    pub fn clear(&self) {
        for partition in &self.partitions {
            let mut cache = partition.write().unwrap();
            cache.clear();
        }

        #[cfg(feature = "std")]
        if self.config.partition_metrics {
            let mut stats = self.stats.write().unwrap();
            *stats = PartitionedStats::default();
        }
    }

    /// Get the number of partitions
    pub fn partition_count(&self) -> usize {
        self.config.partition_count
    }

    /// Get information about a specific partition
    pub fn partition_info(&self, partition_idx: usize) -> Option<PartitionInfo> {
        if partition_idx >= self.partitions.len() {
            return None;
        }

        let cache = self.partitions[partition_idx].read().unwrap();

        #[cfg(feature = "std")]
        let metrics = if self.config.partition_metrics {
            Some(cache.metrics())
        } else {
            None
        };

        Some(PartitionInfo {
            index: partition_idx,
            size: cache.len(),
            capacity: cache.capacity(),
            load_factor: cache.load_factor(),
            hit_rate: {
                #[cfg(feature = "std")]
                {
                    metrics.as_ref().map_or(0.0, |m| m.hit_rate())
                }
                #[cfg(not(feature = "std"))]
                {
                    0.0
                }
            },
            #[cfg(feature = "std")]
            metrics,
        })
    }

    /// Get information about all partitions
    pub fn all_partitions_info(&self) -> Vec<PartitionInfo> {
        (0..self.partition_count())
            .filter_map(|i| self.partition_info(i))
            .collect()
    }

    /// Get overall cache statistics
    #[cfg(feature = "std")]
    pub fn stats(&self) -> PartitionedStats {
        if !self.config.partition_metrics {
            return PartitionedStats::default();
        }

        let mut stats = self.stats.read().unwrap().clone();
        stats.partition_stats = self.all_partitions_info();
        stats
    }

    /// Reset all statistics
    #[cfg(feature = "std")]
    pub fn reset_stats(&self) {
        if self.config.partition_metrics {
            let mut stats = self.stats.write().unwrap();
            *stats = PartitionedStats::default();
        }

        for partition in &self.partitions {
            let cache = partition.read().unwrap();
            cache.reset_metrics();
        }
    }

    /// Get combined cache metrics from all partitions
    #[cfg(feature = "std")]
    pub fn metrics(&self) -> CacheMetrics {
        let mut combined = CacheMetrics::default();

        for partition in &self.partitions {
            let cache = partition.read().unwrap();
            let metrics = cache.metrics();
            combined.merge(&metrics);
        }

        combined
    }

    /// Clean up expired entries in all partitions
    #[cfg(feature = "std")]
    pub fn cleanup_expired(&self) -> usize {
        self.partitions
            .iter()
            .map(|partition| {
                let mut cache = partition.write().unwrap();
                cache.cleanup_expired()
            })
            .sum()
    }

    /// Check if rebalancing is needed
    pub fn needs_rebalancing(&self) -> bool {
        if !self.config.auto_rebalance {
            return false;
        }

        let partition_infos = self.all_partitions_info();
        if partition_infos.len() < 2 {
            return false;
        }

        let load_factors: Vec<f32> = partition_infos.iter().map(|p| p.load_factor).collect();

        let max_load = load_factors.iter().cloned().fold(0.0f32, f32::max);
        let min_load = load_factors.iter().cloned().fold(1.0f32, f32::min);

        (max_load - min_load) > self.config.rebalance_threshold
    }

    /// Perform rebalancing (simplified version)
    #[cfg(feature = "std")]
    pub fn rebalance(&self) -> MemoryResult<usize> {
        if !self.needs_rebalancing() {
            return Ok(0);
        }

        // This is a simplified rebalancing implementation
        // In production, you'd want more sophisticated logic

        let partition_infos = self.all_partitions_info();
        let most_loaded = partition_infos
            .iter()
            .max_by(|a, b| a.load_factor.partial_cmp(&b.load_factor).unwrap());

        let least_loaded = partition_infos
            .iter()
            .min_by(|a, b| a.load_factor.partial_cmp(&b.load_factor).unwrap());

        if let (Some(max_partition), Some(min_partition)) = (most_loaded, least_loaded) {
            // In a real implementation, you would migrate some keys
            // from max_partition to min_partition

            let mut stats = self.stats.write().unwrap();
            stats.rebalance_operations += 1;

            return Ok(1);
        }

        Ok(0)
    }

    /// Warm up cache with data
    pub fn warm_up(&self, entries: &[(K, V)]) -> MemoryResult<()>
    where
        K: Clone,
        V: Clone,
    {
        // Distribute entries across partitions
        let mut partition_entries: Vec<Vec<(K, V)>> = vec![Vec::new(); self.partition_count()];

        for (key, value) in entries {
            let partition_idx = self.get_partition_index(key);
            partition_entries[partition_idx].push((key.clone(), value.clone()));
        }

        // Warm up each partition
        for (partition_idx, entries) in partition_entries.into_iter().enumerate() {
            if !entries.is_empty() {
                let mut cache = self.partitions[partition_idx].write().unwrap();
                for (key, value) in entries {
                    cache.insert(key, value)?;
                }
            }
        }

        Ok(())
    }

    /// Get cache efficiency report
    #[cfg(feature = "std")]
    pub fn efficiency_report(&self) -> PartitionedEfficiencyReport {
        let stats = self.stats();
        let partition_infos = self.all_partitions_info();

        PartitionedEfficiencyReport {
            overall_hit_rate: stats.hit_rate(),
            load_balance_score: stats.load_balance_score(),
            partition_count: self.partition_count(),
            total_capacity: partition_infos.iter().map(|p| p.capacity).sum(),
            total_size: partition_infos.iter().map(|p| p.size).sum(),
            most_loaded_partition: stats.most_loaded_partition(),
            least_loaded_partition: stats.least_loaded_partition(),
            needs_rebalancing: self.needs_rebalancing(),
            hash_strategy: self.config.hash_strategy,
            recommendations: self.generate_recommendations(&stats),
        }
    }

    /// Generate optimization recommendations
    #[cfg(feature = "std")]
    fn generate_recommendations(&self, stats: &PartitionedStats) -> Vec<String> {
        let mut recommendations = Vec::new();

        if stats.hit_rate() < 0.7 {
            recommendations
                .push("Overall hit rate is low - consider increasing cache size".to_string());
        }

        if stats.load_balance_score() > 0.3 {
            recommendations.push(
                "Poor load balancing - consider different hash strategy or rebalancing".to_string(),
            );
        }

        if stats.lock_contentions > stats.total_requests / 10 {
            recommendations
                .push("High lock contention - consider increasing partition count".to_string());
        }

        if self.partition_count() < num_cpus() {
            recommendations.push(
                "Partition count is less than CPU count - consider increasing partitions"
                    .to_string(),
            );
        }

        if self.partition_count() > num_cpus() * 4 {
            recommendations.push("Too many partitions - may cause overhead".to_string());
        }

        if recommendations.is_empty() {
            recommendations.push("Partitioned cache is performing well".to_string());
        }

        recommendations
    }
}

/// Efficiency report for partitioned cache
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub struct PartitionedEfficiencyReport {
    pub overall_hit_rate: f64,
    pub load_balance_score: f64,
    pub partition_count: usize,
    pub total_capacity: usize,
    pub total_size: usize,
    pub most_loaded_partition: Option<usize>,
    pub least_loaded_partition: Option<usize>,
    pub needs_rebalancing: bool,
    pub hash_strategy: HashStrategy,
    pub recommendations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::config::EvictionPolicy;

    #[test]
    fn test_basic_partitioned_caching() {
        let cache = PartitionedCache::<String, usize>::new(100, 4);

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
    fn test_partition_configuration() {
        let config = PartitionedConfig::for_high_concurrency(1000);
        assert!(config.partition_count >= 8);
        assert_eq!(config.hash_strategy, HashStrategy::Consistent);
        assert!(config.partition_metrics);

        let config = PartitionedConfig::for_memory_efficiency(1000);
        assert!(config.partition_count <= 4);
        assert_eq!(config.hash_strategy, HashStrategy::Modulo);
    }

    #[test]
    fn test_direct_operations() {
        let cache = PartitionedCache::<String, usize>::new(100, 4);

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
    fn test_batch_operations() {
        let cache = PartitionedCache::<String, usize>::new(100, 4);

        let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
        let results = cache.get_or_compute_batch(keys, |k| Ok(k.len()));

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));

        // Test batch get
        let batch_keys = vec!["key1".to_string(), "key4".to_string()];
        let batch_results = cache.get_batch(batch_keys);

        assert_eq!(batch_results.len(), 2);
        assert!(batch_results[0].1.is_some()); // key1 exists
        assert!(batch_results[1].1.is_none()); // key4 doesn't exist
    }

    #[test]
    fn test_partition_distribution() {
        let cache = PartitionedCache::<String, usize>::new(100, 4);

        // Add entries
        for i in 0..100 {
            let key = format!("key{}", i);
            let _ = cache.get_or_compute(key, || Ok(i));
        }

        // Check distribution
        let partition_infos = cache.all_partitions_info();
        assert_eq!(partition_infos.len(), 4);

        // No partition should be empty (with 100 keys, very unlikely)
        for info in partition_infos {
            assert!(
                info.size > 0,
                "Partition {} should have entries",
                info.index
            );
        }
    }

    #[test]
    fn test_warm_up() {
        let cache = PartitionedCache::<String, usize>::new(100, 4);

        let entries = vec![
            ("warm1".to_string(), 100),
            ("warm2".to_string(), 200),
            ("warm3".to_string(), 300),
        ];

        cache.warm_up(&entries).unwrap();

        assert_eq!(cache.get(&"warm1".to_string()), Some(100));
        assert_eq!(cache.get(&"warm2".to_string()), Some(200));
        assert_eq!(cache.get(&"warm3".to_string()), Some(300));
    }

    #[test]
    fn test_hash_strategies() {
        let strategies = [
            HashStrategy::Default,
            HashStrategy::Fnv,
            HashStrategy::Consistent,
            HashStrategy::Modulo,
        ];

        for strategy in strategies {
            let config = PartitionedConfig::new(100, 4).with_hash_strategy(strategy);
            let cache = PartitionedCache::<String, usize>::with_config(config);

            // Test that all strategies work
            cache.insert("test".to_string(), 42).unwrap();
            assert_eq!(cache.get(&"test".to_string()), Some(42));
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_statistics() {
        let config = PartitionedConfig::new(100, 4).with_partition_metrics();
        let cache = PartitionedCache::<String, usize>::with_config(config);

        // Generate some activity
        for i in 0..10 {
            let key = format!("key{}", i);
            cache.get_or_compute(key.clone(), || Ok(i)).unwrap();
            cache.get(&key); // Hit
        }

        let stats = cache.stats();
        assert!(stats.total_requests > 0);
        assert!(stats.total_hits > 0);
        assert!(stats.hit_rate() > 0.0);

        let report = cache.efficiency_report();
        assert!(!report.recommendations.is_empty());
    }

    #[test]
    fn test_error_handling() {
        let cache = PartitionedCache::<String, usize>::new(10, 2);

        // Error should be propagated
        let result =
            cache.get_or_compute(
                "error".to_string(),
                || Err(MemoryError::allocation_failed(0, 1)),
            );

        assert!(result.is_err());

        // After error, key should not be cached
        let result = cache.get_or_compute("error".to_string(), || Ok(42));
        assert_eq!(result.unwrap(), 42);
    }
}
