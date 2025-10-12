//! # Nebula Log - Production-Ready Rust Logging
//!
//! Zero-config logging that scales from development to production.
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_log::prelude::*;
//!
//! fn main() -> Result<()> {
//!     // Auto-detect best configuration
//!     nebula_log::auto_init()?;
//!
//!     info!(port = 8080, "Server starting");
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

pub mod core;

mod builder;
mod config;
mod format;
mod layer;
mod macros;
mod timing;
mod writer;

#[cfg(any(feature = "telemetry", feature = "sentry"))]
mod telemetry;

// Metrics module (optional)
#[cfg(feature = "observability")]
pub mod metrics;

// Observability module
pub mod observability;

// Public API
pub use builder::{LoggerBuilder, LoggerGuard};
pub use config::{Config, Format, Level, Rolling, WriterConfig};
pub use layer::context::{Context, ContextGuard, Fields};
pub use timing::{Timed, Timer, TimerGuard};

// Re-export core types
pub use core::{LogError, LogResult, LogResultExt, NebulaError, NebulaResult, ResultExt};

/// Prelude for common imports
pub mod prelude {
    pub use crate::{
        Level, LogError, LogResult, LogResultExt, NebulaError, Timed, Timer, auto_init, debug,
        error, info, init, init_with, instrument, span, trace, warn,
    };

    pub use tracing::{Span, field};

    // Metrics (when observability feature is enabled)
    #[cfg(feature = "observability")]
    pub use crate::metrics::{counter, gauge, histogram, timed_block, timed_block_async};

    // Observability hooks and events
    pub use crate::observability::{
        emit_event, register_hook, LoggingHook, ObservabilityEvent, ObservabilityHook,
        OperationCompleted, OperationFailed, OperationStarted, OperationTracker,
    };

    #[cfg(feature = "observability")]
    pub use crate::observability::MetricsHook;
}

// Re-export tracing macros
pub use tracing::{debug, error, info, instrument, span, trace, warn};

// Test initialization guard
#[cfg(test)]
static TEST_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();

// ============================================================================
// Initialization Functions
// ============================================================================

/// Auto-detect and initialize the best logging configuration
///
/// Checks environment variables (`NEBULA_LOG`, `RUST_LOG`) and debug assertions
/// to choose between development, production, or custom configuration.
///
/// # Errors
///
/// Returns error if filter parsing fails or logger initialization fails
pub fn auto_init() -> LogResult<LoggerGuard> {
    #[cfg(test)]
    {
        TEST_INIT.get_or_init(|| ());
        if tracing::dispatcher::has_been_set() {
            return Ok(LoggerGuard::noop());
        }
    }

    if std::env::var("NEBULA_LOG").is_ok() || std::env::var("RUST_LOG").is_ok() {
        init_with(Config::from_env())
    } else if cfg!(debug_assertions) {
        init_with(Config::development())
    } else {
        init_with(Config::production())
    }
}

/// Initialize with default configuration
///
/// Uses compact format with info level logging to stderr.
///
/// # Errors
///
/// Returns error if logger initialization fails
pub fn init() -> LogResult<LoggerGuard> {
    init_with(Config::default())
}

/// Initialize with custom configuration
///
/// Allows full control over format, output, filters, and telemetry.
///
/// # Errors
///
/// Returns error if:
/// - Filter string is invalid
/// - File writer cannot be created (if using file output)
/// - Telemetry setup fails (if enabled)
pub fn init_with(config: Config) -> LogResult<LoggerGuard> {
    LoggerBuilder::from_config(config).build()
}

/// Initialize for tests (captures logs)
#[cfg(test)]
pub fn init_test() -> LogResult<LoggerGuard> {
    TEST_INIT.get_or_init(|| ());
    if tracing::dispatcher::has_been_set() {
        return Ok(LoggerGuard::noop());
    }
    init_with(Config::test())
}
