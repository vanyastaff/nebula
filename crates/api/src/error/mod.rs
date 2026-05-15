//! Error handling — RFC 9457 `application/problem+json` seam (canon §12.4).
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
    Storage(#[from] nebula_storage::StorageError),

    /// Workflow repository error
    #[classify(category = "internal", code = "API:WORKFLOW_REPO")]
    #[error("Workflow repository error: {0}")]
    WorkflowRepo(#[from] nebula_storage::WorkflowRepoError),

    /// Execution repository error
    #[classify(category = "internal", code = "API:EXECUTION_REPO")]
    #[error("Execution repository error: {0}")]
    ExecutionRepo(#[from] nebula_storage::ExecutionRepoError),

    /// Invalid workflow definition — structurally valid JSON but semantically
    /// invalid per `nebula_workflow::validate_workflow` (RFC 9457 **422**).
    ///
    /// Distinct from [`Self::Validation`] (400), which covers request-level
    /// parse/format errors. This variant is returned only from
    /// `activate_workflow` after the stored definition fails structural
    /// DAG/schema checks.
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
    /// Used by class-(c) stub handlers under ADR-0047 Stub Endpoint Policy
    /// so the runtime status code matches the `responses(501)` annotation
    /// in the OpenAPI document. Migrating from `Internal("not implemented")`
    /// (500) to this variant keeps the stub-honesty contract self-consistent.
    #[classify(category = "internal", code = "API:NOT_IMPLEMENTED")]
    #[error("Not implemented: {0}")]
    NotImplemented(String),
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
            ApiError::WorkflowRepo(err) => {
                tracing::error!("Workflow repository error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails::new(
                        "https://nebula.dev/problems/workflow-repo-error",
                        "Internal Server Error",
                        StatusCode::INTERNAL_SERVER_ERROR,
                    ),
                )
            },
            ApiError::ExecutionRepo(err) => {
                tracing::error!("Execution repository error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails::new(
                        "https://nebula.dev/problems/execution-repo-error",
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
            "application/problem+json".parse().unwrap(),
        );
        response
    }
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

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
}
