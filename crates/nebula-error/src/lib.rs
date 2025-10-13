//! # Nebula Error Handling  
//!
//! Centralized, high-performance error handling for the Nebula workflow engine.
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_error::prelude::*;
//!
//! fn validate_age(age: u32) -> Result<()> {
//!     ensure!(age >= 18, validation_error!("Must be 18+"));
//!     ensure!(age <= 120, validation_error!("Invalid age"));
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - **Stable V1 API**: Production-ready, 64 bytes per error
//! - **Optimized V2**: 48 bytes, 25% memory improvement, bug-fixed
//! - **Ergonomic**: Macros (`validation_error!`, `ensure!`, etc.)
//! - **Smart**: Auto-converts from stdlib/3rd-party errors
//! - **Resilient**: Built-in retry strategies with backoff
//!
//! ## Performance
//!
//! - Memory: V1=64 bytes, V2=48 bytes (25% improvement)
//! - Category checks: 2x faster with bitflags
//! - Zero allocations for static error messages

#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::module_name_repetitions)]

// === V1 API (Stable, Production-Ready) ===
pub mod core;
pub mod kinds;

// === V2 API (Optimized, 25% Smaller) ===
pub mod optimized;

// === Ergonomic Macros ===
pub mod macros;

// === Development Tools ===
#[cfg(test)]
pub mod size_analysis;

// === Public API Exports ===

/// Main error type (V1 - stable, recommended for production)
pub use core::NebulaError;

/// Result type alias for `Result<T, NebulaError>`
pub use core::Result;

/// Error categorization (Client/Server/System/Workflow/etc)
pub use kinds::ErrorKind;

/// Error context with metadata and correlation IDs
pub use core::ErrorContext;

/// Retry strategy for resilient operations
pub use core::RetryStrategy;

/// Retry function with exponential backoff
pub use core::retry;

/// Extension traits for Result types
pub use core::{NebulaResultExt, ResultExt};

/// Conversion traits for external errors
pub use core::{ErrorContextBuilder, IntoNebulaError, Retryable, retry_with_timeout};

/// Convenient prelude with everything you need
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
