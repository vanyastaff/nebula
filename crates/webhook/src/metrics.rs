//! Metric name constants for webhook observability.
//!
//! These constants follow the `nebula_webhook_*` naming convention
//! and are intended for use with the OpenTelemetry metrics SDK.

/// Total webhook requests received.
pub const WEBHOOK_RECEIVED_TOTAL: &str = "nebula_webhook_received_total";

/// Total requests that passed signature verification.
pub const WEBHOOK_VERIFIED_TOTAL: &str = "nebula_webhook_verified_total";

/// Total signature verification failures.
pub const WEBHOOK_VERIFICATION_FAILED_TOTAL: &str = "nebula_webhook_verification_failed_total";

/// Total rate-limited requests.
pub const WEBHOOK_RATE_LIMITED_TOTAL: &str = "nebula_webhook_rate_limited_total";
