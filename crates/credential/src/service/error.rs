//! Error taxonomy for the credential management runtime.
//!
//! `#[non_exhaustive]` so later increments add variants without breaking
//! downstream `match` exhaustiveness. Classified via
//! [`nebula_error::Classify`] using only the codebase-standard categories
//! `internal` / `validation` / `external` (mirrors
//! `crates/credential/src/error.rs`).

use std::fmt;

use thiserror::Error;

use crate::{ReauthReason, RefreshNotAppliedContext, RetryAdvice};

/// One secret-safe credential validation issue.
///
/// Only a structural JSON Pointer and a stable machine code cross the
/// credential boundary. Validator messages, parameters, submitted values, and
/// provider error text stay inside the bounded context because any of them may
/// contain credential material.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialValidationIssue {
    path: String,
    code: String,
}

impl CredentialValidationIssue {
    pub(crate) fn new(path: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            code: code.into(),
        }
    }

    /// RFC 6901 pointer to the rejected field; an empty string denotes the
    /// document root.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Stable machine-readable validation code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Debug for CredentialValidationIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialValidationIssue")
            .field("path", &self.path)
            .field("code", &self.code)
            .finish()
    }
}

/// Non-empty, secret-safe validation report.
///
/// Storing the first issue separately prevents an invalid "validation failed
/// with no issues" state while preserving every field error for clients.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialValidationReport {
    first: CredentialValidationIssue,
    related: Vec<CredentialValidationIssue>,
}

impl CredentialValidationReport {
    pub(crate) fn single(path: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            first: CredentialValidationIssue::new(path, code),
            related: Vec::new(),
        }
    }

    pub(crate) fn from_issues(
        first: CredentialValidationIssue,
        related: Vec<CredentialValidationIssue>,
    ) -> Self {
        Self { first, related }
    }

    /// Primary issue; always present by construction.
    #[must_use]
    pub const fn primary(&self) -> &CredentialValidationIssue {
        &self.first
    }

    /// Iterate over every issue in deterministic report order.
    pub fn issues(&self) -> impl Iterator<Item = &CredentialValidationIssue> {
        std::iter::once(&self.first).chain(self.related.iter())
    }
}

impl fmt::Debug for CredentialValidationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.issues()).finish()
    }
}

/// Failure modes of the credential management facade. The API layer maps
/// each `category` to an HTTP status; `code` is the stable machine label.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CredentialServiceError {
    /// No credential with this id in the caller's tenant scope.
    #[error("credential not found: {id}")]
    NotFound {
        /// The credential id that was not found.
        id: String,
    },

    /// Optimistic-concurrency check failed on update.
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// Credential id under contention.
        id: String,
        /// Version the caller expected (CAS precondition).
        expected: u64,
        /// Version actually stored.
        actual: u64,
    },

    /// A generated credential id collided with an id already reserved by this
    /// owner. This is distinct from a display-name conflict.
    #[error("credential id is already reserved")]
    IdAlreadyExists,

    /// A live credential already owns the requested owner-local display name.
    #[error("credential display name is already in use")]
    NameAlreadyExists,

    /// The bounded structural version cannot advance again.
    #[error("credential version is exhausted")]
    VersionExhausted,

    /// Property payload failed the credential type's schema validation.
    #[error("credential properties were rejected")]
    ValidationFailed {
        /// Non-empty structural report. It deliberately carries no validator
        /// message, submitted value, or provider-controlled text.
        report: CredentialValidationReport,
    },

    /// The requested lifecycle op needs a capability the type lacks.
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
    #[error("unknown credential type: {key}")]
    TypeUnknown {
        /// The unregistered credential key.
        key: String,
    },

    /// Interactive acquisition token is expired or already consumed.
    #[error("pending acquisition expired or already consumed")]
    PendingExpired,

    /// An interactive capability was invoked without a session on the
    /// [`TenantScope`](crate::TenantScope). The pending store binds
    /// on `(kind, owner, session, token)`, so an interactive
    /// acquisition/continuation is structurally impossible without one —
    /// surfaced explicitly here rather than collapsing into a misleading
    /// validation failure deeper in the engine.
    #[error("credential capability '{capability}' requires a session on the tenant scope")]
    SessionRequired {
        /// The interactive capability that needs a session
        /// (`resolve` / `continue`).
        capability: &'static str,
    },

    /// An external secret provider failed.
    #[error("external provider error: {0}")]
    Provider(String),

    /// A replay-safe transient failure during credential work.
    ///
    /// On management refresh this is emitted only when coordination fails
    /// before provider dispatch. The fallback wrapper may then return a
    /// still-valid cached head. Once an erased integration is entered, opaque
    /// network/rate-limit/server errors become [`Self::OutcomeUnknown`] instead
    /// because the trait cannot prove a rotating grant was not consumed.
    ///
    /// [`Provider`]: Self::Provider
    /// [`CredentialService::refresh`]: crate::CredentialService::refresh
    #[error("transient provider error during refresh: {0}")]
    TransientProvider(String),

    /// The credential implementation proved that provider state did not
    /// advance and retained its structured retry advice.
    ///
    /// The proof is durably installed as an aggregate retry gate before the
    /// claim is released. Classification delegates to the embedded advice:
    /// `Never` is non-retryable, while `After` exposes the exact delay floor.
    #[error("credential refresh was not applied: {0}")]
    RefreshNotApplied(Box<RefreshNotAppliedContext>),

    /// The credential can no longer refresh itself and needs interactive
    /// re-authentication — the IdP rejected the stored grant
    /// ([`ReauthReason::ProviderRejected`]), an authoritative sentinel command
    /// requested reauthentication ([`ReauthReason::SentinelRepeated`]), or the
    /// local state lacks refresh material
    /// ([`ReauthReason::MissingRefreshMaterial`]). The lossy sentinel event
    /// alone does not construct this durable service outcome.
    ///
    /// A routine OAuth2 outcome, **not** a server fault: classified
    /// `validation` (a client-actionable 4xx "reconnect", never a retryable
    /// 5xx) so a retry layer keyed on [`is_retryable`](nebula_error::Classify::is_retryable)
    /// does not re-POST a dead grant, and the typed [`ReauthReason`] survives to
    /// the API boundary instead of being flattened into a string.
    ///
    /// [`ReauthReason::ProviderRejected`]: crate::ReauthReason::ProviderRejected
    /// [`ReauthReason::SentinelRepeated`]: crate::ReauthReason::SentinelRepeated
    /// [`ReauthReason::MissingRefreshMaterial`]: crate::ReauthReason::MissingRefreshMaterial
    #[error("credential {credential_id} requires re-authentication")]
    ReauthRequired {
        /// The credential id that needs re-authentication.
        credential_id: String,
        /// Why re-authentication is required (typed, for UI / metrics / audit).
        reason: ReauthReason,
    },

    /// Persisted state is structurally corrupt or a non-recoverable pending
    /// store failure occurred.
    #[error("credential persistence failed")]
    Store,

    /// The authoritative persistence source could not be reached.
    ///
    /// Kept distinct from [`Self::Store`] so API/composition adapters can
    /// return an honest retryable 503 without exposing backend diagnostics.
    #[error("credential persistence is temporarily unavailable")]
    PersistenceUnavailable,

    /// A provider side effect or durable mutation may have completed without
    /// an exact acknowledgement.
    ///
    /// This is deliberately distinct from [`Self::Store`]: callers must
    /// reconcile through an owner-qualified read and must not blindly replay
    /// the provider operation or durable mutation.
    #[error("credential mutation outcome is unknown; reconcile before retrying")]
    OutcomeUnknown,

    /// A concurrent refresh reached an exact finalization failure whose
    /// operation-specific details are intentionally not shared through L1.
    ///
    /// The durable claim remains retained as poison. The result is therefore
    /// known to require reconciliation, not an unknown provider outcome and
    /// not a retryable persistence outage.
    #[error("credential refresh requires reconciliation before retrying")]
    RefreshReconciliationRequired,

    /// An exact no-effect refresh result could not be finalized into the
    /// durable aggregate retry gate.
    ///
    /// The provider operation is known not to have changed provider state
    /// (either dispatch never began or a complete response proved no effect),
    /// but the framework must retain the refresh claim because releasing it
    /// would lose the duplicate-suppression decision. This is exact and
    /// non-retryable; operators must reconcile rather than replaying refresh.
    #[error("credential refresh retry gate could not be finalized; reconcile before retrying")]
    RefreshRetryGateFinalization,

    /// A provider-confirmed reauthentication decision could not be finalized
    /// into durable credential state.
    ///
    /// The provider result itself is exact, but automatically retrying the
    /// credential refresh would re-submit a grant the provider already
    /// rejected. The retained claim requires explicit reconciliation.
    #[error(
        "credential reauthentication decision could not be finalized; reconcile before retrying"
    )]
    ReauthDecisionFinalization,

    /// Provider-side refresh completed, but its durable local transition
    /// definitely failed.
    ///
    /// The failure is exact, but replaying the provider operation is unsafe:
    /// a rotating grant may already have been consumed. Callers must reconcile
    /// or re-authorize rather than retry the whole command.
    #[error(
        "provider refresh completed but durable credential finalization failed; reconcile before retrying"
    )]
    RefreshPostProviderPersistence,

    /// Provider-side revocation completed or was reported complete, but the
    /// durable tombstone/finalization transition definitely failed.
    ///
    /// Replaying revoke may repeat an external side effect. Callers must
    /// reconcile revocation state rather than retrying the command blindly.
    #[error(
        "provider revoke completed but durable credential finalization failed; reconcile before retrying"
    )]
    RevokePostProviderPersistence,

    /// An external [`StateSource`](crate::StateSource) was configured via
    /// the api-layer credential builder's `external_providers`
    /// but the resolution wiring that consumes it is not implemented in
    /// this crate yet — it lands with the external provider bridge external-source
    /// bridge (see spec §8). Returned instead of
    /// silently resolving from the local store, which would hand back
    /// material from the wrong source.
    #[error(
        "external credential source '{provider}' is configured but its resolution wiring is not \
         implemented yet (external provider bridge)"
    )]
    ExternalSourceNotWired {
        /// `ExternalProvider::provider_name()` of the configured source.
        provider: String,
    },

    /// An invariant the runtime owns was violated.
    #[error("internal credential runtime error: {0}")]
    Internal(String),

    /// The caller's cancellation token fired during the operation.
    ///
    /// The operation terminated without partial state mutation.
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
    #[error(
        "scope violation: credential binding validated for a different tenant than `{requested}`"
    )]
    ScopeViolation {
        /// `owner_id` of the caller's scope.
        requested: String,
    },
}

impl nebula_error::Classify for CredentialServiceError {
    fn category(&self) -> nebula_error::ErrorCategory {
        use nebula_error::ErrorCategory;
        match self {
            Self::NotFound { .. }
            | Self::VersionConflict { .. }
            | Self::ValidationFailed { .. }
            | Self::CapabilityUnsupported { .. }
            | Self::TypeUnknown { .. }
            | Self::PendingExpired
            | Self::SessionRequired { .. }
            | Self::ReauthRequired { .. }
            | Self::ScopeViolation { .. } => ErrorCategory::Validation,
            Self::IdAlreadyExists | Self::NameAlreadyExists | Self::VersionExhausted => {
                ErrorCategory::Conflict
            },
            Self::Provider(_)
            | Self::TransientProvider(_)
            | Self::RefreshNotApplied(_)
            | Self::PersistenceUnavailable => ErrorCategory::External,
            Self::CapabilityWithoutOps { .. }
            | Self::Store
            | Self::OutcomeUnknown
            | Self::RefreshReconciliationRequired
            | Self::RefreshRetryGateFinalization
            | Self::ReauthDecisionFinalization
            | Self::RefreshPostProviderPersistence
            | Self::RevokePostProviderPersistence
            | Self::ExternalSourceNotWired { .. }
            | Self::Internal(_)
            | Self::Cancelled => ErrorCategory::Internal,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        let code = match self {
            Self::NotFound { .. } => "CREDENTIAL_SERVICE:NOT_FOUND",
            Self::VersionConflict { .. } => "CREDENTIAL_SERVICE:VERSION_CONFLICT",
            Self::IdAlreadyExists => "CREDENTIAL_SERVICE:ID_ALREADY_EXISTS",
            Self::NameAlreadyExists => "CREDENTIAL_SERVICE:NAME_ALREADY_EXISTS",
            Self::VersionExhausted => "CREDENTIAL_SERVICE:VERSION_EXHAUSTED",
            Self::ValidationFailed { .. } => "CREDENTIAL_SERVICE:VALIDATION_FAILED",
            Self::CapabilityUnsupported { .. } => "CREDENTIAL_SERVICE:CAPABILITY_UNSUPPORTED",
            Self::CapabilityWithoutOps { .. } => "CREDENTIAL_SERVICE:CAPABILITY_WITHOUT_OPS",
            Self::TypeUnknown { .. } => "CREDENTIAL_SERVICE:TYPE_UNKNOWN",
            Self::PendingExpired => "CREDENTIAL_SERVICE:PENDING_EXPIRED",
            Self::SessionRequired { .. } => "CREDENTIAL_SERVICE:SESSION_REQUIRED",
            Self::Provider(_) => "CREDENTIAL_SERVICE:PROVIDER",
            Self::TransientProvider(_) => "CREDENTIAL_SERVICE:TRANSIENT_PROVIDER",
            Self::RefreshNotApplied(_) => "CREDENTIAL_SERVICE:REFRESH_NOT_APPLIED",
            Self::ReauthRequired { .. } => "CREDENTIAL_SERVICE:REAUTH_REQUIRED",
            Self::Store => "CREDENTIAL_SERVICE:STORE",
            Self::PersistenceUnavailable => "CREDENTIAL_SERVICE:PERSISTENCE_UNAVAILABLE",
            Self::OutcomeUnknown => "CREDENTIAL_SERVICE:OUTCOME_UNKNOWN",
            Self::RefreshReconciliationRequired => {
                "CREDENTIAL_SERVICE:REFRESH_RECONCILIATION_REQUIRED"
            },
            Self::RefreshRetryGateFinalization => {
                "CREDENTIAL_SERVICE:REFRESH_RETRY_GATE_FINALIZATION"
            },
            Self::ReauthDecisionFinalization => "CREDENTIAL_SERVICE:REAUTH_DECISION_FINALIZATION",
            Self::RefreshPostProviderPersistence => {
                "CREDENTIAL_SERVICE:REFRESH_POST_PROVIDER_PERSISTENCE"
            },
            Self::RevokePostProviderPersistence => {
                "CREDENTIAL_SERVICE:REVOKE_POST_PROVIDER_PERSISTENCE"
            },
            Self::ExternalSourceNotWired { .. } => "CREDENTIAL_SERVICE:EXTERNAL_NOT_WIRED",
            Self::Internal(_) => "CREDENTIAL_SERVICE:INTERNAL",
            Self::Cancelled => "CREDENTIAL_SERVICE:CANCELLED",
            Self::ScopeViolation { .. } => "CREDENTIAL_SERVICE:SCOPE_VIOLATION",
        };
        nebula_error::ErrorCode::new(code)
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::RefreshNotApplied(context) => {
                matches!(context.retry(), RetryAdvice::After(_))
            },
            _ => self.category().is_default_retryable(),
        }
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::RefreshNotApplied(context) => match context.retry() {
                RetryAdvice::Never => None,
                RetryAdvice::After(delay) => Some(nebula_error::RetryHint::after(delay.get())),
            },
            _ => None,
        }
    }
}

impl CredentialServiceError {
    pub(crate) fn validation(path: impl Into<String>, code: impl Into<String>) -> Self {
        Self::ValidationFailed {
            report: CredentialValidationReport::single(path, code),
        }
    }
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

    #[test]
    fn reauth_required_is_validation_and_not_retryable() {
        use nebula_error::Classify;
        // Re-auth is client-actionable, not a server fault: it must classify
        // `validation` (a 4xx "reconnect") and be non-retryable so the facade's
        // retry layer does not re-POST a rejected grant (F14 / F23).
        let e = CredentialServiceError::ReauthRequired {
            credential_id: "cred-1".to_owned(),
            reason: crate::ReauthReason::ProviderRejected,
        };
        assert_eq!(e.category(), nebula_error::ErrorCategory::Validation);
        assert!(
            !e.is_retryable(),
            "re-authentication must not be retried — retrying re-POSTs a dead grant"
        );
    }

    #[test]
    fn outcome_unknown_is_explicit_and_not_retryable() {
        use nebula_error::Classify;

        let error = CredentialServiceError::OutcomeUnknown;
        assert_eq!(error.category(), nebula_error::ErrorCategory::Internal);
        assert!(
            !error.is_retryable(),
            "an unacknowledged commit requires reconciliation, not replay"
        );
    }

    #[test]
    fn refresh_reconciliation_is_exact_and_not_retryable() {
        use nebula_error::Classify;

        let error = CredentialServiceError::RefreshReconciliationRequired;
        assert_eq!(error.category(), nebula_error::ErrorCategory::Internal);
        assert_eq!(
            error.code(),
            nebula_error::ErrorCode::new("CREDENTIAL_SERVICE:REFRESH_RECONCILIATION_REQUIRED")
        );
        assert!(
            !error.is_retryable(),
            "an exact retry-unsafe result requires reconciliation, not replay"
        );
        assert!(error.retry_hint().is_none());
    }

    #[test]
    fn operation_specific_post_provider_failures_are_explicit_and_not_retryable() {
        use nebula_error::Classify;

        for (error, code) in [
            (
                CredentialServiceError::RefreshPostProviderPersistence,
                "CREDENTIAL_SERVICE:REFRESH_POST_PROVIDER_PERSISTENCE",
            ),
            (
                CredentialServiceError::RevokePostProviderPersistence,
                "CREDENTIAL_SERVICE:REVOKE_POST_PROVIDER_PERSISTENCE",
            ),
        ] {
            assert_eq!(error.category(), nebula_error::ErrorCategory::Internal);
            assert!(
                !error.is_retryable(),
                "provider work cannot be replayed after definite local finalization failure"
            );
            assert_eq!(error.code().as_str(), code);
        }
    }

    #[test]
    fn exact_refresh_finalization_failures_are_distinct_and_not_retryable() {
        use nebula_error::Classify;

        let cases = [
            (
                CredentialServiceError::RefreshRetryGateFinalization,
                "CREDENTIAL_SERVICE:REFRESH_RETRY_GATE_FINALIZATION",
            ),
            (
                CredentialServiceError::ReauthDecisionFinalization,
                "CREDENTIAL_SERVICE:REAUTH_DECISION_FINALIZATION",
            ),
        ];
        for (error, expected_code) in cases {
            assert_eq!(error.category(), nebula_error::ErrorCategory::Internal);
            assert_eq!(error.code().as_str(), expected_code);
            assert!(
                !error.is_retryable(),
                "an exact finalization failure requires reconciliation"
            );
            assert!(error.retry_hint().is_none());
        }
    }

    #[test]
    fn refresh_not_applied_classification_preserves_never_vs_after() {
        use std::time::Duration;

        use nebula_error::Classify;

        let never = CredentialServiceError::RefreshNotApplied(Box::new(
            crate::RefreshNotAppliedContext::from_spec(
                crate::RefreshNotAppliedPhase::BeforeDispatch,
                crate::RefreshFailureSpec::new(
                    crate::RefreshErrorKind::ProtocolError,
                    crate::RetryAdvice::Never,
                ),
            ),
        ));
        assert!(!never.is_retryable());
        assert!(never.retry_hint().is_none());

        let delay = crate::RetryDelay::new(Duration::from_secs(7))
            .expect("seven seconds is a valid retry delay");
        let after = CredentialServiceError::RefreshNotApplied(Box::new(
            crate::RefreshNotAppliedContext::from_spec(
                crate::RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
                crate::RefreshFailureSpec::new(
                    crate::RefreshErrorKind::ProviderUnavailable,
                    crate::RetryAdvice::After(delay),
                ),
            ),
        ));
        assert!(after.is_retryable());
        assert_eq!(
            after.retry_hint().and_then(|hint| hint.after),
            Some(Duration::from_secs(7))
        );
    }
}
