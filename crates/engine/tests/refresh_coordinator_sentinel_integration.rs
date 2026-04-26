//! Integration tests for sentinel threshold + ReauthRequired escalation
//! (Stage 3.3).
//!
//! Per sub-spec §3.4. Each test wires:
//!
//! - An [`InMemoryRefreshClaimRepo`] holding both refresh-claim rows and the sentinel-event log.
//! - A [`SentinelTrigger`] with a custom threshold/window for deterministic timing.
//! - A [`ReclaimSweepHandle`] running on a tight cadence so the test progresses without sleeps.
//! - An `EventBus<CredentialEvent>` to observe `ReauthRequired` emissions.
//!
//! The "below-threshold", "at-threshold", and "two-in-window-one-outside"
//! cases each seed a controlled number of stuck `RefreshInFlight` claim
//! rows via the L2 `try_claim` + `mark_sentinel` path, wait for the
//! background sweep to process them, and then assert the bus output.
//!
//! These tests cover end-to-end behavior — the unit tests on
//! `SentinelTrigger` (in `crates/engine/src/credential/refresh/sentinel.rs`)
//! cover the threshold logic in isolation. The reclaim-sweep unit tests
//! (in `crates/engine/src/credential/refresh/reclaim.rs`) cover the
//! single-sweep path with a synchronous `run_one_sweep` helper.

use std::{sync::Arc, time::Duration};

use nebula_credential::{CredentialEvent, resolve::ReauthReason};
use nebula_engine::credential::refresh::{
    ReclaimSweepHandle, SentinelThresholdConfig, SentinelTrigger,
};
use nebula_eventbus::EventBus;
use nebula_storage::credential::{
    ClaimAttempt, InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
};

/// Seed a stuck `RefreshInFlight` claim row whose TTL has already
/// expired. The next reclaim sweep returns it as a sentinel event.
async fn seed_stuck_inflight_claim(
    repo: &Arc<dyn RefreshClaimRepo>,
    cid: nebula_core::CredentialId,
    holder: &str,
) {
    let claim = match repo
        .try_claim(&cid, &ReplicaId::new(holder), Duration::from_millis(20))
        .await
        .expect("try_claim ok")
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("setup must always acquire"),
    };
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark_sentinel ok");
    tokio::time::sleep(Duration::from_millis(40)).await;
}

/// Drain the subscriber for up to `deadline` and count
/// `ReauthRequired` events for `cid`.
async fn drain_reauth_for(
    subscriber: &mut nebula_eventbus::Subscriber<CredentialEvent>,
    cid: nebula_core::CredentialId,
    deadline: Duration,
) -> Vec<ReauthReason> {
    let mut out = Vec::new();
    let start = tokio::time::Instant::now();
    while start.elapsed() < deadline {
        let remaining = deadline.saturating_sub(start.elapsed());
        match tokio::time::timeout(remaining, subscriber.recv()).await {
            Ok(Some(CredentialEvent::ReauthRequired {
                credential_id,
                reason,
            })) if credential_id == cid => {
                out.push(reason);
            },
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => break,
        }
    }
    out
}

#[tokio::test]
async fn below_threshold_does_not_publish_reauth() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    // Threshold 3; we'll seed only 2 stuck claims.
    let sentinel = Arc::new(SentinelTrigger::new(
        Arc::clone(&repo),
        SentinelThresholdConfig::default(),
    ));
    let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let mut subscriber = bus.subscribe();

    let _handle = ReclaimSweepHandle::spawn(
        Arc::clone(&repo),
        Arc::clone(&sentinel),
        Duration::from_millis(30),
        Some(Arc::clone(&bus)),
    );

    let cid = nebula_core::CredentialId::new();

    // Seed 2 stuck claims with enough delay for the sweep to consume each.
    for i in 0..2 {
        seed_stuck_inflight_claim(&repo, cid, &format!("crashed-{i}")).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    // Wait long enough for any pending event to propagate.
    let observed = drain_reauth_for(&mut subscriber, cid, Duration::from_millis(300)).await;
    assert!(
        observed.is_empty(),
        "below-threshold (2 of 3) must not publish ReauthRequired; got {observed:?}"
    );
}

#[tokio::test]
async fn at_threshold_publishes_reauth_with_sentinel_repeated_reason() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let sentinel = Arc::new(SentinelTrigger::new(
        Arc::clone(&repo),
        SentinelThresholdConfig::default(),
    ));
    let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let mut subscriber = bus.subscribe();

    let _handle = ReclaimSweepHandle::spawn(
        Arc::clone(&repo),
        Arc::clone(&sentinel),
        Duration::from_millis(30),
        Some(Arc::clone(&bus)),
    );

    let cid = nebula_core::CredentialId::new();

    // Seed 3 stuck RefreshInFlight claims for the same credential.
    // Default threshold = 3-in-1h → the third triggers escalation.
    for i in 0..3 {
        seed_stuck_inflight_claim(&repo, cid, &format!("crashed-{i}")).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    let observed = drain_reauth_for(&mut subscriber, cid, Duration::from_millis(500)).await;
    assert_eq!(
        observed.len(),
        1,
        "at threshold must publish exactly one ReauthRequired event; got {observed:?}"
    );
    assert!(
        matches!(
            observed[0],
            ReauthReason::SentinelRepeated {
                event_count: 3,
                window_secs: 3600,
            }
        ),
        "reason must be SentinelRepeated; got {:?}",
        observed[0]
    );
}

#[tokio::test]
async fn two_in_window_one_outside_does_not_publish_reauth() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    // Custom config: threshold 2 with a 100ms rolling window. The
    // first stuck event ages out before the second + third land.
    let sentinel = Arc::new(SentinelTrigger::new(
        Arc::clone(&repo),
        SentinelThresholdConfig {
            threshold: 2,
            window: Duration::from_millis(100),
        },
    ));
    let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let mut subscriber = bus.subscribe();

    let _handle = ReclaimSweepHandle::spawn(
        Arc::clone(&repo),
        Arc::clone(&sentinel),
        Duration::from_millis(20),
        Some(Arc::clone(&bus)),
    );

    let cid = nebula_core::CredentialId::new();

    // Event #1.
    seed_stuck_inflight_claim(&repo, cid, "crashed-1").await;
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Wait long enough for the first event to age past the 100ms
    // rolling window.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Event #2 — first inside the new window.
    seed_stuck_inflight_claim(&repo, cid, "crashed-2").await;
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Drain bus. With 1 event aged out and 1 inside the window,
    // count=1 < threshold=2 → no escalation.
    let observed = drain_reauth_for(&mut subscriber, cid, Duration::from_millis(200)).await;
    assert!(
        observed.is_empty(),
        "two-in-window-one-outside must not escalate; got {observed:?}"
    );
}
