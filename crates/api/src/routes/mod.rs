//! Routes
//!
//! Модульная маршрутизация по доменам.

pub mod health;
pub mod workflow;
pub mod execution;

use axum::Router;
use crate::state::AppState;

/// Create main router with all routes
pub fn create_routes() -> Router<AppState> {
    Router::new()
        // Health checks (no auth required)
        .merge(health::router())
        
        // API v1
        .nest("/api/v1", api_v1_routes())
}

/// API v1 routes
fn api_v1_routes() -> Router<AppState> {
    Router::new()
        .merge(workflow::router())
        .merge(execution::router())
}

