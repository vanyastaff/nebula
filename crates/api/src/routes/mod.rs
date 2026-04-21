//! Routes
//!
//! Modular routing by domain.

pub mod catalog;
#[cfg(feature = "credential-oauth")]
pub mod credential;
pub mod execution;
pub mod health;
pub mod metrics;
pub mod workflow;

use axum::{Router, middleware};

use crate::{config::ApiConfig, middleware::auth::auth_middleware, state::AppState};

/// Create main router with all routes
pub fn create_routes(state: AppState, _config: &ApiConfig) -> Router {
    Router::new()
        // Health checks (no auth required)
        .merge(health::router())
        // Prometheus metrics (no auth required — scraper access)
        .merge(metrics::router())
        // API v1 (JWT auth required)
        .nest("/api/v1", api_v1_routes(state.clone()))
        .with_state(state)
}

/// API v1 routes — all protected by JWT auth middleware
fn api_v1_routes(state: AppState) -> Router<AppState> {
    let router = Router::new()
        .merge(workflow::router())
        .merge(execution::router())
        .merge(catalog::router());

    #[cfg(feature = "credential-oauth")]
    let router = router.merge(credential::router());

    router.layer(middleware::from_fn_with_state(state, auth_middleware))
}
