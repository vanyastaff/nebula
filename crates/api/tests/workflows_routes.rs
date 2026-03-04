use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use nebula_api::{ApiState, api_only_app_with_state};
use nebula_core::WorkflowId;
use nebula_storage::{InMemoryWorkflowRepo, WorkflowRepo};
use std::sync::Arc;
use tower::ServiceExt;

async fn build_app(repo: Option<Arc<dyn WorkflowRepo>>) -> axum::Router {
    let mut state = ApiState::new();
    if let Some(workflow_repo) = repo {
        state = state.with_workflow_repo(workflow_repo);
    }
    api_only_app_with_state(state)
}

async fn issue_mock_token(app: axum::Router) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/oauth/callback")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"provider":"github","code":"mock_workflow_token","redirectUri":"nebula://auth/callback"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    payload["accessToken"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn workflows_list_requires_authentication() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let app = build_app(Some(repo)).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn workflows_list_returns_paginated_payload() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let id1 = WorkflowId::parse("00000000-0000-0000-0000-000000000001").unwrap();
    let id2 = WorkflowId::parse("00000000-0000-0000-0000-000000000002").unwrap();
    let id3 = WorkflowId::parse("00000000-0000-0000-0000-000000000003").unwrap();

    repo.save(
        id1,
        0,
        serde_json::json!({"name":"Alpha","active":true,"updatedAt":"2026-03-04T10:00:00Z"}),
    )
    .await
    .unwrap();
    repo.save(id2, 0, serde_json::json!({"name":"Beta","active":false}))
        .await
        .unwrap();
    repo.save(id3, 0, serde_json::json!({"name":"Gamma","active":true}))
        .await
        .unwrap();

    let app = build_app(Some(repo)).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows?offset=1&limit=2")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["offset"], 1);
    assert_eq!(payload["limit"], 2);
    assert_eq!(payload["items"].as_array().unwrap().len(), 2);
    assert_eq!(payload["items"][0]["name"], "Beta");
    assert_eq!(payload["items"][1]["name"], "Gamma");
}

#[tokio::test]
async fn workflow_get_returns_details() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let id = WorkflowId::parse("00000000-0000-0000-0000-0000000000aa").unwrap();
    repo.save(
        id,
        0,
        serde_json::json!({
            "name":"Sync Contacts",
            "active":true,
            "nodes":[{"id":"n1","type":"http"}]
        }),
    )
    .await
    .unwrap();

    let app = build_app(Some(repo)).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows/00000000-0000-0000-0000-0000000000aa")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], "00000000-0000-0000-0000-0000000000aa");
    assert_eq!(payload["name"], "Sync Contacts");
    assert_eq!(payload["active"], true);
    assert_eq!(payload["definition"]["nodes"][0]["type"], "http");
}

#[tokio::test]
async fn workflow_get_rejects_invalid_uuid() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let app = build_app(Some(repo)).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/workflows/not-a-uuid")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workflow_create_returns_created_workflow() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let app = build_app(Some(repo.clone())).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/workflows")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"name":"Nightly Sync","definition":{"nodes":[{"id":"n1"}]}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["name"], "Nightly Sync");
    assert_eq!(payload["active"], false);
    assert_eq!(payload["definition"]["name"], "Nightly Sync");
}

#[tokio::test]
async fn workflow_patch_updates_selected_fields() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let workflow_id = WorkflowId::parse("10000000-0000-0000-0000-000000000001").unwrap();
    repo.save(
        workflow_id,
        0,
        serde_json::json!({
            "name":"Draft Flow",
            "active": false,
            "nodes":[{"id":"n1","type":"http"}]
        }),
    )
    .await
    .unwrap();

    let app = build_app(Some(repo)).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/workflows/10000000-0000-0000-0000-000000000001")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Draft Flow v2","active":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["name"], "Draft Flow v2");
    assert_eq!(payload["active"], true);
    assert_eq!(payload["definition"]["nodes"][0]["type"], "http");
}

#[tokio::test]
async fn workflow_delete_returns_no_content() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let workflow_id = WorkflowId::parse("20000000-0000-0000-0000-000000000001").unwrap();
    repo.save(
        workflow_id,
        0,
        serde_json::json!({"name":"To Delete","active":false}),
    )
    .await
    .unwrap();

    let app = build_app(Some(repo)).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/workflows/20000000-0000-0000-0000-000000000001")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn workflow_delete_returns_not_found_for_missing_id() {
    let repo = Arc::new(InMemoryWorkflowRepo::default());
    let app = build_app(Some(repo)).await;
    let token = issue_mock_token(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/workflows/20000000-0000-0000-0000-000000000002")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
