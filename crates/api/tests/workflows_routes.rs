use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use nebula_api::{ApiState, WorkerStatus, api_only_app_with_state};
use nebula_core::WorkflowId;
use nebula_ports::{PortsError, WorkflowRepo};
use nebula_webhook::{WebhookServer, WebhookServerConfig};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tower::ServiceExt;

fn test_workers() -> Vec<WorkerStatus> {
    vec![WorkerStatus {
        id: "wrk-1".to_string(),
        status: "idle".to_string(),
        queue_len: 0,
    }]
}

#[derive(Default)]
struct InMemoryWorkflowRepo {
    definitions: Arc<RwLock<HashMap<WorkflowId, serde_json::Value>>>,
    versions: Arc<RwLock<HashMap<WorkflowId, u64>>>,
}

#[async_trait]
impl WorkflowRepo for InMemoryWorkflowRepo {
    async fn get_with_version(
        &self,
        id: WorkflowId,
    ) -> Result<Option<(u64, serde_json::Value)>, PortsError> {
        let definitions = self.definitions.read().await;
        let Some(definition) = definitions.get(&id).cloned() else {
            return Ok(None);
        };
        let versions = self.versions.read().await;
        let version = versions.get(&id).copied().unwrap_or(0);
        Ok(Some((version, definition)))
    }

    async fn save(
        &self,
        id: WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), PortsError> {
        let mut versions = self.versions.write().await;
        let current = versions.get(&id).copied().unwrap_or(0);
        if current != version {
            return Err(PortsError::conflict(
                "workflow",
                id.to_string(),
                version,
                current,
            ));
        }

        versions.insert(id, current + 1);
        self.definitions.write().await.insert(id, definition);
        Ok(())
    }

    async fn delete(&self, id: WorkflowId) -> Result<bool, PortsError> {
        self.versions.write().await.remove(&id);
        Ok(self.definitions.write().await.remove(&id).is_some())
    }

    async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(WorkflowId, serde_json::Value)>, PortsError> {
        let map = self.definitions.read().await;
        let mut rows: Vec<(WorkflowId, serde_json::Value)> =
            map.iter().map(|(id, value)| (*id, value.clone())).collect();
        rows.sort_by_key(|(id, _)| id.to_string());
        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }
}

async fn build_app(repo: Option<Arc<dyn WorkflowRepo>>) -> axum::Router {
    let webhook = WebhookServer::new_embedded(WebhookServerConfig::default()).unwrap();
    let mut state = ApiState::new(webhook, test_workers());
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
