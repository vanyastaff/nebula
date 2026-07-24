//! Conversion between integration-facing refresh evidence and the structural
//! persistence retry gate.

use nebula_storage_port::{
    CredentialMaterialTransition, CredentialPersistence, CredentialPersistenceError,
    CredentialReplacement, CredentialSelector, RefreshRetryAdmission, RefreshRetryBlock,
    RefreshRetryEvidence, RefreshRetryTransition, StoredCredential, StoredLiveCredential,
};

use crate::error::{RefreshFailureSpec, RefreshNotAppliedContext, RetryAdvice};

/// Convert one proof-bearing exact failure into a structural CAS transition.
pub(crate) fn transition_from_context(
    context: &RefreshNotAppliedContext,
) -> RefreshRetryTransition {
    let evidence = evidence_from_context(context);
    match context.retry() {
        RetryAdvice::Never => RefreshRetryTransition::SetNever { evidence },
        RetryAdvice::After(delay) => RefreshRetryTransition::SetAfter { delay, evidence },
    }
}

/// Reconstruct the typed exact failure from backend-clock gate admission.
pub(crate) fn context_from_block(block: RefreshRetryBlock) -> RefreshNotAppliedContext {
    let (evidence, retry) = match block {
        RefreshRetryBlock::Never { evidence } => (evidence, RetryAdvice::Never),
        RefreshRetryBlock::After {
            remaining,
            evidence,
        } => (evidence, RetryAdvice::After(remaining)),
    };

    let mut spec = RefreshFailureSpec::new(evidence.kind(), retry);
    if let Some(code) = evidence.diagnostic_code() {
        spec = spec.with_diagnostic_code(code.clone());
    }
    RefreshNotAppliedContext::from_spec(evidence.phase(), spec)
}

fn evidence_from_context(context: &RefreshNotAppliedContext) -> RefreshRetryEvidence {
    RefreshRetryEvidence::new(
        context.phase(),
        context.kind(),
        context.diagnostic_code().cloned(),
    )
}

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
    let transition = transition_from_context(&context);
    let baseline_epoch = observed.material_epoch();
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
                if !same_refresh_authority(baseline_epoch, &latest) {
                    return RetryGateWrite::Superseded(conflict);
                }
                match store.refresh_retry_snapshot(selector).await {
                    Ok(snapshot) => match snapshot.admission() {
                        RefreshRetryAdmission::Blocked(block) => {
                            return RetryGateWrite::Applied(Box::new(context_from_block(
                                block.clone(),
                            )));
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
    let baseline_epoch = observed.material_epoch();
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
                if !same_refresh_authority(baseline_epoch, &latest) {
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

fn same_refresh_authority(
    baseline_epoch: nebula_storage_port::CredentialMaterialEpoch,
    current: &StoredLiveCredential,
) -> bool {
    baseline_epoch == current.material_epoch()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_storage_port::{
        RefreshRetryBlock, RefreshRetryDiagnosticCode, RefreshRetryKind, RefreshRetryPhase,
        RefreshRetryTransition,
    };

    use super::{context_from_block, transition_from_context};
    use crate::error::{
        RefreshDiagnosticCode, RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext,
        RefreshNotAppliedPhase, RetryAdvice, RetryDelay,
    };

    #[test]
    fn credential_brands_are_the_storage_port_primitives() {
        let delay = RetryDelay::new(Duration::from_millis(1_001))
            .expect("canonical delay rounds conservatively");
        let code = RefreshDiagnosticCode::parse("oauth.server_error")
            .expect("fixed diagnostic code is valid");
        let context = RefreshNotAppliedContext::from_spec(
            RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
            RefreshFailureSpec::new(
                RefreshErrorKind::ProviderUnavailable,
                RetryAdvice::After(delay),
            )
            .with_diagnostic_code(code.clone()),
        );

        let transition = transition_from_context(&context);
        let RefreshRetryTransition::SetAfter {
            delay: stored_delay,
            evidence,
        } = transition
        else {
            panic!("timed advice must install a timed gate");
        };

        assert_eq!(stored_delay, delay);
        assert_eq!(
            evidence.phase(),
            RefreshRetryPhase::ProviderConfirmedNotApplied
        );
        assert_eq!(evidence.kind(), RefreshRetryKind::ProviderUnavailable);
        assert_eq!(
            evidence
                .diagnostic_code()
                .map(RefreshRetryDiagnosticCode::as_str),
            Some(code.as_str())
        );
    }

    #[test]
    fn durable_block_reuses_the_same_validated_values_infallibly() {
        let remaining = RetryDelay::new(Duration::from_secs(7)).expect("test delay is in range");
        let code =
            RefreshDiagnosticCode::parse("oauth.retry").expect("fixed diagnostic code is valid");
        let block = RefreshRetryBlock::After {
            remaining,
            evidence: nebula_storage_port::RefreshRetryEvidence::new(
                RefreshNotAppliedPhase::BeforeDispatch,
                RefreshErrorKind::TransientNetwork,
                Some(code.clone()),
            ),
        };

        let context = context_from_block(block);

        assert_eq!(context.phase(), RefreshNotAppliedPhase::BeforeDispatch);
        assert_eq!(context.kind(), RefreshErrorKind::TransientNetwork);
        assert_eq!(context.retry(), RetryAdvice::After(remaining));
        assert_eq!(
            context.diagnostic_code().map(RefreshDiagnosticCode::as_str),
            Some(code.as_str())
        );
    }
}
