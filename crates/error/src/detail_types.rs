//! Typed error detail structs.
//!
//! Placeholder types — full implementation in a later task.

/// Describes field-level violations in a bad request.
#[derive(Debug)]
pub struct BadRequest;

/// Debug/diagnostic information attached to an error.
#[derive(Debug)]
pub struct DebugInfo;

/// A single field violation within a [`BadRequest`].
#[derive(Debug)]
pub struct FieldViolation;

/// Precondition failures that prevented an operation.
#[derive(Debug)]
pub struct PreconditionFailure;

/// A single precondition that was violated.
#[derive(Debug)]
pub struct PreconditionViolation;

/// Quota/resource-limit information.
#[derive(Debug)]
pub struct QuotaInfo;

/// Identifies the resource an error relates to.
#[derive(Debug)]
pub struct ResourceInfo;

/// Advisory retry information attached as a detail.
#[derive(Debug)]
pub struct RetryInfo;
