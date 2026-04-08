//! Metric name constants for webhook observability.
//!
//! These constants follow the `nebula_webhook_*` naming convention
//! and are intended for registration and emission through
//! `nebula_telemetry::metrics::MetricsRegistry`, with names aligned to
//! the shared `nebula-metrics` conventions used across the workspace.
//!
//! # Wiring
//!
//! These constants are string keys only — integration with a metrics registry
//! (e.g. `nebula-metrics` or OpenTelemetry) is the responsibility of the
//! embedding application.  The webhook crate increments them via `tracing`
//! events today; future work will wire them through `nebula_telemetry`.

/// Total webhook requests received.
pub const WEBHOOK_RECEIVED_TOTAL: &str = "nebula_webhook_received_total";

/// Total requests that passed signature verification.
pub const WEBHOOK_VERIFIED_TOTAL: &str = "nebula_webhook_verified_total";

/// Total signature verification failures.
pub const WEBHOOK_VERIFICATION_FAILED_TOTAL: &str = "nebula_webhook_verification_failed_total";

/// Total rate-limited requests.
pub const WEBHOOK_RATE_LIMITED_TOTAL: &str = "nebula_webhook_rate_limited_total";

/// Total events enqueued to the durable inbound queue.
pub const WEBHOOK_QUEUED_TOTAL: &str = "nebula_webhook_queued_total";

/// Total events successfully processed (dispatched to subscribers).
pub const WEBHOOK_PROCESSED_TOTAL: &str = "nebula_webhook_processed_total";

/// Total outbound webhook delivery attempts.
pub const WEBHOOK_DELIVERY_TOTAL: &str = "nebula_webhook_delivery_total";

/// Total outbound webhook delivery failures (after all retries).
pub const WEBHOOK_DELIVERY_FAILED_TOTAL: &str = "nebula_webhook_delivery_failed_total";

/// Lag between event receipt and queue processing, in seconds.
///
/// Used as a histogram / gauge name; the embedding application decides
/// the instrument type appropriate for its metrics backend.
pub const WEBHOOK_QUEUE_LAG_SECONDS: &str = "nebula_webhook_queue_lag_seconds";
