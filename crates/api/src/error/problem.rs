//! RFC 9457 Problem Details shape.
//!
//! `ProblemDetails` is the **wire type** for every error response this API
//! emits.  Its field names, `#[serde(rename)]` annotations,
//! `skip_serializing_if` conditions, and the `application/problem+json`
//! content-type header are a **public contract** enforced by
//! `tests/openapi_canon_compliance.rs` — do not alter them.

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// RFC 9457 Problem Details
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

    /// Additional extension members — RFC 9457 allows arbitrary
    /// problem-type-specific keys to be flattened onto the document. utoipa
    /// describes the `Value` payload as an open `Object`.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<serde_json::Value>)]
    pub extensions: Option<serde_json::Value>,

    /// Validation errors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ValidationFieldError>>,
}

/// Validation field error
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
