//! Pool statistics and latency percentiles.

use hdrhistogram::Histogram;

// ---------------------------------------------------------------------------
// LatencyHistogram (private to pool internals)
// ---------------------------------------------------------------------------

pub(super) type LatencyHistogram = Histogram<u64>;

pub(super) fn new_latency_histogram() -> LatencyHistogram {
    Histogram::<u64>::new_with_bounds(1, 60_000, 3).expect("latency histogram bounds must be valid")
}

// ---------------------------------------------------------------------------
// LatencyPercentiles
// ---------------------------------------------------------------------------

/// Acquire latency percentiles and mean (milliseconds).
#[derive(Debug, Clone)]
pub struct LatencyPercentiles {
    /// 50th percentile (median).
    pub p50_ms: u64,
    /// 95th percentile.
    pub p95_ms: u64,
    /// 99th percentile.
    pub p99_ms: u64,
    /// 99.9th percentile.
    pub p999_ms: u64,
    /// Arithmetic mean latency.
    pub mean_ms: f64,
}

// ---------------------------------------------------------------------------
// PoolStats
// ---------------------------------------------------------------------------

/// Pool statistics snapshot.
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    /// Total successful acquisitions.
    pub total_acquisitions: u64,
    /// Total releases back to pool.
    pub total_releases: u64,
    /// Current number of instances checked out.
    pub active: usize,
    /// Current number of idle instances in pool.
    pub idle: usize,
    /// Total instances ever created.
    pub created: u64,
    /// Total instances ever destroyed.
    pub destroyed: u64,
    /// Cumulative wait time across all acquisitions (milliseconds).
    pub total_wait_time_ms: u64,
    /// Maximum observed wait time for a single acquisition (milliseconds).
    pub max_wait_time_ms: u64,
    /// Number of times the pool was exhausted (acquire timed out).
    pub exhausted_count: u64,
    /// Acquire latency distribution summary.
    /// `None` when no acquisitions have been recorded yet.
    pub acquire_latency: Option<LatencyPercentiles>,
}
