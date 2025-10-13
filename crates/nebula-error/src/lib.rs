//! # Nebula Error Handling
//!
//! Centralized error handling system for the Nebula workflow engine.
//! This crate provides a unified error type system with proper error classification,
//! context propagation, and retry logic for workflow orchestration.

// Strategic allows for pedantic clippy warnings that don't affect production code quality
pub mod core;
pub mod kinds;
pub mod macros;



// Re-export core types
pub use core::{NebulaError, NebulaResultExt, Result, ResultExt};
pub use kinds::ErrorKind;

// Re-export utilities from core
pub use core::{
    ErrorContext, ErrorContextBuilder, IntoNebulaError, RetryStrategy, Retryable, retry,
    retry_with_timeout,
};

// Note: Constructors were integrated into NebulaError impl blocks for better organization

/// Common prelude for error handling
pub mod prelude {
    pub use super::{
        ErrorContext, ErrorContextBuilder, ErrorKind, IntoNebulaError, NebulaError, Result,
        ResultExt, RetryStrategy, Retryable, retry, retry_with_timeout,
    };
    pub use thiserror::Error as ThisError;
    
    // Re-export commonly used macros for convenience
    pub use crate::{
        auth_error, ensure, error_with_context, internal_error, memory_error, not_found_error,
        permission_denied_error, rate_limit_error, resource_error, retryable_error,
        service_unavailable_error, timeout_error, validation_error, workflow_error,
    };
}

// Public re-export of thiserror::Error at the crate root for convenience
pub use thiserror::Error;



