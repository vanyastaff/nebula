//! Integration test (Stage 2.5): two-tier coalesce smoke test.
//!
//! Per sub-spec §10 DoD #1.
//!
//! **Caveat (acknowledged in the plan).** Two `RefreshCoordinator`s in
//! the same process share neither L1 (each owns its own
//! `L1RefreshCoalescer`) nor the same `tokio::sync::oneshot` waiter
//! list. Two such coordinators DO race at L2 against the shared repo,
//! but until Stage 3.1 lands the post-backoff state re-check the
//! losing replica's retry will eventually succeed and run the user
//! closure a second time. So this test exercises the **single-coord,
//! many concurrent callers** scenario, which is the realistic same-
//! process shape (one engine instance, many tasks needing the same
//! credential at once). True cross-process L2 contention with the
//! `CoalescedByOtherReplica` short-circuit lands in the Stage 4 chaos
//! test (separate tokio runtimes + shared SQLite).
//!
//! What this test verifies:
//! * `RefreshCoordinator::refresh_coalesced` collapses many concurrent same-process calls to
//!   exactly one inner closure invocation (sub-spec §10 DoD #1 single-process flavor).
//! * After all calls return the L2 row is released so a fresh acquire wins immediately.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_engine::credential::refresh::{RefreshCoordConfig, RefreshCoordinator, RefreshError};
use nebula_storage::credential::{
    ClaimAttempt, InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
};

#[tokio::test]
async fn shared_coordinator_collapses_to_one_idp_call() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("replica-A"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid"),
    );

    let cid = nebula_core::CredentialId::new();
    let idp_calls = Arc::new(AtomicU32::new(0));

    // Fire 8 concurrent calls into the same coordinator.
    let mut tasks = Vec::with_capacity(8);
    for _ in 0..8 {
        let coord = Arc::clone(&coord);
        let calls = Arc::clone(&idp_calls);
        tasks.push(tokio::spawn(async move {
            coord
                .refresh_coalesced(&cid, |_claim| async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, RefreshError>(())
                })
                .await
        }));
    }

    let results = futures::future::join_all(tasks).await;

    // Hard invariant: exactly one closure invocation.
    let total_calls = idp_calls.load(Ordering::SeqCst);
    assert_eq!(
        total_calls, 1,
        "two-tier coalesce must collapse to exactly 1 IdP closure invocation, saw {total_calls}"
    );

    // Each call must surface either Ok(()) (the Winner) or
    // Err(CoalescedByOtherReplica) (one of the L1 Waiters whose Winner
    // already finished and released the L2 claim).
    for (i, r) in results.into_iter().enumerate() {
        let inner = r.expect("task should not panic");
        let acceptable = matches!(&inner, Ok(()))
            || matches!(&inner, Err(RefreshError::CoalescedByOtherReplica));
        assert!(
            acceptable,
            "task {i} surfaced unexpected outcome at Stage 2: {inner:?}"
        );
    }

    // L2 row was released — fresh acquire wins immediately.
    let attempt = repo
        .try_claim(&cid, &ReplicaId::new("replica-B"), Duration::from_secs(5))
        .await
        .expect("try_claim ok");
    assert!(
        matches!(attempt, ClaimAttempt::Acquired(_)),
        "after refresh_coalesced returns, L2 row must be reclaimable: {attempt:?}"
    );
}

/// Documents the **separate-coordinator** behavior at Stage 2: two
/// distinct `RefreshCoordinator`s sharing a repo will both run their
/// closures because the post-backoff state re-check (`CoalescedByOtherReplica`
/// short-circuit) lands in Stage 3.1. Once that lands, the loser will
/// observe state-already-fresh and skip its closure — at which point
/// this test should flip its assertion to `total_calls == 1`.
///
/// Marked `#[ignore]` until Stage 3.1 because the assertion would fail
/// today.
#[tokio::test]
#[ignore = "Stage 3.1 — CoalescedByOtherReplica state re-check on L2 backoff path"]
async fn two_replicas_collapse_to_one_idp_call_after_stage_3_1() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());

    let coord_a = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("replica-A"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid"),
    );
    let coord_b = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("replica-B"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid"),
    );

    let cid = nebula_core::CredentialId::new();
    let idp_calls = Arc::new(AtomicU32::new(0));

    let coord_a_clone = Arc::clone(&coord_a);
    let coord_b_clone = Arc::clone(&coord_b);
    let calls_a = Arc::clone(&idp_calls);
    let calls_b = Arc::clone(&idp_calls);

    let fut_a = tokio::spawn(async move {
        coord_a_clone
            .refresh_coalesced(&cid, |_claim| {
                let calls = Arc::clone(&calls_a);
                async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, RefreshError>(())
                }
            })
            .await
    });
    let fut_b = tokio::spawn(async move {
        coord_b_clone
            .refresh_coalesced(&cid, |_claim| {
                let calls = Arc::clone(&calls_b);
                async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, RefreshError>(())
                }
            })
            .await
    });

    let _ = futures::future::join_all([fut_a, fut_b]).await;

    let total_calls = idp_calls.load(Ordering::SeqCst);
    assert_eq!(
        total_calls, 1,
        "Stage 3.1 — two separate coordinators sharing a repo must collapse to 1 closure invocation"
    );
}
