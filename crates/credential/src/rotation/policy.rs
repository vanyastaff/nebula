//! Rotation Policy Types
//!
//! Defines when and how credentials should be rotated.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::error::{RotationError, RotationResult};

/// Rotation policy defining when to rotate credentials
///
/// # T102: Policy Usage Examples
///
/// Supports four rotation strategies with different use cases:
///
/// # Examples
///
/// ## Periodic Rotation (Compliance)
///
/// Rotate credentials every 90 days for PCI-DSS compliance:
///
/// ```
/// use nebula_credential::rotation::{RotationPolicy, PeriodicConfig};
/// use std::time::Duration;
///
/// let policy = RotationPolicy::Periodic(PeriodicConfig::new(
///     Duration::from_secs(90 * 24 * 3600), // 90 days
///     Duration::from_secs(24 * 3600),      // 24 hours grace
///     true,                                // enable jitter
/// ).unwrap());
/// ```
///
/// ## Before Expiry (OAuth Tokens)
///
/// Rotate OAuth tokens at 80% of TTL to prevent expiration:
///
/// ```
/// use nebula_credential::rotation::{RotationPolicy, BeforeExpiryConfig};
/// use std::time::Duration;
///
/// let policy = RotationPolicy::BeforeExpiry(BeforeExpiryConfig::new(
///     0.80,                              // Rotate at 80% TTL
///     Duration::from_secs(3600),         // Don't rotate if TTL < 1 hour
///     Duration::from_secs(600),          // 10 minutes grace period
/// ).unwrap());
/// ```
///
/// ## Scheduled (Maintenance Window)
///
/// Rotate during planned maintenance with notifications:
///
/// ```
/// use nebula_credential::rotation::{RotationPolicy, ScheduledConfig};
/// use chrono::{Utc, Duration};
/// use std::time::Duration as StdDuration;
///
/// let policy = RotationPolicy::Scheduled(ScheduledConfig::new(
///     Utc::now() + Duration::days(7),
///     StdDuration::from_secs(3600),                     // 1 hour grace period
///     Some(StdDuration::from_secs(24 * 3600)),          // Notify 24h before
/// ).unwrap());
/// ```
///
/// ## Manual (Security Incident)
///
/// Emergency rotation with immediate revocation:
///
/// ```
/// use nebula_credential::rotation::{RotationPolicy, ManualConfig};
///
/// // Emergency: revoke immediately
/// let emergency = RotationPolicy::Manual(ManualConfig::emergency());
///
/// // Planned manual rotation with grace period
/// use std::time::Duration;
/// let planned = RotationPolicy::Manual(
///     ManualConfig::planned(Duration::from_secs(3600)) // 1 hour grace
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RotationPolicy {
    /// Rotate at fixed intervals (e.g., every 90 days)
    Periodic(PeriodicConfig),

    /// Rotate when credential approaches expiration (e.g., at 80% TTL)
    BeforeExpiry(BeforeExpiryConfig),

    /// Rotate at specific date/time (e.g., maintenance window)
    Scheduled(ScheduledConfig),

    /// Rotate on-demand (manual trigger for emergency incidents)
    Manual(ManualConfig),
}

/// Configuration for periodic rotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeriodicConfig {
    /// Rotation interval (minimum 1 hour)
    interval: Duration,

    /// Grace period where both old and new credentials are valid
    grace_period: Duration,

    /// Enable jitter (±10% randomization) to prevent rotation storms
    enable_jitter: bool,
}

/// Configuration for before-expiry rotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BeforeExpiryConfig {
    /// Rotate when credential reaches this percentage of TTL (0.5-0.95)
    threshold_percentage: f32,

    /// Minimum time before expiry to trigger rotation (safety buffer)
    minimum_time_before_expiry: Duration,

    /// Grace period where both old and new credentials are valid
    grace_period: Duration,
}

/// Configuration for scheduled rotation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduledConfig {
    /// Exact time to perform rotation
    scheduled_at: DateTime<Utc>,

    /// Grace period where both old and new credentials are valid
    grace_period: Duration,

    /// Send notification this long before rotation (e.g., 24 hours)
    notify_before: Option<Duration>,
}

/// Configuration for manual rotation (emergency incident response)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManualConfig {
    /// Immediately revoke old credential (skip grace period)
    ///
    /// Use true for security incidents where compromised credentials
    /// must be invalidated immediately. Use false for planned manual
    /// rotations where grace period is desired.
    immediate_revoke: bool,

    /// Grace period (only used if immediate_revoke is false)
    ///
    /// Duration where both old and new credentials remain valid.
    /// Ignored when immediate_revoke is true.
    grace_period: Option<Duration>,
}

impl ManualConfig {
    /// Check if immediate revocation is enabled
    pub fn immediate_revoke(&self) -> bool {
        self.immediate_revoke
    }

    /// Get grace period (if not immediate revocation)
    pub fn grace_period(&self) -> Option<Duration> {
        self.grace_period
    }

    /// Create configuration for emergency rotation (immediate revocation)
    pub fn emergency() -> Self {
        Self {
            immediate_revoke: true,
            grace_period: None,
        }
    }

    /// Create configuration for planned manual rotation (with grace period)
    pub fn planned(grace_period: Duration) -> Self {
        Self {
            immediate_revoke: false,
            grace_period: Some(grace_period),
        }
    }
}

impl RotationPolicy {
    /// Validate the rotation policy configuration
    pub fn validate(&self) -> RotationResult<()> {
        match self {
            RotationPolicy::Periodic(config) => config.validate(),
            RotationPolicy::BeforeExpiry(config) => config.validate(),
            RotationPolicy::Scheduled(config) => config.validate(),
            RotationPolicy::Manual(_) => Ok(()), // Manual rotation has no validation constraints
        }
    }

    /// Get the grace period for this policy
    pub fn grace_period(&self) -> Option<Duration> {
        match self {
            RotationPolicy::Periodic(config) => Some(config.grace_period),
            RotationPolicy::BeforeExpiry(config) => Some(config.grace_period),
            RotationPolicy::Scheduled(config) => Some(config.grace_period),
            RotationPolicy::Manual(config) => {
                // Return grace period only if not immediate revocation
                if config.immediate_revoke {
                    None
                } else {
                    config.grace_period
                }
            }
        }
    }
}

impl PeriodicConfig {
    /// Get rotation interval
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Get grace period
    pub fn grace_period(&self) -> Duration {
        self.grace_period
    }

    /// Check if jitter is enabled
    pub fn enable_jitter(&self) -> bool {
        self.enable_jitter
    }

    /// Create a new periodic rotation configuration with validation
    ///
    /// # Arguments
    ///
    /// * `interval` - Rotation interval (minimum 1 hour)
    /// * `grace_period` - Grace period where both credentials are valid
    /// * `enable_jitter` - Enable ±10% jitter to prevent rotation storms
    ///
    /// # Returns
    ///
    /// * `RotationResult<Self>` - Ok if configuration is valid
    ///
    /// # Errors
    ///
    /// * `InvalidPolicy` if interval < 1 hour or grace_period > interval
    ///
    /// # Example
    ///
    /// ```
    /// use nebula_credential::rotation::PeriodicConfig;
    /// use std::time::Duration;
    ///
    /// let config = PeriodicConfig::new(
    ///     Duration::from_secs(90 * 24 * 3600), // 90 days
    ///     Duration::from_secs(24 * 3600),      // 24 hours
    ///     true,
    /// ).unwrap();
    /// ```
    pub fn new(
        interval: Duration,
        grace_period: Duration,
        enable_jitter: bool,
    ) -> RotationResult<Self> {
        let config = Self {
            interval,
            grace_period,
            enable_jitter,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate periodic rotation configuration
    pub fn validate(&self) -> RotationResult<()> {
        // Interval must be at least 1 hour
        if self.interval < Duration::from_secs(3600) {
            return Err(RotationError::InvalidPolicy {
                reason: format!(
                    "Rotation interval must be at least 1 hour, got {:?}",
                    self.interval
                ),
            });
        }

        // Grace period cannot exceed rotation interval
        if self.grace_period > self.interval {
            return Err(RotationError::InvalidPolicy {
                reason: format!(
                    "Grace period ({:?}) cannot exceed rotation interval ({:?})",
                    self.grace_period, self.interval
                ),
            });
        }

        Ok(())
    }
}

impl BeforeExpiryConfig {
    /// Get threshold percentage
    pub fn threshold_percentage(&self) -> f32 {
        self.threshold_percentage
    }

    /// Get minimum time before expiry
    pub fn minimum_time_before_expiry(&self) -> Duration {
        self.minimum_time_before_expiry
    }

    /// Get grace period
    pub fn grace_period(&self) -> Duration {
        self.grace_period
    }

    /// Create a new before-expiry rotation configuration with validation
    ///
    /// # Arguments
    ///
    /// * `threshold_percentage` - Rotate at this percentage of TTL (0.5-0.95)
    /// * `minimum_time_before_expiry` - Safety buffer before expiry
    /// * `grace_period` - Grace period where both credentials are valid
    ///
    /// # Returns
    ///
    /// * `RotationResult<Self>` - Ok if configuration is valid
    ///
    /// # Errors
    ///
    /// * `InvalidPolicy` if threshold not in range [0.5, 0.95] or minimum_time is zero
    ///
    /// # Example
    ///
    /// ```
    /// use nebula_credential::rotation::BeforeExpiryConfig;
    /// use std::time::Duration;
    ///
    /// let config = BeforeExpiryConfig::new(
    ///     0.80, // Rotate at 80% TTL
    ///     Duration::from_secs(3600),  // 1 hour minimum
    ///     Duration::from_secs(600),   // 10 minutes grace
    /// ).unwrap();
    /// ```
    pub fn new(
        threshold_percentage: f32,
        minimum_time_before_expiry: Duration,
        grace_period: Duration,
    ) -> RotationResult<Self> {
        let config = Self {
            threshold_percentage,
            minimum_time_before_expiry,
            grace_period,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate before-expiry rotation configuration
    pub fn validate(&self) -> RotationResult<()> {
        // Threshold must be between 50% and 95%
        if !(0.5..=0.95).contains(&self.threshold_percentage) {
            return Err(RotationError::InvalidPolicy {
                reason: format!(
                    "Threshold percentage must be between 0.5 and 0.95, got {}",
                    self.threshold_percentage
                ),
            });
        }

        // Minimum time before expiry must be positive
        if self.minimum_time_before_expiry.is_zero() {
            return Err(RotationError::InvalidPolicy {
                reason: "Minimum time before expiry must be positive".to_string(),
            });
        }

        Ok(())
    }
}

impl ScheduledConfig {
    /// Get scheduled rotation time
    pub fn scheduled_at(&self) -> DateTime<Utc> {
        self.scheduled_at
    }

    /// Get grace period
    pub fn grace_period(&self) -> Duration {
        self.grace_period
    }

    /// Get notification lead time
    pub fn notify_before(&self) -> Option<Duration> {
        self.notify_before
    }

    /// Create a new scheduled rotation configuration without validation (for tests)
    #[cfg(test)]
    pub fn new_unchecked(
        scheduled_at: DateTime<Utc>,
        grace_period: Duration,
        notify_before: Option<Duration>,
    ) -> Self {
        Self {
            scheduled_at,
            grace_period,
            notify_before,
        }
    }

    /// Create a new scheduled rotation configuration with validation
    ///
    /// # Arguments
    ///
    /// * `scheduled_at` - Exact time to perform rotation (must be in future)
    /// * `grace_period` - Grace period where both credentials are valid
    /// * `notify_before` - Optional notification lead time
    ///
    /// # Returns
    ///
    /// * `RotationResult<Self>` - Ok if configuration is valid
    ///
    /// # Errors
    ///
    /// * `InvalidPolicy` if scheduled_at is in the past
    ///
    /// # Example
    ///
    /// ```
    /// use nebula_credential::rotation::ScheduledConfig;
    /// use chrono::{Utc, Duration as ChronoDuration};
    /// use std::time::Duration;
    ///
    /// let config = ScheduledConfig::new(
    ///     Utc::now() + ChronoDuration::days(7),
    ///     Duration::from_secs(3600),
    ///     Some(Duration::from_secs(24 * 3600)), // Notify 24h before
    /// ).unwrap();
    /// ```
    pub fn new(
        scheduled_at: DateTime<Utc>,
        grace_period: Duration,
        notify_before: Option<Duration>,
    ) -> RotationResult<Self> {
        let config = Self {
            scheduled_at,
            grace_period,
            notify_before,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate scheduled rotation configuration
    pub fn validate(&self) -> RotationResult<()> {
        // Scheduled time must be in the future
        if self.scheduled_at <= Utc::now() {
            return Err(RotationError::InvalidPolicy {
                reason: format!(
                    "Scheduled time must be in the future, got {}",
                    self.scheduled_at
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_periodic_validation() {
        // Valid configuration using constructor
        let valid = PeriodicConfig::new(
            Duration::from_secs(90 * 24 * 60 * 60), // 90 days
            Duration::from_secs(24 * 60 * 60),      // 24 hours
            true,
        );
        assert!(valid.is_ok());

        // Invalid: interval too short
        let invalid_interval = PeriodicConfig::new(
            Duration::from_secs(1800), // 30 minutes
            Duration::from_secs(600),
            false,
        );
        assert!(invalid_interval.is_err());
        assert!(
            invalid_interval
                .unwrap_err()
                .to_string()
                .contains("at least 1 hour")
        );

        // Invalid: grace period exceeds interval
        let invalid_grace =
            PeriodicConfig::new(Duration::from_secs(3600), Duration::from_secs(7200), false);
        assert!(invalid_grace.is_err());
        assert!(
            invalid_grace
                .unwrap_err()
                .to_string()
                .contains("cannot exceed")
        );
    }

    #[test]
    fn test_before_expiry_validation() {
        // Valid configuration using constructor
        let valid = BeforeExpiryConfig::new(
            0.80,
            Duration::from_secs(5 * 60),  // 5 minutes
            Duration::from_secs(10 * 60), // 10 minutes
        );
        assert!(valid.is_ok());

        // Invalid: threshold too low
        let invalid_threshold_low =
            BeforeExpiryConfig::new(0.3, Duration::from_secs(300), Duration::from_secs(600));
        assert!(invalid_threshold_low.is_err());
        assert!(
            invalid_threshold_low
                .unwrap_err()
                .to_string()
                .contains("between 0.5 and 0.95")
        );

        // Invalid: threshold too high
        let invalid_threshold_high =
            BeforeExpiryConfig::new(0.99, Duration::from_secs(300), Duration::from_secs(600));
        assert!(invalid_threshold_high.is_err());
        assert!(
            invalid_threshold_high
                .unwrap_err()
                .to_string()
                .contains("between 0.5 and 0.95")
        );

        // Invalid: zero minimum time
        let invalid_min_time =
            BeforeExpiryConfig::new(0.80, Duration::ZERO, Duration::from_secs(600));
        assert!(invalid_min_time.is_err());
        assert!(
            invalid_min_time
                .unwrap_err()
                .to_string()
                .contains("must be positive")
        );
    }
}
