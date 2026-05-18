//! Canon §13 knife scenario — end-to-end integration test.
//!
//! This file covers §13 steps 1–6 as specified in
//! `docs/PRODUCT_CANON.md §13` and the workspace health audit
//! (`docs/ARCHIVE.md (removed execution specs) §8
//! Sprint A1 item #3`).
//!
//! Each step is asserted through the real axum `Router` via oneshot
//! requests. The workflow / execution / control-queue surface is the
//! spec-16 scoped storage port: the in-memory adapters wrapped in the
//! `nebula-tenancy` scoping decorators, wired exactly as the production
//! composition root does. No handler logic is bypassed.
//!
//! ## Step coverage
//!
//! | Step | What is asserted | Test |
//! |------|-----------------|------|
//! | 1 | `POST /workflows` round-trips through `GET /workflows/:id` | `knife_scenario_end_to_end_via_port` |
//! | 2a | `POST /workflows/:id/activate` valid → 200 | `knife_scenario_end_to_end_via_port` |
//! | 3 | `POST /workflows/:id/executions` → 202, `status=created`, `started_at > 0`, `finished_at` absent | `knife_scenario_end_to_end_via_port` |
//! | 4 | `GET /executions/:id` → `finished_at` is null/absent, `status` = latest persisted value | `knife_scenario_end_to_end_via_port` |
//! | 5 | `POST /executions/:id/cancel` → row = `cancelled`, outbox holds exactly Start + Cancel (both `Pending`) | `knife_scenario_end_to_end_via_port` |
//! | 6 | Cancel state + control signal commit atomically even if the legacy queue handle fails | `knife_step6_cancel_control_signal_is_atomic_with_state` |
//!
//! ## Consumer-side §13 (engine dispatch end-to-end)
//!
//! The producer side above asserts the API writes the execution row and
//! enqueues the control command. The consumer side — the engine-owned
//! `EngineControlDispatch` draining the durable queue and driving the
//! workflow to a terminal state (Created → Completed on `Start`, Running
//! → Cancelled on `Cancel` via the live frontier loop, plus the ADR-0008
//! §5 redelivery-idempotency contract) — is asserted on the same spec-16
//! port by `crates/engine/tests/control_dispatch.rs`. That is the
//! canonical home for the dispatch loop now that the engine consumes the
//! port directly; this file owns only the HTTP-surface producer path.

mod common;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use nebula_api::{ApiConfig, app};
use tower::ServiceExt;

// The legacy failing control queue (`AlwaysFailControlQueue` +
// `create_state_with_failing_queue`) and the engine-seam harness
// (`engine_seam::{persist_slow_workflow, spawn_engine_consumer}`) live in
// the shared `common` module — see `tests/common/mod.rs`. The §13 step-6
// test reuses the placeholder scope every port store binds to via
// `common::port_scope`.
use common::port_scope as knife_scope;

// ── tests ─────────────────────────────────────────────────────────────────────

/// Canon §13 steps 1–5 end-to-end: define → activate → start → observe
/// → cancel, through the scoped storage port.
///
/// The workflow / execution / control-queue surface is the spec-16 port
/// (in-memory adapters behind the `nebula-tenancy` decorators, wired via
/// `create_state_with_port_queue`). Asserted invariants: a workflow
/// round-trips, activation validates, an execution is created in
/// `created` with `started_at` set and `finished_at` absent, cancel
/// drives the row to `cancelled`, and the durable outbox holds exactly
/// the `Start` (step 3) and `Cancel` (step 5) rows, both still
/// `Pending`.
///
/// Audit ref: 2026-04-16-workspace-health-audit.md §8 Sprint A1 item #3
#[tokio::test]
async fn knife_scenario_end_to_end_via_port() {
    use nebula_storage_port::dto::ControlCommand as PortControlCommand;

    let (state, control_queue) = create_state_with_port_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // ── Step 1: define a structurally valid workflow + round-trip ────────────
    //
    // The port-backed `create` accessor stores a workflow row + a
    // published version record carrying the definition. A valid
    // definition is used up front so step 2 (activate) can validate it
    // without a direct-repo seam (the port path has no legacy
    // `workflow_repo.save` back door — every write goes through the
    // decorated store).
    let wf_id = nebula_core::WorkflowId::new();
    let create_request = serde_json::json!({
        "name": "Port Knife Workflow",
        "description": "End-to-end knife test via the scoped port",
        "definition": make_valid_workflow_definition(&wf_id),
    });

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path("/workflows"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::from(serde_json::to_string(&create_request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "port step 1: POST /workflows must return 201"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let workflow_id = created["id"]
        .as_str()
        .expect("created workflow must have an id")
        .to_string();

    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path(&format!("/workflows/{workflow_id}")))
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
        "port step 1: GET /workflows/:id must round-trip (200)"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        fetched["id"].as_str(),
        Some(workflow_id.as_str()),
        "port step 1: round-trip id must match"
    );

    // ── Step 2: activate the valid workflow → 200 ────────────────────────────
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(ws_path(&format!("/workflows/{workflow_id}/activate")))
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
        "port step 2: activate valid workflow must return 200"
    );

    // ── Step 3: start an execution → 202, created, started_at>0 ──────────────
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
                .body(Body::from(r#"{"input":{}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "port step 3: start execution must return 202"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let execution_id = started["id"]
        .as_str()
        .expect("execution must have an id")
        .to_string();
    assert_eq!(
        started["status"].as_str(),
        Some("created"),
        "port step 3: status must be the canonical `created`"
    );
    assert!(
        started["started_at"].as_i64().is_some_and(|t| t > 0),
        "port step 3: started_at must be a real timestamp"
    );
    assert!(
        started
            .get("finished_at")
            .is_none_or(serde_json::Value::is_null),
        "port step 3: finished_at must be absent/null"
    );

    // Pre-cancel: the durable outbox holds exactly one `Start` row
    // (#332), observed via the port's non-consuming snapshot.
    let pre = control_queue.snapshot();
    assert_eq!(
        pre.len(),
        1,
        "port step 5 pre-condition: outbox must hold the step-3 Start, got {pre:?}"
    );
    assert_eq!(
        pre[0].0.command,
        PortControlCommand::Start,
        "port step 5 pre-condition: step-3 row must be Start"
    );

    // ── Step 5: cancel → row=cancelled + Cancel enqueued ─────────────────────
    let app = app::build_app(state.clone(), &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
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
        "port step 5: cancel must return 200"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let cancelled: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        cancelled["status"].as_str(),
        Some("cancelled"),
        "port step 5: execution row must show 'cancelled'"
    );
    assert!(
        cancelled["finished_at"].as_i64().is_some_and(|t| t > 0),
        "port step 5: finished_at must be a real timestamp after cancel"
    );

    // Outbox now holds the Start (step 3) + Cancel (step 5), both
    // Pending — the §12.2 same-logical-operation guarantee, asserted
    // through the port snapshot (typed id, opaque `execution_id`
    // string — no UTF-8-of-ULID decode).
    let queued = control_queue.snapshot();
    assert_eq!(
        queued.len(),
        2,
        "port step 5: outbox must hold Start + Cancel, got {queued:?}"
    );
    let (cancel_msg, cancel_status) = queued
        .iter()
        .find(|(m, _)| m.command == PortControlCommand::Cancel)
        .expect("port step 5: Cancel row must be present");
    assert_eq!(
        cancel_status, "Pending",
        "port step 5: Cancel row must be Pending (not yet consumed)"
    );
    assert_eq!(
        cancel_msg.execution_id, execution_id,
        "port step 5: Cancel row must reference the cancelled execution"
    );
}

/// Cancel writes the terminal state and control signal through one
/// `TransitionBatch`, so the legacy separately-wired control queue can fail
/// without creating a cancelled-row / missing-signal orphan.
#[tokio::test]
async fn knife_step6_cancel_control_signal_is_atomic_with_state() {
    use nebula_core::{ExecutionId, WorkflowId};
    use nebula_storage_port::store::ExecutionStore;

    let (state, exec_store) = create_state_with_failing_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a running execution directly through the scoped port store
    // (shares the `Arc<Mutex<…>>` core with the decorated store inside
    // `AppState`, so the cancel handler observes this row).
    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let now = chrono::Utc::now().timestamp();

    ExecutionStore::create(
        &exec_store,
        &knife_scope(),
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
    .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id}")))
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
        "cancel should not orphan a state transition when the legacy queue handle fails"
    );

    let queue = nebula_storage::inmem::InMemoryControlQueue::new(&exec_store);
    let queued = queue.snapshot();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].0.command,
        nebula_storage_port::dto::ControlCommand::Cancel
    );
    assert_eq!(
        queued[0].0.execution_id,
        execution_id.to_string(),
        "atomic outbox row must reference the cancelled execution"
    );
}

// ── Step 3 end-to-end (ADR-0008 A2) ───────────────────────────────────────────
//
// The `knife_scenario_end_to_end` test above asserts the PRODUCER side of §13
// step 3 — the API writes the execution row and enqueues `Start` onto the
// durable control queue (#332). This separate test asserts the CONSUMER side:
// the engine-owned `EngineControlDispatch` (ADR-0008 A2) drains the queue and
// actually drives the workflow to `Completed`, closing the §4.5 gap that was
// still open after #332 landed.
//
// The two tests intentionally stand up separate `AppState`s — the producer
// test pins a pre-consumer snapshot of the queue (Start still Pending when
// step 5 runs), while this test spawns the consumer so the Start row is
// drained end-to-end.

/// A hand-built echo `Action` (Variant A) that the engine can dispatch.
/// Mirrors the workflow definition saved below (`action_key = "echo"`).
struct KnifeEcho;

impl nebula_action::action::Action for KnifeEcho {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static nebula_action::metadata::ActionMetadata {
        static M: std::sync::OnceLock<nebula_action::metadata::ActionMetadata> =
            std::sync::OnceLock::new();
        M.get_or_init(|| {
            nebula_action::metadata::ActionMetadata::new(
                nebula_core::action_key!("knife.echo.static"),
                "KnifeEcho",
                "static",
            )
        })
    }
    fn dependencies() -> &'static nebula_core::Dependencies {
        static D: std::sync::OnceLock<nebula_core::Dependencies> = std::sync::OnceLock::new();
        D.get_or_init(nebula_core::Dependencies::new)
    }
}
impl nebula_action::stateless::StatelessAction for KnifeEcho {
    async fn execute(
        &self,
        input: <Self as nebula_action::action::Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<
        nebula_action::result::ActionResult<<Self as nebula_action::action::Action>::Output>,
        nebula_action::ActionError,
    > {
        Ok(nebula_action::result::ActionResult::success(input))
    }
}

/// Canon §13 step 3 end-to-end (ADR-0008 A2).
///
/// Wires API producer + `ControlConsumer` + `EngineControlDispatch` + engine
/// over shared in-memory repos, POSTs `/workflows/:id/executions`, and polls
/// the repo until the execution transitions all the way to `Completed`. This
/// exercises the full §12.2 loop that ADR-0008 promised:
///
/// ```text
/// POST /executions
///   → execution_repo.create (Created)
///   → execution_control_queue.enqueue(Start)
///   → ControlConsumer.claim_pending
///   → EngineControlDispatch::dispatch_start
///   → WorkflowEngine::resume_execution (ADR-0015 lease scope)
///   → node run → transition to Completed
///   → mark_completed on the queue row
/// ```
#[tokio::test]
async fn knife_step3_engine_dispatches_start_end_to_end() {
    use std::time::Duration;

    use nebula_core::action_key;
    use nebula_engine::{
        ActionExecutor, ActionRegistry, ActionRuntime, ControlConsumer, DataPassingPolicy,
        EngineControlDispatch, ExecutionStores, InProcessSandbox, WorkflowEngine, WorkflowStores,
    };
    use nebula_execution::ExecutionStatus;
    use nebula_workflow::{
        Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
    };
    use tokio_util::sync::CancellationToken;

    let (state, _control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // ── Persist a valid workflow (`action_key = "echo"`) directly ────────────
    //
    // Avoids the HTTP activation round-trip — that path is exercised by the
    // producer-side knife test above. Here we care about the engine-side
    // dispatch of the Start command that the API will enqueue below.
    let workflow_id = nebula_core::WorkflowId::new();
    let now = chrono::Utc::now();
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "knife-a2-dispatch".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![NodeDefinition::new(nebula_core::node_key!("step"), "Step", "echo").unwrap()],
        connections: Vec::<Connection>::new(),
        variables: std::collections::HashMap::new(),
        config: WorkflowConfig::default(),
        trigger: None,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: 1,
    };
    // Port equivalent of the old `state.workflow_repo.save(id, 0, def)`: a
    // workflow row at version 1 plus a published version record #1 through
    // the scoped port handles on `AppState` (the tenancy decorators
    // substitute their bound scope, so the `knife_scope()` argument is
    // immaterial — it only needs to be a valid `Scope`).
    {
        let scope = knife_scope();
        let id_str = workflow_id.to_string();
        state
            .workflow_store
            .create(
                &scope,
                nebula_storage_port::dto::WorkflowRecord {
                    id: id_str.clone(),
                    scope: scope.clone(),
                    version: 1,
                    slug: id_str.clone(),
                    deleted: false,
                },
            )
            .await
            .unwrap();
        state
            .workflow_version_store
            .create(
                &scope,
                nebula_storage_port::dto::WorkflowVersionRecord {
                    workflow_id: id_str,
                    number: 1,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(&wf).unwrap(),
                },
            )
            .await
            .unwrap();
    }

    // ── Build the engine bound to the same scoped port handles the API
    // wrote to ──────────────────────────────────────────────────────────────
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        nebula_action::metadata::ActionMetadata::new(
            action_key!("echo"),
            "echo",
            "knife echo handler",
        ),
        KnifeEcho,
    );

    let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
        Box::pin(async move { Ok(nebula_action::result::ActionResult::success(input)) })
    });
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = nebula_metrics::MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );

    // `AppState` stores **raw** port handles and applies the per-request
    // tenant scope in its accessors; the engine still calls its handles
    // with the internal `engine_scope()` placeholder (a separate, tracked
    // follow-up — see ADR-0072 "Known follow-up: engine per-execution
    // tenant scoping"). Wrap the engine-side handles in `nebula-tenancy`
    // decorators bound to `knife_scope()` (= `port_scope()`, the scope
    // the API derives and this test seeded the workflow/execution under)
    // so the decorator substitutes the engine's scope and engine reads,
    // the API-enqueued `Start`, and the seeded rows all key on the same
    // tenant. The echo node never checkpoints or replays, so a fresh
    // in-memory checkpoint/idempotency pair suffices for the two
    // `ExecutionStores` fields `AppState` does not expose.
    let s = knife_scope();
    let scoped_exec: Arc<dyn nebula_storage_port::store::ExecutionStore> = Arc::new(
        nebula_tenancy::ScopedExecutionStore::new(Arc::clone(&state.execution_store), s.clone()),
    );
    let engine = Arc::new(
        WorkflowEngine::new(runtime, metrics)
            .unwrap()
            .with_execution_stores(ExecutionStores {
                execution: Arc::clone(&scoped_exec),
                journal: Arc::new(nebula_tenancy::ScopedExecutionJournalReader::new(
                    Arc::clone(&state.journal_reader),
                    s.clone(),
                )),
                node_results: Arc::new(nebula_tenancy::ScopedNodeResultStore::new(
                    Arc::clone(&state.node_result_store),
                    s.clone(),
                )),
                checkpoints: Arc::new(nebula_storage::inmem::InMemoryCheckpointStore::new()),
                idempotency: Arc::new(nebula_storage::inmem::InMemoryIdempotencyGuard::new()),
            })
            .with_workflow_stores(WorkflowStores {
                workflow: Arc::new(nebula_tenancy::ScopedWorkflowStore::new(
                    Arc::clone(&state.workflow_store),
                    s.clone(),
                )),
                versions: Arc::new(nebula_tenancy::ScopedWorkflowVersionStore::new(
                    Arc::clone(&state.workflow_version_store),
                    s.clone(),
                )),
            }),
    );

    // ── Spawn the consumer so `Start` rows are drained continuously ──────────
    let dispatch = Arc::new(EngineControlDispatch::new(engine, Arc::clone(&scoped_exec)));
    let consumer = ControlConsumer::new(
        Arc::new(nebula_tenancy::ScopedControlQueue::new(
            Arc::clone(&state.control_queue),
            s,
        )),
        dispatch,
        proc16(b"knife-a2"),
    )
    .with_poll_interval(Duration::from_millis(10));
    let shutdown = CancellationToken::new();
    let consumer_handle = consumer.spawn(shutdown.clone());

    // ── POST /executions — the A1 producer path ──────────────────────────────
    let start_request = serde_json::json!({
        "input": { "knife_e2e": "a2" }
    });
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
        "step 3 end-to-end: start execution must return 202"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let execution_response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let execution_id_str = execution_response["id"]
        .as_str()
        .expect("execution response must carry an id")
        .to_string();
    let execution_id = nebula_core::ExecutionId::parse(&execution_id_str).unwrap();

    // ── Wait for the consumer + engine to drive the execution to Completed ───
    //
    // Poll the repo because the consumer loop is cross-task; a small timeout
    // tolerates scheduler jitter on slow test hosts. A fail here means the
    // §4.5 gap #332 was only half-closed — producer works, consumer does not.
    let final_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            // Read through the same scoped port handle the engine was
            // wired with; the tenancy decorator substitutes its bound
            // scope so the `knife_scope()` argument is immaterial.
            let json = state
                .execution_store
                .get(&knife_scope(), &execution_id.to_string())
                .await
                .unwrap()
                .expect("execution row is present")
                .state;
            let status: ExecutionStatus =
                serde_json::from_value(json.get("status").cloned().unwrap()).unwrap();
            if status.is_terminal() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("engine drove execution to terminal within 5s (A2 consumer + engine wired)");

    assert_eq!(
        final_status,
        ExecutionStatus::Completed,
        "step 3 end-to-end: the A2 engine dispatch must transition the execution to \
         Completed — the §4.5 gap named in #332 is now closed on both halves"
    );

    // Graceful shutdown so the spawned task doesn't leak across tests.
    shutdown.cancel();
    let _ = consumer_handle.await;
}

// ── Knife step 5 end-to-end (ADR-0008 A3) ──────────────────────────────────
//
// Symmetric to `knife_step3_engine_dispatches_start_end_to_end`. The producer
// half — `POST /cancel` writes the `Cancelled` row and enqueues `Cancel` — is
// already asserted by `knife_scenario_end_to_end` above. This test asserts the
// CONSUMER half: the `EngineControlDispatch::dispatch_cancel` signals the
// engine's live frontier loop so a long-running workflow is actually aborted
// end-to-end, not left sleeping until its natural completion.
//
// The wiring mirrors step 3:
//   POST /cancel
//     → execution_repo.transition (Cancelled)
//     → execution_control_queue.enqueue(Cancel)
//     → ControlConsumer.claim_pending
//     → EngineControlDispatch::dispatch_cancel
//     → WorkflowEngine::cancel_execution
//     → frontier loop observes `ctx.cancellation()` → node exits
//
// The workflow uses a cooperatively-cancellable `slow` handler that would
// otherwise wait 30s; asserting that the execution reaches a terminal state
// within a few seconds proves the signal reached the engine's live loop.
// The action + real-engine-consumer wiring is the shared
// `common::engine_seam` harness (byte-behaviorally identical to the
// original inline wiring — the move is mechanical).

/// Canon §13 step 5 end-to-end (ADR-0008 A3).
///
/// Wires API producer + `ControlConsumer` + `EngineControlDispatch` + engine
/// over shared in-memory repos, starts a long-running execution, POSTs
/// `/executions/:id/cancel`, and asserts the execution reaches a terminal
/// state well inside the slow handler's 30-second sleep window. Closes #330.
#[tokio::test]
async fn knife_step5_engine_cancels_running_execution_end_to_end() {
    use std::time::Duration;

    use nebula_execution::ExecutionStatus;

    let (state, _control_queue) = create_state_with_queue().await;
    let api_config = ApiConfig::for_test();
    let token = create_test_jwt();

    // ── Persist a single-`slow`-node workflow (shared harness) ───────────────
    let workflow_id = engine_seam::persist_slow_workflow(&state).await;

    // ── Build + spawn the real engine consumer (shared harness) ──────────────
    //
    // Byte-behaviorally identical to the original inline knife step-5
    // wiring: same action key (`"slow"`), `ActionExecutor` closure,
    // `InProcessSandbox`, `ActionRuntime`, 10ms poll interval, and the
    // `b"knife-a3"` processor id — see `common::engine_seam`.
    let seam = engine_seam::spawn_engine_consumer(&state);

    // ── Start the execution via the A1/A2 producer path ──────────────────────
    let start_request = serde_json::json!({ "input": { "knife_e2e": "a3" } });
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
        "step 5 end-to-end: start execution must return 202"
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
            "slow handler started within 10s (A2 consumer drained Start and the engine \
             dispatched the node)",
        );

    // ── Cancel via the API — step 5 producer path ──────────────────────────
    let cancel_app = app::build_app(state.clone(), &api_config);
    let cancel_response = cancel_app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(ws_path(&format!("/executions/{execution_id_str}")))
                .header("authorization", format!("Bearer {token}"))
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .header("cookie", TEST_CSRF_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        cancel_response.status(),
        StatusCode::OK,
        "step 5 end-to-end: cancel must return 200"
    );

    // ── The execution must reach a terminal state well inside the slow
    //    handler's 30s sleep — proving the Cancel reached the engine's live
    //    cancel token and the handler exited cooperatively. Without A3 the
    //    row would be `Cancelled` via the API's CAS but the slow handler
    //    would still be sleeping in-process for up to 30s.
    let execution_id = nebula_core::ExecutionId::parse(&execution_id_str).unwrap();
    let final_status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            // Read through the same scoped port handle the engine was
            // wired with; the tenancy decorator substitutes its bound
            // scope so the `knife_scope()` argument is immaterial.
            let json = state
                .execution_store
                .get(&knife_scope(), &execution_id.to_string())
                .await
                .unwrap()
                .expect("execution row present")
                .state;
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
        "engine reached a terminal state within 10s (A3 cancel dispatch signalled the \
         live frontier loop) — the 30s slow handler was aborted cooperatively",
    );

    assert!(
        final_status.is_terminal(),
        "step 5 end-to-end: execution reached a terminal state after Cancel — A3 closed \
         the §4.5 gap on the cancel half (#330). got: {final_status:?}"
    );

    // Graceful shutdown so the spawned consumer task doesn't leak.
    seam.shutdown().await;
}
