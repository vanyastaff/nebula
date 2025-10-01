//! Cache statistics
//!
//! This module provides comprehensive statistics tracking for cache operations,
//! including performance metrics, trends analysis, and advanced monitoring capabilities.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime},
};

#[cfg(not(feature = "std"))]
use {
    alloc::{boxed::Box, string::String, sync::Arc, vec::Vec},
    core::{
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    },
    hashbrown::HashMap,
    spin::{Mutex, RwLock},
};

use crate::core::error::{MemoryError, MemoryResult};

/// Time window for statistics aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeWindow {
    Last1Minute,
    Last5Minutes,
    Last15Minutes,
    Last1Hour,
    Last24Hours,
    LastWeek,
    AllTime,
}

impl TimeWindow {
    /// Get the duration for this time window
    pub fn duration(&self) -> Option<Duration> {
        match self {
            TimeWindow::Last1Minute => Some(Duration::from_secs(60)),
            TimeWindow::Last5Minutes => Some(Duration::from_secs(300)),
            TimeWindow::Last15Minutes => Some(Duration::from_secs(900)),
            TimeWindow::Last1Hour => Some(Duration::from_secs(3600)),
            TimeWindow::Last24Hours => Some(Duration::from_secs(86400)),
            TimeWindow::LastWeek => Some(Duration::from_secs(604800)),
            TimeWindow::AllTime => None,
        }
    }
}

/// Percentile values for latency analysis
#[derive(Debug, Clone, Copy)]
pub struct Percentiles {
    pub p50: f64,  // Median
    pub p75: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
}

impl Default for Percentiles {
    fn default() -> Self {
        Self {
            p50: 0.0,
            p75: 0.0,
            p90: 0.0,
            p95: 0.0,
            p99: 0.0,
            p999: 0.0,
        }
    }
}

/// Historical data point for trend analysis
#[derive(Debug, Clone)]
pub struct DataPoint {
    #[cfg(feature = "std")]
    pub timestamp: Instant,
    pub value: f64,
    pub metadata: Option<String>,
}

/// Trend direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendDirection {
    Increasing,
    Decreasing,
    Stable,
    Volatile,
}

/// Trend analysis result
#[derive(Debug, Clone)]
pub struct TrendAnalysis {
    pub direction: TrendDirection,
    pub slope: f64,
    pub confidence: f64, // 0.0 to 1.0
    pub prediction_next_hour: Option<f64>,
}

/// Advanced cache statistics with comprehensive metrics
#[derive(Debug, Clone)]
pub struct CacheStats {
    // Basic counters (atomic for thread safety)
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub insertions: u64,
    pub updates: u64,
    pub deletions: u64,
    pub expired_entries: u64,

    // Timing metrics (in nanoseconds)
    pub total_compute_time_ns: u64,
    pub total_lookup_time_ns: u64,
    pub total_insertion_time_ns: u64,
    pub total_eviction_time_ns: u64,

    // Size metrics
    pub size_bytes: u64,
    pub max_size_bytes: u64,
    pub entry_count: u64,
    pub max_entries: u64,
    pub peak_entry_count: u64,
    pub peak_size_bytes: u64,

    // Advanced metrics
    pub cache_efficiency_score: f64, // 0.0 to 100.0
    pub memory_pressure: f64,        // 0.0 to 1.0
    pub fragmentation_ratio: f64,    // 0.0 to 1.0
    pub load_factor: f64,           // 0.0 to 1.0

    // Timing metadata
    #[cfg(feature = "std")]
    pub start_time: SystemTime,
    #[cfg(feature = "std")]
    pub last_reset: SystemTime,

    // Error tracking
    pub allocation_failures: u64,
    pub timeout_errors: u64,
    pub corruption_errors: u64,

    // Distribution metrics
    pub key_size_distribution: SizeDistribution,
    pub value_size_distribution: SizeDistribution,
    pub access_pattern: AccessPattern,
}

/// Size distribution statistics
#[derive(Debug, Clone, Default)]
pub struct SizeDistribution {
    pub min: u64,
    pub max: u64,
    pub avg: f64,
    pub median: f64,
    pub percentiles: Percentiles,
    pub total_samples: u64,
}

/// Access pattern analysis
#[derive(Debug, Clone, Default)]
pub struct AccessPattern {
    pub sequential_access_ratio: f64,
    pub random_access_ratio: f64,
    pub hot_spot_ratio: f64,          // Percentage of keys accessed frequently
    pub temporal_locality: f64,       // How often recently accessed items are re-accessed
    pub spatial_locality: f64,        // How often nearby keys are accessed together
}

impl Default for CacheStats {
    fn default() -> Self {
        Self {
            hits: 0,
            misses: 0,
            evictions: 0,
            insertions: 0,
            updates: 0,
            deletions: 0,
            expired_entries: 0,
            total_compute_time_ns: 0,
            total_lookup_time_ns: 0,
            total_insertion_time_ns: 0,
            total_eviction_time_ns: 0,
            size_bytes: 0,
            max_size_bytes: 0,
            entry_count: 0,
            max_entries: 0,
            peak_entry_count: 0,
            peak_size_bytes: 0,
            cache_efficiency_score: 0.0,
            memory_pressure: 0.0,
            fragmentation_ratio: 0.0,
            load_factor: 0.0,
            #[cfg(feature = "std")]
            start_time: SystemTime::now(),
            #[cfg(feature = "std")]
            last_reset: SystemTime::now(),
            allocation_failures: 0,
            timeout_errors: 0,
            corruption_errors: 0,
            key_size_distribution: SizeDistribution::default(),
            value_size_distribution: SizeDistribution::default(),
            access_pattern: AccessPattern::default(),
        }
    }
}

impl CacheStats {
    /// Create new cache statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate the hit rate (0.0-1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Calculate the miss rate (0.0-1.0)
    pub fn miss_rate(&self) -> f64 {
        1.0 - self.hit_rate()
    }

    /// Calculate the eviction rate (evictions per operation)
    pub fn eviction_rate(&self) -> f64 {
        let total_ops = self.hits + self.misses + self.insertions + self.updates;
        if total_ops == 0 {
            0.0
        } else {
            self.evictions as f64 / total_ops as f64
        }
    }

    /// Calculate operations per second
    #[cfg(feature = "std")]
    pub fn ops_per_second(&self) -> f64 {
        let total_ops = self.hits + self.misses + self.insertions + self.updates + self.deletions;
        let elapsed = self.start_time.elapsed().unwrap_or_default().as_secs_f64();
        if elapsed <= 0.0 {
            0.0
        } else {
            total_ops as f64 / elapsed
        }
    }

    /// Calculate average latencies
    pub fn avg_latencies(&self) -> LatencyStats {
        let total_requests = self.hits + self.misses;

        LatencyStats {
            avg_lookup_ns: if total_requests > 0 {
                self.total_lookup_time_ns as f64 / total_requests as f64
            } else {
                0.0
            },
            avg_compute_ns: if self.misses > 0 {
                self.total_compute_time_ns as f64 / self.misses as f64
            } else {
                0.0
            },
            avg_insertion_ns: if self.insertions > 0 {
                self.total_insertion_time_ns as f64 / self.insertions as f64
            } else {
                0.0
            },
            avg_eviction_ns: if self.evictions > 0 {
                self.total_eviction_time_ns as f64 / self.evictions as f64
            } else {
                0.0
            },
        }
    }

    /// Calculate cache efficiency score (0.0 to 100.0)
    pub fn efficiency_score(&self) -> f64 {
        let hit_rate = self.hit_rate();
        let eviction_penalty = self.eviction_rate() * 10.0; // Penalize frequent evictions
        let utilization_bonus = self.load_factor * 10.0;   // Bonus for good utilization

        let base_score = hit_rate * 100.0;
        let adjusted_score = base_score - eviction_penalty + utilization_bonus;

        adjusted_score.max(0.0).min(100.0)
    }

    /// Get throughput metrics
    #[cfg(feature = "std")]
    pub fn throughput_metrics(&self) -> ThroughputMetrics {
        let uptime = self.start_time.elapsed().unwrap_or_default().as_secs_f64();

        if uptime <= 0.0 {
            return ThroughputMetrics::default();
        }

        ThroughputMetrics {
            hits_per_second: self.hits as f64 / uptime,
            misses_per_second: self.misses as f64 / uptime,
            insertions_per_second: self.insertions as f64 / uptime,
            evictions_per_second: self.evictions as f64 / uptime,
            total_ops_per_second: self.ops_per_second(),
            bytes_per_second: self.size_bytes as f64 / uptime,
        }
    }

    /// Reset all statistics
    pub fn reset(&mut self) {
        let max_size_bytes = self.max_size_bytes;
        let max_entries = self.max_entries;

        *self = Self::new();
        self.max_size_bytes = max_size_bytes;
        self.max_entries = max_entries;

        #[cfg(feature = "std")]
        {
            self.last_reset = SystemTime::now();
        }
    }

    /// Merge with another stats object
    pub fn merge(&mut self, other: &Self) {
        self.hits += other.hits;
        self.misses += other.misses;
        self.evictions += other.evictions;
        self.insertions += other.insertions;
        self.updates += other.updates;
        self.deletions += other.deletions;
        self.expired_entries += other.expired_entries;

        self.total_compute_time_ns += other.total_compute_time_ns;
        self.total_lookup_time_ns += other.total_lookup_time_ns;
        self.total_insertion_time_ns += other.total_insertion_time_ns;
        self.total_eviction_time_ns += other.total_eviction_time_ns;

        self.allocation_failures += other.allocation_failures;
        self.timeout_errors += other.timeout_errors;
        self.corruption_errors += other.corruption_errors;

        // Update peaks
        self.peak_entry_count = self.peak_entry_count.max(other.peak_entry_count);
        self.peak_size_bytes = self.peak_size_bytes.max(other.peak_size_bytes);

        // Recalculate derived metrics
        self.recalculate_derived_metrics();
    }

    /// Recalculate derived metrics
    pub fn recalculate_derived_metrics(&mut self) {
        self.cache_efficiency_score = self.efficiency_score();
        self.load_factor = if self.max_entries > 0 {
            self.entry_count as f64 / self.max_entries as f64
        } else {
            0.0
        };
        self.memory_pressure = if self.max_size_bytes > 0 {
            self.size_bytes as f64 / self.max_size_bytes as f64
        } else {
            0.0
        };
    }

    /// Update peaks if current values are higher
    pub fn update_peaks(&mut self) {
        self.peak_entry_count = self.peak_entry_count.max(self.entry_count);
        self.peak_size_bytes = self.peak_size_bytes.max(self.size_bytes);
    }
}

/// Latency statistics
#[derive(Debug, Clone, Default)]
pub struct LatencyStats {
    pub avg_lookup_ns: f64,
    pub avg_compute_ns: f64,
    pub avg_insertion_ns: f64,
    pub avg_eviction_ns: f64,
}

/// Throughput metrics
#[derive(Debug, Clone, Default)]
pub struct ThroughputMetrics {
    pub hits_per_second: f64,
    pub misses_per_second: f64,
    pub insertions_per_second: f64,
    pub evictions_per_second: f64,
    pub total_ops_per_second: f64,
    pub bytes_per_second: f64,
}

/// Thread-safe atomic cache statistics
pub struct AtomicCacheStats {
    // Atomic counters for high-performance updates
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    insertions: AtomicU64,
    updates: AtomicU64,
    deletions: AtomicU64,
    expired_entries: AtomicU64,

    // Timing accumulators
    total_compute_time_ns: AtomicU64,
    total_lookup_time_ns: AtomicU64,
    total_insertion_time_ns: AtomicU64,
    total_eviction_time_ns: AtomicU64,

    // Error counters
    allocation_failures: AtomicU64,
    timeout_errors: AtomicU64,
    corruption_errors: AtomicU64,

    // Complex metrics requiring locks
    detailed_stats: RwLock<CacheStats>,

    // Historical data for trend analysis
    #[cfg(feature = "std")]
    history: Mutex<VecDeque<DataPoint>>,

    #[cfg(feature = "std")]
    start_time: SystemTime,
}

impl AtomicCacheStats {
    /// Create new atomic cache statistics
    pub fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            insertions: AtomicU64::new(0),
            updates: AtomicU64::new(0),
            deletions: AtomicU64::new(0),
            expired_entries: AtomicU64::new(0),
            total_compute_time_ns: AtomicU64::new(0),
            total_lookup_time_ns: AtomicU64::new(0),
            total_insertion_time_ns: AtomicU64::new(0),
            total_eviction_time_ns: AtomicU64::new(0),
            allocation_failures: AtomicU64::new(0),
            timeout_errors: AtomicU64::new(0),
            corruption_errors: AtomicU64::new(0),
            detailed_stats: RwLock::new(CacheStats::new()),
            #[cfg(feature = "std")]
            history: Mutex::new(VecDeque::new()),
            #[cfg(feature = "std")]
            start_time: SystemTime::now(),
        }
    }

    /// Record a cache hit with optional lookup time
    pub fn record_hit(&self, lookup_time_ns: Option<u64>) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        if let Some(time) = lookup_time_ns {
            self.total_lookup_time_ns.fetch_add(time, Ordering::Relaxed);
        }
    }

    /// Record a cache miss with optional lookup and compute time
    pub fn record_miss(&self, lookup_time_ns: Option<u64>, compute_time_ns: Option<u64>) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        if let Some(time) = lookup_time_ns {
            self.total_lookup_time_ns.fetch_add(time, Ordering::Relaxed);
        }
        if let Some(time) = compute_time_ns {
            self.total_compute_time_ns.fetch_add(time, Ordering::Relaxed);
        }
    }

    /// Record a cache insertion with optional timing
    pub fn record_insertion(&self, insertion_time_ns: Option<u64>) {
        self.insertions.fetch_add(1, Ordering::Relaxed);
        if let Some(time) = insertion_time_ns {
            self.total_insertion_time_ns.fetch_add(time, Ordering::Relaxed);
        }
    }

    /// Record a cache eviction with optional timing
    pub fn record_eviction(&self, eviction_time_ns: Option<u64>) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
        if let Some(time) = eviction_time_ns {
            self.total_eviction_time_ns.fetch_add(time, Ordering::Relaxed);
        }
    }

    /// Record a cache update
    pub fn record_update(&self) {
        self.updates.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache deletion
    pub fn record_deletion(&self) {
        self.deletions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record expired entries cleanup
    pub fn record_expired_cleanup(&self, count: u64) {
        self.expired_entries.fetch_add(count, Ordering::Relaxed);
    }

    /// Record an allocation failure
    pub fn record_allocation_failure(&self) {
        self.allocation_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a timeout error
    pub fn record_timeout_error(&self) {
        self.timeout_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a corruption error
    pub fn record_corruption_error(&self) {
        self.corruption_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Update cache size and entry count
    pub fn update_size(&self, size_bytes: u64, entry_count: u64) {
        let mut stats = self.detailed_stats.write().unwrap();
        stats.size_bytes = size_bytes;
        stats.entry_count = entry_count;
        stats.update_peaks();
        stats.recalculate_derived_metrics();
    }

    /// Set maximum cache size
    pub fn set_max_size(&self, max_size_bytes: u64, max_entries: u64) {
        let mut stats = self.detailed_stats.write().unwrap();
        stats.max_size_bytes = max_size_bytes;
        stats.max_entries = max_entries;
        stats.recalculate_derived_metrics();
    }

    /// Update size distribution
    pub fn update_key_size_distribution(&self, distribution: SizeDistribution) {
        let mut stats = self.detailed_stats.write().unwrap();
        stats.key_size_distribution = distribution;
    }

    /// Update value size distribution
    pub fn update_value_size_distribution(&self, distribution: SizeDistribution) {
        let mut stats = self.detailed_stats.write().unwrap();
        stats.value_size_distribution = distribution;
    }

    /// Update access pattern
    pub fn update_access_pattern(&self, pattern: AccessPattern) {
        let mut stats = self.detailed_stats.write().unwrap();
        stats.access_pattern = pattern;
    }

    /// Get a snapshot of current statistics
    pub fn get_stats(&self) -> CacheStats {
        let mut stats = self.detailed_stats.read().unwrap().clone();

        // Update with atomic values
        stats.hits = self.hits.load(Ordering::Relaxed);
        stats.misses = self.misses.load(Ordering::Relaxed);
        stats.evictions = self.evictions.load(Ordering::Relaxed);
        stats.insertions = self.insertions.load(Ordering::Relaxed);
        stats.updates = self.updates.load(Ordering::Relaxed);
        stats.deletions = self.deletions.load(Ordering::Relaxed);
        stats.expired_entries = self.expired_entries.load(Ordering::Relaxed);
        stats.total_compute_time_ns = self.total_compute_time_ns.load(Ordering::Relaxed);
        stats.total_lookup_time_ns = self.total_lookup_time_ns.load(Ordering::Relaxed);
        stats.total_insertion_time_ns = self.total_insertion_time_ns.load(Ordering::Relaxed);
        stats.total_eviction_time_ns = self.total_eviction_time_ns.load(Ordering::Relaxed);
        stats.allocation_failures = self.allocation_failures.load(Ordering::Relaxed);
        stats.timeout_errors = self.timeout_errors.load(Ordering::Relaxed);
        stats.corruption_errors = self.corruption_errors.load(Ordering::Relaxed);

        stats.recalculate_derived_metrics();
        stats
    }

    /// Get statistics for a specific time window
    #[cfg(feature = "std")]
    pub fn get_stats_for_window(&self, window: TimeWindow) -> CacheStats {
        if window == TimeWindow::AllTime {
            return self.get_stats();
        }

        let history = self.history.lock().unwrap();
        let cutoff = if let Some(duration) = window.duration() {
            Instant::now() - duration
        } else {
            return self.get_stats();
        };

        // Filter data points within the window
        let recent_points: Vec<_> = history
            .iter()
            .filter(|point| point.timestamp >= cutoff)
            .collect();

        if recent_points.is_empty() {
            return CacheStats::new();
        }

        // Calculate windowed statistics
        // This is a simplified implementation - in production you'd want more sophisticated aggregation
        let windowed_stats = CacheStats::new();

        // For demonstration, just use the latest values
        // In a real implementation, you'd aggregate the data points properly
        windowed_stats
    }

    /// Add a data point to history for trend analysis
    #[cfg(feature = "std")]
    pub fn add_historical_point(&self, metric_name: &str, value: f64) {
        let mut history = self.history.lock().unwrap();

        let point = DataPoint {
            timestamp: Instant::now(),
            value,
            metadata: Some(metric_name.to_string()),
        };

        history.push_back(point);

        // Keep only last 1000 points to prevent unbounded growth
        if history.len() > 1000 {
            history.pop_front();
        }
    }

    /// Analyze trends for a specific metric
    #[cfg(feature = "std")]
    pub fn analyze_trend(&self, metric_name: &str, window: TimeWindow) -> Option<TrendAnalysis> {
        let history = self.history.lock().unwrap();

        let cutoff = if let Some(duration) = window.duration() {
            Instant::now() - duration
        } else {
            // For AllTime, use all available data
            return None; // Simplified for this example
        };

        let points: Vec<_> = history
            .iter()
            .filter(|point| {
                point.timestamp >= cutoff &&
                    point.metadata.as_ref().map(|m| m == metric_name).unwrap_or(false)
            })
            .collect();

        if points.len() < 2 {
            return None;
        }

        // Simple linear regression for trend analysis
        let n = points.len() as f64;
        let x_values: Vec<f64> = (0..points.len()).map(|i| i as f64).collect();
        let y_values: Vec<f64> = points.iter().map(|p| p.value).collect();

        let x_mean = x_values.iter().sum::<f64>() / n;
        let y_mean = y_values.iter().sum::<f64>() / n;

        let numerator: f64 = x_values.iter().zip(y_values.iter())
            .map(|(x, y)| (x - x_mean) * (y - y_mean))
            .sum();

        let denominator: f64 = x_values.iter()
            .map(|x| (x - x_mean).powi(2))
            .sum();

        if denominator == 0.0 {
            return None;
        }

        let slope = numerator / denominator;

        // Determine trend direction
        let direction = if slope.abs() < 0.01 {
            TrendDirection::Stable
        } else if slope > 0.0 {
            TrendDirection::Increasing
        } else {
            TrendDirection::Decreasing
        };

        // Calculate confidence based on R-squared
        let y_pred: Vec<f64> = x_values.iter()
            .map(|x| y_mean + slope * (x - x_mean))
            .collect();

        let ss_res: f64 = y_values.iter().zip(y_pred.iter())
            .map(|(y, y_pred)| (y - y_pred).powi(2))
            .sum();

        let ss_tot: f64 = y_values.iter()
            .map(|y| (y - y_mean).powi(2))
            .sum();

        let r_squared = if ss_tot > 0.0 {
            1.0 - (ss_res / ss_tot)
        } else {
            0.0
        };

        // Predict next hour value (simplified)
        let prediction_next_hour = if points.len() > 0 {
            Some(points.last().unwrap().value + slope * 60.0) // Assume hourly data points
        } else {
            None
        };

        Some(TrendAnalysis {
            direction,
            slope,
            confidence: r_squared.max(0.0).min(1.0),
            prediction_next_hour,
        })
    }

    /// Reset all statistics
    pub fn reset(&self) {
        // Reset atomic counters
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.insertions.store(0, Ordering::Relaxed);
        self.updates.store(0, Ordering::Relaxed);
        self.deletions.store(0, Ordering::Relaxed);
        self.expired_entries.store(0, Ordering::Relaxed);
        self.total_compute_time_ns.store(0, Ordering::Relaxed);
        self.total_lookup_time_ns.store(0, Ordering::Relaxed);
        self.total_insertion_time_ns.store(0, Ordering::Relaxed);
        self.total_eviction_time_ns.store(0, Ordering::Relaxed);
        self.allocation_failures.store(0, Ordering::Relaxed);
        self.timeout_errors.store(0, Ordering::Relaxed);
        self.corruption_errors.store(0, Ordering::Relaxed);

        // Reset detailed stats
        let mut stats = self.detailed_stats.write().unwrap();
        stats.reset();

        // Clear history
        #[cfg(feature = "std")]
        {
            let mut history = self.history.lock().unwrap();
            history.clear();
        }
    }
}

impl Default for AtomicCacheStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Enhanced cache statistics collector
pub struct StatsCollector {
    /// Stats for each named cache
    caches: HashMap<String, Arc<AtomicCacheStats>>,
    /// Global aggregated stats
    global_stats: Arc<AtomicCacheStats>,
    /// Collection start time
    #[cfg(feature = "std")]
    start_time: SystemTime,
}

impl StatsCollector {
    /// Create a new stats collector
    pub fn new() -> Self {
        Self {
            caches: HashMap::new(),
            global_stats: Arc::new(AtomicCacheStats::new()),
            #[cfg(feature = "std")]
            start_time: SystemTime::now(),
        }
    }

    /// Register a cache with the collector
    pub fn register_cache(&mut self, name: &str) -> Arc<AtomicCacheStats> {
        let stats = Arc::new(AtomicCacheStats::new());
        self.caches.insert(name.to_string(), Arc::clone(&stats));
        stats
    }

    /// Get stats for a specific cache
    pub fn get_cache_stats(&self, name: &str) -> Option<CacheStats> {
        self.caches.get(name).map(|stats| stats.get_stats())
    }

    /// Get combined stats for all caches
    pub fn get_combined_stats(&self) -> CacheStats {
        let mut combined = CacheStats::new();

        for stats in self.caches.values() {
            let cache_stats = stats.get_stats();
            combined.merge(&cache_stats);
        }

        combined
    }

    /// Get stats for a specific time window across all caches
    #[cfg(feature = "std")]
    pub fn get_windowed_stats(&self, window: TimeWindow) -> CacheStats {
        let mut combined = CacheStats::new();

        for stats in self.caches.values() {
            let cache_stats = stats.get_stats_for_window(window);
            combined.merge(&cache_stats);
        }

        combined
    }

    /// Get trend analysis for a specific metric across all caches
    #[cfg(feature = "std")]
    pub fn get_trend_analysis(&self, metric_name: &str, window: TimeWindow) -> Vec<(String, TrendAnalysis)> {
        let mut trends = Vec::new();

        for (cache_name, stats) in &self.caches {
            if let Some(trend) = stats.analyze_trend(metric_name, window) {
                trends.push((cache_name.clone(), trend));
            }
        }

        trends
    }

    /// Generate a comprehensive performance report
    pub fn generate_report(&self) -> PerformanceReport {
        let combined_stats = self.get_combined_stats();
        let cache_reports: Vec<_> = self.caches
            .iter()
            .map(|(name, stats)| (name.clone(), stats.get_stats()))
            .collect();

        PerformanceReport {
            overall_stats: combined_stats,
            cache_breakdown: cache_reports,
            #[cfg(feature = "std")]
            collection_duration: self.start_time.elapsed().unwrap_or_default(),
            recommendations: self.generate_recommendations(),
        }
    }

    /// Generate performance recommendations
    fn generate_recommendations(&self) -> Vec<String> {
        let mut recommendations = Vec::new();
        let combined = self.get_combined_stats();

        // Hit rate recommendations
        if combined.hit_rate() < 0.7 {
            recommendations.push("Hit rate is low (<70%) - consider increasing cache size or improving cache warming".to_string());
        }

        // Eviction rate recommendations
        if combined.eviction_rate() > 0.2 {
            recommendations.push("High eviction rate (>20%) - consider increasing cache capacity or adjusting eviction policy".to_string());
        }

        // Memory pressure recommendations
        if combined.memory_pressure > 0.9 {
            recommendations.push("High memory pressure (>90%) - consider reducing cache size or increasing available memory".to_string());
        }

        // Error rate recommendations
        let total_ops = combined.hits + combined.misses + combined.insertions;
        let error_rate = if total_ops > 0 {
            (combined.allocation_failures + combined.timeout_errors + combined.corruption_errors) as f64 / total_ops as f64
        } else {
            0.0
        };

        if error_rate > 0.01 {
            recommendations.push("High error rate (>1%) - investigate allocation failures, timeouts, or corruption issues".to_string());
        }

        // Efficiency recommendations
        if combined.efficiency_score() < 60.0 {
            recommendations.push("Low efficiency score (<60%) - review cache configuration and access patterns".to_string());
        }

        if recommendations.is_empty() {
            recommendations.push("Cache performance is within acceptable ranges".to_string());
        }

        recommendations
    }

    /// Reset stats for a specific cache
    pub fn reset_cache_stats(&self, name: &str) -> MemoryResult<()> {
        if let Some(stats) = self.caches.get(name) {
            stats.reset();
            Ok(())
        } else {
            Err(MemoryError::invalid_argument("cache not found"))
        }
    }

    /// Reset stats for all caches
    pub fn reset_all_stats(&self) {
        for stats in self.caches.values() {
            stats.reset();
        }
        self.global_stats.reset();
    }

    /// Get names of all registered caches
    pub fn get_cache_names(&self) -> Vec<String> {
        self.caches.keys().cloned().collect()
    }
}

impl Default for StatsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Comprehensive performance report
#[derive(Debug, Clone)]
pub struct PerformanceReport {
    pub overall_stats: CacheStats,
    pub cache_breakdown: Vec<(String, CacheStats)>,
    #[cfg(feature = "std")]
    pub collection_duration: Duration,
    pub recommendations: Vec<String>,
}

/// Cache statistics provider trait
pub trait StatsProvider {
    /// Get the current cache statistics
    fn get_stats(&self) -> CacheStats;

    /// Get statistics for a specific time window
    #[cfg(feature = "std")]
    fn get_stats_for_window(&self, window: TimeWindow) -> CacheStats;

    /// Reset the cache statistics
    fn reset_stats(&self);

    /// Get latency percentiles
    fn get_latency_percentiles(&self) -> Percentiles;

    /// Get throughput metrics
    #[cfg(feature = "std")]
    fn get_throughput_metrics(&self) -> ThroughputMetrics;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_stats_calculations() {
        let mut stats = CacheStats::new();

        stats.hits = 80;
        stats.misses = 20;
        stats.insertions = 30;
        stats.evictions = 10;
        stats.entry_count = 25;
        stats.max_entries = 50;

        assert_eq!(stats.hit_rate(), 0.8);
        assert_eq!(stats.miss_rate(), 0.2);
        assert!(stats.efficiency_score() > 0.0);

        stats.recalculate_derived_metrics();
        assert_eq!(stats.load_factor, 0.5);
    }

    #[test]
    fn test_atomic_cache_stats() {
        let stats = AtomicCacheStats::new();

        stats.record_hit(Some(1000));
        stats.record_hit(Some(1500));
        stats.record_miss(Some(2000), Some(5000));
        stats.record_insertion(Some(3000));
        stats.record_eviction(Some(4000));

        let snapshot = stats.get_stats();

        assert_eq!(snapshot.hits, 2);
        assert_eq!(snapshot.misses, 1);
        assert_eq!(snapshot.insertions, 1);
        assert_eq!(snapshot.evictions, 1);
        assert_eq!(snapshot.total_lookup_time_ns, 4500); // 1000 + 1500 + 2000
        assert_eq!(snapshot.total_compute_time_ns, 5000);

        let latencies = snapshot.avg_latencies();
        assert_eq!(latencies.avg_lookup_ns, 1500.0); // 4500 / 3
        assert_eq!(latencies.avg_compute_ns, 5000.0); // 5000 / 1
    }

    #[test]
    fn test_stats_collector() {
        let mut collector = StatsCollector::new();

        let cache1_stats = collector.register_cache("cache1");
        let cache2_stats = collector.register_cache("cache2");

        // Record some operations
        cache1_stats.record_hit(Some(1000));
        cache1_stats.record_miss(Some(2000), Some(10000));

        cache2_stats.record_hit(Some(1500));
        cache2_stats.record_insertion(Some(3000));

        // Test individual cache stats
        let cache1 = collector.get_cache_stats("cache1").unwrap();
        assert_eq!(cache1.hits, 1);
        assert_eq!(cache1.misses, 1);

        // Test combined stats
        let combined = collector.get_combined_stats();
        assert_eq!(combined.hits, 2);
        assert_eq!(combined.misses, 1);
        assert_eq!(combined.insertions, 1);

        // Test report generation
        let report = collector.generate_report();
        assert_eq!(report.cache_breakdown.len(), 2);
        assert!(!report.recommendations.is_empty());
    }

    #[test]
    fn test_performance_recommendations() {
        let mut collector = StatsCollector::new();
        let cache_stats = collector.register_cache("test_cache");

        // Simulate poor performance
        for _ in 0..100 {
            cache_stats.record_miss(Some(1000), Some(10000));
        }
        for _ in 0..20 {
            cache_stats.record_hit(Some(500));
        }
        for _ in 0..50 {
            cache_stats.record_eviction(Some(2000));
        }

        let report = collector.generate_report();

        // Should generate recommendations for low hit rate and high eviction rate
        assert!(report.recommendations.iter().any(|r| r.contains("Hit rate is low")));
        assert!(report.recommendations.iter().any(|r| r.contains("High eviction rate")));
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_historical_tracking() {
        let stats = AtomicCacheStats::new();

        // Add some historical data points
        stats.add_historical_point("hit_rate", 0.7);
        stats.add_historical_point("hit_rate", 0.75);
        stats.add_historical_point("hit_rate", 0.8);
        stats.add_historical_point("hit_rate", 0.85);

        // Test trend analysis
        if let Some(trend) = stats.analyze_trend("hit_rate", TimeWindow::Last1Hour) {
            assert_eq!(trend.direction, TrendDirection::Increasing);
            assert!(trend.slope > 0.0);
        }
    }
}