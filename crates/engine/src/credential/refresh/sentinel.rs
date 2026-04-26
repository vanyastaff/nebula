//! Sentinel mid-refresh crash detection + threshold escalation.
//!
//! Per sub-spec
//! `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
//! §3.4 + §6 audit.
//!
//! When a holder is about to perform the IdP POST (the operation that
//! risks invalidating the refresh token if not persisted), it marks the
//! claim as `SentinelState::RefreshInFlight` via
//! [`RefreshClaimRepo::mark_sentinel`]. On successful release the claim
//! row is **deleted** entirely (via
//! [`RefreshClaimRepo::release`]) so the sentinel clears by removal —
//! no separate "clear sentinel" call is needed on the success path.
//! If the reclaim sweep finds an expired claim still flagged
//! `RefreshInFlight`, the holder is presumed crashed mid-refresh.
//!
//! N=3 sentinel events within 1h (default) escalate the credential to
//! `ReauthRequired`. Stage 3 lands the threshold-counting + state-
//! transition logic; this Stage 2.4 skeleton just establishes the type
//! shape so callers can plumb it through.

use std::{sync::Arc, time::Duration};

use nebula_storage::credential::RefreshClaimRepo;

/// Configuration for the sentinel threshold logic. Default = 3-in-1h.
#[derive(Clone, Debug)]
pub struct SentinelThresholdConfig {
    /// Sentinel events within `window` before escalation to
    /// `ReauthRequired`.
    pub threshold: u32,
    /// Rolling window over which `threshold` events are counted.
    pub window: Duration,
}

impl Default for SentinelThresholdConfig {
    fn default() -> Self {
        Self {
            threshold: 3,
            window: Duration::from_hours(1),
        }
    }
}

/// Tracks sentinel events per credential; emits escalations when the
/// rolling-window threshold is reached.
///
/// Stage 2.4 skeleton — actual recording + threshold logic lands in
/// Stage 3. The type exists here so the engine composition root can
/// thread it through the same way as the coordinator.
pub struct SentinelTrigger {
    repo: Arc<dyn RefreshClaimRepo>,
    config: SentinelThresholdConfig,
}

impl SentinelTrigger {
    /// Create a trigger wired to the given `RefreshClaimRepo` and
    /// threshold configuration.
    pub fn new(repo: Arc<dyn RefreshClaimRepo>, config: SentinelThresholdConfig) -> Self {
        Self { repo, config }
    }

    /// Borrow the threshold configuration.
    #[must_use]
    pub fn config(&self) -> &SentinelThresholdConfig {
        &self.config
    }

    /// Borrow the repo handle (consumed by Stage 3 reclaim-sweep
    /// integration).
    #[allow(
        dead_code,
        reason = "consumed by the threshold counter landing in Stage 3.2"
    )]
    pub(crate) fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
        &self.repo
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_storage::credential::InMemoryRefreshClaimRepo;

    use super::*;

    #[test]
    fn default_threshold_is_three_in_one_hour() {
        let cfg = SentinelThresholdConfig::default();
        assert_eq!(cfg.threshold, 3);
        assert_eq!(cfg.window, Duration::from_hours(1));
    }

    #[test]
    fn trigger_construction_round_trips_config() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let cfg = SentinelThresholdConfig {
            threshold: 5,
            window: Duration::from_mins(10),
        };
        let trigger = SentinelTrigger::new(repo, cfg.clone());
        assert_eq!(trigger.config().threshold, cfg.threshold);
        assert_eq!(trigger.config().window, cfg.window);
    }
}
