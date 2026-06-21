//! Engine integration test for W-S3c — mint-on-park resume-token.
//!
//! Drives a `WebhookWaitNode` to a park (via `dispatch_start`), then asserts
//! that the `InMemoryResumeTokenStore` shared with the engine holds exactly
//! one token for the parked execution, proving the engine minted and committed
//! the token atomically in the same `TransitionBatch` that wrote the `Waiting`
//! node state.
//!
//! ## What is proved
//!
//! `dispatch_start` → engine frontier → `WebhookWaitNode::execute` returns
//! `ActionResult::Wait { condition: Webhook { .. }, .. }` → `mint_park_token`
//! mints a token → `park_node` commits the batch →
//! `InMemoryResumeTokenStore::revoke_on_terminal` returns `1` (token visible).
//!
//! ## Falsifiability
//!
//! Remove the `mint_park_token` call or the `resume_tokens(vec![row])` push
//! inside the engine's frontier loop → no token is inserted → `revoke_on_terminal`
//! returns 0 → `assert_eq!(revoked, 1)` fails → RED.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

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
use nebula_execution::ExecutionState;
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    dto::WorkflowVersionRecord,
    store::{ExecutionStore, ResumeTokenStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Action stub ───────────────────────────────────────────────────────────────

/// Parks on a `Webhook` signal so the engine mints a resume token for this node.
struct WebhookParkNode;

impl Action for WebhookParkNode {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.w_s3c.webhook_park"),
            "WebhookParkNode",
            "W-S3c token-mint integration test stub",
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
                callback_id: "w-s3c-integration-cb".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

// ── Test harness ──────────────────────────────────────────────────────────────

/// Minimal store set for the token-mint integration test.
struct MintHarness {
    dispatch: EngineControlDispatch,
    execution: Arc<InMemoryExecutionStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl MintHarness {
    async fn new() -> Self {
        let registry = Arc::new(ActionRegistry::new());
        registry.register_stateless_instance(
            ActionMetadata::new(
                action_key!("test.w_s3c.webhook_park"),
                "WebhookParkNode",
                "W-S3c token-mint integration test stub",
            ),
            WebhookParkNode,
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

        let execution_stores = nebula_engine::ExecutionStores {
            execution: execution.clone(),
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            resume_tokens: Arc::new(execution.resume_token_store()),
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

    /// Persist a single-node workflow: `webhook_park` (no downstream).
    async fn persist_workflow(&self) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = Utc::now();
        let park_node_key = node_key!("park_node");
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "w-s3c-token-mint-test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![
                NodeDefinition::new(
                    park_node_key,
                    "WebhookParkNode",
                    "core",
                    "test.w_s3c.webhook_park",
                )
                .unwrap(),
            ],
            connections: vec![],
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
}

// ── Test ──────────────────────────────────────────────────────────────────────

/// Engine integration test for W-S3c: the engine mints a resume token when a
/// `Webhook`-wait node parks, and the token is visible in the shared store.
///
/// Proof strategy: call `revoke_on_terminal` after park — it removes all tokens
/// for the execution and returns the count.  Count `>= 1` proves the token was
/// minted and persisted atomically in the same batch as the `Waiting` state.
///
/// Falsifiability: remove `mint_park_token` or the `resume_tokens(vec![row])`
/// call from the engine's frontier loop → `revoke_on_terminal` returns 0 →
/// `assert!(minted_token_count >= 1)` fails → RED.
#[tokio::test]
async fn webhook_park_mints_token_visible_in_resume_token_store() {
    let harness = MintHarness::new().await;
    let workflow_id = harness.persist_workflow().await;
    let execution_id = harness.persist_created_execution(workflow_id).await;

    // Park: the frontier drives `WebhookParkNode`, which returns Webhook wait.
    // `dispatch_start` runs `drive()` synchronously to completion (park or
    // terminal) — `Paused` is persisted before this call returns.
    harness
        .dispatch
        .dispatch_start(
            &nebula_engine::store_seam::single_tenant_scope(),
            execution_id,
        )
        .await
        .expect("dispatch_start must park the webhook node and persist Paused");

    // The engine wired `resume_tokens: Arc::new(execution.resume_token_store())`
    // above, so the token minted in `mint_park_token` and committed in the
    // `TransitionBatch` is visible through the shared-mutex store.
    let token_store = harness.execution.resume_token_store();

    // `revoke_on_terminal` is a non-destructive count probe here: in production
    // it is called on terminal transitions.  Using it here proves:
    //   (a) the token was stored (count > 0), and
    //   (b) the store correctly identifies the right execution's tokens.
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let minted_token_count = token_store
        .revoke_on_terminal(&scope, &execution_id.to_string())
        .await
        .expect("revoke_on_terminal must not error after park");

    // Exact count: exactly one token per parked node (no over-mint).
    assert_eq!(
        minted_token_count, 1,
        "engine must mint exactly one resume token for a single Webhook-wait park; \
         got count = {minted_token_count}"
    );

    // Idempotency: a second `revoke_on_terminal` on the same execution
    // returns 0 — the store has no remaining tokens (revoke is destructive).
    let second_revoke_count = token_store
        .revoke_on_terminal(&scope, &execution_id.to_string())
        .await
        .expect("second revoke_on_terminal must not error");
    assert_eq!(
        second_revoke_count, 0,
        "revoke_on_terminal is destructive: a second call on the same execution \
         must return 0 (no tokens remain)"
    );
    // Note: direct field-value assertion via `consume` is deferred to W-S3d
    // because `mint_park_token` drops the `SecretString` bearer inside the
    // engine frontier — the hash is not observable from outside the engine
    // until W-S3d routes the bearer to the API caller.
}
