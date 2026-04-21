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
                _ = sleep_until(next) => {
                    if let Err(error) = rotation_fn(credential_id).await {
                        tracing::error!(
                            credential_id = %credential_id,
                            error = %error,
                            "Periodic rotation failed"
                        );
                    }
                }
                _ = shutdown.cancelled() => {
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

    let base_secs = base.as_secs_f64();
    let min_secs = base_secs * 0.9;
    let max_secs = base_secs * 1.1;
    let jittered_secs = rand::rng().random_range(min_secs..=max_secs);
    Duration::from_secs_f64(jittered_secs)
}
