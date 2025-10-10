//! Simple cache statistics
//!
//! This module provides basic statistics tracking for cache operations.
//! It focuses on the essential metrics needed for monitoring cache performance
//! without the complexity of advanced profiling and trend analysis.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(feature = "std"))]
use core::sync::atomic::{AtomicU64, Ordering};

/// Simple cache statistics with basic counters
///
/// This struct provides thread-safe atomic counters for tracking cache operations.
/// All fields use atomic operations for lock-free updates in multi-threaded scenarios.
///
/// # Examples
///
/// ```ignore
/// use nebula_memory::cache::AtomicCacheStats;
///
/// let stats = AtomicCacheStats::new();
///
/// // Record operations
/// stats.record_hit();
/// stats.record_miss();
/// stats.record_insertion();
///
/// // Get snapshot
/// let snapshot = stats.snapshot();
/// println!("Hit rate: {:.2}%", snapshot.hit_rate());
/// ```
#[derive(Debug)]
pub struct AtomicCacheStats {
    /// Number of cache hits
    hits: AtomicU64,
    /// Number of cache misses
    misses: AtomicU64,
    /// Number of evictions
    evictions: AtomicU64,
    /// Number of insertions
    insertions: AtomicU64,
    /// Number of deletions
    deletions: AtomicU64,
    /// Current number of entries
    entry_count: AtomicU64,
    /// Current size in bytes
    size_bytes: AtomicU64,
}

impl AtomicCacheStats {
    /// Create a new stats tracker
    pub fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            insertions: AtomicU64::new(0),
            deletions: AtomicU64::new(0),
            entry_count: AtomicU64::new(0),
            size_bytes: AtomicU64::new(0),
        }
    }

    /// Record a cache hit
    #[inline]
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss
    #[inline]
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an eviction
    #[inline]
    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an insertion
    #[inline]
    pub fn record_insertion(&self, size_bytes: usize) {
        self.insertions.fetch_add(1, Ordering::Relaxed);
        self.entry_count.fetch_add(1, Ordering::Relaxed);
        self.size_bytes
            .fetch_add(size_bytes as u64, Ordering::Relaxed);
    }

    /// Record a deletion
    #[inline]
    pub fn record_deletion(&self, size_bytes: usize) {
        self.deletions.fetch_add(1, Ordering::Relaxed);
        self.entry_count.fetch_sub(1, Ordering::Relaxed);
        self.size_bytes
            .fetch_sub(size_bytes as u64, Ordering::Relaxed);
    }

    /// Get current hit count
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get current miss count
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get current eviction count
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Get current insertion count
    pub fn insertions(&self) -> u64 {
        self.insertions.load(Ordering::Relaxed)
    }

    /// Get current deletion count
    pub fn deletions(&self) -> u64 {
        self.deletions.load(Ordering::Relaxed)
    }

    /// Get current entry count
    pub fn entry_count(&self) -> u64 {
        self.entry_count.load(Ordering::Relaxed)
    }

    /// Get current size in bytes
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes.load(Ordering::Relaxed)
    }

    /// Get a snapshot of current statistics
    pub fn snapshot(&self) -> CacheStats {
        CacheStats {
            hits: self.hits(),
            misses: self.misses(),
            evictions: self.evictions(),
            insertions: self.insertions(),
            deletions: self.deletions(),
            entry_count: self.entry_count(),
            size_bytes: self.size_bytes(),
        }
    }

    /// Reset all counters to zero
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.insertions.store(0, Ordering::Relaxed);
        self.deletions.store(0, Ordering::Relaxed);
        self.entry_count.store(0, Ordering::Relaxed);
        self.size_bytes.store(0, Ordering::Relaxed);
    }
}

impl Default for AtomicCacheStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of cache statistics
///
/// This is a point-in-time snapshot of cache statistics that can be
/// cheaply copied and analyzed without holding locks.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of evictions
    pub evictions: u64,
    /// Number of insertions
    pub insertions: u64,
    /// Number of deletions
    pub deletions: u64,
    /// Current number of entries
    pub entry_count: u64,
    /// Current size in bytes
    pub size_bytes: u64,
}

impl CacheStats {
    /// Calculate hit rate as a percentage (0.0 to 100.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }

    /// Calculate miss rate as a percentage (0.0 to 100.0)
    pub fn miss_rate(&self) -> f64 {
        100.0 - self.hit_rate()
    }

    /// Get total number of requests (hits + misses)
    pub fn total_requests(&self) -> u64 {
        self.hits + self.misses
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }
}

impl Default for CacheStats {
    fn default() -> Self {
        Self {
            hits: 0,
            misses: 0,
            evictions: 0,
            insertions: 0,
            deletions: 0,
            entry_count: 0,
            size_bytes: 0,
        }
    }
}

/// Trait for types that can provide cache statistics
pub trait StatsProvider {
    /// Get a snapshot of current statistics
    fn stats(&self) -> CacheStats;

    /// Reset statistics counters
    fn reset_stats(&self);
}

/// Simple stats collector for manual tracking
///
/// Unlike AtomicCacheStats, this is not thread-safe and should be used
/// only in single-threaded contexts or with external synchronization.
#[derive(Debug, Clone)]
pub struct StatsCollector {
    stats: CacheStats,
}

impl StatsCollector {
    /// Create a new stats collector
    pub fn new() -> Self {
        Self {
            stats: CacheStats::default(),
        }
    }

    /// Record a cache hit
    pub fn record_hit(&mut self) {
        self.stats.hits += 1;
    }

    /// Record a cache miss
    pub fn record_miss(&mut self) {
        self.stats.misses += 1;
    }

    /// Record an eviction
    pub fn record_eviction(&mut self) {
        self.stats.evictions += 1;
    }

    /// Record an insertion
    pub fn record_insertion(&mut self, size_bytes: usize) {
        self.stats.insertions += 1;
        self.stats.entry_count += 1;
        self.stats.size_bytes += size_bytes as u64;
    }

    /// Record a deletion
    pub fn record_deletion(&mut self, size_bytes: usize) {
        self.stats.deletions += 1;
        self.stats.entry_count = self.stats.entry_count.saturating_sub(1);
        self.stats.size_bytes = self.stats.size_bytes.saturating_sub(size_bytes as u64);
    }

    /// Get current statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Reset all counters
    pub fn reset(&mut self) {
        self.stats = CacheStats::default();
    }
}

impl Default for StatsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_stats_basic() {
        let stats = AtomicCacheStats::new();

        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 0);

        stats.record_hit();
        stats.record_hit();
        stats.record_miss();

        assert_eq!(stats.hits(), 2);
        assert_eq!(stats.misses(), 1);
    }

    #[test]
    fn test_atomic_stats_insertion() {
        let stats = AtomicCacheStats::new();

        stats.record_insertion(100);
        stats.record_insertion(200);

        assert_eq!(stats.insertions(), 2);
        assert_eq!(stats.entry_count(), 2);
        assert_eq!(stats.size_bytes(), 300);
    }

    #[test]
    fn test_atomic_stats_deletion() {
        let stats = AtomicCacheStats::new();

        stats.record_insertion(100);
        stats.record_insertion(200);
        stats.record_deletion(100);

        assert_eq!(stats.deletions(), 1);
        assert_eq!(stats.entry_count(), 1);
        assert_eq!(stats.size_bytes(), 200);
    }

    #[test]
    fn test_snapshot() {
        let stats = AtomicCacheStats::new();

        stats.record_hit();
        stats.record_hit();
        stats.record_miss();
        stats.record_insertion(100);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.hits, 2);
        assert_eq!(snapshot.misses, 1);
        assert_eq!(snapshot.entry_count, 1);
        assert_eq!(snapshot.size_bytes, 100);
    }

    #[test]
    fn test_hit_rate() {
        let mut snapshot = CacheStats::default();
        snapshot.hits = 80;
        snapshot.misses = 20;

        assert_eq!(snapshot.hit_rate(), 80.0);
        assert_eq!(snapshot.miss_rate(), 20.0);
        assert_eq!(snapshot.total_requests(), 100);
    }

    #[test]
    fn test_hit_rate_no_requests() {
        let snapshot = CacheStats::default();
        assert_eq!(snapshot.hit_rate(), 0.0);
        assert_eq!(snapshot.total_requests(), 0);
    }

    #[test]
    fn test_reset() {
        let stats = AtomicCacheStats::new();

        stats.record_hit();
        stats.record_miss();
        stats.record_insertion(100);

        assert_eq!(stats.hits(), 1);

        stats.reset();

        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 0);
        assert_eq!(stats.insertions(), 0);
        assert_eq!(stats.entry_count(), 0);
    }

    #[test]
    fn test_stats_collector() {
        let mut collector = StatsCollector::new();

        collector.record_hit();
        collector.record_hit();
        collector.record_miss();
        collector.record_insertion(100);

        let stats = collector.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.size_bytes, 100);

        collector.reset();
        assert_eq!(collector.stats().hits, 0);
    }
}
