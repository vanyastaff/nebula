//! Sentinel mid-refresh crash detection + threshold escalation.
//!
//! Per sub-spec
//! `docs/INTEGRATION_MODEL.md`
//! + audit.
//!
//! When a holder is about to perform the IdP POST (the operation that
//! risks invalidating the refresh token if not persisted), it marks the
//! claim as `SentinelState::RefreshInFlight` via
//! [`RefreshClaimStore::mark_sentinel`]. On successful release the claim
//! row is **deleted** entirely (via
//! [`RefreshClaimStore::release`]) so the sentinel clears by removal --
//! no separate "clear sentinel" call is needed on the success path.
//! If the reclaim sweep finds an expired claim still flagged
//! `RefreshInFlight`, the holder is presumed crashed mid-refresh.
//!
//! N=3 sentinel events within 1h (default) escalate the credential to
//! `ReauthRequired`. The Stage 3.2 implementation records each detected
//! event in `credential_sentinel_events` and consults the rolling-window
//! count via `RefreshClaimStore::count_sentinel_events_in_window`.

use std::{sync::Arc, time::Duration};

use nebula_storage_port::store::{
    RefreshClaimError as RepoError, RefreshClaimStore as RefreshClaimRepo, ReplicaId,
};

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

/// Decision returned by [`SentinelTrigger::on_sentinel_detected`].
///
/// `Recoverable` means the threshold has not yet been reached -- the
/// reclaim sweep should clear the claim row and let normal refresh
/// retry resume. `EscalateToReauth` means the threshold was met or
/// exceeded -- the engine must transition the credential to
/// `ReauthRequired` and emit
/// `nebula_credential::CredentialEvent::ReauthRequired` per
/// sub-spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentinelDecision {
    /// Sentinel event count is below threshold; resume normal flow.
    Recoverable {
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
/// Stage 3.2 implementation: each call to `on_sentinel_detected`
/// inserts one row into `credential_sentinel_events` and queries the
/// rolling-window count to decide whether to escalate. The persistence
/// layer (in-memory / SQLite / Postgres) lives behind
/// [`RefreshClaimRepo`].
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

    // guard-justified: consumed by reclaim-sweep wiring that lands in Stage 3.3; removing
    // silences the compiler but breaks the planned composition root
    #[allow(dead_code)]
    pub(crate) fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
        &self.repo
    }

    /// Called by the reclaim sweep when an expired claim has
    /// `sentinel = RefreshInFlight`. Records the event in
    /// `credential_sentinel_events` then queries the rolling-window
    /// count; returns `EscalateToReauth` when the count is at or above
    /// threshold (default 3-in-1h per sub-spec).
    ///
    /// # Concurrency
    ///
    /// The record-then-count sequence is **not atomic**. Two failure
    /// modes to consider:
    ///
    /// - **Same-row case** (two sweepers race on the same stuck claim): prevented at the storage
    ///   layer -- `RefreshClaimRepo::reclaim_stuck` uses `DELETE ... RETURNING` (postgres + sqlite) so
    ///   each expired row is observed by exactly one sweeper, never double-counted.
    /// - **Distinct-row near-threshold case** (two sweepers each find their own stuck row for the
    ///   same credential, then both insert and read count >= threshold): both will return
    ///   [`SentinelDecision::EscalateToReauth`] and the consumer will see two
    ///   `CredentialEvent::ReauthRequired` for the same credential. The consumer
    ///   (`CredentialEvent::ReauthRequired` handler in the credential engine) MUST be idempotent --
    ///   flipping `reauth_required = true` is a fixed-point write so a duplicate is harmless.
    ///   Tightening this to bit-exact single-emit would require an atomic `record_and_count` repo
    ///   primitive (deferred, out of scope for the wave 2 review).
    ///
    /// # Errors
    ///
    /// Surfaces underlying [`RepoError`] from
    /// `record_sentinel_event` / `count_sentinel_events_in_window`.
    pub async fn on_sentinel_detected(
        &self,
        credential_id: &nebula_core::CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<SentinelDecision, RepoError> {
        // Record the event first, then query so the just-recorded row
        // is included in the threshold count.
        self.repo
            .record_sentinel_event(credential_id, crashed_holder, generation)
            .await?;

        let window = chrono::Duration::from_std(self.config.window)
            .unwrap_or_else(|_| chrono::Duration::seconds(3600));
        let window_start = chrono::Utc::now() - window;
        let count = self
            .repo
            .count_sentinel_events_in_window(credential_id, window_start)
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
            Ok(SentinelDecision::Recoverable { event_count: count })
        }
    }
}
