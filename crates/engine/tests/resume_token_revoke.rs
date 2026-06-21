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
    sync::{Arc, OnceLock},
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
    Scope, StorageError, TransitionBatch, TransitionOutcome,
    dto::{WorkflowVersionRecord, resume_token::ResumeTokenRow},
    store::{ExecutionStore, ResumeTokenStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

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
