//! Engine integration tests for execution-lease heartbeat enforcement
//! across simulated runner restarts.
//!
//! Verifies that when runner A holds an execution's lease via the engine
//! heartbeat task and "crashes" (we simulate a process death by aborting
//! its `execute_workflow` future), runner B can resume the same execution
//! after the lease TTL expires, and the resume path respects per-node
//! terminal status (no double-execution of completed work).
//!
//! ## Invariant-equivalence note (port migration)
//!
//! These tests originally drove the engine through the legacy
//! `nebula_storage::{ExecutionRepo, WorkflowRepo}` + `repos::ControlQueueRepo`
//! god-traits. They now drive the spec-16 scoped port
//! (`ExecutionStore` + `WorkflowVersionStore` + `ControlQueue`, bundled
//! via `WorkflowEngine::with_execution_stores`/`with_workflow_stores`,
//! `EngineControlDispatch::new_port`, `ControlConsumer::new_port`). The
//! engine threads the per-message scope from the DTO on the production
//! path; test wiring uses `single_tenant_scope()` so the raw in-memory adapters
//! behave as one coherent tenant — identical observable behaviour to the old single-tenant
//! repos. The §M2.2 lease-handoff guarantees are unchanged and asserted
//! verbatim:
//! - heartbeat-loss → TTL-expiry takeover by a second runner with no
//!   double-execution of terminal work (per-node idempotency);
//! - durable Cancel survives runner death and is redelivered to the new
//!   runner via the control-queue reclaim sweep;
//! - lease-less `replay_execution` never contends for a held lease.
//!
//! Two semantic refinements the port path makes explicit (both
//! strengthen, not weaken, the guarantee): the engine now threads a
//! `FencingToken` into every committed transition, so a superseded holder
//! is rejected even on a matching CAS version; and the durable-Cancel
//! reclaim shape is reproduced via the port `ControlQueue` (claim by a
//! dead processor, then the new runner's reclaim sweep redelivers it)
//! rather than by pre-seeding a wall-clock-stale `Processing` row, since
//! the port in-memory queue tracks staleness with a monotonic `Instant`.
//! The redelivery invariant (Cancel reaches runner B, row ends
//! `Completed`, `reclaim_count >= 1`) is asserted unchanged.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{ActionKey, Dependencies, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, ControlConsumer, ControlDispatch,
    DataPassingPolicy, EngineControlDispatch, ExecutionEvent, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryControlQueue, InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, WorkflowVersionRecord};
use nebula_storage_port::store::{ControlQueue, ExecutionStore, WorkflowVersionStore};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};
use tokio_util::sync::CancellationToken;

/// Widen a short test label into the fixed 16-byte `ControlConsumer`
/// processor id. Explicit padding at the test boundary — the production
/// type is `[u8; 16]` so workers can no longer silently fence-collapse.
fn proc16(label: &[u8]) -> [u8; 16] {
    let mut id = [0u8; 16];
    let n = label.len().min(16);
    id[..n].copy_from_slice(&label[..n]);
    id
}

/// Bundled port adapters for one shared in-memory tenant. Mirrors the
/// in-source `TestStores` pattern: every field is a real port trait the
/// engine consumes; `single_tenant_scope()` makes the raw adapters behave as a
/// single coherent tenant.
#[derive(Clone)]
struct LeaseStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl LeaseStores {
    fn new() -> Self {
        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = InMemoryWorkflowVersionStore::new();
        let workflow = nebula_storage::InMemoryWorkflowStore::new_with_versions(&versions);
        Self {
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            workflow: Arc::new(workflow),
            versions: Arc::new(versions),
        }
    }

    fn execution_stores(&self) -> nebula_engine::ExecutionStores {
        nebula_engine::ExecutionStores {
            execution: self.execution.clone(),
            journal: self.journal.clone(),
            node_results: self.node_results.clone(),
            checkpoints: self.checkpoints.clone(),
            idempotency: self.idempotency.clone(),
            resume_tokens: Arc::new(self.execution.resume_token_store()),
        }
    }

    fn workflow_stores(&self) -> nebula_engine::WorkflowStores {
        nebula_engine::WorkflowStores {
            workflow: self.workflow.clone(),
            versions: self.versions.clone(),
        }
    }

    /// Attach both bundles to `engine` (mirrors the production
    /// composition root minus the tenancy decorator).
    fn attach(&self, engine: WorkflowEngine) -> WorkflowEngine {
        engine
            .with_execution_stores(self.execution_stores())
            .with_workflow_stores(self.workflow_stores())
    }

    /// Persist a workflow definition as published version 0 so the
    /// resume path's `get_published` lookup resolves it.
    async fn save_workflow(&self, wf: &WorkflowDefinition) {
        let scope = nebula_engine::store_seam::single_tenant_scope();
        let definition = serde_json::to_value(wf).unwrap();
        self.versions
            .create(
                &scope,
                WorkflowVersionRecord {
                    workflow_id: wf.id.to_string(),
                    number: 0,
                    published: true,
                    pinned: false,
                    definition,
                },
            )
            .await
            .unwrap();
    }

    /// Whether a stranger holder is blocked from acquiring the lease
    /// (the port analog of the legacy `!acquire_lease(...)` probe).
    async fn lease_held(&self, id: nebula_core::id::ExecutionId, ttl: Duration) -> bool {
        let scope = nebula_engine::store_seam::single_tenant_scope();
        self.execution
            .acquire_lease(&scope, &id.to_string(), "lease-probe-stranger", ttl)
            .await
            .unwrap()
            .is_none()
    }
}

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Macro to emit Variant A `impl Action` with placeholder static metadata.
/// Real per-instance metadata flows through `register_stateless_instance`.
macro_rules! placeholder_action_impl {
    ($ty:ty, $key:expr, $name:expr, $desc:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, $desc)
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

/// Counts invocations and echoes input. Used for nodes that should NOT
/// re-run after lease handoff (terminal-completed before the simulated
/// crash).
struct CountingEchoHandler {
    invocations: Arc<AtomicU32>,
}

placeholder_action_impl!(
    CountingEchoHandler,
    action_key!("placeholder.counting_echo"),
    "CountingEchoPlaceholder",
    "placeholder"
);

impl StatelessAction for CountingEchoHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
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
    started: Arc<tokio::sync::Notify>,
    invocations: Arc<AtomicU32>,
}

placeholder_action_impl!(
    ParkHandler,
    action_key!("placeholder.park"),
    "ParkPlaceholder",
    "placeholder"
);

impl StatelessAction for ParkHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
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
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

fn make_engine(registry: Arc<ActionRegistry>) -> WorkflowEngine {
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    WorkflowEngine::new(runtime, metrics).unwrap()
}

// ---------------------------------------------------------------------------
// T3 — heartbeat-loss → takeover
// ---------------------------------------------------------------------------

/// Heartbeat-loss → TTL-expiry takeover.
///
/// Two engines share one in-memory execution store and workflow store.
/// The workflow has a fast `echo` node X followed by a parking node Y.
/// Runner A starts the workflow; X completes, Y enters the parking
/// handler and signals `started_a`. We simulate a runner crash by
/// aborting A's `execute_workflow` future — the `LeaseGuard` drops,
/// killing A's heartbeat task. After advancing tokio's paused clock past
/// the lease TTL, runner B's `resume_execution` is called.
///
/// Asserts "no double-execution of completed work":
/// - X (echo) is **not** re-dispatched on B (idempotency idempotency)
/// - Y (park) **is** re-dispatched on B because A never finished it (legitimate retry after lease
///   handoff)
/// - B drives the workflow to `Succeeded`
#[tokio::test(start_paused = true)]
async fn engine_b_takes_over_after_engine_a_runner_dies() {
    let stores = LeaseStores::new();

    let echo_invocations = Arc::new(AtomicU32::new(0));
    let park_invocations = Arc::new(AtomicU32::new(0));
    let started_a = Arc::new(tokio::sync::Notify::new());

    // Note on timings under paused time: the in-memory execution store
    // clamps the lease TTL to >= 1.0s, so tests cannot use sub-second
    // TTLs to exercise expiry. We pick 1.5s (above the clamp floor) and a
    // 500ms heartbeat (TTL/3) — the wall-clock cost is zero under
    // `tokio::time::pause()`.
    let lease_ttl = Duration::from_millis(1500);
    let heartbeat_interval = Duration::from_millis(500);

    // Runner A — has a parking handler for "park" so it holds the lease.
    let registry_a = Arc::new(ActionRegistry::new());
    registry_a.register_stateless_instance(
        meta(action_key!("echo")),
        CountingEchoHandler {
            invocations: Arc::clone(&echo_invocations),
        },
    );
    registry_a.register_stateless_instance(
        meta(action_key!("park")),
        ParkHandler {
            started: Arc::clone(&started_a),
            invocations: Arc::clone(&park_invocations),
        },
    );

    // Runner B — same action_keys, but "park" is a fast-completing
    // handler so the resumed workflow can finish.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless_instance(
        meta(action_key!("echo")),
        CountingEchoHandler {
            invocations: Arc::clone(&echo_invocations),
        },
    );
    registry_b.register_stateless_instance(
        meta(action_key!("park")),
        CountingEchoHandler {
            invocations: Arc::clone(&park_invocations),
        },
    );

    // Event bus on engine_a — used to capture the execution_id without
    // racing against store internals.
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();

    let engine_a = Arc::new(
        stores
            .attach(make_engine(registry_a))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval)
            .with_event_bus(event_bus),
    );
    let engine_b = Arc::new(
        stores
            .attach(make_engine(registry_b))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval),
    );

    // Workflow: X (echo) → Y (park).
    let x = node_key!("x");
    let y = node_key!("y");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(x.clone(), "X", "core", "echo").unwrap(),
            NodeDefinition::new(y.clone(), "Y", "core", "park").unwrap(),
        ],
        vec![Connection::new(x.clone(), y.clone())],
    );

    // Persist workflow definition so `resume_execution` can reload it.
    stores.save_workflow(&wf).await;

    // Start runner A.
    let task_a = {
        let engine_a = Arc::clone(&engine_a);
        let wf = wf.clone();
        tokio::spawn(async move {
            engine_a
                .execute_workflow(
                    &nebula_engine::store_seam::single_tenant_scope(),
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
    // - `LeaseGuard::drop` signalling heartbeat shutdown + aborting the
    //   heartbeat handle;
    // - the lease holder/expires_at row left intact in storage — TTL
    // expiry is the takeover path.
    task_a.abort();
    let _ = task_a.await;

    // Advance tokio's paused clock past the lease TTL. The in-memory
    // execution store's `acquire_lease` liveness predicate
    // (`expires_at >= now`) then returns false for the stale holder, so a
    // fresh holder can claim the row. Buffer = 200ms past TTL covers any
    // heartbeat-induced expiry bump that landed before abort.
    tokio::time::advance(lease_ttl + Duration::from_millis(200)).await;

    // Runner B resumes. Should:
    // - acquire the lease (fenced on "no live holder"; the port path
    //   mints a fresh fencing token so a zombie A is rejected even on a
    //   matching version);
    // - load state, see X terminal, skip re-dispatch;
    // - re-dispatch Y (non-terminal, `Running` reset to `Pending`); B's
    //   "park" handler completes immediately;
    // - drive the workflow to Succeeded.
    let result_b = tokio::time::timeout(
        Duration::from_secs(10),
        engine_b.resume_execution(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        ),
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
        "X (echo) was completed by runner A; runner B must NOT re-run it (idempotency idempotency)"
    );
    assert_eq!(
        park_invocations.load(Ordering::SeqCst),
        2,
        "Y (park) was incomplete on runner-A abort; runner B must re-dispatch it once \
         (legitimate retry after lease handoff — A's invocation + B's invocation)"
    );
}

// ---------------------------------------------------------------------------
// Cancel-token loss across runner restart (control-queue redeliver)
// ---------------------------------------------------------------------------

/// Durable Cancel survives runner death and reaches runner B via the
/// reclaim sweep.
///
/// Setup: runner A holds a parked workflow. The Cancel command is
/// enqueued on the port `ControlQueue` and then **claimed once by a
/// dead processor id** — the port-faithful equivalent of "runner A's
/// consumer claimed the Cancel and then the process died before acking
/// it" (the legacy test pre-seeded a wall-clock-stale `Processing` row;
/// the port in-memory queue tracks staleness with a monotonic `Instant`
/// and has no public seed-as-Processing API, so we drive the same state
/// through the real `claim_pending`). We then abort runner A and bring
/// up runner B with its own port-backed `EngineControlDispatch` +
/// `ControlConsumer`. The reclaim sweep moves the orphaned row back to
/// `Pending`, the consumer claims it and dispatches Cancel into runner
/// B's running registry — runner B's frontier observes the cancel, the
/// parking handler returns `ActionError::Cancelled`, and runner B
/// finalizes the execution to `Cancelled`.
///
/// Asserts (unchanged from the legacy test):
/// - runner B's `resume_execution` returns with status `Cancelled`
/// - X (echo) was completed by runner A; runner B does NOT re-run it
/// - Y (park) was re-dispatched by runner B (1 invocation each → total 2)
/// - the Cancel row ends `Completed` with `reclaim_count >= 1`
///
/// Real wall-clock time is used here (not `tokio::time::pause`) because
/// the port in-memory queue's reclaim staleness is tracked with a
/// monotonic `std::time::Instant` that is not driven by tokio's paused
/// clock, while the lease TTL (`tokio::time::Instant`) is — same
/// trade-off the legacy test made (~1.8s wall-clock cost).
#[tokio::test]
async fn engine_b_cancels_execution_after_runner_a_death_via_reclaim_redeliver() {
    let stores = LeaseStores::new();
    let queue = Arc::new(InMemoryControlQueue::new(&stores.execution));

    let echo_invocations = Arc::new(AtomicU32::new(0));
    let park_invocations = Arc::new(AtomicU32::new(0));
    let started_a = Arc::new(tokio::sync::Notify::new());
    let started_b = Arc::new(tokio::sync::Notify::new());

    // Lease TTL clamped to >= 1.0s by the in-memory execution store.
    // Heartbeat is longer than the time-to-abort so runner A's heartbeat
    // does not tick (and thus does not bump expires_at) before abort.
    let lease_ttl = Duration::from_millis(1500);
    let heartbeat_interval = Duration::from_secs(2);

    // engine_a — parking Y so it holds the lease.
    let registry_a = Arc::new(ActionRegistry::new());
    registry_a.register_stateless_instance(
        meta(action_key!("echo")),
        CountingEchoHandler {
            invocations: Arc::clone(&echo_invocations),
        },
    );
    registry_a.register_stateless_instance(
        meta(action_key!("park")),
        ParkHandler {
            started: Arc::clone(&started_a),
            invocations: Arc::clone(&park_invocations),
        },
    );

    // engine_b — also parking Y so the cancel signal has somewhere to land.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless_instance(
        meta(action_key!("echo")),
        CountingEchoHandler {
            invocations: Arc::clone(&echo_invocations),
        },
    );
    registry_b.register_stateless_instance(
        meta(action_key!("park")),
        ParkHandler {
            started: Arc::clone(&started_b),
            invocations: Arc::clone(&park_invocations),
        },
    );

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();

    let engine_a = Arc::new(
        stores
            .attach(make_engine(registry_a))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval)
            .with_event_bus(event_bus),
    );
    let engine_b = Arc::new(
        stores
            .attach(make_engine(registry_b))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval),
    );

    // Workflow X (echo) → Y (park).
    let x = node_key!("x");
    let y = node_key!("y");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(x.clone(), "X", "core", "echo").unwrap(),
            NodeDefinition::new(y.clone(), "Y", "core", "park").unwrap(),
        ],
        vec![Connection::new(x.clone(), y.clone())],
    );
    stores.save_workflow(&wf).await;

    // Start runner A.
    let task_a = {
        let engine_a = Arc::clone(&engine_a);
        let wf = wf.clone();
        tokio::spawn(async move {
            engine_a
                .execute_workflow(
                    &nebula_engine::store_seam::single_tenant_scope(),
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

    // Enqueue the Cancel, then claim it once with a dead processor id —
    // the port-faithful "engine_a's consumer claimed the Cancel and then
    // the process died before acking it" state. The row is now
    // `Processing`, owned by a processor that never comes back; runner
    // B's consumer reclaim sweep (short `reclaim_after`, real wall-clock)
    // will move it back to `Pending` and redeliver it.
    let cancel_row_id = [0xcau8; 16];
    queue
        .enqueue(&ControlMsg {
            id: cancel_row_id,
            execution_id: execution_id.to_string(),
            command: ControlCommand::Cancel,
            scope: nebula_engine::store_seam::single_tenant_scope(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        })
        .await
        .unwrap();
    let dead_processor = *b"engine-a-deadrnr";
    let claimed = queue.claim_pending(&dead_processor, 8).await.unwrap();
    assert!(
        claimed.iter().any(|m| m.id == cancel_row_id),
        "the dead processor must have claimed the Cancel row (now orphaned Processing)"
    );

    // Simulate runner A's process death.
    task_a.abort();
    let _ = task_a.await;

    // Wait for the lease to expire so engine_b's resume can acquire it.
    // tokio::time::Instant under non-paused time advances with wall-clock
    // here, so this real sleep is the right primitive.
    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;

    // Runner B resumes — acquires the lease, resets non-terminal nodes,
    // re-dispatches Y. Its parking handler signals started_b on entry.
    let task_b = {
        let engine_b = Arc::clone(&engine_b);
        tokio::spawn(async move {
            engine_b
                .resume_execution(
                    &nebula_engine::store_seam::single_tenant_scope(),
                    execution_id,
                )
                .await
        })
    };

    // Wait until engine_b's running registry has the entry (parking
    // handler entered Y); otherwise the consumer's dispatch_cancel
    // would land before runner B owns the execution and the cancel
    // signal would be a no-op.
    tokio::time::timeout(Duration::from_secs(5), started_b.notified())
        .await
        .expect("engine_b parking handler must signal `started_b` within 5s");

    // Bring up the consumer with engine_b's port-backed
    // EngineControlDispatch + a small reclaim window so the orphaned
    // Processing row is swept back to Pending and re-claimed within
    // seconds. Same scoped store the engine was configured with, so the
    // dispatch idempotency read and the engine's CAS observe one row.
    let dispatch_b: Arc<dyn ControlDispatch> = Arc::new(EngineControlDispatch::new(
        Arc::clone(&engine_b),
        stores.execution.clone(),
    ));
    let consumer = ControlConsumer::new(queue.clone(), dispatch_b, proc16(b"runner-b"))
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
    //
    // Budget: 30s headroom for slow CI runners (macOS GitHub-hosted runners
    // were flaky at the original 10s budget — wall-clock-based reclaim
    // sweep + real-time sleep make this test sensitive to runner load).
    let result_b = tokio::time::timeout(Duration::from_secs(30), task_b)
        .await
        .expect("engine_b.resume_execution must complete within 30s")
        .expect("task_b joined")
        .expect("resume_execution Ok");

    consumer_shutdown.cancel();
    consumer_handle.await.expect("graceful consumer shutdown");

    // Final invariants — the cross-runner cancel-redeliver claim.
    assert!(
        matches!(result_b.status, ExecutionStatus::Cancelled),
        "runner B must drive the workflow to Cancelled (got {:?})",
        result_b.status
    );
    assert_eq!(
        echo_invocations.load(Ordering::SeqCst),
        1,
        "X (echo) was completed by runner A; runner B must NOT re-run it (idempotency)"
    );
    assert_eq!(
        park_invocations.load(Ordering::SeqCst),
        2,
        "Y (park) was re-dispatched by runner B (A's invocation + B's invocation, B then cancelled)"
    );

    // Cancel queue row should end in Completed (consumer acked), with
    // `reclaim_count >= 1` proving it went through the reclaim sweep.
    let snap = queue.snapshot();
    let (row, status) = snap
        .iter()
        .find(|(m, _)| m.id == cancel_row_id)
        .expect("Cancel row must still be present in the queue");
    assert_eq!(
        status, "Completed",
        "consumer must have acked the Cancel row to Completed after dispatch (got {status:?})"
    );
    assert!(
        row.reclaim_count >= 1,
        "row must have been reclaimed at least once (got reclaim_count={})",
        row.reclaim_count
    );
}

// ---------------------------------------------------------------------------
// replay_execution does not contend for the source-execution lease
// ---------------------------------------------------------------------------

/// `replay_execution` is intentionally lease-less.
///
/// `WorkflowEngine::replay_execution` mints a fresh `ExecutionId` per
/// call and runs the frontier loop without calling
/// `acquire_and_heartbeat_lease`. So a replay started by runner B while
/// runner A holds the source execution's lease must:
/// - return a fresh `execution_id` distinct from runner A's
/// - complete on its own without blocking on or stealing runner A's lease
/// - leave runner A's lease intact in storage
///
/// This locks down the lease-less replay invariant — without this test,
/// a future "wire replay through `acquire_lease`" refactor could
/// silently introduce contention.
#[tokio::test(start_paused = true)]
async fn replay_does_not_contend_for_held_lease() {
    use nebula_execution::ReplayPlan;

    let stores = LeaseStores::new();

    let echo_invocations = Arc::new(AtomicU32::new(0));
    let park_invocations = Arc::new(AtomicU32::new(0));
    let started_a = Arc::new(tokio::sync::Notify::new());

    let lease_ttl = Duration::from_millis(1500);
    let heartbeat_interval = Duration::from_millis(500);

    // engine_a — parking workflow to hold the lease.
    let registry_a = Arc::new(ActionRegistry::new());
    registry_a.register_stateless_instance(
        meta(action_key!("echo")),
        CountingEchoHandler {
            invocations: Arc::clone(&echo_invocations),
        },
    );
    registry_a.register_stateless_instance(
        meta(action_key!("park")),
        ParkHandler {
            started: Arc::clone(&started_a),
            invocations: Arc::clone(&park_invocations),
        },
    );

    // engine_b — only needs the echo handler for its replay workflow.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless_instance(
        meta(action_key!("echo")),
        CountingEchoHandler {
            invocations: Arc::clone(&echo_invocations),
        },
    );

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();

    let engine_a = Arc::new(
        make_engine(registry_a)
            .with_execution_stores(stores.execution_stores())
            .with_workflow_stores(stores.workflow_stores())
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat_interval)
            .with_event_bus(event_bus),
    );
    // Note: engine_b is intentionally NOT given execution stores.
    // `replay_execution` runs in-process (it never calls
    // `ExecutionStore::create` for its fresh ExecutionId — only
    // `execute_workflow` seeds the row), so attaching an execution store
    // would make every `commit` a CAS-against-missing-row and fail the
    // workflow. The test still observes runner A's lease through the
    // shared `stores.execution` directly — that is what the invariant
    // cares about.
    let engine_b = make_engine(registry_b)
        .with_workflow_stores(stores.workflow_stores())
        .with_lease_ttl(lease_ttl)
        .with_lease_heartbeat_interval(heartbeat_interval);

    // Workflow for engine_a: parks at Y so engine_a holds the lease.
    let x_a = node_key!("x");
    let y_a = node_key!("y");
    let wf_a = make_workflow(
        vec![
            NodeDefinition::new(x_a.clone(), "X", "core", "echo").unwrap(),
            NodeDefinition::new(y_a.clone(), "Y", "core", "park").unwrap(),
        ],
        vec![Connection::new(x_a.clone(), y_a.clone())],
    );

    // Workflow for engine_b's replay: a single echo node so the replay
    // completes without needing engine_a's "park" handler.
    let x_b = node_key!("rx");
    let wf_b = make_workflow(
        vec![NodeDefinition::new(x_b.clone(), "RX", "core", "echo").unwrap()],
        vec![],
    );

    // Start runner A so it holds the lease.
    let task_a = {
        let engine_a = Arc::clone(&engine_a);
        let wf_a = wf_a.clone();
        tokio::spawn(async move {
            engine_a
                .execute_workflow(
                    &nebula_engine::store_seam::single_tenant_scope(),
                    &wf_a,
                    serde_json::json!("a"),
                    ExecutionBudget::default(),
                )
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

    // Sanity check: runner A's lease IS in storage (a stranger holder's
    // `acquire_lease` must observe it as held → `None`).
    let lease_held_by_a = stores.lease_held(execution_id_a, lease_ttl).await;
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
        engine_b.replay_execution(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf_b,
            plan,
            ExecutionBudget::default(),
        ),
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
    let stranger_still_blocked = stores.lease_held(execution_id_a, lease_ttl).await;
    assert!(
        stranger_still_blocked,
        "runner A's lease must still be held in storage after the replay"
    );

    // Tear down runner A so the test process exits cleanly.
    task_a.abort();
    let _ = task_a.await;
}
