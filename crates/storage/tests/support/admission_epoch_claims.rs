//! Claim-side admission-epoch cases for the two SQL deployment backends.
//!
//! The credential admission epoch (the use revision) advances in the same
//! transaction as every write that closes credential use. The replacement-side
//! bumps are pinned by the credential semantic oracle; these cases pin the
//! claim-side ones — a won revoke claim, a provider-egress sentinel, and
//! threshold escalation — and the two edges that reopen use without any write:
//! a sentinel-free revoke claim reaching expiry, and a pre-sweep release.
//!
//! They run only where claims and credentials share one transactional backend.
//! The in-memory claim repository is a separate object from any credential
//! store and cannot advance the epoch, which the port documents as a
//! limitation; the shared refresh-claim oracle therefore does not carry them.
//!
//! Every case reads the epoch the way a consumer does — through the credential
//! store's authoritative operation status — and reads the raw row only to prove
//! that a claim-side bump moved neither `version` nor `updated_at`.

use std::time::Duration;

use nebula_core::CredentialId;
use nebula_storage::credential::refresh_claim::{
    CredentialIncidentRef, CredentialOperationDecision, CredentialOperationIntent,
    RefreshClaimAdjudicator, RefreshOutcomeDecision,
};
use nebula_storage::credential::{
    ClaimAttempt, ExpiredClaim, RefreshClaim, RefreshClaimReclaimer, RefreshClaimRepo, ReplicaId,
    RepoError, SentinelEscalationPolicy,
};
use nebula_storage_port::store::{CredentialOperationKind, CredentialOperationStatus};
use nebula_storage_port::{
    CredentialAdmissionEpoch, CredentialCreate, CredentialMaterialEpoch, CredentialOwner,
    CredentialPersistence, CredentialSelector, SecretBytes,
};

/// The production lease; expiry is always forced through the backend.
const TTL: Duration = Duration::from_secs(30);

/// Raw aggregate columns a claim-side bump must, or must not, move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Authority {
    pub(crate) version: i64,
    pub(crate) material_epoch: i64,
    pub(crate) admission_epoch: i64,
    pub(crate) updated_at: String,
    pub(crate) reauth_required: bool,
}

/// One SQL backend: a credential store and the claim repository sharing its
/// database, plus the two seams a case cannot reach through the port.
#[async_trait::async_trait]
pub(crate) trait AdmissionEpochBackend: Send + Sync {
    /// Credential store over the backend's database.
    type Store: CredentialPersistence;
    /// Claim repository over the same database.
    type Claims: RefreshClaimRepo + RefreshClaimReclaimer + RefreshClaimAdjudicator;

    fn store(&self) -> &Self::Store;

    fn claims(&self) -> &Self::Claims;

    /// Put every claim on `selector` past its deadline, in the database.
    async fn expire_claim(&self, selector: &CredentialSelector);

    /// Read the raw aggregate columns of `selector`.
    async fn authority(&self, selector: &CredentialSelector) -> Authority;

    /// Place the live row of `selector` at an exact admission epoch.
    async fn force_admission_epoch(&self, selector: &CredentialSelector, epoch: i64);
}

async fn create<B: AdmissionEpochBackend>(backend: &B) -> CredentialSelector {
    let selector = CredentialSelector::new(
        CredentialOwner::from_canonical("admission-epoch-owner"),
        CredentialId::new(),
    );
    backend
        .store()
        .create(
            &selector,
            CredentialCreate::new(
                "provider.token".to_owned(),
                SecretBytes::new(b"admission-material".to_vec()),
                "active".to_owned(),
                1,
                None,
                None,
                false,
                Default::default(),
            ),
        )
        .await
        .expect("create a live credential");
    selector
}

async fn status<B: AdmissionEpochBackend>(
    backend: &B,
    selector: &CredentialSelector,
) -> CredentialOperationStatus {
    backend
        .store()
        .operation_status(selector)
        .await
        .expect("the operation status is readable")
}

/// The admission and material epochs of an `Open` status, or a panic naming
/// the status that was not open.
async fn open<B: AdmissionEpochBackend>(
    backend: &B,
    selector: &CredentialSelector,
) -> (i64, CredentialMaterialEpoch) {
    match status(backend, selector).await {
        CredentialOperationStatus::Open {
            admission_epoch,
            material_epoch,
            ..
        } => (admission_epoch.get(), material_epoch),
        other => panic!("the credential must read open, got {other:?}"),
    }
}

fn replica(seed: u8) -> ReplicaId {
    ReplicaId::new(format!("admission-replica-{seed:02x}"))
}

fn acquired(attempt: Result<ClaimAttempt, RepoError>) -> RefreshClaim {
    match attempt.expect("the claim attempt reaches the backend") {
        ClaimAttempt::Acquired(claim) => claim,
        other => panic!("the claim must be acquired, got {other:?}"),
    }
}

fn revoke(material_epoch: CredentialMaterialEpoch) -> CredentialOperationIntent {
    CredentialOperationIntent::Revoke { material_epoch }
}

/// Assert that only the admission epoch moved between two raw reads.
fn only_admission_moved(before: &Authority, after: &Authority, by: i64) {
    assert_eq!(after.admission_epoch, before.admission_epoch + by);
    assert_eq!(
        after.version, before.version,
        "an admission bump is not a version"
    );
    assert_eq!(after.material_epoch, before.material_epoch);
    assert_eq!(
        after.updated_at, before.updated_at,
        "an admission bump is not an aggregate mutation"
    );
    assert_eq!(after.reauth_required, before.reauth_required);
}

/// A won revoke CAS advances the epoch once; a lost claim and a material-epoch
/// conflict advance nothing.
pub(crate) async fn revoke_claim_win_advances_admission_and_loss_or_conflict_does_not<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let (epoch, material) = open(backend, &selector).await;
    assert_eq!(epoch, CredentialAdmissionEpoch::MIN.get());
    let before = backend.authority(&selector).await;

    acquired(
        backend
            .claims()
            .try_claim(&selector, &replica(1), TTL, revoke(material))
            .await,
    );
    only_admission_moved(&before, &backend.authority(&selector).await, 1);
    assert_eq!(
        status(backend, &selector).await,
        CredentialOperationStatus::InFlight {
            operation: CredentialOperationKind::Revoke,
        }
    );

    for intent in [revoke(material), CredentialOperationIntent::Refresh] {
        assert!(matches!(
            backend
                .claims()
                .try_claim(&selector, &replica(2), TTL, intent)
                .await,
            Ok(ClaimAttempt::Contended { .. })
        ));
    }
    assert_eq!(
        backend.authority(&selector).await.admission_epoch,
        before.admission_epoch + 1,
        "a lost claim closes nothing"
    );

    let conflicted = create(backend).await;
    let conflicted_before = backend.authority(&conflicted).await;
    let stale = CredentialMaterialEpoch::MIN
        .next()
        .expect("epoch two is representable");
    assert!(matches!(
        backend
            .claims()
            .try_claim(&conflicted, &replica(1), TTL, revoke(stale))
            .await,
        Err(RepoError::MaterialEpochConflict { .. })
    ));
    assert_eq!(backend.authority(&conflicted).await, conflicted_before);
    assert_eq!(
        open(backend, &conflicted).await.0,
        conflicted_before.admission_epoch
    );
}

/// A revoke claim that never crossed the provider boundary reopens use by
/// clock expiry alone — and is observed at the epoch its acquisition advanced
/// to, so a consumer bound before the claim cannot mistake the interval for
/// continuity.
pub(crate) async fn abandoned_sentinel_free_revoke_reads_open_at_the_next_epoch<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let (epoch, material) = open(backend, &selector).await;
    acquired(
        backend
            .claims()
            .try_claim(&selector, &replica(1), TTL, revoke(material))
            .await,
    );
    backend.expire_claim(&selector).await;

    let (reopened, reopened_material) = open(backend, &selector).await;
    assert_eq!(
        reopened,
        epoch + 1,
        "the reopen edge has no write of its own"
    );
    assert_eq!(reopened_material, material);

    acquired(
        backend
            .claims()
            .try_claim(&selector, &replica(2), TTL, revoke(material))
            .await,
    );
    assert_eq!(
        backend.authority(&selector).await.admission_epoch,
        epoch + 2,
        "taking over the expired revoke claim closes use again"
    );
}

/// A refresh claim alone closes nothing; its sentinel does, and a release
/// after a no-state-change outcome reads open at the sentinel's epoch.
pub(crate) async fn refresh_claim_is_silent_until_the_sentinel_closes_use<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let (epoch, _) = open(backend, &selector).await;
    let before = backend.authority(&selector).await;
    let claim = acquired(
        backend
            .claims()
            .try_claim(
                &selector,
                &replica(1),
                TTL,
                CredentialOperationIntent::Refresh,
            )
            .await,
    );
    assert_eq!(backend.authority(&selector).await, before);
    assert_eq!(open(backend, &selector).await.0, epoch);

    backend
        .claims()
        .mark_sentinel(&claim.token)
        .await
        .expect("an unexpired token marks the sentinel");
    only_admission_moved(&before, &backend.authority(&selector).await, 1);
    assert_eq!(
        status(backend, &selector).await,
        CredentialOperationStatus::InFlight {
            operation: CredentialOperationKind::Refresh,
        }
    );

    backend
        .claims()
        .heartbeat(&claim.token, TTL)
        .await
        .expect("a heartbeat keeps the claim");
    backend
        .claims()
        .release(claim.token)
        .await
        .expect("release after a no-state-change outcome");
    assert_eq!(open(backend, &selector).await.0, epoch + 1);
    only_admission_moved(&before, &backend.authority(&selector).await, 1);
}

/// A holder whose claim expired after the sentinel may still release it
/// before the sweep accounts the incident; use then reopens at the epoch the
/// sentinel advanced to, for either operation kind.
pub(crate) async fn late_pre_sweep_release_reads_open_at_the_sentinel_epoch<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let (epoch, material) = open(backend, &selector).await;
    for (round, intent) in [CredentialOperationIntent::Refresh, revoke(material)]
        .into_iter()
        .enumerate()
    {
        let claim = acquired(
            backend
                .claims()
                .try_claim(&selector, &replica(1), TTL, intent)
                .await,
        );
        backend
            .claims()
            .mark_sentinel(&claim.token)
            .await
            .expect("mark the sentinel");
        backend.expire_claim(&selector).await;
        assert!(matches!(
            status(backend, &selector).await,
            CredentialOperationStatus::ReconciliationRequired { .. }
        ));
        backend
            .claims()
            .release(claim.token)
            .await
            .expect("the exact token releases before the sweep");
        // Refresh: +1 (sentinel). Revoke: +1 (won claim) +1 (sentinel).
        let expected = epoch + if round == 0 { 1 } else { 3 };
        assert_eq!(open(backend, &selector).await.0, expected);
    }
}

/// Reconciliation reopens use without a write of its own, above the epoch the
/// credential had before the claim.
pub(crate) async fn reconciled_refresh_reopens_above_the_pre_claim_epoch<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let (epoch, _) = open(backend, &selector).await;
    let claim = acquired(
        backend
            .claims()
            .try_claim(
                &selector,
                &replica(1),
                TTL,
                CredentialOperationIntent::Refresh,
            )
            .await,
    );
    backend
        .claims()
        .mark_sentinel(&claim.token)
        .await
        .expect("mark the sentinel");
    backend.expire_claim(&selector).await;
    let policy = SentinelEscalationPolicy::new(99, Duration::from_hours(1)).expect("valid policy");
    let accounted = backend
        .claims()
        .reclaim_stuck(policy)
        .await
        .expect("the sweep accounts the incident");
    assert!(accounted.iter().any(|expired| matches!(
        expired,
        ExpiredClaim::OutcomeUnknownAccounted { selector: accounted, .. } if accounted == &selector
    )));
    let incident = match status(backend, &selector).await {
        CredentialOperationStatus::ReconciliationRequired { incident, .. } => incident,
        other => panic!("the accounted claim requires reconciliation, got {other:?}"),
    };
    assert_eq!(
        incident,
        CredentialIncidentRef::from_uuid(claim.token.claim_id)
    );

    backend
        .claims()
        .adjudicate(
            &selector,
            incident,
            CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderNotApplied),
            "provider audit shows the refresh was not applied",
        )
        .await
        .expect("adjudicate the incident");
    let (reopened, _) = open(backend, &selector).await;
    assert!(
        reopened > epoch,
        "reconciliation reopens above the pre-claim epoch"
    );
    assert_eq!(reopened, epoch + 1);
}

/// Threshold escalation advances material authority and the admission epoch in
/// one statement, on top of the sentinel's own advance.
pub(crate) async fn threshold_escalation_advances_material_and_admission_together<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let before = backend.authority(&selector).await;
    let claim = acquired(
        backend
            .claims()
            .try_claim(
                &selector,
                &replica(1),
                TTL,
                CredentialOperationIntent::Refresh,
            )
            .await,
    );
    backend
        .claims()
        .mark_sentinel(&claim.token)
        .await
        .expect("mark the sentinel");
    backend.expire_claim(&selector).await;
    let policy = SentinelEscalationPolicy::new(1, Duration::from_hours(1)).expect("valid policy");
    backend
        .claims()
        .reclaim_stuck(policy)
        .await
        .expect("the sweep escalates");

    let after = backend.authority(&selector).await;
    assert_eq!(after.admission_epoch, before.admission_epoch + 2);
    assert_eq!(after.material_epoch, before.material_epoch + 1);
    assert_eq!(after.version, before.version + 1);
    assert!(after.reauth_required);
}

/// A sentinel refused for an expired or superseded token advances nothing.
pub(crate) async fn rejected_sentinel_advances_nothing<B: AdmissionEpochBackend>(backend: &B) {
    let selector = create(backend).await;
    let before = backend.authority(&selector).await;
    let stale = acquired(
        backend
            .claims()
            .try_claim(
                &selector,
                &replica(1),
                TTL,
                CredentialOperationIntent::Refresh,
            )
            .await,
    );
    backend.expire_claim(&selector).await;
    assert!(matches!(
        backend.claims().mark_sentinel(&stale.token).await,
        Err(RepoError::InvalidState)
    ));
    assert_eq!(backend.authority(&selector).await, before);

    acquired(
        backend
            .claims()
            .try_claim(
                &selector,
                &replica(2),
                TTL,
                CredentialOperationIntent::Refresh,
            )
            .await,
    );
    assert!(matches!(
        backend.claims().mark_sentinel(&stale.token).await,
        Err(RepoError::InvalidState)
    ));
    assert_eq!(backend.authority(&selector).await, before);
}

/// At the terminal epoch a claim transition that must close use fails closed
/// and commits nothing: the revoke claim is not acquired and the sentinel is
/// not marked.
pub(crate) async fn exhausted_admission_epoch_fails_claim_transitions_closed<
    B: AdmissionEpochBackend,
>(
    backend: &B,
) {
    let selector = create(backend).await;
    let (_, material) = open(backend, &selector).await;
    backend
        .force_admission_epoch(&selector, CredentialAdmissionEpoch::MAX.get())
        .await;
    let before = backend.authority(&selector).await;

    assert!(matches!(
        backend
            .claims()
            .try_claim(&selector, &replica(1), TTL, revoke(material))
            .await,
        Err(RepoError::AdmissionEpochExhausted)
    ));
    assert_eq!(backend.authority(&selector).await, before);
    assert_eq!(
        open(backend, &selector).await.0,
        CredentialAdmissionEpoch::MAX.get(),
        "the revoke claim was not acquired"
    );

    let claim = acquired(
        backend
            .claims()
            .try_claim(
                &selector,
                &replica(2),
                TTL,
                CredentialOperationIntent::Refresh,
            )
            .await,
    );
    assert!(matches!(
        backend.claims().mark_sentinel(&claim.token).await,
        Err(RepoError::AdmissionEpochExhausted)
    ));
    assert_eq!(backend.authority(&selector).await, before);
    assert_eq!(
        open(backend, &selector).await.0,
        CredentialAdmissionEpoch::MAX.get(),
        "an unmarked refresh claim still reads open"
    );
}

/// Instantiate every claim-side admission-epoch case for one backend.
///
/// `$fixture` is an expression yielding `Option<Backend>`; `None` fails the
/// case naming the unreachable backend rather than passing unchecked.
macro_rules! admission_epoch_claim_cases {
    ($fixture:expr) => {
        admission_epoch_claim_cases!(@case $fixture,
            revoke_claim_win_advances_admission_and_loss_or_conflict_does_not,
            abandoned_sentinel_free_revoke_reads_open_at_the_next_epoch,
            refresh_claim_is_silent_until_the_sentinel_closes_use,
            late_pre_sweep_release_reads_open_at_the_sentinel_epoch,
            reconciled_refresh_reopens_above_the_pre_claim_epoch,
            threshold_escalation_advances_material_and_admission_together,
            rejected_sentinel_advances_nothing,
            exhausted_admission_epoch_fails_claim_transitions_closed
        );
    };
    (@case $fixture:expr, $($case:ident),+ $(,)?) => {
        mod admission_epoch_claims_cases {
            use super::*;
            $(
                #[tokio::test]
                async fn $case() {
                    let Some(backend) = $fixture.await else {
                        panic!(concat!(
                            stringify!($case),
                            ": backend unreachable — the case cannot run and must fail \
                             rather than pass unchecked"
                        ));
                    };
                    super::admission_epoch_claims::$case(&backend).await;
                }
            )+
        }
    };
}
