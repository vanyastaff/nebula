//! Integration tests for the workspace-scoped resource catalog handlers.
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` is a config-CRUD
//! read endpoint (ADR-0047 Stub Endpoint Policy retired for this route).
//! These tests boot the in-memory app with a fake [`ResourceRepo`] and
//! assert the honest `ResourceEntry` → `ResourceSummary` mapping:
//! `res_<ULID>` id encoding, `display_name` → `name`, soft-deleted rows
//! excluded, and no raw `config` ever surfaced (ADR-0028 §7).

mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use nebula_api::{ApiConfig, AppState, app};
use nebula_core::WorkspaceId;
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo,
    repos::{InMemoryControlQueueRepo, ResourceEntry, ResourceRepo},
};
use tower::ServiceExt;

/// Raw 16-byte workspace id matching [`TEST_WS`], as the repo stores it.
fn test_ws_bytes() -> Vec<u8> {
    TEST_WS
        .parse::<WorkspaceId>()
        .expect("TEST_WS is a valid ws_<ULID>")
        .as_bytes()
        .to_vec()
}

/// Build a `ResourceEntry` with deterministic 16-byte id bytes.
fn entry(
    id_byte: u8,
    slug: &str,
    display_name: &str,
    kind: &str,
    version: i64,
    deleted: bool,
) -> ResourceEntry {
    ResourceEntry {
        id: vec![id_byte; 16],
        workspace_id: test_ws_bytes(),
        slug: slug.to_owned(),
        display_name: display_name.to_owned(),
        kind: kind.to_owned(),
        // A config value that must NEVER appear in the response body.
        config: serde_json::json!({ "secret_looking_key": "do-not-leak" }),
        created_at: chrono::Utc::now(),
        created_by: vec![0u8; 16],
        version,
        deleted_at: deleted.then_some(chrono::Utc::now()),
    }
}

/// Fake repo returning two live resources plus one soft-deleted one for
/// the test workspace. `list` returns every row (including soft-deleted)
/// — the handler is responsible for excluding `deleted_at.is_some()`.
struct FakeResourceRepo;

#[async_trait::async_trait]
impl ResourceRepo for FakeResourceRepo {
    async fn create(&self, _r: &ResourceEntry) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn get(&self, _id: &[u8]) -> Result<Option<ResourceEntry>, nebula_storage::StorageError> {
        Ok(None)
    }

    async fn get_by_slug(
        &self,
        _ws: &[u8],
        _slug: &str,
    ) -> Result<Option<ResourceEntry>, nebula_storage::StorageError> {
        Ok(None)
    }

    async fn update(
        &self,
        _r: &ResourceEntry,
        _expected: i64,
    ) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn soft_delete(&self, _id: &[u8]) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn list(
        &self,
        workspace_id: &[u8],
    ) -> Result<Vec<ResourceEntry>, nebula_storage::StorageError> {
        assert_eq!(
            workspace_id,
            test_ws_bytes().as_slice(),
            "handler must scope list() to the tenant workspace id bytes"
        );
        Ok(vec![
            entry(0x11, "http-pool", "HTTP Pool", "http_pool", 3, false),
            entry(0x22, "redis", "Redis Cache", "redis_cache", 1, false),
            entry(0x33, "old-pool", "Deleted Pool", "http_pool", 7, true),
        ])
    }
}

async fn state_with_resource_repo(api_config: &ApiConfig) -> AppState {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let repo: Arc<dyn ResourceRepo> = Arc::new(FakeResourceRepo);

    AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
    .with_resource_repo(repo)
}

#[tokio::test]
async fn list_resources_returns_200_with_mapped_summaries() {
    let api_config = ApiConfig::for_test();
    let state = state_with_resource_repo(&api_config).await;
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/resources"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Honesty contract: the route is implemented, NOT a 501 stub.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "list_resources must be implemented (200), not the ADR-0047 501 stub"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8(body.to_vec()).expect("body is utf-8");

    // Raw, non-secret config must never leak into the summary payload.
    assert!(
        !raw.contains("secret_looking_key") && !raw.contains("do-not-leak"),
        "response must not surface raw resource config; body: {raw}"
    );

    let json: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON body");
    let resources = json["resources"]
        .as_array()
        .expect("`resources` must be a JSON array");

    // Soft-deleted entry (0x33) is excluded → exactly 2 live summaries.
    assert_eq!(
        resources.len(),
        2,
        "soft-deleted resources must be excluded; got {resources:?}"
    );

    let first = &resources[0];
    let id = first["id"].as_str().expect("id is a string");
    assert!(
        id.starts_with("res_"),
        "id must be the prefixed `res_<ULID>` encoding, got {id}"
    );
    assert_eq!(first["name"], "HTTP Pool", "name must be display_name");
    assert_eq!(first["kind"], "http_pool");
    assert_eq!(first["version"], "3", "version must render the i64 version");
    assert_eq!(
        first["attached_to_workflows"]
            .as_array()
            .expect("attached_to_workflows is an array")
            .len(),
        0,
        "workflow attachment is not tracked yet — must be honestly empty"
    );

    let second = &resources[1];
    assert_eq!(second["name"], "Redis Cache");
    assert_eq!(second["kind"], "redis_cache");
    assert_eq!(second["version"], "1");
}

#[tokio::test]
async fn list_resources_without_repo_is_service_unavailable() {
    // No `.with_resource_repo(...)` → the optional dependency is None.
    // Mirrors the action/plugin catalog convention: 503, not a 501 stub.
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::for_test();
    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver));

    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path("/resources"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured resource repo must be 503 (not the retired 501 stub)"
    );
}
