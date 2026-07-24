//! API-owned credential command port.
//!
//! HTTP handlers submit authenticated intent through this object-safe seam.
//! The port deliberately exposes no storage selector, owner key, repository,
//! credential service, or authority proof. First-party deployment adapters
//! live in `apps/` and translate this contract into the credential-owned
//! authority/controller boundary.

use std::{collections::BTreeMap, fmt, num::NonZeroU64};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nebula_storage_port::Scope;
use thiserror::Error;

use crate::{
    domain::credential::dto::{
        AcquisitionInteraction, ContinueResolveRequest, CreateCredentialRequest,
        ResolveCredentialRequest, UpdateCredentialRequest,
    },
    middleware::auth::AuthenticatedPrincipal,
    ports::credential_schema::{CredentialValidationCode, CredentialValidationLocation},
};

/// Authenticated public credential intent.
#[non_exhaustive]
pub enum CredentialGatewayCommand {
    /// Create a credential.
    Create(CreateCredentialRequest),
    /// Read one credential.
    Get {
        /// Credential identifier.
        credential_id: String,
    },
    /// Enumerate the caller's workspace credentials.
    List,
    /// Update one credential.
    Update {
        /// Credential identifier.
        credential_id: String,
        /// Partial update request.
        request: UpdateCredentialRequest,
    },
    /// Tombstone one credential and remove its live material.
    Delete {
        /// Credential identifier.
        credential_id: String,
    },
    /// Test provider connectivity.
    Test {
        /// Credential identifier.
        credential_id: String,
    },
    /// Refresh provider material.
    Refresh {
        /// Credential identifier.
        credential_id: String,
    },
    /// Revoke provider material.
    Revoke {
        /// Credential identifier.
        credential_id: String,
    },
    /// Begin credential acquisition.
    Resolve(ResolveCredentialRequest),
    /// Continue credential acquisition.
    ContinueResolve(ContinueResolveRequest),
}

impl CredentialGatewayCommand {
    /// Stable operation label for traces and diagnostics.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::Get { .. } => "get",
            Self::List => "list",
            Self::Update { .. } => "update",
            Self::Delete { .. } => "delete",
            Self::Test { .. } => "test",
            Self::Refresh { .. } => "refresh",
            Self::Revoke { .. } => "revoke",
            Self::Resolve(_) => "resolve",
            Self::ContinueResolve(_) => "continue_resolve",
        }
    }
}

impl fmt::Debug for CredentialGatewayCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialGatewayCommand")
            .field("operation", &self.operation())
            .finish_non_exhaustive()
    }
}

/// Secret-free management projection returned by the gateway.
///
/// Display fields are deliberately present on the wire but omitted from
/// `Debug`, because they are user controlled even though the contract marks
/// them as non-secret metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialGatewayRecord {
    /// Credential identifier.
    pub id: String,
    /// Registered credential type key.
    pub credential_key: String,
    /// Optimistic-concurrency version.
    pub version: u64,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last-write timestamp.
    pub updated_at: DateTime<Utc>,
    /// Material expiry, when applicable.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether interactive re-authorization is required.
    pub reauth_required: bool,
    /// Optional human-facing name.
    pub display_name: Option<String>,
    /// Optional human-facing description.
    pub description: Option<String>,
    /// Deterministically ordered user tags.
    pub tags: BTreeMap<String, String>,
}

impl fmt::Debug for CredentialGatewayRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialGatewayRecord")
            .field("id", &self.id)
            .field("credential_key", &self.credential_key)
            .field("version", &self.version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("reauth_required", &self.reauth_required)
            .field("display_name_present", &self.display_name.is_some())
            .field("description_present", &self.description.is_some())
            .field("tag_count", &self.tags.len())
            .finish()
    }
}

/// Secret-free provider-test classification crossing the API port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialGatewayTestFailure {
    /// Provider rejected the authentication material.
    AuthenticationRejected,
    /// Authentication succeeded but permission was insufficient.
    PermissionDenied,
    /// Provider account is disabled, locked, or restricted.
    AccountRestricted,
    /// Credential or provider configuration is invalid.
    InvalidConfiguration,
    /// Another safely classified failure.
    Other,
}

/// Provider connectivity result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialGatewayTestResult {
    /// Provider accepted the credential.
    Success,
    /// Provider rejected the credential with a payload-free classification.
    Failed(CredentialGatewayTestFailure),
}

/// Credential acquisition result.
#[non_exhaustive]
pub enum CredentialGatewayAcquisition {
    /// Acquisition completed and the credential was persisted.
    Complete {
        /// Persisted credential identifier.
        credential_id: String,
    },
    /// Further interaction is required.
    Pending {
        /// Opaque short-lived continuation token.
        pending_token: String,
        /// API-owned interaction description.
        interaction: AcquisitionInteraction,
    },
    /// The caller should poll again after this delay.
    Retry {
        /// Delay in seconds.
        retry_after_secs: u64,
    },
}

impl fmt::Debug for CredentialGatewayAcquisition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete { credential_id } => formatter
                .debug_struct("Complete")
                .field("credential_id", credential_id)
                .finish(),
            Self::Pending { interaction, .. } => formatter
                .debug_struct("Pending")
                .field("pending_token", &"[REDACTED]")
                .field("interaction", interaction)
                .finish(),
            Self::Retry { retry_after_secs } => formatter
                .debug_struct("Retry")
                .field("retry_after_secs", retry_after_secs)
                .finish(),
        }
    }
}

/// Result of one gateway command.
#[non_exhaustive]
pub enum CredentialGatewayResult {
    /// One secret-free credential record.
    Record(CredentialGatewayRecord),
    /// Secret-free records in the authorized workspace.
    Records(Vec<CredentialGatewayRecord>),
    /// Credential was deleted.
    Deleted,
    /// Provider connectivity result.
    Tested(CredentialGatewayTestResult),
    /// Provider refresh result.
    Refreshed {
        /// Secret-free post-call record.
        record: CredentialGatewayRecord,
        /// Whether this caller persisted refreshed material.
        refreshed: bool,
    },
    /// Credential was revoked.
    Revoked,
    /// Credential acquisition result.
    Acquisition(CredentialGatewayAcquisition),
}

impl fmt::Debug for CredentialGatewayResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Record(record) => formatter.debug_tuple("Record").field(record).finish(),
            Self::Records(records) => formatter
                .debug_struct("Records")
                .field("count", &records.len())
                .finish(),
            Self::Deleted => formatter.write_str("Deleted"),
            Self::Tested(result) => formatter.debug_tuple("Tested").field(result).finish(),
            Self::Refreshed { record, refreshed } => formatter
                .debug_struct("Refreshed")
                .field("record", record)
                .field("refreshed", refreshed)
                .finish(),
            Self::Revoked => formatter.write_str("Revoked"),
            Self::Acquisition(acquisition) => formatter
                .debug_tuple("Acquisition")
                .field(acquisition)
                .finish(),
        }
    }
}

/// One structural, secret-safe validation issue crossing the command port.
///
/// The gateway deliberately carries no validator message, parameters, source
/// error, or submitted value. The API owns the user-facing copy for each
/// stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialGatewayValidationIssue {
    location: CredentialValidationLocation,
    code: CredentialValidationCode,
}

impl CredentialGatewayValidationIssue {
    /// Construct a value-free validation issue in a first-party gateway
    /// adapter.
    #[must_use]
    pub const fn new(
        location: CredentialValidationLocation,
        code: CredentialValidationCode,
    ) -> Self {
        Self { location, code }
    }

    /// API-owned RFC 6901 pointer.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.location.pointer()
    }

    /// Stable machine-readable validation code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code.as_str()
    }

    /// API-owned value-free message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.code.message()
    }
}

/// Non-empty validation report crossing the API command port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialGatewayValidationReport {
    first: CredentialGatewayValidationIssue,
    related: Vec<CredentialGatewayValidationIssue>,
}

impl CredentialGatewayValidationReport {
    /// Construct a report from its mandatory primary issue and any related
    /// issues in deterministic order.
    #[must_use]
    pub fn new(
        first: CredentialGatewayValidationIssue,
        related: Vec<CredentialGatewayValidationIssue>,
    ) -> Self {
        Self { first, related }
    }

    /// Construct a one-issue report.
    #[must_use]
    pub fn single(location: CredentialValidationLocation, code: CredentialValidationCode) -> Self {
        Self::new(
            CredentialGatewayValidationIssue::new(location, code),
            Vec::new(),
        )
    }

    /// Iterate over every issue in report order.
    pub fn issues(&self) -> impl Iterator<Item = &CredentialGatewayValidationIssue> {
        std::iter::once(&self.first).chain(self.related.iter())
    }
}

/// Closed retry policy for a refresh proven not to have changed provider state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialGatewayRefreshRetry {
    /// The current credential state must not be retried automatically.
    Never,
    /// Retry after a non-zero whole-second delay.
    After {
        /// Validated non-zero delay exposed through HTTP `Retry-After`.
        seconds: NonZeroU64,
    },
}

/// Stable failure taxonomy at the API/composition boundary.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialGatewayError {
    /// Credential is absent in the authorized owner partition.
    #[error("credential not found")]
    NotFound,
    /// Compare-and-swap precondition failed.
    #[error("credential version conflict: expected {expected}, found {actual}")]
    VersionConflict {
        /// Requested version.
        expected: u64,
        /// Stored version.
        actual: u64,
    },
    /// A generated credential id is already reserved.
    #[error("credential id is already reserved")]
    IdAlreadyExists,
    /// A live credential already owns the requested display name.
    #[error("credential display name is already in use")]
    NameAlreadyExists,
    /// The bounded structural version cannot advance.
    #[error("credential version is exhausted")]
    VersionExhausted,
    /// Type-specific properties were rejected.
    #[error("credential properties were rejected")]
    ValidationFailed {
        /// Non-empty structural report safe to return to the authenticated
        /// client.
        report: CredentialGatewayValidationReport,
    },
    /// The requested credential type is unknown.
    #[error("unknown credential type: {key}")]
    TypeUnknown {
        /// Requested type key.
        key: String,
    },
    /// The credential type does not implement an operation.
    #[error("credential type '{key}' does not support '{capability}'")]
    CapabilityUnsupported {
        /// Capability name.
        capability: String,
        /// Credential type key.
        key: String,
    },
    /// Pending acquisition authority expired or was consumed.
    #[error("pending credential acquisition expired")]
    PendingExpired,
    /// Credential must be connected again.
    #[error("credential requires re-authentication")]
    ReauthRequired,
    /// Refresh was proven not to have changed provider state.
    ///
    /// Retry policy is closed and structurally excludes a zero-second delay.
    #[error("credential refresh was not applied")]
    RefreshNotApplied {
        /// Exact retry policy retained from the credential integration.
        retry: CredentialGatewayRefreshRetry,
    },
    /// The refresh outcome is known, but durable local refresh finalization
    /// definitely failed.
    ///
    /// Automatic replay is unsafe. The caller must reconcile or reconnect the
    /// integration credential before issuing another refresh.
    #[error("credential refresh requires reconciliation")]
    RefreshReconciliationRequired,
    /// The revoke outcome is known, but durable local finalization definitely
    /// failed.
    ///
    /// Automatic replay is unsafe. The caller must reconcile credential state
    /// before issuing another revoke.
    #[error("credential revoke requires reconciliation")]
    RevokeReconciliationRequired,
    /// Authenticated actor is not authorized for the tenant operation.
    #[error("credential command forbidden")]
    Forbidden,
    /// A required provider, authority, or persistence service is unavailable.
    #[error("credential command service unavailable")]
    Unavailable,
    /// A provider side effect or durable mutation may have completed without
    /// exact acknowledgement.
    ///
    /// The caller must reconcile the owner-qualified credential state before
    /// deciding whether replaying either operation is safe.
    #[error("credential mutation outcome is unknown; reconcile before retrying")]
    OutcomeUnknown,
    /// Internal invariant or composition failure.
    #[error("credential command failed internally")]
    Internal,
}

/// Object-safe authenticated credential command seam.
#[async_trait]
pub trait CredentialCommandGateway: fmt::Debug + Send + Sync {
    /// Execute one public intent for one middleware-authenticated principal and
    /// one already-resolved tenant scope.
    async fn execute(
        &self,
        principal: &AuthenticatedPrincipal,
        scope: &Scope,
        command: CredentialGatewayCommand,
    ) -> Result<CredentialGatewayResult, CredentialGatewayError>;
}

#[cfg(feature = "test-util")]
mod testkit;
#[cfg(feature = "test-util")]
pub use testkit::test_gateway_from_service;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_debug_redacts_request_payload() {
        const CANARY: &str = "api-gateway-secret-never-debug";
        let command = CredentialGatewayCommand::Create(CreateCredentialRequest {
            credential_key: "api_key".to_owned(),
            name: CANARY.to_owned(),
            description: Some(CANARY.to_owned()),
            data: serde_json::json!({ "api_key": CANARY }),
            tags: None,
        });
        assert!(!format!("{command:?}").contains(CANARY));
    }

    #[test]
    fn record_debug_redacts_user_controlled_display_values() {
        const CANARY: &str = "api-gateway-display-never-debug";
        let now = Utc::now();
        let record = CredentialGatewayRecord {
            id: "cred_safe".to_owned(),
            credential_key: "api_key".to_owned(),
            version: 1,
            created_at: now,
            updated_at: now,
            expires_at: None,
            reauth_required: false,
            display_name: Some(CANARY.to_owned()),
            description: Some(CANARY.to_owned()),
            tags: BTreeMap::from([(CANARY.to_owned(), CANARY.to_owned())]),
        };

        let debug = format!("{record:?}");
        assert!(!debug.contains(CANARY));
        assert!(debug.contains("tag_count: 1"));
    }

    #[test]
    fn refresh_not_applied_preserves_never_and_after_without_dynamic_text() {
        let never = CredentialGatewayError::RefreshNotApplied {
            retry: CredentialGatewayRefreshRetry::Never,
        };
        let after = CredentialGatewayError::RefreshNotApplied {
            retry: CredentialGatewayRefreshRetry::After {
                seconds: NonZeroU64::new(17).expect("test delay is non-zero"),
            },
        };

        assert_eq!(never.clone(), never);
        assert_eq!(after.clone(), after);
        assert_ne!(never, after);
    }

    #[test]
    fn operation_reconciliation_variants_are_distinct_from_each_other_and_lost_acknowledgement() {
        assert_ne!(
            CredentialGatewayError::RefreshReconciliationRequired,
            CredentialGatewayError::OutcomeUnknown
        );
        assert_ne!(
            CredentialGatewayError::RevokeReconciliationRequired,
            CredentialGatewayError::OutcomeUnknown
        );
        assert_ne!(
            CredentialGatewayError::RefreshReconciliationRequired,
            CredentialGatewayError::RevokeReconciliationRequired
        );
    }
}
