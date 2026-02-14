//! Statistics tracking for object pools

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Statistics for pool operations
#[derive(Debug)]
pub struct PoolStats {
    // Basic counters
    pub(crate) gets: AtomicU64,
    pub(crate) returns: AtomicU64,
    pub(crate) creates: AtomicU64,
    pub(crate) destroys: AtomicU64,
    pub(crate) hits: AtomicU64,
    pub(crate) misses: AtomicU64,

    // Size tracking
    pub(crate) current_size: AtomicUsize,
    pub(crate) peak_size: AtomicUsize,
    pub(crate) total_created: AtomicUsize,

    // Memory tracking
    pub(crate) memory_usage: AtomicUsize,
    pub(crate) peak_memory: AtomicUsize,

    // Adaptive optimization tracking
    #[cfg(feature = "adaptive")]
    pub(crate) compression_attempts: AtomicU64,
    #[cfg(feature = "adaptive")]
    pub(crate) successful_compressions: AtomicU64,
    #[cfg(feature = "adaptive")]
    pub(crate) memory_saved: AtomicUsize,

    // Timing
    pub(crate) created_at: Instant,
    pub(crate) last_get: AtomicU64, // Stored as micros since creation
    pub(crate) last_return: AtomicU64,
}

impl Default for PoolStats {
    fn default() -> Self {
        Self {
            gets: AtomicU64::new(0),
            returns: AtomicU64::new(0),
            creates: AtomicU64::new(0),
            destroys: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            current_size: AtomicUsize::new(0),
            peak_size: AtomicUsize::new(0),
            total_created: AtomicUsize::new(0),
            memory_usage: AtomicUsize::new(0),
            peak_memory: AtomicUsize::new(0),
            created_at: Instant::now(),
            last_get: AtomicU64::new(0),
            last_return: AtomicU64::new(0),
            #[cfg(feature = "adaptive")]
            compression_attempts: AtomicU64::new(0),
            #[cfg(feature = "adaptive")]
            successful_compressions: AtomicU64::new(0),
            #[cfg(feature = "adaptive")]
            memory_saved: AtomicUsize::new(0),
        }
    }
}

impl PoolStats {
    /// Record a get operation
    pub(crate) fn record_get(&self) {
        self.gets.fetch_add(1, Ordering::Relaxed);

        let micros = self.created_at.elapsed().as_micros() as u64;
        self.last_get.store(micros, Ordering::Relaxed);
    }

    /// Record a cache hit
    pub(crate) fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss
    pub(crate) fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record object creation
    pub(crate) fn record_creation(&self) {
        self.creates.fetch_add(1, Ordering::Relaxed);
        self.total_created.fetch_add(1, Ordering::Relaxed);

        let new_size = self.current_size.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_peak_size(new_size);
    }

    /// Record object return
    pub(crate) fn record_return(&self) {
        self.returns.fetch_add(1, Ordering::Relaxed);

        let micros = self.created_at.elapsed().as_micros() as u64;
        self.last_return.store(micros, Ordering::Relaxed);
    }

    /// Record object destruction
    pub(crate) fn record_destruction(&self) {
        self.destroys.fetch_add(1, Ordering::Relaxed);
        self.current_size.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record pool clear
    pub(crate) fn record_clear(&self) {
        let cleared = self.current_size.swap(0, Ordering::Relaxed);
        self.destroys.fetch_add(cleared as u64, Ordering::Relaxed);
    }

    /// Update memory usage
    pub(crate) fn update_memory(&self, usage: usize) {
        self.memory_usage.store(usage, Ordering::Relaxed);
        self.update_peak_memory(usage);
    }

    /// Record compression attempt
    #[cfg(feature = "adaptive")]
    pub(crate) fn record_compression_attempt(
        &self,
        before_size: usize,
        after_size: usize,
        success: bool,
    ) {
        self.compression_attempts.fetch_add(1, Ordering::Relaxed);

        if success {
            self.successful_compressions.fetch_add(1, Ordering::Relaxed);

            if before_size > after_size {
                let saved = before_size - after_size;
                self.memory_saved.fetch_add(saved, Ordering::Relaxed);
            }
        }
    }

    /// Update peak size if needed
    fn update_peak_size(&self, new_size: usize) {
        let mut peak = self.peak_size.load(Ordering::Relaxed);
        while new_size > peak {
            match self.peak_size.compare_exchange_weak(
                peak,
                new_size,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => peak = current,
            }
        }
    }

    /// Update peak memory if needed
    fn update_peak_memory(&self, new_memory: usize) {
        let mut peak = self.peak_memory.load(Ordering::Relaxed);
        while new_memory > peak {
            match self.peak_memory.compare_exchange_weak(
                peak,
                new_memory,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => peak = current,
            }
        }
    }

    /// Get hit rate (0.0 - 1.0)
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let total = self.gets.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get compression success rate (0.0 - 1.0)
    #[cfg(feature = "adaptive")]
    pub fn compression_success_rate(&self) -> f64 {
        let attempts = self.compression_attempts.load(Ordering::Relaxed);
        let successes = self.successful_compressions.load(Ordering::Relaxed);

        if attempts == 0 {
            0.0
        } else {
            successes as f64 / attempts as f64
        }
    }

    /// Get total memory saved through compression
    #[cfg(feature = "adaptive")]
    pub fn total_memory_saved(&self) -> usize {
        self.memory_saved.load(Ordering::Relaxed)
    }

    /// Get current pool size
    pub fn current_size(&self) -> usize {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Get peak pool size
    pub fn peak_size(&self) -> usize {
        self.peak_size.load(Ordering::Relaxed)
    }

    /// Get total objects created
    pub fn total_created(&self) -> usize {
        self.total_created.load(Ordering::Relaxed)
    }

    /// Get total gets
    pub fn total_gets(&self) -> u64 {
        self.gets.load(Ordering::Relaxed)
    }

    /// Get total returns
    pub fn total_returns(&self) -> u64 {
        self.returns.load(Ordering::Relaxed)
    }

    /// Get uptime duration
    pub fn uptime(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Get time since last get
    pub fn time_since_last_get(&self) -> Option<Duration> {
        let last_micros = self.last_get.load(Ordering::Relaxed);
        if last_micros == 0 {
            None
        } else {
            let current_micros = self.created_at.elapsed().as_micros() as u64;
            Some(Duration::from_micros(current_micros - last_micros))
        }
    }

    /// Reset all statistics
    pub fn reset(&self) {
        self.gets.store(0, Ordering::Relaxed);
        self.returns.store(0, Ordering::Relaxed);
        self.creates.store(0, Ordering::Relaxed);
        self.destroys.store(0, Ordering::Relaxed);
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);

        #[cfg(feature = "adaptive")]
        {
            self.compression_attempts.store(0, Ordering::Relaxed);
            self.successful_compressions.store(0, Ordering::Relaxed);
            self.memory_saved.store(0, Ordering::Relaxed);
        }

        // Don't reset size/memory counters - they reflect actual state
    }
}

/// Pool statistics snapshot
#[derive(Debug, Clone)]
pub struct PoolStatsSnapshot {
    pub total_gets: u64,
    pub total_returns: u64,
    pub total_creates: u64,
    pub total_destroys: u64,
    pub hit_rate: f64,
    pub current_size: usize,
    pub peak_size: usize,
    pub memory_usage: usize,
    pub peak_memory: usize,
    #[cfg(feature = "adaptive")]
    pub compression_attempts: u64,
    #[cfg(feature = "adaptive")]
    pub compression_success_rate: f64,
    #[cfg(feature = "adaptive")]
    pub memory_saved: usize,
    pub uptime: Duration,
}

impl From<&PoolStats> for PoolStatsSnapshot {
    fn from(stats: &PoolStats) -> Self {
        Self {
            total_gets: stats.gets.load(Ordering::Relaxed),
            total_returns: stats.returns.load(Ordering::Relaxed),
            total_creates: stats.creates.load(Ordering::Relaxed),
            total_destroys: stats.destroys.load(Ordering::Relaxed),
            hit_rate: stats.hit_rate(),
            current_size: stats.current_size(),
            peak_size: stats.peak_size(),
            memory_usage: stats.memory_usage.load(Ordering::Relaxed),
            peak_memory: stats.peak_memory.load(Ordering::Relaxed),
            #[cfg(feature = "adaptive")]
            compression_attempts: stats.compression_attempts.load(Ordering::Relaxed),
            #[cfg(feature = "adaptive")]
            compression_success_rate: stats.compression_success_rate(),
            #[cfg(feature = "adaptive")]
            memory_saved: stats.memory_saved.load(Ordering::Relaxed),
            uptime: stats.uptime(),
        }
    }
}

impl core::fmt::Display for PoolStatsSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Pool Statistics:")?;
        writeln!(
            f,
            "  Gets: {} (hit rate: {:.2}%)",
            self.total_gets,
            self.hit_rate * 100.0
        )?;
        writeln!(f, "  Returns: {}", self.total_returns)?;
        writeln!(f, "  Creates: {}", self.total_creates)?;
        writeln!(f, "  Destroys: {}", self.total_destroys)?;
        writeln!(
            f,
            "  Current size: {} (peak: {})",
            self.current_size, self.peak_size
        )?;
        writeln!(
            f,
            "  Memory: {} bytes (peak: {} bytes)",
            self.memory_usage, self.peak_memory
        )?;

        writeln!(f, "  Uptime: {:?}", self.uptime)?;

        #[cfg(feature = "adaptive")]
        writeln!(f, "  Compression attempts: {}", self.compression_attempts)?;
        #[cfg(feature = "adaptive")]
        writeln!(
            f,
            "  Compression success rate: {:.2}%",
            self.compression_success_rate * 100.0
        )?;
        #[cfg(feature = "adaptive")]
        writeln!(f, "  Total memory saved: {} bytes", self.memory_saved)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_tracking() {
        let stats = PoolStats::default();

        // Record some operations
        stats.record_get();
        stats.record_hit();
        stats.record_get();
        stats.record_miss();
        stats.record_creation();

        assert_eq!(stats.total_gets(), 2);
        assert_eq!(stats.hit_rate(), 0.5);
        assert_eq!(stats.current_size(), 1);
        assert_eq!(stats.peak_size(), 1);
    }

    #[test]
    fn test_peak_tracking() {
        let stats = PoolStats::default();

        // Create objects
        for _ in 0..5 {
            stats.record_creation();
        }
        assert_eq!(stats.peak_size(), 5);

        // Destroy some
        for _ in 0..3 {
            stats.record_destruction();
        }
        assert_eq!(stats.current_size(), 2);
        assert_eq!(stats.peak_size(), 5); // Peak unchanged
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_compression_stats() {
        let stats = PoolStats::default();

        // Record compression attempts
        stats.record_compression_attempt(1000, 500, true);
        stats.record_compression_attempt(2000, 1000, true);
        stats.record_compression_attempt(3000, 3000, false); // No savings

        // Check statistics
        assert_eq!(stats.compression_attempts.load(Ordering::Relaxed), 3);
        assert_eq!(stats.successful_compressions.load(Ordering::Relaxed), 2);
        assert_eq!(stats.compression_success_rate(), 2.0 / 3.0);
        assert_eq!(stats.memory_saved.load(Ordering::Relaxed), 1500); // 500 + 1000

        let snapshot = PoolStatsSnapshot::from(&stats);
        assert_eq!(snapshot.compression_attempts, 3);
        assert_eq!(snapshot.compression_success_rate, 2.0 / 3.0);
        assert_eq!(snapshot.memory_saved, 1500);
    }
}
