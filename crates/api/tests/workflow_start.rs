mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use nebula_api::{ApiConfig, AppState, app};
use tower::ServiceExt;

async fn request(
    state: &AppState,
    workflow: nebula_core::WorkflowId,
    route: &str,
    key: Option<&str>,
    input: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(ws_path(&format!("/workflows/{workflow}/{route}")))
        .header("authorization", format!("Bearer {}", create_test_jwt()))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .header("content-type", "application/json");
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    let response = app::build_app(state.clone(), &ApiConfig::for_test())
        .oneshot(
            request
                .body(Body::from(serde_json::json!({"input": input}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.headers().get("retry-after").is_none());
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn publish(state: &AppState, workflow: nebula_core::WorkflowId, expected: u64) {
    let definition =
        serde_json::from_str(&make_valid_workflow_definition(&workflow).to_string()).unwrap();
    state
        .workflow_activation
        .as_ref()
        .unwrap()
        .activate(&port_scope(), workflow, expected, definition)
        .await
        .unwrap();
}

#[tokio::test]
async fn keyed_replay_crosses_routes_and_survives_republication() {
    let (state, handles) = create_state_with_port_handles().await;
    let workflow = nebula_core::WorkflowId::new();
    handles
        .seed_workflow(workflow, make_valid_workflow_definition(&workflow))
        .await;
    publish(&state, workflow, 1).await;
    let (status, original) = request(
        &state,
        workflow,
        "executions",
        Some("same-intent"),
        serde_json::json!({"value": 7}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = original["id"].as_str().unwrap();
    let bundle = state
        .start_acceptance_scoped(&port_scope())
        .read_contract_bundle(&port_scope(), id)
        .await
        .unwrap()
        .unwrap();
    publish(&state, workflow, 2).await;
    let (status, replay) = request(
        &state,
        workflow,
        "execute",
        Some("same-intent"),
        serde_json::json!({"value": 7}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(replay, original);
    assert_eq!(
        state
            .start_acceptance_scoped(&port_scope())
            .read_contract_bundle(&port_scope(), id)
            .await
            .unwrap()
            .unwrap(),
        bundle
    );
    assert_eq!(handles.control_queue.snapshot().len(), 1);
    let (status, _) = request(
        &state,
        workflow,
        "execute",
        Some("same-intent"),
        serde_json::json!({"value": 8}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(handles.control_queue.snapshot().len(), 1);
}

#[tokio::test]
async fn unkeyed_starts_each_commit_a_checked_bundle_and_command() {
    let (state, handles) = create_state_with_port_handles().await;
    let workflow = nebula_core::WorkflowId::new();
    handles
        .seed_workflow(workflow, make_valid_workflow_definition(&workflow))
        .await;
    publish(&state, workflow, 1).await;
    let mut ids = Vec::new();
    for route in ["execute", "executions"] {
        let (status, receipt) =
            request(&state, workflow, route, None, serde_json::Value::Null).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let id = receipt["id"].as_str().unwrap();
        let stored = state
            .execution_store
            .get(&port_scope(), id)
            .await
            .unwrap()
            .unwrap();
        let bundle = state
            .start_acceptance_scoped(&port_scope())
            .read_contract_bundle(&port_scope(), id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bundle.execution_id(), id);
        assert_eq!(
            stored.state["executable_plan_revision_id"],
            serde_json::json!(bundle.record().identity().revisions().plan())
        );
        assert!(receipt["started_at"].as_i64().unwrap() > 0);
        ids.push(id.to_string());
    }
    assert_ne!(ids[0], ids[1]);
    assert_eq!(handles.control_queue.snapshot().len(), 2);
}

#[tokio::test]
async fn missing_start_owner_fails_closed_without_writes() {
    let (mut state, handles) = create_state_with_port_handles().await;
    state.workflow_start = None;
    for route in ["execute", "executions"] {
        let (status, _) = request(
            &state,
            nebula_core::WorkflowId::new(),
            route,
            None,
            serde_json::Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
    assert!(handles.control_queue.snapshot().is_empty());
}
