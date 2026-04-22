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
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    // DEMO ONLY — does not honor start / cancel (canon §12.2, ADR-0008 §4).
    //
    // ADR-0008 A2 + A3 landed `nebula_engine::EngineControlDispatch` with
    // full Start / Resume / Restart / Cancel / Terminate dispatch wired into
    // the engine, so a production composition root can now drop this file's
    // repos into an `AppState` and spawn a real `ControlConsumer` bound to a
    // `WorkflowEngine`. This `simple_server.rs` intentionally does not pull
    // in the full engine stack (plugin registry, action runtime, sandbox,
    // metrics, credential / resource managers) — the dedicated `apps/server`
    // composition root is still tracked as a separate follow-up. Until then
    // this example stays DEMO ONLY: `POST /executions` persists the row and
    // enqueues `Start`, but with no consumer wired here the row never
    // transitions to `Running`. `crates/api/tests/knife.rs` exercises the
    // full producer → consumer → engine path end-to-end for both Start
    // (step 3) and Cancel (step 5) regression coverage.
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

    app::serve(app, api_config.bind_address).await?;
    Ok(())
}
