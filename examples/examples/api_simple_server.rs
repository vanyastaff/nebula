//! Minimal `nebula-api` library startup demo.
//!
//! Builds an `AppState` from in-memory repos, loads `ApiConfig` from the
//! environment, assembles the axum router via `build_app`, and starts
//! listening. Ctrl-C / SIGTERM trigger graceful shutdown.
//!
//! ## Prerequisites
//!
//! Set `API_JWT_SECRET` to a 32+ byte value before running, or set
//! `NEBULA_ENV=development` to let the loader generate an ephemeral
//! per-process secret (tokens are invalidated on restart in that mode).
//!
//! ## Run
//!
//! ```shell
//! API_JWT_SECRET="at-least-32-bytes-for-hs256-signing!" \
//!   cargo run -p nebula-examples --example api_simple_server
//! ```
//!
//! The server binds to `0.0.0.0:3000` by default (`API_BIND_ADDRESS` overrides
//! this). All routes live under `/api/v1/…`; the OpenAPI spec is served at
//! `GET /api/v1/openapi.json` and browsable via `GET /api/v1/docs/`.
//!
//! ## In-memory repos
//!
//! This example uses in-memory repos (`InMemoryWorkflowRepo`,
//! `InMemoryExecutionRepo`, `InMemoryControlQueueRepo`). State is lost on
//! restart. For a production deployment wire in the Postgres-backed repos from
//! `nebula-storage` and enable the `nebula-api/postgres` feature — that
//! wiring lives in the `apps/server` composition root, not here.

use std::sync::Arc;

use nebula_api::{ApiConfig, AppState, app, middleware::InMemoryIdempotencyStore};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let api_config = ApiConfig::from_env()?;

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());

    let idempotency_store = Arc::new(InMemoryIdempotencyStore::default());

    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
    .with_idempotency_store(idempotency_store);

    let bind_address = api_config.bind_address;
    let app = app::build_app(state, &api_config);

    // `app::serve` installs a built-in Ctrl-C / SIGTERM graceful-shutdown handler.
    app::serve(app, bind_address).await?;

    Ok(())
}
