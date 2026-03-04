//! # Nebula API Server
//!
//! Unified HTTP server that exposes:
//! - **API** — status, workers, health for the UI and tooling
//!
//! **Одна точка входа (Docker / local):** в `main` через `tokio::spawn` запускаются N воркеров
//! (цикл: очередь → выполнение → ack), затем на одном порту поднимается HTTP API.
//! См. [architecture](docs/architecture.md).

#![warn(missing_docs)]
#![warn(clippy::all)]

mod app;
mod auth;
mod config;
mod errors;
mod extractors;
mod handlers;
mod middleware;
pub mod models;
mod routes;
mod services;
mod state;

pub use app::{ApiError, ApiServer, ApiServerConfig};
pub use models::WorkerStatus;
pub use state::ApiState;
use tokio::net::TcpListener;

use axum::Router;
use tracing::info;

/// Build the API application.
///
/// - `GET /health` — liveness
/// - `GET /api/v1/status` — workers status snapshot
pub fn app(workers: Vec<WorkerStatus>) -> Router {
    let state = ApiState::new(workers);
    app_with_state(state)
}

/// Build the combined application from a fully configured [`ApiState`].
///
/// This is the preferred entry point for dependency injection in tests and
/// in host binaries that compose API ports explicitly.
pub fn app_with_state(state: ApiState) -> Router {
    app::api_router().with_state(state)
}

/// Backward-compatible alias for API-only app construction from full state.
pub fn api_only_app_with_state(state: ApiState) -> Router {
    app_with_state(state)
}

/// Build API application from workers snapshot.
pub fn api_only_app(workers: Vec<WorkerStatus>) -> Router {
    app(workers)
}

/// Run the API server on the given listener.
///
/// Uses fixed snapshot of 4 workers for status; replace with dynamic pool later if needed.
pub async fn run(config: ApiServerConfig, workers: Vec<WorkerStatus>) -> Result<(), ApiError> {
    let listener = TcpListener::bind(&config.bind_addr).await?;
    let addr = listener.local_addr()?;
    info!(%addr, "Nebula API server listening");

    let app = app(workers);
    axum::serve(listener, app).await?;
    Ok(())
}
