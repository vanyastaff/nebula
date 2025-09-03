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

mod builder;
mod config;
mod format;
mod layer;
mod macros;
mod timing;
mod utils;
mod writer;

#[cfg(any(feature = "telemetry", feature = "sentry"))]
mod telemetry;

// Public API
pub use builder::{LoggerBuilder, LoggerGuard};
pub use config::{Config, Format, Level, Rolling, WriterConfig};
pub use layer::context::{Context, ContextGuard, Fields};
pub use timing::{Timed, Timer, TimerGuard};

/// Prelude for common imports
pub mod prelude {
    pub use crate::{
        Level, Result, Timed, Timer, auto_init, debug, error, info, init, init_with, instrument,
        span, trace, warn,
    };

    pub use tracing::{Span, field};
}

// Re-export tracing macros
pub use tracing::{debug, error, info, instrument, span, trace, warn};

/// Result type for logger operations
pub type Result<T> = anyhow::Result<T>;

/// Error type for logger operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Filter parsing error
    #[error("Invalid filter: {0}")]
    Filter(String),
}

// Test initialization guard
#[cfg(test)]
static TEST_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();

// ============================================================================
// Initialization Functions
// ============================================================================

/// Auto-detect and initialize the best logging configuration
pub fn auto_init() -> Result<LoggerGuard> {
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
pub fn init() -> Result<LoggerGuard> {
    init_with(Config::default())
}

/// Initialize with custom configuration
pub fn init_with(config: Config) -> Result<LoggerGuard> {
    LoggerBuilder::from_config(config).build()
}

/// Initialize for tests (captures logs)
#[cfg(test)]
pub fn init_test() -> Result<LoggerGuard> {
    TEST_INIT.get_or_init(|| ());
    if tracing::dispatcher::has_been_set() {
        return Ok(LoggerGuard::noop());
    }
    init_with(Config::test())
}
