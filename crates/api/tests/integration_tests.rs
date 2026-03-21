//! Integration tests for Nebula API

use nebula_api::{ApiConfig, AppState, app};
use nebula_config::ConfigBuilder;
use nebula_storage::{InMemoryExecutionRepo, InMemoryWorkflowRepo};
use std::sync::Arc;

const TEST_JWT_SECRET: &str = "test-secret-for-integration-tests";

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
    AppState::new(config, workflow_repo, execution_repo, TEST_JWT_SECRET)
}

/// Helper to create a valid JWT token for testing
fn create_test_jwt() -> String {
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct Claims {
        sub: String,
        exp: u64,
        iat: u64,
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = Claims {
        sub: "test-user".to_string(),
        exp: now + 3600, // Expires in 1 hour
        iat: now,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
    )
    .unwrap()
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

#[tokio::test]
async fn create_workflow() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Create a workflow
    let create_request = serde_json::json!({
        "name": "Test Workflow",
        "description": "A test workflow",
        "definition": {
            "nodes": [],
            "edges": []
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&create_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // Parse response
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(workflow["name"], "Test Workflow");
    assert_eq!(workflow["description"], "A test workflow");
    assert!(workflow["id"].is_string());
    assert!(workflow["created_at"].is_number());
    assert!(workflow["updated_at"].is_number());
}
