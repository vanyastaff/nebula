//! Prometheus metrics endpoint.
//!
//! Returned as a plain `OpenApiRouter` (no `#[utoipa::path]`) because the
//! Prometheus text format is consumed by scrapers, not by HTTP API clients
//! — the spec deliberately omits this endpoint.

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing};
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// GET /metrics -- Prometheus text format.
///
/// Returns 503 if no metrics registry is configured.
async fn prometheus_handler(State(state): State<AppState>) -> impl IntoResponse {
    match &state.metrics_registry {
        Some(registry) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                nebula_metrics::content_type(),
            )],
            nebula_metrics::snapshot(registry),
        ),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            "Metrics not configured".to_string(),
        ),
    }
}

/// Metrics router -- unauthenticated (Prometheus scrapes without auth).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().route("/metrics", routing::get(prometheus_handler))
}
