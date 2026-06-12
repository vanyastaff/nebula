//! Error taxonomy for the credential management runtime.
//!
//! `#[non_exhaustive]` so later increments add variants without breaking
//! downstream `match` exhaustiveness. Classified via
//! [`nebula_error::Classify`] using only the codebase-standard categories
//! `internal` / `validation` / `external` (mirrors
//! `crates/credential/src/error.rs`).

use thiserror::Error;

/// Failure modes of the credential management facade. The API layer maps
/// each `category` to an HTTP status; `code` is the stable machine label.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum CredentialServiceError {
    /// No credential with this id in the caller's tenant scope.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:NOT_FOUND")]
    #[error("credential not found: {id}")]
    NotFound {
        /// The credential id that was not found.
        id: String,
    },

    /// Optimistic-concurrency check failed on update.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:VERSION_CONFLICT")]
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// Credential id under contention.
        id: String,
        /// Version the caller expected (CAS precondition).
        expected: u64,
        /// Version actually stored.
        actual: u64,
    },

    /// Property payload failed the credential type's schema validation.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:VALIDATION_FAILED")]
    #[error("credential property validation failed: {reason}")]
    ValidationFailed {
        /// Human-readable validation failure (never echoes secret values).
        reason: String,
    },

    /// The requested lifecycle op needs a capability the type lacks.
    #[classify(
        category = "validation",
        code = "CREDENTIAL_SERVICE:CAPABILITY_UNSUPPORTED"
    )]
    #[error("credential type '{key}' does not support capability '{capability}'")]
    CapabilityUnsupported {
        /// Capability name (`refresh` / `revoke` / `test`).
        capability: String,
        /// `Credential::KEY` of the target type.
        key: String,
    },

    /// A credential type advertises a capability in the registry but the
    /// matching operation closure was never registered in `DispatchOps`
    /// (a `register_*_ops` call was skipped at the composition root). Caught
    /// at the api-layer credential builder's `build()` so a misconfigured
    /// service fails loud at startup instead of returning
    /// [`CapabilityUnsupported`](Self::CapabilityUnsupported) at first use.
    #[classify(
        category = "internal",
        code = "CREDENTIAL_SERVICE:CAPABILITY_WITHOUT_OPS"
    )]
    #[error(
        "credential type '{key}' advertises capability '{capability}' but no matching operation closure is registered"
    )]
    CapabilityWithoutOps {
        /// Capability name (`refresh` / `test` / `revoke` / `interactive`).
        capability: String,
        /// `Credential::KEY` of the misconfigured type.
        key: String,
    },

    /// No credential type registered under this key.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:TYPE_UNKNOWN")]
    #[error("unknown credential type: {key}")]
    TypeUnknown {
        /// The unregistered credential key.
        key: String,
    },

    /// Interactive acquisition token is expired or already consumed.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:PENDING_EXPIRED")]
    #[error("pending acquisition expired or already consumed")]
    PendingExpired,

    /// An interactive capability was invoked without a session on the
    /// [`TenantScope`](crate::TenantScope). The pending store binds
    /// on `(kind, owner, session, token)`, so an interactive
    /// acquisition/continuation is structurally impossible without one —
    /// surfaced explicitly here rather than collapsing into a misleading
    /// validation failure deeper in the engine.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:SESSION_REQUIRED")]
    #[error("credential capability '{capability}' requires a session on the tenant scope")]
    SessionRequired {
        /// The interactive capability that needs a session
        /// (`resolve` / `continue`).
        capability: &'static str,
    },

    /// An external secret provider failed.
    #[classify(category = "external", code = "CREDENTIAL_SERVICE:PROVIDER")]
    #[error("external provider error: {0}")]
    Provider(String),

    /// A transient provider failure during refresh — network error, rate-limit,
    /// or temporary unavailability. Discriminated from [`Provider`] so the
    /// fallback-on-interrupt path can pattern-match without string-scanning.
    ///
    /// The fallback wrapper in [`CredentialService::refresh`] intercepts this
    /// variant when the stored material is still non-expired and returns the
    /// cached head instead of propagating the error.
    ///
    /// [`Provider`]: Self::Provider
    /// [`CredentialService::refresh`]: crate::CredentialService::refresh
    #[classify(category = "external", code = "CREDENTIAL_SERVICE:TRANSIENT_PROVIDER")]
    #[error("transient provider error during refresh: {0}")]
    TransientProvider(String),

    /// The persistence layer failed.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:STORE")]
    #[error("credential store error: {0}")]
    Store(String),

    /// An external [`StateSource`](crate::StateSource) was configured via
    /// the api-layer credential builder's `external_providers`
    /// but the resolution wiring that consumes it is not implemented in
    /// this crate yet — it lands with the external provider bridge external-source
    /// bridge (see spec §8). Returned instead of
    /// silently resolving from the local store, which would hand back
    /// material from the wrong source.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:EXTERNAL_NOT_WIRED")]
    #[error(
        "external credential source '{provider}' is configured but its resolution wiring is not \
         implemented yet (external provider bridge)"
    )]
    ExternalSourceNotWired {
        /// `ExternalProvider::provider_name()` of the configured source.
        provider: String,
    },

    /// An invariant the runtime owns was violated.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:INTERNAL")]
    #[error("internal credential runtime error: {0}")]
    Internal(String),

    /// The caller's cancellation token fired during the operation.
    ///
    /// The operation terminated without partial state mutation.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:CANCELLED")]
    #[error("credential operation cancelled")]
    Cancelled,

    /// The validated binding's tenant fingerprint did not match the
    /// caller's scope.
    ///
    /// Defence-in-depth check: [`validate_credential_binding`] already
    /// enforced the scope at construction; this variant fires only when
    /// the binding is presented against a mismatched scope at
    /// `resolve_for_slot` time.
    ///
    /// [`validate_credential_binding`]: crate::CredentialService::validate_credential_binding
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:SCOPE_VIOLATION")]
    #[error(
        "scope violation: credential binding validated for a different tenant than `{requested}`"
    )]
    ScopeViolation {
        /// `owner_id` of the caller's scope.
        requested: String,
    },
}

#[cfg(test)]
mod tests {
    use super::CredentialServiceError;

    #[test]
    fn display_messages_are_actionable() {
        let e = CredentialServiceError::NotFound {
            id: "cred-1".to_owned(),
        };
        assert_eq!(e.to_string(), "credential not found: cred-1");

        let e = CredentialServiceError::VersionConflict {
            id: "cred-1".to_owned(),
            expected: 3,
            actual: 4,
        };
        assert_eq!(
            e.to_string(),
            "version conflict for cred-1: expected 3, got 4"
        );

        let e = CredentialServiceError::CapabilityUnsupported {
            capability: "refresh".to_owned(),
            key: "bearer_token".to_owned(),
        };
        assert_eq!(
            e.to_string(),
            "credential type 'bearer_token' does not support capability 'refresh'"
        );
    }

    #[test]
    fn is_std_error() {
        fn assert_error<E: std::error::Error + Send + Sync + 'static>() {}
        assert_error::<CredentialServiceError>();
    }
}
