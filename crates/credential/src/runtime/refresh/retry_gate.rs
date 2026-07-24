//! Conversion between integration-facing refresh evidence and the structural
//! persistence retry gate.

use nebula_storage_port::{
    CredentialMaterialTransition, CredentialPersistence, CredentialPersistenceError,
    CredentialReplacement, CredentialSelector, RefreshRetryAdmission, RefreshRetryBlock,
    RefreshRetryDelay, RefreshRetryDiagnosticCode, RefreshRetryEvidence, RefreshRetryKind,
    RefreshRetryPhase, RefreshRetryTransition, StoredCredential, StoredLiveCredential,
};

use crate::error::{
    RefreshDiagnosticCode, RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext,
    RefreshNotAppliedPhase, RetryAdvice, RetryDelay,
};

/// A supposedly equivalent credential/storage retry representation drifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RefreshRetryGateConversionError {
    /// The fixed diagnostic-code contracts no longer agree.
    #[error("credential refresh retry diagnostic contract mismatch")]
    DiagnosticCode,
    /// The bounded whole-second delay contracts no longer agree.
    #[error("credential refresh retry delay contract mismatch")]
    Delay,
}

/// Convert one proof-bearing exact failure into a structural CAS transition.
pub(crate) fn transition_from_context(
    context: &RefreshNotAppliedContext,
) -> Result<RefreshRetryTransition, RefreshRetryGateConversionError> {
    let evidence = evidence_from_context(context)?;
    match context.retry() {
        RetryAdvice::Never => Ok(RefreshRetryTransition::SetNever { evidence }),
        RetryAdvice::After(delay) => {
            let delay = RefreshRetryDelay::new(delay.get())
                .map_err(|_| RefreshRetryGateConversionError::Delay)?;
            Ok(RefreshRetryTransition::SetAfter { delay, evidence })
        },
    }
}

/// Reconstruct the typed exact failure from backend-clock gate admission.
pub(crate) fn context_from_block(
    block: RefreshRetryBlock,
) -> Result<Box<RefreshNotAppliedContext>, RefreshRetryGateConversionError> {
    let (evidence, retry) = match block {
        RefreshRetryBlock::Never { evidence } => (evidence, RetryAdvice::Never),
        RefreshRetryBlock::After {
            remaining,
            evidence,
        } => {
            let delay = RetryDelay::new(remaining.get())
                .map_err(|_| RefreshRetryGateConversionError::Delay)?;
            (evidence, RetryAdvice::After(delay))
        },
    };

    let phase = match evidence.phase() {
        RefreshRetryPhase::BeforeDispatch => RefreshNotAppliedPhase::BeforeDispatch,
        RefreshRetryPhase::ProviderConfirmedNotApplied => {
            RefreshNotAppliedPhase::ProviderConfirmedNotApplied
        },
    };
    let kind = match evidence.kind() {
        RefreshRetryKind::TransientNetwork => RefreshErrorKind::TransientNetwork,
        RefreshRetryKind::ProviderUnavailable => RefreshErrorKind::ProviderUnavailable,
        RefreshRetryKind::ProtocolError => RefreshErrorKind::ProtocolError,
    };
    let mut spec = RefreshFailureSpec::new(kind, retry);
    if let Some(code) = evidence.diagnostic_code() {
        let code = RefreshDiagnosticCode::parse(code.as_str())
            .map_err(|_| RefreshRetryGateConversionError::DiagnosticCode)?;
        spec = spec.with_diagnostic_code(code);
    }
    Ok(Box::new(RefreshNotAppliedContext::from_spec(phase, spec)))
}

fn evidence_from_context(
    context: &RefreshNotAppliedContext,
) -> Result<RefreshRetryEvidence, RefreshRetryGateConversionError> {
    let phase = match context.phase() {
        RefreshNotAppliedPhase::BeforeDispatch => RefreshRetryPhase::BeforeDispatch,
        RefreshNotAppliedPhase::ProviderConfirmedNotApplied => {
            RefreshRetryPhase::ProviderConfirmedNotApplied
        },
    };
    let kind = match context.kind() {
        RefreshErrorKind::TransientNetwork => RefreshRetryKind::TransientNetwork,
        RefreshErrorKind::ProviderUnavailable => RefreshRetryKind::ProviderUnavailable,
        RefreshErrorKind::ProtocolError => RefreshRetryKind::ProtocolError,
    };
    let diagnostic_code = context
        .diagnostic_code()
        .map(|code| RefreshRetryDiagnosticCode::parse(code.as_str()))
        .transpose()
        .map_err(|_| RefreshRetryGateConversionError::DiagnosticCode)?;
    Ok(RefreshRetryEvidence::new(phase, kind, diagnostic_code))
}

static_assertions::const_assert_eq!(RetryDelay::MAX_SECS, RefreshRetryDelay::MAX_SECS);

const MAX_DISPLAY_RACE_RETRIES: usize = 3;

/// Result of durably installing a proof-bearing retry gate.
pub(crate) enum RetryGateWrite {
    /// The requested gate, or an equivalent already-authoritative gate for the
    /// same refresh material, is now durable.
    Applied(Box<RefreshNotAppliedContext>),
    /// A newer credential-material epoch superseded the attempted gate.
    Superseded(CredentialPersistenceError),
    /// Persistence definitely rejected the gate transition.
    DefiniteFailure(CredentialPersistenceError),
    /// A persistence write or reconciliation-read acknowledgement was
    /// genuinely ambiguous, so the transition may have committed.
    ///
    /// Bounded display-CAS churn is a definite conflict and never enters this
    /// variant. The refresh claim must be retained.
    OutcomeUnknown,
}

/// Result of durably marking an unchanged credential as requiring reauth.
pub(crate) enum ReauthWrite {
    /// The row now durably carries `reauth_required = true`.
    Applied,
    /// New credential material superseded the old reauth decision.
    Superseded(CredentialPersistenceError),
    /// Persistence definitely rejected the transition.
    DefiniteFailure(CredentialPersistenceError),
    /// A persistence commit/read acknowledgement is genuinely uncertain.
    OutcomeUnknown,
}

/// Install a retry gate without losing a concurrent display-only mutation.
///
/// A CAS conflict is re-read and retried only while the durable material epoch
/// still identifies the same refresh authority. Material replacement advances
/// that epoch even when the serialized bytes happen to be identical and
/// supersedes the old provider result. Repeated display churn is bounded and
/// fails closed so it cannot turn an exact no-effect result into an immediate
/// duplicate provider request.
pub(crate) async fn persist_retry_gate<S>(
    store: &S,
    selector: &CredentialSelector,
    observed: StoredLiveCredential,
    context: Box<RefreshNotAppliedContext>,
) -> RetryGateWrite
where
    S: CredentialPersistence + ?Sized,
{
    let transition = match transition_from_context(&context) {
        Ok(transition) => transition,
        Err(_) => {
            return RetryGateWrite::DefiniteFailure(CredentialPersistenceError::CorruptRecord);
        },
    };
    let baseline = observed.clone();
    let mut current = observed;
    let mut last_conflict = None;

    for _ in 0..MAX_DISPLAY_RACE_RETRIES {
        let replacement =
            unchanged_replacement(&current, current.reauth_required(), transition.clone());
        match store.replace(selector, replacement).await {
            Ok(_) => return RetryGateWrite::Applied(context),
            Err(CredentialPersistenceError::OutcomeUnknown) => {
                return RetryGateWrite::OutcomeUnknown;
            },
            Err(conflict @ CredentialPersistenceError::VersionConflict { .. }) => {
                last_conflict = Some(conflict);
                let latest = match store.get(selector).await {
                    Ok(StoredCredential::Live(latest)) => latest,
                    Ok(StoredCredential::Tombstoned(_)) => {
                        return RetryGateWrite::Superseded(CredentialPersistenceError::NotFound);
                    },
                    Err(CredentialPersistenceError::OutcomeUnknown) => {
                        return RetryGateWrite::OutcomeUnknown;
                    },
                    Err(error) => return RetryGateWrite::DefiniteFailure(error),
                };
                if !same_refresh_authority(&baseline, &latest) {
                    return RetryGateWrite::Superseded(conflict);
                }
                match store.refresh_retry_snapshot(selector).await {
                    Ok(snapshot) => match snapshot.admission() {
                        RefreshRetryAdmission::Blocked(block) => {
                            return match context_from_block(block.clone()) {
                                Ok(context) => RetryGateWrite::Applied(context),
                                Err(_) => RetryGateWrite::DefiniteFailure(
                                    CredentialPersistenceError::CorruptRecord,
                                ),
                            };
                        },
                        RefreshRetryAdmission::Open => current = latest,
                    },
                    Err(CredentialPersistenceError::OutcomeUnknown) => {
                        return RetryGateWrite::OutcomeUnknown;
                    },
                    Err(error) => return RetryGateWrite::DefiniteFailure(error),
                }
            },
            Err(error) => return RetryGateWrite::DefiniteFailure(error),
        }
    }

    RetryGateWrite::DefiniteFailure(
        last_conflict.unwrap_or(CredentialPersistenceError::CorruptRecord),
    )
}

/// Persist `reauth_required = true`, advancing refresh authority while merging
/// concurrent display-only writes.
pub(crate) async fn persist_reauth_required<S>(
    store: &S,
    selector: &CredentialSelector,
    observed: StoredLiveCredential,
) -> ReauthWrite
where
    S: CredentialPersistence + ?Sized,
{
    let baseline = observed.clone();
    let mut current = observed;
    let mut last_conflict = None;

    for _ in 0..MAX_DISPLAY_RACE_RETRIES {
        let replacement = reauth_replacement(&current);
        match store.replace(selector, replacement).await {
            Ok(_) => return ReauthWrite::Applied,
            Err(CredentialPersistenceError::OutcomeUnknown) => {
                return ReauthWrite::OutcomeUnknown;
            },
            Err(conflict @ CredentialPersistenceError::VersionConflict { .. }) => {
                last_conflict = Some(conflict);
                let latest = match store.get(selector).await {
                    Ok(StoredCredential::Live(latest)) => latest,
                    Ok(StoredCredential::Tombstoned(_)) => {
                        return ReauthWrite::Superseded(CredentialPersistenceError::NotFound);
                    },
                    Err(CredentialPersistenceError::OutcomeUnknown) => {
                        return ReauthWrite::OutcomeUnknown;
                    },
                    Err(error) => return ReauthWrite::DefiniteFailure(error),
                };
                if latest.reauth_required() {
                    return ReauthWrite::Applied;
                }
                if !same_refresh_authority(&baseline, &latest) {
                    return ReauthWrite::Superseded(conflict);
                }
                current = latest;
            },
            Err(error) => return ReauthWrite::DefiniteFailure(error),
        }
    }

    ReauthWrite::DefiniteFailure(last_conflict.unwrap_or(CredentialPersistenceError::CorruptRecord))
}

fn unchanged_replacement(
    current: &StoredLiveCredential,
    reauth_required: bool,
    transition: RefreshRetryTransition,
) -> CredentialReplacement {
    CredentialReplacement::new(
        current.version(),
        current.data().clone(),
        current.state_kind().to_owned(),
        current.state_version(),
        current.name().map(str::to_owned),
        current.expires_at(),
        reauth_required,
        current.metadata().clone(),
        CredentialMaterialTransition::preserve(transition),
    )
}

fn reauth_replacement(current: &StoredLiveCredential) -> CredentialReplacement {
    CredentialReplacement::new(
        current.version(),
        current.data().clone(),
        current.state_kind().to_owned(),
        current.state_version(),
        current.name().map(str::to_owned),
        current.expires_at(),
        true,
        current.metadata().clone(),
        CredentialMaterialTransition::advance(),
    )
}

fn same_refresh_authority(baseline: &StoredLiveCredential, current: &StoredLiveCredential) -> bool {
    baseline.material_epoch() == current.material_epoch()
}
