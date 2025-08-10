//! Telemetry integrations

#[cfg(feature = "telemetry")]
pub mod otel;

#[cfg(feature = "sentry")]
pub mod sentry;