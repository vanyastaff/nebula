//! Rotation Scheduler
//!
//! Provides scheduling functionality for automatic credential rotation.

use chrono::{DateTime, Utc};
use std::time::Duration;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::core::CredentialId;
use crate::rotation::RotationResult;
use crate::rotation::policy::{BeforeExpiryConfig, PeriodicConfig, ScheduledConfig};

/// Periodic rotation scheduler with jitter support
///
/// Schedules credential rotations at fixed intervals with optional
/// randomization to prevent rotation storms.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::scheduler::PeriodicScheduler;
/// use nebula_credential::rotation::policy::PeriodicConfig;
/// use std::time::Duration;
///
/// let config = PeriodicConfig {
///     interval: Duration::from_secs(90 * 24 * 3600), // 90 days
///     grace_period: Duration::from_secs(7 * 24 * 3600), // 7 days
///     enable_jitter: true,
/// };
///
/// let scheduler = PeriodicScheduler::new(config);
/// let next_rotation = scheduler.calculate_next_rotation_time();
/// ```
pub struct PeriodicScheduler {
    /// Rotation policy configuration
    config: PeriodicConfig,
}

impl PeriodicScheduler {
    /// Create a new periodic scheduler
    ///
    /// # Arguments
    ///
    /// * `config` - Periodic rotation configuration
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::rotation::scheduler::PeriodicScheduler;
    /// use nebula_credential::rotation::policy::PeriodicConfig;
    /// use std::time::Duration;
    ///
    /// let config = PeriodicConfig {
    ///     interval: Duration::from_secs(90 * 24 * 3600),
    ///     grace_period: Duration::from_secs(7 * 24 * 3600),
    ///     enable_jitter: true,
    /// };
    ///
    /// let scheduler = PeriodicScheduler::new(config);
    /// ```
    pub fn new(config: PeriodicConfig) -> Self {
        Self { config }
    }

    /// Schedule next rotation with optional jitter
    ///
    /// Returns the time when the next rotation should occur.
    /// If jitter is enabled, adds ±10% randomization to prevent rotation storms.
    ///
    /// # Returns
    ///
    /// * `Instant` - Time when next rotation should execute
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let next_time = scheduler.schedule_rotation();
    /// println!("Next rotation at: {:?}", next_time);
    /// ```
    pub fn schedule_rotation(&self) -> Instant {
        self.calculate_next_rotation_time()
    }

    /// Calculate next rotation time with jitter
    ///
    /// Applies ±10% jitter if enabled in config to spread out rotations
    /// and prevent multiple credentials from rotating simultaneously.
    ///
    /// # Returns
    ///
    /// * `Instant` - Calculated rotation time
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let base_interval = Duration::from_secs(90 * 24 * 3600);
    /// let next_time = scheduler.calculate_next_rotation_time();
    /// // With jitter enabled, actual interval will be 81-99 days
    /// ```
    pub fn calculate_next_rotation_time(&self) -> Instant {
        let base_interval = self.config.interval();

        let actual_interval = if self.config.enable_jitter() {
            // Apply ±10% jitter
            apply_jitter(base_interval)
        } else {
            base_interval
        };

        Instant::now() + actual_interval
    }

    /// Run background rotation loop
    ///
    /// Continuously schedules and executes rotations at the configured interval.
    /// This is a long-running async task that should be spawned separately.
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential to rotate
    /// * `rotation_fn` - Async function to execute rotation
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use tokio::task;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// let scheduler = PeriodicScheduler::new(config);
    /// let credential_id = CredentialId::new("db-password").unwrap();
    /// let shutdown = CancellationToken::new();
    ///
    /// task::spawn(async move {
    ///     scheduler.run_rotation_loop(credential_id, |id| async move {
    ///         // Perform rotation
    ///         manager.rotate_credential(&id).await
    ///     }, shutdown).await
    /// });
    /// ```
    pub async fn run_rotation_loop<F, Fut>(
        &self,
        credential_id: CredentialId,
        mut rotation_fn: F,
        shutdown: CancellationToken,
    ) -> RotationResult<()>
    where
        F: FnMut(CredentialId) -> Fut,
        Fut: std::future::Future<Output = RotationResult<()>>,
    {
        loop {
            // Calculate next rotation time with jitter
            let next_rotation = self.calculate_next_rotation_time();

            // Sleep until next rotation or shutdown signal
            tokio::select! {
                _ = sleep_until(next_rotation) => {
                    // Execute rotation
                    match rotation_fn(credential_id.clone()).await {
                        Ok(()) => {
                            tracing::info!(
                                credential_id = %credential_id,
                                "Periodic rotation completed successfully"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                credential_id = %credential_id,
                                error = %e,
                                "Periodic rotation failed"
                            );
                            // Continue loop even on failure - will retry at next interval
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    tracing::info!(
                        credential_id = %credential_id,
                        "Rotation loop shutting down gracefully"
                    );
                    return Ok(());
                }
            }
        }
    }
}

/// Expiry monitor for TTL-based credential rotation
///
/// Monitors credentials with expiration times and triggers rotation
/// before they expire based on configured thresholds.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::scheduler::ExpiryMonitor;
/// use nebula_credential::rotation::policy::BeforeExpiryConfig;
/// use std::time::Duration;
///
/// let config = BeforeExpiryConfig {
///     threshold_percentage: 0.8, // Rotate at 80% of TTL
///     minimum_time_before_expiry: Duration::from_secs(3600), // At least 1 hour before
/// };
///
/// let monitor = ExpiryMonitor::new(config);
/// let trigger_time = monitor.calculate_rotation_trigger_time(expiry_time);
/// ```
pub struct ExpiryMonitor {
    /// Before-expiry rotation configuration
    config: BeforeExpiryConfig,
}

impl ExpiryMonitor {
    /// Create a new expiry monitor
    ///
    /// # Arguments
    ///
    /// * `config` - Before-expiry rotation configuration
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::rotation::scheduler::ExpiryMonitor;
    /// use nebula_credential::rotation::policy::BeforeExpiryConfig;
    /// use std::time::Duration;
    ///
    /// let config = BeforeExpiryConfig {
    ///     threshold_percentage: 0.8,
    ///     minimum_time_before_expiry: Duration::from_secs(3600),
    /// };
    ///
    /// let monitor = ExpiryMonitor::new(config);
    /// ```
    pub fn new(config: BeforeExpiryConfig) -> Self {
        Self { config }
    }

    /// Calculate when to trigger rotation based on expiry time
    ///
    /// Determines the rotation trigger time using the configured threshold percentage,
    /// while ensuring the minimum time before expiry is respected.
    ///
    /// # Arguments
    ///
    /// * `created_at` - When the credential was created
    /// * `expires_at` - When the credential expires
    ///
    /// # Returns
    ///
    /// * `DateTime<Utc>` - Time when rotation should be triggered
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use chrono::{Utc, Duration};
    ///
    /// let created = Utc::now();
    /// let expires = created + Duration::days(30);
    ///
    /// // With 80% threshold, will trigger at day 24 (80% of 30 days)
    /// let trigger_time = monitor.calculate_rotation_trigger_time(created, expires);
    /// ```
    pub fn calculate_rotation_trigger_time(
        &self,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> DateTime<Utc> {
        // Calculate total TTL
        let total_ttl = expires_at - created_at;

        // Calculate threshold-based trigger time
        let threshold_duration = match total_ttl.to_std() {
            Ok(duration) => duration.mul_f64(self.config.threshold_percentage() as f64),
            Err(_) => {
                tracing::warn!(
                    created_at = %created_at,
                    expires_at = %expires_at,
                    "TTL duration overflow in rotation trigger calculation, using zero"
                );
                Duration::ZERO
            }
        };

        let threshold_trigger = created_at
            + chrono::Duration::from_std(threshold_duration).unwrap_or(chrono::Duration::zero());

        // Calculate minimum-time-based trigger time
        let minimum_trigger = expires_at
            - chrono::Duration::from_std(self.config.minimum_time_before_expiry())
                .unwrap_or(chrono::Duration::zero());

        // Use whichever comes first (more conservative)
        std::cmp::min(threshold_trigger, minimum_trigger)
    }

    /// Check multiple credentials for approaching expiration
    ///
    /// Batch operation to check which credentials need rotation based on their
    /// expiration times. Returns credentials that have reached their rotation trigger time.
    ///
    /// # Arguments
    ///
    /// * `credentials` - Slice of (credential_id, created_at, expires_at) tuples
    ///
    /// # Returns
    ///
    /// * `Vec<CredentialId>` - IDs of credentials that should be rotated now
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let credentials = vec![
    ///     (id1, created1, expires1),
    ///     (id2, created2, expires2),
    ///     (id3, created3, expires3),
    /// ];
    ///
    /// let to_rotate = monitor.check_credentials(&credentials);
    /// for id in to_rotate {
    ///     manager.rotate_credential(&id).await?;
    /// }
    /// ```
    pub fn check_credentials(
        &self,
        credentials: &[(CredentialId, DateTime<Utc>, DateTime<Utc>)],
    ) -> Vec<CredentialId> {
        let now = Utc::now();
        let mut to_rotate = Vec::new();

        for (id, created_at, expires_at) in credentials {
            let trigger_time = self.calculate_rotation_trigger_time(*created_at, *expires_at);

            if now >= trigger_time {
                to_rotate.push(id.clone());
            }
        }

        to_rotate
    }
}

/// Scheduled rotation for one-time rotation events
///
/// Schedules credential rotation at a specific date/time, typically for
/// maintenance windows or planned security updates.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::scheduler::ScheduledRotation;
/// use nebula_credential::rotation::policy::ScheduledConfig;
/// use chrono::{Utc, Duration};
///
/// let scheduled_at = Utc::now() + Duration::hours(24);
/// let config = ScheduledConfig {
///     scheduled_at,
///     grace_period: Duration::hours(2).to_std().unwrap(),
///     notify_before: Some(Duration::hours(1).to_std().unwrap()),
/// };
///
/// let rotation = ScheduledRotation::new(config);
/// let trigger_time = rotation.schedule_at();
/// ```
pub struct ScheduledRotation {
    /// Scheduled rotation configuration
    config: ScheduledConfig,
}

impl ScheduledRotation {
    /// Create a new scheduled rotation
    ///
    /// # Arguments
    ///
    /// * `config` - Scheduled rotation configuration
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::rotation::scheduler::ScheduledRotation;
    /// use nebula_credential::rotation::policy::ScheduledConfig;
    /// use chrono::Utc;
    /// use std::time::Duration;
    ///
    /// let config = ScheduledConfig {
    ///     scheduled_at: Utc::now() + chrono::Duration::days(7),
    ///     grace_period: Duration::from_secs(7200),
    ///     notify_before: Some(Duration::from_secs(86400)),
    /// };
    ///
    /// let rotation = ScheduledRotation::new(config);
    /// ```
    pub fn new(config: ScheduledConfig) -> Self {
        Self { config }
    }

    /// Get the scheduled rotation time
    ///
    /// Returns the exact DateTime when rotation should occur.
    ///
    /// # Returns
    ///
    /// * `DateTime<Utc>` - Scheduled rotation time
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let rotation_time = rotation.schedule_at();
    /// println!("Rotation scheduled for: {}", rotation_time);
    /// ```
    pub fn schedule_at(&self) -> DateTime<Utc> {
        self.config.scheduled_at()
    }

    /// Calculate notification time based on notify_before setting
    ///
    /// Returns when to send pre-rotation notification, if configured.
    ///
    /// # Returns
    ///
    /// * `Option<DateTime<Utc>>` - Notification time, or None if not configured
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(notify_time) = rotation.notification_time() {
    ///     println!("Send notification at: {}", notify_time);
    /// }
    /// ```
    pub fn notification_time(&self) -> Option<DateTime<Utc>> {
        self.config.notify_before().map(|notify_before| {
            self.config.scheduled_at()
                - chrono::Duration::from_std(notify_before).unwrap_or(chrono::Duration::zero())
        })
    }

    /// Check if it's time to send notification
    ///
    /// Returns true if current time has reached the notification time.
    ///
    /// # Returns
    ///
    /// * `bool` - True if notification should be sent now
    pub fn should_notify_now(&self) -> bool {
        self.notification_time()
            .map(|notify_at| Utc::now() >= notify_at)
            .unwrap_or(false)
    }

    /// Check if it's time to execute rotation
    ///
    /// Returns true if current time has reached the scheduled rotation time.
    ///
    /// # Returns
    ///
    /// * `bool` - True if rotation should execute now
    pub fn should_rotate_now(&self) -> bool {
        Utc::now() >= self.config.scheduled_at()
    }

    /// Time remaining until rotation
    ///
    /// Returns the duration until scheduled rotation time.
    ///
    /// # Returns
    ///
    /// * `chrono::Duration` - Time until rotation (negative if past due)
    pub fn time_until_rotation(&self) -> chrono::Duration {
        self.config.scheduled_at() - Utc::now()
    }
}

/// Apply ±10% jitter to duration
///
/// Adds randomization to prevent rotation storms when multiple
/// credentials are configured with the same rotation interval.
///
/// # Arguments
///
/// * `base` - Base duration to apply jitter to
///
/// # Returns
///
/// * `Duration` - Duration with applied jitter (90-110% of base)
///
/// # Example
///
/// ```rust,ignore
/// let base = Duration::from_secs(90 * 24 * 3600); // 90 days
/// let jittered = apply_jitter(base); // 81-99 days
/// ```
fn apply_jitter(base: Duration) -> Duration {
    use rand::Rng;

    let base_secs = base.as_secs_f64();

    // ±10% jitter
    let min_secs = base_secs * 0.9;
    let max_secs = base_secs * 1.1;

    let mut rng = rand::rng();
    let jittered_secs = rng.random_range(min_secs..=max_secs);

    Duration::from_secs_f64(jittered_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_creation() {
        let config = PeriodicConfig::new(
            Duration::from_secs(90 * 24 * 3600),
            Duration::from_secs(7 * 24 * 3600),
            false,
        )
        .unwrap();

        let scheduler = PeriodicScheduler::new(config);
        let next_time = scheduler.calculate_next_rotation_time();

        // Should be approximately 90 days from now
        let expected = Instant::now() + Duration::from_secs(90 * 24 * 3600);
        let diff = if next_time > expected {
            next_time - expected
        } else {
            expected - next_time
        };

        // Allow 1 second tolerance for test execution time
        assert!(diff < Duration::from_secs(1));
    }

    #[test]
    fn test_jitter_application() {
        let base = Duration::from_secs(90 * 24 * 3600); // 90 days

        // Test multiple times to ensure jitter is within range
        for _ in 0..100 {
            let jittered = apply_jitter(base);
            let jittered_secs = jittered.as_secs();
            let base_secs = base.as_secs();

            // Should be between 90% and 110% of base
            let min_allowed = (base_secs as f64 * 0.9) as u64;
            let max_allowed = (base_secs as f64 * 1.1) as u64;

            assert!(jittered_secs >= min_allowed && jittered_secs <= max_allowed);
        }
    }

    #[test]
    fn test_schedule_with_jitter_enabled() {
        let config = PeriodicConfig::new(
            Duration::from_secs(90 * 24 * 3600),
            Duration::from_secs(7 * 24 * 3600),
            true,
        )
        .unwrap();

        let scheduler = PeriodicScheduler::new(config);

        // Test multiple schedules to ensure jitter varies
        let mut times = Vec::new();
        for _ in 0..10 {
            let next_time = scheduler.calculate_next_rotation_time();
            times.push(next_time);
        }

        // With jitter, not all times should be identical
        // (though theoretically they could be in rare cases)
        let all_same = times.windows(2).all(|w| w[0] == w[1]);
        assert!(!all_same, "Jitter should produce varying rotation times");
    }

    #[test]
    fn test_expiry_monitor_creation() {
        let config =
            BeforeExpiryConfig::new(0.8, Duration::from_secs(3600), Duration::from_secs(600))
                .unwrap();

        let _monitor = ExpiryMonitor::new(config);
    }

    #[test]
    fn test_calculate_rotation_trigger_time_with_threshold() {
        use chrono::Duration as ChronoDuration;

        let config = BeforeExpiryConfig::new(
            0.8,                       // 80% of TTL
            Duration::from_secs(3600), // 1 hour
            Duration::from_secs(600),
        )
        .unwrap();

        let monitor = ExpiryMonitor::new(config);

        let created = Utc::now();
        let expires = created + ChronoDuration::days(30); // 30-day TTL

        let trigger_time = monitor.calculate_rotation_trigger_time(created, expires);

        // Should trigger at 80% of 30 days = 24 days
        let expected_trigger = created + ChronoDuration::days(24);

        // Allow small tolerance for calculation precision
        let diff = if trigger_time > expected_trigger {
            trigger_time - expected_trigger
        } else {
            expected_trigger - trigger_time
        };

        assert!(
            diff < ChronoDuration::seconds(1),
            "Trigger time should be at 80% of TTL"
        );
    }

    #[test]
    fn test_calculate_rotation_trigger_time_with_minimum() {
        use chrono::Duration as ChronoDuration;

        let config = BeforeExpiryConfig::new(
            0.9,                                 // 90% of TTL
            Duration::from_secs(10 * 24 * 3600), // 10 days
            Duration::from_secs(600),
        )
        .unwrap();

        let monitor = ExpiryMonitor::new(config);

        let created = Utc::now();
        let expires = created + ChronoDuration::days(30); // 30-day TTL

        let trigger_time = monitor.calculate_rotation_trigger_time(created, expires);

        // 90% would be day 27, but minimum is 10 days before expiry = day 20
        // Should use the more conservative (earlier) time = day 20
        let expected_trigger = expires - ChronoDuration::days(10);

        let diff = if trigger_time > expected_trigger {
            trigger_time - expected_trigger
        } else {
            expected_trigger - trigger_time
        };

        assert!(
            diff < ChronoDuration::seconds(1),
            "Should use minimum time before expiry when more conservative"
        );
    }

    #[test]
    fn test_check_credentials_batch() {
        use chrono::Duration as ChronoDuration;

        let config =
            BeforeExpiryConfig::new(0.8, Duration::from_secs(3600), Duration::from_secs(600))
                .unwrap();

        let monitor = ExpiryMonitor::new(config);

        let now = Utc::now();

        // Credential 1: Already past trigger time (needs rotation)
        let id1 = CredentialId::new("cred-1").unwrap();
        let created1 = now - ChronoDuration::days(30);
        let expires1 = now + ChronoDuration::days(2); // Expires in 2 days, created 30 days ago

        // Credential 2: Not yet at trigger time
        let id2 = CredentialId::new("cred-2").unwrap();
        let created2 = now - ChronoDuration::days(5);
        let expires2 = now + ChronoDuration::days(25); // Expires in 25 days, created 5 days ago

        // Credential 3: Past trigger time (needs rotation)
        // Created 25 days ago, expires in 5 days = 30 day TTL
        // Trigger at 80% = 24 days from creation = 1 day ago (clearly past)
        let id3 = CredentialId::new("cred-3").unwrap();
        let created3 = now - ChronoDuration::days(25);
        let expires3 = now + ChronoDuration::days(5);

        let credentials = vec![
            (id1.clone(), created1, expires1),
            (id2.clone(), created2, expires2),
            (id3.clone(), created3, expires3),
        ];

        let to_rotate = monitor.check_credentials(&credentials);

        // Should identify cred-1 and cred-3 as needing rotation
        assert_eq!(to_rotate.len(), 2);
        assert!(to_rotate.contains(&id1));
        assert!(to_rotate.contains(&id3));
        assert!(!to_rotate.contains(&id2));
    }

    #[test]
    fn test_check_credentials_empty() {
        let config =
            BeforeExpiryConfig::new(0.8, Duration::from_secs(3600), Duration::from_secs(600))
                .unwrap();

        let monitor = ExpiryMonitor::new(config);

        let credentials = vec![];
        let to_rotate = monitor.check_credentials(&credentials);

        assert!(to_rotate.is_empty());
    }

    #[test]
    fn test_scheduled_rotation_creation() {
        use chrono::Duration as ChronoDuration;

        let scheduled_at = Utc::now() + ChronoDuration::days(7);
        let config = ScheduledConfig::new(
            scheduled_at,
            Duration::from_secs(7200),
            Some(Duration::from_secs(86400)),
        )
        .unwrap();

        let rotation = ScheduledRotation::new(config);
        assert_eq!(rotation.schedule_at(), scheduled_at);
    }

    #[test]
    fn test_scheduled_rotation_notification_time() {
        use chrono::Duration as ChronoDuration;

        let scheduled_at = Utc::now() + ChronoDuration::days(7);
        let notify_before = Duration::from_secs(86400); // 24 hours
        let config =
            ScheduledConfig::new(scheduled_at, Duration::from_secs(7200), Some(notify_before))
                .unwrap();

        let rotation = ScheduledRotation::new(config);
        let notification_time = rotation.notification_time().unwrap();

        // Notification should be 24 hours before rotation
        let expected_notify = scheduled_at - ChronoDuration::days(1);
        let diff = if notification_time > expected_notify {
            notification_time - expected_notify
        } else {
            expected_notify - notification_time
        };

        assert!(diff < ChronoDuration::seconds(1));
    }

    #[test]
    fn test_scheduled_rotation_no_notification() {
        use chrono::Duration as ChronoDuration;

        let scheduled_at = Utc::now() + ChronoDuration::days(7);
        let config = ScheduledConfig::new(scheduled_at, Duration::from_secs(7200), None).unwrap();

        let rotation = ScheduledRotation::new(config);
        assert!(rotation.notification_time().is_none());
        assert!(!rotation.should_notify_now());
    }

    #[test]
    fn test_scheduled_rotation_should_rotate_now() {
        use chrono::Duration as ChronoDuration;

        // Scheduled in the past
        let scheduled_at = Utc::now() - ChronoDuration::hours(1);
        let config = ScheduledConfig::new_unchecked(scheduled_at, Duration::from_secs(7200), None);

        let rotation = ScheduledRotation::new(config);
        assert!(rotation.should_rotate_now());
    }

    #[test]
    fn test_scheduled_rotation_time_until() {
        use chrono::Duration as ChronoDuration;

        let scheduled_at = Utc::now() + ChronoDuration::hours(24);
        let config = ScheduledConfig::new(scheduled_at, Duration::from_secs(7200), None).unwrap();

        let rotation = ScheduledRotation::new(config);
        let time_until = rotation.time_until_rotation();

        // Should be approximately 24 hours (allow some tolerance for test execution time)
        assert!(time_until > ChronoDuration::hours(23));
        assert!(time_until < ChronoDuration::hours(25));
    }
}
