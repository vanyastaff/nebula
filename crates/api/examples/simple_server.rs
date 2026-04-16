//! Minimal Nebula API server.
//!
//! Set `API_JWT_SECRET` (32+ bytes) before running, or set
//! `NEBULA_ENV=development` to let the loader generate an ephemeral
//! per-process secret. Tokens signed with an ephemeral secret are
//! invalidated on restart.

use std::sync::Arc;

use nebula_api::{ApiConfig, AppState, app};
use nebula_config::ConfigBuilder;
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let config = ConfigBuilder::new()
        .with_defaults(serde_json::json!({
            "api": {
                "host": "127.0.0.1",
                "port": 8080
            }
        }))
        .build()
        .await?;
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    // NOTE: InMemoryControlQueueRepo does not persist across restarts and has
    // no real engine consumer — this server is DEMO ONLY for cancel signals.
    // A production deployment must substitute a Postgres-backed implementation
    // and wire a real dispatcher (canon §12.2, §13 step 5).
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::from_env()?;

    // Wire api_keys from ApiConfig so X-API-Key auth is honoured.
    let state = AppState::new(
        config,
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone());
    let app = app::build_app(state, &api_config);

    app::serve(app, api_config.bind_address).await
}
