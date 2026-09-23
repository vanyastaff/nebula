//! One shared acceptance oracle for the durable refresh-claim store.
//!
//! The in-memory reference model, SQLite, and PostgreSQL implement the same two
//! roles — `RefreshClaimStore` (lease, heartbeat, release, reclaim) and
//! `RefreshClaimAdjudicator` (reconciliation of an unknown provider outcome) —
//! so they must answer every claim question identically. This module owns those
//! answers once; each backend's test file supplies a fixture and runs the same
//! cases against it.
//!
//! The inventory is the union of the four hand-copied suites this replaces,
//! deduplicated by behaviour rather than by name: `heartbeat_validates_generation`
//! and `heartbeat_extends_expiry_and_rejects_stale_token` were one case written
//! twice, as were `try_claim_after_expiry_bumps_generation_in_place` and
//! `expired_normal_claim_can_be_taken_over_in_place`. No behaviour lost a case;
//! three backends now run each surviving one, where two names had run on one
//! backend each. Thirty cases reach every backend: the nineteen that survived
//! deduplication, plus eleven for reconciliation — ten for the mechanism, and
//! one that pins what a reconciled claim stops authorizing.
//!
//! A case states what it observes, never how a backend stores it. The four
//! behaviours that differ by backend — whether a claim can be expired without
//! waiting, whether incidents can be aged out of a window, whether a decision
//! write can be made to fail, and how resolved incidents are counted — are the
//! seam [`RefreshClaimFixture`] exists for; nothing else in a case needs to know
//! which backend answered.

use std::time::Duration;

use nebula_storage::credential::refresh_claim::{
    MAX_ADJUDICATION_EVIDENCE_BYTES, RefreshAdjudication, RefreshClaimAdjudicationError,
    RefreshClaimAdjudicator, RefreshOutcomeDecision,
};
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaim, RefreshClaimReclaimer,
    RefreshClaimRepo, ReplicaId, RepoError, SentinelEscalationPolicy,
};
use nebula_storage_port::{CredentialOwner, CredentialSelector};

/// The reconciliation role's error type, under a name that reads in case bodies.
type AdjudicationError = RefreshClaimAdjudicationError;

/// SHA-256 of an evidence note, computed the way the adapters digest it.
///
/// The oracle pins digests against this independent computation, so a shared
/// case fails if a backend starts storing (or returning) a different identity.
fn evidence_digest(evidence: &str) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};

    Sha256::digest(evidence.as_bytes()).into()
}

/// Everything a shared case needs from a backend, beyond the two port roles.
///
/// Implementors hold one backend and one isolated namespace. The three clock-
/// and failure-shaped methods are the only places a case cannot use the port:
/// the in-memory adapter derives expiry from an injectable clock, while the two
/// SQL adapters derive it from the database and must be backdated instead.
#[async_trait::async_trait]
pub(crate) trait RefreshClaimFixture:
    RefreshClaimRepo + RefreshClaimAdjudicator + RefreshClaimReclaimer + Send + Sync
{
    /// A credential unique to `case`, to this run, and to this backend.
    ///
    /// Two SQL runs share one database, and every case in this inventory
    /// touches poison and incident rows, so identity — not cleanup — is what
    /// keeps cases independent.
    fn credential(&self, case: &str) -> CredentialSelector;

    /// A replica identity derived from `seed`, so cases do not share holders.
    fn replica(&self, seed: u8) -> ReplicaId;

    /// The TTL a case should ask for when it means to expire the claim next.
    ///
    /// Backends differ: the in-memory fixture expires a claim by advancing its
    /// clock, so a 30-second TTL would make every expiry case time-travel 30
    /// seconds and drag its incidents out of the windows the sentinel cases
    /// assert on. The SQL fixtures backdate the row with the database clock and
    /// never move `now`, so they keep the production TTL.
    fn claim_ttl(&self) -> Duration;

    /// Put `credential`'s claim past its lease deadline without waiting.
    async fn expire_claims(&self, credential: &CredentialSelector);

    /// Leave `count` incidents on record for `credential`, the last unresolved.
    ///
    /// A credential holds one claim row and an incident is keyed by that row's
    /// claim id, so at most one incident can be unresolved at a time: `count`
    /// replays of the real lifecycle, all but the last reconciled, are what a
    /// caller observes as "an incident that has not been decided".
    async fn seed_unresolved_incidents(&self, credential: &CredentialSelector, count: u32);

    /// Incidents on record for `credential`, over a window wide enough to hold
    /// everything this run recorded.
    ///
    /// Resolution-blind, like the sentinel threshold itself: a reconciled
    /// incident still counts.
    async fn incident_count(&self, credential: &CredentialSelector) -> u64;

    /// Test-only window query over durable incidents. Production threshold
    /// evaluation is part of the atomic reclaimer operation.
    async fn count_sentinel_events_in_window(
        &self,
        credential: &CredentialSelector,
        window: Duration,
    ) -> Result<u32, RepoError>;

    /// Incidents on record for `credential` that carry a reconciliation.
    async fn resolved_incident_count(&self, credential: &CredentialSelector) -> u64;

    /// Push `credential`'s incidents `by` further into the past.
    ///
    /// The sentinel window is derived from the backend's own clock, so a case
    /// that wants an incident *outside* a window ages the incident rather than
    /// the window.
    async fn age_incidents(&self, credential: &CredentialSelector, by: Duration);

    /// Make the next resolution write for `credential` fail.
    async fn inject_decision_write_failure(&self, credential: &CredentialSelector);

    /// Remove the injection installed by [`Self::inject_decision_write_failure`].
    async fn clear_decision_write_failure(&self, credential: &CredentialSelector);

    /// Does `credential` still hold the poison `try_claim` answers
    /// `OutcomeUnknown` with?
    async fn poisoned_claim_exists(&self, credential: &CredentialSelector) -> bool;
}

/// A credential id derived from `namespace` and `case`, with no randomness.
///
/// Deterministic within a namespace, so a failing case names the same
/// credential on every run of that namespace. Each runner mints a fresh
/// namespace per fixture, so the id differs between runs; that randomness is
/// what keeps a durable backend from meeting an earlier run's rows.
#[must_use]
pub(crate) fn case_credential(namespace: &str, case: &str) -> CredentialSelector {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&fnv1a(namespace, case, 0x11).to_be_bytes());
    bytes[8..].copy_from_slice(&fnv1a(namespace, case, 0x5D).to_be_bytes());
    CredentialSelector::new(
        CredentialOwner::from_canonical(namespace),
        nebula_core::CredentialId::from_bytes(bytes),
    )
}

/// FNV-1a over `namespace`, `case`, and a discriminator, so one case can mint
/// several independent credentials.
fn fnv1a(namespace: &str, case: &str, discriminator: u8) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in namespace
        .as_bytes()
        .iter()
        .chain(case.as_bytes())
        .chain(std::iter::once(&discriminator))
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A window wide enough to hold every incident a single run records.
///
/// The port exposes a windowed count, never an unbounded one, so a fixture's
/// `incident_count` is this duration: no case ages an incident beyond a couple
/// of minutes, and the in-memory fixture's manual clock advances by seconds.
pub(crate) const INCIDENT_WINDOW: Duration = Duration::from_hours(24);

fn escalation_policy() -> SentinelEscalationPolicy {
    SentinelEscalationPolicy::new(u32::MAX, INCIDENT_WINDOW)
        .expect("the shared oracle policy is non-zero")
}

/// Acquire `credential` for `holder` and return the claim.
async fn claim(
    fixture: &impl RefreshClaimFixture,
    credential: &CredentialSelector,
    holder: u8,
    ttl: Duration,
) -> RefreshClaim {
    match fixture
        .try_claim(credential, &fixture.replica(holder), ttl)
        .await
        .expect("acquisition must not fail")
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } => panic!("a free claim must be acquirable"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("a free claim cannot be poisoned"),
    }
}

/// Acquire `credential` for `seed`'s replica with the case TTL.
///
/// The holder is derived from the case seed so that a case comparing provenance
/// — `previous_holder` in a reclaim outcome — can name it again.
async fn acquire(
    fixture: &impl RefreshClaimFixture,
    credential: &CredentialSelector,
    seed: u8,
) -> RefreshClaim {
    claim(fixture, credential, seed, fixture.claim_ttl()).await
}

/// Take `credential` across the provider boundary and past its lease deadline.
///
/// This is the state `try_claim` answers `OutcomeUnknown` with: provider egress
/// has begun and the outcome is not known. Nothing here reclaims it, so the
/// sweep has not necessarily accounted it yet — which is the point of several
/// cases.
async fn poison(
    fixture: &impl RefreshClaimFixture,
    credential: &CredentialSelector,
    seed: u8,
) -> RefreshClaim {
    let claim = acquire(fixture, credential, seed).await;
    fixture
        .mark_sentinel(&claim.token)
        .await
        .expect("a live holder may mark provider egress");
    fixture.expire_claims(credential).await;
    claim
}

/// Is `credential` in `outcomes`, and as an accounted poison?
fn accounted_poison<'a>(
    outcomes: &'a [ExpiredClaim],
    credential: &CredentialSelector,
) -> Option<&'a ExpiredClaim> {
    outcomes.iter().find(|outcome| match outcome {
        ExpiredClaim::OutcomeUnknownAccounted { selector, .. }
        | ExpiredClaim::ReclaimedNormal { selector, .. } => selector == credential,
    })
}

/// Reclaim and assert this credential's poisoned generation was accounted once.
async fn sweep_accounts_poison(
    fixture: &impl RefreshClaimFixture,
    credential: &CredentialSelector,
) -> ExpiredClaim {
    let outcomes = fixture
        .reclaim_stuck(escalation_policy())
        .await
        .expect("the reclaim sweep must run");
    let ours = accounted_poison(&outcomes, credential)
        .unwrap_or_else(|| panic!("the poisoned generation must be reclaimed: {outcomes:?}"))
        .clone();
    assert!(
        matches!(ours, ExpiredClaim::OutcomeUnknownAccounted { .. }),
        "an expired in-flight claim is poison, not ordinary expiry: {ours:?}"
    );
    ours
}

/// Record `decision` for `credential` and require that this call recorded it.
///
/// Pins the success shape on every path that writes: the digest reported is the
/// digest of the evidence this call just stored.
async fn adjudicate_fresh(
    fixture: &impl RefreshClaimFixture,
    credential: &CredentialSelector,
    decision: RefreshOutcomeDecision,
    evidence: &str,
) -> RefreshAdjudication {
    let recorded = fixture
        .adjudicate(credential, decision, evidence)
        .await
        .expect("a poisoned claim must be adjudicable");
    assert!(
        recorded.changed,
        "the first decision for a poisoned claim must be recorded, not reported as a recommit"
    );
    assert_eq!(recorded.decision, decision);
    assert_eq!(
        recorded.evidence_digest,
        evidence_digest(evidence),
        "a recording call must report the digest of the evidence it stored"
    );
    recorded
}

/// Replay the real poisoned lifecycle `count` times, reconciling every incident
/// but the last.
///
/// A credential holds one claim row and an incident is keyed by that row's
/// claim id, so at most one incident can be undecided at a time: reaching
/// `count` incidents means every earlier one was decided before the next
/// lifecycle began. The shared part of a fixture's
/// [`RefreshClaimFixture::seed_unresolved_incidents`], which supplies only the
/// clock and the poison predicate its backend needs.
pub(crate) async fn replay_poisoned_lifecycles(
    fixture: &impl RefreshClaimFixture,
    credential: &CredentialSelector,
    count: u32,
) {
    for index in 0..count {
        let seed = u8::try_from(index).expect("an incident index fits a byte");
        let claim = acquire(fixture, credential, seed).await;
        fixture
            .mark_sentinel(&claim.token)
            .await
            .expect("a live holder may mark provider egress");
        fixture.expire_claims(credential).await;
        let outcomes = fixture
            .reclaim_stuck(escalation_policy())
            .await
            .expect("the reclaim sweep must run");
        assert!(
            matches!(
                accounted_poison(&outcomes, credential),
                Some(ExpiredClaim::OutcomeUnknownAccounted { .. })
            ),
            "every replay must be accounted as poison: {outcomes:?}"
        );
        if index + 1 < count {
            let recorded = fixture
                .adjudicate(
                    credential,
                    RefreshOutcomeDecision::ProviderNotApplied,
                    "earlier replay reconciled so the next lifecycle can begin",
                )
                .await
                .expect("an earlier replay must be reconcilable");
            assert!(recorded.changed);
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Lease: acquire, contend, heartbeat, release
// ──────────────────────────────────────────────────────────────────────────

pub(crate) async fn try_claim_acquires_when_no_holder(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("try_claim_acquires_when_no_holder");
    let attempt = fixture
        .try_claim(&credential, &fixture.replica(seed), fixture.claim_ttl())
        .await
        .expect("acquisition must not fail");

    let ClaimAttempt::Acquired(claim) = attempt else {
        panic!("a credential nobody holds must be acquirable, got {attempt:?}");
    };
    assert_eq!(claim.selector, credential);
    assert!(
        claim.expires_at > claim.acquired_at,
        "a lease must expire after it begins"
    );
    assert_eq!(
        claim.token.generation, 0,
        "the first holder of a credential is generation zero"
    );
    // The returned token is the authority the store itself now answers with:
    // an acquisition that never reached the row would show up as a second
    // `Acquired` here.
    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a contended acquisition must not fail");
    assert!(
        matches!(attempt, ClaimAttempt::Contended { .. }),
        "the acquired claim must be the one on record, got {attempt:?}"
    );
}

pub(crate) async fn try_claim_returns_contended_when_held(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("try_claim_returns_contended_when_held");
    let held = acquire(fixture, &credential, seed).await;

    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a contended acquisition must not fail");

    let ClaimAttempt::Contended {
        existing_expires_at,
    } = attempt
    else {
        panic!("a live claim must contend, got {attempt:?}");
    };
    // The backoff hint is the holder's own deadline, so a challenger can wait
    // for exactly the moment it becomes eligible.
    assert_eq!(
        existing_expires_at, held.expires_at,
        "contention must report the holder's expiry"
    );
    assert!(existing_expires_at > held.acquired_at);
}

pub(crate) async fn try_claim_acquire_then_release_then_reacquire(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("try_claim_acquire_then_release_then_reacquire");
    let first = acquire(fixture, &credential, seed).await;
    assert_eq!(first.token.generation, 0);

    fixture
        .release(first.token.clone())
        .await
        .expect("a claim with no incident must be releasable");

    let second = acquire(fixture, &credential, seed.wrapping_add(1)).await;
    assert_eq!(
        second.token.generation, 0,
        "the released row is gone, so the next holder starts a new lifecycle"
    );
    assert_ne!(
        second.token.claim_id, first.token.claim_id,
        "a reacquired claim is a new generation of identity, not the released one"
    );
}

pub(crate) async fn heartbeat_extends_expiry_and_rejects_a_stale_token(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("heartbeat_extends_expiry_and_rejects_a_stale_token");
    let claim = acquire(fixture, &credential, seed).await;

    let extended = fixture.claim_ttl() * 2;
    fixture
        .heartbeat(&claim.token, extended)
        .await
        .expect("the holder's own token must be heartbeatable");

    // A heartbeat that landed is observable as contention reporting the later
    // deadline — no case needs to read the stored column.
    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a contended acquisition must not fail");
    let ClaimAttempt::Contended {
        existing_expires_at,
    } = attempt
    else {
        panic!("a heartbeated claim is still live, got {attempt:?}");
    };
    assert!(
        existing_expires_at > claim.expires_at,
        "heartbeat must move the deadline past the acquisition expiry"
    );

    // The same claim id with a generation that never held it is not an
    // authority: the CAS is (claim id, generation), not claim id alone.
    let stale = ClaimToken {
        selector: credential.clone(),
        claim_id: claim.token.claim_id,
        generation: claim.token.generation.wrapping_add(1),
    };
    assert!(
        matches!(
            fixture.heartbeat(&stale, extended).await,
            Err(HeartbeatError::ClaimLost)
        ),
        "a superseded generation must not extend the lease"
    );
    fixture
        .heartbeat(&claim.token, extended)
        .await
        .expect("the stale attempt must not have disturbed the live token");
}

pub(crate) async fn release_is_idempotent(fixture: &impl RefreshClaimFixture, seed: u8) {
    let credential = fixture.credential("release_is_idempotent");
    let claim = acquire(fixture, &credential, seed).await;

    fixture
        .release(claim.token.clone())
        .await
        .expect("the exact holder may release");
    fixture
        .release(claim.token.clone())
        .await
        .expect("releasing an already-released claim is a no-op, not an error");

    let next = acquire(fixture, &credential, seed.wrapping_add(1)).await;
    assert_eq!(next.token.generation, 0);
}

// ──────────────────────────────────────────────────────────────────────────
// Reclaim: ordinary expiry is released, in-flight expiry is retained as poison
// ──────────────────────────────────────────────────────────────────────────

pub(crate) async fn reclaim_accounts_expired_in_flight_as_retained_poison(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("reclaim_accounts_expired_in_flight_as_retained_poison");
    let crashed = fixture.replica(seed);
    poison(fixture, &credential, seed).await;

    let outcome = sweep_accounts_poison(fixture, &credential).await;
    std::assert_matches!(
        outcome,
        ExpiredClaim::OutcomeUnknownAccounted { ref previous_holder, previous_generation: 0, .. }
            if *previous_holder == crashed
    );
    assert_eq!(
        fixture.incident_count(&credential).await,
        1,
        "accounting records exactly one incident for the poisoned generation"
    );

    // Accounting is not release: the row stays, and keeps refusing egress.
    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a poisoned acquisition must not fail");
    std::assert_matches!(attempt, ClaimAttempt::OutcomeUnknown { .. });

    let repeat = fixture
        .reclaim_stuck(escalation_policy())
        .await
        .expect("the sweep must be repeatable");
    assert!(
        accounted_poison(&repeat, &credential).is_none(),
        "one poisoned generation is accounted exactly once: {repeat:?}"
    );
}

pub(crate) async fn mark_sentinel_after_reclaim_returns_invalid_state(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // Once a claim has been reclaimed the original holder must not be able to
    // proceed to the provider POST: another replica owns the credential.
    let credential = fixture.credential("mark_sentinel_after_reclaim_returns_invalid_state");
    let claim = acquire(fixture, &credential, seed).await;
    fixture.expire_claims(&credential).await;

    let outcomes = fixture
        .reclaim_stuck(escalation_policy())
        .await
        .expect("the reclaim sweep must run");
    let ours = accounted_poison(&outcomes, &credential)
        .expect("an expired Normal claim is released, not retained");
    std::assert_matches!(
        ours,
        ExpiredClaim::ReclaimedNormal { previous_holder, previous_generation: 0, .. }
            if *previous_holder == fixture.replica(seed)
    );

    let error = fixture
        .mark_sentinel(&claim.token)
        .await
        .expect_err("mark_sentinel must fail after the row is gone");
    std::assert_matches!(error, RepoError::InvalidState);
}

pub(crate) async fn expired_normal_claim_can_be_taken_over_in_place(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("expired_normal_claim_can_be_taken_over_in_place");
    let first = acquire(fixture, &credential, seed).await;
    fixture.expire_claims(&credential).await;

    // No sweep: an expired Normal row is overwritten in place, bumping the
    // generation, because nothing crossed the provider boundary.
    let second = acquire(fixture, &credential, seed.wrapping_add(1)).await;
    assert_eq!(
        second.token.generation,
        first.token.generation + 1,
        "a takeover in place must bump the generation"
    );
    assert_eq!(second.selector, credential);
    assert!(
        matches!(
            fixture.heartbeat(&first.token, fixture.claim_ttl()).await,
            Err(HeartbeatError::ClaimLost)
        ),
        "the superseded holder must lose its lease"
    );
}

pub(crate) async fn out_of_range_generation_is_rejected_without_touching_the_claim(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential =
        fixture.credential("out_of_range_generation_is_rejected_without_touching_the_claim");
    let claim = acquire(fixture, &credential, seed).await;
    let forged = ClaimToken {
        selector: credential.clone(),
        claim_id: claim.token.claim_id,
        generation: u64::MAX,
    };

    // A generation that never held the lease has no authority at all. The
    // heartbeat variant is not pinned: the in-memory adapter carries a `u64`
    // generation end to end and reports `ClaimLost`, while both SQL adapters
    // must round-trip it through their column type and report `InvalidState`.
    std::assert_matches!(
        fixture.mark_sentinel(&forged).await,
        Err(RepoError::InvalidState)
    );
    match fixture.heartbeat(&forged, fixture.claim_ttl()).await {
        Err(HeartbeatError::ClaimLost | HeartbeatError::Repo(RepoError::InvalidState)) => {},
        other => panic!("a forged generation must not hold or extend a lease: {other:?}"),
    }
    match fixture.release(forged).await {
        Ok(()) | Err(RepoError::InvalidState) => {},
        Err(other) => panic!("a forged generation must not be released: {other:?}"),
    }

    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a contended acquisition must not fail");
    std::assert_matches!(
        attempt,
        ClaimAttempt::Contended { .. },
        "a forged token must not have touched the real claim"
    );
    fixture
        .release(claim.token)
        .await
        .expect("the real token still releases");
}

pub(crate) async fn expired_claim_cannot_be_marked_in_flight(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("expired_claim_cannot_be_marked_in_flight");
    let claim = acquire(fixture, &credential, seed).await;
    fixture.expire_claims(&credential).await;

    let error = fixture
        .mark_sentinel(&claim.token)
        .await
        .expect_err("an expired claim must not authorize provider egress");
    std::assert_matches!(error, RepoError::InvalidState);

    // The rejected mark left the row Normal: an expired Normal claim is still
    // directly acquirable, whereas a marked one would answer poison.
    let next = acquire(fixture, &credential, seed.wrapping_add(1)).await;
    assert_eq!(next.token.generation, claim.token.generation + 1);
}

pub(crate) async fn expired_in_flight_claim_is_preserved_until_reclaim(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("expired_in_flight_claim_is_preserved_until_reclaim");
    poison(fixture, &credential, seed).await;

    for challenger in [seed.wrapping_add(1), seed.wrapping_add(2)] {
        let attempt = fixture
            .try_claim(
                &credential,
                &fixture.replica(challenger),
                fixture.claim_ttl(),
            )
            .await
            .expect("a poisoned acquisition must not fail");
        std::assert_matches!(
            attempt,
            ClaimAttempt::OutcomeUnknown { .. },
            "try_claim must fail closed on an expired in-flight claim"
        );
    }

    let outcome = sweep_accounts_poison(fixture, &credential).await;
    std::assert_matches!(
        outcome,
        ExpiredClaim::OutcomeUnknownAccounted { ref previous_holder, previous_generation: 0, .. }
            if *previous_holder == fixture.replica(seed)
    );
    assert_eq!(
        fixture.incident_count(&credential).await,
        1,
        "reclaim must durably account in-flight evidence while retaining poison"
    );

    let next = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a poisoned acquisition must not fail");
    std::assert_matches!(
        next,
        ClaimAttempt::OutcomeUnknown { .. },
        "accounting must not release an unknown provider outcome"
    );
    let repeat = fixture
        .reclaim_stuck(escalation_policy())
        .await
        .expect("the sweep must be repeatable");
    assert!(
        accounted_poison(&repeat, &credential).is_none(),
        "the retained poison must be accounted exactly once: {repeat:?}"
    );
}

pub(crate) async fn exact_confirmed_release_clears_expired_in_flight_claim(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // Before the sweep runs there is no incident on record, so exact
    // `Confirmed` finalization is still the authority that ends the lifecycle —
    // the boundary `release_refuses_an_undecided_incident` pins from the other
    // side.
    let credential = fixture.credential("exact_confirmed_release_clears_expired_in_flight_claim");
    let claim = poison(fixture, &credential, seed).await;

    fixture
        .release(claim.token)
        .await
        .expect("exact confirmed finalization must clear an expired in-flight claim");
    assert!(
        !fixture.poisoned_claim_exists(&credential).await,
        "the released row must not remain as poison"
    );

    let next = acquire(fixture, &credential, seed.wrapping_add(1)).await;
    assert_eq!(
        next.token.generation, 0,
        "the released row is gone, so the next holder starts a new lifecycle"
    );
}

pub(crate) async fn old_generation_zero_evidence_does_not_mask_a_new_claim_lifecycle(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // Incident identity is the claim id, not the generation: a second row for
    // the same credential starts again at generation zero, so evidence from the
    // first lifecycle must not suppress the second one's poison.
    let credential =
        fixture.credential("old_generation_zero_evidence_does_not_mask_a_new_claim_lifecycle");
    let holder = seed;

    let first = poison(fixture, &credential, holder).await;
    sweep_accounts_poison(fixture, &credential).await;
    // A poisoned lifecycle ends by reconciliation, not by release: the incident
    // the sweep just recorded must be decided before the row may go away, and
    // `release` refuses an undecided one.
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderNotApplied,
        "operator confirmed the first provider call never landed",
    )
    .await;

    let second = acquire(fixture, &credential, holder).await;
    assert_eq!(
        second.token.generation, 0,
        "a new row demonstrates why generation alone is not event identity"
    );
    assert_ne!(
        second.token.claim_id, first.token.claim_id,
        "the second lifecycle is a new incident identity, not a replay of the first"
    );

    fixture
        .mark_sentinel(&second.token)
        .await
        .expect("mark second provider boundary");
    fixture.expire_claims(&credential).await;
    sweep_accounts_poison(fixture, &credential).await;

    assert_eq!(
        fixture.incident_count(&credential).await,
        2,
        "evidence from the prior row lifecycle must not suppress new poison"
    );
    assert_eq!(
        fixture.resolved_incident_count(&credential).await,
        1,
        "only the first lifecycle's incident was decided"
    );
}

pub(crate) async fn concurrent_try_claim_yields_one_acquired(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("concurrent_try_claim_yields_one_acquired");
    let ttl = fixture.claim_ttl();
    // Bound to locals: a `join!` future borrows them, and a temporary would be
    // dropped while the other side of the race still holds it.
    let left_holder = fixture.replica(seed);
    let right_holder = fixture.replica(seed.wrapping_add(1));

    let (left, right) = tokio::join!(
        fixture.try_claim(&credential, &left_holder, ttl),
        fixture.try_claim(&credential, &right_holder, ttl),
    );
    let left = left.expect("the left acquisition must not fail");
    let right = right.expect("the right acquisition must not fail");

    let acquired = [&left, &right]
        .into_iter()
        .filter(|attempt| matches!(attempt, ClaimAttempt::Acquired(_)))
        .count();
    let contended = [&left, &right]
        .into_iter()
        .filter(|attempt| matches!(attempt, ClaimAttempt::Contended { .. }))
        .count();
    assert_eq!(acquired, 1, "exactly one acquirer must win the CAS");
    assert_eq!(
        contended, 1,
        "the loser must observe contention, not an error"
    );
}

pub(crate) async fn concurrent_reclaim_accounts_each_poison_exactly_once(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // Two sweepers observing one poisoned generation would double-count toward
    // the sentinel threshold, so accounting must be a single-winner operation.
    const ROWS: usize = 24;
    let mut expected = std::collections::HashSet::new();
    for index in 0..ROWS {
        let credential = fixture.credential(&format!(
            "concurrent_reclaim_accounts_each_poison_exactly_once/{index:02}"
        ));
        poison(fixture, &credential, seed).await;
        expected.insert(credential);
    }

    let (left, right) = tokio::join!(
        fixture.reclaim_stuck(escalation_policy()),
        fixture.reclaim_stuck(escalation_policy())
    );
    let left = left.expect("the left sweep must not fail");
    let right = right.expect("the right sweep must not fail");
    let accounted_ids = |outcomes: &[ExpiredClaim]| {
        outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                ExpiredClaim::OutcomeUnknownAccounted { selector, .. } => Some(selector.clone()),
                ExpiredClaim::ReclaimedNormal { .. } => None,
            })
            .collect::<std::collections::HashSet<_>>()
    };
    let left_ids = accounted_ids(&left);
    let right_ids = accounted_ids(&right);
    assert!(
        left_ids.is_disjoint(&right_ids),
        "one poisoned generation must be observed by exactly one sweeper"
    );
    let accounted: std::collections::HashSet<_> = left_ids.union(&right_ids).cloned().collect();
    let ours: std::collections::HashSet<_> = accounted.intersection(&expected).cloned().collect();
    assert_eq!(
        ours, expected,
        "every poisoned generation must be accounted exactly once"
    );

    for credential in &expected {
        assert_eq!(
            fixture.incident_count(credential).await,
            1,
            "each poisoned generation records one incident"
        );
        let attempt = fixture
            .try_claim(
                credential,
                &fixture.replica(seed.wrapping_add(1)),
                fixture.claim_ttl(),
            )
            .await
            .expect("a poisoned acquisition must not fail");
        std::assert_matches!(attempt, ClaimAttempt::OutcomeUnknown { .. });
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Sentinel window: accounted incidents, scoped and windowed
// ──────────────────────────────────────────────────────────────────────────

pub(crate) async fn sentinel_event_count_in_window(fixture: &impl RefreshClaimFixture, _seed: u8) {
    let credential = fixture.credential("sentinel_event_count_in_window");
    assert_eq!(
        fixture.incident_count(&credential).await,
        0,
        "a credential nobody has raced holds no incident"
    );

    fixture.seed_unresolved_incidents(&credential, 3).await;
    assert_eq!(
        fixture.incident_count(&credential).await,
        3,
        "three reconciled or not, three incidents are on record"
    );
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&credential, Duration::from_mins(1))
            .await
            .expect("the sentinel count must be readable"),
        3,
        "the threshold is resolution-blind: a decided incident still counts"
    );
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&credential, Duration::ZERO)
            .await
            .expect("the sentinel count must be readable"),
        0,
        "a zero-width window excludes every recorded incident"
    );
}

pub(crate) async fn sentinel_count_filters_by_credential_id(
    fixture: &impl RefreshClaimFixture,
    _seed: u8,
) {
    let scoped = fixture.credential("sentinel_count_filters_by_credential_id");
    let other = fixture.credential("sentinel_count_filters_by_credential_id/other");
    let window = Duration::from_mins(1);

    fixture.seed_unresolved_incidents(&scoped, 2).await;
    fixture.seed_unresolved_incidents(&other, 1).await;

    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&scoped, window)
            .await
            .expect("the sentinel count must be readable"),
        2
    );
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&other, window)
            .await
            .expect("the sentinel count must be readable"),
        1,
        "another credential's incidents must not leak into this count"
    );
    assert_eq!(fixture.incident_count(&scoped).await, 2);
    assert_eq!(fixture.incident_count(&other).await, 1);
}

pub(crate) async fn sentinel_count_excludes_events_before_window_start(
    fixture: &impl RefreshClaimFixture,
    _seed: u8,
) {
    let credential = fixture.credential("sentinel_count_excludes_events_before_window_start");
    let window = Duration::from_mins(1);

    fixture.seed_unresolved_incidents(&credential, 1).await;
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&credential, window)
            .await
            .expect("the sentinel count must be readable"),
        1,
        "the freshly accounted incident is inside the window"
    );

    fixture
        .age_incidents(&credential, Duration::from_mins(2))
        .await;
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&credential, window)
            .await
            .expect("the sentinel count must be readable"),
        0,
        "an incident older than the window must not be counted"
    );

    // A later incident is inside the window again: the boundary is each
    // incident's own detection time, not a property of the credential.
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderNotApplied,
        "aged incident reconciled so a fresh lifecycle can begin",
    )
    .await;
    fixture.seed_unresolved_incidents(&credential, 1).await;
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&credential, window)
            .await
            .expect("the sentinel count must be readable"),
        1,
        "only the incident recorded after the window start counts"
    );
    assert_eq!(
        fixture.incident_count(&credential).await,
        2,
        "the aged incident is still on record, just outside the window"
    );
}

pub(crate) async fn accounted_sentinel_events_are_windowed_and_credential_scoped(
    fixture: &impl RefreshClaimFixture,
    _seed: u8,
) {
    let first = fixture.credential("accounted_sentinel_events_are_windowed_and_credential_scoped");
    let second =
        fixture.credential("accounted_sentinel_events_are_windowed_and_credential_scoped/other");
    let window = Duration::from_mins(1);

    for credential in [&first, &second] {
        assert_eq!(
            fixture
                .count_sentinel_events_in_window(credential, window)
                .await
                .expect("the sentinel count must be readable"),
            0,
            "no incident is on record yet"
        );
    }

    fixture.seed_unresolved_incidents(&first, 1).await;
    fixture.seed_unresolved_incidents(&second, 2).await;
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&first, window)
            .await
            .expect("the sentinel count must be readable"),
        1,
        "the accounted incident is inside the window"
    );
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&second, window)
            .await
            .expect("the sentinel count must be readable"),
        2,
        "a separately accounted credential keeps its own count"
    );

    // Deciding an incident does not erase it from the threshold's view.
    adjudicate_fresh(
        fixture,
        &first,
        RefreshOutcomeDecision::ProviderApplied,
        "provider support confirmed the refresh landed",
    )
    .await;
    assert_eq!(
        fixture.resolved_incident_count(&first).await,
        1,
        "the decision is on record"
    );
    assert_eq!(
        fixture
            .count_sentinel_events_in_window(&first, window)
            .await
            .expect("the sentinel count must be readable"),
        1,
        "a resolved incident still counts toward the sentinel threshold"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Reconciliation: recording the provider outcome an expired claim never had
// ──────────────────────────────────────────────────────────────────────────

pub(crate) async fn adjudication_clears_poison_and_the_claim_becomes_acquirable(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // The poison is the claim row and the incident is its accounting, written
    // at different times: reconciliation must work in either order.
    let early = fixture
        .credential("adjudication_clears_poison_and_the_claim_becomes_acquirable/before-sweep");
    poison(fixture, &early, seed).await;
    assert!(
        fixture.poisoned_claim_exists(&early).await,
        "the expired in-flight row is the poison"
    );
    assert_eq!(
        fixture.incident_count(&early).await,
        0,
        "the sweep has not accounted this poison yet"
    );

    adjudicate_fresh(
        fixture,
        &early,
        RefreshOutcomeDecision::ProviderNotApplied,
        "operator confirmed the call never left the replica",
    )
    .await;
    assert!(
        !fixture.poisoned_claim_exists(&early).await,
        "reconciliation must clear the poison"
    );
    assert_eq!(
        fixture.incident_count(&early).await,
        1,
        "the incident is created from the claim row's own identity"
    );
    assert_eq!(fixture.resolved_incident_count(&early).await, 1);
    let claim = acquire(fixture, &early, seed.wrapping_add(1)).await;
    assert_eq!(
        claim.token.generation, 0,
        "the reconciled row is gone, so the credential is acquirable again"
    );

    let swept = fixture
        .credential("adjudication_clears_poison_and_the_claim_becomes_acquirable/after-sweep");
    poison(fixture, &swept, seed).await;
    sweep_accounts_poison(fixture, &swept).await;
    assert_eq!(fixture.incident_count(&swept).await, 1);

    adjudicate_fresh(
        fixture,
        &swept,
        RefreshOutcomeDecision::ProviderApplied,
        "provider support ticket confirms the refresh landed",
    )
    .await;
    assert!(!fixture.poisoned_claim_exists(&swept).await);
    assert_eq!(
        fixture.incident_count(&swept).await,
        1,
        "deciding the sweep's incident must not add a second one"
    );
    assert_eq!(fixture.resolved_incident_count(&swept).await, 1);
    let next = fixture
        .try_claim(
            &swept,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a reconciled credential must be acquirable");
    std::assert_matches!(next, ClaimAttempt::Acquired(_));
}

pub(crate) async fn adjudication_of_an_unpoisoned_credential_is_refused(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let never = fixture.credential("adjudication_of_an_unpoisoned_credential_is_refused/never");
    std::assert_matches!(
        fixture
            .adjudicate(
                &never,
                RefreshOutcomeDecision::ProviderApplied,
                "no claim was ever held"
            )
            .await,
        Err(AdjudicationError::NotPoisoned),
        "a credential with no claim row has nothing to reconcile"
    );
    assert_eq!(fixture.incident_count(&never).await, 0);
    assert_eq!(fixture.resolved_incident_count(&never).await, 0);

    let live = fixture.credential("adjudication_of_an_unpoisoned_credential_is_refused/live");
    acquire(fixture, &live, seed).await;
    std::assert_matches!(
        fixture
            .adjudicate(
                &live,
                RefreshOutcomeDecision::ProviderApplied,
                "a live holder owns this decision"
            )
            .await,
        Err(AdjudicationError::NotPoisoned),
        "a live claim is not an unknown provider outcome"
    );

    // An expired Normal claim is ordinary lease loss: nobody crossed the
    // provider boundary, so there is no outcome to decide.
    fixture.expire_claims(&live).await;
    std::assert_matches!(
        fixture
            .adjudicate(
                &live,
                RefreshOutcomeDecision::ProviderApplied,
                "no provider egress ever began"
            )
            .await,
        Err(AdjudicationError::NotPoisoned),
        "an expired claim that never crossed the provider boundary is not poison"
    );
    let attempt = fixture
        .try_claim(
            &live,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a refused adjudication must not disturb the claim");
    std::assert_matches!(
        attempt,
        ClaimAttempt::Acquired(_),
        "a refused adjudication must leave an ordinary expired claim acquirable"
    );
}

pub(crate) async fn adjudication_evidence_is_bounded(fixture: &impl RefreshClaimFixture, seed: u8) {
    let credential = fixture.credential("adjudication_evidence_is_bounded");
    poison(fixture, &credential, seed).await;
    let oversized = "x".repeat(MAX_ADJUDICATION_EVIDENCE_BYTES + 1);

    for evidence in ["", oversized.as_str()] {
        std::assert_matches!(
            fixture
                .adjudicate(
                    &credential,
                    RefreshOutcomeDecision::ProviderApplied,
                    evidence
                )
                .await,
            Err(AdjudicationError::InvalidEvidence),
            "evidence must be a non-empty, bounded operator note"
        );
    }
    assert!(
        fixture.poisoned_claim_exists(&credential).await,
        "refused evidence must not clear the poison"
    );
    assert_eq!(
        fixture.incident_count(&credential).await,
        0,
        "refused evidence must not write an incident"
    );

    // The bound is the boundary: exactly `MAX_ADJUDICATION_EVIDENCE_BYTES` is
    // an admissible note, one byte more is not.
    let exact = "y".repeat(MAX_ADJUDICATION_EVIDENCE_BYTES);
    let recorded = adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderApplied,
        &exact,
    )
    .await;
    assert!(recorded.changed);
    assert!(!fixture.poisoned_claim_exists(&credential).await);
}

pub(crate) async fn recommitting_the_same_adjudication_is_a_no_op_that_adds_no_incident(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential =
        fixture.credential("recommitting_the_same_adjudication_is_a_no_op_that_adds_no_incident");
    let evidence = "provider support ticket INC-4711 confirms the refresh landed";
    poison(fixture, &credential, seed).await;
    sweep_accounts_poison(fixture, &credential).await;
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderApplied,
        evidence,
    )
    .await;
    assert_eq!(fixture.incident_count(&credential).await, 1);

    let recommit = fixture
        .adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            evidence,
        )
        .await
        .expect("an identical recommit is the authorized follow-up, not a conflict");
    assert!(
        !recommit.changed,
        "the recorded decision must be reported, not re-recorded"
    );
    assert_eq!(
        recommit.decision,
        RefreshOutcomeDecision::ProviderApplied,
        "the recommit must return the recorded resolution"
    );
    assert_eq!(
        recommit.evidence_digest,
        evidence_digest(evidence),
        "the recommit must report the matched digest on record"
    );
    assert_eq!(
        fixture.incident_count(&credential).await,
        1,
        "an idempotent recommit must not write a second incident"
    );
    assert_eq!(fixture.resolved_incident_count(&credential).await, 1);
    assert!(!fixture.poisoned_claim_exists(&credential).await);
}

pub(crate) async fn recommit_of_an_older_resolution_is_a_no_op_not_a_conflict(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // The **no-poison** replay path answers this case, and that is the branch
    // under test: each lifecycle's reconcile clears its own claim row, so by the
    // final recommit there is no poison left to read and the rule applied is the
    // exact match over the credential's resolved set. The poisoned branch's
    // claim-keyed comparison is not this case's subject and has no case here —
    // no port-reachable path builds a live poisoned row whose incident already
    // carries a resolution (see the note in the adapters), so no behavioural
    // test can cover it.
    //
    // The re-committed pair is compared against the incident it describes, not
    // against whichever resolved incident happens to be newest: a credential
    // reconciled twice holds two resolutions, and the holder of the older
    // outcome is still entitled to the `AcknowledgementUnknown` recovery that
    // re-commits it. Comparing against the newer resolution denies that
    // recovery outright when the newer incident decided the other way.
    let credential =
        fixture.credential("recommit_of_an_older_resolution_is_a_no_op_not_a_conflict");
    let older_evidence = "provider support ticket INC-4711 confirms the first refresh landed";

    poison(fixture, &credential, seed).await;
    sweep_accounts_poison(fixture, &credential).await;
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderApplied,
        older_evidence,
    )
    .await;

    // A second, later lifecycle decides the opposite way, so the newest
    // resolution on record contradicts the older request on both the digest and
    // the decision.
    poison(fixture, &credential, seed.wrapping_add(1)).await;
    sweep_accounts_poison(fixture, &credential).await;
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderNotApplied,
        "provider support ticket INC-4712 confirms the second refresh never landed",
    )
    .await;
    assert_eq!(
        fixture.resolved_incident_count(&credential).await,
        2,
        "both lifecycles are on record as decided"
    );

    let recommit = fixture
        .adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            older_evidence,
        )
        .await
        .expect("re-committing a genuine earlier resolution must not conflict with a newer one");
    assert!(
        !recommit.changed,
        "the earlier resolution is already on record, so this call records nothing"
    );
    assert_eq!(
        recommit.decision,
        RefreshOutcomeDecision::ProviderApplied,
        "the recommit answers with the resolution it matched"
    );
    assert_eq!(
        recommit.evidence_digest,
        evidence_digest(older_evidence),
        "a superseded replay must report the digest of the pair it matched, \
         not the newest resolution's"
    );
    assert_eq!(
        fixture.incident_count(&credential).await,
        2,
        "an idempotent recommit must not write a third incident"
    );
}

pub(crate) async fn a_conflicting_adjudication_is_refused(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("a_conflicting_adjudication_is_refused");
    let evidence = "provider support ticket INC-4711 confirms the refresh landed";
    poison(fixture, &credential, seed).await;
    sweep_accounts_poison(fixture, &credential).await;
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderApplied,
        evidence,
    )
    .await;

    std::assert_matches!(
        fixture
            .adjudicate(
                &credential,
                RefreshOutcomeDecision::ProviderNotApplied,
                evidence
            )
            .await,
        Err(AdjudicationError::EvidenceConflict {
            recorded_digest,
            recorded_decision,
        }) if recorded_digest == evidence_digest(evidence)
            && recorded_decision == RefreshOutcomeDecision::ProviderApplied,
        "the same evidence cannot be turned into the opposite decision, and the \
         refusal must name the pair on record"
    );
    std::assert_matches!(
        fixture
            .adjudicate(
                &credential,
                RefreshOutcomeDecision::ProviderApplied,
                "a different reconciliation query reached the same conclusion"
            )
            .await,
        Err(AdjudicationError::EvidenceConflict {
            recorded_digest,
            recorded_decision,
        }) if recorded_digest == evidence_digest(evidence)
            && recorded_decision == RefreshOutcomeDecision::ProviderApplied,
        "different evidence for a decided incident is a conflict, and the \
         refusal must name the pair on record"
    );

    // Reconciliation resolves an unknown outcome; it does not overrule a
    // recorded one, and a refused conflict changes nothing. Both refusals above
    // came from the no-poison branch, which issues no DELETE at all, and the
    // row was already cleared before them — so this asserts that a refused
    // conflict leaves the store as it found it, not that it avoided
    // resurrecting a poison that was still there.
    let recorded = fixture
        .adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            evidence,
        )
        .await
        .expect("the recorded pair is still accepted");
    assert!(!recorded.changed);
    assert_eq!(fixture.incident_count(&credential).await, 1);
    assert_eq!(fixture.resolved_incident_count(&credential).await, 1);
    assert!(
        !fixture.poisoned_claim_exists(&credential).await,
        "a refused conflict must leave the claim row cleared"
    );
}

pub(crate) async fn concurrent_adjudication_yields_exactly_one_change(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("concurrent_adjudication_yields_exactly_one_change");
    let evidence = "the only reconciliation note for this incident";
    poison(fixture, &credential, seed).await;
    sweep_accounts_poison(fixture, &credential).await;

    let (left, right) = tokio::join!(
        fixture.adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            evidence
        ),
        fixture.adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            evidence
        ),
    );

    // Recommitting the same decision is the authorized follow-up to a lost
    // acknowledgement, so the loser of the race must be a no-op rather than a
    // `Storage` or `AcknowledgementUnknown` failure.
    for (side, result) in [("left", &left), ("right", &right)] {
        if let Err(error) = result {
            panic!("the {side} adjudication must not fail: {error:?}");
        }
    }
    let changed = [&left, &right]
        .into_iter()
        .filter(|result| matches!(result, Ok(adjudication) if adjudication.changed))
        .count();
    let unchanged = [&left, &right]
        .into_iter()
        .filter(|result| matches!(result, Ok(adjudication) if !adjudication.changed))
        .count();
    assert_eq!(changed, 1, "exactly one adjudication records the decision");
    assert_eq!(unchanged, 1, "the other observes the recorded decision");

    assert_eq!(fixture.incident_count(&credential).await, 1);
    assert_eq!(fixture.resolved_incident_count(&credential).await, 1);
    assert!(!fixture.poisoned_claim_exists(&credential).await);
    let recorded = fixture
        .adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            evidence,
        )
        .await
        .expect("the race's winner recorded this exact pair");
    assert!(!recorded.changed);
}

pub(crate) async fn release_refuses_an_undecided_incident(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("release_refuses_an_undecided_incident");
    let claim = poison(fixture, &credential, seed).await;
    sweep_accounts_poison(fixture, &credential).await;
    assert_eq!(fixture.resolved_incident_count(&credential).await, 0);

    let error = fixture
        .release(claim.token.clone())
        .await
        .expect_err("release must refuse a claim whose incident is undecided");
    std::assert_matches!(error, RepoError::ReleaseRefused);
    assert!(
        fixture.poisoned_claim_exists(&credential).await,
        "a refused release must not delete the poison row"
    );
    assert_eq!(fixture.incident_count(&credential).await, 1);
    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a poisoned acquisition must not fail");
    std::assert_matches!(
        attempt,
        ClaimAttempt::OutcomeUnknown { .. },
        "the retained poison still refuses egress"
    );

    // The refusal ends with the decision, not with another release.
    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderApplied,
        "provider support ticket INC-4711 confirms the refresh landed",
    )
    .await;
    fixture
        .release(claim.token)
        .await
        .expect("with the incident decided the retained row is gone, so release is a no-op");
    assert_eq!(
        fixture.incident_count(&credential).await,
        1,
        "the release must not have added an incident"
    );
}

pub(crate) async fn release_deletes_a_claim_that_carries_no_incident(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential = fixture.credential("release_deletes_a_claim_that_carries_no_incident");
    let claim = acquire(fixture, &credential, seed).await;
    assert!(
        !fixture.poisoned_claim_exists(&credential).await,
        "a live claim is not poison"
    );

    fixture
        .release(claim.token.clone())
        .await
        .expect("exact pre-provider cleanup must delete the row");
    assert!(!fixture.poisoned_claim_exists(&credential).await);
    assert_eq!(fixture.incident_count(&credential).await, 0);
    fixture
        .release(claim.token.clone())
        .await
        .expect("releasing the row that is already gone is a no-op");

    // A superseded generation must not release the current holder's claim on
    // the same credential.
    let superseded = fixture.credential("release_deletes_a_claim_that_carries_no_incident/retaken");
    let stale = acquire(fixture, &superseded, seed).await;
    fixture.expire_claims(&superseded).await;
    let current = acquire(fixture, &superseded, seed.wrapping_add(1)).await;
    fixture
        .release(stale.token)
        .await
        .expect("a superseded generation's release is a no-op");
    let attempt = fixture
        .try_claim(
            &superseded,
            &fixture.replica(seed.wrapping_add(2)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a contended acquisition must not fail");
    std::assert_matches!(
        attempt,
        ClaimAttempt::Contended { .. },
        "a superseded release must not delete the current holder's claim"
    );
    assert_eq!(current.token.generation, 1);
}

pub(crate) async fn a_reconciled_claim_never_authorizes_its_former_holder(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    // Reconciliation is the authority that ends the poisoned lifecycle, so the
    // holder that observed `OutcomeUnknown` is left with a token that proves
    // nothing: another replica is free to claim the credential, and the old
    // token must not extend a lease or open provider egress on that new row.
    let credential = fixture.credential("a_reconciled_claim_never_authorizes_its_former_holder");
    let claim = poison(fixture, &credential, seed).await;
    assert!(
        fixture.poisoned_claim_exists(&credential).await,
        "the expired in-flight row is the poison"
    );

    adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderNotApplied,
        "operator confirmed the call never left the replica",
    )
    .await;
    assert!(!fixture.poisoned_claim_exists(&credential).await);

    std::assert_matches!(
        fixture.heartbeat(&claim.token, fixture.claim_ttl()).await,
        Err(HeartbeatError::ClaimLost),
        "a reconciled claim leaves nothing for a stale holder to extend"
    );
    std::assert_matches!(
        fixture.mark_sentinel(&claim.token).await,
        Err(RepoError::InvalidState),
        "a reconciled claim cannot be re-opened for provider egress"
    );
    let next = acquire(fixture, &credential, seed.wrapping_add(1)).await;
    assert_eq!(
        next.token.generation, 0,
        "the reconciled row is gone, so the next holder starts a new lifecycle"
    );
    assert_ne!(
        next.token.claim_id, claim.token.claim_id,
        "the stale token belongs to no row the store still answers with"
    );
}

pub(crate) async fn a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded(
    fixture: &impl RefreshClaimFixture,
    seed: u8,
) {
    let credential =
        fixture.credential("a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded");
    let evidence = "provider support ticket INC-4711 confirms the refresh landed";
    poison(fixture, &credential, seed).await;
    sweep_accounts_poison(fixture, &credential).await;
    assert_eq!(fixture.resolved_incident_count(&credential).await, 0);

    fixture.inject_decision_write_failure(&credential).await;
    let error = fixture
        .adjudicate(
            &credential,
            RefreshOutcomeDecision::ProviderApplied,
            evidence,
        )
        .await
        .expect_err("the injected failure must surface");
    std::assert_matches!(
        error,
        AdjudicationError::Storage,
        "a failed statement is a storage failure, not a lost acknowledgement"
    );

    // Clearing the claim and recording its resolution are one transaction:
    // neither committed, so the credential is still poison and no decision is
    // on record.
    assert!(
        fixture.poisoned_claim_exists(&credential).await,
        "the claim-row delete must have rolled back with the failed write"
    );
    assert_eq!(fixture.incident_count(&credential).await, 1);
    assert_eq!(
        fixture.resolved_incident_count(&credential).await,
        0,
        "the resolution must not be committed"
    );
    let attempt = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a poisoned acquisition must not fail");
    std::assert_matches!(attempt, ClaimAttempt::OutcomeUnknown { .. });

    // With the injection removed the same call succeeds, so what failed was the
    // injected write and not the claim.
    fixture.clear_decision_write_failure(&credential).await;
    let recorded = adjudicate_fresh(
        fixture,
        &credential,
        RefreshOutcomeDecision::ProviderApplied,
        evidence,
    )
    .await;
    assert!(recorded.changed);
    assert!(!fixture.poisoned_claim_exists(&credential).await);
    assert_eq!(fixture.resolved_incident_count(&credential).await, 1);
    let next = fixture
        .try_claim(
            &credential,
            &fixture.replica(seed.wrapping_add(1)),
            fixture.claim_ttl(),
        )
        .await
        .expect("a reconciled credential must be acquirable");
    std::assert_matches!(next, ClaimAttempt::Acquired(_));
}

/// Generate one `#[tokio::test]` per shared case against `$fixture`.
///
/// `$fixture` is an async expression yielding `Option<impl RefreshClaimFixture>`;
/// `None` means this backend is unreachable in the current environment and the
/// case fails loudly rather than asserting against a substitute. The expression
/// is evaluated once per case, so each case gets its own isolated fixture — not
/// a shared store that would let one case's poison become another's.
///
/// The one backend-conditional case is not generated here; each runner declares
/// it, so a backend that cannot run it can mark it `#[ignore]`. See the note at
/// the end of the list.
///
/// The including file must declare this module as `oracle`.
#[macro_export]
macro_rules! refresh_claim_conformance_suite {
    ($fixture:expr) => {
        $crate::refresh_claim_case!(try_claim_acquires_when_no_holder, 0x11, $fixture);
        $crate::refresh_claim_case!(try_claim_returns_contended_when_held, 0x12, $fixture);
        $crate::refresh_claim_case!(
            try_claim_acquire_then_release_then_reacquire,
            0x13,
            $fixture
        );
        $crate::refresh_claim_case!(
            heartbeat_extends_expiry_and_rejects_a_stale_token,
            0x14,
            $fixture
        );
        $crate::refresh_claim_case!(release_is_idempotent, 0x15, $fixture);
        $crate::refresh_claim_case!(
            reclaim_accounts_expired_in_flight_as_retained_poison,
            0x16,
            $fixture
        );
        $crate::refresh_claim_case!(
            mark_sentinel_after_reclaim_returns_invalid_state,
            0x17,
            $fixture
        );
        $crate::refresh_claim_case!(
            expired_normal_claim_can_be_taken_over_in_place,
            0x18,
            $fixture
        );
        $crate::refresh_claim_case!(
            out_of_range_generation_is_rejected_without_touching_the_claim,
            0x19,
            $fixture
        );
        $crate::refresh_claim_case!(expired_claim_cannot_be_marked_in_flight, 0x1A, $fixture);
        $crate::refresh_claim_case!(
            expired_in_flight_claim_is_preserved_until_reclaim,
            0x1B,
            $fixture
        );
        $crate::refresh_claim_case!(
            exact_confirmed_release_clears_expired_in_flight_claim,
            0x1C,
            $fixture
        );
        $crate::refresh_claim_case!(
            old_generation_zero_evidence_does_not_mask_a_new_claim_lifecycle,
            0x1D,
            $fixture
        );
        $crate::refresh_claim_case!(concurrent_try_claim_yields_one_acquired, 0x1E, $fixture);
        $crate::refresh_claim_case!(
            concurrent_reclaim_accounts_each_poison_exactly_once,
            0x1F,
            $fixture
        );
        $crate::refresh_claim_case!(sentinel_event_count_in_window, 0x20, $fixture);
        $crate::refresh_claim_case!(sentinel_count_filters_by_credential_id, 0x21, $fixture);
        $crate::refresh_claim_case!(
            sentinel_count_excludes_events_before_window_start,
            0x22,
            $fixture
        );
        $crate::refresh_claim_case!(
            accounted_sentinel_events_are_windowed_and_credential_scoped,
            0x23,
            $fixture
        );
        $crate::refresh_claim_case!(
            adjudication_clears_poison_and_the_claim_becomes_acquirable,
            0x24,
            $fixture
        );
        $crate::refresh_claim_case!(
            adjudication_of_an_unpoisoned_credential_is_refused,
            0x25,
            $fixture
        );
        $crate::refresh_claim_case!(adjudication_evidence_is_bounded, 0x26, $fixture);
        $crate::refresh_claim_case!(
            recommitting_the_same_adjudication_is_a_no_op_that_adds_no_incident,
            0x27,
            $fixture
        );
        $crate::refresh_claim_case!(
            recommit_of_an_older_resolution_is_a_no_op_not_a_conflict,
            0x2E,
            $fixture
        );
        $crate::refresh_claim_case!(a_conflicting_adjudication_is_refused, 0x28, $fixture);
        $crate::refresh_claim_case!(
            concurrent_adjudication_yields_exactly_one_change,
            0x29,
            $fixture
        );
        $crate::refresh_claim_case!(release_refuses_an_undecided_incident, 0x2A, $fixture);
        $crate::refresh_claim_case!(
            release_deletes_a_claim_that_carries_no_incident,
            0x2B,
            $fixture
        );
        $crate::refresh_claim_case!(
            a_reconciled_claim_never_authorizes_its_former_holder,
            0x2C,
            $fixture
        );
        // `a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded` is
        // not generated here: each runner declares it in its own file, so a
        // backend that cannot run it marks the generated test `#[ignore]` and
        // the skip shows up in nextest's denominator instead of a pass. See the
        // runner module docs for the per-backend counts.
    };
}

/// Generate one `#[tokio::test]` for `$case` against `$fixture`.
///
/// `$fixture` is an async expression yielding `Option<impl RefreshClaimFixture>`.
/// `None` is a hard failure naming the unreachable backend, never a silent pass
/// and never a skip: a case body that returns early is counted as a pass, so a
/// backend the case cannot run against has to fail loudly instead. A green run
/// therefore means every case ran against a live backend.
///
/// Leading attributes are forwarded to the generated test, which is how the one
/// backend-conditional case is marked `#[ignore]` on the backend that cannot run
/// it: `#[ignore]` is the only skip path, and it is the one that shows up in
/// nextest's denominator.
#[macro_export]
macro_rules! refresh_claim_case {
    ($(#[$attr:meta])* $case:ident, $seed:expr, $fixture:expr) => {
        $(#[$attr])*
        #[tokio::test]
        async fn $case() {
            let Some(fixture) = $fixture.await else {
                panic!(concat!(
                    stringify!($case),
                    ": backend unreachable — the case cannot run and must fail rather than \
                     pass unchecked; reach the backend (set DATABASE_URL for postgres) or run \
                     without this feature"
                ));
            };
            oracle::$case(&fixture, $seed).await;
        }
    };
}
