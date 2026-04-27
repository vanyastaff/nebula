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

use nebula_credential::{
    CredentialStore,
    store::{PutMode, StoredCredential},
};
use nebula_engine::credential::refresh::{RefreshCoordConfig, RefreshCoordinator, RefreshError};
use nebula_storage::credential::{
    ClaimAttempt, InMemoryRefreshClaimRepo, InMemoryStore, RefreshClaimRepo, ReplicaId,
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
                .refresh_coalesced(
                    &cid,
                    // Single-coordinator path collapses at L1; the L2
                    // backoff predicate is unreachable here. Pass the
                    // legacy "still needs refresh" stub so semantics
                    // match Stage 2.
                    |_id| async { true },
                    |_claim| async move {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, RefreshError>(())
                    },
                )
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

/// Two distinct `RefreshCoordinator`s sharing a repo collapse to one
/// closure invocation per sub-spec §3.6 (post-backoff state recheck).
///
/// Each replica passes a `needs_refresh_after_backoff` predicate keyed
/// on a shared `AtomicBool` — replica A flips it to `false` after its
/// closure runs (mirroring "credential is now fresh in the store"),
/// and replica B's predicate sees the flip after the L2 backoff sleep
/// and short-circuits with `RefreshError::CoalescedByOtherReplica`.
///
/// The atomic is the test's stand-in for the resolver's real state-
/// check (which re-reads the credential's `expires_at`); the path
/// under test is identical — `try_acquire_l2_with_backoff` consults
/// the predicate after sleeping on `Contended` and surfaces
/// `CoalescedByOtherReplica` when the contender finished.
#[tokio::test]
async fn two_replicas_collapse_to_one_idp_call_after_stage_3_1() {
    use std::sync::atomic::AtomicBool;

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
    // Starts true: "credential needs refresh." Replica A flips it to
    // false right before releasing the L2 claim. Replica B's predicate
    // observes the flip and short-circuits.
    let still_stale = Arc::new(AtomicBool::new(true));

    let coord_a_clone = Arc::clone(&coord_a);
    let coord_b_clone = Arc::clone(&coord_b);
    let calls_a = Arc::clone(&idp_calls);
    let calls_b = Arc::clone(&idp_calls);
    let stale_a_pred = Arc::clone(&still_stale);
    let stale_a_closure = Arc::clone(&still_stale);
    let stale_b_pred = Arc::clone(&still_stale);

    let fut_a = tokio::spawn(async move {
        coord_a_clone
            .refresh_coalesced(
                &cid,
                move |_id| {
                    let stale = Arc::clone(&stale_a_pred);
                    async move { stale.load(Ordering::SeqCst) }
                },
                |_claim| {
                    let calls = Arc::clone(&calls_a);
                    let stale = Arc::clone(&stale_a_closure);
                    async move {
                        // Mimic the IdP POST + state commit. After this
                        // returns, the L2 row is released and "the
                        // credential is fresh in the store."
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        calls.fetch_add(1, Ordering::SeqCst);
                        // Flip *before* the closure returns so even if
                        // B retries try_claim immediately after A's
                        // release the predicate already reads false.
                        stale.store(false, Ordering::SeqCst);
                        Ok::<_, RefreshError>(())
                    }
                },
            )
            .await
    });
    // Tiny delay so A wins the L2 race deterministically.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let fut_b = tokio::spawn(async move {
        coord_b_clone
            .refresh_coalesced(
                &cid,
                move |_id| {
                    let stale = Arc::clone(&stale_b_pred);
                    async move { stale.load(Ordering::SeqCst) }
                },
                |_claim| {
                    let calls = Arc::clone(&calls_b);
                    async move {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, RefreshError>(())
                    }
                },
            )
            .await
    });

    let results = futures::future::join_all([fut_a, fut_b]).await;

    let total_calls = idp_calls.load(Ordering::SeqCst);
    assert_eq!(
        total_calls, 1,
        "Stage 3.1 — two separate coordinators sharing a repo must collapse to 1 closure invocation"
    );

    // Replica A must succeed; replica B must short-circuit with
    // CoalescedByOtherReplica.
    let result_a = results[0].as_ref().expect("task A panicked");
    let result_b = results[1].as_ref().expect("task B panicked");
    assert!(
        matches!(result_a, Ok(())),
        "replica A must succeed: got {result_a:?}"
    );
    assert!(
        matches!(result_b, Err(RefreshError::CoalescedByOtherReplica)),
        "replica B must surface CoalescedByOtherReplica: got {result_b:?}"
    );
}

/// Replica B does NOT retry the IdP after replica A's refresh returns
/// `RefreshOutcome::ReauthRequired` and persists `reauth_required=true`
/// on the credential row.
///
/// Sub-spec §3.6 / review feedback I1. Before the persisted reauth flag
/// landed, replica B's predicate at `resolver.rs:232-242` re-read
/// `expires_at`, computed `expired`, returned `true`, and replica B's
/// closure ran a second IdP POST that produced another `invalid_grant`
/// rejection — `O(replicas)` IdP load on a credential that has already
/// been rejected once. With the fix, replica B's predicate observes
/// `reauth_required=true` on the freshly-stored row and short-circuits
/// to `false` so the coordinator surfaces `CoalescedByOtherReplica`.
///
/// Test setup mirrors the production resolver path: a shared
/// `CredentialStore` carries the row; replica A's closure persists
/// `reauth_required=true` (mimicking
/// `CredentialResolver::perform_refresh`'s `ReauthRequired` arm) and
/// returns an error to mark the IdP-rejected outcome. Replica B's
/// predicate reads the store and consults `reauth_required`. The
/// assertion is on a per-replica IdP-call counter — replica B's
/// closure must NEVER be invoked.
#[tokio::test]
async fn replica_b_does_not_retry_after_replica_a_reauth_required() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let store = Arc::new(InMemoryStore::new());

    // Seed an expired credential so the predicate's secondary
    // `expires_at` check would normally return `true` (still needs
    // refresh) — proving that the short-circuit is driven by
    // `reauth_required`, not by a fresh `expires_at`.
    let cid = nebula_core::CredentialId::new();
    let credential_id = cid.to_string();
    let expires_at = chrono::Utc::now() - chrono::Duration::minutes(1); // already expired
    let seed = StoredCredential {
        id: credential_id.clone(),
        credential_key: "i1_regression".into(),
        data: br#"{"token":"old"}"#.to_vec(),
        state_kind: "i1_regression".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: Some(expires_at),
        reauth_required: false,
        metadata: Default::default(),
    };
    store.put(seed, PutMode::CreateOnly).await.unwrap();

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

    // Per-replica IdP counters so we can verify replica B's closure is
    // never invoked (the criterion the review feedback explicitly calls
    // out).
    let idp_calls_a = Arc::new(AtomicU32::new(0));
    let idp_calls_b = Arc::new(AtomicU32::new(0));

    // Replica A's predicate: first refresh attempt, the row's
    // reauth_required is false, so this returns true (the credential is
    // expired). Real resolver builds this predicate from the same
    // store + expires_at check; the test reproduces that shape.
    let store_for_pred_a = Arc::clone(&store);
    let credential_id_pred_a = credential_id.clone();
    let pred_a = move |_id: &nebula_core::CredentialId| {
        let store = Arc::clone(&store_for_pred_a);
        let cid = credential_id_pred_a.clone();
        async move {
            let stored = match store.get(&cid).await {
                Ok(s) => s,
                Err(_) => return true,
            };
            if stored.reauth_required {
                return false;
            }
            stored
                .expires_at
                .is_some_and(|e| e <= chrono::Utc::now() + chrono::Duration::minutes(5))
        }
    };

    // Replica A's closure: mimic
    // `CredentialResolver::perform_refresh`'s ReauthRequired arm —
    // persist `reauth_required=true` then return a refresh error.
    let store_for_closure_a = Arc::clone(&store);
    let credential_id_closure_a = credential_id.clone();
    let calls_a = Arc::clone(&idp_calls_a);
    let closure_a = move |_claim| {
        let store = Arc::clone(&store_for_closure_a);
        let cid = credential_id_closure_a.clone();
        let calls = Arc::clone(&calls_a);
        async move {
            // IdP POST happens here.
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(150)).await;

            // Persist reauth_required=true via CAS (mirrors resolver
            // ReauthRequired arm).
            let current = store.get(&cid).await.expect("get current row");
            let updated = StoredCredential {
                updated_at: chrono::Utc::now(),
                reauth_required: true,
                ..current.clone()
            };
            store
                .put(
                    updated,
                    PutMode::CompareAndSwap {
                        expected_version: current.version,
                    },
                )
                .await
                .expect("CAS persist reauth_required=true");

            // Surface a synthetic refresh error so replica A's outer
            // `Result` is `Err(_)` — like the real resolver returning
            // `Err(ResolveError::ReauthRequired { .. })`.
            Err::<(), RefreshError>(RefreshError::Timeout(Duration::from_secs(0)))
        }
    };

    // Replica B's predicate: same shape. After A persists, this MUST
    // observe `reauth_required=true` and return false.
    let store_for_pred_b = Arc::clone(&store);
    let credential_id_pred_b = credential_id.clone();
    let pred_b = move |_id: &nebula_core::CredentialId| {
        let store = Arc::clone(&store_for_pred_b);
        let cid = credential_id_pred_b.clone();
        async move {
            let stored = match store.get(&cid).await {
                Ok(s) => s,
                Err(_) => return true,
            };
            if stored.reauth_required {
                return false;
            }
            stored
                .expires_at
                .is_some_and(|e| e <= chrono::Utc::now() + chrono::Duration::minutes(5))
        }
    };

    // Replica B's closure: if it ever runs the test fails — that is the
    // ProviderRejected gap.
    let calls_b = Arc::clone(&idp_calls_b);
    let closure_b = move |_claim| {
        let calls = Arc::clone(&calls_b);
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<(), RefreshError>(())
        }
    };

    let coord_a_clone = Arc::clone(&coord_a);
    let coord_b_clone = Arc::clone(&coord_b);

    let cid_for_a = cid;
    let fut_a = tokio::spawn(async move {
        coord_a_clone
            .refresh_coalesced(&cid_for_a, pred_a, closure_a)
            .await
    });
    // Tiny delay so A wins the L2 race deterministically.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let cid_for_b = cid;
    let fut_b = tokio::spawn(async move {
        coord_b_clone
            .refresh_coalesced(&cid_for_b, pred_b, closure_b)
            .await
    });

    let results = futures::future::join_all([fut_a, fut_b]).await;

    // The criterion — replica B's closure (its own IdP POST) must
    // NEVER have been invoked.
    assert_eq!(
        idp_calls_b.load(Ordering::SeqCst),
        0,
        "replica B IdP closure must never run after replica A persists \
         reauth_required=true (sub-spec §3.6 / I1)"
    );
    // Replica A ran exactly once.
    assert_eq!(
        idp_calls_a.load(Ordering::SeqCst),
        1,
        "replica A must run the IdP closure exactly once"
    );

    // Replica A surfaced the synthetic refresh error. Replica B
    // surfaced `CoalescedByOtherReplica` — the predicate-driven
    // short-circuit.
    let result_a = results[0].as_ref().expect("task A panicked");
    let result_b = results[1].as_ref().expect("task B panicked");
    assert!(
        matches!(result_a, Err(RefreshError::Timeout(_))),
        "replica A must surface its synthetic refresh error: got {result_a:?}"
    );
    assert!(
        matches!(result_b, Err(RefreshError::CoalescedByOtherReplica)),
        "replica B must surface CoalescedByOtherReplica (predicate read \
         reauth_required=true): got {result_b:?}"
    );

    // Verify the persisted row carries the reauth flag.
    let final_row = store.get(&credential_id).await.unwrap();
    assert!(
        final_row.reauth_required,
        "credential row must carry reauth_required=true after replica A's persist"
    );
}
