//! Configuration types and builders
//!
//! This module provides configuration types for the logging system, organized into:
//! - `base`: Core configuration structs (Config, Format, Level)
//! - `writer`: Writer and display configuration
//! - `fields`: Global fields configuration
//! - `presets`: Pre-configured setups (development, production, test)

mod base;
mod fields;
mod presets;
mod writer;

// Re-export all public types
#[cfg(feature = "telemetry")]
pub use base::TelemetryConfig;
pub use base::{Config, Format, Level};
pub use fields::Fields;
pub use writer::{DisplayConfig, Rolling, WriterConfig};
