//! Unit tests for `EngineControlDispatch` (ADR-0008 A2 / A3).
//!
//! These tests mirror the API → consumer → engine seam without running the
//! full `ControlConsumer` polling loop: they invoke `dispatch_start` /
//! `dispatch_resume` / `dispatch_restart` / `dispatch_cancel` /
//! `dispatch_terminate` directly against an engine wired to in-memory repos,
//! and assert both the happy-path transitions (Created → Completed,
//! Running → Cancelled) and the ADR-0008 §5 idempotency contract
//! (re-delivery does not re-run or double-signal).

use std::{
    collections::HashMap,
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
use nebula_core::{ActionKey, DeclaresDependencies, action_key, id::ExecutionId, node_key};
use nebula_engine::{ControlDispatch, ControlDispatchError, EngineControlDispatch, WorkflowEngine};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_runtime::{
    ActionExecutor, ActionRuntime, DataPassingPolicy, InProcessSandbox, registry::ActionRegistry,
};
use nebula_storage::{ExecutionRepo, InMemoryExecutionRepo, InMemoryWorkflowRepo, WorkflowRepo};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};
use tokio::sync::Notify;

// ── Test handler ──────────────────────────────────────────────────────────

/// Echo handler that counts invocations so idempotency tests can assert no
/// second dispatch happened.
#[derive(Clone)]
struct CountingEchoHandler {
    meta: ActionMetadata,
    count: Arc<AtomicU32>,
}

impl DeclaresDependencies for CountingEchoHandler {}
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
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

/// Cooperatively-cancellable handler. Notifies when it enters the sleep so
/// tests know the frontier loop is live before delivering a `Cancel`.
struct SlowCancellableHandler {
    meta: ActionMetadata,
    started: Arc<Notify>,
    count: Arc<AtomicU32>,
}

impl DeclaresDependencies for SlowCancellableHandler {}
impl Action for SlowCancellableHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for SlowCancellableHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(30)) => Ok(ActionResult::success(input)),
            () = ctx.cancellation().cancelled() => Err(ActionError::Cancelled),
        }
    }
}

fn meta(key: ActionKey) -> ActionMetadata {
    let name = key.to_string();
    ActionMetadata::new(key, name, "control_dispatch test handler")
}

// ── Harness ───────────────────────────────────────────────────────────────

struct Harness {
    dispatch: EngineControlDispatch,
    engine: Arc<WorkflowEngine>,
    execution_repo: Arc<InMemoryExecutionRepo>,
    workflow_repo: Arc<InMemoryWorkflowRepo>,
    action_count: Arc<AtomicU32>,
    slow_count: Arc<AtomicU32>,
    slow_started: Arc<Notify>,
}

impl Harness {
    async fn new() -> Self {
        let action_count = Arc::new(AtomicU32::new(0));
        let slow_count = Arc::new(AtomicU32::new(0));
        let slow_started = Arc::new(Notify::new());
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless(CountingEchoHandler {
            meta: meta(action_key!("echo")),
            count: Arc::clone(&action_count),
        });
        registry.register_stateless(SlowCancellableHandler {
            meta: meta(action_key!("slow")),
            started: Arc::clone(&slow_started),
            count: Arc::clone(&slow_count),
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

        let dispatch = EngineControlDispatch::new(Arc::clone(&engine), execution_repo_dyn);

        Self {
            dispatch,
            engine,
            execution_repo,
            workflow_repo,
            action_count,
            slow_count,
            slow_started,
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

    /// Persist a single-node `slow` workflow — the node sleeps until cancelled.
    async fn persist_slow_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = chrono::Utc::now();
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "a3-cancel-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![NodeDefinition::new(node_key!("step"), "Step", "slow").unwrap()],
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

    async fn status(&self, id: ExecutionId) -> ExecutionStatus {
        let (_, json) = self
            .execution_repo
            .get_state(id)
            .await
            .unwrap()
            .expect("execution exists");
        serde_json::from_value(json.get("status").cloned().unwrap()).unwrap()
    }

    /// Poll the execution row until `status.is_terminal()`, bounded by `deadline`.
    async fn wait_terminal(&self, id: ExecutionId, deadline: Duration) -> ExecutionStatus {
        tokio::time::timeout(deadline, async {
            loop {
                let status = self.status(id).await;
                if status.is_terminal() {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("execution reached terminal within deadline")
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

// ── A3 tests — dispatch_cancel / dispatch_terminate ───────────────────────

/// Happy path: a `Cancel` delivered while the frontier loop is inside a
/// live node aborts the execution cooperatively. The spawned run returns,
/// the node surfaces its typed `ActionError::Cancelled`, and the execution
/// row lands on a terminal state. This is the §13-step-5 invariant that A3
/// closes — the durable `Cancel` signal actually reaches the engine.
#[tokio::test]
async fn dispatch_cancel_aborts_running_execution() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_slow_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("cancel-me"))
        .await;

    // Spawn the engine on a separate task so the test thread can drive the
    // `Cancel` dispatch while the frontier loop is live.
    let engine = Arc::clone(&harness.engine);
    let run_handle = tokio::spawn(async move { engine.resume_execution(execution_id).await });

    // Wait for the slow handler to confirm it entered the `select!` — the
    // frontier loop is now observing the cancel token.
    tokio::time::timeout(Duration::from_secs(5), harness.slow_started.notified())
        .await
        .expect("slow handler started within 5s");

    // Deliver the Cancel. ADR-0008 A3: signal reaches the live token, the
    // handler exits via `ActionError::Cancelled`, frontier loop tears down.
    harness
        .dispatch
        .dispatch_cancel(execution_id)
        .await
        .expect("dispatch_cancel succeeds");

    // The spawned run must complete quickly once the cancel fires. The
    // InMemoryExecutionRepo does not enforce a separate Cancelled status on
    // CAS without a prior external transition, so we assert the broader
    // invariant: the run finishes in a terminal state promptly (no 30s sleep).
    let result = tokio::time::timeout(Duration::from_secs(5), run_handle)
        .await
        .expect("spawned run returns within 5s of cancel")
        .expect("join ok");
    let _run_result = result.expect("engine run returns Ok (node cancelled counts as finished)");

    // The A3 invariant: the frontier loop exited promptly on cancel (not
    // the full 30s sleep the slow handler would otherwise wait for). The
    // exact terminal label (`Cancelled` on the abort-select path vs
    // `Failed` on the node-error path) depends on a scheduler race between
    // the cancel-token select arm and the handler's own error return; both
    // are terminal per `ExecutionStatus::is_terminal`, so we assert the
    // broad invariant here.
    let terminal = harness
        .wait_terminal(execution_id, Duration::from_secs(5))
        .await;
    assert!(
        terminal.is_terminal(),
        "execution reached a terminal state after Cancel, got: {terminal:?}"
    );
    assert_eq!(
        harness.slow_count.load(Ordering::SeqCst),
        1,
        "slow handler entered exactly once — cancel did not cause a re-dispatch"
    );
}

/// ADR-0008 §5 idempotency on terminal: a `Cancel` re-delivered for an
/// already-terminal execution must be `Ok(())` without disturbing state or
/// triggering a second dispatch of the workflow.
///
/// The A3 contract (ADR-0016) makes this property hold through **two
/// layers**, not a status short-circuit:
///
///   1. `dispatch_cancel` signals the engine on every non-orphan delivery, including terminal.
///      `engine.cancel_execution(id)` looks up the registry — by the time the run is terminal, its
///      `RunningRegistration` guard has already removed the entry, so the lookup returns `false`
///      and the call is a no-op.
///   2. The underlying `CancellationToken::cancel` is idempotent per token, so even a racy delivery
///      where the registry entry is still live cannot re-run the workflow (the frontier loop
///      already observed the original cancel or completed naturally).
///
/// This test asserts the observable effect: no second dispatch of the
/// echo handler, terminal row untouched.
#[tokio::test]
async fn dispatch_cancel_is_idempotent_on_terminal() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("already-done"))
        .await;

    // Drive the run to Completed first.
    harness.dispatch.dispatch_start(execution_id).await.unwrap();
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    assert_eq!(harness.action_count.load(Ordering::SeqCst), 1);

    // Re-deliver Cancel. The A3 body signals `engine.cancel_execution`
    // unconditionally — but the registry entry was removed when the run
    // finished, so the lookup is a no-op and the workflow is not
    // re-dispatched. The return value is `Ok(())`.
    harness
        .dispatch
        .dispatch_cancel(execution_id)
        .await
        .expect("re-delivered Cancel on terminal execution is Ok");

    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed,
        "terminal state must not be disturbed by a re-delivered Cancel"
    );
    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        1,
        "no second dispatch happened — registry entry was already cleared \
         when the run finished, so the engine signal was a no-op"
    );
    // Confirm the registry is indeed empty for this id — the observable
    // truth behind point (1) above.
    assert!(
        !harness.engine.cancel_execution(execution_id),
        "registry entry was removed on run completion"
    );
}

/// Cross-runner case: a Cancel arrives at an engine instance that never
/// held this execution's frontier loop. `cancel_execution` returns `false`
/// (nothing to signal locally), but `dispatch_cancel` must still return
/// `Ok(())` — the durable CAS transition to `Cancelled` happens on the API
/// handler side, and the holding runner will observe it on its next state
/// transition. Returning an error here would mark the control-queue row
/// `Failed` on a path that is actually healthy.
#[tokio::test]
async fn dispatch_cancel_ok_when_execution_not_held_locally() {
    let harness = Harness::new().await;
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("elsewhere"))
        .await;

    // The execution row sits at `Created`; this engine never started it.
    // The registry does not know about this id.
    assert_eq!(harness.status(execution_id).await, ExecutionStatus::Created);
    assert!(
        !harness.engine.cancel_execution(execution_id),
        "precondition: engine's registry does not hold this id"
    );

    harness
        .dispatch
        .dispatch_cancel(execution_id)
        .await
        .expect("cross-runner Cancel is Ok — no local token is not an error");

    assert_eq!(
        harness.action_count.load(Ordering::SeqCst),
        0,
        "no side effect: echo handler was never dispatched"
    );
}

/// Orphan `Cancel` — producer bug, row enqueued without the execution row.
/// Must surface a typed reject so the diagnosis lands on the queue row's
/// `error_message`, matching the A2 symmetric path for `Start`.
#[tokio::test]
async fn dispatch_cancel_rejects_nonexistent_execution() {
    let harness = Harness::new().await;
    let orphan = ExecutionId::new();

    let err = harness
        .dispatch
        .dispatch_cancel(orphan)
        .await
        .expect_err("missing execution rejects");
    match err {
        ControlDispatchError::Rejected(msg) => {
            assert!(
                msg.contains("not found")
                    && msg.contains(&orphan.to_string())
                    && msg.contains("cancel"),
                "reject message must name the orphan + command, got: {msg}"
            );
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
}

/// `Terminate` is a synonym for `Cancel` until a forced-shutdown path is
/// wired (see ADR-0016 and the module doc). Same idempotency / orphan /
/// cross-runner contracts apply — this smoke test just asserts the
/// delegation is wired, not a separate code path.
#[tokio::test]
async fn dispatch_terminate_behaves_like_cancel() {
    let harness = Harness::new().await;
    let orphan = ExecutionId::new();

    // Orphan -> Rejected (same code path as dispatch_cancel).
    let err = harness
        .dispatch
        .dispatch_terminate(orphan)
        .await
        .expect_err("orphan terminate rejects");
    assert!(matches!(err, ControlDispatchError::Rejected(_)));

    // Terminal -> Ok.
    let workflow_id = harness.persist_echo_workflow().await;
    let execution_id = harness
        .persist_created_execution(workflow_id, serde_json::json!("t"))
        .await;
    harness.dispatch.dispatch_start(execution_id).await.unwrap();
    assert_eq!(
        harness.status(execution_id).await,
        ExecutionStatus::Completed
    );
    harness
        .dispatch
        .dispatch_terminate(execution_id)
        .await
        .expect("terminal terminate is Ok");
}
