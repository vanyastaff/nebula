//! `POST /executions/:id/terminate` end-to-end + parity coverage.
//!
//! `terminate_execution` graduated stub→implemented as a real canon §12.2
//! durable-control-plane endpoint (ADR-0008 A3 / ADR-0016). It mirrors
//! `cancel_execution` exactly except it enqueues
//! `ControlCommand::Terminate`. Per ADR-0016 the engine has no distinct
//! forced-shutdown path: `Terminate` is wired end-to-end
//! (`ControlConsumer` → `EngineControlDispatch::dispatch_terminate` →
//! `dispatch_cancel` → the engine cancel registry's live
//! `CancellationToken`) and the operator-visible terminal state is
//! `ExecutionStatus::Cancelled` (no `Terminated` variant exists —
//! `crates/execution/src/status.rs`).
//!
//! The engine-seam harness (`common::engine_seam`) and the
//! orchestration-absent control queue (`common::create_state_with_failing_queue`)
//! are shared with `knife.rs` so the cancel/terminate seam wiring lives in
//! exactly one place.
//!
//! ## Coverage
//!
//! | Scenario | What is asserted | Test |
//! |----------|------------------|------|
//! | Engine-visible seam (canon §13 bar) | Running exec + POST terminate → control_queue gets a `Terminate` entry → the wired real engine consumer drives the execution to terminal `Cancelled`, well inside the 30s slow-handler window | `terminate_engine_drives_running_execution_to_terminal_end_to_end` |
//! | Producer durability | POST terminate persists `cancelled` + enqueues exactly one `Terminate` entry referencing the execution | `terminate_enqueues_durable_control_signal` |
//! | 503 orchestration absent | control-queue backend down → 503 (mirrors knife step 6) | `terminate_queue_failure_returns_503` |
//! | 404 | unknown execution → 404 | `terminate_unknown_execution_returns_404` |
//! | 404 malformed id | malformed execution-id path segment → 404 (tenancy middleware rejects it before the handler runs) | `terminate_invalid_execution_id_rejected_by_middleware` |
//! | 400 terminal guard | already-terminal execution → 400, no spurious enqueue | `terminate_terminal_execution_rejected_and_does_not_enqueue` |

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use nebula_api::{ApiConfig, app};
use tower::ServiceExt;

// ── Engine-visible seam (canon §13 integration bar) ──────────────────────────
//
// Symmetric to `knife.rs::knife_step5_engine_cancels_running_execution_end_to_end`,
// but exercises the `Terminate` command instead of `Cancel`. The wiring:
//
//   POST /executions/:id/terminate
//     → execution_repo.transition (Cancelled)        [API handler, §12.2 order]
//     → execution_control_queue.enqueue(Terminate)   [API handler, §12.2 order]
//     → ControlConsumer.claim_pending
//     → EngineControlDispatch::dispatch_terminate     (ADR-0008 A3 / ADR-0016)
//     → EngineControlDispatch::dispatch_cancel        (Terminate == cooperative
//                                                      cancel synonym today)
//     → WorkflowEngine::cancel_execution              (live cancel registry)
//     → frontier loop observes `ctx.cancellation()` → slow node exits
//
// The engine + consumer + cancellable `slow` node wiring is the shared
// `common::engine_seam` harness — byte-behaviorally identical to the
// inline knife step-5 wiring. This test differs from `knife_step5` ONLY
// in the final HTTP call (POST-terminate vs DELETE-cancel) and the
// command/terminal assertion. Asserting the execution reaches a terminal
// state inside the 30s slow window proves the `Terminate` signal reached
// the engine's *live* loop — not merely that the API CAS-flipped the row
// (the two-truth gap canon §14 calls out).

/// Engine-visible seam: a Running execution + `POST .../terminate` drives
/// the execution all the way to a terminal state via the real engine
/// consumer — proving the durable `Terminate` signal reaches the live
/// frontier loop (ADR-0008 A3 / ADR-0016), not just the DB row.
#[tokio::test]
async fn terminate_engine_drives_running_execution_to_terminal_end_to_end() {
    use std::time::Duration;

    use nebula_execution::ExecutionStatus;
    use nebula_storage::repos::ControlCommand;

    let (state, control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // ── Persist a single-`slow`-node workflow (shared harness) ───────────────
    let workflow_id = engine_seam::persist_slow_workflow(&state).await;

    // ── Build + spawn the real engine consumer (shared harness) ──────────────
    let seam = engine_seam::spawn_engine_consumer(&state);

    // ── Start the execution via the producer path ───────────────────────────
    let start_request = serde_json::json!({ "input": { "terminate_e2e": true } });
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id}/executions")))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
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
        "start execution must return 202"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let execution_id_str = execution_response["id"]
        .as_str()
        .expect("start response carries an id")
        .to_string();

    // ── Wait until the slow handler enters its select{} — frontier is live ──
    tokio::time::timeout(Duration::from_secs(10), seam.slow_started.notified())
        .await
        .expect(
            "slow handler started within 10s (consumer drained Start and the engine \
             dispatched the node)",
        );

    // ── Terminate via the API — the endpoint under test ─────────────────────
    let terminate_app = app::build_app(state.clone(), &api_config);
    let terminate_response = terminate_app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!(
                    "/executions/{execution_id_str}/terminate"
                )))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        terminate_response.status(),
        StatusCode::OK,
        "terminate must return 200"
    );
    let body = axum::body::to_bytes(terminate_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let terminated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        terminated["status"].as_str(),
        Some("cancelled"),
        "terminate response must show terminal `cancelled` status (ADR-0016: \
         Terminate is a cooperative-cancel synonym, no `Terminated` variant)"
    );

    // The control queue must hold the `Start` from the producer path AND a
    // fresh `Terminate` from the endpoint under test — proving the §12.2
    // same-logical-operation enqueue happened, engine-visible.
    let queued = control_queue.snapshot().await;
    let terminate_entry = queued
        .iter()
        .find(|e| e.command == ControlCommand::Terminate)
        .expect("a Terminate entry must be present in the durable control queue");
    let queued_eid = String::from_utf8(terminate_entry.execution_id.clone())
        .expect("execution_id bytes must be valid UTF-8");
    assert_eq!(
        queued_eid, execution_id_str,
        "Terminate entry must reference the terminated execution"
    );

    // ── The execution must reach a terminal state well inside the slow
    //    handler's 30s sleep — proving the Terminate reached the engine's
    //    live cancel token and the handler exited cooperatively.
    let execution_id = nebula_core::ExecutionId::parse(&execution_id_str).unwrap();
    let final_status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (_version, json) = state
                .execution_repo
                .get_state(execution_id)
                .await
                .unwrap()
                .expect("execution row present");
            let status: ExecutionStatus =
                serde_json::from_value(json.get("status").cloned().unwrap()).unwrap();
            if status.is_terminal() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect(
        "engine reached a terminal state within 10s (Terminate dispatch signalled the \
         live frontier loop via the ADR-0016 cancel registry) — the 30s slow handler \
         was aborted cooperatively, not left sleeping",
    );

    assert!(
        final_status.is_terminal(),
        "execution reached a terminal state after Terminate — the engine honors \
         ControlCommand::Terminate end-to-end (ADR-0008 A3 / ADR-0016). got: {final_status:?}"
    );

    // Graceful shutdown so the spawned consumer task doesn't leak.
    seam.shutdown().await;
}

// ── Producer durability + parity coverage (mirrors cancel) ───────────────────

/// Canon §12.2: terminating a non-terminal execution must both
/// (1) persist the terminal state in the execution row, AND
/// (2) enqueue a `Terminate` command in the durable control queue.
/// Mirror of `integration_tests.rs::cancel_enqueues_durable_control_signal`.
#[tokio::test]
async fn terminate_enqueues_durable_control_signal() {
    use nebula_core::{ExecutionId, WorkflowId};
    use nebula_storage::repos::ControlCommand;

    let (state, control_queue) = create_state_with_queue().await;
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
                "status": "running",
                "started_at": now,
                "input": {}
            }),
        )
        .await
        .unwrap();

    assert!(
        control_queue.snapshot().await.is_empty(),
        "control queue must be empty before terminate"
    );

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/executions/{execution_id}/terminate")))
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
        StatusCode::OK,
        "terminate must return 200"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let terminated: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // (1) Execution row must reflect the terminal (cancelled) state.
    assert_eq!(
        terminated["status"], "cancelled",
        "execution row must show terminal `cancelled` status"
    );
    assert!(
        terminated["finished_at"].is_number(),
        "finished_at must be set after terminate"
    );

    // (2) Exactly one Terminate command must have been written to the queue.
    let queued = control_queue.snapshot().await;
    assert_eq!(
        queued.len(),
        1,
        "exactly one control queue entry must exist after terminate"
    );
    let entry = &queued[0];
    assert_eq!(
        entry.command,
        ControlCommand::Terminate,
        "queued command must be Terminate"
    );
    assert_eq!(
        entry.status, "Pending",
        "entry must be in Pending state (not yet consumed by engine)"
    );
    let queued_eid = String::from_utf8(entry.execution_id.clone())
        .expect("execution_id bytes must be valid UTF-8");
    assert_eq!(
        queued_eid,
        execution_id.to_string(),
        "queued entry must reference the terminated execution"
    );
}

/// Canon §13 step 6 — "orchestration absent": when the control-queue
/// backend is unavailable, terminate must return **503** with RFC 9457
/// problem+json, not fake success and not an unparsable 500. Uses the
/// shared `common::create_state_with_failing_queue` double (mirror of
/// `knife.rs::knife_step6_queue_failure_returns_error`).
#[tokio::test]
async fn terminate_queue_failure_returns_503() {
    use nebula_core::{ExecutionId, WorkflowId};

    let state = create_state_with_failing_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

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
                .uri(ws_path(&format!("/executions/{execution_id}/terminate")))
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
        "orchestration-absent enqueue failure must return 503 (canon §13 step 6)"
    );

    // RFC 9457: failure body must be application/problem+json (§12.4).
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        content_type,
        Some("application/problem+json"),
        "503 body must use the RFC 9457 content-type"
    );
}

/// Terminating a non-existent execution must return 404. Mirror of
/// `integration_tests.rs::test_execution_cancel_not_found`.
#[tokio::test]
async fn terminate_unknown_execution_returns_404() {
    let (state, _control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let nonexistent_id = nebula_core::ExecutionId::new().to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/executions/{nonexistent_id}/terminate")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A malformed execution-id path segment is rejected with **404** by the
/// tenancy/path middleware *before* the handler runs — the established,
/// already-locked API contract for malformed tenant-scoped path segments
/// (see `integration_tests.rs::test_execution_get_invalid_id` and the
/// "invalid UUID format (caught by tenancy middleware)" 404 case in
/// `integration_tests.rs`). The handler's own `ExecutionId::parse` 400
/// guard (identical to `cancel_execution`'s) covers syntactically-parsed
/// ids; it sits behind this middleware and is exercised by the
/// not-found / terminal-guard parity tests.
#[tokio::test]
async fn terminate_invalid_execution_id_rejected_by_middleware() {
    let (state, _control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let token = create_test_jwt();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path("/executions/not-a-valid-ulid/terminate"))
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
        StatusCode::NOT_FOUND,
        "malformed execution-id path segment is rejected with 404 by the \
         tenancy/path middleware before the handler runs (established API \
         contract — mirrors test_execution_get_invalid_id)"
    );
}

/// Terminating an already-terminal execution must be rejected with 400 and
/// must NOT enqueue a spurious `Terminate` signal (idempotency / §12.2
/// terminal-state guard). Mirror of
/// `integration_tests.rs::cancel_terminal_execution_does_not_enqueue`.
#[tokio::test]
async fn terminate_terminal_execution_rejected_and_does_not_enqueue() {
    use nebula_core::{ExecutionId, WorkflowId};

    let (state, control_queue) = create_state_with_queue().await;
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
                "status": "completed",
                "started_at": now,
                "finished_at": now + 5,
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
                .uri(ws_path(&format!("/executions/{execution_id}/terminate")))
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
        StatusCode::BAD_REQUEST,
        "terminate on completed execution must return 400"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        error["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("Cannot terminate execution"),
        "400 body must carry the terminal-guard message; got: {error:?}"
    );

    assert!(
        control_queue.snapshot().await.is_empty(),
        "control queue must be empty after rejected terminate of terminal execution"
    );
}
