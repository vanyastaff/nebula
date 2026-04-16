//! Canon §13 knife scenario — end-to-end integration test.
//!
//! This file covers §13 steps 1–6 as specified in
//! `docs/PRODUCT_CANON.md §13` and the workspace health audit
//! (`docs/superpowers/specs/2026-04-16-workspace-health-audit.md §8
//! Sprint A1 item #3`).
//!
//! Each step is asserted through the real axum `Router` via oneshot requests
//! against in-memory repos — no handler logic is bypassed.
//!
//! ## Step coverage
//!
//! | Step | What is asserted | Test(s) |
//! |------|-----------------|---------|
//! | 1 | `POST /workflows` round-trips through `GET /workflows/:id` | `knife_scenario_end_to_end` |
//! | 2a | `POST /workflows/:id/activate` valid → 200 | `knife_scenario_end_to_end` |
//! | 2b | `POST /workflows/:id/activate` cyclic → 422 RFC 9457 | `knife_scenario_end_to_end` |
//! | 3 | `POST /workflows/:id/executions` → 202, `status=pending`, `started_at > 0`, `finished_at` absent | `knife_scenario_end_to_end` |
//! | 4 | `GET /executions/:id` → `finished_at` is null/absent, `status` = latest persisted value | `knife_scenario_end_to_end` |
//! | 5 | `POST /executions/:id/cancel` → DB row = `cancelled`, control queue has exactly one `Cancel` entry | `knife_scenario_end_to_end` |
//! | 6 | Enqueue failure → 500 (handler uses `Internal` not `ServiceUnavailable`; documented discrepancy) | `knife_step6_queue_failure_returns_error` |
//!
//! ### Step 6 — known discrepancy
//!
//! Canon §13 step 6 mandates **503** when "orchestration is intentionally
//! absent". The current `cancel_execution` handler maps enqueue failures to
//! `ApiError::Internal` → HTTP 500. Changing this requires a handler edit,
//! which is out of scope for this test-only task. The test below documents the
//! actual behavior (500) and is marked with `// CANON-GAP §13-step6` so a
//! future PR can flip the assertion to 503 once the handler is corrected.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use nebula_api::{ApiConfig, AppState, app};
use nebula_config::ConfigBuilder;
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};
use tower::ServiceExt;

// ── shared constants ─────────────────────────────────────────────────────────

const TEST_JWT_SECRET: &str = "test-secret-for-integration-tests-0123456789";

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a minimal, structurally valid `WorkflowDefinition` JSON that passes
/// `nebula_workflow::validate_workflow` (single node, no cycles, schema_version=1).
fn make_valid_workflow_definition(workflow_id: &nebula_core::WorkflowId) -> serde_json::Value {
    serde_json::json!({
        "id": workflow_id.to_string(),
        "name": "Knife Valid Workflow",
        "version": { "major": 0, "minor": 1, "patch": 0 },
        "nodes": [
            { "id": "step_a", "name": "Step A", "action_key": "echo" }
        ],
        "connections": [],
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "schema_version": 1
    })
}

/// Build a `WorkflowDefinition` JSON that parses correctly but fails
/// `validate_workflow` due to a cycle (A → B, B → A) and therefore also
/// fails entry-node detection.
fn make_cyclic_workflow_definition(workflow_id: &nebula_core::WorkflowId) -> serde_json::Value {
    serde_json::json!({
        "id": workflow_id.to_string(),
        "name": "Knife Cyclic Workflow",
        "version": { "major": 0, "minor": 1, "patch": 0 },
        "nodes": [
            { "id": "step_a", "name": "A", "action_key": "echo" },
            { "id": "step_b", "name": "B", "action_key": "echo" }
        ],
        "connections": [
            { "from_node": "step_a", "to_node": "step_b" },
            { "from_node": "step_b", "to_node": "step_a" }
        ],
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "schema_version": 1
    })
}

/// Build a valid JWT token for the test secret.
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

    encode(
        &Header::default(),
        &Claims {
            sub: "knife-test-user".to_string(),
            exp: now + 3600,
            iat: now,
        },
        &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
    )
    .unwrap()
}

/// Create an `AppState` with fully functional in-memory repos; return both the
/// state and a typed reference to the control queue so tests can inspect it.
async fn create_state_with_queue() -> (AppState, Arc<InMemoryControlQueueRepo>) {
    let config = ConfigBuilder::new()
        .with_defaults(serde_json::json!({
            "api": { "port": 8080, "host": "127.0.0.1" }
        }))
        .build()
        .await
        .unwrap();

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::for_test();

    let control_queue_dyn: Arc<dyn nebula_storage::repos::ControlQueueRepo> =
        Arc::clone(&control_queue_repo) as _;

    let state = AppState::new(
        config,
        workflow_repo,
        execution_repo,
        control_queue_dyn,
        api_config.jwt_secret.clone(),
    );

    (state, control_queue_repo)
}

/// A control-queue repo that always fails on `enqueue` — used to simulate
/// the "orchestration backend unavailable" scenario in §13 step 6.
struct AlwaysFailControlQueueRepo;

#[async_trait::async_trait]
impl nebula_storage::repos::ControlQueueRepo for AlwaysFailControlQueueRepo {
    async fn enqueue(
        &self,
        _entry: &nebula_storage::repos::ControlQueueEntry,
    ) -> Result<(), nebula_storage::StorageError> {
        Err(nebula_storage::StorageError::Internal(
            "control queue backend unavailable (simulated)".to_string(),
        ))
    }

    async fn claim_pending(
        &self,
        _processor: &[u8],
        _batch_size: u32,
    ) -> Result<Vec<nebula_storage::repos::ControlQueueEntry>, nebula_storage::StorageError> {
        Ok(vec![])
    }

    async fn mark_completed(&self, _id: &[u8]) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn mark_failed(
        &self,
        _id: &[u8],
        _error: &str,
    ) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn cleanup(
        &self,
        _retention: std::time::Duration,
    ) -> Result<u64, nebula_storage::StorageError> {
        Ok(0)
    }
}

/// Create an `AppState` wired with the always-failing control queue repo.
/// All other repos are fully functional in-memory implementations.
async fn create_state_with_failing_queue() -> AppState {
    let config = ConfigBuilder::new()
        .with_defaults(serde_json::json!({
            "api": { "port": 8080, "host": "127.0.0.1" }
        }))
        .build()
        .await
        .unwrap();

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo: Arc<dyn nebula_storage::repos::ControlQueueRepo> =
        Arc::new(AlwaysFailControlQueueRepo);
    let api_config = ApiConfig::for_test();

    AppState::new(
        config,
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Canon §13 steps 1–5 end-to-end: define → activate → start → observe → cancel.
///
/// Each sub-step is labelled with the canon section it exercises.
///
/// Audit ref: 2026-04-16-workspace-health-audit.md §8 Sprint A1 item #3
#[tokio::test]
async fn knife_scenario_end_to_end() {
    use nebula_storage::repos::ControlCommand;

    let (state, control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // ── Step 1: Define a valid workflow and verify round-trip ────────────────
    //
    // Canon §13 step 1: "Define and persist a workflow through the API —
    // definition round-trips."
    //
    // POST /api/v1/workflows with a minimal request body (name + definition
    // skeleton). The handler stores the workflow and returns 201 with the
    // created resource. A subsequent GET must return the same `id` and `name`.

    let create_request = serde_json::json!({
        "name": "Knife Scenario Workflow",
        "description": "End-to-end knife test",
        "definition": { "nodes": [], "edges": [] }
    });

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::to_string(&create_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "step 1: POST /workflows must return 201"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = created_workflow["id"]
        .as_str()
        .expect("created workflow must have an id")
        .to_string();

    assert_eq!(
        created_workflow["name"], "Knife Scenario Workflow",
        "step 1: name must round-trip"
    );

    // Round-trip: GET must return the same workflow.
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "step 1: GET /workflows/:id must return 200"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let fetched_workflow: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        fetched_workflow["id"].as_str(),
        Some(workflow_id.as_str()),
        "step 1: round-trip id must match"
    );
    assert_eq!(
        fetched_workflow["name"], "Knife Scenario Workflow",
        "step 1: round-trip name must match"
    );

    // ── Step 2a: Activate a valid workflow — must succeed with 200 ───────────
    //
    // Canon §13 step 2: "Activation runs validation and rejects invalid
    // definitions — it does not silently flip a flag."
    //
    // The workflow created above has an empty definition which isn't a
    // structurally valid WorkflowDefinition (it lacks the required fields for
    // `validate_workflow`). We therefore write a structurally valid definition
    // directly to the repo (as the existing `activate_valid_returns_200` test
    // does) so we can assert the valid-activation path.

    let valid_wf_id = nebula_core::WorkflowId::new();
    state
        .workflow_repo
        .save(valid_wf_id, 0, make_valid_workflow_definition(&valid_wf_id))
        .await
        .unwrap();

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/workflows/{valid_wf_id}/activate"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "step 2a: activate valid workflow must return 200"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let activated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        activated["id"].as_str(),
        Some(valid_wf_id.to_string().as_str()),
        "step 2a: activated response must echo the workflow id"
    );

    // ── Step 2b: Activate an invalid (cyclic) workflow — must return 422 ─────
    //
    // Canon §13 step 2: "rejects invalid definitions with structured RFC 9457
    // errors"
    //
    // The cyclic definition parses as WorkflowDefinition but fails the DAG
    // cycle check in validate_workflow.

    let cyclic_wf_id = nebula_core::WorkflowId::new();
    state
        .workflow_repo
        .save(
            cyclic_wf_id,
            0,
            make_cyclic_workflow_definition(&cyclic_wf_id),
        )
        .await
        .unwrap();

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/workflows/{cyclic_wf_id}/activate"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "step 2b: activate cyclic workflow must return 422"
    );

    // RFC 9457: Content-Type must be application/problem+json
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        content_type,
        Some("application/problem+json"),
        "step 2b: 422 body must use RFC 9457 content-type"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        problem["status"], 422,
        "step 2b: RFC 9457 status field must be 422"
    );
    assert!(
        problem["type"].as_str().is_some(),
        "step 2b: RFC 9457 type field must be present"
    );
    assert!(
        problem["errors"].as_array().is_some_and(|e| !e.is_empty()),
        "step 2b: RFC 9457 errors array must be present and non-empty"
    );

    // ── Step 3: Start an execution ───────────────────────────────────────────
    //
    // Canon §13 step 3: "The execution row exists with consistent status,
    // monotonic version, and a real started_at (no synthetic zero, no
    // placeholder now() where the field should be None)."
    //
    // POST /api/v1/workflows/:id/executions → 202.
    // `started_at` must be > 0 (real chrono timestamp).
    // `finished_at` must be absent from the JSON (Option::None, skipped by
    // serde).
    // `status` must be "pending".
    //
    // Note: `ExecutionResponse` does not expose a `version` field — the repo
    // stores a version but the DTO omits it. The "monotonic version" invariant
    // is enforced at the storage layer; the API test can only observe the DTO.

    let start_request = serde_json::json!({
        "input": { "knife_key": "knife_value" }
    });

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/workflows/{workflow_id}/executions"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::to_string(&start_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "step 3: start execution must return 202"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let execution_id = execution_response["id"]
        .as_str()
        .expect("step 3: execution response must have an id")
        .to_string();

    assert_eq!(
        execution_response["status"].as_str(),
        Some("pending"),
        "step 3: initial status must be 'pending'"
    );

    let started_at = execution_response["started_at"]
        .as_i64()
        .expect("step 3: started_at must be a number");
    assert!(
        started_at > 0,
        "step 3: started_at must be a real chrono timestamp, got {started_at}"
    );

    // finished_at must be absent from the JSON (serde skips None fields).
    assert!(
        execution_response.get("finished_at").is_none()
            || execution_response["finished_at"].is_null(),
        "step 3: finished_at must be absent (None) on a pending execution, got: {:?}",
        execution_response.get("finished_at")
    );

    // ── Step 4: Observe via GET — finished_at is null, status is latest ──────
    //
    // Canon §13 step 4: "finished_at is None (not 0) until terminal; status
    // reflects the latest persisted value."

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/executions/{execution_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "step 4: GET /executions/:id must return 200"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        observed["id"].as_str(),
        Some(execution_id.as_str()),
        "step 4: observed id must match"
    );
    assert_eq!(
        observed["status"].as_str(),
        Some("pending"),
        "step 4: status must reflect the latest persisted value (pending)"
    );

    // finished_at must be absent (not "0") — canon explicitly forbids synthetic zero.
    let finished_at_value = observed.get("finished_at");
    let finished_at_is_zero = finished_at_value
        .and_then(|v| v.as_i64())
        .map(|v| v == 0)
        .unwrap_or(false);
    assert!(
        !finished_at_is_zero,
        "step 4: finished_at must NOT be synthetic 0 — must be absent or a real timestamp"
    );
    // Also verify it is either absent or null — not a number for a pending execution.
    let is_absent_or_null = finished_at_value.is_none() || finished_at_value.unwrap().is_null();
    assert!(
        is_absent_or_null,
        "step 4: finished_at must be absent/null for non-terminal execution, got: {finished_at_value:?}"
    );

    // ── Step 5: Cancel — DB transition + control queue enqueue in same op ────
    //
    // Canon §13 step 5: "the handler transitions through ExecutionRepo (CAS),
    // the same logical operation enqueues Cancel in execution_control_queue,
    // …the execution reaches a terminal Cancelled state."
    //
    // We assert both the durable row and the queue entry in a single test body,
    // proving the §12.2 same-logical-operation guarantee.

    // Pre-condition: queue is empty before cancel.
    assert!(
        control_queue.snapshot().await.is_empty(),
        "step 5 pre-condition: control queue must be empty before cancel"
    );

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/executions/{execution_id}/cancel"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "step 5: cancel must return 200"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let cancelled: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Observation 1: execution row must reflect cancelled state.
    assert_eq!(
        cancelled["status"].as_str(),
        Some("cancelled"),
        "step 5: execution row must show 'cancelled' status"
    );
    assert!(
        cancelled["finished_at"].as_i64().is_some_and(|t| t > 0),
        "step 5: finished_at must be a real timestamp after cancellation, got: {:?}",
        cancelled.get("finished_at")
    );

    // Observation 2: control queue must have exactly one Cancel entry.
    // Both observations are in this single test body — §12.2 same-logical-operation.
    let queued = control_queue.snapshot().await;
    assert_eq!(
        queued.len(),
        1,
        "step 5: exactly one control queue entry must exist after cancel"
    );

    let entry = &queued[0];
    assert_eq!(
        entry.command,
        ControlCommand::Cancel,
        "step 5: queued command must be Cancel"
    );
    assert_eq!(
        entry.status, "Pending",
        "step 5: queue entry must be in Pending state (not yet consumed by engine)"
    );

    // The entry's execution_id bytes must decode back to the cancelled execution.
    let queued_eid =
        String::from_utf8(entry.execution_id.clone()).expect("execution_id must be UTF-8");
    assert_eq!(
        queued_eid, execution_id,
        "step 5: queue entry must reference the cancelled execution"
    );

    // Verify DB state persisted via a GET (not just the cancel response).
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/executions/{execution_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "step 5 verify: GET after cancel must return 200"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let after_cancel: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        after_cancel["status"].as_str(),
        Some("cancelled"),
        "step 5 verify: GET after cancel must persist 'cancelled' status"
    );
}

/// Canon §13 step 6 — "orchestration absent" scenario.
///
/// When the control queue backend is unavailable, the cancel endpoint must
/// return an error (not fake success).
///
/// **Known canon gap:** §13 step 6 specifies **503** Service Unavailable.
/// The current `cancel_execution` handler maps enqueue failures to
/// `ApiError::Internal` → **500**. This test documents the actual behavior
/// (500) so the discrepancy is explicit and tracked.
///
/// To fix: change the cancel handler to map `StorageError` from enqueue to
/// `ApiError::ServiceUnavailable` (503). The assertion below is marked
/// `// CANON-GAP §13-step6` — flip it to `SERVICE_UNAVAILABLE` once the
/// handler is corrected.
///
/// Audit ref: 2026-04-16-workspace-health-audit.md §8 Sprint A1 item #3
#[tokio::test]
async fn knife_step6_queue_failure_returns_error() {
    use nebula_core::{ExecutionId, WorkflowId};

    let state = create_state_with_failing_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a running execution directly into the repo.
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = chrono::Utc::now().timestamp();

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

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/executions/{execution_id}/cancel"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // CANON-GAP §13-step6: canon mandates 503 but handler currently returns 500.
    // This assertion documents the actual behavior — change to SERVICE_UNAVAILABLE
    // once the handler is corrected to use ApiError::ServiceUnavailable for
    // control-queue enqueue failures.
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "step 6: queue failure must not return fake success; \
         expected 500 (actual) — should become 503 after handler fix (canon §13 step 6)"
    );

    // Whatever the error code, it must NOT be 2xx (no fake success).
    assert!(
        response.status().is_server_error(),
        "step 6: a queue-backend failure must always result in a server error, not 2xx"
    );
}
