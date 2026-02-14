//! Core memory statistics types
//!
//! Provides atomic, lock-free statistics tracking for memory operations.

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Core memory statistics tracked atomically
#[derive(Debug)]
pub struct MemoryStats {
    // Allocation counters
    pub(crate) allocations: AtomicU64,
    pub(crate) deallocations: AtomicU64,

    // Memory tracking
    pub(crate) allocated_bytes: AtomicUsize, // Current allocated
    pub(crate) peak_allocated: AtomicUsize,  // Peak allocated
    pub(crate) total_allocated_bytes: AtomicUsize, // Total ever allocated
    pub(crate) total_deallocated_bytes: AtomicUsize, // Total ever deallocated

    // Timing for allocations
    pub(crate) total_allocation_time_nanos: AtomicU64,

    // Operation stats (for caches, pools, etc.)
    pub(crate) operations: AtomicU64,
    pub(crate) hits: AtomicU64,
    pub(crate) misses: AtomicU64,
    pub(crate) evictions: AtomicU64,

    // Error stats
    pub(crate) allocation_failures: AtomicU64,
    pub(crate) oom_errors: AtomicU64,

    // Timing metadata
    pub(crate) created_at: Instant,
}

impl MemoryStats {
    /// Create new statistics instance
    pub fn new() -> Self {
        Self {
            allocations: AtomicU64::new(0),
            deallocations: AtomicU64::new(0),
            allocated_bytes: AtomicUsize::new(0),
            peak_allocated: AtomicUsize::new(0),
            total_allocated_bytes: AtomicUsize::new(0),
            total_deallocated_bytes: AtomicUsize::new(0),
            total_allocation_time_nanos: AtomicU64::new(0),
            operations: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            allocation_failures: AtomicU64::new(0),
            oom_errors: AtomicU64::new(0),
            created_at: Instant::now(),
        }
    }

    /// Record an allocation
    #[inline]
    pub fn record_allocation(&self, size: usize) {
        self.allocations.fetch_add(1, Ordering::Relaxed);
        self.total_allocated_bytes
            .fetch_add(size, Ordering::Relaxed);
        let current = self.allocated_bytes.fetch_add(size, Ordering::Relaxed) + size;

        // Update peak if necessary
        let mut peak = self.peak_allocated.load(Ordering::Relaxed);
        while current > peak {
            match self.peak_allocated.compare_exchange_weak(
                peak,
                current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(p) => peak = p,
            }
        }
    }

    /// Record allocation timing
    #[inline]
    pub fn record_allocation_time(&self, duration: Duration) {
        self.total_allocation_time_nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record a deallocation
    #[inline]
    pub fn record_deallocation(&self, size: usize) {
        self.deallocations.fetch_add(1, Ordering::Relaxed);
        self.total_deallocated_bytes
            .fetch_add(size, Ordering::Relaxed);
        self.allocated_bytes.fetch_sub(size, Ordering::Relaxed);
    }

    /// Record a cache hit
    #[inline]
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        self.operations.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss
    #[inline]
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        self.operations.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an eviction
    #[inline]
    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an allocation failure
    #[inline]
    pub fn record_allocation_failure(&self) {
        self.allocation_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an OOM error
    #[inline]
    pub fn record_oom(&self) {
        self.oom_errors.fetch_add(1, Ordering::Relaxed);
    }

    // Getters
    #[inline]
    pub fn current_allocated(&self) -> usize {
        self.allocated_bytes.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn peak_allocated(&self) -> usize {
        self.peak_allocated.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn allocations(&self) -> u64 {
        self.allocations.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn deallocations(&self) -> u64 {
        self.deallocations.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn total_allocated_bytes(&self) -> usize {
        self.total_allocated_bytes.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn total_deallocated_bytes(&self) -> usize {
        self.total_deallocated_bytes.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn total_allocation_time_nanos(&self) -> u64 {
        self.total_allocation_time_nanos.load(Ordering::Relaxed)
    }

    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let ops = self.operations.load(Ordering::Relaxed);
        if ops == 0 {
            0.0
        } else {
            hits as f64 / ops as f64
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Reset all statistics
    pub fn reset(&self) {
        self.allocations.store(0, Ordering::Relaxed);
        self.deallocations.store(0, Ordering::Relaxed);
        self.allocated_bytes.store(0, Ordering::Relaxed);
        self.peak_allocated.store(0, Ordering::Relaxed);
        self.total_allocated_bytes.store(0, Ordering::Relaxed);
        self.total_deallocated_bytes.store(0, Ordering::Relaxed);
        self.total_allocation_time_nanos.store(0, Ordering::Relaxed);
        self.operations.store(0, Ordering::Relaxed);
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.allocation_failures.store(0, Ordering::Relaxed);
        self.oom_errors.store(0, Ordering::Relaxed);
    }

    /// Get a snapshot of current metrics
    pub fn metrics(&self) -> MemoryMetrics {
        MemoryMetrics {
            allocations: self.allocations(),
            deallocations: self.deallocations(),
            current_allocated: self.current_allocated(),
            peak_allocated: self.peak_allocated(),
            total_allocated_bytes: self.total_allocated_bytes(),
            total_deallocated_bytes: self.total_deallocated_bytes(),
            total_allocation_time_nanos: self.total_allocation_time_nanos(),
            operations: self.operations.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            allocation_failures: self.allocation_failures.load(Ordering::Relaxed),
            oom_errors: self.oom_errors.load(Ordering::Relaxed),
            hit_rate: self.hit_rate(),
            elapsed_secs: self.elapsed().as_secs_f64(),
            timestamp: Instant::now(),
        }
    }
}

impl Default for MemoryStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of memory metrics at a specific point in time
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryMetrics {
    // Allocation metrics
    pub allocations: u64,
    pub deallocations: u64,
    pub current_allocated: usize,
    pub peak_allocated: usize,
    pub total_allocated_bytes: usize,
    pub total_deallocated_bytes: usize,

    // Timing
    pub total_allocation_time_nanos: u64,

    // Operation metrics
    pub operations: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,

    // Error metrics
    pub allocation_failures: u64,
    pub oom_errors: u64,

    // Derived metrics
    pub hit_rate: f64,

    // Timing metadata
    pub elapsed_secs: f64,
    pub timestamp: Instant,
}

impl MemoryMetrics {
    /// Calculate fragmentation ratio
    pub fn fragmentation_ratio(&self) -> f64 {
        if self.peak_allocated == 0 {
            0.0
        } else {
            1.0 - (self.current_allocated as f64 / self.peak_allocated as f64)
        }
    }

    /// Calculate allocation rate (allocs per second)
    pub fn allocation_rate(&self) -> f64 {
        if self.elapsed_secs == 0.0 {
            0.0
        } else {
            self.allocations as f64 / self.elapsed_secs
        }
    }

    /// Calculate average allocation size
    pub fn avg_allocation_size(&self) -> usize {
        if self.allocations == 0 {
            0
        } else {
            self.total_allocated_bytes / self.allocations as usize
        }
    }

    /// Calculate average allocation latency
    pub fn avg_allocation_latency_nanos(&self) -> f64 {
        if self.allocations == 0 {
            0.0
        } else {
            self.total_allocation_time_nanos as f64 / self.allocations as f64
        }
    }

    /// Check if metrics indicate memory pressure
    pub fn is_under_pressure(&self) -> bool {
        self.allocation_failures > 0 || self.oom_errors > 0 || self.fragmentation_ratio() > 0.5
    }
}

impl Default for MemoryMetrics {
    fn default() -> Self {
        Self {
            allocations: 0,
            deallocations: 0,
            current_allocated: 0,
            peak_allocated: 0,
            total_allocated_bytes: 0,
            total_deallocated_bytes: 0,
            total_allocation_time_nanos: 0,
            operations: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            allocation_failures: 0,
            oom_errors: 0,
            hit_rate: 0.0,
            elapsed_secs: 0.0,
            timestamp: Instant::now(),
        }
    }
}

impl fmt::Display for MemoryMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Memory Metrics:")?;
        writeln!(f, "  Current: {} bytes", self.current_allocated)?;
        writeln!(f, "  Peak: {} bytes", self.peak_allocated)?;
        writeln!(
            f,
            "  Allocations: {} ({} failed)",
            self.allocations, self.allocation_failures
        )?;
        writeln!(f, "  Hit Rate: {:.2}%", self.hit_rate * 100.0)?;

        if self.oom_errors > 0 {
            writeln!(f, "  OOM Errors: {}", self.oom_errors)?;
        }

        if self.elapsed_secs > 0.0 {
            writeln!(f, "  Alloc Rate: {:.2} ops/sec", self.allocation_rate())?;
            writeln!(
                f,
                "  Avg Latency: {:.2} ns",
                self.avg_allocation_latency_nanos()
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_stats_basic() {
        let stats = MemoryStats::new();

        // Record allocations
        stats.record_allocation(1024);
        stats.record_allocation(2048);

        assert_eq!(stats.allocations(), 2);
        assert_eq!(stats.current_allocated(), 3072);
        assert_eq!(stats.peak_allocated(), 3072);

        // Record deallocation
        stats.record_deallocation(1024);

        assert_eq!(stats.deallocations(), 1);
        assert_eq!(stats.current_allocated(), 2048);
        assert_eq!(stats.peak_allocated(), 3072);
    }

    #[test]
    fn test_metrics_snapshot() {
        let stats = MemoryStats::new();

        stats.record_allocation(1024);
        stats.record_hit();
        stats.record_miss();

        let metrics = stats.metrics();

        assert_eq!(metrics.allocations, 1);
        assert_eq!(metrics.current_allocated, 1024);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.hit_rate, 0.5);
    }

    #[test]
    fn test_fragmentation_calculation() {
        let mut metrics = MemoryMetrics::default();
        metrics.current_allocated = 500;
        metrics.peak_allocated = 1000;

        assert!((metrics.fragmentation_ratio() - 0.5).abs() < 0.001);
    }
}
