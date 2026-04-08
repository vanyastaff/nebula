use nebula_api::{ApiConfig, AppState, app};
use nebula_config::ConfigBuilder;
use nebula_storage::{InMemoryExecutionRepo, InMemoryWorkflowRepo};
use std::sync::Arc;

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
    let api_config = ApiConfig::default();

    // Wire api_keys from ApiConfig so X-API-Key auth is honoured.
    let state = AppState::new(
        config,
        workflow_repo,
        execution_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone());
    let app = app::build_app(state, &api_config);

    app::serve(app, api_config.bind_address).await
}
