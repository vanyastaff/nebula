//! # Nebula API Server
//!
//! Unified HTTP server that exposes:
//! - **API** — status, workers, health for the UI and tooling
//! - **Webhook** — same process, one port (embedded webhook router)
//!
//! **Одна точка входа (Docker / local):** в `main` через `tokio::spawn` запускаются N воркеров
//! (цикл: очередь → выполнение → ack), затем на одном порту поднимается HTTP (API + webhook).
//! Один процесс = один бинарник в одном контейнере. См. [architecture](docs/architecture.md).

#![warn(missing_docs)]
#![warn(clippy::all)]

mod server;
mod status;

pub use server::{ApiError, ApiServer, ApiServerConfig};
pub use status::{WebhookStatus, WorkerStatus};
use std::sync::Arc;
use tokio::net::TcpListener;

use axum::Router;
use nebula_webhook::WebhookServer;
use tracing::info;

/// Build the combined application: API routes + embedded webhook router.
///
/// - `GET /health` — liveness
/// - `GET /api/v1/status` — workers + webhook status
/// - `POST /webhooks/*` — webhook endpoints (from embedded webhook server)
pub fn app(webhook_server: Arc<WebhookServer>, workers: Vec<WorkerStatus>) -> Router {
    let state = server::ApiState {
        webhook: webhook_server.clone(),
        workers,
    };
    server::api_router()
        .with_state(state)
        .merge(webhook_server.router())
}

/// Run the unified API server (4 workers + webhook) on the given listener.
///
/// Uses fixed snapshot of 4 workers for status; replace with dynamic pool later if needed.
pub async fn run(
    config: ApiServerConfig,
    webhook_config: nebula_webhook::WebhookServerConfig,
    workers: Vec<WorkerStatus>,
) -> Result<(), ApiError> {
    let webhook = WebhookServer::new_embedded(webhook_config)?;
    let listener = TcpListener::bind(&config.bind_addr).await?;
    let addr = listener.local_addr()?;
    info!(%addr, "Nebula API server listening (API + webhooks on one port)");

    let app = app(webhook, workers);
    axum::serve(listener, app).await?;
    Ok(())
}
