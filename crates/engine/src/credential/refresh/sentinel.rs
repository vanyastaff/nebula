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
//! `ReauthRequired`. The Stage 3.2 implementation records each detected
//! event in `credential_sentinel_events` and consults the rolling-window
//! count via `RefreshClaimRepo::count_sentinel_events_in_window`.

use std::{sync::Arc, time::Duration};

use nebula_storage::credential::{RefreshClaimRepo, ReplicaId, RepoError};

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
/// `Recoverable` means the threshold has not yet been reached — the
/// reclaim sweep should clear the claim row and let normal refresh
/// retry resume. `EscalateToReauth` means the threshold was met or
/// exceeded — the engine must transition the credential to
/// `ReauthRequired` and emit
/// [`nebula_credential::CredentialEvent::ReauthRequired`] per
/// sub-spec §3.4.
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

    /// Borrow the repo handle. Currently unused outside the trigger
    /// itself — Stage 3.3 will consume it from the reclaim sweep
    /// when threading the trigger through the engine composition root.
    #[allow(
        dead_code,
        reason = "consumed by the reclaim-sweep wiring landing in Stage 3.3"
    )]
    pub(crate) fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
        &self.repo
    }

    /// Called by the reclaim sweep when an expired claim has
    /// `sentinel = RefreshInFlight`. Records the event in
    /// `credential_sentinel_events` then queries the rolling-window
    /// count; returns `EscalateToReauth` when the count is at or above
    /// threshold (default 3-in-1h per sub-spec §3.4).
    ///
    /// # Concurrency
    ///
    /// The record-then-count sequence is **not atomic**. Two failure
    /// modes to consider:
    ///
    /// - **Same-row case** (two sweepers race on the same stuck claim): prevented at the storage
    ///   layer — `RefreshClaimRepo::reclaim_stuck` uses `DELETE … RETURNING` (postgres + sqlite) so
    ///   each expired row is observed by exactly one sweeper, never double-counted.
    /// - **Distinct-row near-threshold case** (two sweepers each find their own stuck row for the
    ///   same credential, then both insert and read count ≥ threshold): both will return
    ///   [`SentinelDecision::EscalateToReauth`] and the consumer will see two
    ///   `CredentialEvent::ReauthRequired` for the same credential. The consumer
    ///   (`CredentialEvent::ReauthRequired` handler in the credential engine) MUST be idempotent —
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
                window_secs: self.config.window.as_secs(),
            })
        } else {
            Ok(SentinelDecision::Recoverable { event_count: count })
        }
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

    #[tokio::test]
    async fn first_event_below_threshold_returns_recoverable() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let trigger = SentinelTrigger::new(Arc::clone(&repo), SentinelThresholdConfig::default());

        let cid = nebula_core::CredentialId::new();
        let holder = ReplicaId::new("replica-A");

        let decision = trigger
            .on_sentinel_detected(&cid, &holder, 1)
            .await
            .expect("on_sentinel_detected ok");

        // First event: count = 1, threshold = 3 → recoverable.
        assert_eq!(decision, SentinelDecision::Recoverable { event_count: 1 });
    }

    #[tokio::test]
    async fn second_event_still_below_threshold_returns_recoverable() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let trigger = SentinelTrigger::new(Arc::clone(&repo), SentinelThresholdConfig::default());

        let cid = nebula_core::CredentialId::new();
        let holder = ReplicaId::new("replica-A");

        let _first = trigger
            .on_sentinel_detected(&cid, &holder, 1)
            .await
            .expect("first ok");
        let second = trigger
            .on_sentinel_detected(&cid, &holder, 2)
            .await
            .expect("second ok");

        // Second event: count = 2, threshold = 3 → still recoverable.
        assert_eq!(second, SentinelDecision::Recoverable { event_count: 2 });
    }

    #[tokio::test]
    async fn third_event_at_threshold_returns_escalate() {
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let trigger = SentinelTrigger::new(Arc::clone(&repo), SentinelThresholdConfig::default());

        let cid = nebula_core::CredentialId::new();
        let holder = ReplicaId::new("replica-A");

        let _first = trigger
            .on_sentinel_detected(&cid, &holder, 1)
            .await
            .unwrap();
        let _second = trigger
            .on_sentinel_detected(&cid, &holder, 2)
            .await
            .unwrap();
        let third = trigger
            .on_sentinel_detected(&cid, &holder, 3)
            .await
            .unwrap();

        // Third event: count = 3, threshold = 3 → escalate.
        assert_eq!(
            third,
            SentinelDecision::EscalateToReauth {
                event_count: 3,
                window_secs: 3600,
            }
        );
    }

    #[tokio::test]
    async fn other_credentials_do_not_count_toward_threshold() {
        // The N-in-1h count is per-credential. Events on a different
        // credential ID must not push us over the threshold for the
        // credential under test.
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let trigger = SentinelTrigger::new(Arc::clone(&repo), SentinelThresholdConfig::default());

        let cid_a = nebula_core::CredentialId::new();
        let cid_b = nebula_core::CredentialId::new();
        let holder = ReplicaId::new("replica-A");

        // 2 events on B, then 1 event on A.
        let _ = trigger
            .on_sentinel_detected(&cid_b, &holder, 1)
            .await
            .unwrap();
        let _ = trigger
            .on_sentinel_detected(&cid_b, &holder, 2)
            .await
            .unwrap();
        let a_first = trigger
            .on_sentinel_detected(&cid_a, &holder, 1)
            .await
            .unwrap();

        // A has only 1 event; threshold not reached.
        assert_eq!(a_first, SentinelDecision::Recoverable { event_count: 1 });
    }

    #[tokio::test]
    async fn events_outside_window_do_not_count() {
        // Custom config: threshold 2 with a tiny 50ms window. Insert
        // an event, wait past the window, then insert two more —
        // the first event must NOT count toward the rolling threshold.
        let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
        let cfg = SentinelThresholdConfig {
            threshold: 2,
            window: Duration::from_millis(50),
        };
        let trigger = SentinelTrigger::new(Arc::clone(&repo), cfg);

        let cid = nebula_core::CredentialId::new();
        let holder = ReplicaId::new("replica-A");

        let _ = trigger
            .on_sentinel_detected(&cid, &holder, 1)
            .await
            .unwrap();
        // Wait past the window.
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Two new events inside the next 50ms — these alone should
        // trip the threshold.
        let _ = trigger
            .on_sentinel_detected(&cid, &holder, 2)
            .await
            .unwrap();
        let third = trigger
            .on_sentinel_detected(&cid, &holder, 3)
            .await
            .unwrap();

        // The original event aged out; count = 2 (the two new ones).
        assert!(
            matches!(
                third,
                SentinelDecision::EscalateToReauth {
                    event_count: 2,
                    window_secs: 0,
                }
            ),
            "expected EscalateToReauth with event_count=2; got {third:?}"
        );
    }
}
