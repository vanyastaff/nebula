//! Statistics tracking for arena allocators

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};


/// Statistics for arena allocators
#[derive(Debug)]
pub struct ArenaStats {
    // Memory statistics
    bytes_allocated: AtomicUsize,
    bytes_used: AtomicUsize,
    bytes_wasted: AtomicUsize,

    // Allocation statistics
    allocations: AtomicU64,
    deallocations: AtomicU64,
    resets: AtomicU64,

    // Chunk statistics
    chunks_allocated: AtomicUsize,
    current_chunks: AtomicUsize,
    max_chunks: AtomicUsize,

    // Performance statistics
    allocation_time_ns: AtomicU64,
    reset_time_ns: AtomicU64,

    // Created time for uptime calculation
    created_at: Instant,
}

impl Default for ArenaStats {
    fn default() -> Self {
        Self {
            bytes_allocated: AtomicUsize::new(0),
            bytes_used: AtomicUsize::new(0),
            bytes_wasted: AtomicUsize::new(0),
            allocations: AtomicU64::new(0),
            deallocations: AtomicU64::new(0),
            resets: AtomicU64::new(0),
            chunks_allocated: AtomicUsize::new(0),
            current_chunks: AtomicUsize::new(0),
            max_chunks: AtomicUsize::new(0),
            allocation_time_ns: AtomicU64::new(0),
            reset_time_ns: AtomicU64::new(0),
            created_at: Instant::now(),
        }
    }
}

impl ArenaStats {
    /// Creates a new ArenaStats instance
    pub fn new() -> Self {
        Self::default()
    }

    // Getters
    pub fn bytes_allocated(&self) -> usize {
        self.bytes_allocated.load(Ordering::Relaxed)
    }

    pub fn bytes_used(&self) -> usize {
        self.bytes_used.load(Ordering::Relaxed)
    }

    pub fn bytes_wasted(&self) -> usize {
        self.bytes_wasted.load(Ordering::Relaxed)
    }

    pub fn bytes_available(&self) -> usize {
        self.bytes_allocated().saturating_sub(self.bytes_used())
    }

    pub fn allocations(&self) -> u64 {
        self.allocations.load(Ordering::Relaxed)
    }

    pub fn deallocations(&self) -> u64 {
        self.deallocations.load(Ordering::Relaxed)
    }

    pub fn resets(&self) -> u64 {
        self.resets.load(Ordering::Relaxed)
    }

    pub fn chunks_allocated(&self) -> usize {
        self.chunks_allocated.load(Ordering::Relaxed)
    }

    pub fn current_chunks(&self) -> usize {
        self.current_chunks.load(Ordering::Relaxed)
    }

    pub fn max_chunks(&self) -> usize {
        self.max_chunks.load(Ordering::Relaxed)
    }

    /// Calculates memory fragmentation ratio (0..1)
    pub fn fragmentation_ratio(&self) -> f64 {
        let allocated = self.bytes_allocated() as f64;
        if allocated == 0.0 {
            0.0
        } else {
            (self.bytes_available() as f64) / allocated
        }
    }

    /// Calculates memory utilization ratio (0..1)
    pub fn utilization_ratio(&self) -> f64 {
        let allocated = self.bytes_allocated() as f64;
        if allocated == 0.0 {
            0.0
        } else {
            (self.bytes_used() as f64) / allocated
        }
    }

    /// Calculates average allocation size in bytes
    pub fn average_allocation_size(&self) -> f64 {
        let allocations = self.allocations() as f64;
        if allocations == 0.0 {
            0.0
        } else {
            self.bytes_used() as f64 / allocations
        }
    }

    /// Calculates average allocation time
    pub fn average_allocation_time(&self) -> Duration {
        let allocations = self.allocations();
        if allocations == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(self.allocation_time_ns.load(Ordering::Relaxed) / allocations)
        }
    }

    /// Calculates average reset time
    pub fn average_reset_time(&self) -> Duration {
        let resets = self.resets();
        if resets == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(self.reset_time_ns.load(Ordering::Relaxed) / resets)
        }
    }

    /// Returns uptime since creation
    pub fn uptime(&self) -> Duration {
        self.created_at.elapsed()
    }

    // Internal update methods
    pub(crate) fn record_allocation(&self, bytes: usize, time_ns: u64) {
        self.bytes_used.fetch_add(bytes, Ordering::Relaxed);
        self.allocations.fetch_add(1, Ordering::Relaxed);
        self.allocation_time_ns.fetch_add(time_ns, Ordering::Relaxed);
    }

    pub(crate) fn record_deallocation(&self, bytes: usize) {
        self.bytes_used.fetch_sub(bytes, Ordering::Relaxed);
        self.deallocations.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_chunk_allocation(&self, bytes: usize) {
        self.bytes_allocated.fetch_add(bytes, Ordering::Relaxed);
        self.chunks_allocated.fetch_add(1, Ordering::Relaxed);

        let prev_chunks = self.current_chunks.fetch_add(1, Ordering::Relaxed);
        self.max_chunks.fetch_max(prev_chunks + 1, Ordering::Relaxed);
    }

    pub(crate) fn record_chunk_deallocation(&self, bytes: usize) {
        self.bytes_allocated.fetch_sub(bytes, Ordering::Relaxed);
        self.current_chunks.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn record_reset(&self, time_ns: u64) {
        self.bytes_used.store(0, Ordering::Relaxed);
        self.resets.fetch_add(1, Ordering::Relaxed);
        self.reset_time_ns.fetch_add(time_ns, Ordering::Relaxed);
    }

    pub(crate) fn record_waste(&self, bytes: usize) {
        self.bytes_wasted.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Merges stats from another instance (for thread-local aggregation)
    pub fn merge(&self, other: &ArenaStats) {
        self.bytes_allocated.fetch_add(other.bytes_allocated(), Ordering::Relaxed);
        self.bytes_used.fetch_add(other.bytes_used(), Ordering::Relaxed);
        self.bytes_wasted.fetch_add(other.bytes_wasted(), Ordering::Relaxed);

        self.allocations.fetch_add(other.allocations(), Ordering::Relaxed);
        self.deallocations.fetch_add(other.deallocations(), Ordering::Relaxed);
        self.resets.fetch_add(other.resets(), Ordering::Relaxed);

        self.chunks_allocated.fetch_add(other.chunks_allocated(), Ordering::Relaxed);
        self.current_chunks.fetch_add(other.current_chunks(), Ordering::Relaxed);
        self.max_chunks.fetch_max(other.max_chunks(), Ordering::Relaxed);

        self.allocation_time_ns
            .fetch_add(other.allocation_time_ns.load(Ordering::Relaxed), Ordering::Relaxed);
        self.reset_time_ns
            .fetch_add(other.reset_time_ns.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Creates a snapshot of current statistics
    pub fn snapshot(&self) -> ArenaStatsSnapshot {
        ArenaStatsSnapshot {
            bytes_allocated: self.bytes_allocated(),
            bytes_used: self.bytes_used(),
            bytes_wasted: self.bytes_wasted(),
            bytes_available: self.bytes_available(),
            allocations: self.allocations(),
            deallocations: self.deallocations(),
            resets: self.resets(),
            chunks_allocated: self.chunks_allocated(),
            current_chunks: self.current_chunks(),
            max_chunks: self.max_chunks(),
            fragmentation_ratio: self.fragmentation_ratio(),
            utilization_ratio: self.utilization_ratio(),
            average_allocation_size: self.average_allocation_size(),
            average_allocation_time: self.average_allocation_time(),
            average_reset_time: self.average_reset_time(),
            uptime: self.uptime(),
        }
    }
}

/// Immutable snapshot of arena statistics
#[derive(Debug, Clone)]
pub struct ArenaStatsSnapshot {
    pub bytes_allocated: usize,
    pub bytes_used: usize,
    pub bytes_wasted: usize,
    pub bytes_available: usize,
    pub allocations: u64,
    pub deallocations: u64,
    pub resets: u64,
    pub chunks_allocated: usize,
    pub current_chunks: usize,
    pub max_chunks: usize,
    pub fragmentation_ratio: f64,
    pub utilization_ratio: f64,
    pub average_allocation_size: f64,
    pub average_allocation_time: Duration,
    pub average_reset_time: Duration,
    pub uptime: Duration,
}

impl std::fmt::Display for ArenaStatsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Arena Statistics:")?;
        writeln!(f, "  Memory:")?;
        writeln!(f, "    Allocated: {} bytes", self.bytes_allocated)?;
        writeln!(f, "    Used: {} bytes", self.bytes_used)?;
        writeln!(f, "    Available: {} bytes", self.bytes_available)?;
        writeln!(f, "    Wasted: {} bytes", self.bytes_wasted)?;
        writeln!(f, "  Efficiency:")?;
        writeln!(f, "    Utilization: {:.1}%", self.utilization_ratio * 100.0)?;
        writeln!(f, "    Fragmentation: {:.1}%", self.fragmentation_ratio * 100.0)?;
        writeln!(f, "  Operations:")?;
        writeln!(f, "    Allocations: {}", self.allocations)?;
        writeln!(f, "    Deallocations: {}", self.deallocations)?;
        writeln!(f, "    Resets: {}", self.resets)?;
        writeln!(f, "    Avg allocation size: {:.1} bytes", self.average_allocation_size)?;
        writeln!(f, "    Avg allocation time: {:?}", self.average_allocation_time)?;
        writeln!(f, "    Avg reset time: {:?}", self.average_reset_time)?;
        writeln!(f, "  Chunks:")?;
        writeln!(f, "    Total allocated: {}", self.chunks_allocated)?;
        writeln!(f, "    Currently active: {}", self.current_chunks)?;
        writeln!(f, "    Maximum active: {}", self.max_chunks)?;
        writeln!(f, "  Uptime: {:?}", self.uptime)?;
        Ok(())
    }
}

impl From<ArenaStatsSnapshot> for ArenaStats {
    fn from(snapshot: ArenaStatsSnapshot) -> Self {
        let stats = ArenaStats::new();
        stats.bytes_allocated.store(snapshot.bytes_allocated, Ordering::Relaxed);
        stats.bytes_used.store(snapshot.bytes_used, Ordering::Relaxed);
        stats.bytes_wasted.store(snapshot.bytes_wasted, Ordering::Relaxed);
        stats.allocations.store(snapshot.allocations, Ordering::Relaxed);
        stats.deallocations.store(snapshot.deallocations, Ordering::Relaxed);
        stats.resets.store(snapshot.resets, Ordering::Relaxed);
        stats.chunks_allocated.store(snapshot.chunks_allocated, Ordering::Relaxed);
        stats.current_chunks.store(snapshot.current_chunks, Ordering::Relaxed);
        stats.max_chunks.store(snapshot.max_chunks, Ordering::Relaxed);
        // Note: Time values are not restored from snapshot
        stats
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn test_initial_state() {
        let stats = ArenaStats::new();
        assert_eq!(stats.bytes_allocated(), 0);
        assert_eq!(stats.allocations(), 0);
        assert_eq!(stats.resets(), 0);
    }

    #[test]
    fn test_allocation_tracking() {
        let stats = ArenaStats::new();
        stats.record_chunk_allocation(1024);
        stats.record_allocation(128, 100);

        assert_eq!(stats.bytes_allocated(), 1024);
        assert_eq!(stats.bytes_used(), 128);
        assert_eq!(stats.allocations(), 1);
    }

    #[test]
    fn test_reset_behavior() {
        let stats = ArenaStats::new();
        stats.record_chunk_allocation(2048);
        stats.record_allocation(512, 200);
        stats.record_reset(150);

        assert_eq!(stats.bytes_used(), 0);
        assert_eq!(stats.resets(), 1);
    }

    #[test]
    fn test_utilization_calculation() {
        let stats = ArenaStats::new();
        stats.record_chunk_allocation(1000);
        stats.record_allocation(750, 50);

        assert!((stats.utilization_ratio() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_thread_safety() {
        let stats = std::sync::Arc::new(ArenaStats::new());
        let mut handles = vec![];

        for _ in 0..4 {
            let stats = stats.clone();
            handles.push(thread::spawn(move || {
                stats.record_chunk_allocation(512);
                stats.record_allocation(64, 10);
                stats.record_deallocation(32);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(stats.bytes_allocated(), 512 * 4);
        assert_eq!(stats.bytes_used(), (64 - 32) * 4);
        assert_eq!(stats.allocations(), 4);
    }

    #[test]
    fn test_snapshot_consistency() {
        let stats = ArenaStats::new();
        stats.record_chunk_allocation(4096);
        stats.record_allocation(1024, 100);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.bytes_allocated, stats.bytes_allocated());
        assert_eq!(snapshot.bytes_used, stats.bytes_used());
    }
}
