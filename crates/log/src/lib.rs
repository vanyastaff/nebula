//! # nebula-log
//!
//! Structured logging and observability foundation for Nebula, built on top of
//! [`tracing`].
//!
//! This crate provides a single logging pipeline for development and
//! production:
//! - structured logs in multiple formats
//! - configurable writer backends (stderr/stdout/file/fanout)
//! - observability hooks and operation events
//! - optional OpenTelemetry and Sentry integrations
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_log::prelude::*;
//!
//! fn main() -> LogResult<()> {
//!     let _guard = nebula_log::auto_init()?;
//!     info!(service = "api", "server started");
//!     Ok(())
//! }
//! ```
//!
//! Keep [`LoggerGuard`] alive for the process lifetime (or until intentional
//! shutdown).
//!
//! ## Configuration Modes
//!
//! - [`auto_init`]: startup auto-resolution (`explicit > env > preset`)
//! - [`init`]: default config (`Config::default()`)
//! - [`init_with`]: fully explicit setup with [`Config`]
//!
//! Use [`auto_init`] when you want zero-config behavior and environment-driven
//! overrides. Use [`init_with`] when you need deterministic production setup.
//!
//! ## Explicit Configuration Example
//!
//! ```rust
//! use nebula_log::{Config, Format, WriterConfig};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut cfg = Config::production();
//!     cfg.format = Format::Json;
//!     cfg.writer = WriterConfig::Stderr;
//!     cfg.fields.service = Some("nebula-api".to_string());
//!     cfg.fields.env = Some("prod".to_string());
//!
//!     let _guard = nebula_log::init_with(cfg)?;
//!     Ok(())
//! }
//! ```
//!
//! ## Core Capabilities
//!
//! - startup presets (`development`, `production`, env overrides)
//! - formats: `pretty`, `compact`, `json`, `logfmt`
//! - writer backends: stderr/stdout/file, fanout with failure policy
//! - rolling files: hourly/daily/size/size+retention
//! - timing utilities and macros
//! - observability hooks/events with typed event kinds
//! - optional telemetry integrations: OpenTelemetry OTLP and Sentry
//!
//! ## Feature Flags
//!
//! - `default`: `ansi`, `async`
//! - `file`: file writer + rolling support
//! - `log-compat`: bridge `log` crate events into `tracing`
//! - `observability`: metrics helpers + hook APIs
//! - `telemetry`: OpenTelemetry OTLP tracing
//! - `sentry`: Sentry integration
//! - `full`: enables all major capabilities
//!
//! ## Environment Variables
//!
//! Common runtime controls:
//! - `NEBULA_LOG` or `RUST_LOG`: log level/filter directives
//! - `NEBULA_LOG_FORMAT`: `pretty|compact|json|logfmt`
//! - `NEBULA_LOG_TIME`, `NEBULA_LOG_SOURCE`, `NEBULA_LOG_COLORS`
//! - `NEBULA_SERVICE`, `NEBULA_ENV`, `NEBULA_VERSION`, `NEBULA_INSTANCE`, `NEBULA_REGION`
//!
//! Telemetry/Sentry related:
//! - `OTEL_EXPORTER_OTLP_ENDPOINT`
//! - `SENTRY_DSN`
//! - `SENTRY_ENV`
//! - `SENTRY_RELEASE`
//! - `SENTRY_TRACES_SAMPLE_RATE`
//!
//! ## Main Entry Points
//!
//! - [`auto_init`] for zero-config initialization
//! - [`init`] for default config initialization
//! - [`init_with`] for explicit [`Config`]
//! - [`prelude`] for common imports
//! - [`LoggerBuilder`] for builder-style initialization
//!
//! ## Related API Surface
//!
//! - [`Config`], [`Format`], [`WriterConfig`], [`Level`] for pipeline setup
//! - [`Timer`], [`TimerGuard`], [`Timed`] for timing instrumentation
//! - [`Context`], [`Fields`] for context propagation helpers
//! - [`observability`] module for hook/event integration
//!
//! ## Internal Design Docs
//!
//! For Nebula internal engineering docs, see `crates/log/docs/README.md` in the
//! repository.

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
pub use config::{
    Config, DestinationFailurePolicy, Format, Level, ResolvedConfig, ResolvedSource, Rolling,
    WriterConfig,
};
pub use layer::context::{Context, Fields};
pub use timing::{Timed, Timer, TimerGuard};

// Re-export core types
pub use core::{LogError, LogResult, LogResultExt};

// Re-export telemetry config when the feature is enabled
#[cfg(feature = "telemetry")]
pub use config::TelemetryConfig;

/// Prelude for common imports
pub mod prelude {
    pub use crate::{
        Level, LogError, LogResult, LogResultExt, Timed, Timer, auto_init, debug, error, info,
        init, init_with, instrument, span, trace, warn,
    };

    pub use tracing::{Span, field};

    // Metrics (when observability feature is enabled)
    #[cfg(feature = "observability")]
    pub use crate::metrics::{counter, gauge, histogram, timed_block, timed_block_async};

    // Observability hooks and events
    pub use crate::observability::{
        LoggingHook, ObservabilityEvent, ObservabilityHook, OperationCompleted, OperationFailed,
        OperationStarted, OperationTracker, emit_event, register_hook,
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

    let (guard, source) = LoggerBuilder::build_startup(None)?;
    info!(source = ?source, "logging initialized");
    Ok(guard)
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

/// Initialize for tests (captures logs).
///
/// Safe to call multiple times in test runs: once a dispatcher is already set,
/// this returns a no-op [`LoggerGuard`].
///
/// # Errors
///
/// Returns error if test configuration initialization fails for the same
/// reasons as [`init_with`].
#[cfg(test)]
pub fn init_test() -> LogResult<LoggerGuard> {
    TEST_INIT.get_or_init(|| ());
    if tracing::dispatcher::has_been_set() {
        return Ok(LoggerGuard::noop());
    }
    init_with(Config::test())
}
