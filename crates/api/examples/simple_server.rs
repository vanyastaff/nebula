use nebula_api::{app, ApiConfig, AppState};
use nebula_config::ConfigBuilder;
use nebula_storage::{InMemoryExecutionRepo, InMemoryWorkflowRepo};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt::init();

	let config = ConfigBuilder::new()
		.with_defaults_json(serde_json::json!({
			"api": {
				"host": "127.0.0.1",
				"port": 8080
			}
		}))
		.build()
		.await?;
	let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
	let execution_repo = Arc::new(InMemoryExecutionRepo::new());

	let state = AppState::new(config, workflow_repo, execution_repo);
	let api_config = ApiConfig::default();
	let app = app::build_app(state, &api_config);

	app::serve(app, api_config.bind_address).await
}
