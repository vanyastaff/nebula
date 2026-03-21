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

#[tokio::test]
async fn test_activate_workflow() {
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

    // Activate the workflow
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/v1/workflows/{}/activate", workflow_id))
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
    let activated_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(activated_workflow["id"], workflow_id);
    assert_eq!(activated_workflow["name"], "Test Workflow");
    assert!(activated_workflow["created_at"].is_number());
    assert!(activated_workflow["updated_at"].is_number());
}

#[tokio::test]
async fn test_activate_workflow_not_found() {
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
                .method("POST")
                .uri(&format!("/api/v1/workflows/{}/activate", nonexistent_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_execute_workflow() {
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

    // Execute the workflow with input data
    let execute_request = serde_json::json!({
        "input": {
            "key": "value"
        }
    });

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/v1/workflows/{}/execute", workflow_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&execute_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Note: Currently the execute endpoint is not fully implemented
    // so we expect an internal server error. Once implemented, this should
    // be changed to expect StatusCode::ACCEPTED (202)
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_execute_workflow_not_found() {
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

    let execute_request = serde_json::json!({
        "input": null
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/v1/workflows/{}/execute", nonexistent_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&execute_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Note: Currently returns 500 because execute is not implemented
    // When implemented, this should verify 404 NOT_FOUND for non-existent workflow
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ===== Execution Management Tests =====

#[tokio::test]
async fn test_execution_list_all_empty() {
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
                .uri("/api/v1/executions")
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

    assert_eq!(response_data["executions"].as_array().unwrap().len(), 0);
    assert_eq!(response_data["total"], 0);
    assert_eq!(response_data["page"], 1);
    assert_eq!(response_data["page_size"], 10);
}

#[tokio::test]
async fn test_execution_list_for_workflow_empty() {
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

    // List executions for the workflow
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/workflows/{}/executions", workflow_id))
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

    assert_eq!(response_data["executions"].as_array().unwrap().len(), 0);
    assert_eq!(response_data["total"], 0);
}

#[tokio::test]
async fn test_execution_get_by_id() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::ExecutionId;
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create an execution directly in the repo
    let execution_id = ExecutionId::new();
    let workflow_id = "test-workflow-id";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let execution_state = serde_json::json!({
        "workflow_id": workflow_id,
        "status": "running",
        "started_at": now,
        "input": {"key": "value"}
    });

    state
        .execution_repo
        .transition(execution_id, 0, execution_state.clone())
        .await
        .unwrap();

    // Get the execution by ID
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/executions/{}", execution_id))
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
    let execution: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(execution["id"], execution_id.to_string());
    assert_eq!(execution["workflow_id"], workflow_id);
    assert_eq!(execution["status"], "running");
    assert_eq!(execution["started_at"], now);
    assert_eq!(execution["input"]["key"], "value");
}

#[tokio::test]
async fn test_execution_get_not_found() {
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
                .uri(&format!("/api/v1/executions/{}", nonexistent_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_execution_start() {
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

    // Start an execution
    let start_request = serde_json::json!({
        "input": {
            "test_key": "test_value"
        }
    });

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/v1/workflows/{}/executions", workflow_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&start_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Note: Currently the start endpoint is not fully implemented
    // so we expect an internal server error. Once implemented, this should
    // be changed to expect StatusCode::ACCEPTED (202)
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_execution_cancel() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::ExecutionId;
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create an execution directly in the repo
    let execution_id = ExecutionId::new();
    let workflow_id = "test-workflow-id";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let execution_state = serde_json::json!({
        "workflow_id": workflow_id,
        "status": "running",
        "started_at": now,
        "input": {"key": "value"}
    });

    state
        .execution_repo
        .transition(execution_id, 0, execution_state.clone())
        .await
        .unwrap();

    // Cancel the execution
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/v1/executions/{}/cancel", execution_id))
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
    let cancelled_execution: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(cancelled_execution["id"], execution_id.to_string());
    assert_eq!(cancelled_execution["status"], "cancelled");
    assert!(cancelled_execution["finished_at"].is_number());

    // Verify the execution was actually cancelled in the repo
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/executions/{}", execution_id))
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
    let execution: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(execution["status"], "cancelled");
}

#[tokio::test]
async fn test_execution_cancel_not_found() {
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
                .method("POST")
                .uri(&format!("/api/v1/executions/{}/cancel", nonexistent_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_execution_cancel_already_completed() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::ExecutionId;
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let token = create_test_jwt();

    // Create a completed execution directly in the repo
    let execution_id = ExecutionId::new();
    let workflow_id = "test-workflow-id";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let execution_state = serde_json::json!({
        "workflow_id": workflow_id,
        "status": "completed",
        "started_at": now,
        "finished_at": now + 10,
        "input": {"key": "value"},
        "output": {"result": "success"}
    });

    state
        .execution_repo
        .transition(execution_id, 0, execution_state.clone())
        .await
        .unwrap();

    // Try to cancel the completed execution
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/v1/executions/{}/cancel", execution_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should fail with a validation error
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Check that the detail field contains the error message
    assert!(error["detail"]
        .as_str()
        .unwrap()
        .contains("Cannot cancel execution"));
}

#[tokio::test]
async fn test_execution_get_invalid_id() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::default();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use an invalid UUID format
    let invalid_id = "not-a-valid-uuid";

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/v1/executions/{}", invalid_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

