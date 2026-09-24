//! Error handling — RFC 9457 `application/problem+json` seam (problem+json error seam).
//!
//! ## Structure
//!
//! - [`problem`] — `ProblemDetails` wire type (RFC 9457) and `ValidationFieldError`.
//! - [`classify`] — `From<…>` conversions from domain error types into [`ApiError`].
//! - This file — [`ApiError`] enum, [`ApiResult`] alias, and the
//!   [`axum::response::IntoResponse`] impl that sets `Content-Type:
//!   application/problem+json`.
//!
//! ## Wire contract
//!
//! The serialized shape of [`ProblemDetails`] and the HTTP status codes
//! produced by [`ApiError::to_problem_details`] are enforced byte-for-byte by
//! `tests/openapi_canon_compliance.rs`.  Do not alter field names,
//! `type_uri` strings, or status codes without updating that test.

pub mod classify;
pub mod problem;

pub use problem::{ProblemDetails, ValidationFieldError};

use std::num::NonZeroU64;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

/// Main API Error Type
#[non_exhaustive]
#[derive(Debug, Error, nebula_error::Classify)]
pub enum ApiError {
    /// Validation error (400)
    #[classify(category = "validation", code = "API:VALIDATION")]
    #[error("Validation failed: {detail}")]
    Validation {
        /// High-level validation summary.
        detail: String,
        /// Field-level validation details with code and JSON pointer.
        errors: Vec<ValidationFieldError>,
    },

    /// Authentication error (401)
    #[classify(category = "authentication", code = "API:UNAUTHORIZED")]
    #[error("Authentication failed: {0}")]
    Unauthorized(String),

    /// Authorization error (403)
    #[classify(category = "authorization", code = "API:FORBIDDEN")]
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// Not found (404)
    #[classify(category = "not_found", code = "API:NOT_FOUND")]
    #[error("Not found: {0}")]
    NotFound(String),

    /// Conflict (409)
    #[classify(category = "conflict", code = "API:CONFLICT")]
    #[error("Conflict: {0}")]
    Conflict(String),

    /// A unique domain identity or name is already reserved (409).
    #[classify(category = "conflict", code = "API:ALREADY_EXISTS")]
    #[error("Already exists: {0}")]
    AlreadyExists(String),

    /// The start key is reserved for a different request (409).
    ///
    /// A start key identifies one accepted command. Reusing it for a request
    /// that canonicalizes differently is refused rather than accepted, and
    /// nothing durable changed. The response carries no execution id: the
    /// caller proved knowledge of a key, not of the execution behind it.
    #[classify(category = "conflict", code = "API:START_CONFLICT")]
    #[error("Start key already reserved for a different request")]
    StartConflict,

    /// A bounded structural version can no longer advance (409).
    #[classify(category = "conflict", code = "API:VERSION_EXHAUSTED")]
    #[error("Version exhausted: {0}")]
    VersionExhausted(String),

    /// A mutation may have committed, but its acknowledgement was lost (409).
    ///
    /// This is non-retryable by default: the client must reconcile state before
    /// deciding whether replay is safe.
    #[classify(category = "conflict", code = "API:OUTCOME_UNKNOWN")]
    #[error("Operation outcome unknown: {0}")]
    OutcomeUnknown(String),

    /// The external integration credential must be reconnected (409).
    ///
    /// This is deliberately distinct from [`Self::Unauthorized`]: the
    /// caller's Nebula identity/session is still authenticated. Retrying the
    /// same provider grant is unsafe or impossible until the integration is
    /// re-authorized.
    #[classify(category = "conflict", code = "API:CREDENTIAL_REAUTH_REQUIRED")]
    #[error("Integration credential requires re-authentication")]
    CredentialReauthRequired,

    /// A refresh attempt was proven not to have changed provider state and
    /// automatic retry is forbidden (409).
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_REFRESH_NOT_APPLIED_NEVER",
        retryable = false
    )]
    #[error("Credential refresh was not applied and is not retryable")]
    CredentialRefreshNotAppliedNever,

    /// A refresh attempt was proven not to have changed provider state and may
    /// be retried after a non-zero delay (409).
    ///
    /// `Classify::is_retryable` is true. The field-dependent delay is carried
    /// by HTTP `Retry-After`; the derive macro intentionally emits no static
    /// `RetryHint`.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_REFRESH_NOT_APPLIED_AFTER",
        retryable = true
    )]
    #[error("Credential refresh was not applied; retry after {retry_after_secs} seconds")]
    CredentialRefreshNotAppliedAfter {
        /// Validated non-zero whole-second delay.
        retry_after_secs: NonZeroU64,
    },

    /// The refresh outcome is known, but durable local finalization
    /// definitely failed (409).
    ///
    /// This is distinct from [`Self::OutcomeUnknown`]: the mutation outcome is
    /// known, so automatic replay is unsafe and the integration credential
    /// must be reconciled or reconnected.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_REFRESH_RECONCILIATION_REQUIRED",
        retryable = false
    )]
    #[error("Credential refresh requires reconciliation")]
    CredentialRefreshReconciliationRequired,

    /// Credential acquisition completed, but its durable create or
    /// replacement definitely failed (409).
    ///
    /// Replaying may repeat provider work or re-submit a one-time grant, so
    /// the client must reconcile state or restart authorization deliberately.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_ACQUISITION_RECONCILIATION_REQUIRED",
        retryable = false
    )]
    #[error("Credential acquisition requires reconciliation")]
    CredentialAcquisitionReconciliationRequired,

    /// The revoke outcome is known, but durable local finalization definitely
    /// failed (409).
    ///
    /// Automatic replay is unsafe. The client must reconcile credential state
    /// before deciding whether another revoke is appropriate.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_REVOKE_RECONCILIATION_REQUIRED",
        retryable = false
    )]
    #[error("Credential revoke requires reconciliation")]
    CredentialRevokeReconciliationRequired,

    /// Rate limit exceeded (429)
    #[classify(category = "rate_limit", code = "API:RATE_LIMIT")]
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// Internal server error (500)
    #[classify(category = "internal", code = "API:INTERNAL")]
    #[error("Internal server error: {0}")]
    Internal(String),

    /// Service unavailable (503)
    #[classify(category = "external", code = "API:SERVICE_UNAVAILABLE")]
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Storage error
    #[classify(category = "internal", code = "API:STORAGE")]
    #[error("Storage error: {0}")]
    Storage(#[from] nebula_storage_port::StorageError),

    /// Invalid workflow definition — structurally valid JSON but semantically
    /// invalid per `nebula_workflow::validate_workflow` (RFC 9457 **422**).
    ///
    /// Distinct from [`Self::Validation`] (400), which covers request-level
    /// parse/format errors. Carries structured definition validation diagnostics.
    #[classify(category = "validation", code = "API:INVALID_WORKFLOW")]
    #[error("Invalid workflow definition: {detail}")]
    InvalidWorkflowDefinition {
        /// Human-readable summary of all validation failures.
        detail: String,
        /// One entry per `WorkflowError` returned by `validate_workflow`.
        ///
        /// The typed errors are carried rather than pre-rendered so
        /// `to_problem_details` can emit each rejection's activation diagnostic —
        /// its code, the element it is about, the contract it required, what
        /// was found, and how to fix it.
        errors: Vec<nebula_workflow::WorkflowError>,
    },

    /// Exact compilation rejected a workflow; preserve its structured diagnostics.
    #[classify(category = "validation", code = "API:WORKFLOW_COMPILATION")]
    #[error("Workflow compilation failed")]
    WorkflowCompilation(#[source] nebula_plugin::PlanCompilationError),

    /// Publication may have committed; these identify the original attempt.
    #[classify(category = "internal", code = "API:WORKFLOW_PUBLICATION_INDETERMINATE")]
    #[error("Workflow publication outcome is indeterminate")]
    WorkflowPublicationIndeterminate {
        /// Workflow in the authenticated request scope.
        workflow_id: String,
        /// Exact version attempted, not the current version.
        version: u32,
        /// Immutable compiler identity allocated for this attempt.
        workflow_revision: String,
    },

    /// Runtime admission rejected an inactive or unsupported workflow contract.
    #[classify(category = "validation", code = "API:WORKFLOW_START_REJECTED")]
    #[error("Workflow start was rejected: {reason}")]
    WorkflowStartRejected {
        /// Fixed, payload-free admission explanation.
        reason: &'static str,
    },

    /// Acceptance is known, but a checked persisted receipt could not be read.
    #[classify(category = "internal", code = "API:WORKFLOW_START_RECEIPT_UNAVAILABLE")]
    #[error("Accepted workflow start receipt is unavailable")]
    WorkflowStartReceiptUnavailable {
        /// Known accepted execution in the authenticated scope.
        execution_id: String,
    },

    /// The original start transaction may have committed.
    #[classify(category = "internal", code = "API:WORKFLOW_START_INDETERMINATE")]
    #[error("Workflow start outcome is indeterminate")]
    WorkflowStartIndeterminate {
        /// Execution identity allocated for the original attempt.
        execution_id: String,
        /// Original workflow selection.
        workflow_id: String,
        /// Original immutable contract bundle identity.
        bundle_id: String,
    },

    /// Session has expired — caller must re-authenticate (401).
    #[classify(category = "authentication", code = "API:SESSION_EXPIRED")]
    #[error("Session expired")]
    SessionExpired,

    /// Multi-factor authentication step required before proceeding (401).
    #[classify(category = "authentication", code = "API:MFA_REQUIRED")]
    #[error("MFA verification required")]
    MfaRequired,

    /// Caller's role is insufficient for the requested operation (403).
    #[classify(category = "authorization", code = "API:INSUFFICIENT_ROLE")]
    #[error("Insufficient role: {required_role} required, current role {current_role}")]
    InsufficientRole {
        /// Role that the operation demands.
        required_role: String,
        /// Role that the caller actually holds.
        current_role: String,
    },

    /// Tenant quota exceeded (403).
    #[classify(category = "authorization", code = "API:QUOTA_EXCEEDED")]
    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    /// Optimistic-concurrency version mismatch (409).
    #[classify(category = "conflict", code = "API:VERSION_MISMATCH")]
    #[error("Version mismatch: {0}")]
    VersionMismatch(String),

    /// Resource existed but has been permanently removed (410).
    #[classify(category = "not_found", code = "API:GONE")]
    #[error("Resource gone: {0}")]
    Gone(String),

    /// Semantically invalid entity that cannot be processed (422).
    #[classify(category = "validation", code = "API:UNPROCESSABLE")]
    #[error("Unprocessable entity: {0}")]
    Unprocessable(String),

    /// Account is locked (423).
    #[classify(category = "authorization", code = "API:LOCKED")]
    #[error("Account locked: {0}")]
    AccountLocked(String),

    /// Upstream/external service returned an error (502).
    #[classify(category = "external", code = "API:UPSTREAM_ERROR")]
    #[error("Upstream error: {0}")]
    UpstreamError(String),

    /// Storage subsystem is full (507).
    #[classify(category = "internal", code = "API:STORAGE_FULL")]
    #[error("Storage full")]
    StorageFull,

    /// The endpoint is documented but the handler is not yet implemented (501).
    ///
    /// Used by class-(c) stub handlers under stub endpoint policy
    /// so the runtime status code matches the `responses(501)` annotation
    /// in the OpenAPI document. Migrating from `Internal("not implemented")`
    /// (500) to this variant keeps the stub-honesty contract self-consistent.
    #[classify(category = "internal", code = "API:NOT_IMPLEMENTED")]
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// The credential holds no poisoned provider-operation claim to adjudicate (409).
    ///
    /// Distinct from the generic [`Self::Conflict`]: the caller is authorized
    /// and the credential is not in the state the command names, and a client
    /// has to tell that apart from a contradictory decision before it can act.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_RECONCILIATION_NOT_REQUIRED",
        retryable = false
    )]
    #[error("Credential has no provider-operation claim requiring reconciliation")]
    CredentialReconciliationNotRequired,

    /// The provider-operation claim already records a different `(evidence, decision)`
    /// pair than the one submitted (409).
    ///
    /// A caller that repeats its own request does not get this: an identical
    /// pair is a no-op success. This means two operator observations disagree
    /// about the same claim. The recorded pair is carried so the problem
    /// document can name what is on record: a client holding its original
    /// evidence can confirm it disagreed rather than wonder.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_RECONCILIATION_CONFLICT",
        retryable = false
    )]
    #[error(
        "Credential provider-operation claim already records a different reconciliation decision"
    )]
    CredentialReconciliationConflict {
        /// SHA-256 of the recorded evidence, lowercase hex.
        recorded_digest: String,
        /// The recorded decision in its wire spelling.
        recorded_decision: String,
    },

    /// Persisted credential state uses a shape this runtime refuses to read
    /// (409). The refusal is permanent for this runtime build and exposes no
    /// stored envelope values.
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_STATE_REFUSED",
        retryable = false
    )]
    #[error("Stored credential state is not compatible with this runtime")]
    CredentialStateRefused,

    /// A durable provider operation gate blocks the requested command (409).
    #[classify(
        category = "conflict",
        code = "API:CREDENTIAL_OPERATION_BLOCKED",
        retryable = false
    )]
    #[error("Credential operation is blocked by {operation}")]
    CredentialOperationBlocked {
        /// Stable, secret-free operation spelling.
        operation: &'static str,
    },
}

/// Project a [`nebula_tenancy::TenancyError`] (raised when a request's
/// `TenantContext` is turned into a port `Scope`) onto the HTTP surface.
///
/// Both variants are deliberately coarse — the tenancy layer never
/// discloses *why* scope resolution failed in a way that lets a caller
/// probe the tenant graph (the same existence-non-disclosure rule the
/// scoped decorators enforce for row access, spec §6.1):
///
/// - `MissingWorkspace` → **404**. Every workspace-scoped resource lives
///   under `/orgs/{org}/workspaces/{ws}/…`; reaching a scoped handler
///   with no workspace binding is a routing-invariant violation, surfaced
///   as the same opaque `not found` the tenancy middleware already uses
///   for an unresolvable workspace segment (never "you lack a workspace",
///   which would confirm the org exists).
/// - `Unauthorized` → **403**, coarse on purpose: it never reveals which
///   half (org vs workspace) mismatched.
impl From<nebula_tenancy::TenancyError> for ApiError {
    fn from(err: nebula_tenancy::TenancyError) -> Self {
        use nebula_tenancy::TenancyError as Te;
        match err {
            Te::MissingWorkspace => Self::NotFound("not found".to_string()),
            Te::Unauthorized => {
                Self::Forbidden("not authorized for the requested tenant".to_string())
            },
        }
    }
}

impl ApiError {
    /// Create validation error without field-level details.
    pub fn validation_message(detail: impl Into<String>) -> Self {
        Self::Validation {
            detail: detail.into(),
            errors: Vec::new(),
        }
    }

    /// Convert this error to its RFC 9457 representation.
    pub fn to_problem_details(&self) -> (StatusCode, ProblemDetails) {
        match self {
            Self::Validation { detail, errors } => validation_problem(detail, errors),
            Self::Unauthorized(message) => standard_problem(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Unauthorized",
                Some(message),
            ),
            Self::Forbidden(message) => standard_problem(
                StatusCode::FORBIDDEN,
                "forbidden",
                "Forbidden",
                Some(message),
            ),
            Self::NotFound(message) => standard_problem(
                StatusCode::NOT_FOUND,
                "not-found",
                "Not Found",
                Some(message),
            ),
            Self::Conflict(message) => conflict_problem("conflict", "Conflict", message),
            Self::AlreadyExists(message) => {
                conflict_problem("already-exists", "Already Exists", message)
            },
            Self::StartConflict => start_conflict_problem(),
            Self::VersionExhausted(message) => {
                conflict_problem("version-exhausted", "Version Exhausted", message)
            },
            Self::OutcomeUnknown(message) => {
                conflict_problem("outcome-unknown", "Operation Outcome Unknown", message)
            },
            Self::CredentialReauthRequired => {
                credential_problem(CredentialProblem::ReauthenticationRequired)
            },
            Self::CredentialRefreshNotAppliedNever => {
                credential_problem(CredentialProblem::RefreshNotApplied)
            },
            Self::CredentialRefreshNotAppliedAfter { .. } => {
                credential_problem(CredentialProblem::RefreshRetryDelayed)
            },
            Self::CredentialRefreshReconciliationRequired => {
                credential_problem(CredentialProblem::RefreshReconciliationRequired)
            },
            Self::CredentialAcquisitionReconciliationRequired => {
                credential_problem(CredentialProblem::AcquisitionReconciliationRequired)
            },
            Self::CredentialStateRefused => credential_problem(CredentialProblem::StateRefused),
            Self::CredentialOperationBlocked { operation } => {
                credential_problem(CredentialProblem::OperationBlocked { operation })
            },
            Self::CredentialRevokeReconciliationRequired => {
                credential_problem(CredentialProblem::RevokeReconciliationRequired)
            },
            Self::CredentialReconciliationNotRequired => {
                credential_problem(CredentialProblem::ReconciliationNotRequired)
            },
            Self::CredentialReconciliationConflict {
                recorded_digest,
                recorded_decision,
            } => credential_problem(CredentialProblem::ReconciliationConflict {
                recorded_digest: recorded_digest.clone(),
                recorded_decision: recorded_decision.clone(),
            }),
            Self::RateLimitExceeded => standard_problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rate-limit",
                "Rate Limit Exceeded",
                None,
            ),
            Self::Internal(message) => internal_problem(message),
            Self::ServiceUnavailable(message) => standard_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "service-unavailable",
                "Service Unavailable",
                Some(message),
            ),
            Self::Storage(error) => storage_problem(error),
            Self::InvalidWorkflowDefinition { detail, errors } => {
                invalid_workflow_problem(detail, activation_field_errors(errors))
            },
            Self::WorkflowCompilation(error) => invalid_workflow_problem(
                "The workflow cannot be compiled for the selected worker release.",
                error
                    .diagnostics()
                    .iter()
                    .map(ValidationFieldError::from)
                    .collect(),
            ),
            Self::WorkflowPublicationIndeterminate {
                workflow_id,
                version,
                workflow_revision,
            } => workflow_publication_problem(workflow_id, *version, workflow_revision),
            Self::WorkflowStartRejected { reason } => workflow_start_rejected_problem(reason),
            Self::WorkflowStartReceiptUnavailable { execution_id } => {
                workflow_start_receipt_problem(execution_id)
            },
            Self::WorkflowStartIndeterminate {
                execution_id,
                workflow_id,
                bundle_id,
            } => workflow_start_indeterminate_problem(execution_id, workflow_id, bundle_id),
            Self::SessionExpired => standard_problem(
                StatusCode::UNAUTHORIZED,
                "session-expired",
                "Session Expired",
                None,
            ),
            Self::MfaRequired => standard_problem(
                StatusCode::UNAUTHORIZED,
                "mfa-required",
                "MFA Required",
                None,
            ),
            Self::InsufficientRole {
                required_role,
                current_role,
            } => insufficient_role_problem(required_role, current_role),
            Self::QuotaExceeded(message) => standard_problem(
                StatusCode::FORBIDDEN,
                "quota-exceeded",
                "Quota Exceeded",
                Some(message),
            ),
            Self::VersionMismatch(message) => {
                conflict_problem("version-mismatch", "Version Mismatch", message)
            },
            Self::Gone(message) => {
                standard_problem(StatusCode::GONE, "gone", "Gone", Some(message))
            },
            Self::Unprocessable(message) => standard_problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unprocessable",
                "Unprocessable Entity",
                Some(message),
            ),
            Self::AccountLocked(message) => standard_problem(
                StatusCode::LOCKED,
                "account-locked",
                "Account Locked",
                Some(message),
            ),
            Self::UpstreamError(message) => standard_problem(
                StatusCode::BAD_GATEWAY,
                "upstream-error",
                "Upstream Error",
                Some(message),
            ),
            Self::StorageFull => standard_problem(
                StatusCode::INSUFFICIENT_STORAGE,
                "storage-full",
                "Storage Full",
                None,
            ),
            Self::NotImplemented(reason) => standard_problem(
                StatusCode::NOT_IMPLEMENTED,
                "not-implemented",
                "Not Implemented",
                Some(reason),
            ),
        }
    }
}

fn standard_problem(
    status: StatusCode,
    problem_type: &'static str,
    title: &'static str,
    detail: Option<&str>,
) -> (StatusCode, ProblemDetails) {
    let problem = ProblemDetails::new(
        format!("https://nebula.dev/problems/{problem_type}"),
        title,
        status,
    );
    let problem = if let Some(value) = detail {
        problem.with_detail(value)
    } else {
        problem
    };
    (status, problem)
}

fn conflict_problem(
    problem_type: &'static str,
    title: &'static str,
    detail: &str,
) -> (StatusCode, ProblemDetails) {
    standard_problem(StatusCode::CONFLICT, problem_type, title, Some(detail))
}

fn validation_problem(
    detail: &str,
    errors: &[ValidationFieldError],
) -> (StatusCode, ProblemDetails) {
    let (status, problem) = standard_problem(
        StatusCode::BAD_REQUEST,
        "validation-error",
        "Validation Error",
        Some(detail),
    );
    (status, problem.with_errors(errors.to_vec()))
}

fn start_conflict_problem() -> (StatusCode, ProblemDetails) {
    let (status, problem) = standard_problem(
        StatusCode::CONFLICT,
        "start-conflict",
        "Start Key Conflict",
        Some(
            "This start key was already accepted for a different request. \
             Retry with the original request body, or use a new key.",
        ),
    );
    (
        status,
        problem.with_extensions(serde_json::json!({"code": "operation_mismatch"})),
    )
}

enum CredentialProblem {
    ReauthenticationRequired,
    RefreshNotApplied,
    RefreshRetryDelayed,
    RefreshReconciliationRequired,
    AcquisitionReconciliationRequired,
    StateRefused,
    RevokeReconciliationRequired,
    ReconciliationNotRequired,
    /// The pair the comparison refused against, in wire spelling — named by
    /// the problem document's extensions so a client holding its original
    /// evidence can confirm what is on record.
    ReconciliationConflict {
        recorded_digest: String,
        recorded_decision: String,
    },
    OperationBlocked {
        operation: &'static str,
    },
}

fn credential_problem(error: CredentialProblem) -> (StatusCode, ProblemDetails) {
    let (problem_type, title, detail) = match &error {
        CredentialProblem::ReauthenticationRequired => (
            "credential-reauth-required",
            "Credential Reauthentication Required",
            "Reconnect the integration credential before retrying this operation.",
        ),
        CredentialProblem::RefreshNotApplied => (
            "credential-refresh-not-applied",
            "Credential Refresh Not Applied",
            "The credential refresh was not applied for the current credential state.",
        ),
        CredentialProblem::RefreshRetryDelayed => (
            "credential-refresh-not-applied",
            "Credential Refresh Not Applied",
            "The credential refresh was not applied. Retry only after the Retry-After delay.",
        ),
        CredentialProblem::RefreshReconciliationRequired => (
            "credential-refresh-reconciliation-required",
            "Credential Refresh Reconciliation Required",
            "The refresh outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile or reconnect the integration credential.",
        ),
        CredentialProblem::AcquisitionReconciliationRequired => (
            "credential-acquisition-reconciliation-required",
            "Credential Acquisition Reconciliation Required",
            "Credential acquisition completed, but durable local finalization definitely failed. Do not retry automatically; reconcile credential state or restart authorization deliberately.",
        ),
        CredentialProblem::StateRefused => (
            "credential-state-refused",
            "Credential State Refused",
            "The stored credential state is not compatible with this runtime. Update the runtime or repair the credential state before retrying.",
        ),
        CredentialProblem::RevokeReconciliationRequired => (
            "credential-revoke-reconciliation-required",
            "Credential Revoke Reconciliation Required",
            "The revoke outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile credential state.",
        ),
        CredentialProblem::ReconciliationNotRequired => (
            "credential-reconciliation-not-required",
            "Credential Reconciliation Not Required",
            "The credential has no poisoned provider-operation claim to adjudicate. A repeated reconciliation of a decision already on record is reported as success instead.",
        ),
        CredentialProblem::ReconciliationConflict { .. } => (
            "credential-reconciliation-conflict",
            "Credential Reconciliation Conflict",
            "The provider-operation claim already records a different evidence and decision pair. Repeating an identical request is a no-op success; the evidence_digest and recorded_decision extensions name the recorded pair this request disagreed with.",
        ),
        CredentialProblem::OperationBlocked { .. } => (
            "credential-operation-blocked",
            "Credential Operation Blocked",
            "A provider operation is in flight or requires reconciliation before this command can proceed.",
        ),
    };
    let (status, problem) = conflict_problem(problem_type, title, detail);
    match error {
        CredentialProblem::ReconciliationConflict {
            recorded_digest,
            recorded_decision,
        } => (
            status,
            problem.with_extensions(serde_json::json!({
                "evidence_digest": recorded_digest,
                "recorded_decision": recorded_decision,
            })),
        ),
        CredentialProblem::OperationBlocked { operation } => (
            status,
            problem.with_extensions(serde_json::json!({ "operation": operation })),
        ),
        _ => (status, problem),
    }
}

fn internal_problem(message: &str) -> (StatusCode, ProblemDetails) {
    tracing::error!(error = message, "internal API error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        ProblemDetails::new(
            "about:blank",
            "Internal Server Error",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    )
}

fn storage_problem(error: &nebula_storage_port::StorageError) -> (StatusCode, ProblemDetails) {
    tracing::error!(error = %error, "storage error at API boundary");
    standard_problem(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage-error",
        "Internal Server Error",
        None,
    )
}

fn invalid_workflow_problem(
    detail: &str,
    errors: Vec<ValidationFieldError>,
) -> (StatusCode, ProblemDetails) {
    let (status, problem) = standard_problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid-workflow-definition",
        "Invalid Workflow Definition",
        Some(detail),
    );
    (status, problem.with_errors(errors))
}

fn workflow_publication_problem(
    workflow_id: &str,
    version: u32,
    workflow_revision: &str,
) -> (StatusCode, ProblemDetails) {
    let (status, problem) = standard_problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "workflow-publication-indeterminate",
        "Workflow Publication Indeterminate",
        Some(
            "Publication may have committed. Reconcile the original version before submitting another activation.",
        ),
    );
    (
        status,
        problem.with_extensions(serde_json::json!({
            "workflow_id": workflow_id,
            "version": version,
            "workflow_revision": workflow_revision,
        })),
    )
}

fn workflow_start_rejected_problem(reason: &str) -> (StatusCode, ProblemDetails) {
    standard_problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "workflow-start-rejected",
        "Workflow Start Rejected",
        Some(reason),
    )
}

fn workflow_start_receipt_problem(execution_id: &str) -> (StatusCode, ProblemDetails) {
    let (status, problem) = standard_problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "workflow-start-receipt-unavailable",
        "Workflow Start Receipt Unavailable",
        Some(
            "The start was accepted. Read this execution to recover its receipt; submitting another unkeyed start creates a different execution.",
        ),
    );
    (
        status,
        problem.with_extensions(serde_json::json!({
            "execution_id": execution_id,
            "accepted": true,
        })),
    )
}

fn workflow_start_indeterminate_problem(
    execution_id: &str,
    workflow_id: &str,
    bundle_id: &str,
) -> (StatusCode, ProblemDetails) {
    let (status, problem) = standard_problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "workflow-start-indeterminate",
        "Workflow Start Indeterminate",
        Some(
            "The start may have committed. Reconcile the original execution before submitting another start.",
        ),
    );
    (
        status,
        problem.with_extensions(serde_json::json!({
            "execution_id": execution_id,
            "workflow_id": workflow_id,
            "bundle_id": bundle_id,
        })),
    )
}

fn insufficient_role_problem(
    required_role: &str,
    current_role: &str,
) -> (StatusCode, ProblemDetails) {
    let detail = format!("{required_role} required, current role {current_role}");
    standard_problem(
        StatusCode::FORBIDDEN,
        "insufficient-role",
        "Insufficient Role",
        Some(&detail),
    )
}

/// Render typed workflow rejections as activation problem-details entries.
///
/// Ordering is canonical, so the same invalid workflow always produces the
/// same response body: a client diffing two responses sees only real changes,
/// and a snapshot test is meaningful.
///
/// The JSON Pointer comes from each diagnostic's own `path`, which is why the
/// API no longer keeps a second `WorkflowError` → pointer table: two mappings
/// for one fact drift, and the crate that raises the rejection is the one that
/// knows which element it is about.
fn activation_field_errors(errors: &[nebula_workflow::WorkflowError]) -> Vec<ValidationFieldError> {
    use nebula_error::ActivationDiagnostics;

    let diagnostics = nebula_error::canonical_diagnostics(
        errors
            .iter()
            .flat_map(nebula_workflow::WorkflowError::activation_diagnostics)
            .collect(),
    );
    diagnostics.iter().map(ValidationFieldError::from).collect()
}

impl From<nebula_engine::WorkflowActivationError> for ApiError {
    fn from(error: nebula_engine::WorkflowActivationError) -> Self {
        use nebula_engine::WorkflowActivationError;
        match error {
            WorkflowActivationError::MissingWorkflow => Self::NotFound("Workflow not found".into()),
            WorkflowActivationError::CasConflict => {
                Self::Conflict("Workflow version conflict".into())
            },
            WorkflowActivationError::InvalidDefinition => {
                Self::validation_message("Invalid workflow definition")
            },
            WorkflowActivationError::Compilation(error) => Self::WorkflowCompilation(error),
            WorkflowActivationError::UnsupportedRecordedSemantics => {
                Self::validation_message("Workflow uses runtime semantics that cannot be recorded")
            },
            WorkflowActivationError::PublicationIndeterminate(attempt) => {
                Self::WorkflowPublicationIndeterminate {
                    workflow_id: attempt.workflow_id().to_string(),
                    version: attempt.number(),
                    workflow_revision: attempt.activation().workflow_version_id().to_string(),
                }
            },
            WorkflowActivationError::RevisionNotAdmitted => {
                Self::Conflict("Workflow revisions are not admitted".into())
            },
            _ => Self::ServiceUnavailable("Workflow activation is unavailable".into()),
        }
    }
}

impl From<nebula_engine::WorkflowStartError> for ApiError {
    fn from(error: nebula_engine::WorkflowStartError) -> Self {
        Self::from(&error)
    }
}

impl From<&nebula_engine::WorkflowStartError> for ApiError {
    fn from(error: &nebula_engine::WorkflowStartError) -> Self {
        use nebula_engine::WorkflowStartError;
        match error {
            WorkflowStartError::InvalidScope
            | WorkflowStartError::InvalidKey
            | WorkflowStartError::InvalidInput => {
                Self::validation_message("Invalid workflow start request")
            },
            WorkflowStartError::MissingWorkflow => Self::NotFound("Workflow not found".into()),
            WorkflowStartError::WorkflowNotActivated => Self::WorkflowStartRejected {
                reason: "The workflow has no exact activation.",
            },
            WorkflowStartError::UnsupportedBindings => Self::WorkflowStartRejected {
                reason: "The workflow requires bindings that cannot be admitted.",
            },
            WorkflowStartError::BindingResolution(source) => match source {
                nebula_engine::BindingResolutionError::Unavailable => {
                    Self::ServiceUnavailable("Workflow binding resolution is unavailable".into())
                },
                _ => Self::WorkflowStartRejected {
                    reason: "The workflow bindings could not be resolved.",
                },
            },
            WorkflowStartError::UnsupportedRecordedSemantics => Self::WorkflowStartRejected {
                reason: "The workflow contains unsupported runtime semantics.",
            },
            WorkflowStartError::FingerprintMismatch => Self::StartConflict,
            WorkflowStartError::RevisionNotAdmitted(_) => {
                Self::Conflict("Workflow revisions are not admitted".into())
            },
            WorkflowStartError::RevisionUnavailable(source) => match source.as_ref() {
                nebula_engine::PlanFlavorRevisionBridgeError::Catalog {
                    source:
                        nebula_storage_port::dto::RevisionCatalogError::CorruptRecord { .. }
                        | nebula_storage_port::dto::RevisionCatalogError::UnsupportedRecordFormat {
                            ..
                        }
                        | nebula_storage_port::dto::RevisionCatalogError::EmptyRecord
                        | nebula_storage_port::dto::RevisionCatalogError::RecordTooLarge { .. }
                        | nebula_storage_port::dto::RevisionCatalogError::RecordNestingTooDeep { .. }
                        | nebula_storage_port::dto::RevisionCatalogError::RecordStringTooLarge { .. }
                        | nebula_storage_port::dto::RevisionCatalogError::RecordStringBudgetExceeded {
                            ..
                        }
                        | nebula_storage_port::dto::RevisionCatalogError::RecordCollectionBudgetExceeded {
                            ..
                        },
                    ..
                } => Self::Internal("Stored workflow revisions are inconsistent".into()),
                nebula_engine::PlanFlavorRevisionBridgeError::Catalog { .. } => {
                    Self::ServiceUnavailable("Exact workflow revisions are unavailable".into())
                },
                _ => Self::Internal("Stored workflow revisions are inconsistent".into()),
            },
            WorkflowStartError::ReceiptUnavailable { execution_id } => {
                Self::WorkflowStartReceiptUnavailable {
                    execution_id: execution_id.to_string(),
                }
            },
            WorkflowStartError::MaterializationIndeterminate(attempt) => {
                Self::WorkflowStartIndeterminate {
                    execution_id: attempt.execution_id().to_string(),
                    workflow_id: attempt.workflow_id().to_string(),
                    bundle_id: attempt.bundle_id().to_string(),
                }
            },
            WorkflowStartError::InvalidActivation
            | WorkflowStartError::InvalidReceipt
            | WorkflowStartError::MaterializationRejected => {
                Self::Internal("Stored workflow start contract is inconsistent".into())
            },
            _ => Self::ServiceUnavailable("Workflow start is unavailable".into()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, problem) = self.to_problem_details();
        let retry_after_secs = match &self {
            Self::CredentialRefreshNotAppliedAfter { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        };

        // Log error
        tracing::error!(
            error = ?self,
            status = status.as_u16(),
            "API error occurred"
        );

        // RFC 9457: Content-Type MUST be application/problem+json
        let mut response = (status, Json(problem)).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/problem+json"),
        );
        if let Some(retry_after_secs) = retry_after_secs
            && let Ok(value) = retry_after_secs.get().to_string().parse()
        {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        response
    }
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests;
