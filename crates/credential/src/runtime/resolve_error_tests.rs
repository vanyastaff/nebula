use super::*;
use crate::{CredentialId, StoredLiveCredential};
use nebula_storage_port::{CredentialMaterialEpoch, CredentialVersion, StoredTombstonedCredential};

fn live() -> StoredCredential {
    StoredLiveCredential::new(
        CredentialId::new(),
        None,
        "github_oauth".to_owned(),
        Vec::new().into(),
        "oauth2_state".to_owned(),
        1,
        CredentialVersion::MIN,
        CredentialMaterialEpoch::MIN,
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        false,
        serde_json::Map::new(),
        None,
    )
    .expect("fixture is a valid live record")
    .into()
}

fn tombstoned() -> StoredCredential {
    let now = chrono::Utc::now();
    StoredTombstonedCredential::new(
        CredentialId::new(),
        "github_oauth".to_owned(),
        "oauth2_state".to_owned(),
        1,
        CredentialVersion::MIN,
        now,
        now,
        now,
    )
    .into()
}

#[test]
fn permanent_resolve_errors_are_not_retryable() {
    use nebula_error::Classify;
    // Re-auth (rejected grant), corrupt stored bytes, a state-kind mismatch,
    // an unwired external source, and a not-found row are all terminal —
    // retrying only hammers the provider or loops forever. Each must
    // classify non-retryable so a retry-driven caller stops.
    let permanent = [
        ResolveError::OperationBlocked {
            operation: nebula_storage_port::store::CredentialOperationKind::Revoke,
        },
        ResolveError::Store(CredentialPersistenceError::OperationBlocked {
            operation: nebula_storage_port::store::CredentialOperationKind::Revoke,
        }),
        ResolveError::ReauthRequired {
            credential_id: "cred_x".to_owned(),
            reason: ReauthReason::ProviderRejected,
        },
        ResolveError::Deserialize {
            credential_id: "cred_x".to_owned(),
            reason: "bad bytes".to_owned(),
        },
        ResolveError::KindMismatch {
            credential_id: "cred_x".to_owned(),
            expected: "a".to_owned(),
            actual: "b".to_owned(),
        },
        // The four envelope choke-point refusals are permanent — the
        // stored shape will never match this build on retry.
        ResolveError::UnknownSchemaVersion {
            credential_id: "cred_x".to_owned(),
            stored_version: 9,
            supported_version: 1,
        },
        ResolveError::StateVersionAxesDisagree {
            credential_id: "cred_x".to_owned(),
            envelope_version: 2,
            row_version: 1,
        },
        ResolveError::EnvelopeKindMismatch {
            credential_id: "cred_x".to_owned(),
            expected: "oauth2",
        },
        ResolveError::SchemaFingerprintMismatch {
            credential_id: "cred_x".to_owned(),
            expected: 0x1234_5678_9abc_def0,
            stored: 0xdead_beef_dead_beef,
        },
        ResolveError::ExternalSourceNotWired,
        ResolveError::Store(CredentialPersistenceError::NotFound),
        ResolveError::PostProviderPersistence {
            credential_id: "cred_x".to_owned(),
            source: CredentialPersistenceError::VersionConflict {
                expected: CredentialVersion::MIN,
                actual: CredentialVersion::MIN
                    .next_live()
                    .expect("fixture has version headroom"),
            },
        },
        ResolveError::PostProviderPersistence {
            credential_id: "cred_x".to_owned(),
            source: CredentialPersistenceError::Unavailable,
        },
        ResolveError::PostProviderStateEncoding {
            credential_id: "cred_x".to_owned(),
            reason: "state serializer rejected the value".to_owned(),
        },
        ResolveError::RefreshOutcomePending {
            credential_id: "cred_x".to_owned(),
        },
        ResolveError::ProviderOutcomeUnknown {
            credential_id: "cred_x".to_owned(),
        },
    ];
    for err in permanent {
        let mapped = resolve_error_to_credential_error(err);
        assert!(
            !mapped.is_retryable(),
            "permanent resolve error must map to a non-retryable CredentialError, \
             got retryable: {mapped}"
        );
    }
}

#[test]
fn transient_resolve_errors_are_retryable() {
    use nebula_error::Classify;
    // A definite backend outage and a failed provider refresh call are genuinely
    // transient — the retry layer may legitimately re-attempt them.
    let backend = resolve_error_to_credential_error(ResolveError::Store(
        CredentialPersistenceError::Unavailable,
    ));
    assert!(backend.is_retryable(), "backend blip should be retryable");
    let refresh = resolve_error_to_credential_error(ResolveError::Refresh {
        credential_id: "cred_x".to_owned(),
        reason: "502 from IdP".to_owned(),
    });
    assert!(
        refresh.is_retryable(),
        "provider refresh call should be retryable"
    );
}

#[test]
fn not_applied_refresh_preserves_typed_retry_advice() {
    use crate::error::{
        RefreshDiagnosticCode, RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext,
        RefreshNotAppliedPhase, RetryAdvice, RetryDelay,
    };

    let backoff = std::time::Duration::from_secs(7);
    let delay = RetryDelay::new(backoff).expect("non-zero test backoff");
    let code =
        RefreshDiagnosticCode::parse("server_error").expect("fixed diagnostic code is valid");
    let mapped = resolve_error_to_credential_error(ResolveError::RefreshNotApplied {
        credential_id: "cred_x".to_owned(),
        context: Box::new(RefreshNotAppliedContext::from_spec(
            RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
            RefreshFailureSpec::new(
                RefreshErrorKind::ProviderUnavailable,
                RetryAdvice::After(delay),
            )
            .with_diagnostic_code(code),
        )),
    });

    let CredentialError::RefreshNotApplied(context) = mapped else {
        panic!("not-applied refresh must retain its public typed context");
    };
    assert_eq!(context.kind(), RefreshErrorKind::ProviderUnavailable);
    assert_eq!(context.retry(), RetryAdvice::After(delay));
    assert_eq!(
        context.diagnostic_code().map(RefreshDiagnosticCode::as_str),
        Some("server_error")
    );
}

#[test]
fn unknown_commit_outcome_is_distinct_and_not_retryable() {
    use nebula_error::{Classify, ErrorCategory, ErrorCode};
    let mapped = resolve_error_to_credential_error(ResolveError::Store(
        CredentialPersistenceError::OutcomeUnknown,
    ));
    assert!(matches!(mapped, CredentialError::OutcomeUnknown));
    assert_eq!(mapped.category(), ErrorCategory::Internal);
    assert_eq!(mapped.code(), ErrorCode::new("CREDENTIAL:OUTCOME_UNKNOWN"));
    assert!(
        !mapped.is_retryable(),
        "unknown outcome must not be replayed blindly"
    );
}

#[test]
fn provider_boundary_timeout_is_unknown_and_not_retryable() {
    use nebula_error::{Classify, ErrorCategory, ErrorCode};

    let mapped = resolve_error_to_credential_error(ResolveError::RefreshOutcomePending {
        credential_id: "cred_x".to_owned(),
    });
    assert!(matches!(mapped, CredentialError::OutcomeUnknown));
    assert_eq!(mapped.category(), ErrorCategory::Internal);
    assert_eq!(mapped.code(), ErrorCode::new("CREDENTIAL:OUTCOME_UNKNOWN"));
    assert!(
        !mapped.is_retryable(),
        "a caller timeout after provider dispatch must not replay the grant"
    );
}

#[test]
fn exact_refresh_finalization_failures_do_not_collapse_into_unknown_outcome() {
    use nebula_error::{Classify, ErrorCategory, ErrorCode};

    let cases = [
        ResolveError::RefreshReconciliationRequired {
            credential_id: "cred_x".to_owned(),
        },
        ResolveError::RefreshRetryGateFinalization {
            credential_id: "cred_x".to_owned(),
        },
        ResolveError::ReauthDecisionFinalization {
            credential_id: "cred_x".to_owned(),
        },
    ];
    for error in cases {
        let mapped = resolve_error_to_credential_error(error);
        assert!(matches!(mapped, CredentialError::RefreshFinalization));
        assert_eq!(mapped.category(), ErrorCategory::Internal);
        assert_eq!(
            mapped.code(),
            ErrorCode::new("CREDENTIAL:REFRESH_FINALIZATION")
        );
        assert!(!mapped.is_retryable());
        assert!(mapped.retry_hint().is_none());
    }
}

#[test]
fn post_provider_persistence_is_distinct_from_retryable_pre_provider_store_failure() {
    use nebula_error::{Classify, ErrorCategory, ErrorCode};

    for source in [
        CredentialPersistenceError::VersionConflict {
            expected: CredentialVersion::MIN,
            actual: CredentialVersion::MIN
                .next_live()
                .expect("fixture has version headroom"),
        },
        CredentialPersistenceError::Unavailable,
    ] {
        let mapped = resolve_error_to_credential_error(ResolveError::PostProviderPersistence {
            credential_id: "cred_x".to_owned(),
            source,
        });
        assert!(matches!(mapped, CredentialError::PostProviderPersistence));
        assert_eq!(mapped.category(), ErrorCategory::Internal);
        assert_eq!(
            mapped.code(),
            ErrorCode::new("CREDENTIAL:POST_PROVIDER_PERSISTENCE")
        );
        assert!(
            !mapped.is_retryable(),
            "provider-success persistence failures must never replay the provider call"
        );
    }

    let pre_provider = resolve_error_to_credential_error(ResolveError::Store(
        CredentialPersistenceError::Unavailable,
    ));
    assert!(
        pre_provider.is_retryable(),
        "the same definite outage remains retryable before provider contact"
    );
}

#[test]
fn tombstoned_row_is_rejected_as_not_found() {
    // Resolve-during-revoke race: a row revoked after its binding was
    // validated must not project a guard — it fails closed as NotFound,
    // never exposing the revoked secret.
    let err = reject_tombstoned(&tombstoned()).unwrap_err();
    assert!(matches!(
        err,
        ResolveError::Store(CredentialPersistenceError::NotFound)
    ));
}

#[test]
fn live_row_passes_tombstone_check() {
    assert!(reject_tombstoned(&live()).is_ok());
}

/// F3 containment violation must map to `CredentialError::InvalidInput`
/// (Validation, non-retriable), NOT to `Provider(ServerError)` (External,
/// retriable). Misclassifying a configuration defect as a retriable provider
/// error would cause infinite retry loops and misleading observability.
#[test]
fn containment_violation_maps_to_invalid_input_not_provider_error() {
    let resolve_err = ResolveError::RefreshContainmentViolation {
        credential_id: "cred_abc".to_owned(),
        refresh_kind: "RefreshToken".to_owned(),
        family_pattern: "SecretToken".to_owned(),
    };
    let mapped = resolve_error_to_credential_error(resolve_err);
    assert!(
        matches!(mapped, CredentialError::InvalidInput),
        "expected InvalidInput (non-retriable, Validation category), got {mapped:?}"
    );
    // Confirm it is NOT mapped to a retriable provider error.
    assert!(
        !matches!(mapped, CredentialError::Provider(_)),
        "F3 violation must not be mapped to a provider error (would trigger retries)"
    );
}

#[test]
fn envelope_check_failures_map_to_distinct_resolve_errors() {
    // Each envelope choke-point check keeps its own ResolveError variant —
    // none collapses into Deserialize/KindMismatch.
    let mapped = envelope_error_to_resolve_error(
        "cred_x".to_owned(),
        StateEnvelopeError::UnknownSchemaVersion {
            stored_version: 9,
            supported_version: 1,
        },
    );
    assert!(
        matches!(
            &mapped,
            ResolveError::UnknownSchemaVersion {
                credential_id,
                stored_version: 9,
                supported_version: 1,
            } if credential_id == "cred_x"
        ),
        "unknown version must keep its variant, got {mapped:?}"
    );

    let mapped = envelope_error_to_resolve_error(
        "cred_x".to_owned(),
        StateEnvelopeError::VersionAxesDisagree {
            envelope_version: 2,
            row_version: 1,
        },
    );
    assert!(
        matches!(
            &mapped,
            ResolveError::StateVersionAxesDisagree {
                credential_id,
                envelope_version: 2,
                row_version: 1,
            } if credential_id == "cred_x"
        ),
        "axis disagreement must keep its variant, got {mapped:?}"
    );

    let mapped = envelope_error_to_resolve_error(
        "cred_x".to_owned(),
        StateEnvelopeError::KindMismatch { expected: "oauth2" },
    );
    assert!(
        matches!(
            &mapped,
            ResolveError::EnvelopeKindMismatch {
                credential_id,
                expected: "oauth2",
            } if credential_id == "cred_x"
        ),
        "envelope kind mismatch must not collapse into row KindMismatch, got {mapped:?}"
    );

    let mapped = envelope_error_to_resolve_error(
        "cred_x".to_owned(),
        StateEnvelopeError::SchemaFingerprintMismatch {
            expected: 0xaaaa,
            stored: 0xbbbb,
        },
    );
    assert!(
        matches!(
            &mapped,
            ResolveError::SchemaFingerprintMismatch {
                credential_id,
                expected: 0xaaaa,
                stored: 0xbbbb,
            } if credential_id == "cred_x"
        ),
        "fingerprint mismatch must keep its variant, got {mapped:?}"
    );

    let mapped = envelope_error_to_resolve_error(
        "cred_x".to_owned(),
        StateEnvelopeError::StateTooLarge {
            bytes: 1_048_577,
            limit: 1_048_576,
        },
    );
    assert!(
        matches!(
            &mapped,
            ResolveError::StateTooLarge {
                credential_id,
                bytes: 1_048_577,
                limit: 1_048_576,
            } if credential_id == "cred_x"
        ),
        "an oversized plaintext must keep its variant, got {mapped:?}"
    );
    let mapped = resolve_error_to_credential_error(mapped);
    assert!(
        matches!(mapped, CredentialError::InvalidInput),
        "an oversized stored plaintext is a permanent, non-retryable refusal, got {mapped:?}"
    );
}

#[test]
fn corrupt_legacy_payload_keeps_deserialize_classification() {
    // A non-envelope payload that also fails legacy JSON parse keeps the
    // pre-envelope classification — envelope checks are not implicated.
    let mapped = envelope_error_to_resolve_error(
        "cred_x".to_owned(),
        StateEnvelopeError::LegacyStateParseFailed,
    );
    assert!(
        matches!(
            &mapped,
            ResolveError::Deserialize {
                credential_id,
                ..
            } if credential_id == "cred_x"
        ),
        "corrupt legacy bytes must map to Deserialize, got {mapped:?}"
    );
}
