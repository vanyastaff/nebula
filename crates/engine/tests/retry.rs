//! Engine-level retry integration tests (ADR-0042 §M2.1 T6).
//!
//! Covers the canonical retry path plus the four ADR-named edge
//! cases (cancel/terminate/budget/idempotency) and the three
//! resolution scenarios for the effective retry policy
//! (per-node / workflow-default / no policy).
//!
//! Each test uses small (1-5ms) backoffs so the suite stays under
//! the integration-test budget while still exercising the real
//! `tokio::sleep` path in the frontier loop.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{DeclaresDependencies, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionEvent,
    InProcessSandbox, WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_storage::ExecutionRepo;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{NodeDefinition, RetryConfig, Version, WorkflowConfig, WorkflowDefinition};

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Fails on attempts 1..fail_count, then succeeds with the input on
/// attempt fail_count+1. The shared counter records each invocation
/// so tests can assert exact attempt counts.
struct FlakyHandler {
    meta: ActionMetadata,
    fail_count: u32,
    invocations: Arc<AtomicU32>,
}

impl DeclaresDependencies for FlakyHandler {}
impl Action for FlakyHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for FlakyHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let n = self.invocations.fetch_add(1, Ordering::SeqCst) + 1;
        if n <= self.fail_count {
            Err(ActionError::retryable(format!("flaky: attempt {n} failed")))
        } else {
            Ok(ActionResult::success(input))
        }
    }
}

/// Always fails — used to verify retry exhaustion.
struct AlwaysFailingHandler {
    meta: ActionMetadata,
    invocations: Arc<AtomicU32>,
}

impl DeclaresDependencies for AlwaysFailingHandler {}
impl Action for AlwaysFailingHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for AlwaysFailingHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        _input: Self::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Err(ActionError::retryable("always fails"))
    }
}

/// Returns `ActionResult::Terminate` to test cooperative shutdown
/// while a sibling is parked in WaitingRetry.
struct TerminateHandler {
    meta: ActionMetadata,
}

impl DeclaresDependencies for TerminateHandler {}
impl Action for TerminateHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for TerminateHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        _input: Self::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::terminate_success(Some(
            "test-stop".to_owned(),
        )))
    }
}

// ---------------------------------------------------------------------------
// Engine assembly + workflow helpers
// ---------------------------------------------------------------------------

fn make_engine(registry: Arc<ActionRegistry>) -> WorkflowEngine {
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        metrics.clone(),
    ));
    WorkflowEngine::new(runtime, metrics)
}

fn make_workflow(
    nodes: Vec<NodeDefinition>,
    connections: Vec<nebula_workflow::Connection>,
    config: WorkflowConfig,
) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "retry-test".to_owned(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
        connections,
        variables: Default::default(),
        config,
        trigger: None,
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: 1,
    }
}

// ---------------------------------------------------------------------------
// Tests (9 — ADR-0042 §M2.1 T6 acceptance)
// ---------------------------------------------------------------------------

/// 1) Basic retry path — handler fails once, succeeds on attempt 2.
#[tokio::test]
async fn retry_succeeds_on_attempt_2() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(FlakyHandler {
        meta: ActionMetadata::new(action_key!("flaky"), "Flaky", "fails once"),
        fail_count: 1,
        invocations: Arc::clone(&invocations),
    });

    let engine = make_engine(registry);
    let n = node_key!("flake");
    let mut node = NodeDefinition::new(n.clone(), "flake_node", "flaky").unwrap();
    node.retry_policy = Some(RetryConfig::fixed(3, 1));

    let wf = make_workflow(vec![node], vec![], WorkflowConfig::default());
    let result = engine
        .execute_workflow(
            &wf,
            serde_json::json!("payload"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success(), "retry should succeed on attempt 2");
    assert_eq!(invocations.load(Ordering::SeqCst), 2);
    assert_eq!(result.status, ExecutionStatus::Completed);
}

/// 2) Retry exhausts `max_attempts` — handler always fails, workflow
/// fails after `max_attempts` invocations.
#[tokio::test]
async fn retry_exhausts_max_attempts() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(AlwaysFailingHandler {
        meta: ActionMetadata::new(action_key!("doomed"), "Doomed", "always fails"),
        invocations: Arc::clone(&invocations),
    });

    let engine = make_engine(registry);
    let n = node_key!("d");
    let mut node = NodeDefinition::new(n, "doom_node", "doomed").unwrap();
    node.retry_policy = Some(RetryConfig::fixed(3, 1));

    let wf = make_workflow(vec![node], vec![], WorkflowConfig::default());
    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(!result.is_success());
    assert_eq!(result.status, ExecutionStatus::Failed);
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        3,
        "max_attempts=3 must run the handler exactly 3 times"
    );
}

/// 3) Cancel-during-retry-wait — `WorkflowEngine::cancel_execution`
/// flips the parked WaitingRetry node to Cancelled when the cancel
/// token fires. Uses a long backoff so the test reliably catches
/// the node mid-wait.
#[tokio::test]
async fn cancel_during_retry_wait() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(AlwaysFailingHandler {
        meta: ActionMetadata::new(action_key!("flaky_long"), "FlakyLong", "fails forever"),
        invocations: Arc::clone(&invocations),
    });

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();

    let engine = Arc::new(make_engine(registry).with_event_bus(event_bus));

    let n = node_key!("c");
    let mut node = NodeDefinition::new(n, "c_node", "flaky_long").unwrap();
    // 60s backoff — will not fire during test.
    node.retry_policy = Some(RetryConfig::fixed(5, 60_000));

    let wf = make_workflow(vec![node], vec![], WorkflowConfig::default());
    let engine_h = Arc::clone(&engine);
    let task = tokio::spawn(async move {
        engine_h
            .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
            .await
    });

    // Wait for the engine to schedule a retry (proves the node is
    // parked in WaitingRetry), then cancel. Bound the wait so a
    // missing `NodeRetryScheduled` emission fails the test instead
    // of hanging CI indefinitely.
    let execution_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeRetryScheduled {
                    execution_id: id, ..
                }) => break id,
                Some(_) => continue,
                None => panic!("event bus closed before NodeRetryScheduled"),
            }
        }
    })
    .await
    .expect("timed out waiting for NodeRetryScheduled");
    let cancelled = engine.cancel_execution(execution_id);
    assert!(cancelled, "cancel_execution must find the live frontier");

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("workflow must wind down within 5s")
        .unwrap()
        .unwrap();

    assert!(!result.is_success());
    assert!(
        matches!(result.status, ExecutionStatus::Cancelled),
        "expected Cancelled, got {:?}",
        result.status
    );
}

/// 4) Terminate-during-retry-wait — a sibling returning
/// `ActionResult::Terminate` shuts down the workflow even when the
/// other branch is parked in WaitingRetry. Tests the same
/// cancel-token tear-down path as test #3 but driven by a node, not
/// an external cancel.
///
/// Subscribes to the event bus so we can assert the `NodeRetryScheduled`
/// event actually fired before completion — a green test without
/// that check would only prove the workflow finished, not that the
/// drain path was exercised.
#[tokio::test]
async fn terminate_during_retry_wait() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(AlwaysFailingHandler {
        meta: ActionMetadata::new(action_key!("flaky_t"), "FlakyT", "fails forever"),
        invocations: Arc::clone(&invocations),
    });
    registry.register_stateless(TerminateHandler {
        meta: ActionMetadata::new(action_key!("term"), "Term", "terminates"),
    });

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine = make_engine(registry).with_event_bus(event_bus);
    let parked = node_key!("parked");
    let stopper = node_key!("stopper");
    let mut parked_node = NodeDefinition::new(parked.clone(), "parked_node", "flaky_t").unwrap();
    // 60s backoff parks the node so terminate has to drain it.
    parked_node.retry_policy = Some(RetryConfig::fixed(5, 60_000));
    let stopper_node = NodeDefinition::new(stopper, "stopper_node", "term").unwrap();

    let wf = make_workflow(
        vec![parked_node, stopper_node],
        vec![],
        WorkflowConfig::default(),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        engine.execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default()),
    )
    .await
    .expect("workflow must wind down within 5s")
    .unwrap();

    // The terminate handler emits ExplicitStop → Completed status.
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert!(
        invocations.load(Ordering::SeqCst) <= 2,
        "terminate must drain the WaitingRetry queue without re-dispatching"
    );

    // Drain the event bus and assert that the parked node DID get a
    // `NodeRetryScheduled` event before terminate fired. Without this
    // assertion the test would pass even if terminate beat the retry
    // schedule entirely, which means the drain path was never exercised.
    let mut saw_retry_scheduled_for_parked = false;
    while let Some(event) = events_rx.try_recv() {
        if let ExecutionEvent::NodeRetryScheduled { node_key, .. } = event
            && node_key == parked
        {
            saw_retry_scheduled_for_parked = true;
            break;
        }
    }
    assert!(
        saw_retry_scheduled_for_parked,
        "parked node must have been parked in WaitingRetry before terminate \
         drained it (otherwise the test passes without exercising the drain)"
    );
}

/// 5) `ExecutionBudget.max_total_retries` caps the global retry
/// counter regardless of per-node policy. With cap=1 and a 3-attempt
/// policy, the engine schedules exactly one retry across the run.
#[tokio::test]
async fn execution_budget_max_total_retries_caps_globally() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(AlwaysFailingHandler {
        meta: ActionMetadata::new(action_key!("doomed_g"), "DoomedG", "always fails"),
        invocations: Arc::clone(&invocations),
    });

    let engine = make_engine(registry);
    let n = node_key!("g");
    let mut node = NodeDefinition::new(n, "g_node", "doomed_g").unwrap();
    node.retry_policy = Some(RetryConfig::fixed(5, 1));

    let wf = make_workflow(vec![node], vec![], WorkflowConfig::default());
    let budget = ExecutionBudget::default().with_max_total_retries(1);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), budget)
        .await
        .unwrap();

    assert!(!result.is_success());
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        2,
        "cap=1 retry → exactly 2 invocations (initial + 1 retry)"
    );
}

/// 6) Idempotency key differentiates attempts — a successful retry
/// gets a different key than the failed first attempt.
#[tokio::test]
async fn idempotency_key_differentiates_attempts() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(FlakyHandler {
        meta: ActionMetadata::new(action_key!("flaky_idem"), "FlakyIdem", "fails once"),
        fail_count: 1,
        invocations: Arc::clone(&invocations),
    });

    let exec_repo = Arc::new(nebula_storage::InMemoryExecutionRepo::new());
    let engine = make_engine(registry).with_execution_repo(exec_repo.clone());

    let n = node_key!("idem");
    let mut node = NodeDefinition::new(n.clone(), "idem_node", "flaky_idem").unwrap();
    node.retry_policy = Some(RetryConfig::fixed(3, 1));

    let wf = make_workflow(vec![node], vec![], WorkflowConfig::default());
    let result = engine
        .execute_workflow(&wf, serde_json::json!({"v": 1}), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(invocations.load(Ordering::SeqCst), 2);

    // Pull the persisted state and inspect the attempts.
    let (_, state_json) = exec_repo
        .get_state(result.execution_id)
        .await
        .unwrap()
        .expect("state must be persisted");
    let state_str = serde_json::to_string(&state_json).unwrap();
    let exec_state: nebula_execution::state::ExecutionState =
        serde_json::from_str(&state_str).unwrap();
    let ns = exec_state.node_state(n).unwrap();
    assert_eq!(ns.attempts.len(), 2, "two attempts pushed");
    assert_ne!(
        ns.attempts[0].idempotency_key, ns.attempts[1].idempotency_key,
        "retry must mint a fresh idempotency key per attempt (canon §11.3)"
    );
    assert_eq!(ns.attempts[0].attempt_number, 1);
    assert_eq!(ns.attempts[1].attempt_number, 2);
    assert!(ns.attempts[0].is_failure());
    assert!(ns.attempts[1].is_success());
}

/// 7) Per-node `retry_policy` overrides the workflow default.
/// Workflow default is "1 attempt, no retry"; per-node policy is
/// "3 attempts". The node MUST follow the per-node value.
#[tokio::test]
async fn per_node_retry_policy_overrides_workflow_default() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(FlakyHandler {
        meta: ActionMetadata::new(
            action_key!("flaky_o"),
            "FlakyO",
            "fails twice then succeeds",
        ),
        fail_count: 2,
        invocations: Arc::clone(&invocations),
    });

    let engine = make_engine(registry);
    let n = node_key!("o");
    let mut node = NodeDefinition::new(n, "o_node", "flaky_o").unwrap();
    node.retry_policy = Some(RetryConfig::fixed(3, 1)); // 3 attempts

    let config = WorkflowConfig {
        retry_policy: Some(RetryConfig::fixed(1, 1)), // 1 attempt only
        ..WorkflowConfig::default()
    };

    let wf = make_workflow(vec![node], vec![], config);
    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(
        result.is_success(),
        "per-node 3-attempt policy must override workflow's 1-attempt default"
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 3);
}

/// 8) Workflow-default `retry_policy` applies when the node does
/// not declare its own.
#[tokio::test]
async fn workflow_default_applies_when_node_has_none() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(FlakyHandler {
        meta: ActionMetadata::new(action_key!("flaky_d"), "FlakyD", "fails once"),
        fail_count: 1,
        invocations: Arc::clone(&invocations),
    });

    let engine = make_engine(registry);
    let n = node_key!("d");
    let node = NodeDefinition::new(n, "d_node", "flaky_d").unwrap(); // no policy

    let config = WorkflowConfig {
        retry_policy: Some(RetryConfig::fixed(3, 1)),
        ..WorkflowConfig::default()
    };

    let wf = make_workflow(vec![node], vec![], config);
    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(
        result.is_success(),
        "workflow default must drive retry when node has no per-node policy"
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 2);
}

/// 9) No policy anywhere — first failure finalizes the workflow
/// (one-shot semantics). Regression-guards the "no policy = no
/// retry" branch in `compute_retry_decision`.
#[tokio::test]
async fn no_retry_policy_means_one_shot_failure() {
    let invocations = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(AlwaysFailingHandler {
        meta: ActionMetadata::new(action_key!("oneshot"), "OneShot", "fails"),
        invocations: Arc::clone(&invocations),
    });

    let engine = make_engine(registry);
    let n = node_key!("os");
    let node = NodeDefinition::new(n, "os_node", "oneshot").unwrap();

    let wf = make_workflow(vec![node], vec![], WorkflowConfig::default());
    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .unwrap();

    assert!(!result.is_success());
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "without a retry policy the engine must finalize after the first failure"
    );
}
