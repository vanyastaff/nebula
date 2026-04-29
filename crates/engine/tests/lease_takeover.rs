//! Engine integration tests for execution-lease heartbeat enforcement
//! across simulated runner restarts (ROADMAP §M2.2 — Layer 1).
//!
//! Verifies that when runner A holds an execution's lease via the engine
//! heartbeat task and "crashes" (we simulate a process death by aborting
//! its `execute_workflow` future), runner B can resume the same execution
//! after the lease TTL expires, and the resume path respects per-node
//! terminal status (no double-execution of completed work).

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
use nebula_core::{ActionKey, DeclaresDependencies, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionEvent,
    InProcessSandbox, WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_storage::{ExecutionRepo, InMemoryExecutionRepo, InMemoryWorkflowRepo, WorkflowRepo};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Counts invocations and echoes input. Used for nodes that should NOT
/// re-run after lease handoff (terminal-completed before the simulated
/// crash).
struct CountingEchoHandler {
    meta: ActionMetadata,
    invocations: Arc<AtomicU32>,
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
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

/// Increments invocations, signals start, then parks on the cancel
/// token. Used to hold runner A's lease while we simulate a crash —
/// the action stays in `Running` until either runner A is aborted (and
/// the dropped future tears down the parked await) or the engine
/// cancels via cooperative-cancel (e.g. a Cancel command).
struct ParkHandler {
    meta: ActionMetadata,
    started: Arc<tokio::sync::Notify>,
    invocations: Arc<AtomicU32>,
}

impl DeclaresDependencies for ParkHandler {}
impl Action for ParkHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for ParkHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        _input: Self::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        ctx.cancellation().cancelled().await;
        Err(ActionError::Cancelled)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn meta(key: ActionKey) -> ActionMetadata {
    let name = key.to_string();
    ActionMetadata::new(key, name, "lease-takeover test handler")
}

fn make_workflow(nodes: Vec<NodeDefinition>, connections: Vec<Connection>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "lease-takeover-test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
        connections,
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger: None,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: 1,
    }
}

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

// ---------------------------------------------------------------------------
// T3 — heartbeat-loss → takeover
// ---------------------------------------------------------------------------

/// ROADMAP §M2.2 / T3.
///
/// Two engines share an `InMemoryExecutionRepo` and an
/// `InMemoryWorkflowRepo`. The workflow has a fast `echo` node X
/// followed by a parking node Y. Runner A starts the workflow; X
/// completes, Y enters the parking handler and signals `started_a`.
/// We simulate a runner crash by aborting A's `execute_workflow`
/// future — the `LeaseGuard` drops, killing A's heartbeat task. After
/// advancing tokio's paused clock past the lease TTL, runner B's
/// `resume_execution` is called.
///
/// Asserts (M2.2 DoD — "no double-execution of completed work"):
/// - X (echo) is **not** re-dispatched on B (canon §11.3 idempotency)
/// - Y (park) **is** re-dispatched on B because A never finished it (legitimate retry after lease
///   handoff)
/// - B drives the workflow to `Succeeded`
#[tokio::test(start_paused = true)]
async fn engine_b_takes_over_after_engine_a_runner_dies() {
    let exec_repo = Arc::new(InMemoryExecutionRepo::new());
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());

    let echo_invocations = Arc::new(AtomicU32::new(0));
    let park_invocations = Arc::new(AtomicU32::new(0));
    let started_a = Arc::new(tokio::sync::Notify::new());

    // Note on timings under paused time: `InMemoryExecutionRepo` clamps
    // TTL to >= 1.0s (`normalized_lease_ttl` at execution_repo.rs:510),
    // so tests cannot use sub-second TTLs to exercise expiry. We pick
    // 1.5s (above the clamp floor) and a 500ms heartbeat (TTL/3) — the
    // wall-clock cost is zero under `tokio::time::pause()`.
    let lease_ttl = Duration::from_millis(1500);
    let heartbeat_interval = Duration::from_millis(500);

    // Runner A — has a parking handler for "park" so it holds the lease.
    let registry_a = Arc::new(ActionRegistry::new());
    registry_a.register_stateless(CountingEchoHandler {
        meta: meta(action_key!("echo")),
        invocations: Arc::clone(&echo_invocations),
    });
    registry_a.register_stateless(ParkHandler {
        meta: meta(action_key!("park")),
        started: Arc::clone(&started_a),
        invocations: Arc::clone(&park_invocations),
    });

    // Runner B — same action_keys, but "park" is a fast-completing
    // handler so the resumed workflow can finish.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless(CountingEchoHandler {
        meta: meta(action_key!("echo")),
        invocations: Arc::clone(&echo_invocations),
    });
    registry_b.register_stateless(CountingEchoHandler {
        meta: meta(action_key!("park")),
        invocations: Arc::clone(&park_invocations),
    });

    // Event bus on engine_a — used to capture the execution_id without
    // racing against repo internals.
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();

    let engine_a = Arc::new(
        make_engine(registry_a)
            .with_execution_repo(Arc::clone(&exec_repo) as Arc<dyn ExecutionRepo>)
            .with_workflow_repo(Arc::clone(&workflow_repo) as Arc<dyn WorkflowRepo>)
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval)
            .with_event_bus(event_bus),
    );
    let engine_b = Arc::new(
        make_engine(registry_b)
            .with_execution_repo(Arc::clone(&exec_repo) as Arc<dyn ExecutionRepo>)
            .with_workflow_repo(Arc::clone(&workflow_repo) as Arc<dyn WorkflowRepo>)
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval),
    );

    // Workflow: X (echo) → Y (park).
    let x = node_key!("x");
    let y = node_key!("y");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(x.clone(), "X", "echo").unwrap(),
            NodeDefinition::new(y.clone(), "Y", "park").unwrap(),
        ],
        vec![Connection::new(x.clone(), y.clone())],
    );
    let workflow_id = wf.id;

    // Persist workflow definition for `resume_execution` (engine.rs:1320).
    workflow_repo
        .save(workflow_id, 0, serde_json::to_value(&wf).unwrap())
        .await
        .unwrap();

    // Start runner A.
    let task_a = {
        let engine_a = Arc::clone(&engine_a);
        let wf = wf.clone();
        tokio::spawn(async move {
            engine_a
                .execute_workflow(
                    &wf,
                    serde_json::json!("payload"),
                    ExecutionBudget::default(),
                )
                .await
        })
    };

    // Wait for the parking handler to enter its `await` — proves X
    // completed and Y started on runner A. Bound the wait so a missing
    // signal fails the test fast instead of hanging CI.
    tokio::time::timeout(Duration::from_secs(5), started_a.notified())
        .await
        .expect("parking handler must signal `started_a` within 5s");

    // Capture execution_id from the first NodeStarted event observed.
    let execution_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeStarted { execution_id, .. }) => break execution_id,
                Some(_) => continue,
                None => panic!("event bus closed before NodeStarted"),
            }
        }
    })
    .await
    .expect("NodeStarted event must arrive within 5s");

    // Mid-flight invariants.
    assert_eq!(
        echo_invocations.load(Ordering::SeqCst),
        1,
        "X (echo) should have completed exactly once on runner A"
    );
    assert_eq!(
        park_invocations.load(Ordering::SeqCst),
        1,
        "Y (park) should have entered its handler exactly once on runner A"
    );

    // Simulate runner crash: abort runner A's task. This drops the
    // future holding the LeaseGuard, which cascades to:
    // - LeaseGuard::drop signals heartbeat shutdown + aborts heartbeat handle
    //   (engine.rs:4207-4216);
    // - the lease holder/expires_at row is left intact in storage — TTL expiry is the takeover path
    //   (ADR-0008 §5).
    task_a.abort();
    let _ = task_a.await;

    // Advance tokio's paused clock past the lease TTL. The InMemory
    // repo's `acquire_lease` predicate (`expires_at >= now`) then
    // returns false for the stale holder, so a fresh holder can claim
    // the row. Buffer = 200ms past TTL covers any heartbeat-induced
    // expiry bump that landed before abort.
    tokio::time::advance(lease_ttl + Duration::from_millis(200)).await;

    // Runner B resumes. Should:
    // - acquire the lease (engine.rs:828, fence on `lease_holder IS NULL OR lease_expires_at <
    //   NOW()` — in-memory parity at execution_repo.rs:594-609);
    // - load state, see X terminal, skip re-dispatch;
    // - re-dispatch Y (non-terminal, `Running` reset to `Pending` per engine.rs:1375-1383); B's
    //   "park" handler completes immediately;
    // - drive the workflow to Succeeded.
    let result_b = tokio::time::timeout(
        Duration::from_secs(10),
        engine_b.resume_execution(execution_id),
    )
    .await
    .expect("resume_execution must complete within 10s")
    .expect("resume_execution must return Ok");

    assert!(
        matches!(result_b.status, ExecutionStatus::Completed),
        "runner B must drive the workflow to Succeeded (got {:?})",
        result_b.status
    );

    // Final invariants — the M2.2 DoD claim "no double-execution of
    // completed work after lease handoff".
    assert_eq!(
        echo_invocations.load(Ordering::SeqCst),
        1,
        "X (echo) was completed by runner A; runner B must NOT re-run it (canon §11.3 idempotency)"
    );
    assert_eq!(
        park_invocations.load(Ordering::SeqCst),
        2,
        "Y (park) was incomplete on runner-A abort; runner B must re-dispatch it once \
         (legitimate retry after lease handoff — A's invocation + B's invocation)"
    );
}
