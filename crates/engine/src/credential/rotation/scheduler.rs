//! Engine-owned credential rotation schedulers.
//!
//! These schedulers operate on credential rotation policies while keeping the
//! orchestration runtime in `nebula-engine` (exec layer). Contract/state types
//! remain in `nebula-credential`.

use std::{future::Future, time::Duration};

use chrono::{DateTime, Utc};
use nebula_credential::{
    CredentialId,
    rotation::{
        RotationResult,
        policy::{BeforeExpiryConfig, PeriodicConfig, ScheduledConfig},
    },
};
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

/// Periodic scheduler with optional jitter to avoid herd rotations.
pub struct PeriodicScheduler {
    config: PeriodicConfig,
}

impl PeriodicScheduler {
    /// Build a scheduler from periodic policy config.
    #[must_use]
    pub fn new(config: PeriodicConfig) -> Self {
        Self { config }
    }

    /// Returns the next rotation instant.
    #[must_use]
    pub fn schedule_rotation(&self) -> Instant {
        self.calculate_next_rotation_time()
    }

    /// Calculate next rotation time, applying jitter when enabled.
    #[must_use]
    pub fn calculate_next_rotation_time(&self) -> Instant {
        let interval = if self.config.enable_jitter() {
            apply_jitter(self.config.interval())
        } else {
            self.config.interval()
        };

        Instant::now() + interval
    }

    /// Run a best-effort loop that executes rotation by schedule until stopped.
    pub async fn run_rotation_loop<F, Fut>(
        &self,
        credential_id: CredentialId,
        mut rotation_fn: F,
        shutdown: CancellationToken,
    ) -> RotationResult<()>
    where
        F: FnMut(CredentialId) -> Fut,
        Fut: Future<Output = RotationResult<()>>,
    {
        loop {
            let next = self.calculate_next_rotation_time();
            tokio::select! {
                () = sleep_until(next) => {
                    if let Err(error) = rotation_fn(credential_id).await {
                        tracing::error!(
                            credential_id = %credential_id,
                            error = %error,
                            "Periodic rotation failed"
                        );
                    }
                }
                () = shutdown.cancelled() => {
                    tracing::info!(
                        credential_id = %credential_id,
                        "Rotation loop shut down"
                    );
                    return Ok(());
                }
            }
        }
    }
}

/// TTL monitor that selects credentials which reached rotation threshold.
pub struct ExpiryMonitor {
    config: BeforeExpiryConfig,
}

impl ExpiryMonitor {
    /// Build an expiry monitor from policy config.
    #[must_use]
    pub fn new(config: BeforeExpiryConfig) -> Self {
        Self { config }
    }

    /// Calculate trigger time for a credential lifecycle.
    #[must_use]
    pub fn calculate_rotation_trigger_time(
        &self,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let total_ttl = expires_at - created_at;
        let threshold_duration = match total_ttl.to_std() {
            Ok(duration) => duration.mul_f64(self.config.threshold_percentage() as f64),
            Err(_) => Duration::ZERO,
        };

        let threshold_trigger = created_at
            + chrono::Duration::from_std(threshold_duration).unwrap_or(chrono::Duration::zero());

        let minimum_trigger = expires_at
            - chrono::Duration::from_std(self.config.minimum_time_before_expiry())
                .unwrap_or(chrono::Duration::zero());

        std::cmp::min(threshold_trigger, minimum_trigger)
    }

    /// Return credential ids that should rotate now.
    #[must_use]
    pub fn check_credentials(
        &self,
        credentials: &[(CredentialId, DateTime<Utc>, DateTime<Utc>)],
    ) -> Vec<CredentialId> {
        let now = Utc::now();
        credentials
            .iter()
            .filter_map(|(id, created_at, expires_at)| {
                let trigger = self.calculate_rotation_trigger_time(*created_at, *expires_at);
                (now >= trigger).then_some(*id)
            })
            .collect()
    }
}

/// One-time scheduled rotation helper.
pub struct ScheduledRotation {
    config: ScheduledConfig,
}

impl ScheduledRotation {
    /// Build a one-time scheduled rotation helper.
    #[must_use]
    pub fn new(config: ScheduledConfig) -> Self {
        Self { config }
    }

    /// Returns the configured rotation timestamp.
    #[must_use]
    pub fn schedule_at(&self) -> DateTime<Utc> {
        self.config.scheduled_at()
    }

    /// Returns when to notify prior to rotation, if configured.
    #[must_use]
    pub fn notification_time(&self) -> Option<DateTime<Utc>> {
        self.config.notify_before().map(|notify_before| {
            self.config.scheduled_at()
                - chrono::Duration::from_std(notify_before).unwrap_or(chrono::Duration::zero())
        })
    }

    /// Returns true when notification should be emitted now.
    #[must_use]
    pub fn should_notify_now(&self) -> bool {
        self.notification_time()
            .map(|notify_at| Utc::now() >= notify_at)
            .unwrap_or(false)
    }

    /// Returns true when scheduled rotation should fire now.
    #[must_use]
    pub fn should_rotate_now(&self) -> bool {
        Utc::now() >= self.config.scheduled_at()
    }

    /// Returns signed duration until the scheduled rotation point.
    #[must_use]
    pub fn time_until_rotation(&self) -> chrono::Duration {
        self.config.scheduled_at() - Utc::now()
    }
}

fn apply_jitter(base: Duration) -> Duration {
    use rand::RngExt;

    /// `Duration::from_nanos` upper bound — avoids `from_secs_f64` panics on huge intervals.
    const MAX_NS: u128 = u64::MAX as u128;

    let base_ns = base.as_nanos().min(MAX_NS);
    if base_ns == 0 {
        return Duration::ZERO;
    }
    let min_ns = (base_ns.saturating_mul(9) / 10).max(1);
    let max_ns = (base_ns.saturating_mul(11) / 10).max(min_ns).min(MAX_NS);
    let jittered_ns = rand::rng().random_range(min_ns..=max_ns);
    let jittered_u64 = u64::try_from(jittered_ns).unwrap_or(u64::MAX);
    Duration::from_nanos(jittered_u64)
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;

    use super::*;

    #[test]
    fn jitter_stays_within_ten_percent_window() {
        let base = Duration::from_secs(90 * 24 * 3600);
        let min_allowed = (base.as_secs_f64() * 0.9) as u64;
        let max_allowed = (base.as_secs_f64() * 1.1) as u64;

        for _ in 0..100 {
            let jittered = apply_jitter(base);
            let secs = jittered.as_secs();
            assert!(secs >= min_allowed && secs <= max_allowed);
        }
    }

    #[test]
    fn periodic_scheduler_respects_interval_without_jitter() {
        let interval = Duration::from_secs(3600);
        let config = PeriodicConfig::new(interval, Duration::from_secs(60), false)
            .expect("valid periodic config");
        let scheduler = PeriodicScheduler::new(config);

        let now = Instant::now();
        let next = scheduler.schedule_rotation();
        assert!(next > now);
    }

    #[test]
    fn expiry_monitor_picks_conservative_trigger_time() {
        let config = BeforeExpiryConfig::new(
            0.9,
            Duration::from_secs(10 * 24 * 3600),
            Duration::from_secs(600),
        )
        .expect("valid before-expiry config");
        let monitor = ExpiryMonitor::new(config);

        let created = Utc::now();
        let expires = created + ChronoDuration::days(30);
        let trigger = monitor.calculate_rotation_trigger_time(created, expires);

        // 90% trigger would be day 27; minimum-time trigger is day 20.
        let expected = expires - ChronoDuration::days(10);
        let diff = if trigger > expected {
            trigger - expected
        } else {
            expected - trigger
        };
        assert!(diff < ChronoDuration::seconds(1));
    }

    #[test]
    fn expiry_monitor_filters_credentials_ready_for_rotation() {
        let config =
            BeforeExpiryConfig::new(0.8, Duration::from_secs(3600), Duration::from_secs(600))
                .expect("valid before-expiry config");
        let monitor = ExpiryMonitor::new(config);
        let now = Utc::now();

        let id_due = CredentialId::new();
        let id_not_due = CredentialId::new();
        let credentials = vec![
            (
                id_due,
                now - ChronoDuration::days(25),
                now + ChronoDuration::days(5),
            ),
            (
                id_not_due,
                now - ChronoDuration::days(5),
                now + ChronoDuration::days(25),
            ),
        ];
        let due = monitor.check_credentials(&credentials);

        assert!(due.contains(&id_due));
        assert!(!due.contains(&id_not_due));
    }

    #[test]
    fn scheduled_rotation_notification_and_rotation_checks() {
        let scheduled_at = Utc::now() + ChronoDuration::hours(24);
        let notify_before = Duration::from_secs(3600);
        let config =
            ScheduledConfig::new(scheduled_at, Duration::from_secs(600), Some(notify_before))
                .expect("valid scheduled config");
        let rotation = ScheduledRotation::new(config);

        assert_eq!(rotation.schedule_at(), scheduled_at);
        assert!(rotation.notification_time().is_some());
        assert!(!rotation.should_rotate_now());
        assert!(rotation.time_until_rotation() > ChronoDuration::hours(23));
    }
}
