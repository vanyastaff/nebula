//! Integration tests for Nebula API

mod common;
use std::sync::Arc;

use common::*;
use nebula_api::{ApiConfig, AppState, app};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};

/// Helper to create test app state
async fn create_test_state() -> AppState {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::for_test();
    AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
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
                .uri(ws_path("/workflows"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{}", WorkflowId::new())))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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

    // Test 2: Not Found (404) - invalid UUID format (caught by tenancy middleware)
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/workflows/invalid-uuid"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

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
    assert_eq!(problem["status"], 404);

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
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                    .uri(ws_path("/workflows"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows?page=1&page_size=2"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows?page=2&page_size=2"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{nonexistent_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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

    // Write a structurally valid WorkflowDefinition directly to the repo so
    // activate_workflow can parse and validate it successfully.
    let workflow_id = nebula_core::WorkflowId::new();
    state
        .workflow_repo
        .save(workflow_id, 0, make_valid_workflow_definition(&workflow_id))
        .await
        .unwrap();

    // Activate the workflow
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id}/activate")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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

    assert_eq!(activated_workflow["id"], workflow_id.to_string());
    assert_eq!(activated_workflow["name"], "Valid Workflow");
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
                .uri(ws_path(&format!("/workflows/{nonexistent_id}/activate")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}/execute")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
    assert_eq!(execution_response["status"].as_str().unwrap(), "created");
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
                .uri(ws_path(&format!("/workflows/{nonexistent_id}/execute")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/executions"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}/executions")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/executions/{nonexistent_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}/executions")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
    assert_eq!(execution_response["status"].as_str().unwrap(), "created");
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
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{nonexistent_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/executions/{invalid_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
            .uri(ws_path("/workflows"))
            .header("x-real-ip", "10.0.0.99")
            .header("authorization", format!("Bearer {token}"))
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    // Second immediate request from the same IP must be throttled.
    let second_response = tower::ServiceExt::oneshot(
        svc,
        Request::builder()
            .uri(ws_path("/workflows"))
            .header("x-real-ip", "10.0.0.99")
            .header("authorization", format!("Bearer {token}"))
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
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
            .uri(ws_path("/workflows"))
            .header("x-real-ip", "10.0.0.1")
            .header("authorization", format!("Bearer {token}"))
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    // Second IP — first request (within its own separate quota)
    let r2 = tower::ServiceExt::oneshot(
        svc,
        Request::builder()
            .uri(ws_path("/workflows"))
            .header("x-real-ip", "10.0.0.2")
            .header("authorization", format!("Bearer {token}"))
            .header("x-csrf-token", TEST_CSRF_TOKEN)
            .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path("/workflows"))
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
                .uri(ws_path("/workflows"))
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
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
                    .uri(ws_path(&format!("/workflows/{workflow_id}")))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
                    .header("x-csrf-token", TEST_CSRF_TOKEN)
                    .header("cookie", TEST_CSRF_COOKIE)
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
                .uri(ws_path(&format!("/workflows/{workflow_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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

/// Activation of a valid workflow definition succeeds with 200 OK.
/// Canon §10 step 2 regression lock: the validation gate must not block
/// structurally correct definitions.
#[tokio::test]
async fn activate_valid_returns_200() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    let workflow_id = nebula_core::WorkflowId::new();
    state
        .workflow_repo
        .save(workflow_id, 0, make_valid_workflow_definition(&workflow_id))
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id}/activate")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["id"], workflow_id.to_string());
}

/// Activation of a definitionally invalid workflow returns 422 Unprocessable
/// Entity with a structured RFC 9457 body.
/// Canon §10 step 2: activation MUST reject invalid definitions — not silently
/// flip the active flag.
#[tokio::test]
async fn activate_invalid_returns_422() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Cyclic workflow: parses as WorkflowDefinition but fails validate_workflow
    // (CycleDetected + NoEntryNodes).
    let workflow_id = nebula_core::WorkflowId::new();
    state
        .workflow_repo
        .save(
            workflow_id,
            0,
            make_cyclic_workflow_definition(&workflow_id),
        )
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id}/activate")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // RFC 9457: Content-Type must be application/problem+json
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/problem+json"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // RFC 9457 required fields
    assert_eq!(
        problem["type"],
        "https://nebula.dev/problems/invalid-workflow-definition"
    );
    assert_eq!(problem["title"], "Invalid Workflow Definition");
    assert_eq!(problem["status"], 422);
    assert!(
        problem["detail"].as_str().is_some(),
        "detail must be present"
    );

    // errors array must be non-empty and well-formed
    let errors = problem["errors"]
        .as_array()
        .expect("errors array must be present for invalid workflow activation");
    assert!(
        !errors.is_empty(),
        "errors array must list at least one validation failure"
    );
    for err in errors {
        assert!(err["code"].is_string(), "each error must have a code");
        assert!(err["detail"].is_string(), "each error must have a detail");
        // RFC 6901 JSON Pointer: either the root pointer ("") or a path starting with "/".
        // Structural errors (CycleDetected, NoEntryNodes, …) produce "" (root);
        // node/connection errors produce "/nodes/<key>" or "/connections/<from>/<to>".
        assert!(
            err["pointer"]
                .as_str()
                .is_some_and(|p| p.is_empty() || p.starts_with('/')),
            "pointer must be a valid RFC 6901 JSON Pointer (empty string or starts with /)"
        );
    }
}

// ── §12.2 / audit §2.2 cancel control-queue wiring tests ────────────────────

/// Canon §12.2 regression lock: cancelling a non-terminal execution must both
/// (1) persist the `cancelled` state in the execution row, AND
/// (2) enqueue a `Cancel` command in the durable control queue.
///
/// Before this fix only (1) happened — the engine never saw the cancel signal.
///
/// Audit ref: 2026-04-16-workspace-health-audit.md §2.2
/// Knife ref: PRODUCT_CANON.md §13 step 5
#[tokio::test]
async fn cancel_enqueues_durable_control_signal() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::{ExecutionId, WorkflowId};
    use nebula_storage::repos::ControlCommand;
    use tower::ServiceExt;

    let (state, control_queue) = create_test_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a running execution directly into the repo.
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    state
        .execution_repo
        .create(
            execution_id,
            workflow_id,
            serde_json::json!({
                "workflow_id": workflow_id.to_string(),
                "status": "running",
                "started_at": now,
                "input": {}
            }),
        )
        .await
        .unwrap();

    // Pre-condition: queue is empty.
    assert!(
        control_queue.snapshot().await.is_empty(),
        "control queue must be empty before cancel"
    );

    // Issue the cancel request.
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "cancel must return 200");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let cancelled: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // (1) Execution row must reflect the cancelled state.
    assert_eq!(
        cancelled["status"], "cancelled",
        "execution row must show `cancelled` status"
    );
    assert!(
        cancelled["finished_at"].is_number(),
        "finished_at must be set"
    );

    // (2) A Cancel command must have been written to the control queue — this is
    //     the engine-visible signal required by canon §12.2 and §13 step 5.
    let queued = control_queue.snapshot().await;
    assert_eq!(
        queued.len(),
        1,
        "exactly one control queue entry must exist after cancel"
    );

    let entry = &queued[0];
    assert_eq!(
        entry.command,
        ControlCommand::Cancel,
        "queued command must be Cancel"
    );
    assert_eq!(
        entry.status, "Pending",
        "entry must be in Pending state (not yet consumed by engine)"
    );
    // The entry's execution_id bytes must decode back to the cancelled execution.
    let queued_eid = String::from_utf8(entry.execution_id.clone())
        .expect("execution_id bytes must be valid UTF-8");
    assert_eq!(
        queued_eid,
        execution_id.to_string(),
        "queued entry must reference the cancelled execution"
    );
}

/// Cancelling an already-terminal execution must NOT enqueue a second Cancel
/// signal. The handler short-circuits at the terminal-status guard.
///
/// This locks the idempotency guarantee: re-trying a cancel on an already
/// cancelled execution is safe — no duplicate signals are written.
#[tokio::test]
async fn cancel_terminal_execution_does_not_enqueue() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::{ExecutionId, WorkflowId};
    use tower::ServiceExt;

    let (state, control_queue) = create_test_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a completed execution.
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    state
        .execution_repo
        .create(
            execution_id,
            workflow_id,
            serde_json::json!({
                "workflow_id": workflow_id.to_string(),
                "status": "completed",
                "started_at": now,
                "finished_at": now + 5,
                "input": {}
            }),
        )
        .await
        .unwrap();

    // Issue cancel on the completed execution — must be rejected.
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "cancel on completed execution must return 400"
    );

    // Queue must remain empty — no spurious signal enqueued.
    assert!(
        control_queue.snapshot().await.is_empty(),
        "control queue must be empty after rejected cancel of terminal execution"
    );
}

/// Regression for #329: `get_execution` must parse canonical RFC3339 timestamps
/// from engine-persisted `ExecutionState` blobs, not silently collapse to 0.
///
/// Canonical shape per `crates/execution/src/state.rs`: `started_at` and
/// `completed_at` are `Option<DateTime<Utc>>` serialized as RFC3339 strings.
/// The API response maps both into `started_at` / `finished_at` fields.
#[tokio::test]
async fn get_execution_parses_rfc3339_timestamps() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::{ExecutionId, WorkflowId};
    use tower::ServiceExt;

    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();

    // Seed with canonical engine-shape state: RFC3339 string timestamps
    // under the canonical field names (`completed_at`, not `finished_at`).
    state
        .execution_repo
        .create(
            execution_id,
            workflow_id,
            serde_json::json!({
                "workflow_id": workflow_id.to_string(),
                "status": "completed",
                "started_at": "2024-01-15T12:34:56Z",
                "completed_at": "2024-02-20T08:00:00Z",
                "input": {}
            }),
        )
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
    assert_eq!(execution["started_at"].as_i64(), Some(1_705_322_096));
    assert_eq!(execution["finished_at"].as_i64(), Some(1_708_416_000));
}

/// Regression for #331: `cancel_execution` must reject cancellation of an
/// execution already in `timed_out` state (another terminal state besides
/// completed/failed/cancelled).
#[tokio::test]
async fn cancel_timed_out_execution_rejected() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::{ExecutionId, WorkflowId};
    use tower::ServiceExt;

    let (state, control_queue) = create_test_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    state
        .execution_repo
        .create(
            execution_id,
            workflow_id,
            serde_json::json!({
                "workflow_id": workflow_id.to_string(),
                "status": "timed_out",
                "started_at": now,
                "finished_at": now + 30,
                "input": {}
            }),
        )
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "cancel on timed_out execution must be rejected (timed_out is terminal)"
    );

    // Queue must remain empty — terminal-status guard short-circuits before enqueue.
    assert!(
        control_queue.snapshot().await.is_empty(),
        "control queue must be empty after rejected cancel of timed_out execution"
    );
}

// ── Issue #327 regression ─────────────────────────────────────────────────────
//
// Canon §4.5: a public surface exists iff the engine honors it end-to-end.
// The API's `start_execution` previously persisted a hand-rolled JSON with
// `status: "pending"` — a string that is not in `ExecutionStatus` and that
// neither `list_running` (storage filter) nor `ExecutionState::deserialize`
// (engine resume path) would accept. Starting an execution therefore produced
// a row the engine could never read back: split-brain schema.
//
// This test pins the fix by asserting the exact contract that failed:
//
//   1. `POST /workflows/:id/executions` → 202.
//   2. The persisted `execution_repo.get_state(id)` row round-trips through
//      `serde_json::from_value::<ExecutionState>` without error — the same operation the engine's
//      `resume_execution` performs.
//   3. The deserialized `ExecutionStatus` is the canonical `Created`, not a synthetic "pending"
//      variant.
//   4. The `list_running` storage query — which filters on canonical status names — actually
//      returns the newly-created execution ID (it did not before, because `"pending"` is not in the
//      filter).
#[tokio::test]
async fn test_issue_327_start_execution_persists_canonical_execution_state() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_core::ExecutionId;
    use nebula_execution::{ExecutionState, ExecutionStatus};
    use tower::ServiceExt;

    let (state, _control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a workflow via POST /workflows — the same path a real client uses.
    let create_request = serde_json::json!({
        "name": "Issue 327 Workflow",
        "description": "Contract test: API-created execution must round-trip through ExecutionState",
        "definition": { "nodes": [], "edges": [] }
    });
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from(serde_json::to_string(&create_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "#327: seed workflow must return 201"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id_str = created_workflow["id"].as_str().unwrap().to_string();
    let workflow_id = nebula_core::WorkflowId::parse(&workflow_id_str).unwrap();

    // POST /api/v1/workflows/:id/executions with a real input.
    let input = serde_json::json!({ "knife_key": "knife_value" });
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id_str}/executions")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "input": input })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "#327: POST must return 202"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let execution_id_str = execution_response["id"]
        .as_str()
        .expect("id field")
        .to_string();
    let execution_id = ExecutionId::parse(&execution_id_str).expect("parse id");

    // Response status must be canonical.
    assert_eq!(
        execution_response["status"].as_str(),
        Some("created"),
        "#327: API response status must be canonical 'created', not 'pending'"
    );

    // Load the persisted row directly through the repo — same path the engine
    // uses in `resume_execution`.
    let (_version, state_json) = state
        .execution_repo
        .get_state(execution_id)
        .await
        .expect("get_state must not error")
        .expect("row must exist after start_execution");

    // The exact contract `resume_execution` performs at
    // `crates/engine/src/engine.rs` lines 778-781: serialize the stored
    // Value to a string and deserialize back into `ExecutionState`. If this
    // fails, the engine cannot resume the execution — the exact bug #327.
    let state_str = serde_json::to_string(&state_json).expect("state must serialize");
    let exec_state: ExecutionState = serde_json::from_str(&state_str).unwrap_or_else(|e| {
        panic!(
            "#327: persisted row must deserialize into canonical ExecutionState \
             (engine resume_execution contract); got error: {e}; row was: {state_json}"
        )
    });

    // Canonical status — not the deprecated "pending".
    assert_eq!(
        exec_state.status,
        ExecutionStatus::Created,
        "#327: persisted status must be canonical Created, not a drifted variant"
    );
    assert_eq!(
        exec_state.execution_id, execution_id,
        "#327: persisted execution_id must round-trip"
    );
    assert_eq!(
        exec_state.workflow_id, workflow_id,
        "#327: persisted workflow_id must round-trip"
    );
    // `started_at` is None for a Created execution — the engine has not
    // transitioned to Running yet.
    assert!(
        exec_state.started_at.is_none(),
        "#327: started_at must be None until the engine transitions to Running \
         (canon §11.1: authority over lifecycle transitions lives in the engine)"
    );
    assert!(
        exec_state.completed_at.is_none(),
        "#327: completed_at must be None on a non-terminal execution"
    );
    // Workflow input (#311) round-trips.
    assert_eq!(
        exec_state.workflow_input,
        Some(input),
        "#327 (and #311): workflow_input must be persisted so resume can replay entry nodes"
    );

    // The list_running storage filter — which only accepts canonical statuses —
    // must actually see the newly-created execution. Before the fix this list
    // was empty because "pending" is not in the accepted set
    // (created|running|paused|cancelling).
    let running = state
        .execution_repo
        .list_running()
        .await
        .expect("list_running must not error");
    assert!(
        running.contains(&execution_id),
        "#327: list_running must include the newly-created execution \
         (split-brain check — status is canonical and the filter matches)"
    );
}

// ── Issue #332 regression ────────────────────────────────────────────────────
//
// Canon §4.5: a public surface exists iff the engine honors it end-to-end.
// Canon §12.2: every engine-visible signal must travel the durable control
// queue — in-process channels are not a durable backbone.
// Knife §13 step 3: starting an execution must cause node dispatch.
//
// Before this fix the API `start_execution` / `execute_workflow` handlers
// persisted a row and returned 202 without ever notifying the engine — so
// the execution sat in `Created` forever and no node ran. This is the
// §4.5 "advertise capability engine doesn't deliver end-to-end" violation.
//
// The fix enqueues `ControlCommand::Start` on the shared durable control
// queue immediately after persisting the row (ADR-0008). The engine-side
// `ControlConsumer` drains that queue to drive the actual run.
//
// These regression tests pin the exact contract that was missing:
//   a) POST /workflows/:id/executions writes a `Start` row for the new id.
//   b) POST /workflows/:id/execute writes a `Start` row for the new id.
//   c) The queue entry shape matches the cancel-side contract (so the
//      engine consumer can decode both without special-casing either).

#[tokio::test]
async fn test_issue_332_start_execution_enqueues_control_start() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_storage::repos::ControlCommand;
    use tower::ServiceExt;

    let (state, control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a workflow via the real POST /workflows path.
    let create_request = serde_json::json!({
        "name": "Issue 332 Workflow",
        "description": "API start must emit a Start command on the control queue",
        "definition": { "nodes": [], "edges": [] }
    });
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
    let workflow_id_str = created_workflow["id"].as_str().unwrap().to_string();

    // Pre-condition: queue is empty — no other path has written to it.
    assert!(
        control_queue.snapshot().await.is_empty(),
        "#332: control queue must be empty before start"
    );

    // POST /api/v1/workflows/:id/executions — the exact path the bug names.
    let start_request = serde_json::json!({ "input": { "k": "v" } });
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id_str}/executions")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from(serde_json::to_string(&start_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "#332: POST must return 202"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let execution_id_str = execution_response["id"].as_str().unwrap().to_string();

    // Assert: exactly one Start entry exists on the queue, targeting the
    // execution id the API just returned. This is the engine-visible signal
    // canon §12.2 requires — without it the engine never dispatches and
    // the execution stays `Created` forever (the bug).
    let queued = control_queue.snapshot().await;
    assert_eq!(
        queued.len(),
        1,
        "#332: exactly one control queue entry must exist after start_execution, got {queued:?}"
    );
    let entry = &queued[0];
    assert_eq!(
        entry.command,
        ControlCommand::Start,
        "#332: queued command must be Start, got {:?}",
        entry.command
    );
    assert_eq!(
        entry.status, "Pending",
        "#332: entry must be in Pending state until the engine consumer processes it"
    );
    let queued_eid = String::from_utf8(entry.execution_id.clone())
        .expect("execution_id bytes must be valid UTF-8");
    assert_eq!(
        queued_eid, execution_id_str,
        "#332: queued entry must reference the newly-created execution id"
    );
}

#[tokio::test]
async fn test_issue_332_execute_workflow_enqueues_control_start() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nebula_storage::repos::ControlCommand;
    use tower::ServiceExt;

    let (state, control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a workflow so the handler can locate it.
    let create_request = serde_json::json!({
        "name": "Issue 332 Execute Workflow",
        "description": "POST /workflows/:id/execute must also emit a Start command",
        "definition": { "nodes": [], "edges": [] }
    });
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
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
    let workflow_id_str = created_workflow["id"].as_str().unwrap().to_string();

    assert!(
        control_queue.snapshot().await.is_empty(),
        "#332: control queue must be empty before execute"
    );

    // POST /api/v1/workflows/:id/execute — the second audit-cited failure site.
    let execute_request = serde_json::json!({ "input": { "k": "v" } });
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id_str}/execute")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from(serde_json::to_string(&execute_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "#332: /execute must return 202"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let execution_id_str = execution_response["id"].as_str().unwrap().to_string();

    let queued = control_queue.snapshot().await;
    assert_eq!(
        queued.len(),
        1,
        "#332: exactly one control queue entry must exist after execute_workflow"
    );
    let entry = &queued[0];
    assert_eq!(
        entry.command,
        ControlCommand::Start,
        "#332: queued command from /execute must also be Start"
    );
    let queued_eid = String::from_utf8(entry.execution_id.clone())
        .expect("execution_id bytes must be valid UTF-8");
    assert_eq!(
        queued_eid, execution_id_str,
        "#332: queued entry must reference the newly-created execution id"
    );
}
