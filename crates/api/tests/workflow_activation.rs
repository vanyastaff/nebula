mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN, create_state_with_port_handles, create_test_jwt,
    make_valid_workflow_definition, port_scope, ws_path,
};
use nebula_api::{ApiConfig, AppState, app};
use tower::ServiceExt;

async fn activate(state: AppState, workflow: nebula_core::WorkflowId) -> axum::response::Response {
    app::build_app(state, &ApiConfig::for_test())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow}/activate")))
                .header("authorization", format!("Bearer {}", create_test_jwt()))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn unwired_activation_is_unavailable_and_publishes_nothing() {
    let (mut state, stores) = create_state_with_port_handles().await;
    let workflow = nebula_core::WorkflowId::new();
    stores
        .seed_workflow(workflow, make_valid_workflow_definition(&workflow))
        .await;
    state.workflow_activation = None;
    let response = activate(state.clone(), workflow).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let versions = state
        .workflow_version_store
        .list(&port_scope(), &workflow.to_string())
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert!(versions[0].activation.is_none());
}

#[tokio::test]
async fn compilation_diagnostics_are_structured_and_leave_publication_unchanged() {
    let (state, stores) = create_state_with_port_handles().await;
    let workflow = nebula_core::WorkflowId::new();
    let mut definition = make_valid_workflow_definition(&workflow);
    definition["nodes"][0]["action_key"] = serde_json::json!("missing_action");
    definition["description"] = serde_json::json!("private-description-canary");
    stores.seed_workflow(workflow, definition).await;
    let response = activate(state.clone(), workflow).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-description-canary"));
    let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let errors = problem["errors"].as_array().unwrap();
    assert!(!errors.is_empty());
    for error in errors {
        for field in ["code", "expected", "actual", "remediation"] {
            assert!(!error[field].as_str().unwrap().is_empty());
        }
        assert!(error["path"].is_string() || error["pointer"].is_string());
    }
    let versions = state
        .workflow_version_store
        .list(&port_scope(), &workflow.to_string())
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert!(versions[0].activation.is_none());
}

#[tokio::test]
async fn malformed_stored_definition_does_not_echo_its_payload() {
    let (state, stores) = create_state_with_port_handles().await;
    let workflow = nebula_core::WorkflowId::new();
    let mut definition = make_valid_workflow_definition(&workflow);
    definition["nodes"][0]["id"] = serde_json::json!("secret-canary invalid key");
    stores.seed_workflow(workflow, definition).await;
    let response = activate(state, workflow).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("secret-canary"));
}
