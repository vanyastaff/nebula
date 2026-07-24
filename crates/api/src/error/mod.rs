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

use crate::error::classify::workflow_error_pointer;

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
    /// parse/format errors. Returned by `activate_workflow` and by the
    /// shift-left dispatch gate (`execute_workflow` / `start_execution`, via
    /// `validate_for_dispatch`) after the stored definition fails structural
    /// DAG/schema checks (ROADMAP M3.6).
    #[classify(category = "validation", code = "API:INVALID_WORKFLOW")]
    #[error("Invalid workflow definition: {detail}")]
    InvalidWorkflowDefinition {
        /// Human-readable summary of all validation failures.
        detail: String,
        /// One entry per `WorkflowError` returned by `validate_workflow`.
        /// Carrying the typed errors allows `to_problem_details` to produce
        /// real RFC 6901 JSON Pointers rather than synthetic positional ones.
        errors: Vec<nebula_workflow::WorkflowError>,
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

    /// Convert to ProblemDetails
    pub fn to_problem_details(&self) -> (StatusCode, ProblemDetails) {
        match self {
            ApiError::Validation { detail, errors } => (
                StatusCode::BAD_REQUEST,
                ProblemDetails::new(
                    "https://nebula.dev/problems/validation-error",
                    "Validation Error",
                    StatusCode::BAD_REQUEST,
                )
                .with_detail(detail)
                .with_errors(errors.clone()),
            ),
            ApiError::Unauthorized(msg) => (
                StatusCode::UNAUTHORIZED,
                ProblemDetails::new(
                    "https://nebula.dev/problems/unauthorized",
                    "Unauthorized",
                    StatusCode::UNAUTHORIZED,
                )
                .with_detail(msg),
            ),
            ApiError::Forbidden(msg) => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new(
                    "https://nebula.dev/problems/forbidden",
                    "Forbidden",
                    StatusCode::FORBIDDEN,
                )
                .with_detail(msg),
            ),
            ApiError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                ProblemDetails::new(
                    "https://nebula.dev/problems/not-found",
                    "Not Found",
                    StatusCode::NOT_FOUND,
                )
                .with_detail(msg),
            ),
            ApiError::Conflict(msg) => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/conflict",
                    "Conflict",
                    StatusCode::CONFLICT,
                )
                .with_detail(msg),
            ),
            ApiError::AlreadyExists(msg) => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/already-exists",
                    "Already Exists",
                    StatusCode::CONFLICT,
                )
                .with_detail(msg),
            ),
            ApiError::VersionExhausted(msg) => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/version-exhausted",
                    "Version Exhausted",
                    StatusCode::CONFLICT,
                )
                .with_detail(msg),
            ),
            ApiError::OutcomeUnknown(msg) => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/outcome-unknown",
                    "Operation Outcome Unknown",
                    StatusCode::CONFLICT,
                )
                .with_detail(msg),
            ),
            ApiError::CredentialReauthRequired => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/credential-reauth-required",
                    "Credential Reauthentication Required",
                    StatusCode::CONFLICT,
                )
                .with_detail(
                    "Reconnect the integration credential before retrying this operation.",
                ),
            ),
            ApiError::CredentialRefreshNotAppliedNever => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/credential-refresh-not-applied",
                    "Credential Refresh Not Applied",
                    StatusCode::CONFLICT,
                )
                .with_detail(
                    "The credential refresh was not applied for the current credential state.",
                ),
            ),
            ApiError::CredentialRefreshNotAppliedAfter { .. } => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/credential-refresh-not-applied",
                    "Credential Refresh Not Applied",
                    StatusCode::CONFLICT,
                )
                .with_detail(
                    "The credential refresh was not applied. Retry only after the Retry-After delay and with a new Idempotency-Key; reusing the same key intentionally replays this response."
                ),
            ),
            ApiError::CredentialRefreshReconciliationRequired => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/credential-refresh-reconciliation-required",
                    "Credential Refresh Reconciliation Required",
                    StatusCode::CONFLICT,
                )
                .with_detail(
                    "The refresh outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile or reconnect the integration credential."
                ),
            ),
            ApiError::CredentialRevokeReconciliationRequired => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/credential-revoke-reconciliation-required",
                    "Credential Revoke Reconciliation Required",
                    StatusCode::CONFLICT,
                )
                .with_detail(
                    "The revoke outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile credential state."
                ),
            ),
            ApiError::RateLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                ProblemDetails::new(
                    "https://nebula.dev/problems/rate-limit",
                    "Rate Limit Exceeded",
                    StatusCode::TOO_MANY_REQUESTS,
                ),
            ),
            ApiError::Internal(msg) => {
                // Security: don't reveal internal details to client
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails::new(
                        "about:blank",
                        "Internal Server Error",
                        StatusCode::INTERNAL_SERVER_ERROR,
                    ),
                )
            },
            ApiError::ServiceUnavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ProblemDetails::new(
                    "https://nebula.dev/problems/service-unavailable",
                    "Service Unavailable",
                    StatusCode::SERVICE_UNAVAILABLE,
                )
                .with_detail(msg),
            ),
            ApiError::Storage(err) => {
                tracing::error!("Storage error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails::new(
                        "https://nebula.dev/problems/storage-error",
                        "Internal Server Error",
                        StatusCode::INTERNAL_SERVER_ERROR,
                    ),
                )
            },
            ApiError::InvalidWorkflowDefinition { detail, errors } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemDetails::new(
                    "https://nebula.dev/problems/invalid-workflow-definition",
                    "Invalid Workflow Definition",
                    StatusCode::UNPROCESSABLE_ENTITY,
                )
                .with_detail(detail)
                .with_errors(
                    errors
                        .iter()
                        .map(|err| ValidationFieldError {
                            code: "workflow_definition_invalid".to_string(),
                            detail: err.to_string(),
                            pointer: workflow_error_pointer(err),
                        })
                        .collect(),
                ),
            ),
            ApiError::SessionExpired => (
                StatusCode::UNAUTHORIZED,
                ProblemDetails::new(
                    "https://nebula.dev/problems/session-expired",
                    "Session Expired",
                    StatusCode::UNAUTHORIZED,
                ),
            ),
            ApiError::MfaRequired => (
                StatusCode::UNAUTHORIZED,
                ProblemDetails::new(
                    "https://nebula.dev/problems/mfa-required",
                    "MFA Required",
                    StatusCode::UNAUTHORIZED,
                ),
            ),
            ApiError::InsufficientRole {
                required_role,
                current_role,
            } => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new(
                    "https://nebula.dev/problems/insufficient-role",
                    "Insufficient Role",
                    StatusCode::FORBIDDEN,
                )
                .with_detail(format!(
                    "{required_role} required, current role {current_role}"
                )),
            ),
            ApiError::QuotaExceeded(msg) => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new(
                    "https://nebula.dev/problems/quota-exceeded",
                    "Quota Exceeded",
                    StatusCode::FORBIDDEN,
                )
                .with_detail(msg),
            ),
            ApiError::VersionMismatch(msg) => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "https://nebula.dev/problems/version-mismatch",
                    "Version Mismatch",
                    StatusCode::CONFLICT,
                )
                .with_detail(msg),
            ),
            ApiError::Gone(msg) => (
                StatusCode::GONE,
                ProblemDetails::new("https://nebula.dev/problems/gone", "Gone", StatusCode::GONE)
                    .with_detail(msg),
            ),
            ApiError::Unprocessable(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemDetails::new(
                    "https://nebula.dev/problems/unprocessable",
                    "Unprocessable Entity",
                    StatusCode::UNPROCESSABLE_ENTITY,
                )
                .with_detail(msg),
            ),
            ApiError::AccountLocked(msg) => (
                StatusCode::from_u16(423).unwrap_or(StatusCode::FORBIDDEN),
                ProblemDetails::new(
                    "https://nebula.dev/problems/account-locked",
                    "Account Locked",
                    StatusCode::from_u16(423).unwrap_or(StatusCode::FORBIDDEN),
                )
                .with_detail(msg),
            ),
            ApiError::UpstreamError(msg) => (
                StatusCode::BAD_GATEWAY,
                ProblemDetails::new(
                    "https://nebula.dev/problems/upstream-error",
                    "Upstream Error",
                    StatusCode::BAD_GATEWAY,
                )
                .with_detail(msg),
            ),
            ApiError::StorageFull => (
                StatusCode::INSUFFICIENT_STORAGE,
                ProblemDetails::new(
                    "https://nebula.dev/problems/storage-full",
                    "Storage Full",
                    StatusCode::INSUFFICIENT_STORAGE,
                ),
            ),
            ApiError::NotImplemented(reason) => (
                StatusCode::NOT_IMPLEMENTED,
                ProblemDetails::new(
                    "https://nebula.dev/problems/not-implemented",
                    "Not Implemented",
                    StatusCode::NOT_IMPLEMENTED,
                )
                .with_detail(reason),
            ),
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
        assert_eq!(errors[0].pointer, "/profile/name");
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
                .any(|e| e.code == "required" && e.pointer == "/email")
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
            errors[0].pointer.starts_with("/nodes/"),
            "DuplicateNodeKey must produce a /nodes/<key> pointer, got: {:?}",
            errors[0].pointer
        );
        assert_eq!(errors[0].pointer, "/nodes/step_a");
    }

    #[test]
    fn invalid_workflow_definition_structural_error_produces_root_pointer() {
        use nebula_workflow::WorkflowError;

        let api_error = ApiError::InvalidWorkflowDefinition {
            detail: "1 error(s)".to_string(),
            errors: vec![WorkflowError::CycleDetected],
        };
        let (status, problem) = api_error.to_problem_details();

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let errors = problem.errors.expect("errors must be present");
        assert_eq!(errors.len(), 1);
        // Structural errors (cycle, no entry nodes, etc.) point at root — RFC 6901 empty string.
        assert_eq!(
            errors[0].pointer, "",
            "CycleDetected must produce the RFC 6901 root pointer (empty string), got: {:?}",
            errors[0].pointer
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
            errors[0].pointer, "/connections/a/b",
            "DuplicateConnection must produce /connections/<from>/<to>, got: {:?}",
            errors[0].pointer
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
            Some(
                "The credential refresh was not applied. Retry only after the Retry-After delay and with a new Idempotency-Key; reusing the same key intentionally replays this response."
            )
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
