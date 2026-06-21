//! Engine integration tests for W-S3e — revoke resume tokens on terminal
//! transition (ADR-0099).
//!
//! W-S3c mints a resume token, atomically, in the same `TransitionBatch` that
//! writes a node's `Waiting` snapshot (mint-on-park). W-S3e wires the
//! *cleanup* side: when the parked execution reaches a terminal state, the
//! engine calls [`ResumeTokenStore::revoke_on_terminal`] at both terminal
//! sinks so dead, un-consumed tokens are purged proactively rather than
//! lingering until the `ON DELETE CASCADE` backstop fires.
//!
//! These tests drive a real store-backed engine through the two sinks and
//! assert the *effect* — token rows actually gone after terminal, surviving a
//! non-terminal commit — via a non-destructive count probe
//! (`token_count_for_test`), so the probe never masks the wire under test.
//!
//! ## Sinks under test
//!
//! - **SINK 1** — `persist_final_state_port`: the consolidated live-runner
//!   terminal write (Completed / Failed / Cancelled / TimedOut). Driven here
//!   via `dispatch_start` (park) → `dispatch_resume` (drive to Completed).
//! - **SINK 2** — `cancel_dangling_nodes_under_lease`: the no-live-runner
//!   cancel-of-parked path. Driven here via `dispatch_start` (park) →
//!   `force_status(Cancelled)` → `dispatch_cancel` with no live runner.
//!
//! ## Falsifiability
//!
//! Remove the `revoke_on_terminal` call from a sink → the minted token is not
//! purged on terminal → the post-terminal `token_count_for_test == 0`
//! assertion fails (count stays 1) → RED.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
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
    EngineControlDispatch, InProcessRunner, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{
    InMemoryExecutionStore, InMemoryResumeTokenStore, InMemoryWorkflowVersionStore,
};
use nebula_storage_port::{
    FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome,
    dto::{ExecutionRecord, WorkflowVersionRecord, resume_token::ResumeTokenRow},
    store::{ExecutionStore, ResumeTokenStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── CAS-retry interceptor ─────────────────────────────────────────────────────

/// Wraps an [`ExecutionStore`] and returns [`TransitionOutcome::VersionConflict`]
/// on the **next** `commit` call after [`ArmableConflictStore::arm`] is called,
/// then delegates all subsequent calls to the inner store unchanged.
///
/// The arm/disarm design lets the test control WHICH commit is intercepted.
/// The park commit (inside `dispatch_start`) is NOT intercepted because the
/// test only arms the store AFTER `dispatch_start` returns. The final-state
/// commit (inside `dispatch_resume`'s `persist_final_state_port`) is the
/// next commit after arming, so that one returns a `VersionConflict`.
///
/// The intercepted commit leaves the inner storage row unmodified — the real
/// row keeps its original version, so the engine's reconcile `get` refetch
/// observes the actual row and (when no `get` override is set) the retry
/// commit succeeds.
///
/// # Optional `get` override after intercept
///
/// Call [`ArmableConflictStore::arm_with_terminal_get`] to also override the
/// `get` response that follows the `VersionConflict`. When the override is set,
/// the first `get` call AFTER the interceptor fires returns a row whose
/// `"status"` field is replaced by the supplied snake_case status string
/// (e.g. `"cancelled"`). This drives the engine into the
/// `observed-terminal → honor external` branch (§11.5, #333 line ~6051)
/// rather than the retry branch.
#[derive(Debug)]
struct ArmableConflictStore {
    inner: Arc<InMemoryExecutionStore>,
    /// `true` once armed; the next matching `commit` fires the interception.
    armed: AtomicBool,
    /// `true` once the interception has fired (i.e. armed + commit consumed).
    fired: AtomicBool,
    /// If `Some`, the first `get` after `fired` returns a row with `"status"`
    /// overridden to this value, directing the engine to the honor-external-
    /// terminal branch. `None` = delegate to the inner store (retry branch).
    get_status_override: Mutex<Option<String>>,
    /// `true` once the `get` override has been consumed (single-use).
    get_override_consumed: AtomicBool,
}

impl ArmableConflictStore {
    fn new(inner: Arc<InMemoryExecutionStore>) -> Self {
        Self {
            inner,
            armed: AtomicBool::new(false),
            fired: AtomicBool::new(false),
            get_status_override: Mutex::new(None),
            get_override_consumed: AtomicBool::new(false),
        }
    }

    /// Arm the interceptor: the next terminal-status `commit` returns
    /// `VersionConflict`. The subsequent `get` delegates to the real store
    /// (engine takes the CAS-retry branch).
    fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    /// Arm the interceptor AND configure the post-fire `get` to return a row
    /// with `status` overridden to `terminal_status` (snake_case, e.g.
    /// `"cancelled"`). The engine observes a terminal external state and takes
    /// the honor-external branch (~line 6051) rather than the retry branch.
    fn arm_with_terminal_get(&self, terminal_status: impl Into<String>) {
        *self.get_status_override.lock().unwrap() = Some(terminal_status.into());
        self.armed.store(true, Ordering::Release);
    }

    /// Whether the interception has fired at least once.
    fn has_fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }
}

#[async_trait]
impl ExecutionStore for ArmableConflictStore {
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
        // If the interceptor has fired and a get-status override is configured,
        // apply it on the first `get` call after the fire (single-use).
        // Single-use `get` override: after the interceptor fires, the FIRST get
        // returns the real row with `"status"` patched to the configured terminal
        // value, steering the engine into the honor-external-terminal branch.
        // The `compare_exchange` is the actual single-use gate; the load-before is
        // an optimistic fast-path skip to avoid taking the mutex when not needed.
        let should_override = self.fired.load(Ordering::Acquire)
            && !self.get_override_consumed.load(Ordering::Acquire)
            && self.get_status_override.lock().unwrap().is_some()
            && self
                .get_override_consumed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
        if should_override {
            let status = self
                .get_status_override
                .lock()
                .unwrap()
                .clone()
                .expect("checked is_some above");
            return match self.inner.get(scope, id).await? {
                None => Ok(None),
                Some(mut rec) => {
                    if let Some(obj) = rec.state.as_object_mut() {
                        obj.insert("status".to_owned(), serde_json::Value::String(status));
                    }
                    Ok(Some(rec))
                },
            };
        }
        self.inner.get(scope, id).await
    }

    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        // Intercept only when armed AND the commit carries a terminal new state.
        // This skips intermediate commits (e.g., `satisfy_signal_waits`) and
        // fires exclusively on `persist_final_state_port`'s terminal write.
        // The status field in the batch JSON mirrors `ExecutionStatus::to_string()`.
        if self.armed.load(Ordering::Acquire) {
            let new_status = batch
                .new_state()
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // `ExecutionStatus` serializes with `#[serde(rename_all = "snake_case")]`.
            let is_terminal_state = matches!(
                new_status,
                "completed" | "failed" | "cancelled" | "timed_out"
            );
            if is_terminal_state
                && self
                    .armed
                    .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                self.fired.store(true, Ordering::Release);
                // Return a VersionConflict as if an external writer bumped the
                // version between the engine's read and this commit. The inner
                // store is untouched — the real row keeps its original version so
                // the engine's reconcile `get` refetch will observe it and succeed.
                let fabricated_conflict_version = batch.expected_version().saturating_add(1);
                return Ok(TransitionOutcome::VersionConflict {
                    actual: fabricated_conflict_version,
                });
            }
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

// ── Action stubs ──────────────────────────────────────────────────────────────

/// Parks on a `Webhook` signal so the engine mints a resume token for this node.
struct WebhookParkNode;

impl Action for WebhookParkNode {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.w_s3e.webhook_park"),
            "WebhookParkNode",
            "W-S3e revoke-on-terminal integration test stub",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPS: OnceLock<Dependencies> = OnceLock::new();
        DEPS.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for WebhookParkNode {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "w-s3e-integration-cb".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// A plain echo node used downstream of the parked node so that a satisfied
/// wait drives the execution all the way to `Completed`.
struct EchoNode;

impl Action for EchoNode {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.w_s3e.echo"),
            "EchoNode",
            "W-S3e downstream echo stub",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPS: OnceLock<Dependencies> = OnceLock::new();
        DEPS.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for EchoNode {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

// ── Failing resume-token store (best-effort posture probe) ──────────────────────

/// A `ResumeTokenStore` whose `revoke_on_terminal` always errors, used to prove
/// the terminal transition still succeeds when revoke fails (best-effort).
#[derive(Debug)]
struct FailingRevokeStore;

#[async_trait]
impl ResumeTokenStore for FailingRevokeStore {
    async fn consume(
        &self,
        _token_hash: &nebula_storage_port::dto::resume_token::TokenHash,
    ) -> Result<Option<ResumeTokenRow>, StorageError> {
        Ok(None)
    }

    async fn revoke_on_terminal(
        &self,
        _scope: &Scope,
        _execution_id: &str,
    ) -> Result<u64, StorageError> {
        Err(StorageError::Connection(
            "injected revoke failure (best-effort posture test)".to_owned(),
        ))
    }
}

// ── Harness ─────────────────────────────────────────────────────────────────────

/// Store-backed engine wired with a shared in-memory resume-token store so the
/// tokens the engine mints at park are observable from the test via
/// [`InMemoryResumeTokenStore::token_count_for_test`].
struct RevokeHarness {
    dispatch: EngineControlDispatch,
    execution: Arc<InMemoryExecutionStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl RevokeHarness {
    /// Build a harness; `resume_tokens` is the store wired into the engine —
    /// pass `None` to use the shared in-memory store (the default), or `Some`
    /// to inject a custom store (e.g. the failing one).
    async fn new(resume_tokens: Option<Arc<dyn ResumeTokenStore>>) -> Self {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.w_s3e.webhook_park"),
                "WebhookParkNode",
                "W-S3e revoke-on-terminal integration test stub",
            ),
            WebhookParkNode,
        );
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.w_s3e.echo"),
                "EchoNode",
                "W-S3e downstream echo stub",
            ),
            EchoNode,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                registry,
                runner,
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .unwrap(),
        );

        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = Arc::new(InMemoryWorkflowVersionStore::new());
        let workflow = Arc::new(nebula_storage::InMemoryWorkflowStore::new_with_versions(
            &versions,
        ));

        let resume_tokens =
            resume_tokens.unwrap_or_else(|| Arc::new(execution.resume_token_store()));

        let execution_stores = nebula_engine::ExecutionStores {
            execution: execution.clone(),
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            resume_tokens,
        };
        let workflow_stores = nebula_engine::WorkflowStores {
            workflow,
            versions: versions.clone(),
        };

        let engine = Arc::new(
            WorkflowEngine::new(runtime, metrics)
                .unwrap()
                .with_execution_stores(execution_stores)
                .with_workflow_stores(workflow_stores),
        );
        let dispatch = EngineControlDispatch::new(Arc::clone(&engine), execution.clone());

        Self {
            dispatch,
            execution,
            versions,
        }
    }

    /// Build a harness that injects an [`ArmableConflictStore`] as the
    /// `ExecutionStore` seen by the engine, while retaining the underlying
    /// `InMemoryExecutionStore` for the token-store probe. The returned
    /// `Arc<ArmableConflictStore>` lets the test arm the interceptor between
    /// `dispatch_start` and `dispatch_resume`.
    async fn new_with_cas_interceptor() -> (Self, Arc<ArmableConflictStore>) {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.w_s3e.webhook_park"),
                "WebhookParkNode",
                "W-S3e revoke-on-terminal integration test stub",
            ),
            WebhookParkNode,
        );
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.w_s3e.echo"),
                "EchoNode",
                "W-S3e downstream echo stub",
            ),
            EchoNode,
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(ActionResult::success(input)) })
        });
        let runner = Arc::new(InProcessRunner::new(executor));
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                registry,
                runner,
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .unwrap(),
        );

        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = Arc::new(InMemoryWorkflowVersionStore::new());
        let workflow = Arc::new(nebula_storage::InMemoryWorkflowStore::new_with_versions(
            &versions,
        ));

        let interceptor = Arc::new(ArmableConflictStore::new(Arc::clone(&execution)));
        let resume_tokens = Arc::new(execution.resume_token_store());

        let execution_stores = nebula_engine::ExecutionStores {
            // The engine sees the interceptor; the harness holds the real store.
            execution: Arc::clone(&interceptor) as Arc<dyn ExecutionStore>,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            resume_tokens,
        };
        let workflow_stores = nebula_engine::WorkflowStores {
            workflow,
            versions: versions.clone(),
        };

        let engine = Arc::new(
            WorkflowEngine::new(runtime, metrics)
                .unwrap()
                .with_execution_stores(execution_stores)
                .with_workflow_stores(workflow_stores),
        );
        // EngineControlDispatch reads status from the interceptor so it sees the
        // same row version the engine CAS'd against.
        let dispatch = EngineControlDispatch::new(
            Arc::clone(&engine),
            Arc::clone(&interceptor) as Arc<dyn ExecutionStore>,
        );

        let harness = Self {
            dispatch,
            execution,
            versions,
        };
        (harness, interceptor)
    }

    /// The shared in-memory token store (same `SharedState` the engine writes to).
    fn token_store(&self) -> InMemoryResumeTokenStore {
        self.execution.resume_token_store()
    }

    /// Persist a two-node workflow `park → echo` (downstream gate) and return its id.
    async fn persist_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = Utc::now();
        let park = node_key!("park_node");
        let echo = node_key!("echo_node");
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "w-s3e-revoke-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![
                NodeDefinition::new(
                    park.clone(),
                    "WebhookParkNode",
                    "core",
                    "test.w_s3e.webhook_park",
                )
                .unwrap(),
                NodeDefinition::new(echo.clone(), "EchoNode", "core", "test.w_s3e.echo").unwrap(),
            ],
            connections: vec![Connection::new(park, echo)],
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
        self.versions
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                WorkflowVersionRecord {
                    workflow_id: workflow_id.to_string(),
                    number: 0,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(&wf).unwrap(),
                },
            )
            .await
            .unwrap();
        workflow_id
    }

    /// Persist a `Created` execution row (mirrors the API plane pre-enqueue write).
    async fn persist_created_execution(&self, workflow_id: nebula_core::WorkflowId) -> ExecutionId {
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
        exec_state.set_workflow_input(serde_json::json!(null));
        let state_json = serde_json::to_value(&exec_state).unwrap();
        self.execution
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
                &workflow_id.to_string(),
                state_json,
            )
            .await
            .unwrap();
        execution_id
    }

    /// Read the persisted `ExecutionStatus`.
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

    /// Force the persisted execution `status` under a fencing token — mirrors
    /// how the API `cancel_execution` handler writes `Cancelled` before the
    /// `Cancel` control command drains. Node states are left untouched.
    async fn force_status(&self, execution_id: ExecutionId, status: ExecutionStatus) {
        let scope = nebula_engine::store_seam::single_tenant_scope();
        let id = execution_id.to_string();
        let token = self
            .execution
            .acquire_lease(&scope, &id, "test-api-cancel", Duration::from_secs(30))
            .await
            .unwrap()
            .expect("lease must be free for the simulated API cancel write");
        let record = self.execution.get(&scope, &id).await.unwrap().unwrap();
        let mut state = record.state;
        state
            .as_object_mut()
            .unwrap()
            .insert("status".to_owned(), serde_json::json!(status.to_string()));
        let batch = TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(&id)
            .expected_version(record.version)
            .fencing(token)
            .new_state(state)
            .build()
            .unwrap();
        assert!(matches!(
            self.execution.commit(batch).await.unwrap(),
            TransitionOutcome::Applied { .. }
        ));
        self.execution
            .release_lease(&scope, &id, token)
            .await
            .unwrap();
    }
}

// ── SINK 1 — terminal completion revokes (decisive) ─────────────────────────────

/// **W-S3e SINK 1 (decisive)**: an execution that parks on a `Webhook` wait
/// (minting a token) and is then driven to `Completed` via `dispatch_resume`
/// must have its un-consumed token revoked by `persist_final_state_port`.
///
/// Asserts the real effect with a non-destructive probe:
///   - BEFORE terminal (Paused): `token_count_for_test == 1` (token minted).
///   - AFTER terminal (Completed): `token_count_for_test == 0` (token purged).
///
/// **Falsifiability**: remove the `revoke_on_terminal` call from
/// `persist_final_state_port` → the count stays 1 after `Completed` → the
/// `== 0` assertion fails → RED.
#[tokio::test]
async fn terminal_transition_revokes_unconsumed_resume_tokens() {
    let harness = RevokeHarness::new(None).await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let exec = execution_id.to_string();

    // Park: the frontier drives `WebhookParkNode`, which parks, minting a token.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook node");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after the park"
    );

    // The token is present BEFORE the terminal transition.
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        1,
        "exactly one resume token must be minted for the parked webhook node"
    );

    // Resume: satisfy the wait and drive to Completed (SINK 1 fires here).
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await
        .expect("dispatch_resume must satisfy the wait and drive to Completed");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed after dispatch_resume"
    );

    // The token is GONE after the terminal transition — revoke fired at SINK 1.
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        0,
        "the un-consumed resume token must be revoked on the terminal transition (SINK 1)"
    );
}

// ── SINK 2 — cancel-of-parked revokes ───────────────────────────────────────────

/// **W-S3e SINK 2**: a signal-parked execution that is cancelled with NO live
/// runner (driving the `cancel_dangling_nodes_under_lease` path) must have its
/// minted token revoked in that cleanup.
///
/// **Falsifiability**: remove the `revoke_on_terminal` call from
/// `cancel_dangling_nodes_under_lease` → the token survives the cancel → the
/// `== 0` assertion fails → RED.
#[tokio::test]
async fn cancel_of_parked_execution_revokes_token() {
    let harness = RevokeHarness::new(None).await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let exec = execution_id.to_string();

    // Park (mints token).
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook node");
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        1,
        "a token must be minted before the cancel"
    );

    // The API cancel path records the terminal status BEFORE the Cancel drains.
    harness
        .force_status(execution_id, ExecutionStatus::Cancelled)
        .await;

    // No live runner: `dispatch_cancel` reaches `cancel_dangling_nodes` (SINK 2),
    // which terminalizes the parked node under a freshly-acquired lease.
    harness
        .dispatch
        .dispatch_cancel(&scope, execution_id)
        .await
        .expect("dispatch_cancel must terminalize the parked node (no live runner)");

    // The token is revoked by the cancel-of-parked cleanup.
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        0,
        "the resume token must be revoked when a parked execution is cancelled (SINK 2)"
    );
}

// ── Guard — a non-terminal commit must NOT revoke ────────────────────────────────

/// **W-S3e over-revoke guard**: the park commit itself (which leaves the
/// execution `Paused`, non-terminal) must NOT revoke the token it just minted.
/// Only a terminal transition revokes.
///
/// **Falsifiability**: move the revoke off the `is_terminal()` gate so it fires
/// on every commit → the token is gone right after park → the `== 1` assertion
/// fails → RED.
#[tokio::test]
async fn non_terminal_commit_does_not_revoke() {
    let harness = RevokeHarness::new(None).await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let exec = execution_id.to_string();

    // Park: a non-terminal (`Paused`) commit that mints the token.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook node");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be non-terminal (Paused) after the park"
    );

    // The token SURVIVES the non-terminal park commit.
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        1,
        "a non-terminal commit must NOT revoke the resume token"
    );
}

// ── Best-effort — a revoke error does NOT fail the terminal transition ──────────

/// **W-S3e best-effort posture**: if `revoke_on_terminal` errors, the terminal
/// transition must STILL succeed (the execution is already durably terminal;
/// the FK CASCADE backstop cleans up later).
///
/// **Falsifiability**: propagate the revoke error with `?` instead of logging
/// and continuing → `dispatch_resume` returns `Err` → the `expect` panics → RED.
#[tokio::test]
async fn revoke_failure_does_not_fail_terminal_transition() {
    let failing: Arc<dyn ResumeTokenStore> = Arc::new(FailingRevokeStore);
    let harness = RevokeHarness::new(Some(failing)).await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();

    // Park (the failing store's `consume`/`revoke` are not on the mint path;
    // mint rides the `TransitionBatch`, which the execution store applies).
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook node");

    // Drive to Completed: SINK 1 calls `revoke_on_terminal`, which errors — the
    // terminal transition must still succeed (best-effort).
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await
        .expect("a revoke failure must NOT fail the terminal transition (best-effort)");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must be Completed even when revoke_on_terminal errors"
    );
}

// ── SINK 1 retry arm — CAS-reconcile path revokes (P2 gap fix) ─────────────────

/// **W-S3e SINK 1 retry path (Codex P2)**: the `persist_final_state_port`
/// function has a §11.5/#333 CAS-reconcile retry — if the first `commit` sees a
/// `VersionConflict` with a non-terminal observed row, the engine refetches and
/// retries. The terminal state ACTUALLY lands in the RETRY `Applied` arm on that
/// path. This test proves the revoke fires there too.
///
/// Flow:
/// 1. Park → token minted.
/// 2. Arm the interceptor (after park returns, before resume is called).
/// 3. `dispatch_resume` → engine runs `persist_final_state_port` → first commit
///    hits the armed interceptor → `VersionConflict` returned (inner store
///    unchanged) → engine reconciles, refetches (non-terminal), retries → retry
///    `Applied` arm fires → `revoke_resume_tokens_best_effort` is called there.
/// 4. Assert token count == 0 (the retry arm revoked).
///
/// **Falsifiability**: remove the `revoke_on_terminal` call from only the RETRY
/// `Applied` arm (leave the first arm) → the interceptor causes the engine to
/// land on the retry arm exclusively → the `== 0` assertion fails (token stays
/// 1) → RED.
///
/// **Why `ArmableConflictStore` rather than racing goroutines**: the inner
/// `InMemoryExecutionStore::commit` holds its mutex for the full duration of the
/// call — there is no async yield point between the engine's `get` and `commit`
/// inside the reconcile path. An out-of-band concurrent bump cannot physically
/// race into that window. The interceptor is the only clean way to inject a
/// `VersionConflict` without invasive test-only hooks in production code.
#[tokio::test]
async fn cas_retry_terminal_path_revokes_token() {
    let (harness, interceptor) = RevokeHarness::new_with_cas_interceptor().await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let exec = execution_id.to_string();

    // Park: the frontier drives `WebhookParkNode`, which parks, minting a token.
    // The interceptor is NOT yet armed, so this commit goes straight through.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook node");
    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after the park"
    );
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        1,
        "a resume token must be minted by the park"
    );

    // Arm the interceptor NOW — the next `commit` call (from `dispatch_resume`'s
    // `persist_final_state_port`) will return `VersionConflict`, driving the
    // engine into the CAS-reconcile retry path.
    interceptor.arm();

    // Resume: drives to Completed via the CAS-reconcile retry path.
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await
        .expect("dispatch_resume must succeed via the CAS-retry path");

    // Confirm the interceptor actually fired (validates test correctness — if
    // it did NOT fire, the test would pass for the wrong reason because the
    // first Applied arm revoked instead).
    assert!(
        interceptor.has_fired(),
        "the CAS-conflict interceptor must have fired during dispatch_resume"
    );

    assert_eq!(
        harness.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must reach Completed via the retry path"
    );

    // The token must be gone — revoked by the RETRY Applied arm (not the first).
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        0,
        "the resume token must be revoked on the CAS-retry terminal path (SINK 1 retry arm)"
    );
}

// ── SINK 1 honor-external-terminal path — API-cancel writer gap (CodeRabbit Major) ─

/// **W-S3e SINK 1 honor-external path (CodeRabbit Major, engine.rs:6057)**: when
/// `persist_final_state_port` hits a `VersionConflict`, refetches, and observes a
/// TERMINAL external state (§11.5/#333 line ~6051), it honors the external terminal
/// without re-committing. Before the fix, this path had NO revoke — the fix in this
/// session calls `revoke_resume_tokens_best_effort` there. This test exercises that
/// exact path.
///
/// **Why this gap matters**: the external writer on this path may be a NON-engine path
/// (e.g. the API `cancel_execution` handler, which writes `Cancelled` directly without
/// calling `revoke_on_terminal`). Engine sinks (persist_final_state_port Applied /
/// cancel_dangling_nodes_under_lease Applied) DO revoke, but a non-engine writer does
/// not. The engine observing that external terminal was the only remaining revoke gap.
///
/// Flow:
/// 1. Park → token minted.
/// 2. Arm with a terminal-get override (`"cancelled"`) — after the interceptor fires,
///    the reconcile `get` returns a row with status patched to `"cancelled"`.
/// 3. `dispatch_resume` → engine runs `persist_final_state_port` → first commit hits
///    the armed interceptor → `VersionConflict` returned (inner store unchanged) →
///    engine reconcile `get` returns the patched-terminal row → engine calls
///    `revoke_resume_tokens_best_effort` THEN `return Ok(observed_status_enum)`.
/// 4. Assert token count == 0 (the honor-external path revoked).
///
/// **Falsifiability**: remove the `revoke_resume_tokens_best_effort` call from the
/// honor-external branch (~line 6051) → the token count stays 1 after the honor-
/// external `Ok(observed_status_enum)` return → the `== 0` assertion fails → RED.
///
/// **Test correctness guard**: `interceptor.has_fired()` ensures the interceptor
/// actually triggered (i.e. the test DID exercise the honor-external path, not a
/// different code path). If the interceptor does NOT fire, `dispatch_resume` would
/// complete via the normal first-Applied arm — and the test would pass for the wrong
/// reason (that arm already had the revoke from the original W-S3e PR).
#[tokio::test]
async fn honor_external_terminal_path_revokes_token() {
    let (harness, interceptor) = RevokeHarness::new_with_cas_interceptor().await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let exec = execution_id.to_string();

    // Park: mints a resume token. The interceptor is NOT yet armed.
    harness
        .dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook node");
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        1,
        "a resume token must be minted by the park"
    );

    // Arm the interceptor with a terminal-get override. On the NEXT commit that
    // carries a terminal status, the interceptor returns `VersionConflict`, and the
    // subsequent `get` (reconcile refetch) returns a row with status `"cancelled"`.
    // This drives the engine into the honor-external-terminal branch (~line 6051).
    interceptor.arm_with_terminal_get("cancelled");

    // Resume: `persist_final_state_port` will hit `VersionConflict` → refetch →
    // observe `"cancelled"` (terminal) → revoke tokens → honor external.
    // `dispatch_resume` completes `Ok(())` because `execute_workflow` treats the
    // honored external_status as a valid terminal outcome.
    harness
        .dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await
        .expect("dispatch_resume must complete Ok even when honoring external terminal");

    // Validity guard: the interceptor must have fired; if it did NOT, the engine
    // took the first-Applied arm (which already revoked) and the test proves nothing.
    assert!(
        interceptor.has_fired(),
        "the CAS-conflict interceptor must have fired so the test exercises the \
         honor-external-terminal branch, not the first-Applied branch"
    );

    // The token is revoked by the honor-external branch — the gap fixed in this session.
    assert_eq!(
        harness.token_store().token_count_for_test(&scope, &exec),
        0,
        "the resume token must be revoked on the honor-external-terminal path (CodeRabbit Major)"
    );
}
