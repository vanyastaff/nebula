//! Storage provider metrics collection
//!
//! Foundation for Phase 8 observability - provides atomic counters for
//! tracking operation performance and error rates.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Per-provider metrics for observability
///
/// Thread-safe metrics collection using atomic operations. This is a foundation
/// for Phase 8 comprehensive observability - currently provides basic counters.
///
/// # Example
///
/// ```rust
/// use nebula_credential::providers::StorageMetrics;
/// use std::time::Duration;
///
/// let metrics = StorageMetrics::default();
///
/// // Record successful operation
/// metrics.record_operation("store", Duration::from_millis(15), true);
///
/// // Record failed operation
/// metrics.record_operation("retrieve", Duration::from_millis(100), false);
///
/// // Check metrics
/// assert_eq!(metrics.store_count(), 1);
/// assert_eq!(metrics.error_count(), 1);
/// ```
#[derive(Debug, Default)]
pub struct StorageMetrics {
    /// Total store operations
    store_count: AtomicU64,

    /// Sum of store operation latencies (milliseconds)
    store_latency_sum_ms: AtomicU64,

    /// Total retrieve operations
    retrieve_count: AtomicU64,

    /// Sum of retrieve operation latencies (milliseconds)
    retrieve_latency_sum_ms: AtomicU64,

    /// Total delete operations
    delete_count: AtomicU64,

    /// Total list operations
    list_count: AtomicU64,

    /// Total errors across all operations
    error_count: AtomicU64,

    /// Total retries attempted
    retry_count: AtomicU64,
}

impl StorageMetrics {
    /// Create new metrics instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an operation with duration and success status
    ///
    /// # Arguments
    ///
    /// * `operation` - Operation type: "store", "retrieve", "delete", "list"
    /// * `duration` - Time taken for the operation
    /// * `success` - Whether the operation succeeded
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_credential::providers::StorageMetrics;
    /// use std::time::Duration;
    ///
    /// let metrics = StorageMetrics::new();
    /// metrics.record_operation("store", Duration::from_millis(10), true);
    /// ```
    pub fn record_operation(&self, operation: &str, duration: Duration, success: bool) {
        let latency_ms = duration.as_millis() as u64;

        match operation {
            "store" => {
                self.store_count.fetch_add(1, Ordering::Relaxed);
                self.store_latency_sum_ms
                    .fetch_add(latency_ms, Ordering::Relaxed);
            }
            "retrieve" => {
                self.retrieve_count.fetch_add(1, Ordering::Relaxed);
                self.retrieve_latency_sum_ms
                    .fetch_add(latency_ms, Ordering::Relaxed);
            }
            "delete" => {
                self.delete_count.fetch_add(1, Ordering::Relaxed);
            }
            "list" => {
                self.list_count.fetch_add(1, Ordering::Relaxed);
            }
            _ => {} // Unknown operation type, ignore
        }

        if !success {
            self.error_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a retry attempt
    ///
    /// Call this each time an operation is retried due to transient failure.
    pub fn record_retry(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total store operations
    pub fn store_count(&self) -> u64 {
        self.store_count.load(Ordering::Relaxed)
    }

    /// Get total retrieve operations
    pub fn retrieve_count(&self) -> u64 {
        self.retrieve_count.load(Ordering::Relaxed)
    }

    /// Get total delete operations
    pub fn delete_count(&self) -> u64 {
        self.delete_count.load(Ordering::Relaxed)
    }

    /// Get total list operations
    pub fn list_count(&self) -> u64 {
        self.list_count.load(Ordering::Relaxed)
    }

    /// Get total errors across all operations
    pub fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    /// Get total retry attempts
    pub fn retry_count(&self) -> u64 {
        self.retry_count.load(Ordering::Relaxed)
    }

    /// Calculate average store latency in milliseconds
    ///
    /// Returns 0 if no store operations have been recorded.
    pub fn avg_store_latency_ms(&self) -> u64 {
        let count = self.store_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        self.store_latency_sum_ms.load(Ordering::Relaxed) / count
    }

    /// Calculate average retrieve latency in milliseconds
    ///
    /// Returns 0 if no retrieve operations have been recorded.
    pub fn avg_retrieve_latency_ms(&self) -> u64 {
        let count = self.retrieve_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        self.retrieve_latency_sum_ms.load(Ordering::Relaxed) / count
    }

    /// Calculate error rate (errors / total operations)
    ///
    /// Returns value between 0.0 and 1.0. Returns 0.0 if no operations recorded.
    pub fn error_rate(&self) -> f64 {
        let total = self.store_count.load(Ordering::Relaxed)
            + self.retrieve_count.load(Ordering::Relaxed)
            + self.delete_count.load(Ordering::Relaxed)
            + self.list_count.load(Ordering::Relaxed);

        if total == 0 {
            return 0.0;
        }

        self.error_count.load(Ordering::Relaxed) as f64 / total as f64
    }

    /// Reset all metrics to zero
    ///
    /// Useful for testing or when starting a new measurement period.
    pub fn reset(&self) {
        self.store_count.store(0, Ordering::Relaxed);
        self.store_latency_sum_ms.store(0, Ordering::Relaxed);
        self.retrieve_count.store(0, Ordering::Relaxed);
        self.retrieve_latency_sum_ms.store(0, Ordering::Relaxed);
        self.delete_count.store(0, Ordering::Relaxed);
        self.list_count.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        self.retry_count.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_store_operation() {
        let metrics = StorageMetrics::new();

        metrics.record_operation("store", Duration::from_millis(10), true);
        metrics.record_operation("store", Duration::from_millis(20), true);

        assert_eq!(metrics.store_count(), 2);
        assert_eq!(metrics.avg_store_latency_ms(), 15); // (10 + 20) / 2
        assert_eq!(metrics.error_count(), 0);
    }

    #[test]
    fn test_record_retrieve_operation() {
        let metrics = StorageMetrics::new();

        metrics.record_operation("retrieve", Duration::from_millis(5), true);
        metrics.record_operation("retrieve", Duration::from_millis(15), true);

        assert_eq!(metrics.retrieve_count(), 2);
        assert_eq!(metrics.avg_retrieve_latency_ms(), 10); // (5 + 15) / 2
    }

    #[test]
    fn test_record_error() {
        let metrics = StorageMetrics::new();

        metrics.record_operation("store", Duration::from_millis(10), true);
        metrics.record_operation("retrieve", Duration::from_millis(5), false);

        assert_eq!(metrics.store_count(), 1);
        assert_eq!(metrics.retrieve_count(), 1);
        assert_eq!(metrics.error_count(), 1);
        assert_eq!(metrics.error_rate(), 0.5); // 1 error / 2 total operations
    }

    #[test]
    fn test_record_retry() {
        let metrics = StorageMetrics::new();

        metrics.record_retry();
        metrics.record_retry();
        metrics.record_retry();

        assert_eq!(metrics.retry_count(), 3);
    }

    #[test]
    fn test_avg_latency_with_no_operations() {
        let metrics = StorageMetrics::new();

        assert_eq!(metrics.avg_store_latency_ms(), 0);
        assert_eq!(metrics.avg_retrieve_latency_ms(), 0);
    }

    #[test]
    fn test_error_rate_with_no_operations() {
        let metrics = StorageMetrics::new();

        assert_eq!(metrics.error_rate(), 0.0);
    }

    #[test]
    fn test_reset() {
        let metrics = StorageMetrics::new();

        metrics.record_operation("store", Duration::from_millis(10), true);
        metrics.record_operation("retrieve", Duration::from_millis(5), false);
        metrics.record_retry();

        assert_eq!(metrics.store_count(), 1);
        assert_eq!(metrics.error_count(), 1);
        assert_eq!(metrics.retry_count(), 1);

        metrics.reset();

        assert_eq!(metrics.store_count(), 0);
        assert_eq!(metrics.retrieve_count(), 0);
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.retry_count(), 0);
    }

    #[test]
    fn test_all_operation_types() {
        let metrics = StorageMetrics::new();

        metrics.record_operation("store", Duration::from_millis(10), true);
        metrics.record_operation("retrieve", Duration::from_millis(5), true);
        metrics.record_operation("delete", Duration::from_millis(3), true);
        metrics.record_operation("list", Duration::from_millis(20), true);

        assert_eq!(metrics.store_count(), 1);
        assert_eq!(metrics.retrieve_count(), 1);
        assert_eq!(metrics.delete_count(), 1);
        assert_eq!(metrics.list_count(), 1);
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.error_rate(), 0.0);
    }
}
