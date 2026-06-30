//! Telemetry integrations

#[cfg(feature = "telemetry")]
pub(crate) mod otel;

#[cfg(feature = "sentry")]
pub(crate) mod sentry;
