//! Resolution error taxonomy and the fail-closed owner / tombstone gates.
//!
//! Split out of `resolver.rs` (behaviour-preserving code motion — no logic
//! change): the [`ResolveError`] enum, its mapping onto the public
//! [`CredentialError`](crate::error::CredentialError), and the two O(1) gate
//! helpers (`verify_owner` / `reject_tombstoned`) that the scoped resolution
//! path uses to fail closed on cross-tenant or revoked rows. Kept in the
//! `runtime` module so `resolver.rs` reaches the `pub(crate)` gate fns.

use crate::error::{CredentialError, ProviderErrorContext, ProviderErrorKind, SecretFreeMessage};
use crate::resolve::ReauthReason;
use crate::store::{OWNER_ID_METADATA_KEY, OwnerScopedKey, StoreError, StoredCredential};

/// Map a [`ResolveError`] onto the public [`CredentialError`] returned by the
/// `scheme_factory` path, preserving the permanent-vs-transient distinction that
/// [`CredentialError`]'s [`is_retryable`](nebula_error::Classify::is_retryable)
/// contract keys on.
///
/// Retryable (`Provider{ServerError}`) is reserved for genuinely transient
/// faults — a backend I/O blip, a CAS version conflict, or a failed provider
/// refresh call. Everything permanent — corrupt stored bytes, a state-kind
/// mismatch, an unwired external source, a not-found/already-exists row, a
/// fail-closed audit alarm, and (critically) a rejected refresh grant that needs
/// re-authentication — maps to a **non-retryable** variant, so a caller that
/// drives retries off `is_retryable` cannot hammer the IdP or loop forever on a
/// failure that will never succeed. (Previously every non-containment error was
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
                "credential {credential_id}: re-authentication required ({reason:?})"
            )),
        ))),
        // Permanent data-integrity / configuration faults — no better on retry.
        ResolveError::Deserialize { .. }
        | ResolveError::KindMismatch { .. }
        | ResolveError::ExternalSourceNotWired => CredentialError::InvalidInput(err.to_string()),
        // Permanent store faults for a specific row — missing or already
        // existing. Retrying will not change the outcome.
        ResolveError::Store(StoreError::NotFound { .. } | StoreError::AlreadyExists { .. }) => {
            CredentialError::InvalidInput(err.to_string())
        },
        // A fail-closed audit-sink alarm is an operational fault, NOT user
        // input: the audit trail is compromised and an operator must
        // investigate (the store contract says retry only once the sink is
        // healthy). Keep it non-retryable (`Other`) but off the validation
        // path so it never reads as a client 4xx that hides a compromised
        // audit trail.
        ResolveError::Store(StoreError::AuditFailure(_)) => {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::Other,
                SecretFreeMessage::new(err.to_string()),
            )))
        },
        // Genuinely transient: backend I/O, a CAS version conflict, or a
        // provider refresh call that failed — retryable `ServerError`.
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
    Store(#[from] StoreError),
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
    /// The service is configured with an external [`StateSource`](crate::service)
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

/// Fail-closed owner gate for the scoped resolution path: the loaded row's
/// stamped `owner_id` must equal the key's owner. A mismatch maps to
/// [`StoreError::NotFound`] (existence-hiding, matching the management facade) so
/// a cross-tenant probe cannot tell "absent" from "owned by another tenant". An
/// unstamped row (no `owner_id` metadata) is treated as foreign and rejected.
///
/// Complexity: O(1).
pub(crate) fn verify_owner(
    key: &OwnerScopedKey,
    stored: &StoredCredential,
) -> Result<(), ResolveError> {
    let stored_owner = stored
        .metadata
        .get(OWNER_ID_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if stored_owner != key.owner_id() {
        return Err(ResolveError::Store(StoreError::NotFound {
            id: key.credential_id().to_owned(),
        }));
    }
    Ok(())
}

/// Fail-closed tombstone gate for the scoped resolution path.
///
/// Defence in depth for the resolve-during-revoke race:
/// `CredentialService::validate_credential_binding` already rejects a tombstoned
/// id when the binding is minted, but a binding validated immediately before a
/// concurrent `revoke` could still reach `resolve_scoped`. A revoked row is
/// mapped to [`StoreError::NotFound`] (same existence-hiding shape as
/// [`verify_owner`]) so a revoked secret is never projected to a guard.
///
/// Complexity: O(1).
pub(crate) fn reject_tombstoned(
    credential_id: &str,
    stored: &StoredCredential,
) -> Result<(), ResolveError> {
    if stored.is_tombstoned() {
        return Err(ResolveError::Store(StoreError::NotFound {
            id: credential_id.to_owned(),
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_with_owner(owner: Option<&str>) -> StoredCredential {
        let mut metadata = serde_json::Map::new();
        if let Some(o) = owner {
            metadata.insert(
                OWNER_ID_METADATA_KEY.to_owned(),
                serde_json::Value::String(o.to_owned()),
            );
        }
        StoredCredential {
            id: "cred_x".to_owned(),
            name: None,
            credential_key: "github_oauth".to_owned(),
            data: Vec::new(),
            state_kind: "oauth2_state".to_owned(),
            state_version: 1,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata,
        }
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
                reason: ReauthReason::ProviderRejected {
                    detail: "invalid_grant".to_owned(),
                },
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
            ResolveError::Store(StoreError::NotFound {
                id: "cred_x".to_owned(),
            }),
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
        // A backend I/O blip and a failed provider refresh call are genuinely
        // transient — the retry layer may legitimately re-attempt them.
        let backend = resolve_error_to_credential_error(ResolveError::Store(StoreError::Backend(
            "db timeout".into(),
        )));
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
    fn audit_failure_is_operational_not_client_validation() {
        use nebula_error::Classify;
        // A fail-closed audit-sink alarm must NOT be mislabelled as user input
        // (which would surface as a client 4xx and hide a compromised audit
        // trail). It stays non-retryable but is classified operational, not
        // validation.
        let mapped = resolve_error_to_credential_error(ResolveError::Store(
            StoreError::AuditFailure("audit sink refused".to_owned()),
        ));
        assert!(
            !matches!(mapped, CredentialError::InvalidInput(_)),
            "audit-sink failure must not be classified as client validation input"
        );
        assert!(
            !mapped.is_retryable(),
            "audit-sink failure must be non-retryable (retry only once the sink is healthy)"
        );
    }

    #[test]
    fn verify_owner_accepts_matching_owner() {
        let key = OwnerScopedKey::new("alice".to_owned(), "cred_x".to_owned());
        assert!(verify_owner(&key, &stored_with_owner(Some("alice"))).is_ok());
    }

    #[test]
    fn cross_tenant_load_is_not_found() {
        // Confused-deputy regression: a key for owner "bob" must not read a row
        // stamped "alice"; the load fails closed as NotFound (existence-hiding),
        // so a foreign tenant cannot even distinguish existence.
        let key = OwnerScopedKey::new("bob".to_owned(), "cred_x".to_owned());
        let err = verify_owner(&key, &stored_with_owner(Some("alice"))).unwrap_err();
        assert!(matches!(
            err,
            ResolveError::Store(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn unstamped_row_is_treated_as_foreign() {
        let key = OwnerScopedKey::new("alice".to_owned(), "cred_x".to_owned());
        let err = verify_owner(&key, &stored_with_owner(None)).unwrap_err();
        assert!(matches!(
            err,
            ResolveError::Store(StoreError::NotFound { .. })
        ));
    }

    fn tombstoned(owner: Option<&str>) -> StoredCredential {
        let mut stored = stored_with_owner(owner);
        stored.metadata.insert(
            crate::store::REVOKED_AT_METADATA_KEY.to_owned(),
            serde_json::Value::String("2026-06-13T10:00:00Z".to_owned()),
        );
        stored
    }

    #[test]
    fn tombstoned_row_is_rejected_as_not_found() {
        // Resolve-during-revoke race: a row revoked after its binding was
        // validated must not project a guard — it fails closed as NotFound,
        // never exposing the revoked secret.
        let err = reject_tombstoned("cred_x", &tombstoned(Some("alice"))).unwrap_err();
        assert!(matches!(
            err,
            ResolveError::Store(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn live_row_passes_tombstone_check() {
        assert!(reject_tombstoned("cred_x", &stored_with_owner(Some("alice"))).is_ok());
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
