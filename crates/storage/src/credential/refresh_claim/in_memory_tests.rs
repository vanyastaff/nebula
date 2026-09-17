use super::*;

/// A clock this module moves by hand, so "expired" and "an hour ago" cost
/// no sleeping. The adapter's own clock is the seam; nothing else in the
/// claim path reads time.
struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    fn starting_now() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Utc::now()),
        })
    }

    fn advance(&self, by: chrono::Duration) {
        *self.now.lock() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock()
    }

    fn monotonic(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

async fn acquired_claim(
    repo: &InMemoryRefreshClaimRepo,
    credential_id: &CredentialId,
) -> RefreshClaim {
    match repo
        .try_claim(
            credential_id,
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

fn expire_claim(repo: &InMemoryRefreshClaimRepo, credential_id: &CredentialId) {
    let mut guard = repo.inner.lock();
    let row = guard
        .get_mut(credential_id)
        .expect("acquired claim row must exist");
    row.expires_at = repo.clock.now() - chrono::Duration::seconds(1);
}

#[tokio::test]
async fn expired_claim_cannot_be_marked_in_flight() {
    let repo = InMemoryRefreshClaimRepo::new();
    let credential_id = CredentialId::new();
    let claim = acquired_claim(&repo, &credential_id).await;
    expire_claim(&repo, &credential_id);

    let error = repo
        .mark_sentinel(&claim.token)
        .await
        .expect_err("an expired claim must not authorize provider egress");

    assert!(matches!(error, RepoError::InvalidState));
    assert_eq!(
        repo.inner
            .lock()
            .get(&credential_id)
            .expect("rejected mark must preserve the claim row")
            .sentinel,
        SentinelState::Normal,
        "a rejected mark must not mutate sentinel state"
    );
}

#[tokio::test]
async fn expired_in_flight_claim_is_preserved_until_reclaim() {
    let repo = InMemoryRefreshClaimRepo::new();
    let credential_id = CredentialId::new();
    let claim = acquired_claim(&repo, &credential_id).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("live holder may mark provider egress");
    expire_claim(&repo, &credential_id);

    let attempt = repo
        .try_claim(
            &credential_id,
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
            &credential_id,
            &ReplicaId::new("second-challenger"),
            Duration::from_secs(30),
        )
        .await
        .expect("repeated poisoned acquisition");
    assert!(matches!(repeated, ClaimAttempt::OutcomeUnknown { .. }));

    let reclaimed = repo.reclaim_stuck().await.expect("reclaim expired claim");
    assert_eq!(reclaimed.len(), 1);
    assert!(matches!(
        &reclaimed[0],
        ExpiredClaim::OutcomeUnknownAccounted {
            credential_id: accounted_id,
            previous_holder,
            previous_generation: 0,
        } if *accounted_id == credential_id
            && *previous_holder == ReplicaId::new("original-holder")
    ));
    let recorded = repo
        .count_sentinel_events_in_window(&credential_id, Duration::from_mins(1))
        .await
        .expect("count atomically recorded evidence");
    assert_eq!(
        recorded, 1,
        "reclaim must durably account in-flight evidence while retaining poison"
    );

    let next = repo
        .try_claim(
            &credential_id,
            &ReplicaId::new("challenger"),
            Duration::from_secs(30),
        )
        .await
        .expect("poisoned claim result");
    assert!(
        matches!(next, ClaimAttempt::OutcomeUnknown { .. }),
        "accounting must not release an unknown provider outcome"
    );
    assert!(
        repo.reclaim_stuck()
            .await
            .expect("idempotent poison accounting")
            .is_empty(),
        "the retained poison event must be accounted exactly once"
    );
}

#[tokio::test]
async fn expired_normal_claim_can_be_taken_over_in_place() {
    let repo = InMemoryRefreshClaimRepo::new();
    let credential_id = CredentialId::new();
    let first = acquired_claim(&repo, &credential_id).await;
    expire_claim(&repo, &credential_id);

    let second = repo
        .try_claim(
            &credential_id,
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
    let credential_id = CredentialId::new();
    let claim = acquired_claim(&repo, &credential_id).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark provider boundary");
    expire_claim(&repo, &credential_id);

    repo.release(claim.token)
        .await
        .expect("exact confirmed finalization");
    let next = repo
        .try_claim(
            &credential_id,
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
async fn old_generation_zero_evidence_does_not_mask_a_new_claim_lifecycle() {
    let repo = InMemoryRefreshClaimRepo::new();
    let credential_id = CredentialId::new();

    let first = acquired_claim(&repo, &credential_id).await;
    repo.mark_sentinel(&first.token)
        .await
        .expect("mark first provider boundary");
    expire_claim(&repo, &credential_id);
    assert_eq!(
        repo.reclaim_stuck()
            .await
            .expect("account first poison")
            .len(),
        1
    );
    // Reconciliation, not release, is how a poisoned lifecycle ends: the
    // incident the sweep just recorded must be resolved before the row can
    // go away, and `release` would now refuse it.
    repo.adjudicate(
        &credential_id,
        RefreshOutcomeDecision::ProviderNotApplied,
        "operator confirmed the first provider call never landed",
    )
    .await
    .expect("reconcile the first lifecycle's unknown outcome");

    let second = acquired_claim(&repo, &credential_id).await;
    assert_eq!(
        second.token.generation, 0,
        "a new row demonstrates why generation alone is not event identity"
    );
    repo.mark_sentinel(&second.token)
        .await
        .expect("mark second provider boundary");
    expire_claim(&repo, &credential_id);

    assert_eq!(
        repo.reclaim_stuck()
            .await
            .expect("account second poison")
            .len(),
        1,
        "evidence from the prior row lifecycle must not suppress new poison"
    );
    assert_eq!(
        repo.count_sentinel_events_in_window(&credential_id, Duration::from_mins(1))
            .await
            .expect("count both lifecycle events"),
        2
    );
}

#[tokio::test]
async fn sentinel_window_excludes_evidence_older_than_the_window() -> Result<(), RepoError> {
    let clock = ManualClock::starting_now();
    let repo = InMemoryRefreshClaimRepo::with_clock(Arc::clone(&clock) as Arc<dyn Clock>);
    let credential_id = CredentialId::new();

    let claim = acquired_claim(&repo, &credential_id).await;
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark provider boundary");
    clock.advance(chrono::Duration::seconds(31));
    assert_eq!(
        repo.reclaim_stuck()
            .await
            .expect("account the in-flight expiry")
            .len(),
        1
    );

    let window = Duration::from_hours(24);
    assert_eq!(
        repo.count_sentinel_events_in_window(&credential_id, window)
            .await?,
        1,
        "the accounted incident is inside the window"
    );
    clock.advance(chrono::Duration::hours(25));
    assert_eq!(
        repo.count_sentinel_events_in_window(&credential_id, window)
            .await?,
        0,
        "events before the window must be excluded"
    );
    Ok(())
}
