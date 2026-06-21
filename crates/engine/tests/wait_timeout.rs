//! Integration tests for ADR-0099 **W-S2b** — signal-driven `ActionResult::Wait`
//! conditions parked WITH an explicit `timeout`.
//!
//! A signal wait (`Webhook` / `Approval` / `Execution`) parked with
//! `timeout: Some(dur)` keeps its execution **`Running`** (a live frontier loop
//! sits on the timeout timer in `wait_heap`). Two outcomes:
//!
//! - **Timeout fires first** → the node FAILS with `RuntimeError::WaitTimedOut`,
//!   its outgoing edges route through the failure path (OnError / Skip /
//!   FailFast), and `ExecutionEvent::NodeWaitTimedOut` is emitted.
//! - **Resume arrives first** → it reaches the LIVE loop through the resume
//!   channel (`WorkflowEngine::resume_live`), which self-arms the node for
//!   completion under the loop's own lease; Phase-0b completes it on the
//!   `main` port and the timeout timer is discarded by the state re-check.
//!
//! The persisted `wait_wake = Timeout` discriminator survives a crash so a
//! recovered runner re-seeds the timer and still times the wait out.
//!
//! ## Timing model
//!
//! The engine's `wait_heap` deadlines are **wall-clock** (`chrono::Utc`), not
//! tokio timers, so `tokio::time::pause()`/`advance` cannot drive them. Mirroring
//! the established convention in `wait.rs`, these tests use **real, short**
//! durations with a generous outer `tokio::time::timeout` safety bound — the
//! durations are small (sub-second) and the assertions are on settled state, so
//! there is no wall-clock-flakiness window. Lease TTLs that gate crash recovery
//! likewise use the in-mem store's clamp floor (1s) under real time.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
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
    ActionExecutor, ActionRegistry, ActionRuntime, ControlDispatch, DataPassingPolicy,
    EngineControlDispatch, ExecutionEvent, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome,
    dto::{ExecutionRecord, WorkflowVersionRecord},
    store::{ExecutionStore, WorkflowStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Action stubs ────────────────────────────────────────────────────────────

macro_rules! static_action_impl {
    ($ty:ty, $key:expr, $name:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, "wait_timeout integration test stub")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

/// Parks itself via `WaitCondition::Webhook` WITH an explicit `timeout`.
/// This is the W-S2b signal+timeout case: the execution stays `Running` with
/// the timeout timer on `wait_heap`.
struct WebhookWaitWithTimeout {
    timeout: Duration,
}

static_action_impl!(
    WebhookWaitWithTimeout,
    action_key!("test.wt.webhook_timeout"),
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
                callback_id: "wt-webhook".to_owned(),
            },
            timeout: Some(self.timeout),
            partial_output: None,
        })
    }
}

/// Counts invocations and succeeds. Main-port downstream gate probe.
struct CountingEcho {
    invocations: Arc<AtomicU32>,
}

static_action_impl!(
    CountingEcho,
    action_key!("test.wt.main_echo"),
    "CountingEcho"
);

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

/// Error-port probe under a distinct key.
struct CountingError {
    invocations: Arc<AtomicU32>,
}

static_action_impl!(
    CountingError,
    action_key!("test.wt.error_echo"),
    "CountingError"
);

impl StatelessAction for CountingError {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

/// Parks itself via `WaitCondition::Duration` (a TIMER wait, not a signal
/// wait). It is NOT a signal wait per the engine's `is_signal_wait` filter, so
/// a live-frontier Resume does NOT arm it — it stays parked until its own
/// duration elapses. Used as a "blocker" that keeps the frontier loop ALIVE
/// past another node's stale timeout deadline, so a stale timeout-heap entry
/// actually pops in a live loop (CR#7).
struct DurationWaitBlocker {
    duration: Duration,
}

static_action_impl!(
    DurationWaitBlocker,
    action_key!("test.wt.duration_blocker"),
    "DurationWaitBlocker"
);

impl StatelessAction for DurationWaitBlocker {
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

// ── Shared store bundle ──────────────────────────────────────────────────────

#[derive(Clone)]
struct WtStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl WtStores {
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
        // ExecutionState has borrowed-string fields; round-trip via string the
        // same way the engine's reload does.
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

/// Build a registry with the signal+timeout wait node, a main-port echo, and an
/// error-port echo. The two echo counters are returned for assertions.
fn build_registry(
    timeout: Duration,
    main_count: &Arc<AtomicU32>,
    error_count: &Arc<AtomicU32>,
) -> Arc<ActionRegistry> {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wt.webhook_timeout"),
            "WebhookWaitWithTimeout",
            "wait_timeout stub",
        ),
        WebhookWaitWithTimeout { timeout },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wt.main_echo"),
            "CountingEcho",
            "wait_timeout stub",
        ),
        CountingEcho {
            invocations: Arc::clone(main_count),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wt.error_echo"),
            "CountingError",
            "wait_timeout stub",
        ),
        CountingError {
            invocations: Arc::clone(error_count),
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

/// Build a workflow `wait ──main──> main_node` and (optionally) `wait ──error──>
/// error_node`. `wait` parks on a Webhook signal with a timeout.
fn make_workflow(with_error_port: bool, timeout: Duration) -> WorkflowDefinition {
    let _ = timeout; // timeout is configured on the action instance, not the def.
    let now = chrono::Utc::now();
    let wait = node_key!("wait_node");
    let main_node = node_key!("main_node");
    let error_node = node_key!("error_node");
    let mut nodes = vec![
        NodeDefinition::new(wait.clone(), "WaitNode", "core", "test.wt.webhook_timeout").unwrap(),
        NodeDefinition::new(main_node.clone(), "MainNode", "core", "test.wt.main_echo").unwrap(),
    ];
    let mut connections = vec![Connection::new(wait.clone(), main_node)];
    if with_error_port {
        nodes.push(
            NodeDefinition::new(
                error_node.clone(),
                "ErrorNode",
                "core",
                "test.wt.error_echo",
            )
            .unwrap(),
        );
        connections.push(Connection::new(wait, error_node).with_from_port("error"));
    }
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "wait-timeout-test".into(),
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

/// Build a workflow with TWO parallel signal+timeout wait nodes, each with its
/// own `main` and `error` downstream:
///
/// ```text
/// wait_a ──main──> main_a       wait_b ──main──> main_b
///        └─error─> error_a             └─error─> error_b
/// ```
///
/// Both waits are entry nodes (no upstream), so they dispatch and park
/// concurrently. The four downstreams share the two echo counters
/// (`test.wt.main_echo` / `test.wt.error_echo`): a clean N=2 resume must drive
/// `main_count == 2`, `error_count == 0`. This is the only workflow that
/// exercises the resume self-arm `for node_key in &to_arm` loop, the
/// heap purge/rebuild, and the multi-node checkpoint with `to_arm.len() > 1`.
fn make_two_wait_workflow() -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let wait_a = node_key!("wait_a");
    let wait_b = node_key!("wait_b");
    let main_a = node_key!("main_a");
    let main_b = node_key!("main_b");
    let error_a = node_key!("error_a");
    let error_b = node_key!("error_b");
    let nodes = vec![
        NodeDefinition::new(wait_a.clone(), "WaitA", "core", "test.wt.webhook_timeout").unwrap(),
        NodeDefinition::new(wait_b.clone(), "WaitB", "core", "test.wt.webhook_timeout").unwrap(),
        NodeDefinition::new(main_a.clone(), "MainA", "core", "test.wt.main_echo").unwrap(),
        NodeDefinition::new(main_b.clone(), "MainB", "core", "test.wt.main_echo").unwrap(),
        NodeDefinition::new(error_a.clone(), "ErrorA", "core", "test.wt.error_echo").unwrap(),
        NodeDefinition::new(error_b.clone(), "ErrorB", "core", "test.wt.error_echo").unwrap(),
    ];
    let connections = vec![
        Connection::new(wait_a.clone(), main_a),
        Connection::new(wait_a, error_a).with_from_port("error"),
        Connection::new(wait_b.clone(), main_b),
        Connection::new(wait_b, error_b).with_from_port("error"),
    ];
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "two-wait-timeout-test".into(),
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

/// Build a registry with the standard signal+timeout / echo nodes PLUS a
/// `DurationWaitBlocker` parked for `blocker_for`. The blocker keeps the
/// frontier loop alive past the signal wait's stale timeout deadline (CR#7).
fn build_registry_with_blocker(
    timeout: Duration,
    blocker_for: Duration,
    main_count: &Arc<AtomicU32>,
    error_count: &Arc<AtomicU32>,
) -> Arc<ActionRegistry> {
    let registry = build_registry(timeout, main_count, error_count);
    registry.register_stateless_instance(
        ActionMetadata::new(
            action_key!("test.wt.duration_blocker"),
            "DurationWaitBlocker",
            "wait_timeout stub",
        ),
        DurationWaitBlocker {
            duration: blocker_for,
        },
    );
    registry
}

/// Build a workflow `wait ──main──> main_node` / `wait ──error──> error_node`
/// PLUS an independent `blocker` node that parks on a `Duration` wait. The
/// blocker has no downstream — its sole job is to keep the frontier loop alive
/// (the row stays `Running` on the blocker's timer) while the signal wait's
/// stale timeout entry pops and is purged/re-read (CR#7).
fn make_workflow_with_blocker() -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let wait = node_key!("wait_node");
    let main_node = node_key!("main_node");
    let error_node = node_key!("error_node");
    let blocker = node_key!("blocker_node");
    let nodes = vec![
        NodeDefinition::new(wait.clone(), "WaitNode", "core", "test.wt.webhook_timeout").unwrap(),
        NodeDefinition::new(main_node.clone(), "MainNode", "core", "test.wt.main_echo").unwrap(),
        NodeDefinition::new(
            error_node.clone(),
            "ErrorNode",
            "core",
            "test.wt.error_echo",
        )
        .unwrap(),
        NodeDefinition::new(blocker, "Blocker", "core", "test.wt.duration_blocker").unwrap(),
    ];
    let connections = vec![
        Connection::new(wait.clone(), main_node),
        Connection::new(wait, error_node).with_from_port("error"),
    ];
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "wait-timeout-blocker-test".into(),
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

/// Await `NodeParked` for the wait node from a subscribed event stream. Bounds
/// the wait so a missing park fails fast instead of hanging.
async fn await_parked(events_rx: &mut nebula_eventbus::Subscriber<ExecutionEvent>) -> ExecutionId {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::NodeParked { execution_id, .. }) => break execution_id,
                Some(_) => continue,
                None => panic!("event bus closed before NodeParked"),
            }
        }
    })
    .await
    .expect("engine must emit NodeParked for the signal+timeout wait")
}

/// Await `NodeParked` events until `expected` DISTINCT nodes have parked. Used
/// by the N>1 self-arm test, where a single Resume must arm MULTIPLE parallel
/// signal+timeout waits — every wait node must be `Waiting` before the Resume is
/// delivered, otherwise the single delivery would arm only the nodes parked so
/// far. Keyed on `node_key` so a duplicate `NodeParked` for one node cannot
/// satisfy the count.
async fn await_n_parked(
    events_rx: &mut nebula_eventbus::Subscriber<ExecutionEvent>,
    expected: usize,
) {
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

/// Await the `NodeWaitTimedOut` event from a subscribed stream, returning it so
/// the caller can assert its fields. Bounds the wait so a missing event fails
/// fast instead of hanging.
async fn await_wait_timed_out(
    events_rx: &mut nebula_eventbus::Subscriber<ExecutionEvent>,
) -> ExecutionEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(event @ ExecutionEvent::NodeWaitTimedOut { .. }) => break event,
                Some(_) => continue,
                None => panic!("event bus closed before NodeWaitTimedOut"),
            }
        }
    })
    .await
    .expect("engine must emit NodeWaitTimedOut when a signal+timeout wait elapses")
}

// ── Fault-injecting execution store (P1#1 fenced-out self-arm) ────────────────

/// Wraps an [`InMemoryExecutionStore`] and forces the NEXT `commit` after
/// `fence_next` is armed to report [`TransitionOutcome::FencedOut`] — modelling
/// the live loop losing its lease mid-iteration so the self-arm checkpoint is
/// fenced. The park checkpoint(s) run with `fence_next == false`; the test arms
/// the flag after the park and before the Resume, so only the self-arm commit
/// is fenced. All other methods delegate.
#[derive(Debug)]
struct FenceArmStore {
    inner: Arc<InMemoryExecutionStore>,
    fence_next: AtomicBool,
}

impl FenceArmStore {
    fn new(inner: Arc<InMemoryExecutionStore>) -> Self {
        Self {
            inner,
            fence_next: AtomicBool::new(false),
        }
    }

    /// Arm the fence so the next `commit` reports `FencedOut` instead of
    /// delegating.
    fn arm_fence(&self) {
        self.fence_next.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ExecutionStore for FenceArmStore {
    async fn create(
        &self,
        scope: &Scope,
        id: &str,
        workflow_id: &str,
        initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.inner
            .create(scope, id, workflow_id, initial_state)
            .await
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        self.inner.get(scope, id).await
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        // Fence exactly one commit (the self-arm checkpoint) once armed.
        if self.fence_next.swap(false, Ordering::SeqCst) {
            return Ok(TransitionOutcome::FencedOut);
        }
        self.inner.commit(batch).await
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        self.inner.acquire_lease(scope, id, holder, ttl).await
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        self.inner.renew_lease(scope, id, token, ttl).await
    }

    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: FencingToken,
    ) -> Result<bool, StorageError> {
        self.inner.release_lease(scope, id, token).await
    }

    async fn list_running(&self, scope: &Scope) -> Result<Vec<String>, StorageError> {
        self.inner.list_running(scope).await
    }

    async fn list_running_for_workflow(
        &self,
        scope: &Scope,
        workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        self.inner
            .list_running_for_workflow(scope, workflow_id)
            .await
    }

    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError> {
        self.inner.count(scope, workflow_id).await
    }
}

/// Await the `ResumeDeferred` event from a subscribed stream, returning its
/// `reason`. Bounds the wait so a missing event fails fast.
async fn await_resume_deferred(
    events_rx: &mut nebula_eventbus::Subscriber<ExecutionEvent>,
) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Some(ExecutionEvent::ResumeDeferred { reason, .. }) => break reason,
                Some(_) => continue,
                None => panic!("event bus closed before ResumeDeferred"),
            }
        }
    })
    .await
    .expect("engine must emit ResumeDeferred when a live-frontier Resume does not durably arm")
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// **W-S2b — a signal wait with timeout fires the error port on timeout.**
///
/// A `wait ──main──> main_node` / `wait ──error──> error_node` workflow where
/// `wait` parks on a Webhook signal with a short timeout. No Resume arrives; the
/// timeout fires. The wait node must FAIL (`WaitTimedOut`), the `error` branch
/// must run, and the `main` branch must NOT.
///
/// **Falsifiability**: revert the Phase-0b `Timeout` branch to the unconditional
/// `Waiting → Completed` completion → the wait completes on the main port →
/// `main_count == 1`, `error_count == 0` → both asserts flip → RED.
#[tokio::test]
async fn signal_wait_with_timeout_fires_error_port_on_timeout() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_millis(120);
    let registry = build_registry(timeout, &main_count, &error_count);

    // Subscribe to the event stream so the `NodeWaitTimedOut` contract is
    // asserted, not just the downstream branch counters (CR#9): the engine
    // could route the error port correctly yet regress the typed event.
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine = Arc::new(make_engine(registry).with_event_bus(event_bus));
    let wf = Arc::new(make_workflow(/* with_error_port */ true, timeout));

    let engine_h = Arc::clone(&engine);
    let wf_h = Arc::clone(&wf);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::spawn(async move {
            engine_h
                .execute_workflow(
                    &nebula_engine::store_seam::single_tenant_scope(),
                    &wf_h,
                    serde_json::json!(null),
                    nebula_execution::context::ExecutionBudget::default(),
                )
                .await
        }),
    )
    .await
    .expect("execution must settle after the timeout fires")
    .unwrap()
    .unwrap();

    // The error branch ran; the main branch never did.
    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "the error-port branch must run on a wait timeout"
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        0,
        "the main-port branch must NOT run on a wait timeout"
    );
    // The node error references the wait timeout (the error-port branch handled
    // the failure, so the execution itself completes).
    assert!(
        result
            .node_errors
            .values()
            .any(|e| e.contains("timed out") || e.contains("WAIT_TIMED_OUT")),
        "node_errors must reference the wait timeout; got: {:?}",
        result.node_errors
    );

    // The typed `NodeWaitTimedOut` event must fire with the signal
    // discriminator and a timeout reconstructed from the persisted deadline.
    // These tests run under real wall-clock timers (not `tokio::time::pause`),
    // so `timeout_ms` is reconstructed from `deadline - started_at` and may
    // slightly exceed the declared timeout — assert `>=`.
    let timed_out = await_wait_timed_out(&mut events_rx).await;
    let ExecutionEvent::NodeWaitTimedOut {
        condition_kind,
        timeout_ms,
        ..
    } = timed_out
    else {
        unreachable!("await_wait_timed_out only returns NodeWaitTimedOut")
    };
    assert_eq!(
        condition_kind, "signal",
        "the timed-out wait must report the signal condition kind"
    );
    assert!(
        timeout_ms >= timeout.as_millis() as u64,
        "timeout_ms ({timeout_ms}) must be at least the declared timeout ({}ms)",
        timeout.as_millis()
    );
}

/// **W-S2b — Resume before the timeout completes the main port and the timer is
/// discarded.**
///
/// The live frontier (Running) receives a Resume through the resume channel
/// before the (long) timeout fires. The node completes on the `main` port; the
/// timeout never fires.
///
/// **Falsifiability**: make the live-resume self-arm stamp `Timeout` instead of
/// `Completion` (or never re-read state at the pop) → the node would fail
/// instead of completing → `main_count == 0` and status `Failed` → the
/// `Completed` + `main_count == 1` asserts flip → RED.
#[tokio::test]
async fn resume_before_timeout_completes_main_port_and_cancels_timer() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    // Long timeout: it must NOT fire during the test — only the Resume completes.
    let timeout = Duration::from_hours(1);
    let registry = build_registry(timeout, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    let wf = make_workflow(/* with_error_port */ true, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    // Spawn the drive (resume_execution drives a Created/Paused execution).
    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });

    await_parked(&mut events_rx).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "a signal+timeout wait must keep the execution Running (live timer), not Paused"
    );

    // Resume well before the 1h timeout. dispatch_resume reads `Running` and
    // delivers to the live resume channel.
    dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("dispatch_resume on a live Running execution must succeed");

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("execution must complete after the live Resume")
        .unwrap()
        .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "a Resume before the timeout must complete the execution, got {:?}",
        result.status
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        1,
        "the main-port branch must run exactly once on a pre-timeout Resume"
    );
    assert_eq!(
        error_count.load(Ordering::SeqCst),
        0,
        "the error-port branch must NOT run on a pre-timeout Resume (no late timeout failure)"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed (and the timeout discarded)"
    );
}

/// **W-S2b — a Resume to a Running execution reaches the live loop.**
///
/// `dispatch_resume` on a `Running` execution must deliver to the live resume
/// channel (NOT no-op as the pre-W-S2b code did). With a long timeout, only the
/// live delivery can complete the node before the test bound.
///
/// **Falsifiability**: restore `dispatch_resume`'s `Running => Ok(())` no-op arm
/// (drop the `resume_live` call) → the live loop never wakes → the node only
/// times out at t=1h → the spawned drive never settles within 5s → the final
/// `expect` fails → RED.
#[tokio::test]
async fn resume_to_running_execution_reaches_live_loop() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1);
    let registry = build_registry(timeout, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    let wf = make_workflow(/* with_error_port */ false, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });

    await_parked(&mut events_rx).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running
    );

    dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("dispatch_resume to a Running execution must be Ok (delivered to live loop)");

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect(
            "the live loop must complete the node on Resume — without live delivery it would only \
             time out at t=1h and this would elapse",
        )
        .unwrap()
        .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "the live Resume must drive the execution to Completed without waiting for the timeout"
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        1,
        "the main branch must run exactly once on the live Resume"
    );
}

/// **W-S2b — crash mid-wait with a timeout recovers and can still time out.**
///
/// Runner A parks the signal+timeout wait, then "crashes" (its drive task is
/// aborted). After the lease TTL expires, runner B resumes; the persisted
/// `wait_wake = Timeout` re-seeds the timer, and because the (short) deadline
/// has already passed in real time, runner B times the wait out.
///
/// **Falsifiability**: drop the `wait_wake` field (or its serde persistence) →
/// after recovery the re-seeded wait reads `wait_wake = None` → Phase-0b
/// COMPLETES it instead of failing → `error_count == 0`, `main_count == 1` →
/// the timeout asserts flip → RED.
#[tokio::test]
async fn crash_mid_wait_with_timeout_recovers_and_can_still_timeout() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    // A timeout short enough that the deadline has elapsed by the time runner B
    // recovers (after the ~1s lease TTL floor), but long enough that it cannot
    // fire in the brief window before we abort runner A — 200ms was fragile
    // under CI load (CR#6). 400ms is comfortably below the lease-expiry wait.
    let timeout = Duration::from_millis(400);
    // In-mem store clamps lease TTL to >= 1s.
    let lease_ttl = Duration::from_secs(1);
    let heartbeat = Duration::from_millis(300);

    let stores = WtStores::new();

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(
                make_engine(build_registry(timeout, &main_count, &error_count))
                    .with_event_bus(event_bus),
            )
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_registry(
                timeout,
                &main_count,
                &error_count,
            )))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );

    let wf = make_workflow(/* with_error_port */ true, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    // Runner A parks the wait. Capture the park, then crash A immediately so its
    // own short timer does not fire before we abort.
    let engine_a_h = Arc::clone(&engine_a);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task_a =
        tokio::spawn(async move { engine_a_h.resume_execution(&scope, execution_id).await });
    await_parked(&mut events_rx).await;

    // Crash runner A IMMEDIATELY after the park event, before its timer can
    // fire (CR#6): the park checkpoint already landed before `NodeParked` was
    // emitted, so the durable row is intact and the abort cannot race the
    // timer. The persisted-state assertion below reads that durable row.
    task_a.abort();
    let _ = task_a.await;

    // Confirm the persisted discriminator survived the park (read from the
    // durable row, unaffected by the abort).
    let parked = stores.load_state(execution_id).await;
    assert!(
        parked
            .node_states
            .values()
            .any(|ns| ns.state == nebula_workflow::NodeState::Waiting
                && ns.wait_wake == Some(nebula_execution::state::WaitWake::Timeout)),
        "the parked node must persist wait_wake = Timeout"
    );

    // Wait out the lease TTL (real time) so runner B can take over. By now the
    // wait deadline has long passed.
    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;

    // Runner B resumes: re-seeds the persisted Timeout wait, sees the deadline
    // already past, and fails the node on the error branch.
    let engine_b_h = Arc::clone(&engine_b);
    let scope_b = nebula_engine::store_seam::single_tenant_scope();
    let result_b = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::spawn(async move { engine_b_h.resume_execution(&scope_b, execution_id).await }),
    )
    .await
    .expect("runner B must settle the recovered timeout within 10s")
    .unwrap()
    .unwrap();

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "the recovered wait must still time out on runner B and route the error branch"
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        0,
        "the main branch must NOT run on a recovered timeout"
    );
    let _ = result_b;
}

/// **W-S2b — crash after a durable arm completes on recovery.**
///
/// A signal+timeout wait is parked, then armed for COMPLETION (a Resume) in the
/// durable row under the lease. Simulate a crash AFTER the arm is durable but
/// before Phase-0b drains it: runner B recovers, re-seeds the armed timer
/// (`wait_wake = Completion`), and completes the node on the main port.
///
/// **Falsifiability**: make Phase-0b's recovery path read `wait_wake = Timeout`
/// for an armed `Completion` wait (e.g. ignore the discriminator) → recovery
/// would FAIL the node → `main_count == 0` and status `Failed` → the
/// `Completed` + `main_count == 1` asserts flip → RED.
#[tokio::test]
async fn crash_after_durable_arm_completes_on_recovery() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    // Long timeout: recovery must complete via the COMPLETION arm, never via the
    // timeout (which would not have fired yet).
    let timeout = Duration::from_hours(1);
    let lease_ttl = Duration::from_secs(1);
    let heartbeat = Duration::from_millis(300);
    let stores = WtStores::new();

    let wf = make_workflow(/* with_error_port */ true, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    // Runner A parks the signal+timeout wait (Running), then we crash it.
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine_a = Arc::new(
        stores
            .attach(
                make_engine(build_registry(timeout, &main_count, &error_count))
                    .with_event_bus(event_bus),
            )
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let engine_a_h = Arc::clone(&engine_a);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_a_h.resume_execution(&scope, execution_id).await });
    await_parked(&mut events_rx).await;
    task.abort();
    let _ = task.await;

    // Expire A's lease, then manually arm the wait for COMPLETION in the durable
    // row — models the live loop's self-arm checkpoint having landed before the
    // crash (armed-but-not-yet-drained).
    tokio::time::sleep(lease_ttl + Duration::from_millis(300)).await;
    arm_wait_for_completion(&stores, execution_id).await;

    // Recover with a fresh runner B. The re-seed picks up the armed timer; since
    // its `wait_wake = Completion`, Phase-0b COMPLETES the node (does not fail).
    let engine_b = Arc::new(
        stores
            .attach(make_engine(build_registry(
                timeout,
                &main_count,
                &error_count,
            )))
            .with_lease_ttl(lease_ttl)
            .with_lease_heartbeat_interval(heartbeat),
    );
    let engine_b_h = Arc::clone(&engine_b);
    let scope_b = nebula_engine::store_seam::single_tenant_scope();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::spawn(async move { engine_b_h.resume_execution(&scope_b, execution_id).await }),
    )
    .await
    .expect("recovery must settle within 10s")
    .unwrap()
    .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "an armed (Completion) wait must complete on recovery, not fail, got {:?}",
        result.status
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        1,
        "the main branch must run exactly once after recovery completes the armed wait"
    );
    assert_eq!(
        error_count.load(Ordering::SeqCst),
        0,
        "the error branch must NOT run when the armed wait completes"
    );
}

/// Acquire the lease and rewrite the parked wait node to an armed Completion
/// state (`next_attempt_at = now`, `wait_wake = Completion`). Models the live
/// loop's self-arm checkpoint having landed before a crash.
async fn arm_wait_for_completion(stores: &WtStores, execution_id: ExecutionId) {
    use nebula_storage_port::{TransitionBatch, TransitionOutcome};
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let id = execution_id.to_string();
    let token = stores
        .execution
        .acquire_lease(&scope, &id, "test-armer", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("abandoned lease must be free after TTL");
    let record = stores
        .execution
        .get(&scope, &id)
        .await
        .unwrap()
        .expect("row exists");
    let s = serde_json::to_string(&record.state).unwrap();
    let mut state: ExecutionState = serde_json::from_str(&s).unwrap();
    let now = chrono::Utc::now();
    for ns in state.node_states.values_mut() {
        if ns.state == nebula_workflow::NodeState::Waiting {
            ns.next_attempt_at = Some(now);
            ns.wait_wake = Some(nebula_execution::state::WaitWake::Completion);
        }
    }
    state.version += 1;
    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(&id)
        .expected_version(record.version)
        .fencing(token)
        .new_state(serde_json::to_value(&state).unwrap())
        .build()
        .unwrap();
    assert!(matches!(
        stores.execution.commit(batch).await.unwrap(),
        TransitionOutcome::Applied { .. }
    ));
    stores
        .execution
        .release_lease(&scope, &id, token)
        .await
        .unwrap();
}

/// **W-S2b — Resume wins, then the stale timeout timer fires IN A LIVE LOOP:
/// still exactly one terminal outcome.**
///
/// The load-bearing R2 race invariant, made deterministic by ordering AND by
/// keeping the frontier loop alive while the stale entry pops (CR#7). An
/// independent `blocker` node parks on a long `Duration` wait, so the row stays
/// `Running` and the loop keeps polling `wait_heap` long after the signal
/// wait's short timeout deadline. The signal wait's stale `(deadline, key)`
/// heap entry therefore pops in a LIVE loop; the purge (it should already be
/// gone) and the Phase-0b state re-read (a non-`Waiting` node is skipped) must
/// keep the error branch from EVER running. We assert no error routing fired
/// while the drive is STILL alive, then let the blocker elapse so the drive
/// settles.
///
/// Without the blocker the drive returns the instant the single wait completes,
/// so the stale entry never pops in a live loop — the test could pass even if
/// the purge/re-read were broken. The blocker makes the guard genuinely
/// exercised.
///
/// This is the `Resume-then-timeout` ordering of the race; the
/// `timeout-then-Resume` ordering is covered by
/// [`resume_before_timeout_completes_main_port_and_cancels_timer`] (Resume wins)
/// and [`signal_wait_with_timeout_fires_error_port_on_timeout`] (timeout wins),
/// proving both orderings reach a single outcome.
///
/// **Falsifiability**: TWO independent guards keep the stale timeout from
/// re-routing — the resume self-arm PURGES the stale future heap entry, and the
/// Phase-0b pop RE-READS node state (a non-`Waiting` node is skipped). To turn
/// it RED, drop BOTH guards (e.g. the `wait_heap` rebuild that filters out the
/// re-armed keys AND the pop-time state re-read) → the stale `(deadline, key)`
/// entry survives, pops while the loop is alive on the blocker, finds the node
/// `Completed`, and routes it through `route_failure_edges` → the error branch
/// runs → the `error_count == 0` assert fails → RED.
#[tokio::test]
async fn resume_and_timeout_race_reaches_single_terminal_outcome() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    // A short signal timeout: after the Resume completes the node, its STALE
    // timeout heap entry fires while the blocker keeps the loop alive. The R2
    // purge + state re-read must turn that stale wake into a no-op.
    let timeout = Duration::from_millis(120);
    // The blocker outlives the stale-timeout pop window (well past 120ms) so the
    // loop is genuinely alive when the stale entry fires, then elapses so the
    // execution can settle within the test bound.
    let blocker_for = Duration::from_millis(900);
    let registry = build_registry_with_blocker(timeout, blocker_for, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    let wf = make_workflow_with_blocker();
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });

    // Both the signal wait and the Duration blocker must be parked before the
    // Resume — the blocker is a timer wait (not a signal wait), so the Resume
    // arms only the signal wait, leaving the blocker holding the loop open.
    await_n_parked(&mut events_rx, 2).await;

    // Resume wins: it self-arms the signal wait for completion and Phase-0b
    // completes it on the main port. The signal wait's original timeout entry
    // remains on the heap until purged / re-read.
    dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("dispatch_resume must deliver to the live loop");

    // Sleep past the signal wait's stale deadline WHILE THE LOOP IS STILL ALIVE
    // (the blocker is still parked). The stale entry pops in the live loop here.
    tokio::time::sleep(timeout + Duration::from_millis(80)).await;

    // The drive must still be running (the blocker keeps it alive) — this is
    // what makes the stale pop happen in a live loop, not after teardown.
    assert!(
        !task.is_finished(),
        "the blocker must keep the frontier loop alive past the stale timeout deadline"
    );
    // The stale timeout did NOT re-route the error branch: the main completion
    // ran exactly once and the error branch never did.
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        1,
        "the Resume completion must route the MAIN branch exactly once"
    );
    assert_eq!(
        error_count.load(Ordering::SeqCst),
        0,
        "the stale timeout must NOT re-route the error branch in the live loop"
    );

    // Release the loop: let the blocker's Duration elapse so the execution
    // settles. (No external signal — the blocker completes on its own timer.)
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("execution must settle once the blocker's Duration elapses")
        .unwrap()
        .unwrap();

    // Final state: exactly one branch ran across the whole run, and the
    // execution reached a single coherent terminal outcome.
    let main_ran = main_count.load(Ordering::SeqCst);
    let error_ran = error_count.load(Ordering::SeqCst);
    assert_eq!(
        main_ran + error_ran,
        1,
        "exactly one branch must run on a Resume-then-stale-timeout sequence; \
         got main={main_ran}, error={error_ran}"
    );
    assert_eq!(
        main_ran, 1,
        "the surviving branch must be the main completion"
    );
    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "the execution must reach a single coherent terminal outcome (Completed), got {:?}",
        result.status
    );
}

/// **W-S2b — a wait timeout does NOT count against the retry budget.**
///
/// A `WaitTimedOut` is terminal and bypasses the retry decision entirely. The
/// timed-out node must NOT enter `WaitingRetry`, and `total_retries` must NOT
/// bump.
///
/// **Falsifiability**: route the timeout through the retry path
/// (`schedule_node_retry` / `compute_retry_decision`) instead of straight to
/// `Failed` → the node would land in `WaitingRetry` and `total_retries` would
/// bump → the `WaitingRetry`-absent / `total_retries == 0` asserts fail → RED.
#[tokio::test]
async fn timeout_does_not_count_against_retry_budget() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_millis(120);
    let registry = build_registry(timeout, &main_count, &error_count);

    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry)));

    // No error port: a top-level FailFast timeout fails the execution.
    let wf = make_workflow(/* with_error_port */ false, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await }),
    )
    .await
    .expect("execution must settle after the timeout")
    .unwrap()
    .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "an unhandled wait timeout must fail the execution"
    );

    let state = stores.load_state(execution_id).await;
    assert_eq!(
        state.total_retries, 0,
        "a wait timeout must not bump total_retries (bypasses the retry budget)"
    );
    assert!(
        state
            .node_states
            .values()
            .all(|ns| ns.state != nebula_workflow::NodeState::WaitingRetry),
        "no node may be in WaitingRetry after a wait timeout"
    );
    // The timed-out wait node ends Failed.
    assert!(
        state
            .node_states
            .values()
            .any(|ns| ns.state == nebula_workflow::NodeState::Failed),
        "the timed-out wait node must be Failed; states: {:?}",
        state
            .node_states
            .values()
            .map(|ns| ns.state)
            .collect::<Vec<_>>()
    );
}

/// **W-S2b — only `resume_live` satisfies a live signal+timeout wait; a
/// re-delivered `dispatch_start` / `dispatch_restart` does NOT (SECURITY test).**
///
/// A `Running` signal+timeout execution must be satisfied ONLY by the live
/// resume channel (`dispatch_resume` → `resume_live`). The other control
/// commands short-circuit on `Running` to an `Ok(())` no-op — they must NEVER
/// reach into the live loop and complete the parked wait. (Mirrors the case-a
/// `dispatch_start_redelivery_does_not_satisfy_signal_wait` security test for
/// the Running/timeout shape.)
///
/// **Falsifiability**: wire `resume_live` (or a satisfy) into `dispatch_start` /
/// `dispatch_restart`'s `Running` arm → the re-delivered Start/Restart completes
/// the wait → the main branch runs → `main_count == 1` after the re-drive → the
/// `== 0` assert before the genuine Resume fails → RED.
#[tokio::test]
async fn redelivered_start_restart_do_not_satisfy_live_signal_timeout_wait() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1); // long: must not fire during the test
    let registry = build_registry(timeout, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    let wf = make_workflow(/* with_error_port */ false, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });
    await_parked(&mut events_rx).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "the signal+timeout wait must keep the execution Running"
    );

    // A re-delivered Start on a Running execution must be a no-op (it must NOT
    // satisfy the wait). `dispatch_start` short-circuits `Running => Ok(())`.
    dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("re-delivered Start on a Running execution must be an Ok no-op");
    // A re-delivered Restart likewise short-circuits `Running => Ok(())`.
    dispatch
        .dispatch_restart(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("re-delivered Restart on a Running execution must be an Ok no-op");

    // Neither re-drive may have completed the wait — give the runtime a tick to
    // surface any erroneous wake, then assert the gate is still closed.
    tokio::task::yield_now().await;
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        0,
        "neither Start nor Restart re-delivery may satisfy a live signal+timeout wait — \
         only resume_live does"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "the execution must still be Running (wait not satisfied) after Start/Restart re-drives"
    );

    // Wind the live loop down deterministically (the 1h timer would otherwise
    // keep the drive alive). Cancel reaches the live frontier and tears it down.
    assert!(
        engine.cancel_execution(execution_id),
        "cancel_execution must find the live frontier"
    );
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the cancelled execution must wind down within 5s")
        .unwrap()
        .unwrap();
    assert_eq!(
        result.status,
        ExecutionStatus::Cancelled,
        "the execution must end Cancelled after cancel teardown"
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        0,
        "the main branch must never run — the wait was cancelled, never satisfied"
    );
}

/// **W-S2b — ONE Resume arms MULTIPLE parallel signal+timeout waits (N>1
/// self-arm).**
///
/// Two parallel signal+timeout wait nodes both park (the row stays `Running`).
/// A single `dispatch_resume` reaches the live loop, whose `ResumeSignalled`
/// arm self-arms BOTH `Waiting` nodes in one pass (`to_arm.len() == 2`),
/// rebuilds the `wait_heap` purging the stale timeout entries, and commits the
/// multi-node arm with a single checkpoint. Phase-0b then completes both nodes
/// on their `main` ports. Every other test drives a single wait, so this is the
/// only coverage of the `for node_key in &to_arm` loop, the heap purge, and the
/// `to_arm[0]`-attributed multi-node checkpoint with len > 1.
///
/// **Falsifiability**: make the self-arm loop arm only `to_arm[0]` (e.g.
/// `for node_key in to_arm.iter().take(1)`) → the second wait is never armed,
/// its 1h timeout never fires within the bound → the spawned drive never settles
/// → the outer `expect` elapses → RED. (Captured as the RED evidence for this
/// change.)
#[tokio::test]
async fn one_resume_arms_multiple_parallel_signal_timeout_waits() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    // Long timeout: neither wait may time out during the test — only the single
    // Resume completes both. If the second node is not armed it would only fire
    // at t=1h, past the 5s settle bound.
    let timeout = Duration::from_hours(1);
    let registry = build_registry(timeout, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    let wf = make_two_wait_workflow();
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });

    // Both waits must be parked before the single Resume — otherwise the lone
    // `notify_one()` would arm only the node(s) parked so far.
    await_n_parked(&mut events_rx, 2).await;
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Running,
        "two signal+timeout waits must keep the execution Running (live timers)"
    );

    // A single Resume reaches the live loop and arms BOTH parked waits in one
    // `ResumeSignalled` pass.
    dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("dispatch_resume on the live Running execution must succeed");

    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect(
            "both waits must complete on the single Resume — if only the first is armed the second \
             only times out at t=1h and this bound elapses",
        )
        .unwrap()
        .unwrap();

    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "one Resume must complete BOTH parallel waits, got {:?}",
        result.status
    );
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        2,
        "both waits must complete on their MAIN ports exactly once each"
    );
    assert_eq!(
        error_count.load(Ordering::SeqCst),
        0,
        "no wait may route its error port on a clean Resume"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "the execution must reach a terminal Completed status"
    );
}

// ── P1#1 ack-gating: dispatch_resume acks only after the durable self-arm ─────

/// **W-S2b P1#1 — `dispatch_resume` acks ONLY after the self-arm checkpoint
/// durably lands.**
///
/// When `dispatch_resume` returns `Ok` for a live `Running` execution, the
/// self-arm MUST already be durable: a re-read of the persisted row shows the
/// wait node armed (`wait_wake = Completion`, `next_attempt_at = Some`). The ack
/// is gated on the loop's durable reply, so the arm cannot still be in flight.
///
/// **Falsifiability**: revert to the old notify-then-`Ok` path (ack at notify,
/// before the loop wakes and checkpoints) → `dispatch_resume` returns `Ok`
/// before the arm is durable → the immediate re-read finds the node still
/// `wait_wake = Timeout` (unarmed) → the assertion flips → RED.
#[tokio::test]
async fn resume_acks_only_after_successful_checkpoint() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    // Long timeout: the only way the node arms is the Resume, never a timeout.
    let timeout = Duration::from_hours(1);
    let registry = build_registry(timeout, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone());

    let wf = make_workflow(/* with_error_port */ true, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });
    await_parked(&mut events_rx).await;

    dispatch
        .dispatch_resume(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
            None,
        )
        .await
        .expect("dispatch_resume on a live Running execution must succeed");

    // The ack returned — therefore the self-arm checkpoint is DURABLE. Re-read
    // the persisted row: NO node may still be the UNARMED signal+timeout wait
    // (`Waiting` with `wait_wake = Timeout`). The arm flipped it to `Completion`
    // before the ack, and Phase-0b may have already drained it to `Completed`;
    // both prove the arm was durable when dispatch_resume returned. The old
    // notify-then-Ok path acked before the loop woke, leaving the node still
    // `Waiting` / `Timeout` here.
    let state = stores.load_state(execution_id).await;
    let still_unarmed = state.node_states.values().any(|ns| {
        ns.state == nebula_workflow::NodeState::Waiting
            && ns.wait_wake == Some(nebula_execution::state::WaitWake::Timeout)
    });
    assert!(
        !still_unarmed,
        "dispatch_resume returned Ok, so no node may still be the unarmed \
         (Waiting, Timeout) wait — the self-arm must be durable; node_states: {:?}",
        state
            .node_states
            .values()
            .map(|ns| (ns.state, ns.wait_wake))
            .collect::<Vec<_>>()
    );
    // The wait node must be armed-for-completion or already completed — never
    // failed (a timeout) and never still unarmed.
    let arm_durable = state.node_states.values().any(|ns| {
        (ns.state == nebula_workflow::NodeState::Waiting
            && ns.wait_wake == Some(nebula_execution::state::WaitWake::Completion))
            || ns.state == nebula_workflow::NodeState::Completed
    });
    assert!(
        arm_durable,
        "the wait node must be armed (Waiting/Completion) or already Completed; node_states: {:?}",
        state
            .node_states
            .values()
            .map(|ns| (ns.state, ns.wait_wake))
            .collect::<Vec<_>>()
    );

    // Let the armed wait drain to completion so the execution settles.
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the armed wait must complete the execution")
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(main_count.load(Ordering::SeqCst), 1);
    assert_eq!(error_count.load(Ordering::SeqCst), 0);
}

/// **W-S2b P1#1 — a fenced-out self-arm checkpoint defers (does NOT ack).**
///
/// When the live loop's self-arm checkpoint is FENCED (the loop lost its lease
/// mid-iteration), the loop replies `ArmFailed`; `dispatch_resume` must return
/// `Deferred` (NOT `Ok`), emit a `ResumeDeferred` event, and the wait node must
/// NOT complete on the main port — the arm did not durably land, so the Resume
/// is left for B1 reclaim.
///
/// **Falsifiability**: revert to the code that acked regardless of the
/// checkpoint result → `dispatch_resume` returns `Ok` despite the fenced arm →
/// the `Deferred` assertion flips → RED.
#[tokio::test]
async fn fenced_out_self_arm_sends_arm_failed_then_deferred() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1);
    let registry = build_registry(timeout, &main_count, &error_count);

    // Build stores with a fence-injecting execution store. The journal reads the
    // SAME inner store, so status reads agree across the wrapper.
    let inner = Arc::new(InMemoryExecutionStore::new());
    let fenced = Arc::new(FenceArmStore::new(Arc::clone(&inner)));
    let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&inner));
    let versions = Arc::new(InMemoryWorkflowVersionStore::new());
    let workflow = Arc::new(nebula_storage::InMemoryWorkflowStore::new_with_versions(
        &versions,
    ));

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let engine = Arc::new(
        make_engine(registry)
            .with_event_bus(event_bus)
            .with_execution_stores(nebula_engine::ExecutionStores {
                execution: Arc::clone(&fenced) as Arc<dyn ExecutionStore>,
                journal,
                node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
                checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
                idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
                resume_tokens: Arc::new(inner.resume_token_store()),
            })
            .with_workflow_stores(nebula_engine::WorkflowStores {
                workflow: workflow as Arc<dyn WorkflowStore>,
                versions: Arc::clone(&versions) as Arc<dyn WorkflowVersionStore>,
            }),
    );
    let dispatch = EngineControlDispatch::new(
        Arc::clone(&engine),
        Arc::clone(&fenced) as Arc<dyn ExecutionStore>,
    );

    // Persist + save the workflow through the same scope the engine reads.
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let wf = make_workflow(/* with_error_port */ true, timeout);
    versions
        .create(
            &scope,
            WorkflowVersionRecord {
                workflow_id: wf.id.to_string(),
                number: 0,
                published: true,
                pinned: false,
                definition: serde_json::to_value(&wf).unwrap(),
            },
        )
        .await
        .unwrap();
    let execution_id = ExecutionId::new();
    {
        let mut exec_state = ExecutionState::new(execution_id, wf.id, &[]);
        exec_state.set_workflow_input(serde_json::json!(null));
        fenced
            .create(
                &scope,
                &execution_id.to_string(),
                &wf.id.to_string(),
                serde_json::to_value(&exec_state).unwrap(),
            )
            .await
            .unwrap();
    }

    let engine_h = Arc::clone(&engine);
    let drive_scope = scope.clone();
    let task =
        tokio::spawn(async move { engine_h.resume_execution(&drive_scope, execution_id).await });
    await_parked(&mut events_rx).await;

    // Arm the fence so the NEXT commit (the self-arm checkpoint) reports
    // FencedOut. The park checkpoint already landed before `NodeParked`.
    fenced.arm_fence();

    let dispatch_outcome = dispatch.dispatch_resume(&scope, execution_id, None).await;
    assert!(
        matches!(
            dispatch_outcome,
            Err(nebula_engine::ControlDispatchError::Deferred(_))
        ),
        "a fenced self-arm must defer the Resume, not ack it; got {dispatch_outcome:?}"
    );

    // A ResumeDeferred event is emitted with the fenced reason.
    let reason = await_resume_deferred(&mut events_rx).await;
    assert!(
        reason.contains("self-arm") || reason.contains("checkpoint"),
        "the ResumeDeferred reason must name the failed self-arm; got: {reason}"
    );

    // The fenced arm aborts the frontier (the loop returns). The wait node never
    // completed on the main port.
    let _ = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the fenced frontier must wind down within 5s");
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        0,
        "a fenced self-arm must NOT complete the wait on the main port"
    );
}

/// **W-S2b P1#1 — two concurrent Resumes to the same parked node: one arms it
/// (`Armed`), the other finds no signal-Waiting node and returns `NothingToArm`.**
///
/// Both dispatches reach the live `ResumeSignalled` arm in the frontier loop while
/// the execution is `Running`.  The channel is bounded to 8, so both sends land
/// before the loop consumes them.  The first one the loop processes arms the wait;
/// the second finds `to_arm.is_empty()` → `NothingToArm` → `Ok(())`.  This is the
/// genuine idempotent live-loop no-op path — the `ResumeSignalled` arm, not the
/// terminal short-circuit that fires when the execution has already settled.
///
/// **Falsifiability**: make the live loop re-arm an already-armed/absent wait and
/// route it again → `main_count == 2` → the `== 1` assertion flips → RED.
#[tokio::test]
async fn duplicate_resume_to_armed_node_is_noop() {
    let main_count = Arc::new(AtomicU32::new(0));
    let error_count = Arc::new(AtomicU32::new(0));
    let timeout = Duration::from_hours(1);
    let registry = build_registry(timeout, &main_count, &error_count);

    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut events_rx = event_bus.subscribe();
    let stores = WtStores::new();
    let engine = Arc::new(stores.attach(make_engine(registry).with_event_bus(event_bus)));
    let dispatch = Arc::new(EngineControlDispatch::new(
        Arc::clone(&engine),
        stores.execution.clone(),
    ));

    let wf = make_workflow(/* with_error_port */ true, timeout);
    stores.save_workflow(&wf).await;
    let execution_id = stores.persist_created_execution(wf.id).await;

    let engine_h = Arc::clone(&engine);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let task = tokio::spawn(async move { engine_h.resume_execution(&scope, execution_id).await });
    await_parked(&mut events_rx).await;

    // Issue two Resumes CONCURRENTLY while the execution is live and parked.
    // Both sends land in the channel (capacity 8) before the loop processes
    // either.  The first one the loop picks up arms the wait; the second finds
    // `to_arm.is_empty()` (the node is no longer signal-Waiting) → `NothingToArm`
    // → `Ok(())`.  Both return `Ok`: the first as `Armed`, the second as the
    // idempotent live-loop no-op.
    let dispatch_a = Arc::clone(&dispatch);
    let dispatch_b = Arc::clone(&dispatch);
    let (out_a, out_b) = tokio::join!(
        async move {
            dispatch_a
                .dispatch_resume(
                    &nebula_engine::store_seam::single_tenant_scope(),
                    execution_id,
                    None,
                )
                .await
        },
        async move {
            dispatch_b
                .dispatch_resume(
                    &nebula_engine::store_seam::single_tenant_scope(),
                    execution_id,
                    None,
                )
                .await
        },
    );
    out_a.expect("first concurrent dispatch_resume must succeed");
    out_b.expect("second concurrent dispatch_resume must succeed (NothingToArm)");

    // Now let the frontier run to completion.
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the armed wait must complete the execution")
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(
        main_count.load(Ordering::SeqCst),
        1,
        "two concurrent Resumes must not double-route the downstream — main runs exactly once"
    );
    let _ = events_rx;
}
