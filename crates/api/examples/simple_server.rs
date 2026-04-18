//! Minimal Nebula API server.
//!
//! Set `API_JWT_SECRET` (32+ bytes) before running, or set
//! `NEBULA_ENV=development` to let the loader generate an ephemeral
//! per-process secret. Tokens signed with an ephemeral secret are
//! invalidated on restart.

use std::sync::Arc;

use nebula_api::{ApiConfig, AppState, app};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    // DEMO ONLY — does not honor cancel/start (canon §12.2).
    //
    // This example wires the API producer side of `execution_control_queue`
    // but does NOT construct a `WorkflowEngine` or spawn a
    // `nebula_engine::ControlConsumer`, so enqueued commands are never
    // dispatched. Use ADR-0008's consumer skeleton from a production
    // composition root once A2 / A3 land the dispatch paths.
    // `InMemoryControlQueueRepo` also does not persist across restarts; a
    // real deployment additionally requires a Postgres-backed repo.
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::from_env()?;

    // Wire api_keys from ApiConfig so X-API-Key auth is honoured.
    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone());
    let app = app::build_app(state, &api_config);

    app::serve(app, api_config.bind_address).await
}
