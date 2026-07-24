//! Sentinel mid-refresh crash detection + threshold escalation.
//!
//! Per sub-spec
//! `docs/INTEGRATION_MODEL.md`
//! + audit.
//!
//! When a holder is about to perform the IdP POST (the operation that
//! risks invalidating the refresh token if not persisted), it marks the
//! claim as `SentinelState::RefreshInFlight` via
//! [`RefreshClaimRepo::mark_sentinel`]. On successful release the claim
//! row is **deleted** entirely (via
//! [`RefreshClaimRepo::release`]) so the sentinel clears by removal --
//! no separate "clear sentinel" call is needed on the success path.
//! If the reclaim sweep finds an expired claim still flagged
//! `RefreshInFlight`, the holder is presumed crashed mid-refresh. The
//! repository atomically records one event and retains that claim as durable
//! fail-closed poison; threshold evaluation never authorizes provider replay.
//!
//! N=3 distinct, explicitly reconciled incidents within 1h (default) produce
//! an `EscalateToReauth` decision. Repeated denied requests or sweeps of one
//! poisoned claim do not consume the budget. The implementation records each
//! newly-accounted claim UUID in `credential_sentinel_events` and consults the
//! database-clock-authoritative rolling-window count via
//! `RefreshClaimStore::count_sentinel_events_in_window`. The current
//! reclaim sweep publishes a lossy observation for that decision; it
//! does not durably mutate the credential aggregate.

use std::{sync::Arc, time::Duration};

use nebula_storage_port::store::{
    RefreshClaimError as RepoError, RefreshClaimStore as RefreshClaimRepo,
};

/// Configuration for the distinct-incident threshold. Default = 3-in-1h.
#[derive(Clone, Debug)]
pub struct SentinelThresholdConfig {
    /// Newly-accounted claim incidents within `window` before escalation to
    /// `ReauthRequired`. Repeated sweeps of one incident count once.
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

/// Decision returned after an expired provider-side-effect claim has been
/// durably accounted.
///
/// `BelowThreshold` means only that escalation has not yet been reached; the
/// claim remains poison and provider replay is still forbidden.
/// `EscalateToReauth` means the threshold was met or exceeded -- the aggregate
/// owner must durably transition the credential to `ReauthRequired`. A
/// `nebula_credential::CredentialEvent::ReauthRequired` may accompany
/// that command as an observation, but cannot substitute for it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum SentinelDecision {
    /// Sentinel event count is below threshold. The claim remains poisoned
    /// pending explicit reconciliation.
    BelowThreshold {
        /// Number of sentinel events observed in the rolling window
        /// after this one.
        event_count: u32,
    },
    /// Sentinel event count met or exceeded threshold; escalate.
    EscalateToReauth {
        /// Number of sentinel events observed in the rolling window
        /// after this one.
        event_count: u32,
        /// Length of the rolling window in seconds.
        window_secs: u64,
    },
}

/// Tracks sentinel events per credential; emits escalations when the
/// rolling-window threshold is reached.
///
/// The repository atomically inserts the sentinel evidence while retaining
/// the claim row, then this trigger queries the rolling-window count to decide
/// whether to escalate. The persistence layer (in-memory / SQLite / Postgres)
/// lives behind [`RefreshClaimRepo`].
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

    /// Evaluate an event that the claim repository already recorded atomically
    /// while retaining an expired `RefreshInFlight` row as durable poison.
    ///
    /// This method does not insert an event. The accountable reclaim contract
    /// guarantees exactly-once evidence before returning
    /// [`ExpiredClaim::OutcomeUnknownAccounted`](nebula_storage_port::store::ExpiredClaim::OutcomeUnknownAccounted).
    ///
    /// Distinct, explicitly reconciled incidents for one credential may cross
    /// the threshold over time. Repeated sweeps of one poisoned claim are
    /// idempotent and do not increment the count. Event-bus observations
    /// remain non-authoritative; durable reauth transition belongs to the
    /// aggregate owner.
    pub(crate) async fn decision_for_accounted_event(
        &self,
        credential_id: &nebula_core::CredentialId,
    ) -> Result<SentinelDecision, RepoError> {
        self.decision_for_recorded_event(credential_id).await
    }

    async fn decision_for_recorded_event(
        &self,
        credential_id: &nebula_core::CredentialId,
    ) -> Result<SentinelDecision, RepoError> {
        let count = self
            .repo
            .count_sentinel_events_in_window(credential_id, self.config.window)
            .await?;

        if count >= self.config.threshold {
            Ok(SentinelDecision::EscalateToReauth {
                event_count: count,
                // Saturate at 1 so sub-second test windows (e.g.
                // `Duration::from_millis(50)`) don't surface as
                // `window_secs: 0`. Production keeps full precision
                // (1h >> 1s); only sub-second test escalations differ,
                // and the saturation keeps `window_secs > 0` filters
                // on dashboards usable. See PR #583 wave-3 review m3.
                window_secs: self.config.window.as_secs().max(1),
            })
        } else {
            Ok(SentinelDecision::BelowThreshold { event_count: count })
        }
    }
}
