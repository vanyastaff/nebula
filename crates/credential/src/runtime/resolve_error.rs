//! Resolution error taxonomy and the fail-closed owner / tombstone gates.
//!
//! Split out of `resolver/mod.rs` (behaviour-preserving code motion — no logic
//! change): the [`ResolveError`] enum, its mapping onto the public
//! [`CredentialError`](crate::error::CredentialError), and the structural
//! tombstone gate that the scoped resolution path uses to fail closed after a
//! concurrent revoke. Owner isolation is enforced by the owner-qualified
//! persistence selector, not metadata. Kept in the
//! `runtime` module so `resolver/mod.rs` reaches the `pub(crate)` gate fns.

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
        ResolveError::OperationBlocked { .. }
        | ResolveError::Store(CredentialPersistenceError::OperationBlocked { .. }) => {
            CredentialError::OperationBlocked
        },
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
            | CredentialPersistenceError::AdmissionEpochExhausted
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
    /// A durable operation prevents issuing any new projection of this
    /// credential, including still-unexpired or non-refreshable material.
    #[error("credential operation must finish or be reconciled before use")]
    OperationBlocked {
        /// Secret-free kind of the operation retaining authority.
        operation: nebula_storage_port::store::CredentialOperationKind,
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
#[path = "resolve_error_tests.rs"]
mod tests;
