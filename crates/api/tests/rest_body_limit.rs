//! REST router body-limit integration test (ADR-0020 §3 pre-condition #3).
//!
//! The webhook transport caps itself at
//! `crates/api/src/webhook/transport.rs`. The REST surface
//! (`/workflows`, `/credentials` POST) did not — this test pins the
//! router-level `DefaultBodyLimit` wired in `crates/api/src/app.rs` at
//! 413 for a 2 MiB payload, twice the advertised `REST_BODY_LIMIT_BYTES`
//! (1 MiB) default defined in `crates/api/src/config.rs` and wired
//! through `ApiConfig::max_body_size`. Tracked as issue
//! <https://github.com/vanyastaff/nebula/issues/520>.

mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN, TestOrgResolver, TestWorkspaceResolver, create_test_jwt,
};
use nebula_api::{ApiConfig, AppState, app};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};
use pretty_assertions::assert_eq;
use tower::ServiceExt;

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

/// A 2 MiB POST on `/api/v1/workflows` must return `413 Payload Too Large`
/// — the REST router carries a 1 MiB `DefaultBodyLimit` per ADR-0020 §3.
#[tokio::test]
async fn rest_post_exceeding_limit_returns_413() {
    let state = create_test_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    // 2 MiB of 'a' bytes — double the 1 MiB REST cap. Content is not
    // valid JSON, but the body-limit check runs during extraction
    // before JSON parsing, so the 413 surfaces regardless.
    let payload = vec![b'a'; 2 * 1024 * 1024];

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(common::ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "2 MiB POST to /api/v1/workflows should trip the REST body \
         limit and return 413",
    );
}
