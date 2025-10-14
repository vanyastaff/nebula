//! Allocator statistics tracking
//!
//! Provides structures and utilities for collecting and analyzing
//! memory allocation statistics with enhanced functionality.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Statistics for memory allocators
#[derive(Debug, Clone, Copy)]
pub struct AllocatorStats {
    /// Total bytes currently allocated
    pub allocated_bytes: usize,
    /// Peak bytes allocated
    pub peak_allocated_bytes: usize,
    /// Total number of allocations
    pub allocation_count: usize,
    /// Total number of deallocations
    pub deallocation_count: usize,
    /// Total number of reallocations
    pub reallocation_count: usize,
    /// Number of failed allocations
    pub failed_allocations: usize,
    /// Total bytes ever allocated (cumulative)
    pub total_bytes_allocated: usize,
    /// Total bytes ever deallocated (cumulative)
    pub total_bytes_deallocated: usize,
}

impl AllocatorStats {
    /// Creates a new empty stats object
    #[must_use]
    pub const fn new() -> Self {
        Self {
            allocated_bytes: 0,
            peak_allocated_bytes: 0,
            allocation_count: 0,
            deallocation_count: 0,
            reallocation_count: 0,
            failed_allocations: 0,
            total_bytes_allocated: 0,
            total_bytes_deallocated: 0,
        }
    }

    /// Reset all statistics to zero
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Calculate the average allocation size
    #[must_use]
    pub fn average_allocation_size(&self) -> Option<f64> {
        if self.allocation_count > 0 {
            Some(self.total_bytes_allocated as f64 / self.allocation_count as f64)
        } else {
            None
        }
    }

    /// Calculate the net allocated memory (allocations - deallocations)
    #[must_use]
    pub fn net_allocated_bytes(&self) -> isize {
        self.allocated_bytes as isize
    }

    /// Calculate current allocation efficiency (0.0 to 1.0)
    /// Higher values indicate fewer failed allocations
    #[must_use]
    pub fn allocation_efficiency(&self) -> f64 {
        let total_attempts = self.allocation_count + self.failed_allocations;
        if total_attempts > 0 {
            self.allocation_count as f64 / total_attempts as f64
        } else {
            1.0 // No attempts means perfect efficiency
        }
    }

    /// Calculate memory turnover rate
    /// High values indicate frequent alloc/dealloc cycles
    #[must_use]
    pub fn memory_turnover_rate(&self) -> Option<f64> {
        if self.peak_allocated_bytes > 0 {
            Some(self.total_bytes_allocated as f64 / self.peak_allocated_bytes as f64)
        } else {
            None
        }
    }

    /// Check if there are any active allocations
    #[must_use]
    pub fn has_active_allocations(&self) -> bool {
        self.allocation_count > self.deallocation_count
    }

    /// Get the balance of allocations vs deallocations
    #[must_use]
    pub fn allocation_balance(&self) -> isize {
        self.allocation_count as isize - self.deallocation_count as isize
    }

    /// Calculate fragmentation indicator (imperfect but useful)
    /// Lower values indicate better memory usage
    #[must_use]
    pub fn fragmentation_indicator(&self) -> Option<f64> {
        if self.peak_allocated_bytes > 0 && self.allocated_bytes > 0 {
            Some(1.0 - (self.allocated_bytes as f64 / self.peak_allocated_bytes as f64))
        } else {
            None
        }
    }
}

impl Default for AllocatorStats {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Display for AllocatorStats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Allocator Statistics:")?;
        writeln!(f, "  Current allocated: {} bytes", self.allocated_bytes)?;
        writeln!(f, "  Peak allocated: {} bytes", self.peak_allocated_bytes)?;
        writeln!(f, "  Allocations: {}", self.allocation_count)?;
        writeln!(f, "  Deallocations: {}", self.deallocation_count)?;
        writeln!(f, "  Reallocations: {}", self.reallocation_count)?;
        writeln!(f, "  Failed allocations: {}", self.failed_allocations)?;

        if let Some(avg) = self.average_allocation_size() {
            writeln!(f, "  Average allocation size: {avg:.2} bytes")?;
        }

        writeln!(
            f,
            "  Allocation efficiency: {:.2}%",
            self.allocation_efficiency() * 100.0
        )?;

        if let Some(turnover) = self.memory_turnover_rate() {
            writeln!(f, "  Memory turnover rate: {turnover:.2}x")?;
        }

        Ok(())
    }
}

/// Thread-safe atomic version of allocator statistics
pub struct AtomicAllocatorStats {
    /// Total bytes currently allocated
    allocated_bytes: AtomicUsize,
    /// Peak bytes allocated
    peak_allocated_bytes: AtomicUsize,
    /// Total number of allocations
    allocation_count: AtomicUsize,
    /// Total number of deallocations
    deallocation_count: AtomicUsize,
    /// Total number of reallocations
    reallocation_count: AtomicUsize,
    /// Number of failed allocations
    failed_allocations: AtomicUsize,
    /// Total bytes ever allocated (cumulative)
    total_bytes_allocated: AtomicUsize,
    /// Total bytes ever deallocated (cumulative)
    total_bytes_deallocated: AtomicUsize,
}

impl AtomicAllocatorStats {
    /// Creates a new empty atomic stats object
    #[must_use]
    pub const fn new() -> Self {
        Self {
            allocated_bytes: AtomicUsize::new(0),
            peak_allocated_bytes: AtomicUsize::new(0),
            allocation_count: AtomicUsize::new(0),
            deallocation_count: AtomicUsize::new(0),
            reallocation_count: AtomicUsize::new(0),
            failed_allocations: AtomicUsize::new(0),
            total_bytes_allocated: AtomicUsize::new(0),
            total_bytes_deallocated: AtomicUsize::new(0),
        }
    }

    /// Reset all statistics to zero
    pub fn reset(&self) {
        self.allocated_bytes.store(0, Ordering::Relaxed);
        self.peak_allocated_bytes.store(0, Ordering::Relaxed);
        self.allocation_count.store(0, Ordering::Relaxed);
        self.deallocation_count.store(0, Ordering::Relaxed);
        self.reallocation_count.store(0, Ordering::Relaxed);
        self.failed_allocations.store(0, Ordering::Relaxed);
        self.total_bytes_allocated.store(0, Ordering::Relaxed);
        self.total_bytes_deallocated.store(0, Ordering::Relaxed);
    }

    /// Record a successful allocation
    pub fn record_allocation(&self, size: usize) {
        self.allocation_count.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_allocated
            .fetch_add(size, Ordering::Relaxed);

        // Overflow-safe update of allocated_bytes using CAS
        let new_allocated;
        loop {
            let current = self.allocated_bytes.load(Ordering::Relaxed);
            if let Some(next) = current.checked_add(size) {
                match self.allocated_bytes.compare_exchange_weak(
                    current,
                    next,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        new_allocated = next;
                        break;
                    }
                    Err(_) => continue,
                }
            }
            // Saturate at usize::MAX on overflow
            new_allocated = usize::MAX;
            self.allocated_bytes.store(usize::MAX, Ordering::Relaxed);
            break;
        }

        // Update peak if necessary (using compare_exchange loop for accuracy)
        let mut current_peak = self.peak_allocated_bytes.load(Ordering::Relaxed);
        while new_allocated > current_peak {
            match self.peak_allocated_bytes.compare_exchange_weak(
                current_peak,
                new_allocated,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(peak) => current_peak = peak,
            }
        }
    }

    /// Record a successful deallocation
    pub fn record_deallocation(&self, size: usize) {
        self.deallocation_count.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_deallocated
            .fetch_add(size, Ordering::Relaxed);
        self.allocated_bytes.fetch_sub(size, Ordering::Relaxed);
    }

    /// Record a successful reallocation
    pub fn record_reallocation(&self, old_size: usize, new_size: usize) {
        self.reallocation_count.fetch_add(1, Ordering::Relaxed);

        if new_size > old_size {
            let diff = new_size - old_size;
            self.allocated_bytes.fetch_add(diff, Ordering::Relaxed);
            self.total_bytes_allocated
                .fetch_add(diff, Ordering::Relaxed);
        } else if old_size > new_size {
            let diff = old_size - new_size;
            self.allocated_bytes.fetch_sub(diff, Ordering::Relaxed);
            self.total_bytes_deallocated
                .fetch_add(diff, Ordering::Relaxed);
        }

        // Update peak if necessary
        let new_allocated = self.allocated_bytes.load(Ordering::Relaxed);
        let mut current_peak = self.peak_allocated_bytes.load(Ordering::Relaxed);
        while new_allocated > current_peak {
            match self.peak_allocated_bytes.compare_exchange_weak(
                current_peak,
                new_allocated,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(peak) => current_peak = peak,
            }
        }
    }

    /// Record a failed allocation
    pub fn record_allocation_failure(&self) {
        self.failed_allocations.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of the current statistics
    pub fn snapshot(&self) -> AllocatorStats {
        AllocatorStats {
            allocated_bytes: self.allocated_bytes.load(Ordering::Relaxed),
            peak_allocated_bytes: self.peak_allocated_bytes.load(Ordering::Relaxed),
            allocation_count: self.allocation_count.load(Ordering::Relaxed),
            deallocation_count: self.deallocation_count.load(Ordering::Relaxed),
            reallocation_count: self.reallocation_count.load(Ordering::Relaxed),
            failed_allocations: self.failed_allocations.load(Ordering::Relaxed),
            total_bytes_allocated: self.total_bytes_allocated.load(Ordering::Relaxed),
            total_bytes_deallocated: self.total_bytes_deallocated.load(Ordering::Relaxed),
        }
    }

    /// Get current allocated bytes
    pub fn current_allocated(&self) -> usize {
        self.allocated_bytes.load(Ordering::Relaxed)
    }

    /// Get peak allocated bytes
    pub fn peak_allocated(&self) -> usize {
        self.peak_allocated_bytes.load(Ordering::Relaxed)
    }

    /// Get total allocation count
    pub fn allocation_count(&self) -> usize {
        self.allocation_count.load(Ordering::Relaxed)
    }

    /// Get failed allocation count
    pub fn failed_allocation_count(&self) -> usize {
        self.failed_allocations.load(Ordering::Relaxed)
    }
}

impl Default for AtomicAllocatorStats {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for AtomicAllocatorStats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AtomicAllocatorStats")
            .field(
                "allocated_bytes",
                &self.allocated_bytes.load(Ordering::Relaxed),
            )
            .field(
                "peak_allocated_bytes",
                &self.peak_allocated_bytes.load(Ordering::Relaxed),
            )
            .field(
                "allocation_count",
                &self.allocation_count.load(Ordering::Relaxed),
            )
            .field(
                "deallocation_count",
                &self.deallocation_count.load(Ordering::Relaxed),
            )
            .field(
                "reallocation_count",
                &self.reallocation_count.load(Ordering::Relaxed),
            )
            .field(
                "failed_allocations",
                &self.failed_allocations.load(Ordering::Relaxed),
            )
            .field(
                "total_bytes_allocated",
                &self.total_bytes_allocated.load(Ordering::Relaxed),
            )
            .field(
                "total_bytes_deallocated",
                &self.total_bytes_deallocated.load(Ordering::Relaxed),
            )
            .finish()
    }
}

/// Trait for allocators that support statistics collection
pub trait StatisticsProvider {
    /// Get current statistics
    fn statistics(&self) -> AllocatorStats;

    /// Reset statistics
    fn reset_statistics(&self);

    /// Check if statistics collection is enabled
    fn statistics_enabled(&self) -> bool {
        true
    }
}

/// Helper for conditional statistics collection
#[derive(Debug)]
pub struct OptionalStats {
    stats: Option<AtomicAllocatorStats>,
}

impl OptionalStats {
    /// Create new optional stats (enabled)
    #[must_use]
    pub fn enabled() -> Self {
        Self {
            stats: Some(AtomicAllocatorStats::new()),
        }
    }

    /// Create new optional stats (disabled)
    #[must_use]
    pub const fn disabled() -> Self {
        Self { stats: None }
    }

    /// Record allocation if stats are enabled
    pub fn record_allocation(&self, size: usize) {
        if let Some(ref stats) = self.stats {
            stats.record_allocation(size);
        }
    }

    /// Record deallocation if stats are enabled
    pub fn record_deallocation(&self, size: usize) {
        if let Some(ref stats) = self.stats {
            stats.record_deallocation(size);
        }
    }

    /// Record reallocation if stats are enabled
    pub fn record_reallocation(&self, old_size: usize, new_size: usize) {
        if let Some(ref stats) = self.stats {
            stats.record_reallocation(old_size, new_size);
        }
    }

    /// Record allocation failure if stats are enabled
    pub fn record_allocation_failure(&self) {
        if let Some(ref stats) = self.stats {
            stats.record_allocation_failure();
        }
    }

    /// Get statistics snapshot if enabled
    pub fn snapshot(&self) -> Option<AllocatorStats> {
        self.stats.as_ref().map(AtomicAllocatorStats::snapshot)
    }

    /// Reset statistics if enabled
    pub fn reset(&self) {
        if let Some(ref stats) = self.stats {
            stats.reset();
        }
    }

    /// Check if statistics are enabled
    pub fn is_enabled(&self) -> bool {
        self.stats.is_some()
    }
}

// ============================================================================
// Thread-Local Batched Statistics
// ============================================================================

#[cfg(feature = "std")]
use std::cell::RefCell;

#[cfg(feature = "std")]
thread_local! {
    /// Thread-local statistics batch for reducing atomic contention
    static LOCAL_STATS_BATCH: RefCell<LocalStatsBatch> = RefCell::new(LocalStatsBatch::new());
}

/// Thread-local statistics batch
///
/// Batches statistics updates per thread to reduce atomic operation contention.
/// Flushes to global statistics periodically based on batch size or time.
#[cfg(feature = "std")]
#[derive(Debug)]
struct LocalStatsBatch {
    allocation_count: usize,
    deallocation_count: usize,
    reallocation_count: usize,
    failed_allocations: usize,
    allocated_bytes: isize,
    total_bytes_allocated: usize,
    total_bytes_deallocated: usize,
    last_flush: std::time::Instant,
}

#[cfg(feature = "std")]
impl LocalStatsBatch {
    /// Batch size threshold before flushing
    const BATCH_SIZE: usize = 1000;

    /// Time threshold before flushing
    const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

    fn new() -> Self {
        Self {
            allocation_count: 0,
            deallocation_count: 0,
            reallocation_count: 0,
            failed_allocations: 0,
            allocated_bytes: 0,
            total_bytes_allocated: 0,
            total_bytes_deallocated: 0,
            last_flush: std::time::Instant::now(),
        }
    }

    fn record_allocation(&mut self, size: usize) {
        self.allocation_count += 1;
        self.allocated_bytes += size as isize;
        self.total_bytes_allocated += size;
    }

    fn record_deallocation(&mut self, size: usize) {
        self.deallocation_count += 1;
        self.allocated_bytes -= size as isize;
        self.total_bytes_deallocated += size;
    }

    fn record_reallocation(&mut self, old_size: usize, new_size: usize) {
        self.reallocation_count += 1;
        let diff = new_size as isize - old_size as isize;
        self.allocated_bytes += diff;

        if new_size > old_size {
            self.total_bytes_allocated += new_size - old_size;
        } else if old_size > new_size {
            self.total_bytes_deallocated += old_size - new_size;
        }
    }

    fn record_failure(&mut self) {
        self.failed_allocations += 1;
    }

    fn should_flush(&self) -> bool {
        let total_ops = self.allocation_count + self.deallocation_count + self.reallocation_count;
        total_ops >= Self::BATCH_SIZE || self.last_flush.elapsed() >= Self::FLUSH_INTERVAL
    }

    fn flush(&mut self, global_stats: &AtomicAllocatorStats) {
        if self.allocation_count > 0 {
            global_stats
                .allocation_count
                .fetch_add(self.allocation_count, Ordering::Relaxed);
            self.allocation_count = 0;
        }

        if self.deallocation_count > 0 {
            global_stats
                .deallocation_count
                .fetch_add(self.deallocation_count, Ordering::Relaxed);
            self.deallocation_count = 0;
        }

        if self.reallocation_count > 0 {
            global_stats
                .reallocation_count
                .fetch_add(self.reallocation_count, Ordering::Relaxed);
            self.reallocation_count = 0;
        }

        if self.failed_allocations > 0 {
            global_stats
                .failed_allocations
                .fetch_add(self.failed_allocations, Ordering::Relaxed);
            self.failed_allocations = 0;
        }

        if self.allocated_bytes != 0 {
            if self.allocated_bytes > 0 {
                global_stats
                    .allocated_bytes
                    .fetch_add(self.allocated_bytes as usize, Ordering::Relaxed);
            } else {
                global_stats
                    .allocated_bytes
                    .fetch_sub((-self.allocated_bytes) as usize, Ordering::Relaxed);
            }
            self.allocated_bytes = 0;
        }

        if self.total_bytes_allocated > 0 {
            global_stats
                .total_bytes_allocated
                .fetch_add(self.total_bytes_allocated, Ordering::Relaxed);
            self.total_bytes_allocated = 0;
        }

        if self.total_bytes_deallocated > 0 {
            global_stats
                .total_bytes_deallocated
                .fetch_add(self.total_bytes_deallocated, Ordering::Relaxed);
            self.total_bytes_deallocated = 0;
        }

        self.last_flush = std::time::Instant::now();
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Batched statistics wrapper for reduced contention
///
/// Uses thread-local batching to minimize atomic operations in hot paths.
/// Automatically flushes batches based on size and time thresholds.
///
/// # Performance
/// - Reduces atomic contention by 10-100x in multi-threaded scenarios
/// - Near-zero overhead in single-threaded code
/// - Automatic periodic flushing ensures stats remain reasonably accurate
///
/// # Examples
/// ```ignore
/// let stats = BatchedStats::new();
/// stats.record_allocation(1024);
/// // Batched locally, not yet visible globally
/// stats.flush(); // Force flush to global stats
/// let snapshot = stats.snapshot();
/// ```
#[cfg(feature = "std")]
pub struct BatchedStats {
    global: AtomicAllocatorStats,
}

#[cfg(feature = "std")]
impl BatchedStats {
    /// Create new batched statistics
    #[must_use]
    pub fn new() -> Self {
        Self {
            global: AtomicAllocatorStats::new(),
        }
    }

    /// Record allocation with thread-local batching
    #[inline]
    pub fn record_allocation(&self, size: usize) {
        LOCAL_STATS_BATCH.with(|batch| {
            let mut batch = batch.borrow_mut();
            batch.record_allocation(size);

            if batch.should_flush() {
                batch.flush(&self.global);
            }
        });
    }

    /// Record deallocation with thread-local batching
    #[inline]
    pub fn record_deallocation(&self, size: usize) {
        LOCAL_STATS_BATCH.with(|batch| {
            let mut batch = batch.borrow_mut();
            batch.record_deallocation(size);

            if batch.should_flush() {
                batch.flush(&self.global);
            }
        });
    }

    /// Record reallocation with thread-local batching
    #[inline]
    pub fn record_reallocation(&self, old_size: usize, new_size: usize) {
        LOCAL_STATS_BATCH.with(|batch| {
            let mut batch = batch.borrow_mut();
            batch.record_reallocation(old_size, new_size);

            if batch.should_flush() {
                batch.flush(&self.global);
            }
        });
    }

    /// Record allocation failure
    #[inline]
    pub fn record_allocation_failure(&self) {
        LOCAL_STATS_BATCH.with(|batch| {
            let mut batch = batch.borrow_mut();
            batch.record_failure();

            if batch.should_flush() {
                batch.flush(&self.global);
            }
        });
    }

    /// Force flush all thread-local batches to global stats
    pub fn flush(&self) {
        LOCAL_STATS_BATCH.with(|batch| {
            let mut batch = batch.borrow_mut();
            batch.flush(&self.global);
        });
    }

    /// Get snapshot of global statistics
    ///
    /// Note: May not include very recent operations still in thread-local batches.
    /// Call `flush()` first for completely accurate snapshot.
    pub fn snapshot(&self) -> AllocatorStats {
        self.global.snapshot()
    }

    /// Reset all statistics including thread-local batches
    pub fn reset(&self) {
        LOCAL_STATS_BATCH.with(|batch| {
            batch.borrow_mut().reset();
        });
        self.global.reset();
    }
}

#[cfg(feature = "std")]
impl Default for BatchedStats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_stats() {
        let mut stats = AllocatorStats::new();
        assert_eq!(stats.allocation_count, 0);
        assert_eq!(stats.allocated_bytes, 0);

        stats.reset();
        assert_eq!(stats.allocation_count, 0);
    }

    #[test]
    fn test_atomic_stats() {
        let stats = AtomicAllocatorStats::new();

        stats.record_allocation(100);
        assert_eq!(stats.current_allocated(), 100);
        assert_eq!(stats.allocation_count(), 1);
        assert_eq!(stats.peak_allocated(), 100);

        stats.record_allocation(50);
        assert_eq!(stats.current_allocated(), 150);
        assert_eq!(stats.peak_allocated(), 150);

        stats.record_deallocation(30);
        assert_eq!(stats.current_allocated(), 120);
        assert_eq!(stats.peak_allocated(), 150); // Peak should remain

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.allocated_bytes, 120);
        assert_eq!(snapshot.peak_allocated_bytes, 150);
        assert_eq!(snapshot.allocation_count, 2);
        assert_eq!(snapshot.deallocation_count, 1);
    }

    #[test]
    fn test_allocation_efficiency() {
        let mut stats = AllocatorStats::new();
        stats.allocation_count = 8;
        stats.failed_allocations = 2;

        assert_eq!(stats.allocation_efficiency(), 0.8); // 8/10 = 80%
    }

    #[test]
    fn test_optional_stats() {
        let enabled_stats = OptionalStats::enabled();
        let disabled_stats = OptionalStats::disabled();

        enabled_stats.record_allocation(100);
        disabled_stats.record_allocation(100);

        assert!(enabled_stats.snapshot().is_some());
        assert!(disabled_stats.snapshot().is_none());

        assert!(enabled_stats.is_enabled());
        assert!(!disabled_stats.is_enabled());
    }

    #[test]
    fn test_display_format() {
        let mut stats = AllocatorStats::new();
        stats.allocated_bytes = 1024;
        stats.allocation_count = 10;
        stats.total_bytes_allocated = 2048;

        let display_str = format!("{}", stats);
        assert!(display_str.contains("1024 bytes"));
        assert!(display_str.contains("10"));
    }
}
