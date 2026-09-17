//! Resolution error taxonomy and the fail-closed owner / tombstone gates.
//!
//! Split out of `resolver.rs` (behaviour-preserving code motion — no logic
//! change): the [`ResolveError`] enum, its mapping onto the public
//! [`CredentialError`](crate::error::CredentialError), and the structural
//! tombstone gate that the scoped resolution path uses to fail closed after a
//! concurrent revoke. Owner isolation is enforced by the owner-qualified
//! persistence selector, not metadata. Kept in the
//! `runtime` module so `resolver.rs` reaches the `pub(crate)` gate fns.

use crate::error::{
    CredentialError, ProviderErrorContext, ProviderErrorKind, RefreshNotAppliedContext,
    SecretFreeMessage,
};
use crate::resolve::ReauthReason;
use crate::state_envelope::StateEnvelopeError;
use crate::{CredentialPersistenceError, StoredCredential};

/// Map a [`ResolveError`] onto the public [`CredentialError`] returned by the
/// `scheme_factory` path, preserving the permanent-vs-transient distinction that
/// [`CredentialError`]'s [`is_retryable`](nebula_error::Classify::is_retryable)
/// contract keys on.
///
/// Replay-safe framework faults map to retryable `Provider{ServerError}`;
/// proof-bearing credential failures preserve their structured
/// `RefreshNotAppliedContext` (including retry advice). Everything permanent or
/// ambiguous — corrupt stored bytes, a state-kind mismatch, an unwired
/// external source, a not-found/already-exists row, an unknown provider/commit
/// outcome, and (critically) a rejected refresh grant that needs
/// re-authentication — maps to a **non-retryable** variant. A caller that
/// drives retries off `is_retryable` therefore cannot hammer the IdP or loop
/// forever on a failure that will never succeed.
pub(crate) fn resolve_error_to_credential_error(err: ResolveError) -> CredentialError {
    match err {
        // Preserve the credential's proof-bearing failure class and retry
        // advice end-to-end instead of flattening it into a generic provider
        // server error.
        ResolveError::RefreshNotApplied { context, .. } => {
            CredentialError::RefreshNotApplied(context)
        },
        // Local policy/configuration defect — non-retryable, actionable.
        ResolveError::RefreshContainmentViolation { .. } => CredentialError::InvalidInput,
        // Re-auth: the stored refresh grant was rejected — terminal until the
        // user reconnects. `InvalidGrant` is non-retryable, so the resolve path
        // does not re-POST a dead grant.
        ResolveError::ReauthRequired { .. } => {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::InvalidGrant,
                SecretFreeMessage::new("credential re-authentication is required"),
            )))
        },
        // Permanent data-integrity / configuration faults — no better on retry.
        // The envelope choke-point failures are here as DISTINCT variants, not
        // collapsed into Deserialize/KindMismatch/Internal: a stored shape this
        // build cannot understand is a permanent, observable refusal.
        ResolveError::Deserialize { .. }
        | ResolveError::KindMismatch { .. }
        | ResolveError::UnknownSchemaVersion { .. }
        | ResolveError::StateVersionAxesDisagree { .. }
        | ResolveError::EnvelopeKindMismatch { .. }
        | ResolveError::SchemaFingerprintMismatch { .. }
        | ResolveError::StateTooLarge { .. }
        | ResolveError::ExternalSourceNotWired => CredentialError::InvalidInput,
        // Permanent store faults for a specific row — missing or already
        // existing. Retrying will not change the outcome.
        ResolveError::Store(
            CredentialPersistenceError::NotFound
            | CredentialPersistenceError::AlreadyExists { .. }
            | CredentialPersistenceError::VersionExhausted
            | CredentialPersistenceError::MaterialEpochExhausted
            | CredentialPersistenceError::CorruptRecord,
        ) => CredentialError::InvalidInput,
        // A post-provider commit with a lost acknowledgement is operational but
        // explicitly non-retryable: replay could duplicate or conflict with a
        // mutation that already committed.
        ResolveError::PostProviderPersistence {
            source: CredentialPersistenceError::OutcomeUnknown,
            ..
        }
        | ResolveError::Store(CredentialPersistenceError::OutcomeUnknown)
        | ResolveError::RefreshOutcomePending { .. }
        | ResolveError::ProviderOutcomeUnknown { .. } => CredentialError::OutcomeUnknown,
        ResolveError::RefreshReconciliationRequired { .. }
        | ResolveError::RefreshRetryGateFinalization { .. }
        | ResolveError::ReauthDecisionFinalization { .. } => CredentialError::RefreshFinalization,
        // Once the provider accepted a refresh, even a *definite* persistence
        // failure is no longer an ordinary retryable backend outage. Repeating
        // the whole resolution path could POST the already-consumed grant a
        // second time. Surface a phase-aware, non-retryable public variant.
        ResolveError::PostProviderPersistence { .. }
        | ResolveError::PostProviderStateEncoding { .. } => {
            CredentialError::PostProviderPersistence
        },
        // Replay-safe: backend I/O/CAS before provider contact, local
        // pre-dispatch rejection, or a replay-safe runtime failure. Exact
        // provider no-effect responses are handled above as
        // `RefreshNotApplied` or `RefreshFinalization`.
        ResolveError::Store(_) | ResolveError::Refresh { .. } => {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::ServerError,
                SecretFreeMessage::new("credential runtime dependency is unavailable"),
            )))
        },
    }
}

/// Errors produced by [`CredentialResolver`](super::resolver::CredentialResolver).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// Backing credential store operation failed.
    #[error("store error: {0}")]
    Store(#[from] CredentialPersistenceError),
    /// The provider accepted a refresh, but the following persistence
    /// transition did not receive a confirmed success.
    ///
    /// This phase boundary matters for retry policy: unlike
    /// [`Self::Store`] before provider contact, replaying the whole operation
    /// can send an already-consumed or already-rotated grant to the provider.
    #[error(
        "credential {credential_id}: provider refresh succeeded but persistence failed: {source}"
    )]
    PostProviderPersistence {
        /// Credential identifier.
        credential_id: String,
        /// Closed persistence disposition observed after provider success.
        #[source]
        source: CredentialPersistenceError,
    },
    /// The provider accepted a refresh, but its updated state could not be
    /// encoded into the durable representation.
    ///
    /// The old provider grant may already be consumed, so this is a definite
    /// local failure but not a safe full-operation retry.
    #[error(
        "credential {credential_id}: provider refresh succeeded but state encoding failed: {reason}"
    )]
    PostProviderStateEncoding {
        /// Credential identifier.
        credential_id: String,
        /// Secret-free serialization diagnostic.
        reason: String,
    },
    /// Stored state kind does not match the credential state type.
    #[error("credential {credential_id}: expected kind {expected}, found {actual}")]
    KindMismatch {
        /// Credential identifier.
        credential_id: String,
        /// Expected state kind.
        expected: String,
        /// Actual state kind from storage.
        actual: String,
    },
    /// Stored state bytes failed deserialization.
    #[error("credential {credential_id}: deserialize failed: {reason}")]
    Deserialize {
        /// Credential identifier.
        credential_id: String,
        /// Deserialization error message.
        reason: String,
    },
    /// Refresh path failed.
    #[error("credential {credential_id}: refresh failed: {reason}")]
    Refresh {
        /// Credential identifier.
        credential_id: String,
        /// Refresh error message.
        reason: String,
    },
    /// A credential implementation proved that refresh failed without an
    /// accepted provider-side state transition.
    ///
    /// Unlike [`Self::Refresh`], this carries the credential's structured
    /// retry advice intact across coordinator handling and back to the public
    /// [`CredentialError`] surface.
    #[error("credential {credential_id}: refresh was not applied: {context}")]
    RefreshNotApplied {
        /// Credential identifier.
        credential_id: String,
        /// Proof-bearing typed failure context.
        context: Box<RefreshNotAppliedContext>,
    },
    /// The provider/persistence critical section crossed its irreversible
    /// boundary, but the caller stopped waiting before an exact disposition.
    ///
    /// The owned section may still be running under its durable claim. This is
    /// non-retryable: a second provider request could replay an already-consumed
    /// grant.
    #[error(
        "credential {credential_id}: provider/persistence refresh outcome is pending or unknown"
    )]
    RefreshOutcomePending {
        /// Credential identifier.
        credential_id: String,
    },
    /// A concurrent refresh reached an exact finalization failure that cannot
    /// safely be replayed without reconciling durable aggregate state.
    ///
    /// L1 completion signals are intentionally payload-free, so a waiter
    /// cannot recover the winner's more specific finalization error. It still
    /// retains the stronger proof that the outcome is exact rather than
    /// unknown.
    #[error("credential {credential_id}: refresh requires reconciliation before retrying")]
    RefreshReconciliationRequired {
        /// Credential identifier.
        credential_id: String,
    },
    /// Provider dispatch began, but no response proves whether the grant was
    /// consumed or rotated.
    ///
    /// This includes opaque custom `Refreshable::refresh` failures and OAuth
    /// transport/read/2xx-decode failures. Replaying is unsafe until the stored
    /// credential is reconciled.
    #[error("credential {credential_id}: provider refresh outcome is unknown after dispatch")]
    ProviderOutcomeUnknown {
        /// Credential identifier.
        credential_id: String,
    },
    /// An exact no-effect refresh result could not be made durable as a retry
    /// gate for this credential epoch.
    ///
    /// The refresh outcome itself is known (including exact pre-dispatch
    /// refusal), but releasing coordination would discard `Never`/`After`
    /// suppression and permit an immediate duplicate request. The coordinator
    /// therefore retains fail-closed poison until an operator or a new material
    /// epoch reconciles the aggregate.
    #[error("credential {credential_id}: refresh retry gate finalization could not be confirmed")]
    RefreshRetryGateFinalization {
        /// Credential identifier.
        credential_id: String,
    },
    /// A provider-confirmed reauthentication decision could not be made
    /// durable, so releasing coordination would allow another request to
    /// replay the known-dead grant.
    #[error(
        "credential {credential_id}: provider-confirmed reauthentication decision could not be finalized"
    )]
    ReauthDecisionFinalization {
        /// Credential identifier.
        credential_id: String,
    },
    /// Credential requires full re-authentication.
    ///
    /// Carries a typed [`ReauthReason`] so callers (UI, metrics, audit)
    /// can distinguish provider-rejected refresh from sentinel-threshold
    /// escalation per sub-spec.
    #[error("credential {credential_id}: re-authentication required")]
    ReauthRequired {
        /// Credential identifier.
        credential_id: String,
        /// Why re-authentication is required.
        reason: ReauthReason,
    },
    /// The service is configured with an external [`StateSource`](crate::StateSource)
    /// whose resolution bridge (ADR-0051) is not yet wired, so the resolver
    /// refuses to read local bytes. Fail-closed: never a silent local-store
    /// fallback. The facade maps this to
    /// `CredentialServiceError::ExternalSourceNotWired`.
    #[error("external state source is not wired; cannot resolve credential material")]
    ExternalSourceNotWired,

    /// The credential's live `policy()` returned a refresh kind that its scheme
    /// family does not permit — an F3 containment violation (policy drift from
    /// the `AuthScheme::Family` declaration). The refresh is aborted rather than
    /// proceeding with an out-of-family strategy.
    ///
    /// This indicates a hand-written or plugin `policy()` that drifted from the
    /// scheme's `AuthScheme::Family::refresh_classes()`. Fix the credential's
    /// `CredentialLifecycle::policy` implementation or its `AuthScheme::Family`
    /// declaration.
    #[error(
        "credential {credential_id}: refresh kind {refresh_kind:?} is not permitted by \
         scheme family {family_pattern:?} — F3 containment violation"
    )]
    RefreshContainmentViolation {
        /// Credential identifier.
        credential_id: String,
        /// The disallowed refresh kind the live policy returned.
        refresh_kind: String,
        /// The scheme family pattern that rejected it.
        family_pattern: String,
    },
    /// Stored state's shape version is newer than this build supports — from
    /// the envelope's `interface_version`, or the row's `state_version` on the
    /// legacy decode path. Fail closed: the row is left untouched.
    #[error(
        "credential {credential_id}: stored state version {stored_version} is newer than the \
         version this build supports ({supported_version})"
    )]
    UnknownSchemaVersion {
        /// Credential identifier.
        credential_id: String,
        /// Version recorded by the producer.
        stored_version: u32,
        /// This build's supported maximum for the state type.
        supported_version: u32,
    },
    /// The envelope's `interface_version` disagrees with the row's
    /// `state_version` axis — the two must agree (the envelope carries the
    /// same value the row column records).
    #[error(
        "credential {credential_id}: state envelope version {envelope_version} disagrees with \
         the stored row's state_version {row_version}"
    )]
    StateVersionAxesDisagree {
        /// Credential identifier.
        credential_id: String,
        /// The envelope's `interface_version`.
        envelope_version: u32,
        /// The row column's `state_version`.
        row_version: u32,
    },
    /// The envelope's `kind_tag` is not the kind this reader was invoked for.
    /// The found value is deliberately not echoed — only the expected kind.
    #[error(
        "credential {credential_id}: stored state kind does not match the expected kind \
         {expected}"
    )]
    EnvelopeKindMismatch {
        /// Credential identifier.
        credential_id: String,
        /// Expected state kind (`CredentialState::KIND`).
        expected: &'static str,
    },
    /// The envelope's schema fingerprint is not this build's fingerprint for
    /// the state type — the stored wire shape is not the shape this build
    /// understands. Fail closed before deserialization.
    #[error(
        "credential {credential_id}: stored state schema fingerprint {stored:#018x} does not \
         match the fingerprint this build expects ({expected:#018x})"
    )]
    SchemaFingerprintMismatch {
        /// Credential identifier.
        credential_id: String,
        /// This build's `SCHEMA_FINGERPRINT`.
        expected: u64,
        /// The fingerprint read from the envelope.
        stored: u64,
    },
    /// The stored state's decrypted plaintext exceeds the reader's bound
    /// (state_envelope::MAX_STATE_PLAINTEXT_BYTES), refused before any parse.
    /// Fail closed: the row is left untouched.
    #[error(
        "credential {credential_id}: stored state plaintext is {bytes} bytes, above the \
         supported bound of {limit} bytes"
    )]
    StateTooLarge {
        /// Credential identifier.
        credential_id: String,
        /// The plaintext length observed.
        bytes: usize,
        /// The reader's bound.
        limit: usize,
    },
}

/// Map a state-envelope choke-point failure onto the resolve error taxonomy.
///
/// [`StateEnvelopeError::LegacyStateParseFailed`] maps to
/// [`ResolveError::Deserialize`] — a corrupt non-envelope row keeps the
/// pre-envelope classification. The envelope checks map to distinct variants
/// so the refusal is observable as a shape/version/fingerprint/size mismatch,
/// never collapsed into `Deserialize` or `KindMismatch`.
pub(crate) fn envelope_error_to_resolve_error(
    credential_id: String,
    error: StateEnvelopeError,
) -> ResolveError {
    match error {
        StateEnvelopeError::UnknownSchemaVersion {
            stored_version,
            supported_version,
        } => ResolveError::UnknownSchemaVersion {
            credential_id,
            stored_version,
            supported_version,
        },
        StateEnvelopeError::VersionAxesDisagree {
            envelope_version,
            row_version,
        } => ResolveError::StateVersionAxesDisagree {
            credential_id,
            envelope_version,
            row_version,
        },
        StateEnvelopeError::KindMismatch { expected } => ResolveError::EnvelopeKindMismatch {
            credential_id,
            expected,
        },
        StateEnvelopeError::SchemaFingerprintMismatch { expected, stored } => {
            ResolveError::SchemaFingerprintMismatch {
                credential_id,
                expected,
                stored,
            }
        },
        StateEnvelopeError::StateTooLarge { bytes, limit } => ResolveError::StateTooLarge {
            credential_id,
            bytes,
            limit,
        },
        StateEnvelopeError::LegacyStateParseFailed => ResolveError::Deserialize {
            credential_id,
            reason: "stored state is neither a valid state envelope nor legacy state JSON"
                .to_owned(),
        },
    }
}

/// Fail-closed tombstone gate for the scoped resolution path.
///
/// Defence in depth for the resolve-during-revoke race:
/// `CredentialService::validate_credential_binding` already rejects a tombstoned
/// id when the binding is minted, but a binding validated immediately before a
/// concurrent `revoke` could still reach `resolve_scoped`. A revoked row is
/// mapped to [`CredentialPersistenceError::NotFound`] (same existence-hiding shape as
/// the ordinary live lookup) so a revoked secret is never projected to a guard.
///
/// Complexity: O(1).
pub(crate) fn reject_tombstoned(stored: &StoredCredential) -> Result<(), ResolveError> {
    if matches!(stored, StoredCredential::Tombstoned(_)) {
        return Err(ResolveError::Store(CredentialPersistenceError::NotFound));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CredentialId, StoredLiveCredential};
    use nebula_storage_port::{
        CredentialMaterialEpoch, CredentialVersion, StoredTombstonedCredential,
    };

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
}
