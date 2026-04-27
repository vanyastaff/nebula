//! Behavioural smoke tests for `InMemoryRefreshClaimRepo`.
//!
//! Verifies the CAS / heartbeat / release / reclaim contract per ADR-0041
//! and sub-spec §3.2.

use std::time::Duration;

use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, HeartbeatError, InMemoryRefreshClaimRepo, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState,
};

#[tokio::test]
async fn try_claim_acquires_when_no_holder() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();
    let holder = ReplicaId::new("test-replica");

    let outcome = repo
        .try_claim(&cid, &holder, Duration::from_secs(30))
        .await
        .unwrap();

    match outcome {
        ClaimAttempt::Acquired(claim) => {
            assert_eq!(claim.credential_id, cid);
            assert!(claim.expires_at > claim.acquired_at);
            assert_eq!(claim.token.generation, 0);
        },
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    }
}

#[tokio::test]
async fn try_claim_returns_contended_when_held() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();

    let _first = repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap();

    let second = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap();

    match second {
        ClaimAttempt::Contended {
            existing_expires_at,
        } => {
            // The contended response must surface the holder's expiry so the
            // caller can back off until that moment.
            assert!(existing_expires_at > chrono::Utc::now());
        },
        ClaimAttempt::Acquired(_) => panic!("expected Contended"),
    }
}

#[tokio::test]
async fn heartbeat_validates_generation() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };

    // Stale token — bump generation manually
    let stale = ClaimToken {
        claim_id: claim.token.claim_id,
        generation: claim.token.generation + 99,
    };

    let result = repo.heartbeat(&stale, Duration::from_secs(30)).await;
    assert!(matches!(result, Err(HeartbeatError::ClaimLost)));

    // The original token still works.
    repo.heartbeat(&claim.token, Duration::from_secs(30))
        .await
        .expect("live token");
}

#[tokio::test]
async fn release_is_idempotent() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };

    repo.release(claim.token.clone()).await.unwrap();
    repo.release(claim.token.clone()).await.unwrap(); // idempotent

    // After release, a fresh acquirer must win.
    let next = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap();
    assert!(matches!(next, ClaimAttempt::Acquired(_)));
}

#[tokio::test]
async fn reclaim_returns_expired_with_sentinel_state() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_millis(50))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };
    repo.mark_sentinel(&claim.token).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    let reclaimed = repo.reclaim_stuck().await.unwrap();

    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].credential_id, cid);
    assert_eq!(reclaimed[0].previous_holder, ReplicaId::new("A"));
    assert_eq!(reclaimed[0].previous_generation, 0);
    assert_eq!(reclaimed[0].sentinel, SentinelState::RefreshInFlight);

    // After reclaim, a new acquirer wins with bumped generation.
    let next = match repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired after reclaim"),
    };
    // Generation resets only because reclaim_stuck removed the row entirely;
    // a fresh row starts at 0. (Generation only bumps when an expired row is
    // overwritten in-place, not when it has been swept.)
    assert_eq!(next.token.generation, 0);
}

#[tokio::test]
async fn mark_sentinel_after_reclaim_returns_invalid_state() {
    // Contract (sub-spec §3.4 + trait doc): once a claim has been reclaimed
    // (row removed by `reclaim_stuck`, or generation bumped by an in-place
    // overwrite), the original holder's `mark_sentinel` MUST return
    // `RepoError::InvalidState` so the holder cannot proceed to the IdP
    // POST while another replica owns the credential. Silent success would
    // re-introduce the pre-fix race that this trait change closed.
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    let token = match repo
        .try_claim(&cid, &holder, Duration::from_millis(20))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c.token,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };

    // Wait past TTL, then sweep — the row is now gone.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let reclaimed = repo.reclaim_stuck().await.unwrap();
    assert_eq!(reclaimed.len(), 1, "the expired row must be reclaimed");

    let err = repo
        .mark_sentinel(&token)
        .await
        .expect_err("mark_sentinel must fail after reclaim");
    assert!(
        matches!(err, RepoError::InvalidState(_)),
        "expected InvalidState, got {err:?}"
    );
}

#[tokio::test]
async fn try_claim_after_expiry_bumps_generation_in_place() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::new();

    let first = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_millis(40))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };
    assert_eq!(first.token.generation, 0);

    tokio::time::sleep(Duration::from_millis(80)).await;

    // No reclaim_stuck call: the expired row is overwritten in place.
    let second = match repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired after expiry"),
    };
    assert_eq!(
        second.token.generation, 1,
        "generation must bump on overwrite"
    );

    // The first holder's heartbeat must now fail.
    let stale = repo.heartbeat(&first.token, Duration::from_secs(30)).await;
    assert!(matches!(stale, Err(HeartbeatError::ClaimLost)));
}
