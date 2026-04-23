//! Error Handling
//!
//! RFC 9457 Problem Details for HTTP APIs implementation.
//! Единая обработка ошибок для всего API.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use nebula_validator::foundation::{ValidationError, ValidationErrors};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// RFC 9457 Problem Details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemDetails {
    /// URI reference identifying the problem type
    #[serde(rename = "type")]
    pub type_uri: String,

    /// Short human-readable summary
    pub title: String,

    /// HTTP status code
    pub status: u16,

    /// Human-readable explanation specific to this occurrence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// URI reference identifying the specific occurrence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,

    /// Additional extension members
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,

    /// Validation errors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ValidationFieldError>>,
}

/// Validation field error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationFieldError {
    /// Validator error code
    pub code: String,
    /// Error detail message
    pub detail: String,
    /// JSON Pointer to the field (RFC 6901), e.g. "/age"
    pub pointer: String,
}

impl ProblemDetails {
    /// Create a new ProblemDetails
    pub fn new(type_uri: impl Into<String>, title: impl Into<String>, status: StatusCode) -> Self {
        Self {
            type_uri: type_uri.into(),
            title: title.into(),
            status: status.as_u16(),
            detail: None,
            instance: None,
            extensions: None,
            errors: None,
        }
    }

    /// Add detail message
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Add instance URI
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// Add extension data
    pub fn with_extensions(mut self, extensions: serde_json::Value) -> Self {
        self.extensions = Some(extensions);
        self
    }

    /// Add validation errors
    pub fn with_errors(mut self, errors: Vec<ValidationFieldError>) -> Self {
        self.errors = Some(errors);
        self
    }
}

/// Main API Error Type
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
}

/// Map a [`nebula_workflow::WorkflowError`] to a JSON Pointer (RFC 6901)
/// that identifies the offending location in the workflow JSON document.
///
/// The workflow JSON schema is:
/// ```json
/// {
///   "nodes":       [ { "id": "<key>", … }, … ],
///   "connections": [ { "from_node": "<f>", "to_node": "<t>", … }, … ],
///   "trigger":     { … },
///   …
/// }
/// ```
///
/// Pointer conventions used here:
/// - Node-specific errors: `/nodes/<node_key>`
/// - Connection-specific: `/connections/<from>/<to>`
/// - Trigger errors:      `/trigger`
/// - Structural / whole-document errors: `""` (the root pointer, RFC 6901 §4)
fn workflow_error_pointer(err: &nebula_workflow::WorkflowError) -> String {
    use nebula_workflow::WorkflowError;
    match err {
        // Node-keyed errors
        WorkflowError::DuplicateNodeKey(key)
        | WorkflowError::UnknownNode(key)
        | WorkflowError::SelfLoop(key) => format!("/nodes/{key}"),

        WorkflowError::InvalidParameterReference { node_key, .. } => {
            format!("/nodes/{node_key}")
        },

        WorkflowError::InvalidActionKey { key, .. } => {
            // The key string is the node's action_key; best we can do without
            // the node key is to point at the nodes array.
            let _ = key;
            "/nodes".to_owned()
        },

        // Connection-keyed errors
        WorkflowError::DuplicateConnection { from, to } => {
            format!("/connections/{from}/{to}")
        },

        // Trigger errors
        WorkflowError::InvalidTrigger { .. } => "/trigger".to_owned(),

        // Schema-level / structural — point at root
        WorkflowError::EmptyName
        | WorkflowError::NoNodes
        | WorkflowError::CycleDetected
        | WorkflowError::NoEntryNodes
        | WorkflowError::UnsupportedSchema { .. }
        | WorkflowError::InvalidOwnerId
        | WorkflowError::GraphError(_) => String::new(), // RFC 6901 root pointer
    }
}

fn normalize_pointer(pointer: Option<&str>) -> String {
    let pointer = pointer.unwrap_or("/").trim();
    if pointer.is_empty() || pointer == "#" {
        return "/".to_owned();
    }

    if let Some(rest) = pointer.strip_prefix('#') {
        if rest.is_empty() {
            return "/".to_owned();
        }
        if rest.starts_with('/') {
            return rest.to_owned();
        }
    }

    if pointer.starts_with('/') {
        pointer.to_owned()
    } else {
        format!("/{pointer}")
    }
}

fn flatten_validation_error(
    err: &ValidationError,
    inherited_pointer: Option<&str>,
    out: &mut Vec<ValidationFieldError>,
) {
    let pointer = err
        .field_pointer()
        .map(std::borrow::Cow::into_owned)
        .or_else(|| inherited_pointer.map(str::to_owned))
        .unwrap_or_else(|| "/".to_owned());

    out.push(ValidationFieldError {
        code: err.code.to_string(),
        detail: err.message.to_string(),
        pointer: normalize_pointer(Some(&pointer)),
    });

    for nested in err.nested() {
        flatten_validation_error(nested, Some(&pointer), out);
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
}

impl From<ValidationError> for ApiError {
    fn from(value: ValidationError) -> Self {
        let mut errors = Vec::new();
        flatten_validation_error(&value, None, &mut errors);
        let detail = if value.code.is_empty() {
            value.message.to_string()
        } else {
            format!("[{}] {}", value.code, value.message)
        };

        Self::Validation { detail, errors }
    }
}

impl From<nebula_core::PermissionDenied> for ApiError {
    fn from(pd: nebula_core::PermissionDenied) -> Self {
        Self::InsufficientRole {
            required_role: pd.required_role,
            current_role: pd.current_role,
        }
    }
}

impl From<ValidationErrors> for ApiError {
    fn from(value: ValidationErrors) -> Self {
        let mut errors = Vec::new();
        for item in value.errors() {
            flatten_validation_error(item, None, &mut errors);
        }

        Self::Validation {
            detail: format!("Validation failed with {} error(s)", errors.len()),
            errors,
        }
    }
}

impl ApiError {
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
    use super::*;

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
