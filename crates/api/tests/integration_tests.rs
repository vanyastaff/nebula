//! Integration tests for Nebula API

use nebula_api::{ApiConfig, AppState, app};
use nebula_config::ConfigBuilder;
use nebula_storage::{InMemoryExecutionRepo, InMemoryWorkflowRepo};
use std::sync::Arc;

/// Helper to create test app state
async fn create_test_state() -> AppState {
    let config = ConfigBuilder::new()
        .with_defaults_json(serde_json::json!({
            "api": {
                "port": 8080,
                "host": "127.0.0.1"
            }
        }))
        .build()
        .await
        .unwrap();
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    AppState::new(config, workflow_repo, execution_repo)
}

#[tokio::test]
async fn test_health_endpoint() {
    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let _app = app::build_app(state, &api_config);

    // TODO: Use axum test helpers to test endpoints
    // let response = app.oneshot(Request::builder()
    //     .uri("/health")
    //     .body(Body::empty())
    //     .unwrap())
    //     .await
    //     .unwrap();

    // assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_workflow_list_empty() {
    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let _app = app::build_app(state, &api_config);

    // TODO: Test /api/v1/workflows returns empty list
}

#[tokio::test]
async fn test_error_format_rfc9457() {
    // TODO: Test that errors return RFC 9457 Problem Details format
}
