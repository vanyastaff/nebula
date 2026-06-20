//! Integration tests for ADR-0099 **W-S3b** — no-live-owner `Running`-resume
//! recovery.
//!
//! A signal wait parked WITH a `timeout` keeps its execution **`Running`** (the
//! parking runner's frontier loop sits on the timeout timer). If that runner
//! crashes, its in-process loop is gone but the durable row stays `Running` with
//! the wait still parked. A `Resume` for such an execution reaches
//! `dispatch_resume`'s `Running` arm → `resume_live` finds NO live `RunningEntry`
//! → `ResumeDelivery::NoLiveEntry`. W-S3b RECOVERS that case via
//! `recover_running_resume` → `WorkflowEngine::satisfy_running_signal_waits`,
//! using the execution lease as the dead-vs-live oracle:
//!
//! - lease free / TTL-expired (the parking runner crashed) ⇒ arm the matching
//!   signal wait(s) under the dead lease and re-drive to completion;
//! - lease still LIVE elsewhere (a real owner is driving) ⇒ `Deferred`, never a
//!   double-drive;
//! - a forged / absent / corrupt id ⇒ ack-drop (no forever-redelivery).
//!
//! ## Security invariant (explicitly tested)
//!
//! Only a genuine `Resume` recovers a no-live-owner wait. A plain crash-recovery
//! re-drive (the worker sink / `dispatch_start` re-entering `resume_execution`)
//! must NOT complete the wait — it re-parks instead. That is the same structural
//! discriminator W-S2 enforces for the `Paused` case, extended to `Running` here.
//!
//! ## Timing model
//!
//! The engine's `wait_heap` deadlines are wall-clock (`chrono::Utc`), not tokio
//! timers, so `tokio::time::pause`/`advance` cannot drive them. Mirroring
//! `wait_timeout.rs`, these tests use real, short durations with a generous
//! outer `tokio::time::timeout` safety bound; lease-TTL crash recovery uses the
//! in-mem store's 1s clamp floor under real time.

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
    ActionExecutor, ActionRegistry, ActionRuntime, ControlDispatch, ControlDispatchError,
    DataPassingPolicy, EngineControlDispatch, ExecutionEvent, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    dto::{ResumeTarget, WorkflowVersionRecord},
    store::{ExecutionStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Action stubs ──────────────────────────────────────────────────────────────

macro_rules! static_action_impl {
    ($ty:ty, $key:expr, $name:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, "wait_recovery integration test stub")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

/// Parks on a `Webhook` signal WITH a timeout, so the execution stays `Running`
/// (live timer) rather than `Paused`. The `callback_id` is fixed per instance so
/// a targeted recovery `ResumeTarget::Webhook` can match it by identity.
struct WebhookWaitWithTimeout {
    callback_id: String,
    timeout: Duration,
}

static_action_impl!(
    WebhookWaitWithTimeout,
    action_key!("test.wr.webhook_timeout"),
    "WebhookWaitWithTimeout"
);

impl StatelessAction for WebhookWaitWithTimeout {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: self.callback_id.clone(),
            },
            timeout: Some(self.timeout),
            partial_output: None,
        })
    }
}

/// A second webhook-wait action key (distinct identity) for the targeted-recovery
/// mixed-wait test.
struct WebhookWaitWithTimeoutB {
    callback_id: String,
    timeout: Duration,
}

static_action_impl!(
    WebhookWaitWithTimeoutB,
    action_key!("test.wr.webhook_timeout_b"),
    "WebhookWaitWithTimeoutB"
);

impl StatelessAction for WebhookWaitWithTimeoutB {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: self.callback_id.clone(),
            },
            timeout: Some(self.timeout),
            partial_output: None,
        })
    }
}

/// Counts invocations and succeeds. The downstream gate probe — tests assert on
/// the exact count to verify Phase-0b edge activation after recovery.
struct CountingEcho {
    invocations: Arc<AtomicU32>,
}

static_action_impl!(CountingEcho, action_key!("test.wr.echo"), "CountingEcho");

impl StatelessAction for CountingEcho {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

/// A second downstream probe under a distinct key (for the sibling node in the
/// mixed-wait test).
struct CountingEchoB {
    invocations: Arc<AtomicU32>,
}

static_action_impl!(
    CountingEchoB,
    action_key!("test.wr.echo_b"),
    "CountingEchoB"
);

impl StatelessAction for CountingEchoB {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// ── Shared store bundle ──────────────────────────────────────────────────────

#[derive(Clone)]
struct RecoveryStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl RecoveryStores {
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
        let s = serde_json::to_string(&record.state).unwrap();
        serde_json::from_str(&s).unwrap()
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
        serde_json::from_value(record.state.get("status").cloned().unwrap()).unwrap()
    }
}

// ── Engine assembly ──────────────────────────────────────────────────────────

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

/// Single signal+timeout wait `wait ──main──> downstream`.
fn build_single_registry(
    callback_id: &str,
    timeout: Duration,
    downstream: &Arc<AtomicU32>,
) -> Arc<ActionRegistry> {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.webhook_timeout"),
            "WebhookWaitWithTimeout",
            "wait_recovery stub",
        ),
        WebhookWaitWithTimeout {
            callback_id: callback_id.to_owned(),
            timeout,
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.echo"),
            "CountingEcho",
            "wait_recovery stub",
        ),
        CountingEcho {
            invocations: Arc::clone(downstream),
        },
    );
    registry
}

fn make_single_workflow() -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let wait = node_key!("wait_node");
    let downstream = node_key!("downstream_node");
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "wait-recovery-single".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(wait.clone(), "WaitNode", "core", "test.wr.webhook_timeout")
                .unwrap(),
            NodeDefinition::new(downstream.clone(), "DownstreamNode", "core", "test.wr.echo")
                .unwrap(),
        ],
        connections: vec![Connection::new(wait, downstream)],
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

/// Await `NodeParked` for `expected` DISTINCT nodes, so all waits are parked
/// before a Resume is delivered.
async fn await_n_parked(
    events_rx: &mut nebula_eventbus::Subscriber<ExecutionEvent>,
    expected: usize,
) {
    use std::collections::HashSet;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut parked: HashSet<nebula_core::NodeKey> = HashSet::new();
        while parked.len() < expected {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeParked { node_key, .. }) => {
                    parked.insert(node_key);
                },
                Some(_) => continue,
                None => panic!("event bus closed before all NodeParked"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("engine must emit {expected} distinct NodeParked events"));
}

/// Park a single signal+timeout wait under "runner A", then crash A (abort its
/// drive task) so NO live `RunningEntry` remains and the lease is left to expire.
/// Returns the parked, owner-less `Running` execution id plus the scope. The
/// caller then delivers a Resume that hits `NoLiveEntry`.
async fn park_then_crash_owner(
    stores: &RecoveryStores,
    engine_a: &Arc<WorkflowEngine>,
    workflow_id: nebula_core::WorkflowId,
    events_rx: &mut nebula_eventbus::Subscriber<ExecutionEvent>,
) -> ExecutionId {
    let execution_id = stores.persist_created_execution(workflow_id).await;
    let engine_a_h = Arc::clone(engine_a);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task_a =
        tokio::spawn(async move { engine_a_h.resume_execution(&scope, execution_id).await });
    await_n_parked(events_rx, 1).await;
    // Crash runner A immediately after the park: the park checkpoint already
    // landed before `NodeParked` fired, so the durable row is intact and the
    // abort drops the `RunningEntry` (no live loop) without racing the timer.
    task_a.abort();
    let _ = task_a.await;
    execution_id
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// **W-S3b — a crashed owner's `Running` Resume recovers via the expired lease.**
///
/// Runner A parks a signal+timeout wait (long timeout), then crashes (its drive
/// task is aborted; no live `RunningEntry`). After the lease TTL expires, a
/// `dispatch_resume` (on an engine with no live entry for this execution) hits
/// `NoLiveEntry` → `recover_running_resume` acquires the dead lease, arms the
/// wait, drives `Waiting → Completed`, runs downstream, and completes.
///
/// **Falsifiability**: revert the `NoLiveEntry` arm to the unconditional
/// `defer_running_resume` → the Resume is never recovered → the execution stays
/// `Running` (or times out at t=1h), downstream never runs → `Completed` +
/// `downstream == 1` asserts fail → RED.
#[tokio::test]
async fn crashed_owner_resume_recovers_via_expired_lease() {
    let downstream = Arc::new(AtomicU32::new(0));
    // Long timeout: recovery must complete via the Resume arm, never via timeout.
    let timeout = Duration::from_hours(1);
    let lease_ttl = Duration::from_secs(1); // in-mem clamp floor
    let heartbeat = Duration::from_millis(300);

    let stores = RecoveryStores::new();
    let wf = make_single_workflow();
    stores.save_workflow(&wf).await;

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(
                make_engine(build_single_registry("cb-recover", timeout, &downstream))
                    .with_event_bus(event_bus),
            )
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );

    let execution_id = park_then_crash_owner(&stores, &engine_a, wf.id, &mut events_rx).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "a signal+timeout wait must keep the crashed execution Running (no live loop)"
    );
    assert_eq!(
        downstream.load(Ordering::SeqCst),
        0,
        "downstream must be gated while parked"
    );

    // Wait out the lease TTL so the recovering runner can acquire the dead lease.
    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;

    // A fresh runner B with NO live entry for this execution receives the Resume.
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_single_registry(
                "cb-recover",
                timeout,
                &downstream,
            )))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let dispatch_b = EngineControlDispatch::new(Arc::clone(&engine_b), stores.execution.clone());
    let scope = nebula_engine::store_seam::single_tenant_scope();

    tokio::time::timeout(
        Duration::from_secs(10),
        dispatch_b.dispatch_resume(&scope, execution_id, None),
    )
    .await
    .expect("recovery must settle within 10s")
    .expect("no-live-owner Resume must recover the crashed Running execution");

    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "the recovered execution must complete after the no-live-owner Resume"
    );
    assert_eq!(
        downstream.load(Ordering::SeqCst),
        1,
        "downstream must run exactly once after recovery arms+drives the wait"
    );
}

/// **W-S3b — a Resume to a still-LIVE owner elsewhere DEFERS, never recovers.**
///
/// The execution lease is held (live, NOT TTL-expired) by another runner.
/// `satisfy_running_signal_waits` returns `Leased`, so `recover_running_resume`
/// returns `Deferred` (B1 reclaim redelivers once the lease frees) — it must NOT
/// arm or double-drive the wait.
///
/// Mechanics: runner A parks the wait and "crashes" (its drive task is aborted)
/// WITHOUT the lease TTL expiring — an aborted `resume_execution` drops its
/// `LeaseGuard` without an explicit release, so the lease stays held until its
/// (long, 30s) TTL. That is exactly a live-owner-elsewhere shape: the row is
/// `Running` with no live entry on the recovering runner, but the lease is not
/// yet acquirable. The Resume then arrives on a runner with no live entry →
/// `NoLiveEntry` → recovery → the still-held lease blocks the acquire →
/// `Leased` → `Deferred`.
///
/// **Falsifiability**: make `satisfy_running_signal_waits` ignore the held lease
/// and arm anyway → the Resume would recover under a live owner (double-drive) →
/// the `Deferred` assertion fails → RED.
#[tokio::test]
async fn live_owner_elsewhere_resume_defers() {
    let downstream = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1);
    let lease_ttl = Duration::from_secs(30); // long: the held lease stays LIVE for the test
    let heartbeat = Duration::from_millis(300);

    let stores = RecoveryStores::new();
    let wf = make_single_workflow();
    stores.save_workflow(&wf).await;

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(
                make_engine(build_single_registry("cb-live", timeout, &downstream))
                    .with_event_bus(event_bus),
            )
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );

    // Park + crash A, but DO NOT wait out the (30s) lease TTL — A's lease stays
    // held, modelling a live owner that has not released the execution.
    let execution_id = park_then_crash_owner(&stores, &engine_a, wf.id, &mut events_rx).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running
    );
    let scope = nebula_engine::store_seam::single_tenant_scope();

    // Runner B (no live entry) receives the Resume → NoLiveEntry → recovery →
    // satisfy_running_signal_waits sees the still-held lease → Leased → Deferred.
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_single_registry(
                "cb-live",
                timeout,
                &downstream,
            )))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let dispatch_b = EngineControlDispatch::new(Arc::clone(&engine_b), stores.execution.clone());

    let result = dispatch_b.dispatch_resume(&scope, execution_id, None).await;
    assert!(
        matches!(result, Err(ControlDispatchError::Deferred(_))),
        "a Resume to a live-owner-held execution must Defer, got {result:?}"
    );

    // The wait must NOT have been armed for completion (no double-drive): a
    // signal+timeout wait stays `Waiting` with `wait_wake == Timeout` (the arm
    // would have flipped it to `Completion`), on its future timeout deadline.
    let state = stores.load_state(execution_id).await;
    assert!(
        state
            .node_states
            .values()
            .any(|ns| ns.state == nebula_workflow::NodeState::Waiting
                && ns.wait_wake == Some(nebula_execution::state::WaitWake::Timeout)),
        "the wait node must remain Waiting with wait_wake == Timeout (un-armed) after a \
         deferred (not recovered) Resume; node_states: {:?}",
        state
            .node_states
            .values()
            .map(|ns| (ns.state, ns.wait_wake))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        downstream.load(Ordering::SeqCst),
        0,
        "downstream must not run while the Resume is deferred behind a live owner"
    );
}

/// **W-S3b — a Resume to an unknown execution ACK-DROPS (no forever-redelivery).**
///
/// A `Resume` for an id that no execution row exists for is moot (a forged /
/// garbage / corrupt callback id). `dispatch_resume` must ack-drop it (`Ok(())`,
/// the control-queue row is consumed) rather than `Deferred` (which would
/// redeliver against the non-existent execution forever) or `Rejected` (which
/// would record a noisy Failed row).
///
/// **Falsifiability**: revert the not-found arm to `Deferred` (or `Rejected`) →
/// the `Ok(())` assertion fails → RED. (`Rejected`/`Internal`/any other `Err`
/// also fails the `is_ok` assertion below.)
#[tokio::test]
async fn not_found_resume_acks_drops() {
    let downstream = Arc::new(AtomicU32::new(0));
    let stores = RecoveryStores::new();
    // A workflow exists, but we deliver a Resume for an execution id that was
    // never created — there is no row for it.
    let engine = Arc::new(stores.attach(make_engine(build_single_registry(
        "cb-absent",
        Duration::from_hours(1),
        &downstream,
    ))));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());
    let scope = nebula_engine::store_seam::single_tenant_scope();

    let unknown = ExecutionId::new();
    let result = dispatch.dispatch_resume(&scope, unknown, None).await;
    assert!(
        result.is_ok(),
        "a Resume to an unknown execution must ack-drop (Ok), not Deferred/Rejected; got {result:?}"
    );
}

/// **W-S3b (Decision-3) — targeted recovery arms ONLY the matching node.**
///
/// A no-live-owner `Running` execution has TWO parallel parked signal+timeout
/// waits of DISTINCT identity (`cb-a` / `cb-b`), each feeding its own downstream.
/// A TARGETED recovery Resume (`ResumeTarget::Webhook { callback_id: "cb-a" }`)
/// must arm ONLY `wait_a` — `downstream_a` runs, `downstream_b` stays gated and
/// `wait_b` stays `Waiting`.
///
/// **Falsifiability**: drop the `resume_target` pass-through in
/// `recover_running_resume` (arm untargeted) → BOTH waits arm → `downstream_b`
/// runs (== 1) → the `downstream_b == 0` assertion fails → RED.
#[tokio::test]
async fn mixed_wait_targeted_recovery_arms_only_match() {
    let downstream_a = Arc::new(AtomicU32::new(0));
    let downstream_b = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1);
    let lease_ttl = Duration::from_secs(1);
    let heartbeat = Duration::from_millis(300);

    // Two distinct webhook waits + two distinct downstreams.
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.webhook_timeout"),
            "WebhookWaitWithTimeout",
            "wait_recovery stub",
        ),
        WebhookWaitWithTimeout {
            callback_id: "cb-a".to_owned(),
            timeout,
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.webhook_timeout_b"),
            "WebhookWaitWithTimeoutB",
            "wait_recovery stub",
        ),
        WebhookWaitWithTimeoutB {
            callback_id: "cb-b".to_owned(),
            timeout,
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.echo"),
            "CountingEcho",
            "wait_recovery stub",
        ),
        CountingEcho {
            invocations: Arc::clone(&downstream_a),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.echo_b"),
            "CountingEchoB",
            "wait_recovery stub",
        ),
        CountingEchoB {
            invocations: Arc::clone(&downstream_b),
        },
    );

    let now = chrono::Utc::now();
    let wait_a = node_key!("wait_a");
    let wait_b = node_key!("wait_b");
    let down_a = node_key!("down_a");
    let down_b = node_key!("down_b");
    let wf = WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "wait-recovery-mixed".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(wait_a.clone(), "WaitA", "core", "test.wr.webhook_timeout")
                .unwrap(),
            NodeDefinition::new(wait_b.clone(), "WaitB", "core", "test.wr.webhook_timeout_b")
                .unwrap(),
            NodeDefinition::new(down_a.clone(), "DownA", "core", "test.wr.echo").unwrap(),
            NodeDefinition::new(down_b.clone(), "DownB", "core", "test.wr.echo_b").unwrap(),
        ],
        connections: vec![
            Connection::new(wait_a, down_a),
            Connection::new(wait_b, down_b),
        ],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };

    let stores = RecoveryStores::new();
    stores.save_workflow(&wf).await;

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(make_engine(registry).with_event_bus(event_bus))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );

    // Park BOTH waits under runner A, then crash A.
    let execution_id = stores.persist_created_execution(wf.id).await;
    let engine_a_h = Arc::clone(&engine_a);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task_a =
        tokio::spawn(async move { engine_a_h.resume_execution(&scope, execution_id).await });
    await_n_parked(&mut events_rx, 2).await;
    task_a.abort();
    let _ = task_a.await;

    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running
    );

    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;

    // Recover with a TARGETED Resume for cb-a only.
    let registry_b = Arc::new(ActionRegistry::new());
    registry_b.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.webhook_timeout"),
            "WebhookWaitWithTimeout",
            "wait_recovery stub",
        ),
        WebhookWaitWithTimeout {
            callback_id: "cb-a".to_owned(),
            timeout,
        },
    );
    registry_b.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.webhook_timeout_b"),
            "WebhookWaitWithTimeoutB",
            "wait_recovery stub",
        ),
        WebhookWaitWithTimeoutB {
            callback_id: "cb-b".to_owned(),
            timeout,
        },
    );
    registry_b.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.echo"),
            "CountingEcho",
            "wait_recovery stub",
        ),
        CountingEcho {
            invocations: Arc::clone(&downstream_a),
        },
    );
    registry_b.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wr.echo_b"),
            "CountingEchoB",
            "wait_recovery stub",
        ),
        CountingEchoB {
            invocations: Arc::clone(&downstream_b),
        },
    );
    let engine_b = Arc::new(
        stores
            .attach(make_engine(registry_b))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let dispatch_b = EngineControlDispatch::new(Arc::clone(&engine_b), stores.execution.clone());

    // The targeted recovery arms cb-a and re-drives. Because cb-b (NOT armed)
    // stays `Waiting` with its live 1h timer, the recovery drive parks cb-b and
    // sits on that timer rather than returning — exactly the correct behavior
    // (runner B is now the live owner of cb-b). So spawn the dispatch and observe
    // the SELECTIVE effect (cb-a completed, cb-b untouched) without waiting for
    // the whole execution to settle.
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let dispatch_task = tokio::spawn(async move {
        dispatch_b
            .dispatch_resume(
                &scope,
                execution_id,
                Some(ResumeTarget::Webhook {
                    callback_id: "cb-a".to_owned(),
                }),
            )
            .await
    });

    // Poll (bounded) until cb-a's downstream has run — the targeted arm landed.
    let recovered = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if downstream_a.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        recovered.is_ok(),
        "the targeted wait (cb-a) must be armed and its downstream must run within 10s"
    );

    assert_eq!(
        downstream_b.load(Ordering::SeqCst),
        0,
        "the SIBLING wait (cb-b) must NOT be armed by a cb-a-targeted recovery — \
         targeting arms only the kind+identity match"
    );

    // cb-b must remain `Waiting` (the targeted recovery left it parked on its
    // own live timer).
    let state = stores.load_state(execution_id).await;
    let waiting_count = state
        .node_states
        .values()
        .filter(|ns| ns.state == nebula_workflow::NodeState::Waiting)
        .count();
    assert_eq!(
        waiting_count, 1,
        "exactly one wait (cb-b) must remain Waiting after the targeted recovery"
    );

    // The recovery drive is still sitting on cb-b's 1h timer; abort it (the test
    // has proven the selective arm).
    dispatch_task.abort();
    let _ = dispatch_task.await;
}

/// **W-S3b (SECURITY) — a crash-recovery re-drive WITHOUT a Resume does NOT
/// satisfy the wait.**
///
/// A crashed-`Running` signal+timeout execution recovered by a plain re-drive
/// (the worker-sink / `dispatch_start` path: `resume_execution` re-entry, which
/// does NOT short-circuit on `Running`) must NOT complete the signal wait — only
/// a genuine `dispatch_resume` arms it. The re-drive re-seeds the timer and
/// re-parks; with a long timeout the wait stays `Waiting` and downstream stays
/// gated.
///
/// **Falsifiability**: wire `satisfy_running_signal_waits` (or any arm-for-
/// completion) into the worker-sink/`dispatch_start` re-drive path → the re-drive
/// auto-completes the wait → downstream runs (== 1) → the `== 0` assertion before
/// the genuine Resume fails → RED.
#[tokio::test]
async fn crash_recovery_redrive_without_resume_does_not_satisfy() {
    let downstream = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1);
    let lease_ttl = Duration::from_secs(1);
    let heartbeat = Duration::from_millis(300);

    let stores = RecoveryStores::new();
    let wf = make_single_workflow();
    stores.save_workflow(&wf).await;

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(
                make_engine(build_single_registry("cb-redrive", timeout, &downstream))
                    .with_event_bus(event_bus),
            )
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );

    let execution_id = park_then_crash_owner(&stores, &engine_a, wf.id, &mut events_rx).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running
    );

    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;

    // Crash-recovery re-drive (NO Resume): the worker sink / dispatch_start path
    // re-enters `resume_execution` directly. It must re-park the signal wait, NOT
    // complete it. We bound it: a healthy re-park returns promptly (it parks on
    // the live timer and exits the synchronous drive once the frontier yields).
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_single_registry(
                "cb-redrive",
                timeout,
                &downstream,
            )))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let engine_b_h = Arc::clone(&engine_b);
    let scope_b = nebula_engine::store_seam::single_tenant_scope();
    let task_b =
        tokio::spawn(async move { engine_b_h.resume_execution(&scope_b, execution_id).await });

    // The re-drive re-parks the signal wait on its (live, 1h) timer and then
    // sits on it — so `task_b` does NOT return. Give the re-drive ample time to
    // run its frontier (it would have auto-completed the wait by now IF the
    // re-drive path armed signal waits), then assert it did NOT.
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(
        downstream.load(Ordering::SeqCst),
        0,
        "a crash-recovery re-drive (no Resume) must NOT complete the signal wait — \
         downstream must stay gated until a genuine Resume arrives"
    );
    let state = stores.load_state(execution_id).await;
    assert!(
        state
            .node_states
            .values()
            .any(|ns| ns.state == nebula_workflow::NodeState::Waiting
                && ns.next_attempt_at.is_some_and(|at| at > chrono::Utc::now())),
        "the wait node must remain Waiting on its FUTURE timeout timer after a crash-recovery \
         re-drive (re-parked, not armed-for-completion / auto-completed)"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "the re-driven execution must stay Running (re-parked), not Completed"
    );

    // The re-drive is still sitting on the 1h timer; abort it (the long timeout
    // never fires within the test).
    task_b.abort();
    let _ = task_b.await;
}
