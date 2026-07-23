//! Resolution error taxonomy and the fail-closed owner / tombstone gates.
//!
//! Split out of `resolver.rs` (behaviour-preserving code motion — no logic
//! change): the [`ResolveError`] enum, its mapping onto the public
//! [`CredentialError`](crate::error::CredentialError), and the structural
//! tombstone gate that the scoped resolution path uses to fail closed after a
//! concurrent revoke. Owner isolation is enforced by the owner-qualified
//! persistence selector, not metadata. Kept in the
//! `runtime` module so `resolver.rs` reaches the `pub(crate)` gate fns.

use crate::error::{CredentialError, ProviderErrorContext, ProviderErrorKind, SecretFreeMessage};
use crate::resolve::ReauthReason;
use crate::{CredentialPersistenceError, StoredCredential};

/// Map a [`ResolveError`] onto the public [`CredentialError`] returned by the
/// `scheme_factory` path, preserving the permanent-vs-transient distinction that
/// [`CredentialError`]'s [`is_retryable`](nebula_error::Classify::is_retryable)
/// contract keys on.
///
/// Retryable (`Provider{ServerError}`) is reserved for replay-safe transient
/// faults — a backend I/O blip or CAS conflict before provider contact, a local
/// pre-dispatch rejection, or a complete provider response known not to have
/// consumed the grant. Everything permanent or ambiguous — corrupt stored
/// bytes, a state-kind mismatch, an unwired external source, a
/// not-found/already-exists row, an unknown provider/commit outcome, and
/// (critically) a rejected refresh grant that needs re-authentication — maps to
/// a **non-retryable** variant. A caller that drives retries off
/// `is_retryable` therefore cannot hammer the IdP or loop forever on a failure
/// that will never succeed. (Previously every non-containment error was
/// blanket-mapped to retryable `ServerError`.)
pub(crate) fn resolve_error_to_credential_error(err: ResolveError) -> CredentialError {
    match &err {
        // Local policy/configuration defect — non-retryable, actionable.
        ResolveError::RefreshContainmentViolation {
            credential_id,
            refresh_kind,
            family_pattern,
        } => CredentialError::InvalidInput(format!(
            "credential {credential_id}: F3 containment violation — \
             refresh kind {refresh_kind:?} is not permitted by scheme family {family_pattern:?}; \
             fix the credential's policy() implementation or its AuthScheme::Family declaration"
        )),
        // Re-auth: the stored refresh grant was rejected — terminal until the
        // user reconnects. `InvalidGrant` is non-retryable, so the resolve path
        // does not re-POST a dead grant.
        ResolveError::ReauthRequired {
            credential_id,
            reason,
        } => CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::InvalidGrant,
            SecretFreeMessage::new(format!(
                "credential {credential_id}: re-authentication required ({})",
                reason.code()
            )),
        ))),
        // Permanent data-integrity / configuration faults — no better on retry.
        ResolveError::Deserialize { .. }
        | ResolveError::KindMismatch { .. }
        | ResolveError::ExternalSourceNotWired => CredentialError::InvalidInput(err.to_string()),
        // Permanent store faults for a specific row — missing or already
        // existing. Retrying will not change the outcome.
        ResolveError::Store(
            CredentialPersistenceError::NotFound
            | CredentialPersistenceError::AlreadyExists { .. }
            | CredentialPersistenceError::VersionExhausted
            | CredentialPersistenceError::CorruptRecord,
        ) => CredentialError::InvalidInput(err.to_string()),
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
        // Once the provider accepted a refresh, even a *definite* persistence
        // failure is no longer an ordinary retryable backend outage. Repeating
        // the whole resolution path could POST the already-consumed grant a
        // second time. Surface a phase-aware, non-retryable public variant.
        ResolveError::PostProviderPersistence { .. }
        | ResolveError::PostProviderStateEncoding { .. } => {
            CredentialError::PostProviderPersistence
        },
        // Replay-safe: backend I/O/CAS before provider contact, local
        // pre-dispatch rejection, or an exact provider response that did not
        // accept the grant — retryable `ServerError`.
        ResolveError::Store(_) | ResolveError::Refresh { .. } => {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::ServerError,
                SecretFreeMessage::new(err.to_string()),
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
    use nebula_storage_port::{CredentialVersion, StoredTombstonedCredential};

    fn live() -> StoredCredential {
        StoredLiveCredential::new(
            CredentialId::new(),
            None,
            "github_oauth".to_owned(),
            Vec::new().into(),
            "oauth2_state".to_owned(),
            1,
            CredentialVersion::MIN,
            chrono::Utc::now(),
            chrono::Utc::now(),
            None,
            false,
            serde_json::Map::new(),
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
            matches!(mapped, CredentialError::InvalidInput(_)),
            "expected InvalidInput (non-retriable, Validation category), got {mapped:?}"
        );
        // Confirm it is NOT mapped to a retriable provider error.
        assert!(
            !matches!(mapped, CredentialError::Provider(_)),
            "F3 violation must not be mapped to a provider error (would trigger retries)"
        );
    }
}
