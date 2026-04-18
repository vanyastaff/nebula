//! # nebula-error
//!
//! Error taxonomy and classification boundary for the Nebula workflow engine.
//!
//! Every library crate in the workspace uses `thiserror` + `Classify` + `NebulaError`
//! so that transient vs permanent failure is an explicit decision, not folklore
//! scattered across individual action implementations. See `crates/error/README.md`
//! for the full role description and contract invariants.
//!
//! ## Purpose
//!
//! Provides the foundational error primitives — classification traits, a generic
//! wrapper with extensible typed details (inspired by Google's error model and the
//! AWS SDK), and structured retry guidance that `nebula-resilience` consumes.
//!
//! ## Role
//!
//! **Error Taxonomy and Classification Boundary** (canon §3.10, §4.2, §12.4).
//! `nebula-api` maps `NebulaError` to RFC 9457 `problem+json` at the HTTP boundary.
//!
//! ## Public API
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Classify`] | Core trait — category, code, severity, retryability |
//! | [`ErrorClassifier`] | Pattern: use `Classify` at decision points, not folklore |
//! | [`NebulaError`] | Generic wrapper adding details + context chain |
//! | [`ErrorDetails`] | TypeId-keyed extensible detail storage |
//! | [`ErrorCategory`] | Canonical "what happened" classification |
//! | [`ErrorSeverity`] | Error / Warning / Info severity levels |
//! | [`ErrorCode`] | Machine-readable error code newtype |
//! | [`ErrorCollection`] | Batch/validation error aggregation |
//! | [`RetryHint`] | Structured retry guidance consumed by `nebula-resilience` |

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod category;
mod code;
mod collection;
mod convert;
mod detail_types;
mod details;
mod error;
mod retry;
mod severity;
mod traits;

pub use category::ErrorCategory;
pub use code::{ErrorCode, codes};
pub use collection::{BatchResult, ErrorCollection};
pub use detail_types::{
    BadRequest, DebugInfo, DependencyInfo, ErrorRoute, ExecutionContext, FieldViolation, HelpLink,
    PreconditionFailure, PreconditionViolation, QuotaInfo, RequestInfo, ResourceInfo, TypeMismatch,
};
pub use details::{ErrorDetail, ErrorDetails};
pub use error::NebulaError;
pub use retry::RetryHint;
pub use severity::ErrorSeverity;
pub use traits::{Classify, ErrorClassifier};

/// Convenience result type alias.
///
/// Wraps `std::result::Result` with [`NebulaError<E>`] as the error type,
/// so callers can write `nebula_error::Result<T, MyError>` instead of
/// `Result<T, NebulaError<MyError>>`.
pub type Result<T, E> = std::result::Result<T, NebulaError<E>>;

/// Re-export derive macro when feature is enabled.
#[cfg(feature = "derive")]
pub use nebula_error_macros::Classify;
