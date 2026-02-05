//! Rotation Metrics Collection
//!
//! Provides metrics tracking for credential rotation operations including
//! success rates, durations, and failure tracking.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use crate::core::CredentialId;

/// Rotation metrics collector
///
/// # T096: RotationMetrics Struct
///
/// Tracks rotation performance and reliability metrics:
/// - Success/failure counters
/// - Duration histograms
/// - Error classification
/// - Success rate calculations
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::metrics::RotationMetrics;
///
/// let metrics = RotationMetrics::new();
///
/// // Record successful rotation
/// metrics.record_rotation_duration(
///     &credential_id,
///     Duration::from_secs(45),
///     true,
/// );
///
/// // Record failure
/// metrics.record_rotation_failure(&credential_id, "Timeout");
///
/// // Calculate success rate
/// let success_rate = metrics.rotation_success_rate();
/// println!("Success rate: {:.2}%", success_rate * 100.0);
/// ```
#[derive(Debug, Clone)]
pub struct RotationMetrics {
    /// Inner state protected by RwLock
    inner: Arc<RwLock<MetricsInner>>,
}

#[derive(Debug, Clone, Default)]
struct MetricsInner {
    /// Total rotation attempts
    total_rotations: u64,

    /// Successful rotations
    successful_rotations: u64,

    /// Failed rotations
    failed_rotations: u64,

    /// Rotation durations (for calculating percentiles)
    /// Limited to max_durations to prevent unbounded growth
    durations: VecDeque<Duration>,

    /// Maximum number of durations to keep
    max_durations: usize,

    /// Failure counts by error type
    failures_by_type: HashMap<String, u64>,

    /// Per-credential metrics
    per_credential: HashMap<CredentialId, CredentialMetrics>,

    /// Last rotation timestamp
    last_rotation_at: Option<DateTime<Utc>>,

    /// Last cleanup timestamp for stale metrics
    last_cleanup: DateTime<Utc>,
}

/// Per-credential rotation metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CredentialMetrics {
    /// Total rotations for this credential
    pub total: u64,

    /// Successful rotations
    pub successful: u64,

    /// Failed rotations
    pub failed: u64,

    /// Average rotation duration
    pub avg_duration: Option<Duration>,

    /// Last rotation timestamp
    pub last_rotation: Option<DateTime<Utc>>,

    /// Last rotation success status
    pub last_success: Option<bool>,
}

impl RotationMetrics {
    /// Maximum number of duration samples to keep (prevents unbounded memory growth)
    const MAX_DURATIONS: usize = 10_000;

    /// Maximum age of per-credential metrics before cleanup (days)
    const CREDENTIAL_METRICS_MAX_AGE_DAYS: i64 = 90;

    /// Create a new metrics collector
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MetricsInner {
                total_rotations: 0,
                successful_rotations: 0,
                failed_rotations: 0,
                durations: VecDeque::with_capacity(Self::MAX_DURATIONS),
                max_durations: Self::MAX_DURATIONS,
                failures_by_type: HashMap::new(),
                per_credential: HashMap::new(),
                last_rotation_at: None,
                last_cleanup: Utc::now(),
            })),
        }
    }

    /// Record rotation duration and outcome
    ///
    /// # T097: Record Rotation Duration
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential that was rotated
    /// * `duration` - How long rotation took
    /// * `success` - Whether rotation succeeded
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// metrics.record_rotation_duration(
    ///     &cred_id,
    ///     Duration::from_secs(30),
    ///     true,
    /// );
    /// ```
    pub fn record_rotation_duration(
        &self,
        credential_id: &CredentialId,
        duration: Duration,
        success: bool,
    ) {
        let mut inner = self.inner.write();
        let now = Utc::now();

        inner.total_rotations += 1;

        // Prune old durations if we've hit the limit (prevents unbounded growth)
        if inner.durations.len() >= inner.max_durations {
            inner.durations.pop_front();
        }
        inner.durations.push_back(duration);
        inner.last_rotation_at = Some(now);

        if success {
            inner.successful_rotations += 1;
        } else {
            inner.failed_rotations += 1;
        }

        // Periodically clean up stale per-credential metrics
        if (now - inner.last_cleanup).num_days() >= 1 {
            inner.per_credential.retain(|_, metrics| {
                metrics
                    .last_rotation
                    .map(|last_rot| {
                        (now - last_rot).num_days() < Self::CREDENTIAL_METRICS_MAX_AGE_DAYS
                    })
                    .unwrap_or(false)
            });
            inner.last_cleanup = now;
        }

        // Calculate average duration before mutable borrow
        let total_duration: Duration = inner.durations.iter().sum();
        let duration_count = inner.durations.len() as u32;
        let avg_duration = Some(total_duration / duration_count);

        // Update per-credential metrics
        let cred_metrics = inner
            .per_credential
            .entry(credential_id.clone())
            .or_default();

        cred_metrics.total += 1;
        if success {
            cred_metrics.successful += 1;
        } else {
            cred_metrics.failed += 1;
        }

        cred_metrics.avg_duration = avg_duration;
        cred_metrics.last_rotation = Some(now);
        cred_metrics.last_success = Some(success);
    }

    /// Record rotation failure with error details
    ///
    /// # T098: Record Rotation Failure
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential that failed rotation
    /// * `error_type` - Error classification (e.g., "Timeout", "AuthError")
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// metrics.record_rotation_failure(&cred_id, "Timeout");
    /// ```
    pub fn record_rotation_failure(&self, credential_id: &CredentialId, error_type: &str) {
        let mut inner = self.inner.write();

        // Increment failure counter for this error type
        *inner
            .failures_by_type
            .entry(error_type.to_string())
            .or_insert(0) += 1;

        // Update per-credential failure count
        let cred_metrics = inner
            .per_credential
            .entry(credential_id.clone())
            .or_default();

        cred_metrics.failed += 1;
        cred_metrics.last_success = Some(false);
        cred_metrics.last_rotation = Some(Utc::now());
    }

    /// Calculate overall rotation success rate
    ///
    /// # T099: Rotation Success Rate
    ///
    /// Returns success rate as a value between 0.0 and 1.0.
    ///
    /// # Returns
    ///
    /// * `f64` - Success rate (0.0 = 0%, 1.0 = 100%)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let rate = metrics.rotation_success_rate();
    /// assert!(rate >= 0.0 && rate <= 1.0);
    /// println!("Success rate: {:.2}%", rate * 100.0);
    /// ```
    pub fn rotation_success_rate(&self) -> f64 {
        let inner = self.inner.read();

        if inner.total_rotations == 0 {
            return 0.0;
        }

        inner.successful_rotations as f64 / inner.total_rotations as f64
    }

    /// Get total rotation count
    pub fn total_rotations(&self) -> u64 {
        self.inner.read().total_rotations
    }

    /// Get successful rotation count
    pub fn successful_rotations(&self) -> u64 {
        self.inner.read().successful_rotations
    }

    /// Get failed rotation count
    pub fn failed_rotations(&self) -> u64 {
        self.inner.read().failed_rotations
    }

    /// Get average rotation duration
    pub fn avg_duration(&self) -> Option<Duration> {
        let inner = self.inner.read();

        if inner.durations.is_empty() {
            return None;
        }

        let total: Duration = inner.durations.iter().sum();
        Some(total / inner.durations.len() as u32)
    }

    /// Get median rotation duration (p50)
    ///
    /// # Performance
    ///
    /// This method sorts the duration samples, which is O(n log n) where n is the
    /// number of samples (capped at MAX_DURATIONS = 10,000). For high-frequency
    /// metrics access, consider caching the result or using a streaming quantile
    /// algorithm (e.g., t-digest, ckms) in future versions.
    pub fn median_duration(&self) -> Option<Duration> {
        let inner = self.inner.read();

        if inner.durations.is_empty() {
            return None;
        }

        let mut sorted: Vec<Duration> = inner.durations.iter().copied().collect();
        sorted.sort();

        Some(sorted[sorted.len() / 2])
    }

    /// Get p95 rotation duration
    pub fn p95_duration(&self) -> Option<Duration> {
        let inner = self.inner.read();

        if inner.durations.is_empty() {
            return None;
        }

        let mut sorted: Vec<Duration> = inner.durations.iter().copied().collect();
        sorted.sort();

        let index = (sorted.len() as f64 * 0.95) as usize;
        Some(sorted[index.min(sorted.len() - 1)])
    }

    /// Get p99 rotation duration
    pub fn p99_duration(&self) -> Option<Duration> {
        let inner = self.inner.read();

        if inner.durations.is_empty() {
            return None;
        }

        let mut sorted: Vec<Duration> = inner.durations.iter().copied().collect();
        sorted.sort();

        let index = (sorted.len() as f64 * 0.99) as usize;
        Some(sorted[index.min(sorted.len() - 1)])
    }

    /// Get failure counts by error type
    pub fn failures_by_type(&self) -> HashMap<String, u64> {
        self.inner.read().failures_by_type.clone()
    }

    /// Get metrics for specific credential
    pub fn get_credential_metrics(&self, id: &CredentialId) -> Option<CredentialMetrics> {
        self.inner.read().per_credential.get(id).cloned()
    }

    /// Get last rotation timestamp
    pub fn last_rotation_at(&self) -> Option<DateTime<Utc>> {
        self.inner.read().last_rotation_at
    }

    /// Reset all metrics
    pub fn reset(&self) {
        let mut inner = self.inner.write();
        *inner = MetricsInner::default();
    }
}

impl Default for RotationMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_metrics_creation() {
        let metrics = RotationMetrics::new();
        assert_eq!(metrics.total_rotations(), 0);
        assert_eq!(metrics.rotation_success_rate(), 0.0);
    }

    #[test]
    fn test_record_rotation_duration_success() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        metrics.record_rotation_duration(&cred_id, Duration::from_secs(30), true);

        assert_eq!(metrics.total_rotations(), 1);
        assert_eq!(metrics.successful_rotations(), 1);
        assert_eq!(metrics.failed_rotations(), 0);
        assert_eq!(metrics.rotation_success_rate(), 1.0);
    }

    #[test]
    fn test_record_rotation_duration_failure() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        metrics.record_rotation_duration(&cred_id, Duration::from_secs(15), false);

        assert_eq!(metrics.total_rotations(), 1);
        assert_eq!(metrics.successful_rotations(), 0);
        assert_eq!(metrics.failed_rotations(), 1);
        assert_eq!(metrics.rotation_success_rate(), 0.0);
    }

    #[test]
    fn test_record_rotation_failure() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        metrics.record_rotation_failure(&cred_id, "Timeout");

        let failures = metrics.failures_by_type();
        assert_eq!(failures.get("Timeout"), Some(&1));
    }

    #[test]
    fn test_rotation_success_rate() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        // 3 successful, 1 failed = 75% success rate
        metrics.record_rotation_duration(&cred_id, Duration::from_secs(30), true);
        metrics.record_rotation_duration(&cred_id, Duration::from_secs(25), true);
        metrics.record_rotation_duration(&cred_id, Duration::from_secs(35), true);
        metrics.record_rotation_duration(&cred_id, Duration::from_secs(40), false);

        assert_eq!(metrics.total_rotations(), 4);
        assert_eq!(metrics.successful_rotations(), 3);
        assert_eq!(metrics.failed_rotations(), 1);
        assert!((metrics.rotation_success_rate() - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_avg_duration() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        metrics.record_rotation_duration(&cred_id, Duration::from_secs(20), true);
        metrics.record_rotation_duration(&cred_id, Duration::from_secs(30), true);
        metrics.record_rotation_duration(&cred_id, Duration::from_secs(40), true);

        let avg = metrics.avg_duration().unwrap();
        assert_eq!(avg, Duration::from_secs(30));
    }

    #[test]
    fn test_per_credential_metrics() {
        let metrics = RotationMetrics::new();
        let cred1 = CredentialId::new("cred-1").unwrap();
        let cred2 = CredentialId::new("cred-2").unwrap();

        metrics.record_rotation_duration(&cred1, Duration::from_secs(30), true);
        metrics.record_rotation_duration(&cred1, Duration::from_secs(25), false);
        metrics.record_rotation_duration(&cred2, Duration::from_secs(40), true);

        let cred1_metrics = metrics.get_credential_metrics(&cred1).unwrap();
        assert_eq!(cred1_metrics.total, 2);
        assert_eq!(cred1_metrics.successful, 1);
        assert_eq!(cred1_metrics.failed, 1);

        let cred2_metrics = metrics.get_credential_metrics(&cred2).unwrap();
        assert_eq!(cred2_metrics.total, 1);
        assert_eq!(cred2_metrics.successful, 1);
        assert_eq!(cred2_metrics.failed, 0);
    }

    #[test]
    fn test_percentiles() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        // Add 100 rotations with increasing durations
        for i in 1..=100 {
            metrics.record_rotation_duration(&cred_id, Duration::from_secs(i), true);
        }

        let median = metrics.median_duration().unwrap();
        let p95 = metrics.p95_duration().unwrap();
        let p99 = metrics.p99_duration().unwrap();

        assert!(median < p95);
        assert!(p95 < p99);
    }

    #[test]
    fn test_reset() {
        let metrics = RotationMetrics::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        metrics.record_rotation_duration(&cred_id, Duration::from_secs(30), true);
        assert_eq!(metrics.total_rotations(), 1);

        metrics.reset();
        assert_eq!(metrics.total_rotations(), 0);
        assert_eq!(metrics.rotation_success_rate(), 0.0);
    }
}
