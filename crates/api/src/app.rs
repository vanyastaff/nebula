//! App/server composition and public server types.

use axum::Router;
use thiserror::Error;

use crate::{models::WorkerStatus, routes, state::ApiState};

/// Configuration for the API server.
#[derive(Debug, Clone)]
pub struct ApiServerConfig {
    /// Bind address (e.g. `0.0.0.0:5678`).
    pub bind_addr: std::net::SocketAddr,
}

impl Default for ApiServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:5678".parse().unwrap(),
        }
    }
}

/// API router.
pub(crate) fn api_router() -> Router<ApiState> {
    routes::api_router()
}

/// Unified API server: holds config and can run the combined app.
pub struct ApiServer {
    #[allow(dead_code)] // reserved for future per-request config use
    config: ApiServerConfig,
}

impl ApiServer {
    /// Create with default config.
    pub fn new(config: ApiServerConfig) -> Self {
        Self { config }
    }

    /// Build the API app for this server.
    pub fn app(&self, workers: Vec<WorkerStatus>) -> Router {
        crate::app(workers)
    }
}

/// Errors from the API server.
#[derive(Debug, Error)]
pub enum ApiError {
    /// IO (bind, serve).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
