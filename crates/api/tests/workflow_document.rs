mod common;

use axum::{body::Body, http::Request};
use common::*;
use nebula_api::{ApiConfig, AppState, app};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn exchange(state: &AppState, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = app::build_app(state.clone(), &ApiConfig::for_test())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(ws_path(path))
                .header("authorization", format!("Bearer {}", create_test_jwt()))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn document_read_and_stale_editor_save_cross_the_public_router() {
    let (state, handles) = create_state_with_port_handles().await;
    let id = nebula_core::WorkflowId::new();
    let definition = make_valid_workflow_definition(&id);
    handles.seed_workflow(id, definition.clone()).await;
    let path = format!("/workflows/{id}");
    let (status, loaded) = exchange(&state, "GET", &path, Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(loaded["definition"], definition);
    assert_eq!(loaded["revision"], 1);
    let (status, saved) = exchange(
        &state,
        "PUT",
        &path,
        json!({"expected_revision":1,"definition":{"settings":{"label":"first"}}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(saved["revision"], 2);
    let (status, _) = exchange(
        &state,
        "PUT",
        &path,
        json!({"expected_revision":1,"definition":{"settings":{"label":"stale"}}}),
    )
    .await;
    assert_eq!(status, 409);
    let (_, current) = exchange(&state, "GET", &path, Value::Null).await;
    assert_eq!(current["definition"]["settings"]["label"], "first");
    assert_eq!(current["revision"], 2);
    let (status, _) = exchange(
        &state,
        "POST",
        &format!("{path}/activate?expected_revision=1"),
        json!({}),
    )
    .await;
    assert_eq!(status, 409);
    let (_, after) = exchange(&state, "GET", &path, Value::Null).await;
    assert_eq!(after["revision"], 2);
}
