//! Integration test: crash-without-Resume timer-wait recovery.
//!
//! Verifies the HYPOTHESIS from the engine audit:
//!
//! > crash → control-queue row stays Processing → reclaim re-delivers the
//! > original command → dispatch → `resume_execution` → re-seeds `wait_heap`
//! > from persisted `next_attempt_at` → Phase 0b fires the now-overdue timer.
//!
//! Specifically, this test exercises the **pure timer-wait** path
//! (`WaitCondition::Duration`, `timeout: None`) — distinct from the
//! signal+timeout wait already covered in `wait_timeout.rs`. The timer-only
//! path:
//!
//! - Produces `wake_at = Some(deadline)` → pushed to `wait_heap`.
//! - Persists `next_attempt_at = deadline`, `wait_wake = None` (Completion).
//! - Keeps the execution `Running` (not `Paused`) because `wake_at.is_some()`.
//!
//! On a fresh re-drive (`resume_execution` from a new engine instance, same
//! durable store, NO Resume command), `run_frontier` re-seeds `wait_heap` from
//! `next_attempt_at` at lines 105-111 of `frontier.rs`. If the deadline is
//! already past, Phase 0b drains the entry immediately, reads `wait_wake = None`
//! (treated as `Completion`), transitions `Waiting → Completed`, and activates
//! downstream edges. The execution then completes.
//!
//! ## Timing model
//!
//! The engine's `wait_heap` deadlines are wall-clock (`chrono::Utc`), not tokio
//! timers, so `tokio::time::pause`/`advance` cannot drive them. The test uses
//! real, short durations: 1s timer deadline, abort at 200ms, sleep 1.2s to
//! ensure the deadline is overdue before the re-drive. This mirrors the pattern
//! from `wait_timeout.rs` (`crash_mid_wait_with_timeout_recovers_and_can_still_timeout`).

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError,
    action::Action,
    metadata::ActionMetadata,
    result::{ActionResult, WaitCondition},
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionEvent,
    InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    dto::WorkflowVersionRecord,
    store::{ExecutionStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Action stubs ─────────────────────────────────────────────────────────────

macro_rules! static_action_impl {
    ($ty:ty, $key:expr, $name:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, "timer_wait_crash_recovery test stub")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

/// Parks on a `WaitCondition::Duration` timer — the pure timer-wait path.
/// `timeout: None` means `wait_wake = None` on the persisted row, which
/// Phase 0b treats as `Completion` (not `Timeout`). The execution stays
/// `Running` (not `Paused`) because `wake_at = Some(deadline)`.
struct TimerWaitAction {
    duration: Duration,
}

static_action_impl!(
    TimerWaitAction,
    action_key!("test.twcr.timer_wait"),
    "TimerWaitAction"
);

impl StatelessAction for TimerWaitAction {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Duration {
                duration: self.duration,
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Counts invocations. The downstream node is the observable gate: it must run
/// after the recovered timer fires, proving Phase 0b completed the wait node.
struct CountingDownstream {
    invocations: Arc<AtomicU32>,
}

static_action_impl!(
    CountingDownstream,
    action_key!("test.twcr.downstream"),
    "CountingDownstream"
);

impl StatelessAction for CountingDownstream {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// ── Shared store bundle ───────────────────────────────────────────────────────

struct CrashRecoveryStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl CrashRecoveryStores {
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

    fn attach(&self, engine: WorkflowEngine) -> WorkflowEngine {
        engine
            .with_execution_stores(self.execution_stores())
            .with_workflow_stores(self.workflow_stores())
    }

    async fn save_workflow(&self, wf: &WorkflowDefinition) {
        self.versions
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                WorkflowVersionRecord {
                    workflow_id: wf.id.to_string(),
                    number: 0,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(wf).unwrap(),
                },
            )
            .await
            .unwrap();
    }

    async fn persist_created_execution(&self, workflow_id: nebula_core::WorkflowId) -> ExecutionId {
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
        exec_state.set_workflow_input(serde_json::json!(null));
        self.execution
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
                &workflow_id.to_string(),
                serde_json::to_value(&exec_state).unwrap(),
            )
            .await
            .unwrap();
        execution_id
    }

    async fn load_state(&self, execution_id: ExecutionId) -> ExecutionState {
        let record = self
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
            )
            .await
            .unwrap()
            .expect("execution row must exist");
        let raw = serde_json::to_string(&record.state).unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    async fn persisted_status(&self, execution_id: ExecutionId) -> ExecutionStatus {
        let record = self
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
            )
            .await
            .unwrap()
            .expect("execution row must exist");
        serde_json::from_value(record.state["status"].clone()).unwrap()
    }
}

// ── Engine assembly ───────────────────────────────────────────────────────────

fn build_registry(
    timer_duration: Duration,
    downstream_invocations: &Arc<AtomicU32>,
) -> Arc<ActionRegistry> {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.twcr.timer_wait"),
            "TimerWaitAction",
            "timer_wait_crash_recovery stub",
        ),
        TimerWaitAction {
            duration: timer_duration,
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.twcr.downstream"),
            "CountingDownstream",
            "timer_wait_crash_recovery stub",
        ),
        CountingDownstream {
            invocations: Arc::clone(downstream_invocations),
        },
    );
    registry
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

/// Workflow: `timer_node ──main──> downstream_node`.
/// `timer_node` parks on a `Duration` wait; `downstream_node` counts invocations.
fn build_workflow() -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let timer_node = node_key!("timer_node");
    let downstream_node = node_key!("downstream_node");
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "timer-wait-crash-recovery".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                timer_node.clone(),
                "TimerNode",
                "core",
                "test.twcr.timer_wait",
            )
            .unwrap(),
            NodeDefinition::new(
                downstream_node.clone(),
                "DownstreamNode",
                "core",
                "test.twcr.downstream",
            )
            .unwrap(),
        ],
        connections: vec![Connection::new(timer_node, downstream_node)],
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

// ── Shared park-then-crash setup ───────────────────────────────────────────────

/// Lease TTL: the in-mem store clamps to >= 1s, so 1s is the floor — long enough
/// that runner A holds the lease while alive, short enough that it expires in the
/// overdue-sleep window so the recovery path can re-acquire the abandoned lease.
const LEASE_TTL: Duration = Duration::from_secs(1);
const HEARTBEAT: Duration = Duration::from_millis(300);
/// 1s timer — will NOT fire before we abort runner A, clearly overdue after the
/// 1.2s sleep.
const TIMER_DURATION: Duration = Duration::from_secs(1);
/// Sleep until the deadline is overdue AND the abandoned lease's TTL has lapsed.
const OVERDUE_SLEEP: Duration = Duration::from_millis(1_200);

/// Drive a single-timer workflow to its park, then "crash" the runner (abort the
/// drive task right after `NodeParked`), and sleep until the deadline is overdue
/// and the abandoned lease has expired. Returns the persisted `execution_id` of a
/// `Running` execution holding a `Waiting` timer node with an overdue
/// `next_attempt_at` and no live owner — the exact durable state a recovery path
/// must wake.
async fn park_timer_then_crash(
    stores: &CrashRecoveryStores,
    downstream_invocations: &Arc<AtomicU32>,
) -> ExecutionId {
    let wf = build_workflow();
    stores.save_workflow(&wf).await;

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(
                make_engine(build_registry(TIMER_DURATION, downstream_invocations))
                    .with_event_bus(event_bus),
            )
            .with_lease_ttl(LEASE_TTL)
            .with_lease_heartbeat_interval(HEARTBEAT),
    );

    let execution_id = stores.persist_created_execution(wf.id).await;
    let engine_a_handle = Arc::clone(&engine_a);
    let scope_a = nebula_engine::store_seam::single_tenant_scope();
    let task_a = tokio::spawn(async move {
        engine_a_handle
            .resume_execution(&scope_a, execution_id)
            .await
    });

    // `NodeParked` fires only after the park checkpoint has durably landed, so
    // aborting now cannot race the park write.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeParked { .. }) => break,
                Some(_) => continue,
                None => panic!("event bus closed before NodeParked"),
            }
        }
    })
    .await
    .expect("runner A must emit NodeParked for the timer-wait node");

    assert_eq!(
        downstream_invocations.load(Ordering::SeqCst),
        0,
        "downstream must be gated while the timer node is Waiting"
    );

    // Crash: abort the parking runner. The park is already durable.
    task_a.abort();
    let _ = task_a.await;

    // The parked node is Waiting with a future deadline, and the execution is
    // Running (a timer-wait with `wake_at = Some` keeps the frontier alive, so
    // the row is NOT Paused — confirming nothing but a timer scanner can wake it).
    let parked_state = stores.load_state(execution_id).await;
    let timer_node_state = parked_state
        .node_states
        .values()
        .find(|ns| ns.state == nebula_workflow::NodeState::Waiting)
        .expect("timer_node must be in Waiting after the park checkpoint");
    let persisted_deadline = timer_node_state
        .next_attempt_at
        .expect("next_attempt_at must be persisted so a re-drive can re-seed the wait_heap");
    assert!(
        persisted_deadline > chrono::Utc::now(),
        "deadline must be in the future right after the park: deadline={persisted_deadline}"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "a timer-wait node with wake_at must keep the execution Running, not Paused"
    );

    // Sleep until the deadline is overdue AND the abandoned lease has expired.
    tokio::time::sleep(OVERDUE_SLEEP).await;
    assert!(
        persisted_deadline <= chrono::Utc::now(),
        "deadline must be in the past after the overdue sleep: deadline={persisted_deadline}, \
         now={}",
        chrono::Utc::now()
    );

    execution_id
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// **Mechanism:** a fresh re-drive of `resume_execution` fires the overdue timer.
///
/// Proves the underlying wake path is sound: `run_frontier` re-seeds `wait_heap`
/// from the persisted `next_attempt_at` (frontier.rs lines 105-111); because the
/// deadline is overdue, Phase 0b drains it immediately, reads `wait_wake = None`
/// (Completion), transitions `Waiting → Completed`, and activates downstream — all
/// with NO Resume command.
///
/// **Falsifiability**: drop the `wait_heap` re-seed loop → the overdue entry is
/// never pushed → Phase 0b never fires → the wait stays `Waiting`, downstream
/// stays gated → both final assertions fail → RED.
#[tokio::test]
async fn resume_execution_redrive_fires_overdue_timer() {
    let downstream_invocations = Arc::new(AtomicU32::new(0));
    let stores = CrashRecoveryStores::new();
    let execution_id = park_timer_then_crash(&stores, &downstream_invocations).await;

    // Fresh runner B (no in-process RunningEntry) re-drives directly.
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_registry(
                TIMER_DURATION,
                &downstream_invocations,
            )))
            .with_lease_ttl(LEASE_TTL)
            .with_lease_heartbeat_interval(HEARTBEAT),
    );
    let engine_b_handle = Arc::clone(&engine_b);
    let scope_b = nebula_engine::store_seam::single_tenant_scope();
    let recovery_result = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::spawn(async move {
            engine_b_handle
                .resume_execution(&scope_b, execution_id)
                .await
        }),
    )
    .await
    .expect("runner B must settle the recovered timer within 10s")
    .unwrap()
    .unwrap();

    assert_eq!(
        recovery_result.status,
        ExecutionStatus::Completed,
        "re-drive must complete the execution after re-seeding the overdue timer; got {:?}",
        recovery_result.status
    );
    assert_eq!(
        downstream_invocations.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once after the recovered timer fires"
    );
}

/// **Production path:** the durable-timer scanner (`sweep_overdue_timers`) is what
/// actually wakes a crashed timer.
///
/// Nothing else re-drives a crashed pure-timer execution — a re-delivered `Start`
/// no-ops on the `Running` status, and `Resume` recovery only arms *signal* waits.
/// So the scanner is the recovery mechanism: it must DISCOVER the `Running`
/// execution with an overdue `Waiting` timer (whose lease is now free) and re-drive
/// it. This test runs the scanner — NOT `resume_execution` directly — and asserts
/// it found and woke the one stranded execution.
///
/// **Falsifiability**: if the sweep's overdue predicate is wrong (never matches the
/// row) or it does not call `resume_execution`, it returns `0`, downstream stays
/// gated at `0`, and the execution stays `Running` with a `Waiting` node → RED.
#[tokio::test]
async fn durable_timer_scanner_recovers_crashed_timer_without_resume() {
    let downstream_invocations = Arc::new(AtomicU32::new(0));
    let stores = CrashRecoveryStores::new();
    let execution_id = park_timer_then_crash(&stores, &downstream_invocations).await;

    // Fresh runner B runs the SCANNER (the production recovery path), not a direct
    // resume_execution. The scanner lists Running executions, finds the overdue
    // Waiting timer, sees the lease is free (crashed owner), and re-drives.
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_registry(
                TIMER_DURATION,
                &downstream_invocations,
            )))
            .with_lease_ttl(LEASE_TTL)
            .with_lease_heartbeat_interval(HEARTBEAT),
    );
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let redriven = tokio::time::timeout(
        Duration::from_secs(10),
        engine_b.sweep_overdue_timers(&scope),
    )
    .await
    .expect("the scanner must settle within 10s")
    .expect("the scanner sweep must not error");

    assert_eq!(
        redriven, 1,
        "the scanner must discover and re-drive exactly one crashed overdue-timer execution"
    );
    assert_eq!(
        downstream_invocations.load(Ordering::SeqCst),
        1,
        "the scanner must fire the overdue timer so downstream runs exactly once"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "the execution must reach Completed after the scanner wakes the stranded timer"
    );
}
