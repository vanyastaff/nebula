//! Integration tests for Nebula API

use std::sync::Arc;

use nebula_api::{ApiConfig, AppState, app};
use nebula_config::ConfigBuilder;
use nebula_storage::{InMemoryExecutionRepo, InMemoryWorkflowRepo};

const TEST_JWT_SECRET: &str = "test-secret-for-integration-tests-0123456789";

/// Helper to create test app state
async fn create_test_state() -> AppState {
    let config = ConfigBuilder::new()
        .with_defaults(serde_json::json!({
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
    let api_config = ApiConfig::for_test();
    AppState::new(
        config,
        workflow_repo,
        execution_repo,
        api_config.jwt_secret.clone(),
    )
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
    let api_config = ApiConfig::for_test();
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
    let api_config = ApiConfig::for_test();
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
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Test 1: Not Found (404) - workflow not found
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/workflows/{}",
                    nebula_core::WorkflowId::new()
                ))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Verify Content-Type header is application/problem+json
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/problem+json"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify RFC 9457 required fields
    assert!(problem["type"].is_string());
    assert_eq!(problem["type"], "https://nebula.dev/problems/not-found");
    assert!(problem["title"].is_string());
    assert_eq!(problem["title"], "Not Found");
    assert!(problem["status"].is_number());
    assert_eq!(problem["status"], 404);
    assert!(problem["detail"].is_string());

    // Test 2: Bad Request (400) - invalid UUID format
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows/invalid-uuid")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/problem+json"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify RFC 9457 structure
    assert!(problem["type"].is_string());
    assert!(problem["title"].is_string());
    assert!(problem["status"].is_number());
    assert_eq!(problem["status"], 400);

    // Test 3: Validation Error (400) - cancel already completed execution
    use nebula_core::{ExecutionId, WorkflowId};

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
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
        .create(execution_id, workflow_id, execution_state.clone())
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/executions/{}/cancel", execution_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/problem+json"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify RFC 9457 structure for validation errors
    assert!(problem["type"].is_string());
    assert_eq!(
        problem["type"],
        "https://nebula.dev/problems/validation-error"
    );
    assert!(problem["title"].is_string());
    assert_eq!(problem["title"], "Validation Error");
    assert!(problem["status"].is_number());
    assert_eq!(problem["status"], 400);
    assert!(problem["detail"].is_string());

    // For validation errors, check if "errors" field is present with field-level details
    if problem["errors"].is_array() {
        let errors = problem["errors"].as_array().unwrap();
        for error in errors {
            // Each validation error should have code, detail, and pointer (RFC 6901)
            assert!(error["code"].is_string());
            assert!(error["detail"].is_string());
            assert!(error["pointer"].is_string());
            // Pointer should follow RFC 6901 (JSON Pointer) format
            assert!(error["pointer"].as_str().unwrap().starts_with('/'));
        }
    }
}

#[tokio::test]
async fn create_workflow() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
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
    let api_config = ApiConfig::for_test();
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

/// `total` must reflect the full dataset size, not the current page length (issue #342).
#[tokio::test]
async fn test_workflow_list_total_matches_full_dataset_across_pages() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    for i in 0..3 {
        let create_request = serde_json::json!({
            "name": format!("Pagination Total {i}"),
            "definition": { "nodes": [], "edges": [] }
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
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "create workflow {i}"
        );
    }

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows?page=1&page_size=2")
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
    let page1: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page1["workflows"].as_array().unwrap().len(), 2);
    assert_eq!(page1["total"], 3);

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows?page=2&page_size=2")
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
    let page2: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page2["workflows"].as_array().unwrap().len(), 1);
    assert_eq!(page2["total"], 3);
}

#[tokio::test]
async fn test_get_workflow_by_id() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}", workflow_id))
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
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use a valid UUID format that doesn't exist
    let nonexistent_id = nebula_core::WorkflowId::new().to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/workflows/{}", nonexistent_id))
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
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}", workflow_id))
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
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}", workflow_id))
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
                .uri(format!("/api/v1/workflows/{}", workflow_id))
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
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}/activate", workflow_id))
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
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use a valid UUID format that doesn't exist
    let nonexistent_id = nebula_core::WorkflowId::new().to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/workflows/{}/activate", nonexistent_id))
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
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}/execute", workflow_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&execute_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Verify 202 ACCEPTED response
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Verify response body contains execution data
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(execution_response["id"].is_string());
    assert_eq!(
        execution_response["workflow_id"].as_str().unwrap(),
        workflow_id
    );
    assert_eq!(execution_response["status"].as_str().unwrap(), "pending");
}

#[tokio::test]
async fn test_execute_workflow_not_found() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use a valid UUID format that doesn't exist
    let nonexistent_id = nebula_core::WorkflowId::new().to_string();

    let execute_request = serde_json::json!({
        "input": null
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/workflows/{}/execute", nonexistent_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&execute_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Verify 404 NOT_FOUND for non-existent workflow
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
    let api_config = ApiConfig::for_test();
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
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}/executions", workflow_id))
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
    use nebula_core::{ExecutionId, WorkflowId};
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Create an execution directly in the repo
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let execution_state = serde_json::json!({
        "workflow_id": workflow_id.to_string(),
        "status": "running",
        "started_at": now,
        "input": {"key": "value"}
    });

    state
        .execution_repo
        .create(execution_id, workflow_id, execution_state.clone())
        .await
        .unwrap();

    // Get the execution by ID
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/executions/{}", execution_id))
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
    assert_eq!(execution["workflow_id"], workflow_id.to_string());
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
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use a valid ULID format that doesn't exist
    let nonexistent_id = nebula_core::ExecutionId::new().to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/executions/{}", nonexistent_id))
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
    let api_config = ApiConfig::for_test();
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
                .uri(format!("/api/v1/workflows/{}/executions", workflow_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::from(serde_json::to_string(&start_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Verify 202 ACCEPTED response
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Verify response body contains execution data
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(execution_response["id"].is_string());
    assert_eq!(execution_response["status"].as_str().unwrap(), "pending");
}

#[tokio::test]
async fn test_execution_cancel() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::{ExecutionId, WorkflowId};
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Create an execution directly in the repo
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let execution_state = serde_json::json!({
        "workflow_id": workflow_id.to_string(),
        "status": "running",
        "started_at": now,
        "input": {"key": "value"}
    });

    state
        .execution_repo
        .create(execution_id, workflow_id, execution_state.clone())
        .await
        .unwrap();

    // Cancel the execution
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/executions/{}/cancel", execution_id))
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
                .uri(format!("/api/v1/executions/{}", execution_id))
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
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use a valid ULID format that doesn't exist
    let nonexistent_id = nebula_core::ExecutionId::new().to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/executions/{}/cancel", nonexistent_id))
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
    use nebula_core::{ExecutionId, WorkflowId};
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Create a completed execution directly in the repo
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let execution_state = serde_json::json!({
        "workflow_id": workflow_id.to_string(),
        "status": "completed",
        "started_at": now,
        "finished_at": now + 10,
        "input": {"key": "value"},
        "output": {"result": "success"}
    });

    state
        .execution_repo
        .create(execution_id, workflow_id, execution_state.clone())
        .await
        .unwrap();

    // Try to cancel the completed execution
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/executions/{}/cancel", execution_id))
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
    assert!(
        error["detail"]
            .as_str()
            .unwrap()
            .contains("Cannot cancel execution")
    );
}

#[tokio::test]
async fn test_execution_get_invalid_id() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // Use an invalid UUID format
    let invalid_id = "not-a-valid-uuid";

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/executions/{}", invalid_id))
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Rate limiting tests ──────────────────────────────────────────────────────

/// Verify that requests within the per-IP limit are allowed through.
#[tokio::test]
async fn rate_limit_allows_requests_within_quota() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    // Set a generous limit so a single test request is never throttled.
    let api_config = ApiConfig {
        rate_limit_per_second: 100,
        ..ApiConfig::for_test()
    };
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

/// Verify that `/health` is never rate-limited, even with a 1 req/s quota and
/// many rapid requests.
#[tokio::test]
async fn rate_limit_excludes_health_path() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    // Minimal quota — would throttle any non-excluded path immediately.
    let api_config = ApiConfig {
        rate_limit_per_second: 1,
        ..ApiConfig::for_test()
    };

    // Send more requests than the quota allows; all must succeed because /health
    // is on the exclusion list.
    for _ in 0..5 {
        let app = app::build_app(state.clone(), &api_config);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "/health must never be rate-limited"
        );
    }
}

/// Verify that `/ready` is never rate-limited.
#[tokio::test]
async fn rate_limit_excludes_ready_path() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig {
        rate_limit_per_second: 1,
        ..ApiConfig::for_test()
    };

    for _ in 0..5 {
        let app = app::build_app(state.clone(), &api_config);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "/ready must never be rate-limited"
        );
    }
}

/// Verify that exceeding the per-IP quota returns 429 with a `Retry-After` header.
#[tokio::test]
async fn rate_limit_returns_429_when_quota_exceeded() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };

    let state = create_test_state().await;
    // 1 req/s burst — the second request from the same IP must be throttled.
    let api_config = ApiConfig {
        rate_limit_per_second: 1,
        ..ApiConfig::for_test()
    };

    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // `into_service::<Body>()` produces a cloneable service; clones share the same Arc-backed
    // rate-limiter map, so both requests hit the same per-IP counter.
    let svc = app.into_service::<Body>();

    // First request — should be allowed (burst of 1 at 1 req/s).
    let first_response = tower::ServiceExt::oneshot(
        svc.clone(),
        Request::builder()
            .uri("/api/v1/workflows")
            .header("x-real-ip", "10.0.0.99")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    // Second immediate request from the same IP must be throttled.
    let second_response = tower::ServiceExt::oneshot(
        svc,
        Request::builder()
            .uri("/api/v1/workflows")
            .header("x-real-ip", "10.0.0.99")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(first_response.status(), StatusCode::OK);
    assert_eq!(second_response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        second_response.headers().contains_key("retry-after"),
        "429 response must include Retry-After header"
    );
}

/// Verify that two different IPs do not share quota.
#[tokio::test]
async fn rate_limit_is_per_ip() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };

    let state = create_test_state().await;
    let api_config = ApiConfig {
        rate_limit_per_second: 1,
        ..ApiConfig::for_test()
    };

    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let svc = app.into_service::<Body>();

    // First IP — first request (within quota)
    let r1 = tower::ServiceExt::oneshot(
        svc.clone(),
        Request::builder()
            .uri("/api/v1/workflows")
            .header("x-real-ip", "10.0.0.1")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    // Second IP — first request (within its own separate quota)
    let r2 = tower::ServiceExt::oneshot(
        svc,
        Request::builder()
            .uri("/api/v1/workflows")
            .header("x-real-ip", "10.0.0.2")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(r1.status(), StatusCode::OK, "first IP first request");
    assert_eq!(r2.status(), StatusCode::OK, "second IP first request");
}
// ── #320: CORS preflight regression tests ──────────────────────────────

/// Preflight for `X-API-Key` must succeed so browser clients using
/// API key auth can reach protected routes. Regression for #320.
#[tokio::test]
async fn cors_preflight_allows_x_api_key() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/workflows")
                .header("origin", "https://app.example")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "x-api-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let allow_headers = response
        .headers()
        .get("access-control-allow-headers")
        .expect("preflight response must include Access-Control-Allow-Headers")
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        allow_headers.contains("x-api-key"),
        "x-api-key must appear in allow-headers, got: {allow_headers}"
    );
}

/// Regression lock for the pre-existing `Authorization` header
/// allowance. Kept alongside #320's addition so future CORS edits
/// cannot silently drop the Bearer-token path either.
#[tokio::test]
async fn cors_preflight_allows_authorization() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/workflows")
                .header("origin", "https://app.example")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let allow_headers = response
        .headers()
        .get("access-control-allow-headers")
        .expect("preflight response must include Access-Control-Allow-Headers")
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        allow_headers.contains("authorization"),
        "authorization must appear in allow-headers, got: {allow_headers}"
    );
}

#[tokio::test]
async fn update_workflow_rejects_immutable_identity_fields() {
    // Regression for #344: update_workflow must refuse payloads that try to
    // overwrite identity/control fields (id, version, owner_id, schema_version)
    // smuggled inside the nested `definition` object.
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Create a workflow to update.
    let create_request = serde_json::json!({
        "name": "Identity Guard",
        "description": "immutable-fields regression",
        "definition": { "nodes": [], "edges": [] }
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
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = created["id"].as_str().unwrap().to_string();

    // Every protected field must cause a 4xx rejection.
    for field in ["id", "version", "owner_id", "schema_version"] {
        // NOTE: in `serde_json::json!`, bare identifiers on the left of `:`
        // are stringified literally. Wrap the loop variable in parentheses
        // so the macro interpolates the *value* ("id", "version", ...) as
        // the object key, otherwise every iteration would test the same
        // literal key `"field"`. (Flagged by Copilot on PR #406.)
        let mut inner = serde_json::Map::new();
        inner.insert(field.to_string(), serde_json::json!("attacker-supplied"));
        let update_request = serde_json::json!({
            "definition": serde_json::Value::Object(inner),
        });
        let app = app::build_app(state.clone(), &api_config);
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/workflows/{workflow_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::from(serde_json::to_string(&update_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            response.status().is_client_error(),
            "update with `{field}` must be rejected, got status {}",
            response.status()
        );
    }
}

#[tokio::test]
async fn get_workflow_parses_rfc3339_timestamps() {
    // Regression for #343: a stored workflow whose timestamps are RFC3339
    // strings (canonical `WorkflowDefinition` shape) must round-trip through
    // the API without collapsing to 0.
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::WorkflowId;
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Write directly through the repo with canonical string timestamps so we
    // skip the create_workflow handler's i64 write path and exercise the read
    // path against exactly the shape that `WorkflowDefinition` serializes.
    let workflow_id = WorkflowId::new();
    let canonical = serde_json::json!({
        "name": "Canonical Timestamps",
        "description": "rfc3339-regression",
        "created_at": "2024-01-15T12:34:56Z",
        "updated_at": "2024-02-20T08:00:00Z",
    });
    state
        .workflow_repo
        .save(workflow_id, 0, canonical)
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/workflows/{}", workflow_id))
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
    assert_eq!(workflow["created_at"].as_i64(), Some(1_705_322_096));
    assert_eq!(workflow["updated_at"].as_i64(), Some(1_708_416_000));
}
