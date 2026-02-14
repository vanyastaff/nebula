//! Grace Period Management
//!
//! Provides grace period functionality for zero-downtime credential rotation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use crate::core::CredentialId;

/// Grace period configuration
///
/// Defines the period during which both old and new credentials are valid
/// to allow for seamless migration without service interruption.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::grace_period::GracePeriodConfig;
/// use std::time::Duration;
///
/// let config = GracePeriodConfig {
///     duration: Duration::from_secs(7 * 24 * 3600), // 7 days
///     allow_overlap: true,
///     notify_on_expiry: true,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GracePeriodConfig {
    /// Duration of the grace period
    pub duration: Duration,

    /// Allow old and new credentials to work simultaneously
    pub allow_overlap: bool,

    /// Send notification before old credential expires
    pub notify_on_expiry: bool,
}

impl GracePeriodConfig {
    /// Create a new grace period configuration
    ///
    /// # Arguments
    ///
    /// * `duration` - How long the grace period lasts
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
    /// ```
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            allow_overlap: true,
            notify_on_expiry: true,
        }
    }

    /// Get the grace period end time from a start time
    ///
    /// # Arguments
    ///
    /// * `start` - When the grace period begins
    ///
    /// # Returns
    ///
    /// * `Option<DateTime<Utc>>` - When the grace period expires, or None if overflow
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let start = Utc::now();
    /// if let Some(end) = config.calculate_end_time(start) {
    ///     println!("Grace period ends at: {}", end);
    /// }
    /// ```
    pub fn calculate_end_time(&self, start: DateTime<Utc>) -> Option<DateTime<Utc>> {
        chrono::Duration::from_std(self.duration)
            .ok()
            .and_then(|d| start.checked_add_signed(d))
    }
}

/// Grace period state for a credential rotation
///
/// Tracks the state of dual-credential validity during the grace period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GracePeriodState {
    /// Credential being rotated
    pub credential_id: CredentialId,

    /// Old credential version
    pub old_version: u32,

    /// New credential version
    pub new_version: u32,

    /// When the grace period started
    pub started_at: DateTime<Utc>,

    /// When the grace period expires
    pub expires_at: DateTime<Utc>,

    /// Whether both credentials are currently valid
    pub dual_valid: bool,
}

impl GracePeriodState {
    /// Create a new grace period state
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential being rotated
    /// * `old_version` - Version number of old credential
    /// * `new_version` - Version number of new credential
    /// * `config` - Grace period configuration
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let state = GracePeriodState::new(
    ///     credential_id,
    ///     1, // old version
    ///     2, // new version
    ///     &config,
    /// )?;
    /// ```
    pub fn new(
        credential_id: CredentialId,
        old_version: u32,
        new_version: u32,
        config: &GracePeriodConfig,
    ) -> Result<Self, &'static str> {
        let started_at = Utc::now();
        let expires_at = config
            .calculate_end_time(started_at)
            .ok_or("Grace period duration overflow")?;

        Ok(Self {
            credential_id,
            old_version,
            new_version,
            started_at,
            expires_at,
            dual_valid: config.allow_overlap,
        })
    }

    /// Check if the grace period is still active
    ///
    /// # Returns
    ///
    /// * `bool` - True if grace period has not expired
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if state.is_active() {
    ///     println!("Grace period still active - both credentials valid");
    /// } else {
    ///     println!("Grace period expired - old credential should be revoked");
    /// }
    /// ```
    pub fn is_active(&self) -> bool {
        Utc::now() < self.expires_at
    }

    /// Check if old credential should still be accepted
    ///
    /// During the grace period with overlap enabled, both credentials are valid.
    /// After grace period expires, only the new credential is valid.
    ///
    /// # Returns
    ///
    /// * `bool` - True if old credential should still be accepted
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if state.should_accept_old_credential() {
    ///     // Accept requests with old credential
    /// } else {
    ///     // Reject old credential, force migration to new one
    /// }
    /// ```
    pub fn should_accept_old_credential(&self) -> bool {
        self.dual_valid && self.is_active()
    }

    /// Check if new credential should be accepted
    ///
    /// New credential is always valid once created (after rotation starts).
    ///
    /// # Returns
    ///
    /// * `bool` - True if new credential should be accepted
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if state.should_accept_new_credential() {
    ///     // Accept requests with new credential
    /// }
    /// ```
    pub fn should_accept_new_credential(&self) -> bool {
        true // New credential is always valid
    }

    /// Force end the grace period immediately
    ///
    /// Useful for emergency rotations where we want to immediately
    /// invalidate the old credential.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Security incident - revoke old credential immediately
    /// state.force_end();
    /// assert!(!state.is_active());
    /// ```
    pub fn force_end(&mut self) {
        self.expires_at = Utc::now();
        self.dual_valid = false;
    }
}

/// Usage metrics for credential tracking
///
/// Tracks request counts and last usage timestamp for monitoring
/// credential migration during grace periods.
///
/// # T064: Usage Metrics
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::grace_period::UsageMetrics;
///
/// let mut metrics = UsageMetrics::default();
/// metrics.record_usage(); // Increment request count
/// println!("Total requests: {}", metrics.request_count);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageMetrics {
    /// Total number of requests using this credential
    pub request_count: u64,

    /// Timestamp of the last request
    pub last_used: Option<DateTime<Utc>>,

    /// Timestamp of the first request
    pub first_used: Option<DateTime<Utc>>,
}

impl UsageMetrics {
    /// Create new usage metrics
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a credential usage event
    ///
    /// # T065: Track Credential Usage
    pub fn record_usage(&mut self) {
        let now = Utc::now();
        self.request_count += 1;
        self.last_used = Some(now);

        if self.first_used.is_none() {
            self.first_used = Some(now);
        }
    }

    /// Check if credential has been used recently
    ///
    /// # Arguments
    ///
    /// * `threshold` - Time threshold for "recent" usage
    ///
    /// # Returns
    ///
    /// * `bool` - True if last_used is within threshold
    pub fn is_recently_used(&self, threshold: Duration) -> bool {
        if let Some(last_used) = self.last_used {
            let elapsed = Utc::now() - last_used;
            elapsed < chrono::Duration::from_std(threshold).unwrap_or(chrono::Duration::zero())
        } else {
            false
        }
    }

    /// Check if credential has never been used
    pub fn is_unused(&self) -> bool {
        self.request_count == 0
    }
}

/// Track credential usage during grace period migration
///
/// Monitors both old and new credential usage to determine when it's safe
/// to revoke the old credential.
///
/// # T066: Grace Period Tracker
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::grace_period::GracePeriodTracker;
///
/// let tracker = GracePeriodTracker::new(
///     old_credential_id,
///     new_credential_id,
///     grace_period_state,
/// );
///
/// // Track usage
/// tracker.track_old_credential_usage();
/// tracker.track_new_credential_usage();
///
/// // Check if migration is complete
/// if tracker.can_revoke_old_credential(Duration::from_secs(3600)) {
///     println!("Safe to revoke old credential");
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GracePeriodTracker {
    /// Old credential being phased out
    pub old_credential_id: CredentialId,

    /// New credential being adopted
    pub new_credential_id: CredentialId,

    /// Usage metrics for old credential
    pub old_metrics: UsageMetrics,

    /// Usage metrics for new credential
    pub new_metrics: UsageMetrics,

    /// Grace period state
    pub grace_period: GracePeriodState,
}

impl GracePeriodTracker {
    /// Create a new grace period tracker
    pub fn new(
        old_credential_id: CredentialId,
        new_credential_id: CredentialId,
        grace_period: GracePeriodState,
    ) -> Self {
        Self {
            old_credential_id,
            new_credential_id,
            old_metrics: UsageMetrics::new(),
            new_metrics: UsageMetrics::new(),
            grace_period,
        }
    }

    /// Track usage of old credential
    ///
    /// # T065: Track Credential Usage
    pub fn track_old_credential_usage(&mut self) {
        self.old_metrics.record_usage();
    }

    /// Track usage of new credential
    ///
    /// # T065: Track Credential Usage
    pub fn track_new_credential_usage(&mut self) {
        self.new_metrics.record_usage();
    }

    /// Check if old credential is still being used
    ///
    /// # T067: Check Old Credential Usage
    ///
    /// # Arguments
    ///
    /// * `threshold` - Time threshold for "recent" usage
    ///
    /// # Returns
    ///
    /// * `bool` - True if old credential was used within threshold
    pub fn check_old_credential_usage(&self, threshold: Duration) -> bool {
        self.old_metrics.is_recently_used(threshold)
    }

    /// Determine if it's safe to revoke the old credential
    ///
    /// # T068: Can Revoke Old Credential
    ///
    /// Old credential can be revoked if:
    /// 1. Grace period has expired OR
    /// 2. Old credential hasn't been used recently AND new credential is being used
    ///
    /// # Arguments
    ///
    /// * `inactivity_threshold` - How long old credential must be unused
    ///
    /// # Returns
    ///
    /// * `bool` - True if old credential can be safely revoked
    pub fn can_revoke_old_credential(&self, inactivity_threshold: Duration) -> bool {
        // Grace period expired - always safe to revoke
        if !self.grace_period.is_active() {
            return true;
        }

        // Old credential not used recently AND new credential is in use
        let old_inactive = !self.check_old_credential_usage(inactivity_threshold);
        let new_active = self.new_metrics.request_count > 0;

        old_inactive && new_active
    }
}

/// Track usage of a credential
///
/// # T065: Track Credential Usage
///
/// # Arguments
///
/// * `credential_id` - Credential to track
/// * `metrics` - Metrics map to update
pub fn track_credential_usage(
    credential_id: &CredentialId,
    metrics: &mut HashMap<CredentialId, UsageMetrics>,
) {
    metrics
        .entry(credential_id.clone())
        .or_default()
        .record_usage();
}

/// Clean up expired credentials after grace period
///
/// # T069: Cleanup Expired Credentials
///
/// # Arguments
///
/// * `trackers` - List of grace period trackers
///
/// # Returns
///
/// * `Vec<CredentialId>` - List of credentials that can be revoked
pub fn cleanup_expired_credentials(trackers: &[GracePeriodTracker]) -> Vec<CredentialId> {
    trackers
        .iter()
        .filter(|tracker| !tracker.grace_period.is_active())
        .map(|tracker| tracker.old_credential_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grace_period_config_creation() {
        let duration = Duration::from_secs(7 * 24 * 3600); // 7 days
        let config = GracePeriodConfig::new(duration);

        assert_eq!(config.duration, duration);
        assert!(config.allow_overlap);
        assert!(config.notify_on_expiry);
    }

    #[test]
    fn test_calculate_end_time() {
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
        let start = Utc::now();
        let end = config
            .calculate_end_time(start)
            .expect("Should calculate end time");

        let expected_duration = chrono::Duration::days(7);
        let actual_duration = end - start;

        // Allow 1 second tolerance
        assert!((actual_duration - expected_duration).num_seconds().abs() <= 1);
    }

    #[test]
    fn test_grace_period_state_creation() {
        let credential_id = CredentialId::new("test-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));

        let state = GracePeriodState::new(credential_id.clone(), 1, 2, &config)
            .expect("Should create grace period state");

        assert_eq!(state.credential_id, credential_id);
        assert_eq!(state.old_version, 1);
        assert_eq!(state.new_version, 2);
        assert!(state.dual_valid);
        assert!(state.is_active());
    }

    #[test]
    fn test_credential_acceptance_during_grace_period() {
        let credential_id = CredentialId::new("test-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));

        let state = GracePeriodState::new(credential_id, 1, 2, &config)
            .expect("Should create grace period state");

        // During grace period, both credentials should be accepted
        assert!(state.should_accept_old_credential());
        assert!(state.should_accept_new_credential());
        assert!(state.is_active());
    }

    #[test]
    fn test_force_end_grace_period() {
        let credential_id = CredentialId::new("test-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));

        let mut state = GracePeriodState::new(credential_id, 1, 2, &config)
            .expect("Should create grace period state");
        assert!(state.is_active());

        // Force end the grace period
        state.force_end();

        assert!(!state.is_active());
        assert!(!state.should_accept_old_credential());
        assert!(state.should_accept_new_credential());
    }

    #[test]
    fn test_grace_period_without_overlap() {
        let credential_id = CredentialId::new("test-cred").unwrap();
        let mut config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
        config.allow_overlap = false;

        let state = GracePeriodState::new(credential_id, 1, 2, &config)
            .expect("Should create grace period state");

        // Without overlap, old credential should not be accepted
        assert!(!state.should_accept_old_credential());
        assert!(state.should_accept_new_credential());
    }

    #[test]
    fn test_usage_metrics_creation() {
        let metrics = UsageMetrics::new();
        assert_eq!(metrics.request_count, 0);
        assert!(metrics.last_used.is_none());
        assert!(metrics.first_used.is_none());
        assert!(metrics.is_unused());
    }

    #[test]
    fn test_usage_metrics_record_usage() {
        let mut metrics = UsageMetrics::new();

        metrics.record_usage();
        assert_eq!(metrics.request_count, 1);
        assert!(metrics.last_used.is_some());
        assert!(metrics.first_used.is_some());
        assert!(!metrics.is_unused());

        let first_used = metrics.first_used.unwrap();

        // Record another usage
        metrics.record_usage();
        assert_eq!(metrics.request_count, 2);
        assert_eq!(metrics.first_used.unwrap(), first_used); // Should not change
    }

    #[test]
    fn test_usage_metrics_is_recently_used() {
        let mut metrics = UsageMetrics::new();
        metrics.record_usage();

        // Just used - should be recent
        assert!(metrics.is_recently_used(Duration::from_secs(60)));
    }

    #[test]
    fn test_grace_period_tracker_creation() {
        let old_id = CredentialId::new("old-cred").unwrap();
        let new_id = CredentialId::new("new-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
        let grace_period = GracePeriodState::new(old_id.clone(), 1, 2, &config)
            .expect("Should create grace period state");

        let tracker = GracePeriodTracker::new(old_id.clone(), new_id.clone(), grace_period);

        assert_eq!(tracker.old_credential_id, old_id);
        assert_eq!(tracker.new_credential_id, new_id);
        assert_eq!(tracker.old_metrics.request_count, 0);
        assert_eq!(tracker.new_metrics.request_count, 0);
    }

    #[test]
    fn test_grace_period_tracker_usage_tracking() {
        let old_id = CredentialId::new("old-cred").unwrap();
        let new_id = CredentialId::new("new-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
        let grace_period = GracePeriodState::new(old_id.clone(), 1, 2, &config)
            .expect("Should create grace period state");

        let mut tracker = GracePeriodTracker::new(old_id, new_id, grace_period);

        tracker.track_old_credential_usage();
        tracker.track_new_credential_usage();
        tracker.track_new_credential_usage();

        assert_eq!(tracker.old_metrics.request_count, 1);
        assert_eq!(tracker.new_metrics.request_count, 2);
    }

    #[test]
    fn test_can_revoke_old_credential_grace_period_expired() {
        let old_id = CredentialId::new("old-cred").unwrap();
        let new_id = CredentialId::new("new-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(0)); // Immediate expiry
        let grace_period = GracePeriodState::new(old_id.clone(), 1, 2, &config)
            .expect("Should create grace period state");

        let tracker = GracePeriodTracker::new(old_id, new_id, grace_period);

        // Grace period expired - should always be revokable
        assert!(tracker.can_revoke_old_credential(Duration::from_secs(3600)));
    }

    #[test]
    fn test_can_revoke_old_credential_migration_complete() {
        let old_id = CredentialId::new("old-cred").unwrap();
        let new_id = CredentialId::new("new-cred").unwrap();
        let config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
        let grace_period = GracePeriodState::new(old_id.clone(), 1, 2, &config)
            .expect("Should create grace period state");

        let mut tracker = GracePeriodTracker::new(old_id, new_id, grace_period);

        // New credential being used, old not used recently
        tracker.track_new_credential_usage();

        // Old credential not used within threshold - safe to revoke
        assert!(tracker.can_revoke_old_credential(Duration::from_secs(1)));
    }

    #[test]
    fn test_track_credential_usage_function() {
        let mut metrics_map = HashMap::new();
        let cred_id = CredentialId::new("test-cred").unwrap();

        track_credential_usage(&cred_id, &mut metrics_map);
        track_credential_usage(&cred_id, &mut metrics_map);

        assert_eq!(metrics_map.get(&cred_id).unwrap().request_count, 2);
    }

    #[test]
    fn test_cleanup_expired_credentials() {
        let old_id1 = CredentialId::new("old-1").unwrap();
        let new_id1 = CredentialId::new("new-1").unwrap();
        let old_id2 = CredentialId::new("old-2").unwrap();
        let new_id2 = CredentialId::new("new-2").unwrap();

        // Expired grace period
        let config_expired = GracePeriodConfig::new(Duration::from_secs(0));
        let grace_expired = GracePeriodState::new(old_id1.clone(), 1, 2, &config_expired)
            .expect("Should create grace period state");
        let tracker1 = GracePeriodTracker::new(old_id1.clone(), new_id1, grace_expired);

        // Active grace period
        let config_active = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600));
        let grace_active = GracePeriodState::new(old_id2.clone(), 1, 2, &config_active)
            .expect("Should create grace period state");
        let tracker2 = GracePeriodTracker::new(old_id2, new_id2, grace_active);

        let trackers = vec![tracker1, tracker2];
        let expired = cleanup_expired_credentials(&trackers);

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], old_id1);
    }
}
