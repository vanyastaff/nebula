//! Health check handlers

use crate::models::{DependenciesStatus, HealthResponse, ReadinessResponse};
use axum::{extract::State, Json};
use chrono::Utc;
use crate::state::AppState;

/// Health check endpoint
/// GET /health
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now().timestamp(),
    })
}

/// Readiness check endpoint
/// GET /ready
pub async fn readiness_check(State(_state): State<AppState>) -> Json<ReadinessResponse> {
    // TODO: Check actual dependencies (DB, cache, etc.)
    Json(ReadinessResponse {
        ready: true,
        dependencies: DependenciesStatus {
            database: true,
            cache: None,
        },
    })
}


