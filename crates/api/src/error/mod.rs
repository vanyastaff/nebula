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
}

/// api ↔ legacy-storage seam: classify a `nebula_storage::StorageError`
/// onto the HTTP contract.
///
/// `ApiError::Storage` bridges the **spec-16 port** error
/// ([`nebula_storage_port::StorageError`]) — the canonical surface every
/// port-migrated path returns. The resource-catalog path is the one
/// surface still on the **retained legacy** `nebula_storage::repos::ResourceRepo`
/// (deliberately not migrated to the row-model port — storage port migration), which
/// returns the legacy `nebula_storage::StorageError`. This is the seam
/// adapter for that single path: a direct legacy→`ApiError` classification
/// (NotFound → 404, Conflict/Duplicate → 409, everything else → opaque
/// 500 with no internal-detail leak per no secret echo) — **not** a
/// back-compat re-export of the deleted legacy surface, and **not** a
/// re-route through the port error type.
impl From<nebula_storage::StorageError> for ApiError {
    fn from(err: nebula_storage::StorageError) -> Self {
        use nebula_storage::StorageError as Se;
        match err {
            Se::NotFound { entity, id } => Self::NotFound(format!("{entity} not found: {id}")),
            Se::Conflict {
                entity,
                id,
                expected,
                actual,
            } => Self::Conflict(format!(
                "{entity} {id}: version conflict (expected {expected}, actual {actual}); \
                 re-read and retry"
            )),
            Se::Duplicate { entity, detail } => {
                Self::Conflict(format!("duplicate {entity}: {detail}"))
            },
            // Lease / timeout / serialization / connection / configuration /
            // internal are genuine backend faults — the opaque
            // `Self::Storage` arm (still a 500 with no internal detail
            // leaked to the client per no secret echo). Mapped through the
            // port `StorageError` so the variant is preserved end-to-end
            // (`map_resource_create_storage_error`'s contract: a
            // non-caller fault stays the opaque `Storage` variant, never
            // a catch-all `Internal`).
            other => Self::Storage(storage_fault_to_port(other)),
        }
    }
}

/// Map a non-caller [`nebula_storage::StorageError`] fault onto the
/// equivalent port [`nebula_storage_port::StorageError`] so
/// [`ApiError::Storage`] carries the original failure class.
///
/// Only the genuine-backend-fault variants reach this — caller-conflict
/// variants (`NotFound` / `Conflict` / `Duplicate`) are handled by the
/// `From` arms above and never get here. The message text is
/// store-authored (no submitted payload), so it is safe to carry; the
/// HTTP surface still collapses every `Storage` to a detail-free 500.
fn storage_fault_to_port(err: nebula_storage::StorageError) -> nebula_storage_port::StorageError {
    use nebula_storage::StorageError as Se;
    use nebula_storage_port::StorageError as Pe;
    match err {
        Se::LeaseUnavailable { entity, id } => Pe::LeaseUnavailable { entity, id },
        Se::Timeout {
            operation,
            duration,
        } => Pe::Timeout {
            operation,
            duration,
        },
        Se::Serialization(detail) => Pe::Serialization(detail),
        Se::Connection(detail) => Pe::Connection(detail),
        Se::Configuration(detail) => Pe::Configuration(detail),
        // `Se::Internal` and any future non-caller variant fold into the
        // port `Internal` — fail-closed, never silently dropped.
        other => Pe::Internal(other.to_string()),
    }
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
            Self::CredentialRevokeReconciliationRequired => {
                credential_problem(CredentialProblem::RevokeReconciliationRequired)
            },
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
    RevokeReconciliationRequired,
}

fn credential_problem(error: CredentialProblem) -> (StatusCode, ProblemDetails) {
    let (problem_type, title, detail) = match error {
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
        CredentialProblem::RevokeReconciliationRequired => (
            "credential-revoke-reconciliation-required",
            "Credential Revoke Reconciliation Required",
            "The revoke outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile credential state.",
        ),
    };
    conflict_problem(problem_type, title, detail)
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
            WorkflowActivationError::UnresolvedBindings => Self::validation_message(
                "Workflow contains unresolved resource or credential bindings",
            ),
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
                        | nebula_storage_port::dto::RevisionCatalogError::EmptyRecord,
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
mod tests {
    #[test]
    fn emitter_admission_source_maps_to_the_same_transport_error() {
        let error = nebula_action::ActionError::fatal_from(
            nebula_engine::WorkflowStartError::MissingWorkflow,
        );
        let source = std::error::Error::source(&error).expect("typed emitter error retains source");
        let admission = source
            .downcast_ref::<nebula_engine::WorkflowStartError>()
            .expect("source is the original runtime admission error");
        assert_eq!(
            ApiError::from(admission).to_problem_details().0,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn accepted_receipt_failure_retains_identity_without_a_retry_hint() {
        let execution_id = nebula_core::ExecutionId::new();
        let error =
            ApiError::from(&nebula_engine::WorkflowStartError::ReceiptUnavailable { execution_id });
        let (status, problem) = error.to_problem_details();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let encoded = serde_json::to_value(problem).unwrap();
        assert_eq!(encoded["execution_id"], execution_id.to_string());
        assert_eq!(encoded["accepted"], true);
        assert!(
            error
                .into_response()
                .headers()
                .get(header::RETRY_AFTER)
                .is_none()
        );
    }

    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };

    use super::*;
    use nebula_validator::foundation::ValidationError;

    #[test]
    fn validation_error_conversion_preserves_code_and_pointer() {
        let err = ValidationError::new("min_length", "Must be at least 3 characters")
            .with_field("profile.name");

        let api_error = ApiError::from(err);
        let (status, problem) = api_error.to_problem_details();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let errors = problem.errors.expect("validation errors must be present");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "min_length");
        assert_eq!(errors[0].pointer.as_deref(), Some("/profile/name"));
    }

    #[test]
    fn nested_validation_error_conversion_keeps_nested_entries() {
        let err =
            ValidationError::new("object_invalid", "Object validation failed").with_nested(vec![
                ValidationError::new("required", "Field is required").with_pointer("/email"),
            ]);

        let api_error = ApiError::from(err);
        let (status, problem) = api_error.to_problem_details();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let errors = problem.errors.expect("validation errors must be present");
        assert!(errors.iter().any(|e| e.code == "object_invalid"));
        assert!(
            errors
                .iter()
                .any(|e| e.code == "required" && e.pointer.as_deref() == Some("/email"))
        );
    }

    #[test]
    fn invalid_workflow_definition_node_error_produces_node_pointer() {
        use nebula_core::node_key;
        use nebula_workflow::WorkflowError;

        let node = node_key!("step_a");
        let api_error = ApiError::InvalidWorkflowDefinition {
            detail: "1 error(s)".to_string(),
            errors: vec![WorkflowError::DuplicateNodeKey(node)],
        };
        let (status, problem) = api_error.to_problem_details();

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let errors = problem.errors.expect("errors must be present");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .path
                .as_deref()
                .is_some_and(|path| path.starts_with("/nodes/")),
            "DuplicateNodeKey must produce a /nodes/<key> pointer, got: {:?}",
            errors[0].pointer
        );
        assert_eq!(errors[0].path.as_deref(), Some("/nodes/step_a"));
    }

    #[test]
    fn invalid_workflow_definition_structural_error_points_at_the_offending_section() {
        use nebula_workflow::WorkflowError;

        let api_error = ApiError::InvalidWorkflowDefinition {
            detail: "1 error(s)".to_string(),
            errors: vec![WorkflowError::CycleDetected],
        };
        let (status, problem) = api_error.to_problem_details();

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let errors = problem.errors.expect("errors must be present");
        assert_eq!(errors.len(), 1);

        // Structural rejections used to collapse to the RFC 6901 root pointer
        // (the empty string). Activation diagnostics require a non-empty path on every
        // diagnostic, and a cycle is always a property of the connections, so
        // pointing there is both required and strictly more useful than
        // pointing at the whole document.
        assert_eq!(errors[0].path.as_deref(), Some("/connections"));
        assert_eq!(errors[0].code, "WORKFLOW:CYCLE_DETECTED");
        assert_eq!(
            errors[0].expected.as_deref(),
            Some("an acyclic graph"),
            "the contract that was required travels as its own field"
        );
        assert_eq!(
            errors[0].actual.as_deref(),
            Some("a graph containing a cycle")
        );
        assert!(
            errors[0]
                .remediation
                .as_deref()
                .is_some_and(|text| text.contains("cycle")),
            "an author is told what to change, not just what is wrong"
        );
    }

    #[test]
    fn invalid_workflow_definition_connection_error_produces_connection_pointer() {
        use nebula_core::node_key;
        use nebula_workflow::WorkflowError;

        let from = node_key!("a");
        let to = node_key!("b");
        let api_error = ApiError::InvalidWorkflowDefinition {
            detail: "1 error(s)".to_string(),
            errors: vec![WorkflowError::DuplicateConnection { from, to }],
        };
        let (status, problem) = api_error.to_problem_details();

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let errors = problem.errors.expect("errors must be present");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].path.as_deref(),
            Some("/connections/a/b"),
            "a connection rejection names both endpoints it wires"
        );
        assert_eq!(
            errors[0].pointer, None,
            "an activation diagnostic reports a logical path, never a JSON Pointer it \
             could not resolve against an array of connections"
        );
    }

    #[test]
    fn refresh_not_applied_never_is_a_fixed_409_without_retry_after() {
        use nebula_error::Classify;

        let error = ApiError::CredentialRefreshNotAppliedNever;
        let (status, problem) = error.to_problem_details();

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
        assert_eq!(
            error.code().as_str(),
            "API:CREDENTIAL_REFRESH_NOT_APPLIED_NEVER"
        );
        assert!(!error.is_retryable());
        assert_eq!(error.retry_hint(), None);
        assert_eq!(
            problem.type_uri,
            "https://nebula.dev/problems/credential-refresh-not-applied"
        );
        assert_eq!(problem.title, "Credential Refresh Not Applied");
        assert_eq!(
            problem.detail.as_deref(),
            Some("The credential refresh was not applied for the current credential state.")
        );
        assert!(
            !problem
                .detail
                .as_deref()
                .is_some_and(|detail| detail.to_ascii_lowercase().contains("retry")),
            "Never must not advise the client to retry"
        );

        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
    }

    #[test]
    fn refresh_not_applied_after_is_a_fixed_409_with_retry_after() {
        use nebula_error::Classify;

        let error = ApiError::CredentialRefreshNotAppliedAfter {
            retry_after_secs: NonZeroU64::new(17).expect("test delay is non-zero"),
        };
        let (status, problem) = error.to_problem_details();

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
        assert_eq!(
            error.code().as_str(),
            "API:CREDENTIAL_REFRESH_NOT_APPLIED_AFTER"
        );
        assert!(error.is_retryable());
        assert_eq!(error.retry_hint(), None);
        assert_eq!(
            problem.type_uri,
            "https://nebula.dev/problems/credential-refresh-not-applied"
        );
        assert_eq!(problem.title, "Credential Refresh Not Applied");
        assert_eq!(
            problem.detail.as_deref(),
            Some("The credential refresh was not applied. Retry only after the Retry-After delay.")
        );

        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers().get(header::RETRY_AFTER),
            Some(&axum::http::HeaderValue::from_static("17"))
        );
    }

    #[test]
    fn refresh_reconciliation_required_is_a_fixed_non_retryable_409() {
        use nebula_error::Classify;

        let error = ApiError::CredentialRefreshReconciliationRequired;
        let (status, problem) = error.to_problem_details();

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
        assert_eq!(
            error.code().as_str(),
            "API:CREDENTIAL_REFRESH_RECONCILIATION_REQUIRED"
        );
        assert!(!error.is_retryable());
        assert_eq!(error.retry_hint(), None);
        assert_eq!(
            problem.type_uri,
            "https://nebula.dev/problems/credential-refresh-reconciliation-required"
        );
        assert_eq!(problem.title, "Credential Refresh Reconciliation Required");
        assert_eq!(
            problem.detail.as_deref(),
            Some(
                "The refresh outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile or reconnect the integration credential."
            )
        );

        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
    }

    #[test]
    fn revoke_reconciliation_required_is_a_fixed_non_retryable_409() {
        use nebula_error::Classify;

        let error = ApiError::CredentialRevokeReconciliationRequired;
        let (status, problem) = error.to_problem_details();

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
        assert_eq!(
            error.code().as_str(),
            "API:CREDENTIAL_REVOKE_RECONCILIATION_REQUIRED"
        );
        assert!(!error.is_retryable());
        assert_eq!(error.retry_hint(), None);
        assert_eq!(
            problem.type_uri,
            "https://nebula.dev/problems/credential-revoke-reconciliation-required"
        );
        assert_eq!(problem.title, "Credential Revoke Reconciliation Required");
        assert_eq!(
            problem.detail.as_deref(),
            Some(
                "The revoke outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile credential state."
            )
        );

        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
    }
}
