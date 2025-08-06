//! # Nebula Log - Simple and Fast
//!
//! A simple, fast, and beautiful logging library built on tracing.
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_log::Logger;
//!
//! // Simple setup
//! Logger::init();
//!
//! // Development setup with colors and debug level
//! Logger::init_dev();
//!
//! // Production setup with JSON output
//! Logger::init_production();
//!
//! // Custom setup
//! Logger::new()
//!     .level("debug")
//!     .with_colors(true)
//!     .with_source(true)
//!     .init();
//! ```

mod logger;
mod timer;

pub use logger::*;
pub use timer::Timer;

// Re-export tracing macros
pub use tracing::{debug, error, info, trace, warn, instrument, span, Level, Span};