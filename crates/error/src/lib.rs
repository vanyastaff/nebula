//! # nebula-error
//!
//! Enterprise error infrastructure for the Nebula workflow engine.
//!
//! This crate provides the foundational error primitives used across all Nebula
//! crates — classification traits, a generic error wrapper, and extensible
//! typed details inspired by Google's error model and the AWS SDK.
//!
//! ## Key Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Classify`] | Core trait — category, code, severity, retryability |
//! | [`NebulaError`] | Generic wrapper adding details + context chain |
//! | [`ErrorDetails`] | TypeId-keyed extensible detail storage |
//! | [`ErrorCategory`] | Canonical "what happened" classification |
//! | [`ErrorSeverity`] | Error / Warning / Info severity levels |
//! | [`ErrorCode`] | Machine-readable error code newtype |
//! | [`ErrorCollection`] | Batch/validation error aggregation |

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
    BadRequest, DebugInfo, ErrorRoute, ExecutionContext, FieldViolation, PreconditionFailure,
    PreconditionViolation, QuotaInfo, ResourceInfo, RetryInfo, TypeMismatch,
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
pub use nebula_error_macros::Classify as DeriveClassify;
