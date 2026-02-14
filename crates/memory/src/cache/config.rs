//! Configuration for cache implementations
//!
//! This module provides configuration options for various cache implementations
//! in the nebula-memory crate.

use std::time::Duration;

use crate::error::{MemoryError, MemoryResult};

/// Eviction policy for cache entries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EvictionPolicy {
    /// Least Recently Used - evict entries that haven't been accessed in the
    /// longest time
    #[default]
    LRU,
    /// Least Frequently Used - evict entries that are accessed least frequently
    LFU,
    /// First In First Out - evict oldest entries first
    FIFO,
    /// Random - evict random entries
    Random,
    /// Time To Live - evict entries that have expired
    TTL,
    /// Adaptive - dynamically choose policy based on access patterns
    Adaptive,
}

/// Configuration for cache implementations
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries in the cache
    pub max_entries: usize,
    /// Eviction policy to use when cache is full
    pub policy: EvictionPolicy,
    /// Time-to-live for cache entries (None means no expiration)
    pub ttl: Option<Duration>,
    /// Whether to track cache metrics
    pub track_metrics: bool,
    /// Initial capacity hint
    pub initial_capacity: Option<usize>,
    /// Load factor for hash-based caches (0.0-1.0)
    pub load_factor: f32,
    /// Enable automatic cleanup of expired entries
    pub auto_cleanup: bool,
    /// Cleanup interval for expired entries
    pub cleanup_interval: Option<Duration>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            policy: EvictionPolicy::default(),
            ttl: None,
            track_metrics: false,
            initial_capacity: None,
            load_factor: 0.75,
            auto_cleanup: false,
            cleanup_interval: None,
        }
    }
}

impl CacheConfig {
    /// Create a new cache configuration
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            ..Default::default()
        }
    }

    /// Set the eviction policy
    #[must_use = "builder methods must be chained or built"]
    pub fn with_policy(mut self, policy: EvictionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set the time-to-live for cache entries
    #[must_use = "builder methods must be chained or built"]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Enable metrics tracking
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metrics(mut self) -> Self {
        self.track_metrics = true;
        self
    }

    /// Set the initial capacity hint
    #[must_use = "builder methods must be chained or built"]
    pub fn with_initial_capacity(mut self, capacity: usize) -> Self {
        self.initial_capacity = Some(capacity);
        self
    }

    /// Set the load factor for hash-based caches
    #[must_use = "builder methods must be chained or built"]
    pub fn with_load_factor(mut self, load_factor: f32) -> Self {
        self.load_factor = load_factor.clamp(0.1, 0.95);
        self
    }

    /// Enable automatic cleanup of expired entries
    #[must_use = "builder methods must be chained or built"]
    pub fn with_auto_cleanup(mut self) -> Self {
        self.auto_cleanup = true;
        self
    }

    /// Set cleanup interval for expired entries
    #[must_use = "builder methods must be chained or built"]
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = Some(interval);
        self.auto_cleanup = true;
        self
    }

    /// Preset configuration for high-throughput scenarios
    #[must_use]
    pub fn for_high_throughput(max_entries: usize) -> Self {
        Self::new(max_entries)
            .with_policy(EvictionPolicy::LFU)
            .with_load_factor(0.85)
            .with_metrics()
            .with_initial_capacity(max_entries / 2)
    }

    /// Preset configuration for memory-constrained environments
    #[must_use]
    pub fn for_memory_constrained(max_entries: usize) -> Self {
        Self::new(max_entries)
            .with_policy(EvictionPolicy::LRU)
            .with_load_factor(0.65)
            .with_initial_capacity(max_entries / 4)
            .with_auto_cleanup()
    }

    /// Preset configuration for time-sensitive caching
    #[must_use]
    pub fn for_time_sensitive(max_entries: usize, ttl: Duration) -> Self {
        Self::new(max_entries)
            .with_policy(EvictionPolicy::TTL)
            .with_ttl(ttl)
            .with_metrics()
            .with_auto_cleanup()
            .with_cleanup_interval(Duration::from_secs(ttl.as_secs() / 4))
    }

    /// Validate the configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if self.max_entries == 0 {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        if !(0.1..=0.95).contains(&self.load_factor) {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        if let Some(ttl) = self.ttl
            && ttl.as_nanos() == 0
        {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        if let Some(initial) = self.initial_capacity
            && initial > self.max_entries
        {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        if let Some(cleanup_interval) = self.cleanup_interval {
            if cleanup_interval.as_nanos() == 0 {
                return Err(MemoryError::invalid_config("configuration error"));
            }

            if let Some(ttl) = self.ttl
                && cleanup_interval >= ttl
            {
                return Err(MemoryError::invalid_config("configuration error"));
            }
        }

        // Validate policy-specific requirements
        if self.policy == EvictionPolicy::TTL && self.ttl.is_none() {
            return Err(MemoryError::invalid_config("configuration error"));
        }

        Ok(())
    }

    /// Get the recommended initial capacity
    #[must_use]
    pub fn effective_initial_capacity(&self) -> usize {
        self.initial_capacity.unwrap_or({
            // Calculate based on load factor and max entries
            ((self.max_entries as f32) * self.load_factor) as usize
        })
    }

    /// Check if the configuration is optimized for the given scenario
    #[must_use]
    pub fn is_optimized_for_scenario(&self, scenario: CacheScenario) -> bool {
        match scenario {
            CacheScenario::HighThroughput => {
                matches!(self.policy, EvictionPolicy::LFU | EvictionPolicy::Adaptive)
                    && self.load_factor >= 0.8
                    && self.track_metrics
            }
            CacheScenario::MemoryConstrained => {
                matches!(self.policy, EvictionPolicy::LRU | EvictionPolicy::FIFO)
                    && self.load_factor <= 0.7
                    && self.auto_cleanup
            }
            CacheScenario::TimeSensitive => {
                self.ttl.is_some()
                    && matches!(self.policy, EvictionPolicy::TTL)
                    && self.auto_cleanup
            }
            CacheScenario::Embedded => {
                !self.track_metrics
                    && self.load_factor <= 0.6
                    && self
                        .initial_capacity
                        .is_some_and(|cap| cap <= self.max_entries / 4)
            }
        }
    }
}

/// Cache usage scenarios for optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScenario {
    /// High request rate, favor hit rate over memory usage
    HighThroughput,
    /// Limited memory, favor memory efficiency
    MemoryConstrained,
    /// Time-based expiration is critical
    TimeSensitive,
    /// Embedded/no-std environment
    Embedded,
}

/// Cache metrics for monitoring and optimization
#[derive(Debug, Clone, Default)]
pub struct CacheMetrics {
    /// Number of cache hits
    pub hits: usize,
    /// Number of cache misses
    pub misses: usize,
    /// Number of cache evictions
    pub evictions: usize,
    /// Number of cache insertions
    pub insertions: usize,
    /// Number of cache updates
    pub updates: usize,
    /// Total compute time for cache misses (nanoseconds)
    pub compute_time_ns: u64,
    /// Peak number of entries in cache
    pub peak_size: usize,
    /// Number of expired entries cleaned up
    pub expired_cleanups: usize,
}

impl CacheMetrics {
    /// Create a new empty metrics object
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate the hit rate (0.0-1.0)
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Calculate the miss rate (0.0-1.0)
    #[must_use]
    pub fn miss_rate(&self) -> f64 {
        1.0 - self.hit_rate()
    }

    /// Calculate average compute time per miss (nanoseconds)
    #[must_use]
    pub fn avg_compute_time_ns(&self) -> u64 {
        if self.misses > 0 {
            self.compute_time_ns / self.misses as u64
        } else {
            0
        }
    }

    /// Calculate cache efficiency score (0.0-100.0)
    /// Higher score means better performance
    #[must_use]
    pub fn efficiency_score(&self) -> f64 {
        let hit_rate = self.hit_rate();
        let compute_penalty = if self.misses > 0 {
            // Convert to milliseconds and normalize
            (self.avg_compute_time_ns() as f64) / 1_000_000.0
        } else {
            0.0
        };

        // Score based on hit rate minus compute penalty
        (hit_rate * 100.0 - (compute_penalty / 10.0).min(50.0)).max(0.0)
    }

    /// Get total number of cache operations
    #[must_use]
    pub fn total_operations(&self) -> usize {
        self.hits + self.misses
    }

    /// Calculate eviction rate (evictions per operation)
    #[must_use]
    pub fn eviction_rate(&self) -> f64 {
        let total_ops = self.total_operations();
        if total_ops > 0 {
            self.evictions as f64 / total_ops as f64
        } else {
            0.0
        }
    }

    /// Reset all metrics to zero
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Merge with another metrics object
    pub fn merge(&mut self, other: &Self) {
        self.hits += other.hits;
        self.misses += other.misses;
        self.evictions += other.evictions;
        self.insertions += other.insertions;
        self.updates += other.updates;
        self.compute_time_ns += other.compute_time_ns;
        self.peak_size = self.peak_size.max(other.peak_size);
        self.expired_cleanups += other.expired_cleanups;
    }

    /// Update peak size if current size is larger
    pub fn update_peak_size(&mut self, current_size: usize) {
        self.peak_size = self.peak_size.max(current_size);
    }

    /// Generate a performance report
    #[must_use]
    pub fn performance_report(&self) -> CachePerformanceReport {
        CachePerformanceReport {
            hit_rate: self.hit_rate(),
            miss_rate: self.miss_rate(),
            efficiency_score: self.efficiency_score(),
            avg_compute_time_ms: self.avg_compute_time_ns() as f64 / 1_000_000.0,
            total_operations: self.total_operations(),
            eviction_rate: self.eviction_rate(),
            peak_memory_usage: self.peak_size,
        }
    }
}

/// Performance report for cache analysis
#[derive(Debug, Clone)]
pub struct CachePerformanceReport {
    /// Hit rate percentage (0.0-1.0)
    pub hit_rate: f64,
    /// Miss rate percentage (0.0-1.0)
    pub miss_rate: f64,
    /// Overall efficiency score (0.0-100.0)
    pub efficiency_score: f64,
    /// Average computation time per miss in milliseconds
    pub avg_compute_time_ms: f64,
    /// Total number of cache operations
    pub total_operations: usize,
    /// Rate of evictions per operation
    pub eviction_rate: f64,
    /// Peak number of entries in cache
    pub peak_memory_usage: usize,
}

impl CachePerformanceReport {
    /// Check if the cache performance is considered good
    pub fn is_performing_well(&self) -> bool {
        self.hit_rate >= 0.8 && self.efficiency_score >= 70.0 && self.eviction_rate <= 0.1
    }

    /// Get recommendations for improving cache performance
    pub fn recommendations(&self) -> Vec<&'static str> {
        let mut recommendations = Vec::new();

        if self.hit_rate < 0.6 {
            recommendations.push("Consider increasing cache size or adjusting eviction policy");
        }

        if self.eviction_rate > 0.2 {
            recommendations.push("High eviction rate - consider increasing max_entries");
        }

        if self.avg_compute_time_ms > 100.0 {
            recommendations.push("High compute time - optimize computation functions");
        }

        if self.efficiency_score < 50.0 {
            recommendations.push("Poor efficiency - review cache configuration and usage patterns");
        }

        if recommendations.is_empty() {
            recommendations.push("Cache is performing well");
        }

        recommendations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CacheConfig::default();
        assert_eq!(config.max_entries, 1000);
        assert_eq!(config.policy, EvictionPolicy::LRU);
        assert!(config.ttl.is_none());
        assert!(!config.track_metrics);
        assert!(!config.auto_cleanup);
    }

    #[test]
    fn test_builder_pattern() {
        let config = CacheConfig::new(500)
            .with_policy(EvictionPolicy::LFU)
            .with_ttl(Duration::from_secs(60))
            .with_metrics()
            .with_initial_capacity(100)
            .with_load_factor(0.8)
            .with_auto_cleanup();

        assert_eq!(config.max_entries, 500);
        assert_eq!(config.policy, EvictionPolicy::LFU);
        assert_eq!(config.ttl, Some(Duration::from_secs(60)));
        assert!(config.track_metrics);
        assert_eq!(config.initial_capacity, Some(100));
        assert_eq!(config.load_factor, 0.8);
        assert!(config.auto_cleanup);
    }

    #[test]
    fn test_preset_configurations() {
        let high_throughput = CacheConfig::for_high_throughput(1000);
        assert_eq!(high_throughput.policy, EvictionPolicy::LFU);
        assert!(high_throughput.track_metrics);

        let memory_constrained = CacheConfig::for_memory_constrained(500);
        assert_eq!(memory_constrained.policy, EvictionPolicy::LRU);
        assert!(memory_constrained.auto_cleanup);
        assert_eq!(memory_constrained.load_factor, 0.65);
    }

    #[test]
    fn test_config_validation() {
        // Valid config
        let config = CacheConfig::new(100);
        assert!(config.validate().is_ok());

        // Invalid max_entries
        let config = CacheConfig::new(0);
        assert!(config.validate().is_err());

        // Invalid load_factor - set directly since with_load_factor clamps
        let mut config = CacheConfig::new(100);
        config.load_factor = 1.5;
        assert!(config.validate().is_err());

        // Invalid initial_capacity
        let config = CacheConfig::new(100).with_initial_capacity(200);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_metrics() {
        let mut metrics = CacheMetrics::default();
        metrics.hits = 80;
        metrics.misses = 20;
        metrics.compute_time_ns = 1_000_000_000; // 1 second total

        assert!((metrics.hit_rate() - 0.8).abs() < f64::EPSILON);
        assert!((metrics.miss_rate() - 0.2).abs() < f64::EPSILON);
        assert_eq!(metrics.avg_compute_time_ns(), 50_000_000); // 50ms per miss
        assert_eq!(metrics.total_operations(), 100);

        let score = metrics.efficiency_score();
        assert!(score > 70.0); // Should be a good score

        let mut other = CacheMetrics::default();
        other.hits = 40;
        other.misses = 60;
        other.evictions = 10;
        other.peak_size = 150;

        metrics.merge(&other);
        assert_eq!(metrics.hits, 120);
        assert_eq!(metrics.misses, 80);
        assert_eq!(metrics.evictions, 10);
        assert_eq!(metrics.peak_size, 150);

        metrics.reset();
        assert_eq!(metrics.hits, 0);
        assert_eq!(metrics.misses, 0);
        assert_eq!(metrics.evictions, 0);
    }

    #[test]
    fn test_performance_report() {
        let mut metrics = CacheMetrics::default();
        metrics.hits = 900;
        metrics.misses = 100;
        metrics.evictions = 5;
        metrics.compute_time_ns = 500_000_000; // 500ms total

        let report = metrics.performance_report();
        assert!((report.hit_rate - 0.9).abs() < f64::EPSILON);
        assert!((report.miss_rate - 0.1).abs() < f64::EPSILON);
        assert!(report.is_performing_well());

        let recommendations = report.recommendations();
        assert!(recommendations.contains(&"Cache is performing well"));
    }

    #[test]
    fn test_scenario_optimization() {
        let high_throughput = CacheConfig::for_high_throughput(1000);
        assert!(high_throughput.is_optimized_for_scenario(CacheScenario::HighThroughput));

        let memory_constrained = CacheConfig::for_memory_constrained(500);
        assert!(memory_constrained.is_optimized_for_scenario(CacheScenario::MemoryConstrained));
    }

    #[test]
    fn test_time_sensitive_config() {
        let ttl = Duration::from_secs(300);
        let config = CacheConfig::for_time_sensitive(1000, ttl);

        assert_eq!(config.policy, EvictionPolicy::TTL);
        assert_eq!(config.ttl, Some(ttl));
        assert!(config.auto_cleanup);
        assert!(config.cleanup_interval.is_some());
        assert!(config.is_optimized_for_scenario(CacheScenario::TimeSensitive));
    }
}
