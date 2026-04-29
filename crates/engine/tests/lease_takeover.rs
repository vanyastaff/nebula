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
    ActionExecutor, ActionRegistry, ActionRuntime, ControlConsumer, ControlDispatch,
    DataPassingPolicy, EngineControlDispatch, ExecutionEvent, InProcessSandbox, WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_storage::{
    ExecutionRepo, InMemoryExecutionRepo, InMemoryWorkflowRepo, WorkflowRepo,
    repos::{ControlCommand, ControlQueueEntry, ControlQueueRepo, InMemoryControlQueueRepo},
};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};
use tokio_util::sync::CancellationToken;

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

// ---------------------------------------------------------------------------
// T4 — cancel-token loss across runner restart (control-queue redeliver)
// ---------------------------------------------------------------------------

/// ROADMAP §M2.2 / T4 — durable Cancel survives runner death and reaches
/// runner B via the reclaim sweep (ADR-0008 §5 + ADR-0017).
///
/// Setup: runner A holds a parked workflow. The control-queue row for
/// the Cancel command is **pre-seeded** in `Processing` with a stale
/// `processed_at` timestamp — this simulates "runner A's consumer
/// claimed the Cancel and then the process died before acking it",
/// the same shape as the existing `reclaim_sweep_recovers_orphaned_processing_row_end_to_end`
/// test in `control_consumer_wiring.rs:362`. We then abort runner A
/// and bring up runner B with its own `EngineControlDispatch`-backed
/// `ControlConsumer`. The reclaim sweep moves the stale row back to
/// `Pending`, the consumer claims it and dispatches Cancel into runner
/// B's running registry — runner B's frontier observes the cancel,
/// the parking handler returns `ActionError::Cancelled`, and runner B
/// finalizes the execution to `Cancelled`.
///
/// Asserts:
/// - runner B's `resume_execution` returns with status `Cancelled`
/// - X (echo) was completed by runner A; runner B does NOT re-run it
/// - Y (park) was re-dispatched by runner B (1 invocation each → total 2)
///
/// Real wall-clock time is used here (not `tokio::time::pause`) because
/// `chrono::Utc::now()` (control-queue staleness) is not paused by
/// tokio, while `tokio::time::Instant` (lease TTL) is — the existing
/// `reclaim_sweep_recovers_orphaned_processing_row_end_to_end` test
/// makes the same trade-off (~1.8s wall-clock cost).
#[tokio::test]
async fn engine_b_cancels_execution_after_runner_a_death_via_reclaim_redeliver() {
    let exec_repo = Arc::new(InMemoryExecutionRepo::new());
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let queue: Arc<dyn ControlQueueRepo> = queue_repo.clone();

    let echo_invocations = Arc::new(AtomicU32::new(0));
    let park_invocations = Arc::new(AtomicU32::new(0));
    let started_a = Arc::new(tokio::sync::Notify::new());
    let started_b = Arc::new(tokio::sync::Notify::new());

    // Lease TTL clamped to >= 1.0s by InMemoryExecutionRepo. Heartbeat is
    // longer than the time-to-abort so runner A's heartbeat does not
    // tick (and thus does not bump expires_at) before we abort it.
    let lease_ttl = Duration::from_millis(1500);
    let heartbeat_interval = Duration::from_secs(2);

    // engine_a — parking Y so it holds the lease.
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

    // engine_b — also parking Y so the cancel signal has somewhere to land.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless(CountingEchoHandler {
        meta: meta(action_key!("echo")),
        invocations: Arc::clone(&echo_invocations),
    });
    registry_b.register_stateless(ParkHandler {
        meta: meta(action_key!("park")),
        started: Arc::clone(&started_b),
        invocations: Arc::clone(&park_invocations),
    });

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

    // Workflow X (echo) → Y (park).
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

    // Wait for engine_a to park at Y.
    tokio::time::timeout(Duration::from_secs(5), started_a.notified())
        .await
        .expect("engine_a parking handler must signal `started_a` within 5s");

    // Capture execution_id for the queue row.
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

    // Pre-seed the Cancel queue row in Processing with a stale
    // processed_at — simulates "engine_a's consumer claimed the Cancel
    // and then the process died before acking it" (same shape as
    // control_consumer_wiring.rs:486 reclaim_sweep_emits_counter_metric_per_outcome).
    // 600s past is far beyond any realistic reclaim_after window.
    let stale = chrono::Utc::now() - chrono::Duration::seconds(600);
    let cancel_row_id = vec![0xcau8; 16];
    queue_repo
        .enqueue(&ControlQueueEntry {
            id: cancel_row_id.clone(),
            execution_id: execution_id.to_string().into_bytes(),
            command: ControlCommand::Cancel,
            issued_by: None,
            issued_at: chrono::Utc::now(),
            status: "Processing".to_string(),
            processed_by: Some(b"engine-a-dead-runner".to_vec()),
            processed_at: Some(stale),
            error_message: None,
            reclaim_count: 0,
        })
        .await
        .unwrap();

    // Simulate runner A's process death.
    task_a.abort();
    let _ = task_a.await;

    // Wait for the Layer-1 lease to expire so engine_b's resume can
    // acquire it. tokio::time::Instant under non-paused time advances
    // with wall-clock here, so this real sleep is the right primitive.
    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;

    // Runner B resumes — acquires the lease, resets non-terminal nodes,
    // re-dispatches Y. Its parking handler signals started_b on entry.
    let task_b = {
        let engine_b = Arc::clone(&engine_b);
        tokio::spawn(async move { engine_b.resume_execution(execution_id).await })
    };

    // Wait until engine_b's running registry has the entry (parking
    // handler entered Y); otherwise the consumer's dispatch_cancel
    // would land before runner B owns the execution and the cancel
    // signal would be a no-op.
    tokio::time::timeout(Duration::from_secs(5), started_b.notified())
        .await
        .expect("engine_b parking handler must signal `started_b` within 5s");

    // Bring up the consumer with engine_b's EngineControlDispatch +
    // small reclaim window so the stale Processing row is swept back to
    // Pending and re-claimed within seconds.
    let dispatch_b: Arc<dyn ControlDispatch> = Arc::new(EngineControlDispatch::new(
        Arc::clone(&engine_b),
        Arc::clone(&exec_repo) as Arc<dyn ExecutionRepo>,
    ));
    let consumer = ControlConsumer::new(queue.clone(), dispatch_b, b"runner-b".to_vec())
        .with_batch_size(4)
        .with_poll_interval(Duration::from_millis(50))
        .with_reclaim_after(Duration::from_millis(100))
        .with_reclaim_interval(Duration::from_millis(80))
        .with_max_reclaim_count(3);
    let consumer_shutdown = CancellationToken::new();
    let consumer_handle = consumer.spawn(consumer_shutdown.clone());

    // engine_b should finalize to Cancelled within a few seconds:
    // reclaim_interval(~80ms) sweeps the stale Processing row back to
    // Pending → next claim_pending picks it up → dispatch_cancel signals
    // engine_b's running registry → frontier cancels → parking handler
    // returns ActionError::Cancelled → engine finalizes Cancelled.
    let result_b = tokio::time::timeout(Duration::from_secs(10), task_b)
        .await
        .expect("engine_b.resume_execution must complete within 10s")
        .expect("task_b joined")
        .expect("resume_execution Ok");

    consumer_shutdown.cancel();
    consumer_handle.await.expect("graceful consumer shutdown");

    // Final invariants — the M2.2 cross-runner-cancel-redeliver claim.
    assert!(
        matches!(result_b.status, ExecutionStatus::Cancelled),
        "runner B must drive the workflow to Cancelled (got {:?})",
        result_b.status
    );
    assert_eq!(
        echo_invocations.load(Ordering::SeqCst),
        1,
        "X (echo) was completed by runner A; runner B must NOT re-run it (canon §11.3)"
    );
    assert_eq!(
        park_invocations.load(Ordering::SeqCst),
        2,
        "Y (park) was re-dispatched by runner B (A's invocation + B's invocation, B then cancelled)"
    );

    // Cancel queue row should end in Completed (consumer acked).
    let snap = queue_repo.snapshot().await;
    let row = snap
        .iter()
        .find(|e| e.id == cancel_row_id)
        .expect("Cancel row must still be present in the queue");
    assert_eq!(
        row.status, "Completed",
        "consumer must have acked the Cancel row to Completed after dispatch (got {:?})",
        row.status
    );
    assert!(
        row.reclaim_count >= 1,
        "row must have been reclaimed at least once (got reclaim_count={})",
        row.reclaim_count
    );
}

// ---------------------------------------------------------------------------
// T5 — replay_execution does not contend for the source-execution lease
// ---------------------------------------------------------------------------

/// ROADMAP §M2.2 / T5 — `replay_execution` is intentionally lease-less.
///
/// `WorkflowEngine::replay_execution` mints a fresh `ExecutionId` per call
/// (`engine.rs:608`) and runs the frontier loop without calling
/// `acquire_and_heartbeat_lease`. So a replay started by runner B while
/// runner A holds the source execution's lease must:
/// - return a fresh `execution_id` distinct from runner A's
/// - complete on its own without blocking on or stealing runner A's lease
/// - leave runner A's lease intact in storage
///
/// This locks down the invariant documented at `engine.rs:594-612` —
/// without this test, a future "wire replay through `acquire_lease`"
/// refactor could silently introduce contention.
#[tokio::test(start_paused = true)]
async fn replay_does_not_contend_for_held_lease() {
    use nebula_execution::ReplayPlan;

    let exec_repo = Arc::new(InMemoryExecutionRepo::new());
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());

    let echo_invocations = Arc::new(AtomicU32::new(0));
    let park_invocations = Arc::new(AtomicU32::new(0));
    let started_a = Arc::new(tokio::sync::Notify::new());

    let lease_ttl = Duration::from_millis(1500);
    let heartbeat_interval = Duration::from_millis(500);

    // engine_a — parking workflow to hold the lease.
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

    // engine_b — only needs the echo handler for its replay workflow.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless(CountingEchoHandler {
        meta: meta(action_key!("echo")),
        invocations: Arc::clone(&echo_invocations),
    });

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
    // Note: engine_b is intentionally NOT given an `execution_repo`.
    // `replay_execution` runs in-process (it never calls `repo.create`
    // for its fresh ExecutionId — only `execute_workflow` seeds the row),
    // so configuring a repo would make every `repo.transition` return
    // `Ok(false)` (CAS-against-missing-row) and fail the workflow. The
    // test still observes runner A's lease through the shared
    // `exec_repo` directly — that is what the invariant cares about.
    let engine_b = make_engine(registry_b)
        .with_workflow_repo(Arc::clone(&workflow_repo) as Arc<dyn WorkflowRepo>)
        .with_lease_ttl(lease_ttl)
        .with_lease_heartbeat_interval(heartbeat_interval);

    // Workflow for engine_a: parks at Y so engine_a holds the lease.
    let x_a = node_key!("x");
    let y_a = node_key!("y");
    let wf_a = make_workflow(
        vec![
            NodeDefinition::new(x_a.clone(), "X", "echo").unwrap(),
            NodeDefinition::new(y_a.clone(), "Y", "park").unwrap(),
        ],
        vec![Connection::new(x_a.clone(), y_a.clone())],
    );

    // Workflow for engine_b's replay: a single echo node so the replay
    // completes without needing engine_a's "park" handler.
    let x_b = node_key!("rx");
    let wf_b = make_workflow(
        vec![NodeDefinition::new(x_b.clone(), "RX", "echo").unwrap()],
        vec![],
    );

    // Start runner A so it holds the lease.
    let task_a = {
        let engine_a = Arc::clone(&engine_a);
        let wf_a = wf_a.clone();
        tokio::spawn(async move {
            engine_a
                .execute_workflow(&wf_a, serde_json::json!("a"), ExecutionBudget::default())
                .await
        })
    };

    tokio::time::timeout(Duration::from_secs(5), started_a.notified())
        .await
        .expect("engine_a parking handler must signal `started_a` within 5s");

    let execution_id_a = tokio::time::timeout(Duration::from_secs(5), async {
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

    // Sanity check: runner A's lease IS in storage (in-memory repo's
    // `acquire_lease` for a stranger holder must observe it as held).
    let lease_held_by_a = !exec_repo
        .acquire_lease(execution_id_a, "stranger".into(), lease_ttl)
        .await
        .unwrap();
    assert!(
        lease_held_by_a,
        "engine_a must hold the lease for execution_id_a before the replay"
    );

    // Now runner B replays a *different* workflow with a fresh
    // execution_id minted internally. The replay must complete without
    // touching engine_a's lease.
    let plan = ReplayPlan::new(execution_id_a, x_b.clone());
    let result_b = tokio::time::timeout(
        Duration::from_secs(10),
        engine_b.replay_execution(&wf_b, plan, ExecutionBudget::default()),
    )
    .await
    .expect("replay_execution must complete within 10s")
    .expect("replay_execution must return Ok");

    // Invariants:
    // - Replay minted a fresh ExecutionId (not engine_a's).
    assert_ne!(
        result_b.execution_id, execution_id_a,
        "replay_execution must mint a fresh ExecutionId distinct from runner A's"
    );
    // - Replay completed.
    assert!(
        matches!(result_b.status, ExecutionStatus::Completed),
        "replay must drive its own workflow to Completed (got {:?})",
        result_b.status
    );
    // - Replay's echo invocation increments the shared counter exactly once on top of runner A's
    //   (which already invoked X once before parking).
    assert_eq!(
        echo_invocations.load(Ordering::SeqCst),
        2,
        "replay's echo must run once (on top of runner A's earlier echo)"
    );
    // - Runner A's lease is still held — replay did NOT release or steal it.
    let stranger_still_blocked = !exec_repo
        .acquire_lease(execution_id_a, "stranger-2".into(), lease_ttl)
        .await
        .unwrap();
    assert!(
        stranger_still_blocked,
        "runner A's lease must still be held in storage after the replay"
    );

    // Tear down runner A so the test process exits cleanly.
    task_a.abort();
    let _ = task_a.await;
}
