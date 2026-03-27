//! Typed error detail structs inspired by the Google `google.rpc` error model.
//!
//! Each struct carries structured metadata about a specific failure aspect
//! and can be stored in [`ErrorDetails`](crate::ErrorDetails) via the
//! [`ErrorDetail`](crate::ErrorDetail) marker trait.

use std::borrow::Cow;
use std::time::Duration;

use crate::code::ErrorCode;
use crate::details::ErrorDetail;

/// Advisory retry information attached to a retriable error.
///
/// Mirrors `google.rpc.RetryInfo`. Consumers can inspect this to decide
/// how long to wait before retrying and how many attempts remain.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, RetryInfo};
/// use std::time::Duration;
///
/// let mut details = ErrorDetails::new();
/// details.insert(RetryInfo {
///     retry_delay: Some(Duration::from_millis(500)),
///     max_attempts: Some(3),
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryInfo {
    /// Suggested delay before the next retry attempt.
    pub retry_delay: Option<Duration>,
    /// Maximum number of retry attempts the caller should make.
    pub max_attempts: Option<u32>,
}

impl ErrorDetail for RetryInfo {}

/// Identifies the resource an error relates to.
///
/// Mirrors `google.rpc.ResourceInfo`. Attach this when the error is
/// caused by a specific named resource (workflow, node, credential, etc.).
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, ResourceInfo};
///
/// let mut details = ErrorDetails::new();
/// details.insert(ResourceInfo {
///     resource_type: "workflow".into(),
///     resource_name: "daily-report".into(),
///     owner: None,
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInfo {
    /// The kind of resource (e.g. `"workflow"`, `"credential"`).
    pub resource_type: Cow<'static, str>,
    /// The name or identifier of the resource.
    pub resource_name: String,
    /// Optional owner of the resource.
    pub owner: Option<String>,
}

impl ErrorDetail for ResourceInfo {}

/// Describes field-level violations in a bad request.
///
/// Mirrors `google.rpc.BadRequest`. Attach this when input validation
/// fails on one or more fields.
///
/// # Examples
///
/// ```
/// use nebula_error::{BadRequest, ErrorDetails, FieldViolation, codes};
///
/// let mut details = ErrorDetails::new();
/// details.insert(BadRequest {
///     violations: vec![FieldViolation {
///         field: "email".into(),
///         description: "must be a valid email address".into(),
///         code: codes::VALIDATION,
///     }],
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadRequest {
    /// The list of field-level violations.
    pub violations: Vec<FieldViolation>,
}

impl ErrorDetail for BadRequest {}

/// A single field violation within a [`BadRequest`].
///
/// Describes which field failed validation, why, and with what error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldViolation {
    /// The field path that failed validation (e.g. `"config.timeout"`).
    pub field: String,
    /// Human-readable description of the violation.
    pub description: String,
    /// Machine-readable error code for this violation.
    pub code: ErrorCode,
}

/// Debug/diagnostic information attached to an error.
///
/// Mirrors `google.rpc.DebugInfo`. Typically stripped before sending
/// errors to end users but valuable for internal logging.
///
/// # Examples
///
/// ```
/// use nebula_error::{DebugInfo, ErrorDetails};
///
/// let mut details = ErrorDetails::new();
/// details.insert(DebugInfo {
///     detail: "connection pool exhausted after 30s".into(),
///     stack_entries: vec!["engine::execute".into(), "action::http::run".into()],
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugInfo {
    /// Free-form diagnostic detail string.
    pub detail: String,
    /// Logical stack trace entries (not necessarily OS stack frames).
    pub stack_entries: Vec<String>,
}

impl ErrorDetail for DebugInfo {}

/// Quota/resource-limit information.
///
/// Mirrors `google.rpc.QuotaFailure` (simplified). Attach this when an
/// operation fails because a quota or resource limit was exceeded.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, QuotaInfo};
///
/// let mut details = ErrorDetails::new();
/// details.insert(QuotaInfo {
///     metric: "api_calls_per_minute".into(),
///     limit: 100,
///     used: 101,
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaInfo {
    /// The quota metric that was exceeded.
    pub metric: String,
    /// The maximum allowed value.
    pub limit: u64,
    /// The current usage value that exceeded the limit.
    pub used: u64,
}

impl ErrorDetail for QuotaInfo {}

/// Precondition failures that prevented an operation.
///
/// Mirrors `google.rpc.PreconditionFailure`. Attach this when one or
/// more preconditions were not met before the operation could proceed.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, PreconditionFailure, PreconditionViolation};
///
/// let mut details = ErrorDetails::new();
/// details.insert(PreconditionFailure {
///     violations: vec![PreconditionViolation {
///         r#type: "TOS".into(),
///         subject: "user:123".into(),
///         description: "Terms of service not accepted".into(),
///     }],
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreconditionFailure {
    /// The list of precondition violations.
    pub violations: Vec<PreconditionViolation>,
}

impl ErrorDetail for PreconditionFailure {}

/// A single precondition that was violated.
///
/// Used within [`PreconditionFailure`] to describe each unmet precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreconditionViolation {
    /// The type of precondition (e.g. `"TOS"`, `"AGE"`).
    pub r#type: String,
    /// The subject the precondition applies to.
    pub subject: String,
    /// Human-readable description of the precondition failure.
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes;
    use crate::details::ErrorDetails;

    #[test]
    fn retry_info_stored_and_retrieved() {
        let mut details = ErrorDetails::new();
        let info = RetryInfo {
            retry_delay: Some(Duration::from_secs(5)),
            max_attempts: Some(3),
        };
        details.insert(info.clone());

        let retrieved = details.get::<RetryInfo>().unwrap();
        assert_eq!(retrieved, &info);
    }

    #[test]
    fn resource_info_fields() {
        let info = ResourceInfo {
            resource_type: "workflow".into(),
            resource_name: "daily-report".into(),
            owner: Some("team-a".into()),
        };
        assert_eq!(info.resource_type, "workflow");
        assert_eq!(info.resource_name, "daily-report");
        assert_eq!(info.owner.as_deref(), Some("team-a"));
    }

    #[test]
    fn bad_request_with_violations() {
        let bad = BadRequest {
            violations: vec![
                FieldViolation {
                    field: "email".into(),
                    description: "invalid format".into(),
                    code: codes::VALIDATION,
                },
                FieldViolation {
                    field: "age".into(),
                    description: "must be positive".into(),
                    code: codes::VALIDATION,
                },
            ],
        };
        assert_eq!(bad.violations.len(), 2);
        assert_eq!(bad.violations[0].field, "email");
        assert_eq!(bad.violations[1].field, "age");
    }

    #[test]
    fn multiple_detail_types_coexist() {
        let mut details = ErrorDetails::new();
        details.insert(RetryInfo {
            retry_delay: None,
            max_attempts: Some(1),
        });
        details.insert(ResourceInfo {
            resource_type: "node".into(),
            resource_name: "http-1".into(),
            owner: None,
        });
        details.insert(QuotaInfo {
            metric: "requests".into(),
            limit: 100,
            used: 150,
        });

        assert!(details.has::<RetryInfo>());
        assert!(details.has::<ResourceInfo>());
        assert!(details.has::<QuotaInfo>());
        assert_eq!(details.len(), 3);
    }
}
