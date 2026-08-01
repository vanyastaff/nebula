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

/// One structured validation or activation rejection.
///
/// `code` and `pointer` are stable machine-readable contracts: a client
/// matches on them rather than parsing `detail`. `expected`, `actual`, and
/// `remediation` are the NS14 activation-diagnostic fields — present whenever
/// the rejection came from an activation diagnostic, absent for the
/// request-level validators that have no contract to compare against.
///
/// Flattening these into `detail` prose was the previous shape and it cost
/// every one of them: a client could not tell which rule fired, a UI could not
/// point at the offending element, and an author was told a count rather than
/// what to change.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidationFieldError {
    /// Stable machine-readable rejection code.
    pub code: String,
    /// Human-readable message. Never the only place a field appears.
    pub detail: String,
    /// JSON Pointer to the offending element (RFC 6901), e.g. `/nodes/a`.
    pub pointer: String,
    /// Secret-free description of the contract that was required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// Secret-free description of what was found, or a safe sentinel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Stable, actionable guidance for resolving the rejection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl ValidationFieldError {
    /// Build a request-level validation error with no contract comparison.
    ///
    /// Request-level validators reject a malformed field rather than a
    /// mismatched contract, so they have no `expected`/`actual` pair to report
    /// and deliberately omit the NS14 fields instead of inventing them.
    #[must_use]
    pub fn field(
        code: impl Into<String>,
        detail: impl Into<String>,
        pointer: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            pointer: pointer.into(),
            expected: None,
            actual: None,
            remediation: None,
        }
    }
}

impl From<&nebula_error::ActivationDiagnostic> for ValidationFieldError {
    /// Carry all five NS14 fields onto the wire without flattening any of them.
    ///
    /// `detail` stays populated for a human reading the response, but every
    /// field it summarises is also present on its own, so a client never has to
    /// parse prose to recover a value.
    fn from(diagnostic: &nebula_error::ActivationDiagnostic) -> Self {
        Self {
            code: diagnostic.code().to_owned(),
            detail: format!(
                "{}: expected {}, found {}",
                diagnostic.code(),
                diagnostic.expected(),
                diagnostic.actual()
            ),
            pointer: diagnostic.path().to_owned(),
            expected: Some(diagnostic.expected().to_owned()),
            actual: Some(diagnostic.actual().to_owned()),
            remediation: Some(diagnostic.remediation().to_owned()),
        }
    }
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
