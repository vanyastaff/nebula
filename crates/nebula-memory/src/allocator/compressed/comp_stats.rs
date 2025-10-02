//! Compression statistics and strategies

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Compression strategy
#[derive(Debug, Clone, Copy)]
pub enum CompressionStrategy {
    /// Always compress (regardless of size)
    Always,

    /// Never compress
    Never,

    /// Compress only if size >= threshold
    Threshold(usize),

    /// Compress when memory pressure is high
    OnPressure {
        threshold: usize,
        pressure_threshold: f64,
    },

    /// Adaptive: compress based on past compression ratios
    Adaptive { min_size: usize, min_ratio: f64 },
}

impl Default for CompressionStrategy {
    fn default() -> Self {
        Self::Threshold(1024) // 1KB default threshold
    }
}

impl CompressionStrategy {
    /// Check if data should be compressed
    pub fn should_compress(&self, size: usize, stats: &CompressionStats) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Threshold(threshold) => size >= *threshold,
            Self::OnPressure {
                threshold,
                pressure_threshold,
            } => {
                if size < *threshold {
                    return false;
                }
                stats.memory_pressure() >= *pressure_threshold
            }
            Self::Adaptive {
                min_size,
                min_ratio,
            } => {
                if size < *min_size {
                    return false;
                }
                stats.average_compression_ratio() <= *min_ratio
            }
        }
    }

    /// Create threshold-based strategy
    pub fn threshold(size: usize) -> Self {
        Self::Threshold(size)
    }

    /// Create pressure-based strategy
    pub fn on_pressure(threshold: usize, pressure: f64) -> Self {
        Self::OnPressure {
            threshold,
            pressure_threshold: pressure.clamp(0.0, 1.0),
        }
    }

    /// Create adaptive strategy
    pub fn adaptive(min_size: usize, min_ratio: f64) -> Self {
        Self::Adaptive {
            min_size,
            min_ratio: min_ratio.clamp(0.0, 1.0),
        }
    }
}

/// Compression statistics
#[derive(Debug)]
pub struct CompressionStats {
    /// Total compressions performed
    total_compressions: AtomicU64,

    /// Total decompressions performed
    total_decompressions: AtomicU64,

    /// Total bytes compressed (original size)
    total_bytes_in: AtomicU64,

    /// Total bytes after compression
    total_bytes_out: AtomicU64,

    /// Total compression time (microseconds)
    total_compression_time_us: AtomicU64,

    /// Total decompression time (microseconds)
    total_decompression_time_us: AtomicU64,

    /// Current memory usage
    current_memory: AtomicUsize,

    /// Peak memory usage
    peak_memory: AtomicUsize,

    /// Total memory limit (for pressure calculation)
    memory_limit: AtomicUsize,
}

impl CompressionStats {
    /// Create new compression stats
    pub fn new() -> Self {
        Self {
            total_compressions: AtomicU64::new(0),
            total_decompressions: AtomicU64::new(0),
            total_bytes_in: AtomicU64::new(0),
            total_bytes_out: AtomicU64::new(0),
            total_compression_time_us: AtomicU64::new(0),
            total_decompression_time_us: AtomicU64::new(0),
            current_memory: AtomicUsize::new(0),
            peak_memory: AtomicUsize::new(0),
            memory_limit: AtomicUsize::new(usize::MAX),
        }
    }

    /// Create with memory limit
    pub fn with_limit(limit: usize) -> Self {
        let mut stats = Self::new();
        stats.memory_limit.store(limit, Ordering::Relaxed);
        stats
    }

    /// Record a compression operation
    pub fn record_compression(
        &self,
        original_size: usize,
        compressed_size: usize,
        duration: Duration,
    ) {
        self.total_compressions.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_in
            .fetch_add(original_size as u64, Ordering::Relaxed);
        self.total_bytes_out
            .fetch_add(compressed_size as u64, Ordering::Relaxed);
        self.total_compression_time_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    }

    /// Record a decompression operation
    pub fn record_decompression(&self, size: usize, duration: Duration) {
        self.total_decompressions.fetch_add(1, Ordering::Relaxed);
        self.total_decompression_time_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    }

    /// Update memory usage
    pub fn update_memory(&self, current: usize) {
        self.current_memory.store(current, Ordering::Relaxed);

        // Update peak if necessary
        let mut peak = self.peak_memory.load(Ordering::Relaxed);
        while current > peak {
            match self.peak_memory.compare_exchange_weak(
                peak,
                current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(new_peak) => peak = new_peak,
            }
        }
    }

    /// Get average compression ratio
    pub fn average_compression_ratio(&self) -> f64 {
        let bytes_in = self.total_bytes_in.load(Ordering::Relaxed);
        if bytes_in == 0 {
            return 1.0;
        }
        let bytes_out = self.total_bytes_out.load(Ordering::Relaxed);
        bytes_out as f64 / bytes_in as f64
    }

    /// Get space saved (bytes)
    pub fn space_saved(&self) -> u64 {
        let bytes_in = self.total_bytes_in.load(Ordering::Relaxed);
        let bytes_out = self.total_bytes_out.load(Ordering::Relaxed);
        bytes_in.saturating_sub(bytes_out)
    }

    /// Get space saved (percentage)
    pub fn space_saved_percent(&self) -> f64 {
        (1.0 - self.average_compression_ratio()) * 100.0
    }

    /// Get current memory pressure (0.0 to 1.0)
    pub fn memory_pressure(&self) -> f64 {
        let current = self.current_memory.load(Ordering::Relaxed);
        let limit = self.memory_limit.load(Ordering::Relaxed);

        if limit == usize::MAX {
            return 0.0;
        }

        (current as f64 / limit as f64).min(1.0)
    }

    /// Get total compressions
    pub fn total_compressions(&self) -> u64 {
        self.total_compressions.load(Ordering::Relaxed)
    }

    /// Get total decompressions
    pub fn total_decompressions(&self) -> u64 {
        self.total_decompressions.load(Ordering::Relaxed)
    }

    /// Get average compression time
    pub fn avg_compression_time(&self) -> Duration {
        let total = self.total_compression_time_us.load(Ordering::Relaxed);
        let count = self.total_compressions.load(Ordering::Relaxed);

        if count == 0 {
            return Duration::ZERO;
        }

        Duration::from_micros(total / count)
    }

    /// Get average decompression time
    pub fn avg_decompression_time(&self) -> Duration {
        let total = self.total_decompression_time_us.load(Ordering::Relaxed);
        let count = self.total_decompressions.load(Ordering::Relaxed);

        if count == 0 {
            return Duration::ZERO;
        }

        Duration::from_micros(total / count)
    }
}

impl Default for CompressionStats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_strategy_threshold() {
        let strategy = CompressionStrategy::threshold(1024);
        let stats = CompressionStats::new();

        assert!(!strategy.should_compress(512, &stats));
        assert!(strategy.should_compress(2048, &stats));
    }

    #[test]
    fn test_compression_stats() {
        let stats = CompressionStats::new();

        stats.record_compression(1000, 500, Duration::from_micros(100));
        stats.record_compression(2000, 1000, Duration::from_micros(200));

        assert_eq!(stats.total_compressions(), 2);
        assert_eq!(stats.space_saved(), 1500);
        assert_eq!(stats.average_compression_ratio(), 0.5);
        assert_eq!(stats.space_saved_percent(), 50.0);
    }

    #[test]
    fn test_memory_pressure() {
        let stats = CompressionStats::with_limit(10000);

        stats.update_memory(5000);
        assert_eq!(stats.memory_pressure(), 0.5);

        stats.update_memory(9000);
        assert_eq!(stats.memory_pressure(), 0.9);
    }

    #[test]
    fn test_adaptive_strategy() {
        let strategy = CompressionStrategy::adaptive(1024, 0.8);
        let stats = CompressionStats::new();

        // Record good compression ratio
        stats.record_compression(10000, 5000, Duration::from_micros(100));

        // Should compress (good ratio < 0.8)
        assert!(strategy.should_compress(2048, &stats));

        // Record bad compression ratio
        stats.record_compression(10000, 9000, Duration::from_micros(100));

        // Might not compress (average ratio getting worse)
        // This depends on the overall average
    }
}
