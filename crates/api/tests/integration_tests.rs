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
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_data: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(response_data["workflows"].as_array().unwrap().len(), 0);
    assert_eq!(response_data["total"], 0);
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

#[tokio::test]
async fn test_workflow_list_with_data() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create a workflow first
    let create_request = serde_json::json!({
        "name": "Test Workflow",
        "description": "A test workflow",
        "definition": {
            "nodes": [],
            "edges": []
        }
    });

    let app = app::build_app(state.clone(), &api_config);
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

    // Now list workflows
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response_data: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(response_data["workflows"].as_array().unwrap().len(), 1);
    assert_eq!(response_data["total"], 1);
    assert_eq!(response_data["workflows"][0]["name"], "Test Workflow");
}

#[tokio::test]
async fn test_get_workflow_by_id() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create a workflow first
    let create_request = serde_json::json!({
        "name": "Test Workflow",
        "description": "A test workflow",
        "definition": {
            "nodes": [],
            "edges": []
        }
    });

    let app = app::build_app(state.clone(), &api_config);
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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = created_workflow["id"].as_str().unwrap();

    // Now get the workflow by ID
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/workflows/{}", workflow_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(workflow["id"], workflow_id);
    assert_eq!(workflow["name"], "Test Workflow");
    assert_eq!(workflow["description"], "A test workflow");
}

#[tokio::test]
async fn test_get_workflow_not_found() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use a valid UUID format that doesn't exist
    let nonexistent_id = "00000000-0000-0000-0000-000000000000";

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/workflows/{}", nonexistent_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_workflow() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create a workflow first
    let create_request = serde_json::json!({
        "name": "Test Workflow",
        "description": "A test workflow",
        "definition": {
            "nodes": [],
            "edges": []
        }
    });

    let app = app::build_app(state.clone(), &api_config);
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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = created_workflow["id"].as_str().unwrap();

    // Update the workflow
    let update_request = serde_json::json!({
        "name": "Updated Workflow",
        "description": "An updated workflow",
        "definition": {
            "nodes": [{"id": "node1", "type": "action"}],
            "edges": []
        }
    });

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/v1/workflows/{}", workflow_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&update_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let updated_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(updated_workflow["id"], workflow_id);
    assert_eq!(updated_workflow["name"], "Updated Workflow");
    assert_eq!(updated_workflow["description"], "An updated workflow");
    // Note: WorkflowResponse doesn't include the definition field
    // It only includes id, name, description, created_at, updated_at
    assert!(updated_workflow["created_at"].is_number());
    assert!(updated_workflow["updated_at"].is_number());
}

#[tokio::test]
async fn test_delete_workflow() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create a workflow first
    let create_request = serde_json::json!({
        "name": "Test Workflow",
        "description": "A test workflow",
        "definition": {
            "nodes": [],
            "edges": []
        }
    });

    let app = app::build_app(state.clone(), &api_config);
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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = created_workflow["id"].as_str().unwrap();

    // Delete the workflow
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/api/v1/workflows/{}", workflow_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify the workflow is deleted
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/workflows/{}", workflow_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
