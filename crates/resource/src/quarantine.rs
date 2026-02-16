//! Quarantine management for unhealthy resources.
//!
//! When a resource exceeds the failure threshold, it is quarantined (removed
//! from the active pool) and scheduled for recovery attempts with exponential
//! backoff.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;

// ---------------------------------------------------------------------------
// QuarantineReason
// ---------------------------------------------------------------------------

/// Why a resource was quarantined.
#[derive(Debug, Clone)]
pub enum QuarantineReason {
    /// Consecutive health checks failed past the threshold.
    HealthCheckFailed {
        /// How many consecutive failures triggered quarantine.
        consecutive_failures: u32,
    },
    /// An operator or automated system explicitly quarantined the resource.
    ManualQuarantine {
        /// Human-readable reason.
        reason: String,
    },
}

impl std::fmt::Display for QuarantineReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HealthCheckFailed {
                consecutive_failures,
            } => write!(
                f,
                "health check failed ({consecutive_failures} consecutive)"
            ),
            Self::ManualQuarantine { reason } => write!(f, "manual: {reason}"),
        }
    }
}

// ---------------------------------------------------------------------------
// QuarantineEntry
// ---------------------------------------------------------------------------

/// A single quarantined resource.
#[derive(Debug, Clone)]
pub struct QuarantineEntry {
    /// The resource identifier.
    pub resource_id: String,
    /// Why the resource was quarantined.
    pub reason: QuarantineReason,
    /// When the resource was quarantined.
    pub quarantined_at: DateTime<Utc>,
    /// How many recovery attempts have been made.
    pub recovery_attempts: u32,
    /// Maximum number of recovery attempts before giving up.
    pub max_recovery_attempts: u32,
    /// When the next recovery attempt should be scheduled.
    pub next_recovery_at: Option<DateTime<Utc>>,
}

impl QuarantineEntry {
    /// Whether the entry has exhausted all recovery attempts.
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.recovery_attempts >= self.max_recovery_attempts
    }

    /// Record a failed recovery attempt and compute the next retry time.
    pub fn record_failed_recovery(&mut self, strategy: &RecoveryStrategy) {
        self.recovery_attempts += 1;
        if !self.is_exhausted() {
            let delay = strategy.delay_for(self.recovery_attempts);
            self.next_recovery_at = Some(
                Utc::now()
                    + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::seconds(60)),
            );
        } else {
            self.next_recovery_at = None;
        }
    }
}

// ---------------------------------------------------------------------------
// RecoveryStrategy
// ---------------------------------------------------------------------------

/// Strategy for scheduling recovery attempts.
#[derive(Debug, Clone)]
pub struct RecoveryStrategy {
    /// Base delay for the first retry (default: 1s).
    pub base_delay: Duration,
    /// Maximum delay cap (default: 60s).
    pub max_delay: Duration,
    /// Multiplier per attempt (default: 2.0 for exponential backoff).
    pub multiplier: f64,
}

impl Default for RecoveryStrategy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
        }
    }
}

impl RecoveryStrategy {
    /// Calculate the delay for a given attempt number (1-based).
    #[must_use]
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1);
        let factor = self.multiplier.powi(exponent as i32);
        let delay_secs = self.base_delay.as_secs_f64() * factor;
        let capped = delay_secs.min(self.max_delay.as_secs_f64());
        Duration::from_secs_f64(capped)
    }
}

// ---------------------------------------------------------------------------
// QuarantineConfig
// ---------------------------------------------------------------------------

/// Configuration for the quarantine manager.
#[derive(Debug, Clone)]
pub struct QuarantineConfig {
    /// Number of consecutive health failures before quarantining.
    pub failure_threshold: u32,
    /// Maximum number of recovery attempts before giving up.
    pub max_recovery_attempts: u32,
    /// Recovery backoff strategy.
    pub recovery_strategy: RecoveryStrategy,
}

impl Default for QuarantineConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            max_recovery_attempts: 5,
            recovery_strategy: RecoveryStrategy::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// QuarantineManager
// ---------------------------------------------------------------------------

/// Manages quarantined resources with recovery scheduling.
///
/// Thread-safe via `DashMap`. Resources are quarantined when they exceed the
/// failure threshold and released either on successful recovery or manual
/// intervention.
#[derive(Debug)]
pub struct QuarantineManager {
    entries: DashMap<String, QuarantineEntry>,
    config: QuarantineConfig,
}

impl QuarantineManager {
    /// Create a new quarantine manager with the given configuration.
    #[must_use]
    pub fn new(config: QuarantineConfig) -> Self {
        Self {
            entries: DashMap::new(),
            config,
        }
    }

    /// The failure threshold from config.
    #[must_use]
    pub fn failure_threshold(&self) -> u32 {
        self.config.failure_threshold
    }

    /// The recovery strategy from config.
    #[must_use]
    pub fn recovery_strategy(&self) -> &RecoveryStrategy {
        &self.config.recovery_strategy
    }

    /// Quarantine a resource.
    ///
    /// If the resource is already quarantined, this is a no-op.
    /// Returns `true` if the resource was newly quarantined.
    pub fn quarantine(&self, resource_id: &str, reason: QuarantineReason) -> bool {
        if self.entries.contains_key(resource_id) {
            return false;
        }

        let delay = self.config.recovery_strategy.delay_for(1);
        let next =
            Utc::now() + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::seconds(60));

        self.entries.insert(
            resource_id.to_string(),
            QuarantineEntry {
                resource_id: resource_id.to_string(),
                reason,
                quarantined_at: Utc::now(),
                recovery_attempts: 0,
                max_recovery_attempts: self.config.max_recovery_attempts,
                next_recovery_at: Some(next),
            },
        );

        true
    }

    /// Release a resource from quarantine.
    ///
    /// Returns the entry if it was quarantined, `None` otherwise.
    pub fn release(&self, resource_id: &str) -> Option<QuarantineEntry> {
        self.entries.remove(resource_id).map(|(_, entry)| entry)
    }

    /// Check if a resource is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, resource_id: &str) -> bool {
        self.entries.contains_key(resource_id)
    }

    /// Get the quarantine entry for a resource (if quarantined).
    #[must_use]
    pub fn get(&self, resource_id: &str) -> Option<QuarantineEntry> {
        self.entries.get(resource_id).map(|e| e.value().clone())
    }

    /// Record a failed recovery attempt for a quarantined resource.
    ///
    /// Returns `true` if the entry was updated, `false` if the resource
    /// is not quarantined.
    pub fn record_failed_recovery(&self, resource_id: &str) -> bool {
        let Some(mut entry) = self.entries.get_mut(resource_id) else {
            return false;
        };
        entry.record_failed_recovery(&self.config.recovery_strategy);
        true
    }

    /// Get all quarantined resource IDs.
    #[must_use]
    pub fn quarantined_ids(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.key().clone()).collect()
    }

    /// Get all quarantine entries.
    #[must_use]
    pub fn entries(&self) -> Vec<QuarantineEntry> {
        self.entries.iter().map(|e| e.value().clone()).collect()
    }

    /// Get entries that are due for a recovery attempt.
    #[must_use]
    pub fn due_for_recovery(&self) -> Vec<QuarantineEntry> {
        let now = Utc::now();
        self.entries
            .iter()
            .filter(|e| {
                let entry = e.value();
                !entry.is_exhausted() && entry.next_recovery_at.is_some_and(|next| next <= now)
            })
            .map(|e| e.value().clone())
            .collect()
    }

    /// Number of quarantined resources.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether any resources are quarantined.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for QuarantineManager {
    fn default() -> Self {
        Self::new(QuarantineConfig::default())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_strategy_exponential_backoff() {
        let strategy = RecoveryStrategy::default();
        // base_delay=1s, multiplier=2.0
        assert_eq!(strategy.delay_for(1), Duration::from_secs(1)); // 1 * 2^0 = 1
        assert_eq!(strategy.delay_for(2), Duration::from_secs(2)); // 1 * 2^1 = 2
        assert_eq!(strategy.delay_for(3), Duration::from_secs(4)); // 1 * 2^2 = 4
        assert_eq!(strategy.delay_for(4), Duration::from_secs(8)); // 1 * 2^3 = 8
    }

    #[test]
    fn recovery_strategy_caps_at_max_delay() {
        let strategy = RecoveryStrategy {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
        };
        // 1 * 2^5 = 32, capped to 10
        assert_eq!(strategy.delay_for(6), Duration::from_secs(10));
        assert_eq!(strategy.delay_for(20), Duration::from_secs(10));
    }

    #[test]
    fn quarantine_and_release() {
        let qm = QuarantineManager::default();

        assert!(!qm.is_quarantined("db"));
        assert!(qm.is_empty());

        let added = qm.quarantine(
            "db",
            QuarantineReason::HealthCheckFailed {
                consecutive_failures: 3,
            },
        );
        assert!(added);
        assert!(qm.is_quarantined("db"));
        assert_eq!(qm.len(), 1);

        // Duplicate quarantine is a no-op
        let added_again = qm.quarantine(
            "db",
            QuarantineReason::HealthCheckFailed {
                consecutive_failures: 5,
            },
        );
        assert!(!added_again);
        assert_eq!(qm.len(), 1);

        let entry = qm.release("db").expect("should have been quarantined");
        assert_eq!(entry.resource_id, "db");
        assert!(!qm.is_quarantined("db"));
        assert!(qm.is_empty());
    }

    #[test]
    fn release_nonexistent_returns_none() {
        let qm = QuarantineManager::default();
        assert!(qm.release("nope").is_none());
    }

    #[test]
    fn record_failed_recovery_increments_attempts() {
        let config = QuarantineConfig {
            failure_threshold: 3,
            max_recovery_attempts: 3,
            recovery_strategy: RecoveryStrategy::default(),
        };
        let qm = QuarantineManager::new(config);

        qm.quarantine(
            "db",
            QuarantineReason::HealthCheckFailed {
                consecutive_failures: 3,
            },
        );

        assert!(qm.record_failed_recovery("db"));
        let entry = qm.get("db").unwrap();
        assert_eq!(entry.recovery_attempts, 1);
        assert!(!entry.is_exhausted());

        assert!(qm.record_failed_recovery("db"));
        assert!(qm.record_failed_recovery("db"));
        let entry = qm.get("db").unwrap();
        assert_eq!(entry.recovery_attempts, 3);
        assert!(entry.is_exhausted());
        assert!(entry.next_recovery_at.is_none());
    }

    #[test]
    fn record_failed_recovery_on_nonexistent_returns_false() {
        let qm = QuarantineManager::default();
        assert!(!qm.record_failed_recovery("nope"));
    }

    #[test]
    fn quarantined_ids_and_entries() {
        let qm = QuarantineManager::default();

        qm.quarantine(
            "a",
            QuarantineReason::ManualQuarantine {
                reason: "test".into(),
            },
        );
        qm.quarantine(
            "b",
            QuarantineReason::HealthCheckFailed {
                consecutive_failures: 5,
            },
        );

        let mut ids = qm.quarantined_ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);

        let entries = qm.entries();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn quarantine_reason_display() {
        let r1 = QuarantineReason::HealthCheckFailed {
            consecutive_failures: 5,
        };
        assert!(r1.to_string().contains("5 consecutive"));

        let r2 = QuarantineReason::ManualQuarantine {
            reason: "maintenance".into(),
        };
        assert!(r2.to_string().contains("maintenance"));
    }

    #[test]
    fn quarantine_entry_exhausted_check() {
        let mut entry = QuarantineEntry {
            resource_id: "db".into(),
            reason: QuarantineReason::HealthCheckFailed {
                consecutive_failures: 3,
            },
            quarantined_at: Utc::now(),
            recovery_attempts: 0,
            max_recovery_attempts: 2,
            next_recovery_at: None,
        };

        assert!(!entry.is_exhausted());
        entry.recovery_attempts = 1;
        assert!(!entry.is_exhausted());
        entry.recovery_attempts = 2;
        assert!(entry.is_exhausted());
    }
}
