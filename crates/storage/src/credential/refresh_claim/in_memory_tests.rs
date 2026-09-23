use super::*;
use nebula_core::CredentialId;
use nebula_storage_port::{CredentialOwner, CredentialSelector};

async fn acquired_claim(
    repo: &InMemoryRefreshClaimRepo,
    selector: &CredentialSelector,
) -> RefreshClaim {
    match repo
        .try_claim(
            selector,
            &ReplicaId::new("original-holder"),
            Duration::from_secs(30),
        )
        .await
        .expect("initial claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } => panic!("fresh credential must be claimable"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    }
}

fn expire_claim(repo: &InMemoryRefreshClaimRepo, selector: &CredentialSelector) {
    let mut guard = repo.inner.lock();
    let row = guard
        .get_mut(selector)
        .expect("acquired claim row must exist");
    row.expires_at = repo.clock.now() - chrono::Duration::seconds(1);
}

#[tokio::test]
async fn expired_claim_cannot_be_marked_in_flight() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let claim = acquired_claim(&repo, &selector).await;
    expire_claim(&repo, &selector);

    let error = repo
        .mark_sentinel(&claim.token)
        .await
        .expect_err("an expired claim must not authorize provider egress");

    assert!(matches!(error, RepoError::InvalidState));
    assert_eq!(
        repo.inner
            .lock()
            .get(&selector)
            .expect("rejected mark must preserve the claim row")
            .sentinel,
        SentinelState::Normal,
        "a rejected mark must not mutate sentinel state"
    );
}

#[tokio::test]
async fn expired_in_flight_claim_is_preserved_until_reclaim() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let claim = acquired_claim(&repo, &selector).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("live holder may mark provider egress");
    expire_claim(&repo, &selector);

    let attempt = repo
        .try_claim(
            &selector,
            &ReplicaId::new("challenger"),
            Duration::from_secs(30),
        )
        .await
        .expect("poisoned acquisition");
    assert!(
        matches!(attempt, ClaimAttempt::OutcomeUnknown { .. }),
        "try_claim must fail closed without erasing in-flight evidence"
    );
    let repeated = repo
        .try_claim(
            &selector,
            &ReplicaId::new("second-challenger"),
            Duration::from_secs(30),
        )
        .await
        .expect("repeated poisoned acquisition");
    assert!(matches!(repeated, ClaimAttempt::OutcomeUnknown { .. }));
}

#[tokio::test]
async fn expired_normal_claim_can_be_taken_over_in_place() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let first = acquired_claim(&repo, &selector).await;
    expire_claim(&repo, &selector);

    let second = repo
        .try_claim(
            &selector,
            &ReplicaId::new("challenger"),
            Duration::from_secs(30),
        )
        .await
        .expect("expired normal takeover");
    let ClaimAttempt::Acquired(second) = second else {
        panic!("expired normal claim must remain directly reclaimable");
    };

    assert_eq!(second.token.generation, first.token.generation + 1);
}

#[tokio::test]
async fn exact_confirmed_release_clears_expired_in_flight_claim() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let claim = acquired_claim(&repo, &selector).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark provider boundary");
    expire_claim(&repo, &selector);

    repo.release(claim.token)
        .await
        .expect("exact confirmed finalization");
    let next = repo
        .try_claim(
            &selector,
            &ReplicaId::new("next-holder"),
            Duration::from_secs(30),
        )
        .await
        .expect("claim after exact finalization");
    assert!(
        matches!(next, ClaimAttempt::Acquired(_)),
        "exact confirmed finalization must not leave false poison"
    );
}

#[tokio::test]
async fn owner_partitions_are_independent_for_the_same_credential_id() {
    let repo = InMemoryRefreshClaimRepo::new();
    let id = CredentialId::new();
    let owner_a = CredentialSelector::new(CredentialOwner::from_canonical("owner-a"), id);
    let owner_b = CredentialSelector::new(CredentialOwner::from_canonical("owner-b"), id);

    assert!(matches!(
        repo.try_claim(&owner_a, &ReplicaId::new("a"), Duration::from_secs(30))
            .await
            .expect("owner A claim"),
        ClaimAttempt::Acquired(_)
    ));
    assert!(matches!(
        repo.try_claim(&owner_b, &ReplicaId::new("b"), Duration::from_secs(30))
            .await
            .expect("owner B claim"),
        ClaimAttempt::Acquired(_)
    ));
}

#[tokio::test]
async fn heartbeat_rejects_a_forged_generation_without_extending_the_claim() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let claim = acquired_claim(&repo, &selector).await;
    let original_expiry = claim.expires_at;
    let forged = ClaimToken {
        selector: selector.clone(),
        claim_id: claim.token.claim_id,
        generation: claim.token.generation + 1,
    };

    assert!(matches!(
        repo.heartbeat(&forged, Duration::from_mins(1)).await,
        Err(HeartbeatError::ClaimLost)
    ));
    assert_eq!(
        repo.inner
            .lock()
            .get(&selector)
            .expect("claim row")
            .expires_at,
        original_expiry
    );
}

#[tokio::test]
async fn adjudication_is_idempotent_and_conflicting_evidence_is_refused() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let claim = acquired_claim(&repo, &selector).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark provider egress");
    expire_claim(&repo, &selector);

    let first = repo
        .adjudicate(
            &selector,
            RefreshOutcomeDecision::ProviderNotApplied,
            "provider confirmed no mutation",
        )
        .await
        .expect("first decision");
    assert!(first.changed);

    let repeat = repo
        .adjudicate(
            &selector,
            RefreshOutcomeDecision::ProviderNotApplied,
            "provider confirmed no mutation",
        )
        .await
        .expect("idempotent recommit");
    assert!(!repeat.changed);

    let conflict = repo
        .adjudicate(
            &selector,
            RefreshOutcomeDecision::ProviderApplied,
            "different evidence",
        )
        .await
        .expect_err("a recorded outcome cannot be overwritten");
    assert!(matches!(
        conflict,
        RepoAdjudicationError::EvidenceConflict { .. }
    ));
}

#[tokio::test]
async fn adjudication_evidence_bounds_preserve_poison() {
    let repo = InMemoryRefreshClaimRepo::new();
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("owner-a"),
        CredentialId::new(),
    );
    let claim = acquired_claim(&repo, &selector).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark provider egress");
    expire_claim(&repo, &selector);

    assert!(matches!(
        repo.adjudicate(&selector, RefreshOutcomeDecision::ProviderNotApplied, "",)
            .await,
        Err(RepoAdjudicationError::InvalidEvidence)
    ));
    assert!(matches!(
        repo.try_claim(&selector, &ReplicaId::new("next"), Duration::from_secs(30))
            .await
            .expect("poison check"),
        ClaimAttempt::OutcomeUnknown { .. }
    ));
}
