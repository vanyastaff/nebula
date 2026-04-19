//! Unit tests for `EngineControlDispatch` (ADR-0008 A2).
//!
//! These tests mirror the API → consumer → engine seam without running the
//! full `ControlConsumer` polling loop: they invoke `dispatch_start` /
//! `dispatch_resume` / `dispatch_restart` directly against an engine wired
//! to in-memory repos, and assert both the happy-path transition (Created →
//! Completed) and the ADR-0008 §5 idempotency contract (re-delivery does
//! not re-run the workflow).

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use nebula_action::{
    ActionError, action::Action, context::Context, dependency::ActionDependencies,
    metadata::ActionMetadata, result::ActionResult, stateless::StatelessAction,
};
use nebula_core::{ActionKey, action_key, id::ExecutionId, node_key};
use nebula_engine::{ControlDispatch, ControlDispatchError, EngineControlDispatch, WorkflowEngine};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_runtime::{
    ActionExecutor, ActionRuntime, DataPassingPolicy, InProcessSandbox, registry::ActionRegistry,
};
use nebula_storage::{ExecutionRepo, InMemoryExecutionRepo, InMemoryWorkflowRepo, WorkflowRepo};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};

// ── Test handler ──────────────────────────────────────────────────────────

/// Echo handler that counts invocations so idempotency tests can assert no
/// second dispatch happened.
#[derive(Clone)]
struct CountingEchoHandler {
    meta: ActionMetadata,
    count: Arc<AtomicU32>,
}

impl ActionDependencies for CountingEchoHandler {}
impl Action for CountingEchoHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for CountingEchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

fn meta(key: ActionKey) -> ActionMetadata {
    let name = key.to_string();
    ActionMetadata::new(key, name, "control_dispatch test handler")
}

// ── Harness ───────────────────────────────────────────────────────────────

struct Harness {
    dispatch: EngineControlDispatch,
    execution_repo: Arc<InMemoryExecutionRepo>,
    workflow_repo: Arc<InMemoryWorkflowRepo>,
    action_count: Arc<AtomicU32>,
}

impl Harness {
    async fn new() -> Self {
        let action_count = Arc::new(AtomicU32::new(0));
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(CountingEchoHandler {
            meta: meta(action_key!("echo")),
            count: Arc::clone(&action_count),
        });

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(ActionRuntime::new(
            registry,
            sandbox,
            DataPassingPolicy::default(),
            metrics.clone(),
        ));

        let execution_repo = Arc::new(InMemoryExecutionRepo::new());
        let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());

        let execution_repo_dyn: Arc<dyn ExecutionRepo> = Arc::clone(&execution_repo) as _;
        let workflow_repo_dyn: Arc<dyn WorkflowRepo> = Arc::clone(&workflow_repo) as _;

        let engine = WorkflowEngine::new(runtime, metrics)
            .with_execution_repo(Arc::clone(&execution_repo_dyn))
            .with_workflow_repo(workflow_repo_dyn);
        let engine = Arc::new(engine);

        let dispatch = EngineControlDispatch::new(engine, execution_repo_dyn);

        Self {
            dispatch,
            execution_repo,
            workflow_repo,
            action_count,
        }
    }

    /// Persist a single-node echo workflow and return its id.
    async fn persist_echo_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = chrono::Utc::now();
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "a2-dispatch-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![NodeDefinition::new(node_key!("step"), "Step", "echo").unwrap()],
            connections: Vec::<Connection>::new(),
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        };
        self.workflow_repo
            .save(workflow_id, 0, serde_json::to_value(&wf).unwrap())
            .await
            .unwrap();
        workflow_id
    }

    /// Persist a pristine `Created` execution row, mirroring how the API
    /// `start_execution` handler writes the row before enqueueing `Start`.
    async fn persist_created_execution(
        &self,
        workflow_id: nebula_core::WorkflowId,
        input: serde_json::Value,
    ) -> ExecutionId {
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
        exec_state.set_workflow_input(input);
        let state_json = serde_json::to_value(&exec_state).unwrap();
        self.execution_repo
            .create(execution_id, workflow_id, state_json)
            .await
            .unwrap();
        execution_id
    }

    async fn status(&self, id: ExecutionId) -> ExecutionStatus {
        let (_, json) = self
            .execution_repo
            .get_state(id)
            .await
            .unwrap()
            .expect("execution exists");
        serde_json::from_value(json.get("status").cloned().unwrap()).unwrap()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Happy path: dispatch_start on a fresh `Created` execution row drives the
/// engine to completion. This is the A2 §13-step-3 invariant — a POST to
/// `/executions` ends with the workflow actually running, not stranded at
/// `Created`.
#[tokio::test]
async fn dispatch_start_drives_created_execution_to_completion() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("hello"))
        .await;

    assert_eq!(harness.status(execution_id).await, ExecutionStatus::Created);

    harness
        .dispatch
        .dispatch_start(execution_id)
        .await
        .expect("dispatch_start succeeds");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed,
        "engine transitioned the execution all the way to Completed"
    );
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "echo action was dispatched exactly once"
    );
}

/// ADR-0008 §5 idempotency: a re-delivered `Start` for an execution the
/// engine already completed is a no-op — no second run of the workflow.
/// This is the load-bearing guard against at-least-once redelivery
/// double-running the work.
#[tokio::test]
async fn dispatch_start_is_idempotent_on_redelivery() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!(42))
        .await;

    harness.dispatch.dispatch_start(execution_id).await.unwrap();
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );

    // Re-deliver Start. `EngineControlDispatch` must read the terminal
    // status and short-circuit; the engine must not be entered a second time.
    harness
        .dispatch
        .dispatch_start(execution_id)
        .await
        .expect("redelivered Start is idempotent");

    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "re-delivered Start must NOT run the workflow again (ADR-0008 §5)"
    );
}

/// `Resume` converges on the same engine entry as `Start` today. Delivered
/// against a pristine `Created` row (e.g. an operator-issued resume of a
/// run that was never started), it must drive the execution to completion
/// just like `Start`.
#[tokio::test]
async fn dispatch_resume_drives_created_execution_to_completion() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("resume"))
        .await;

    harness
        .dispatch
        .dispatch_resume(execution_id)
        .await
        .expect("dispatch_resume succeeds");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);
}

/// Redelivered `Resume` on an already-completed execution is a no-op per
/// ADR-0008 §5 — symmetric with the `Start` idempotency guard.
#[tokio::test]
async fn dispatch_resume_is_idempotent_on_completed_execution() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("x"))
        .await;

    harness
        .dispatch
        .dispatch_resume(execution_id)
        .await
        .unwrap();
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);

    harness
        .dispatch
        .dispatch_resume(execution_id)
        .await
        .expect("second resume is idempotent");

    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "re-delivered Resume must NOT re-run the workflow"
    );
}

/// `Restart` on a pristine `Created` execution behaves like `Start` — it
/// drives the engine to completion. This is the non-terminal arm of the
/// A2 restart path.
#[tokio::test]
async fn dispatch_restart_drives_created_execution_to_completion() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("restart"))
        .await;

    harness
        .dispatch
        .dispatch_restart(execution_id)
        .await
        .expect("dispatch_restart on a Created execution drives it to completion");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);
}

/// `Restart` on an already-terminal execution surfaces a typed reject
/// rather than silently succeeding — full rewind-from-input requires
/// durable output purge and a restart counter, neither of which exists in
/// A2. This keeps the capability gap honest on the
/// `execution_control_queue.error_message` column.
#[tokio::test]
async fn dispatch_restart_rejects_terminal_execution() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("done"))
        .await;

    // Drive it to Completed first.
    harness.dispatch.dispatch_start(execution_id).await.unwrap();
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);

    // A real restart-from-input would reset everything and re-run; A2 does not
    // support that yet — the dispatch must reject so operators can see the gap.
    let err = harness
        .dispatch
        .dispatch_restart(execution_id)
        .await
        .expect_err("restart of terminal execution rejects for A2");
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("durable output purge") || msg.contains("ADR-0008 follow-up"),
                "reject message must name the A2 gap, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "rejected restart must NOT re-run the workflow"
    );
}

/// `Start` for an execution id that was never persisted (producer bug —
/// queue row written without the execution row) surfaces a typed reject
/// so operators see the diagnosis on the row instead of the consumer
/// quietly acking a broken command.
#[tokio::test]
async fn dispatch_start_rejects_nonexistent_execution() {
    let harness = Harness::new().await;
    let orphan = ExecutionId::new();

    let err = harness
        .dispatch
        .dispatch_start(orphan)
        .await
        .expect_err("missing execution rejects");
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("not found") && msg.contains(&orphan.to_string()),
                "reject message must identify the orphan, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 0);
}
