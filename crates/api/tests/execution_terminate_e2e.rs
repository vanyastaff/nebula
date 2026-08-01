//! `POST /executions/:id/terminate` producer durability and running-handler
//! race coverage.
//!
//! `terminate_execution` graduated stub→implemented as a real durable control queue
//! endpoint. It mirrors `cancel_execution` except that it enqueues
//! `ControlCommand::Terminate`; the operator-visible terminal state is
//! `ExecutionStatus::Cancelled` because no `Terminated` variant exists
//! (`crates/execution/src/status.rs`). The running-handler fixture manually
//! installs a consumer over shared in-memory ports so the request can race
//! with active work, but it exposes no handler-exit signal. Its terminal row
//! is written by the API and is not evidence of consumer delivery or handler
//! interruption. Current first-party composition roots do not install that
//! consumer. The separate `CANCELFX` gate owns interruption evidence.
//!
//! The engine-seam harness (`common::engine_seam`) and the legacy failing
//! control queue (`common::create_state_with_failing_queue`) are shared with
//! `knife.rs` so the cancel/terminate seam wiring lives in exactly one place.
//!
//! ## Coverage
//!
//! | Scenario | What is asserted | Test |
//! |----------|------------------|------|
//! | Running-handler producer boundary | Slow handler starts → POST terminate persists terminal `Cancelled` + a `Terminate` entry for that execution; the terminal row is explicitly not a handler-exit oracle | `terminate_persists_terminal_state_and_control_intent_while_handler_runs` |
//! | Producer durability | POST terminate persists `cancelled` + enqueues exactly one `Terminate` entry referencing the execution | `terminate_enqueues_durable_control_signal` |
//! | Atomic outbox | legacy control-queue handle down → POST terminate still commits state + `Terminate` outbox together via `TransitionBatch` | `terminate_control_signal_is_atomic_with_state` |
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

// ── Running-handler producer boundary (not an interruption oracle) ──────────
//
// Symmetric to
// `knife.rs::knife_step5_api_persists_terminal_cancel_control_intent_while_handler_runs`,
// but exercises the `Terminate` command instead of `Cancel`. The wiring:
//
//   POST /executions/:id/terminate
//     → execution_store CAS-transition (Cancelled)    [API producer]
//     → control_queue.enqueue(Terminate)              [API producer]
//
// The shared `common::engine_seam` starts the cancellable `slow` node before
// the request. It exposes only `slow_started`, not completion of the
// `Terminate` command or handler exit. The terminal assertion therefore
// verifies the API producer result only; live interruption remains outside
// this test's evidence.

/// Persist terminal `Terminate` control intent while a handler is running.
///
/// The manually composed harness establishes active work. This test does not
/// observe downstream command delivery or handler exit.
#[tokio::test]
async fn terminate_records_intent_without_terminalizing_a_running_handler() {
    use std::time::Duration;

    use nebula_execution::ExecutionStatus;
    use nebula_storage_port::dto::ControlCommand;

    // Port handles: `handles.control_queue` is the non-consuming outbox
    // snapshot; `handles.{seed_execution,execution_state}` are the port
    // equivalents of the old `state.execution_repo.{create,get_state}`.
    let (state, handles) = create_state_with_port_handles().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // ── Persist a single-`slow`-node workflow (shared harness) ───────────────
    let workflow_id = engine_seam::persist_slow_workflow(&state).await;

    // ── Start a handler through the manually composed engine harness ────────
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

    // ── Wait until the slow handler is active ───────────────────────────────
    tokio::time::timeout(Duration::from_secs(10), seam.slow_started.notified())
        .await
        .expect("slow handler started within 10s after the test-installed consumer drained Start");

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
        StatusCode::ACCEPTED,
        "terminate must return 202 — it records intent, it does not stop the run"
    );
    let body = axum::body::to_bytes(terminate_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let terminated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(
        terminated["status"].as_str(),
        Some("cancelled"),
        "the handler is still running, so the response must not report the run as cancelled"
    );

    // The control queue must hold the `Start` from the producer path AND a
    // fresh `Terminate` from the endpoint under test, proving the durable
    // producer enqueue happened in the same logical operation.
    let queued = handles.control_queue.snapshot();
    let (terminate_msg, _status) = queued
        .iter()
        .find(|(msg, _status)| msg.command == ControlCommand::Terminate)
        .expect("a Terminate entry must be present in the durable control queue");
    assert_eq!(
        terminate_msg.execution_id, execution_id_str,
        "Terminate entry must reference the terminated execution"
    );

    // The load-bearing assertion: the slow handler is *still running*, so the
    // durable row must not say the execution is over. The API used to write
    // `Cancelled` plus a `completed_at` of its own making here, under a fencing
    // token rebuilt from a read — a terminal claim about work still in flight,
    // made by a process that held no lease. Only the runtime terminalizes, and
    // only once it has honored the command.
    let execution_id = nebula_core::ExecutionId::parse(&execution_id_str).unwrap();
    let (_version, json) = handles
        .execution_state(execution_id)
        .await
        .expect("execution row present");
    let persisted: ExecutionStatus =
        serde_json::from_value(json.get("status").cloned().unwrap()).unwrap();
    assert!(
        !persisted.is_terminal(),
        "the API must not terminalize an execution whose handler is still running; got {persisted:?}"
    );
    assert!(
        json.get("completed_at")
            .is_none_or(serde_json::Value::is_null),
        "no completion timestamp may be stamped for work that has not completed"
    );

    // Handler exit is outside this test's oracle; abort and join the
    // unobserved consumer so cleanup cannot masquerade as evidence.
    seam.abort_unobserved_consumer().await;
}

// ── Producer durability + parity coverage (mirrors cancel) ───────────────────

/// durable control queue: terminating a non-terminal execution must
/// (1) enqueue a `Terminate` command in the durable control queue, and
/// (2) leave the execution row alone — the runtime owns that write.
/// Mirror of `integration_tests.rs::cancel_enqueues_durable_control_signal`.
#[tokio::test]
async fn terminate_enqueues_durable_control_signal() {
    use nebula_core::{ExecutionId, WorkflowId};
    use nebula_storage_port::dto::ControlCommand;

    let (state, handles) = create_state_with_port_handles().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    handles
        .seed_execution(
            execution_id,
            workflow_id,
            serde_json::json!({
                "workflow_id": workflow_id.to_string(),
                "status": "running",
                "started_at": now,
                "input": {}
            }),
        )
        .await;

    assert!(
        handles.control_queue.snapshot().is_empty(),
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
        StatusCode::ACCEPTED,
        "terminate must return 202 — the request is accepted, the run has not stopped"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let terminated: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // (1) The response describes the execution as it stands — still running —
    // and the row is untouched. The runtime performs the transition.
    assert_eq!(
        terminated["status"], "running",
        "the response must report the persisted state, not one the handler asserted"
    );
    assert!(
        terminated["finished_at"].is_null(),
        "no finish timestamp may be reported for a run that has not finished"
    );
    let (_version, persisted) = handles
        .execution_state(execution_id)
        .await
        .expect("execution row present");
    assert_eq!(
        persisted.get("status").and_then(serde_json::Value::as_str),
        Some("running"),
        "terminate must not write the execution aggregate"
    );

    // (2) Exactly one Terminate command must have been written to the queue.
    let queued = handles.control_queue.snapshot();
    assert_eq!(
        queued.len(),
        1,
        "exactly one control queue entry must exist after terminate"
    );
    let (msg, status) = &queued[0];
    assert_eq!(
        msg.command,
        ControlCommand::Terminate,
        "queued command must be Terminate"
    );
    assert_eq!(
        status, "Pending",
        "entry must be in Pending state (not yet consumed by engine)"
    );
    assert_eq!(
        msg.execution_id,
        execution_id.to_string(),
        "queued entry must reference the terminated execution"
    );
}

/// A control queue that cannot accept the command must surface as an error, not
/// as a silent success.
///
/// This used to be an atomicity test: the handler wrote the terminal state and
/// the outbox row in one `TransitionBatch`, so a failing queue could not orphan
/// a cancelled row. The handler writes no state at all now, so there is nothing
/// to keep in step — the enqueue *is* the whole operation, and a failure means
/// nothing happened and the caller must be told.
#[tokio::test]
async fn terminate_reports_a_failed_enqueue_and_writes_nothing() {
    use nebula_core::{ExecutionId, WorkflowId};

    let (state, exec_store) = create_state_with_failing_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = chrono::Utc::now().timestamp();

    // Seed via the shared port execution store (the failing-queue harness
    // returns the raw store; the `AppState` decorators read the same core).
    {
        use nebula_storage_port::store::ExecutionStore;
        ExecutionStore::create(
            &exec_store,
            &port_scope(),
            &execution_id.to_string(),
            &workflow_id.to_string(),
            serde_json::json!({
                "workflow_id": workflow_id.to_string(),
                "status": "running",
                "started_at": now,
                "input": {}
            }),
        )
        .await
        .expect("seed execution: port create must succeed");
    }

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
        "a control queue that cannot accept the command must be reported, not swallowed"
    );

    // Nothing was written: no command row, and the execution is untouched.
    let queue = nebula_storage::inmem::InMemoryControlQueue::new(&exec_store);
    assert!(
        queue.snapshot().is_empty(),
        "a failed enqueue must leave no command row behind"
    );
    {
        use nebula_storage_port::store::ExecutionStore;
        let record = ExecutionStore::get(&exec_store, &port_scope(), &execution_id.to_string())
            .await
            .expect("read back the seeded execution")
            .expect("execution row must still exist");
        assert_eq!(
            record
                .state
                .get("status")
                .and_then(serde_json::Value::as_str),
            Some("running"),
            "a failed terminate must not have moved the execution"
        );
    }
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
/// must NOT enqueue a spurious `Terminate` signal (idempotency / durable control queue
/// terminal-state guard). Mirror of
/// `integration_tests.rs::cancel_terminal_execution_does_not_enqueue`.
#[tokio::test]
async fn terminate_terminal_execution_rejected_and_does_not_enqueue() {
    use nebula_core::{ExecutionId, WorkflowId};

    let (state, handles) = create_state_with_port_handles().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    handles
        .seed_execution(
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
        .await;

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
        handles.control_queue.snapshot().is_empty(),
        "control queue must be empty after rejected terminate of terminal execution"
    );
}
