use super::*;
use crate::error::{
    RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext, RefreshNotAppliedPhase,
    RetryAdvice,
};

#[test]
fn slot_mapping_preserves_unknown_and_definite_post_provider_failures() {
    let source = StateSource::LocalEncrypted;
    let unknown = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::ProviderOutcomeUnknown {
            credential_id: "cred-test".to_owned(),
        },
    );
    assert!(matches!(unknown, CredentialServiceError::OutcomeUnknown));

    let lost_commit = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::PostProviderPersistence {
            credential_id: "cred-test".to_owned(),
            source: CredentialPersistenceError::OutcomeUnknown,
        },
    );
    assert!(matches!(
        lost_commit,
        CredentialServiceError::OutcomeUnknown
    ));

    let definite = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::PostProviderStateEncoding {
            credential_id: "cred-test".to_owned(),
            reason: "closed test failure".to_owned(),
        },
    );
    assert!(matches!(
        definite,
        CredentialServiceError::RefreshPostProviderPersistence
    ));

    let retry_gate = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::RefreshRetryGateFinalization {
            credential_id: "cred-test".to_owned(),
        },
    );
    assert!(matches!(
        retry_gate,
        CredentialServiceError::RefreshRetryGateFinalization
    ));

    let reconciliation = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::RefreshReconciliationRequired {
            credential_id: "cred-test".to_owned(),
        },
    );
    assert!(matches!(
        reconciliation,
        CredentialServiceError::RefreshReconciliationRequired
    ));

    let reauth = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::ReauthDecisionFinalization {
            credential_id: "cred-test".to_owned(),
        },
    );
    assert!(matches!(
        reauth,
        CredentialServiceError::ReauthDecisionFinalization
    ));
}

#[test]
fn slot_mapping_preserves_proof_bearing_refresh_failure() {
    let source = StateSource::LocalEncrypted;
    let context = RefreshNotAppliedContext::from_spec(
        RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
        RefreshFailureSpec::new(RefreshErrorKind::ProtocolError, RetryAdvice::Never),
    );

    let mapped = map_slot_resolve_error(
        &source,
        "cred-test",
        ResolveError::RefreshNotApplied {
            credential_id: "cred-test".to_owned(),
            context: Box::new(context),
        },
    );

    let CredentialServiceError::RefreshNotApplied(context) = mapped else {
        panic!("slot resolution must preserve the typed exact failure");
    };
    assert_eq!(
        context.phase(),
        RefreshNotAppliedPhase::ProviderConfirmedNotApplied
    );
    assert_eq!(context.retry(), RetryAdvice::Never);
}
