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

/// Execution context identifying where in a workflow an error occurred.
///
/// Attach this to errors that originate during workflow execution so
/// that error handlers, loggers, and monitoring can correlate failures
/// back to specific nodes and runs.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, ExecutionContext};
///
/// let mut details = ErrorDetails::new();
/// details.insert(ExecutionContext {
///     node_id: Some("http-fetch-1".into()),
///     workflow_id: Some("wf-daily-report".into()),
///     correlation_id: Some("req-abc-123".into()),
///     attempt: Some(2),
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    /// The node that produced this error, if known.
    pub node_id: Option<String>,
    /// The workflow run that this error belongs to.
    pub workflow_id: Option<String>,
    /// A correlation ID for distributed tracing (e.g. OTel trace ID).
    pub correlation_id: Option<String>,
    /// The retry attempt number (1-based), if this is a retried operation.
    pub attempt: Option<u32>,
}

impl ErrorDetail for ExecutionContext {}

/// Routing hint for error-edge traversal in workflow DAGs.
///
/// When a node fails and the DAG has error edges, this detail tells
/// the engine which error handler to route to, or whether the error
/// should go to a dead letter queue.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, ErrorRoute};
///
/// let mut details = ErrorDetails::new();
/// details.insert(ErrorRoute {
///     suggested_handler: Some("alert-oncall".into()),
///     dead_letter: false,
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorRoute {
    /// Name/ID of the suggested error handler node.
    pub suggested_handler: Option<String>,
    /// Whether this error should be routed to the dead letter queue.
    pub dead_letter: bool,
}

impl ErrorDetail for ErrorRoute {}

/// Type mismatch between connected DAG nodes.
///
/// Attached when a type validation check detects that an upstream
/// node's output type doesn't match a downstream node's expected
/// input type. This prevents silent casts and data corruption.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorDetails, TypeMismatch};
///
/// let mut details = ErrorDetails::new();
/// details.insert(TypeMismatch {
///     expected: "u64".into(),
///     actual: "f64".into(),
///     location: Some("edge: fetch → transform".into()),
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeMismatch {
    /// The expected type name.
    pub expected: String,
    /// The actual type name.
    pub actual: String,
    /// Where in the DAG this mismatch was detected.
    pub location: Option<String>,
}

impl ErrorDetail for TypeMismatch {}

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

    #[test]
    fn execution_context_stored_and_retrieved() {
        let mut details = ErrorDetails::new();
        details.insert(ExecutionContext {
            node_id: Some("http-fetch-1".into()),
            workflow_id: Some("wf-daily-report".into()),
            correlation_id: Some("req-abc-123".into()),
            attempt: Some(2),
        });

        let ctx = details.get::<ExecutionContext>().unwrap();
        assert_eq!(ctx.node_id.as_deref(), Some("http-fetch-1"));
        assert_eq!(ctx.attempt, Some(2));
    }

    #[test]
    fn error_route_stored_and_retrieved() {
        let mut details = ErrorDetails::new();
        details.insert(ErrorRoute {
            suggested_handler: Some("retry-with-backoff".into()),
            dead_letter: false,
        });

        let route = details.get::<ErrorRoute>().unwrap();
        assert_eq!(
            route.suggested_handler.as_deref(),
            Some("retry-with-backoff")
        );
        assert!(!route.dead_letter);
    }

    #[test]
    fn error_route_dead_letter() {
        let mut details = ErrorDetails::new();
        details.insert(ErrorRoute {
            suggested_handler: None,
            dead_letter: true,
        });

        let route = details.get::<ErrorRoute>().unwrap();
        assert!(route.dead_letter);
    }

    #[test]
    fn type_mismatch_stored_and_retrieved() {
        let mut details = ErrorDetails::new();
        details.insert(TypeMismatch {
            expected: "JsonObject".into(),
            actual: "JsonArray".into(),
            location: Some("edge from http-fetch → parse-response".into()),
        });

        let tm = details.get::<TypeMismatch>().unwrap();
        assert_eq!(tm.expected, "JsonObject");
        assert_eq!(tm.actual, "JsonArray");
        assert!(tm.location.is_some());
    }
}
