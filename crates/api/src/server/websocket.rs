use axum::{Json, Router, http::StatusCode, routing::get};
use serde_json::json;

use super::{ServerTransport, TransportInitError, health_ok};
use crate::{ApiConfig, AppState};

/// Realtime transport placeholder.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealtimeTransport;

impl ServerTransport for RealtimeTransport {
    fn name(&self) -> &'static str {
        "realtime"
    }

    fn bind_override_var(&self) -> Option<&'static str> {
        Some("REALTIME_BIND_ADDRESS")
    }

    fn build_router(
        &self,
        _state: AppState,
        _api_config: &ApiConfig,
    ) -> Result<Router, TransportInitError> {
        Ok(Router::new()
            .route("/health", get(health_ok))
            .route("/ready", get(health_ok))
            .route("/ws", get(ws_not_implemented)))
    }
}

async fn ws_not_implemented() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "realtime transport scaffold is enabled, but websocket upgrade path is not wired yet"
        })),
    )
}
