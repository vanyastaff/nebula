//! Unit test for sentinel set-then-clear sequence (Stage 2.4).
//!
//! Verifies that `RefreshCoordinator::refresh_coalesced` runs the user
//! closure with a real `RefreshClaim` whose token can be sentinel-marked
//! via `RefreshClaimRepo::mark_sentinel`, and that on the success path
//! the row is **deleted** by `release` so no `RefreshInFlight` sentinel
//! lingers — sub-spec §3.4.
//!
//! Mid-refresh crash detection (sentinel surviving a sweep) is tested at
//! the storage level in `nebula-storage::refresh_claim_in_memory_smoke`
//! and `refresh_claim_sqlite_integration`. Here we just confirm the
//! engine-side wiring through the coordinator-closure boundary works.

use std::{sync::Arc, time::Duration};

use nebula_engine::credential::refresh::{RefreshCoordConfig, RefreshCoordinator, RefreshError};
use nebula_storage::credential::{InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId};

#[tokio::test]
async fn refresh_marks_sentinel_before_idp_call_and_clears_on_release() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord = RefreshCoordinator::new_with(
        Arc::clone(&repo),
        ReplicaId::new("test-A"),
        RefreshCoordConfig::default(),
    )
    .expect("default config valid");

    let cid = nebula_core::CredentialId::new();
    let repo_in_closure = Arc::clone(&repo);

    // Run a refresh closure that simulates the token_refresh.rs path:
    //   1. mark_sentinel = RefreshInFlight
    //   2. "POST /token" succeeds
    //   3. closure returns Ok
    // RefreshCoordinator::refresh_coalesced then calls release which
    // deletes the row entirely.
    let result: Result<u32, RefreshError> = coord
        .refresh_coalesced(
            &cid,
            |_id| async { true },
            |claim| async move {
                // Step 1: sentinel
                repo_in_closure
                    .mark_sentinel(&claim.token)
                    .await
                    .map_err(RefreshError::Repo)?;
                // Step 2: simulated IdP POST (no-op here)
                // Step 3: success
                Ok(1234)
            },
        )
        .await;

    assert_eq!(result.unwrap(), 1234);

    // After successful release, the row is deleted entirely. A reclaim
    // sweep must observe no stuck claims.
    let stuck = repo.reclaim_stuck().await.expect("reclaim_stuck ok");
    assert!(
        stuck.is_empty(),
        "successful release path must leave no stuck sentinel claims, got: {stuck:?}"
    );

    // A fresh acquire from a different replica wins immediately
    // (confirms the row really was deleted).
    let attempt = repo
        .try_claim(&cid, &ReplicaId::new("test-B"), Duration::from_secs(5))
        .await
        .expect("try_claim ok");
    assert!(
        matches!(
            attempt,
            nebula_storage::credential::ClaimAttempt::Acquired(_)
        ),
        "release must have deleted the row; got: {attempt:?}"
    );
}

#[tokio::test]
async fn closure_error_path_still_releases_claim_via_idempotent_release() {
    // Even when the user closure returns Err, the coordinator must
    // call release(token) so a stuck-sentinel row never lingers in the
    // success-path table. The crash-mid-refresh case (sentinel
    // surviving past TTL) is exercised at the storage layer.
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord = RefreshCoordinator::new_with(
        Arc::clone(&repo),
        ReplicaId::new("test-A"),
        RefreshCoordConfig::default(),
    )
    .expect("default config valid");

    let cid = nebula_core::CredentialId::new();
    let repo_in_closure = Arc::clone(&repo);

    let result: Result<u32, RefreshError> = coord
        .refresh_coalesced(
            &cid,
            |_id| async { true },
            |claim| async move {
                repo_in_closure
                    .mark_sentinel(&claim.token)
                    .await
                    .map_err(RefreshError::Repo)?;
                // Simulate IdP POST failure: returning a Repo-flavored
                // RefreshError so the closure errors.
                Err(RefreshError::Repo(
                    nebula_storage::credential::RepoError::InvalidState("simulated IdP 500".into()),
                ))
            },
        )
        .await;

    assert!(matches!(result, Err(RefreshError::Repo(_))));

    // Row was deleted via release even on the error path.
    let attempt = repo
        .try_claim(&cid, &ReplicaId::new("test-B"), Duration::from_secs(5))
        .await
        .expect("try_claim ok");
    assert!(
        matches!(
            attempt,
            nebula_storage::credential::ClaimAttempt::Acquired(_)
        ),
        "release must delete the row even on error path; got: {attempt:?}"
    );
}
